use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use chrono::Utc;
use ironclaw_host_api::{AgentId, TenantId, ThreadId, UserId};
use ironclaw_loop_host::{
    HostManagedModelError, HostManagedModelGateway, HostManagedModelRequest,
    HostManagedModelResponse,
};
use ironclaw_product_workflow::WebUiAuthenticatedCaller;
use ironclaw_triggers::{TriggerFire, TriggerFireIdentity, TriggerId};
use ironclaw_turns::run_profile::LoopCapabilityPort;
use ironclaw_turns::{TurnRunId, TurnScope};
use tower::ServiceExt;

use super::*;
use crate::{
    RebornBuildInput, RebornRuntimeIdentity, RebornRuntimeInput, build_reborn_runtime,
    local_dev_runtime_policy,
};
use ironclaw_telegram_extension::ingress::TELEGRAM_UPDATES_PATH;

const TENANT: &str = "telegram-host-tenant";
const AGENT: &str = "telegram-host-agent";
const OPERATOR: &str = "telegram-host-operator";

#[derive(Debug)]
struct StaticGateway;

#[async_trait::async_trait]
impl HostManagedModelGateway for StaticGateway {
    async fn stream_model(
        &self,
        _request: HostManagedModelRequest,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        Ok(HostManagedModelResponse::assistant_reply("ok"))
    }

    async fn stream_model_with_capabilities(
        &self,
        request: HostManagedModelRequest,
        _capabilities: Arc<dyn LoopCapabilityPort>,
    ) -> Result<HostManagedModelResponse, HostManagedModelError> {
        self.stream_model(request).await
    }
}

async fn telegram_runtime() -> (crate::RebornRuntime, tempfile::TempDir) {
    telegram_runtime_with(|input| input).await
}

async fn telegram_runtime_with(
    customize: impl FnOnce(RebornRuntimeInput) -> RebornRuntimeInput,
) -> (crate::RebornRuntime, tempfile::TempDir) {
    let root = tempfile::tempdir().expect("tempdir"); // safety: test-only fixture
    let input = RebornRuntimeInput::from_services(
        RebornBuildInput::local_dev("telegram-host-owner", root.path().join("local-dev"))
            .with_runtime_policy(local_dev_runtime_policy().expect("local policy")), // safety: test-only fixture
    )
    .with_identity(RebornRuntimeIdentity {
        tenant_id: TENANT.to_string(),
        agent_id: AGENT.to_string(),
        source_binding_id: "telegram-host-source".to_string(),
        reply_target_binding_id: "telegram-host-reply".to_string(),
    })
    .with_model_gateway_override(Arc::new(StaticGateway));
    let runtime = build_reborn_runtime(customize(input))
        .await
        .expect("runtime builds"); // safety: test-only fixture
    (runtime, root)
}

fn host_config() -> TelegramHostRuntimeConfig {
    TelegramHostRuntimeConfig::new(
        TenantId::new(TENANT).expect("tenant"), // safety: test-only fixture
        AgentId::new(AGENT).expect("agent"),    // safety: test-only fixture
        None,
        UserId::new(OPERATOR).expect("operator"), // safety: test-only fixture
        Some("https://ironclaw.example".to_string()),
    )
}

/// Caller-level guard for the T7 assembly: mounts build against a real
/// local-dev runtime without any setup record, the public updates route is
/// mounted at the manifest-projected path and fails closed (401) while
/// unconfigured, the protected setup/pairing routes are mounted, and the
/// operator sees the Settings bot-setup card through the facade pair.
#[tokio::test]
async fn build_telegram_host_runtime_mounts_exposes_routes_and_facades_unconfigured() {
    let (runtime, _root) = telegram_runtime().await;

    let mounts = build_telegram_host_runtime_mounts(&runtime, host_config())
        .await
        .expect("telegram host mounts build without a setup record");

    assert_eq!(mounts.events.descriptors.len(), 1);
    assert_eq!(
        mounts.events.descriptors[0].route_pattern().as_str(),
        TELEGRAM_UPDATES_PATH,
        "updates route must mount at the manifest-projected path"
    );
    let response = mounts
        .events
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(TELEGRAM_UPDATES_PATH)
                .body(Body::from(r#"{"update_id":1}"#))
                .expect("request builds"),
        )
        .await
        .expect("router responds");
    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "unconfigured deployments must fail closed at the webhook"
    );

    let protected = mounts.protected_routes();
    assert!(
            protected
                .descriptors
                .iter()
                .any(|descriptor| descriptor.route_pattern().as_str()
                    == ironclaw_telegram_extension::channel_routes::WEBUI_V2_CHANNELS_TELEGRAM_SETUP_PATH),
            "setup route must be mounted"
        );
    assert!(
            protected
                .descriptors
                .iter()
                .any(|descriptor| descriptor.route_pattern().as_str()
                    == ironclaw_telegram_extension::channel_routes::WEBUI_V2_CHANNELS_TELEGRAM_PAIRING_PATH),
            "pairing route must be mounted"
        );

    // Facades: the operator sees the admin bot-setup card even before a
    // bot is configured; pairedness reads report not-connected.
    let operator_caller = WebUiAuthenticatedCaller::new(
        TenantId::new(TENANT).expect("tenant"),
        UserId::new(OPERATOR).expect("operator"),
        Some(AgentId::new(AGENT).expect("agent")),
        None,
    )
    .with_operator_webui_config(true);
    let channels = mounts
        .connectable_channels()
        .list_connectable_channels(operator_caller.clone())
        .await
        .expect("connectable channels list");
    assert_eq!(channels.channels.len(), 1, "admin setup card only");
    assert_eq!(channels.channels[0].channel, "telegram");
    let connections = mounts
        .channel_connection()
        .caller_channel_connections(operator_caller)
        .await
        .expect("caller connections");
    assert_eq!(connections.get("telegram"), Some(&false));

    runtime.shutdown().await.expect("runtime shuts down");
}

