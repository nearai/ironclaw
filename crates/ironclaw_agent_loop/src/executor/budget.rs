use async_trait::async_trait;
use ironclaw_turns::{LoopExit, LoopFailureKind};

use crate::state::{CheckpointKind, LoopExecutionState};

use super::{
    AgentLoopExecutorError, CancelCheck, CheckpointStage, ExecutorStage, FailedExitDetails,
    PendingInputAck, StageContext, attach_failure_explanation, failed_exit,
};

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct BudgetStage;

pub(super) struct BudgetInput {
    pub(super) state: LoopExecutionState,
    pub(super) pending_input_ack: PendingInputAck,
}

pub(super) enum BudgetStep {
    Continue {
        state: Box<LoopExecutionState>,
        pending_input_ack: PendingInputAck,
    },
    Exit(LoopExit),
}

#[async_trait]
impl ExecutorStage<BudgetInput> for BudgetStage {
    type Output = BudgetStep;

    async fn process(
        &self,
        ctx: StageContext<'_>,
        input: BudgetInput,
    ) -> Result<BudgetStep, AgentLoopExecutorError> {
        let mut pending_input_ack = input.pending_input_ack;
        let state = input.state;
        if state.iteration < ctx.planner.budget().iteration_limit(&state) {
            return Ok(BudgetStep::Continue {
                state: Box::new(state),
                pending_input_ack,
            });
        }

        let mut state = match CheckpointStage
            .cancel_if_requested_after_pending_input_ack(ctx, state, &mut pending_input_ack)
            .await?
        {
            CancelCheck::Continue(state) => *state,
            CancelCheck::Exit(exit) => return Ok(BudgetStep::Exit(exit)),
        };
        let explanation_message_ref =
            attach_failure_explanation(ctx, &mut state, LoopFailureKind::IterationLimit).await?;

        let checked = CheckpointStage
            .write(ctx, state, CheckpointKind::Final)
            .await?;
        pending_input_ack.ack(ctx.host).await?;
        Ok(BudgetStep::Exit(failed_exit(
            ctx.host,
            checked.state,
            LoopFailureKind::IterationLimit,
            Some(checked.checkpoint_id),
            FailedExitDetails {
                diagnostic_ref: None,
                safe_summary: None,
                explanation_message_ref,
            },
        )?))
    }
}
