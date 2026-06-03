//! Host-beta Slack Events API composition.
//!
//! This module is the single composition point for the native Slack route:
//! the CLI supplies explicit host config, and this module reuses the already
//! assembled Reborn runtime services instead of creating a second agent loop.

use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;

use ironclaw_conversations::InMemoryConversationServices;
use ironclaw_host_api::{AgentId, ProjectId, ResourceScope, TenantId, UserId};
use ironclaw_outbound::{InMemoryOutboundStateStore, OutboundStateStore};
use ironclaw_product_adapters::{
    AdapterInstallationId, DeliveryStatus, EgressCredentialHandle, ExternalActorRef,
    OutboundDeliverySink, ProductAdapter, ProductAdapterId,
};
use ironclaw_product_workflow::{
    DefaultInboundTurnService, DefaultProductWorkflow, InMemoryIdempotencyLedger,
    ProductConversationBindingService, ProductInstallationKey, ProductInstallationScope,
    StaticProductInstallationResolver,
};
use ironclaw_slack_v2_adapter::{
    SLACK_USER_ACTOR_KIND, SlackV2Adapter, SlackV2AdapterConfig,
    slack_request_signature_auth_requirement,
};
use ironclaw_wasm_product_adapters::{
    EgressPolicy, HmacWebhookAuth, NativeProductAdapterRunner, NativeProductAdapterRunnerConfig,
    WebhookAuth,
};
use secrecy::{ExposeSecret, SecretString};
use thiserror::Error;

use crate::RebornRuntime;
use crate::slack_delivery::{
    SlackFinalReplyDeliveryObserver, SlackFinalReplyDeliveryServices,
    SlackFinalReplyDeliverySettings,
};
use crate::slack_egress::{SlackProtocolHttpEgress, StaticSlackEgressCredentialProvider};
use crate::slack_serve::{
    SlackEventsRouteState, SlackInstallationRecord, SlackInstallationSelector,
    StaticSlackInstallationResolver, slack_events_route_mount,
};
use crate::webui_serve::PublicRouteMount;

const SLACK_ADAPTER_ID: &str = "slack_v2";
const SLACK_BOT_TOKEN_HANDLE: &str = "slack_bot_token";
const SLACK_SIGNATURE_HEADER: &str = "X-Slack-Signature";
const SLACK_TIMESTAMP_HEADER: &str = "X-Slack-Request-Timestamp";
const SLACK_WEBHOOK_WORKFLOW_TIMEOUT: Duration = Duration::from_secs(2);
const SLACK_MAX_IN_FLIGHT_WEBHOOKS: usize = 64;

struct NoopSlackDeliverySink;

#[async_trait::async_trait]
impl OutboundDeliverySink for NoopSlackDeliverySink {
    async fn record(&self, _status: DeliveryStatus) {}
}

#[derive(Clone)]
pub struct SlackHostBetaConfig {
    pub tenant_id: TenantId,
    pub agent_id: AgentId,
    pub project_id: Option<ProjectId>,
    pub installation_id: AdapterInstallationId,
    pub installation_selector: SlackInstallationSelector,
    pub slack_actor: ExternalActorRef,
    pub user_id: UserId,
    pub signing_secret: SecretString,
    pub bot_token: SecretString,
}

impl SlackHostBetaConfig {
    // arch-exempt: too_many_args, needs SlackHostBetaBootstrap aggregation, plan #4418
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        tenant_id: TenantId,
        agent_id: AgentId,
        project_id: Option<ProjectId>,
        installation_id: String,
        team_id: String,
        api_app_id: Option<String>,
        slack_user_id: String,
        user_id: UserId,
        signing_secret: SecretString,
        bot_token: SecretString,
    ) -> Result<Self, SlackHostBetaBuildError> {
        let installation_id = AdapterInstallationId::new(installation_id)
            .map_err(|reason| invalid_config("installation_id", reason.to_string()))?;
        let installation_selector = match api_app_id {
            Some(api_app_id) => SlackInstallationSelector::app_team(api_app_id, team_id),
            None => SlackInstallationSelector::team(team_id),
        };
        let slack_actor =
            ExternalActorRef::new(SLACK_USER_ACTOR_KIND, slack_user_id, None::<String>)
                .map_err(|reason| invalid_config("slack_user_id", reason.to_string()))?;
        Ok(Self {
            tenant_id,
            agent_id,
            project_id,
            installation_id,
            installation_selector,
            slack_actor,
            user_id,
            signing_secret,
            bot_token,
        })
    }
}

impl std::fmt::Debug for SlackHostBetaConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SlackHostBetaConfig")
            .field("tenant_id", &self.tenant_id)
            .field("agent_id", &self.agent_id)
            .field("project_id", &self.project_id)
            .field("installation_id", &self.installation_id)
            .field("installation_selector", &self.installation_selector)
            .field("slack_actor", &self.slack_actor)
            .field("user_id", &self.user_id)
            .field("signing_secret", &"[REDACTED]")
            .field("bot_token", &"[REDACTED]")
            .finish()
    }
}

