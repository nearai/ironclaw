use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use ironclaw_host_api::{TenantId, ThreadId};
use ironclaw_turns::{
    AcceptedMessageRef, BlockedReason, EventCursor, GateRef, LoopBlocked, LoopBlockedKind,
    LoopCancelled, LoopCancelledReasonKind, LoopCompleted, LoopCompletionKind, LoopExit,
    LoopExitId, LoopExitMapping, LoopFailed, LoopFailureKind, LoopMessageRef,
    ReplyTargetBindingRef, RunProfileId, RunProfileVersion, SourceBindingRef, TurnCheckpointId,
    TurnError, TurnId, TurnLeaseToken, TurnRunId, TurnRunState, TurnRunnerId, TurnScope,
    TurnStatus,
    run_profile::*,
    runner::{
        ApplyValidatedLoopExitRequest, BlockRunRequest, CancelRunCompletionRequest,
        ClaimRunRequest, ClaimedTurnRun, CompleteRunRequest, FailRunRequest, HeartbeatRequest,
        RecordRecoveryRequiredRequest, RecoverExpiredLeasesRequest, RecoverExpiredLeasesResponse,
        TurnRunTransitionPort, TurnRunnerOutcome,
    },
};

use super::{InMemoryLoopExitEvidencePort, LoopExitApplier};

// ─── Helpers ────────────────────────────────────────────────────────────────

fn test_scope() -> TurnScope {
    TurnScope::new(
        TenantId::new("test-tenant").expect("valid"),
        None,
        None,
        ThreadId::new("test-thread").expect("valid"),
    )
}

fn test_run_state(status: TurnStatus) -> TurnRunState {
    TurnRunState {
        scope: test_scope(),
        turn_id: TurnId::new(),
        run_id: TurnRunId::new(),
        status,
        accepted_message_ref: AcceptedMessageRef::new("test-msg").expect("valid"),
        source_binding_ref: SourceBindingRef::new("test-source").expect("valid"),
        reply_target_binding_ref: ReplyTargetBindingRef::new("test-reply").expect("valid"),
        resolved_run_profile_id: RunProfileId::default_profile(),
        resolved_run_profile_version: RunProfileVersion::new(1),
        received_at: chrono::Utc::now(),
        checkpoint_id: None,
        gate_ref: None,
        failure: None,
        event_cursor: EventCursor(0),
    }
}

fn test_profile(require_final_checkpoint: bool) -> ResolvedRunProfile {
    ResolvedRunProfile {
        run_class_id: RunClassId::new("test_class").expect("valid"),
        profile_id: RunProfileId::default_profile(),
        profile_version: RunProfileVersion::new(1),
        loop_driver: AgentLoopDriverDescriptor {
            id: LoopDriverId::new("test_loop").expect("valid"),
            version: RunProfileVersion::new(1),
            checkpoint_schema_id: Some(CheckpointSchemaId::new("test_cp").expect("valid")),
            checkpoint_schema_version: Some(RunProfileVersion::new(1)),
        },
        checkpoint_schema_id: CheckpointSchemaId::new("test_cp").expect("valid"),
        checkpoint_schema_version: RunProfileVersion::new(1),
        model_profile_id: ModelProfileId::new("test_model").expect("valid"),
        capability_surface_profile_id: CapabilitySurfaceProfileId::new("test_caps").expect("valid"),
        context_profile_id: ContextProfileId::new("test_ctx").expect("valid"),
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
            require_final_checkpoint,
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
        resolution_fingerprint: RunProfileFingerprint::new("test-fp-v1").expect("valid"),
        provenance: RedactedRunProfileProvenance {
            sources: vec![],
            effective_privileges: vec![],
        },
    }
}

// ─── Mock transition port ───────────────────────────────────────────────────

#[derive(Debug)]
struct CapturingTransitionPort {
    captured_requests: Mutex<Vec<ApplyValidatedLoopExitRequest>>,
    result_status: TurnStatus,
}

impl CapturingTransitionPort {
    fn new(result_status: TurnStatus) -> Self {
        Self {
            captured_requests: Mutex::new(Vec::new()),
            result_status,
        }
    }

    fn captured(&self) -> Vec<ApplyValidatedLoopExitRequest> {
        self.captured_requests.lock().expect("lock").clone()
    }
}

