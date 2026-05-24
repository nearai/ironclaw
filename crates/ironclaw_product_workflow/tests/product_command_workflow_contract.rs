//! Contract tests for product command dispatch through the workflow facade.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_product_adapters::{
    AdapterInstallationId, AuthRequirement, ExternalActorRef, ExternalConversationRef,
    ExternalEventId, InboundCommandPayload, ProductAdapterId, ProductInboundAck,
    ProductInboundEnvelope, ProductInboundPayload, ProductTriggerReason, ProductWorkflow,
    ProtocolAuthEvidence, TrustedInboundContext,
};
use ironclaw_product_workflow::{
    ActionDispatchKind, DefaultProductWorkflow, FakeConversationBindingService,
    FakeIdempotencyLedger, FakeInboundTurnService, LifecyclePackageKind, LifecyclePhase,
    LifecycleProductAction, LifecycleProductCommandService, LifecycleProductContext,
    LifecycleProductFacade, LifecycleProductResponse, ProductCommand, ProductCommandAdmission,
    ProductCommandAdmissionService, ProductCommandContext, ProductCommandService,
    ProductModelCommand, ProductWorkflowError,
};
use ironclaw_turns::{AcceptedMessageRef, TurnRunId};

fn sample_command_envelope(
    event_suffix: &str,
    command: &str,
    arguments: &str,
) -> ProductInboundEnvelope {
    let adapter_id = ProductAdapterId::new("test_adapter").expect("valid adapter");
    let installation_id = AdapterInstallationId::new("install_alpha").expect("valid installation");
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
    let parsed = ironclaw_product_adapters::ParsedProductInbound::new(
        ExternalEventId::new(format!("evt:{event_suffix}")).expect("valid event"),
        ExternalActorRef::new("test", "user1", Option::<String>::None).expect("valid actor"),
        ExternalConversationRef::new(None, "conv1", None, None).expect("valid conversation"),
        ProductInboundPayload::Command(
            InboundCommandPayload::new(command, arguments, ProductTriggerReason::BotCommand)
                .expect("valid command"),
        ),
    )
    .expect("parsed");

    ProductInboundEnvelope::from_trusted_parse(context, parsed).expect("envelope")
}

struct RecordingProductCommandAdmissionService {
    records: Mutex<Vec<(ProductCommandContext, ProductCommand)>>,
    result: Result<ProductCommandAdmission, ProductWorkflowError>,
}

impl RecordingProductCommandAdmissionService {
    fn new(result: Result<ProductCommandAdmission, ProductWorkflowError>) -> Self {
        Self {
            records: Mutex::new(Vec::new()),
            result,
        }
    }

    fn allowing() -> Self {
        Self::new(Ok(ProductCommandAdmission::Allowed))
    }

    fn failing(error: ProductWorkflowError) -> Self {
        Self::new(Err(error))
    }

    fn records(&self) -> Vec<(ProductCommandContext, ProductCommand)> {
        self.records.lock().expect("lock").clone()
    }
}

#[async_trait]
impl ProductCommandAdmissionService for RecordingProductCommandAdmissionService {
    async fn admit(
        &self,
        context: &ProductCommandContext,
        command: &ProductCommand,
    ) -> Result<ProductCommandAdmission, ProductWorkflowError> {
        self.records
            .lock()
            .expect("lock")
            .push((context.clone(), command.clone()));
        self.result.clone()
    }
}

struct RecordingProductCommandService {
    commands: Mutex<Vec<ProductCommand>>,
    result: Result<ProductInboundAck, ProductWorkflowError>,
}

#[derive(Default)]
struct RecordingLifecycleProductFacade {
    commands: Mutex<Vec<LifecycleProductAction>>,
}

impl RecordingLifecycleProductFacade {
    fn commands(&self) -> Vec<LifecycleProductAction> {
        self.commands.lock().expect("lock").clone()
    }
}

