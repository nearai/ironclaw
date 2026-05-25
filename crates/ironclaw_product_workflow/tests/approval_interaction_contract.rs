use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_approvals::{DenyApproval, LeaseApproval};
use ironclaw_authorization::InMemoryCapabilityLeaseStore;
use ironclaw_events::InMemoryAuditSink;
use ironclaw_host_api::{
    Action, ApprovalRequest, ApprovalRequestId, CapabilityId, CorrelationId, EffectKind,
    InvocationId, MountView, NetworkPolicy, Principal, ResourceEstimate, ResourceScope, TenantId,
    ThreadId, UserId,
};
use ironclaw_product_workflow::{
    ApprovalInteractionDecision, ApprovalInteractionReadModel, ApprovalInteractionRejectionKind,
    ApprovalInteractionScope, ApprovalInteractionService, ApprovalLeaseTermsProvider,
    ApprovalResolutionPort, ApprovalResolverPort, DefaultApprovalInteractionService,
    ListPendingApprovalsRequest, PendingApprovalGateRecord, ResolveApprovalInteractionRequest,
    ResolveApprovalInteractionResponse, RunStateApprovalInteractionReadModel, approval_gate_ref,
};
use ironclaw_run_state::{
    ApprovalRequestStore, InMemoryApprovalRequestStore, InMemoryRunStateStore, RunStart,
    RunStateStore,
};
use ironclaw_turns::{
    AcceptedMessageRef, CancelRunRequest, CancelRunResponse, EventCursor, GateRef,
    GetRunStateRequest, IdempotencyKey, ReplyTargetBindingRef, ResumeTurnPrecondition,
    ResumeTurnRequest, ResumeTurnResponse, RunProfileId, RunProfileVersion, SourceBindingRef,
    SubmitTurnRequest, SubmitTurnResponse, TurnActor, TurnCoordinator, TurnError, TurnId,
    TurnRunId, TurnRunState, TurnScope, TurnStatus,
};

#[derive(Default)]
struct FakeReadModel {
    gates: Mutex<Vec<PendingApprovalGateRecord>>,
}

impl FakeReadModel {
    fn with_gate(gate: PendingApprovalGateRecord) -> Self {
        Self {
            gates: Mutex::new(vec![gate]),
        }
    }
}

#[async_trait]
impl ApprovalInteractionReadModel for FakeReadModel {
    async fn pending_approvals(
        &self,
        scope: &ApprovalInteractionScope,
    ) -> Result<Vec<PendingApprovalGateRecord>, ironclaw_product_workflow::ProductWorkflowError>
    {
        Ok(self
            .gates
            .lock()
            .expect("lock")
            .iter()
            .filter(|gate| gate.scope() == scope)
            .cloned()
            .collect())
    }
}

#[derive(Default)]
struct FixedLeaseTermsProvider;

#[async_trait]
impl ApprovalLeaseTermsProvider for FixedLeaseTermsProvider {
    async fn lease_terms_for(
        &self,
        _gate: &PendingApprovalGateRecord,
    ) -> Result<LeaseApproval, ironclaw_product_workflow::ProductWorkflowError> {
        Ok(LeaseApproval {
            issued_by: Principal::HostRuntime,
            allowed_effects: vec![EffectKind::DispatchCapability],
            mounts: MountView::default(),
            network: NetworkPolicy::default(),
            secrets: vec![],
            resource_ceiling: None,
            expires_at: None,
            max_invocations: Some(1),
        })
    }
}

#[derive(Default)]
struct RecordingApprovalResolver {
    approvals: Mutex<Vec<RecordedApproval>>,
    denials: Mutex<Vec<(ResourceScope, ApprovalRequestId, Principal)>>,
}

#[derive(Clone)]
struct RecordedApproval {
    scope: ResourceScope,
    request_id: ApprovalRequestId,
    issued_by: Principal,
    allowed_effects: Vec<EffectKind>,
}

impl RecordingApprovalResolver {
    fn approval_count(&self) -> usize {
        self.approvals.lock().expect("lock").len()
    }

