use std::collections::BTreeMap;

use crate::state::{IndexedMessageKind, LoopExecutionState, MessageIndexEntry};
use ironclaw_host_api::CapabilityId;
use ironclaw_turns::run_profile::{CompactionInitiator, LoopRunContext, PromptContextTokenBudget};

/// Decides whether to replace older transcript context with a host-managed summary.
///
/// The strategy is pure policy: it reads durable compaction state and returns
/// either `Skip` or the inclusive user-message boundary the executor should
/// compact through. State mutation, transcript reads, inference, persistence,
/// and progress events stay in the executor and host compaction port.
///
/// `Trigger.drop_through_seq` must point at a model-visible user message. The
/// host compaction port rejects non-user terminal boundaries so custom
/// strategies cannot compact through assistant, summary, or reference messages.
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

    pub(super) fn can_evaluate(&self, state: &LoopExecutionState) -> bool {
        if state.compaction_prompt.message_index.is_empty() {
            return false;
        }
        let threshold = self.prompt_context_budget.visible_transcript_tokens();
        if threshold <= self.preserve_tail_tokens {
            return false;
        }
        state.compaction_state.force_compact_on_next_iteration
            || state.compaction_prompt.observed_prompt_tokens >= threshold
    }

    pub(super) fn trigger_at(&self, drop_through_seq: u64) -> CompactionDecision {
        CompactionDecision::Trigger {
            drop_through_seq,
            preserve_tail_tokens: self.preserve_tail_tokens,
            deadline_ms: self.deadline_ms,
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
        _ctx: &LoopRunContext,
    ) -> CompactionDecision {
        if !self.can_evaluate(state) {
            return CompactionDecision::Skip;
        }
        let prompt_fingerprint = state.compaction_prompt.fingerprint();
        if state.compaction_state.force_compact_on_next_iteration {
            return latest_eligible_user_boundary(state, prompt_fingerprint)
                .map(|sequence| self.trigger_at(sequence))
                .unwrap_or(CompactionDecision::Skip);
        }

        tail_preserving_user_boundary(
            state,
            prompt_fingerprint,
            self.preserve_tail_tokens,
            0,
            |_| true,
        )
        .map(|sequence| self.trigger_at(sequence))
        .unwrap_or(CompactionDecision::Skip)
    }
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
mod tests {
    use super::*;
    use crate::state::{
        CompactionPromptSnapshot, CompactionStrategyState, DeferredCompactionWatermark,
        LoopExecutionState, MessageIndexEntry,
    };
    use ironclaw_host_api::CapabilityId;
    use ironclaw_turns::run_profile::PromptContextTokenBudget;

    #[test]
    fn evaluate_skips_when_message_index_is_empty() {
        let context = crate::test_support::test_run_context("compaction-strategy-empty");
        let mut state = LoopExecutionState::initial_for_run(&context);
        state.compaction_state.force_compact_on_next_iteration = true;
        let strategy = DefaultCompactionStrategy {
            prompt_context_budget: PromptContextTokenBudget::new(100, 10, 0),
            preserve_tail_tokens: 1,
            deadline_ms: 1,
        };

        assert_eq!(
            strategy.should_compact(&state, &context),
            CompactionDecision::Skip
        );
    }

    #[test]
    fn evaluate_skips_when_no_eligible_user_message_boundary_exists() {
        let context = crate::test_support::test_run_context("compaction-strategy");
        let mut state = LoopExecutionState::initial_for_run(&context);
        state.compaction_prompt =
            CompactionPromptSnapshot::from_message_index(vec![MessageIndexEntry {
                sequence: 1,
                kind: IndexedMessageKind::Assistant,
                estimated_tokens: 100,
            }]);
        let strategy = DefaultCompactionStrategy {
            prompt_context_budget: PromptContextTokenBudget::new(100, 10, 0),
            preserve_tail_tokens: 1,
            deadline_ms: 1,
        };
        assert_eq!(
            strategy.should_compact(&state, &context),
            CompactionDecision::Skip
        );
    }

    #[test]
    fn evaluate_skips_when_below_threshold_with_valid_user_boundary_and_forcing_is_off() {
        let context = crate::test_support::test_run_context("compaction-strategy-below-threshold");
        let mut state = LoopExecutionState::initial_for_run(&context);
        state.compaction_prompt = CompactionPromptSnapshot::from_message_index(vec![
            MessageIndexEntry {
                sequence: 1,
                kind: IndexedMessageKind::User,
                estimated_tokens: 20,
            },
            MessageIndexEntry {
                sequence: 2,
                kind: IndexedMessageKind::Assistant,
                estimated_tokens: 20,
            },
        ]);
        let strategy = DefaultCompactionStrategy {
            prompt_context_budget: PromptContextTokenBudget::new(100, 10, 0),
            preserve_tail_tokens: 60,
            deadline_ms: 1,
        };

        assert_eq!(
            strategy.should_compact(&state, &context),
            CompactionDecision::Skip
        );
    }

    #[test]
    fn can_evaluate_skips_when_visible_threshold_equals_preserve_tail() {
        let context = crate::test_support::test_run_context("compaction-strategy-equal-tail");
        let mut state = LoopExecutionState::initial_for_run(&context);
        state.compaction_prompt =
            CompactionPromptSnapshot::from_message_index(vec![MessageIndexEntry {
                sequence: 1,
                kind: IndexedMessageKind::User,
                estimated_tokens: 100,
            }]);
        let strategy = DefaultCompactionStrategy {
            prompt_context_budget: PromptContextTokenBudget::new(100, 10, 0),
            preserve_tail_tokens: 90,
            deadline_ms: 1,
        };

        assert_eq!(
            strategy.should_compact(&state, &context),
            CompactionDecision::Skip
        );
    }

    #[test]
    fn evaluate_triggers_at_latest_user_boundary_outside_tail() {
        let context = crate::test_support::test_run_context("compaction-strategy-trigger");
        let mut state = LoopExecutionState::initial_for_run(&context);
        state.compaction_state = CompactionStrategyState::default();
        state.compaction_prompt = CompactionPromptSnapshot::from_message_index(vec![
            MessageIndexEntry {
                sequence: 1,
                kind: IndexedMessageKind::User,
                estimated_tokens: 30,
            },
            MessageIndexEntry {
                sequence: 2,
                kind: IndexedMessageKind::Assistant,
                estimated_tokens: 30,
            },
            MessageIndexEntry {
                sequence: 3,
                kind: IndexedMessageKind::User,
                estimated_tokens: 30,
            },
            MessageIndexEntry {
                sequence: 4,
                kind: IndexedMessageKind::Assistant,
                estimated_tokens: 30,
            },
        ]);
        let strategy = DefaultCompactionStrategy {
            prompt_context_budget: PromptContextTokenBudget::new(100, 10, 0),
            preserve_tail_tokens: 60,
            deadline_ms: 7,
        };

        assert_eq!(
            strategy.should_compact(&state, &context),
            CompactionDecision::Trigger {
                drop_through_seq: 1,
                preserve_tail_tokens: 60,
                deadline_ms: 7,
            }
        );
    }

    #[test]
    fn evaluate_triggers_when_newest_assistant_block_exceeds_tail_budget() {
        let context = crate::test_support::test_run_context("compaction-strategy-tail-overflow");
        let mut state = LoopExecutionState::initial_for_run(&context);
        state.compaction_state = CompactionStrategyState::default();
        state.compaction_prompt = CompactionPromptSnapshot::from_message_index(vec![
            MessageIndexEntry {
                sequence: 1,
                kind: IndexedMessageKind::User,
                estimated_tokens: 10,
            },
            MessageIndexEntry {
                sequence: 2,
                kind: IndexedMessageKind::Assistant,
                estimated_tokens: 100,
            },
        ]);
        let strategy = DefaultCompactionStrategy {
            prompt_context_budget: PromptContextTokenBudget::new(100, 10, 0),
            preserve_tail_tokens: 60,
            deadline_ms: 7,
        };

        assert_eq!(
            strategy.should_compact(&state, &context),
            CompactionDecision::Trigger {
                drop_through_seq: 1,
                preserve_tail_tokens: 60,
                deadline_ms: 7,
            }
        );
    }

    #[test]
    fn evaluate_skips_when_latest_user_boundary_was_already_compacted() {
        let context = crate::test_support::test_run_context("compaction-strategy-compacted");
        let mut state = LoopExecutionState::initial_for_run(&context);
        state.compaction_state.last_compacted_through_seq = Some(3);
        state.compaction_state.force_compact_on_next_iteration = true;
        state.compaction_prompt = CompactionPromptSnapshot::from_message_index(vec![
            MessageIndexEntry {
                sequence: 1,
                kind: IndexedMessageKind::User,
                estimated_tokens: 10,
            },
            MessageIndexEntry {
                sequence: 2,
                kind: IndexedMessageKind::Assistant,
                estimated_tokens: 10,
            },
            MessageIndexEntry {
                sequence: 3,
                kind: IndexedMessageKind::User,
                estimated_tokens: 10,
            },
            MessageIndexEntry {
                sequence: 4,
                kind: IndexedMessageKind::Assistant,
                estimated_tokens: 100,
            },
        ]);
        let strategy = DefaultCompactionStrategy {
            prompt_context_budget: PromptContextTokenBudget::new(100, 10, 0),
            preserve_tail_tokens: 60,
            deadline_ms: 7,
        };

        assert_eq!(
            strategy.should_compact(&state, &context),
            CompactionDecision::Skip
        );
    }

    #[test]
    fn evaluate_skips_previously_deferred_boundary_when_forced() {
        let context = crate::test_support::test_run_context("compaction-strategy-deferred");
        let mut state = LoopExecutionState::initial_for_run(&context);
        state.compaction_state.force_compact_on_next_iteration = true;
        state.compaction_prompt = CompactionPromptSnapshot::from_message_index(vec![
            MessageIndexEntry {
                sequence: 1,
                kind: IndexedMessageKind::User,
                estimated_tokens: 10,
            },
            MessageIndexEntry {
                sequence: 2,
                kind: IndexedMessageKind::Assistant,
                estimated_tokens: 10,
            },
            MessageIndexEntry {
                sequence: 3,
                kind: IndexedMessageKind::User,
                estimated_tokens: 10,
            },
        ]);
        state.compaction_state.last_deferred = Some(DeferredCompactionWatermark {
            through_seq: 3,
            prompt_fingerprint: state.compaction_prompt.fingerprint(),
        });
        let strategy = DefaultCompactionStrategy {
            prompt_context_budget: PromptContextTokenBudget::new(100, 10, 0),
            preserve_tail_tokens: 1,
            deadline_ms: 7,
        };

        assert_eq!(
            strategy.should_compact(&state, &context),
            CompactionDecision::Trigger {
                drop_through_seq: 1,
                preserve_tail_tokens: 1,
                deadline_ms: 7,
            }
        );
    }

    #[test]
    fn evaluate_skips_deferred_boundary_in_threshold_overflow_path() {
        let context =
            crate::test_support::test_run_context("compaction-strategy-deferred-threshold");
        let mut state = LoopExecutionState::initial_for_run(&context);
        state.compaction_prompt = CompactionPromptSnapshot::from_message_index(vec![
            MessageIndexEntry {
                sequence: 1,
                kind: IndexedMessageKind::User,
                estimated_tokens: 50,
            },
            MessageIndexEntry {
                sequence: 2,
                kind: IndexedMessageKind::Assistant,
                estimated_tokens: 50,
            },
            MessageIndexEntry {
                sequence: 3,
                kind: IndexedMessageKind::User,
                estimated_tokens: 50,
            },
            MessageIndexEntry {
                sequence: 4,
                kind: IndexedMessageKind::Assistant,
                estimated_tokens: 50,
            },
        ]);
        state.compaction_state.last_deferred = Some(DeferredCompactionWatermark {
            through_seq: 3,
            prompt_fingerprint: state.compaction_prompt.fingerprint(),
        });
        let strategy = DefaultCompactionStrategy {
            prompt_context_budget: PromptContextTokenBudget::new(100, 10, 0),
            preserve_tail_tokens: 60,
            deadline_ms: 7,
        };

        assert_eq!(
            strategy.should_compact(&state, &context),
            CompactionDecision::Trigger {
                drop_through_seq: 1,
                preserve_tail_tokens: 60,
                deadline_ms: 7,
            }
        );
    }

    #[test]
    fn evaluate_skips_when_only_deferred_boundary_is_eligible_in_threshold_overflow_path() {
        let context = crate::test_support::test_run_context("compaction-strategy-deferred-skip");
        let mut state = LoopExecutionState::initial_for_run(&context);
        state.compaction_prompt = CompactionPromptSnapshot::from_message_index(vec![
            MessageIndexEntry {
                sequence: 1,
                kind: IndexedMessageKind::User,
                estimated_tokens: 50,
            },
            MessageIndexEntry {
                sequence: 2,
                kind: IndexedMessageKind::Assistant,
                estimated_tokens: 50,
            },
        ]);
        state.compaction_state.last_deferred = Some(DeferredCompactionWatermark {
            through_seq: 1,
            prompt_fingerprint: state.compaction_prompt.fingerprint(),
        });
        let strategy = DefaultCompactionStrategy {
            prompt_context_budget: PromptContextTokenBudget::new(100, 10, 0),
            preserve_tail_tokens: 60,
            deadline_ms: 7,
        };

        assert_eq!(
            strategy.should_compact(&state, &context),
            CompactionDecision::Skip
        );
    }

    #[test]
    fn evaluate_retries_deferred_boundary_after_prompt_snapshot_changes() {
        let context = crate::test_support::test_run_context("compaction-strategy-deferred-changed");
        let mut state = LoopExecutionState::initial_for_run(&context);
        state.compaction_state.last_deferred = Some(DeferredCompactionWatermark {
            through_seq: 3,
            prompt_fingerprint: 42,
        });
        state.compaction_state.force_compact_on_next_iteration = true;
        state.compaction_prompt = CompactionPromptSnapshot::from_message_index(vec![
            MessageIndexEntry {
                sequence: 1,
                kind: IndexedMessageKind::User,
                estimated_tokens: 10,
            },
            MessageIndexEntry {
                sequence: 2,
                kind: IndexedMessageKind::Assistant,
                estimated_tokens: 10,
            },
            MessageIndexEntry {
                sequence: 3,
                kind: IndexedMessageKind::User,
                estimated_tokens: 10,
            },
        ]);
        let strategy = DefaultCompactionStrategy {
            prompt_context_budget: PromptContextTokenBudget::new(100, 10, 0),
            preserve_tail_tokens: 1,
            deadline_ms: 7,
        };

        assert_eq!(
            strategy.should_compact(&state, &context),
            CompactionDecision::Trigger {
                drop_through_seq: 3,
                preserve_tail_tokens: 1,
                deadline_ms: 7,
            }
        );
    }

    #[test]
    fn evaluate_retries_after_transcript_advances_past_deferred_boundary() {
        let context = crate::test_support::test_run_context("compaction-strategy-deferred-newer");
        let mut state = LoopExecutionState::initial_for_run(&context);
        state.compaction_state.force_compact_on_next_iteration = true;
        state.compaction_prompt = CompactionPromptSnapshot::from_message_index(vec![
            MessageIndexEntry {
                sequence: 1,
                kind: IndexedMessageKind::User,
                estimated_tokens: 10,
            },
            MessageIndexEntry {
                sequence: 2,
                kind: IndexedMessageKind::Assistant,
                estimated_tokens: 10,
            },
            MessageIndexEntry {
                sequence: 3,
                kind: IndexedMessageKind::User,
                estimated_tokens: 10,
            },
            MessageIndexEntry {
                sequence: 4,
                kind: IndexedMessageKind::Assistant,
                estimated_tokens: 10,
            },
            MessageIndexEntry {
                sequence: 5,
                kind: IndexedMessageKind::User,
                estimated_tokens: 10,
            },
        ]);
        state.compaction_state.last_deferred = Some(DeferredCompactionWatermark {
            through_seq: 3,
            prompt_fingerprint: state.compaction_prompt.fingerprint(),
        });
        let strategy = DefaultCompactionStrategy {
            prompt_context_budget: PromptContextTokenBudget::new(100, 10, 0),
            preserve_tail_tokens: 1,
            deadline_ms: 7,
        };

        assert_eq!(
            strategy.should_compact(&state, &context),
            CompactionDecision::Trigger {
                drop_through_seq: 5,
                preserve_tail_tokens: 1,
                deadline_ms: 7,
            }
        );
    }

    #[test]
    fn evaluate_uses_output_budget_when_larger_than_reserve() {
        let context = crate::test_support::test_run_context("compaction-strategy-output-budget");
        let mut state = LoopExecutionState::initial_for_run(&context);
        state.compaction_prompt = CompactionPromptSnapshot::from_message_index(vec![
            MessageIndexEntry {
                sequence: 1,
                kind: IndexedMessageKind::User,
                estimated_tokens: 40,
            },
            MessageIndexEntry {
                sequence: 2,
                kind: IndexedMessageKind::Assistant,
                estimated_tokens: 35,
            },
        ]);
        let strategy = DefaultCompactionStrategy {
            prompt_context_budget: PromptContextTokenBudget::new(100, 10, 30),
            preserve_tail_tokens: 1,
            deadline_ms: 7,
        };

        assert_eq!(
            strategy.should_compact(&state, &context),
            CompactionDecision::Trigger {
                drop_through_seq: 1,
                preserve_tail_tokens: 1,
                deadline_ms: 7,
            }
        );
    }

    #[test]
    fn tail_preserving_user_boundary_respects_minimum_tail_message_count() {
        let context = crate::test_support::test_run_context("compaction-strategy-min-tail");
        let mut state = LoopExecutionState::initial_for_run(&context);
        state.compaction_prompt = CompactionPromptSnapshot::from_message_index(vec![
            MessageIndexEntry {
                sequence: 1,
                kind: IndexedMessageKind::User,
                estimated_tokens: 10,
            },
            MessageIndexEntry {
                sequence: 2,
                kind: IndexedMessageKind::Assistant,
                estimated_tokens: 10,
            },
            MessageIndexEntry {
                sequence: 3,
                kind: IndexedMessageKind::User,
                estimated_tokens: 10,
            },
            MessageIndexEntry {
                sequence: 4,
                kind: IndexedMessageKind::Assistant,
                estimated_tokens: 10,
            },
        ]);

        let boundary = tail_preserving_user_boundary(
            &state,
            state.compaction_prompt.fingerprint(),
            1,
            2,
            |_| true,
        );

        assert_eq!(boundary, Some(1));
    }

    // --- ByteCapStrategy tests ---

    #[test]
    fn byte_cap_strategy_trips_when_capability_exceeds_cap() {
        let context = crate::test_support::test_run_context("byte-cap-policy-trips");
        let mut state = LoopExecutionState::initial_for_run(&context);
        let id = CapabilityId::new("builtin.http").expect("valid capability");
        // 32_000 is the cap; 32_001 exceeds it.
        state
            .post_capability_state
            .pending_capability_bytes
            .insert(id, 32_001);

        let strategy = ByteCapStrategy::with_defaults();
        assert_eq!(
            strategy.should_force_compact(&state),
            Some(CompactionInitiator::CapabilityResultOverflow)
        );
    }

    #[test]
    fn byte_cap_strategy_skips_when_under_threshold() {
        let context = crate::test_support::test_run_context("byte-cap-policy-under");
        let mut state = LoopExecutionState::initial_for_run(&context);
        let http_id = CapabilityId::new("builtin.http").expect("valid capability");
        let subagent_id = CapabilityId::new("builtin.spawn_subagent").expect("valid capability");
        // Both under their respective caps.
        state
            .post_capability_state
            .pending_capability_bytes
            .insert(http_id, 31_999);
        state
            .post_capability_state
            .pending_capability_bytes
            .insert(subagent_id, 47_999);

        let strategy = ByteCapStrategy::with_defaults();
        assert_eq!(strategy.should_force_compact(&state), None);
    }

    #[test]
    fn byte_cap_strategy_uses_default_cap_for_unknown_capability() {
        let context = crate::test_support::test_run_context("byte-cap-policy-unknown");
        let mut state = LoopExecutionState::initial_for_run(&context);
        let id = CapabilityId::new("custom.unknown_tool").expect("valid capability");
        // DEFAULT_FALLBACK_CAP_BYTES is 32_000; 32_001 exceeds it.
        state
            .post_capability_state
            .pending_capability_bytes
            .insert(id, ByteCapStrategy::DEFAULT_FALLBACK_CAP_BYTES + 1);

        let strategy = ByteCapStrategy::with_defaults();
        assert_eq!(
            strategy.should_force_compact(&state),
            Some(CompactionInitiator::CapabilityResultOverflow)
        );
    }

    #[test]
    fn byte_cap_strategy_empty_accumulator_returns_none() {
        let context = crate::test_support::test_run_context("byte-cap-policy-empty");
        let state = LoopExecutionState::initial_for_run(&context);
        // pending_capability_bytes is empty by default.
        let strategy = ByteCapStrategy::with_defaults();
        assert_eq!(strategy.should_force_compact(&state), None);
    }

    #[test]
    fn byte_cap_strategy_with_cap_overrides_default_cap() {
        let ctx = crate::test_support::test_run_context("byte-cap-with-cap");
        let mut state = LoopExecutionState::initial_for_run(&ctx);
        let id = CapabilityId::new("custom.large_tool").unwrap();
        state
            .post_capability_state
            .pending_capability_bytes
            .insert(id.clone(), 5_000);
        // Default cap (32_000) would NOT trip at 5_000; custom cap of 4_000 should trip.
        let strategy = ByteCapStrategy::with_defaults().with_cap(id, 4_000);
        assert_eq!(
            strategy.should_force_compact(&state),
            Some(CompactionInitiator::CapabilityResultOverflow)
        );
    }
}
