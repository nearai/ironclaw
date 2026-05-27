//! Contract tests for the product workflow facade.

use std::sync::{Arc, Mutex};
use std::time::Duration as StdDuration;

use async_trait::async_trait;
use chrono::{Duration, Utc};
use ironclaw_conversations::InMemoryConversationServices;
use ironclaw_host_api::{AgentId, ApprovalRequestId, ProjectId, TenantId, ThreadId, UserId};
use ironclaw_product_adapters::{
    AdapterInstallationId, ApprovalDecision, ApprovalResolutionPayload, AuthRequirement,
    AuthResolutionPayload, AuthResolutionResult, ExternalActorRef, ExternalConversationRef,
    ExternalEventId, InboundCommandPayload, LinkedThreadActionPayload, ParsedProductInbound,
    ProductAdapterError, ProductAdapterId, ProductInboundAck, ProductInboundEnvelope,
    ProductInboundPayload, ProductRejection, ProductRejectionDisposition, ProductRejectionKind,
    ProductTriggerReason, ProductWorkflow, ProductWorkflowRejectionKind, ProjectionCursor,
    ProjectionSubscriptionPayload, ProtocolAuthEvidence, TrustedInboundContext, UserMessagePayload,
};
use ironclaw_product_workflow::{
    ActionDispatchKind, ActionFingerprintKey, ApprovalInteractionDecision,
    ApprovalInteractionScope, ApprovalInteractionService, AuthRequestRef, BeforeInboundPolicy,
    BeforeInboundPolicyOutcome, BeforeInboundPolicyRequest, DefaultInboundTurnService,
    DefaultProductWorkflow, FakeBeforeInboundPolicy, FakeConversationBindingService,
    FakeIdempotencyLedger, FakeInboundTurnService, IdempotencyDecision, IdempotencyLedger,
    InMemoryIdempotencyLedger, InboundTurnOutcome, InboundTurnService, InboundUserMessageDispatch,
    LinkedThreadActionId, ListPendingApprovalsRequest, ListPendingApprovalsResponse,
    PendingApprovalInteractionView, ProductCommandName, ProductConversationBindingService,
    ProductInstallationKey, ProductInstallationScope, ProductWorkflowError,
    ResolveApprovalInteractionRequest, ResolveApprovalInteractionResponse, ResolvedBinding,
    SourceBindingKey, StaticProductInstallationResolver, approval_gate_ref,
};
use ironclaw_threads::InMemorySessionThreadService;
use ironclaw_turns::{
    AcceptedMessageRef, CancelRunRequest, CancelRunResponse, EventCursor, GateRef,
    GetRunStateRequest, LoopGateRef, ResumeTurnRequest, ResumeTurnResponse, RunProfileId,
    RunProfileVersion, SubmitTurnRequest, SubmitTurnResponse, ThreadBusy, TurnCoordinator,
    TurnError, TurnId, TurnRunId, TurnRunState, TurnScope, TurnStatus,
};

fn sample_envelope(event_suffix: &str) -> ProductInboundEnvelope {
    sample_envelope_with_payload(
        event_suffix,
        ProductInboundPayload::UserMessage(
            UserMessagePayload::new("hello", vec![], ProductTriggerReason::DirectChat)
                .expect("valid"),
        ),
    )
}

fn sample_noop_envelope(event_suffix: &str) -> ProductInboundEnvelope {
    sample_envelope_with_payload(event_suffix, ProductInboundPayload::NoOp)
}

fn sample_envelope_with_payload(
    event_suffix: &str,
    payload: ProductInboundPayload,
) -> ProductInboundEnvelope {
    sample_envelope_with_context(
        ProductAdapterId::new("test_adapter").expect("valid"),
        AdapterInstallationId::new("install_alpha").expect("valid"),
        ExternalEventId::new(format!("evt:{event_suffix}")).expect("valid"),
        ExternalActorRef::new("test", "user1", Option::<String>::None).expect("valid"),
        ExternalConversationRef::new(None, "conv1", None, None).expect("valid"),
        payload,
    )
}

fn sample_envelope_with_context(
    adapter_id: ProductAdapterId,
    installation_id: AdapterInstallationId,
    external_event_id: ExternalEventId,
    external_actor_ref: ExternalActorRef,
    external_conversation_ref: ExternalConversationRef,
    payload: ProductInboundPayload,
) -> ProductInboundEnvelope {
    let evidence = ProtocolAuthEvidence::test_verified(
        AuthRequirement::SharedSecretHeader {
            header_name: "X-Secret".into(),
        },
        installation_id.as_str(),
    );
    let context = TrustedInboundContext::from_verified_evidence(
        adapter_id,
        installation_id,
        Utc::now(),
        &evidence,
    )
    .expect("verified");

    let parsed = ParsedProductInbound::new(
        external_event_id,
        external_actor_ref,
        external_conversation_ref,
        payload,
    )
    .expect("parsed");

    ProductInboundEnvelope::from_trusted_parse(context, parsed).expect("envelope")
}

#[derive(Default)]
struct RecordingTurnCoordinator {
    submissions: Mutex<Vec<SubmitTurnRequest>>,
    busy_once: Mutex<Option<TurnRunId>>,
}

impl RecordingTurnCoordinator {
    fn submissions(&self) -> Vec<SubmitTurnRequest> {
        self.submissions.lock().expect("lock").clone()
    }

    fn force_thread_busy_once(&self, active_run_id: TurnRunId) {
        *self.busy_once.lock().expect("lock") = Some(active_run_id);
    }
}

#[async_trait]
impl TurnCoordinator for RecordingTurnCoordinator {
    async fn prepare_turn(&self, _scope: TurnScope) -> Result<TurnRunId, TurnError> {
        Ok(TurnRunId::new())
    }

    async fn submit_turn(
        &self,
        request: SubmitTurnRequest,
    ) -> Result<SubmitTurnResponse, TurnError> {
        if let Some(active_run_id) = self.busy_once.lock().expect("lock").take() {
            return Err(TurnError::ThreadBusy(ThreadBusy {
                active_run_id,
                status: TurnStatus::Running,
                event_cursor: EventCursor::default(),
            }));
        }
        let response = SubmitTurnResponse::Accepted {
            turn_id: TurnId::new(),
            run_id: TurnRunId::new(),
            status: TurnStatus::Queued,
            resolved_run_profile_id: RunProfileId::default_profile(),
            resolved_run_profile_version: RunProfileVersion::new(1),
            event_cursor: EventCursor::default(),
            accepted_message_ref: request.accepted_message_ref.clone(),
            reply_target_binding_ref: request.reply_target_binding_ref.clone(),
        };
        self.submissions.lock().expect("lock").push(request);
        Ok(response)
    }

    async fn resume_turn(
        &self,
        _request: ResumeTurnRequest,
    ) -> Result<ResumeTurnResponse, TurnError> {
        panic!("resume_turn is not used by product workflow contract tests")
    }

    async fn cancel_run(&self, _request: CancelRunRequest) -> Result<CancelRunResponse, TurnError> {
        panic!("cancel_run is not used by product workflow contract tests")
    }

    async fn get_run_state(&self, _request: GetRunStateRequest) -> Result<TurnRunState, TurnError> {
        panic!("get_run_state is not used by product workflow contract tests")
    }
}

struct RecordingApprovalInteractionService {
    gate_ref: GateRef,
    run_id: TurnRunId,
    resolutions: Mutex<Vec<ResolveApprovalInteractionRequest>>,
}

impl RecordingApprovalInteractionService {
    fn new(gate_ref: GateRef, run_id: TurnRunId) -> Self {
        Self {
            gate_ref,
            run_id,
            resolutions: Mutex::new(Vec::new()),
        }
    }

    fn resolutions(&self) -> Vec<ResolveApprovalInteractionRequest> {
        self.resolutions.lock().expect("lock").clone()
    }
}

#[async_trait]
impl ApprovalInteractionService for RecordingApprovalInteractionService {
    async fn list_pending(
        &self,
        request: ListPendingApprovalsRequest,
    ) -> Result<ListPendingApprovalsResponse, ProductWorkflowError> {
        let scope = ApprovalInteractionScope::from_turn(&request.scope, &request.actor);
        Ok(ListPendingApprovalsResponse {
            approvals: vec![PendingApprovalInteractionView {
                scope,
                run_id: self.run_id,
                gate_ref: self.gate_ref.clone(),
                approval_request_id: ApprovalRequestId::new(),
                summary: "Approval required".to_string(),
                action: ironclaw_product_workflow::ApprovalInteractionActionView::Other,
            }],
        })
    }

    async fn resolve(
        &self,
        request: ResolveApprovalInteractionRequest,
    ) -> Result<ResolveApprovalInteractionResponse, ProductWorkflowError> {
        let run_id = request.run_id_hint.unwrap_or(self.run_id);
        self.resolutions.lock().expect("lock").push(request);
        Ok(
            match self
                .resolutions
                .lock()
                .expect("lock")
                .last()
                .expect("recorded")
                .decision
            {
                ApprovalInteractionDecision::ApproveOnce => {
                    ResolveApprovalInteractionResponse::Approved(ResumeTurnResponse {
                        run_id,
                        status: TurnStatus::Queued,
                        event_cursor: EventCursor(21),
                    })
                }
                ApprovalInteractionDecision::Deny => {
                    ResolveApprovalInteractionResponse::Denied(CancelRunResponse {
                        run_id,
                        status: TurnStatus::Cancelled,
                        event_cursor: EventCursor(22),
                        already_terminal: false,
                        actor: None,
                    })
                }
            },
        )
    }
}

#[test]
fn action_fingerprint_retains_typed_identifiers() {
    let adapter_id = ProductAdapterId::new("test_adapter").expect("valid");
    let installation_id = AdapterInstallationId::new("install_alpha").expect("valid");
    let external_actor_ref =
        ExternalActorRef::new("test", "user1", Option::<String>::None).expect("valid actor");
    let source_binding_key = SourceBindingKey::new("space:0:;conversation:5:conv1;topic:0:;")
        .expect("valid source binding key");
    let external_event_id = ExternalEventId::new("evt:typed").expect("valid");

    let fingerprint = ActionFingerprintKey::new(
        adapter_id.clone(),
        installation_id.clone(),
        external_actor_ref.clone(),
        source_binding_key.clone(),
        external_event_id.clone(),
    );

    assert_eq!(fingerprint.adapter_id, adapter_id);
    assert_eq!(fingerprint.installation_id, installation_id);
    assert_eq!(fingerprint.external_actor_ref, external_actor_ref);
    assert_eq!(fingerprint.source_binding_key, source_binding_key);
    assert_eq!(fingerprint.external_event_id, external_event_id);
}

#[test]
fn turn_submission_error_maps_to_stable_product_category() {
    let err: ProductAdapterError = ProductWorkflowError::TurnSubmissionFailed {
        error: TurnError::Unauthorized,
    }
    .into();

    match err {
        ProductAdapterError::WorkflowRejected {
            kind,
            status_code,
            retryable,
            ..
        } => {
            assert_eq!(kind, ProductWorkflowRejectionKind::Unauthorized);
            assert_eq!(status_code, 403);
            assert!(!retryable);
        }
        other => panic!("expected typed workflow rejection, got {other:?}"),
    }
}