    fn denial_count(&self) -> usize {
        self.denials.lock().expect("lock").len()
    }

    fn approvals(&self) -> Vec<RecordedApproval> {
        self.approvals.lock().expect("lock").clone()
    }
}

#[async_trait]
impl ApprovalResolutionPort for RecordingApprovalResolver {
    async fn approve_dispatch(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
        approval: LeaseApproval,
    ) -> Result<(), ironclaw_product_workflow::ProductWorkflowError> {
        self.approvals.lock().expect("lock").push(RecordedApproval {
            scope: scope.clone(),
            request_id,
            issued_by: approval.issued_by,
            allowed_effects: approval.allowed_effects,
        });
        Ok(())
    }

    async fn approve_spawn(
        &self,
        _scope: &ResourceScope,
        _request_id: ApprovalRequestId,
        _approval: LeaseApproval,
    ) -> Result<(), ironclaw_product_workflow::ProductWorkflowError> {
        panic!("dispatch test should not approve spawn")
    }

    async fn deny(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
        denial: DenyApproval,
    ) -> Result<(), ironclaw_product_workflow::ProductWorkflowError> {
        self.denials
            .lock()
            .expect("lock")
            .push((scope.clone(), request_id, denial.denied_by));
        Ok(())
    }
}

struct FakeTurnCoordinator {
    actor: TurnActor,
    status: Mutex<TurnStatus>,
    gate_ref: Mutex<Option<GateRef>>,
    resumptions: Mutex<Vec<ResumeTurnRequest>>,
    cancellations: Mutex<Vec<CancelRunRequest>>,
}

impl FakeTurnCoordinator {
    fn blocked(actor: TurnActor, gate_ref: GateRef) -> Self {
        Self {
            actor,
            status: Mutex::new(TurnStatus::BlockedApproval),
            gate_ref: Mutex::new(Some(gate_ref)),
            resumptions: Mutex::new(Vec::new()),
            cancellations: Mutex::new(Vec::new()),
        }
    }

    fn set_status(&self, status: TurnStatus) {
        *self.status.lock().expect("lock") = status;
    }

    fn resumption_count(&self) -> usize {
        self.resumptions.lock().expect("lock").len()
    }

    fn cancellation_count(&self) -> usize {
        self.cancellations.lock().expect("lock").len()
    }

    fn last_resumption_precondition(&self) -> Option<ResumeTurnPrecondition> {
        self.resumptions
            .lock()
            .expect("lock")
            .last()
            .map(|request| request.precondition)
    }
}

#[async_trait]
impl TurnCoordinator for FakeTurnCoordinator {
    async fn submit_turn(
        &self,
        _request: SubmitTurnRequest,
    ) -> Result<SubmitTurnResponse, TurnError> {
        panic!("approval interactions must not submit a turn")
    }

    async fn resume_turn(
        &self,
        request: ResumeTurnRequest,
    ) -> Result<ResumeTurnResponse, TurnError> {
        let run_id = request.run_id;
        self.resumptions.lock().expect("lock").push(request);
        Ok(ResumeTurnResponse {
            run_id,
            status: TurnStatus::Queued,
            event_cursor: EventCursor(11),
        })
    }

    async fn cancel_run(&self, request: CancelRunRequest) -> Result<CancelRunResponse, TurnError> {
        let run_id = request.run_id;
        self.cancellations.lock().expect("lock").push(request);
        Ok(CancelRunResponse {
            run_id,
            status: TurnStatus::Cancelled,
            event_cursor: EventCursor(13),
            already_terminal: false,
        })
    }