/// FIX-B wiring smoke, driven through the production mounts builder with
/// the trigger poller enabled: one build registers the outbound target
/// provider under the host-config key AND appends the triggered-run
/// delivery hook into the poller's post-submit slot; a second build for
/// the SAME config is tolerated (idempotent — no duplicate provider, no
/// duplicate hook, no error).
#[tokio::test]
async fn build_telegram_host_runtime_mounts_wires_outbound_provider_and_trigger_hook() {
    let (runtime, _root) = telegram_runtime_with(|input| {
        input.with_trigger_poller_settings(
            crate::TriggerPollerSettings::enabled_with_tenant_scoped_authorizer_for_test(),
        )
    })
    .await;
    assert!(
        !runtime.trigger_post_submit_hook_is_set(),
        "no delivery hook may exist before the host mounts are built"
    );

    let _mounts = build_telegram_host_runtime_mounts(&runtime, host_config())
        .await
        .expect("telegram host mounts build");

    assert!(
        runtime.trigger_post_submit_hook_is_set(),
        "mounts must append the Telegram triggered-run delivery hook"
    );
    let provider_key = telegram_outbound_delivery_target_provider_key(&host_config());
    assert!(
        runtime
            .outbound_delivery_target_provider_key_registered(&provider_key)
            .expect("provider key lookup"),
        "mounts must register the Telegram outbound delivery target provider"
    );

    // Same-config rebuild: provider already registered + hook key already
    // present must be tolerated, mirroring the Slack mounts idempotency.
    let _mounts_again = build_telegram_host_runtime_mounts(&runtime, host_config())
        .await
        .expect("second mounts build for the same config is idempotent");

    runtime.shutdown().await.expect("runtime shuts down");
}

#[tokio::test]
async fn unconfigured_dynamic_trigger_hook_records_terminal_skipped_outcome() {
    let (runtime, _root) = telegram_runtime_with(|input| {
        input.with_trigger_poller_settings(
            crate::TriggerPollerSettings::enabled_with_tenant_scoped_authorizer_for_test(),
        )
    })
    .await;
    let _mounts = build_telegram_host_runtime_mounts(&runtime, host_config())
        .await
        .expect("telegram host mounts build");
    let hook = runtime
        .trigger_post_submit_hook_for_test()
        .expect("mounted Telegram hook");
    let run_id = TurnRunId::new();
    let tenant_id = TenantId::new(TENANT).expect("tenant");
    let agent_id = AgentId::new(AGENT).expect("agent");
    let owner = UserId::new(OPERATOR).expect("owner");
    let fire = TriggerFire {
        identity: TriggerFireIdentity::new(tenant_id.clone(), TriggerId::new(), Utc::now()),
        creator_user_id: owner.clone(),
        agent_id: Some(agent_id.clone()),
        project_id: None,
        prompt: "unconfigured Telegram delivery".to_string(),
        delivery_target: None,
    };
    let scope = TurnScope::new_with_owner(
        tenant_id,
        Some(agent_id),
        None,
        ThreadId::new("telegram-unconfigured-trigger-thread").expect("thread"),
        Some(owner),
    );

    hook.on_trigger_submitted(fire, run_id, scope)
        .await
        .expect("unconfigured hook persists terminal outcome");

    let record = runtime
        .services()
        .local_runtime
        .as_ref()
        .expect("local runtime")
        .triggered_run_delivery
        .load_triggered_run_delivery(run_id)
        .await
        .expect("outcome lookup")
        .expect("terminal outcome persisted");
    assert_eq!(
        record.outcome,
        ironclaw_outbound::TriggeredRunDeliveryOutcomeKind::Skipped
    );

    runtime.shutdown().await.expect("runtime shuts down");
}
