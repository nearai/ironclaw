use super::input::{UserFacingInputDrainMode, consume_drainable_inputs};
use super::*;

impl CanonicalAgentLoopExecutor {
    pub(super) async fn checkpoint(
        &self,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
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
        let state_ref = host
            .stage_checkpoint_payload(StageCheckpointPayloadRequest {
                kind: host_kind,
                schema_id: crate::state::CHECKPOINT_SCHEMA_ID.to_string(),
                payload,
            })
            .await
            .map_err(|_| AgentLoopExecutorError::CheckpointFailed { stage: kind })?;
        let checkpoint_id = host
            .checkpoint(LoopCheckpointRequest {
                kind: host_kind,
                state_ref: state_ref.clone(),
            })
            .await
            .map_err(|_| AgentLoopExecutorError::CheckpointFailed { stage: kind })?;
        self.emit_progress(
            host,
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

    pub(super) async fn emit_progress(
        &self,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        event: LoopProgressEvent,
    ) {
        let _ = host.emit_loop_progress(event).await;
    }

    // Cancellation is checked cooperatively at N boundary points between external calls.
    // A macro refactor was considered but deferred; the explicit sites are self-documenting.
    pub(super) async fn checkpoint_and_exit_if_cancelled(
        &self,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        state: LoopExecutionState,
    ) -> Result<CancelCheck, AgentLoopExecutorError> {
        let Some(signal) = host.observe_cancellation() else {
            return Ok(CancelCheck::Continue(Box::new(state)));
        };

        let fallback_state = state.clone();
        match self.checkpoint(host, state, CheckpointKind::Final).await {
            Ok(checked) => Ok(CancelCheck::Exit(cancelled_exit_with_reason(
                host,
                checked.state,
                cancelled_reason_from_signal(&signal),
                Some(checked.checkpoint_id),
            )?)),
            // Permissive profile: only checkpoint-write failures are absorbed
            // into a checkpoint-free `Cancelled` exit. Other variants (e.g.
            // `HostUnavailable`) must propagate so the runner can apply its
            // recovery policy.
            Err(AgentLoopExecutorError::CheckpointFailed { .. })
                if !host
                    .run_context()
                    .resolved_run_profile
                    .checkpoint_policy
                    .require_final_checkpoint =>
            {
                Ok(CancelCheck::Exit(cancelled_exit_with_reason(
                    host,
                    fallback_state,
                    cancelled_reason_from_signal(&signal),
                    None,
                )?))
            }
            Err(error) => Err(error),
        }
    }

    pub(super) async fn checkpoint_and_exit_if_cancelled_after_pending_input_ack(
        &self,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        state: LoopExecutionState,
        pending_input_ack: &mut PendingInputAck,
    ) -> Result<CancelCheck, AgentLoopExecutorError> {
        let Some(signal) = host.observe_cancellation() else {
            return Ok(CancelCheck::Continue(Box::new(state)));
        };

        let fallback_state = state.clone();
        match self.checkpoint(host, state, CheckpointKind::Final).await {
            Ok(checked) => {
                pending_input_ack.ack(host).await?;
                Ok(CancelCheck::Exit(cancelled_exit_with_reason(
                    host,
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
                if !host
                    .run_context()
                    .resolved_run_profile
                    .checkpoint_policy
                    .require_final_checkpoint =>
            {
                Ok(CancelCheck::Exit(cancelled_exit_with_reason(
                    host,
                    fallback_state,
                    cancelled_reason_from_signal(&signal),
                    None,
                )?))
            }
            // Strict profile (or non-checkpoint error variant): propagate the
            // error so the runner sees the same failure mode as
            // `checkpoint_and_exit_if_cancelled`. Returning `Ok(LoopExit::failed)`
            // would silently mask `HostUnavailable` and break the strict
            // require-final-checkpoint contract.
            Err(error) => Err(error),
        }
    }

    pub(super) async fn drain_user_inputs(
        &self,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        mut state: LoopExecutionState,
    ) -> Result<DrainedInputs, AgentLoopExecutorError> {
        let batch = host
            .poll_inputs(state.input_cursor.clone(), MAX_INPUT_DRAIN)
            .await
            .map_err(|_| AgentLoopExecutorError::HostUnavailable {
                stage: HostStage::Input,
            })?;
        let (drained, ack_tokens, cancelled_reason_kind) =
            consume_drainable_inputs(&batch, UserFacingInputDrainMode::Steering, &mut state)?;
        Ok(DrainedInputs {
            state,
            drained,
            ack_tokens,
            cancelled_reason_kind,
        })
    }

    pub(super) async fn drain_followup(
        &self,
        host: &(dyn AgentLoopDriverHost + Send + Sync),
        mut state: LoopExecutionState,
    ) -> Result<DrainedInputs, AgentLoopExecutorError> {
        let batch = host
            .poll_inputs(state.input_cursor.clone(), MAX_INPUT_DRAIN)
            .await
            .map_err(|_| AgentLoopExecutorError::HostUnavailable {
                stage: HostStage::Input,
            })?;
        let (drained, ack_tokens, cancelled_reason_kind) =
            consume_drainable_inputs(&batch, UserFacingInputDrainMode::FollowUp, &mut state)?;
        Ok(DrainedInputs {
            state,
            drained,
            ack_tokens,
            cancelled_reason_kind,
        })
    }
}
