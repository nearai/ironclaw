//! Host-beta Telegram updates-route composition.
//!
//! Single composition point for the native Telegram channel host: the CLI
//! supplies explicit host config and this module assembles the durable setup /
//! pairing stores, the pairing-aware dynamic ingress resolver, the product
//! workflow (binding + idempotency + gates), the final-reply delivery
//! observer, and the WebUI channel-route/facade surfaces from the already
//! built Reborn runtime. Mirrors `slack_host_beta`'s runtime-setup path
//! (operator-managed setup, DM-only — no shared channel routes, no subject
//! route resolver, no outbound target provider).
//!
//! ## Boot-time installation scope (restart limitation)
//!
//! Inbound verification/parsing rebuilds per setup revision through
//! [`DynamicTelegramInstallationResolver`], but the product workflow's
//! installation scope and the delivery observer's rendering adapter are built
//! once from the setup read at mount-build time. Configuring the bot for the
//! first time — or swapping to a different bot — after boot therefore
//! requires a process restart before messages flow end to end (replies only
//! matter once the bot is configured and a user is paired, and a token
//! rotation for the SAME bot keeps its installation id, so the common rotate
//! path needs no restart).

use std::num::NonZeroUsize;
use std::sync::Arc;

use ironclaw_conversations::RebornFilesystemConversationServices;
use ironclaw_host_api::{AgentId, ProjectId, ResourceScope, TenantId, UserId};
use ironclaw_product_adapters::{
    AdapterInstallationId, AuthRequirement, DeclaredEgressTarget, DeliveryStatus,
    EgressCredentialHandle, OutboundDeliverySink, ProductAdapter, ProductAdapterId,
    ProductWorkflow, ProtocolHttpEgress,
};
use ironclaw_product_workflow::{
    ApprovalInteractionService, AuthInteractionService, ChannelConnectionFacade,
    ConnectableChannelsProductFacade, DefaultInboundTurnService, DefaultProductWorkflow,
    LifecyclePackageKind, LifecyclePackageRef, LifecyclePhase, ProductActorUserResolver,
    ProductConversationBindingService, ProductInstallationKey, ProductInstallationScope,
    RebornFilesystemIdempotencyLedger, StaticProductInstallationResolver,
};
use ironclaw_safety::{SafetyConfig, SafetyLayer};
use ironclaw_telegram_v2_adapter::{
    GroupTriggerPolicy, TelegramV2Adapter, TelegramV2AdapterConfig, telegram_declared_egress_hosts,
};
use ironclaw_threads::SessionThreadService;
use ironclaw_turns::TurnCoordinator;
use ironclaw_wasm_product_adapters::EgressPolicy;
use thiserror::Error;

use crate::RebornRuntime;
use crate::channel_identity::RebornUserIdentityLookup;
use crate::extension_host::extension_lifecycle::{
    ExtensionActivationMode, RebornLocalExtensionManagementPort,
};
// The Slack-named delivery observer is adapter-generic machinery (adapter,
// egress, and sink are injected); the Telegram host reuses it pending a
// vendor-neutral rename in the #6116 fold.
use crate::slack::slack_delivery::{
    SlackFinalReplyDeliveryObserver, SlackFinalReplyDeliveryServices,
    SlackFinalReplyDeliverySettings,
};
use crate::telegram::telegram_actor_identity::{
    TELEGRAM_V2_ADAPTER_ID, TelegramUserIdentityActorResolver,
};
use crate::telegram::telegram_bot_api::HostEgressTelegramBotApi;
use crate::telegram::telegram_channel_connection::TelegramPairedStatusSource;
use crate::telegram::telegram_channel_routes::{
    TelegramChannelRouteConfig, TelegramChannelSetupActivation,
    TelegramChannelSetupActivationError, telegram_channel_route_mount,
};
use crate::telegram::telegram_connectable_channel::{
    TelegramChannelConnectionFacade, TelegramConnectableChannelsProductFacade,
};
use crate::telegram::telegram_egress::{
    SetupServiceTelegramEgressCredentialProvider, TELEGRAM_BOT_TOKEN_CREDENTIAL_HANDLE,
    TelegramProtocolHttpEgress,
};
use crate::telegram::telegram_host_state::FilesystemTelegramHostState;
use crate::telegram::telegram_pairing::{
    TelegramDmTargetStore, TelegramPairingService, TelegramPairingStore, TelegramUserBindingStore,
};
use crate::telegram::telegram_serve::{
    DynamicTelegramInstallationResolver, TELEGRAM_SECRET_TOKEN_HEADER,
    TelegramInstallationResolver, TelegramUpdatesRouteState, telegram_updates_route_mount,
};
use crate::telegram::telegram_setup::{
    TelegramInstallationSetup, TelegramInstallationSetupStore, TelegramSetupService,
};
use crate::webui::webui_serve::{ProtectedRouteMount, PublicRouteMount};