#[test]
fn action_dispatch_kind_retains_typed_payload_refs() {
    let command_payload = ProductInboundPayload::Command(
        InboundCommandPayload::new("help", "", ProductTriggerReason::BotCommand).expect("valid"),
    );
    assert_eq!(
        ActionDispatchKind::try_from_payload(&command_payload).expect("command kind"),
        ActionDispatchKind::Command {
            command: ProductCommandName::new("help").expect("valid command")
        }
    );

    let gate_ref = LoopGateRef::new("gate:approval-1").expect("valid gate ref");
    let approval_payload = ProductInboundPayload::ApprovalResolution(
        ApprovalResolutionPayload::new(gate_ref.as_str(), ApprovalDecision::ApproveOnce)
            .expect("valid"),
    );
    assert_eq!(
        ActionDispatchKind::try_from_payload(&approval_payload).expect("approval kind"),
        ActionDispatchKind::ApprovalResolution { gate_ref }
    );

    let auth_payload = ProductInboundPayload::AuthResolution(
        AuthResolutionPayload::new("auth-request-1", AuthResolutionResult::Denied).expect("valid"),
    );
    assert_eq!(
        ActionDispatchKind::try_from_payload(&auth_payload).expect("auth kind"),
        ActionDispatchKind::AuthResolution {
            auth_request_ref: AuthRequestRef::new("auth-request-1").expect("valid auth ref")
        }
    );

    let linked_payload = ProductInboundPayload::LinkedThreadAction(
        LinkedThreadActionPayload::new("open-thread", None, None).expect("valid"),
    );
    assert_eq!(
        ActionDispatchKind::try_from_payload(&linked_payload).expect("linked kind"),
        ActionDispatchKind::LinkedThreadAction {
            action_id: LinkedThreadActionId::new("open-thread").expect("valid action id")
        }
    );
}

fn fake_binding() -> ResolvedBinding {
    ResolvedBinding {
        tenant_id: TenantId::new("tenant:fake").expect("valid tenant"),
        user_id: UserId::new("user:fake").expect("valid user"),
        thread_id: ThreadId::new("thread:fake").expect("valid thread"),
        agent_id: Some(AgentId::new("agent:fake").expect("valid agent")),
        project_id: None,
    }
}

#[derive(Default)]
struct ReplayCountingInboundTurnService {
    replay_attempts: Mutex<usize>,
    attempts: Mutex<usize>,
    accepted: Mutex<Vec<ProductInboundEnvelope>>,
}

impl ReplayCountingInboundTurnService {
    fn replay_attempt_count(&self) -> usize {
        *self
            .replay_attempts
            .lock()
            .expect("replay counter lock poisoned")
    }

    fn attempt_count(&self) -> usize {
        *self.attempts.lock().expect("attempt counter lock poisoned")
    }

    fn accepted_envelopes(&self) -> Vec<ProductInboundEnvelope> {
        self.accepted
            .lock()
            .expect("accepted envelopes lock poisoned")
            .clone()
    }

    fn accept_fresh_user_message(
        &self,
        envelope: &ProductInboundEnvelope,
    ) -> Result<InboundTurnOutcome, ProductWorkflowError> {
        *self.attempts.lock().expect("attempt counter lock poisoned") += 1;
        self.accepted
            .lock()
            .expect("accepted envelopes lock poisoned")
            .push(envelope.clone());
        Ok(InboundTurnOutcome::Submitted {
            accepted_message_ref: AcceptedMessageRef::new(format!(
                "msg:{}",
                envelope.external_event_id()
            ))
            .expect("valid accepted message ref"),
            submitted_run_id: TurnRunId::new(),
            binding: fake_binding(),
        })
    }
}

#[async_trait]
impl InboundTurnService for ReplayCountingInboundTurnService {
    async fn replay_accepted_user_message(
        &self,
        _envelope: &ProductInboundEnvelope,
    ) -> Result<Option<InboundTurnOutcome>, ProductWorkflowError> {
        *self
            .replay_attempts
            .lock()
            .expect("replay counter lock poisoned") += 1;
        Ok(None)
    }

    async fn accept_user_message(
        &self,
        envelope: &ProductInboundEnvelope,
    ) -> Result<InboundTurnOutcome, ProductWorkflowError> {
        if let Some(outcome) = self.replay_accepted_user_message(envelope).await? {
            return Ok(outcome);
        }
        self.accept_fresh_user_message(envelope)
    }

    async fn accept_user_message_with_before_policy(
        &self,
        envelope: &ProductInboundEnvelope,
        before_inbound_policy: &dyn BeforeInboundPolicy,
    ) -> Result<InboundUserMessageDispatch, ProductWorkflowError> {
        if let Some(outcome) = self.replay_accepted_user_message(envelope).await? {
            return Ok(InboundUserMessageDispatch::Accepted(outcome));
        }

        let ProductInboundPayload::UserMessage(payload) = envelope.payload() else {
            return Err(ProductWorkflowError::UnsupportedActionKind {
                kind: "non_user_message".into(),
            });
        };
        let policy_outcome = before_inbound_policy
            .check_user_message(BeforeInboundPolicyRequest::new(envelope, payload)?)
            .await?;
        let dispatch_envelope;
        let envelope_for_turn = match policy_outcome {
            BeforeInboundPolicyOutcome::Allow => envelope,
            BeforeInboundPolicyOutcome::RewriteUserMessage(payload) => {
                dispatch_envelope =
                    envelope.with_rewritten_user_message(payload).map_err(|_| {
                        ProductWorkflowError::TurnSubmissionRejected {
                            reason: "invalid policy-rewritten user message".into(),
                        }
                    })?;
                &dispatch_envelope
            }
            BeforeInboundPolicyOutcome::Reject(rejection) => {
                return Ok(InboundUserMessageDispatch::Rejected(rejection));
            }
            _ => {
                return Err(ProductWorkflowError::Transient {
                    reason: "unsupported before-inbound policy outcome".into(),
                });
            }
        };

        self.accept_fresh_user_message(envelope_for_turn)
            .map(InboundUserMessageDispatch::Accepted)
    }
}

fn fingerprint_actor() -> ExternalActorRef {
    ExternalActorRef::new("test", "user1", Option::<String>::None).expect("valid actor")
}

fn build_workflow() -> (
    DefaultProductWorkflow,
    Arc<FakeInboundTurnService>,
    Arc<FakeIdempotencyLedger>,
) {
    let inbound = Arc::new(FakeInboundTurnService::new());
    let ledger = Arc::new(FakeIdempotencyLedger::new());
    let binding = Arc::new(FakeConversationBindingService::new());
    let workflow = DefaultProductWorkflow::new(inbound.clone(), ledger.clone(), binding);
    (workflow, inbound, ledger)
}

fn build_workflow_with_policy() -> (
    DefaultProductWorkflow,
    Arc<FakeInboundTurnService>,
    Arc<FakeIdempotencyLedger>,
    Arc<FakeBeforeInboundPolicy>,
) {
    let inbound = Arc::new(FakeInboundTurnService::new());
    let ledger = Arc::new(FakeIdempotencyLedger::new());
    let binding = Arc::new(FakeConversationBindingService::new());
    let policy = Arc::new(FakeBeforeInboundPolicy::new());
    let workflow = DefaultProductWorkflow::new(inbound.clone(), ledger.clone(), binding)
        .with_before_inbound_policy(policy.clone());
    (workflow, inbound, ledger, policy)
}

fn build_workflow_with_binding() -> (
    DefaultProductWorkflow,
    Arc<FakeInboundTurnService>,
    Arc<FakeIdempotencyLedger>,
    Arc<FakeConversationBindingService>,
) {
    let inbound = Arc::new(FakeInboundTurnService::new());
    let ledger = Arc::new(FakeIdempotencyLedger::new());
    let binding = Arc::new(FakeConversationBindingService::new());
    let workflow = DefaultProductWorkflow::new(inbound.clone(), ledger.clone(), binding.clone());
    (workflow, inbound, ledger, binding)
}

#[tokio::test]
async fn user_message_dispatches_through_inbound_turn_service() {
    let (workflow, inbound, ledger) = build_workflow();
    let envelope = sample_envelope("1");

    let ack = workflow.accept_inbound(envelope).await.expect("accept");

    assert!(matches!(ack, ProductInboundAck::Accepted { .. }));
    assert_eq!(inbound.accepted_count(), 1);
    assert_eq!(ledger.settled_count(), 1);
}

#[tokio::test]
async fn approval_resolution_payload_routes_through_approval_interaction_service() {
    let inbound = Arc::new(FakeInboundTurnService::new());
    let ledger = Arc::new(FakeIdempotencyLedger::new());
    let binding = Arc::new(FakeConversationBindingService::new());
    let gate_ref = approval_gate_ref(ApprovalRequestId::new()).expect("approval gate ref");
    let run_id = TurnRunId::new();
    let approval_service = Arc::new(RecordingApprovalInteractionService::new(
        gate_ref.clone(),
        run_id,
    ));
    let workflow = DefaultProductWorkflow::new(inbound, ledger, binding)
        .with_approval_interaction_service(approval_service.clone());
    let envelope = sample_envelope_with_payload(
        "approval-resolution",
        ProductInboundPayload::ApprovalResolution(
            ApprovalResolutionPayload::new(gate_ref.as_str(), ApprovalDecision::ApproveOnce)
                .expect("approval payload"),
        ),
    );

    let ack = workflow.accept_inbound(envelope).await.expect("accept");

    assert_eq!(ack, ProductInboundAck::NoOp);
    let resolutions = approval_service.resolutions();
    assert_eq!(resolutions.len(), 1);
    assert_eq!(resolutions[0].gate_ref, gate_ref);
    assert_eq!(resolutions[0].run_id_hint, None);
    assert!(
        resolutions[0]
            .idempotency_key
            .as_str()
            .contains("approval-resolution")
    );
    assert_eq!(
        resolutions[0].decision,
        ApprovalInteractionDecision::ApproveOnce
    );
}

#[tokio::test]
async fn approval_resolution_idempotency_key_is_stable_for_same_external_event() {
    let gate_ref = approval_gate_ref(ApprovalRequestId::new()).expect("approval gate ref");
    let build = || {
        let inbound = Arc::new(FakeInboundTurnService::new());
        let ledger = Arc::new(FakeIdempotencyLedger::new());
        let binding = Arc::new(FakeConversationBindingService::new());
        let approval_service = Arc::new(RecordingApprovalInteractionService::new(
            gate_ref.clone(),
            TurnRunId::new(),
        ));
        let workflow = DefaultProductWorkflow::new(inbound, ledger, binding)
            .with_approval_interaction_service(approval_service.clone());
        (workflow, approval_service)
    };
    let envelope = || {
        sample_envelope_with_payload(
            "approval-resolution-stable",
            ProductInboundPayload::ApprovalResolution(
                ApprovalResolutionPayload::new(gate_ref.as_str(), ApprovalDecision::ApproveOnce)
                    .expect("approval payload"),
            ),
        )
    };
    let (workflow_a, approval_a) = build();
    let (workflow_b, approval_b) = build();

    workflow_a
        .accept_inbound(envelope())
        .await
        .expect("first accept");
    workflow_b
        .accept_inbound(envelope())
        .await
        .expect("second accept");

    assert_eq!(
        approval_a.resolutions()[0].idempotency_key,
        approval_b.resolutions()[0].idempotency_key
    );
}

#[tokio::test]
async fn approval_resolution_idempotency_key_ignores_actor_display_name() {
    let gate_ref = approval_gate_ref(ApprovalRequestId::new()).expect("approval gate ref");
    let build = || {
        let inbound = Arc::new(FakeInboundTurnService::new());
        let ledger = Arc::new(FakeIdempotencyLedger::new());
        let binding = Arc::new(FakeConversationBindingService::new());
        let approval_service = Arc::new(RecordingApprovalInteractionService::new(
            gate_ref.clone(),
            TurnRunId::new(),
        ));
        let workflow = DefaultProductWorkflow::new(inbound, ledger, binding)
            .with_approval_interaction_service(approval_service.clone());
        (workflow, approval_service)
    };
    let envelope = |display_name| {
        sample_envelope_with_context(
            ProductAdapterId::new("test_adapter").expect("valid"),
            AdapterInstallationId::new("install_alpha").expect("valid"),
            ExternalEventId::new("evt:approval-display-name").expect("valid"),
            ExternalActorRef::new("test", "user1", Some(display_name)).expect("valid actor"),
            ExternalConversationRef::new(None, "conv1", None, None).expect("valid conversation"),
            ProductInboundPayload::ApprovalResolution(
                ApprovalResolutionPayload::new(gate_ref.as_str(), ApprovalDecision::ApproveOnce)
                    .expect("approval payload"),
            ),
        )
    };
    let (workflow_a, approval_a) = build();
    let (workflow_b, approval_b) = build();

    workflow_a
        .accept_inbound(envelope("Alice A."))
        .await
        .expect("first accept");
    workflow_b
        .accept_inbound(envelope("Alice B."))
        .await
        .expect("second accept");

    assert_eq!(
        approval_a.resolutions()[0].idempotency_key,
        approval_b.resolutions()[0].idempotency_key
    );
}

