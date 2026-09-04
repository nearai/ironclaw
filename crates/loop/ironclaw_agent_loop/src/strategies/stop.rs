//! Stop-condition strategy contract.

use async_trait::async_trait;
use ironclaw_host_api::ids::CapabilityId;
use ironclaw_host_api::prepared_context::STRUCTURED_RESULT_CAPABILITY_ID;
use ironclaw_host_api::turn::{LoopMessageRef, LoopResultRef};
use ironclaw_loop_contracts::LoopFailureKind;

use crate::state::{
    CapabilityCallSignature, LoopExecutionState, RepeatedCallWarningState, StopStrategyState,
};
use crate::strategies::RepeatedOutputProgressStrategy;

/// Observes completed turns and decides whether the loop should stop.
///
/// Observation and terminal decision are split so the executor can always
/// account for a completed turn before any follow-up input preempts final exit.
/// Async because future strategies may consult host state for milestone
/// tracking.
#[async_trait]
pub(crate) trait StopConditionStrategy: Send + Sync {
    /// Called exactly once after a turn completes to update resumable stop
    /// state.
    async fn observe_completed_turn(
        &self,
        state: &LoopExecutionState,
        just_completed: &TurnSummary,
    ) -> StopStrategyState;

    /// Called after `observe_completed_turn` has been applied to `state`.
    async fn should_stop_after_observed_turn(
        &self,
        state: &LoopExecutionState,
        just_completed: &TurnSummary,
    ) -> StopOutcome;
}

#[allow(dead_code)]
fn assert_stop_condition_strategy_object_safe(_: &dyn StopConditionStrategy) {}

/// Loop-side projection of what just happened in the completed turn.
///
/// This carries refs only. Strategies that need content must read it through
/// host ports so host-side redaction and scope policy remain authoritative.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct TurnSummary {
    pub kind: TurnEndKind,
    pub assistant_message_ref: Option<LoopMessageRef>,
    pub batch_result_refs: Vec<LoopResultRef>,
    #[serde(default)]
    pub capability_batch: CapabilityBatchTurnSummary,
}

impl TurnSummary {
    pub(crate) fn reply_only(reply_ref: LoopMessageRef) -> Self {
        Self {
            kind: TurnEndKind::ReplyOnly,
            assistant_message_ref: Some(reply_ref),
            batch_result_refs: Vec::new(),
            capability_batch: CapabilityBatchTurnSummary::default(),
        }
    }

    pub(crate) fn after_capability_batch(
        result_refs: Vec<LoopResultRef>,
        capability_batch: CapabilityBatchTurnSummary,
    ) -> Self {
        Self {
            kind: TurnEndKind::AfterCapabilityBatch,
            assistant_message_ref: None,
            batch_result_refs: result_refs,
            capability_batch,
        }
    }

    pub(crate) fn reply_rejected() -> Self {
        Self {
            kind: TurnEndKind::ReplyRejected,
            assistant_message_ref: None,
            batch_result_refs: Vec::new(),
            capability_batch: CapabilityBatchTurnSummary::default(),
        }
    }

