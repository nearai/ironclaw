use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use ironclaw_turns::{
    CheckpointStateStore, GetCheckpointStateRequest, GetLoopCheckpointRequest,
    LoopCheckpointStateRef, LoopCheckpointStore, PutCheckpointStateRequest,
    PutLoopCheckpointRequest, TurnCheckpointId, TurnRunId,
    run_profile::{
        AgentLoopHostError, AgentLoopHostErrorKind, LoadCheckpointPayloadRequest,
        LoadedCheckpointPayload, LoopCheckpointKind, LoopCheckpointPort, LoopCheckpointRequest,
        LoopHostMilestoneEmitter, LoopHostMilestoneSink, LoopInputAckToken, LoopInputBatch,
        LoopInputCursor, LoopInputPort, LoopProgressEvent, LoopProgressPort, LoopRunContext,
        LoopRunInfoPort, StageCheckpointPayloadRequest,
    },
};

use super::turn_error_to_host_error;

#[derive(Clone)]
pub(super) struct NoExtraLoopInputPort {
    run_context: LoopRunContext,
}

impl NoExtraLoopInputPort {
    pub(super) fn new(run_context: LoopRunContext) -> Self {
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
pub(super) struct HostManagedLoopCheckpointPort {
    run_context: LoopRunContext,
    checkpoint_state_store: Arc<dyn CheckpointStateStore>,
    loop_checkpoint_store: Arc<dyn LoopCheckpointStore>,
    milestone_sink: Arc<dyn LoopHostMilestoneSink>,
    staged_checkpoint_refs: Arc<Mutex<HashMap<LoopCheckpointStateRef, LoopCheckpointKind>>>,
}

impl HostManagedLoopCheckpointPort {
    pub(super) fn new(
        run_context: LoopRunContext,
        checkpoint_state_store: Arc<dyn CheckpointStateStore>,
        loop_checkpoint_store: Arc<dyn LoopCheckpointStore>,
        milestone_sink: Arc<dyn LoopHostMilestoneSink>,
    ) -> Self {
        Self {
            run_context,
            checkpoint_state_store,
            loop_checkpoint_store,
            milestone_sink,
            staged_checkpoint_refs: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn staged_checkpoint_kind(
        &self,
        state_ref: &LoopCheckpointStateRef,
    ) -> Result<Option<LoopCheckpointKind>, AgentLoopHostError> {
        self.staged_checkpoint_refs
            .lock()
            .map_err(|_| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Internal,
                    "checkpoint staged-ref cache lock was poisoned",
                )
            })
            .map(|staged| staged.get(state_ref).copied())
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
        // `stage_checkpoint_payload` returns a run-scoped ref of the form
        // `checkpoint:{run_id}:{token}`. The underlying store indexed the payload
        // under the original `checkpoint:{token}` key (which `new_state_ref()`
        // generated). Unwrap to the store key so the look-up succeeds, then pass
        // the caller-supplied (run-scoped) ref through to the loop-checkpoint
        // record so `is_for_run` validators see the correct form.
        let store_ref = checkpoint_state_store_ref(&self.run_context, &request.state_ref)?;

        match self.staged_checkpoint_kind(&request.state_ref)? {
            Some(kind) if kind == request.kind => {}
            Some(_) => {
                return Err(AgentLoopHostError::new(
                    AgentLoopHostErrorKind::CheckpointRejected,
                    "checkpoint state ref kind does not match the checkpoint request",
                ));
            }
            None => {
                let loaded = self
                    .checkpoint_state_store
                    .get_checkpoint_state(GetCheckpointStateRequest {
                        scope: self.run_context.scope.clone(),
                        turn_id: self.run_context.turn_id,
                        run_id: self.run_context.run_id,
                        state_ref: store_ref,
                        schema_id: self.run_context.checkpoint_schema_id.clone(),
                        schema_version: self.run_context.checkpoint_schema_version,
                        kind: request.kind,
                    })
                    .await
                    .map_err(turn_error_to_host_error)?;
                if loaded.is_none() {
                    return Err(AgentLoopHostError::new(
                        AgentLoopHostErrorKind::CheckpointRejected,
                        "checkpoint state ref is unavailable for this loop run",
                    ));
                }
            }
        }

        let checkpoint = self
            .loop_checkpoint_store
            .put_loop_checkpoint(PutLoopCheckpointRequest {
                scope: self.run_context.scope.clone(),
                turn_id: self.run_context.turn_id,
                run_id: self.run_context.run_id,
                state_ref: request.state_ref,
                schema_id: self.run_context.checkpoint_schema_id.clone(),
                schema_version: self.run_context.checkpoint_schema_version,
                kind: request.kind,
                gate_ref: request.gate_ref,
            })
            .await
            .map_err(turn_error_to_host_error)?;
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
        // profile's resolved checkpoint schema — the read-side
        // `get_checkpoint_state` checks `(state_ref, schema_id, kind)` as a
        // unit, so mismatches here would lead to phantom resume rejections.
        if request.schema_id != self.run_context.checkpoint_schema_id.as_str() {
            return Err(AgentLoopHostError::new(
                AgentLoopHostErrorKind::CheckpointRejected,
                "staged checkpoint payload schema_id does not match the run profile's checkpoint schema",
            ));
        }

        let record = self
            .checkpoint_state_store
            .put_checkpoint_state(PutCheckpointStateRequest::new(
                self.run_context.scope.clone(),
                self.run_context.turn_id,
                self.run_context.run_id,
                self.run_context.checkpoint_schema_id.clone(),
                self.run_context.checkpoint_schema_version,
                request.kind,
                request.payload,
            ))
            .await
            .map_err(turn_error_to_host_error)?;

        // The store produces `checkpoint:{uuid}` refs. Wrap into the run-scoped
        // form `checkpoint:{run_id}:{token}` so that `LoopCheckpointStateRef::
        // is_for_run` validators accept the returned ref without treating it as
        // a cross-run ref. The token is the opaque UUID the store already minted.
        let raw = record.state_ref.as_str();
        let token = raw.strip_prefix("checkpoint:").ok_or_else(|| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::Internal,
                "checkpoint state store returned ref without expected `checkpoint:` prefix",
            )
        })?;
        let run_scoped_ref =
            LoopCheckpointStateRef::for_run(&self.run_context, token).map_err(|reason| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Internal,
                    format!("could not build run-scoped checkpoint state ref: {reason}"),
                )
            })?;
        self.staged_checkpoint_refs
            .lock()
            .map_err(|_| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Internal,
                    "checkpoint staged-ref cache lock was poisoned",
                )
            })?
            .insert(run_scoped_ref.clone(), request.kind);
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

        let (state_ref, state_run_id) =
            checkpoint_state_store_ref_and_run_id(&self.run_context, &metadata.state_ref)?;
        let state_record = self
            .checkpoint_state_store
            .get_checkpoint_state(GetCheckpointStateRequest {
                scope: self.run_context.scope.clone(),
                turn_id: self.run_context.turn_id,
                run_id: state_run_id,
                state_ref,
                schema_id: metadata.schema_id.clone(),
                schema_version: metadata.schema_version,
                kind: metadata.kind,
            })
            .await
            .map_err(turn_error_to_host_error)?
            .ok_or_else(|| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Unavailable,
                    "checkpoint payload was not found for this loop run",
                )
            })?;

        Ok(LoadedCheckpointPayload {
            kind: state_record.kind,
            schema_id: state_record.schema_id,
            schema_version: state_record.schema_version,
            payload: state_record.payload,
        })
    }
}

