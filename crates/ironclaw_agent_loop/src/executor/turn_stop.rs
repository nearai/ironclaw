use async_trait::async_trait;
use ironclaw_turns::LoopExit;

use crate::{
    state::LoopExecutionState,
    strategies::{StopKind, StopOutcome, TurnSummary},
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
        // `decide` is also a cancellation boundary for callers that split
        // observation from the terminal decision.
        match ctx
            .planner
            .stop()
            .should_stop_after_observed_turn(&state, &input.summary)
            .await
        {
            StopOutcome::Stop { kind } => {
                state = match CheckpointStage.cancel_if_requested(ctx, state).await? {
                    CancelCheck::Continue(state) => *state,
                    CancelCheck::Exit(exit) => return Ok(StopStep::Exit(exit)),
                };
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
