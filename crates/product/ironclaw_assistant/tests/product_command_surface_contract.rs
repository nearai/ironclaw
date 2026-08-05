//! Contract tests for product command dispatch through the product surface.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::Utc;
use ironclaw_assistant::{
    ActionDispatchKind, DefaultProductSurface, DirectConversationCommandAdmission,
    FakeConversationBindingService, FakeIdempotencyLedger, FakeInboundTurnService,
    PRODUCT_LIFECYCLE_COMMAND_OPERATION_ID, PRODUCT_MODEL_COMMAND_OPERATION_ID,
    PRODUCT_STATUS_COMMAND_OPERATION_ID, ProductCommand, ProductCommandAdmission,
    ProductCommandAdmissionService, ProductInboundAck, ProductRejectionKind,
};
use ironclaw_assistant::{
    AdapterInstallationId, AuthRequirement, ExternalActorRef, ExternalConversationRef,
    ExternalEventId, InboundCommandPayload, ProductAdapterId, ProductInboundEnvelope,
    ProductInboundPayload, ProductTriggerReason, ProtocolAuthEvidence, ResolveBindingRequest,
    ResolvedBinding, TrustedInboundContext,
};
use ironclaw_product_contracts::admin_users::AdminUserRole;
use ironclaw_product_contracts::binding::ProductBindingResolver;
use ironclaw_product_contracts::command::{CommandActorRoleResolver, ProductCommandContext};
use ironclaw_product_contracts::error::ProductOperationFailure;
use ironclaw_product_contracts::surface::{
    ProductSurface, ProductSurfaceCaller, ProductSurfaceError, ProductSurfaceErrorCode,
    ProductSurfaceInvokeRequest, ProductSurfaceInvokeResponse,
};

fn sample_command_envelope(
    event_suffix: &str,
    command: &str,
    arguments: &str,
) -> ProductInboundEnvelope {
    sample_command_envelope_with_trigger(
        event_suffix,
        command,
        arguments,
        ProductTriggerReason::BotCommand,
    )
}

