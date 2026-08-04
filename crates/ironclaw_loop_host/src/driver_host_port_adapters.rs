use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use ironclaw_loop_contracts::{
    AgentLoopHostError, AgentLoopHostErrorKind, LoadCheckpointPayloadRequest,
    LoadedCheckpointPayload, LoopCheckpointPort, LoopCheckpointRequest, LoopCheckpointStateRef,
    LoopHostMilestoneEmitter, LoopHostMilestoneSink, LoopInputAckToken, LoopInputBatch,
    LoopInputCursor, LoopInputPort, LoopProgressEvent, LoopProgressPort, LoopRunContext,
    LoopRunInfoPort, RedactedCheckpointPayload, StageCheckpointPayloadRequest,
};
use ironclaw_turns::{
    GetLoopCheckpointRequest, LoopCheckpointStore, PutLoopCheckpointRequest, TurnCheckpointId,
    TurnError,
};

#[derive(Clone)]
pub struct NoExtraLoopInputPort {
    run_context: LoopRunContext,
}

impl NoExtraLoopInputPort {
    pub fn new(run_context: LoopRunContext) -> Self {
        Self { run_context }
    }

    fn validate_cursor(&self, cursor: &LoopInputCursor) -> Result<(), AgentLoopHostError> {
        if cursor.is_for_run(&self.run_context) {
            Ok(())
        } else {
            Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::ScopeMismatch,
                "input cursor is not scoped to this loop run",
            ))
        }
    }
}

impl LoopRunInfoPort for NoExtraLoopInputPort {
    fn run_context(&self) -> &LoopRunContext {
        &self.run_context
    }
}

#[async_trait]
impl LoopInputPort for NoExtraLoopInputPort {
    async fn poll_inputs(
        &self,
        after: LoopInputCursor,
        _limit: usize,
    ) -> Result<LoopInputBatch, AgentLoopHostError> {
        self.validate_cursor(&after)?;
        Ok(LoopInputBatch {
            inputs: Vec::new(),
            input_acks: Vec::new(),
            next_cursor: after,
        })
    }

    async fn ack_inputs(&self, tokens: Vec<LoopInputAckToken>) -> Result<(), AgentLoopHostError> {
        if tokens.is_empty() {
            Ok(())
        } else {
            Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "input ack token was not issued by this host",
            ))
        }
    }
}

#[derive(Clone)]
pub struct HostManagedLoopCheckpointPort {
    run_context: LoopRunContext,
    loop_checkpoint_store: Arc<dyn LoopCheckpointStore>,
    milestone_sink: Arc<dyn LoopHostMilestoneSink>,
    staged_checkpoints: Arc<Mutex<HashMap<LoopCheckpointStateRef, StageCheckpointPayloadRequest>>>,
}

impl HostManagedLoopCheckpointPort {
    pub fn new(
        run_context: LoopRunContext,
        loop_checkpoint_store: Arc<dyn LoopCheckpointStore>,
        milestone_sink: Arc<dyn LoopHostMilestoneSink>,
    ) -> Self {
        Self {
            run_context,
            loop_checkpoint_store,
            milestone_sink,
            staged_checkpoints: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn staged_checkpoint(
        &self,
        state_ref: &LoopCheckpointStateRef,
    ) -> Result<Option<StageCheckpointPayloadRequest>, AgentLoopHostError> {
        self.staged_checkpoints
            .lock()
            .map_err(|_| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Internal,
                    "checkpoint staged-ref cache lock was poisoned",
                )
            })
            .map(|staged| staged.get(state_ref).cloned())
    }
}

impl LoopRunInfoPort for HostManagedLoopCheckpointPort {
    fn run_context(&self) -> &LoopRunContext {
        &self.run_context
    }
}

