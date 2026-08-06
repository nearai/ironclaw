use async_trait::async_trait;
use ironclaw_loop_contracts::{LoopExit, LoopFailureKind};

use crate::state::{CheckpointKind, LoopExecutionState, TerminalWarningObservation};

use super::{
    AgentLoopExecutorError, CancelCheck, CheckpointStage, ExecutorStage, FailedExitDetails,
    StageContext, attach_failure_explanation, failed_exit,
};

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct BudgetStage;

pub(super) struct BudgetInput {
    pub(super) state: LoopExecutionState,
}

pub(super) enum BudgetStep {
    Continue { state: Box<LoopExecutionState> },
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
        let mut state = input.state;
        let iteration_limit = ctx.planner.budget().iteration_limit(&state);
        if state.iteration < iteration_limit
            || state.terminal_warning_state.pending().is_some()
            || state.terminal_warning_state.active().is_some()
        {
            return Ok(BudgetStep::Continue {
                state: Box::new(state),
            });
        }

        if state
            .terminal_warning_state
            .schedule(TerminalWarningObservation::iteration_limit(iteration_limit))
        {
            return Ok(BudgetStep::Continue {
                state: Box::new(state),
            });
        }

        // The one model-visible final iteration was already consumed: preserve
        // the existing explained terminal failure.
        let mut state = match CheckpointStage.cancel_if_requested(ctx, state).await? {
            CancelCheck::Continue(state) => *state,
            CancelCheck::Exit(exit) => return Ok(BudgetStep::Exit(exit)),
        };
        let explanation_message_ref =
            attach_failure_explanation(ctx, &mut state, LoopFailureKind::IterationLimit).await?;

        let checked = CheckpointStage
            .write(ctx, state, CheckpointKind::Final)
            .await?;
        Ok(BudgetStep::Exit(failed_exit(
            ctx.host,
            checked.state,
            LoopFailureKind::IterationLimit,
            Some(checked.checkpoint_id),
            FailedExitDetails {
                safe_summary: None,
                explanation_message_ref,
            },
        )?))
    }
}
