#![cfg(feature = "root-llm-provider")]

use std::sync::Arc;

use chrono::Utc;
use ironclaw_product_adapters::{
    AdapterInstallationId, AuthRequirement, ExternalActorRef, ExternalConversationRef,
    ExternalEventId, InboundCommandPayload, ProductAdapterId, ProductInboundAck,
    ProductInboundEnvelope, ProductInboundPayload, ProductTriggerReason, ProductWorkflow,
    ProtocolAuthEvidence, TrustedInboundContext,
};
use ironclaw_product_workflow::{
    DefaultProductWorkflow, FakeConversationBindingService, FakeIdempotencyLedger,
    FakeInboundTurnService, ProductCommandAdmission, ProductCommandAdmissionService,
    ProductCommandContext, ProductWorkflowError,
};
use ironclaw_reborn_composition::{RebornProviderAdmin, RebornProviderAdminProductCommandService};
use ironclaw_reborn_config::{RebornBootConfig, RebornHome, RebornProfile};

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

struct AllowingCommandAdmissionService;

#[async_trait::async_trait]
impl ProductCommandAdmissionService for AllowingCommandAdmissionService {
    async fn admit(
        &self,
        _context: &ProductCommandContext,
        _command: &ironclaw_product_workflow::ProductCommand,
    ) -> Result<ProductCommandAdmission, ProductWorkflowError> {
        Ok(ProductCommandAdmission::Allowed)
    }
}

#[tokio::test]
async fn model_provider_command_executes_through_reborn_provider_admin_service() {
    let temp = tempfile::tempdir().expect("tempdir");
    let reborn_home = temp.path().join("reborn-home");
    let home =
        RebornHome::resolve_from_env_parts(Some(reborn_home.clone().into_os_string()), None, None)
            .expect("valid reborn home");
    let admin = RebornProviderAdmin::new(RebornBootConfig::new(home, RebornProfile::LocalDev));
    let command_service = Arc::new(RebornProviderAdminProductCommandService::new(admin));
    let inbound = Arc::new(FakeInboundTurnService::new());
    let ledger = Arc::new(FakeIdempotencyLedger::new());
    let binding = Arc::new(FakeConversationBindingService::new());
    let workflow = DefaultProductWorkflow::new(inbound.clone(), ledger, binding)
        .with_product_command_admission_service(Arc::new(AllowingCommandAdmissionService))
        .with_product_command_service(command_service);
    let envelope = sample_command_envelope(
        "command-model-provider",
        "model",
        "set-provider openai --model gpt-5-mini",
    );

    let ack = workflow.accept_inbound(envelope).await.expect("accept");

    let ProductInboundAck::CommandResult { command, payload } = ack else {
        panic!("expected provider-admin command result");
    };
    assert_eq!(command, "model");
    assert_eq!(payload.as_value()["provider_id"], "openai");
    assert_eq!(payload.as_value()["model"], "gpt-5-mini");
    assert_eq!(inbound.accepted_count(), 0);
    let config = std::fs::read_to_string(reborn_home.join("config.toml")).expect("read config");
    assert!(
        config.contains("provider_id = \"openai\""),
        "config: {config}"
    );
    assert!(
        config.contains("model = \"gpt-5-mini\""),
        "config: {config}"
    );
}
