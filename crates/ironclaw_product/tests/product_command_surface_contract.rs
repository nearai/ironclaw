//! Contract tests for product command dispatch through the product surface.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_host_api::{
    ProductSurface, ProductSurfaceCaller, ProductSurfaceError, ProductSurfaceInvokeRequest,
    ProductSurfaceInvokeResponse,
};
use ironclaw_product::{
    ActionDispatchKind, DefaultProductSurface, FakeConversationBindingService,
    FakeIdempotencyLedger, FakeInboundTurnService, PRODUCT_LIFECYCLE_COMMAND_OPERATION_ID,
    PRODUCT_MODEL_COMMAND_OPERATION_ID, ProductCommand, ProductCommandAdmission,
    ProductCommandAdmissionService, ProductCommandContext, ProductInboundAck,
};
use ironclaw_product::{
    AdapterInstallationId, AuthRequirement, ExternalActorRef, ExternalConversationRef,
    ExternalEventId, InboundCommandPayload, ProductAdapterId, ProductInboundEnvelope,
    ProductInboundPayload, ProductTriggerReason, ProtocolAuthEvidence, TrustedInboundContext,
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
    let parsed = ironclaw_product::ParsedProductInbound::new(
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
    result: Result<ProductCommandAdmission, ProductSurfaceError>,
}

impl RecordingProductCommandAdmissionService {
    fn new(result: Result<ProductCommandAdmission, ProductSurfaceError>) -> Self {
        Self {
            records: Mutex::new(Vec::new()),
            result,
        }
    }

    fn allowing() -> Self {
        Self::new(Ok(ProductCommandAdmission::Allowed))
    }

    fn failing(error: ProductSurfaceError) -> Self {
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
    ) -> Result<ProductCommandAdmission, ProductSurfaceError> {
        self.records
            .lock()
            .expect("lock")
            .push((context.clone(), command.clone()));
        self.result.clone()
    }
}

#[derive(Clone)]
struct RecordedInvoke {
    caller: ProductSurfaceCaller,
    request: ProductSurfaceInvokeRequest,
}

struct RecordingCommandSurface {
    invokes: Mutex<Vec<RecordedInvoke>>,
    result: Result<ProductSurfaceInvokeResponse, ProductSurfaceError>,
}

impl RecordingCommandSurface {
    fn new(result: Result<ProductSurfaceInvokeResponse, ProductSurfaceError>) -> Self {
        Self {
            invokes: Mutex::new(Vec::new()),
            result,
        }
    }

    fn output(output: serde_json::Value) -> Self {
        Self::new(Ok(ProductSurfaceInvokeResponse { output }))
    }

    fn failing(error: ProductSurfaceError) -> Self {
        Self::new(Err(error))
    }

    fn invokes(&self) -> Vec<RecordedInvoke> {
        self.invokes.lock().expect("lock").clone()
    }
}

#[async_trait]
impl ProductSurface for RecordingCommandSurface {
    async fn invoke(
        &self,
        caller: ProductSurfaceCaller,
        request: ProductSurfaceInvokeRequest,
    ) -> Result<ProductSurfaceInvokeResponse, ProductSurfaceError> {
        self.invokes
            .lock()
            .expect("lock")
            .push(RecordedInvoke { caller, request });
        self.result.clone()
    }

    async fn query(
        &self,
        _caller: ProductSurfaceCaller,
        _request: ironclaw_host_api::ProductSurfaceQueryRequest,
    ) -> Result<ironclaw_host_api::ProductSurfaceQueryPage, ProductSurfaceError> {
        Err(ProductSurfaceError::internal())
    }

    async fn stream_events(
        &self,
        _caller: ProductSurfaceCaller,
        _request: ironclaw_host_api::ProductSurfaceStreamRequest,
    ) -> Result<ironclaw_host_api::ProductSurfaceStreamResponse, ProductSurfaceError> {
        Err(ProductSurfaceError::internal())
    }
}

#[tokio::test]
async fn command_payload_invokes_product_surface_not_inbound_turn_service() {
    let inbound = Arc::new(FakeInboundTurnService::new());
    let ledger = Arc::new(FakeIdempotencyLedger::new());
    let binding = Arc::new(FakeConversationBindingService::new());
    let admission_service = Arc::new(RecordingProductCommandAdmissionService::allowing());
    let command_surface = Arc::new(RecordingCommandSurface::output(serde_json::json!({
        "ok": true
    })));
    let workflow = DefaultProductSurface::new(inbound.clone(), ledger.clone(), binding)
        .with_product_command_admission_service(admission_service)
        .with_product_command_surface(command_surface.clone());
    let envelope =
        sample_command_envelope("command-model", "model", "gpt-5-mini --ignored-for-now");

    let ack = workflow.submit_inbound(envelope).await.expect("accept");

    let ProductInboundAck::CommandResult { command, payload } = ack else {
        panic!("expected command result ack");
    };
    assert_eq!(command, "model");
    assert_eq!(payload.as_value().get("ok"), Some(&serde_json::json!(true)));
    assert_eq!(inbound.accepted_count(), 0);
    assert_eq!(inbound.attempt_count(), 0);
    assert_eq!(inbound.replay_attempt_count(), 0);

    let invokes = command_surface.invokes();
    assert_eq!(invokes.len(), 1);
    assert_eq!(
        invokes[0].request.operation_id.as_str(),
        PRODUCT_MODEL_COMMAND_OPERATION_ID
    );
    assert_eq!(invokes[0].caller.tenant_id.as_str(), "tenant:install_alpha");
    assert_eq!(invokes[0].caller.user_id.as_str(), "user:user1");

    let settled = ledger.settled_actions();
    assert_eq!(settled.len(), 1);
    assert!(matches!(
        settled[0].dispatch_kind,
        Some(ActionDispatchKind::Command { .. })
    ));
}

#[tokio::test]
async fn lifecycle_command_uses_lifecycle_product_surface_operation() {
    let inbound = Arc::new(FakeInboundTurnService::new());
    let ledger = Arc::new(FakeIdempotencyLedger::new());
    let binding = Arc::new(FakeConversationBindingService::new());
    let admission_service = Arc::new(RecordingProductCommandAdmissionService::allowing());
    let command_surface = Arc::new(RecordingCommandSurface::output(serde_json::json!({
        "phase": "installed"
    })));
    let workflow = DefaultProductSurface::new(inbound.clone(), ledger, binding)
        .with_product_command_admission_service(admission_service)
        .with_product_command_surface(command_surface.clone());
    let envelope =
        sample_command_envelope("command-extension-install", "extension_install", "github");

    let ack = workflow.submit_inbound(envelope).await.expect("accept");

    let ProductInboundAck::CommandResult { command, payload } = ack else {
        panic!("expected lifecycle command result ack");
    };
    assert_eq!(command, "extension_install");
    assert_eq!(
        payload
            .as_value()
            .get("phase")
            .and_then(serde_json::Value::as_str),
        Some("installed")
    );
    assert_eq!(inbound.accepted_count(), 0);
    let invokes = command_surface.invokes();
    assert_eq!(invokes.len(), 1);
    assert_eq!(
        invokes[0].request.operation_id.as_str(),
        PRODUCT_LIFECYCLE_COMMAND_OPERATION_ID
    );
}

#[tokio::test]
async fn malformed_known_lifecycle_command_rejects_before_admission() {
    let inbound = Arc::new(FakeInboundTurnService::new());
    let ledger = Arc::new(FakeIdempotencyLedger::new());
    let binding = Arc::new(FakeConversationBindingService::new());
    let admission_service = Arc::new(RecordingProductCommandAdmissionService::allowing());
    let command_surface = Arc::new(RecordingCommandSurface::output(serde_json::json!({})));
    let workflow = DefaultProductSurface::new(inbound.clone(), ledger.clone(), binding)
        .with_product_command_admission_service(admission_service.clone())
        .with_product_command_surface(command_surface.clone());
    let envelope = sample_command_envelope("command-extension-invalid", "extension_install", "{}");

    let ack = workflow.submit_inbound(envelope).await.expect("accept");

    assert!(matches!(
        ack,
        ProductInboundAck::Rejected(rejection)
            if rejection.kind == ironclaw_product::ProductRejectionKind::InvalidRequest
    ));
    assert!(admission_service.records().is_empty());
    assert!(command_surface.invokes().is_empty());
    assert_eq!(inbound.accepted_count(), 0);
    assert_eq!(ledger.settled_count(), 1);
}

#[tokio::test]
async fn command_admission_receives_authority_context_and_action_metadata() {
    let inbound = Arc::new(FakeInboundTurnService::new());
    let ledger = Arc::new(FakeIdempotencyLedger::new());
    let binding = Arc::new(FakeConversationBindingService::new());
    let admission_service = Arc::new(RecordingProductCommandAdmissionService::allowing());
    let command_surface = Arc::new(RecordingCommandSurface::output(serde_json::json!({})));
    let workflow = DefaultProductSurface::new(inbound.clone(), ledger.clone(), binding)
        .with_product_command_admission_service(admission_service.clone())
        .with_product_command_surface(command_surface);
    let envelope = sample_command_envelope("command-context", "status", "");
    let expected_adapter_id = envelope.adapter_id().clone();
    let expected_installation_id = envelope.installation_id().clone();
    let expected_actor = envelope.external_actor_ref().clone();
    let expected_conversation = envelope.external_conversation_ref().clone();
    let expected_auth_claim = envelope.auth_claim().clone();
    let expected_received_at = envelope.received_at();

    let ack = workflow.submit_inbound(envelope).await.expect("accept");

    assert!(matches!(ack, ProductInboundAck::Rejected(_)));
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
        ProductSurfaceError::service_unavailable(true),
    ));
    let command_surface = Arc::new(RecordingCommandSurface::output(serde_json::json!({})));
    let workflow = DefaultProductSurface::new(inbound.clone(), ledger.clone(), binding)
        .with_product_command_admission_service(admission_service)
        .with_product_command_surface(command_surface.clone());
    let envelope = sample_command_envelope("command-admission-error", "model", "gpt-5-mini");

    let err = workflow
        .submit_inbound(envelope)
        .await
        .expect_err("transient admission error must bubble");

    assert!(err.is_retryable());
    assert!(command_surface.invokes().is_empty());
    assert_eq!(inbound.accepted_count(), 0);
    assert_eq!(ledger.settled_count(), 0);
    assert_eq!(ledger.released_count(), 1);
}

