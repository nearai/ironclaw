use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ironclaw_host_api::{TenantId, ThreadId};
use ironclaw_threads::InMemorySessionThreadService;
use ironclaw_turns::{
    AcceptedMessageRef, CancelRunRequest, CancelRunResponse, EventCursor, GateRef,
    GetLoopCheckpointRequest, GetRunStateRequest, LoopBlocked, LoopBlockedKind, LoopCheckpointKind,
    LoopCheckpointRecord, LoopCheckpointStateRef, LoopCheckpointStore, LoopCompleted,
    LoopCompletionKind, LoopExit, LoopExitId, LoopFailed, LoopFailureKind, LoopGateRef,
    LoopMessageRef, PutLoopCheckpointRequest, ReplyTargetBindingRef, ResumeTurnRequest,
    ResumeTurnResponse, RunProfileVersion, SanitizedFailure, SourceBindingRef, SubmitTurnRequest,
    SubmitTurnResponse, TurnCheckpointId, TurnError, TurnId, TurnLeaseToken, TurnRunId,
    TurnRunState, TurnRunnerId, TurnScope, TurnStateStore, TurnStatus,
    run_profile::{CheckpointSchemaId, LoopDriverId},
    runner::{
        ApplyValidatedLoopExitRequest, BlockRunRequest, CancelRunCompletionRequest,
        ClaimRunRequest, ClaimedTurnRun, CompleteRunRequest, FailRunRequest, HeartbeatRequest,
        RecordModelRouteSnapshotRequest, RecordRecoveryRequiredRequest,
        RecoverExpiredLeasesRequest, RecoverExpiredLeasesResponse, TurnRunTransitionPort,
    },
};

use super::*;

#[tokio::test]
async fn loop_exit_applier_rejects_driver_supplied_evidence_policy() {
    let fixture = Fixture::new(InMemoryLoopExitEvidencePort::new());
    let exit = completed_exit(vec![LoopMessageRef::new("msg:reply").expect("valid")], None);

    let state = fixture
        .applier
        .apply(&fixture.claimed, exit)
        .await
        .expect("applied");

    assert_eq!(state.status, TurnStatus::Failed);
    assert_eq!(
        state.failure.expect("failure").category(),
        "driver_protocol_violation"
    );
}

#[tokio::test]
async fn completed_exit_requires_same_run_finalized_message_ref() {
    let evidence = InMemoryLoopExitEvidencePort::all_verified();
    let fixture = Fixture::new(evidence);
    let final_checkpoint = TurnCheckpointId::new();
    let exit = completed_exit(
        vec![LoopMessageRef::new("msg:reply").expect("valid")],
        Some(final_checkpoint),
    );

    let state = fixture
        .applier
        .apply(&fixture.claimed, exit)
        .await
        .expect("applied");

    assert_eq!(state.status, TurnStatus::Completed);
}

#[tokio::test]
async fn no_reply_completion_requires_profile_permission() {
    let evidence = InMemoryLoopExitEvidencePort::all_verified();
    let fixture = Fixture::new(evidence);
    let exit = LoopExit::Completed(LoopCompleted {
        completion_kind: LoopCompletionKind::NoReply,
        reply_message_refs: vec![],
        result_refs: vec![],
        final_checkpoint_id: None,
        usage_summary_ref: None,
        exit_id: test_exit_id(),
    });

    let state = fixture
        .applier
        .apply(&fixture.claimed, exit)
        .await
        .expect("applied");

    assert_eq!(state.status, TurnStatus::Failed);
    assert_eq!(
        state.failure.expect("failure").category(),
        "driver_protocol_violation"
    );
}

#[tokio::test]
async fn production_completed_exit_requires_final_checkpoint() {
    let mut claimed = claimed_run();
    claimed
        .resolved_run_profile
        .checkpoint_policy
        .require_final_checkpoint = true;
    let transition = Arc::new(RecordingTransitionPort::new());
    let applier = Arc::new(LoopExitApplier::new(
        transition,
        Arc::new(InMemoryLoopExitEvidencePort::all_verified()),
    ));
    let exit = completed_exit(vec![LoopMessageRef::new("msg:reply").expect("valid")], None);

    let state = applier.apply(&claimed, exit).await.expect("applied");

    assert_eq!(state.status, TurnStatus::Failed);
    assert_eq!(
        state.failure.expect("failure").category(),
        "driver_protocol_violation"
    );
}

