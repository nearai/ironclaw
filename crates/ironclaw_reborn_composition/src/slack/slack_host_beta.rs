//! Host-beta Slack Events API composition.
//!
//! This module is the single composition point for the native Slack route:
//! the CLI supplies explicit host config, and this module reuses the already
//! assembled Reborn runtime services instead of creating a second agent loop.

// arch-exempt: large_file, Slack host composition and lifecycle wiring tests, plan #5905

use std::collections::HashSet;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;

use ironclaw_conversations::InMemoryConversationServices;
#[cfg(feature = "test-support")]
use ironclaw_filesystem::{InMemoryBackend, ScopedFilesystem};
use ironclaw_host_api::{AgentId, ProjectId, ResourceScope, TenantId, UserId};
#[cfg(feature = "test-support")]
use ironclaw_host_api::{MountAlias, MountGrant, MountPermissions, MountView, VirtualPath};
use ironclaw_outbound::{DeliveredGateRouteStore, OutboundStateStore, TriggeredRunDeliveryStore};
use ironclaw_product_adapters::{
    AdapterInstallationId, DeclaredEgressHost, DeclaredEgressTarget, DeliveryStatus,
    EgressCredentialHandle, OutboundDeliverySink, ProductAdapter, ProductAdapterId,
    ProtocolHttpEgress,
};
use ironclaw_product_workflow::RebornFilesystemIdempotencyLedger;
use ironclaw_product_workflow::{
    ApprovalInteractionService, AuthInteractionService, ConversationBindingService,
    DefaultInboundTurnService, DefaultProductWorkflow, ProductActorUserResolutionRequest,
    ProductActorUserResolver, ProductConversationBindingService, ProductConversationRouteKey,
    ProductConversationSubjectRouteResolver, ProductInstallationKey, ProductInstallationScope,
    ProductWorkflowError, ResolveBindingRequest, ResolvedBinding, ResolvedProductActorUser,
    StaticProductInstallationResolver,
};
use ironclaw_slack_v2_adapter::{
    SLACK_API_HOST, SLACK_V2_ADAPTER_ID, SlackV2Adapter, SlackV2AdapterConfig,
    slack_request_signature_auth_requirement,
};
use ironclaw_threads::SessionThreadService;
use ironclaw_turns::TurnCoordinator;
use ironclaw_wasm_product_adapters::{
    EgressPolicy, HmacWebhookAuth, NativeProductAdapterRunner, NativeProductAdapterRunnerConfig,
    WebhookAuth,
};
use secrecy::{ExposeSecret, SecretString};
use sha2::{Digest, Sha256};
use thiserror::Error;

mod runtime_setup;

use crate::RebornRuntime;
use crate::outbound::{OutboundDeliveryTargetProvider, OutboundDeliveryTargetRegistrationOutcome};
use crate::product_auth::serve::SlackPersonalOAuthBindingConfig;
use crate::slack::slack_actor_identity::SlackUserIdentityActorResolver;
use crate::slack::slack_channel_routes::{
    SlackChannelRouteAdminRouteConfig, SlackChannelRouteStore, SlackChannelRouteSubjectResolver,
};
use crate::slack::slack_egress::{SlackProtocolHttpEgress, StaticSlackEgressCredentialProvider};
use crate::slack::slack_host_state::FilesystemSlackHostState;
use crate::slack::slack_outbound_targets::{
    SlackConfiguredChannelRoute, SlackHostBetaOutboundTargetProvider,
    SlackOutboundTargetProviderConfig, SlackPersonalDmTarget, SlackPersonalDmTargetError,
    SlackPersonalDmTargetProvisioner, SlackPersonalDmTargetStore,
};
use crate::slack::slack_personal_binding::{
    RebornUserIdentityBindingDeleteStore, RebornUserIdentityBindingStore, SlackConnectionEpoch,
    SlackPersonalBindingInstallation, SlackPersonalBindingPrincipal, SlackPersonalUserBinder,
    SlackPersonalUserBindingError, SlackPersonalUserBindingOutcome,
    SlackPersonalUserBindingRequest, SlackPersonalUserBindingService,
    SlackUserBindingLifecycleStore,
};
use crate::slack::slack_serve::{
    SlackEventsRouteState, SlackInstallationRecord, SlackInstallationResolver,
    SlackInstallationSelector, SlackTeamId, SlackUserId, StaticSlackInstallationResolver,
    slack_events_route_mount,
};
use crate::webui::route_mounts::PublicRouteMount;
use ironclaw_channel_delivery::{
    FinalReplyDeliveryObserver, FinalReplyDeliveryServices, FinalReplyDeliverySettings,
    TriggeredRunDeliveryDriver,
};
use ironclaw_channel_host::identity::RebornUserIdentityLookup;

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
    /// Host/runtime user used for Slack host-mediated state and
    /// backward-compatible shared-route fallback when `shared_subject_user_id`
    /// is not configured.
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
    pub user_id: UserId,
    pub shared_subject_user_id: Option<UserId>,
    pub channel_routes: Vec<SlackHostBetaChannelRoute>,
    pub signing_secret: SecretString,
    pub bot_token: SecretString,
}

#[derive(Debug, Clone)]
pub struct SlackHostBetaRuntimeConfig {
    pub tenant_id: TenantId,
    pub agent_id: AgentId,
    pub project_id: Option<ProjectId>,
    pub operator_user_id: UserId,
    pub legacy_setup: Option<SlackHostBetaLegacySetup>,
}

#[derive(Debug, Clone)]
pub struct SlackHostBetaLegacySetup {
    pub installation_id: String,
    pub team_id: String,
    pub api_app_id: String,
    pub user_id: UserId,
    pub shared_subject_user_id: Option<UserId>,
    pub channel_routes: Vec<SlackHostBetaChannelRoute>,
}

impl SlackHostBetaRuntimeConfig {
    pub fn new(
        tenant_id: TenantId,
        agent_id: AgentId,
        project_id: Option<ProjectId>,
        operator_user_id: UserId,
    ) -> Self {
        Self {
            tenant_id,
            agent_id,
            project_id,
            operator_user_id,
            legacy_setup: None,
        }
    }

