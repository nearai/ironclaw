use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_approvals::{
    DenyApproval, InMemoryPersistentApprovalPolicyStore, LeaseApproval, PersistentApprovalAction,
    PersistentApprovalPolicy, PersistentApprovalPolicyError, PersistentApprovalPolicyInput,
    PersistentApprovalPolicyKey, PersistentApprovalPolicyStore,
};
use ironclaw_authorization::{
    CapabilityLeaseStatus, CapabilityLeaseStore, InMemoryCapabilityLeaseStore,
};
use ironclaw_events::InMemoryAuditSink;
use ironclaw_host_api::{
    Action, ApprovalRequest, ApprovalRequestId, CapabilityId, CorrelationId, EffectKind,
    InvocationFingerprint, InvocationId, MountView, NetworkPolicy, Principal, ResourceEstimate,
    ResourceScope, TenantId, ThreadId, UserId,
};
use ironclaw_product_workflow::{
    ApprovalBlockedTurnRun, ApprovalGateRecord, ApprovalInteractionDecision,
    ApprovalInteractionReadModel, ApprovalInteractionRejectionKind, ApprovalInteractionScope,
    ApprovalInteractionService, ApprovalLeaseTermsProvider, ApprovalResolutionPort,
    ApprovalResolverPort, ApprovalTurnRunLocator, DefaultApprovalInteractionService,
    ListPendingApprovalsRequest, ResolveApprovalInteractionRequest,
    ResolveApprovalInteractionResponse, RunStateApprovalInteractionReadModel, approval_gate_ref,
};
use ironclaw_run_state::{ApprovalRequestStore, ApprovalStatus, InMemoryApprovalRequestStore};
use ironclaw_turns::{
    AcceptedMessageRef, CancelRunRequest, CancelRunResponse, EventCursor, GateRef,
    GetRunStateRequest, IdempotencyKey, ReplyTargetBindingRef, ResumeTurnPrecondition,
    ResumeTurnRequest, ResumeTurnResponse, RunProfileId, RunProfileVersion, SourceBindingRef,
    SubmitTurnRequest, SubmitTurnResponse, TurnActor, TurnCoordinator, TurnError, TurnId,
    TurnRunId, TurnRunState, TurnScope, TurnStatus,
};

#[derive(Default)]
struct FakeReadModel {
    gates: Mutex<Vec<ApprovalGateRecord>>,
}

impl FakeReadModel {
    fn with_gate(gate: ApprovalGateRecord) -> Self {
        Self {
            gates: Mutex::new(vec![gate]),
        }
    }

    fn with_gates(gates: Vec<ApprovalGateRecord>) -> Self {
        Self {
            gates: Mutex::new(gates),
        }
    }
}

#[derive(Default)]
struct FakeTurnRunLocator {
    runs: Mutex<Vec<ApprovalBlockedTurnRun>>,
    historical_runs: Mutex<Vec<ApprovalBlockedTurnRun>>,
}

impl FakeTurnRunLocator {
    fn with_run(run_id: TurnRunId, gate_ref: GateRef) -> Self {
        Self {
            runs: Mutex::new(vec![ApprovalBlockedTurnRun { run_id, gate_ref }]),
            historical_runs: Mutex::new(Vec::new()),
        }
    }

    fn with_historical_run(run_id: TurnRunId, gate_ref: GateRef) -> Self {
        Self {
            runs: Mutex::new(Vec::new()),
            historical_runs: Mutex::new(vec![ApprovalBlockedTurnRun { run_id, gate_ref }]),
        }
    }
}

#[async_trait]
impl ApprovalTurnRunLocator for FakeTurnRunLocator {
    async fn blocked_approval_runs(
        &self,
        _scope: &ApprovalInteractionScope,
    ) -> Result<Vec<ApprovalBlockedTurnRun>, ironclaw_product_workflow::ProductWorkflowError> {
        Ok(self.runs.lock().expect("lock").clone())
    }

    async fn approval_run_for_gate(
        &self,
        _scope: &ApprovalInteractionScope,
        gate_ref: &GateRef,
    ) -> Result<Option<TurnRunId>, ironclaw_product_workflow::ProductWorkflowError> {
        Ok(self
            .runs
            .lock()
            .expect("lock")
            .iter()
            .chain(self.historical_runs.lock().expect("lock").iter())
            .find(|run| &run.gate_ref == gate_ref)
            .map(|run| run.run_id))
    }
}

#[async_trait]
impl ApprovalInteractionReadModel for FakeReadModel {
    async fn approval_gates(
        &self,
        scope: &ApprovalInteractionScope,
    ) -> Result<Vec<ApprovalGateRecord>, ironclaw_product_workflow::ProductWorkflowError> {
        Ok(self
            .gates
            .lock()
            .expect("lock")
            .iter()
            .filter(|gate| gate.scope() == scope)
            .cloned()
            .collect())
    }

    async fn approval_gate(
        &self,
        scope: &ApprovalInteractionScope,
        run_id_hint: Option<TurnRunId>,
        gate_ref: &GateRef,
    ) -> Result<Option<ApprovalGateRecord>, ironclaw_product_workflow::ProductWorkflowError> {
        Ok(self
            .gates
            .lock()
            .expect("lock")
            .iter()
            .find(|gate| {
                gate.scope() == scope
                    && gate.gate_ref() == gate_ref
                    && run_id_hint.is_none_or(|run_id| gate.run_id() == run_id)
            })
            .cloned())
    }
}

#[derive(Default)]
struct FixedLeaseTermsProvider;

#[async_trait]
impl ApprovalLeaseTermsProvider for FixedLeaseTermsProvider {
    async fn lease_terms_for(
        &self,
        _gate: &ApprovalGateRecord,
    ) -> Result<LeaseApproval, ironclaw_product_workflow::ProductWorkflowError> {
        Ok(dispatch_lease_approval(Principal::HostRuntime))
    }

    async fn persistent_approval_allowed(
        &self,
        _gate: &ApprovalGateRecord,
    ) -> Result<(), ironclaw_product_workflow::ProductWorkflowError> {
        Ok(())
    }
}

#[derive(Default)]
struct RejectingPersistentLeaseTermsProvider;

#[async_trait]
impl ApprovalLeaseTermsProvider for RejectingPersistentLeaseTermsProvider {
    async fn lease_terms_for(
        &self,
        _gate: &ApprovalGateRecord,
    ) -> Result<LeaseApproval, ironclaw_product_workflow::ProductWorkflowError> {
        Ok(dispatch_lease_approval(Principal::HostRuntime))
    }