#[tokio::test]
async fn approval_resolution_deny_routes_through_approval_interaction_service() {
    let inbound = Arc::new(FakeInboundTurnService::new());
    let ledger = Arc::new(FakeIdempotencyLedger::new());
    let binding = Arc::new(FakeConversationBindingService::new());
    let gate_ref = approval_gate_ref(ApprovalRequestId::new()).expect("approval gate ref");
    let approval_service = Arc::new(RecordingApprovalInteractionService::new(
        gate_ref.clone(),
        TurnRunId::new(),
    ));
    let workflow = DefaultProductWorkflow::new(inbound, ledger, binding)
        .with_approval_interaction_service(approval_service.clone());
    let envelope = sample_envelope_with_payload(
        "approval-deny",
        ProductInboundPayload::ApprovalResolution(
            ApprovalResolutionPayload::new(gate_ref.as_str(), ApprovalDecision::Deny)
                .expect("approval payload"),
        ),
    );

    let ack = workflow.accept_inbound(envelope).await.expect("accept");

    assert_eq!(ack, ProductInboundAck::NoOp);
    let resolutions = approval_service.resolutions();
    assert_eq!(resolutions.len(), 1);
    assert_eq!(resolutions[0].gate_ref, gate_ref);
    assert_eq!(resolutions[0].run_id_hint, None);
    assert_eq!(resolutions[0].decision, ApprovalInteractionDecision::Deny);
}

#[tokio::test]
async fn approval_resolution_always_allow_is_rejected_without_approval_interaction() {
    let inbound = Arc::new(FakeInboundTurnService::new());
    let ledger = Arc::new(FakeIdempotencyLedger::new());
    let binding = Arc::new(FakeConversationBindingService::new());
    let gate_ref = approval_gate_ref(ApprovalRequestId::new()).expect("approval gate ref");
    let approval_service = Arc::new(RecordingApprovalInteractionService::new(
        gate_ref.clone(),
        TurnRunId::new(),
    ));
    let workflow = DefaultProductWorkflow::new(inbound, ledger, binding)
        .with_approval_interaction_service(approval_service.clone());
    let envelope = sample_envelope_with_payload(
        "approval-always-allow",
        ProductInboundPayload::ApprovalResolution(
            ApprovalResolutionPayload::new(gate_ref.as_str(), ApprovalDecision::AlwaysAllow)
                .expect("approval payload"),
        ),
    );

    let err = workflow
        .accept_inbound(envelope)
        .await
        .expect_err("always allow unsupported");

    assert!(matches!(
        err,
        ProductAdapterError::WorkflowRejected {
            kind: ironclaw_product_adapters::ProductWorkflowRejectionKind::InvalidRequest,
            status_code: 400,
            retryable: false,
            ..
        }
    ));
    assert!(approval_service.resolutions().is_empty());
}

#[tokio::test]
async fn approval_resolution_without_interaction_service_returns_retryable_unavailable() {
    let inbound = Arc::new(FakeInboundTurnService::new());
    let ledger = Arc::new(FakeIdempotencyLedger::new());
    let binding = Arc::new(FakeConversationBindingService::new());
    let workflow = DefaultProductWorkflow::new(inbound, ledger, binding);
    let gate_ref = approval_gate_ref(ApprovalRequestId::new()).expect("approval gate ref");
    let envelope = sample_envelope_with_payload(
        "approval-unwired",
        ProductInboundPayload::ApprovalResolution(
            ApprovalResolutionPayload::new(gate_ref.as_str(), ApprovalDecision::ApproveOnce)
                .expect("approval payload"),
        ),
    );

    let err = workflow
        .accept_inbound(envelope)
        .await
        .expect_err("unwired approval service");

    assert!(matches!(
        err,
        ProductAdapterError::WorkflowRejected {
            kind: ProductWorkflowRejectionKind::Unavailable,
            status_code: 503,
            retryable: true,
            ..
        }
    ));
}

#[tokio::test]
async fn before_inbound_policy_rewrite_reaches_inbound_turn_service() {
    let (workflow, inbound, ledger, policy) = build_workflow_with_policy();
    policy.rewrite_user_message(
        UserMessagePayload::new(
            "rewritten by policy",
            vec![],
            ProductTriggerReason::DirectChat,
        )
        .expect("valid rewrite"),
    );
    let envelope = sample_envelope("policy-rewrite");

    let ack = workflow.accept_inbound(envelope).await.expect("accept");

    assert!(matches!(ack, ProductInboundAck::Accepted { .. }));
    assert_eq!(policy.request_count(), 1);
    let request = &policy.requests()[0];
    assert_eq!(request.adapter_id.as_str(), "test_adapter");
    assert_eq!(request.installation_id.as_str(), "install_alpha");
    assert_eq!(request.user_message.text, "hello");
    assert_eq!(request.external_actor_ref.id(), "user1");
    assert_eq!(
        request.external_conversation_ref.conversation_fingerprint(),
        "space:0:;conversation:5:conv1;topic:0:;"
    );
    assert_eq!(
        request.source_binding_key.as_str(),
        "space:0:;conversation:5:conv1;topic:0:;"
    );
    assert_eq!(
        request.rate_limit_key.as_str(),
        "space:0:;conversation:5:conv1;topic:0:;"
    );
    let accepted = inbound.accepted_envelopes();
    assert_eq!(accepted.len(), 1);
    let ProductInboundPayload::UserMessage(payload) = accepted[0].payload() else {
        panic!("expected rewritten user message payload")
    };
    assert_eq!(payload.text, "rewritten by policy");
    assert_eq!(ledger.settled_count(), 1);
}

#[tokio::test]
async fn before_inbound_policy_path_probes_replay_once() {
    let inbound = Arc::new(ReplayCountingInboundTurnService::default());
    let ledger = Arc::new(FakeIdempotencyLedger::new());
    let binding = Arc::new(FakeConversationBindingService::new());
    let policy = Arc::new(FakeBeforeInboundPolicy::new());
    policy.rewrite_user_message(
        UserMessagePayload::new("rewritten once", vec![], ProductTriggerReason::DirectChat)
            .expect("valid rewrite"),
    );
    let workflow = DefaultProductWorkflow::new(inbound.clone(), ledger.clone(), binding)
        .with_before_inbound_policy(policy.clone());
    let envelope = sample_envelope("policy-replay-once");

    let ack = workflow.accept_inbound(envelope).await.expect("accept");

    assert!(matches!(ack, ProductInboundAck::Accepted { .. }));
    assert_eq!(inbound.replay_attempt_count(), 1);
    assert_eq!(inbound.attempt_count(), 1);
    let accepted = inbound.accepted_envelopes();
    let ProductInboundPayload::UserMessage(payload) = accepted[0].payload() else {
        panic!("expected rewritten user message payload")
    };
    assert_eq!(payload.text, "rewritten once");
    assert_eq!(policy.request_count(), 1);
    assert_eq!(ledger.settled_count(), 1);
}

#[tokio::test]
async fn before_inbound_policy_rewrite_revalidates_payload_before_turn_path() {
    let (workflow, inbound, ledger, policy) = build_workflow_with_policy();
    policy.rewrite_user_message(UserMessagePayload {
        text: "a".repeat(64 * 1024 + 1),
        attachments: vec![],
        trigger: ProductTriggerReason::DirectChat,
    });
    let envelope = sample_envelope("policy-rewrite-invalid");

    let err = workflow
        .accept_inbound(envelope)
        .await
        .expect_err("invalid policy rewrite should fail before staging");

    assert!(!err.is_retryable());
    assert_eq!(policy.request_count(), 1);
    assert_eq!(inbound.accepted_count(), 0);
    assert_eq!(ledger.settled_count(), 0);
    assert_eq!(ledger.in_flight_count(), 0);
}

#[tokio::test]
async fn before_inbound_policy_rejection_skips_transcript_and_turn_path() {
    let (workflow, inbound, ledger, policy) = build_workflow_with_policy();
    policy.reject(ProductRejection::permanent(
        ProductRejectionKind::PolicyDenied,
        "blocked by before-inbound policy",
    ));
    let envelope = sample_envelope("policy-reject");

    let ack = workflow
        .accept_inbound(envelope.clone())
        .await
        .expect("policy rejection ack");

    let ProductInboundAck::Rejected(rejection) = ack else {
        panic!("expected rejected ack")
    };
    assert_eq!(rejection.kind, ProductRejectionKind::PolicyDenied);
    assert_eq!(
        rejection.disposition(),
        ProductRejectionDisposition::Permanent
    );
    assert_eq!(policy.request_count(), 1);
    assert_eq!(inbound.accepted_count(), 0);
    assert_eq!(ledger.settled_count(), 1);
    let actions = ledger.settled_actions();
    assert_eq!(
        actions[0].dispatch_kind,
        Some(ActionDispatchKind::Rejected {
            kind: ProductRejectionKind::PolicyDenied
        })
    );

    let replay = workflow
        .accept_inbound(envelope)
        .await
        .expect("policy rejection replay");
    assert!(matches!(replay, ProductInboundAck::Duplicate { .. }));
    assert_eq!(policy.request_count(), 1);
    assert_eq!(inbound.accepted_count(), 0);
}

#[tokio::test]
async fn before_inbound_policy_retryable_rejection_releases_fingerprint() {
    let (workflow, inbound, ledger, policy) = build_workflow_with_policy();
    policy.reject(ProductRejection::retryable(
        ProductRejectionKind::PolicyDenied,
        "transient policy refusal",
    ));
    let envelope = sample_envelope("policy-reject-retryable");

    let first = workflow
        .accept_inbound(envelope.clone())
        .await
        .expect("retryable rejection still returns an ack");
    let ProductInboundAck::Rejected(rejection) = first else {
        panic!("expected rejected ack")
    };
    assert_eq!(
        rejection.disposition(),
        ProductRejectionDisposition::Retryable
    );
    assert_eq!(policy.request_count(), 1);
    assert_eq!(inbound.accepted_count(), 0);
    assert_eq!(ledger.settled_count(), 0);
    assert_eq!(ledger.in_flight_count(), 0);
    assert_eq!(ledger.released_count(), 1);
    assert!(
        ledger
            .last_released_action()
            .expect("released action")
            .dispatch_kind
            .is_none()
    );

    // Re-submitting the same envelope must re-invoke the policy (no duplicate
    // replay caching), because retryable rejections release the fingerprint.
    let second = workflow
        .accept_inbound(envelope)
        .await
        .expect("released fingerprint should let retryable rejection re-run policy");
    assert!(matches!(second, ProductInboundAck::Rejected(_)));
    assert_eq!(policy.request_count(), 2);
    assert_eq!(inbound.accepted_count(), 0);
    assert_eq!(ledger.settled_count(), 0);
}

#[tokio::test]
async fn before_inbound_policy_rewrite_replays_rewritten_outcome_on_duplicate() {
    let (workflow, inbound, ledger, policy) = build_workflow_with_policy();
    policy.rewrite_user_message(
        UserMessagePayload::new(
            "rewritten by policy",
            vec![],
            ProductTriggerReason::DirectChat,
        )
        .expect("valid rewrite"),
    );
    let envelope = sample_envelope("policy-rewrite-dup");

    let first = workflow
        .accept_inbound(envelope.clone())
        .await
        .expect("first accept");
    let ProductInboundAck::Accepted {
        submitted_run_id: first_run,
        ..
    } = first
    else {
        panic!("expected accepted ack on first dispatch")
    };
    assert_eq!(policy.request_count(), 1);
    assert_eq!(inbound.accepted_count(), 1);
    assert_eq!(ledger.settled_count(), 1);

    let replay = workflow
        .accept_inbound(envelope)
        .await
        .expect("duplicate replay");
    let ProductInboundAck::Duplicate { prior } = replay else {
        panic!("expected duplicate ack on replay")
    };
    let ProductInboundAck::Accepted {
        submitted_run_id: prior_run,
        ..
    } = *prior
    else {
        panic!("expected replayed prior accepted ack")
    };
    assert_eq!(prior_run, first_run);
    // Policy and inbound must NOT be re-invoked on duplicate replay.
    assert_eq!(policy.request_count(), 1);
    assert_eq!(inbound.accepted_count(), 1);
}

