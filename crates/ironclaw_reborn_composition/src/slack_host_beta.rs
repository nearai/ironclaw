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
use ironclaw_outbound::{FilesystemOutboundStateStore, OutboundStateStore};
use ironclaw_product_adapters::{
    AdapterInstallationId, DeclaredEgressHost, DeclaredEgressTarget, DeliveryStatus,
    EgressCredentialHandle, ExternalActorRef, OutboundDeliverySink, ProductAdapter,
    ProductAdapterId, ProtocolHttpEgress,
};
use ironclaw_product_workflow::{
    DefaultInboundTurnService, DefaultProductWorkflow, ProductActorUserResolutionRequest,
    ProductActorUserResolver, ProductConversationBindingService, ProductInstallationKey,
    ProductInstallationScope, ProductWorkflowError, StaticProductInstallationResolver,
};
use ironclaw_product_workflow_storage::RebornFilesystemIdempotencyLedger;
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
const SLACK_WEBHOOK_WORKFLOW_TIMEOUT: Duration = Duration::from_secs(55);
const SLACK_MAX_IN_FLIGHT_WEBHOOKS: usize = 64;
const SLACK_IDEMPOTENCY_LEDGER_SETTLED_LIMIT: usize = 10_000;
const SLACK_IDEMPOTENCY_LEDGER_PRUNE_INTERVAL: usize = 1_000;

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

pub async fn build_slack_events_route_mount(
    runtime: &RebornRuntime,
    config: SlackHostBetaConfig,
) -> Result<PublicRouteMount, SlackHostBetaBuildError> {
    build_slack_host_beta_mounts(runtime, config)
        .await
        .map(|mounts| mounts.events)
}

pub async fn build_slack_host_beta_mounts(
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
    )
    .await?;

    Ok(SlackHostBetaMounts {
        events,
        personal_binding_pairing: SlackPersonalBindingPairingRouteConfig::new(pairing),
    })
}

pub async fn build_slack_events_route_mount_with_actor_user_resolver(
    runtime: &RebornRuntime,
    config: SlackHostBetaConfig,
    actor_user_resolver: Arc<dyn ProductActorUserResolver>,
) -> Result<PublicRouteMount, SlackHostBetaBuildError> {
    // The resolver controls inbound Slack actor binding. `config.user_id` still
    // scopes the host-mediated Slack bot-token egress for this beta route.
    let local_runtime = runtime
        .services()
        .local_runtime
        .as_ref()
        .ok_or(SlackHostBetaBuildError::DurableHostStateUnavailable)?;
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
            Arc::new(
                RebornFilesystemIdempotencyLedger::new(
                    Arc::clone(&local_runtime.host_state_filesystem),
                    slack_egress_scope(&config),
                )
                .with_settled_entry_limit(
                    NonZeroUsize::new(SLACK_IDEMPOTENCY_LEDGER_SETTLED_LIMIT).ok_or_else(|| {
                        invalid_config("settled_entry_limit", "must be non-zero".to_string())
                    })?,
                )
                .with_settled_prune_interval(
                    NonZeroUsize::new(SLACK_IDEMPOTENCY_LEDGER_PRUNE_INTERVAL).ok_or_else(
                        || invalid_config("settled_prune_interval", "must be non-zero".to_string()),
                    )?,
                ),
            ),
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
    let outbound = Arc::new(FilesystemOutboundStateStore::new(Arc::clone(
        &local_runtime.host_state_filesystem,
    )));
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
        EgressPolicy::new(slack_declared_egress_targets(token_handle)?),
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
) -> Result<Vec<DeclaredEgressTarget>, SlackHostBetaBuildError> {
    let host = DeclaredEgressHost::new(SLACK_API_HOST)
        .map_err(|reason| invalid_config("slack_api_host", reason.to_string()))?;
    Ok(vec![DeclaredEgressTarget::new(host, Some(token_handle))])
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
mod tests;
