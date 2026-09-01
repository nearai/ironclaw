use async_trait::async_trait;
use ironclaw_loop_contracts::{AgentLoopDriverHost, LoopExit};
use tracing::debug;

use crate::{
    state::{BoundedRing, LoopExecutionState, TerminalWarningObservation},
    strategies::{RepeatedOutputProgressStrategy, StopKind, StopOutcome, TurnSummary},
};

use super::{
    AgentLoopExecutorError, COMPLETION_NUDGE_LIMIT, CancelCheck, CheckpointStage, ExecutorStage,
    StageContext, scheduled_trigger_run,
};

/// Stop-stage helper for callers that can observe and decide back-to-back.
///
/// Reply-only executor paths that need to drain queued follow-up input before
/// the terminal stop decision must call `observe`, perform the drain, then
/// call `decide` instead of using the combined `process` entry point.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct StopStage;

pub(super) struct StopInput {
    pub(super) state: LoopExecutionState,
    pub(super) summary: TurnSummary,
}

pub(super) struct StopObservationInput {
    pub(super) state: LoopExecutionState,
    pub(super) summary: TurnSummary,
}

pub(super) enum StopObservationStep {
    Continue {
        state: Box<LoopExecutionState>,
        summary: TurnSummary,
    },
    Exit(LoopExit),
}

pub(super) enum StopStep {
    Continue {
        state: LoopExecutionState,
    },
    Stop {
        state: LoopExecutionState,
        kind: StopKind,
    },
    Exit(LoopExit),
}

#[async_trait]
impl ExecutorStage<StopInput> for StopStage {
    type Output = StopStep;

    async fn process(
        &self,
        ctx: StageContext<'_>,
        input: StopInput,
    ) -> Result<StopStep, AgentLoopExecutorError> {
        match self
            .observe(
                ctx,
                StopObservationInput {
                    state: input.state,
                    summary: input.summary,
                },
            )
            .await?
        {
            StopObservationStep::Continue { state, summary } => {
                self.decide(
                    ctx,
                    StopInput {
                        state: *state,
                        summary,
                    },
                )
                .await
            }
            StopObservationStep::Exit(exit) => Ok(StopStep::Exit(exit)),
        }
    }
}

impl StopStage {
    /// Apply the fresh prepared-turn completion transition after the stop
    /// strategy has decided. Resume and `SkipModel` callers intentionally do
    /// not use this entry point.
    pub(super) fn apply_fresh_completion_nudge(
        &self,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        step: StopStep,
    ) -> StopStep {
        let StopStep::Stop {
            state: mut stop_state,
            kind,
        } = step
        else {
            return step;
        };
        if !completion_nudge_should_fire(host, &stop_state, &kind) {
            return StopStep::Stop {
                state: stop_state,
                kind,
            };
        }

        // Re-enter the loop with the full tool surface and a completion
        // directive, mirroring a drained-follow-up continuation.
        stop_state.completion_nudges_used += 1;
        stop_state.completion_nudge_pending = true;
        stop_state.last_reply_trailed_off = false;
        stop_state.last_reply_empty = false;
        stop_state.last_reply_ended_with_question = false;
        debug!(
            iteration = stop_state.iteration,
            ?kind,
            completion_nudges_used = stop_state.completion_nudges_used,
            "agent loop issuing tools-capable completion nudge instead of stopping"
        );
        StopStep::Continue { state: stop_state }
    }

    pub(super) async fn observe(
        &self,
        ctx: StageContext<'_>,
        input: StopObservationInput,
    ) -> Result<StopObservationStep, AgentLoopExecutorError> {
        let mut state = input.state;
        state.stop_state = ctx
            .planner
            .stop()
            .observe_completed_turn(&state, &input.summary)
            .await;
        state = match CheckpointStage.cancel_if_requested(ctx, state).await? {
            CancelCheck::Continue(state) => *state,
            CancelCheck::Exit(exit) => return Ok(StopObservationStep::Exit(exit)),
        };
        Ok(StopObservationStep::Continue {
            state: Box::new(state),
            summary: input.summary,
        })
    }