#[derive(Debug, Error)]
pub enum SlackHostBetaBuildError {
    #[error("Slack host-beta requires local runtime HTTP egress")]
    RuntimeHttpEgressUnavailable,
    #[error("invalid Slack host-beta config field {field}: {reason}")]
    InvalidConfig { field: &'static str, reason: String },
}

pub fn build_slack_events_route_mount(
    runtime: &RebornRuntime,
    config: SlackHostBetaConfig,
) -> Result<PublicRouteMount, SlackHostBetaBuildError> {
    tracing::warn!(
        "Slack host-beta uses in-memory conversation bindings, idempotency ledger, and outbound state; Slack continuity, retry deduplication, and delivery state are lost on process restart"
    );

    let adapter_id = ProductAdapterId::new(SLACK_ADAPTER_ID)
        .map_err(|reason| invalid_config("adapter_id", reason.to_string()))?;
    let token_handle = EgressCredentialHandle::new(SLACK_BOT_TOKEN_HANDLE)
        .map_err(|reason| invalid_config("bot_token_handle", reason.to_string()))?;
    let adapter: Arc<dyn ProductAdapter> = Arc::new(SlackV2Adapter::new(SlackV2AdapterConfig {
        adapter_id: adapter_id.clone(),
        installation_id: config.installation_id.clone(),
        egress_credential_handle: token_handle.clone(),
        auth_requirement: slack_request_signature_auth_requirement(),
    }));

    let conversations = Arc::new(InMemoryConversationServices::default());
    let conversation_port: Arc<dyn ironclaw_conversations::ConversationBindingService> =
        conversations.clone();
    let actor_pairings: Arc<dyn ironclaw_conversations::ConversationActorPairingService> =
        conversations;
    let scope = ProductInstallationScope::with_default_scope(
        config.tenant_id.clone(),
        config.agent_id.clone(),
        config.project_id.clone(),
    )
    .with_preconfigured_actor_binding(config.slack_actor.clone(), config.user_id.clone());
    let installation_resolver = StaticProductInstallationResolver::new([(
        ProductInstallationKey::new(adapter_id, config.installation_id.clone()),
        scope,
    )]);
    let binding = ProductConversationBindingService::new(conversation_port, installation_resolver)
        .with_actor_pairings(actor_pairings);

    let inbound = Arc::new(DefaultInboundTurnService::new(
        binding.clone(),
        runtime.webui_thread_service(),
        runtime.webui_turn_coordinator(),
    ));
    let workflow = Arc::new(
        DefaultProductWorkflow::new(
            inbound,
            Arc::new(InMemoryIdempotencyLedger::new()),
            Arc::new(binding.clone()),
        )
        .with_approval_interaction_service(runtime.webui_approval_interaction_service())
        .with_auth_interaction_service(runtime.webui_auth_interaction_service()),
    );

    let runner = Arc::new(NativeProductAdapterRunner::with_config(
        adapter.clone(),
        workflow,
        WebhookAuth::Hmac(HmacWebhookAuth::new(
            SLACK_SIGNATURE_HEADER,
            SLACK_TIMESTAMP_HEADER,
            config.signing_secret.expose_secret().as_bytes().to_vec(),
            config.installation_id.as_str(),
        )),
        NativeProductAdapterRunnerConfig::new(
            SLACK_WEBHOOK_WORKFLOW_TIMEOUT,
            NonZeroUsize::new(SLACK_MAX_IN_FLIGHT_WEBHOOKS)
                .ok_or_else(|| invalid_config("max_in_flight", "must be non-zero".to_string()))?,
        ),
    ));

    let local_runtime = runtime
        .services()
        .local_runtime
        .as_ref()
        .ok_or(SlackHostBetaBuildError::RuntimeHttpEgressUnavailable)?;
    let runtime_http_egress = local_runtime
        .runtime_http_egress
        .clone()
        .ok_or(SlackHostBetaBuildError::RuntimeHttpEgressUnavailable)?;
    let egress = Arc::new(SlackProtocolHttpEgress::new(
        runtime_http_egress,
        Arc::new(StaticSlackEgressCredentialProvider::new(
            token_handle,
            config.bot_token.expose_secret().to_string(),
        )),
        EgressPolicy::new(adapter.declared_egress().to_vec()),
        ResourceScope::local_default(
            config.user_id.clone(),
            ironclaw_host_api::InvocationId::new(),
        )
        .map_err(|reason| invalid_config("resource_scope", reason.to_string()))?,
    ));
    let outbound = Arc::new(InMemoryOutboundStateStore::default());
    let outbound_store: Arc<dyn OutboundStateStore> = outbound.clone();
    let preferences: Arc<dyn ironclaw_outbound::CommunicationPreferenceRepository> = outbound;
    let delivery_sink: Arc<dyn OutboundDeliverySink> = Arc::new(NoopSlackDeliverySink);
    let observer = Arc::new(SlackFinalReplyDeliveryObserver::with_settings(
        SlackFinalReplyDeliveryServices {
            binding_service: Arc::new(binding),
            thread_service: runtime.webui_thread_service(),
            turn_coordinator: runtime.webui_turn_coordinator(),
            outbound_store,
            communication_preferences: preferences,
            adapter,
            egress,
            delivery_sink,
        },
        SlackFinalReplyDeliverySettings::default(),
    ));

    let slack_resolver = StaticSlackInstallationResolver::new([SlackInstallationRecord::new(
        config.tenant_id,
        config.installation_id,
        config.installation_selector,
        runner,
    )
    .with_workflow_observer(observer)]);

    Ok(slack_events_route_mount(
        SlackEventsRouteState::from_resolver(Arc::new(slack_resolver)),
    ))
}

fn invalid_config(field: &'static str, reason: String) -> SlackHostBetaBuildError {
    SlackHostBetaBuildError::InvalidConfig { field, reason }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use hmac::{Hmac, Mac};
    use http_body_util::BodyExt;
    use ironclaw_loop_support::{
        HostManagedModelError, HostManagedModelGateway, HostManagedModelRequest,
        HostManagedModelResponse,
    };
    use ironclaw_turns::run_profile::LoopCapabilityPort;
    use secrecy::ExposeSecret;
    use tower::ServiceExt;

    use super::*;
    use crate::{
        RebornBuildInput, RebornRuntimeIdentity, RebornRuntimeInput, SLACK_EVENTS_PATH,
        build_reborn_runtime, local_dev_runtime_policy,
    };

    const TENANT: &str = "tenant:slack-host";
    const AGENT: &str = "agent:slack-host";
    const PROJECT: &str = "project:slack-host";
    const USER: &str = "user:slack-host";
    const INSTALLATION: &str = "install_host_beta";
    const TEAM: &str = "T-HOST";
    const API_APP: &str = "A-HOST";
    const SLACK_USER: &str = "U-HOST";
    const SECRET: &str = "host-signing-secret";

    type HmacSha256 = Hmac<sha2::Sha256>;

    #[tokio::test]
    async fn build_slack_events_route_mount_builds_signed_route_from_reborn_runtime() {
        let root = tempfile::tempdir().expect("tempdir");
        let runtime = build_reborn_runtime(
            RebornRuntimeInput::from_services(
                RebornBuildInput::local_dev("slack-host-beta-owner", root.path().join("local-dev"))
                    .with_runtime_policy(local_dev_runtime_policy().expect("local policy")),
            )
            .with_identity(RebornRuntimeIdentity {
                tenant_id: TENANT.to_string(),
                agent_id: AGENT.to_string(),
                source_binding_id: "slack-host-source".to_string(),
                reply_target_binding_id: "slack-host-reply".to_string(),
            })
            .with_model_gateway_override(Arc::new(StaticGateway)),
        )
        .await
        .expect("runtime builds");

        let mount = build_slack_events_route_mount(&runtime, config()).expect("route builds");
        assert_eq!(mount.descriptors.len(), 1);
        assert!(mount.drain.is_some());

        let body = r#"{"type":"url_verification","challenge":"reborn-slack-ok"}"#;
        let timestamp = current_unix_timestamp();
        let response = mount
            .router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(SLACK_EVENTS_PATH)
                    .header(SLACK_TIMESTAMP_HEADER, timestamp.to_string())
                    .header(SLACK_SIGNATURE_HEADER, slack_signature(timestamp, body))
                    .body(Body::from(body))
                    .expect("request builds"),
            )
            .await
            .expect("router responds");

        assert_eq!(response.status(), StatusCode::OK);
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("body collects")
            .to_bytes();
        assert!(String::from_utf8_lossy(&bytes).contains("reborn-slack-ok"));

        runtime.shutdown().await.expect("runtime shuts down");
    }