fn checkpoint_state_store_ref(
    run_context: &LoopRunContext,
    state_ref: &LoopCheckpointStateRef,
) -> Result<LoopCheckpointStateRef, AgentLoopHostError> {
    // Write path: a checkpoint record may only be staged under the current
    // run's scope. Cross-run links (`checkpoint:{other_run}:{token}`) are a
    // read-only retry-resume affordance resolved in `load_checkpoint_payload`;
    // accepting one here would index the write against a foreign run's payload
    // and later fail to load. Reject anything not scoped to this run.
    let (store_ref, source_run_id) = checkpoint_state_store_ref_and_run_id(run_context, state_ref)?;
    if source_run_id != run_context.run_id {
        return Err(AgentLoopHostError::new(
            AgentLoopHostErrorKind::CheckpointRejected,
            "checkpoint state ref is not scoped to this loop run",
        ));
    }
    Ok(store_ref)
}

fn checkpoint_state_store_ref_and_run_id(
    run_context: &LoopRunContext,
    state_ref: &LoopCheckpointStateRef,
) -> Result<(LoopCheckpointStateRef, TurnRunId), AgentLoopHostError> {
    let run_scoped_prefix = format!("checkpoint:{}:", run_context.run_id);
    if let Some(token) = state_ref.as_str().strip_prefix(&run_scoped_prefix) {
        return Ok((store_checkpoint_state_ref(token)?, run_context.run_id));
    }
    let Some(rest) = state_ref.as_str().strip_prefix("checkpoint:") else {
        return Ok((state_ref.clone(), run_context.run_id));
    };
    let Some((run_id, token)) = rest.split_once(':') else {
        return Ok((state_ref.clone(), run_context.run_id));
    };
    let source_run_id = TurnRunId::parse(run_id).map_err(|error| {
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::Invalid,
            format!("checkpoint state ref contains invalid source run id: {error}"),
        )
    })?;
    Ok((store_checkpoint_state_ref(token)?, source_run_id))
}

fn store_checkpoint_state_ref(token: &str) -> Result<LoopCheckpointStateRef, AgentLoopHostError> {
    LoopCheckpointStateRef::new(format!("checkpoint:{token}")).map_err(|reason| {
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::Internal,
            format!("could not rebuild store key from run-scoped checkpoint ref: {reason}"),
        )
    })
}

#[derive(Clone)]
pub(super) struct HostManagedLoopProgressPort {
    run_context: LoopRunContext,
    milestone_sink: Arc<dyn LoopHostMilestoneSink>,
}

impl HostManagedLoopProgressPort {
    pub(super) fn new(
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
            } => emitter.compaction_leak_detected(task_id, reason_kind).await,
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