    pub(crate) fn compaction_only() -> Self {
        Self {
            kind: TurnEndKind::CompactionOnly,
            assistant_message_ref: None,
            batch_result_refs: Vec::new(),
            capability_batch: CapabilityBatchTurnSummary::default(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct CapabilityBatchTurnSummary {
    /// Number of capability invocations in the executed batch.
    pub invocation_count: u32,
    /// Count of completed results in the batch that requested natural termination.
    pub terminate_hint_count: u32,
    /// Completed-call signatures observed in this batch.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub observed_signatures: Vec<CapabilityCallSignature>,
}

impl CapabilityBatchTurnSummary {
    pub(crate) fn for_invocation_count(invocation_count: usize) -> Self {
        Self {
            invocation_count: invocation_count as u32,
            terminate_hint_count: 0,
            observed_signatures: Vec::new(),
        }
    }

    pub(crate) fn record_result(
        &mut self,
        signature: CapabilityCallSignature,
        terminate_hint: bool,
    ) {
        push_unique_signature(&mut self.observed_signatures, signature);
        if terminate_hint {
            self.terminate_hint_count = self.terminate_hint_count.saturating_add(1);
        }
    }
}

fn push_unique_signature(
    signatures: &mut Vec<CapabilityCallSignature>,
    signature: CapabilityCallSignature,
) {
    if !signatures.contains(&signature) {
        signatures.push(signature);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
#[serde(rename_all = "snake_case")]
pub(crate) enum TurnEndKind {
    /// The model returned a reply and no capability batch executed this turn.
    ReplyOnly,
    /// The model returned capability calls and the listed refs are the
    /// finalized batch outcomes for this turn.
    AfterCapabilityBatch,
    /// The model returned a reply that was rejected before transcript
    /// finalization.
    ReplyRejected,
    /// The turn ran proactive compaction and skipped the model call entirely.
    /// No assistant reply, no capability batch — just compaction, observe stop,
    /// then iterate. Emitted via `PromptStep::SkipModel`.
    CompactionOnly,
}

/// Strategy decision after completed-turn observation has already updated
/// `stop_state`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
#[serde(rename_all = "snake_case")]
pub(crate) enum StopOutcome {
    Continue {},
    Stop { kind: StopKind },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
#[serde(rename_all = "snake_case")]
pub(crate) enum StopKind {
    /// Strategy is satisfied; the executor maps this to graceful completion.
    GracefulStop,
    /// Explicit no-progress stop requested by a non-default strategy. The
    /// executor gives the model one recovery turn before resolving it to a
    /// typed `LoopFailureKind::NoProgressDetected` failure. The default
    /// strategy's CONSECUTIVE-call check (`repetition_threshold`) stays a
    /// pure advisory and never returns this kind on its own; its independent
    /// windowed OUTPUT-repetition check does.
    NoProgressDetected,
    /// Strategy aborts with an explicit failure kind.
    Aborted(LoopFailureKind),
}

/// Reference baseline `StopConditionStrategy`.
///
/// 1. **Reply completion**: a reply-only turn means the model returned its
///    assistant answer → `Stop { GracefulStop }`.
/// 2. **Graceful terminate-hint**: every result in the just-completed batch
///    asked to terminate → `Stop { GracefulStop }`.
/// 3. **Repetition advisory**: the same `CapabilityCallSignature` is observed
///    in `repetition_threshold` (default 3) consecutive iterations → render a
///    model-visible warning. Repetition never terminates the run; deterministic
///    iteration and budget strategies remain the backstop.
/// 4. **Terminating non-progress**: delegated to `RepeatedOutputProgressStrategy`
///    (`progress.rs`), whose default threshold (8) occurrences of the SAME
///    (signature, digest) pair within the trailing default window (32) →
///    `Stop { NoProgressDetected }`. Multiset scan, catches alternation,
///    requires real output repetition — see the header disclosure. One
///    recovery turn on first occurrence; a second anywhere later terminates.
/// 5. **Rejected-reply escape**: reply admission rejects
///    `rejected_reply_threshold` replies in a row →
///    `Stop { Aborted(InvalidModelOutput) }`.
///
/// On no signal, returns `Continue`.
#[derive(Debug, Clone, Copy)]
pub struct DefaultStopConditionStrategy {
    /// Consecutive identical calls required before rendering an advisory.
    pub repetition_threshold: usize,
    /// Min trailing rejected replies required before aborting as invalid model
    /// output. Capability failures are deliberately not counted here:
    /// `LoopFailureKind` is too coarse to distinguish repeated failure from
    /// unrelated model attempts that happen to share the same category.
    pub rejected_reply_threshold: usize,
    /// Owns the trailing-window dominant-repeated-output no-progress
    /// decision (`progress.rs`). Wider than the consecutive-run advisory so
    /// an alternating pattern (A, B, A, B, ...) is still visible — this
    /// defeated `trailing_repeated_call` in #7892. Requires real OUTPUT
    /// repetition, not just a repeated call — see the plan header's
    /// precedent disclosure (PR #7531/#7486) for why this is a new
    /// mechanism, not a reinstatement of #7531's removed escalation path.
    progress: RepeatedOutputProgressStrategy,
}

impl Default for DefaultStopConditionStrategy {
    fn default() -> Self {
        Self {
            repetition_threshold: 3,
            rejected_reply_threshold: 3,
            progress: RepeatedOutputProgressStrategy::default(),
        }
    }
}

#[async_trait]
impl StopConditionStrategy for DefaultStopConditionStrategy {
    async fn observe_completed_turn(
        &self,
        state: &LoopExecutionState,
        just_completed: &TurnSummary,
    ) -> StopStrategyState {
        // Bump `turns_completed` regardless of stop/continue — every
        // completed turn counts.
        let stop_state = StopStrategyState {
            turns_completed: state.stop_state.turns_completed.saturating_add(1),
            trailing_rejected_replies: if just_completed.kind == TurnEndKind::ReplyRejected {
                state.stop_state.trailing_rejected_replies.saturating_add(1)
            } else {
                0
            },
            // Retained in the checkpoint shape for rollback compatibility;
            // heuristic no-progress is advisory-only and no longer counted.
            trailing_no_progress_results: 0,
            // Counted only by the unbound structured-output strategy; the
            // default family leaves it at rest.
            trailing_all_failed_batches: 0,
            structured_result_recorded: state.stop_state.structured_result_recorded
                || just_completed
                    .capability_batch
                    .observed_signatures
                    .iter()
                    .any(|signature| signature.name.as_str() == STRUCTURED_RESULT_CAPABILITY_ID),
            repeated_call_warning: state.stop_state.repeated_call_warning.clone(),
        };

        observe_repeated_call_warning(state, stop_state, self.repetition_threshold)
    }

    async fn should_stop_after_observed_turn(
        &self,
        state: &LoopExecutionState,
        just_completed: &TurnSummary,
    ) -> StopOutcome {
        // (a) reply completion: the executor already drained queued follow-up
        // input before asking the stop strategy, so a reply-only turn is
        // terminal for the default family.
        if just_completed.kind == TurnEndKind::ReplyOnly {
            return StopOutcome::Stop {
                kind: StopKind::GracefulStop,
            };
        }

        // (b) graceful terminate-hint: every result in the just-completed
        // batch said terminate.
        if just_completed.kind == TurnEndKind::AfterCapabilityBatch
            && just_completed.capability_batch.invocation_count > 0
            && just_completed.capability_batch.terminate_hint_count
                == just_completed.capability_batch.invocation_count
        {
            return StopOutcome::Stop {
                kind: StopKind::GracefulStop,
            };
        }

        // (b.5) terminating non-progress: the same (signature, output_digest)
        // pair dominates the trailing window of completed capability-call
        // OUTPUTS. First occurrence this run: turn_stop.rs's
        // schedule_no_progress_warning gives one more recovery turn and
        // resets both rings. Any later occurrence finalizes the typed
        // failure (TerminalWarningState::schedule is a run-lifetime one-shot
        // latch per kind — state/terminal_warning.rs:141-149).
        if self
            .progress
            .is_no_progress(&state.seen_capability_output_digests)
        {
            return StopOutcome::Stop {
                kind: StopKind::NoProgressDetected,
            };
        }

        // (c) rejected-reply escape — repeated rejected final-answer
        // candidates are invalid model output, not generic no-progress.
        // This threshold permits extra model calls after each rejection; keep
        // deployments with tight LLM budgets on a low value.
        if state.stop_state.trailing_rejected_replies as usize >= self.rejected_reply_threshold {
            return StopOutcome::Stop {
                kind: StopKind::Aborted(LoopFailureKind::InvalidModelOutput),
            };
        }

        StopOutcome::Continue {}
    }
}

/// Stop conditions for the unbound structured family (unbound turns
/// design §4.5): the run completes when the host-owned result tool records a
/// validated structured result — the completed-call signature in the batch
/// summary is the completion signal — and a run of all-failed capability
/// batches (repeated invalid result-tool arguments under a surface whose only
/// tool is the result tool) fails as `invalid_model_output`. Everything else
/// delegates to the default stop conditions.
#[derive(Debug, Clone)]
pub struct StructuredResultStopStrategy {
    inner: DefaultStopConditionStrategy,
    result_capability: CapabilityId,
    /// Trailing all-failed capability batches required to abort.
    all_failed_batch_threshold: usize,
}

impl StructuredResultStopStrategy {
    pub fn new(result_capability: CapabilityId) -> Self {
        Self {
            inner: DefaultStopConditionStrategy::default(),
            result_capability,
            all_failed_batch_threshold: 3,
        }
    }

    fn result_recorded(&self, summary: &TurnSummary) -> bool {
        summary.kind == TurnEndKind::AfterCapabilityBatch
            && summary
                .capability_batch
                .observed_signatures
                .iter()
                .any(|signature| signature.name == self.result_capability)
    }
}

#[async_trait]
impl StopConditionStrategy for StructuredResultStopStrategy {
    async fn observe_completed_turn(
        &self,
        state: &LoopExecutionState,
        just_completed: &TurnSummary,
    ) -> StopStrategyState {
        let mut stop_state = self
            .inner
            .observe_completed_turn(state, just_completed)
            .await;
        let all_failed = just_completed.kind == TurnEndKind::AfterCapabilityBatch
            && just_completed.capability_batch.invocation_count > 0
            && just_completed
                .capability_batch
                .observed_signatures
                .is_empty();
        stop_state.trailing_all_failed_batches = if all_failed {
            state
                .stop_state
                .trailing_all_failed_batches
                .saturating_add(1)
        } else {
            0
        };
        stop_state
    }

    async fn should_stop_after_observed_turn(
        &self,
        state: &LoopExecutionState,
        just_completed: &TurnSummary,
    ) -> StopOutcome {
        if self.result_recorded(just_completed) {
            return StopOutcome::Stop {
                kind: StopKind::GracefulStop,
            };
        }
        if state.stop_state.trailing_all_failed_batches as usize >= self.all_failed_batch_threshold
        {
            return StopOutcome::Stop {
                kind: StopKind::Aborted(LoopFailureKind::InvalidModelOutput),
            };
        }
        self.inner
            .should_stop_after_observed_turn(state, just_completed)
            .await
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RepeatedCallObservation {
    signature: CapabilityCallSignature,
}

fn trailing_repeated_call(
    state: &LoopExecutionState,
    threshold: usize,
) -> Option<RepeatedCallObservation> {
    if state.recent_call_signatures.same_run_length() < threshold {
        return None;
    }
    let signature = state.recent_call_signatures.iter().next_back()?.clone();
    Some(RepeatedCallObservation { signature })
}

fn observe_repeated_call_warning(
    state: &LoopExecutionState,
    mut stop_state: StopStrategyState,
    threshold: usize,
) -> StopStrategyState {
    let Some(repeated) = trailing_repeated_call(state, threshold) else {
        stop_state.repeated_call_warning = None;
        return stop_state;
    };

    stop_state.repeated_call_warning = match state.stop_state.repeated_call_warning.as_ref() {
        Some(existing) if existing.signature == repeated.signature => Some(
            RepeatedCallWarningState::rendered(existing.signature.clone()),
        ),
        _ => Some(RepeatedCallWarningState::pending_render(repeated.signature)),
    };
    stop_state
}

#[cfg(test)]
#[path = "stop_tests.rs"]
mod tests;
