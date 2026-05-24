use async_trait::async_trait;
use ironclaw_turns::{LoopExit, LoopFailureKind};

use crate::state::{CheckpointKind, LoopExecutionState};

use super::{
    AgentLoopExecutorError, CheckpointStage, ExecutorStage, PendingInputAck, StageContext,
    failed_exit,
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

        let checked = CheckpointStage
            .write(ctx, state, CheckpointKind::Final)
            .await?;
        pending_input_ack.ack(ctx.host).await?;
        Ok(BudgetStep::Exit(failed_exit(
            ctx.host,
            checked.state,
            LoopFailureKind::IterationLimit,
            Some(checked.checkpoint_id),
        )?))
    }
}