const TELEGRAM_IDEMPOTENCY_LEDGER_SETTLED_LIMIT: usize = 10_000;
const TELEGRAM_IDEMPOTENCY_LEDGER_PRUNE_INTERVAL: usize = 1_000;
/// Placeholder installation id used to key the boot-time workflow scope and
/// observer adapter while no bot is configured yet. Real traffic never reaches
/// it: the dynamic resolver 401s every update until a setup record exists (see
/// the module doc's restart limitation for first-configure-after-boot).
const TELEGRAM_UNCONFIGURED_INSTALLATION_ID: &str = "tg-unconfigured";

/// Host config for the Telegram channel host, mirroring
/// [`crate::SlackHostBetaRuntimeConfig`]: identity scope only — bot identity
/// and secrets are operator-managed at runtime through the WebUI setup
/// surface.
#[derive(Debug, Clone)]
pub struct TelegramHostRuntimeConfig {
    pub tenant_id: TenantId,
    pub agent_id: AgentId,
    pub project_id: Option<ProjectId>,
    pub operator_user_id: UserId,
    /// Deployment public origin (`https://…`) used to derive the `setWebhook`
    /// registration URL. `None` means setup requires an explicit webhook URL
    /// override from the admin.
    pub public_base_url: Option<String>,
}

impl TelegramHostRuntimeConfig {
    pub fn new(
        tenant_id: TenantId,
        agent_id: AgentId,
        project_id: Option<ProjectId>,
        operator_user_id: UserId,
        public_base_url: Option<String>,
    ) -> Self {
        Self {
            tenant_id,
            agent_id,
            project_id,
            operator_user_id,
            public_base_url,
        }
    }
}

#[derive(Debug, Error)]
pub enum TelegramHostBuildError {
    #[error("Telegram host requires local runtime HTTP egress")]
    RuntimeHttpEgressUnavailable,
    #[error("Telegram host requires durable host state")]
    DurableHostStateUnavailable,
    #[error("Telegram host requires composed product-auth services")]
    ProductAuthUnavailable,
    #[error("Telegram host conversation store unavailable: {reason}")]
    ConversationStoreUnavailable { reason: String },
    #[error("Telegram host setup state unavailable: {reason}")]
    SetupStateUnavailable { reason: String },
    #[error("invalid Telegram host config field {field}: {reason}")]
    InvalidConfig { field: &'static str, reason: String },
}

/// The route mounts plus WebUI facades the `serve` layer wires from one
/// [`build_telegram_host_runtime_mounts`] call.
#[non_exhaustive]
pub struct TelegramHostMounts {
    /// Public Telegram updates webhook route
    /// (`/webhooks/extensions/telegram/updates`).
    pub events: PublicRouteMount,
    channel_routes: TelegramChannelRouteConfig,
    connectable: Arc<dyn ConnectableChannelsProductFacade>,
    channel_connection: Arc<dyn ChannelConnectionFacade>,
}

impl TelegramHostMounts {
    /// Bearer-authed WebUI channel routes (operator setup + per-member
    /// pairing), mounted through the generic [`ProtectedRouteMount`] seam.
    pub fn protected_routes(&self) -> ProtectedRouteMount {
        telegram_channel_route_mount(self.channel_routes.clone())
    }

    /// Settings connectable-channels descriptor facade for this host.
    pub(crate) fn connectable_channels(&self) -> Arc<dyn ConnectableChannelsProductFacade> {
        Arc::clone(&self.connectable)
    }