#[async_trait]
impl LifecycleProductFacade for RecordingLifecycleProductFacade {
    async fn execute(
        &self,
        _context: LifecycleProductContext,
        action: LifecycleProductAction,
    ) -> Result<LifecycleProductResponse, ProductWorkflowError> {
        self.commands.lock().expect("lock").push(action.clone());
        Ok(LifecycleProductResponse::projection(
            action.package_ref().cloned(),
            LifecyclePhase::Installed,
            vec![],
        ))
    }
}

struct FailingLifecycleProductFacade {
    error: ProductWorkflowError,
}

#[async_trait]
impl LifecycleProductFacade for FailingLifecycleProductFacade {
    async fn execute(
        &self,
        _context: LifecycleProductContext,
        _action: LifecycleProductAction,
    ) -> Result<LifecycleProductResponse, ProductWorkflowError> {
        Err(self.error.clone())
    }
}

impl RecordingProductCommandService {
    fn new(result: Result<ProductInboundAck, ProductWorkflowError>) -> Self {
        Self {
            commands: Mutex::new(Vec::new()),
            result,
        }
    }

    fn with_ack(ack: ProductInboundAck) -> Self {
        Self::new(Ok(ack))
    }

    fn failing(error: ProductWorkflowError) -> Self {
        Self::new(Err(error))
    }

    fn commands(&self) -> Vec<ProductCommand> {
        self.commands.lock().expect("lock").clone()
    }
}

#[async_trait]
impl ProductCommandService for RecordingProductCommandService {
    async fn execute(
        &self,
        _context: ProductCommandContext,
        command: ProductCommand,
    ) -> Result<ProductInboundAck, ProductWorkflowError> {
        self.commands.lock().expect("lock").push(command);
        self.result.clone()
    }
}

#[tokio::test]
async fn command_payload_dispatches_through_command_service_not_inbound_turn_service() {
    let inbound = Arc::new(FakeInboundTurnService::new());
    let ledger = Arc::new(FakeIdempotencyLedger::new());
    let binding = Arc::new(FakeConversationBindingService::new());
    let admission_service = Arc::new(RecordingProductCommandAdmissionService::allowing());
    let command_service = Arc::new(RecordingProductCommandService::with_ack(
        ProductInboundAck::NoOp,
    ));
    let workflow = DefaultProductWorkflow::new(inbound.clone(), ledger.clone(), binding)
        .with_product_command_admission_service(admission_service)
        .with_product_command_service(command_service.clone());
    let envelope =
        sample_command_envelope("command-model", "model", "gpt-5-mini --ignored-for-now");

    let ack = workflow.accept_inbound(envelope).await.expect("accept");

    assert!(matches!(ack, ProductInboundAck::NoOp));
    assert_eq!(inbound.accepted_count(), 0);
    assert_eq!(inbound.attempt_count(), 0);
    assert_eq!(inbound.replay_attempt_count(), 0);
    assert_eq!(
        command_service.commands(),
        vec![ProductCommand::Model {
            action: ProductModelCommand::Set {
                model: "gpt-5-mini".to_string()
            }
        }]
    );
    let settled = ledger.settled_actions();
    assert_eq!(settled.len(), 1);
    assert!(matches!(
        settled[0].dispatch_kind,
        Some(ActionDispatchKind::Command { .. })
    ));
}

#[tokio::test]
async fn lifecycle_command_dispatches_through_lifecycle_facade() {
    let inbound = Arc::new(FakeInboundTurnService::new());
    let ledger = Arc::new(FakeIdempotencyLedger::new());
    let binding = Arc::new(FakeConversationBindingService::new());
    let admission_service = Arc::new(RecordingProductCommandAdmissionService::allowing());
    let lifecycle_facade = Arc::new(RecordingLifecycleProductFacade::default());
    let command_service = Arc::new(LifecycleProductCommandService::new(
        lifecycle_facade.clone(),
    ));
    let workflow = DefaultProductWorkflow::new(inbound.clone(), ledger.clone(), binding)
        .with_product_command_admission_service(admission_service)
        .with_product_command_service(command_service);
    let envelope =
        sample_command_envelope("command-extension-install", "extension_install", "github");

    let ack = workflow.accept_inbound(envelope).await.expect("accept");

    assert!(matches!(ack, ProductInboundAck::NoOp));
    assert_eq!(inbound.accepted_count(), 0);
    assert_eq!(
        lifecycle_facade.commands(),
        vec![LifecycleProductAction::ExtensionInstall {
            package_ref: ironclaw_product_workflow::LifecyclePackageRef::new(
                LifecyclePackageKind::Extension,
                "github",
            )
            .unwrap(),
        }]
    );
}

