use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ironclaw_host_api::{TenantId, ThreadId};
use ironclaw_threads::{
    AppendToolResultReferenceRequest, InMemorySessionThreadService, SessionThreadService,
    ThreadMessageRecord, ThreadScope, ToolResultSafeSummary,
};
use ironclaw_turns::{
    AcceptedMessageRef, CancelRunRequest, CancelRunResponse, EventCursor, GateRef,
    GetLoopCheckpointRequest, GetRunStateRequest, LoopBlocked, LoopBlockedKind, LoopCheckpointKind,
    LoopCheckpointRecord, LoopCheckpointStateRef, LoopCheckpointStore, LoopCompleted,
    LoopCompletionKind, LoopExit, LoopExitId, LoopGateRef, LoopMessageRef, LoopResultRef,
    PutLoopCheckpointRequest, ReplyTargetBindingRef, ResumeTurnRequest, ResumeTurnResponse,
    RunProfileVersion, SanitizedFailure, SourceBindingRef, SubmitTurnRequest, SubmitTurnResponse,
    TurnCheckpointId, TurnError, TurnId, TurnLeaseToken, TurnRunId, TurnRunState, TurnRunnerId,
    TurnScope, TurnStateStore, TurnStatus,
    run_profile::{CheckpointSchemaId, LoopDriverId},
    runner::{
        ApplyValidatedLoopExitRequest, BlockRunRequest, CancelRunCompletionRequest,
        ClaimRunRequest, ClaimedTurnRun, CompleteRunRequest, FailRunRequest, HeartbeatRequest,
        RecordModelRouteSnapshotRequest, RecordRunnerFailureRequest, RecoverExpiredLeasesRequest,
        RecoverExpiredLeasesResponse, TurnRunTransitionPort,
    },
};

use crate::loop_exit_applier::{
    InMemoryLoopExitEvidencePort, LoopExitApplier, ThreadCheckpointLoopExitEvidencePort,
};

pub(super) fn text_checkpoint_evidence(
    loop_checkpoint_store: Arc<dyn LoopCheckpointStore>,
) -> ThreadCheckpointLoopExitEvidencePort<InMemorySessionThreadService> {
    ThreadCheckpointLoopExitEvidencePort::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(StaticTurnStateStore::new(claimed_run().state)),
        loop_checkpoint_store,
    )
}

pub(super) async fn append_tool_result_reference<S>(
    thread_service: &S,
    thread_scope: ThreadScope,
    thread_id: ThreadId,
    run_id: TurnRunId,
    result_ref: LoopResultRef,
) -> ThreadMessageRecord
where
    S: SessionThreadService + ?Sized,
{
    thread_service
        .append_tool_result_reference(AppendToolResultReferenceRequest {
            scope: thread_scope,
            thread_id,
            turn_run_id: run_id.to_string(),
            result_ref: result_ref.as_str().to_string(),
            safe_summary: ToolResultSafeSummary::new("tool completed").expect("safe summary"),
            provider_call: None,
        })
        .await
        .expect("tool result reference")
}

pub(super) struct StaticTurnStateStore {
    state: TurnRunState,
}

impl StaticTurnStateStore {
    pub(super) fn new(state: TurnRunState) -> Self {
        Self { state }
    }
}

#[async_trait]
impl TurnStateStore for StaticTurnStateStore {
    async fn submit_turn(
        &self,
        _request: SubmitTurnRequest,
        _admission_policy: &dyn ironclaw_turns::TurnAdmissionPolicy,
        _run_profile_resolver: &dyn ironclaw_turns::RunProfileResolver,
    ) -> Result<SubmitTurnResponse, TurnError> {
        panic!("submit_turn should not be called by evidence tests")
    }

    async fn resume_turn(
        &self,
        _request: ResumeTurnRequest,
    ) -> Result<ResumeTurnResponse, TurnError> {
        panic!("resume_turn should not be called by evidence tests")
    }

    async fn request_cancel(
        &self,
        _request: CancelRunRequest,
    ) -> Result<CancelRunResponse, TurnError> {
        panic!("request_cancel should not be called by evidence tests")
    }

    async fn get_run_state(&self, request: GetRunStateRequest) -> Result<TurnRunState, TurnError> {
        assert_eq!(request.scope, self.state.scope);
        assert_eq!(request.run_id, self.state.run_id);
        Ok(self.state.clone())
    }
}

pub(super) struct PanicLoopCheckpointStore;

#[async_trait]
impl LoopCheckpointStore for PanicLoopCheckpointStore {
    async fn put_loop_checkpoint(
        &self,
        _request: PutLoopCheckpointRequest,
    ) -> Result<LoopCheckpointRecord, TurnError> {
        panic!("put_loop_checkpoint should not be called by evidence tests")
    }