    /// Per-caller channel-connection (pairedness/disconnect) facade.
    pub(crate) fn channel_connection(&self) -> Arc<dyn ChannelConnectionFacade> {
        Arc::clone(&self.channel_connection)
    }
}

/// Runtime handles the Telegram host needs, resolved once. Field-for-field
/// mirror of `SlackHostBetaRuntimeParts::from_runtime` (kept module-private on
/// both sides; folds into a shared struct with the #6116 rename).
#[derive(Clone)]
struct TelegramHostRuntimeParts {
    local_runtime: Arc<crate::factory::RebornLocalRuntimeServices>,
    thread_service: Arc<dyn SessionThreadService>,
    turn_coordinator: Arc<dyn TurnCoordinator>,
    approval_interaction_service: Arc<dyn ApprovalInteractionService>,
    auth_interaction_service: Arc<dyn AuthInteractionService>,
    auth_challenge_provider: Option<Arc<dyn crate::AuthChallengeProvider>>,
    auth_flow_canceller: Option<Arc<dyn crate::BlockedAuthFlowCanceller>>,
}

impl TelegramHostRuntimeParts {
    fn from_runtime(runtime: &RebornRuntime) -> Result<Self, TelegramHostBuildError> {
        let local_runtime = runtime
            .services()
            .local_runtime
            .as_ref()
            .ok_or(TelegramHostBuildError::DurableHostStateUnavailable)?;
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

struct NoopTelegramDeliverySink;

#[async_trait::async_trait]
impl OutboundDeliverySink for NoopTelegramDeliverySink {
    async fn record(&self, _status: DeliveryStatus) {}
}

/// Post-save extension activation: mirror of the Slack
/// `DynamicSlackChannelSetupActivation` over the single `telegram` package.
struct DynamicTelegramChannelSetupActivation {
    extension_management: Arc<RebornLocalExtensionManagementPort>,
}

impl DynamicTelegramChannelSetupActivation {
    fn new(extension_management: Arc<RebornLocalExtensionManagementPort>) -> Self {
        Self {
            extension_management,
        }
    }
}

#[async_trait::async_trait]
impl TelegramChannelSetupActivation for DynamicTelegramChannelSetupActivation {
    async fn activate_telegram_channel_after_setup_save(
        &self,
    ) -> Result<(), TelegramChannelSetupActivationError> {
        let package_ref = LifecyclePackageRef::new(LifecyclePackageKind::Extension, "telegram")
            .map_err(telegram_setup_activation_error)?;
        // Telegram is a tenant-shared channel; host setup activates it as the
        // tenant operator so it operates the shared install (mirrors the Slack
        // host-beta activation, #5459 P1).
        let caller = self.extension_management.tenant_operator_user_id().clone();
        let projection = self
            .extension_management
            .project(package_ref.clone(), &caller)
            .await
            .map_err(telegram_setup_activation_error)?;
        if projection.phase == LifecyclePhase::Discovered {
            return Ok(());
        }
        self.extension_management
            .activate(package_ref, ExtensionActivationMode::Static, &caller)
            .await
            .map_err(telegram_setup_activation_error)?;
        Ok(())
    }
}

fn telegram_setup_activation_error(
    error: impl std::fmt::Display,
) -> TelegramChannelSetupActivationError {
    TelegramChannelSetupActivationError::new(error.to_string())
}

/// Assemble the Telegram host mounts from an already-built Reborn runtime.
///
/// Builds even when no bot is configured yet: the updates route 401s every
/// request until the admin saves a setup record, and the setup/pairing WebUI
/// routes are exactly how that record gets created. See the module doc for the
/// boot-time installation-scope restart limitation.
pub async fn build_telegram_host_runtime_mounts(
    runtime: &RebornRuntime,
    config: TelegramHostRuntimeConfig,
) -> Result<TelegramHostMounts, TelegramHostBuildError> {
    let parts = TelegramHostRuntimeParts::from_runtime(runtime)?;
    let host_state_filesystem = Arc::clone(&parts.local_runtime.telegram_host_state_filesystem);
    let state = Arc::new(FilesystemTelegramHostState::new(
        Arc::clone(&host_state_filesystem),
        config.tenant_id.clone(),
        config.operator_user_id.clone(),
        config.agent_id.clone(),
        config.project_id.clone(),
    ));
    let setup_store: Arc<dyn TelegramInstallationSetupStore> = state.clone();
    let pairing_store: Arc<dyn TelegramPairingStore> = state.clone();
    let binding_store: Arc<dyn TelegramUserBindingStore> = state.clone();
    let dm_target_store: Arc<dyn TelegramDmTargetStore> = state.clone();
    let identity_lookup: Arc<dyn RebornUserIdentityLookup> = state.clone();

    let host_egress = parts
        .local_runtime
        .host_runtime_http_egress
        .clone()
        .ok_or(TelegramHostBuildError::RuntimeHttpEgressUnavailable)?;
    let bot_api = HostEgressTelegramBotApi::arced(
        host_egress.clone(),
        telegram_egress_scope_template(&config),
    );
    let setup_service = Arc::new(TelegramSetupService::new(
        config.tenant_id.clone(),
        config.agent_id.clone(),
        config.project_id.clone(),
        config.operator_user_id.clone(),
        setup_store,
        runtime.services().secret_store(),
        bot_api,
        config.public_base_url.clone(),
    ));
    // Pairing completions resume blocked turns through the same composed
    // continuation dispatcher OAuth callbacks use.
    let continuation_dispatcher = runtime
        .services()
        .product_auth
        .as_ref()
        .ok_or(TelegramHostBuildError::ProductAuthUnavailable)?
        .continuation_dispatcher();
    let pairing = Arc::new(TelegramPairingService::new(
        config.tenant_id.clone(),
        config.agent_id.clone(),
        config.project_id.clone(),
        Arc::clone(&setup_service),
        pairing_store,
        binding_store,
        dm_target_store,
        continuation_dispatcher,
    ));

    // Durable, filesystem-backed conversation binding store so Telegram
    // conversation continuity survives a process restart. Backend
    // (libSQL / Postgres / local disk) is a property of the telegram
    // host-state root filesystem, shared with the idempotency ledger.
    let conversation_services = Arc::new(
        RebornFilesystemConversationServices::new(Arc::clone(&host_state_filesystem))
            .await
            .map_err(
                |error| TelegramHostBuildError::ConversationStoreUnavailable {
                    reason: error.to_string(),
                },
            )?,
    );
    let conversation_port: Arc<dyn ironclaw_conversations::ConversationBindingService> =
        conversation_services.clone();
    let actor_pairings: Arc<dyn ironclaw_conversations::ConversationActorPairingService> =
        conversation_services.clone();
    let actor_user_resolver: Arc<dyn ProductActorUserResolver> = Arc::new(
        TelegramUserIdentityActorResolver::new(Arc::clone(&identity_lookup)),
    );

    // Boot-time installation identity for the workflow scope and the delivery
    // observer's rendering adapter. When unconfigured, a placeholder id keys
    // the scope; the dynamic ingress resolver 401s until setup exists, so no
    // envelope can carry the placeholder.
    let current_setup = setup_service.current_setup().await.map_err(|error| {
        TelegramHostBuildError::SetupStateUnavailable {
            reason: error.to_string(),
        }
    })?;
    let installation_id = boot_installation_id(current_setup.as_ref())?;

    // DM-only scope: every conversation binds to the paired actor via the
    // resolver; no channel routes and no subject route resolver (unrouted
    // shared conversations fall back to the operator subject, which Telegram
    // group support would revisit).
    let scope = ProductInstallationScope::with_default_scope(
        config.tenant_id.clone(),
        config.agent_id.clone(),
        config.project_id.clone(),
    )
    .with_default_subject_user_id(config.operator_user_id.clone())
    .with_actor_user_resolver(actor_user_resolver, actor_pairings);
    let adapter_id = ProductAdapterId::new(TELEGRAM_V2_ADAPTER_ID)
        .map_err(|reason| invalid_config("adapter_id", reason.to_string()))?;
    let installation_resolver = StaticProductInstallationResolver::new([(
        ProductInstallationKey::new(adapter_id.clone(), installation_id.clone()),
        scope,
    )]);
    let binding = ProductConversationBindingService::new(conversation_port, installation_resolver);
    let inbound = Arc::new(DefaultInboundTurnService::new(
        binding.clone(),
        Arc::clone(&parts.thread_service),
        Arc::clone(&parts.turn_coordinator),
    ));
    let route_store = Arc::clone(&parts.local_runtime.delivered_gate_routes);
    let workflow = Arc::new(
        DefaultProductWorkflow::new(
            inbound,
            Arc::new(
                RebornFilesystemIdempotencyLedger::new(
                    Arc::clone(&host_state_filesystem),
                    telegram_egress_scope_template(&config),
                )
                .with_settled_entry_limit(
                    NonZeroUsize::new(TELEGRAM_IDEMPOTENCY_LEDGER_SETTLED_LIMIT).ok_or_else(
                        || invalid_config("settled_entry_limit", "must be non-zero".to_string()),
                    )?,
                )
                .with_settled_prune_interval(
                    NonZeroUsize::new(TELEGRAM_IDEMPOTENCY_LEDGER_PRUNE_INTERVAL).ok_or_else(
                        || invalid_config("settled_prune_interval", "must be non-zero".to_string()),
                    )?,
                ),
            ),
            Arc::new(binding.clone()),
        )
        .with_approval_interaction_service(Arc::clone(&parts.approval_interaction_service))
        .with_auth_interaction_service(Arc::clone(&parts.auth_interaction_service))
        .with_delivered_gate_routes(Arc::clone(&route_store)),
    );

    // Rendering adapter + egress for the final-reply delivery observer.
    let token_handle = telegram_bot_token_handle()?;
    let adapter: Arc<dyn ProductAdapter> =
        Arc::new(TelegramV2Adapter::new(TelegramV2AdapterConfig {
            adapter_id,
            installation_id,
            group_trigger_policy: GroupTriggerPolicy {
                bot_username: current_setup
                    .as_ref()
                    .map(|setup| setup.bot_username.clone())
                    .unwrap_or_default(),
                bot_user_id: current_setup
                    .as_ref()
                    .map(|setup| setup.bot_id)
                    .unwrap_or(0),
                recognized_commands: vec![],
            },
            egress_credential_handle: token_handle.clone(),
            auth_requirement: AuthRequirement::SharedSecretHeader {
                header_name: TELEGRAM_SECRET_TOKEN_HEADER.into(),
            },
            progress_push_enabled: false,
        }));
    let egress: Arc<dyn ProtocolHttpEgress> = Arc::new(TelegramProtocolHttpEgress::new(
        host_egress,
        Arc::new(SetupServiceTelegramEgressCredentialProvider::new(
            Arc::clone(&setup_service),
        )),
        EgressPolicy::new(telegram_declared_egress_targets(token_handle)),
        telegram_egress_scope_template(&config),
    ));
    // Generic final-reply delivery machinery reused from the Slack host (see
    // the import comment); adapter/egress/sink passed here are Telegram's.
    let observer = Arc::new(SlackFinalReplyDeliveryObserver::with_settings(
        SlackFinalReplyDeliveryServices {
            binding_service: Arc::new(binding),
            thread_service: Arc::clone(&parts.thread_service),
            turn_coordinator: Arc::clone(&parts.turn_coordinator),
            outbound_store: Arc::clone(&parts.local_runtime.outbound_state),
            route_store,
            communication_preferences: Arc::clone(&parts.local_runtime.outbound_preferences),
            adapter,
            egress,
            delivery_sink: Arc::new(NoopTelegramDeliverySink),
            auth_challenges: parts.auth_challenge_provider.clone(),
            auth_flow_canceller: parts.auth_flow_canceller.clone(),
            approval_requests: Some(Arc::clone(&parts.local_runtime.approval_requests)
                as Arc<dyn ironclaw_run_state::ApprovalRequestStore>),
        },
        SlackFinalReplyDeliverySettings::default(),
    ));

    let resolver: Arc<dyn TelegramInstallationResolver> =
        Arc::new(DynamicTelegramInstallationResolver::new(
            Arc::clone(&setup_service),
            Arc::clone(&pairing),
            Arc::clone(&identity_lookup),
            workflow as Arc<dyn ProductWorkflow>,
        ));
    let events = telegram_updates_route_mount(
        TelegramUpdatesRouteState::from_resolver(resolver).with_workflow_observer(observer),
    );

    let mut channel_routes = TelegramChannelRouteConfig::new(
        Arc::clone(&setup_service),
        Arc::clone(&pairing),
        // Mirrors the Slack channel-route admin config: a route-local safety
        // layer bounding/sanitizing operator-entered setup fields.
        Arc::new(SafetyLayer::new(&SafetyConfig {
            max_output_length: 16 * 1024,
            injection_check_enabled: true,
        })),
    );
    if let Some(extension_management) = &parts.local_runtime.extension_management {
        channel_routes = channel_routes.with_setup_activation(Arc::new(
            DynamicTelegramChannelSetupActivation::new(Arc::clone(extension_management)),
        ));
        // Fill the lifecycle port's pairedness slot so `telegram` extension
        // activation can gate on the caller's pairing state.
        extension_management
            .telegram_paired_status_slot()
            .fill(Arc::clone(&pairing) as Arc<dyn TelegramPairedStatusSource>);
    }

    let connectable: Arc<dyn ConnectableChannelsProductFacade> = Arc::new(
        TelegramConnectableChannelsProductFacade::new(Arc::clone(&setup_service), true),
    );
    let channel_connection: Arc<dyn ChannelConnectionFacade> = Arc::new(
        TelegramChannelConnectionFacade::new(Arc::clone(&pairing), setup_service),
    );

    Ok(TelegramHostMounts {
        events,
        channel_routes,
        connectable,
        channel_connection,
    })
}

fn boot_installation_id(
    current_setup: Option<&TelegramInstallationSetup>,
) -> Result<AdapterInstallationId, TelegramHostBuildError> {
    match current_setup {
        Some(setup) => setup
            .installation_id()
            .map_err(|reason| invalid_config("installation_id", reason.to_string())),
        None => AdapterInstallationId::new(TELEGRAM_UNCONFIGURED_INSTALLATION_ID)
            .map_err(|reason| invalid_config("installation_id", reason.to_string())),
    }
}

fn telegram_bot_token_handle() -> Result<EgressCredentialHandle, TelegramHostBuildError> {
    EgressCredentialHandle::new(TELEGRAM_BOT_TOKEN_CREDENTIAL_HANDLE)
        .map_err(|reason| invalid_config("bot_token_handle", reason.to_string()))
}

fn telegram_declared_egress_targets(
    token_handle: EgressCredentialHandle,
) -> Vec<DeclaredEgressTarget> {
    telegram_declared_egress_hosts()
        .into_iter()
        .map(|host| DeclaredEgressTarget::new(host, Some(token_handle.clone())))
        .collect()
}

fn telegram_egress_scope_template(config: &TelegramHostRuntimeConfig) -> ResourceScope {
    ResourceScope {
        tenant_id: config.tenant_id.clone(),
        user_id: config.operator_user_id.clone(),
        agent_id: Some(config.agent_id.clone()),
        project_id: config.project_id.clone(),
        mission_id: None,
        thread_id: None,
        invocation_id: ironclaw_host_api::InvocationId::new(),
    }
}

fn invalid_config(field: &'static str, reason: String) -> TelegramHostBuildError {
    TelegramHostBuildError::InvalidConfig { field, reason }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use ironclaw_loop_host::{
        HostManagedModelError, HostManagedModelGateway, HostManagedModelRequest,
        HostManagedModelResponse,
    };
    use ironclaw_product_workflow::WebUiAuthenticatedCaller;
    use ironclaw_turns::run_profile::LoopCapabilityPort;
    use tower::ServiceExt;

    use super::*;
    use crate::telegram::telegram_serve::TELEGRAM_UPDATES_PATH;
    use crate::{
        RebornBuildInput, RebornRuntimeIdentity, RebornRuntimeInput, build_reborn_runtime,
        local_dev_runtime_policy,
    };

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
        let root = tempfile::tempdir().expect("tempdir");
        let runtime = build_reborn_runtime(
            RebornRuntimeInput::from_services(
                RebornBuildInput::local_dev("telegram-host-owner", root.path().join("local-dev"))
                    .with_runtime_policy(local_dev_runtime_policy().expect("local policy")),
            )
            .with_identity(RebornRuntimeIdentity {
                tenant_id: TENANT.to_string(),
                agent_id: AGENT.to_string(),
                source_binding_id: "telegram-host-source".to_string(),
                reply_target_binding_id: "telegram-host-reply".to_string(),
            })
            .with_model_gateway_override(Arc::new(StaticGateway)),
        )
        .await
        .expect("runtime builds");
        (runtime, root)
    }

    fn host_config() -> TelegramHostRuntimeConfig {
        TelegramHostRuntimeConfig::new(
            TenantId::new(TENANT).expect("tenant"),
            AgentId::new(AGENT).expect("agent"),
            None,
            UserId::new(OPERATOR).expect("operator"),
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
                    == crate::telegram::telegram_channel_routes::WEBUI_V2_CHANNELS_TELEGRAM_SETUP_PATH),
            "setup route must be mounted"
        );
        assert!(
            protected
                .descriptors
                .iter()
                .any(|descriptor| descriptor.route_pattern().as_str()
                    == crate::telegram::telegram_channel_routes::WEBUI_V2_CHANNELS_TELEGRAM_PAIRING_PATH),
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
}