    async fn get_run_state(&self, request: GetRunStateRequest) -> Result<TurnRunState, TurnError> {
        Ok(TurnRunState {
            scope: request.scope,
            actor: Some(self.actor.clone()),
            turn_id: TurnId::new(),
            run_id: request.run_id,
            status: *self.status.lock().expect("lock"),
            accepted_message_ref: AcceptedMessageRef::new("msg:approval").expect("valid"),
            source_binding_ref: SourceBindingRef::new("src:approval").expect("valid"),
            reply_target_binding_ref: ReplyTargetBindingRef::new("reply:approval").expect("valid"),
            resolved_run_profile_id: RunProfileId::default_profile(),
            resolved_run_profile_version: RunProfileVersion::new(1),
            resolved_model_route: None,
            received_at: Utc::now(),
            checkpoint_id: None,
            gate_ref: self.gate_ref.lock().expect("lock").clone(),
            failure: None,
            event_cursor: EventCursor(17),
        })
    }
}

fn scope() -> TurnScope {
    TurnScope::new(
        TenantId::new("tenant-alpha").expect("tenant"),
        None,
        None,
        ThreadId::new("thread-alpha").expect("thread"),
    )
}

fn actor(user: &str) -> TurnActor {
    TurnActor::new(UserId::new(user).expect("user"))
}

fn resource_scope(actor: &TurnActor) -> ResourceScope {
    ResourceScope {
        tenant_id: TenantId::new("tenant-alpha").expect("tenant"),
        user_id: actor.user_id.clone(),
        agent_id: None,
        project_id: None,
        mission_id: None,
        thread_id: Some(ThreadId::new("thread-alpha").expect("thread")),
        invocation_id: InvocationId::new(),
    }
}

fn approval_request(reason: &str) -> ApprovalRequest {
    ApprovalRequest {
        id: ApprovalRequestId::new(),
        correlation_id: CorrelationId::new(),
        requested_by: Principal::User(UserId::new("user-alpha").expect("user")),
        action: Box::new(Action::Dispatch {
            capability: CapabilityId::new("demo.echo").expect("capability"),
            estimated_resources: ResourceEstimate::default(),
        }),
        invocation_fingerprint: None,
        reason: reason.to_string(),
        reusable_scope: None,
    }
}

fn run_start(scope: ResourceScope, capability_id: CapabilityId) -> RunStart {
    RunStart {
        invocation_id: scope.invocation_id,
        capability_id,
        scope,
    }
}

fn service_fixture(
    reason: &str,
) -> (
    DefaultApprovalInteractionService,
    Arc<RecordingApprovalResolver>,
    Arc<FakeTurnCoordinator>,
    TurnRunId,
    GateRef,
) {
    let actor = actor("user-alpha");
    let request = approval_request(reason);
    let gate_ref = approval_gate_ref(request.id).expect("gate ref");
    let run_id = TurnRunId::new();
    let gate =
        PendingApprovalGateRecord::new(resource_scope(&actor), run_id, gate_ref.clone(), request)
            .expect("pending gate");
    let resolver = Arc::new(RecordingApprovalResolver::default());
    let coordinator = Arc::new(FakeTurnCoordinator::blocked(actor, gate_ref.clone()));
    let service = DefaultApprovalInteractionService::new(
        Arc::new(FakeReadModel::with_gate(gate)),
        Arc::new(FixedLeaseTermsProvider),
        resolver.clone(),
        coordinator.clone(),
    );
    (service, resolver, coordinator, run_id, gate_ref)
}

#[tokio::test]
async fn approve_resolves_pending_gate_then_resumes_blocked_approval() {
    let (service, resolver, coordinator, run_id, gate_ref) = service_fixture("send the email");
    let response = service
        .resolve(ResolveApprovalInteractionRequest {
            scope: scope(),
            actor: actor("user-alpha"),
            run_id,
            gate_ref: gate_ref.clone(),
            decision: ApprovalInteractionDecision::ApproveOnce,
            idempotency_key: IdempotencyKey::new("approve-once").expect("idempotency"),
        })
        .await
        .expect("approve");

    assert!(matches!(
        response,
        ResolveApprovalInteractionResponse::Approved(_)
    ));
    assert_eq!(resolver.approval_count(), 1);
    assert_eq!(resolver.denial_count(), 0);
    let approvals = resolver.approvals();
    assert_eq!(approvals[0].scope.user_id, actor("user-alpha").user_id);
    assert_eq!(
        approvals[0].issued_by,
        Principal::User(actor("user-alpha").user_id)
    );
    assert_eq!(
        approvals[0].allowed_effects,
        vec![EffectKind::DispatchCapability]
    );
    assert_eq!(
        approval_gate_ref(approvals[0].request_id).expect("approval gate"),
        gate_ref
    );
    assert_eq!(coordinator.resumption_count(), 1);
    assert_eq!(
        coordinator.last_resumption_precondition(),
        Some(ResumeTurnPrecondition::BlockedApprovalGate)
    );
}