#[tokio::test]
async fn blocked_exit_requires_before_block_checkpoint() {
    let fixture = Fixture::new(InMemoryLoopExitEvidencePort::new());
    let exit = blocked_exit(LoopBlockedKind::Approval);

    let state = fixture
        .applier
        .apply(&fixture.claimed, exit)
        .await
        .expect("applied");

    assert_eq!(state.status, TurnStatus::Failed);
    assert_eq!(
        state.failure.expect("failure").category(),
        "driver_protocol_violation"
    );
}

#[tokio::test]
async fn cancelled_exit_requires_observed_cancel_input() {
    let fixture =
        Fixture::new(InMemoryLoopExitEvidencePort::new().with_final_checkpoint_verified(true));
    let exit = LoopExit::Cancelled(ironclaw_turns::LoopCancelled {
        reason_kind: ironclaw_turns::LoopCancelledReasonKind::HostCancellation,
        checkpoint_id: None,
        interrupted_message_refs: vec![],
        exit_id: test_exit_id(),
    });

    let state = fixture
        .applier
        .apply(&fixture.claimed, exit)
        .await
        .expect("applied");

    assert_eq!(state.status, TurnStatus::Failed);
    assert_eq!(
        state.failure.expect("failure").category(),
        "interrupted_unexpectedly"
    );
}

#[tokio::test]
async fn observed_host_cancellation_still_requires_final_checkpoint_when_configured() {
    let mut claimed = claimed_run();
    claimed
        .resolved_run_profile
        .checkpoint_policy
        .require_final_checkpoint = true;
    let transition = Arc::new(RecordingTransitionPort::new());
    let applier = Arc::new(LoopExitApplier::new(
        transition,
        Arc::new(InMemoryLoopExitEvidencePort::new().with_cancellation_observed(true)),
    ));
    let exit = LoopExit::Cancelled(ironclaw_turns::LoopCancelled {
        reason_kind: ironclaw_turns::LoopCancelledReasonKind::HostCancellation,
        checkpoint_id: None,
        interrupted_message_refs: vec![],
        exit_id: test_exit_id(),
    });

    let state = applier.apply(&claimed, exit).await.expect("applied");

    assert_eq!(state.status, TurnStatus::Failed);
    assert_eq!(
        state.failure.expect("failure").category(),
        "driver_protocol_violation"
    );
}

#[tokio::test]
async fn thread_checkpoint_evidence_accepts_durable_cancel_requested_run() {
    let claimed = claimed_run();
    let mut observed_state = claimed.state.clone();
    observed_state.status = TurnStatus::CancelRequested;
    let transition = Arc::new(RecordingTransitionPort::new());
    let evidence = Arc::new(ThreadCheckpointLoopExitEvidencePort::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(StaticTurnStateStore::new(observed_state)),
        Arc::new(PanicLoopCheckpointStore),
    ));
    let applier = Arc::new(LoopExitApplier::new(transition.clone(), evidence));
    let exit = LoopExit::Cancelled(ironclaw_turns::LoopCancelled {
        reason_kind: ironclaw_turns::LoopCancelledReasonKind::HostCancellation,
        checkpoint_id: None,
        interrupted_message_refs: vec![],
        exit_id: test_exit_id(),
    });

    let state = applier.apply(&claimed, exit).await.expect("applied");

    assert_eq!(state.status, TurnStatus::Cancelled);
    assert_eq!(transition.apply_count(), 1);
}