    pub fn with_legacy_setup(mut self, legacy_setup: SlackHostBetaLegacySetup) -> Self {
        self.legacy_setup = Some(legacy_setup);
        self
    }
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
        Ok(Self {
            tenant_id: input.tenant_id,
            agent_id: input.agent_id,
            project_id: input.project_id,
            installation_id,
            team_id,
            installation_selector,
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
        #[allow(clippy::let_underscore_must_use)] // writing to a String is infallible
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
        SlackInstallationSelector::AppEnterpriseTeam {
            api_app_id,
            enterprise_id,
            team_id,
        } => {
            hash_slack_mount_field(hasher, "app_enterprise_team");
            hash_slack_mount_field(hasher, api_app_id.as_str());
            hash_slack_mount_field(hasher, enterprise_id.as_str());
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
        SlackInstallationSelector::AppEnterpriseInstallUser {
            api_app_id,
            enterprise_id,
            team_id,
            install_user_id,
        } => {
            hash_slack_mount_field(hasher, "app_enterprise_install_user");
            hash_slack_mount_field(hasher, api_app_id.as_str());
            hash_slack_mount_field(hasher, enterprise_id.as_str());
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
    #[error("Slack host-beta conversation store unavailable: {reason}")]
    ConversationStoreUnavailable { reason: String },
    #[error("Slack host-beta personal OAuth binding requires [slack].api_app_id")]
    TenantAppSelectorRequired,
    #[error("invalid Slack host-beta config field {field}: {reason}")]
    InvalidConfig { field: &'static str, reason: String },
}

#[non_exhaustive]
pub struct SlackHostBetaMounts {
    pub events: PublicRouteMount,
    pub channel_routes: SlackChannelRouteAdminRouteConfig,
    pub(crate) tenant_id: TenantId,
    pub(crate) personal_connection_scope: Option<SlackPersonalConnectionScope>,
    pub(crate) personal_connection_scope_resolver: Arc<dyn SlackPersonalConnectionScopeResolver>,
    pub(crate) personal_oauth_binder: Arc<dyn SlackPersonalUserBinder>,
    /// Reverse identity lookup: tells whether the calling WebUI user has
    /// personally connected this channel through Slack personal OAuth.
    pub(crate) user_identity_lookup: Arc<dyn RebornUserIdentityLookup>,
    /// Personal identity delete handle used when a caller uninstalls the
    /// Slack channel extension from the WebUI.
    pub(crate) user_identity_delete_store: Arc<dyn RebornUserIdentityBindingDeleteStore>,
    pub(crate) user_binding_lifecycle_store: Arc<dyn SlackUserBindingLifecycleStore>,
    /// Actor-pairing handle for revoking personal Slack DM conversation state
    /// when a caller removes the Slack channel extension.
    pub(crate) conversation_actor_pairings:
        Arc<dyn ironclaw_conversations::ConversationActorPairingService>,
    /// Personal Slack DM target store used by the same disconnect path, so
    /// outbound targets no longer show a stale Slack DM after uninstall.
    pub(crate) personal_dm_target_store: Arc<dyn SlackPersonalDmTargetStore>,
    /// Internal target-authority handle consumed only by WebUI product-facade composition.
    pub(crate) outbound_delivery_target_provider: Arc<dyn OutboundDeliveryTargetProvider>,
    pub(crate) outbound_delivery_target_provider_registered: bool,
    setup_service: Option<Arc<crate::slack::slack_setup::SlackSetupService>>,
}

impl SlackHostBetaMounts {
    /// Fill a lazy OAuth slot with this mount's setup service so OAuth providers
    /// can lazily resolve credentials at request time.
    pub fn fill_slack_personal_oauth_slot(
        &self,
        slot: &crate::slack::slack_setup::SlackPersonalSetupServiceSlot,
    ) {
        if let Some(service) = &self.setup_service {
            slot.fill(Arc::clone(service));
            slot.fill_gate_lifecycle(
                crate::slack::slack_personal_oauth::SlackPersonalOAuthGateLifecycle::new(
                    Arc::clone(&self.personal_connection_scope_resolver),
                    Arc::clone(&self.user_binding_lifecycle_store),
                ),
            );
        }
    }
}

#[cfg(feature = "test-support")]
impl SlackPersonalOAuthBindingConfig {
    /// Build the same binding/lifecycle ports used by production around an
    /// in-memory host-state store for caller-level OAuth route tests.
    pub fn in_memory_for_tests(
        tenant_id: TenantId,
        installation_id: AdapterInstallationId,
    ) -> Result<Self, String> {
        let view = MountView::new(vec![MountGrant::new(
            MountAlias::new("/tenant-shared").map_err(|error| error.to_string())?,
            VirtualPath::new("/tenants/test/shared").map_err(|error| error.to_string())?,
            MountPermissions::read_write_list_delete(),
        )])
        .map_err(|error| error.to_string())?;
        let filesystem = Arc::new(ScopedFilesystem::with_fixed_view(
            Arc::new(InMemoryBackend::default()),
            view,
        ));
        let state = Arc::new(FilesystemSlackHostState::new(
            filesystem,
            tenant_id.clone(),
            UserId::new("user:test-host").map_err(|error| error.to_string())?,
            AgentId::new("agent:test-host").map_err(|error| error.to_string())?,
            None,
        ));
        let binding_store: Arc<dyn RebornUserIdentityBindingStore> = state.clone();
        let binding_service: Arc<dyn SlackPersonalUserBinder> =
            Arc::new(SlackPersonalUserBindingService::new(
                [SlackPersonalBindingInstallation {
                    tenant_id,
                    installation_id: installation_id.clone(),
                    selector: SlackInstallationSelector::app_team("A-test", "T-test"),
                }],
                binding_store,
            ));
        let connection_scope = SlackPersonalConnectionScope { installation_id };

        Ok(Self::new(
            binding_service,
            Arc::new(StaticSlackPersonalConnectionScopeResolver::new(Some(
                connection_scope,
            ))),
            state.clone(),
            state,
        ))
    }
}

impl SlackHostBetaMounts {
    pub fn personal_oauth_binding_config(&self) -> SlackPersonalOAuthBindingConfig {
        SlackPersonalOAuthBindingConfig::new(
            Arc::clone(&self.personal_oauth_binder),
            Arc::clone(&self.personal_connection_scope_resolver),
            Arc::clone(&self.user_identity_delete_store),
            Arc::clone(&self.user_binding_lifecycle_store),
        )
    }
}

#[derive(Clone)]
pub(crate) struct SlackPersonalConnectionScope {
    pub(crate) installation_id: AdapterInstallationId,
}

#[async_trait::async_trait]
pub(crate) trait SlackPersonalConnectionScopeResolver: Send + Sync {
    async fn resolve_personal_connection_scope(
        &self,
    ) -> Result<Option<SlackPersonalConnectionScope>, String>;
}

pub(crate) struct StaticSlackPersonalConnectionScopeResolver {
    scope: Option<SlackPersonalConnectionScope>,
}

impl StaticSlackPersonalConnectionScopeResolver {
    pub(crate) fn new(scope: Option<SlackPersonalConnectionScope>) -> Self {
        Self { scope }
    }
}

#[async_trait::async_trait]
impl SlackPersonalConnectionScopeResolver for StaticSlackPersonalConnectionScopeResolver {
    async fn resolve_personal_connection_scope(
        &self,
    ) -> Result<Option<SlackPersonalConnectionScope>, String> {
        Ok(self.scope.clone())
    }
}

#[async_trait::async_trait]
pub(super) trait SlackPersonalDmTargetProvisioning: Send + Sync + std::fmt::Debug {
    async fn provision_for_user_for_epoch(
        &self,
        user_id: UserId,
        slack_user_id: SlackUserId,
        epoch: SlackConnectionEpoch,
    ) -> Result<SlackPersonalDmTarget, SlackPersonalDmTargetError>;
}

#[async_trait::async_trait]
impl SlackPersonalDmTargetProvisioning for SlackPersonalDmTargetProvisioner {
    async fn provision_for_user_for_epoch(
        &self,
        user_id: UserId,
        slack_user_id: SlackUserId,
        epoch: SlackConnectionEpoch,
    ) -> Result<SlackPersonalDmTarget, SlackPersonalDmTargetError> {
        SlackPersonalDmTargetProvisioner::provision_for_user_for_epoch(
            self,
            user_id,
            slack_user_id,
            epoch,
        )
        .await
    }
}

#[derive(Clone)]
pub(super) struct ProvisioningSlackPersonalUserBinder {
    binding_service: Arc<dyn SlackPersonalUserBinder>,
    dm_provisioner: Arc<dyn SlackPersonalDmTargetProvisioning>,
}

impl ProvisioningSlackPersonalUserBinder {
    pub(super) fn new(
        binding_service: Arc<dyn SlackPersonalUserBinder>,
        dm_provisioner: Arc<dyn SlackPersonalDmTargetProvisioning>,
    ) -> Self {
        Self {
            binding_service,
            dm_provisioner,
        }
    }

    fn spawn_dm_provisioning(
        &self,
        user_id: UserId,
        slack_user_id: SlackUserId,
        epoch: SlackConnectionEpoch,
    ) {
        let provisioner = Arc::clone(&self.dm_provisioner);
        tokio::spawn(async move {
            let result = provisioner
                .provision_for_user_for_epoch(user_id, slack_user_id, epoch)
                .await;
            match result {
                Ok(_) => {
                    tracing::debug!("Slack personal DM target provisioned after OAuth binding");
                }
                Err(error) => {
                    tracing::warn!(
                        %error,
                        "Slack personal DM target provisioning failed after OAuth binding; \
                         will retry on the next OAuth connection"
                    );
                }
            }
        });
    }
}

impl std::fmt::Debug for ProvisioningSlackPersonalUserBinder {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProvisioningSlackPersonalUserBinder")
            .field("binding_service", &self.binding_service)
            .field("dm_provisioner", &self.dm_provisioner)
            .finish()
    }
}

#[async_trait::async_trait]
impl SlackPersonalUserBinder for ProvisioningSlackPersonalUserBinder {
    async fn bind_personal_user_for_epoch(
        &self,
        principal: SlackPersonalBindingPrincipal,
        request: SlackPersonalUserBindingRequest,
        epoch: SlackConnectionEpoch,
    ) -> Result<SlackPersonalUserBindingOutcome, SlackPersonalUserBindingError> {
        let slack_user_id = request.slack_user_id.clone();
        let outcome = self
            .binding_service
            .bind_personal_user_for_epoch(principal, request, epoch)
            .await?;
        self.spawn_dm_provisioning(outcome.binding.user_id.clone(), slack_user_id, epoch);
        Ok(outcome)
    }
}

#[derive(Clone)]
struct SlackHostBetaRuntimeParts {
    local_runtime: Arc<crate::factory::RebornRuntimeSubstrate>,
    thread_service: Arc<dyn SessionThreadService>,
    turn_coordinator: Arc<dyn TurnCoordinator>,
    approval_interaction_service: Arc<dyn ApprovalInteractionService>,
    auth_interaction_service: Arc<dyn AuthInteractionService>,
    auth_challenge_provider: Option<Arc<dyn crate::AuthChallengeProvider>>,
    auth_flow_canceller: Option<Arc<dyn crate::BlockedAuthFlowCanceller>>,
}

impl SlackHostBetaRuntimeParts {
    fn from_runtime(runtime: &RebornRuntime) -> Result<Self, SlackHostBetaBuildError> {
        let local_runtime = runtime
            .services()
            .local_runtime
            .as_ref()
            .ok_or(SlackHostBetaBuildError::DurableHostStateUnavailable)?;
        let approval_interaction_service: Arc<dyn ApprovalInteractionService> = Arc::new(
            crate::delivered_gate_routing::DeliveredGateRoutingApprovalService::new(
                runtime.webui_approval_interaction_service(),
                Arc::clone(&local_runtime.delivered_gate_routes),
            ),
        );
        Ok(Self {
            local_runtime: Arc::clone(local_runtime),
            thread_service: runtime.webui_thread_service(),
            turn_coordinator: runtime.webui_turn_coordinator(),
            approval_interaction_service,
            auth_interaction_service: runtime.webui_auth_interaction_service(),
            auth_challenge_provider: runtime.auth_challenge_provider(),
            auth_flow_canceller: runtime.blocked_auth_flow_canceller(),
        })
    }
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
    let parts = SlackHostBetaRuntimeParts::from_runtime(runtime)?;
    build_triggered_run_delivery_hook_from_parts(&parts, config, delivery_store)
}

fn build_triggered_run_delivery_hook_from_parts(
    parts: &SlackHostBetaRuntimeParts,
    config: &SlackHostBetaConfig,
    delivery_store: Arc<dyn TriggeredRunDeliveryStore>,
) -> Result<Arc<TriggeredRunDeliveryDriver>, SlackHostBetaBuildError> {
    let token_handle = slack_bot_token_handle()?;
    let adapter_id = ProductAdapterId::new(SLACK_V2_ADAPTER_ID)
        .map_err(|reason| invalid_config("adapter_id", reason.to_string()))?;
    let adapter: Arc<dyn ProductAdapter> = Arc::new(SlackV2Adapter::new(SlackV2AdapterConfig {
        adapter_id,
        installation_id: config.installation_id.clone(),
        egress_credential_handle: token_handle.clone(),
        auth_requirement: slack_request_signature_auth_requirement(),
    }));
    let egress = slack_protocol_egress_from_parts(parts, config, token_handle)?;
    let outbound_store: Arc<dyn OutboundStateStore> =
        Arc::clone(&parts.local_runtime.outbound_state);
    let route_store: Arc<dyn DeliveredGateRouteStore> =
        Arc::clone(&parts.local_runtime.delivered_gate_routes);
    let preferences: Arc<dyn ironclaw_outbound::CommunicationPreferenceRepository> =
        Arc::clone(&parts.local_runtime.outbound_preferences);
    let delivery_sink: Arc<dyn OutboundDeliverySink> = Arc::new(NoopSlackDeliverySink);
    let binding_service: Arc<dyn ConversationBindingService> =
        Arc::new(NoopConversationBindingService);
    let services = FinalReplyDeliveryServices {
        channel_protocol: Arc::new(crate::slack::slack_delivery::SlackDeliveryProtocol),
        binding_service,
        thread_service: Arc::clone(&parts.thread_service),
        turn_coordinator: Arc::clone(&parts.turn_coordinator),
        outbound_store,
        route_store: Arc::clone(&route_store),
        communication_preferences: preferences,
        adapter,
        egress,
        delivery_sink,
        auth_challenges: parts.auth_challenge_provider.clone(),
        auth_flow_canceller: parts.auth_flow_canceller.clone(),
        approval_requests: Some(Arc::clone(&parts.local_runtime.approval_requests)
            as Arc<dyn ironclaw_run_state::ApprovalRequestStore>),
    };
    // Per-trigger delivery target resolution runs over the same durable Slack
    // host state (channel routes + personal DM targets) the outbound target
    // surface publishes ids from, so an id selected at trigger_create time
    // resolves at fire time or fails closed.
    let host_state = Arc::new(FilesystemSlackHostState::new(
        Arc::clone(&parts.local_runtime.host_state_filesystem),
        config.tenant_id.clone(),
        config.user_id.clone(),
        config.agent_id.clone(),
        config.project_id.clone(),
    ));
    let outbound_target_provider: Arc<dyn OutboundDeliveryTargetProvider> =
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
            host_state.clone(),
            host_state,
        ));
    // Pass config.agent_id as the fallback so the ThreadScope key matches the
    // value ConversationContentRefMaterializer uses (same runtime default_agent_id).
    let driver = TriggeredRunDeliveryDriver::new(
        services,
        delivery_store,
        route_store,
        config.agent_id.clone(),
    )
    .with_outbound_target_provider(outbound_target_provider);
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
    let user_identity_lookup: Arc<dyn RebornUserIdentityLookup> = state.clone();
    let user_identity_delete_store: Arc<dyn RebornUserIdentityBindingDeleteStore> = state.clone();
    let user_binding_lifecycle_store: Arc<dyn SlackUserBindingLifecycleStore> = state.clone();
    let binding_service: Arc<dyn SlackPersonalUserBinder> =
        Arc::new(SlackPersonalUserBindingService::new(
            [SlackPersonalBindingInstallation {
                tenant_id: config.tenant_id.clone(),
                installation_id: config.installation_id.clone(),
                selector: config.installation_selector.clone(),
            }],
            binding_store,
        ));
    let token_handle = slack_bot_token_handle()?;
    let dm_provisioner = Arc::new(SlackPersonalDmTargetProvisioner::new(
        config.tenant_id.clone(),
        config.installation_id.clone(),
        config.team_id.clone(),
        slack_protocol_egress(runtime, &config, token_handle.clone())?,
        token_handle,
        state.clone(),
    ));
    let personal_oauth_binder: Arc<dyn SlackPersonalUserBinder> = Arc::new(
        ProvisioningSlackPersonalUserBinder::new(Arc::clone(&binding_service), dm_provisioner),
    );
    let actor_user_resolver = Arc::new(SlackHostBetaActorUserResolver::new(Arc::new(
        SlackUserIdentityActorResolver::new(state.clone()),
    )));
    let channel_route_store: Arc<dyn SlackChannelRouteStore> = state.clone();
    let personal_dm_target_store: Arc<dyn SlackPersonalDmTargetStore> = state.clone();
    let subject_route_resolver: Arc<dyn ProductConversationSubjectRouteResolver> =
        Arc::new(SlackChannelRouteSubjectResolver::new(
            config.tenant_id.clone(),
            config.installation_id.clone(),
            Arc::clone(&channel_route_store),
        ));
    let conversations = Arc::new(InMemoryConversationServices::default());
    let conversation_actor_pairings: Arc<
        dyn ironclaw_conversations::ConversationActorPairingService,
    > = conversations.clone();
    let parts = SlackHostBetaRuntimeParts::from_runtime(runtime)?;
    let record = build_slack_installation_record_with_resolvers(
        &parts,
        config.clone(),
        actor_user_resolver,
        Some(subject_route_resolver),
        SlackConversationServices::from_shared(conversations),
    )?;
    // Build the installation resolver once so the events route has a single
    // source of truth for the Slack signing identity.
    let resolver: Arc<dyn SlackInstallationResolver> =
        Arc::new(StaticSlackInstallationResolver::new([record]));
    let events =
        slack_events_route_mount(SlackEventsRouteState::from_resolver(Arc::clone(&resolver)));
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
        let personal_connection_scope = SlackPersonalConnectionScope {
            installation_id: config.installation_id.clone(),
        };
        return Ok(SlackHostBetaMounts {
            events,
            channel_routes,
            tenant_id: config.tenant_id.clone(),
            personal_connection_scope: Some(personal_connection_scope.clone()),
            personal_connection_scope_resolver: Arc::new(
                StaticSlackPersonalConnectionScopeResolver::new(Some(personal_connection_scope)),
            ),
            personal_oauth_binder,
            user_identity_lookup: user_identity_lookup.clone(),
            user_identity_delete_store: user_identity_delete_store.clone(),
            user_binding_lifecycle_store: user_binding_lifecycle_store.clone(),
            conversation_actor_pairings: conversation_actor_pairings.clone(),
            personal_dm_target_store: personal_dm_target_store.clone(),
            outbound_delivery_target_provider,
            outbound_delivery_target_provider_registered: true,
            setup_service: None,
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
    let personal_connection_scope = SlackPersonalConnectionScope {
        installation_id: config.installation_id.clone(),
    };
    Ok(SlackHostBetaMounts {
        events,
        channel_routes,
        tenant_id: config.tenant_id.clone(),
        personal_connection_scope: Some(personal_connection_scope.clone()),
        personal_connection_scope_resolver: Arc::new(
            StaticSlackPersonalConnectionScopeResolver::new(Some(personal_connection_scope)),
        ),
        personal_oauth_binder,
        user_identity_lookup: user_identity_lookup.clone(),
        user_identity_delete_store,
        user_binding_lifecycle_store,
        conversation_actor_pairings,
        personal_dm_target_store,
        outbound_delivery_target_provider,
        outbound_delivery_target_provider_registered: true,
        setup_service: None,
    })
}

pub async fn build_slack_host_beta_runtime_mounts(
    runtime: &RebornRuntime,
    config: SlackHostBetaRuntimeConfig,
) -> Result<SlackHostBetaMounts, SlackHostBetaBuildError> {
    runtime_setup::build_runtime_mounts(runtime, config).await
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
    let resolver = build_slack_installation_resolver_with_resolvers(
        runtime,
        config,
        actor_user_resolver,
        subject_route_resolver,
    )?;
    Ok(slack_events_route_mount(
        SlackEventsRouteState::from_resolver(resolver),
    ))
}

/// Build the static installation resolver used by the Slack Events route.
fn build_slack_installation_resolver_with_resolvers(
    runtime: &RebornRuntime,
    config: SlackHostBetaConfig,
    actor_user_resolver: Arc<dyn ProductActorUserResolver>,
    subject_route_resolver: Option<Arc<dyn ProductConversationSubjectRouteResolver>>,
) -> Result<Arc<StaticSlackInstallationResolver>, SlackHostBetaBuildError> {
    let parts = SlackHostBetaRuntimeParts::from_runtime(runtime)?;
    // Sync/test entrypoint: no async context to rehydrate a durable store, so
    // bindings are in-memory here. Production Slack traffic flows through the
    // async `runtime_setup` path, which supplies the durable store.
    let record = build_slack_installation_record_with_resolvers(
        &parts,
        config,
        actor_user_resolver,
        subject_route_resolver,
        SlackConversationServices::from_shared(Arc::new(InMemoryConversationServices::default())),
    )?;
    Ok(Arc::new(StaticSlackInstallationResolver::new([record])))
}

/// The conversation binding + actor-pairing services backing a Slack
/// installation. Both handles share one backing store: production (async)
/// wiring supplies a durable filesystem-backed store that survives process
/// restarts, while the sync/test path supplies an in-memory one.
pub(crate) struct SlackConversationServices {
    binding: Arc<dyn ironclaw_conversations::ConversationBindingService>,
    pairing: Arc<dyn ironclaw_conversations::ConversationActorPairingService>,
}

impl SlackConversationServices {
    pub(crate) fn from_shared<S>(services: Arc<S>) -> Self
    where
        S: ironclaw_conversations::ConversationBindingService
            + ironclaw_conversations::ConversationActorPairingService
            + 'static,
    {
        Self {
            binding: services.clone(),
            pairing: services,
        }
    }
}

fn build_slack_installation_record_with_resolvers(
    parts: &SlackHostBetaRuntimeParts,
    config: SlackHostBetaConfig,
    actor_user_resolver: Arc<dyn ProductActorUserResolver>,
    subject_route_resolver: Option<Arc<dyn ProductConversationSubjectRouteResolver>>,
    conversation_services: SlackConversationServices,
) -> Result<SlackInstallationRecord, SlackHostBetaBuildError> {
    // The resolver controls inbound Slack actor binding. `config.user_id`
    // scopes host-mediated Slack bot-token egress and shared-route fallback
    // mapping. Shared Slack channel execution is configured separately.
    let adapter_id = ProductAdapterId::new(SLACK_V2_ADAPTER_ID)
        .map_err(|reason| invalid_config("adapter_id", reason.to_string()))?;
    let token_handle = slack_bot_token_handle()?;
    let adapter: Arc<dyn ProductAdapter> = Arc::new(SlackV2Adapter::new(SlackV2AdapterConfig {
        adapter_id: adapter_id.clone(),
        installation_id: config.installation_id.clone(),
        egress_credential_handle: token_handle.clone(),
        auth_requirement: slack_request_signature_auth_requirement(),
    }));

    let SlackConversationServices {
        binding: conversation_port,
        pairing: actor_pairings,
    } = conversation_services;
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
        Arc::clone(&parts.thread_service),
        Arc::clone(&parts.turn_coordinator),
    ));
    let route_store: Arc<dyn DeliveredGateRouteStore> =
        Arc::clone(&parts.local_runtime.delivered_gate_routes);
    let workflow = Arc::new(
        DefaultProductWorkflow::new(
            inbound,
            Arc::new(
                RebornFilesystemIdempotencyLedger::new(
                    Arc::clone(&parts.local_runtime.host_state_filesystem),
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
        .with_approval_interaction_service(Arc::clone(&parts.approval_interaction_service))
        .with_auth_interaction_service(Arc::clone(&parts.auth_interaction_service))
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

    let egress = slack_protocol_egress_from_parts(parts, &config, token_handle)?;
    let outbound_store: Arc<dyn OutboundStateStore> =
        Arc::clone(&parts.local_runtime.outbound_state);
    let preferences: Arc<dyn ironclaw_outbound::CommunicationPreferenceRepository> =
        Arc::clone(&parts.local_runtime.outbound_preferences);
    let delivery_sink: Arc<dyn OutboundDeliverySink> = Arc::new(NoopSlackDeliverySink);
    let observer = Arc::new(FinalReplyDeliveryObserver::with_settings(
        FinalReplyDeliveryServices {
            channel_protocol: Arc::new(crate::slack::slack_delivery::SlackDeliveryProtocol),
            binding_service: Arc::new(binding),
            thread_service: Arc::clone(&parts.thread_service),
            turn_coordinator: Arc::clone(&parts.turn_coordinator),
            outbound_store,
            route_store,
            communication_preferences: preferences,
            adapter,
            egress,
            delivery_sink,
            auth_challenges: parts.auth_challenge_provider.clone(),
            auth_flow_canceller: parts.auth_flow_canceller.clone(),
            approval_requests: Some(Arc::clone(&parts.local_runtime.approval_requests)
                as Arc<dyn ironclaw_run_state::ApprovalRequestStore>),
        },
        FinalReplyDeliverySettings::default(),
    ));

    Ok(SlackInstallationRecord::new(
        config.tenant_id,
        config.installation_id,
        config.installation_selector,
        runner,
    )
    .with_workflow_observer(observer))
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
    let parts = SlackHostBetaRuntimeParts::from_runtime(runtime)?;
    slack_protocol_egress_from_parts(&parts, config, token_handle)
}

fn slack_protocol_egress_from_parts(
    parts: &SlackHostBetaRuntimeParts,
    config: &SlackHostBetaConfig,
    token_handle: EgressCredentialHandle,
) -> Result<Arc<dyn ProtocolHttpEgress>, SlackHostBetaBuildError> {
    let host_egress = parts
        .local_runtime
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
    cached_identity: Arc<dyn ProductActorUserResolver>,
}

impl SlackHostBetaActorUserResolver {
    fn new(cached_identity: Arc<dyn ProductActorUserResolver>) -> Self {
        Self { cached_identity }
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
    ) -> Result<Option<ResolvedProductActorUser>, ProductWorkflowError> {
        if let Some(resolved_actor) = self
            .cached_identity
            .resolve_product_actor_user(request.clone())
            .await?
        {
            return Ok(Some(resolved_actor));
        }
        Ok(None)
    }

    async fn resolved_product_actor_user_is_current(
        &self,
        request: &ProductActorUserResolutionRequest,
        expected: &ResolvedProductActorUser,
    ) -> Result<bool, ProductWorkflowError> {
        self.cached_identity
            .resolved_product_actor_user_is_current(request, expected)
            .await
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
    use hmac::{Hmac, KeyInit, Mac};
    use http_body_util::BodyExt;
    use ironclaw_authorization::GrantAuthorizer;
    use ironclaw_extensions::ExtensionRegistry;
    use ironclaw_filesystem::{DiskFilesystem, InMemoryBackend};
    use ironclaw_host_runtime::{
        CapabilitySurfaceVersion, HostRuntimeHttpEgressPort, HostRuntimeServices,
    };
    use ironclaw_loop_host::{
        HostManagedModelError, HostManagedModelGateway, HostManagedModelRequest,
        HostManagedModelResponse,
    };
    use ironclaw_network::{
        NetworkHttpEgress, NetworkHttpError, NetworkHttpRequest, NetworkHttpResponse, NetworkUsage,
    };
    use ironclaw_processes::{FilesystemProcessResultStore, FilesystemProcessStore};
    use ironclaw_product_workflow::{
        LifecyclePackageKind, LifecyclePackageRef, ProductActorUserResolutionRequest,
        ProductWorkflowError, RebornExtensionOnboardingState, RebornOutboundDeliveryTargetId,
        RebornOutboundDeliveryTargetStatus, RebornServicesErrorCode, RebornServicesErrorKind,
        RebornSetOutboundPreferencesRequest, WebUiAuthenticatedCaller,
    };
    use ironclaw_resources::InMemoryResourceGovernor;
    use ironclaw_secrets::InMemorySecretStore;
    use ironclaw_slack_v2_adapter::SLACK_USER_ACTOR_KIND;
    use ironclaw_threads::{ListThreadsForScopeRequest, ThreadHistoryRequest, ThreadScope};
    use ironclaw_triggers::TriggerRunHistoryStatus;
    use ironclaw_turns::{
        GetRunStateRequest, ReplyTargetBindingRef, TurnCoordinator, TurnRunId, TurnScope,
        TurnStatus, run_profile::LoopCapabilityPort,
    };
    use secrecy::ExposeSecret;
    use tower::ServiceExt;

    use super::*;
    use crate::slack::slack_channel_routes::{
        InMemorySlackChannelRouteStore, SlackChannelRoute, SlackChannelRouteAdminRouteMount,
        SlackChannelRouteError, SlackChannelRouteKey, SlackChannelRouteListPage,
        WEBUI_V2_CHANNELS_SLACK_ROUTES_PATH, slack_channel_route_admin_route_mount,
    };
    use crate::slack::slack_connectable_channel::{
        SlackOperatorRouteVisibility, build_webui_services_with_slack_host_beta_mounts,
    };
    use crate::slack::slack_outbound_targets::{
        InMemorySlackPersonalDmTargetStore, SLACK_OUTBOUND_TARGET_LIST_PAGE_SIZE,
        SlackPersonalDmTarget, SlackPersonalDmTargetError, SlackPersonalDmTargetKey,
        SlackPersonalDmTargetProvisioner, SlackPersonalDmTargetStore,
        slack_reply_target_binding_ref_from_raw, slack_shared_channel_reply_target_binding_ref,
    };
    use crate::slack::slack_serve::{SlackApiAppId, SlackUserId};
    use crate::{
        RebornBuildError, RebornBuildInput, RebornRuntimeIdentity, RebornRuntimeInput,
        SLACK_EVENTS_PATH, build_reborn_runtime, local_dev_runtime_policy,
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

    /// A persisted run id proves acceptance even if that run later failed;
    /// `Error` without a run id means submission failed before acceptance.
    fn accepted_trigger_run_id(
        run_id: Option<TurnRunId>,
        status: Option<TriggerRunHistoryStatus>,
    ) -> Option<TurnRunId> {
        match (run_id, status) {
            (Some(run_id), _) => Some(run_id),
            (None, Some(TriggerRunHistoryStatus::Error)) => {
                panic!("trigger run failed before acceptance")
            }
            (None, _) => None,
        }
    }

    #[test]
    fn accepted_trigger_run_id_classifies_persisted_states() {
        let run_id = TurnRunId::new();
        assert_eq!(
            accepted_trigger_run_id(Some(run_id), Some(TriggerRunHistoryStatus::Error)),
            Some(run_id)
        );
        assert_eq!(
            accepted_trigger_run_id(None, Some(TriggerRunHistoryStatus::Running)),
            None
        );
        assert_eq!(accepted_trigger_run_id(None, None), None);
    }

    #[test]
    #[should_panic(expected = "trigger run failed before acceptance")]
    fn accepted_trigger_run_id_rejects_pre_acceptance_error() {
        accepted_trigger_run_id(None, Some(TriggerRunHistoryStatus::Error));
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
            _assignments: Vec<crate::slack::slack_channel_routes::SlackChannelRouteAssignment>,
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
        // Inbound Slack actor resolution now requires a durable OAuth identity
        // binding; the removed static `slack_user_id` seed no longer maps the
        // Slack user to a Reborn user. Bind through the production OAuth path
        // before dispatching the signed event.
        let binder = build_slack_host_beta_mounts(&runtime, config()).expect("mounts");
        bind_slack_oauth_user(&binder).await;
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

        // Inbound Slack actor resolution now requires a durable OAuth identity
        // binding (the static `slack_user_id` seed was removed). The binding is
        // durable in the shared runtime state, so it survives the route rebuild
        // this dedup test exercises.
        let binder = build_slack_host_beta_mounts(&runtime, config()).expect("mounts");
        bind_slack_oauth_user(&binder).await;
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
    async fn build_slack_host_beta_mounts_exposes_events_and_oauth_binding_only() {
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
        let _oauth_binding = mounts.personal_oauth_binding_config();

        assert_eq!(mounts.events.descriptors.len(), 1);
        assert!(
            mounts
                .events
                .descriptors
                .iter()
                .any(|descriptor| descriptor.route_pattern().as_str() == SLACK_EVENTS_PATH),
            "static host-beta mounts should expose the Slack Events route"
        );

        runtime.shutdown().await.expect("runtime shuts down");
    }

    #[tokio::test]
    async fn build_slack_host_beta_runtime_mounts_exposes_events_and_oauth_binding_only() {
        let (runtime, _root) = runtime().await;

        let mounts = build_slack_host_beta_runtime_mounts(
            &runtime,
            dynamic_runtime_config_without_legacy_actor(),
        )
        .await
        .expect("dynamic mounts build");
        let _oauth_binding = mounts.personal_oauth_binding_config();

        assert_eq!(mounts.events.descriptors.len(), 1);
        assert!(
            mounts
                .events
                .descriptors
                .iter()
                .any(|descriptor| descriptor.route_pattern().as_str() == SLACK_EVENTS_PATH),
            "dynamic host-beta mounts should expose the Slack Events route"
        );

        runtime.shutdown().await.expect("runtime shuts down");
    }

    #[tokio::test]
    async fn build_slack_host_beta_mounts_routes_oauth_bound_dm_event() {
        let egress = Arc::new(RecordingRuntimeHttpEgress::default());
        let (runtime, _root) = runtime_with_host_egress_override(Some(Some(
            host_egress_port_for_test(Arc::clone(&egress)),
        )))
        .await;
        let mounts =
            build_slack_host_beta_mounts(&runtime, config_without_legacy_actor()).expect("mounts");

        bind_slack_oauth_user(&mounts).await;

        let second_body = dm_event_body_with(
            "Ev-host-beta-oauth-bound",
            "after oauth",
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
            .find(|message| message.content.as_deref() == Some("after oauth"))
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
        // The channel-mention actor is resolved through the durable OAuth
        // identity binding (the static `slack_user_id` seed was removed); bind
        // the Slack user before dispatching so the event resolves to a Reborn
        // user and the channel route maps it to the shared subject.
        bind_slack_oauth_user(&mounts).await;

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
        // Bind the inbound Slack actor through the OAuth path before the admin
        // route assignment; the removed static `slack_user_id` seed no longer
        // maps the Slack user, so an unbound actor's mention fails closed.
        bind_slack_oauth_user(&mounts).await;
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
                        operator_webui_config: true,
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
    async fn slack_tools_extension_lists_as_auth_required_until_personal_oauth_connected() {
        let (runtime, _root) = runtime().await;
        let mounts =
            build_slack_host_beta_mounts(&runtime, config_without_legacy_actor()).expect("mounts");
        let bundle = build_webui_services_with_slack_host_beta_mounts(
            &runtime,
            None,
            Some(&mounts),
            SlackOperatorRouteVisibility::Hidden,
        )
        .expect("webui bundle");
        let caller = operator_caller();
        // Model B: the user installs the tools extension (`slack`); the bot
        // channel (`slack_bot`) is hidden operator infrastructure. The tools
        // extension carries the slack_personal OAuth requirement, so — like any
        // OAuth extension — it lists as SetupRequired until the user connects.
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "slack")
            .expect("valid slack package ref");

        bundle
            .api
            .install_extension(caller.clone(), package_ref)
            .await
            .expect("install Slack tools extension");
        let response = bundle
            .api
            .list_extensions(caller)
            .await
            .expect("list extensions");
        let slack = response
            .extensions
            .iter()
            .find(|extension| extension.package_ref.id.as_str() == "slack")
            .expect("Slack tools extension is listed");

        assert_ne!(
            slack.kind, "channel",
            "the user-installable Slack extension is the tools package, not the bot channel"
        );
        assert_eq!(
            slack.onboarding_state,
            // The tools extension is credential-gated (an OAuth extension, not a
            // channel), so it reports AuthRequired — not the channel-specific
            // SetupRequired — until the slack_personal OAuth is connected.
            Some(RebornExtensionOnboardingState::AuthRequired)
        );
        assert!(!slack.authenticated);

        runtime.shutdown().await.expect("runtime shuts down");
    }

    #[tokio::test]
    async fn dynamic_slack_setup_save_succeeds_and_bot_channel_stays_hidden_from_catalog() {
        let (runtime, _root) = runtime().await;
        let mounts = build_slack_host_beta_runtime_mounts(
            &runtime,
            dynamic_runtime_config_without_legacy_actor(),
        )
        .await
        .expect("dynamic mounts");
        let bundle = build_webui_services_with_slack_host_beta_mounts(
            &runtime,
            None,
            Some(&mounts),
            SlackOperatorRouteVisibility::Visible,
        )
        .expect("webui bundle");
        let caller = operator_caller();
        // Model B: the Slack bot channel is operator-provisioned infrastructure,
        // configured through the operator setup route (not the user catalog). Even
        // when installed, it must never surface in the user-facing extension list —
        // only the tools extension (`slack`) does.
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "slack_bot")
            .expect("valid slack package ref");
        bundle
            .api
            .install_extension(caller.clone(), package_ref)
            .await
            .expect("install Slack bot channel");

        let route_mount = slack_channel_route_admin_route_mount(mounts.channel_routes);
        let response = route_mount
            .protected
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/webchat/v2/channels/slack/setup")
                    .header("content-type", "application/json")
                    .extension(caller.clone())
                    .body(Body::from(
                        r#"{
                            "installation_id":"install_dynamic",
                            "team_id":"T0DYNAMIC",
                            "api_app_id":"A0DYNAMIC",
                            "user_id":"user:slack-operator",
                            "bot_token":"xoxb-secret",
                            "signing_secret":"slack-signing-secret"
                        }"#,
                    ))
                    .expect("setup request builds"),
            )
            .await
            .expect("setup route responds");
        assert_eq!(response.status(), StatusCode::OK);

        let extensions = bundle
            .api
            .list_extensions(caller)
            .await
            .expect("list extensions after setup");
        assert!(
            extensions
                .extensions
                .iter()
                .all(|extension| extension.package_ref.id.as_str() != "slack_bot"),
            "the operator-provisioned Slack bot channel must stay hidden from the user extension catalog"
        );

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
            SlackTeamId::new(TEAM),
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
            SlackTeamId::new(TEAM),
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

    // ── provisioning-after-oauth: the production wiring ──────────────────────

    #[tokio::test]
    async fn oauth_binding_provisions_personal_dm_target_via_real_call_path() {
        // After Slack OAuth binding the provisioner must open the DM and
        // register the personal DM target so it appears in the delivery-target
        // list. This drives the production seam:
        // SlackPersonalOAuthBindingConfig → SlackPersonalUserBinder
        // → background provisioner → SlackPersonalDmTargetStore.
        let egress = Arc::new(RecordingRuntimeHttpEgress::default());
        let (runtime, _root) = runtime_with_host_egress_override(Some(Some(
            host_egress_port_for_test(Arc::clone(&egress)),
        )))
        .await;
        let mounts =
            build_slack_host_beta_mounts(&runtime, config_without_legacy_actor()).expect("mounts");

        bind_slack_oauth_user(&mounts).await;

        // Wait for the personal DM target to appear (the provisioner
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
            "personal DM target must appear after Slack OAuth binding"
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
    async fn dynamic_slack_setup_oauth_binding_registers_runtime_personal_dm_target() {
        // Regression guard for PR #5152's WebUI Slack setup path: dynamic setup
        // must register the same runtime outbound target provider and OAuth DM
        // provisioner that static Slack host-beta setup wires.
        let egress = Arc::new(RecordingRuntimeHttpEgress::default());
        let (runtime, _root) = runtime_with_host_egress_override(Some(Some(
            host_egress_port_for_test(Arc::clone(&egress)),
        )))
        .await;
        let mounts = build_slack_host_beta_runtime_mounts(
            &runtime,
            dynamic_runtime_config_without_legacy_actor(),
        )
        .await
        .expect("dynamic mounts");
        assert!(
            mounts.outbound_delivery_target_provider_registered,
            "dynamic Slack setup must register its target provider with the runtime"
        );

        // Dynamic Slack setup (bot token + signing secret) now arrives through
        // the WebUI save rather than static legacy seeding; provide it before
        // the OAuth binding so the spawned personal DM provisioner can resolve
        // the installation and register the target.
        mounts
            .setup_service
            .as_ref()
            .expect("dynamic mounts expose the Slack setup service")
            .save(crate::slack::slack_setup::SlackInstallationSetupUpdate {
                installation_id: INSTALLATION.to_string(),
                team_id: TEAM.to_string(),
                api_app_id: API_APP.to_string(),
                user_id: Some(USER.to_string()),
                shared_subject_user_id: None,
                bot_token: Some(SecretString::from("xoxb-host-token")),
                signing_secret: Some(SecretString::from(SECRET)),
                oauth_client_id: None,
                oauth_client_secret: None,
            })
            .await
            .expect("seed dynamic Slack setup");

        bind_slack_oauth_user(&mounts).await;

        let runtime_provider = runtime
            .outbound_delivery_target_provider()
            .expect("dynamic Slack setup registers runtime provider");
        let mut runtime_targets = Vec::new();
        for _ in 0..40 {
            runtime_targets = runtime_provider
                .list_outbound_delivery_targets(&operator_caller())
                .await
                .expect("runtime target list");
            if !runtime_targets.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        assert_eq!(
            runtime_targets.len(),
            1,
            "dynamic OAuth binding must provision a runtime-visible personal DM target"
        );
        assert_eq!(
            runtime_targets[0].summary.target_id.as_str(),
            "slack:personal-dm:T0HOST:user:slack-host"
        );

        let bundle = build_webui_services_with_slack_host_beta_mounts(
            &runtime,
            None,
            Some(&mounts),
            SlackOperatorRouteVisibility::Hidden,
        )
        .expect("webui bundle");
        let selected = bundle
            .api
            .set_outbound_preferences(
                operator_caller(),
                RebornSetOutboundPreferencesRequest {
                    final_reply_target_id: Some(runtime_targets[0].summary.target_id.clone()),
                },
            )
            .await
            .expect("set dynamic Slack personal DM target");
        assert_eq!(
            selected.final_reply_target_status,
            RebornOutboundDeliveryTargetStatus::Available
        );

        runtime.shutdown().await.expect("runtime shuts down");
    }

    #[tokio::test]
    async fn oauth_binding_is_idempotent_and_does_not_duplicate_dm_target() {
        // Re-provisioning must not create duplicate targets.
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
    async fn oauth_binding_succeeds_even_when_dm_provisioning_fails() {
        // Provisioning failure must be silent: OAuth binding itself must succeed.
        let egress = Arc::new(RecordingRuntimeHttpEgress::conversations_open_response(
            200,
            br#"{"ok":false,"error":"not_allowed"}"#,
        ));
        let (runtime, _root) = runtime_with_host_egress_override(Some(Some(
            host_egress_port_for_test(Arc::clone(&egress)),
        )))
        .await;
        let mounts =
            build_slack_host_beta_mounts(&runtime, config_without_legacy_actor()).expect("mounts");

        bind_slack_oauth_user(&mounts).await;

        // Wait for the provisioner's conversations.open attempt so we know the
        // background task ran and failed before asserting that no target was persisted.
        wait_for_nth_conversations_open(&egress, 1).await;
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
    async fn build_slack_host_beta_mounts_rejects_team_only_selector_for_oauth_binding() {
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
            user_id: UserId::new(USER).expect("user"),
            shared_subject_user_id: None,
            channel_routes: Vec::new(),
            signing_secret: SecretString::from(SECRET),
            bot_token: SecretString::from("xoxb-host-token"),
        })
        .expect("team-only config still parses");

        let error = match build_slack_host_beta_mounts(&runtime, team_only_config) {
            Ok(_) => panic!("OAuth binding requires tenant app selector"),
            Err(error) => error,
        };

        assert!(matches!(
            error,
            SlackHostBetaBuildError::TenantAppSelectorRequired
        ));
        runtime.shutdown().await.expect("runtime shuts down");
    }

    #[test]
    fn slack_host_beta_config_does_not_carry_static_slack_actor() {
        let config = config();

        assert_eq!(config.installation_id.as_str(), INSTALLATION);
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

    fn config() -> SlackHostBetaConfig {
        SlackHostBetaConfig::new(SlackHostBetaConfigInput {
            tenant_id: TenantId::new(TENANT).expect("tenant"),
            agent_id: AgentId::new(AGENT).expect("agent"),
            project_id: Some(ProjectId::new(PROJECT).expect("project")),
            installation_id: INSTALLATION.to_string(),
            team_id: SlackTeamId::new(TEAM),
            api_app_id: Some(API_APP.to_string()),
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
            user_id: UserId::new(USER).expect("user"),
            shared_subject_user_id: None,
            channel_routes: Vec::new(),
            signing_secret: SecretString::from(SECRET),
            bot_token: SecretString::from("xoxb-host-token"),
        })
        .expect("valid config")
    }

    fn dynamic_runtime_config_without_legacy_actor() -> SlackHostBetaRuntimeConfig {
        // Production resolves the Slack host-beta runtime config with
        // `legacy_setup: None` (serve_slack.rs asserts this). Slack secrets now
        // arrive only through the WebUI dynamic setup save; static legacy
        // seeding fails closed without a bot token by design. Mirror production:
        // build with no legacy setup and drive setup through the dynamic path.
        SlackHostBetaRuntimeConfig::new(
            TenantId::new(TENANT).expect("tenant"),
            AgentId::new(AGENT).expect("agent"),
            Some(ProjectId::new(PROJECT).expect("project")),
            UserId::new(USER).expect("user"),
        )
    }

    fn config_without_channel_routes() -> SlackHostBetaConfig {
        SlackHostBetaConfig::new(SlackHostBetaConfigInput {
            tenant_id: TenantId::new(TENANT).expect("tenant"),
            agent_id: AgentId::new(AGENT).expect("agent"),
            project_id: Some(ProjectId::new(PROJECT).expect("project")),
            installation_id: INSTALLATION.to_string(),
            team_id: SlackTeamId::new(TEAM),
            api_app_id: Some(API_APP.to_string()),
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
        // Slack admin routes now gate on the operator webui-config capability
        // (added in #5185 — only the admin webui-v2 token may mutate admin
        // routes). The test operator represents that authorized admin.
        WebUiAuthenticatedCaller::new(
            TenantId::new(TENANT).expect("tenant"),
            UserId::new(USER).expect("user"),
            Some(AgentId::new(AGENT).expect("agent")),
            Some(ProjectId::new(PROJECT).expect("project")),
        )
        .with_operator_webui_config(true)
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
        format!("v0={}", hex::encode(mac.finalize().into_bytes()))
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

    async fn bind_slack_oauth_user(mounts: &SlackHostBetaMounts) {
        let config = mounts.personal_oauth_binding_config();
        let epoch = SlackConnectionEpoch::new(ironclaw_auth::AuthFlowId::new());
        config
            .lifecycle_store
            .begin_connection(
                &crate::slack::slack_personal_binding::SlackConnectionOwner::new(
                    TenantId::new(TENANT).expect("tenant"),
                    UserId::new(USER).expect("user"),
                    AdapterInstallationId::new(INSTALLATION).expect("installation"),
                ),
                epoch,
                chrono::Utc::now() + chrono::Duration::minutes(5),
            )
            .await
            .expect("Slack OAuth lifecycle begins");
        config
            .binding_service
            .bind_personal_user_for_epoch(
                SlackPersonalBindingPrincipal {
                    tenant_id: TenantId::new(TENANT).expect("tenant"),
                    user_id: UserId::new(USER).expect("user"),
                },
                SlackPersonalUserBindingRequest {
                    installation_id: AdapterInstallationId::new(INSTALLATION)
                        .expect("installation"),
                    slack_user_id: SlackUserId::new(SLACK_USER),
                    team_id: SlackTeamId::new(TEAM),
                    enterprise_id: None,
                    api_app_id: SlackApiAppId::new(API_APP),
                },
                epoch,
            )
            .await
            .expect("Slack OAuth binding succeeds");
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
        ) -> Result<Option<ResolvedProductActorUser>, ProductWorkflowError> {
            self.calls
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(request);
            Ok(Some(ResolvedProductActorUser::new(self.user_id.clone())))
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
        DiskFilesystem,
        InMemoryResourceGovernor,
        FilesystemProcessStore<InMemoryBackend>,
        FilesystemProcessResultStore<InMemoryBackend>,
    > {
        HostRuntimeServices::new(
            Arc::new(ExtensionRegistry::new()),
            Arc::new(DiskFilesystem::new()),
            Arc::new(InMemoryResourceGovernor::new()),
            Arc::new(GrantAuthorizer::new()),
            ironclaw_processes::in_memory_backed_process_services(),
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
            _key: &crate::slack::slack_outbound_targets::SlackPersonalDmTargetKey,
        ) -> Result<
            Option<crate::slack::slack_outbound_targets::SlackPersonalDmTarget>,
            SlackPersonalDmTargetError,
        > {
            Err(SlackPersonalDmTargetError::StoreUnavailable)
        }

        async fn upsert_personal_dm_target_for_epoch(
            &self,
            _target: crate::slack::slack_outbound_targets::SlackPersonalDmTarget,
            _epoch: SlackConnectionEpoch,
        ) -> Result<
            crate::slack::slack_outbound_targets::SlackPersonalDmTarget,
            SlackPersonalDmTargetError,
        > {
            Err(SlackPersonalDmTargetError::StoreUnavailable)
        }

        async fn delete_personal_dm_targets_for_owner(
            &self,
            _tenant_id: &TenantId,
            _user_id: &UserId,
            _installation_id: &AdapterInstallationId,
            _expected_epoch: Option<SlackConnectionEpoch>,
        ) -> Result<usize, SlackPersonalDmTargetError> {
            Err(SlackPersonalDmTargetError::StoreUnavailable)
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

    const TRIGGER_HOOK_E2E_FIRE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
    const TRIGGER_HOOK_E2E_DELIVERY_TIMEOUT: std::time::Duration =
        std::time::Duration::from_secs(30);

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
                        .with_worker_config(
                            TriggerPollerWorkerConfig::default()
                                .set_poll_interval(std::time::Duration::from_millis(20)),
                        ),
                ),
        )
        .await
        .expect("runtime with trigger poller builds");
        (runtime, root)
    }

    #[tokio::test]
    async fn build_slack_host_beta_mounts_wires_trigger_delivery_hook_writes_record() {
        let (runtime, _tmp) = runtime_with_trigger_poller().await;

        // Wire the delivery hook by calling the production mount builder.
        let _mounts =
            build_slack_host_beta_mounts(&runtime, config()).expect("mounts should build");

        assert_due_personal_trigger_writes_delivery_record(&runtime, "hook-wiring-e2e").await;
        runtime.shutdown().await.expect("runtime shutdown");
    }

    #[tokio::test]
    async fn build_slack_host_beta_runtime_mounts_wires_dynamic_trigger_delivery_hook_writes_record()
     {
        let (runtime, _tmp) = runtime_with_trigger_poller().await;

        // Wire the delivery hook by calling the dynamic production mount builder
        // used by WebUI-managed Slack setup.
        let mounts = build_slack_host_beta_runtime_mounts(
            &runtime,
            dynamic_runtime_config_without_legacy_actor(),
        )
        .await
        .expect("dynamic mounts should build");

        // The dynamic delivery hook resolves its driver from the current Slack
        // setup (skips silently when unconfigured). Provide the WebUI-managed
        // setup so the hook builds a driver and records the (no-default-target)
        // delivery outcome the assertion below waits for.
        mounts
            .setup_service
            .as_ref()
            .expect("dynamic mounts expose the Slack setup service")
            .save(crate::slack::slack_setup::SlackInstallationSetupUpdate {
                installation_id: INSTALLATION.to_string(),
                team_id: TEAM.to_string(),
                api_app_id: API_APP.to_string(),
                user_id: Some(USER.to_string()),
                shared_subject_user_id: None,
                bot_token: Some(SecretString::from("xoxb-host-token")),
                signing_secret: Some(SecretString::from(SECRET)),
                oauth_client_id: None,
                oauth_client_secret: None,
            })
            .await
            .expect("seed dynamic Slack setup");

        assert_due_personal_trigger_writes_delivery_record(&runtime, "dynamic-hook-wiring-e2e")
            .await;
        runtime.shutdown().await.expect("runtime shutdown");
    }

    async fn assert_due_personal_trigger_writes_delivery_record(
        runtime: &RebornRuntime,
        trigger_name: &str,
    ) {
        use std::time::Instant;

        use chrono::Utc;
        use ironclaw_conversations::{AdapterInstallationId, AdapterKind, ExternalActorRef};
        use ironclaw_triggers::{
            TRIGGER_TRUSTED_ADAPTER_INSTALLATION_ID, TRIGGER_TRUSTED_ADAPTER_KIND,
            TRIGGER_TRUSTED_EXTERNAL_ACTOR_NAMESPACE, TriggerId, TriggerRecord, TriggerSchedule,
            TriggerSourceKind, TriggerState,
        };

        // Bind the trigger actor so the trusted submitter can resolve the
        // creator's user binding (fails closed for unbound actors by design).
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
            name: trigger_name.to_string(),
            source: TriggerSourceKind::Schedule,
            schedule: TriggerSchedule::cron("* * * * *").expect("valid cron"),
            prompt: format!("{trigger_name}-prompt-marker"),
            delivery_target: None,
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

        // Wait for the poller to persist an accepted run. `active_run_ref` is
        // cleared when a fast run completes, so read the durable run-history
        // row instead of racing that transient field.
        // Keep this budget generous: under `cargo llvm-cov --all-targets`, this
        // E2E runs alongside many instrumented async tests, so the background
        // trigger poller can be scheduled much later than in a focused test.
        let deadline = Instant::now() + TRIGGER_HOOK_E2E_FIRE_TIMEOUT;
        let mut fired_run_id = None;
        let mut last_run_history = None;
        while Instant::now() < deadline {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            let run_history = repo
                .list_trigger_run_history(tenant_id.clone(), trigger_id, 1)
                .await
                .expect("list trigger run history");
            last_run_history = Some(run_history);
            let latest_run = last_run_history.as_ref().and_then(|runs| runs.first());
            if let Some(run_id) = accepted_trigger_run_id(
                latest_run.and_then(|run| run.run_id),
                latest_run.map(|run| run.status),
            ) {
                fired_run_id = Some(run_id);
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

        // Poll for the delivery record.  The driver spawns a background task;
        // the `NoDefaultConfigured` fast-path normally completes well within
        // 2 s, but shared CI runners need the same load headroom as the fire
        // wait above.
        let mut delivery_record = None;
        if let Some(run_id) = fired_run_id {
            let delivery_deadline = Instant::now() + TRIGGER_HOOK_E2E_DELIVERY_TIMEOUT;
            while Instant::now() < delivery_deadline {
                if let Ok(Some(rec)) = delivery_store.load_triggered_run_delivery(run_id).await {
                    delivery_record = Some(rec);
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        }

        assert!(
            fired_run_id.is_some(),
            "trigger did not fire within {:?} — hook wiring e2e stalled; last_run_history={last_run_history:?}",
            TRIGGER_HOOK_E2E_FIRE_TIMEOUT
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
    /// `FinalReplyDeliveryServices.communication_preferences`.
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