#[tokio::test]
async fn lifecycle_command_admission_rejects_before_facade_executes() {
    let inbound = Arc::new(FakeInboundTurnService::new());
    let ledger = Arc::new(FakeIdempotencyLedger::new());
    let binding = Arc::new(FakeConversationBindingService::new());
    let admission_service = Arc::new(RecordingProductCommandAdmissionService::new(Ok(
        ProductCommandAdmission::Rejected(ironclaw_product_adapters::ProductRejection::permanent(
            ironclaw_product_adapters::ProductRejectionKind::PolicyDenied,
            "lifecycle policy denied",
        )),
    )));
    let lifecycle_facade = Arc::new(RecordingLifecycleProductFacade::default());
    let command_service = Arc::new(LifecycleProductCommandService::new(
        lifecycle_facade.clone(),
    ));
    let workflow = DefaultProductWorkflow::new(inbound.clone(), ledger.clone(), binding)
        .with_product_command_admission_service(admission_service)
        .with_product_command_service(command_service);
    let envelope =
        sample_command_envelope("command-extension-denied", "extension_activate", "github");

    let ack = workflow.accept_inbound(envelope).await.expect("accept");

    assert!(matches!(ack, ProductInboundAck::Rejected(_)));
    assert!(lifecycle_facade.commands().is_empty());
    assert_eq!(inbound.accepted_count(), 0);
}

#[tokio::test]
async fn lifecycle_command_service_rejects_non_lifecycle_commands_without_facade_execution() {
    let inbound = Arc::new(FakeInboundTurnService::new());
    let ledger = Arc::new(FakeIdempotencyLedger::new());
    let binding = Arc::new(FakeConversationBindingService::new());
    let admission_service = Arc::new(RecordingProductCommandAdmissionService::allowing());
    let lifecycle_facade = Arc::new(RecordingLifecycleProductFacade::default());
    let command_service = Arc::new(LifecycleProductCommandService::new(
        lifecycle_facade.clone(),
    ));
    let workflow = DefaultProductWorkflow::new(inbound.clone(), ledger.clone(), binding)
        .with_product_command_admission_service(admission_service)
        .with_product_command_service(command_service);
    let envelope = sample_command_envelope("command-status-lifecycle-service", "status", "");

    let ack = workflow.accept_inbound(envelope).await.expect("accept");

    assert!(matches!(
        ack,
        ProductInboundAck::Rejected(rejection)
            if rejection.kind == ironclaw_product_adapters::ProductRejectionKind::PolicyDenied
    ));
    assert!(lifecycle_facade.commands().is_empty());
    assert_eq!(inbound.accepted_count(), 0);
    assert_eq!(ledger.settled_count(), 1);
}

#[tokio::test]
async fn lifecycle_facade_error_bubbles_and_releases_idempotency_lease() {
    let inbound = Arc::new(FakeInboundTurnService::new());
    let ledger = Arc::new(FakeIdempotencyLedger::new());
    let binding = Arc::new(FakeConversationBindingService::new());
    let admission_service = Arc::new(RecordingProductCommandAdmissionService::allowing());
    let command_service = Arc::new(LifecycleProductCommandService::new(Arc::new(
        FailingLifecycleProductFacade {
            error: ProductWorkflowError::Transient {
                reason: "lifecycle backend unavailable".to_string(),
            },
        },
    )));
    let workflow = DefaultProductWorkflow::new(inbound.clone(), ledger.clone(), binding)
        .with_product_command_admission_service(admission_service)
        .with_product_command_service(command_service);
    let envelope = sample_command_envelope(
        "command-extension-facade-error",
        "extension_install",
        "github",
    );

    let err = workflow
        .accept_inbound(envelope)
        .await
        .expect_err("lifecycle facade error must bubble");

    assert!(err.is_retryable());
    assert_eq!(inbound.accepted_count(), 0);
    assert_eq!(ledger.settled_count(), 0);
    assert_eq!(ledger.released_count(), 1);
}