#[tokio::test]
async fn thread_checkpoint_evidence_accepts_durable_cancelled_run() {
    let claimed = claimed_run();
    let mut observed_state = claimed.state.clone();
    observed_state.status = TurnStatus::Cancelled;
    let transition = Arc::new(RecordingTransitionPort::new());
    let evidence = Arc::new(ThreadCheckpointLoopExitEvidencePort::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(StaticTurnStateStore::new(observed_state)),
        Arc::new(PanicLoopCheckpointStore),
    ));
    let applier = Arc::new(LoopExitApplier::new(transition.clone(), evidence));
    let exit = LoopExit::Cancelled(ironclaw_turns::LoopCancelled {
        reason_kind: ironclaw_turns::LoopCancelledReasonKind::HostCancellation,
        checkpoint_id: None,
        interrupted_message_refs: vec![],
        exit_id: test_exit_id(),
    });

    let state = applier.apply(&claimed, exit).await.expect("applied");

    assert_eq!(state.status, TurnStatus::Cancelled);
    assert_eq!(transition.apply_count(), 1);
}

#[tokio::test]
async fn invalid_exit_after_before_side_effect_requires_recovery() {
    let evidence = InMemoryLoopExitEvidencePort::new()
        .with_latest_checkpoint_kind(Some(LoopCheckpointKind::BeforeSideEffect));
    let fixture = Fixture::new(evidence);
    let exit = completed_exit(vec![LoopMessageRef::new("msg:reply").expect("valid")], None);

    let state = fixture
        .applier
        .apply(&fixture.claimed, exit)
        .await
        .expect("applied");

    assert_eq!(state.status, TurnStatus::RecoveryRequired);
    assert_eq!(
        state.failure.expect("failure").category(),
        "driver_protocol_violation"
    );
}

#[tokio::test]
async fn recovery_required_keeps_active_thread_lock() {
    assert!(TurnStatus::RecoveryRequired.keeps_active_lock());
}

#[tokio::test]
async fn loop_exit_events_hide_raw_diagnostics() {
    let evidence = InMemoryLoopExitEvidencePort::new();
    let fixture = Fixture::new(evidence);
    let exit = LoopExit::Failed(LoopFailed {
        reason_kind: LoopFailureKind::ModelError,
        checkpoint_id: None,
        usage_summary_ref: None,
        diagnostic_ref: None,
        exit_id: test_exit_id(),
    });

    let state = fixture
        .applier
        .apply(&fixture.claimed, exit)
        .await
        .expect("applied");

    assert_eq!(state.status, TurnStatus::Failed);
    assert_eq!(
        state.failure.expect("failure").category(),
        "driver_protocol_violation"
    );
    assert_eq!(
        fixture.transition.raw_failure_texts(),
        vec!["driver_protocol_violation"]
    );
}

#[tokio::test]
async fn thread_checkpoint_evidence_rejects_agentless_completion_refs_explicitly() {
    let evidence = text_checkpoint_evidence(Arc::new(PanicLoopCheckpointStore));
    let claimed = claimed_run();
    let err = evidence
        .verify_completion_refs(CompletionEvidenceRequest {
            scope: &claimed.state.scope,
            turn_id: claimed.state.turn_id,
            run_id: claimed.state.run_id,
            reply_message_refs: &[LoopMessageRef::new("msg:reply").expect("valid")],
            result_refs: &[],
        })
        .await
        .expect_err("agentless scope should be rejected explicitly");

    assert!(matches!(err, TurnError::InvalidRequest { .. }));
    assert!(err.to_string().contains("agent-scoped"));
}

#[tokio::test]
async fn applier_rejects_agentless_transcript_evidence_before_transition() {
    let transition = Arc::new(RecordingTransitionPort::new());
    let evidence = text_checkpoint_evidence(Arc::new(PanicLoopCheckpointStore));
    let applier = LoopExitApplier::new(transition.clone(), Arc::new(evidence));
    let claimed = claimed_run();
    let exit = completed_exit(vec![LoopMessageRef::new("msg:reply").expect("valid")], None);

    let err = applier
        .apply(&claimed, exit)
        .await
        .expect_err("agentless transcript evidence should stop before transition");

    assert!(matches!(err, TurnError::InvalidRequest { .. }));
    assert_eq!(transition.apply_count(), 0);
}

