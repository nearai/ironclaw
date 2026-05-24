use super::*;

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct StopStage;

pub(super) struct StopInput {
    pub(super) state: LoopExecutionState,
    pub(super) summary: TurnSummary,
    pub(super) pending_input_ack: PendingInputAck,
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
        let mut state = input.state;
        let pending_input_ack = input.pending_input_ack;
        match ctx
            .planner
            .stop()
            .should_stop_after_turn(&state, &input.summary)
            .await
        {
            StopOutcome::Stop { stop, kind } => {
                state.stop_state = stop;
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
            StopOutcome::Continue { stop } => {
                state.stop_state = stop;
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
