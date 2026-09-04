use std::collections::BTreeMap;

use crate::state::{
    CompactionEffectivenessBaseline, IndexedMessageKind, LoopExecutionState, MessageIndexEntry,
};
use ironclaw_host_api::ids::CapabilityId;
use ironclaw_loop_contracts::{CompactionInitiator, LoopRunContext, PromptContextTokenBudget};

/// Decides whether to replace older transcript context with a host-managed summary.
///
/// The strategy is pure policy: it reads durable compaction state and returns
/// either `Skip` or the inclusive user-message boundary the executor should
/// compact through. State mutation, transcript reads, inference, persistence,
/// and progress events stay in the executor and host compaction port.
///
/// `Trigger.drop_through_seq` normally points at a model-visible user message.
/// Window-eviction recovery may instead use a finalized tool-result boundary
/// after the host reports a typed eviction watermark; the host validates that
/// special case.
pub(crate) trait CompactionStrategy: Send + Sync {
    fn should_compact(
        &self,
        state: &LoopExecutionState,
        ctx: &LoopRunContext,
    ) -> CompactionDecision;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CompactionDecision {
    Skip,
    Trigger {
        drop_through_seq: u64,
        preserve_tail_tokens: u64,
        deadline_ms: u64,
        /// Trigger-kind-specific yardstick for circuit-breaker accounting.
        /// The executor compares the refreshed post-compaction prompt
        /// estimate against it to decide whether the compaction was
        /// effective: threshold-triggered compactions carry the transcript
        /// threshold, forced compactions carry the pre-compaction prompt
        /// estimate.
        effectiveness_baseline: CompactionEffectivenessBaseline,
    },
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct DefaultCompactionStrategy {
    pub prompt_context_budget: PromptContextTokenBudget,
    pub preserve_tail_tokens: u64,
    pub deadline_ms: u64,
}

impl DefaultCompactionStrategy {
    pub const DEFAULT_PRESERVE_TAIL_TOKENS: u64 = 8_000;
    pub const DEFAULT_DEADLINE_MS: u64 = 30_000;

    /// The budget this run actually runs with: the one resolved from the
    /// run's model when present, otherwise this strategy's compiled-in
    /// default.
    pub(super) fn effective_budget(&self, ctx: &LoopRunContext) -> PromptContextTokenBudget {
        ctx.resolved_context_budget
            .unwrap_or(self.prompt_context_budget)
    }

    /// The tail this run can afford to protect: the configured tail, capped
    /// at half the visible transcript so compaction can always drop the
    /// other half.
    ///
    /// `preserve_tail_tokens` is compiled-in, but the visible transcript is
    /// run-resolved from the model's advertised context window and can be
    /// far smaller (a small-window model can derive a visible transcript
    /// below the compiled-in tail). Without this cap, both automatic and
    /// forced-recovery compaction would be permanently disabled for that
    /// run — the `can_evaluate` guard would never clear and the boundary
    /// searches would never find a tail-sized gap to cut before.
    pub(super) fn effective_preserve_tail_tokens(&self, budget: PromptContextTokenBudget) -> u64 {
        self.preserve_tail_tokens
            .min(budget.visible_transcript_tokens() / 2)
    }

    pub(super) fn can_evaluate(
        &self,
        state: &LoopExecutionState,
        budget: PromptContextTokenBudget,
    ) -> bool {
        if state.compaction_prompt.message_index.is_empty() {
            return false;
        }
        let threshold = budget.visible_transcript_tokens();
        if threshold <= self.effective_preserve_tail_tokens(budget) {
            return false;
        }
        // Forced/recovery compactions (context-overflow retry, byte-cap
        // overflow) bypass the circuit breaker: they are the loop's only
        // mechanism for shrinking an oversized prompt before a retry, so
        // suppressing them would silently rebuild the same prompt until the
        // retry budget aborts. The breaker only gates automatic
        // threshold-triggered compaction.
        if state.compaction_state.force_compact_on_next_iteration {
            return true;
        }
        !state.compaction_state.compaction_circuit_open
            && state.compaction_prompt.observed_prompt_tokens >= threshold
    }

    pub(super) fn trigger_at(
        &self,
        state: &LoopExecutionState,
        budget: PromptContextTokenBudget,
        drop_through_seq: u64,
    ) -> CompactionDecision {
        let effectiveness_baseline = if state.compaction_state.force_compact_on_next_iteration {
            CompactionEffectivenessBaseline::PreCompactionPromptTokens {
                tokens: state.compaction_prompt.observed_prompt_tokens,
            }
        } else {
            CompactionEffectivenessBaseline::TriggerThresholdTokens {
                tokens: budget.visible_transcript_tokens(),
            }
        };
        CompactionDecision::Trigger {
            drop_through_seq,
            preserve_tail_tokens: self.effective_preserve_tail_tokens(budget),
            deadline_ms: self.deadline_ms,
            effectiveness_baseline,
        }
    }
}

impl Default for DefaultCompactionStrategy {
    fn default() -> Self {
        Self {
            prompt_context_budget: PromptContextTokenBudget::default(),
            preserve_tail_tokens: Self::DEFAULT_PRESERVE_TAIL_TOKENS,
            deadline_ms: Self::DEFAULT_DEADLINE_MS,
        }
    }
}

impl CompactionStrategy for DefaultCompactionStrategy {
    fn should_compact(
        &self,
        state: &LoopExecutionState,
        ctx: &LoopRunContext,
    ) -> CompactionDecision {
        let budget = self.effective_budget(ctx);
        if !self.can_evaluate(state, budget) {
            return CompactionDecision::Skip;
        }
        let prompt_fingerprint = state.compaction_prompt.fingerprint();
        if state.compaction_state.force_compact_on_next_iteration {
            if state.compaction_state.force_compact_initiator
                == Some(CompactionInitiator::WindowEviction)
            {
                return eligible_window_eviction_boundary(state, prompt_fingerprint, None)
                    .map(|sequence| self.trigger_at(state, budget, sequence))
                    .unwrap_or(CompactionDecision::Skip);
            }
            return latest_eligible_user_boundary(state, prompt_fingerprint)
                .map(|sequence| self.trigger_at(state, budget, sequence))
                .unwrap_or(CompactionDecision::Skip);
        }

        tail_preserving_user_boundary(
            state,
            prompt_fingerprint,
            self.effective_preserve_tail_tokens(budget),
            0,
            |_| true,
        )
        .map(|sequence| self.trigger_at(state, budget, sequence))
        .unwrap_or(CompactionDecision::Skip)
    }
}

pub(super) fn eligible_window_eviction_boundary(
    state: &LoopExecutionState,
    prompt_fingerprint: u64,
    before_sequence: Option<u64>,
) -> Option<u64> {
    let watermark = state.compaction_state.window_eviction.as_ref()?;
    let eligible_kind = matches!(
        watermark.omitted_through_kind,
        ironclaw_loop_contracts::LoopContextCompactionKind::User
            | ironclaw_loop_contracts::LoopContextCompactionKind::ToolResult
    );
    let matches_deferred_boundary = state
        .compaction_state
        .last_deferred
        .is_some_and(|deferred| deferred.prompt_fingerprint == prompt_fingerprint);
    if !eligible_kind || matches_deferred_boundary {
        return None;
    }
    state
        .compaction_prompt
        .message_index
        .iter()
        .rev()
        .find(|entry| {
            matches!(
                entry.kind,
                IndexedMessageKind::User | IndexedMessageKind::ToolResult
            ) && before_sequence.is_none_or(|before| entry.sequence < before)
                && Some(entry.sequence) > state.compaction_state.last_compacted_through_seq
        })
        .map(|entry| entry.sequence)
        .or_else(|| {
            (Some(watermark.omitted_through_sequence)
                > state.compaction_state.last_compacted_through_seq)
                .then_some(watermark.omitted_through_sequence)
                .filter(|sequence| before_sequence.is_none_or(|before| *sequence < before))
        })
}

fn latest_eligible_user_boundary(
    state: &LoopExecutionState,
    prompt_fingerprint: u64,
) -> Option<u64> {
    state
        .compaction_prompt
        .message_index
        .iter()
        .rev()
        .find(|entry| is_eligible_user_boundary(entry, state, prompt_fingerprint))
        .map(|entry| entry.sequence)
}

pub(super) fn tail_preserving_user_boundary(
    state: &LoopExecutionState,
    prompt_fingerprint: u64,
    preserve_tail_tokens: u64,
    minimum_tail_messages: usize,
    boundary_guard: impl Fn(&MessageIndexEntry) -> bool,
) -> Option<u64> {
    let mut tail_tokens = 0_u64;
    let mut tail_messages = 0_usize;
    for entry in state.compaction_prompt.message_index.iter().rev() {
        if tail_tokens >= preserve_tail_tokens
            && tail_messages >= minimum_tail_messages
            && is_eligible_user_boundary(entry, state, prompt_fingerprint)
            && boundary_guard(entry)
        {
            return Some(entry.sequence);
        }
        tail_tokens = tail_tokens.saturating_add(entry.estimated_tokens);
        tail_messages = tail_messages.saturating_add(1);
    }
    None
}

pub(super) fn is_eligible_user_boundary(
    entry: &MessageIndexEntry,
    state: &LoopExecutionState,
    prompt_fingerprint: u64,
) -> bool {
    let matches_deferred_boundary = state
        .compaction_state
        .last_deferred
        .is_some_and(|watermark| {
            watermark.through_seq == entry.sequence
                && watermark.prompt_fingerprint == prompt_fingerprint
        });
    entry.kind == IndexedMessageKind::User
        && Some(entry.sequence) > state.compaction_state.last_compacted_through_seq
        && !matches_deferred_boundary
}

/// Proactive compaction trigger evaluated by `PostCapabilityStage` after a
/// capability batch flushes. Inspects per-capability byte accounting on
/// `state.post_capability_state.pending_capability_bytes` and decides whether
/// any individual capability has exceeded its configured cap, returning the
/// `CompactionInitiator` variant that should be emitted in the resulting
/// `LoopProgressEvent::CompactionStarted` event.
///
/// The name `CompactionForceStrategy` distinguishes this from `CompactionStrategy`
/// (which decides when/how to run normal compaction) — this trait specifically
/// decides whether to FORCE a compact-then-skip-model on the next iteration
/// based on per-capability byte accounting.
///
/// Future impls (e.g. `BudgetFractionStrategy` for #4311) drop in alongside
/// `ByteCapStrategy` without changing call sites.
pub(crate) trait CompactionForceStrategy: Send + Sync {
    fn should_force_compact(&self, state: &LoopExecutionState) -> Option<CompactionInitiator>;
}

/// Per-capability byte-cap compaction force strategy. Trips compaction when any
/// single capability id has accumulated more than its configured byte cap in
/// `pending_capability_bytes` during the current turn.
///
/// v2 (`BudgetFractionStrategy`) will land alongside this once #4311 budget
/// governance collapse merges.
#[derive(Debug, Clone)]
pub(crate) struct ByteCapStrategy {
    caps: BTreeMap<CapabilityId, u64>,
    default_cap: u64,
}

impl ByteCapStrategy {
    /// Default cap applied to any capability not explicitly listed.
    pub const DEFAULT_FALLBACK_CAP_BYTES: u64 = 32_000;

    /// Built-in default caps. Override or extend via `with_cap`.
    pub fn with_defaults() -> Self {
        let mut caps = BTreeMap::new();
        // spawn_subagent results can carry larger structured payloads.
        caps.insert(
            CapabilityId::new("builtin.spawn_subagent").expect("builtin capability id"), // safety: compile-time constant builtin id, structurally valid by construction
            48_000,
        );
        // http + web_fetch occasionally return large pages/JSON.
        caps.insert(
            CapabilityId::new("builtin.http").expect("builtin capability id"), // safety: compile-time constant builtin id, structurally valid by construction
            32_000,
        );
        caps.insert(
            CapabilityId::new("builtin.web_fetch").expect("builtin capability id"), // safety: compile-time constant builtin id, structurally valid by construction
            32_000,
        );
        Self {
            caps,
            default_cap: Self::DEFAULT_FALLBACK_CAP_BYTES,
        }
    }

    pub fn with_cap(mut self, capability_id: CapabilityId, cap_bytes: u64) -> Self {
        self.caps.insert(capability_id, cap_bytes);
        self
    }
}

impl Default for ByteCapStrategy {
    fn default() -> Self {
        Self::with_defaults()
    }
}

impl CompactionForceStrategy for ByteCapStrategy {
    fn should_force_compact(&self, state: &LoopExecutionState) -> Option<CompactionInitiator> {
        for (capability_id, bytes) in &state.post_capability_state.pending_capability_bytes {
            let cap = self
                .caps
                .get(capability_id)
                .copied()
                .unwrap_or(self.default_cap);
            if *bytes > cap {
                return Some(CompactionInitiator::CapabilityResultOverflow);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests;