    #[test]
    fn slack_host_beta_config_binds_slack_actor_to_reborn_user() {
        let config = config();

        assert_eq!(config.installation_id.as_str(), INSTALLATION);
        assert_eq!(config.slack_actor.kind(), SLACK_USER_ACTOR_KIND);
        assert_eq!(config.slack_actor.id(), SLACK_USER);
        assert_eq!(config.user_id, UserId::new(USER).expect("user id"));
        assert_eq!(config.signing_secret.expose_secret(), SECRET);
        assert_eq!(config.bot_token.expose_secret(), "xoxb-host-token");
    }

    fn config() -> SlackHostBetaConfig {
        SlackHostBetaConfig::new(
            TenantId::new(TENANT).expect("tenant"),
            AgentId::new(AGENT).expect("agent"),
            Some(ProjectId::new(PROJECT).expect("project")),
            INSTALLATION.to_string(),
            TEAM.to_string(),
            Some(API_APP.to_string()),
            SLACK_USER.to_string(),
            UserId::new(USER).expect("user"),
            SecretString::from(SECRET),
            SecretString::from("xoxb-host-token"),
        )
        .expect("valid config")
    }

    fn current_unix_timestamp() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after Unix epoch")
            .as_secs()
    }

    fn slack_signature(timestamp: u64, body: &str) -> String {
        let mut mac =
            HmacSha256::new_from_slice(SECRET.as_bytes()).expect("HMAC accepts any key size");
        mac.update(format!("v0:{timestamp}:").as_bytes());
        mac.update(body.as_bytes());
        format!("v0={:x}", mac.finalize().into_bytes())
    }

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
}