    async fn get_loop_checkpoint(
        &self,
        _request: GetLoopCheckpointRequest,
    ) -> Result<Option<LoopCheckpointRecord>, TurnError> {
        panic!("get_loop_checkpoint should not be called by fail-closed evidence tests")
    }
}

pub(super) struct StaticLoopCheckpointStore {
    record: Option<LoopCheckpointRecord>,
}

impl StaticLoopCheckpointStore {
    pub(super) fn new(record: LoopCheckpointRecord) -> Self {
        Self {
            record: Some(record),
        }
    }
}

#[async_trait]
impl LoopCheckpointStore for StaticLoopCheckpointStore {
    async fn put_loop_checkpoint(
        &self,
        _request: PutLoopCheckpointRequest,
    ) -> Result<LoopCheckpointRecord, TurnError> {
        panic!("put_loop_checkpoint should not be called by evidence tests")
    }

    async fn get_loop_checkpoint(
        &self,
        request: GetLoopCheckpointRequest,
    ) -> Result<Option<LoopCheckpointRecord>, TurnError> {
        Ok(self.record.as_ref().and_then(|record| {
            if record.scope == request.scope
                && record.turn_id == request.turn_id
                && record.run_id == request.run_id
                && record.checkpoint_id == request.checkpoint_id
            {
                Some(record.clone())
            } else {
                None
            }
        }))
    }
}

pub(super) fn test_exit_id() -> LoopExitId {
    LoopExitId::new("exit:test").expect("valid")
}

pub(super) fn completed_exit(
    reply_message_refs: Vec<LoopMessageRef>,
    final_checkpoint_id: Option<TurnCheckpointId>,
) -> LoopExit {
    LoopExit::Completed(LoopCompleted {
        completion_kind: LoopCompletionKind::FinalReply,
        reply_message_refs,
        result_refs: vec![],
        final_checkpoint_id,
        usage_summary_ref: None,
        exit_id: test_exit_id(),
    })
}

pub(super) fn blocked_exit(kind: LoopBlockedKind) -> LoopExit {
    blocked_exit_with_checkpoint(
        kind,
        TurnCheckpointId::new(),
        LoopCheckpointStateRef::new("checkpoint:blocked-state").expect("valid"),
    )
}

pub(super) fn blocked_exit_with_checkpoint(
    kind: LoopBlockedKind,
    checkpoint_id: TurnCheckpointId,
    state_ref: LoopCheckpointStateRef,
) -> LoopExit {
    LoopExit::Blocked(LoopBlocked {
        kind,
        gate_ref: LoopGateRef::new("gate:test").expect("valid"),
        checkpoint_id,
        state_ref,
        exit_id: test_exit_id(),
    })
}

pub(super) fn loop_checkpoint_record(
    claimed: &ClaimedTurnRun,
    checkpoint_id: TurnCheckpointId,
    state_ref: LoopCheckpointStateRef,
    kind: LoopCheckpointKind,
) -> LoopCheckpointRecord {
    loop_checkpoint_record_with_gate(claimed, checkpoint_id, state_ref, kind, None)
}

pub(super) fn loop_checkpoint_record_with_gate(
    claimed: &ClaimedTurnRun,
    checkpoint_id: TurnCheckpointId,
    state_ref: LoopCheckpointStateRef,
    kind: LoopCheckpointKind,
    gate_ref: Option<LoopGateRef>,
) -> LoopCheckpointRecord {
    LoopCheckpointRecord {
        checkpoint_id,
        scope: claimed.state.scope.clone(),
        turn_id: claimed.state.turn_id,
        run_id: claimed.state.run_id,
        state_ref,
        schema_id: claimed.resolved_run_profile.checkpoint_schema_id.clone(),
        schema_version: claimed.resolved_run_profile.checkpoint_schema_version,
        kind,
        gate_ref,
        created_at: chrono::Utc::now(),
    }
}

pub(super) struct Fixture {
    pub(super) claimed: ClaimedTurnRun,
    pub(super) transition: Arc<RecordingTransitionPort>,
    pub(super) applier: Arc<LoopExitApplier>,
}

impl Fixture {
    pub(super) fn new(evidence: InMemoryLoopExitEvidencePort) -> Self {
        let claimed = claimed_run();
        let transition = Arc::new(RecordingTransitionPort::new());
        let applier = Arc::new(LoopExitApplier::new(transition.clone(), Arc::new(evidence)));
        Self {
            claimed,
            transition,
            applier,
        }
    }
}