#[tokio::test]
async fn thread_checkpoint_evidence_does_not_read_checkpoint_for_blocked_claims() {
    let evidence = text_checkpoint_evidence(Arc::new(PanicLoopCheckpointStore));
    let claimed = claimed_run();
    let exit = blocked_exit(LoopBlockedKind::Approval);
    let LoopExit::Blocked(blocked) = &exit else {
        unreachable!("blocked helper returns blocked exit")
    };
    let verified = evidence
        .verify_blocked_evidence(BlockedEvidenceRequest {
            scope: &claimed.state.scope,
            turn_id: claimed.state.turn_id,
            run_id: claimed.state.run_id,
            blocked,
        })
        .await
        .expect("blocked evidence should fail closed without checkpoint I/O");

    assert!(!verified);
}

#[tokio::test]
async fn thread_checkpoint_evidence_fails_closed_for_failure_evidence() {
    let evidence = text_checkpoint_evidence(Arc::new(PanicLoopCheckpointStore));
    let claimed = claimed_run();
    let failed = LoopFailed {
        reason_kind: LoopFailureKind::ModelError,
        checkpoint_id: None,
        usage_summary_ref: None,
        diagnostic_ref: None,
        exit_id: test_exit_id(),
    };
    let verified = evidence
        .verify_failure_evidence(FailureEvidenceRequest {
            scope: &claimed.state.scope,
            turn_id: claimed.state.turn_id,
            run_id: claimed.state.run_id,
            failed: &failed,
        })
        .await
        .expect("missing diagnostics store should fail closed");

    assert!(!verified);
}

#[tokio::test]
async fn thread_checkpoint_evidence_assumes_recovery_when_latest_checkpoint_unknown() {
    let evidence = text_checkpoint_evidence(Arc::new(PanicLoopCheckpointStore));
    let claimed = claimed_run();
    let latest = evidence
        .latest_checkpoint_kind(
            &claimed.state.scope,
            claimed.state.turn_id,
            claimed.state.run_id,
        )
        .await
        .expect("latest checkpoint fallback should not read store");

    assert_eq!(latest, Some(LoopCheckpointKind::BeforeSideEffect));
}

fn text_checkpoint_evidence(
    loop_checkpoint_store: Arc<dyn LoopCheckpointStore>,
) -> ThreadCheckpointLoopExitEvidencePort<InMemorySessionThreadService> {
    ThreadCheckpointLoopExitEvidencePort::new(
        Arc::new(InMemorySessionThreadService::default()),
        Arc::new(StaticTurnStateStore::new(claimed_run().state)),
        loop_checkpoint_store,
    )
}

struct StaticTurnStateStore {
    state: TurnRunState,
}

impl StaticTurnStateStore {
    fn new(state: TurnRunState) -> Self {
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

struct PanicLoopCheckpointStore;

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

fn test_exit_id() -> LoopExitId {
    LoopExitId::new("exit:test").expect("valid")
}

fn completed_exit(
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

fn blocked_exit(kind: LoopBlockedKind) -> LoopExit {
    LoopExit::Blocked(LoopBlocked {
        kind,
        gate_ref: LoopGateRef::new("gate:test").expect("valid"),
        checkpoint_id: TurnCheckpointId::new(),
        state_ref: LoopCheckpointStateRef::new("checkpoint:blocked-state").expect("valid"),
        exit_id: test_exit_id(),
    })
}

struct Fixture {
    claimed: ClaimedTurnRun,
    transition: Arc<RecordingTransitionPort>,
    applier: Arc<LoopExitApplier>,
}

impl Fixture {
    fn new(evidence: InMemoryLoopExitEvidencePort) -> Self {
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

fn claimed_run() -> ClaimedTurnRun {
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
struct RecordingTransitionPort {
    raw_failures: Mutex<Vec<String>>,
    apply_calls: Mutex<usize>,
}

impl RecordingTransitionPort {
    fn new() -> Self {
        Self::default()
    }

    fn raw_failure_texts(&self) -> Vec<String> {
        self.raw_failures.lock().expect("lock").clone()
    }

    fn apply_count(&self) -> usize {
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

    async fn record_recovery_required(
        &self,
        request: RecordRecoveryRequiredRequest,
    ) -> Result<TurnRunState, TurnError> {
        self.raw_failures
            .lock()
            .expect("lock")
            .push(request.failure.category().to_string());
        Ok(state_for_mapping(
            TurnStatus::RecoveryRequired,
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
                self.record_recovery_required(RecordRecoveryRequiredRequest {
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