#[async_trait]
impl LoopCheckpointPort for HostManagedLoopCheckpointPort {
    async fn checkpoint(
        &self,
        request: LoopCheckpointRequest,
    ) -> Result<TurnCheckpointId, AgentLoopHostError> {
        let staged = self.staged_checkpoint(&request.state_ref)?.ok_or_else(|| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::CheckpointRejected,
                "checkpoint state ref is unavailable for this loop run",
            )
        })?;
        if staged.kind != request.kind {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::CheckpointRejected,
                "checkpoint state ref kind does not match the checkpoint request",
            ));
        }

        let checkpoint = self
            .loop_checkpoint_store
            .put_loop_checkpoint(PutLoopCheckpointRequest {
                scope: self.run_context.scope.clone(),
                turn_id: self.run_context.turn_id,
                run_id: self.run_context.run_id,
                state_ref: request.state_ref,
                payload: RedactedCheckpointPayload::new(staged.payload).map_err(|reason| {
                    AgentLoopHostError::new(AgentLoopHostErrorKind::CheckpointRejected, reason)
                })?,
                schema_id: self.run_context.checkpoint_schema_id.clone(),
                schema_version: self.run_context.checkpoint_schema_version,
                kind: request.kind,
                gate_ref: request.gate_ref,
            })
            .await
            .map_err(turn_error_to_host_error)?;
        self.staged_checkpoints
            .lock()
            .map_err(|_| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Internal,
                    "checkpoint staged-ref cache lock was poisoned",
                )
            })?
            .remove(&checkpoint.state_ref);
        LoopHostMilestoneEmitter::new(self.run_context.clone(), Arc::clone(&self.milestone_sink))
            .checkpoint_created(checkpoint.checkpoint_id, request.kind)
            .await?;
        Ok(checkpoint.checkpoint_id)
    }

    async fn stage_checkpoint_payload(
        &self,
        request: StageCheckpointPayloadRequest,
    ) -> Result<LoopCheckpointStateRef, AgentLoopHostError> {
        // Reject staged payloads whose schema_id disagrees with the run
        // profile's resolved checkpoint schema. Rejecting mismatches before
        // staging keeps the eventual process checkpoint command self-consistent.
        if request.schema_id != self.run_context.checkpoint_schema_id {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::CheckpointRejected,
                "staged checkpoint payload schema_id does not match the run profile's checkpoint schema",
            ));
        }

        RedactedCheckpointPayload::new(request.payload.clone()).map_err(|reason| {
            AgentLoopHostError::new(AgentLoopHostErrorKind::CheckpointRejected, reason)
        })?;
        let run_scoped_ref = LoopCheckpointStateRef::for_run(
            &self.run_context,
            TurnCheckpointId::new().as_uuid().simple().to_string(),
        )
        .map_err(|reason| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::Internal,
                format!("could not build run-scoped checkpoint state ref: {reason}"),
            )
        })?;
        self.staged_checkpoints
            .lock()
            .map_err(|_| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Internal,
                    "checkpoint staged-ref cache lock was poisoned",
                )
            })?
            .insert(run_scoped_ref.clone(), request);
        Ok(run_scoped_ref)
    }

    async fn load_checkpoint_payload(
        &self,
        request: LoadCheckpointPayloadRequest,
    ) -> Result<LoadedCheckpointPayload, AgentLoopHostError> {
        let metadata = self
            .loop_checkpoint_store
            .get_loop_checkpoint(GetLoopCheckpointRequest {
                scope: self.run_context.scope.clone(),
                turn_id: self.run_context.turn_id,
                run_id: self.run_context.run_id,
                checkpoint_id: request.checkpoint_id,
            })
            .await
            .map_err(turn_error_to_host_error)?
            .ok_or_else(|| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Unavailable,
                    "checkpoint metadata was not found for this loop run",
                )
            })?;

        if metadata.schema_id != request.expected_schema_id
            || metadata.schema_version != request.expected_schema_version
        {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::Invalid,
                "checkpoint schema id/version does not match the resume request",
            ));
        }

        let payload = metadata.payload.ok_or_else(|| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::Unavailable,
                "checkpoint payload was not found for this loop run",
            )
        })?;

        Ok(LoadedCheckpointPayload {
            kind: metadata.kind,
            schema_id: metadata.schema_id,
            schema_version: metadata.schema_version,
            payload,
        })
    }
}

#[derive(Clone)]
pub struct HostManagedLoopProgressPort {
    run_context: LoopRunContext,
    milestone_sink: Arc<dyn LoopHostMilestoneSink>,
}

impl HostManagedLoopProgressPort {
    pub fn new(
        run_context: LoopRunContext,
        milestone_sink: Arc<dyn LoopHostMilestoneSink>,
    ) -> Self {
        Self {
            run_context,
            milestone_sink,
        }
    }
}

impl LoopRunInfoPort for HostManagedLoopProgressPort {
    fn run_context(&self) -> &LoopRunContext {
        &self.run_context
    }
}

