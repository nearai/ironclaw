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
    AdapterInstallationId, DeclaredEgressHost, DeclaredEgressTarget, DeliveryStatus,
    EgressCredentialHandle, ExternalActorRef, OutboundDeliverySink, ProductAdapter,
    ProductAdapterId, ProtocolHttpEgress,
};
use ironclaw_product_workflow::{
    DefaultInboundTurnService, DefaultProductWorkflow, InMemoryIdempotencyLedger,
    ProductActorUserResolutionRequest, ProductActorUserResolver, ProductConversationBindingService,
    ProductInstallationKey, ProductInstallationScope, ProductWorkflowError,
    StaticProductInstallationResolver,
};
use ironclaw_slack_v2_adapter::{
    SLACK_API_HOST, SLACK_USER_ACTOR_KIND, SLACK_V2_ADAPTER_ID, SlackV2Adapter,
    SlackV2AdapterConfig, slack_request_signature_auth_requirement,
};
use ironclaw_wasm_product_adapters::{
    EgressPolicy, HmacWebhookAuth, NativeProductAdapterRunner, NativeProductAdapterRunnerConfig,
    WebhookAuth,
};
use secrecy::{ExposeSecret, SecretString};
use thiserror::Error;

use crate::RebornRuntime;
use crate::slack_actor_identity::SlackUserIdentityActorResolver;
use crate::slack_delivery::{
    SlackFinalReplyDeliveryObserver, SlackFinalReplyDeliveryServices,
    SlackFinalReplyDeliverySettings,
};
use crate::slack_egress::{SlackProtocolHttpEgress, StaticSlackEgressCredentialProvider};
use crate::slack_host_state::FilesystemSlackHostState;
use crate::slack_pairing_notifier::SlackPairingChallengeHttpNotifier;
use crate::slack_personal_binding::{
    RebornUserIdentityBindingStore, SlackPersonalBindingInstallation,
    SlackPersonalUserBindingService,
};
use crate::slack_personal_binding_pairing::{
    SlackPairingActorResolver, SlackPersonalBindingPairingChallengeStore,
    SlackPersonalBindingPairingNotifier, SlackPersonalBindingPairingService,
};
use crate::slack_personal_binding_pairing_serve::SlackPersonalBindingPairingRouteConfig;
use crate::slack_serve::{
    SlackEventsRouteState, SlackInstallationRecord, SlackInstallationSelector,
    StaticSlackInstallationResolver, slack_events_route_mount,
};
use crate::webui_serve::PublicRouteMount;

const SLACK_BOT_TOKEN_HANDLE: &str = "slack_bot_token";
const SLACK_SIGNATURE_HEADER: &str = "X-Slack-Signature";
const SLACK_TIMESTAMP_HEADER: &str = "X-Slack-Request-Timestamp";
const SLACK_WEBHOOK_WORKFLOW_TIMEOUT: Duration = Duration::from_secs(2);
const SLACK_MAX_IN_FLIGHT_WEBHOOKS: usize = 64;
const SLACK_IDEMPOTENCY_LEDGER_SETTLED_LIMIT: usize = 10_000;

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
    /// Optional Slack actor retained only for legacy static personal-binding
    /// tests/config. Tenant app host-beta resolution uses durable personal
    /// bindings and does not require a preselected Slack user.
    pub slack_actor: Option<ExternalActorRef>,
    /// Host user used as the resource owner for Slack bot-token egress.
    pub user_id: UserId,
    pub signing_secret: SecretString,
    pub bot_token: SecretString,
}

pub struct SlackHostBetaConfigInput {
    pub tenant_id: TenantId,
    pub agent_id: AgentId,
    pub project_id: Option<ProjectId>,
    pub installation_id: String,
    pub team_id: String,
    pub api_app_id: Option<String>,
    pub slack_user_id: Option<String>,
    pub user_id: UserId,
    pub signing_secret: SecretString,
    pub bot_token: SecretString,
}

