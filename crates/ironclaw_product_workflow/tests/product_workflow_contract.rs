//! Contract tests for the product workflow facade.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{Duration, Utc};
use ironclaw_conversations::InMemoryConversationServices;
use ironclaw_host_api::{AgentId, ProjectId, TenantId, ThreadId, UserId};
use ironclaw_product_adapters::{
    AdapterInstallationId, ApprovalDecision, ApprovalResolutionPayload, AuthRequirement,
    AuthResolutionPayload, AuthResolutionResult, ExternalActorRef, ExternalConversationRef,
    ExternalEventId, InboundCommandPayload, LinkedThreadActionPayload, ParsedProductInbound,
    ProductAdapterError, ProductAdapterId, ProductInboundAck, ProductInboundEnvelope,
    ProductInboundPayload, ProductRejectionDisposition, ProductTriggerReason, ProductWorkflow,
    ProductWorkflowRejectionKind, ProjectionCursor, ProjectionSubscriptionPayload,
    ProtocolAuthEvidence, TrustedInboundContext, UserMessagePayload,
};
use ironclaw_product_workflow::{
    ActionDispatchKind, ActionFingerprintKey, AuthRequestRef, DefaultInboundTurnService,
    DefaultProductWorkflow, FakeConversationBindingService, FakeIdempotencyLedger,
    FakeInboundTurnService, IdempotencyDecision, IdempotencyLedger, InMemoryIdempotencyLedger,
    InboundTurnOutcome, LinkedThreadActionId, ProductCommandName,
    ProductConversationBindingService, ProductInstallationKey, ProductInstallationScope,
    ProductWorkflowError, ResolvedBinding, SourceBindingKey, StaticProductInstallationResolver,
};
use ironclaw_threads::InMemorySessionThreadService;
use ironclaw_turns::{
    AcceptedMessageRef, CancelRunRequest, CancelRunResponse, EventCursor, GetRunStateRequest,
    LoopGateRef, ResumeTurnRequest, ResumeTurnResponse, RunProfileId, RunProfileVersion,
    SubmitTurnRequest, SubmitTurnResponse, ThreadBusy, TurnCoordinator, TurnError, TurnId,
    TurnRunId, TurnRunState, TurnStatus,
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