#[tokio::test]
async fn deny_marks_pending_gate_denied_then_cancels_run() {
    let (service, resolver, coordinator, run_id, gate_ref) = service_fixture("delete a file");
    let response = service
        .resolve(ResolveApprovalInteractionRequest {
            scope: scope(),
            actor: actor("user-alpha"),
            run_id,
            gate_ref,
            decision: ApprovalInteractionDecision::Deny,
            idempotency_key: IdempotencyKey::new("deny-once").expect("idempotency"),
        })
        .await
        .expect("deny");

    assert!(matches!(
        response,
        ResolveApprovalInteractionResponse::Denied(_)
    ));
    assert_eq!(resolver.approval_count(), 0);
    assert_eq!(resolver.denial_count(), 1);
    assert_eq!(coordinator.cancellation_count(), 1);
}

#[tokio::test]
async fn missing_gate_returns_deterministic_not_found_without_resolution() {
    let (_, resolver, coordinator, run_id, gate_ref) = service_fixture("send the email");
    let service = DefaultApprovalInteractionService::new(
        Arc::new(FakeReadModel::default()),
        Arc::new(FixedLeaseTermsProvider),
        resolver.clone(),
        coordinator,
    );

    let err = service
        .resolve(ResolveApprovalInteractionRequest {
            scope: scope(),
            actor: actor("user-alpha"),
            run_id,
            gate_ref,
            decision: ApprovalInteractionDecision::ApproveOnce,
            idempotency_key: IdempotencyKey::new("missing").expect("idempotency"),
        })
        .await
        .expect_err("missing gate");

    assert!(matches!(
        err,
        ironclaw_product_workflow::ProductWorkflowError::ApprovalInteractionRejected {
            kind: ApprovalInteractionRejectionKind::MissingGate
        }
    ));
    assert_eq!(resolver.approval_count(), 0);
}

#[tokio::test]
async fn stale_gate_returns_conflict_without_resolution() {
    let (service, resolver, coordinator, run_id, gate_ref) = service_fixture("send the email");
    coordinator.set_status(TurnStatus::Queued);

    let err = service
        .resolve(ResolveApprovalInteractionRequest {
            scope: scope(),
            actor: actor("user-alpha"),
            run_id,
            gate_ref,
            decision: ApprovalInteractionDecision::ApproveOnce,
            idempotency_key: IdempotencyKey::new("stale").expect("idempotency"),
        })
        .await
        .expect_err("stale gate");

    assert!(matches!(
        err,
        ironclaw_product_workflow::ProductWorkflowError::ApprovalInteractionRejected {
            kind: ApprovalInteractionRejectionKind::StaleGate
        }
    ));
    assert_eq!(resolver.approval_count(), 0);
}

#[tokio::test]
async fn cross_scope_actor_is_rejected_before_resolution() {
    let (service, resolver, _, run_id, gate_ref) = service_fixture("send the email");

    let err = service
        .resolve(ResolveApprovalInteractionRequest {
            scope: scope(),
            actor: actor("user-beta"),
            run_id,
            gate_ref,
            decision: ApprovalInteractionDecision::ApproveOnce,
            idempotency_key: IdempotencyKey::new("cross-scope").expect("idempotency"),
        })
        .await
        .expect_err("cross-scope actor");

    assert!(matches!(
        err,
        ironclaw_product_workflow::ProductWorkflowError::ApprovalInteractionRejected {
            kind: ApprovalInteractionRejectionKind::CrossScopeDenied
        }
    ));
    assert_eq!(resolver.approval_count(), 0);
}