    async fn persistent_approval_allowed(
        &self,
        _gate: &ApprovalGateRecord,
    ) -> Result<(), ironclaw_product_workflow::ProductWorkflowError> {
        Err(
            ironclaw_product_workflow::ProductWorkflowError::ApprovalInteractionRejected {
                kind: ApprovalInteractionRejectionKind::AlwaysAllowUnsupported,
            },
        )
    }
}

fn dispatch_lease_approval(issued_by: Principal) -> LeaseApproval {
    LeaseApproval {
        issued_by,
        allowed_effects: vec![EffectKind::DispatchCapability],
        mounts: MountView::default(),
        network: NetworkPolicy::default(),
        secrets: vec![],
        resource_ceiling: None,
        expires_at: None,
        max_invocations: Some(1),
    }
}

#[derive(Default)]
struct RecordingApprovalResolver {
    approvals: Mutex<Vec<RecordedApproval>>,
    spawn_approvals: Mutex<Vec<RecordedApproval>>,
    dispatch_lease_retries: Mutex<Vec<RecordedApproval>>,
    spawn_lease_retries: Mutex<Vec<RecordedApproval>>,
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

    fn spawn_approval_count(&self) -> usize {
        self.spawn_approvals.lock().expect("lock").len()
    }

    fn dispatch_lease_retry_count(&self) -> usize {
        self.dispatch_lease_retries.lock().expect("lock").len()
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
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
        approval: LeaseApproval,
    ) -> Result<(), ironclaw_product_workflow::ProductWorkflowError> {
        self.spawn_approvals
            .lock()
            .expect("lock")
            .push(RecordedApproval {
                scope: scope.clone(),
                request_id,
                issued_by: approval.issued_by,
                allowed_effects: approval.allowed_effects,
            });
        Ok(())
    }

    async fn ensure_dispatch_lease(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
        approval: LeaseApproval,
    ) -> Result<(), ironclaw_product_workflow::ProductWorkflowError> {
        self.dispatch_lease_retries
            .lock()
            .expect("lock")
            .push(RecordedApproval {
                scope: scope.clone(),
                request_id,
                issued_by: approval.issued_by,
                allowed_effects: approval.allowed_effects,
            });
        Ok(())
    }

