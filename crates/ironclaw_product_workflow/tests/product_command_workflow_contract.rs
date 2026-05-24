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
    FakeIdempotencyLedger, FakeInboundTurnService, ProductCommand, ProductCommandAdmission,
    ProductCommandAdmissionService, ProductCommandContext, ProductCommandService,
    ProductModelCommand, ProductWorkflowError,
};

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

struct AllowingProductCommandAdmissionService;

#[async_trait]
impl ProductCommandAdmissionService for AllowingProductCommandAdmissionService {
    async fn admit(
        &self,
        _context: &ProductCommandContext,
        _command: &ProductCommand,
    ) -> Result<ProductCommandAdmission, ProductWorkflowError> {
        Ok(ProductCommandAdmission::Allowed)
    }
}

struct RecordingProductCommandService {
    commands: Mutex<Vec<ProductCommand>>,
    ack: ProductInboundAck,
}

impl RecordingProductCommandService {
    fn new(ack: ProductInboundAck) -> Self {
        Self {
            commands: Mutex::new(Vec::new()),
            ack,
        }
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
        Ok(self.ack.clone())
    }
}

#[tokio::test]
async fn command_payload_dispatches_through_command_service_not_inbound_turn_service() {
    let inbound = Arc::new(FakeInboundTurnService::new());
    let ledger = Arc::new(FakeIdempotencyLedger::new());
    let binding = Arc::new(FakeConversationBindingService::new());
    let admission_service = Arc::new(AllowingProductCommandAdmissionService);
    let command_service = Arc::new(RecordingProductCommandService::new(ProductInboundAck::NoOp));
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
async fn default_command_admission_rejects_before_command_service_executes() {
    let inbound = Arc::new(FakeInboundTurnService::new());
    let ledger = Arc::new(FakeIdempotencyLedger::new());
    let binding = Arc::new(FakeConversationBindingService::new());
    let command_service = Arc::new(RecordingProductCommandService::new(ProductInboundAck::NoOp));
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