#[tokio::test]
async fn before_inbound_policy_does_not_block_staged_deferred_retry() {
    let (workflow, inbound, ledger, policy) = build_workflow_with_policy();
    policy.allow();
    let accepted_message_ref = AcceptedMessageRef::new("msg:policy-busy").expect("valid msg ref");
    let busy_run = TurnRunId::new();
    inbound.program_outcome(InboundTurnOutcome::DeferredBusy {
        accepted_message_ref: accepted_message_ref.clone(),
        active_run_id: busy_run,
        binding: fake_binding(),
    });
    let envelope = sample_envelope("policy-busy-retry");

    let first = workflow
        .accept_inbound(envelope.clone())
        .await
        .expect("busy ack");
    assert!(matches!(first, ProductInboundAck::DeferredBusy { .. }));
    assert_eq!(policy.request_count(), 1);
    assert_eq!(inbound.attempt_count(), 1);
    assert_eq!(ledger.settled_count(), 0);
    assert_eq!(ledger.in_flight_count(), 0);

    let submitted_run_id = TurnRunId::new();
    inbound.program_replay_outcome(InboundTurnOutcome::Submitted {
        accepted_message_ref,
        submitted_run_id,
        binding: fake_binding(),
    });
    policy.reject(ProductRejection::permanent(
        ProductRejectionKind::PolicyDenied,
        "policy changed after message was staged",
    ));

    let second = workflow
        .accept_inbound(envelope)
        .await
        .expect("staged message replay should bypass policy");
    assert!(matches!(
        second,
        ProductInboundAck::Accepted {
            submitted_run_id: run_id,
            ..
        } if run_id == submitted_run_id
    ));
    assert_eq!(policy.request_count(), 1);
    assert_eq!(inbound.replay_attempt_count(), 2);
    assert_eq!(inbound.attempt_count(), 1);
    assert_eq!(ledger.settled_count(), 1);
}

#[tokio::test]
async fn before_inbound_policy_transient_failure_releases_fingerprint() {
    let (workflow, inbound, ledger, policy) = build_workflow_with_policy();
    policy.force_failure(ProductWorkflowError::Transient {
        reason: "policy store unavailable".into(),
    });
    let envelope = sample_envelope("policy-transient");

    let first = workflow
        .accept_inbound(envelope.clone())
        .await
        .expect_err("policy failure should be retryable");
    assert!(first.is_retryable());
    assert_eq!(policy.request_count(), 1);
    assert_eq!(inbound.accepted_count(), 0);
    assert_eq!(ledger.settled_count(), 0);
    assert_eq!(ledger.in_flight_count(), 0);

    let second = workflow
        .accept_inbound(envelope)
        .await
        .expect_err("released fingerprint should retry policy");
    assert!(second.is_retryable());
    assert_eq!(policy.request_count(), 2);
    assert_eq!(inbound.accepted_count(), 0);
}

#[tokio::test]
async fn before_inbound_policy_retryable_failure_releases_fingerprint() {
    let (workflow, inbound, ledger, policy) = build_workflow_with_policy();
    policy.force_failure(ProductWorkflowError::BeforeInboundPolicyFailed {
        reason: "policy cache miss".into(),
        permanent: false,
    });
    let envelope = sample_envelope("policy-retryable-failure");

    let first = workflow
        .accept_inbound(envelope.clone())
        .await
        .expect_err("retryable policy failure should release fingerprint");
    assert!(first.is_retryable());
    assert_eq!(policy.request_count(), 1);
    assert_eq!(inbound.accepted_count(), 0);
    assert_eq!(ledger.settled_count(), 0);
    assert_eq!(ledger.in_flight_count(), 0);

    let second = workflow
        .accept_inbound(envelope)
        .await
        .expect_err("released fingerprint should retry policy");
    assert!(second.is_retryable());
    assert_eq!(policy.request_count(), 2);
    assert_eq!(inbound.accepted_count(), 0);
}

#[tokio::test]
async fn before_inbound_policy_timeout_releases_fingerprint_for_retry() {
    let (workflow, inbound, ledger, policy) = build_workflow_with_policy();
    policy.delay_responses_by(StdDuration::from_millis(200));
    let envelope = sample_envelope("policy-timeout-release");

    let first = workflow
        .accept_inbound(envelope.clone())
        .await
        .expect_err("timed-out policy should be retryable");
    assert!(first.is_retryable());
    assert_eq!(policy.request_count(), 1);
    assert_eq!(inbound.accepted_count(), 0);
    assert_eq!(ledger.settled_count(), 0);
    assert_eq!(ledger.in_flight_count(), 0);
    assert_eq!(ledger.released_count(), 1);
    assert!(
        ledger
            .last_released_action()
            .expect("released action")
            .dispatch_kind
            .is_none()
    );

    let second = workflow
        .accept_inbound(envelope)
        .await
        .expect_err("released fingerprint should retry timed-out policy");
    assert!(second.is_retryable());
    assert_eq!(policy.request_count(), 2);
    assert_eq!(inbound.accepted_count(), 0);
    assert_eq!(ledger.settled_count(), 0);
    assert_eq!(ledger.in_flight_count(), 0);
    assert_eq!(ledger.released_count(), 2);
}

#[tokio::test]
async fn before_inbound_policy_permanent_failure_settles_terminal_rejection() {
    let (workflow, inbound, ledger, policy) = build_workflow_with_policy();
    policy.force_failure(ProductWorkflowError::BeforeInboundPolicyFailed {
        reason: "policy configuration is invalid".into(),
        permanent: true,
    });
    let envelope = sample_envelope("policy-permanent-failure");

    let err = workflow
        .accept_inbound(envelope.clone())
        .await
        .expect_err("permanent policy failure should surface rejected error");
    assert!(!err.is_retryable());
    assert_eq!(policy.request_count(), 1);
    assert_eq!(inbound.accepted_count(), 0);
    assert_eq!(ledger.settled_count(), 1);
    let actions = ledger.settled_actions();
    assert_eq!(
        actions[0].dispatch_kind,
        Some(ActionDispatchKind::Rejected {
            kind: ProductRejectionKind::PolicyDenied
        })
    );

    let replay = workflow
        .accept_inbound(envelope)
        .await
        .expect("terminal policy failure should replay duplicate ack");
    let ProductInboundAck::Duplicate { prior } = replay else {
        panic!("expected duplicate replay")
    };
    let ProductInboundAck::Rejected(rejection) = *prior else {
        panic!("expected rejected prior outcome")
    };
    assert_eq!(rejection.kind, ProductRejectionKind::PolicyDenied);
    assert_eq!(
        rejection.disposition(),
        ProductRejectionDisposition::Permanent
    );
    let rejection_debug = format!("{rejection:?}");
    assert!(
        !rejection_debug.contains("policy configuration is invalid"),
        "durable rejection ack must not expose raw policy internals: {rejection_debug}"
    );
    assert!(
        rejection_debug.contains("<redacted>"),
        "durable rejection reason should remain redacted: {rejection_debug}"
    );
}

#[tokio::test]
async fn fake_before_inbound_policy_uses_programmed_outcomes_in_order() {
    let policy = FakeBeforeInboundPolicy::new();
    let envelope = sample_envelope("fake-policy-sequence");
    let ProductInboundPayload::UserMessage(payload) = envelope.payload() else {
        panic!("expected user message")
    };
    policy.program_outcomes([
        Ok(BeforeInboundPolicyOutcome::RewriteUserMessage(
            UserMessagePayload::new("first", vec![], ProductTriggerReason::DirectChat)
                .expect("valid rewrite"),
        )),
        Ok(BeforeInboundPolicyOutcome::Reject(
            ProductRejection::retryable(ProductRejectionKind::PolicyDenied, "try later"),
        )),
    ]);
    policy.allow();

    let first = policy
        .check_user_message(BeforeInboundPolicyRequest::new(&envelope, payload).expect("request"))
        .await
        .expect("first policy result");
    assert!(matches!(
        first,
        BeforeInboundPolicyOutcome::RewriteUserMessage(rewritten) if rewritten.text == "first"
    ));

    let second = policy
        .check_user_message(BeforeInboundPolicyRequest::new(&envelope, payload).expect("request"))
        .await
        .expect("second policy result");
    assert!(matches!(second, BeforeInboundPolicyOutcome::Reject(_)));

    let third = policy
        .check_user_message(BeforeInboundPolicyRequest::new(&envelope, payload).expect("request"))
        .await
        .expect("fallback policy result");
    assert_eq!(third, BeforeInboundPolicyOutcome::Allow);
}

#[tokio::test]
async fn fake_inbound_turn_service_replays_programmed_outcomes_in_order() {
    let inbound = FakeInboundTurnService::new();
    let envelope = sample_envelope("fake-replay-sequence");
    let first_run = TurnRunId::new();
    let second_run = TurnRunId::new();
    inbound.program_replay_outcomes([
        InboundTurnOutcome::DeferredBusy {
            accepted_message_ref: AcceptedMessageRef::new("msg:first").expect("valid"),
            active_run_id: first_run,
            binding: fake_binding(),
        },
        InboundTurnOutcome::Submitted {
            accepted_message_ref: AcceptedMessageRef::new("msg:second").expect("valid"),
            submitted_run_id: second_run,
            binding: fake_binding(),
        },
    ]);

    let first = inbound
        .replay_accepted_user_message(&envelope)
        .await
        .expect("first replay")
        .expect("first programmed outcome");
    assert!(matches!(
        first,
        InboundTurnOutcome::DeferredBusy { active_run_id, .. } if active_run_id == first_run
    ));
    let second = inbound
        .replay_accepted_user_message(&envelope)
        .await
        .expect("second replay")
        .expect("second programmed outcome");
    assert!(matches!(
        second,
        InboundTurnOutcome::Submitted { submitted_run_id, .. } if submitted_run_id == second_run
    ));
    assert!(
        inbound
            .replay_accepted_user_message(&envelope)
            .await
            .expect("third replay")
            .is_none()
    );
}

#[tokio::test]
async fn noop_returns_noop_ack() {
    let (workflow, inbound, ledger) = build_workflow();
    let envelope = sample_noop_envelope("noop1");

    let ack = workflow.accept_inbound(envelope).await.expect("accept");

    assert!(matches!(ack, ProductInboundAck::NoOp));
    assert_eq!(inbound.accepted_count(), 0);
    assert_eq!(ledger.settled_count(), 1);
}

#[tokio::test]
async fn projection_subscription_resolves_through_binding_service() {
    let (workflow, inbound, _ledger, binding_service) = build_workflow_with_binding();
    let binding = fake_binding();
    let cursor = ProjectionCursor::new("cursor:projection-1").expect("valid cursor");
    let envelope = sample_envelope_with_payload(
        "projection-1",
        ProductInboundPayload::SubscriptionRequest(
            ProjectionSubscriptionPayload::new(
                Some(binding.thread_id.as_str().to_string()),
                Some(cursor.clone()),
            )
            .expect("valid subscription"),
        ),
    );
    binding_service.program_binding(envelope.source_binding_key(), binding.clone());

    let subscription = workflow
        .resolve_projection_subscription(envelope)
        .await
        .expect("projection subscription");

    assert_eq!(subscription.actor.user_id, binding.user_id);
    assert_eq!(subscription.scope.tenant_id, binding.tenant_id);
    assert_eq!(subscription.scope.agent_id, binding.agent_id);
    assert_eq!(subscription.scope.project_id, binding.project_id);
    assert_eq!(subscription.scope.thread_id, binding.thread_id);
    assert_eq!(subscription.after_cursor, Some(cursor));
    assert_eq!(binding_service.resolve_count(), 1);
    assert_eq!(inbound.accepted_count(), 0);
}