#[async_trait]
impl TurnRunTransitionPort for CapturingTransitionPort {
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
        Ok(RecoverExpiredLeasesResponse {
            recovered: Vec::new(),
        })
    }

    async fn block_run(&self, _request: BlockRunRequest) -> Result<TurnRunState, TurnError> {
        Ok(test_run_state(TurnStatus::BlockedApproval))
    }

    async fn complete_run(&self, _request: CompleteRunRequest) -> Result<TurnRunState, TurnError> {
        Ok(test_run_state(TurnStatus::Completed))
    }

    async fn cancel_run(
        &self,
        _request: CancelRunCompletionRequest,
    ) -> Result<TurnRunState, TurnError> {
        Ok(test_run_state(TurnStatus::Cancelled))
    }

    async fn fail_run(&self, _request: FailRunRequest) -> Result<TurnRunState, TurnError> {
        Ok(test_run_state(TurnStatus::Failed))
    }

    async fn record_recovery_required(
        &self,
        _request: RecordRecoveryRequiredRequest,
    ) -> Result<TurnRunState, TurnError> {
        Ok(test_run_state(TurnStatus::RecoveryRequired))
    }

    async fn apply_validated_loop_exit(
        &self,
        request: ApplyValidatedLoopExitRequest,
    ) -> Result<TurnRunState, TurnError> {
        self.captured_requests.lock().expect("lock").push(request);
        Ok(test_run_state(self.result_status))
    }
}

// ─── Test setup ─────────────────────────────────────────────────────────────

struct TestSetup {
    applier: LoopExitApplier,
    port: Arc<CapturingTransitionPort>,
    scope: TurnScope,
    run_id: TurnRunId,
    runner_id: TurnRunnerId,
    lease_token: TurnLeaseToken,
}

fn setup(evidence: InMemoryLoopExitEvidencePort, result_status: TurnStatus) -> TestSetup {
    let port = Arc::new(CapturingTransitionPort::new(result_status));
    let applier = LoopExitApplier::new(port.clone(), Arc::new(evidence));
    TestSetup {
        applier,
        port,
        scope: test_scope(),
        run_id: TurnRunId::new(),
        runner_id: TurnRunnerId::new(),
        lease_token: TurnLeaseToken::new(),
    }
}

impl TestSetup {
    async fn apply(
        &self,
        exit: LoopExit,
        profile: &ResolvedRunProfile,
    ) -> Result<TurnRunState, TurnError> {
        self.applier
            .apply(
                &self.scope,
                self.run_id,
                self.runner_id,
                self.lease_token,
                exit,
                profile,
            )
            .await
    }

    fn captured_mapping(&self) -> LoopExitMapping {
        let captured = self.port.captured();
        assert_eq!(captured.len(), 1, "expected exactly one apply call");
        captured[0].mapping.clone()
    }
}

fn completed_exit_with_refs() -> LoopExit {
    LoopExit::Completed(LoopCompleted {
        completion_kind: LoopCompletionKind::FinalReply,
        reply_message_refs: vec![LoopMessageRef::new("msg:test-1").expect("valid")],
        result_refs: vec![],
        final_checkpoint_id: Some(TurnCheckpointId::new()),
        usage_summary_ref: None,
        exit_id: LoopExitId::new("exit:completed-1").expect("valid"),
    })
}

fn completed_exit_no_reply() -> LoopExit {
    LoopExit::Completed(LoopCompleted {
        completion_kind: LoopCompletionKind::NoReply,
        reply_message_refs: vec![],
        result_refs: vec![],
        final_checkpoint_id: None,
        usage_summary_ref: None,
        exit_id: LoopExitId::new("exit:noreply-1").expect("valid"),
    })
}

fn completed_exit_no_checkpoint() -> LoopExit {
    LoopExit::Completed(LoopCompleted {
        completion_kind: LoopCompletionKind::FinalReply,
        reply_message_refs: vec![LoopMessageRef::new("msg:test-1").expect("valid")],
        result_refs: vec![],
        final_checkpoint_id: None,
        usage_summary_ref: None,
        exit_id: LoopExitId::new("exit:nocp-1").expect("valid"),
    })
}

fn blocked_exit(kind: LoopBlockedKind) -> LoopExit {
    blocked_exit_with_ref(kind, "gate:test-1")
}

fn blocked_exit_with_ref(kind: LoopBlockedKind, gate_ref: &str) -> LoopExit {
    LoopExit::Blocked(LoopBlocked {
        kind,
        gate_ref: GateRef::new(gate_ref).expect("valid"),
        checkpoint_id: TurnCheckpointId::new(),
        exit_id: LoopExitId::new("exit:blocked-1").expect("valid"),
    })
}