#[tokio::test]
async fn list_pending_returns_redacted_dto_without_no_exposure_sentinels() {
    let (service, _, _, _, _) = service_fixture("RAW_PROMPT_SENTINEL sk-live /Users/alice/private");
    let response = service
        .list_pending(ListPendingApprovalsRequest {
            scope: scope(),
            actor: actor("user-alpha"),
        })
        .await
        .expect("list pending");
    let serialized = serde_json::to_string(&response).expect("serialize");

    assert_eq!(response.approvals.len(), 1);
    assert_eq!(response.approvals[0].summary, "Approval required");
    for forbidden in ["RAW_PROMPT_SENTINEL", "sk-live", "/Users/alice/private"] {
        assert!(
            !serialized.contains(forbidden),
            "approval DTO leaked {forbidden}"
        );
    }
}

#[tokio::test]
async fn list_pending_uses_loop_safe_summary_boundary_for_display_reasons() {
    for unsafe_reason in [
        "/etc/passwd",
        "password: hunter2",
        "raw tool_input includes private arguments",
    ] {
        let (service, _, _, _, _) = service_fixture(unsafe_reason);
        let response = service
            .list_pending(ListPendingApprovalsRequest {
                scope: scope(),
                actor: actor("user-alpha"),
            })
            .await
            .expect("list pending");

        assert_eq!(response.approvals[0].summary, "Approval required");
    }
}

#[tokio::test]
async fn approval_resolver_port_preserves_audit_sink() {
    let alpha_actor = actor("user-alpha");
    let resource_scope = resource_scope(&alpha_actor);
    let request = approval_request("approval required");
    let request_id = request.id;
    let approvals = Arc::new(InMemoryApprovalRequestStore::new());
    approvals
        .save_pending(resource_scope.clone(), request)
        .await
        .expect("save approval");
    let leases = Arc::new(InMemoryCapabilityLeaseStore::new());
    let audit = Arc::new(InMemoryAuditSink::new());
    let resolver = ApprovalResolverPort::new(approvals, leases).with_audit_sink(audit.clone());

    resolver
        .deny(
            &resource_scope,
            request_id,
            DenyApproval {
                denied_by: Principal::User(alpha_actor.user_id),
            },
        )
        .await
        .expect("deny approval");

    assert_eq!(audit.records().len(), 1);
}

#[tokio::test]
async fn run_state_read_model_lists_canonical_pending_blocked_approvals() {
    let alpha_actor = actor("user-alpha");
    let resource_scope = resource_scope(&alpha_actor);
    let request = approval_request("send the email");
    let capability_id = match request.action.as_ref() {
        Action::Dispatch { capability, .. } => capability.clone(),
        _ => panic!("test request should be dispatch"),
    };
    let run_id = TurnRunId::from_uuid(resource_scope.invocation_id.as_uuid());
    let run_state = Arc::new(InMemoryRunStateStore::new());
    let approvals = Arc::new(InMemoryApprovalRequestStore::new());
    run_state
        .start(run_start(resource_scope.clone(), capability_id))
        .await
        .expect("start run");
    approvals
        .save_pending(resource_scope.clone(), request.clone())
        .await
        .expect("save approval");
    run_state
        .block_approval(
            &resource_scope,
            resource_scope.invocation_id,
            request.clone(),
        )
        .await
        .expect("block approval");
    let read_model = RunStateApprovalInteractionReadModel::new(run_state, approvals);

    let pending = read_model
        .pending_approvals(&ApprovalInteractionScope::from_turn(&scope(), &alpha_actor))
        .await
        .expect("pending approvals");

    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].run_id(), run_id);
    assert_eq!(
        pending[0].gate_ref(),
        &approval_gate_ref(request.id).expect("approval gate")
    );
    assert_eq!(pending[0].request().id, request.id);

    let other_user_pending = read_model
        .pending_approvals(&ApprovalInteractionScope::from_turn(
            &scope(),
            &actor("user-beta"),
        ))
        .await
        .expect("other user pending approvals");
    assert!(other_user_pending.is_empty());
}