#[async_trait]
impl LoopProgressPort for HostManagedLoopProgressPort {
    async fn emit_loop_progress(&self, event: LoopProgressEvent) -> Result<(), AgentLoopHostError> {
        let emitter = LoopHostMilestoneEmitter::new(
            self.run_context.clone(),
            Arc::clone(&self.milestone_sink),
        );
        match event {
            LoopProgressEvent::DriverNote { kind, safe_summary } => {
                emitter.driver_note(kind, safe_summary).await
            }
            LoopProgressEvent::IterationStarted { iteration } => {
                emitter.iteration_started(iteration).await
            }
            // Prompt construction already emits the canonical
            // `PromptBundleBuilt` milestone from `HostManagedLoopPromptPort`,
            // including the bundle ref and redacted skill-context metadata.
            // Treat the executor progress echo as advisory to avoid duplicate
            // prompt milestones for the same bundle.
            LoopProgressEvent::PromptBundleBuilt { .. } => Ok(()),
            LoopProgressEvent::CapabilityBatchStarted {
                iteration,
                call_count,
                policy,
            } => {
                emitter
                    .capability_batch_started(iteration, call_count, policy)
                    .await
            }
            LoopProgressEvent::CapabilityBatchCompleted {
                iteration,
                result_count,
                denied_count,
                gated_count,
                failed_count,
            } => {
                emitter
                    .capability_batch_completed(
                        iteration,
                        result_count,
                        denied_count,
                        gated_count,
                        failed_count,
                    )
                    .await
            }
            LoopProgressEvent::CapabilityActivityFailed {
                activity_id,
                capability_id,
                reason_kind,
                safe_summary,
            } => {
                emitter
                    .capability_failed(
                        activity_id,
                        capability_id,
                        None,
                        None,
                        reason_kind,
                        safe_summary,
                    )
                    .await
            }
            LoopProgressEvent::FailureRecovered {
                sequence,
                stage,
                class,
                disposition,
            } => {
                emitter
                    .failure_recovered(sequence, stage, class, disposition)
                    .await
            }
            LoopProgressEvent::GateBlocked {
                iteration,
                gate_kind,
            } => emitter.gate_blocked(iteration, gate_kind).await,
            // `HostManagedLoopCheckpointPort::checkpoint` publishes the
            // canonical checkpoint milestone with the durable checkpoint id.
            // `CheckpointWritten` carries only the checkpoint kind/iteration,
            // so emitting it here would either duplicate or weaken that record.
            LoopProgressEvent::CheckpointWritten { .. } => Ok(()),
            LoopProgressEvent::CompactionStarted { task_id, initiator } => {
                emitter.compaction_started(task_id, initiator).await
            }
            LoopProgressEvent::CompactionCompleted {
                task_id,
                compression_ratio_ppm,
            } => {
                emitter
                    .compaction_completed(task_id, compression_ratio_ppm)
                    .await
            }
            LoopProgressEvent::CompactionFailed {
                task_id,
                reason_kind,
            } => emitter.compaction_failed(task_id, reason_kind).await,
            LoopProgressEvent::CompactionLeakDetected {
                task_id,
                reason_kind,
                redacted_leak_count,
            } => {
                emitter
                    .compaction_leak_detected(task_id, reason_kind, redacted_leak_count)
                    .await
            }
            // Goal refresh has event types reserved in the run-profile surface,
            // but no producer path in the current loop.
            LoopProgressEvent::GoalRefreshStarted { .. }
            | LoopProgressEvent::GoalRefreshCompleted { .. }
            | LoopProgressEvent::GoalRefreshFailed { .. }
            | LoopProgressEvent::GoalRefreshLeakDetected { .. } => Ok(()),
            _ => Ok(()),
        }
    }
}