impl SlackHostBetaConfig {
    pub fn new(input: SlackHostBetaConfigInput) -> Result<Self, SlackHostBetaBuildError> {
        let installation_id = AdapterInstallationId::new(input.installation_id)
            .map_err(|reason| invalid_config("installation_id", reason.to_string()))?;
        let installation_selector = match input.api_app_id {
            Some(api_app_id) => SlackInstallationSelector::app_team(api_app_id, input.team_id),
            None => SlackInstallationSelector::team(input.team_id),
        };
        let slack_actor = input
            .slack_user_id
            .map(|slack_user_id| {
                ExternalActorRef::new(SLACK_USER_ACTOR_KIND, slack_user_id, None::<String>)
                    .map_err(|reason| invalid_config("slack_user_id", reason.to_string()))
            })
            .transpose()?;
        Ok(Self {
            tenant_id: input.tenant_id,
            agent_id: input.agent_id,
            project_id: input.project_id,
            installation_id,
            installation_selector,
            slack_actor,
            user_id: input.user_id,
            signing_secret: input.signing_secret,
            bot_token: input.bot_token,
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
    #[error("Slack host-beta requires durable host state")]
    DurableHostStateUnavailable,
    #[error(
        "Slack host-beta personal binding requires [slack].api_app_id for tenant app-scoped pairing"
    )]
    TenantAppSelectorRequired,
    #[error("invalid Slack host-beta config field {field}: {reason}")]
    InvalidConfig { field: &'static str, reason: String },
}

pub struct SlackHostBetaMounts {
    pub events: PublicRouteMount,
    pub personal_binding_pairing: SlackPersonalBindingPairingRouteConfig,
}

pub fn build_slack_events_route_mount(
    runtime: &RebornRuntime,
    config: SlackHostBetaConfig,
) -> Result<PublicRouteMount, SlackHostBetaBuildError> {
    build_slack_host_beta_mounts(runtime, config).map(|mounts| mounts.events)
}

pub fn build_slack_host_beta_mounts(
    runtime: &RebornRuntime,
    config: SlackHostBetaConfig,
) -> Result<SlackHostBetaMounts, SlackHostBetaBuildError> {
    if !matches!(
        config.installation_selector,
        SlackInstallationSelector::AppTeam { .. }
    ) {
        return Err(SlackHostBetaBuildError::TenantAppSelectorRequired);
    }
    let local_runtime = runtime
        .services()
        .local_runtime
        .as_ref()
        .ok_or(SlackHostBetaBuildError::DurableHostStateUnavailable)?;
    let state = Arc::new(FilesystemSlackHostState::new(
        Arc::clone(&local_runtime.host_state_filesystem),
        config.tenant_id.clone(),
        config.user_id.clone(),
        config.agent_id.clone(),
        config.project_id.clone(),
    ));
    let binding_store: Arc<dyn RebornUserIdentityBindingStore> = state.clone();
    let binding_service = SlackPersonalUserBindingService::new(
        [SlackPersonalBindingInstallation {
            tenant_id: config.tenant_id.clone(),
            installation_id: config.installation_id.clone(),
            selector: config.installation_selector.clone(),
        }],
        binding_store,
    );
    let token_handle = slack_bot_token_handle()?;
    let notifier: Arc<dyn SlackPersonalBindingPairingNotifier> =
        Arc::new(SlackPairingChallengeHttpNotifier::new(
            slack_protocol_egress(runtime, &config, token_handle.clone())?,
            token_handle,
        ));
    let challenge_store: Arc<dyn SlackPersonalBindingPairingChallengeStore> = state.clone();
    let pairing =
        SlackPersonalBindingPairingService::new(binding_service, challenge_store, notifier);
    let actor_user_resolver = Arc::new(SlackHostBetaActorUserResolver::new(
        config.installation_id.clone(),
        config.slack_actor.clone(),
        config.user_id.clone(),
        Arc::new(SlackUserIdentityActorResolver::new(state.clone())),
        Arc::new(SlackPairingActorResolver::new(state, pairing.clone())),
    ));
    let events = build_slack_events_route_mount_with_actor_user_resolver(
        runtime,
        config,
        actor_user_resolver,
    )?;

    Ok(SlackHostBetaMounts {
        events,
        personal_binding_pairing: SlackPersonalBindingPairingRouteConfig::new(pairing),
    })
}

pub fn build_slack_events_route_mount_with_actor_user_resolver(
    runtime: &RebornRuntime,
    config: SlackHostBetaConfig,
    actor_user_resolver: Arc<dyn ProductActorUserResolver>,
) -> Result<PublicRouteMount, SlackHostBetaBuildError> {
    // The resolver controls inbound Slack actor binding. `config.user_id` still
    // scopes the host-mediated Slack bot-token egress for this beta route.
    tracing::warn!(
        "Slack host-beta uses in-memory conversation bindings, idempotency ledger, and outbound state; Slack continuity, retry deduplication, and delivery state are lost on process restart"
    );
    let adapter_id = ProductAdapterId::new(SLACK_V2_ADAPTER_ID)
        .map_err(|reason| invalid_config("adapter_id", reason.to_string()))?;
    let token_handle = slack_bot_token_handle()?;
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
        conversations.clone();
    let scope = ProductInstallationScope::with_default_scope(
        config.tenant_id.clone(),
        config.agent_id.clone(),
        config.project_id.clone(),
    )
    .with_actor_user_resolver(actor_user_resolver, actor_pairings);
    let installation_resolver = StaticProductInstallationResolver::new([(
        ProductInstallationKey::new(adapter_id, config.installation_id.clone()),
        scope,
    )]);
    let binding = ProductConversationBindingService::new(conversation_port, installation_resolver);

    let inbound = Arc::new(DefaultInboundTurnService::new(
        binding.clone(),
        runtime.webui_thread_service(),
        runtime.webui_turn_coordinator(),
    ));
    let workflow = Arc::new(
        DefaultProductWorkflow::new(
            inbound,
            Arc::new(InMemoryIdempotencyLedger::with_settled_entry_limit(
                NonZeroUsize::new(SLACK_IDEMPOTENCY_LEDGER_SETTLED_LIMIT).ok_or_else(|| {
                    invalid_config("idempotency_ledger_limit", "must be non-zero".to_string())
                })?,
            )),
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

    let egress = slack_protocol_egress(runtime, &config, token_handle)?;
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

fn slack_bot_token_handle() -> Result<EgressCredentialHandle, SlackHostBetaBuildError> {
    EgressCredentialHandle::new(SLACK_BOT_TOKEN_HANDLE)
        .map_err(|reason| invalid_config("bot_token_handle", reason.to_string()))
}

fn slack_protocol_egress(
    runtime: &RebornRuntime,
    config: &SlackHostBetaConfig,
    token_handle: EgressCredentialHandle,
) -> Result<Arc<dyn ProtocolHttpEgress>, SlackHostBetaBuildError> {
    let local_runtime = runtime
        .services()
        .local_runtime
        .as_ref()
        .ok_or(SlackHostBetaBuildError::RuntimeHttpEgressUnavailable)?;
    let runtime_http_egress = local_runtime
        .runtime_http_egress
        .clone()
        .ok_or(SlackHostBetaBuildError::RuntimeHttpEgressUnavailable)?;
    Ok(Arc::new(SlackProtocolHttpEgress::new(
        runtime_http_egress,
        Arc::new(StaticSlackEgressCredentialProvider::new(
            token_handle.clone(),
            config.bot_token.expose_secret().to_string(),
        )),
        EgressPolicy::new(slack_declared_egress_targets(token_handle)),
        slack_egress_scope(config),
    )))
}

fn slack_egress_scope(config: &SlackHostBetaConfig) -> ResourceScope {
    ResourceScope {
        tenant_id: config.tenant_id.clone(),
        user_id: config.user_id.clone(),
        agent_id: Some(config.agent_id.clone()),
        project_id: config.project_id.clone(),
        mission_id: None,
        thread_id: None,
        invocation_id: ironclaw_host_api::InvocationId::new(),
    }
}

fn slack_declared_egress_targets(
    token_handle: EgressCredentialHandle,
) -> Vec<DeclaredEgressTarget> {
    vec![DeclaredEgressTarget::new(
        DeclaredEgressHost::new(SLACK_API_HOST).expect("static Slack host valid"),
        Some(token_handle),
    )]
}

#[derive(Clone)]
struct SlackHostBetaActorUserResolver {
    installation_id: AdapterInstallationId,
    legacy_slack_actor: Option<ExternalActorRef>,
    legacy_user_id: UserId,
    cached_identity: Arc<dyn ProductActorUserResolver>,
    pairing: Arc<dyn ProductActorUserResolver>,
}

impl SlackHostBetaActorUserResolver {
    fn new(
        installation_id: AdapterInstallationId,
        legacy_slack_actor: Option<ExternalActorRef>,
        legacy_user_id: UserId,
        cached_identity: Arc<dyn ProductActorUserResolver>,
        pairing: Arc<dyn ProductActorUserResolver>,
    ) -> Self {
        Self {
            installation_id,
            legacy_slack_actor,
            legacy_user_id,
            cached_identity,
            pairing,
        }
    }

    fn resolve_legacy_static_actor(
        &self,
        request: &ProductActorUserResolutionRequest,
    ) -> Option<UserId> {
        let legacy_actor = self.legacy_slack_actor.as_ref()?;
        if request.adapter_id.as_str() == SLACK_V2_ADAPTER_ID
            && request.installation_id == self.installation_id
            && request.external_actor_ref == *legacy_actor
        {
            return Some(self.legacy_user_id.clone());
        }
        None
    }
}

impl std::fmt::Debug for SlackHostBetaActorUserResolver {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SlackHostBetaActorUserResolver(..)")
    }
}

#[async_trait::async_trait]
impl ProductActorUserResolver for SlackHostBetaActorUserResolver {
    async fn resolve_product_actor_user(
        &self,
        request: ProductActorUserResolutionRequest,
    ) -> Result<Option<UserId>, ProductWorkflowError> {
        if let Some(user_id) = self.resolve_legacy_static_actor(&request) {
            return Ok(Some(user_id));
        }
        if let Some(user_id) = self
            .cached_identity
            .resolve_product_actor_user(request.clone())
            .await?
        {
            return Ok(Some(user_id));
        }
        self.pairing.resolve_product_actor_user(request).await
    }
}

fn invalid_config(field: &'static str, reason: String) -> SlackHostBetaBuildError {
    SlackHostBetaBuildError::InvalidConfig { field, reason }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};
    use std::time::{SystemTime, UNIX_EPOCH};

    use async_trait::async_trait;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use hmac::{Hmac, Mac};
    use http_body_util::BodyExt;
    use ironclaw_host_api::{
        RuntimeHttpEgress, RuntimeHttpEgressRequest, RuntimeHttpEgressResponse,
    };
    use ironclaw_loop_support::{
        HostManagedModelError, HostManagedModelGateway, HostManagedModelRequest,
        HostManagedModelResponse,
    };
    use ironclaw_product_workflow::{ProductActorUserResolutionRequest, ProductWorkflowError};
    use ironclaw_threads::{ListThreadsForScopeRequest, ThreadHistoryRequest, ThreadScope};
    use ironclaw_turns::run_profile::LoopCapabilityPort;
    use secrecy::ExposeSecret;
    use tower::ServiceExt;

    use super::*;
    use crate::slack_personal_binding_pairing_serve::{
        SLACK_PERSONAL_BINDING_PAIRING_REDEEM_PATH, slack_personal_binding_pairing_route_mount,
    };
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
        let (runtime, _root) = runtime().await;

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

    #[tokio::test]
    async fn custom_actor_user_resolver_routes_inbound_slack_event() {
        let (runtime, _root) = runtime().await;
        let resolver = Arc::new(RecordingProductActorUserResolver::new(
            UserId::new(USER).expect("user"),
        ));
        let mount = build_slack_events_route_mount_with_actor_user_resolver(
            &runtime,
            config(),
            resolver.clone(),
        )
        .expect("route builds");

        let body = dm_event_body();
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
        let calls = wait_for_resolver_calls(&resolver, 1).await;
        assert!(!calls.is_empty());
        assert_eq!(calls[0].adapter_id.as_str(), SLACK_V2_ADAPTER_ID);
        assert_eq!(calls[0].installation_id.as_str(), INSTALLATION);
        assert_eq!(calls[0].external_actor_ref.kind(), SLACK_USER_ACTOR_KIND);
        assert_eq!(calls[0].external_actor_ref.id(), SLACK_USER);

        runtime.shutdown().await.expect("runtime shuts down");
    }

    #[tokio::test]
    async fn build_slack_events_route_mount_fails_when_runtime_http_egress_unavailable() {
        let (mut runtime, _root) = runtime().await;
        runtime.set_local_runtime_http_egress_for_test(None);

        let error = match build_slack_events_route_mount(&runtime, config()) {
            Ok(_) => panic!("Slack route requires runtime HTTP egress"),
            Err(error) => error,
        };

        assert!(matches!(
            error,
            SlackHostBetaBuildError::RuntimeHttpEgressUnavailable
        ));
        runtime.shutdown().await.expect("runtime shuts down");
    }

    #[tokio::test]
    async fn build_slack_events_route_mount_dispatches_signed_event_callback() {
        let (mut runtime, _root) = runtime().await;
        let egress = Arc::new(RecordingRuntimeHttpEgress::default());
        runtime.set_local_runtime_http_egress_for_test(Some(egress.clone()));
        let mount = build_slack_events_route_mount(&runtime, config()).expect("route builds");
        let body = r#"{
            "type":"event_callback",
            "team_id":"T-HOST",
            "api_app_id":"A-HOST",
            "event_id":"Ev-host-beta-dispatch",
            "event":{"type":"message","channel_type":"im","user":"U-HOST","channel":"D-HOST","text":"hello","ts":"1710000000.000010"}
        }"#;
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
        if let Some(drain) = mount.drain.as_ref() {
            drain.drain().await;
        }
        let history = wait_for_slack_thread_history(&runtime).await;
        assert_eq!(history.messages.len(), 1);
        assert_eq!(history.messages[0].content.as_deref(), Some("hello"));
        assert_eq!(
            history.messages[0].source_binding_id.as_deref(),
            Some(
                "adapter:8:slack_v2;installation:17:install_host_beta;agent:16:agent:slack-host;project:18:project:slack-host;space:6:T-HOST;conversation:6:D-HOST;topic:0:;"
            )
        );

        runtime.shutdown().await.expect("runtime shuts down");
    }

    #[tokio::test]
    async fn build_slack_host_beta_mounts_exposes_events_and_pairing_redeem_route() {
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

        let mounts = build_slack_host_beta_mounts(&runtime, config()).expect("mounts build");
        let pairing_mount =
            slack_personal_binding_pairing_route_mount(mounts.personal_binding_pairing);

        assert_eq!(mounts.events.descriptors.len(), 1);
        assert!(
            pairing_mount
                .descriptors
                .iter()
                .any(|descriptor| descriptor.route_pattern().as_str()
                    == SLACK_PERSONAL_BINDING_PAIRING_REDEEM_PATH)
        );

        runtime.shutdown().await.expect("runtime shuts down");
    }

    #[tokio::test]
    async fn build_slack_host_beta_mounts_rejects_team_only_selector_for_pairing() {
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
        let team_only_config = SlackHostBetaConfig::new(SlackHostBetaConfigInput {
            tenant_id: TenantId::new(TENANT).expect("tenant"),
            agent_id: AgentId::new(AGENT).expect("agent"),
            project_id: Some(ProjectId::new(PROJECT).expect("project")),
            installation_id: INSTALLATION.to_string(),
            team_id: TEAM.to_string(),
            api_app_id: None,
            slack_user_id: Some(SLACK_USER.to_string()),
            user_id: UserId::new(USER).expect("user"),
            signing_secret: SecretString::from(SECRET),
            bot_token: SecretString::from("xoxb-host-token"),
        })
        .expect("team-only config still parses");

        let error = match build_slack_host_beta_mounts(&runtime, team_only_config) {
            Ok(_) => panic!("pairing requires tenant app selector"),
            Err(error) => error,
        };

        assert!(matches!(
            error,
            SlackHostBetaBuildError::TenantAppSelectorRequired
        ));
        runtime.shutdown().await.expect("runtime shuts down");
    }

    #[test]
    fn slack_host_beta_config_keeps_optional_legacy_slack_actor() {
        let config = config();

        assert_eq!(config.installation_id.as_str(), INSTALLATION);
        let slack_actor = config.slack_actor.as_ref().expect("legacy actor");
        assert_eq!(slack_actor.kind(), SLACK_USER_ACTOR_KIND);
        assert_eq!(slack_actor.id(), SLACK_USER);
        assert_eq!(config.user_id, UserId::new(USER).expect("user id"));
        assert_eq!(config.signing_secret.expose_secret(), SECRET);
        assert_eq!(config.bot_token.expose_secret(), "xoxb-host-token");
    }

    #[test]
    fn slack_egress_scope_uses_configured_tenant_agent_and_project() {
        let config = config();

        let scope = slack_egress_scope(&config);

        assert_eq!(scope.tenant_id, TenantId::new(TENANT).expect("tenant"));
        assert_eq!(scope.user_id, UserId::new(USER).expect("user"));
        assert_eq!(scope.agent_id, Some(AgentId::new(AGENT).expect("agent")));
        assert_eq!(
            scope.project_id,
            Some(ProjectId::new(PROJECT).expect("project"))
        );
    }

    #[tokio::test]
    async fn layered_resolver_preserves_configured_legacy_slack_actor_mapping() {
        let resolver = SlackHostBetaActorUserResolver::new(
            AdapterInstallationId::new(INSTALLATION).expect("installation"),
            Some(
                ExternalActorRef::new(SLACK_USER_ACTOR_KIND, SLACK_USER, None::<String>)
                    .expect("actor"),
            ),
            UserId::new(USER).expect("user"),
            Arc::new(FailingProductActorUserResolver),
            Arc::new(FailingProductActorUserResolver),
        );
        let request = ProductActorUserResolutionRequest::new(
            ProductAdapterId::new(SLACK_V2_ADAPTER_ID).expect("adapter"),
            AdapterInstallationId::new(INSTALLATION).expect("installation"),
            ExternalActorRef::new(SLACK_USER_ACTOR_KIND, SLACK_USER, None::<String>)
                .expect("actor"),
        );

        let resolved = resolver
            .resolve_product_actor_user(request)
            .await
            .expect("resolver succeeds");

        assert_eq!(resolved, Some(UserId::new(USER).expect("user")));
    }

    fn config() -> SlackHostBetaConfig {
        SlackHostBetaConfig::new(SlackHostBetaConfigInput {
            tenant_id: TenantId::new(TENANT).expect("tenant"),
            agent_id: AgentId::new(AGENT).expect("agent"),
            project_id: Some(ProjectId::new(PROJECT).expect("project")),
            installation_id: INSTALLATION.to_string(),
            team_id: TEAM.to_string(),
            api_app_id: Some(API_APP.to_string()),
            slack_user_id: Some(SLACK_USER.to_string()),
            user_id: UserId::new(USER).expect("user"),
            signing_secret: SecretString::from(SECRET),
            bot_token: SecretString::from("xoxb-host-token"),
        })
        .expect("valid config")
    }

    async fn runtime() -> (RebornRuntime, tempfile::TempDir) {
        let root = tempfile::tempdir().expect("tempdir");
        let runtime = build_reborn_runtime(
            RebornRuntimeInput::from_services(
                RebornBuildInput::local_dev(USER, root.path().join("local-dev"))
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
        (runtime, root)
    }

    async fn wait_for_slack_thread_history(
        runtime: &RebornRuntime,
    ) -> ironclaw_threads::ThreadHistory {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        let thread_service = runtime.webui_thread_service();
        let scope = ThreadScope {
            tenant_id: TenantId::new(TENANT).expect("tenant"),
            agent_id: AgentId::new(AGENT).expect("agent"),
            project_id: Some(ProjectId::new(PROJECT).expect("project")),
            owner_user_id: Some(UserId::new(USER).expect("user")),
            mission_id: None,
        };
        loop {
            let threads = thread_service
                .list_threads_for_scope(ListThreadsForScopeRequest {
                    scope: scope.clone(),
                    limit: Some(1),
                    cursor: None,
                })
                .await
                .expect("list Slack-created threads");
            if let Some(thread) = threads.threads.first() {
                return thread_service
                    .list_thread_history(ThreadHistoryRequest {
                        scope,
                        thread_id: thread.thread_id.clone(),
                    })
                    .await
                    .expect("read Slack-created thread history");
            }
            if tokio::time::Instant::now() >= deadline {
                panic!("signed Slack event did not create a thread");
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
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

    fn dm_event_body() -> &'static str {
        r#"{
          "type":"event_callback",
          "team_id":"T-HOST",
          "api_app_id":"A-HOST",
          "event_id":"Ev-host-beta-custom-resolver",
          "event":{
            "type":"message",
            "channel_type":"im",
            "user":"U-HOST",
            "channel":"D-HOST",
            "text":"hello",
            "ts":"1710000000.000001"
          }
        }"#
    }

    async fn wait_for_resolver_calls(
        resolver: &RecordingProductActorUserResolver,
        expected_len: usize,
    ) -> Vec<ProductActorUserResolutionRequest> {
        for _ in 0..40 {
            let calls = resolver.calls();
            if calls.len() >= expected_len {
                return calls;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        resolver.calls()
    }

    #[derive(Debug)]
    struct RecordingProductActorUserResolver {
        user_id: UserId,
        calls: Mutex<Vec<ProductActorUserResolutionRequest>>,
    }

    impl RecordingProductActorUserResolver {
        fn new(user_id: UserId) -> Self {
            Self {
                user_id,
                calls: Mutex::default(),
            }
        }

        fn calls(&self) -> Vec<ProductActorUserResolutionRequest> {
            self.calls
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone()
        }
    }

    #[async_trait::async_trait]
    impl ProductActorUserResolver for RecordingProductActorUserResolver {
        async fn resolve_product_actor_user(
            &self,
            request: ProductActorUserResolutionRequest,
        ) -> Result<Option<UserId>, ProductWorkflowError> {
            self.calls
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(request);
            Ok(Some(self.user_id.clone()))
        }
    }

    #[derive(Debug)]
    struct FailingProductActorUserResolver;

    #[async_trait::async_trait]
    impl ProductActorUserResolver for FailingProductActorUserResolver {
        async fn resolve_product_actor_user(
            &self,
            _request: ProductActorUserResolutionRequest,
        ) -> Result<Option<UserId>, ProductWorkflowError> {
            Err(ProductWorkflowError::BindingResolutionFailed {
                reason: "fallback should not be called".into(),
            })
        }
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

    #[derive(Default)]
    struct RecordingRuntimeHttpEgress {
        requests: std::sync::Mutex<Vec<RuntimeHttpEgressRequest>>,
    }

    #[async_trait]
    impl RuntimeHttpEgress for RecordingRuntimeHttpEgress {
        async fn execute(
            &self,
            request: RuntimeHttpEgressRequest,
        ) -> Result<RuntimeHttpEgressResponse, ironclaw_host_api::RuntimeHttpEgressError> {
            self.requests
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(request);
            Ok(RuntimeHttpEgressResponse {
                status: 200,
                headers: Vec::new(),
                body: br#"{"ok":true}"#.to_vec(),
                saved_body: None,
                request_bytes: 0,
                response_bytes: 0,
                redaction_applied: false,
            })
        }
    }
}
