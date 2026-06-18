//! Host-beta Slack Events API composition.
//!
//! This module is the single composition point for the native Slack route:
//! the CLI supplies explicit host config, and this module reuses the already
//! assembled Reborn runtime services instead of creating a second agent loop.

use std::collections::HashSet;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;

use ironclaw_conversations::InMemoryConversationServices;
use ironclaw_host_api::{AgentId, ProjectId, ResourceScope, TenantId, UserId};
use ironclaw_outbound::{DeliveredGateRouteStore, OutboundStateStore, TriggeredRunDeliveryStore};
use ironclaw_product_adapters::{
    AdapterInstallationId, DeclaredEgressHost, DeclaredEgressTarget, DeliveryStatus,
    EgressCredentialHandle, ExternalActorRef, OutboundDeliverySink, ProductAdapter,
    ProductAdapterId, ProtocolHttpEgress,
};
use ironclaw_product_workflow::{
    ConversationBindingService, DefaultInboundTurnService, DefaultProductWorkflow,
    ProductActorUserResolutionRequest, ProductActorUserResolver, ProductConversationBindingService,
    ProductConversationRouteKey, ProductConversationSubjectRouteResolver, ProductInstallationKey,
    ProductInstallationScope, ProductWorkflowError, ResolveBindingRequest, ResolvedBinding,
    StaticProductInstallationResolver,
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
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::RebornRuntime;
use crate::outbound_preferences::{
    OutboundDeliveryTargetProvider, OutboundDeliveryTargetRegistrationOutcome,
};
use crate::slack_actor_identity::SlackUserIdentityActorResolver;
use crate::slack_channel_routes::{
    SlackChannelRouteAdminRouteConfig, SlackChannelRouteStore, SlackChannelRouteSubjectResolver,
};
use crate::slack_delivery::{
    SlackFinalReplyDeliveryObserver, SlackFinalReplyDeliveryServices,
    SlackFinalReplyDeliverySettings, TriggeredRunDeliveryDriver,
};
use crate::slack_egress::{SlackProtocolHttpEgress, StaticSlackEgressCredentialProvider};
use crate::slack_host_state::FilesystemSlackHostState;
use crate::slack_outbound_targets::{
    SlackConfiguredChannelRoute, SlackHostBetaOutboundTargetProvider,
    SlackOutboundTargetProviderConfig, SlackPersonalDmTargetProvisioner,
    SlackPersonalDmTargetStore,
};
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
    SlackEventsRouteState, SlackInstallationRecord, SlackInstallationSelector, SlackTeamId,
    StaticSlackInstallationResolver, slack_events_route_mount,
};
use crate::webui_serve::PublicRouteMount;

const SLACK_BOT_TOKEN_HANDLE: &str = "slack_bot_token";
const SLACK_SIGNATURE_HEADER: &str = "X-Slack-Signature";
const SLACK_TIMESTAMP_HEADER: &str = "X-Slack-Request-Timestamp";
const SLACK_WEBHOOK_WORKFLOW_TIMEOUT: Duration = Duration::from_secs(2);
const SLACK_MAX_IN_FLIGHT_WEBHOOKS: usize = 64;
const SLACK_IDEMPOTENCY_LEDGER_SETTLED_LIMIT: usize = 10_000;
const SLACK_IDEMPOTENCY_LEDGER_PRUNE_INTERVAL: usize = 1_000;
const SLACK_OUTBOUND_PROVIDER_KEY_PREFIX: &str = "slack-v2-host-beta";

struct NoopSlackDeliverySink;

#[async_trait::async_trait]
impl OutboundDeliverySink for NoopSlackDeliverySink {
    async fn record(&self, _status: DeliveryStatus) {}
}

/// No-op [`ConversationBindingService`] used by [`build_triggered_run_delivery_hook`].
///
/// The triggered-run delivery path never calls `resolve_binding` or
/// `lookup_binding` — it receives the `TurnScope` directly from the poller.
/// This stub satisfies the type system without introducing an unnecessary
/// installation-level conversation registry.
struct NoopConversationBindingService;

#[async_trait::async_trait]
impl ConversationBindingService for NoopConversationBindingService {
    async fn resolve_binding(
        &self,
        _request: ResolveBindingRequest,
    ) -> Result<ResolvedBinding, ProductWorkflowError> {
        Err(ProductWorkflowError::BindingResolutionFailed {
            reason: "NoopConversationBindingService is not supported in triggered delivery"
                .to_string(),
        })
    }

    async fn lookup_binding(
        &self,
        _request: ResolveBindingRequest,
    ) -> Result<ResolvedBinding, ProductWorkflowError> {
        Err(ProductWorkflowError::BindingResolutionFailed {
            reason: "NoopConversationBindingService is not supported in triggered delivery"
                .to_string(),
        })
    }
}

#[derive(Clone)]
pub struct SlackHostBetaConfig {
    pub tenant_id: TenantId,
    pub agent_id: AgentId,
    pub project_id: Option<ProjectId>,
    pub installation_id: AdapterInstallationId,
    pub team_id: SlackTeamId,
    pub installation_selector: SlackInstallationSelector,
    /// Optional Slack actor retained only for legacy static personal-binding
    /// tests/config. Tenant app host-beta resolution uses durable personal
    /// bindings and does not require a preselected Slack user.
    pub slack_actor: Option<ExternalActorRef>,
    /// Host/runtime user used for Slack host-mediated state, legacy static
    /// Slack actor mapping, and backward-compatible shared-route fallback when
    /// `shared_subject_user_id` is not configured.
    pub user_id: UserId,
    /// Optional user scope that owns Slack shared-channel execution, tools,
    /// skills, and memory in this beta route. Personal DM routes still use the
    /// paired actor as the subject.
    pub shared_subject_user_id: Option<UserId>,
    pub channel_routes: Vec<SlackHostBetaChannelRoute>,
    pub signing_secret: SecretString,
    pub bot_token: SecretString,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlackHostBetaChannelRoute {
    pub channel_id: String,
    pub subject_user_id: UserId,
}

impl SlackHostBetaChannelRoute {
    pub fn new(channel_id: impl Into<String>, subject_user_id: UserId) -> Self {
        Self {
            channel_id: channel_id.into(),
            subject_user_id,
        }
    }
}

pub struct SlackHostBetaConfigInput {
    pub tenant_id: TenantId,
    pub agent_id: AgentId,
    pub project_id: Option<ProjectId>,
    pub installation_id: String,
    pub team_id: SlackTeamId,
    pub api_app_id: Option<String>,
    pub slack_user_id: Option<String>,
    pub user_id: UserId,
    pub shared_subject_user_id: Option<UserId>,
    pub channel_routes: Vec<SlackHostBetaChannelRoute>,
    pub signing_secret: SecretString,
    pub bot_token: SecretString,
}

impl SlackHostBetaConfig {
    pub fn new(input: SlackHostBetaConfigInput) -> Result<Self, SlackHostBetaBuildError> {
        let installation_id = AdapterInstallationId::new(input.installation_id)
            .map_err(|reason| invalid_config("installation_id", reason.to_string()))?;
        let team_id = input.team_id;
        let installation_selector = match input.api_app_id {
            Some(api_app_id) => {
                SlackInstallationSelector::app_team(api_app_id, team_id.as_str().to_string())
            }
            None => SlackInstallationSelector::team(team_id.as_str().to_string()),
        };
        let mut seen_channel_ids = HashSet::new();
        for route in &input.channel_routes {
            if !seen_channel_ids.insert(route.channel_id.as_str()) {
                return Err(invalid_config(
                    "channel_routes",
                    format!("duplicate channel_id '{}'", route.channel_id),
                ));
            }
            slack_channel_route_key(&team_id, route)?;
        }
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
            team_id,
            installation_selector,
            slack_actor,
            user_id: input.user_id,
            shared_subject_user_id: input.shared_subject_user_id,
            channel_routes: input.channel_routes,
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
            .field("team_id", &self.team_id)
            .field("installation_selector", &self.installation_selector)
            .field("slack_actor", &self.slack_actor)
            .field("user_id", &self.user_id)
            .field("shared_subject_user_id", &self.shared_subject_user_id)
            .field("channel_routes", &self.channel_routes)
            .field("signing_secret", &"[REDACTED]")
            .field("bot_token", &"[REDACTED]")
            .finish()
    }
}

fn slack_outbound_delivery_target_provider_key(config: &SlackHostBetaConfig) -> String {
    let mut hasher = Sha256::new();
    hash_slack_mount_field(&mut hasher, config.tenant_id.as_str());
    hash_slack_mount_field(&mut hasher, config.agent_id.as_str());
    hash_slack_mount_field(
        &mut hasher,
        config.project_id.as_ref().map_or("", ProjectId::as_str),
    );
    hash_slack_mount_field(&mut hasher, config.installation_id.as_str());
    hash_slack_mount_field(&mut hasher, config.team_id.as_str());
    hash_slack_installation_selector(&mut hasher, &config.installation_selector);
    hash_slack_mount_field(
        &mut hasher,
        config.slack_actor.as_ref().map_or("", ExternalActorRef::id),
    );
    hash_slack_mount_field(&mut hasher, config.user_id.as_str());
    hash_slack_mount_field(
        &mut hasher,
        config
            .shared_subject_user_id
            .as_ref()
            .map_or("", UserId::as_str),
    );
    for route in &config.channel_routes {
        hash_slack_mount_field(&mut hasher, &route.channel_id);
        hash_slack_mount_field(&mut hasher, route.subject_user_id.as_str());
    }
    hash_slack_mount_field(&mut hasher, config.signing_secret.expose_secret());
    hash_slack_mount_field(&mut hasher, config.bot_token.expose_secret());

    let digest = hasher.finalize();
    let mut suffix = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(&mut suffix, "{byte:02x}");
    }
    format!("{SLACK_OUTBOUND_PROVIDER_KEY_PREFIX}:{suffix}")
}

fn hash_slack_installation_selector(hasher: &mut Sha256, selector: &SlackInstallationSelector) {
    match selector {
        SlackInstallationSelector::Team { team_id } => {
            hash_slack_mount_field(hasher, "team");
            hash_slack_mount_field(hasher, team_id.as_str());
        }
        SlackInstallationSelector::AppTeam {
            api_app_id,
            team_id,
        } => {
            hash_slack_mount_field(hasher, "app_team");
            hash_slack_mount_field(hasher, api_app_id.as_str());
            hash_slack_mount_field(hasher, team_id.as_str());
        }
        SlackInstallationSelector::EnterpriseTeam {
            enterprise_id,
            team_id,
        } => {
            hash_slack_mount_field(hasher, "enterprise_team");
            hash_slack_mount_field(hasher, enterprise_id.as_str());
            hash_slack_mount_field(hasher, team_id.as_str());
        }
        SlackInstallationSelector::InstallUser {
            team_id,
            install_user_id,
        } => {
            hash_slack_mount_field(hasher, "install_user");
            hash_slack_mount_field(hasher, team_id.as_str());
            hash_slack_mount_field(hasher, install_user_id.as_str());
        }
        SlackInstallationSelector::EnterpriseInstallUser {
            enterprise_id,
            team_id,
            install_user_id,
        } => {
            hash_slack_mount_field(hasher, "enterprise_install_user");
            hash_slack_mount_field(hasher, enterprise_id.as_str());
            hash_slack_mount_field(hasher, team_id.as_str());
            hash_slack_mount_field(hasher, install_user_id.as_str());
        }
        SlackInstallationSelector::AppInstallUser {
            api_app_id,
            team_id,
            install_user_id,
        } => {
            hash_slack_mount_field(hasher, "app_install_user");
            hash_slack_mount_field(hasher, api_app_id.as_str());
            hash_slack_mount_field(hasher, team_id.as_str());
            hash_slack_mount_field(hasher, install_user_id.as_str());
        }
    }
}

fn hash_slack_mount_field(hasher: &mut Sha256, value: &str) {
    hasher.update(value.len().to_le_bytes());
    hasher.update(value.as_bytes());
}

#[derive(Debug, Error)]
pub enum SlackHostBetaBuildError {
    #[error("Slack host-beta requires local runtime HTTP egress")]
    RuntimeHttpEgressUnavailable,
    #[error("Slack host-beta requires durable host state")]
    DurableHostStateUnavailable,
    #[error("Slack host-beta outbound delivery target registration failed: {reason}")]
    OutboundDeliveryTargetRegistration { reason: String },
    #[error(
        "Slack host-beta personal binding requires [slack].api_app_id for tenant app-scoped pairing"
    )]
    TenantAppSelectorRequired,
    #[error("invalid Slack host-beta config field {field}: {reason}")]
    InvalidConfig { field: &'static str, reason: String },
}

#[non_exhaustive]
pub struct SlackHostBetaMounts {
    pub events: PublicRouteMount,
    pub personal_binding_pairing: SlackPersonalBindingPairingRouteConfig,
    pub channel_routes: SlackChannelRouteAdminRouteConfig,
}

pub fn build_slack_events_route_mount(
    runtime: &RebornRuntime,
    config: SlackHostBetaConfig,
) -> Result<PublicRouteMount, SlackHostBetaBuildError> {
    build_slack_host_beta_mounts(runtime, config).map(|mounts| mounts.events)
}

