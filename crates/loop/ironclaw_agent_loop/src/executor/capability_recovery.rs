use ironclaw_loop_contracts::{CapabilityCallCandidate, LoopFailureKind};

use super::capabilities::{CapabilityStage, OutcomeStep};
use super::{
    AgentLoopExecutorError, FailedExitDetails, StageContext, append_capability_safe_summary_ref,
    attach_failure_explanation, cancelled_exit_with_reason, failed_exit,
};
use crate::state::LoopExecutionState;

impl CapabilityStage {
    pub(super) async fn fail_unsupported_process_wait(
        &self,
        ctx: StageContext<'_>,
        mut state: LoopExecutionState,
        call: &CapabilityCallCandidate,
        _process_ref: &ironclaw_loop_contracts::LoopProcessRef,
    ) -> Result<OutcomeStep, AgentLoopExecutorError> {
        append_capability_safe_summary_ref(
            ctx.host,
            &mut state,
            call,
            "capability process wait is not supported".to_string(),
        )
        .await?;
        let explanation_message_ref =
            attach_failure_explanation(ctx, &mut state, LoopFailureKind::CapabilityProtocolError)
                .await?;
        let exit = failed_exit(
            ctx.host,
            state.clone(),
            LoopFailureKind::CapabilityProtocolError,
            None,
            FailedExitDetails {
                safe_summary: None,
                explanation_message_ref,
            },
        )?;
        Ok(OutcomeStep::Exit {
            exit,
            state: Some(Box::new(state)),
        })
    }

    pub(super) fn cancelled_for_batch_drain(
        &self,
        ctx: StageContext<'_>,
        state: LoopExecutionState,
    ) -> Result<OutcomeStep, AgentLoopExecutorError> {
        self.cancelled_for_batch_drain_with_reason(
            ctx,
            state,
            ironclaw_loop_contracts::LoopCancelledReasonKind::HostCancellation,
        )
    }

    pub(super) fn cancelled_for_batch_drain_with_reason(
        &self,
        ctx: StageContext<'_>,
        state: LoopExecutionState,
        reason_kind: ironclaw_loop_contracts::LoopCancelledReasonKind,
    ) -> Result<OutcomeStep, AgentLoopExecutorError> {
        // The unified batch drain merges every launched sibling before writing
        // the one authoritative Final checkpoint.
        let exit = cancelled_exit_with_reason(ctx.host, state.clone(), reason_kind, None)?;
        Ok(OutcomeStep::Exit {
            exit,
            state: Some(Box::new(state)),
        })
    }
}