#[tokio::test]
async fn projection_subscription_rejects_non_subscription_payload() {
    let (workflow, _inbound, _ledger, _binding_service) = build_workflow_with_binding();

    let err = workflow
        .resolve_projection_subscription(sample_envelope("projection-non-subscription"))
        .await
        .expect_err("non-subscription payload rejects");

    assert!(matches!(
        err,
        ProductAdapterError::MalformedInboundPayload { .. }
    ));
}

#[tokio::test]
async fn projection_subscription_rejects_mismatched_thread_hint() {
    let (workflow, _inbound, _ledger, binding_service) = build_workflow_with_binding();
    let binding = fake_binding();
    let envelope = sample_envelope_with_payload(
        "projection-mismatch",
        ProductInboundPayload::SubscriptionRequest(
            ProjectionSubscriptionPayload::new(Some("thread:other".into()), None)
                .expect("valid subscription"),
        ),
    );
    binding_service.program_binding(envelope.source_binding_key(), binding);

    let err = workflow
        .resolve_projection_subscription(envelope)
        .await
        .expect_err("mismatched hint rejects");

    match err {
        ProductAdapterError::WorkflowRejected {
            kind,
            status_code,
            retryable,
            ..
        } => {
            assert_eq!(kind, ProductWorkflowRejectionKind::InvalidRequest);
            assert_eq!(status_code, 400);
            assert!(!retryable);
        }
        other => panic!("expected workflow rejection, got {other:?}"),
    }
}

#[tokio::test]
async fn projection_subscription_rejects_malformed_thread_hint() {
    let (workflow, _inbound, _ledger, binding_service) = build_workflow_with_binding();
    let binding = fake_binding();
    let envelope = sample_envelope_with_payload(
        "projection-malformed-hint",
        ProductInboundPayload::SubscriptionRequest(
            ProjectionSubscriptionPayload::new(Some("thread/invalid".into()), None)
                .expect("adapter accepts opaque hint"),
        ),
    );
    binding_service.program_binding(envelope.source_binding_key(), binding);

    let err = workflow
        .resolve_projection_subscription(envelope)
        .await
        .expect_err("malformed hint rejects");

    assert!(matches!(
        err,
        ProductAdapterError::MalformedInboundPayload { .. }
    ));
}

#[tokio::test]
async fn projection_subscription_requires_existing_conversation_binding() {
    let conversations = Arc::new(InMemoryConversationServices::default());
    conversations
        .pair_external_actor(
            TenantId::new("tenant:alpha").expect("tenant"),
            ironclaw_conversations::AdapterKind::new("test_adapter").expect("adapter"),
            ironclaw_conversations::AdapterInstallationId::new("install_alpha").expect("install"),
            ironclaw_conversations::ExternalActorRef::new("test", "user1").expect("actor"),
            UserId::new("user:alice").expect("user"),
        )
        .await;
    let binding = product_binding_service(
        conversations,
        vec![(
            "test_adapter",
            "install_alpha",
            "tenant:alpha",
            "agent:alpha",
            Some("project:alpha"),
        )],
    );
    let workflow = DefaultProductWorkflow::new(
        Arc::new(FakeInboundTurnService::new()),
        Arc::new(InMemoryIdempotencyLedger::new()),
        Arc::new(binding),
    );
    let envelope = sample_envelope_with_payload(
        "projection-missing-binding",
        ProductInboundPayload::SubscriptionRequest(
            ProjectionSubscriptionPayload::new(None, None).expect("valid subscription"),
        ),
    );

    let err = workflow
        .resolve_projection_subscription(envelope)
        .await
        .expect_err("subscription must not create a missing binding");

    assert!(matches!(
        err,
        ProductAdapterError::WorkflowRejected {
            kind: ProductWorkflowRejectionKind::ScopeNotFound,
            status_code: 404,
            retryable: false,
            ..
        }
    ));
}

#[tokio::test]
async fn concrete_product_workflow_accepts_user_message_for_trusted_installation() {
    let conversations = Arc::new(InMemoryConversationServices::default());
    conversations
        .pair_external_actor(
            TenantId::new("tenant:alpha").expect("tenant"),
            ironclaw_conversations::AdapterKind::new("test_adapter").expect("adapter"),
            ironclaw_conversations::AdapterInstallationId::new("install_alpha").expect("install"),
            ironclaw_conversations::ExternalActorRef::new("test", "user1").expect("actor"),
            UserId::new("user:alice").expect("user"),
        )
        .await;
    let binding = product_binding_service(
        conversations,
        vec![(
            "test_adapter",
            "install_alpha",
            "tenant:alpha",
            "agent:alpha",
            Some("project:alpha"),
        )],
    );
    let coordinator = Arc::new(RecordingTurnCoordinator::default());
    let inbound = Arc::new(DefaultInboundTurnService::new(
        binding.clone(),
        InMemorySessionThreadService::default(),
        coordinator.clone(),
    ));
    let workflow = DefaultProductWorkflow::new(
        inbound,
        Arc::new(InMemoryIdempotencyLedger::new()),
        Arc::new(binding),
    );
    let envelope = sample_envelope("concrete-happy");

    let first = workflow
        .accept_inbound(envelope.clone())
        .await
        .expect("accepted");
    let duplicate = workflow
        .accept_inbound(envelope)
        .await
        .expect("duplicate replay");

    assert!(matches!(first, ProductInboundAck::Accepted { .. }));
    assert!(matches!(duplicate, ProductInboundAck::Duplicate { .. }));
    let submissions = coordinator.submissions();
    assert_eq!(submissions.len(), 1);
    assert_eq!(submissions[0].scope.tenant_id.as_str(), "tenant:alpha");
    assert_eq!(
        submissions[0].scope.agent_id.as_ref().map(AgentId::as_str),
        Some("agent:alpha")
    );
    assert_eq!(
        submissions[0]
            .scope
            .project_id
            .as_ref()
            .map(ProjectId::as_str),
        Some("project:alpha")
    );
    assert_eq!(submissions[0].actor.user_id.as_str(), "user:alice");
}

#[tokio::test]
async fn concrete_product_workflow_accepts_shared_route_participant_on_existing_thread() {
    let tenant_id = TenantId::new("tenant:alpha").expect("tenant");
    let adapter_kind = ironclaw_conversations::AdapterKind::new("test_adapter").expect("adapter");
    let installation_id =
        ironclaw_conversations::AdapterInstallationId::new("install_alpha").expect("install");
    let conversations = Arc::new(InMemoryConversationServices::default());
    conversations
        .pair_external_actor(
            tenant_id.clone(),
            adapter_kind.clone(),
            installation_id.clone(),
            ironclaw_conversations::ExternalActorRef::new("test", "user1").expect("actor"),
            UserId::new("user:alice").expect("user"),
        )
        .await;
    conversations
        .pair_external_actor(
            tenant_id.clone(),
            adapter_kind,
            installation_id,
            ironclaw_conversations::ExternalActorRef::new("test", "user2").expect("actor"),
            UserId::new("user:bob").expect("user"),
        )
        .await;
    let binding = product_binding_service(
        conversations.clone(),
        vec![(
            "test_adapter",
            "install_alpha",
            "tenant:alpha",
            "agent:alpha",
            Some("project:alpha"),
        )],
    );
    let coordinator = Arc::new(RecordingTurnCoordinator::default());
    let inbound = Arc::new(DefaultInboundTurnService::new(
        binding.clone(),
        InMemorySessionThreadService::default(),
        coordinator.clone(),
    ));
    let workflow = DefaultProductWorkflow::new(
        inbound,
        Arc::new(InMemoryIdempotencyLedger::new()),
        Arc::new(binding),
    );

    workflow
        .accept_inbound(sample_envelope_with_payload(
            "shared-alice",
            ProductInboundPayload::UserMessage(
                UserMessagePayload::new("hello shared", vec![], ProductTriggerReason::BotMention)
                    .expect("message"),
            ),
        ))
        .await
        .expect("alice shared message accepted");
    let shared_thread_id = coordinator.submissions()[0].scope.thread_id.clone();
    conversations
        .add_thread_participant(
            &tenant_id,
            &shared_thread_id,
            UserId::new("user:bob").expect("user"),
        )
        .await
        .expect("bob participant added");

    workflow
        .accept_inbound(sample_envelope_with_context(
            ProductAdapterId::new("test_adapter").expect("adapter"),
            AdapterInstallationId::new("install_alpha").expect("install"),
            ExternalEventId::new("evt:shared-bob").expect("event"),
            ExternalActorRef::new("test", "user2", Option::<String>::None).expect("actor"),
            ExternalConversationRef::new(None, "conv1", None, None).expect("conversation"),
            ProductInboundPayload::UserMessage(
                UserMessagePayload::new("hello from bob", vec![], ProductTriggerReason::BotMention)
                    .expect("message"),
            ),
        ))
        .await
        .expect("shared participant accepted on existing thread");

    let submissions = coordinator.submissions();
    assert_eq!(submissions.len(), 2);
    assert_eq!(
        submissions[0].scope.thread_id,
        submissions[1].scope.thread_id
    );
    assert_eq!(submissions[0].actor.user_id.as_str(), "user:alice");
    assert_eq!(submissions[1].actor.user_id.as_str(), "user:bob");
}

#[tokio::test]
async fn concrete_product_workflow_persists_first_bind_default_scope() {
    let conversations = Arc::new(InMemoryConversationServices::default());
    conversations
        .pair_external_actor(
            TenantId::new("tenant:alpha").expect("tenant"),
            ironclaw_conversations::AdapterKind::new("test_adapter").expect("adapter"),
            ironclaw_conversations::AdapterInstallationId::new("install_alpha").expect("install"),
            ironclaw_conversations::ExternalActorRef::new("test", "user1").expect("actor"),
            UserId::new("user:alice").expect("user"),
        )
        .await;
    let binding_alpha = product_binding_service(
        conversations.clone(),
        vec![(
            "test_adapter",
            "install_alpha",
            "tenant:alpha",
            "agent:alpha",
            Some("project:alpha"),
        )],
    );
    let workflow_alpha = DefaultProductWorkflow::new(
        Arc::new(DefaultInboundTurnService::new(
            binding_alpha.clone(),
            InMemorySessionThreadService::default(),
            Arc::new(RecordingTurnCoordinator::default()),
        )),
        Arc::new(InMemoryIdempotencyLedger::new()),
        Arc::new(binding_alpha),
    );
    workflow_alpha
        .accept_inbound(sample_envelope("persisted-default-scope"))
        .await
        .expect("first bind accepted");

    let binding_beta = product_binding_service(
        conversations,
        vec![(
            "test_adapter",
            "install_alpha",
            "tenant:alpha",
            "agent:beta",
            Some("project:beta"),
        )],
    );
    let workflow_beta = DefaultProductWorkflow::new(
        Arc::new(FakeInboundTurnService::new()),
        Arc::new(InMemoryIdempotencyLedger::new()),
        Arc::new(binding_beta),
    );
    let subscription = workflow_beta
        .resolve_projection_subscription(sample_envelope_with_payload(
            "projection-existing-scope",
            ProductInboundPayload::SubscriptionRequest(
                ProjectionSubscriptionPayload::new(None, None).expect("valid subscription"),
            ),
        ))
        .await
        .expect("existing binding resolves");

    assert_eq!(
        subscription.scope.agent_id.as_ref().map(AgentId::as_str),
        Some("agent:alpha")
    );
    assert_eq!(
        subscription
            .scope
            .project_id
            .as_ref()
            .map(ProjectId::as_str),
        Some("project:alpha")
    );
}