/// `TurnError` -> `AgentLoopHostError` at the checkpoint-state seam.
///
/// Moved here with the checkpoint port that calls it (WS3 runner sheds). It is
/// `pub` because `ironclaw_runner`'s driver host maps the same errors on its
/// own checkpoint path; every arm is typed on `ironclaw_turns` and
/// `ironclaw_loop_contracts` vocabulary, with no runner-specific concept in it.
pub fn turn_error_to_host_error(error: TurnError) -> AgentLoopHostError {
    match &error {
        TurnError::Unauthorized => crate::raw_agent_loop_host_error(
            "checkpoint_state",
            "access",
            AgentLoopHostErrorKind::Unauthorized,
            "checkpoint state access was unauthorized",
            &error,
        ),
        TurnError::InvalidRequest { .. } => crate::raw_agent_loop_host_error(
            "checkpoint_state",
            "request",
            AgentLoopHostErrorKind::InvalidInvocation,
            "checkpoint state request is invalid",
            &error,
        ),
        TurnError::Unavailable { .. } => crate::raw_agent_loop_host_error(
            "checkpoint_state",
            "store",
            AgentLoopHostErrorKind::Unavailable,
            "checkpoint state store is unavailable",
            &error,
        ),
        TurnError::ScopeNotFound => crate::raw_agent_loop_host_error(
            "checkpoint_state",
            "scope_lookup",
            AgentLoopHostErrorKind::CheckpointRejected,
            "checkpoint state scope was not found for this loop run",
            &error,
        ),
        TurnError::Conflict { .. } | TurnError::RunNotRetryable { .. } => {
            crate::raw_agent_loop_host_error(
                "checkpoint_state",
                "write",
                AgentLoopHostErrorKind::CheckpointRejected,
                "checkpoint state write conflicted with current turn state",
                &error,
            )
        }
        TurnError::CapacityExceeded { .. } => crate::raw_agent_loop_host_error(
            "checkpoint_state",
            "write",
            AgentLoopHostErrorKind::Unavailable,
            "checkpoint state store capacity was exceeded",
            &error,
        ),
        TurnError::InvalidTransition { .. } => crate::raw_agent_loop_host_error(
            "checkpoint_state",
            "write",
            AgentLoopHostErrorKind::CheckpointRejected,
            "checkpoint state write was invalid for current turn state",
            &error,
        ),
        TurnError::LeaseMismatch => crate::raw_agent_loop_host_error(
            "checkpoint_state",
            "write",
            AgentLoopHostErrorKind::CheckpointRejected,
            "checkpoint state write lease no longer matches current run",
            &error,
        ),
        TurnError::ThreadBusy(_) | TurnError::AdmissionRejected(_) => {
            crate::raw_agent_loop_host_error(
                "checkpoint_state",
                "admission",
                AgentLoopHostErrorKind::Unavailable,
                "checkpoint state store returned unsupported turn admission status",
                &error,
            )
        }
        TurnError::InvalidRunOriginAdapter => crate::raw_agent_loop_host_error(
            "checkpoint_state",
            "request",
            AgentLoopHostErrorKind::InvalidInvocation,
            "checkpoint state request contains an invalid run origin adapter",
            &error,
        ),
    }
}

#[cfg(test)]
mod turn_error_to_host_error_tests {
    use super::*;
    use ironclaw_turns::{TurnCapacityResource, TurnError, TurnRunId};

    /// The security-relevant arm. It moved crates with the shed and became
    /// `pub`, so a future edit could reclassify an authorization failure as a
    /// generic one with nothing failing — raised in review on #7064.
    #[test]
    fn unauthorized_maps_to_unauthorized() {
        let error = turn_error_to_host_error(TurnError::Unauthorized);
        assert_eq!(error.kind, AgentLoopHostErrorKind::Unauthorized);
    }

    /// Both request-shaped arms, so neither can drift into a retryable or
    /// authorization kind unnoticed.
    #[test]
    fn request_shaped_errors_map_to_invalid_invocation() {
        for turn_error in [
            TurnError::InvalidRequest {
                reason: "bad checkpoint request".to_string(),
            },
            TurnError::InvalidRunOriginAdapter,
        ] {
            let error = turn_error_to_host_error(turn_error);
            assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
        }
    }

    #[test]
    fn capacity_exceeded_maps_to_unavailable() {
        let error = turn_error_to_host_error(TurnError::capacity_exceeded(
            TurnCapacityResource::SpawnTreeDescendants,
            3,
        ));
        assert_eq!(error.kind, AgentLoopHostErrorKind::Unavailable);
    }

    #[test]
    fn conflict_maps_to_checkpoint_rejected() {
        let error = turn_error_to_host_error(TurnError::Conflict {
            reason: "checkpoint conflict".to_string(),
        });
        assert_eq!(error.kind, AgentLoopHostErrorKind::CheckpointRejected);
    }

    #[test]
    fn run_not_retryable_maps_to_checkpoint_rejected() {
        let error = turn_error_to_host_error(TurnError::RunNotRetryable {
            run_id: TurnRunId::new(),
        });
        assert_eq!(error.kind, AgentLoopHostErrorKind::CheckpointRejected);
    }

    #[test]
    fn scope_not_found_maps_to_checkpoint_rejected() {
        let error = turn_error_to_host_error(TurnError::ScopeNotFound);
        assert_eq!(error.kind, AgentLoopHostErrorKind::CheckpointRejected);
    }

    #[test]
    fn invalid_transition_maps_to_checkpoint_rejected() {
        use ironclaw_turns::TurnStatus;
        let error = turn_error_to_host_error(TurnError::InvalidTransition {
            from: TurnStatus::Running,
            to: TurnStatus::Completed,
        });
        assert_eq!(error.kind, AgentLoopHostErrorKind::CheckpointRejected);
    }
}

#[cfg(test)]
mod tests;