fn cancelled_exit() -> LoopExit {
    LoopExit::Cancelled(LoopCancelled {
        reason_kind: LoopCancelledReasonKind::HostCancellation,
        checkpoint_id: None,
        interrupted_message_refs: vec![],
        exit_id: LoopExitId::new("exit:cancelled-1").expect("valid"),
    })
}

fn cancelled_exit_with_checkpoint() -> LoopExit {
    LoopExit::Cancelled(LoopCancelled {
        reason_kind: LoopCancelledReasonKind::HostCancellation,
        checkpoint_id: Some(TurnCheckpointId::new()),
        interrupted_message_refs: vec![],
        exit_id: LoopExitId::new("exit:cancelled-cp-1").expect("valid"),
    })
}

fn failed_exit() -> LoopExit {
    LoopExit::Failed(LoopFailed {
        reason_kind: LoopFailureKind::ModelError,
        checkpoint_id: None,
        usage_summary_ref: None,
        diagnostic_ref: None,
        exit_id: LoopExitId::new("exit:failed-1").expect("valid"),
    })
}

fn failed_exit_with_checkpoint() -> LoopExit {
    LoopExit::Failed(LoopFailed {
        reason_kind: LoopFailureKind::ModelError,
        checkpoint_id: Some(TurnCheckpointId::new()),
        usage_summary_ref: None,
        diagnostic_ref: None,
        exit_id: LoopExitId::new("exit:failed-cp-1").expect("valid"),
    })
}