#[tokio::test]
async fn command_surface_error_releases_idempotency_lease() {
    let inbound = Arc::new(FakeInboundTurnService::new());
    let ledger = Arc::new(FakeIdempotencyLedger::new());
    let binding = Arc::new(FakeConversationBindingService::new());
    let admission_service = Arc::new(RecordingProductCommandAdmissionService::allowing());
    let command_surface = Arc::new(RecordingCommandSurface::failing(
        ProductSurfaceError::service_unavailable(true),
    ));
    let workflow = DefaultProductSurface::new(inbound.clone(), ledger.clone(), binding)
        .with_product_command_admission_service(admission_service)
        .with_product_command_surface(command_surface.clone());
    let envelope = sample_command_envelope("command-surface-error", "model", "gpt-5-mini");

    let err = workflow
        .submit_inbound(envelope)
        .await
        .expect_err("transient command surface error must bubble");

    assert!(err.is_retryable());
    assert_eq!(command_surface.invokes().len(), 1);
    assert_eq!(inbound.accepted_count(), 0);
    assert_eq!(ledger.settled_count(), 0);
    assert_eq!(ledger.released_count(), 1);
}

#[tokio::test]
async fn default_command_surface_rejects_when_admission_is_supplied() {
    let inbound = Arc::new(FakeInboundTurnService::new());
    let ledger = Arc::new(FakeIdempotencyLedger::new());
    let binding = Arc::new(FakeConversationBindingService::new());
    let admission_service = Arc::new(RecordingProductCommandAdmissionService::allowing());
    let workflow = DefaultProductSurface::new(inbound.clone(), ledger.clone(), binding)
        .with_product_command_admission_service(admission_service);
    let envelope = sample_command_envelope("command-default-surface-reject", "model", "gpt-5-mini");

    let ack = workflow.submit_inbound(envelope).await.expect("accept");

    assert!(matches!(
        ack,
        ProductInboundAck::Rejected(rejection)
            if rejection.kind == ironclaw_product::ProductRejectionKind::PolicyDenied
    ));
    assert_eq!(inbound.accepted_count(), 0);
    assert_eq!(ledger.settled_count(), 1);
}