pub(super) fn claimed_run() -> ClaimedTurnRun {
    let descriptor = ironclaw_turns::AgentLoopDriverDescriptor {
        id: LoopDriverId::new("test_loop").expect("valid"),
        version: RunProfileVersion::new(1),
        checkpoint_schema_id: Some(CheckpointSchemaId::new("test_checkpoint").expect("valid")),
        checkpoint_schema_version: Some(RunProfileVersion::new(1)),
    };
    let scope = TurnScope::new(
        TenantId::new("tenant").expect("valid"),
        None,
        None,
        ThreadId::new("thread").expect("valid"),
    );
    let mut profile = test_profile(descriptor);
    profile.checkpoint_policy.require_final_checkpoint = false;
    profile.checkpoint_policy.allow_no_reply_completion = false;
    ClaimedTurnRun {
        state: TurnRunState {
            scope,
            actor: None,
            turn_id: TurnId::new(),
            run_id: TurnRunId::new(),
            status: TurnStatus::Running,
            accepted_message_ref: AcceptedMessageRef::new("msg:accepted").expect("valid"),
            source_binding_ref: SourceBindingRef::new("source").expect("valid"),
            reply_target_binding_ref: ReplyTargetBindingRef::new("reply").expect("valid"),
            resolved_run_profile_id: ironclaw_turns::RunProfileId::default_profile(),
            resolved_run_profile_version: RunProfileVersion::new(1),
            resolved_model_route: None,
            received_at: chrono::Utc::now(),
            checkpoint_id: None,
            gate_ref: None,
            failure: None,
            event_cursor: EventCursor(0),
        },
        resolved_run_profile: profile,
        runner_id: TurnRunnerId::new(),
        lease_token: TurnLeaseToken::new(),
    }
}

fn test_profile(
    descriptor: ironclaw_turns::AgentLoopDriverDescriptor,
) -> ironclaw_turns::ResolvedRunProfile {
    use ironclaw_turns::run_profile::*;
    use ironclaw_turns::*;

    ResolvedRunProfile {
        run_class_id: RunClassId::new("test_class").expect("valid"),
        profile_id: RunProfileId::default_profile(),
        profile_version: RunProfileVersion::new(1),
        loop_driver: descriptor.clone(),
        checkpoint_schema_id: descriptor.checkpoint_schema_id.clone().expect("schema"),
        checkpoint_schema_version: descriptor.checkpoint_schema_version.expect("version"),
        model_profile_id: ModelProfileId::new("test_model").expect("valid"),
        capability_surface_profile_id: CapabilitySurfaceProfileId::new("test_capabilities")
            .expect("valid"),
        context_profile_id: ContextProfileId::new("test_context").expect("valid"),
        steering_policy: SteeringPolicy {
            allow_steering: false,
            allow_interrupt: true,
            allow_driver_specific_nudges: false,
        },
        cancellation_policy: CancellationPolicy {
            allow_cancel: true,
            require_checkpoint_before_cancel: false,
        },
        checkpoint_policy: CheckpointPolicy {
            require_before_model: false,
            require_before_side_effect: false,
            require_before_block: true,
            max_checkpoint_bytes: 64 * 1024,
            require_final_checkpoint: false,
            allow_no_reply_completion: false,
        },
        resource_budget_policy: ResourceBudgetPolicy {
            tier: ResourceBudgetTier::new("test_tier").expect("valid"),
            max_model_calls: 32,
            max_capability_invocations: 64,
        },
        personal_context_policy: ironclaw_turns::run_profile::PersonalContextPolicy::Excluded,
        runtime_constraints: RuntimeProfileConstraints {
            allow_raw_runtime_backend_selection: false,
            allow_broad_capability_surface: false,
        },
        runner_pool_id: None,
        scheduling_class: SchedulingClass::new("interactive").expect("valid"),
        concurrency_class: ConcurrencyClass::new("thread_serial").expect("valid"),
        resolution_fingerprint: RunProfileFingerprint::new("test-fingerprint-v1").expect("valid"),
        provenance: RedactedRunProfileProvenance {
            sources: vec![],
            effective_privileges: vec![],
        },
    }
}

#[derive(Default)]
pub(super) struct RecordingTransitionPort {
    raw_failures: Mutex<Vec<String>>,
    apply_calls: Mutex<usize>,
}

impl RecordingTransitionPort {
    pub(super) fn new() -> Self {
        Self::default()
    }

    pub(super) fn raw_failure_texts(&self) -> Vec<String> {
        self.raw_failures.lock().expect("lock").clone()
    }

    pub(super) fn apply_count(&self) -> usize {
        *self.apply_calls.lock().expect("lock")
    }
}

#[async_trait]
impl TurnRunTransitionPort for RecordingTransitionPort {
    async fn claim_next_run(
        &self,
        _request: ClaimRunRequest,
    ) -> Result<Option<ClaimedTurnRun>, TurnError> {
        Ok(None)
    }

    async fn heartbeat(&self, _request: HeartbeatRequest) -> Result<EventCursor, TurnError> {
        Ok(EventCursor(0))
    }