/// Build a [`TriggeredRunDeliveryDriver`] that delivers triggered-run results
/// to the creator's personal Slack DM.
///
/// Returns the concrete `Arc<TriggeredRunDeliveryDriver>` so tests can assert
/// store-pointer identity through this production entry point (via
/// [`TriggeredRunDeliveryDriver::communication_preferences_for_test`] and
/// `Arc::ptr_eq`).  Call sites that wire the hook into the runtime coerce the
/// concrete Arc to `Arc<dyn PostSubmitDeliveryHook>` implicitly when passing
/// it to [`RebornRuntime::set_trigger_post_submit_hook`].
///
/// Preferences and outbound state come from the composition-owned store (the
/// same instance the WebUI delivery-defaults facade writes through), so a
/// preference set via the WebUI is visible to Slack delivery.
/// See docs/plans/2026-05-29-trigger-loop-delivery-resolution-implementation.md.
pub fn build_triggered_run_delivery_hook(
    runtime: &RebornRuntime,
    config: &SlackHostBetaConfig,
    delivery_store: Arc<dyn TriggeredRunDeliveryStore>,
) -> Result<Arc<TriggeredRunDeliveryDriver>, SlackHostBetaBuildError> {
    let local_runtime = runtime
        .services()
        .local_runtime
        .as_ref()
        .ok_or(SlackHostBetaBuildError::DurableHostStateUnavailable)?;
    let token_handle = slack_bot_token_handle()?;
    let adapter_id = ProductAdapterId::new(SLACK_V2_ADAPTER_ID)
        .map_err(|reason| invalid_config("adapter_id", reason.to_string()))?;
    let adapter: Arc<dyn ProductAdapter> = Arc::new(SlackV2Adapter::new(SlackV2AdapterConfig {
        adapter_id,
        installation_id: config.installation_id.clone(),
        egress_credential_handle: token_handle.clone(),
        auth_requirement: slack_request_signature_auth_requirement(),
    }));
    let egress = slack_protocol_egress(runtime, config, token_handle)?;
    let outbound_store: Arc<dyn OutboundStateStore> = Arc::clone(&local_runtime.outbound_state);
    let route_store: Arc<dyn DeliveredGateRouteStore> =
        Arc::clone(&local_runtime.delivered_gate_routes);
    let preferences: Arc<dyn ironclaw_outbound::CommunicationPreferenceRepository> =
        Arc::clone(&local_runtime.outbound_preferences);
    let delivery_sink: Arc<dyn OutboundDeliverySink> = Arc::new(NoopSlackDeliverySink);
    let binding_service: Arc<dyn ConversationBindingService> =
        Arc::new(NoopConversationBindingService);
    let services = SlackFinalReplyDeliveryServices {
        binding_service,
        thread_service: runtime.webui_thread_service(),
        turn_coordinator: runtime.webui_turn_coordinator(),
        outbound_store,
        route_store: Arc::clone(&route_store),
        communication_preferences: preferences,
        adapter,
        egress,
        delivery_sink,
        auth_challenges: runtime.auth_challenge_provider(),
        auth_flow_canceller: runtime.blocked_auth_flow_canceller(),
        approval_requests: Some(Arc::clone(&local_runtime.approval_requests)
            as Arc<dyn ironclaw_run_state::ApprovalRequestStore>),
    };
    // Pass config.agent_id as the fallback so the ThreadScope key matches the
    // value ConversationContentRefMaterializer uses (same runtime default_agent_id).
    let driver = TriggeredRunDeliveryDriver::new(
        services,
        delivery_store,
        route_store,
        config.agent_id.clone(),
    );
    Ok(Arc::new(driver))
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
            token_handle.clone(),
        ));
    let challenge_store: Arc<dyn SlackPersonalBindingPairingChallengeStore> = state.clone();
    let dm_provisioner = Arc::new(SlackPersonalDmTargetProvisioner::new(
        config.tenant_id.clone(),
        config.installation_id.clone(),
        config.team_id.clone(),
        slack_protocol_egress(runtime, &config, token_handle.clone())?,
        token_handle,
        state.clone(),
    ));
    let pairing =
        SlackPersonalBindingPairingService::new(binding_service, challenge_store, notifier)
            .with_dm_provisioner(dm_provisioner);
    let actor_user_resolver = Arc::new(SlackHostBetaActorUserResolver::new(
        config.installation_id.clone(),
        config.slack_actor.clone(),
        config.user_id.clone(),
        Arc::new(SlackUserIdentityActorResolver::new(state.clone())),
        Arc::new(SlackPairingActorResolver::new(
            state.clone(),
            pairing.clone(),
        )),
    ));
    let channel_route_store: Arc<dyn SlackChannelRouteStore> = state.clone();
    let personal_dm_target_store: Arc<dyn SlackPersonalDmTargetStore> = state.clone();
    let subject_route_resolver: Arc<dyn ProductConversationSubjectRouteResolver> =
        Arc::new(SlackChannelRouteSubjectResolver::new(
            config.tenant_id.clone(),
            config.installation_id.clone(),
            Arc::clone(&channel_route_store),
        ));
    let events = build_slack_events_route_mount_with_resolvers(
        runtime,
        config.clone(),
        actor_user_resolver,
        Some(subject_route_resolver),
    )?;
    let allowed_route_subjects = std::iter::once(config.user_id.clone())
        .chain(config.shared_subject_user_id.clone())
        .chain(
            config
                .channel_routes
                .iter()
                .map(|route| route.subject_user_id.clone()),
        );
    let channel_routes = SlackChannelRouteAdminRouteConfig::new(
        config.tenant_id.clone(),
        config.installation_id.clone(),
        config.team_id.as_str().to_string(),
        config.user_id.clone(),
        Arc::clone(&channel_route_store),
    )
    .with_allowed_subject_user_ids(allowed_route_subjects);

    let outbound_delivery_provider_key = slack_outbound_delivery_target_provider_key(&config);
    let outbound_delivery_provider_already_registered = runtime
        .outbound_delivery_target_provider_key_registered(&outbound_delivery_provider_key)
        .map_err(
            |error| SlackHostBetaBuildError::OutboundDeliveryTargetRegistration {
                reason: error.to_string(),
            },
        )?;

    // Wire the triggered-run delivery hook. The delivery store comes from the
    // composition-owned outbound store, shared with preferences so the same
    // backing tree is used for all outbound roles. `set_trigger_post_submit_hook`
    // is idempotent: a second call (if this function is called more than once)
    // is silently ignored.
    {
        let delivery_store: Arc<dyn TriggeredRunDeliveryStore> =
            Arc::clone(&local_runtime.triggered_run_delivery);
        match build_triggered_run_delivery_hook(runtime, &config, delivery_store) {
            Ok(hook) => {
                let hook_set = runtime.set_trigger_post_submit_hook(hook);
                if !hook_set
                    && runtime.trigger_post_submit_hook_is_set()
                    && !outbound_delivery_provider_already_registered
                {
                    return Err(SlackHostBetaBuildError::OutboundDeliveryTargetRegistration {
                        reason: "Slack triggered delivery hook is already wired for a different Slack host config".to_string(),
                    });
                }
            }
            Err(err) => {
                tracing::warn!(
                    target = "ironclaw::reborn::slack_host_beta",
                    error = %err,
                    "triggered-run delivery hook construction failed; trigger delivery will be disabled"
                );
            }
        }
    }

    let outbound_delivery_target_provider: Arc<dyn OutboundDeliveryTargetProvider> =
        Arc::new(SlackHostBetaOutboundTargetProvider::new(
            SlackOutboundTargetProviderConfig {
                tenant_id: config.tenant_id.clone(),
                agent_id: config.agent_id.clone(),
                project_id: config.project_id.clone(),
                installation_id: config.installation_id.clone(),
                team_id: config.team_id.clone(),
                configured_channel_routes: config
                    .channel_routes
                    .iter()
                    .map(|route| {
                        SlackConfiguredChannelRoute::new(
                            route.channel_id.clone(),
                            route.subject_user_id.clone(),
                        )
                    })
                    .collect(),
            },
            channel_route_store,
            Arc::clone(&personal_dm_target_store),
        ));
    if outbound_delivery_provider_already_registered {
        return Ok(SlackHostBetaMounts {
            events,
            personal_binding_pairing: SlackPersonalBindingPairingRouteConfig::new(pairing),
            channel_routes,
        });
    }
    match runtime
        .register_outbound_delivery_target_provider(
            outbound_delivery_provider_key,
            Arc::clone(&outbound_delivery_target_provider),
        )
        .map_err(
            |error| SlackHostBetaBuildError::OutboundDeliveryTargetRegistration {
                reason: error.to_string(),
            },
        )? {
        OutboundDeliveryTargetRegistrationOutcome::Registered => {}
        OutboundDeliveryTargetRegistrationOutcome::Replaced => {
            return Err(SlackHostBetaBuildError::OutboundDeliveryTargetRegistration {
                reason: "Slack outbound delivery target provider was concurrently registered; replacement would diverge from the first-writer trigger delivery hook".to_string(),
            });
        }
    }
    Ok(SlackHostBetaMounts {
        events,
        personal_binding_pairing: SlackPersonalBindingPairingRouteConfig::new(pairing),
        channel_routes,
    })
}

pub fn build_slack_events_route_mount_with_actor_user_resolver(
    runtime: &RebornRuntime,
    config: SlackHostBetaConfig,
    actor_user_resolver: Arc<dyn ProductActorUserResolver>,
) -> Result<PublicRouteMount, SlackHostBetaBuildError> {
    build_slack_events_route_mount_with_resolvers(runtime, config, actor_user_resolver, None)
}