#[tokio::test]
async fn default_command_admission_rejects_before_command_service_executes() {
    let inbound = Arc::new(FakeInboundTurnService::new());
    let ledger = Arc::new(FakeIdempotencyLedger::new());
    let binding = Arc::new(FakeConversationBindingService::new());
    let command_service = Arc::new(RecordingProductCommandService::with_ack(
        ProductInboundAck::NoOp,
    ));
    let workflow = DefaultProductWorkflow::new(inbound.clone(), ledger.clone(), binding)
        .with_product_command_service(command_service.clone());
    let envelope = sample_command_envelope("command-default-reject", "model", "gpt-5-mini");

    let ack = workflow.accept_inbound(envelope).await.expect("accept");

    assert!(matches!(
        ack,
        ProductInboundAck::Rejected(rejection)
            if rejection.kind == ironclaw_product_adapters::ProductRejectionKind::PolicyDenied
    ));
    assert!(command_service.commands().is_empty());
    assert_eq!(inbound.accepted_count(), 0);
    assert_eq!(ledger.settled_count(), 1);
}

#[tokio::test]
async fn command_admission_receives_authority_context_and_action_metadata() {
    let inbound = Arc::new(FakeInboundTurnService::new());
    let ledger = Arc::new(FakeIdempotencyLedger::new());
    let binding = Arc::new(FakeConversationBindingService::new());
    let admission_service = Arc::new(RecordingProductCommandAdmissionService::allowing());
    let command_service = Arc::new(RecordingProductCommandService::with_ack(
        ProductInboundAck::NoOp,
    ));
    let workflow = DefaultProductWorkflow::new(inbound.clone(), ledger.clone(), binding)
        .with_product_command_admission_service(admission_service.clone())
        .with_product_command_service(command_service);
    let envelope = sample_command_envelope("command-context", "status", "");
    let expected_adapter_id = envelope.adapter_id().clone();
    let expected_installation_id = envelope.installation_id().clone();
    let expected_actor = envelope.external_actor_ref().clone();
    let expected_conversation = envelope.external_conversation_ref().clone();
    let expected_auth_claim = envelope.auth_claim().clone();
    let expected_received_at = envelope.received_at();

    let ack = workflow.accept_inbound(envelope).await.expect("accept");

    assert!(matches!(ack, ProductInboundAck::NoOp));
    let records = admission_service.records();
    assert_eq!(records.len(), 1);
    let (context, command) = &records[0];
    assert_eq!(command, &ProductCommand::Status);
    assert_eq!(context.adapter_id, expected_adapter_id);
    assert_eq!(context.installation_id, expected_installation_id);
    assert_eq!(context.external_actor_ref, expected_actor);
    assert_eq!(context.external_conversation_ref, expected_conversation);
    assert_eq!(context.auth_claim, expected_auth_claim);
    assert_eq!(context.trigger, ProductTriggerReason::BotCommand);
    assert_eq!(context.received_at, expected_received_at);

    let settled = ledger.settled_actions();
    assert_eq!(settled.len(), 1);
    assert_eq!(context.action_id, settled[0].action_id);
    assert_eq!(context.fingerprint, settled[0].fingerprint);
}

#[tokio::test]
async fn command_admission_error_releases_idempotency_lease() {
    let inbound = Arc::new(FakeInboundTurnService::new());
    let ledger = Arc::new(FakeIdempotencyLedger::new());
    let binding = Arc::new(FakeConversationBindingService::new());
    let admission_service = Arc::new(RecordingProductCommandAdmissionService::failing(
        ProductWorkflowError::Transient {
            reason: "admission backend unavailable".into(),
        },
    ));
    let command_service = Arc::new(RecordingProductCommandService::with_ack(
        ProductInboundAck::NoOp,
    ));
    let workflow = DefaultProductWorkflow::new(inbound.clone(), ledger.clone(), binding)
        .with_product_command_admission_service(admission_service)
        .with_product_command_service(command_service.clone());
    let envelope = sample_command_envelope("command-admission-error", "model", "gpt-5-mini");

    let err = workflow
        .accept_inbound(envelope)
        .await
        .expect_err("transient admission error must bubble");

    assert!(err.is_retryable());
    assert!(command_service.commands().is_empty());
    assert_eq!(inbound.accepted_count(), 0);
    assert_eq!(ledger.settled_count(), 0);
    assert_eq!(ledger.released_count(), 1);
}