#[tokio::test]
async fn concrete_product_workflow_keeps_installations_tenant_isolated() {
    let conversations = Arc::new(InMemoryConversationServices::default());
    for (install, tenant, user) in [
        ("install_alpha", "tenant:alpha", "user:alice"),
        ("install_beta", "tenant:beta", "user:bob"),
    ] {
        conversations
            .pair_external_actor(
                TenantId::new(tenant).expect("tenant"),
                ironclaw_conversations::AdapterKind::new("test_adapter").expect("adapter"),
                ironclaw_conversations::AdapterInstallationId::new(install).expect("install"),
                ironclaw_conversations::ExternalActorRef::new("test", "user1").expect("actor"),
                UserId::new(user).expect("user"),
            )
            .await;
    }
    let binding = product_binding_service(
        conversations,
        vec![
            (
                "test_adapter",
                "install_alpha",
                "tenant:alpha",
                "agent:alpha",
                None,
            ),
            (
                "test_adapter",
                "install_beta",
                "tenant:beta",
                "agent:beta",
                None,
            ),
        ],
    );
    let coordinator = Arc::new(RecordingTurnCoordinator::default());
    let inbound = Arc::new(DefaultInboundTurnService::new(
        binding.clone(),
        InMemorySessionThreadService::default(),
        coordinator.clone(),
    ));
    let workflow = DefaultProductWorkflow::new(
        inbound,
        Arc::new(InMemoryIdempotencyLedger::new()),
        Arc::new(binding),
    );

    workflow
        .accept_inbound(sample_envelope("tenant-a"))
        .await
        .expect("tenant a accepted");
    workflow
        .accept_inbound(sample_envelope_with_context(
            ProductAdapterId::new("test_adapter").expect("adapter"),
            AdapterInstallationId::new("install_beta").expect("install"),
            ExternalEventId::new("evt:tenant-b").expect("event"),
            ExternalActorRef::new("test", "user1", Option::<String>::None).expect("actor"),
            ExternalConversationRef::new(None, "conv1", None, None).expect("conversation"),
            ProductInboundPayload::UserMessage(
                UserMessagePayload::new("hello beta", vec![], ProductTriggerReason::DirectChat)
                    .expect("message"),
            ),
        ))
        .await
        .expect("tenant b accepted");

    let submissions = coordinator.submissions();
    assert_eq!(submissions.len(), 2);
    assert_eq!(submissions[0].scope.tenant_id.as_str(), "tenant:alpha");
    assert_eq!(submissions[0].actor.user_id.as_str(), "user:alice");
    assert_eq!(submissions[1].scope.tenant_id.as_str(), "tenant:beta");
    assert_eq!(submissions[1].actor.user_id.as_str(), "user:bob");
    assert_ne!(
        submissions[0].scope.thread_id,
        submissions[1].scope.thread_id
    );
}

#[tokio::test]
async fn concrete_product_workflow_bot_mention_uses_shared_route() {
    let binding = Arc::new(FakeConversationBindingService::new());
    let coordinator = Arc::new(RecordingTurnCoordinator::default());
    let inbound = Arc::new(DefaultInboundTurnService::new(
        binding.clone(),
        InMemorySessionThreadService::default(),
        coordinator.clone(),
    ));
    let workflow = DefaultProductWorkflow::new(
        inbound,
        Arc::new(InMemoryIdempotencyLedger::new()),
        binding.clone(),
    );

    workflow
        .accept_inbound(sample_envelope_with_payload(
            "shared-owner",
            ProductInboundPayload::UserMessage(
                UserMessagePayload::new("hello shared", vec![], ProductTriggerReason::BotMention)
                    .expect("message"),
            ),
        ))
        .await
        .expect("bot mention accepted");

    let submissions = coordinator.submissions();
    assert_eq!(submissions.len(), 1);
    assert_eq!(
        binding.route_kinds(),
        vec![ironclaw_product_workflow::ProductConversationRouteKind::Shared]
    );
}

#[tokio::test]
async fn concrete_product_workflow_reuses_prepared_binding_for_content_only_policy_rewrite() {
    let binding = Arc::new(FakeConversationBindingService::new());
    let coordinator = Arc::new(RecordingTurnCoordinator::default());
    let inbound = Arc::new(DefaultInboundTurnService::new(
        binding.clone(),
        InMemorySessionThreadService::default(),
        coordinator.clone(),
    ));
    let policy = Arc::new(FakeBeforeInboundPolicy::new());
    policy.rewrite_user_message(
        UserMessagePayload::new("rewritten direct", vec![], ProductTriggerReason::DirectChat)
            .expect("message"),
    );
    let workflow = DefaultProductWorkflow::new(
        inbound,
        Arc::new(InMemoryIdempotencyLedger::new()),
        binding.clone(),
    )
    .with_before_inbound_policy(policy);

    workflow
        .accept_inbound(sample_envelope_with_payload(
            "policy-rewrite-direct-route",
            ProductInboundPayload::UserMessage(
                UserMessagePayload::new("hello direct", vec![], ProductTriggerReason::DirectChat)
                    .expect("message"),
            ),
        ))
        .await
        .expect("policy-rewritten message accepted");

    let submissions = coordinator.submissions();
    assert_eq!(submissions.len(), 1);
    assert_eq!(
        submissions[0].scope.tenant_id.as_str(),
        "tenant:install_alpha"
    );
    assert_eq!(submissions[0].actor.user_id.as_str(), "user:user1");
    assert_eq!(
        submissions[0].scope.agent_id.as_ref().map(AgentId::as_str),
        Some("agent:fake")
    );
    assert_eq!(binding.resolve_count(), 1);
    assert_eq!(
        binding.route_kinds(),
        vec![ironclaw_product_workflow::ProductConversationRouteKind::Direct]
    );
}

#[tokio::test]
async fn concrete_product_workflow_recomputes_route_after_policy_rewrites_trigger() {
    let binding = Arc::new(FakeConversationBindingService::new());
    let coordinator = Arc::new(RecordingTurnCoordinator::default());
    let inbound = Arc::new(DefaultInboundTurnService::new(
        binding.clone(),
        InMemorySessionThreadService::default(),
        coordinator.clone(),
    ));
    let policy = Arc::new(FakeBeforeInboundPolicy::new());
    policy.rewrite_user_message(
        UserMessagePayload::new("rewritten shared", vec![], ProductTriggerReason::BotMention)
            .expect("message"),
    );
    let workflow = DefaultProductWorkflow::new(
        inbound,
        Arc::new(InMemoryIdempotencyLedger::new()),
        binding.clone(),
    )
    .with_before_inbound_policy(policy);

    workflow
        .accept_inbound(sample_envelope_with_payload(
            "policy-rewrite-shared-route",
            ProductInboundPayload::UserMessage(
                UserMessagePayload::new("hello direct", vec![], ProductTriggerReason::DirectChat)
                    .expect("message"),
            ),
        ))
        .await
        .expect("policy-rewritten message accepted");

    let submissions = coordinator.submissions();
    assert_eq!(submissions.len(), 1);
    assert_eq!(
        binding.route_kinds(),
        vec![
            ironclaw_product_workflow::ProductConversationRouteKind::Direct,
            ironclaw_product_workflow::ProductConversationRouteKind::Shared,
        ]
    );
}

#[tokio::test]
async fn concrete_product_workflow_rejects_unknown_installation_as_terminal() {
    let conversations = Arc::new(InMemoryConversationServices::default());
    conversations
        .pair_external_actor(
            TenantId::new("tenant:alpha").expect("tenant"),
            ironclaw_conversations::AdapterKind::new("test_adapter").expect("adapter"),
            ironclaw_conversations::AdapterInstallationId::new("install_alpha").expect("install"),
            ironclaw_conversations::ExternalActorRef::new("test", "user1").expect("actor"),
            UserId::new("user:alice").expect("user"),
        )
        .await;
    let binding = product_binding_service(conversations, vec![]);
    let coordinator = Arc::new(RecordingTurnCoordinator::default());
    let inbound = Arc::new(DefaultInboundTurnService::new(
        binding.clone(),
        InMemorySessionThreadService::default(),
        coordinator.clone(),
    ));
    let workflow = DefaultProductWorkflow::new(
        inbound,
        Arc::new(InMemoryIdempotencyLedger::new()),
        Arc::new(binding),
    );
    let envelope = sample_envelope("unknown-install");

    let err = workflow
        .accept_inbound(envelope.clone())
        .await
        .expect_err("unknown installation rejected");
    assert!(matches!(
        err,
        ProductAdapterError::WorkflowRejected {
            kind: ProductWorkflowRejectionKind::Unauthorized,
            status_code: 403,
            retryable: false,
            ..
        }
    ));
    let duplicate = workflow
        .accept_inbound(envelope)
        .await
        .expect("terminal rejection replays");
    assert!(matches!(duplicate, ProductInboundAck::Duplicate { .. }));
    assert!(coordinator.submissions().is_empty());
}

#[tokio::test]
async fn concrete_product_workflow_rejects_unpaired_actor_before_turn_submission() {
    let conversations = Arc::new(InMemoryConversationServices::default());
    let binding = product_binding_service(
        conversations,
        vec![(
            "test_adapter",
            "install_alpha",
            "tenant:alpha",
            "agent:alpha",
            None,
        )],
    );
    let coordinator = Arc::new(RecordingTurnCoordinator::default());
    let inbound = Arc::new(DefaultInboundTurnService::new(
        binding.clone(),
        InMemorySessionThreadService::default(),
        coordinator.clone(),
    ));
    let workflow = DefaultProductWorkflow::new(
        inbound,
        Arc::new(InMemoryIdempotencyLedger::new()),
        Arc::new(binding),
    );

    let envelope = sample_envelope("unpaired");
    let err = workflow
        .accept_inbound(envelope.clone())
        .await
        .expect_err("unpaired actor rejected");
    assert!(matches!(
        err,
        ProductAdapterError::WorkflowRejected {
            kind: ProductWorkflowRejectionKind::ScopeNotFound,
            status_code: 404,
            retryable: false,
            ..
        }
    ));
    assert!(coordinator.submissions().is_empty());

    let duplicate = workflow
        .accept_inbound(envelope)
        .await
        .expect("terminal rejection replays");
    assert!(matches!(duplicate, ProductInboundAck::Duplicate { .. }));
}

#[tokio::test]
async fn terminal_rejection_for_unpaired_actor_does_not_poison_other_actor_event() {
    let conversations = Arc::new(InMemoryConversationServices::default());
    conversations
        .pair_external_actor(
            TenantId::new("tenant:alpha").expect("tenant"),
            ironclaw_conversations::AdapterKind::new("test_adapter").expect("adapter"),
            ironclaw_conversations::AdapterInstallationId::new("install_alpha").expect("install"),
            ironclaw_conversations::ExternalActorRef::new("test", "user2").expect("actor"),
            UserId::new("user:bob").expect("user"),
        )
        .await;
    let binding = product_binding_service(
        conversations,
        vec![(
            "test_adapter",
            "install_alpha",
            "tenant:alpha",
            "agent:alpha",
            None,
        )],
    );
    let coordinator = Arc::new(RecordingTurnCoordinator::default());
    let inbound = Arc::new(DefaultInboundTurnService::new(
        binding.clone(),
        InMemorySessionThreadService::default(),
        coordinator.clone(),
    ));
    let workflow = DefaultProductWorkflow::new(
        inbound,
        Arc::new(InMemoryIdempotencyLedger::new()),
        Arc::new(binding),
    );

    let unpaired = sample_envelope_with_context(
        ProductAdapterId::new("test_adapter").expect("adapter"),
        AdapterInstallationId::new("install_alpha").expect("install"),
        ExternalEventId::new("evt:shared-event").expect("event"),
        ExternalActorRef::new("test", "user1", Option::<String>::None).expect("actor"),
        ExternalConversationRef::new(None, "conv1", None, None).expect("conversation"),
        ProductInboundPayload::UserMessage(
            UserMessagePayload::new("hello", vec![], ProductTriggerReason::DirectChat)
                .expect("message"),
        ),
    );
    let err = workflow
        .accept_inbound(unpaired)
        .await
        .expect_err("unpaired actor rejected");
    assert!(matches!(
        err,
        ProductAdapterError::WorkflowRejected {
            kind: ProductWorkflowRejectionKind::ScopeNotFound,
            ..
        }
    ));

    let valid_other_actor = sample_envelope_with_context(
        ProductAdapterId::new("test_adapter").expect("adapter"),
        AdapterInstallationId::new("install_alpha").expect("install"),
        ExternalEventId::new("evt:shared-event").expect("event"),
        ExternalActorRef::new("test", "user2", Option::<String>::None).expect("actor"),
        ExternalConversationRef::new(None, "conv1", None, None).expect("conversation"),
        ProductInboundPayload::UserMessage(
            UserMessagePayload::new("hello", vec![], ProductTriggerReason::DirectChat)
                .expect("message"),
        ),
    );
    let accepted = workflow
        .accept_inbound(valid_other_actor)
        .await
        .expect("different actor with same event should not replay rejection");
    assert!(matches!(accepted, ProductInboundAck::Accepted { .. }));
    assert_eq!(coordinator.submissions().len(), 1);
}