fn sample_command_envelope_with_trigger(
    event_suffix: &str,
    command: &str,
    arguments: &str,
    trigger: ProductTriggerReason,
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
    let parsed = ironclaw_assistant::ParsedProductInbound::new(
        ExternalEventId::new(format!("evt:{event_suffix}")).expect("valid event"),
        ExternalActorRef::new("test", "user1", Option::<String>::None).expect("valid actor"),
        ExternalConversationRef::new(None, "conv1", None, None).expect("valid conversation"),
        ProductInboundPayload::Command(
            InboundCommandPayload::new(command, arguments, trigger).expect("valid command"),
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

struct FakeRoleResolver {
    role: Option<AdminUserRole>,
    fail: bool,
}

#[async_trait::async_trait]
impl CommandActorRoleResolver for FakeRoleResolver {
    async fn actor_role(
        &self,
        _context: &ProductCommandContext,
    ) -> Result<Option<AdminUserRole>, ProductSurfaceError> {
        if self.fail {
            return Err(ProductSurfaceError::from_status(
                ProductSurfaceErrorCode::Unavailable,
                503,
                true,
            ));
        }
        Ok(self.role)
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
        _request: ironclaw_product_contracts::surface::ProductSurfaceQueryRequest,
    ) -> Result<ironclaw_product_contracts::surface::ProductSurfaceQueryPage, ProductSurfaceError>
    {
        Err(ProductSurfaceError::internal())
    }

    async fn stream_events(
        &self,
        _caller: ProductSurfaceCaller,
        _request: ironclaw_product_contracts::surface::ProductSurfaceStreamRequest,
    ) -> Result<
        ironclaw_product_contracts::surface::ProductSurfaceStreamResponse,
        ProductSurfaceError,
    > {
        Err(ProductSurfaceError::internal())
    }
}

struct FirstCommandBindingService {
    inner: FakeConversationBindingService,
    resolve_count: AtomicUsize,
    lookup_count: AtomicUsize,
}

impl FirstCommandBindingService {
    fn new() -> Self {
        Self {
            inner: FakeConversationBindingService::new(),
            resolve_count: AtomicUsize::new(0),
            lookup_count: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl ProductBindingResolver for FirstCommandBindingService {
    async fn resolve_binding(
        &self,
        request: ResolveBindingRequest,
    ) -> Result<ResolvedBinding, ProductOperationFailure> {
        self.resolve_count.fetch_add(1, Ordering::SeqCst);
        self.inner.resolve_binding(request).await
    }

    async fn lookup_binding(
        &self,
        _request: ResolveBindingRequest,
    ) -> Result<ResolvedBinding, ProductOperationFailure> {
        self.lookup_count.fetch_add(1, Ordering::SeqCst);
        Err(ProductOperationFailure::BindingRequired {
            reason: "no conversation binding exists yet".to_string(),
        })
    }
}

#[tokio::test]
async fn first_command_after_pairing_resolves_a_conversation_binding() {
    let inbound = Arc::new(FakeInboundTurnService::new());
    let ledger = Arc::new(FakeIdempotencyLedger::new());
    let binding = Arc::new(FirstCommandBindingService::new());
    let admission_service = Arc::new(RecordingProductCommandAdmissionService::allowing());
    let command_surface = Arc::new(RecordingCommandSurface::output(serde_json::json!({
        "title": "Model"
    })));
    let workflow = DefaultProductSurface::new(inbound.clone(), ledger, binding.clone())
        .with_product_command_admission_service(admission_service)
        .with_product_command_surface(command_surface.clone());

    let ack = workflow
        .submit_inbound(sample_command_envelope("first-command", "model", ""))
        .await
        .expect("first command should establish its conversation binding");

    assert!(matches!(
        ack,
        ProductInboundAck::CommandResult { ref command, .. } if command == "model"
    ));
    assert_eq!(binding.resolve_count.load(Ordering::SeqCst), 1);
    assert_eq!(binding.lookup_count.load(Ordering::SeqCst), 0);
    assert_eq!(command_surface.invokes().len(), 1);
    assert_eq!(inbound.accepted_count(), 0);
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
async fn status_command_maps_to_its_operation_with_the_bound_thread() {
    let inbound = Arc::new(FakeInboundTurnService::new());
    let ledger = Arc::new(FakeIdempotencyLedger::new());
    let binding = Arc::new(FakeConversationBindingService::new());
    let admission_service = Arc::new(RecordingProductCommandAdmissionService::allowing());
    let command_surface = Arc::new(RecordingCommandSurface::output(serde_json::json!({
        "title": "Status"
    })));
    let workflow = DefaultProductSurface::new(inbound, ledger, binding)
        .with_product_command_admission_service(admission_service)
        .with_product_command_surface(command_surface.clone());

    let ack = workflow
        .submit_inbound(sample_command_envelope("command-status", "status", ""))
        .await
        .expect("accept");

    assert!(matches!(
        ack,
        ProductInboundAck::CommandResult { ref command, .. } if command == "status"
    ));
    let invokes = command_surface.invokes();
    assert_eq!(invokes.len(), 1);
    assert_eq!(
        invokes[0].request.operation_id.as_str(),
        PRODUCT_STATUS_COMMAND_OPERATION_ID
    );
    assert!(
        invokes[0]
            .request
            .input
            .get("thread_id")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|thread_id| !thread_id.is_empty())
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
            if rejection.kind == ironclaw_assistant::ProductRejectionKind::InvalidRequest
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
        .with_product_command_surface(command_surface.clone());
    let envelope = sample_command_envelope("command-context", "progress", "");
    let expected_adapter_id = envelope.adapter_id().clone();
    let expected_installation_id = envelope.installation_id().clone();
    let expected_actor = envelope.external_actor_ref().clone();
    let expected_conversation = envelope.external_conversation_ref().clone();
    let expected_auth_claim = envelope.auth_claim().clone();
    let expected_received_at = envelope.received_at();

    let ack = workflow.submit_inbound(envelope).await.expect("accept");

    // Aliases are retired: `progress` is now an unrecognized token, so the
    // admission-time context is still captured (it records the raw requested
    // command before rejection), but the command itself resolves to `Unknown`
    // and is rejected before it ever reaches the command surface.
    assert!(matches!(
        ack,
        ProductInboundAck::Rejected(ref rejection)
            if rejection.kind == ProductRejectionKind::InvalidRequest
    ));
    let records = admission_service.records();
    assert_eq!(records.len(), 1);
    let (context, command) = &records[0];
    assert_eq!(
        command,
        &ProductCommand::Unknown {
            name: "progress".to_string(),
            arguments: String::new(),
        }
    );
    assert_eq!(context.requested_command, "progress");
    assert_eq!(context.adapter_id, expected_adapter_id);
    assert_eq!(context.installation_id, expected_installation_id);
    assert_eq!(context.external_actor_ref, expected_actor);
    assert_eq!(context.external_conversation_ref, expected_conversation);
    assert_eq!(context.auth_claim, expected_auth_claim);
    assert_eq!(context.trigger, ProductTriggerReason::BotCommand);
    assert_eq!(context.received_at, expected_received_at);
    assert!(command_surface.invokes().is_empty());

    let settled = ledger.settled_actions();
    assert_eq!(settled.len(), 1);
    assert_eq!(context.action_id, settled[0].action_id);
    assert_eq!(context.fingerprint, settled[0].fingerprint);
}

#[tokio::test]
async fn manifest_command_admission_is_exact_and_blocks_sensitive_handlers() {
    for (suffix, command, arguments) in [
        // Retired alias: `progress` is now just an unknown token, not a
        // registered command's alias — the exact-match admission gate must
        // still reject it rather than let it slip past a `status`-only allowlist.
        ("retired-alias", "progress", ""),
        (
            "model-provider",
            "model",
            "set-provider openai --model gpt-5",
        ),
        ("extension-configure", "extension_configure", "slack"),
        ("skill-remove", "skill_remove", "demo"),
    ] {
        let inbound = Arc::new(FakeInboundTurnService::new());
        let ledger = Arc::new(FakeIdempotencyLedger::new());
        let binding = Arc::new(FakeConversationBindingService::new());
        let admission = Arc::new(
            DirectConversationCommandAdmission::new(
                ["status"],
                Arc::new(FakeRoleResolver {
                    role: Some(AdminUserRole::Member),
                    fail: false,
                }),
            )
            .expect("status is a registered command"),
        );
        let command_surface = Arc::new(RecordingCommandSurface::output(serde_json::json!({})));
        let workflow = DefaultProductSurface::new(inbound.clone(), ledger.clone(), binding)
            .with_product_command_admission_service(admission)
            .with_product_command_surface(command_surface.clone());
        let envelope = sample_command_envelope_with_trigger(
            suffix,
            command,
            arguments,
            ProductTriggerReason::DirectChat,
        );

        let ack = workflow.submit_inbound(envelope).await.expect("settle");

        assert!(
            matches!(
                ack,
                ProductInboundAck::Rejected(ref rejection)
                    if rejection.kind == ProductRejectionKind::InvalidRequest
            ),
            "disabled command {command} must be rejected: {ack:?}"
        );
        assert!(
            command_surface.invokes().is_empty(),
            "disabled command {command} reached its handler"
        );
        assert_eq!(inbound.accepted_count(), 0);
        assert_eq!(ledger.settled_count(), 1);
    }
}

#[tokio::test]
async fn manifest_command_admission_is_fail_closed_when_empty() {
    let inbound = Arc::new(FakeInboundTurnService::new());
    let ledger = Arc::new(FakeIdempotencyLedger::new());
    let binding = Arc::new(FakeConversationBindingService::new());
    let admission = Arc::new(
        DirectConversationCommandAdmission::new(
            std::iter::empty::<&str>(),
            Arc::new(FakeRoleResolver {
                role: Some(AdminUserRole::Member),
                fail: false,
            }),
        )
        .expect("empty allowlist is valid"),
    );
    let command_surface = Arc::new(RecordingCommandSurface::output(serde_json::json!({})));
    let workflow = DefaultProductSurface::new(inbound.clone(), ledger.clone(), binding)
        .with_product_command_admission_service(admission)
        .with_product_command_surface(command_surface.clone());

    let ack = workflow
        .submit_inbound(sample_command_envelope_with_trigger(
            "empty-status",
            "status",
            "",
            ProductTriggerReason::DirectChat,
        ))
        .await
        .expect("settle");

    assert!(matches!(
        ack,
        ProductInboundAck::Rejected(ref rejection)
            if rejection.kind == ProductRejectionKind::InvalidRequest
    ));
    assert!(command_surface.invokes().is_empty());
    assert_eq!(inbound.accepted_count(), 0);
    assert_eq!(ledger.settled_count(), 1);
}

#[tokio::test]
async fn manifest_command_admission_allows_status_only_in_direct_conversations() {
    for (suffix, trigger, expected_kind) in [
        ("direct", ProductTriggerReason::DirectChat, None),
        (
            "shared",
            ProductTriggerReason::BotCommand,
            Some(ProductRejectionKind::PolicyDenied),
        ),
    ] {
        let inbound = Arc::new(FakeInboundTurnService::new());
        let ledger = Arc::new(FakeIdempotencyLedger::new());
        let binding = Arc::new(FakeConversationBindingService::new());
        let admission = Arc::new(
            DirectConversationCommandAdmission::new(
                ["status"],
                Arc::new(FakeRoleResolver {
                    role: Some(AdminUserRole::Member),
                    fail: false,
                }),
            )
            .expect("status is a registered command"),
        );
        let command_surface = Arc::new(RecordingCommandSurface::output(serde_json::json!({
            "title": "Status"
        })));
        let workflow = DefaultProductSurface::new(inbound.clone(), ledger.clone(), binding)
            .with_product_command_admission_service(admission)
            .with_product_command_surface(command_surface.clone());

        let ack = workflow
            .submit_inbound(sample_command_envelope_with_trigger(
                suffix, "status", "", trigger,
            ))
            .await
            .expect("settle");

        match expected_kind {
            None => {
                assert!(matches!(
                    ack,
                    ProductInboundAck::CommandResult { ref command, .. } if command == "status"
                ));
                assert_eq!(command_surface.invokes().len(), 1);
            }
            Some(kind) => {
                assert!(matches!(
                    ack,
                    ProductInboundAck::Rejected(ref rejection) if rejection.kind == kind
                ));
                assert!(command_surface.invokes().is_empty());
            }
        }
        assert_eq!(inbound.accepted_count(), 0);
        assert_eq!(ledger.settled_count(), 1);
    }
}

#[tokio::test]
async fn member_admin_action_is_access_denied_without_execution() {
    let inbound = Arc::new(FakeInboundTurnService::new());
    let ledger = Arc::new(FakeIdempotencyLedger::new());
    let binding = Arc::new(FakeConversationBindingService::new());
    let admission = Arc::new(
        DirectConversationCommandAdmission::new(
            ["model", "status"],
            Arc::new(FakeRoleResolver {
                role: Some(AdminUserRole::Member),
                fail: false,
            }),
        )
        .expect("model and status are registered commands"),
    );
    let command_surface = Arc::new(RecordingCommandSurface::output(serde_json::json!({})));
    let workflow = DefaultProductSurface::new(inbound.clone(), ledger.clone(), binding)
        .with_product_command_admission_service(admission)
        .with_product_command_surface(command_surface.clone());
    let envelope = sample_command_envelope_with_trigger(
        "member-admin-action",
        "model",
        "set gpt-x",
        ProductTriggerReason::DirectChat,
    );

    let ack = workflow.submit_inbound(envelope).await.expect("settle");

    assert!(matches!(
        ack,
        ProductInboundAck::Rejected(ref rejection)
            if rejection.kind == ProductRejectionKind::AccessDenied
    ));
    assert!(command_surface.invokes().is_empty());
    assert_eq!(inbound.accepted_count(), 0);
    assert_eq!(ledger.settled_count(), 1);
}

#[tokio::test]
async fn member_user_action_executes() {
    let inbound = Arc::new(FakeInboundTurnService::new());
    let ledger = Arc::new(FakeIdempotencyLedger::new());
    let binding = Arc::new(FakeConversationBindingService::new());
    let admission = Arc::new(
        DirectConversationCommandAdmission::new(
            ["model", "status"],
            Arc::new(FakeRoleResolver {
                role: Some(AdminUserRole::Member),
                fail: false,
            }),
        )
        .expect("model and status are registered commands"),
    );
    let command_surface = Arc::new(RecordingCommandSurface::output(serde_json::json!({
        "title": "Model"
    })));
    let workflow = DefaultProductSurface::new(inbound.clone(), ledger.clone(), binding)
        .with_product_command_admission_service(admission)
        .with_product_command_surface(command_surface.clone());
    let envelope = sample_command_envelope_with_trigger(
        "member-user-action",
        "model",
        "",
        ProductTriggerReason::DirectChat,
    );

    let ack = workflow.submit_inbound(envelope).await.expect("settle");

    assert!(matches!(
        ack,
        ProductInboundAck::CommandResult { ref command, .. } if command == "model"
    ));
    let invokes = command_surface.invokes();
    assert_eq!(invokes.len(), 1);
    assert_eq!(
        invokes[0].request.operation_id.as_str(),
        PRODUCT_MODEL_COMMAND_OPERATION_ID
    );
    assert_eq!(inbound.accepted_count(), 0);
    assert_eq!(ledger.settled_count(), 1);
}

#[tokio::test]
async fn admin_admin_action_executes() {
    let inbound = Arc::new(FakeInboundTurnService::new());
    let ledger = Arc::new(FakeIdempotencyLedger::new());
    let binding = Arc::new(FakeConversationBindingService::new());
    let admission = Arc::new(
        DirectConversationCommandAdmission::new(
            ["model", "status"],
            Arc::new(FakeRoleResolver {
                role: Some(AdminUserRole::Owner),
                fail: false,
            }),
        )
        .expect("model and status are registered commands"),
    );
    let command_surface = Arc::new(RecordingCommandSurface::output(serde_json::json!({
        "title": "Model"
    })));
    let workflow = DefaultProductSurface::new(inbound.clone(), ledger.clone(), binding)
        .with_product_command_admission_service(admission)
        .with_product_command_surface(command_surface.clone());
    let envelope = sample_command_envelope_with_trigger(
        "admin-admin-action",
        "model",
        "set gpt-x",
        ProductTriggerReason::DirectChat,
    );

    let ack = workflow.submit_inbound(envelope).await.expect("settle");

    assert!(matches!(
        ack,
        ProductInboundAck::CommandResult { ref command, .. } if command == "model"
    ));
    let invokes = command_surface.invokes();
    assert_eq!(invokes.len(), 1);
    assert_eq!(
        invokes[0].request.operation_id.as_str(),
        PRODUCT_MODEL_COMMAND_OPERATION_ID
    );
    assert_eq!(inbound.accepted_count(), 0);
    assert_eq!(ledger.settled_count(), 1);
}

#[tokio::test]
async fn resolver_failure_is_a_retryable_error_not_silent_membership() {
    let inbound = Arc::new(FakeInboundTurnService::new());
    let ledger = Arc::new(FakeIdempotencyLedger::new());
    let binding = Arc::new(FakeConversationBindingService::new());
    let admission = Arc::new(
        DirectConversationCommandAdmission::new(
            ["model", "status"],
            Arc::new(FakeRoleResolver {
                role: None,
                fail: true,
            }),
        )
        .expect("model and status are registered commands"),
    );
    let command_surface = Arc::new(RecordingCommandSurface::output(serde_json::json!({})));
    let workflow = DefaultProductSurface::new(inbound.clone(), ledger.clone(), binding)
        .with_product_command_admission_service(admission)
        .with_product_command_surface(command_surface.clone());
    let envelope = sample_command_envelope_with_trigger(
        "resolver-failure",
        "model",
        "set gpt-x",
        ProductTriggerReason::DirectChat,
    );

    let err = workflow
        .submit_inbound(envelope)
        .await
        .expect_err("resolver failure must bubble as a retryable error, not a silent role");

    assert!(err.is_retryable());
    assert!(command_surface.invokes().is_empty());
    assert_eq!(inbound.accepted_count(), 0);
    assert_eq!(ledger.settled_count(), 0);
    assert_eq!(ledger.released_count(), 1);
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
            if rejection.kind == ironclaw_assistant::ProductRejectionKind::PolicyDenied
    ));
    assert_eq!(inbound.accepted_count(), 0);
    assert_eq!(ledger.settled_count(), 1);
}