fn is_recovery_mapping(mapping: &LoopExitMapping) -> bool {
    matches!(mapping, LoopExitMapping::RecoveryRequired { .. })
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn completed_final_reply_verified_refs_trusted() {
    let s = setup(
        InMemoryLoopExitEvidencePort::new().with_completion_refs_verified(true),
        TurnStatus::Completed,
    );
    let profile = test_profile(false);

    s.apply(completed_exit_with_refs(), &profile)
        .await
        .expect("should succeed");

    let mapping = s.captured_mapping();
    assert_eq!(
        mapping,
        LoopExitMapping::RunnerOutcome(TurnRunnerOutcome::Completed)
    );
}

#[tokio::test]
async fn completed_final_reply_unverified_refs_recovery() {
    let s = setup(
        InMemoryLoopExitEvidencePort::new().with_completion_refs_verified(false),
        TurnStatus::RecoveryRequired,
    );
    let profile = test_profile(false);

    s.apply(completed_exit_with_refs(), &profile)
        .await
        .expect("should succeed");

    assert!(is_recovery_mapping(&s.captured_mapping()));
}

#[tokio::test]
async fn completed_no_reply_empty_refs_violation() {
    let s = setup(
        InMemoryLoopExitEvidencePort::all_verified(),
        TurnStatus::RecoveryRequired,
    );
    let profile = test_profile(false);

    s.apply(completed_exit_no_reply(), &profile)
        .await
        .expect("should succeed");

    // NoReply with empty refs → MissingCompletionReference via validate()
    assert!(is_recovery_mapping(&s.captured_mapping()));
}

#[tokio::test]
async fn completed_missing_checkpoint_when_required_violation() {
    let s = setup(
        InMemoryLoopExitEvidencePort::new().with_completion_refs_verified(true),
        TurnStatus::RecoveryRequired,
    );
    let profile = test_profile(true); // require_final_checkpoint = true

    s.apply(completed_exit_no_checkpoint(), &profile)
        .await
        .expect("should succeed");

    assert!(is_recovery_mapping(&s.captured_mapping()));
}

#[tokio::test]
async fn completed_final_checkpoint_requires_durable_evidence() {
    let s = setup(
        InMemoryLoopExitEvidencePort::new().with_completion_refs_verified(true),
        TurnStatus::RecoveryRequired,
    );
    let profile = test_profile(true); // require_final_checkpoint = true

    s.apply(completed_exit_with_refs(), &profile)
        .await
        .expect("should succeed");

    assert!(is_recovery_mapping(&s.captured_mapping()));
}

#[tokio::test]
async fn completed_final_checkpoint_verified_trusted() {
    let s = setup(
        InMemoryLoopExitEvidencePort::new()
            .with_completion_refs_verified(true)
            .with_final_checkpoint_verified(true),
        TurnStatus::Completed,
    );
    let profile = test_profile(true); // require_final_checkpoint = true

    s.apply(completed_exit_with_refs(), &profile)
        .await
        .expect("should succeed");

    assert_eq!(
        s.captured_mapping(),
        LoopExitMapping::RunnerOutcome(TurnRunnerOutcome::Completed)
    );
}

#[tokio::test]
async fn completed_missing_checkpoint_when_not_required_trusted() {
    let s = setup(
        InMemoryLoopExitEvidencePort::new().with_completion_refs_verified(true),
        TurnStatus::Completed,
    );
    let profile = test_profile(false); // require_final_checkpoint = false

    s.apply(completed_exit_no_checkpoint(), &profile)
        .await
        .expect("should succeed");

    assert_eq!(
        s.captured_mapping(),
        LoopExitMapping::RunnerOutcome(TurnRunnerOutcome::Completed)
    );
}

#[tokio::test]
async fn blocked_approval_verified_trusted() {
    let s = setup(
        InMemoryLoopExitEvidencePort::new().with_blocked_evidence_verified(true),
        TurnStatus::BlockedApproval,
    );
    let profile = test_profile(false);

    s.apply(blocked_exit(LoopBlockedKind::Approval), &profile)
        .await
        .expect("should succeed");

    let mapping = s.captured_mapping();
    match &mapping {
        LoopExitMapping::RunnerOutcome(TurnRunnerOutcome::Blocked { reason, .. }) => {
            assert!(matches!(reason, BlockedReason::Approval { .. }));
        }
        other => panic!("expected Blocked outcome, got {other:?}"),
    }
}

#[tokio::test]
async fn blocked_process_verified_trusted() {
    let s = setup(
        InMemoryLoopExitEvidencePort::new().with_blocked_evidence_verified(true),
        TurnStatus::BlockedProcess,
    );
    let profile = test_profile(false);

    s.apply(
        blocked_exit_with_ref(LoopBlockedKind::Process, "process:test-1"),
        &profile,
    )
    .await
    .expect("should succeed");

    let mapping = s.captured_mapping();
    match &mapping {
        LoopExitMapping::RunnerOutcome(TurnRunnerOutcome::Blocked { reason, .. }) => {
            assert!(matches!(reason, BlockedReason::Process { .. }));
        }
        other => panic!("expected Blocked outcome, got {other:?}"),
    }
}

#[tokio::test]
async fn blocked_process_rejects_gate_ref_even_when_evidence_verified() {
    let s = setup(
        InMemoryLoopExitEvidencePort::new().with_blocked_evidence_verified(true),
        TurnStatus::RecoveryRequired,
    );
    let profile = test_profile(false);

    s.apply(blocked_exit(LoopBlockedKind::Process), &profile)
        .await
        .expect("should succeed");

    assert!(is_recovery_mapping(&s.captured_mapping()));
}

#[tokio::test]
async fn blocked_unverified_evidence_violation() {
    let s = setup(
        InMemoryLoopExitEvidencePort::new().with_blocked_evidence_verified(false),
        TurnStatus::RecoveryRequired,
    );
    let profile = test_profile(false);

    s.apply(blocked_exit(LoopBlockedKind::Approval), &profile)
        .await
        .expect("should succeed");

    assert!(is_recovery_mapping(&s.captured_mapping()));
}

#[tokio::test]
async fn cancelled_observed_trusted() {
    let s = setup(
        InMemoryLoopExitEvidencePort::new().with_cancellation_observed(true),
        TurnStatus::Cancelled,
    );
    let profile = test_profile(false);

    s.apply(cancelled_exit(), &profile)
        .await
        .expect("should succeed");

    assert_eq!(
        s.captured_mapping(),
        LoopExitMapping::RunnerOutcome(TurnRunnerOutcome::Cancelled)
    );
}

#[tokio::test]
async fn cancelled_missing_checkpoint_when_required_violation() {
    let s = setup(
        InMemoryLoopExitEvidencePort::new().with_cancellation_observed(true),
        TurnStatus::RecoveryRequired,
    );
    let profile = test_profile(true);

    s.apply(cancelled_exit(), &profile)
        .await
        .expect("should succeed");

    assert!(is_recovery_mapping(&s.captured_mapping()));
}

#[tokio::test]
async fn cancelled_final_checkpoint_requires_durable_evidence() {
    let s = setup(
        InMemoryLoopExitEvidencePort::new().with_cancellation_observed(true),
        TurnStatus::RecoveryRequired,
    );
    let profile = test_profile(true);

    s.apply(cancelled_exit_with_checkpoint(), &profile)
        .await
        .expect("should succeed");

    assert!(is_recovery_mapping(&s.captured_mapping()));
}

#[tokio::test]
async fn cancelled_final_checkpoint_verified_trusted() {
    let s = setup(
        InMemoryLoopExitEvidencePort::new()
            .with_cancellation_observed(true)
            .with_final_checkpoint_verified(true),
        TurnStatus::Cancelled,
    );
    let profile = test_profile(true);

    s.apply(cancelled_exit_with_checkpoint(), &profile)
        .await
        .expect("should succeed");

    assert_eq!(
        s.captured_mapping(),
        LoopExitMapping::RunnerOutcome(TurnRunnerOutcome::Cancelled)
    );
}

#[tokio::test]
async fn cancelled_not_observed_violation() {
    let s = setup(
        InMemoryLoopExitEvidencePort::new().with_cancellation_observed(false),
        TurnStatus::RecoveryRequired,
    );
    let profile = test_profile(false);

    s.apply(cancelled_exit(), &profile)
        .await
        .expect("should succeed");

    assert!(is_recovery_mapping(&s.captured_mapping()));
}

#[tokio::test]
async fn failed_verified_trusted() {
    let s = setup(
        InMemoryLoopExitEvidencePort::new().with_failure_evidence_verified(true),
        TurnStatus::Failed,
    );
    let profile = test_profile(false);

    s.apply(failed_exit(), &profile)
        .await
        .expect("should succeed");

    let mapping = s.captured_mapping();
    match &mapping {
        LoopExitMapping::RunnerOutcome(TurnRunnerOutcome::Failed { failure }) => {
            assert_eq!(failure.category(), "model_error");
        }
        other => panic!("expected Failed outcome, got {other:?}"),
    }
}

#[tokio::test]
async fn failed_unverified_violation() {
    let s = setup(
        InMemoryLoopExitEvidencePort::new().with_failure_evidence_verified(false),
        TurnStatus::RecoveryRequired,
    );
    let profile = test_profile(false);

    s.apply(failed_exit(), &profile)
        .await
        .expect("should succeed");

    assert!(is_recovery_mapping(&s.captured_mapping()));
}

#[tokio::test]
async fn failed_final_checkpoint_requires_durable_evidence() {
    let s = setup(
        InMemoryLoopExitEvidencePort::new().with_failure_evidence_verified(true),
        TurnStatus::RecoveryRequired,
    );
    let profile = test_profile(true);

    s.apply(failed_exit_with_checkpoint(), &profile)
        .await
        .expect("should succeed");

    assert!(is_recovery_mapping(&s.captured_mapping()));
}

#[tokio::test]
async fn failed_final_checkpoint_verified_trusted() {
    let s = setup(
        InMemoryLoopExitEvidencePort::new()
            .with_failure_evidence_verified(true)
            .with_final_checkpoint_verified(true),
        TurnStatus::Failed,
    );
    let profile = test_profile(true);

    s.apply(failed_exit_with_checkpoint(), &profile)
        .await
        .expect("should succeed");

    let mapping = s.captured_mapping();
    match &mapping {
        LoopExitMapping::RunnerOutcome(TurnRunnerOutcome::Failed { failure }) => {
            assert_eq!(failure.category(), "model_error");
        }
        other => panic!("expected Failed outcome, got {other:?}"),
    }
}

#[tokio::test]
async fn applier_passes_correct_request_fields_to_port() {
    let s = setup(
        InMemoryLoopExitEvidencePort::new().with_completion_refs_verified(true),
        TurnStatus::Completed,
    );
    let profile = test_profile(false);

    s.apply(completed_exit_with_refs(), &profile)
        .await
        .expect("should succeed");

    let captured = s.port.captured();
    assert_eq!(captured.len(), 1);
    let req = &captured[0];
    assert_eq!(req.run_id, s.run_id);
    assert_eq!(req.runner_id, s.runner_id);
    assert_eq!(req.lease_token, s.lease_token);
    assert_eq!(
        req.mapping,
        LoopExitMapping::RunnerOutcome(TurnRunnerOutcome::Completed)
    );
}
