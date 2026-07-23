use async_trait::async_trait;
use ironclaw_turns::LoopExit;
use tracing::debug;

use crate::{
    state::{BoundedRing, LoopExecutionState, TerminalWarningKind, TerminalWarningObservation},
    strategies::{StopKind, StopOutcome, TurnEndKind, TurnSummary},
};

use super::{
    AgentLoopExecutorError, CancelCheck, CheckpointStage, ExecutorStage, PendingInputAck,
    StageContext,
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
    pub(super) pending_input_ack: PendingInputAck,
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
        pending_input_ack: PendingInputAck,
    },
    Stop {
        state: LoopExecutionState,
        kind: StopKind,
        pending_input_ack: PendingInputAck,
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
                        pending_input_ack: input.pending_input_ack,
                    },
                )
                .await
            }
            StopObservationStep::Exit(exit) => Ok(StopStep::Exit(exit)),
        }
    }
}

impl StopStage {
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
        let pending_input_ack = input.pending_input_ack;
        let warning_turn_repeated_no_progress = state.terminal_warning_state.active()
            == Some(TerminalWarningKind::NoProgressDetected)
            && input.summary.kind == TurnEndKind::AfterCapabilityBatch
            && input.summary.capability_batch.invocation_count > 0
            && input.summary.capability_batch.no_progress_count
                == input.summary.capability_batch.invocation_count;
        // `decide` is also a cancellation boundary for callers that split
        // observation from the terminal decision.
        let outcome = if warning_turn_repeated_no_progress {
            StopOutcome::Stop {
                kind: StopKind::NoProgressDetected,
            }
        } else {
            ctx.planner
                .stop()
                .should_stop_after_observed_turn(&state, &input.summary)
                .await
        };
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
                    return Ok(StopStep::Continue {
                        state,
                        pending_input_ack,
                    });
                }
                Ok(StopStep::Stop {
                    state,
                    kind,
                    pending_input_ack,
                })
            }
            StopOutcome::Continue {} => {
                state = match CheckpointStage.cancel_if_requested(ctx, state).await? {
                    CancelCheck::Continue(state) => *state,
                    CancelCheck::Exit(exit) => return Ok(StopStep::Exit(exit)),
                };
                Ok(StopStep::Continue {
                    state,
                    pending_input_ack,
                })
            }
        }
    }
}

/// Convert the first no-progress terminal into one normal loop iteration with
/// typed model-visible recovery context. The evidence windows are reset so a
/// changed action can make progress; `TerminalWarningState::active` separately
/// makes an all-`NoChange` warning response terminal on that same turn.
fn schedule_no_progress_warning(state: &mut LoopExecutionState, kind: &StopKind) -> bool {
    if !matches!(kind, StopKind::NoProgressDetected) {
        return false;
    }
    let repeated_call_count = state
        .recent_call_signatures
        .most_common_count_in(8)
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
    state.recent_output_token_counts = BoundedRing::new();
    state.stop_state.trailing_no_progress_results = 0;
    state.stop_state.repeated_call_warning = None;
    true
}