    async fn recover_expired_leases(
        &self,
        _request: RecoverExpiredLeasesRequest,
    ) -> Result<RecoverExpiredLeasesResponse, TurnError> {
        Ok(RecoverExpiredLeasesResponse { recovered: vec![] })
    }

    async fn record_model_route_snapshot(
        &self,
        _request: RecordModelRouteSnapshotRequest,
    ) -> Result<TurnRunState, TurnError> {
        panic!("loop-exit applier tests should not record model route snapshots")
    }

    async fn block_run(&self, request: BlockRunRequest) -> Result<TurnRunState, TurnError> {
        Ok(state_for_mapping(
            TurnStatus::BlockedApproval,
            request.run_id,
            None,
            None,
        ))
    }

    async fn complete_run(&self, request: CompleteRunRequest) -> Result<TurnRunState, TurnError> {
        Ok(state_for_mapping(
            TurnStatus::Completed,
            request.run_id,
            None,
            None,
        ))
    }

    async fn cancel_run(
        &self,
        request: CancelRunCompletionRequest,
    ) -> Result<TurnRunState, TurnError> {
        Ok(state_for_mapping(
            TurnStatus::Cancelled,
            request.run_id,
            None,
            None,
        ))
    }

    async fn fail_run(&self, request: FailRunRequest) -> Result<TurnRunState, TurnError> {
        self.raw_failures
            .lock()
            .expect("lock")
            .push(request.failure.category().to_string());
        Ok(state_for_mapping(
            TurnStatus::Failed,
            request.run_id,
            Some(request.failure),
            None,
        ))
    }

    async fn record_runner_failure(
        &self,
        request: RecordRunnerFailureRequest,
    ) -> Result<TurnRunState, TurnError> {
        self.raw_failures
            .lock()
            .expect("lock")
            .push(request.failure.category().to_string());
        Ok(state_for_mapping(
            TurnStatus::Failed,
            request.run_id,
            Some(request.failure),
            None,
        ))
    }

    async fn apply_validated_loop_exit(
        &self,
        request: ApplyValidatedLoopExitRequest,
    ) -> Result<TurnRunState, TurnError> {
        *self.apply_calls.lock().expect("lock") += 1;
        match request.mapping {
            ironclaw_turns::LoopExitMapping::RunnerOutcome(outcome) => match outcome {
                ironclaw_turns::runner::TurnRunnerOutcome::Completed => {
                    self.complete_run(CompleteRunRequest {
                        run_id: request.run_id,
                        runner_id: request.runner_id,
                        lease_token: request.lease_token,
                    })
                    .await
                }
                ironclaw_turns::runner::TurnRunnerOutcome::Cancelled => {
                    self.cancel_run(CancelRunCompletionRequest {
                        run_id: request.run_id,
                        runner_id: request.runner_id,
                        lease_token: request.lease_token,
                    })
                    .await
                }
                ironclaw_turns::runner::TurnRunnerOutcome::Blocked {
                    checkpoint_id: _,
                    state_ref: _,
                    reason,
                } => {
                    let status = reason.status();
                    Ok(state_for_mapping(
                        status,
                        request.run_id,
                        None,
                        Some(reason.gate_ref().clone()),
                    ))
                }
                ironclaw_turns::runner::TurnRunnerOutcome::Failed { failure } => {
                    self.fail_run(FailRunRequest {
                        run_id: request.run_id,
                        runner_id: request.runner_id,
                        lease_token: request.lease_token,
                        failure,
                    })
                    .await
                }
            },
            ironclaw_turns::LoopExitMapping::RecoveryRequired { failure } => {
                self.record_runner_failure(RecordRunnerFailureRequest {
                    run_id: request.run_id,
                    runner_id: request.runner_id,
                    lease_token: request.lease_token,
                    failure,
                })
                .await
            }
        }
    }
}

fn state_for_mapping(
    status: TurnStatus,
    run_id: TurnRunId,
    failure: Option<SanitizedFailure>,
    gate_ref: Option<GateRef>,
) -> TurnRunState {
    TurnRunState {
        scope: TurnScope::new(
            TenantId::new("tenant").expect("valid"),
            None,
            None,
            ThreadId::new("thread").expect("valid"),
        ),
        actor: None,
        turn_id: TurnId::new(),
        run_id,
        status,
        accepted_message_ref: AcceptedMessageRef::new("msg:accepted").expect("valid"),
        source_binding_ref: SourceBindingRef::new("source").expect("valid"),
        reply_target_binding_ref: ReplyTargetBindingRef::new("reply").expect("valid"),
        resolved_run_profile_id: ironclaw_turns::RunProfileId::default_profile(),
        resolved_run_profile_version: RunProfileVersion::new(1),
        resolved_model_route: None,
        received_at: chrono::Utc::now(),
        checkpoint_id: None,
        gate_ref,
        failure,
        event_cursor: EventCursor(0),
    }
}