    pub(super) async fn decide(
        &self,
        ctx: StageContext<'_>,
        input: StopInput,
    ) -> Result<StopStep, AgentLoopExecutorError> {
        let mut state = input.state;
        // `decide` is also a cancellation boundary for callers that split
        // observation from the terminal decision.
        let outcome = ctx
            .planner
            .stop()
            .should_stop_after_observed_turn(&state, &input.summary)
            .await;
        state.terminal_warning_state.clear_active();

        match outcome {
            StopOutcome::Stop { kind } => {
                state = match CheckpointStage.cancel_if_requested(ctx, state).await? {
                    CancelCheck::Continue(state) => *state,
                    CancelCheck::Exit(exit) => return Ok(StopStep::Exit(exit)),
                };
                if schedule_no_progress_warning(&mut state, &kind) {
                    debug!(
                        iteration = state.iteration,
                        "agent loop scheduling final no-progress recovery iteration"
                    );
                    return Ok(StopStep::Continue { state });
                }
                Ok(StopStep::Stop { state, kind })
            }
            StopOutcome::Continue {} => {
                state = match CheckpointStage.cancel_if_requested(ctx, state).await? {
                    CancelCheck::Continue(state) => *state,
                    CancelCheck::Exit(exit) => return Ok(StopStep::Exit(exit)),
                };
                Ok(StopStep::Continue { state })
            }
        }
    }
}

/// Decide whether a fresh-turn stop should become one more tools-capable
/// completion iteration. Driver nudges are opt-in and capped; graceful stops
/// are nudged only for an unfinished reply (or an unattended scheduled-run
/// question), while no-progress failures and aborts remain terminal.
fn completion_nudge_should_fire(
    host: &(dyn AgentLoopDriverHost + Send + Sync),
    state: &LoopExecutionState,
    kind: &StopKind,
) -> bool {
    if !host
        .run_context()
        .resolved_run_profile
        .steering_policy
        .allow_driver_specific_nudges
        || state.completion_nudges_used >= COMPLETION_NUDGE_LIMIT
    {
        return false;
    }
    match kind {
        StopKind::NoProgressDetected => false,
        StopKind::GracefulStop => {
            state.last_reply_trailed_off
                || (scheduled_trigger_run(host) && state.last_reply_ended_with_question)
        }
        StopKind::Aborted(_) => false,
    }
}

/// Convert a strategy's first no-progress terminal into one normal loop
/// iteration with typed model-visible recovery context — for an explicit
/// non-default strategy AND for the default strategy's own windowed
/// output-repetition check (strategies/stop.rs's
/// `DefaultStopConditionStrategy::should_stop_after_observed_turn`). The
/// default strategy's separate CONSECUTIVE-call advisory renders through a
/// different path and never emits this `StopKind` on its own.
fn schedule_no_progress_warning(state: &mut LoopExecutionState, kind: &StopKind) -> bool {
    if !matches!(kind, StopKind::NoProgressDetected) {
        return false;
    }
    // Same window the stop strategy used to trigger this path
    // (`RepeatedOutputProgressStrategy`, strategies/progress.rs) — the digest
    // ring, not the bare call-signature ring, since the terminating check
    // dominates on (signature, output_digest) pairs and can trip on an
    // alternating call-signature sequence.
    let repeated_call_count = RepeatedOutputProgressStrategy::default()
        .dominant_repeated_output_count(&state.seen_capability_output_digests)
        .min(u32::MAX as usize) as u32;
    let repeated_call_count = (repeated_call_count > 1).then_some(repeated_call_count);
    let last_failure = state.recent_failure_kinds.iter().next_back().copied();
    if !state
        .terminal_warning_state
        .schedule(TerminalWarningObservation::no_progress(
            repeated_call_count,
            last_failure,
        ))
    {
        return false;
    }

    state.recent_call_signatures = BoundedRing::new();
    state.seen_capability_output_digests = BoundedRing::new();
    state.recent_output_token_counts = BoundedRing::new();
    state.stop_state.trailing_no_progress_results = 0;
    state.stop_state.repeated_call_warning = None;
    true
}