    async fn ensure_spawn_lease(
        &self,
        scope: &ResourceScope,
        request_id: ApprovalRequestId,
        approval: LeaseApproval,
    ) -> Result<(), ironclaw_product_workflow::ProductWorkflowError> {
        self.spawn_lease_retries
            .lock()
            .expect("lock")
            .push(RecordedApproval {
                scope: scope.clone(),
                request_id,
                issued_by: approval.issued_by,
                allowed_effects: approval.allowed_effects,
            });
        Ok(())
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

struct FailingApprovalResolver;

#[async_trait]
impl ApprovalResolutionPort for FailingApprovalResolver {
    async fn approve_dispatch(
        &self,
        _scope: &ResourceScope,
        _request_id: ApprovalRequestId,
        _approval: LeaseApproval,
    ) -> Result<(), ironclaw_product_workflow::ProductWorkflowError> {
        Err(resolver_failure())
    }

    async fn approve_spawn(
        &self,
        _scope: &ResourceScope,
        _request_id: ApprovalRequestId,
        _approval: LeaseApproval,
    ) -> Result<(), ironclaw_product_workflow::ProductWorkflowError> {
        Err(resolver_failure())
    }

    async fn ensure_dispatch_lease(
        &self,
        _scope: &ResourceScope,
        _request_id: ApprovalRequestId,
        _approval: LeaseApproval,
    ) -> Result<(), ironclaw_product_workflow::ProductWorkflowError> {
        Err(resolver_failure())
    }

    async fn ensure_spawn_lease(
        &self,
        _scope: &ResourceScope,
        _request_id: ApprovalRequestId,
        _approval: LeaseApproval,
    ) -> Result<(), ironclaw_product_workflow::ProductWorkflowError> {
        Err(resolver_failure())
    }

    async fn deny(
        &self,
        _scope: &ResourceScope,
        _request_id: ApprovalRequestId,
        _denial: DenyApproval,
    ) -> Result<(), ironclaw_product_workflow::ProductWorkflowError> {
        Err(resolver_failure())
    }
}

struct FailingPersistentApprovalPolicyStore;

#[async_trait]
impl PersistentApprovalPolicyStore for FailingPersistentApprovalPolicyStore {
    async fn allow(
        &self,
        _input: PersistentApprovalPolicyInput,
    ) -> Result<PersistentApprovalPolicy, PersistentApprovalPolicyError> {
        Err(PersistentApprovalPolicyError::Filesystem(
            "policy store unavailable".to_string(),
        ))
    }

    async fn lookup(
        &self,
        _key: &PersistentApprovalPolicyKey,
    ) -> Result<Option<PersistentApprovalPolicy>, PersistentApprovalPolicyError> {
        Ok(None)
    }

    async fn revoke(
        &self,
        _key: &PersistentApprovalPolicyKey,
    ) -> Result<PersistentApprovalPolicy, PersistentApprovalPolicyError> {
        Err(PersistentApprovalPolicyError::UnknownPolicy)
    }

    async fn revoke_if_source_approval_request(
        &self,
        _key: &PersistentApprovalPolicyKey,
        _source_approval_request_id: ApprovalRequestId,
    ) -> Result<Option<PersistentApprovalPolicy>, PersistentApprovalPolicyError> {
        Ok(None)
    }
}

fn resolver_failure() -> ironclaw_product_workflow::ProductWorkflowError {
    ironclaw_product_workflow::ProductWorkflowError::Transient {
        reason: "approval resolver unavailable".to_string(),
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

    fn last_resumption_run_id(&self) -> Option<TurnRunId> {
        self.resumptions
            .lock()
            .expect("lock")
            .last()
            .map(|request| request.run_id)
    }
}

#[async_trait]
impl TurnCoordinator for FakeTurnCoordinator {
    async fn prepare_turn(&self, _scope: TurnScope) -> Result<TurnRunId, TurnError> {
        Ok(TurnRunId::new())
    }

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
            actor: None,
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
            credential_requirements: Vec::new(),
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

fn spawn_approval_request(reason: &str) -> ApprovalRequest {
    let mut request = approval_request(reason);
    request.action = Box::new(Action::SpawnCapability {
        capability: CapabilityId::new("demo.worker").expect("capability"),
        estimated_resources: ResourceEstimate::default(),
    });
    request
}

fn unsupported_approval_request(reason: &str) -> ApprovalRequest {
    let mut request = approval_request(reason);
    request.action = Box::new(Action::EmitExternalEffect {
        effect: EffectKind::ExternalWrite,
    });
    request
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
    service_fixture_for_request(approval_request(reason))
}

fn service_fixture_for_request(
    request: ApprovalRequest,
) -> (
    DefaultApprovalInteractionService,
    Arc<RecordingApprovalResolver>,
    Arc<FakeTurnCoordinator>,
    TurnRunId,
    GateRef,
) {
    service_fixture_for_request_status(request, ApprovalStatus::Pending)
}

fn service_fixture_for_request_status(
    request: ApprovalRequest,
    status: ApprovalStatus,
) -> (
    DefaultApprovalInteractionService,
    Arc<RecordingApprovalResolver>,
    Arc<FakeTurnCoordinator>,
    TurnRunId,
    GateRef,
) {
    let actor = actor("user-alpha");
    let gate_ref = approval_gate_ref(request.id).expect("gate ref");
    let run_id = TurnRunId::new();
    let gate = ApprovalGateRecord::with_status(
        resource_scope(&actor),
        run_id,
        gate_ref.clone(),
        request,
        status,
    )
    .expect("approval gate");
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
            run_id_hint: Some(run_id),
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
async fn always_allow_resolves_gate_and_persists_reusable_policy() {
    let request = approval_request("send the email");
    let request_id = request.id;
    let capability = match request.action.as_ref() {
        Action::Dispatch { capability, .. } => capability.clone(),
        _ => panic!("test request should be dispatch"),
    };
    let policy_scope = resource_scope(&actor("user-alpha"));
    let key = PersistentApprovalPolicyKey::new(
        &policy_scope,
        PersistentApprovalAction::Dispatch,
        capability,
        Principal::User(UserId::new("user-alpha").expect("user")),
    )
    .expect("persistent policy key");
    let (service, resolver, coordinator, run_id, gate_ref) = service_fixture_for_request(request);
    let policies = Arc::new(InMemoryPersistentApprovalPolicyStore::new());
    let policy_store: Arc<dyn PersistentApprovalPolicyStore> = policies.clone();
    let service = service.with_persistent_policy_store(policy_store);

    let response = service
        .resolve(ResolveApprovalInteractionRequest {
            scope: scope(),
            actor: actor("user-alpha"),
            run_id_hint: Some(run_id),
            gate_ref,
            decision: ApprovalInteractionDecision::AlwaysAllow,
            idempotency_key: IdempotencyKey::new("approve-always").expect("idempotency"),
        })
        .await
        .expect("always allow");

    assert!(matches!(
        response,
        ResolveApprovalInteractionResponse::Approved(_)
    ));
    assert_eq!(resolver.approval_count(), 1);
    assert_eq!(coordinator.resumption_count(), 1);
    let policy = policies
        .lookup(&key)
        .await
        .expect("persistent policy lookup")
        .expect("persistent policy");
    assert_eq!(policy.source_approval_request_id, Some(request_id));
    assert_eq!(policy.constraints.max_invocations, None);
    assert_eq!(
        policy.constraints.allowed_effects,
        vec![EffectKind::DispatchCapability]
    );
    assert!(policy.active_grant().is_some());
}

#[tokio::test]
async fn always_allow_without_policy_store_rejects_before_approval_side_effects() {
    let (service, resolver, coordinator, run_id, gate_ref) = service_fixture("send the email");

    let err = service
        .resolve(ResolveApprovalInteractionRequest {
            scope: scope(),
            actor: actor("user-alpha"),
            run_id_hint: Some(run_id),
            gate_ref,
            decision: ApprovalInteractionDecision::AlwaysAllow,
            idempotency_key: IdempotencyKey::new("approve-always-no-store").expect("idempotency"),
        })
        .await
        .expect_err("always allow without store");

    assert!(matches!(
        err,
        ironclaw_product_workflow::ProductWorkflowError::ApprovalInteractionRejected {
            kind: ApprovalInteractionRejectionKind::AlwaysAllowUnsupported
        }
    ));
    assert_eq!(resolver.approval_count(), 0);
    assert_eq!(resolver.spawn_approval_count(), 0);
    assert_eq!(coordinator.resumption_count(), 0);
}

#[tokio::test]
async fn always_allow_policy_write_failure_still_returns_approved_after_resume() {
    let (service, resolver, coordinator, run_id, gate_ref) = service_fixture("send the email");
    let policy_store: Arc<dyn PersistentApprovalPolicyStore> =
        Arc::new(FailingPersistentApprovalPolicyStore);
    let service = service.with_persistent_policy_store(policy_store);

    let response = service
        .resolve(ResolveApprovalInteractionRequest {
            scope: scope(),
            actor: actor("user-alpha"),
            run_id_hint: Some(run_id),
            gate_ref,
            decision: ApprovalInteractionDecision::AlwaysAllow,
            idempotency_key: IdempotencyKey::new("approve-always-store-fails")
                .expect("idempotency"),
        })
        .await
        .expect("always allow best-effort persistence");

    assert!(matches!(
        response,
        ResolveApprovalInteractionResponse::Approved(_)
    ));
    assert_eq!(resolver.approval_count(), 1);
    assert_eq!(coordinator.resumption_count(), 1);
}

#[tokio::test]
async fn always_allow_disallowed_by_policy_rejects_without_persisting_or_approving() {
    let request = approval_request("send the email");
    let capability = match request.action.as_ref() {
        Action::Dispatch { capability, .. } => capability.clone(),
        _ => panic!("test request should be dispatch"),
    };
    let policy_scope = resource_scope(&actor("user-alpha"));
    let key = PersistentApprovalPolicyKey::new(
        &policy_scope,
        PersistentApprovalAction::Dispatch,
        capability,
        Principal::User(UserId::new("user-alpha").expect("user")),
    )
    .expect("persistent policy key");
    let actor = actor("user-alpha");
    let gate_ref = approval_gate_ref(request.id).expect("gate ref");
    let run_id = TurnRunId::new();
    let gate = ApprovalGateRecord::with_status(
        resource_scope(&actor),
        run_id,
        gate_ref.clone(),
        request,
        ApprovalStatus::Pending,
    )
    .expect("approval gate");
    let resolver = Arc::new(RecordingApprovalResolver::default());
    let coordinator = Arc::new(FakeTurnCoordinator::blocked(
        actor.clone(),
        gate_ref.clone(),
    ));
    let policies = Arc::new(InMemoryPersistentApprovalPolicyStore::new());
    let policy_store: Arc<dyn PersistentApprovalPolicyStore> = policies.clone();
    let service = DefaultApprovalInteractionService::new(
        Arc::new(FakeReadModel::with_gate(gate)),
        Arc::new(RejectingPersistentLeaseTermsProvider),
        resolver.clone(),
        coordinator.clone(),
    )
    .with_persistent_policy_store(policy_store);

    let err = service
        .resolve(ResolveApprovalInteractionRequest {
            scope: scope(),
            actor,
            run_id_hint: Some(run_id),
            gate_ref,
            decision: ApprovalInteractionDecision::AlwaysAllow,
            idempotency_key: IdempotencyKey::new("approve-always-disallowed").expect("idempotency"),
        })
        .await
        .expect_err("always allow disallowed by policy");

    assert!(matches!(
        err,
        ironclaw_product_workflow::ProductWorkflowError::ApprovalInteractionRejected {
            kind: ApprovalInteractionRejectionKind::AlwaysAllowUnsupported
        }
    ));
    assert_eq!(resolver.approval_count(), 0);
    assert_eq!(resolver.spawn_approval_count(), 0);
    assert_eq!(coordinator.resumption_count(), 0);
    assert!(
        policies
            .lookup(&key)
            .await
            .expect("persistent policy lookup")
            .is_none()
    );
}

#[tokio::test]
async fn always_allow_does_not_persist_policy_when_resolution_fails() {
    let request = approval_request("send the email");
    let capability = match request.action.as_ref() {
        Action::Dispatch { capability, .. } => capability.clone(),
        _ => panic!("test request should be dispatch"),
    };
    let actor = actor("user-alpha");
    let policy_scope = resource_scope(&actor);
    let key = PersistentApprovalPolicyKey::new(
        &policy_scope,
        PersistentApprovalAction::Dispatch,
        capability,
        Principal::User(UserId::new("user-alpha").expect("user")),
    )
    .expect("persistent policy key");
    let gate_ref = approval_gate_ref(request.id).expect("gate ref");
    let run_id = TurnRunId::new();
    let gate = ApprovalGateRecord::with_status(
        policy_scope,
        run_id,
        gate_ref.clone(),
        request,
        ApprovalStatus::Pending,
    )
    .expect("approval gate");
    let coordinator = Arc::new(FakeTurnCoordinator::blocked(
        actor.clone(),
        gate_ref.clone(),
    ));
    let policies = Arc::new(InMemoryPersistentApprovalPolicyStore::new());
    let policy_store: Arc<dyn PersistentApprovalPolicyStore> = policies.clone();
    let service = DefaultApprovalInteractionService::new(
        Arc::new(FakeReadModel::with_gate(gate)),
        Arc::new(FixedLeaseTermsProvider),
        Arc::new(FailingApprovalResolver),
        coordinator.clone(),
    )
    .with_persistent_policy_store(policy_store);

    let err = service
        .resolve(ResolveApprovalInteractionRequest {
            scope: scope(),
            actor,
            run_id_hint: Some(run_id),
            gate_ref,
            decision: ApprovalInteractionDecision::AlwaysAllow,
            idempotency_key: IdempotencyKey::new("approve-always-resolution-fails")
                .expect("idempotency"),
        })
        .await
        .expect_err("resolver failure");

    assert!(matches!(
        err,
        ironclaw_product_workflow::ProductWorkflowError::Transient { .. }
    ));
    assert_eq!(coordinator.resumption_count(), 0);
    assert!(
        policies
            .lookup(&key)
            .await
            .expect("persistent policy lookup")
            .is_none()
    );
}

#[tokio::test]
async fn always_allow_resolution_failure_preserves_existing_policy() {
    let request = approval_request("send the email");
    let capability = match request.action.as_ref() {
        Action::Dispatch { capability, .. } => capability.clone(),
        _ => panic!("test request should be dispatch"),
    };
    let actor = actor("user-alpha");
    let policy_scope = resource_scope(&actor);
    let key = PersistentApprovalPolicyKey::new(
        &policy_scope,
        PersistentApprovalAction::Dispatch,
        capability,
        Principal::User(UserId::new("user-alpha").expect("user")),
    )
    .expect("persistent policy key");
    let gate_ref = approval_gate_ref(request.id).expect("gate ref");
    let run_id = TurnRunId::new();
    let gate = ApprovalGateRecord::with_status(
        policy_scope,
        run_id,
        gate_ref.clone(),
        request,
        ApprovalStatus::Pending,
    )
    .expect("approval gate");
    let coordinator = Arc::new(FakeTurnCoordinator::blocked(
        actor.clone(),
        gate_ref.clone(),
    ));
    let policies = Arc::new(InMemoryPersistentApprovalPolicyStore::new());
    let existing_source = ApprovalRequestId::new();
    policies
        .allow(PersistentApprovalPolicyInput {
            scope: resource_scope(&actor),
            action: PersistentApprovalAction::Dispatch,
            capability_id: CapabilityId::new("demo.echo").expect("capability"),
            grantee: Principal::User(UserId::new("user-alpha").expect("user")),
            approved_by: Principal::User(UserId::new("user-alpha").expect("user")),
            constraints: ironclaw_host_api::GrantConstraints {
                allowed_effects: vec![EffectKind::DispatchCapability],
                mounts: MountView::default(),
                network: NetworkPolicy::default(),
                secrets: Vec::new(),
                resource_ceiling: None,
                expires_at: None,
                max_invocations: None,
            },
            source_approval_request_id: Some(existing_source),
        })
        .await
        .expect("seed existing policy");
    let policy_store: Arc<dyn PersistentApprovalPolicyStore> = policies.clone();
    let service = DefaultApprovalInteractionService::new(
        Arc::new(FakeReadModel::with_gate(gate)),
        Arc::new(FixedLeaseTermsProvider),
        Arc::new(FailingApprovalResolver),
        coordinator.clone(),
    )
    .with_persistent_policy_store(policy_store);

    let err = service
        .resolve(ResolveApprovalInteractionRequest {
            scope: scope(),
            actor,
            run_id_hint: Some(run_id),
            gate_ref,
            decision: ApprovalInteractionDecision::AlwaysAllow,
            idempotency_key: IdempotencyKey::new("approve-always-newer-policy")
                .expect("idempotency"),
        })
        .await
        .expect_err("resolver failure");

    assert!(matches!(
        err,
        ironclaw_product_workflow::ProductWorkflowError::Transient { .. }
    ));
    assert_eq!(coordinator.resumption_count(), 0);
    let policy = policies
        .lookup(&key)
        .await
        .expect("persistent policy lookup")
        .expect("existing persistent policy");
    assert_eq!(policy.source_approval_request_id, Some(existing_source));
    assert!(policy.revoked_at.is_none());
    assert!(policy.active_grant().is_some());
}

#[tokio::test]
async fn approve_without_run_id_hint_uses_parked_turn_run_id() {
    let (service, _resolver, coordinator, run_id, gate_ref) = service_fixture("send the email");
    service
        .resolve(ResolveApprovalInteractionRequest {
            scope: scope(),
            actor: actor("user-alpha"),
            run_id_hint: None,
            gate_ref,
            decision: ApprovalInteractionDecision::ApproveOnce,
            idempotency_key: IdempotencyKey::new("approve-no-hint").expect("idempotency"),
        })
        .await
        .expect("approve without hint");

    assert_eq!(coordinator.last_resumption_run_id(), Some(run_id));
}

#[tokio::test]
async fn approve_spawn_capability_routes_to_spawn_resolver_then_resumes_blocked_approval() {
    let (service, resolver, coordinator, run_id, gate_ref) =
        service_fixture_for_request(spawn_approval_request("start the worker"));
    let response = service
        .resolve(ResolveApprovalInteractionRequest {
            scope: scope(),
            actor: actor("user-alpha"),
            run_id_hint: Some(run_id),
            gate_ref,
            decision: ApprovalInteractionDecision::ApproveOnce,
            idempotency_key: IdempotencyKey::new("approve-spawn").expect("idempotency"),
        })
        .await
        .expect("approve spawn");

    assert!(matches!(
        response,
        ResolveApprovalInteractionResponse::Approved(_)
    ));
    assert_eq!(resolver.approval_count(), 0);
    assert_eq!(resolver.spawn_approval_count(), 1);
    assert_eq!(coordinator.resumption_count(), 1);
}

#[tokio::test]
async fn unsupported_approval_action_returns_invalid_request_without_resolution() {
    let (service, resolver, coordinator, run_id, gate_ref) =
        service_fixture_for_request(unsupported_approval_request("external write"));
    let err = service
        .resolve(ResolveApprovalInteractionRequest {
            scope: scope(),
            actor: actor("user-alpha"),
            run_id_hint: Some(run_id),
            gate_ref,
            decision: ApprovalInteractionDecision::ApproveOnce,
            idempotency_key: IdempotencyKey::new("unsupported-action").expect("idempotency"),
        })
        .await
        .expect_err("unsupported action");

    assert!(matches!(
        err,
        ironclaw_product_workflow::ProductWorkflowError::ApprovalInteractionRejected {
            kind: ApprovalInteractionRejectionKind::UnsupportedAction
        }
    ));
    assert_eq!(resolver.approval_count(), 0);
    assert_eq!(resolver.spawn_approval_count(), 0);
    assert_eq!(coordinator.resumption_count(), 0);
}

#[tokio::test]
async fn already_approved_gate_retries_lease_issue_then_resumes() {
    let (service, resolver, coordinator, run_id, gate_ref) = service_fixture_for_request_status(
        approval_request("send the email"),
        ApprovalStatus::Approved,
    );

    let response = service
        .resolve(ResolveApprovalInteractionRequest {
            scope: scope(),
            actor: actor("user-alpha"),
            run_id_hint: Some(run_id),
            gate_ref,
            decision: ApprovalInteractionDecision::ApproveOnce,
            idempotency_key: IdempotencyKey::new("retry-approved").expect("idempotency"),
        })
        .await
        .expect("retry approved");

    assert!(matches!(
        response,
        ResolveApprovalInteractionResponse::Approved(_)
    ));
    assert_eq!(resolver.approval_count(), 0);
    assert_eq!(resolver.spawn_approval_count(), 0);
    assert_eq!(resolver.dispatch_lease_retry_count(), 1);
    assert_eq!(coordinator.resumption_count(), 1);
}

#[tokio::test]
async fn already_approved_replay_reaches_turn_coordinator_when_run_is_not_parked() {
    let (service, resolver, coordinator, run_id, gate_ref) = service_fixture_for_request_status(
        approval_request("send the email"),
        ApprovalStatus::Approved,
    );
    coordinator.set_status(TurnStatus::Queued);

    let response = service
        .resolve(ResolveApprovalInteractionRequest {
            scope: scope(),
            actor: actor("user-alpha"),
            run_id_hint: Some(run_id),
            gate_ref,
            decision: ApprovalInteractionDecision::ApproveOnce,
            idempotency_key: IdempotencyKey::new("replay-approved").expect("idempotency"),
        })
        .await
        .expect("replay approved");

    assert!(matches!(
        response,
        ResolveApprovalInteractionResponse::Approved(_)
    ));
    assert_eq!(resolver.dispatch_lease_retry_count(), 0);
    assert_eq!(coordinator.resumption_count(), 1);
}

#[tokio::test]
async fn already_approved_always_allow_replay_rejects_without_persisting_policy() {
    let request = approval_request("send the email");
    let capability = match request.action.as_ref() {
        Action::Dispatch { capability, .. } => capability.clone(),
        _ => panic!("test request should be dispatch"),
    };
    let policy_scope = resource_scope(&actor("user-alpha"));
    let key = PersistentApprovalPolicyKey::new(
        &policy_scope,
        PersistentApprovalAction::Dispatch,
        capability,
        Principal::User(UserId::new("user-alpha").expect("user")),
    )
    .expect("persistent policy key");
    let (service, resolver, coordinator, run_id, gate_ref) =
        service_fixture_for_request_status(request, ApprovalStatus::Approved);
    coordinator.set_status(TurnStatus::Queued);
    let policies = Arc::new(InMemoryPersistentApprovalPolicyStore::new());
    let policy_store: Arc<dyn PersistentApprovalPolicyStore> = policies.clone();
    let service = service.with_persistent_policy_store(policy_store);

    let err = service
        .resolve(ResolveApprovalInteractionRequest {
            scope: scope(),
            actor: actor("user-alpha"),
            run_id_hint: Some(run_id),
            gate_ref,
            decision: ApprovalInteractionDecision::AlwaysAllow,
            idempotency_key: IdempotencyKey::new("replay-approved-always").expect("idempotency"),
        })
        .await
        .expect_err("replay approved always allow");

    assert!(matches!(
        err,
        ironclaw_product_workflow::ProductWorkflowError::ApprovalInteractionRejected {
            kind: ApprovalInteractionRejectionKind::AlwaysAllowUnsupported
        }
    ));
    assert_eq!(resolver.approval_count(), 0);
    assert_eq!(resolver.dispatch_lease_retry_count(), 0);
    assert_eq!(coordinator.resumption_count(), 0);
    assert!(
        policies
            .lookup(&key)
            .await
            .expect("persistent policy lookup")
            .is_none()
    );
}

#[tokio::test]
async fn already_approved_product_replay_without_run_hint_recovers_historical_run_id() {
    let actor = actor("user-alpha");
    let approval_scope = resource_scope(&actor);
    let request = approval_request("send the email");
    let request_id = request.id;
    let gate_ref = approval_gate_ref(request_id).expect("approval gate");
    let run_id = TurnRunId::new();
    let approvals = Arc::new(InMemoryApprovalRequestStore::new());
    approvals
        .save_pending(approval_scope.clone(), request)
        .await
        .expect("save approval");
    approvals
        .approve(&approval_scope, request_id)
        .await
        .expect("mark approved");
    let coordinator = Arc::new(FakeTurnCoordinator::blocked(
        actor.clone(),
        gate_ref.clone(),
    ));
    coordinator.set_status(TurnStatus::Queued);
    let resolver = Arc::new(RecordingApprovalResolver::default());
    let service = DefaultApprovalInteractionService::new(
        Arc::new(RunStateApprovalInteractionReadModel::new(
            approvals,
            Arc::new(FakeTurnRunLocator::with_historical_run(
                run_id,
                gate_ref.clone(),
            )),
        )),
        Arc::new(FixedLeaseTermsProvider),
        resolver.clone(),
        coordinator.clone(),
    );

    let response = service
        .resolve(ResolveApprovalInteractionRequest {
            scope: scope(),
            actor,
            run_id_hint: None,
            gate_ref,
            decision: ApprovalInteractionDecision::ApproveOnce,
            idempotency_key: IdempotencyKey::new("replay-approved-no-hint").expect("idempotency"),
        })
        .await
        .expect("replay approved without hint");

    assert!(matches!(
        response,
        ResolveApprovalInteractionResponse::Approved(_)
    ));
    assert_eq!(resolver.dispatch_lease_retry_count(), 0);
    assert_eq!(coordinator.last_resumption_run_id(), Some(run_id));
}

#[tokio::test]
async fn already_denied_gate_cancels_without_reissuing_denial() {
    let (service, resolver, coordinator, run_id, gate_ref) = service_fixture_for_request_status(
        approval_request("delete a file"),
        ApprovalStatus::Denied,
    );

    let response = service
        .resolve(ResolveApprovalInteractionRequest {
            scope: scope(),
            actor: actor("user-alpha"),
            run_id_hint: Some(run_id),
            gate_ref,
            decision: ApprovalInteractionDecision::Deny,
            idempotency_key: IdempotencyKey::new("retry-denied").expect("idempotency"),
        })
        .await
        .expect("retry denied");

    assert!(matches!(
        response,
        ResolveApprovalInteractionResponse::Denied(_)
    ));
    assert_eq!(resolver.denial_count(), 0);
    assert_eq!(coordinator.cancellation_count(), 1);
}

#[tokio::test]
async fn already_denied_replay_reaches_turn_coordinator_when_run_is_not_parked() {
    let (service, resolver, coordinator, run_id, gate_ref) = service_fixture_for_request_status(
        approval_request("delete a file"),
        ApprovalStatus::Denied,
    );
    coordinator.set_status(TurnStatus::Cancelled);

    let response = service
        .resolve(ResolveApprovalInteractionRequest {
            scope: scope(),
            actor: actor("user-alpha"),
            run_id_hint: Some(run_id),
            gate_ref,
            decision: ApprovalInteractionDecision::Deny,
            idempotency_key: IdempotencyKey::new("replay-denied").expect("idempotency"),
        })
        .await
        .expect("replay denied");

    assert!(matches!(
        response,
        ResolveApprovalInteractionResponse::Denied(_)
    ));
    assert_eq!(resolver.denial_count(), 0);
    assert_eq!(coordinator.cancellation_count(), 1);
}

#[tokio::test]
async fn deny_marks_pending_gate_denied_then_cancels_run() {
    let (service, resolver, coordinator, run_id, gate_ref) = service_fixture("delete a file");
    let response = service
        .resolve(ResolveApprovalInteractionRequest {
            scope: scope(),
            actor: actor("user-alpha"),
            run_id_hint: Some(run_id),
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
            run_id_hint: Some(run_id),
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
async fn resolve_rejects_malformed_approval_gate_ref_without_side_effects() {
    let resolver = Arc::new(RecordingApprovalResolver::default());
    let bad_gate_ref = GateRef::new("gate:approval-not-a-request-id").expect("gate ref");
    let coordinator = Arc::new(FakeTurnCoordinator::blocked(
        actor("user-alpha"),
        bad_gate_ref.clone(),
    ));
    let service = DefaultApprovalInteractionService::new(
        Arc::new(RunStateApprovalInteractionReadModel::new(
            Arc::new(InMemoryApprovalRequestStore::new()),
            Arc::new(FakeTurnRunLocator::with_run(
                TurnRunId::new(),
                bad_gate_ref.clone(),
            )),
        )),
        Arc::new(FixedLeaseTermsProvider),
        resolver.clone(),
        coordinator.clone(),
    );

    let err = service
        .resolve(ResolveApprovalInteractionRequest {
            scope: scope(),
            actor: actor("user-alpha"),
            run_id_hint: None,
            gate_ref: bad_gate_ref,
            decision: ApprovalInteractionDecision::ApproveOnce,
            idempotency_key: IdempotencyKey::new("malformed-gate").expect("idempotency"),
        })
        .await
        .expect_err("malformed approval gate");

    assert!(matches!(
        err,
        ironclaw_product_workflow::ProductWorkflowError::ApprovalInteractionRejected {
            kind: ApprovalInteractionRejectionKind::InvalidGateRef
        }
    ));
    assert_eq!(resolver.approval_count(), 0);
    assert_eq!(resolver.denial_count(), 0);
    assert_eq!(coordinator.resumption_count(), 0);
    assert_eq!(coordinator.cancellation_count(), 0);
}

#[tokio::test]
async fn stale_gate_returns_conflict_without_resolution() {
    let (service, resolver, coordinator, run_id, gate_ref) = service_fixture("send the email");
    coordinator.set_status(TurnStatus::Queued);

    let err = service
        .resolve(ResolveApprovalInteractionRequest {
            scope: scope(),
            actor: actor("user-alpha"),
            run_id_hint: Some(run_id),
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
    let request = approval_request("send the email");
    let beta_actor = actor("user-beta");
    let gate_ref = approval_gate_ref(request.id).expect("gate ref");
    let run_id = TurnRunId::new();
    let gate = ApprovalGateRecord::new(
        resource_scope(&beta_actor),
        run_id,
        gate_ref.clone(),
        request,
    )
    .expect("approval gate");
    let resolver = Arc::new(RecordingApprovalResolver::default());
    let coordinator = Arc::new(FakeTurnCoordinator::blocked(
        actor("user-alpha"),
        gate_ref.clone(),
    ));
    let service = DefaultApprovalInteractionService::new(
        Arc::new(FakeReadModel::with_gate(gate)),
        Arc::new(FixedLeaseTermsProvider),
        resolver.clone(),
        coordinator,
    );

    let err = service
        .resolve(ResolveApprovalInteractionRequest {
            scope: scope(),
            actor: beta_actor,
            run_id_hint: Some(run_id),
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
async fn list_pending_does_not_expose_invocation_fingerprint() {
    let alpha_actor = actor("user-alpha");
    let mut request = approval_request("send the email");
    let fingerprint = InvocationFingerprint::for_dispatch(
        &resource_scope(&alpha_actor),
        &CapabilityId::new("demo.echo").expect("capability"),
        &ResourceEstimate::default(),
        &serde_json::json!({"message": "private"}),
    )
    .expect("fingerprint");
    let fingerprint_token = fingerprint.as_str().to_string();
    request.invocation_fingerprint = Some(fingerprint);
    let (service, _, _, _, _) = service_fixture_for_request(request);

    let response = service
        .list_pending(ListPendingApprovalsRequest {
            scope: scope(),
            actor: alpha_actor,
        })
        .await
        .expect("list pending");
    let serialized = serde_json::to_string(&response).expect("serialize");

    assert_eq!(response.approvals.len(), 1);
    assert!(
        !serialized.contains(&fingerprint_token),
        "approval DTO leaked invocation fingerprint"
    );
}

#[tokio::test]
async fn list_pending_never_derives_summary_from_raw_approval_reason() {
    for unsafe_reason in [
        "send the email",
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
async fn list_pending_filters_non_pending_gates_and_sorts_stably() {
    let alpha_actor = actor("user-alpha");
    let pending_late = approval_request("second");
    let pending_early = approval_request("first");
    let approved = approval_request("already approved");
    let denied = approval_request("already denied");
    let gates = vec![
        ApprovalGateRecord::with_status(
            resource_scope(&alpha_actor),
            TurnRunId::new(),
            approval_gate_ref(pending_late.id).expect("late gate"),
            pending_late.clone(),
            ApprovalStatus::Pending,
        )
        .expect("late pending"),
        ApprovalGateRecord::with_status(
            resource_scope(&alpha_actor),
            TurnRunId::new(),
            approval_gate_ref(approved.id).expect("approved gate"),
            approved,
            ApprovalStatus::Approved,
        )
        .expect("approved gate"),
        ApprovalGateRecord::with_status(
            resource_scope(&alpha_actor),
            TurnRunId::new(),
            approval_gate_ref(denied.id).expect("denied gate"),
            denied,
            ApprovalStatus::Denied,
        )
        .expect("denied gate"),
        ApprovalGateRecord::with_status(
            resource_scope(&alpha_actor),
            TurnRunId::new(),
            approval_gate_ref(pending_early.id).expect("early gate"),
            pending_early.clone(),
            ApprovalStatus::Pending,
        )
        .expect("early pending"),
    ];
    let resolver = Arc::new(RecordingApprovalResolver::default());
    let coordinator = Arc::new(FakeTurnCoordinator::blocked(
        alpha_actor.clone(),
        approval_gate_ref(pending_late.id).expect("gate ref"),
    ));
    let service = DefaultApprovalInteractionService::new(
        Arc::new(FakeReadModel::with_gates(gates)),
        Arc::new(FixedLeaseTermsProvider),
        resolver,
        coordinator,
    );

    let response = service
        .list_pending(ListPendingApprovalsRequest {
            scope: scope(),
            actor: alpha_actor,
        })
        .await
        .expect("list pending");

    assert_eq!(response.approvals.len(), 2);
    let mut expected = response
        .approvals
        .iter()
        .map(|approval| (approval.run_id, approval.gate_ref.clone()))
        .collect::<Vec<_>>();
    expected.sort_by(|left, right| {
        left.0
            .as_uuid()
            .cmp(&right.0.as_uuid())
            .then_with(|| left.1.as_str().cmp(right.1.as_str()))
    });
    assert_eq!(
        response
            .approvals
            .iter()
            .map(|approval| (approval.run_id, approval.gate_ref.clone()))
            .collect::<Vec<_>>(),
        expected
    );
    assert!(
        response
            .approvals
            .iter()
            .all(|approval| approval.approval_request_id == pending_late.id
                || approval.approval_request_id == pending_early.id)
    );
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
async fn approval_resolver_port_retries_missing_lease_for_approved_request() {
    let alpha_actor = actor("user-alpha");
    let resource_scope = resource_scope(&alpha_actor);
    let mut request = approval_request("approval required");
    request.invocation_fingerprint = Some(
        InvocationFingerprint::for_dispatch(
            &resource_scope,
            &CapabilityId::new("demo.echo").expect("capability"),
            &ResourceEstimate::default(),
            &serde_json::json!({"message": "approved"}),
        )
        .expect("fingerprint"),
    );
    let request_id = request.id;
    let approvals = Arc::new(InMemoryApprovalRequestStore::new());
    approvals
        .save_pending(resource_scope.clone(), request)
        .await
        .expect("save approval");
    approvals
        .approve(&resource_scope, request_id)
        .await
        .expect("mark approved");
    let leases = Arc::new(InMemoryCapabilityLeaseStore::new());
    let resolver = ApprovalResolverPort::new(approvals, leases.clone());

    resolver
        .ensure_dispatch_lease(
            &resource_scope,
            request_id,
            dispatch_lease_approval(Principal::User(alpha_actor.user_id)),
        )
        .await
        .expect("retry missing lease");

    assert_eq!(leases.leases_for_scope(&resource_scope).await.len(), 1);
}

#[tokio::test]
async fn approval_resolver_port_retries_missing_spawn_lease_for_approved_request() {
    let alpha_actor = actor("user-alpha");
    let resource_scope = resource_scope(&alpha_actor);
    let mut request = spawn_approval_request("approval required");
    let capability = match request.action.as_ref() {
        Action::SpawnCapability { capability, .. } => capability.clone(),
        _ => panic!("test request should be spawn"),
    };
    request.invocation_fingerprint = Some(
        InvocationFingerprint::for_spawn(
            &resource_scope,
            &capability,
            &ResourceEstimate::default(),
            &serde_json::json!({"process": "approved"}),
        )
        .expect("fingerprint"),
    );
    let request_id = request.id;
    let approvals = Arc::new(InMemoryApprovalRequestStore::new());
    approvals
        .save_pending(resource_scope.clone(), request)
        .await
        .expect("save approval");
    approvals
        .approve(&resource_scope, request_id)
        .await
        .expect("mark approved");
    let leases = Arc::new(InMemoryCapabilityLeaseStore::new());
    let resolver = ApprovalResolverPort::new(approvals, leases.clone());
    let mut approval = dispatch_lease_approval(Principal::User(alpha_actor.user_id));
    approval.allowed_effects.push(EffectKind::SpawnProcess);

    resolver
        .ensure_spawn_lease(&resource_scope, request_id, approval)
        .await
        .expect("retry missing spawn lease");

    assert_eq!(leases.leases_for_scope(&resource_scope).await.len(), 1);
}

#[tokio::test]
async fn approval_resolver_port_does_not_duplicate_existing_lease_for_approved_request() {
    let alpha_actor = actor("user-alpha");
    let resource_scope = resource_scope(&alpha_actor);
    let mut request = approval_request("approval required");
    request.invocation_fingerprint = Some(
        InvocationFingerprint::for_dispatch(
            &resource_scope,
            &CapabilityId::new("demo.echo").expect("capability"),
            &ResourceEstimate::default(),
            &serde_json::json!({"message": "approved"}),
        )
        .expect("fingerprint"),
    );
    let request_id = request.id;
    let approvals = Arc::new(InMemoryApprovalRequestStore::new());
    approvals
        .save_pending(resource_scope.clone(), request)
        .await
        .expect("save approval");
    let leases = Arc::new(InMemoryCapabilityLeaseStore::new());
    let resolver = ApprovalResolverPort::new(approvals, leases.clone());
    let approval = dispatch_lease_approval(Principal::User(alpha_actor.user_id.clone()));
    resolver
        .approve_dispatch(&resource_scope, request_id, approval.clone())
        .await
        .expect("approve and issue lease");

    resolver
        .ensure_dispatch_lease(&resource_scope, request_id, approval)
        .await
        .expect("existing lease is enough");

    assert_eq!(leases.leases_for_scope(&resource_scope).await.len(), 1);
}

#[tokio::test]
async fn approval_resolver_port_reissues_when_existing_dispatch_lease_is_claimed() {
    let alpha_actor = actor("user-alpha");
    let resource_scope = resource_scope(&alpha_actor);
    let mut request = approval_request("approval required");
    request.invocation_fingerprint = Some(
        InvocationFingerprint::for_dispatch(
            &resource_scope,
            &CapabilityId::new("demo.echo").expect("capability"),
            &ResourceEstimate::default(),
            &serde_json::json!({"message": "approved"}),
        )
        .expect("fingerprint"),
    );
    let fingerprint = request.invocation_fingerprint.clone().expect("fingerprint");
    let request_id = request.id;
    let approvals = Arc::new(InMemoryApprovalRequestStore::new());
    approvals
        .save_pending(resource_scope.clone(), request)
        .await
        .expect("save approval");
    let leases = Arc::new(InMemoryCapabilityLeaseStore::new());
    let resolver = ApprovalResolverPort::new(approvals, leases.clone());
    let approval = dispatch_lease_approval(Principal::User(alpha_actor.user_id.clone()));
    resolver
        .approve_dispatch(&resource_scope, request_id, approval.clone())
        .await
        .expect("approve and issue lease");
    let lease_id = leases
        .leases_for_scope(&resource_scope)
        .await
        .into_iter()
        .next()
        .expect("lease")
        .grant
        .id;
    leases
        .claim(&resource_scope, lease_id, &fingerprint)
        .await
        .expect("claim lease");

    resolver
        .ensure_dispatch_lease(&resource_scope, request_id, approval)
        .await
        .expect("claimed lease is not enough");

    let leases = leases.leases_for_scope(&resource_scope).await;
    assert_eq!(leases.len(), 2);
    assert!(
        leases
            .iter()
            .any(|lease| lease.status == CapabilityLeaseStatus::Claimed)
    );
    assert!(
        leases
            .iter()
            .any(|lease| lease.status == CapabilityLeaseStatus::Active)
    );
}

#[tokio::test]
async fn approval_resolver_port_does_not_duplicate_existing_spawn_lease_for_approved_request() {
    let alpha_actor = actor("user-alpha");
    let resource_scope = resource_scope(&alpha_actor);
    let mut request = spawn_approval_request("approval required");
    let capability = match request.action.as_ref() {
        Action::SpawnCapability { capability, .. } => capability.clone(),
        _ => panic!("test request should be spawn"),
    };
    request.invocation_fingerprint = Some(
        InvocationFingerprint::for_spawn(
            &resource_scope,
            &capability,
            &ResourceEstimate::default(),
            &serde_json::json!({"process": "approved"}),
        )
        .expect("fingerprint"),
    );
    let request_id = request.id;
    let approvals = Arc::new(InMemoryApprovalRequestStore::new());
    approvals
        .save_pending(resource_scope.clone(), request)
        .await
        .expect("save approval");
    let leases = Arc::new(InMemoryCapabilityLeaseStore::new());
    let resolver = ApprovalResolverPort::new(approvals, leases.clone());
    let mut approval = dispatch_lease_approval(Principal::User(alpha_actor.user_id));
    approval.allowed_effects.push(EffectKind::SpawnProcess);
    resolver
        .approve_spawn(&resource_scope, request_id, approval.clone())
        .await
        .expect("approve and issue spawn lease");

    resolver
        .ensure_spawn_lease(&resource_scope, request_id, approval)
        .await
        .expect("existing spawn lease is enough");

    assert_eq!(leases.leases_for_scope(&resource_scope).await.len(), 1);
}

#[tokio::test]
async fn run_state_read_model_uses_parked_turn_run_id_for_pending_approvals() {
    let alpha_actor = actor("user-alpha");
    let resource_scope = resource_scope(&alpha_actor);
    let request = approval_request("send the email");
    let invocation_derived_run_id = TurnRunId::from_uuid(resource_scope.invocation_id.as_uuid());
    let parked_turn_run_id = TurnRunId::new();
    assert_ne!(
        parked_turn_run_id, invocation_derived_run_id,
        "test must prove capability invocation ids are not turn run ids"
    );
    let approvals = Arc::new(InMemoryApprovalRequestStore::new());
    approvals
        .save_pending(resource_scope.clone(), request.clone())
        .await
        .expect("save approval");
    let gate_ref = approval_gate_ref(request.id).expect("approval gate");
    let read_model = RunStateApprovalInteractionReadModel::new(
        approvals,
        Arc::new(FakeTurnRunLocator::with_run(
            parked_turn_run_id,
            gate_ref.clone(),
        )),
    );

    let pending = read_model
        .approval_gates(&ApprovalInteractionScope::from_turn(&scope(), &alpha_actor))
        .await
        .expect("pending approvals");

    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].run_id(), parked_turn_run_id);
    assert_eq!(pending[0].gate_ref(), &gate_ref);
    assert_eq!(pending[0].request().id, request.id);

    let targeted = read_model
        .approval_gate(
            &ApprovalInteractionScope::from_turn(&scope(), &alpha_actor),
            None,
            &gate_ref,
        )
        .await
        .expect("targeted approval")
        .expect("gate exists");
    assert_eq!(targeted.run_id(), parked_turn_run_id);

    let other_user_pending = read_model
        .approval_gates(&ApprovalInteractionScope::from_turn(
            &scope(),
            &actor("user-beta"),
        ))
        .await
        .expect("other user pending approvals");
    assert!(other_user_pending.is_empty());
}