#[tokio::test]
async fn accepted_message_replay_validates_current_actor_before_submit() {
    let conversations = Arc::new(InMemoryConversationServices::default());
    conversations
        .pair_external_actor(
            TenantId::new("tenant:alpha").expect("tenant"),
            ironclaw_conversations::AdapterKind::new("test_adapter").expect("adapter"),
            ironclaw_conversations::AdapterInstallationId::new("install_alpha").expect("install"),
            ironclaw_conversations::ExternalActorRef::new("test", "user1").expect("actor"),
            UserId::new("user:alice").expect("user"),
        )
        .await;
    let binding = product_binding_service(
        conversations,
        vec![(
            "test_adapter",
            "install_alpha",
            "tenant:alpha",
            "agent:alpha",
            None,
        )],
    );
    let coordinator = Arc::new(RecordingTurnCoordinator::default());
    coordinator.force_thread_busy_once(TurnRunId::new());
    let inbound = Arc::new(DefaultInboundTurnService::new(
        binding.clone(),
        InMemorySessionThreadService::default(),
        coordinator.clone(),
    ));
    let workflow = DefaultProductWorkflow::new(
        inbound,
        Arc::new(InMemoryIdempotencyLedger::new()),
        Arc::new(binding),
    );

    let first = sample_envelope("accepted-replay-actor-check");
    let busy = workflow.accept_inbound(first).await.expect("busy ack");
    assert!(matches!(busy, ProductInboundAck::DeferredBusy { .. }));

    let unpaired_retry = sample_envelope_with_context(
        ProductAdapterId::new("test_adapter").expect("adapter"),
        AdapterInstallationId::new("install_alpha").expect("install"),
        ExternalEventId::new("evt:accepted-replay-actor-check").expect("event"),
        ExternalActorRef::new("test", "user2", Option::<String>::None).expect("actor"),
        ExternalConversationRef::new(None, "conv1", None, None).expect("conversation"),
        ProductInboundPayload::UserMessage(
            UserMessagePayload::new("hello", vec![], ProductTriggerReason::DirectChat)
                .expect("message"),
        ),
    );
    let err = workflow
        .accept_inbound(unpaired_retry)
        .await
        .expect_err("unpaired retry must not replay accepted message");
    assert!(matches!(
        err,
        ProductAdapterError::WorkflowRejected {
            kind: ProductWorkflowRejectionKind::ScopeNotFound,
            ..
        }
    ));
    assert!(coordinator.submissions().is_empty());
}

#[tokio::test]
async fn concrete_product_workflow_replays_binding_access_denied_rejection() {
    let conversations = Arc::new(InMemoryConversationServices::default());
    conversations
        .pair_external_actor(
            TenantId::new("tenant:alpha").expect("tenant"),
            ironclaw_conversations::AdapterKind::new("test_adapter").expect("adapter"),
            ironclaw_conversations::AdapterInstallationId::new("install_alpha").expect("install"),
            ironclaw_conversations::ExternalActorRef::new("test", "user1").expect("actor"),
            UserId::new("user:alice").expect("user"),
        )
        .await;
    let binding = product_binding_service(
        conversations.clone(),
        vec![(
            "test_adapter",
            "install_alpha",
            "tenant:alpha",
            "agent:alpha",
            None,
        )],
    );
    let coordinator = Arc::new(RecordingTurnCoordinator::default());
    let inbound = Arc::new(DefaultInboundTurnService::new(
        binding.clone(),
        InMemorySessionThreadService::default(),
        coordinator.clone(),
    ));
    let workflow = DefaultProductWorkflow::new(
        inbound,
        Arc::new(InMemoryIdempotencyLedger::new()),
        Arc::new(binding),
    );
    workflow
        .accept_inbound(sample_envelope("direct-owner"))
        .await
        .expect("owner accepted");
    let direct_thread = coordinator.submissions()[0].scope.thread_id.clone();
    conversations
        .pair_external_actor(
            TenantId::new("tenant:alpha").expect("tenant"),
            ironclaw_conversations::AdapterKind::new("test_adapter").expect("adapter"),
            ironclaw_conversations::AdapterInstallationId::new("install_alpha").expect("install"),
            ironclaw_conversations::ExternalActorRef::new("test", "user2").expect("actor"),
            UserId::new("user:bob").expect("user"),
        )
        .await;
    conversations
        .add_thread_participant(
            &TenantId::new("tenant:alpha").expect("tenant"),
            &direct_thread,
            UserId::new("user:bob").expect("user"),
        )
        .await
        .expect("participant added");
    let denied = sample_envelope_with_context(
        ProductAdapterId::new("test_adapter").expect("adapter"),
        AdapterInstallationId::new("install_alpha").expect("install"),
        ExternalEventId::new("evt:direct-participant-denied").expect("event"),
        ExternalActorRef::new("test", "user2", Option::<String>::None).expect("actor"),
        ExternalConversationRef::new(None, "conv1", None, None).expect("conversation"),
        ProductInboundPayload::UserMessage(
            UserMessagePayload::new(
                "direct from participant",
                vec![],
                ProductTriggerReason::DirectChat,
            )
            .expect("message"),
        ),
    );

    let err = workflow
        .accept_inbound(denied.clone())
        .await
        .expect_err("direct participant rejected");
    assert!(matches!(
        err,
        ProductAdapterError::WorkflowRejected {
            kind: ProductWorkflowRejectionKind::Unauthorized,
            status_code: 403,
            retryable: false,
            ..
        }
    ));
    let duplicate = workflow
        .accept_inbound(denied)
        .await
        .expect("terminal rejection replays");
    assert!(matches!(duplicate, ProductInboundAck::Duplicate { .. }));
    assert_eq!(coordinator.submissions().len(), 1);
}

#[tokio::test]
async fn in_memory_idempotency_ledger_reclaims_expired_in_flight_actions() {
    let ledger = InMemoryIdempotencyLedger::with_in_flight_lease(Duration::seconds(10));
    let received_at = Utc::now();
    let fingerprint = ActionFingerprintKey::new(
        ProductAdapterId::new("test_adapter").expect("valid"),
        AdapterInstallationId::new("install_alpha").expect("valid"),
        fingerprint_actor(),
        SourceBindingKey::new("space:0:;conversation:5:conv1;topic:0:;")
            .expect("valid source binding key"),
        ExternalEventId::new("evt:lease-memory").expect("valid"),
    );

    assert!(matches!(
        ledger
            .begin_or_replay(fingerprint.clone(), received_at)
            .await
            .expect("first reservation"),
        IdempotencyDecision::New(_)
    ));
    let duplicate = ledger
        .begin_or_replay(fingerprint.clone(), received_at + Duration::seconds(5))
        .await
        .expect_err("fresh in-flight action blocks duplicate dispatch");
    assert!(duplicate.to_string().contains("in flight"));
    assert!(matches!(
        ledger
            .begin_or_replay(fingerprint, received_at + Duration::seconds(11))
            .await
            .expect("expired reservation is reclaimed"),
        IdempotencyDecision::New(_)
    ));
}

#[tokio::test]
async fn in_memory_idempotency_ledger_allows_only_one_concurrent_reservation() {
    let ledger = Arc::new(InMemoryIdempotencyLedger::with_in_flight_lease(
        Duration::seconds(10),
    ));
    let received_at = Utc::now();
    let fingerprint = ActionFingerprintKey::new(
        ProductAdapterId::new("test_adapter").expect("valid"),
        AdapterInstallationId::new("install_alpha").expect("valid"),
        fingerprint_actor(),
        SourceBindingKey::new("space:0:;conversation:5:conv1;topic:0:;")
            .expect("valid source binding key"),
        ExternalEventId::new("evt:lease-concurrent").expect("valid"),
    );
    let barrier = Arc::new(tokio::sync::Barrier::new(3));
    let first = {
        let ledger = ledger.clone();
        let fingerprint = fingerprint.clone();
        let barrier = barrier.clone();
        tokio::spawn(async move {
            barrier.wait().await;
            ledger.begin_or_replay(fingerprint, received_at).await
        })
    };
    let second = {
        let ledger = ledger.clone();
        let barrier = barrier.clone();
        tokio::spawn(async move {
            barrier.wait().await;
            ledger.begin_or_replay(fingerprint, received_at).await
        })
    };

    barrier.wait().await;
    let results = [
        first.await.expect("first task"),
        second.await.expect("second task"),
    ];

    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, Ok(IdempotencyDecision::New(_))))
            .count(),
        1
    );
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, Err(ProductWorkflowError::Transient { .. })))
            .count(),
        1
    );
}

#[tokio::test]
async fn in_memory_idempotency_ledger_ignores_stale_releases_after_reclaim() {
    let ledger = InMemoryIdempotencyLedger::with_in_flight_lease(Duration::seconds(10));
    let received_at = Utc::now();
    let fingerprint = ActionFingerprintKey::new(
        ProductAdapterId::new("test_adapter").expect("valid"),
        AdapterInstallationId::new("install_alpha").expect("valid"),
        fingerprint_actor(),
        SourceBindingKey::new("space:0:;conversation:5:conv1;topic:0:;")
            .expect("valid source binding key"),
        ExternalEventId::new("evt:lease-stale").expect("valid"),
    );

    let first = match ledger
        .begin_or_replay(fingerprint.clone(), received_at)
        .await
        .expect("first reservation")
    {
        IdempotencyDecision::New(action) => action,
        IdempotencyDecision::Replay(_) => panic!("expected first reservation"),
    };
    let second = match ledger
        .begin_or_replay(fingerprint.clone(), received_at + Duration::seconds(11))
        .await
        .expect("expired reservation is reclaimed")
    {
        IdempotencyDecision::New(action) => action,
        IdempotencyDecision::Replay(_) => panic!("expected reclaimed reservation"),
    };

    ledger
        .release(first.clone())
        .await
        .expect("stale release is ignored");
    assert!(
        ledger
            .begin_or_replay(fingerprint.clone(), received_at + Duration::seconds(12))
            .await
            .expect_err("new reservation stays protected after stale release")
            .to_string()
            .contains("in flight")
    );

    let mut stale_settle = first.clone();
    stale_settle.settle(ProductInboundAck::NoOp);
    let stale_settle_err = ledger
        .settle(stale_settle)
        .await
        .expect_err("stale settle fails loudly");
    assert!(stale_settle_err.to_string().contains("superseded"));
    assert!(
        ledger
            .begin_or_replay(fingerprint.clone(), received_at + Duration::seconds(12))
            .await
            .expect_err("new reservation stays protected after stale settle")
            .to_string()
            .contains("in flight")
    );

    let mut current_settle = second;
    current_settle.settle(ProductInboundAck::NoOp);
    ledger
        .settle(current_settle)
        .await
        .expect("current reservation settles");
    let mut stale_after_current_settle = first;
    stale_after_current_settle.settle(ProductInboundAck::NoOp);
    let stale_after_current_err = ledger
        .settle(stale_after_current_settle)
        .await
        .expect_err("stale settle remains rejected after current settle");
    assert!(stale_after_current_err.to_string().contains("superseded"));
    assert!(matches!(
        ledger
            .begin_or_replay(fingerprint, received_at + Duration::seconds(12))
            .await
            .expect("settled action replays"),
        IdempotencyDecision::Replay(_)
    ));
}