#[tokio::test]
async fn command_service_error_releases_idempotency_lease() {
    let inbound = Arc::new(FakeInboundTurnService::new());
    let ledger = Arc::new(FakeIdempotencyLedger::new());
    let binding = Arc::new(FakeConversationBindingService::new());
    let admission_service = Arc::new(RecordingProductCommandAdmissionService::allowing());
    let command_service = Arc::new(RecordingProductCommandService::failing(
        ProductWorkflowError::Transient {
            reason: "command backend unavailable".into(),
        },
    ));
    let workflow = DefaultProductWorkflow::new(inbound.clone(), ledger.clone(), binding)
        .with_product_command_admission_service(admission_service)
        .with_product_command_service(command_service.clone());
    let envelope = sample_command_envelope("command-service-error", "model", "gpt-5-mini");

    let err = workflow
        .accept_inbound(envelope)
        .await
        .expect_err("transient command error must bubble");

    assert!(err.is_retryable());
    assert_eq!(command_service.commands().len(), 1);
    assert_eq!(inbound.accepted_count(), 0);
    assert_eq!(ledger.settled_count(), 0);
    assert_eq!(ledger.released_count(), 1);
}

#[tokio::test]
async fn default_command_service_rejects_when_admission_is_supplied() {
    let inbound = Arc::new(FakeInboundTurnService::new());
    let ledger = Arc::new(FakeIdempotencyLedger::new());
    let binding = Arc::new(FakeConversationBindingService::new());
    let admission_service = Arc::new(RecordingProductCommandAdmissionService::allowing());
    let workflow = DefaultProductWorkflow::new(inbound.clone(), ledger.clone(), binding)
        .with_product_command_admission_service(admission_service);
    let envelope = sample_command_envelope("command-default-service-reject", "status", "");

    let ack = workflow.accept_inbound(envelope).await.expect("accept");

    assert!(matches!(
        ack,
        ProductInboundAck::Rejected(rejection)
            if rejection.kind == ironclaw_product_adapters::ProductRejectionKind::PolicyDenied
    ));
    assert_eq!(inbound.accepted_count(), 0);
    assert_eq!(ledger.settled_count(), 1);
}

#[tokio::test]
async fn command_service_turn_ack_is_rejected_before_turn_dispatch_kind_is_recorded() {
    let inbound = Arc::new(FakeInboundTurnService::new());
    let ledger = Arc::new(FakeIdempotencyLedger::new());
    let binding = Arc::new(FakeConversationBindingService::new());
    let admission_service = Arc::new(RecordingProductCommandAdmissionService::allowing());
    let command_service = Arc::new(RecordingProductCommandService::with_ack(
        ProductInboundAck::Accepted {
            accepted_message_ref: AcceptedMessageRef::new("msg:command").expect("valid ref"),
            submitted_run_id: TurnRunId::new(),
        },
    ));
    let workflow = DefaultProductWorkflow::new(inbound.clone(), ledger.clone(), binding)
        .with_product_command_admission_service(admission_service)
        .with_product_command_service(command_service);
    let envelope = sample_command_envelope("command-turn-ack", "status", "");

    workflow
        .accept_inbound(envelope)
        .await
        .expect_err("turn-shaped command ack must fail");

    assert_eq!(inbound.accepted_count(), 0);
    assert_eq!(ledger.released_count(), 0);
    let settled = ledger.settled_actions();
    assert_eq!(settled.len(), 1);
    assert!(matches!(
        settled[0].dispatch_kind,
        Some(ActionDispatchKind::Rejected { .. })
    ));
}
