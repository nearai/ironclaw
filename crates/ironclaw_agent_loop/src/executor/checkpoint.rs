use async_trait::async_trait;
use ironclaw_turns::run_profile::{
    LoopCheckpointRequest, LoopProgressEvent, StageCheckpointPayloadRequest,
};

use crate::state::{CheckpointKind, LoopExecutionState};

#[cfg(test)]
use crate::executor::CanonicalAgentLoopExecutor;

#[cfg(test)]
use ironclaw_turns::run_profile::AgentLoopDriverHost;

use super::{
    AgentLoopExecutorError, CancelCheck, CheckpointWrite, ExecutorStage, PendingInputAck,
    StageContext, cancelled_exit_with_reason, cancelled_reason_from_signal,
    checkpoint_kind_to_host,
};

#[cfg(test)]
use super::{DrainedInputs, InputStage};

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct CheckpointStage;

impl CheckpointStage {
    pub(super) async fn write(
        &self,
        ctx: StageContext<'_>,
        mut state: LoopExecutionState,
        kind: CheckpointKind,
    ) -> Result<CheckpointWrite, AgentLoopExecutorError> {
        state.last_checkpoint = Some(crate::state::CheckpointMarker {
            kind,
            iteration_at_checkpoint: state.iteration,
        });
        let payload = serde_json::to_vec(&state)
            .map_err(|_| AgentLoopExecutorError::CheckpointFailed { stage: kind })?;
        let host_kind = checkpoint_kind_to_host(kind);
        let state_ref = ctx
            .host
            .stage_checkpoint_payload(StageCheckpointPayloadRequest {
                kind: host_kind,
                schema_id: crate::state::CHECKPOINT_SCHEMA_ID.to_string(),
                payload,
            })
            .await
            .map_err(|_| AgentLoopExecutorError::CheckpointFailed { stage: kind })?;
        let checkpoint_id = ctx
            .host
            .checkpoint(LoopCheckpointRequest {
                kind: host_kind,
                state_ref: state_ref.clone(),
            })
            .await
            .map_err(|_| AgentLoopExecutorError::CheckpointFailed { stage: kind })?;
        self.emit_progress(
            ctx,
            LoopProgressEvent::CheckpointWritten {
                iteration: state.iteration,
                kind: host_kind,
            },
        )
        .await;
        Ok(CheckpointWrite {
            state,
            checkpoint_id,
            state_ref,
        })
    }

    pub(super) async fn emit_progress(&self, ctx: StageContext<'_>, event: LoopProgressEvent) {
        let _ = ctx.host.emit_loop_progress(event).await;
    }

    // Cancellation is checked cooperatively at N boundary points between external calls.
    // A macro refactor was considered but deferred; the explicit sites are self-documenting.
    pub(super) async fn cancel_if_requested(
        &self,
        ctx: StageContext<'_>,
        state: LoopExecutionState,
    ) -> Result<CancelCheck, AgentLoopExecutorError> {
        let Some(signal) = ctx.host.observe_cancellation() else {
            return Ok(CancelCheck::Continue(Box::new(state)));
        };

        let fallback_state = state.clone();
        match self.write(ctx, state, CheckpointKind::Final).await {
            Ok(checked) => Ok(CancelCheck::Exit(cancelled_exit_with_reason(
                ctx.host,
                checked.state,
                cancelled_reason_from_signal(&signal),
                Some(checked.checkpoint_id),
            )?)),
            // Permissive profile: only checkpoint-write failures are absorbed
            // into a checkpoint-free `Cancelled` exit. Other variants (e.g.
            // `HostUnavailable`) must propagate so the runner can apply its
            // recovery policy.
            Err(AgentLoopExecutorError::CheckpointFailed { .. })
                if !ctx
                    .host
                    .run_context()
                    .resolved_run_profile
                    .checkpoint_policy
                    .require_final_checkpoint =>
            {
                Ok(CancelCheck::Exit(cancelled_exit_with_reason(
                    ctx.host,
                    fallback_state,
                    cancelled_reason_from_signal(&signal),
                    None,
                )?))
            }
            Err(error) => Err(error),
        }
    }

    pub(super) async fn cancel_if_requested_after_pending_input_ack(
        &self,
        ctx: StageContext<'_>,
        state: LoopExecutionState,
        pending_input_ack: &mut PendingInputAck,
    ) -> Result<CancelCheck, AgentLoopExecutorError> {
        let Some(signal) = ctx.host.observe_cancellation() else {
            return Ok(CancelCheck::Continue(Box::new(state)));
        };

        let fallback_state = state.clone();
        match self.write(ctx, state, CheckpointKind::Final).await {
            Ok(checked) => {
                pending_input_ack.ack(ctx.host).await?;
                Ok(CancelCheck::Exit(cancelled_exit_with_reason(
                    ctx.host,
                    checked.state,
                    cancelled_reason_from_signal(&signal),
                    Some(checked.checkpoint_id),
                )?))
            }
            // Permissive profile: absorb only checkpoint-write failures. The
            // pending ack is intentionally NOT flushed here — no durable
            // checkpoint was written, so advancing the input cursor would
            // commit progress that the runner has no record of.
            Err(AgentLoopExecutorError::CheckpointFailed { .. })
                if !ctx
                    .host
                    .run_context()
                    .resolved_run_profile
                    .checkpoint_policy
                    .require_final_checkpoint =>
            {
                Ok(CancelCheck::Exit(cancelled_exit_with_reason(
                    ctx.host,
                    fallback_state,
                    cancelled_reason_from_signal(&signal),
                    None,
                )?))
            }
            // Strict profile (or non-checkpoint error variant): propagate the
            // error so the runner sees the same failure mode as
            // `cancel_if_requested`. Returning `Ok(LoopExit::failed)`
            // would silently mask `HostUnavailable` and break the strict
            // require-final-checkpoint contract.
            Err(error) => Err(error),
        }
    }
}

pub(super) struct CheckpointInput {
    pub(super) state: LoopExecutionState,
    pub(super) kind: CheckpointKind,
}

#[async_trait]
impl ExecutorStage<CheckpointInput> for CheckpointStage {
    type Output = CheckpointWrite;

    async fn process(
        &self,
        ctx: StageContext<'_>,
        input: CheckpointInput,
    ) -> Result<CheckpointWrite, AgentLoopExecutorError> {
        self.write(ctx, input.state, input.kind).await
    }
}

#[cfg(test)]
impl CanonicalAgentLoopExecutor {
    pub(super) async fn drain_user_inputs(
        &self,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        state: LoopExecutionState,
    ) -> Result<DrainedInputs, AgentLoopExecutorError> {
        let family = crate::families::default();
        let ctx = StageContext {
            planner: family.planner(),
            host,
        };
        InputStage.drain_user_inputs(ctx, state).await
    }

    pub(super) async fn drain_followup(
        &self,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        state: LoopExecutionState,
    ) -> Result<DrainedInputs, AgentLoopExecutorError> {
        let family = crate::families::default();
        let ctx = StageContext {
            planner: family.planner(),
            host,
        };
        InputStage.drain_followup(ctx, state).await
    }
}