#[tokio::test]
async fn in_memory_idempotency_ledger_rejects_settle_after_expiry_without_reclaim() {
    let ledger = InMemoryIdempotencyLedger::with_in_flight_lease(Duration::seconds(10));
    let received_at = Utc::now();
    let fingerprint = ActionFingerprintKey::new(
        ProductAdapterId::new("test_adapter").expect("valid"),
        AdapterInstallationId::new("install_alpha").expect("valid"),
        fingerprint_actor(),
        SourceBindingKey::new("space:0:;conversation:5:conv1;topic:0:;")
            .expect("valid source binding key"),
        ExternalEventId::new("evt:lease-missing").expect("valid"),
    );

    let mut action = match ledger
        .begin_or_replay(fingerprint, received_at)
        .await
        .expect("first reservation")
    {
        IdempotencyDecision::New(action) => action,
        IdempotencyDecision::Replay(_) => panic!("expected first reservation"),
    };
    assert_eq!(
        ledger
            .expire_in_flight_before(received_at + Duration::seconds(11))
            .expect("expired"),
        1
    );
    action.settle(ProductInboundAck::NoOp);

    let err = ledger
        .settle(action)
        .await
        .expect_err("terminal outcome must not report durable success after expiry");
    assert!(err.to_string().contains("reservation missing"));
}

fn product_binding_service(
    conversations: Arc<InMemoryConversationServices>,
    installations: Vec<(&str, &str, &str, &str, Option<&str>)>,
) -> ProductConversationBindingService {
    let conversation_port: Arc<dyn ironclaw_conversations::ConversationBindingService> =
        conversations;
    let resolver = StaticProductInstallationResolver::new(installations.into_iter().map(
        |(adapter, installation, tenant, agent, project)| {
            (
                ProductInstallationKey::new(
                    ProductAdapterId::new(adapter).expect("adapter"),
                    AdapterInstallationId::new(installation).expect("installation"),
                ),
                ProductInstallationScope::with_default_scope(
                    TenantId::new(tenant).expect("tenant"),
                    AgentId::new(agent).expect("agent"),
                    project.map(|value| ProjectId::new(value).expect("project")),
                ),
            )
        },
    ));
    ProductConversationBindingService::new(conversation_port, resolver)
}

#[tokio::test]
async fn duplicate_envelope_replays_prior_outcome() {
    let (workflow, inbound, _ledger) = build_workflow();

    // First submission.
    let envelope = sample_envelope("dup1");
    let first_ack = workflow
        .accept_inbound(envelope.clone())
        .await
        .expect("first accept");
    assert!(matches!(first_ack, ProductInboundAck::Accepted { .. }));
    assert_eq!(inbound.accepted_count(), 1);

    // Second submission of same envelope.
    let second_ack = workflow
        .accept_inbound(envelope)
        .await
        .expect("second accept");
    assert!(matches!(second_ack, ProductInboundAck::Duplicate { .. }));
    // InboundTurnService should NOT be called a second time.
    assert_eq!(inbound.accepted_count(), 1);
}

#[tokio::test]
async fn settled_user_message_records_actual_submitted_run_id() {
    let (workflow, _inbound, ledger) = build_workflow();
    let envelope = sample_envelope("run-id");

    let ack = workflow.accept_inbound(envelope).await.expect("accept");
    let ProductInboundAck::Accepted {
        submitted_run_id, ..
    } = ack
    else {
        panic!("expected accepted ack");
    };
    let actions = ledger.settled_actions();
    assert_eq!(actions.len(), 1);
    assert_eq!(
        actions[0].dispatch_kind,
        Some(ActionDispatchKind::UserMessageTurn {
            run_id: submitted_run_id
        })
    );
}

#[tokio::test]
async fn retryable_dispatch_failure_releases_fingerprint_for_recovery() {
    let (workflow, inbound, ledger) = build_workflow();
    inbound.force_failure(ProductWorkflowError::Transient {
        reason: "turn coordinator unavailable".into(),
    });

    let envelope = sample_envelope("transient-released");
    let first_err = workflow
        .accept_inbound(envelope.clone())
        .await
        .expect_err("first attempt should be retryable");
    assert!(first_err.is_retryable());
    assert_eq!(inbound.attempt_count(), 1);
    assert_eq!(ledger.in_flight_count(), 0);

    let second_err = workflow
        .accept_inbound(envelope)
        .await
        .expect_err("released fingerprint should retry dispatch");
    assert!(second_err.is_retryable());
    assert_eq!(inbound.attempt_count(), 2);
    assert_eq!(ledger.settled_count(), 0);
}

#[tokio::test]
async fn deferred_busy_is_not_settled_and_retry_can_submit_same_message() {
    let (workflow, inbound, ledger) = build_workflow();
    let accepted_message_ref = AcceptedMessageRef::new("msg:busy-retry").expect("valid msg ref");
    let busy_run = TurnRunId::new();
    inbound.program_outcome(InboundTurnOutcome::DeferredBusy {
        accepted_message_ref: accepted_message_ref.clone(),
        active_run_id: busy_run,
        binding: fake_binding(),
    });

    let envelope = sample_envelope("busy-retry");
    let first = workflow
        .accept_inbound(envelope.clone())
        .await
        .expect("busy ack");
    assert!(matches!(first, ProductInboundAck::DeferredBusy { .. }));
    assert_eq!(ledger.settled_count(), 0);
    assert_eq!(ledger.in_flight_count(), 0);

    let submitted_run_id = TurnRunId::new();
    inbound.program_outcome(InboundTurnOutcome::Submitted {
        accepted_message_ref: accepted_message_ref.clone(),
        submitted_run_id,
        binding: fake_binding(),
    });
    let second = workflow
        .accept_inbound(envelope)
        .await
        .expect("retry submit");
    assert!(matches!(
        second,
        ProductInboundAck::Accepted {
            submitted_run_id: run_id,
            ..
        } if run_id == submitted_run_id
    ));
    assert_eq!(inbound.attempt_count(), 2);
    assert_eq!(ledger.settled_count(), 1);
}

#[tokio::test]
async fn fake_ledger_expiration_reclaims_in_flight_fingerprint() {
    let ledger = FakeIdempotencyLedger::new();
    let received_at = Utc::now();
    let fingerprint = ActionFingerprintKey::new(
        ProductAdapterId::new("test_adapter").expect("valid"),
        AdapterInstallationId::new("install_alpha").expect("valid"),
        fingerprint_actor(),
        SourceBindingKey::new("space:0:;conversation:5:conv1;topic:0:;").expect("valid"),
        ExternalEventId::new("evt:lease").expect("valid"),
    );

    let first = ledger
        .begin_or_replay(fingerprint.clone(), received_at)
        .await
        .expect("reserve");
    assert!(matches!(first, IdempotencyDecision::New(_)));
    let duplicate = ledger
        .begin_or_replay(fingerprint.clone(), received_at)
        .await
        .expect_err("fresh in-flight action blocks duplicate dispatch");
    assert!(matches!(duplicate, ProductWorkflowError::Transient { .. }));

    assert_eq!(
        ledger.expire_in_flight_before(received_at + Duration::seconds(1)),
        1
    );
    let reclaimed = ledger
        .begin_or_replay(fingerprint, received_at)
        .await
        .expect("expired fingerprint can be reclaimed");
    assert!(matches!(reclaimed, IdempotencyDecision::New(_)));
}

#[tokio::test]
async fn permanent_turn_submission_failure_settles_terminal_rejection() {
    let (workflow, inbound, ledger) = build_workflow();
    inbound.force_failure(ProductWorkflowError::TurnSubmissionFailed {
        error: TurnError::Unauthorized,
    });

    let envelope = sample_envelope("terminal-turn-error");
    let err = workflow
        .accept_inbound(envelope.clone())
        .await
        .expect_err("unauthorized turn rejection should surface error");
    assert!(!err.is_retryable());
    assert_eq!(ledger.settled_count(), 1);

    let replay = workflow
        .accept_inbound(envelope)
        .await
        .expect("terminal rejection should replay duplicate ack");
    let ProductInboundAck::Duplicate { prior } = replay else {
        panic!("expected duplicate replay")
    };
    let ProductInboundAck::Rejected(rejection) = *prior else {
        panic!("expected rejected prior outcome")
    };
    assert_eq!(
        rejection.disposition(),
        ProductRejectionDisposition::Permanent
    );
}

#[tokio::test]
async fn retryable_turn_submission_failure_releases_for_retry() {
    let (workflow, inbound, ledger) = build_workflow();
    inbound.force_failure(ProductWorkflowError::TurnSubmissionFailed {
        error: TurnError::Unavailable {
            reason: "turn store unavailable".into(),
        },
    });

    let envelope = sample_envelope("retryable-turn-error");
    let first = workflow
        .accept_inbound(envelope.clone())
        .await
        .expect_err("unavailable turn rejection should surface retryable error");
    assert!(first.is_retryable());
    assert_eq!(ledger.settled_count(), 0);
    assert_eq!(ledger.in_flight_count(), 0);

    let second = workflow
        .accept_inbound(envelope)
        .await
        .expect_err("released retryable turn rejection should dispatch again");
    assert!(second.is_retryable());
    assert_eq!(inbound.attempt_count(), 2);
}

#[tokio::test]
async fn settle_failure_does_not_return_success_ack() {
    let (workflow, inbound, ledger) = build_workflow();
    ledger.force_settle_failure(ironclaw_product_workflow::ProductWorkflowError::Transient {
        reason: "settle timeout".into(),
    });

    let envelope = sample_envelope("settle-fail");
    let err = workflow
        .accept_inbound(envelope)
        .await
        .expect_err("settle failure should fail request");
    assert!(err.is_retryable());
    assert_eq!(inbound.accepted_count(), 1);
    assert_eq!(ledger.settled_count(), 0);
}

#[tokio::test]
async fn unsupported_action_is_settled_as_terminal_rejection() {
    let (workflow, _inbound, ledger) = build_workflow();
    let envelope = sample_noop_envelope("unsupported-base");
    let context = TrustedInboundContext::from_verified_evidence(
        envelope.adapter_id().clone(),
        envelope.installation_id().clone(),
        Utc::now(),
        &ProtocolAuthEvidence::test_verified(
            AuthRequirement::SharedSecretHeader {
                header_name: "X-Secret".into(),
            },
            "install_alpha",
        ),
    )
    .expect("verified");
    let parsed = ParsedProductInbound::new(
        ExternalEventId::new("evt:unsupported").expect("valid"),
        ExternalActorRef::new("test", "user1", Option::<String>::None).expect("valid"),
        ExternalConversationRef::new(None, "conv1", None, None).expect("valid"),
        ProductInboundPayload::AuthResolution(
            ironclaw_product_adapters::AuthResolutionPayload::new(
                "auth:1",
                ironclaw_product_adapters::AuthResolutionResult::Denied,
            )
            .expect("valid"),
        ),
    )
    .expect("parsed");
    let unsupported =
        ProductInboundEnvelope::from_trusted_parse(context, parsed).expect("envelope");

    let err = workflow
        .accept_inbound(unsupported.clone())
        .await
        .expect_err("unsupported should error");
    assert!(!err.is_retryable());
    assert_eq!(ledger.settled_count(), 1);

    let replay = workflow
        .accept_inbound(unsupported)
        .await
        .expect("duplicate replay");
    assert!(matches!(replay, ProductInboundAck::Duplicate { .. }));
}

#[tokio::test]
async fn ledger_transient_failure_surfaces_retryable_error() {
    let (workflow, _inbound, ledger) = build_workflow();
    ledger.force_failure(ironclaw_product_workflow::ProductWorkflowError::Transient {
        reason: "db timeout".into(),
    });

    let envelope = sample_envelope("fail1");
    let err = workflow
        .accept_inbound(envelope)
        .await
        .expect_err("should fail");
    assert!(err.is_retryable());
}