fn build_slack_events_route_mount_with_resolvers(
    runtime: &RebornRuntime,
    config: SlackHostBetaConfig,
    actor_user_resolver: Arc<dyn ProductActorUserResolver>,
    subject_route_resolver: Option<Arc<dyn ProductConversationSubjectRouteResolver>>,
) -> Result<PublicRouteMount, SlackHostBetaBuildError> {
    // The resolver controls inbound Slack actor binding. `config.user_id`
    // scopes host-mediated Slack bot-token egress and legacy static actor
    // mapping. Shared Slack channel execution is configured separately.
    let local_runtime = runtime
        .services()
        .local_runtime
        .as_ref()
        .ok_or(SlackHostBetaBuildError::DurableHostStateUnavailable)?;
    tracing::warn!(
        "Slack host-beta uses in-memory conversation bindings; Slack conversation binding continuity is lost on process restart"
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
    let mut scope = ProductInstallationScope::with_default_scope(
        config.tenant_id.clone(),
        config.agent_id.clone(),
        config.project_id.clone(),
    );
    scope = scope.with_default_subject_user_id(
        config
            .shared_subject_user_id
            .clone()
            .unwrap_or_else(|| config.user_id.clone()),
    );
    if let Some(subject_route_resolver) = subject_route_resolver {
        scope = scope
            .with_conversation_subject_route_resolver(subject_route_resolver)
            .without_default_subject_for_unrouted_shared_conversations();
    }
    for route in &config.channel_routes {
        let route_key = slack_channel_route_key(&config.team_id, route)?;
        scope = scope.with_conversation_subject_route(route_key, route.subject_user_id.clone());
    }
    let scope = scope.with_actor_user_resolver(actor_user_resolver, actor_pairings);
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
    let route_store: Arc<dyn ironclaw_outbound::DeliveredGateRouteStore> =
        Arc::clone(&local_runtime.delivered_gate_routes);
    let workflow = Arc::new(
        DefaultProductWorkflow::new(
            inbound,
            Arc::new(
                RebornFilesystemIdempotencyLedger::new(
                    Arc::clone(&local_runtime.host_state_filesystem),
                    slack_egress_scope_template(&config),
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
        .with_approval_interaction_service(Arc::new(
            crate::delivered_gate_routing::DeliveredGateRoutingApprovalService::new(
                runtime.webui_approval_interaction_service(),
                route_store.clone(),
            ),
        ))
        .with_auth_interaction_service(runtime.webui_auth_interaction_service())
        .with_delivered_gate_routes(route_store.clone()),
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
    let outbound_store: Arc<dyn OutboundStateStore> = Arc::clone(&local_runtime.outbound_state);
    let preferences: Arc<dyn ironclaw_outbound::CommunicationPreferenceRepository> =
        Arc::clone(&local_runtime.outbound_preferences);
    let delivery_sink: Arc<dyn OutboundDeliverySink> = Arc::new(NoopSlackDeliverySink);
    let observer = Arc::new(SlackFinalReplyDeliveryObserver::with_settings(
        SlackFinalReplyDeliveryServices {
            binding_service: Arc::new(binding),
            thread_service: runtime.webui_thread_service(),
            turn_coordinator: runtime.webui_turn_coordinator(),
            outbound_store,
            route_store,
            communication_preferences: preferences,
            adapter,
            egress,
            delivery_sink,
            auth_challenges: runtime.auth_challenge_provider(),
            auth_flow_canceller: runtime.blocked_auth_flow_canceller(),
            approval_requests: Some(Arc::clone(&local_runtime.approval_requests)
                as Arc<dyn ironclaw_run_state::ApprovalRequestStore>),
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

fn slack_channel_route_key(
    team_id: &SlackTeamId,
    route: &SlackHostBetaChannelRoute,
) -> Result<ProductConversationRouteKey, SlackHostBetaBuildError> {
    ProductConversationRouteKey::new(Some(team_id.as_str().to_string()), route.channel_id.clone())
        .map_err(|reason| invalid_config("channel_routes", reason.to_string()))
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
    let host_egress = local_runtime
        .host_runtime_http_egress
        .clone()
        .ok_or(SlackHostBetaBuildError::RuntimeHttpEgressUnavailable)?;
    Ok(Arc::new(SlackProtocolHttpEgress::new(
        host_egress,
        Arc::new(StaticSlackEgressCredentialProvider::new(
            token_handle.clone(),
            config.bot_token.expose_secret().to_string(),
        )),
        EgressPolicy::new(slack_declared_egress_targets(token_handle)?),
        slack_egress_scope_template(config),
    )))
}

fn slack_egress_scope_template(config: &SlackHostBetaConfig) -> ResourceScope {
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
mod tests {
    use std::sync::{Arc, Mutex};
    use std::time::{SystemTime, UNIX_EPOCH};

    use async_trait::async_trait;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use hmac::{Hmac, Mac};
    use http_body_util::BodyExt;
    use ironclaw_authorization::GrantAuthorizer;
    use ironclaw_extensions::ExtensionRegistry;
    use ironclaw_filesystem::LocalFilesystem;
    use ironclaw_host_runtime::{
        CapabilitySurfaceVersion, HostRuntimeHttpEgressPort, HostRuntimeServices,
    };
    use ironclaw_loop_support::{
        HostManagedModelError, HostManagedModelGateway, HostManagedModelRequest,
        HostManagedModelResponse,
    };
    use ironclaw_network::{
        NetworkHttpEgress, NetworkHttpError, NetworkHttpRequest, NetworkHttpResponse, NetworkUsage,
    };
    use ironclaw_processes::{InMemoryProcessResultStore, InMemoryProcessStore, ProcessServices};
    use ironclaw_product_workflow::{
        ProductActorUserResolutionRequest, ProductWorkflowError, RebornChannelConnectStrategy,
        RebornOutboundDeliveryTargetId, RebornOutboundDeliveryTargetStatus,
        RebornServicesErrorCode, RebornServicesErrorKind, RebornSetOutboundPreferencesRequest,
        WebUiAuthenticatedCaller,
    };
    use ironclaw_resources::InMemoryResourceGovernor;
    use ironclaw_secrets::InMemorySecretStore;
    use ironclaw_threads::{ListThreadsForScopeRequest, ThreadHistoryRequest, ThreadScope};
    use ironclaw_turns::{
        GetRunStateRequest, ReplyTargetBindingRef, TurnCoordinator, TurnRunId, TurnScope,
        TurnStatus, run_profile::LoopCapabilityPort,
    };
    use secrecy::ExposeSecret;
    use tower::ServiceExt;

    use super::*;
    use crate::slack_channel_routes::{
        InMemorySlackChannelRouteStore, SlackChannelRoute, SlackChannelRouteAdminRouteMount,
        SlackChannelRouteError, SlackChannelRouteKey, SlackChannelRouteListPage,
        WEBUI_V2_CHANNELS_SLACK_ALLOWED_PATH, WEBUI_V2_CHANNELS_SLACK_ROUTES_PATH,
        slack_channel_route_admin_route_mount,
    };
    use crate::slack_connectable_channel::{
        SlackOperatorRouteVisibility, build_webui_services_with_slack_host_beta_mounts,
    };
    use crate::slack_outbound_targets::{
        InMemorySlackPersonalDmTargetStore, SLACK_OUTBOUND_TARGET_LIST_PAGE_SIZE,
        SlackPersonalDmTarget, SlackPersonalDmTargetError, SlackPersonalDmTargetKey,
        SlackPersonalDmTargetProvisioner, SlackPersonalDmTargetStore,
        slack_reply_target_binding_ref_from_raw, slack_shared_channel_reply_target_binding_ref,
    };
    use crate::slack_personal_binding_pairing_serve::{
        WEBUI_V2_EXTENSION_PAIRING_REDEEM_PATH, slack_personal_binding_pairing_route_mount,
    };
    use crate::slack_serve::SlackUserId;
    use crate::{
        RebornBuildError, RebornBuildInput, RebornRuntimeIdentity, RebornRuntimeInput,
        SLACK_EVENTS_PATH, WebuiAuthentication, WebuiAuthenticator, WebuiServeConfig,
        build_reborn_runtime, local_dev_runtime_policy, webui_v2_app,
    };

    const TENANT: &str = "tenant:slack-host";
    const AGENT: &str = "agent:slack-host";
    const PROJECT: &str = "project:slack-host";
    const USER: &str = "user:slack-host";
    const SHARED_SUBJECT: &str = "user:slack-shared-subject";
    const INSTALLATION: &str = "install_host_beta";
    const TEAM: &str = "T0HOST";
    const API_APP: &str = "A0HOST";
    const SLACK_USER: &str = "U0HOST";
    const SECRET: &str = "host-signing-secret";

    type HmacSha256 = Hmac<sha2::Sha256>;

    struct OperatorTokenAuthenticator;

    #[async_trait]
    impl WebuiAuthenticator for OperatorTokenAuthenticator {
        async fn authenticate(&self, token: &str) -> Option<WebuiAuthentication> {
            if token == "operator-token" {
                Some(WebuiAuthentication::operator(
                    UserId::new(USER).expect("user"),
                ))
            } else {
                None
            }
        }

        fn mounts_operator_webui_config_routes(&self) -> bool {
            true
        }
    }

    struct MultiUserTokenAuthenticator;

    #[async_trait]
    impl WebuiAuthenticator for MultiUserTokenAuthenticator {
        async fn authenticate(&self, token: &str) -> Option<WebuiAuthentication> {
            if token == "operator-token" {
                Some(WebuiAuthentication::user(UserId::new(USER).expect("user")))
            } else {
                None
            }
        }
    }

    #[derive(Debug)]
    struct NonAdvancingCursorRouteStore;

    #[async_trait]
    impl SlackChannelRouteStore for NonAdvancingCursorRouteStore {
        async fn list_routes(
            &self,
            _tenant_id: &TenantId,
            _installation_id: &AdapterInstallationId,
            _team_id: &str,
            cursor: usize,
            _limit: usize,
        ) -> Result<SlackChannelRouteListPage, SlackChannelRouteError> {
            Ok(SlackChannelRouteListPage {
                routes: Vec::new(),
                next_cursor: Some(cursor),
            })
        }

        async fn upsert_route(
            &self,
            _key: SlackChannelRouteKey,
            _subject_user_id: UserId,
        ) -> Result<SlackChannelRoute, SlackChannelRouteError> {
            Err(SlackChannelRouteError::StoreUnavailable)
        }

        async fn delete_route(
            &self,
            _key: &SlackChannelRouteKey,
        ) -> Result<bool, SlackChannelRouteError> {
            Err(SlackChannelRouteError::StoreUnavailable)
        }

        async fn replace_managed_routes(
            &self,
            _tenant_id: &TenantId,
            _installation_id: &AdapterInstallationId,
            _team_id: &str,
            _assignments: Vec<crate::slack_channel_routes::SlackChannelRouteAssignment>,
        ) -> Result<Vec<SlackChannelRoute>, SlackChannelRouteError> {
            Err(SlackChannelRouteError::StoreUnavailable)
        }

        async fn resolve_subject_user_id(
            &self,
            _key: &SlackChannelRouteKey,
        ) -> Result<Option<UserId>, SlackChannelRouteError> {
            Err(SlackChannelRouteError::StoreUnavailable)
        }
    }

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
        let (runtime, _root) = runtime_with_host_egress_override(Some(None)).await;

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
    async fn build_slack_events_route_mount_fails_when_durable_host_state_unavailable() {
        let (mut runtime, _root) = runtime().await;
        runtime.clear_local_runtime_for_test();

        let error = match build_slack_events_route_mount(&runtime, config()) {
            Ok(_) => panic!("Slack route requires durable host state"),
            Err(error) => error,
        };

        assert!(matches!(
            error,
            SlackHostBetaBuildError::DurableHostStateUnavailable
        ));
        runtime.shutdown().await.expect("runtime shuts down");
    }

    #[tokio::test]
    async fn slack_outbound_targets_fail_build_when_local_runtime_missing() {
        let (mut runtime, _root) = runtime().await;
        let mounts = build_slack_host_beta_mounts(&runtime, config()).expect("mounts");
        runtime.clear_local_runtime_for_test();

        let error = build_webui_services_with_slack_host_beta_mounts(
            &runtime,
            None,
            Some(&mounts),
            SlackOperatorRouteVisibility::Hidden,
        )
        .expect_err("outbound target providers require local runtime wiring");

        assert!(matches!(
            error,
            RebornBuildError::InvalidConfig { reason }
                if reason.contains("outbound delivery target providers require local runtime")
        ));
        runtime.shutdown().await.expect("runtime shuts down");
    }

    #[tokio::test]
    async fn build_slack_events_route_mount_dispatches_signed_event_callback() {
        let egress = Arc::new(RecordingRuntimeHttpEgress::default());
        let (runtime, _root) = runtime_with_host_egress_override(Some(Some(
            host_egress_port_for_test(Arc::clone(&egress)),
        )))
        .await;
        let mount = build_slack_events_route_mount(&runtime, config()).expect("route builds");
        let body = r#"{
            "type":"event_callback",
            "team_id":"T0HOST",
            "api_app_id":"A0HOST",
            "event_id":"Ev-host-beta-dispatch",
            "event":{"type":"message","channel_type":"im","user":"U0HOST","channel":"D0HOST","text":"hello","ts":"1710000000.000010"}
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
        let inbound_message = history
            .messages
            .iter()
            .find(|message| message.content.as_deref() == Some("hello"))
            .expect("inbound Slack message should be recorded");
        assert_eq!(
            inbound_message.source_binding_id.as_deref(),
            Some(
                "adapter:8:slack_v2;installation:17:install_host_beta;agent:16:agent:slack-host;project:18:project:slack-host;space:6:T0HOST;conversation:6:D0HOST;topic:0:;"
            )
        );

        runtime.shutdown().await.expect("runtime shuts down");
    }

    #[tokio::test]
    async fn build_slack_events_route_mount_deduplicates_event_after_route_rebuild() {
        let egress = Arc::new(RecordingRuntimeHttpEgress::default());
        let (runtime, _root) = runtime_with_host_egress_override(Some(Some(
            host_egress_port_for_test(Arc::clone(&egress)),
        )))
        .await;
        let body = dm_event_body_with(
            "Ev-host-beta-durable-idempotency",
            "dedupe me",
            "1710000000.000011",
        );

        let first_mount =
            build_slack_events_route_mount(&runtime, config()).expect("first route builds");
        post_signed_slack_event(&first_mount, &body).await;
        if let Some(drain) = first_mount.drain.as_ref() {
            drain.drain().await;
        }
        wait_for_slack_message_count_with_text(
            &runtime,
            Some(UserId::new(USER).expect("user")),
            "dedupe me",
            1,
        )
        .await;

        let rebuilt_mount =
            build_slack_events_route_mount(&runtime, config()).expect("rebuilt route builds");
        post_signed_slack_event(&rebuilt_mount, &body).await;
        if let Some(drain) = rebuilt_mount.drain.as_ref() {
            drain.drain().await;
        }

        assert_eq!(
            slack_message_count_with_text(
                &runtime,
                Some(UserId::new(USER).expect("user")),
                "dedupe me"
            )
            .await,
            1,
            "duplicate Slack event should replay from the durable idempotency ledger"
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
            .with_default_project_id(ProjectId::new(PROJECT).expect("project"))
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
                    == WEBUI_V2_EXTENSION_PAIRING_REDEEM_PATH)
        );

        runtime.shutdown().await.expect("runtime shuts down");
    }

    #[tokio::test]
    async fn build_slack_host_beta_mounts_pairs_unknown_slack_actor_then_routes_bound_event() {
        let egress = Arc::new(RecordingRuntimeHttpEgress::default());
        let (runtime, _root) = runtime_with_host_egress_override(Some(Some(
            host_egress_port_for_test(Arc::clone(&egress)),
        )))
        .await;
        let mounts =
            build_slack_host_beta_mounts(&runtime, config_without_legacy_actor()).expect("mounts");

        let first_body =
            dm_event_body_with("Ev-host-beta-pairing-first", "pair me", "1710000000.000020");
        post_signed_slack_event(&mounts.events, &first_body).await;
        if let Some(drain) = mounts.events.drain.as_ref() {
            drain.drain().await;
        }
        let pairing_code = wait_for_pairing_code(&egress).await;

        let pairing_mount =
            slack_personal_binding_pairing_route_mount(mounts.personal_binding_pairing);
        let redeem_body = format!(r#"{{"channel":"slack","code":"{pairing_code}"}}"#);
        let redeem_response = pairing_mount
            .protected
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(WEBUI_V2_EXTENSION_PAIRING_REDEEM_PATH)
                    .header("content-type", "application/json")
                    .extension(WebUiAuthenticatedCaller {
                        tenant_id: TenantId::new(TENANT).expect("tenant"),
                        user_id: UserId::new(USER).expect("user"),
                        agent_id: Some(AgentId::new(AGENT).expect("agent")),
                        project_id: Some(ProjectId::new(PROJECT).expect("project")),
                    })
                    .body(Body::from(redeem_body))
                    .expect("redeem request builds"),
            )
            .await
            .expect("redeem route responds");

        assert_eq!(redeem_response.status(), StatusCode::OK);

        let second_body = dm_event_body_with(
            "Ev-host-beta-pairing-second",
            "after pairing",
            "1710000000.000030",
        );
        post_signed_slack_event(&mounts.events, &second_body).await;
        if let Some(drain) = mounts.events.drain.as_ref() {
            drain.drain().await;
        }

        let history = wait_for_slack_thread_history(&runtime).await;
        let accepted_message = history
            .messages
            .iter()
            .find(|message| message.content.as_deref() == Some("after pairing"))
            .expect("accepted Slack message is present");
        let run_id = TurnRunId::parse(
            accepted_message
                .turn_run_id
                .as_deref()
                .expect("accepted Slack message should carry submitted run id"),
        )
        .expect("valid submitted run id");
        let run_state = runtime
            .webui_turn_coordinator()
            .get_run_state(GetRunStateRequest {
                scope: TurnScope::new_with_owner(
                    TenantId::new(TENANT).expect("tenant"),
                    Some(AgentId::new(AGENT).expect("agent")),
                    Some(ProjectId::new(PROJECT).expect("project")),
                    accepted_message.thread_id.clone(),
                    Some(UserId::new(USER).expect("user")),
                ),
                run_id,
            })
            .await
            .expect("read DM run state");
        assert_eq!(
            run_state.status,
            TurnStatus::Completed,
            "DM run failed: {:?}",
            run_state.failure
        );
        let final_reply = wait_for_slack_post_message(&egress, "ok").await;
        assert_eq!(final_reply["channel"], "D0HOST");
        assert_eq!(final_reply["text"], "ok");
        assert_eq!(final_reply["mrkdwn"], true);

        runtime.shutdown().await.expect("runtime shuts down");
    }

    #[tokio::test]
    async fn build_slack_host_beta_mounts_replies_to_channel_app_mention_thread() {
        let egress = Arc::new(RecordingRuntimeHttpEgress::default());
        let (runtime, _root) = runtime_with_host_egress_override(Some(Some(
            host_egress_port_for_test(Arc::clone(&egress)),
        )))
        .await;
        let mounts = build_slack_host_beta_mounts(&runtime, config()).expect("mounts");

        let body = app_mention_event_body_with(
            "Ev-host-beta-channel-mention",
            "<@U-BOT> help in channel",
            "1710000000.000040",
        );
        post_signed_slack_event(&mounts.events, &body).await;
        if let Some(drain) = mounts.events.drain.as_ref() {
            drain.drain().await;
        }

        let history = wait_for_slack_thread_history_with_owner(
            &runtime,
            Some(UserId::new(SHARED_SUBJECT).expect("shared subject")),
        )
        .await;
        let accepted_message = history
            .messages
            .iter()
            .find(|message| message.content.as_deref() == Some("help in channel"))
            .expect("accepted Slack app mention message is present");
        let run_id = TurnRunId::parse(
            accepted_message
                .turn_run_id
                .as_deref()
                .expect("accepted Slack message should carry submitted run id"),
        )
        .expect("valid submitted run id");
        let run_state = runtime
            .webui_turn_coordinator()
            .get_run_state(GetRunStateRequest {
                scope: TurnScope::new_with_owner(
                    TenantId::new(TENANT).expect("tenant"),
                    Some(AgentId::new(AGENT).expect("agent")),
                    Some(ProjectId::new(PROJECT).expect("project")),
                    accepted_message.thread_id.clone(),
                    Some(UserId::new(SHARED_SUBJECT).expect("shared subject")),
                ),
                run_id,
            })
            .await
            .expect("read channel mention run state");
        assert_eq!(
            run_state.status,
            TurnStatus::Completed,
            "channel mention run failed: {:?}",
            run_state.failure
        );
        let final_reply = wait_for_slack_post_message(&egress, "ok").await;
        assert_eq!(final_reply["channel"], "C0HOST");
        assert_eq!(final_reply["text"], "ok");
        assert_eq!(final_reply["thread_ts"], "1710000000.000040");

        let thread_reply_body = thread_message_event_body_with(
            "Ev-host-beta-channel-thread-reply",
            "follow up without mention",
            "1710000000.000041",
            "1710000000.000040",
        );
        post_signed_slack_event(&mounts.events, &thread_reply_body).await;
        if let Some(drain) = mounts.events.drain.as_ref() {
            drain.drain().await;
        }

        let final_replies = wait_for_slack_post_messages(&egress, "ok", 2).await;
        let threaded_reply = final_replies
            .iter()
            .find(|body| body["thread_ts"] == "1710000000.000040" && body["channel"] == "C0HOST")
            .expect("thread follow-up reply should post back to original Slack thread");
        assert_eq!(threaded_reply["text"], "ok");

        runtime.shutdown().await.expect("runtime shuts down");
    }

    #[tokio::test]
    async fn slack_channel_route_admin_assignment_routes_channel_mention_to_subject() {
        let egress = Arc::new(RecordingRuntimeHttpEgress::default());
        let (runtime, _root) = runtime_with_host_egress_override(Some(Some(
            host_egress_port_for_test(Arc::clone(&egress)),
        )))
        .await;
        let mounts = build_slack_host_beta_mounts(&runtime, config_without_channel_routes())
            .expect("mounts");
        let route_mount = slack_channel_route_admin_route_mount(mounts.channel_routes);
        let assign_response = route_mount
            .protected
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(WEBUI_V2_CHANNELS_SLACK_ROUTES_PATH)
                    .header("content-type", "application/json")
                    .extension(WebUiAuthenticatedCaller {
                        tenant_id: TenantId::new(TENANT).expect("tenant"),
                        user_id: UserId::new(USER).expect("user"),
                        agent_id: Some(AgentId::new(AGENT).expect("agent")),
                        project_id: Some(ProjectId::new(PROJECT).expect("project")),
                    })
                    .body(Body::from(format!(
                        r#"{{"channel_id":"C0HOST","subject_user_id":"{SHARED_SUBJECT}"}}"#
                    )))
                    .expect("assign request builds"),
            )
            .await
            .expect("assign route responds");
        assert_eq!(assign_response.status(), StatusCode::OK);

        let body = app_mention_event_body_with(
            "Ev-host-beta-admin-routed-channel-mention",
            "<@U-BOT> help in channel",
            "1710000000.000050",
        );
        post_signed_slack_event(&mounts.events, &body).await;
        if let Some(drain) = mounts.events.drain.as_ref() {
            drain.drain().await;
        }

        let history = wait_for_slack_thread_history_with_owner(
            &runtime,
            Some(UserId::new(SHARED_SUBJECT).expect("shared subject")),
        )
        .await;
        let accepted_message = history
            .messages
            .iter()
            .find(|message| message.content.as_deref() == Some("help in channel"))
            .expect("accepted Slack app mention message is present under assigned subject");
        assert_eq!(
            accepted_message.source_binding_id.as_deref(),
            Some(
                "adapter:8:slack_v2;installation:17:install_host_beta;agent:16:agent:slack-host;project:18:project:slack-host;space:6:T0HOST;conversation:6:C0HOST;topic:17:1710000000.000050;"
            )
        );

        runtime.shutdown().await.expect("runtime shuts down");
    }

    #[tokio::test]
    async fn slack_channel_route_admin_rejects_unassigned_channel_mention() {
        let egress = Arc::new(RecordingRuntimeHttpEgress::default());
        let (runtime, _root) = runtime_with_host_egress_override(Some(Some(
            host_egress_port_for_test(Arc::clone(&egress)),
        )))
        .await;
        let mounts = build_slack_host_beta_mounts(&runtime, config_without_channel_routes())
            .expect("mounts");

        let body = app_mention_event_body_with(
            "Ev-host-beta-unassigned-channel-mention",
            "<@U-BOT> help in unassigned channel",
            "1710000000.000060",
        );
        post_signed_slack_event(&mounts.events, &body).await;
        if let Some(drain) = mounts.events.drain.as_ref() {
            drain.drain().await;
        }
        assert_no_slack_threads_for_owner(
            &runtime,
            Some(UserId::new(SHARED_SUBJECT).expect("shared subject")),
        )
        .await;
        assert!(egress.post_message_bodies_with_text("ok").is_empty());

        runtime.shutdown().await.expect("runtime shuts down");
    }

    #[tokio::test]
    async fn slack_allowed_channels_are_reachable_through_webui_v2_app() {
        let (runtime, _root) = runtime().await;
        let mounts = build_slack_host_beta_mounts(&runtime, config_without_channel_routes())
            .expect("mounts");
        let bundle = build_webui_services_with_slack_host_beta_mounts(
            &runtime,
            None,
            Some(&mounts),
            SlackOperatorRouteVisibility::Visible,
        )
        .expect("webui bundle");
        let app = webui_v2_app(
            bundle,
            WebuiServeConfig::new(
                TenantId::new(TENANT).expect("tenant"),
                Arc::new(OperatorTokenAuthenticator),
                Vec::new(),
            )
            .with_default_agent_id(AgentId::new(AGENT).expect("agent"))
            .with_default_project_id(ProjectId::new(PROJECT).expect("project"))
            .with_slack_channel_routes(mounts.channel_routes),
        )
        .expect("webui app");

        let save = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(WEBUI_V2_CHANNELS_SLACK_ALLOWED_PATH)
                    .header("authorization", "Bearer operator-token")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"channel_ids":["C0HOST","C0OPS"]}"#))
                    .expect("save request builds"),
            )
            .await
            .expect("save route responds");
        assert_eq!(save.status(), StatusCode::OK);
        let save_body = axum::body::to_bytes(save.into_body(), 64 * 1024)
            .await
            .expect("save body");
        let save_body: serde_json::Value = serde_json::from_slice(&save_body).expect("save json");
        assert_eq!(save_body["channels"].as_array().expect("channels").len(), 2);
        assert_ne!(
            save_body["channels"][0]["subject_user_id"],
            save_body["channels"][1]["subject_user_id"],
            "allowed API should assign one tenant-scoped subject per channel"
        );

        let list = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(WEBUI_V2_CHANNELS_SLACK_ALLOWED_PATH)
                    .header("authorization", "Bearer operator-token")
                    .body(Body::empty())
                    .expect("list request builds"),
            )
            .await
            .expect("list route responds");
        assert_eq!(list.status(), StatusCode::OK);
        let body = axum::body::to_bytes(list.into_body(), 64 * 1024)
            .await
            .expect("list body");
        let body: serde_json::Value = serde_json::from_slice(&body).expect("list json");
        assert_eq!(
            body["channels"],
            serde_json::json!([
                {
                    "channel_id":"C0HOST",
                    "subject_user_id": save_body["channels"][0]["subject_user_id"].clone(),
                    "subject_display_name": save_body["channels"][0]["subject_display_name"].clone()
                },
                {
                    "channel_id":"C0OPS",
                    "subject_user_id": save_body["channels"][1]["subject_user_id"].clone(),
                    "subject_display_name": save_body["channels"][1]["subject_display_name"].clone()
                }
            ])
        );

        runtime.shutdown().await.expect("runtime shuts down");
    }

    #[tokio::test]
    async fn slack_channel_route_admin_is_reachable_through_webui_v2_app() {
        let (runtime, _root) = runtime().await;
        let mounts = build_slack_host_beta_mounts(&runtime, config_without_channel_routes())
            .expect("mounts");
        let bundle = build_webui_services_with_slack_host_beta_mounts(
            &runtime,
            None,
            Some(&mounts),
            SlackOperatorRouteVisibility::Visible,
        )
        .expect("webui bundle");
        let app = webui_v2_app(
            bundle,
            WebuiServeConfig::new(
                TenantId::new(TENANT).expect("tenant"),
                Arc::new(OperatorTokenAuthenticator),
                Vec::new(),
            )
            .with_default_agent_id(AgentId::new(AGENT).expect("agent"))
            .with_default_project_id(ProjectId::new(PROJECT).expect("project"))
            .with_slack_channel_routes(mounts.channel_routes),
        )
        .expect("webui app");

        let unauthenticated = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(WEBUI_V2_CHANNELS_SLACK_ROUTES_PATH)
                    .header("content-type", "application/json")
                    .body(Body::from(format!(
                        r#"{{"channel_id":"C0HOST","subject_user_id":"{SHARED_SUBJECT}"}}"#
                    )))
                    .expect("unauthenticated request builds"),
            )
            .await
            .expect("unauthenticated route responds");
        assert_eq!(unauthenticated.status(), StatusCode::UNAUTHORIZED);

        let empty_list = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(WEBUI_V2_CHANNELS_SLACK_ROUTES_PATH)
                    .header("authorization", "Bearer operator-token")
                    .body(Body::empty())
                    .expect("empty list request builds"),
            )
            .await
            .expect("empty list route responds");
        assert_eq!(empty_list.status(), StatusCode::OK);
        let body = axum::body::to_bytes(empty_list.into_body(), 64 * 1024)
            .await
            .expect("empty list body");
        let body: serde_json::Value = serde_json::from_slice(&body).expect("empty list json");
        assert_eq!(body["routes"], serde_json::json!([]));
        assert_eq!(body["next_cursor"], serde_json::Value::Null);

        let upsert = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(WEBUI_V2_CHANNELS_SLACK_ROUTES_PATH)
                    .header("authorization", "Bearer operator-token")
                    .header("content-type", "application/json")
                    .body(Body::from(format!(
                        r#"{{"channel_id":"C0HOST","subject_user_id":"{SHARED_SUBJECT}"}}"#
                    )))
                    .expect("upsert request builds"),
            )
            .await
            .expect("upsert route responds");
        assert_eq!(upsert.status(), StatusCode::OK);

        let list = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(WEBUI_V2_CHANNELS_SLACK_ROUTES_PATH)
                    .header("authorization", "Bearer operator-token")
                    .body(Body::empty())
                    .expect("list request builds"),
            )
            .await
            .expect("list route responds");
        assert_eq!(list.status(), StatusCode::OK);
        let body = axum::body::to_bytes(list.into_body(), 64 * 1024)
            .await
            .expect("list body");
        let body: serde_json::Value = serde_json::from_slice(&body).expect("list json");
        assert_eq!(body["routes"][0]["channel_id"], "C0HOST");
        assert_eq!(body["routes"][0]["subject_user_id"], SHARED_SUBJECT);

        let delete = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(WEBUI_V2_CHANNELS_SLACK_ROUTES_PATH)
                    .header("authorization", "Bearer operator-token")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"channel_id":"C0HOST"}"#))
                    .expect("delete request builds"),
            )
            .await
            .expect("delete route responds");
        assert_eq!(delete.status(), StatusCode::OK);
        let body = axum::body::to_bytes(delete.into_body(), 64 * 1024)
            .await
            .expect("delete body");
        let body: serde_json::Value = serde_json::from_slice(&body).expect("delete json");
        assert_eq!(body["deleted"], true);

        let list_after_delete = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(WEBUI_V2_CHANNELS_SLACK_ROUTES_PATH)
                    .header("authorization", "Bearer operator-token")
                    .body(Body::empty())
                    .expect("list request builds"),
            )
            .await
            .expect("list route responds");
        assert_eq!(list_after_delete.status(), StatusCode::OK);
        let body = axum::body::to_bytes(list_after_delete.into_body(), 64 * 1024)
            .await
            .expect("list body");
        let body: serde_json::Value = serde_json::from_slice(&body).expect("list json");
        assert_eq!(body["routes"], serde_json::json!([]));

        runtime.shutdown().await.expect("runtime shuts down");
    }

    #[tokio::test]
    async fn slack_channel_routes_are_not_mounted_for_non_operator_authenticator() {
        let (runtime, _root) = runtime().await;
        let mounts = build_slack_host_beta_mounts(&runtime, config_without_channel_routes())
            .expect("mounts");
        let bundle = build_webui_services_with_slack_host_beta_mounts(
            &runtime,
            None,
            Some(&mounts),
            SlackOperatorRouteVisibility::Hidden,
        )
        .expect("webui bundle");
        let caller = WebUiAuthenticatedCaller::new(
            TenantId::new(TENANT).expect("tenant"),
            UserId::new(USER).expect("user"),
            Some(AgentId::new(AGENT).expect("agent")),
            Some(ProjectId::new(PROJECT).expect("project")),
        );
        let connectable = bundle
            .api
            .list_connectable_channels(caller)
            .await
            .expect("connectable channels");
        assert!(
            connectable
                .channels
                .iter()
                .any(|channel| channel.strategy == RebornChannelConnectStrategy::InboundProofCode),
            "non-operator WebUI should still advertise personal Slack pairing"
        );
        assert!(
            connectable
                .channels
                .iter()
                .all(|channel| channel.strategy
                    != RebornChannelConnectStrategy::AdminManagedChannels),
            "non-operator WebUI must not advertise Slack admin channel management"
        );
        let app = webui_v2_app(
            bundle,
            WebuiServeConfig::new(
                TenantId::new(TENANT).expect("tenant"),
                Arc::new(MultiUserTokenAuthenticator),
                Vec::new(),
            )
            .with_default_agent_id(AgentId::new(AGENT).expect("agent"))
            .with_default_project_id(ProjectId::new(PROJECT).expect("project"))
            .with_slack_channel_routes(mounts.channel_routes),
        )
        .expect("webui app");

        for (method, uri, body) in [
            ("GET", WEBUI_V2_CHANNELS_SLACK_ROUTES_PATH, ""),
            (
                "PUT",
                WEBUI_V2_CHANNELS_SLACK_ROUTES_PATH,
                r#"{"channel_id":"C0HOST","subject_user_id":"user:slack-shared-subject"}"#,
            ),
            (
                "DELETE",
                WEBUI_V2_CHANNELS_SLACK_ROUTES_PATH,
                r#"{"channel_id":"C0HOST"}"#,
            ),
            ("GET", WEBUI_V2_CHANNELS_SLACK_ALLOWED_PATH, ""),
            (
                "PUT",
                WEBUI_V2_CHANNELS_SLACK_ALLOWED_PATH,
                r#"{"channel_ids":["C0HOST"]}"#,
            ),
        ] {
            let mut builder = Request::builder()
                .method(method)
                .uri(uri)
                .header("authorization", "Bearer operator-token");
            if method != "GET" {
                builder = builder.header("content-type", "application/json");
            }
            let response = app
                .clone()
                .oneshot(
                    builder
                        .body(Body::from(body.to_string()))
                        .expect("request builds"),
                )
                .await
                .expect("route responds");
            assert_eq!(
                response.status(),
                StatusCode::NOT_FOUND,
                "{method} route must not be mounted for non-operator auth"
            );
        }

        runtime.shutdown().await.expect("runtime shuts down");
    }

    #[tokio::test]
    async fn slack_host_beta_targets_wire_through_outbound_preferences_facade() {
        let (runtime, _root) = runtime().await;
        let mounts = build_slack_host_beta_mounts(&runtime, config()).expect("mounts");
        let bundle = build_webui_services_with_slack_host_beta_mounts(
            &runtime,
            None,
            Some(&mounts),
            SlackOperatorRouteVisibility::Hidden,
        )
        .expect("webui bundle");
        let shared_subject = WebUiAuthenticatedCaller::new(
            TenantId::new(TENANT).expect("tenant"),
            UserId::new(SHARED_SUBJECT).expect("shared subject"),
            Some(AgentId::new(AGENT).expect("agent")),
            Some(ProjectId::new(PROJECT).expect("project")),
        );
        let operator = WebUiAuthenticatedCaller::new(
            TenantId::new(TENANT).expect("tenant"),
            UserId::new(USER).expect("user"),
            Some(AgentId::new(AGENT).expect("agent")),
            Some(ProjectId::new(PROJECT).expect("project")),
        );

        let operator_targets = bundle
            .api
            .list_outbound_delivery_targets(operator)
            .await
            .expect("operator target list");
        assert!(
            operator_targets.targets.is_empty(),
            "Slack shared-channel target list must be scoped to the route subject"
        );

        let targets = bundle
            .api
            .list_outbound_delivery_targets(shared_subject.clone())
            .await
            .expect("shared subject target list");
        assert_eq!(targets.targets.len(), 1);
        let target = &targets.targets[0];
        assert_eq!(target.target.channel.as_str(), "slack");
        assert_eq!(target.target.display_name.as_str(), "Slack channel C0HOST");
        assert!(target.capabilities.final_replies);
        let runtime_targets = runtime
            .outbound_delivery_target_provider()
            .expect("Slack mounts should register runtime outbound target provider")
            .list_outbound_delivery_targets(&shared_subject)
            .await
            .expect("runtime target list");
        assert_eq!(runtime_targets.len(), 1);
        assert_eq!(
            runtime_targets[0].summary.target_id.as_str(),
            target.target.target_id.as_str()
        );

        let selected = bundle
            .api
            .set_outbound_preferences(
                shared_subject.clone(),
                RebornSetOutboundPreferencesRequest {
                    final_reply_target_id: Some(target.target.target_id.clone()),
                },
            )
            .await
            .expect("set Slack target");
        assert_eq!(
            selected.final_reply_target_status,
            RebornOutboundDeliveryTargetStatus::Available
        );
        assert_eq!(
            selected
                .final_reply_target
                .as_ref()
                .map(|target| target.target_id.as_str()),
            Some(target.target.target_id.as_str())
        );

        let preference = bundle
            .api
            .get_outbound_preferences(shared_subject)
            .await
            .expect("get Slack target preference");
        assert_eq!(
            preference.final_reply_target_status,
            RebornOutboundDeliveryTargetStatus::Available
        );
        assert_eq!(
            preference
                .final_reply_target
                .as_ref()
                .map(|target| target.target_id.as_str()),
            Some(target.target.target_id.as_str())
        );

        runtime.shutdown().await.expect("runtime shuts down");
    }

    #[tokio::test]
    async fn build_slack_host_beta_mounts_allows_same_config_rebuild_without_replacement() {
        let (runtime, _root) = runtime().await;
        let _mounts = build_slack_host_beta_mounts(&runtime, config()).expect("first mount builds");

        build_slack_host_beta_mounts(&runtime, config()).expect("same-config rebuild builds");

        runtime.shutdown().await.expect("runtime shuts down");
    }

    #[tokio::test]
    async fn build_slack_host_beta_mounts_rejects_different_config_after_trigger_hook_wired() {
        let (runtime, _root) = runtime_with_trigger_poller().await;
        let _mounts = build_slack_host_beta_mounts(&runtime, config()).expect("first mount builds");
        let mut different_config = config();
        different_config.channel_routes = vec![SlackHostBetaChannelRoute::new(
            "C1HOST",
            UserId::new(SHARED_SUBJECT).expect("shared subject"),
        )];

        let error = match build_slack_host_beta_mounts(&runtime, different_config) {
            Ok(_) => panic!("different Slack mount must not replace outbound provider"),
            Err(error) => error,
        };

        assert!(
            matches!(
                error,
                SlackHostBetaBuildError::OutboundDeliveryTargetRegistration { ref reason }
                    if reason.contains("different Slack host config")
            ),
            "unexpected replacement error: {error:?}"
        );

        runtime.shutdown().await.expect("runtime shuts down");
    }

    #[tokio::test]
    async fn slack_host_beta_stored_and_static_routes_appear_without_duplicates() {
        let (runtime, _root) = runtime().await;
        let mounts = build_slack_host_beta_mounts(&runtime, config()).expect("mounts");
        let route_mount = slack_channel_route_admin_route_mount(mounts.channel_routes.clone());
        let bundle = build_webui_services_with_slack_host_beta_mounts(
            &runtime,
            None,
            Some(&mounts),
            SlackOperatorRouteVisibility::Hidden,
        )
        .expect("webui bundle");
        upsert_slack_channel_route(&route_mount, "C0DYNAMIC", SHARED_SUBJECT).await;

        let targets = bundle
            .api
            .list_outbound_delivery_targets(shared_subject_caller())
            .await
            .expect("combined route target list");
        let target_ids = targets
            .targets
            .iter()
            .map(|target| target.target.target_id.as_str())
            .collect::<Vec<_>>();

        assert_eq!(
            target_ids,
            vec![
                "slack:shared-channel:T0HOST:C0DYNAMIC",
                "slack:shared-channel:T0HOST:C0HOST",
            ]
        );
        let unique_target_ids = target_ids.iter().copied().collect::<HashSet<_>>();
        assert_eq!(
            unique_target_ids.len(),
            target_ids.len(),
            "stored and static route merge must not duplicate targets"
        );

        runtime.shutdown().await.expect("runtime shuts down");
    }

    #[tokio::test]
    async fn slack_host_beta_targets_page_multiple_route_store_pages() {
        let store = Arc::new(InMemorySlackChannelRouteStore::new());
        let tenant_id = TenantId::new(TENANT).expect("tenant");
        let installation_id = AdapterInstallationId::new(INSTALLATION).expect("installation");
        let subject_user_id = UserId::new(SHARED_SUBJECT).expect("shared subject");
        for index in 0..=SLACK_OUTBOUND_TARGET_LIST_PAGE_SIZE {
            let channel_id = format!("C{index:04}");
            let key = SlackChannelRouteKey::new(
                tenant_id.clone(),
                installation_id.clone(),
                TEAM.to_string(),
                channel_id,
            )
            .expect("route key");
            store
                .upsert_route(key, subject_user_id.clone())
                .await
                .expect("route upserts");
        }
        let provider = outbound_target_provider(config_without_channel_routes(), store);

        let targets = provider
            .list_outbound_delivery_targets(&shared_subject_caller())
            .await
            .expect("paged target list");

        assert_eq!(
            targets.len(),
            SLACK_OUTBOUND_TARGET_LIST_PAGE_SIZE + 1,
            "provider should walk beyond the first route-store page"
        );
        assert_eq!(
            targets
                .last()
                .map(|target| target.summary.target_id.as_str()),
            Some("slack:shared-channel:T0HOST:C0500")
        );
    }

    #[tokio::test]
    async fn slack_shared_channel_targets_survive_personal_dm_store_failure() {
        let provider = SlackHostBetaOutboundTargetProvider::new(
            outbound_target_provider_config(config()),
            Arc::new(InMemorySlackChannelRouteStore::new()),
            Arc::new(FailingSlackPersonalDmTargetStore),
        );

        let targets = provider
            .list_outbound_delivery_targets(&shared_subject_caller())
            .await
            .expect("target list falls back to shared targets");

        assert_eq!(targets.len(), 1);
        assert_eq!(
            targets[0].summary.target_id.as_str(),
            "slack:shared-channel:T0HOST:C0HOST"
        );
    }

    #[tokio::test]
    async fn slack_personal_dm_target_is_not_listed_without_provisioned_authority() {
        let (runtime, _root) = runtime().await;
        let mounts = build_slack_host_beta_mounts(&runtime, config_without_channel_routes())
            .expect("mounts");
        let bundle = build_webui_services_with_slack_host_beta_mounts(
            &runtime,
            None,
            Some(&mounts),
            SlackOperatorRouteVisibility::Hidden,
        )
        .expect("webui bundle");

        let targets = bundle
            .api
            .list_outbound_delivery_targets(operator_caller())
            .await
            .expect("target list");

        assert!(
            targets.targets.is_empty(),
            "identity-only Slack state must not synthesize a personal DM target"
        );
        runtime.shutdown().await.expect("runtime shuts down");
    }

    #[tokio::test]
    async fn slack_personal_dm_target_lists_after_explicit_provisioning() {
        let egress = Arc::new(RecordingRuntimeHttpEgress::default());
        let (runtime, _root) = runtime_with_host_egress_override(Some(Some(
            host_egress_port_for_test(Arc::clone(&egress)),
        )))
        .await;
        let config = config_without_channel_routes();
        personal_dm_target_provisioner_for_test(&runtime, &config)
            .provision_for_user(
                UserId::new(USER).expect("user"),
                SlackUserId::new(SLACK_USER),
            )
            .await
            .expect("DM target provisions");
        let mounts = build_slack_host_beta_mounts(&runtime, config).expect("mounts");
        let bundle = build_webui_services_with_slack_host_beta_mounts(
            &runtime,
            None,
            Some(&mounts),
            SlackOperatorRouteVisibility::Hidden,
        )
        .expect("webui bundle");

        let targets = bundle
            .api
            .list_outbound_delivery_targets(operator_caller())
            .await
            .expect("target list");

        assert_eq!(targets.targets.len(), 1);
        assert_eq!(
            targets.targets[0].target.target_id.as_str(),
            "slack:personal-dm:T0HOST:user:slack-host"
        );
        assert!(targets.targets[0].capabilities.final_replies);
        assert_eq!(
            egress
                .requests()
                .iter()
                .filter(|request| request.url.contains("/api/conversations.open"))
                .count(),
            1
        );
        runtime.shutdown().await.expect("runtime shuts down");
    }

    #[tokio::test]
    async fn slack_personal_dm_target_round_trips_through_outbound_preferences() {
        let egress = Arc::new(RecordingRuntimeHttpEgress::default());
        let (runtime, _root) = runtime_with_host_egress_override(Some(Some(
            host_egress_port_for_test(Arc::clone(&egress)),
        )))
        .await;
        let config = config_without_channel_routes();
        personal_dm_target_provisioner_for_test(&runtime, &config)
            .provision_for_user(
                UserId::new(USER).expect("user"),
                SlackUserId::new(SLACK_USER),
            )
            .await
            .expect("DM target provisions");
        let mounts = build_slack_host_beta_mounts(&runtime, config).expect("mounts");
        let bundle = build_webui_services_with_slack_host_beta_mounts(
            &runtime,
            None,
            Some(&mounts),
            SlackOperatorRouteVisibility::Hidden,
        )
        .expect("webui bundle");
        let caller = operator_caller();
        let targets = bundle
            .api
            .list_outbound_delivery_targets(caller.clone())
            .await
            .expect("target list");
        let target = targets.targets.first().expect("personal DM target");

        let selected = bundle
            .api
            .set_outbound_preferences(
                caller.clone(),
                RebornSetOutboundPreferencesRequest {
                    final_reply_target_id: Some(target.target.target_id.clone()),
                },
            )
            .await
            .expect("set personal DM target");
        assert_eq!(
            selected.final_reply_target_status,
            RebornOutboundDeliveryTargetStatus::Available
        );

        let preference = bundle
            .api
            .get_outbound_preferences(caller)
            .await
            .expect("get personal DM target preference");
        assert_eq!(
            preference.final_reply_target_status,
            RebornOutboundDeliveryTargetStatus::Available
        );
        assert_eq!(
            preference
                .final_reply_target
                .as_ref()
                .map(|target| target.target_id.as_str()),
            Some("slack:personal-dm:T0HOST:user:slack-host")
        );
        runtime.shutdown().await.expect("runtime shuts down");
    }

    #[tokio::test]
    async fn slack_personal_dm_reply_target_binding_ref_round_trips_authorized_dm() {
        let store = Arc::new(InMemorySlackPersonalDmTargetStore::new());
        let key = SlackPersonalDmTargetKey::new(
            TenantId::new(TENANT).expect("tenant"),
            AdapterInstallationId::new(INSTALLATION).expect("installation"),
            TEAM.to_string(),
            UserId::new(USER).expect("user"),
        )
        .expect("personal target key");
        let target =
            SlackPersonalDmTarget::new(key, SlackUserId::new(SLACK_USER), "D0HOST".to_string())
                .expect("personal DM target");
        store
            .upsert_personal_dm_target(target)
            .await
            .expect("personal DM target stores");
        let provider = SlackHostBetaOutboundTargetProvider::new(
            outbound_target_provider_config(config_without_channel_routes()),
            Arc::new(InMemorySlackChannelRouteStore::new()),
            store,
        );
        let listed = provider
            .list_outbound_delivery_targets(&operator_caller())
            .await
            .expect("target list");
        let binding_ref = listed[0].reply_target_binding_ref.clone();

        let resolved = provider
            .resolve_reply_target_binding(&operator_caller(), &binding_ref)
            .await
            .expect("binding resolves")
            .expect("personal DM binding is authorized");

        assert_eq!(
            resolved.summary.target_id.as_str(),
            "slack:personal-dm:T0HOST:user:slack-host"
        );
        assert_eq!(resolved.reply_target_binding_ref, binding_ref);
    }

    #[tokio::test]
    async fn slack_personal_dm_resolve_binding_rejects_mismatched_dm_channel_id() {
        let store = Arc::new(InMemorySlackPersonalDmTargetStore::new());
        let key = SlackPersonalDmTargetKey::new(
            TenantId::new(TENANT).expect("tenant"),
            AdapterInstallationId::new(INSTALLATION).expect("installation"),
            TEAM.to_string(),
            UserId::new(USER).expect("user"),
        )
        .expect("personal target key");
        let target =
            SlackPersonalDmTarget::new(key, SlackUserId::new(SLACK_USER), "D0HOST".to_string())
                .expect("personal DM target");
        store
            .upsert_personal_dm_target(target)
            .await
            .expect("personal DM target stores");
        let provider = SlackHostBetaOutboundTargetProvider::new(
            outbound_target_provider_config(config_without_channel_routes()),
            Arc::new(InMemorySlackChannelRouteStore::new()),
            store,
        );
        let listed = provider
            .list_outbound_delivery_targets(&operator_caller())
            .await
            .expect("target list");
        let mismatched_binding_ref = ReplyTargetBindingRef::new(
            listed[0]
                .reply_target_binding_ref
                .as_str()
                .replace("D0HOST", "D1HOST"),
        )
        .expect("mismatched binding ref still validates");

        assert!(
            provider
                .resolve_reply_target_binding(&operator_caller(), &mismatched_binding_ref)
                .await
                .expect("binding lookup succeeds")
                .is_none()
        );
    }

    #[tokio::test]
    async fn slack_personal_dm_target_provisioning_fails_closed_on_slack_api_error() {
        let egress = Arc::new(RecordingRuntimeHttpEgress::conversations_open_response(
            200,
            br#"{"ok":false,"error":"not_allowed"}"#,
        ));
        let (runtime, _root) = runtime_with_host_egress_override(Some(Some(
            host_egress_port_for_test(Arc::clone(&egress)),
        )))
        .await;
        let config = config_without_channel_routes();
        let error = personal_dm_target_provisioner_for_test(&runtime, &config)
            .provision_for_user(
                UserId::new(USER).expect("user"),
                SlackUserId::new(SLACK_USER),
            )
            .await
            .expect_err("Slack rejection must fail provisioning");
        assert!(matches!(
            error,
            SlackPersonalDmTargetError::ProvisioningFailed(_)
        ));
        let mounts = build_slack_host_beta_mounts(&runtime, config).expect("mounts");
        let bundle = build_webui_services_with_slack_host_beta_mounts(
            &runtime,
            None,
            Some(&mounts),
            SlackOperatorRouteVisibility::Hidden,
        )
        .expect("webui bundle");

        let targets = bundle
            .api
            .list_outbound_delivery_targets(operator_caller())
            .await
            .expect("target list");

        assert!(
            targets.targets.is_empty(),
            "failed Slack DM provisioning must not persist a target authority"
        );
        runtime.shutdown().await.expect("runtime shuts down");
    }

    // ── provisioning-after-pairing: the production wiring ────────────────────

    #[tokio::test]
    async fn pairing_redeem_provisions_personal_dm_target_via_real_call_path() {
        // After pairing-code redemption the provisioner must open the DM and
        // register the personal DM target so it appears in the delivery-target
        // list.  This is the caller-level test for the production seam:
        // pairing-route → SlackPersonalBindingPairingService::redeem_challenge
        // → background provisioner → SlackPersonalDmTargetStore.
        let egress = Arc::new(RecordingRuntimeHttpEgress::default());
        let (runtime, _root) = runtime_with_host_egress_override(Some(Some(
            host_egress_port_for_test(Arc::clone(&egress)),
        )))
        .await;
        let mounts =
            build_slack_host_beta_mounts(&runtime, config_without_legacy_actor()).expect("mounts");

        // Step 1: unknown Slack actor sends a DM → pairing challenge issued.
        let first_body =
            dm_event_body_with("Ev-dm-provision-first", "pair me", "1710000001.000001");
        post_signed_slack_event(&mounts.events, &first_body).await;
        if let Some(drain) = mounts.events.drain.as_ref() {
            drain.drain().await;
        }
        let pairing_code = wait_for_pairing_code(&egress).await;

        // Step 2: authenticated WebUI user redeems the pairing code.
        let pairing_mount =
            slack_personal_binding_pairing_route_mount(mounts.personal_binding_pairing);
        let redeem_body = format!(r#"{{"channel":"slack","code":"{pairing_code}"}}"#);
        let redeem_response = pairing_mount
            .protected
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(WEBUI_V2_EXTENSION_PAIRING_REDEEM_PATH)
                    .header("content-type", "application/json")
                    .extension(WebUiAuthenticatedCaller {
                        tenant_id: TenantId::new(TENANT).expect("tenant"),
                        user_id: UserId::new(USER).expect("user"),
                        agent_id: Some(AgentId::new(AGENT).expect("agent")),
                        project_id: Some(ProjectId::new(PROJECT).expect("project")),
                    })
                    .body(Body::from(redeem_body))
                    .expect("redeem request builds"),
            )
            .await
            .expect("redeem route responds");
        assert_eq!(redeem_response.status(), StatusCode::OK);

        // Step 3: wait for the personal DM target to appear (the provisioner
        // runs in a background task; we poll until it lands in the store).
        let target_listed = {
            let config = config_without_legacy_actor();
            let mut listed = Vec::new();
            for _ in 0..40 {
                let mounts2 =
                    build_slack_host_beta_mounts(&runtime, config.clone()).expect("rebuilt mounts");
                let bundle = build_webui_services_with_slack_host_beta_mounts(
                    &runtime,
                    None,
                    Some(&mounts2),
                    SlackOperatorRouteVisibility::Hidden,
                )
                .expect("webui bundle");
                listed = bundle
                    .api
                    .list_outbound_delivery_targets(operator_caller())
                    .await
                    .expect("target list")
                    .targets;
                if !listed.is_empty() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            listed
        };
        assert_eq!(
            target_listed.len(),
            1,
            "personal DM target must appear after pairing-code redemption"
        );
        assert!(
            target_listed[0]
                .target
                .target_id
                .as_str()
                .contains("personal-dm"),
            "listed target must be a personal DM target"
        );

        runtime.shutdown().await.expect("runtime shuts down");
    }

    #[tokio::test]
    async fn pairing_redeem_is_idempotent_and_does_not_duplicate_dm_target() {
        // Re-provisioning (pairing re-fires) must not create duplicate targets.
        let egress = Arc::new(RecordingRuntimeHttpEgress::default());
        let (runtime, _root) = runtime_with_host_egress_override(Some(Some(
            host_egress_port_for_test(Arc::clone(&egress)),
        )))
        .await;

        // First provisioning.
        let config = config_without_legacy_actor();
        personal_dm_target_provisioner_for_test(&runtime, &config)
            .provision_for_user(
                UserId::new(USER).expect("user"),
                SlackUserId::new(SLACK_USER),
            )
            .await
            .expect("first provisioning succeeds");

        // Second provisioning of the same user — idempotent upsert.
        personal_dm_target_provisioner_for_test(&runtime, &config)
            .provision_for_user(
                UserId::new(USER).expect("user"),
                SlackUserId::new(SLACK_USER),
            )
            .await
            .expect("second provisioning succeeds");

        let mounts = build_slack_host_beta_mounts(&runtime, config).expect("mounts");
        let bundle = build_webui_services_with_slack_host_beta_mounts(
            &runtime,
            None,
            Some(&mounts),
            SlackOperatorRouteVisibility::Hidden,
        )
        .expect("webui bundle");
        let targets = bundle
            .api
            .list_outbound_delivery_targets(operator_caller())
            .await
            .expect("target list");
        assert_eq!(
            targets.targets.len(),
            1,
            "idempotent re-provisioning must not duplicate the DM target"
        );

        runtime.shutdown().await.expect("runtime shuts down");
    }

    #[tokio::test]
    async fn pairing_redeem_succeeds_even_when_dm_provisioning_fails() {
        // Provisioning failure must be silent — the pairing itself must succeed
        // and the caller must receive a successful response.
        //
        // The pairing notifier calls conversations.open once (for the DM used
        // to send the challenge code) — that must succeed so pairing completes.
        // The provisioner then calls conversations.open again; we fail that
        // second call to simulate a Slack API error during provisioning.
        let egress = Arc::new(RecordingRuntimeHttpEgress::conversations_open_fail_after(1));
        let (runtime, _root) = runtime_with_host_egress_override(Some(Some(
            host_egress_port_for_test(Arc::clone(&egress)),
        )))
        .await;
        let mounts =
            build_slack_host_beta_mounts(&runtime, config_without_legacy_actor()).expect("mounts");

        // Trigger pairing challenge.
        let first_body = dm_event_body_with(
            "Ev-dm-provision-fail",
            "pair me (fail)",
            "1710000002.000001",
        );
        post_signed_slack_event(&mounts.events, &first_body).await;
        if let Some(drain) = mounts.events.drain.as_ref() {
            drain.drain().await;
        }
        let pairing_code = wait_for_pairing_code(&egress).await;

        // Redeem the code — provisioning will fail in background but the HTTP
        // response must still be 200.
        let pairing_mount =
            slack_personal_binding_pairing_route_mount(mounts.personal_binding_pairing);
        let redeem_body = format!(r#"{{"channel":"slack","code":"{pairing_code}"}}"#);
        let redeem_response = pairing_mount
            .protected
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(WEBUI_V2_EXTENSION_PAIRING_REDEEM_PATH)
                    .header("content-type", "application/json")
                    .extension(WebUiAuthenticatedCaller {
                        tenant_id: TenantId::new(TENANT).expect("tenant"),
                        user_id: UserId::new(USER).expect("user"),
                        agent_id: Some(AgentId::new(AGENT).expect("agent")),
                        project_id: Some(ProjectId::new(PROJECT).expect("project")),
                    })
                    .body(Body::from(redeem_body))
                    .expect("redeem request builds"),
            )
            .await
            .expect("redeem route responds");

        // Pairing must succeed despite the provisioning failure.
        assert_eq!(
            redeem_response.status(),
            StatusCode::OK,
            "provisioning failure must not propagate to the pairing caller"
        );

        // Wait for the provisioner's conversations.open attempt (the second
        // call) so we know the background task ran and failed before asserting
        // that no target was persisted.
        wait_for_nth_conversations_open(&egress, 2).await;
        let mounts2 =
            build_slack_host_beta_mounts(&runtime, config_without_legacy_actor()).expect("mounts");
        let bundle = build_webui_services_with_slack_host_beta_mounts(
            &runtime,
            None,
            Some(&mounts2),
            SlackOperatorRouteVisibility::Hidden,
        )
        .expect("webui bundle");
        let targets = bundle
            .api
            .list_outbound_delivery_targets(operator_caller())
            .await
            .expect("target list");
        assert!(
            targets.targets.is_empty(),
            "failed DM provisioning must not persist a stale target"
        );

        runtime.shutdown().await.expect("runtime shuts down");
    }

    #[tokio::test]
    async fn slack_host_beta_targets_reject_non_advancing_route_cursor() {
        let provider = outbound_target_provider(
            config_without_channel_routes(),
            Arc::new(NonAdvancingCursorRouteStore),
        );

        let error = provider
            .list_outbound_delivery_targets(&shared_subject_caller())
            .await
            .expect_err("non-advancing cursor must fail closed");

        assert_eq!(error.code, RebornServicesErrorCode::Unavailable);
        assert_eq!(error.kind, RebornServicesErrorKind::ServiceUnavailable);
        assert_eq!(error.status_code, 503);
        assert!(error.retryable);
    }

    #[tokio::test]
    async fn slack_host_beta_targets_ignore_other_tenant_callers() {
        let (runtime, _root) = runtime().await;
        let mounts = build_slack_host_beta_mounts(&runtime, config()).expect("mounts");
        let bundle = build_webui_services_with_slack_host_beta_mounts(
            &runtime,
            None,
            Some(&mounts),
            SlackOperatorRouteVisibility::Hidden,
        )
        .expect("webui bundle");
        let shared_subject = shared_subject_caller();
        let target_id = bundle
            .api
            .list_outbound_delivery_targets(shared_subject)
            .await
            .expect("same tenant target list")
            .targets[0]
            .target
            .target_id
            .clone();
        let other_tenant = WebUiAuthenticatedCaller::new(
            TenantId::new("tenant:other").expect("tenant"),
            UserId::new(SHARED_SUBJECT).expect("shared subject"),
            Some(AgentId::new(AGENT).expect("agent")),
            Some(ProjectId::new(PROJECT).expect("project")),
        );

        let other_targets = bundle
            .api
            .list_outbound_delivery_targets(other_tenant.clone())
            .await
            .expect("other tenant target list");
        assert!(
            other_targets.targets.is_empty(),
            "Slack targets must not leak across tenant boundaries"
        );
        let write = bundle
            .api
            .set_outbound_preferences(
                other_tenant,
                RebornSetOutboundPreferencesRequest {
                    final_reply_target_id: Some(target_id),
                },
            )
            .await
            .expect_err("other tenant caller cannot select same target id");
        assert_eq!(write.code, RebornServicesErrorCode::NotFound);

        runtime.shutdown().await.expect("runtime shuts down");
    }

    #[test]
    fn slack_shared_channel_reply_target_binding_ref_rejects_oversized_raw() {
        let installation_id =
            AdapterInstallationId::new("i".repeat(120)).expect("long installation id validates");
        let agent_id = AgentId::new("a".repeat(120)).expect("long agent id validates");

        let error = slack_shared_channel_reply_target_binding_ref(
            &installation_id,
            &agent_id,
            Some(&ProjectId::new(PROJECT).expect("project")),
            &SlackTeamId::new(TEAM),
            "C0HOST",
        )
        .expect_err("oversized raw binding ref should fail closed");

        assert_eq!(error.code, RebornServicesErrorCode::Unavailable);
        assert_eq!(error.kind, RebornServicesErrorKind::ServiceUnavailable);
        assert_eq!(error.status_code, 503);
        assert!(error.retryable);
    }

    #[test]
    fn slack_shared_channel_reply_target_binding_ref_rejects_control_char_in_raw() {
        let error = slack_reply_target_binding_ref_from_raw("adapter:5:slack;\x01".to_string())
            .expect_err("control char must fail closed");

        assert_eq!(error.code, RebornServicesErrorCode::Unavailable);
        assert_eq!(error.kind, RebornServicesErrorKind::ServiceUnavailable);
        assert_eq!(error.status_code, 503);
        assert!(error.retryable);
    }

    #[test]
    fn slack_shared_channel_reply_target_binding_ref_round_trips_channel_id() {
        let provider =
            outbound_target_provider(config(), Arc::new(InMemorySlackChannelRouteStore::new()));
        let binding_ref = slack_shared_channel_reply_target_binding_ref(
            &AdapterInstallationId::new(INSTALLATION).expect("installation"),
            &AgentId::new(AGENT).expect("agent"),
            Some(&ProjectId::new(PROJECT).expect("project")),
            &SlackTeamId::new(TEAM),
            "C0HOST",
        )
        .expect("binding ref builds");

        assert_eq!(
            provider.channel_id_for_reply_target_binding_ref(&binding_ref),
            Some("C0HOST".to_string())
        );
    }

    #[test]
    fn slack_host_beta_target_id_parser_rejects_empty_channel_suffix() {
        let provider =
            outbound_target_provider(config(), Arc::new(InMemorySlackChannelRouteStore::new()));
        let target_id =
            RebornOutboundDeliveryTargetId::new("slack:shared-channel:T0HOST:").expect("target id");

        assert!(provider.channel_id_for_target_id(&target_id).is_none());
    }

    #[tokio::test]
    async fn slack_host_beta_admin_route_delete_revokes_saved_outbound_target() {
        let (runtime, _root) = runtime().await;
        let mounts = build_slack_host_beta_mounts(&runtime, config_without_channel_routes())
            .expect("mounts");
        let route_mount = slack_channel_route_admin_route_mount(mounts.channel_routes.clone());
        let bundle = build_webui_services_with_slack_host_beta_mounts(
            &runtime,
            None,
            Some(&mounts),
            SlackOperatorRouteVisibility::Hidden,
        )
        .expect("webui bundle");
        upsert_slack_channel_route(&route_mount, "C0HOST", SHARED_SUBJECT).await;

        let shared_subject = shared_subject_caller();
        let targets = bundle
            .api
            .list_outbound_delivery_targets(shared_subject.clone())
            .await
            .expect("shared subject target list");
        assert_eq!(targets.targets.len(), 1);
        let target_id = targets.targets[0].target.target_id.clone();

        bundle
            .api
            .set_outbound_preferences(
                shared_subject.clone(),
                RebornSetOutboundPreferencesRequest {
                    final_reply_target_id: Some(target_id.clone()),
                },
            )
            .await
            .expect("set Slack target");

        delete_slack_channel_route(&route_mount, "C0HOST").await;

        let preference = bundle
            .api
            .get_outbound_preferences(shared_subject.clone())
            .await
            .expect("get Slack target preference");
        assert_eq!(
            preference.final_reply_target_status,
            RebornOutboundDeliveryTargetStatus::Unavailable
        );
        assert!(preference.final_reply_target.is_none());

        let stale_set = bundle
            .api
            .set_outbound_preferences(
                shared_subject,
                RebornSetOutboundPreferencesRequest {
                    final_reply_target_id: Some(target_id),
                },
            )
            .await
            .expect_err("deleted Slack route target must reject writes");
        assert_eq!(stale_set.code, RebornServicesErrorCode::NotFound);

        runtime.shutdown().await.expect("runtime shuts down");
    }

    #[tokio::test]
    async fn slack_host_beta_admin_route_owner_change_overrides_static_channel_route() {
        let (runtime, _root) = runtime().await;
        let mounts = build_slack_host_beta_mounts(&runtime, config()).expect("mounts");
        let route_mount = slack_channel_route_admin_route_mount(mounts.channel_routes.clone());
        let bundle = build_webui_services_with_slack_host_beta_mounts(
            &runtime,
            None,
            Some(&mounts),
            SlackOperatorRouteVisibility::Hidden,
        )
        .expect("webui bundle");
        let shared_subject = shared_subject_caller();
        let operator = operator_caller();
        let target_id = bundle
            .api
            .list_outbound_delivery_targets(shared_subject.clone())
            .await
            .expect("static target list")
            .targets[0]
            .target
            .target_id
            .clone();

        upsert_slack_channel_route(&route_mount, "C0HOST", USER).await;

        assert!(
            bundle
                .api
                .list_outbound_delivery_targets(shared_subject.clone())
                .await
                .expect("old owner target list")
                .targets
                .is_empty(),
            "durable admin route must override static route owner"
        );
        let stale_write = bundle
            .api
            .set_outbound_preferences(
                shared_subject,
                RebornSetOutboundPreferencesRequest {
                    final_reply_target_id: Some(target_id),
                },
            )
            .await
            .expect_err("old static route owner cannot select admin-reassigned target");
        assert_eq!(stale_write.code, RebornServicesErrorCode::NotFound);
        let operator_targets = bundle
            .api
            .list_outbound_delivery_targets(operator)
            .await
            .expect("new owner target list");
        assert_eq!(operator_targets.targets.len(), 1);
        assert_eq!(
            operator_targets.targets[0].target.display_name.as_str(),
            "Slack channel C0HOST"
        );

        runtime.shutdown().await.expect("runtime shuts down");
    }

    #[tokio::test]
    async fn slack_host_beta_admin_route_owner_change_moves_outbound_target_authority() {
        let (runtime, _root) = runtime().await;
        let mounts = build_slack_host_beta_mounts(&runtime, config_without_channel_routes())
            .expect("mounts");
        let route_mount = slack_channel_route_admin_route_mount(mounts.channel_routes.clone());
        let bundle = build_webui_services_with_slack_host_beta_mounts(
            &runtime,
            None,
            Some(&mounts),
            SlackOperatorRouteVisibility::Hidden,
        )
        .expect("webui bundle");
        upsert_slack_channel_route(&route_mount, "C0HOST", SHARED_SUBJECT).await;

        let shared_subject = shared_subject_caller();
        let operator = operator_caller();
        assert_eq!(
            bundle
                .api
                .list_outbound_delivery_targets(shared_subject.clone())
                .await
                .expect("shared target list")
                .targets
                .len(),
            1
        );
        assert!(
            bundle
                .api
                .list_outbound_delivery_targets(operator.clone())
                .await
                .expect("operator target list")
                .targets
                .is_empty()
        );

        upsert_slack_channel_route(&route_mount, "C0HOST", USER).await;

        assert!(
            bundle
                .api
                .list_outbound_delivery_targets(shared_subject)
                .await
                .expect("old owner target list")
                .targets
                .is_empty(),
            "old route subject must lose Slack target authority"
        );
        let operator_targets = bundle
            .api
            .list_outbound_delivery_targets(operator)
            .await
            .expect("new owner target list");
        assert_eq!(operator_targets.targets.len(), 1);
        assert_eq!(
            operator_targets.targets[0].target.display_name.as_str(),
            "Slack channel C0HOST"
        );

        runtime.shutdown().await.expect("runtime shuts down");
    }

    #[tokio::test]
    async fn slack_host_beta_admin_routes_feed_outbound_target_provider() {
        let (runtime, _root) = runtime().await;
        let mounts = build_slack_host_beta_mounts(&runtime, config_without_channel_routes())
            .expect("mounts");
        let bundle = build_webui_services_with_slack_host_beta_mounts(
            &runtime,
            None,
            Some(&mounts),
            SlackOperatorRouteVisibility::Visible,
        )
        .expect("webui bundle");
        let app = webui_v2_app(
            bundle.clone(),
            WebuiServeConfig::new(
                TenantId::new(TENANT).expect("tenant"),
                Arc::new(OperatorTokenAuthenticator),
                Vec::new(),
            )
            .with_default_agent_id(AgentId::new(AGENT).expect("agent"))
            .with_default_project_id(ProjectId::new(PROJECT).expect("project"))
            .with_slack_channel_routes(mounts.channel_routes),
        )
        .expect("webui app");

        let save = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(WEBUI_V2_CHANNELS_SLACK_ALLOWED_PATH)
                    .header("authorization", "Bearer operator-token")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"channel_ids":["C0DYNAMIC"]}"#))
                    .expect("save request builds"),
            )
            .await
            .expect("save route responds");
        assert_eq!(save.status(), StatusCode::OK);
        let body = axum::body::to_bytes(save.into_body(), 64 * 1024)
            .await
            .expect("save body");
        let body: serde_json::Value = serde_json::from_slice(&body).expect("save json");
        let subject_user_id = body["channels"][0]["subject_user_id"]
            .as_str()
            .expect("assigned subject");
        let caller = WebUiAuthenticatedCaller::new(
            TenantId::new(TENANT).expect("tenant"),
            UserId::new(subject_user_id).expect("subject user"),
            Some(AgentId::new(AGENT).expect("agent")),
            Some(ProjectId::new(PROJECT).expect("project")),
        );

        let targets = bundle
            .api
            .list_outbound_delivery_targets(caller)
            .await
            .expect("dynamic route target list");

        assert_eq!(targets.targets.len(), 1);
        assert_eq!(
            targets.targets[0].target.target_id.as_str(),
            "slack:shared-channel:T0HOST:C0DYNAMIC"
        );
        assert_eq!(
            targets.targets[0].target.display_name.as_str(),
            "Slack channel C0DYNAMIC"
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
            .with_default_project_id(ProjectId::new(PROJECT).expect("project"))
            .with_model_gateway_override(Arc::new(StaticGateway)),
        )
        .await
        .expect("runtime builds");
        let team_only_config = SlackHostBetaConfig::new(SlackHostBetaConfigInput {
            tenant_id: TenantId::new(TENANT).expect("tenant"),
            agent_id: AgentId::new(AGENT).expect("agent"),
            project_id: Some(ProjectId::new(PROJECT).expect("project")),
            installation_id: INSTALLATION.to_string(),
            team_id: SlackTeamId::new(TEAM),
            api_app_id: None,
            slack_user_id: Some(SLACK_USER.to_string()),
            user_id: UserId::new(USER).expect("user"),
            shared_subject_user_id: None,
            channel_routes: Vec::new(),
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
    fn slack_host_beta_config_rejects_duplicate_channel_routes() {
        let error = SlackHostBetaConfig::new(SlackHostBetaConfigInput {
            tenant_id: TenantId::new(TENANT).expect("tenant"),
            agent_id: AgentId::new(AGENT).expect("agent"),
            project_id: Some(ProjectId::new(PROJECT).expect("project")),
            installation_id: INSTALLATION.to_string(),
            team_id: SlackTeamId::new(TEAM),
            api_app_id: Some(API_APP.to_string()),
            slack_user_id: Some(SLACK_USER.to_string()),
            user_id: UserId::new(USER).expect("user"),
            shared_subject_user_id: None,
            channel_routes: vec![
                SlackHostBetaChannelRoute::new(
                    "C0HOST",
                    UserId::new("first-subject").expect("first subject"),
                ),
                SlackHostBetaChannelRoute::new(
                    "C0HOST",
                    UserId::new("second-subject").expect("second subject"),
                ),
            ],
            signing_secret: SecretString::from(SECRET),
            bot_token: SecretString::from("xoxb-host-token"),
        })
        .expect_err("duplicate channel routes must fail closed");

        assert!(
            error.to_string().contains("duplicate channel_id 'C0HOST'"),
            "message: {error}"
        );
    }

    #[test]
    fn slack_egress_scope_template_uses_configured_tenant_agent_and_project() {
        let config = config();

        let scope = slack_egress_scope_template(&config);

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
            team_id: SlackTeamId::new(TEAM),
            api_app_id: Some(API_APP.to_string()),
            slack_user_id: Some(SLACK_USER.to_string()),
            user_id: UserId::new(USER).expect("user"),
            shared_subject_user_id: None,
            channel_routes: vec![SlackHostBetaChannelRoute::new(
                "C0HOST",
                UserId::new(SHARED_SUBJECT).expect("shared subject"),
            )],
            signing_secret: SecretString::from(SECRET),
            bot_token: SecretString::from("xoxb-host-token"),
        })
        .expect("valid config")
    }

    fn config_without_legacy_actor() -> SlackHostBetaConfig {
        SlackHostBetaConfig::new(SlackHostBetaConfigInput {
            tenant_id: TenantId::new(TENANT).expect("tenant"),
            agent_id: AgentId::new(AGENT).expect("agent"),
            project_id: Some(ProjectId::new(PROJECT).expect("project")),
            installation_id: INSTALLATION.to_string(),
            team_id: SlackTeamId::new(TEAM),
            api_app_id: Some(API_APP.to_string()),
            slack_user_id: None,
            user_id: UserId::new(USER).expect("user"),
            shared_subject_user_id: None,
            channel_routes: Vec::new(),
            signing_secret: SecretString::from(SECRET),
            bot_token: SecretString::from("xoxb-host-token"),
        })
        .expect("valid config")
    }

    fn config_without_channel_routes() -> SlackHostBetaConfig {
        SlackHostBetaConfig::new(SlackHostBetaConfigInput {
            tenant_id: TenantId::new(TENANT).expect("tenant"),
            agent_id: AgentId::new(AGENT).expect("agent"),
            project_id: Some(ProjectId::new(PROJECT).expect("project")),
            installation_id: INSTALLATION.to_string(),
            team_id: SlackTeamId::new(TEAM),
            api_app_id: Some(API_APP.to_string()),
            slack_user_id: Some(SLACK_USER.to_string()),
            user_id: UserId::new(USER).expect("user"),
            shared_subject_user_id: Some(UserId::new(SHARED_SUBJECT).expect("shared subject")),
            channel_routes: Vec::new(),
            signing_secret: SecretString::from(SECRET),
            bot_token: SecretString::from("xoxb-host-token"),
        })
        .expect("valid config")
    }

    fn outbound_target_provider_config(
        config: SlackHostBetaConfig,
    ) -> SlackOutboundTargetProviderConfig {
        SlackOutboundTargetProviderConfig {
            tenant_id: config.tenant_id,
            agent_id: config.agent_id,
            project_id: config.project_id,
            installation_id: config.installation_id,
            team_id: config.team_id,
            configured_channel_routes: config
                .channel_routes
                .into_iter()
                .map(|route| {
                    SlackConfiguredChannelRoute::new(route.channel_id, route.subject_user_id)
                })
                .collect(),
        }
    }

    fn outbound_target_provider(
        config: SlackHostBetaConfig,
        channel_route_store: Arc<dyn SlackChannelRouteStore>,
    ) -> SlackHostBetaOutboundTargetProvider {
        SlackHostBetaOutboundTargetProvider::new(
            outbound_target_provider_config(config),
            channel_route_store,
            Arc::new(InMemorySlackPersonalDmTargetStore::new()),
        )
    }

    fn operator_caller() -> WebUiAuthenticatedCaller {
        WebUiAuthenticatedCaller::new(
            TenantId::new(TENANT).expect("tenant"),
            UserId::new(USER).expect("user"),
            Some(AgentId::new(AGENT).expect("agent")),
            Some(ProjectId::new(PROJECT).expect("project")),
        )
    }

    fn shared_subject_caller() -> WebUiAuthenticatedCaller {
        WebUiAuthenticatedCaller::new(
            TenantId::new(TENANT).expect("tenant"),
            UserId::new(SHARED_SUBJECT).expect("shared subject"),
            Some(AgentId::new(AGENT).expect("agent")),
            Some(ProjectId::new(PROJECT).expect("project")),
        )
    }

    fn personal_dm_target_provisioner_for_test(
        runtime: &RebornRuntime,
        config: &SlackHostBetaConfig,
    ) -> SlackPersonalDmTargetProvisioner {
        let token_handle = slack_bot_token_handle().expect("bot token handle");
        SlackPersonalDmTargetProvisioner::new(
            config.tenant_id.clone(),
            config.installation_id.clone(),
            config.team_id.clone(),
            slack_protocol_egress(runtime, config, token_handle.clone()).expect("Slack egress"),
            token_handle,
            personal_dm_target_store_for_test(runtime, config),
        )
    }

    fn personal_dm_target_store_for_test(
        runtime: &RebornRuntime,
        config: &SlackHostBetaConfig,
    ) -> Arc<dyn SlackPersonalDmTargetStore> {
        let local_runtime = runtime
            .services()
            .local_runtime
            .as_ref()
            .expect("local runtime");
        Arc::new(FilesystemSlackHostState::new(
            Arc::clone(&local_runtime.host_state_filesystem),
            config.tenant_id.clone(),
            config.user_id.clone(),
            config.agent_id.clone(),
            config.project_id.clone(),
        ))
    }

    async fn upsert_slack_channel_route(
        route_mount: &SlackChannelRouteAdminRouteMount,
        channel_id: &str,
        subject_user_id: &str,
    ) {
        let response = route_mount
            .protected
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(WEBUI_V2_CHANNELS_SLACK_ROUTES_PATH)
                    .header("content-type", "application/json")
                    .extension(operator_caller())
                    .body(Body::from(format!(
                        r#"{{"channel_id":"{channel_id}","subject_user_id":"{subject_user_id}"}}"#
                    )))
                    .expect("upsert request builds"),
            )
            .await
            .expect("upsert route responds");
        assert_eq!(response.status(), StatusCode::OK);
    }

    async fn delete_slack_channel_route(
        route_mount: &SlackChannelRouteAdminRouteMount,
        channel_id: &str,
    ) {
        let response = route_mount
            .protected
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(WEBUI_V2_CHANNELS_SLACK_ROUTES_PATH)
                    .header("content-type", "application/json")
                    .extension(operator_caller())
                    .body(Body::from(format!(r#"{{"channel_id":"{channel_id}"}}"#)))
                    .expect("delete request builds"),
            )
            .await
            .expect("delete route responds");
        assert_eq!(response.status(), StatusCode::OK);
    }

    async fn post_signed_slack_event(mount: &PublicRouteMount, body: &str) {
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
                    .body(Body::from(body.to_string()))
                    .expect("request builds"),
            )
            .await
            .expect("router responds");

        assert_eq!(response.status(), StatusCode::OK);
    }

    async fn runtime() -> (RebornRuntime, tempfile::TempDir) {
        runtime_with_host_egress_override(None).await
    }

    async fn runtime_with_host_egress_override(
        host_egress_override: Option<Option<HostRuntimeHttpEgressPort>>,
    ) -> (RebornRuntime, tempfile::TempDir) {
        let root = tempfile::tempdir().expect("tempdir");
        let mut build_input = RebornBuildInput::local_dev(USER, root.path().join("local-dev"))
            .with_runtime_policy(local_dev_runtime_policy().expect("local policy"));
        if let Some(host_egress) = host_egress_override {
            build_input = build_input.with_host_runtime_http_egress_for_test(host_egress);
        }
        let runtime = build_reborn_runtime(
            RebornRuntimeInput::from_services(build_input)
                .with_identity(RebornRuntimeIdentity {
                    tenant_id: TENANT.to_string(),
                    agent_id: AGENT.to_string(),
                    source_binding_id: "slack-host-source".to_string(),
                    reply_target_binding_id: "slack-host-reply".to_string(),
                })
                .with_default_project_id(ProjectId::new(PROJECT).expect("project"))
                .with_model_gateway_override(Arc::new(StaticGateway)),
        )
        .await
        .expect("runtime builds");
        (runtime, root)
    }

    async fn wait_for_slack_thread_history(
        runtime: &RebornRuntime,
    ) -> ironclaw_threads::ThreadHistory {
        wait_for_slack_thread_history_with_owner(runtime, Some(UserId::new(USER).expect("user")))
            .await
    }

    async fn wait_for_slack_thread_history_with_owner(
        runtime: &RebornRuntime,
        owner_user_id: Option<UserId>,
    ) -> ironclaw_threads::ThreadHistory {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        let thread_service = runtime.webui_thread_service();
        let scope = ThreadScope {
            tenant_id: TenantId::new(TENANT).expect("tenant"),
            agent_id: AgentId::new(AGENT).expect("agent"),
            project_id: Some(ProjectId::new(PROJECT).expect("project")),
            owner_user_id,
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

    async fn slack_message_count_with_text(
        runtime: &RebornRuntime,
        owner_user_id: Option<UserId>,
        text: &str,
    ) -> usize {
        let thread_service = runtime.webui_thread_service();
        let scope = ThreadScope {
            tenant_id: TenantId::new(TENANT).expect("tenant"),
            agent_id: AgentId::new(AGENT).expect("agent"),
            project_id: Some(ProjectId::new(PROJECT).expect("project")),
            owner_user_id,
            mission_id: None,
        };
        let threads = thread_service
            .list_threads_for_scope(ListThreadsForScopeRequest {
                scope: scope.clone(),
                limit: Some(100),
                cursor: None,
            })
            .await
            .expect("list Slack-created threads");
        let mut count = 0;
        for thread in threads.threads {
            let history = thread_service
                .list_thread_history(ThreadHistoryRequest {
                    scope: scope.clone(),
                    thread_id: thread.thread_id,
                })
                .await
                .expect("read Slack-created thread history");
            count += history
                .messages
                .iter()
                .filter(|message| message.content.as_deref() == Some(text))
                .count();
        }
        count
    }

    async fn wait_for_slack_message_count_with_text(
        runtime: &RebornRuntime,
        owner_user_id: Option<UserId>,
        text: &str,
        expected: usize,
    ) -> usize {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            let count = slack_message_count_with_text(runtime, owner_user_id.clone(), text).await;
            if count >= expected {
                return count;
            }
            if tokio::time::Instant::now() >= deadline {
                panic!(
                    "Slack message {text:?} count stayed below {expected}; latest count: {count}"
                );
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    async fn assert_no_slack_threads_for_owner(
        runtime: &RebornRuntime,
        owner_user_id: Option<UserId>,
    ) {
        let thread_service = runtime.webui_thread_service();
        let scope = ThreadScope {
            tenant_id: TenantId::new(TENANT).expect("tenant"),
            agent_id: AgentId::new(AGENT).expect("agent"),
            project_id: Some(ProjectId::new(PROJECT).expect("project")),
            owner_user_id,
            mission_id: None,
        };
        tokio::time::sleep(Duration::from_millis(100)).await;
        let threads = thread_service
            .list_threads_for_scope(ListThreadsForScopeRequest {
                scope,
                limit: Some(1),
                cursor: None,
            })
            .await
            .expect("list Slack-created threads");
        assert!(
            threads.threads.is_empty(),
            "unexpected Slack-created thread"
        );
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
          "team_id":"T0HOST",
          "api_app_id":"A0HOST",
          "event_id":"Ev-host-beta-custom-resolver",
          "event":{
            "type":"message",
            "channel_type":"im",
            "user":"U0HOST",
            "channel":"D0HOST",
            "text":"hello",
            "ts":"1710000000.000001"
          }
        }"#
    }

    fn dm_event_body_with(event_id: &str, text: &str, ts: &str) -> String {
        serde_json::json!({
            "type": "event_callback",
            "team_id": TEAM,
            "api_app_id": API_APP,
            "event_id": event_id,
            "event": {
                "type": "message",
                "channel_type": "im",
                "user": SLACK_USER,
                "channel": "D0HOST",
                "text": text,
                "ts": ts
            }
        })
        .to_string()
    }

    fn app_mention_event_body_with(event_id: &str, text: &str, ts: &str) -> String {
        serde_json::json!({
            "type": "event_callback",
            "team_id": TEAM,
            "api_app_id": API_APP,
            "event_id": event_id,
            "event": {
                "type": "app_mention",
                "user": SLACK_USER,
                "channel": "C0HOST",
                "text": text,
                "ts": ts
            }
        })
        .to_string()
    }

    fn thread_message_event_body_with(
        event_id: &str,
        text: &str,
        ts: &str,
        thread_ts: &str,
    ) -> String {
        serde_json::json!({
            "type": "event_callback",
            "team_id": TEAM,
            "api_app_id": API_APP,
            "event_id": event_id,
            "event": {
                "type": "message",
                "user": SLACK_USER,
                "channel": "C0HOST",
                "text": text,
                "ts": ts,
                "thread_ts": thread_ts
            }
        })
        .to_string()
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

    async fn wait_for_pairing_code(egress: &RecordingRuntimeHttpEgress) -> String {
        for _ in 0..40 {
            if let Some(code) = egress.pairing_code() {
                return code;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("Slack pairing notifier did not post a pairing code");
    }

    async fn wait_for_nth_conversations_open(egress: &RecordingRuntimeHttpEgress, n: usize) {
        for _ in 0..80 {
            let count = egress
                .requests()
                .iter()
                .filter(|r| r.url.contains("/api/conversations.open"))
                .count();
            if count >= n {
                return;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!(
            "expected {n} conversations.open call(s); only {} recorded",
            egress
                .requests()
                .iter()
                .filter(|r| r.url.contains("/api/conversations.open"))
                .count()
        );
    }

    async fn wait_for_slack_post_message(
        egress: &RecordingRuntimeHttpEgress,
        expected_text: &str,
    ) -> serde_json::Value {
        for _ in 0..80 {
            if let Some(body) = egress.post_message_body_with_text(expected_text) {
                return body;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!(
            "Slack final reply was not posted; recorded egress requests: {:?}",
            egress.request_bodies()
        );
    }

    async fn wait_for_slack_post_messages(
        egress: &RecordingRuntimeHttpEgress,
        expected_text: &str,
        expected_len: usize,
    ) -> Vec<serde_json::Value> {
        for _ in 0..80 {
            let bodies = egress.post_message_bodies_with_text(expected_text);
            if bodies.len() >= expected_len {
                return bodies;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!(
            "expected {expected_len} Slack posts with text {expected_text:?}; recorded egress requests: {:?}",
            egress.request_bodies()
        );
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
        requests: std::sync::Mutex<Vec<NetworkHttpRequest>>,
        /// If set, returned for ALL conversations.open calls.
        conversations_open_response: Option<(u16, Vec<u8>)>,
        /// If set, conversations.open succeeds this many times then fails.
        conversations_open_fail_after: Option<usize>,
    }

    #[async_trait]
    impl NetworkHttpEgress for RecordingRuntimeHttpEgress {
        async fn execute(
            &self,
            request: NetworkHttpRequest,
        ) -> Result<NetworkHttpResponse, NetworkHttpError> {
            let (status, response) = if request.url.contains("/api/conversations.open") {
                if let Some(n) = self.conversations_open_fail_after {
                    let count = self
                        .requests
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .iter()
                        .filter(|r| r.url.contains("/api/conversations.open"))
                        .count();
                    if count >= n {
                        (200, br#"{"ok":false,"error":"not_allowed"}"#.to_vec())
                    } else {
                        (200, br#"{"ok":true,"channel":{"id":"D0HOST"}}"#.to_vec())
                    }
                } else {
                    self.conversations_open_response.clone().unwrap_or_else(|| {
                        (200, br#"{"ok":true,"channel":{"id":"D0HOST"}}"#.to_vec())
                    })
                }
            } else {
                (200, br#"{"ok":true}"#.to_vec())
            };
            self.requests
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(request);
            Ok(NetworkHttpResponse {
                status,
                headers: Vec::new(),
                body: response,
                usage: NetworkUsage {
                    request_bytes: 0,
                    response_bytes: 0,
                    resolved_ip: None,
                },
            })
        }
    }

    fn host_egress_port_for_test(
        network: Arc<RecordingRuntimeHttpEgress>,
    ) -> HostRuntimeHttpEgressPort {
        test_host_runtime_services()
            .with_secret_store(Arc::new(InMemorySecretStore::new()))
            .try_with_host_http_egress(RecordingNetworkHttpEgress(network))
            .expect("host HTTP egress should wire")
            .host_runtime_http_egress_port()
            .expect("host runtime HTTP egress port should be configured")
    }

    fn test_host_runtime_services() -> HostRuntimeServices<
        LocalFilesystem,
        InMemoryResourceGovernor,
        InMemoryProcessStore,
        InMemoryProcessResultStore,
    > {
        HostRuntimeServices::new(
            Arc::new(ExtensionRegistry::new()),
            Arc::new(LocalFilesystem::new()),
            Arc::new(InMemoryResourceGovernor::new()),
            Arc::new(GrantAuthorizer::new()),
            ProcessServices::in_memory(),
            CapabilitySurfaceVersion::new("surface-v1").expect("surface version"),
        )
    }

    struct RecordingNetworkHttpEgress(Arc<RecordingRuntimeHttpEgress>);

    #[async_trait]
    impl NetworkHttpEgress for RecordingNetworkHttpEgress {
        async fn execute(
            &self,
            request: NetworkHttpRequest,
        ) -> Result<NetworkHttpResponse, NetworkHttpError> {
            self.0.execute(request).await
        }
    }

    #[derive(Debug)]
    struct FailingSlackPersonalDmTargetStore;

    #[async_trait]
    impl SlackPersonalDmTargetStore for FailingSlackPersonalDmTargetStore {
        async fn load_personal_dm_target(
            &self,
            _key: &crate::slack_outbound_targets::SlackPersonalDmTargetKey,
        ) -> Result<
            Option<crate::slack_outbound_targets::SlackPersonalDmTarget>,
            SlackPersonalDmTargetError,
        > {
            Err(SlackPersonalDmTargetError::StoreUnavailable)
        }

        async fn upsert_personal_dm_target(
            &self,
            target: crate::slack_outbound_targets::SlackPersonalDmTarget,
        ) -> Result<crate::slack_outbound_targets::SlackPersonalDmTarget, SlackPersonalDmTargetError>
        {
            Ok(target)
        }
    }

    impl RecordingRuntimeHttpEgress {
        fn conversations_open_response(status: u16, body: &[u8]) -> Self {
            Self {
                requests: std::sync::Mutex::new(Vec::new()),
                conversations_open_response: Some((status, body.to_vec())),
                conversations_open_fail_after: None,
            }
        }

        /// Returns a successful conversations.open response for the first
        /// `n` calls, then returns an error response for all subsequent calls.
        fn conversations_open_fail_after(n: usize) -> Self {
            Self {
                requests: std::sync::Mutex::new(Vec::new()),
                conversations_open_response: None,
                conversations_open_fail_after: Some(n),
            }
        }

        fn requests(&self) -> Vec<NetworkHttpRequest> {
            self.requests
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .clone()
        }

        fn request_bodies(&self) -> Vec<serde_json::Value> {
            self.requests
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .iter()
                .filter_map(|request| {
                    serde_json::from_slice::<serde_json::Value>(&request.body).ok()
                })
                .collect()
        }

        fn pairing_code(&self) -> Option<String> {
            self.requests
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .iter()
                .filter(|request| request.url.contains("/api/chat.postMessage"))
                .filter_map(|request| {
                    serde_json::from_slice::<serde_json::Value>(&request.body).ok()
                })
                .filter_map(|body| body["text"].as_str().map(str::to_string))
                .find_map(|text| {
                    text.split(" code ")
                        .nth(1)
                        .and_then(|suffix| suffix.split(" in WebChat").next())
                        .map(str::to_string)
                })
        }

        fn post_message_body_with_text(&self, expected_text: &str) -> Option<serde_json::Value> {
            self.post_message_bodies_with_text(expected_text)
                .into_iter()
                .next()
        }

        fn post_message_bodies_with_text(&self, expected_text: &str) -> Vec<serde_json::Value> {
            self.requests
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .iter()
                .filter(|request| request.url.contains("/api/chat.postMessage"))
                .filter_map(|request| {
                    serde_json::from_slice::<serde_json::Value>(&request.body).ok()
                })
                .filter(|body| body["text"].as_str() == Some(expected_text))
                .collect()
        }
    }

    // ---------------------------------------------------------------------------
    // Test 3 — hook wiring e2e
    // ---------------------------------------------------------------------------
    //
    // Build a runtime with the trigger poller enabled, call
    // `build_slack_host_beta_mounts` (which wires `set_trigger_post_submit_hook`
    // internally), seed a due personal trigger, wait for the poller to fire it,
    // then assert that a `TriggeredRunDeliveryRecord` was written to the
    // host-state filesystem via the production hook → driver path.

    async fn runtime_with_trigger_poller() -> (RebornRuntime, tempfile::TempDir) {
        use ironclaw_triggers::TriggerPollerWorkerConfig;
        let root = tempfile::tempdir().expect("tempdir");
        let build_input = RebornBuildInput::local_dev(USER, root.path().join("local-dev"))
            .with_runtime_policy(local_dev_runtime_policy().expect("local policy"));
        let runtime = build_reborn_runtime(
            RebornRuntimeInput::from_services(build_input)
                .with_identity(RebornRuntimeIdentity {
                    tenant_id: TENANT.to_string(),
                    agent_id: AGENT.to_string(),
                    source_binding_id: "hook-wiring-e2e-source".to_string(),
                    reply_target_binding_id: "hook-wiring-e2e-reply".to_string(),
                })
                .with_model_gateway_override(Arc::new(StaticGateway))
                .with_trigger_poller_settings(
                    crate::TriggerPollerSettings::enabled_with_tenant_scoped_authorizer_for_test()
                        .with_worker_config(TriggerPollerWorkerConfig {
                            poll_interval: std::time::Duration::from_millis(20),
                            ..TriggerPollerWorkerConfig::default()
                        }),
                ),
        )
        .await
        .expect("runtime with trigger poller builds");
        (runtime, root)
    }

    #[tokio::test]
    async fn build_slack_host_beta_mounts_wires_trigger_delivery_hook_writes_record() {
        use std::time::Instant;

        use chrono::Utc;
        use ironclaw_conversations::{AdapterInstallationId, AdapterKind, ExternalActorRef};
        use ironclaw_triggers::{
            TRIGGER_TRUSTED_ADAPTER_INSTALLATION_ID, TRIGGER_TRUSTED_ADAPTER_KIND,
            TRIGGER_TRUSTED_EXTERNAL_ACTOR_NAMESPACE, TriggerCompletionPolicy, TriggerId,
            TriggerRecord, TriggerSchedule, TriggerSourceKind, TriggerState,
        };

        let (runtime, _tmp) = runtime_with_trigger_poller().await;

        // Wire the delivery hook by calling the production mount builder.
        let _mounts =
            build_slack_host_beta_mounts(&runtime, config()).expect("mounts should build");

        // Pair the trigger actor so the trusted submitter can resolve the
        // creator's user binding (fails closed for unpaired actors by design).
        let tenant_id = TenantId::new(TENANT).expect("tenant");
        let user_id = UserId::new(USER).expect("user");
        let pairing = runtime
            .trigger_conversation_pairing()
            .expect("trigger conversation pairing service");
        pairing
            .pair_external_actor(
                tenant_id.clone(),
                AdapterKind::new(TRIGGER_TRUSTED_ADAPTER_KIND).expect("adapter kind"),
                AdapterInstallationId::new(TRIGGER_TRUSTED_ADAPTER_INSTALLATION_ID)
                    .expect("installation id"),
                ExternalActorRef::new(TRIGGER_TRUSTED_EXTERNAL_ACTOR_NAMESPACE, user_id.as_str())
                    .expect("actor ref"),
                user_id.clone(),
            )
            .await
            .expect("pair external actor for trigger creator");

        // Seed a due trigger so the poller picks it up immediately.
        let repo = runtime
            .trigger_repository()
            .expect("local-dev runtime exposes trigger repository");
        let trigger_id = TriggerId::new();
        repo.upsert_trigger(TriggerRecord {
            trigger_id,
            tenant_id: tenant_id.clone(),
            creator_user_id: user_id.clone(),
            agent_id: Some(AgentId::new(AGENT).expect("agent")),
            project_id: None,
            name: "hook-wiring-e2e".to_string(),
            source: TriggerSourceKind::Schedule,
            schedule: TriggerSchedule::cron("* * * * *").expect("valid cron"),
            completion_policy: TriggerCompletionPolicy::CompleteAfterFirstFire,
            prompt: "hook-wiring-e2e-prompt-marker".to_string(),
            state: TriggerState::Scheduled,
            next_run_at: Utc::now() - chrono::Duration::seconds(120),
            last_run_at: None,
            last_fired_slot: None,
            last_status: None,
            active_fire_slot: None,
            active_run_ref: None,
            created_at: Utc::now(),
        })
        .await
        .expect("upsert trigger record");

        // Wait for the poller to fire the trigger.  `mark_fire_accepted` sets
        // both `last_fired_slot` and `active_run_ref` atomically, so if we see
        // `last_fired_slot` we can also safely read the run_id.
        let deadline = Instant::now() + std::time::Duration::from_secs(15);
        let mut fired_run_id = None;
        while Instant::now() < deadline {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            let current = repo
                .get_trigger(tenant_id.clone(), trigger_id)
                .await
                .expect("get_trigger")
                .expect("record present");
            if current.last_fired_slot.is_some() {
                fired_run_id = current.active_run_ref;
                break;
            }
        }

        // Read delivery records from the unified outbound store that the
        // production hook writes through.  `local_runtime` is `pub(crate)`
        // — accessible here because this test lives in the same crate.
        let local_runtime = runtime
            .services()
            .local_runtime
            .as_ref()
            .expect("local-dev runtime has local_runtime services");
        let delivery_store = Arc::clone(&local_runtime.triggered_run_delivery);

        // Poll briefly for the delivery record.  The driver spawns a background
        // task; for the `NoDefaultConfigured` fast-path it should complete well
        // within 2 s.
        let mut delivery_record = None;
        if let Some(run_id) = fired_run_id {
            let delivery_deadline = Instant::now() + std::time::Duration::from_secs(5);
            while Instant::now() < delivery_deadline {
                if let Ok(Some(rec)) = delivery_store.load_triggered_run_delivery(run_id).await {
                    delivery_record = Some(rec);
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        }

        runtime.shutdown().await.expect("runtime shutdown");

        assert!(
            fired_run_id.is_some(),
            "trigger did not fire within 15 s — hook wiring e2e stalled"
        );
        assert!(
            delivery_record.is_some(),
            "no TriggeredRunDeliveryRecord written after trigger fire — \
             hook → driver wiring broken; fired_run_id={fired_run_id:?}"
        );
    }

    /// Regression guard: `build_triggered_run_delivery_hook` must wire the same
    /// `CommunicationPreferenceRepository` Arc that the WebUI
    /// `RebornOutboundPreferencesFacade` writes through
    /// (`local_runtime.outbound_preferences`) into
    /// `SlackFinalReplyDeliveryServices.communication_preferences`.
    ///
    /// Pre-fix bug: `build_triggered_run_delivery_hook` constructed a fresh
    /// `FilesystemOutboundStateStore::new(Arc::clone(&local_runtime.host_state_filesystem))`
    /// as the `communication_preferences` argument, while the WebUI facade wrote
    /// through `local_runtime.outbound_preferences` — a different store backed by the
    /// same filesystem path but carrying independent in-memory state.  Any preference
    /// saved through the WebUI was therefore never seen by the delivery hook.
    ///
    /// This test uses `Arc::ptr_eq` to verify both sides hold the *same pointer*,
    /// which is the only invariant that guarantees a write on one side is immediately
    /// visible on the other without filesystem round-trips.  If
    /// `build_triggered_run_delivery_hook` is regressed to create a new store this
    /// assertion fails deterministically and immediately, without needing an E2E run
    /// that could be silenced by an unrelated `Skipped` outcome.
    #[tokio::test]
    async fn webui_saved_preference_is_visible_to_triggered_slack_delivery() {
        use ironclaw_outbound::TriggeredRunDeliveryStore;

        let (runtime, _tmp) = runtime_with_trigger_poller().await;

        let local_runtime = runtime
            .services()
            .local_runtime
            .as_ref()
            .expect("local-dev runtime has local_runtime services");

        // Build the delivery driver via the production entry point.
        // `build_triggered_run_delivery_hook` now returns the concrete
        // `Arc<TriggeredRunDeliveryDriver>` directly, so we can inspect
        // `communication_preferences_for_test` through the same code path
        // that the production call site uses.
        let delivery_store: Arc<dyn TriggeredRunDeliveryStore> =
            Arc::clone(&local_runtime.triggered_run_delivery);
        let driver = build_triggered_run_delivery_hook(&runtime, &config(), delivery_store)
            .expect("build_triggered_run_delivery_hook should succeed");

        // The pointer stored in the driver must be the same Arc that the WebUI
        // delivery-defaults facade uses.  Arc::ptr_eq compares allocation identity
        // (for trait objects, data and vtable pointers); it passes only when both
        // handles came from the same composition-owned store instance.  If
        // `build_triggered_run_delivery_hook` is regressed to
        // `Arc::new(FilesystemOutboundStateStore::new(...))`, the new allocation
        // will produce a different pointer pair and this assertion fails.
        let driver_store = driver.communication_preferences_for_test();
        let facade_store: Arc<dyn ironclaw_outbound::CommunicationPreferenceRepository> =
            Arc::clone(&local_runtime.outbound_preferences);
        assert!(
            Arc::ptr_eq(&driver_store, &facade_store),
            "build_triggered_run_delivery_hook (production entry point) wired a DIFFERENT \
             CommunicationPreferenceRepository than local_runtime.outbound_preferences — any \
             preference written through the WebUI delivery-defaults facade \
             (RebornOutboundPreferencesFacade) will NOT be visible to the Slack \
             triggered-delivery hook; the hook must use \
             Arc::clone(&local_runtime.outbound_preferences) as `communication_preferences`"
        );

        runtime.shutdown().await.expect("runtime shuts down");
    }
}
