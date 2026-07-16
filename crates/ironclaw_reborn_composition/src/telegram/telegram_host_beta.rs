//! Host-beta Telegram updates-route composition.
//!
//! Single composition point for the native Telegram channel host: the CLI
//! supplies explicit host config and this module assembles the durable setup /
//! pairing stores, the pairing-aware dynamic ingress resolver, the
//! per-setup-revision product workflow (binding + idempotency + gates) and
//! final-reply delivery observer, the personal-DM outbound target provider,
//! the triggered-run delivery hook, and the WebUI channel-route/facade
//! surfaces from the already built Reborn runtime. Mirrors
//! `slack_host_beta`'s runtime-setup path (operator-managed setup, DM-only —
//! no shared channel routes and no subject route resolver).
//!
//! ## Setup-revision keyed workflow (no restart required)
//!
//! Everything derived from the operator's setup record — webhook verifier,
//! inbound parsing, the product workflow's installation scope, and the
//! delivery observer's rendering adapter — rebuilds per setup revision inside
//! [`DynamicTelegramInstallationResolver`] via the
//! [`crate::telegram::telegram_serve::TelegramRevisionWorkflowBuilder`] this
//! module implements over revision-independent runtime parts. Configuring the
//! bot for the first time after boot, or swapping to a different bot, takes
//! effect on the next webhook without a process restart. A token rotation for
//! the SAME bot keeps its installation id, so bindings and conversations
//! continue uninterrupted.

use std::num::NonZeroUsize;
use std::sync::Arc;

use ironclaw_conversations::RebornFilesystemConversationServices;
use ironclaw_host_api::{AgentId, ProjectId, ResourceScope, TenantId, UserId};
use ironclaw_outbound::{DeliveredGateRouteStore, TriggeredRunDeliveryStore};
use ironclaw_product_adapters::{
    AdapterInstallationId, AuthRequirement, DeclaredEgressTarget, DeliveryStatus,
    EgressCredentialHandle, OutboundDeliverySink, ProductAdapter, ProductAdapterId,
    ProtocolHttpEgress,
};
use ironclaw_product_workflow::{
    ApprovalInteractionService, AuthInteractionService, ChannelConnectionFacade,
    ConnectableChannelsProductFacade, ConversationBindingService, DefaultInboundTurnService,
    DefaultProductWorkflow, LifecyclePackageKind, LifecyclePackageRef, LifecyclePhase,
    ProductActorUserResolver, ProductConversationBindingService, ProductInstallationKey,
    ProductInstallationScope, ProductWorkflowError, RebornFilesystemIdempotencyLedger,
    ResolveBindingRequest, ResolvedBinding, StaticProductInstallationResolver,
};
use ironclaw_safety::{SafetyConfig, SafetyLayer};
use ironclaw_telegram_v2_adapter::{
    GroupTriggerPolicy, TelegramV2Adapter, TelegramV2AdapterConfig, telegram_declared_egress_hosts,
};
use ironclaw_threads::SessionThreadService;
use ironclaw_triggers::TriggerFire;
use ironclaw_turns::{TurnCoordinator, TurnRunId, TurnScope};
use ironclaw_wasm_product_adapters::EgressPolicy;
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::sync::Mutex;

use crate::RebornRuntime;
use crate::channel_identity::RebornUserIdentityLookup;
use crate::extension_host::extension_lifecycle::{
    ExtensionActivationMode, RebornLocalExtensionManagementPort,
};
use crate::outbound::channel_delivery::{
    FinalReplyDeliveryObserver, FinalReplyDeliveryServices, FinalReplyDeliverySettings,
    PostSubmitDeliveryHook, TriggeredRunDeliveryDriver,
};
use crate::outbound::{OutboundDeliveryTargetProvider, OutboundDeliveryTargetRegistrationOutcome};
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
use crate::telegram::telegram_outbound_targets::TelegramOutboundTargetProvider;
use crate::telegram::telegram_pairing::{
    TelegramDmTargetStore, TelegramPairingService, TelegramPairingStore, TelegramUserBindingStore,
};
use crate::telegram::telegram_serve::{
    DynamicTelegramInstallationResolver, TELEGRAM_SECRET_TOKEN_HEADER,
    TelegramInstallationResolver, TelegramRevisionWorkflow, TelegramRevisionWorkflowBuildError,
    TelegramRevisionWorkflowBuilder, TelegramUpdatesRouteState, telegram_updates_route_mount,
};
use crate::telegram::telegram_setup::{
    TelegramInstallationSetup, TelegramInstallationSetupStore, TelegramSetupService,
};
use crate::webui::webui_serve::{ProtectedRouteMount, PublicRouteMount};

const TELEGRAM_IDEMPOTENCY_LEDGER_SETTLED_LIMIT: usize = 10_000;
const TELEGRAM_IDEMPOTENCY_LEDGER_PRUNE_INTERVAL: usize = 1_000;
const TELEGRAM_OUTBOUND_PROVIDER_KEY_PREFIX: &str = "telegram-host-runtime-setup";

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
    #[error("Telegram host outbound delivery target registration failed: {reason}")]
    OutboundDeliveryTargetRegistration { reason: String },
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

/// No-op [`ConversationBindingService`] for the triggered-run delivery driver
/// (mirror of the Slack host's `NoopConversationBindingService`): the
/// triggered path receives its `TurnScope` directly from the poller and never
/// resolves a product conversation binding.
struct NoopTelegramConversationBindingService;

#[async_trait::async_trait]
impl ConversationBindingService for NoopTelegramConversationBindingService {
    async fn resolve_binding(
        &self,
        _request: ResolveBindingRequest,
    ) -> Result<ResolvedBinding, ProductWorkflowError> {
        Err(ProductWorkflowError::BindingResolutionFailed {
            reason: "NoopTelegramConversationBindingService is not supported in triggered delivery"
                .to_string(),
        })
    }

    async fn lookup_binding(
        &self,
        _request: ResolveBindingRequest,
    ) -> Result<ResolvedBinding, ProductWorkflowError> {
        Err(ProductWorkflowError::BindingResolutionFailed {
            reason: "NoopTelegramConversationBindingService is not supported in triggered delivery"
                .to_string(),
        })
    }
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
/// routes are exactly how that record gets created. Once a setup exists (or
/// changes), the dynamic resolver assembles that revision's verifier,
/// workflow, and delivery observer on the next webhook — no restart (see the
/// module doc).
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
        Arc::clone(&dm_target_store),
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

    let token_handle = telegram_bot_token_handle()?;
    let egress: Arc<dyn ProtocolHttpEgress> = Arc::new(TelegramProtocolHttpEgress::new(
        host_egress,
        Arc::new(SetupServiceTelegramEgressCredentialProvider::new(
            Arc::clone(&setup_service),
        )),
        EgressPolicy::new(telegram_declared_egress_targets(token_handle.clone())),
        telegram_egress_scope_template(&config),
    ));
    // Shared across setup revisions: the durable ledger tree is keyed by the
    // host scope template (tenant/operator/agent), not the installation id,
    // so sharing one instance keeps inbound dedup continuous across a
    // first-configure or bot swap instead of resetting its in-memory
    // settled-entry accounting per revision.
    let idempotency_ledger = Arc::new(
        RebornFilesystemIdempotencyLedger::new(
            Arc::clone(&host_state_filesystem),
            telegram_egress_scope_template(&config),
        )
        .with_settled_entry_limit(
            NonZeroUsize::new(TELEGRAM_IDEMPOTENCY_LEDGER_SETTLED_LIMIT).ok_or_else(|| {
                invalid_config("settled_entry_limit", "must be non-zero".to_string())
            })?,
        )
        .with_settled_prune_interval(
            NonZeroUsize::new(TELEGRAM_IDEMPOTENCY_LEDGER_PRUNE_INTERVAL).ok_or_else(|| {
                invalid_config("settled_prune_interval", "must be non-zero".to_string())
            })?,
        ),
    );

    // Revision-independent parts bundle: the dynamic ingress resolver calls
    // back into it to assemble the workflow + delivery observer for each new
    // setup revision (first configure, bot swap) without a restart.
    let revision_parts = Arc::new(TelegramRevisionWorkflowParts {
        config: config.clone(),
        parts: parts.clone(),
        conversation_port,
        actor_pairings,
        actor_user_resolver,
        idempotency_ledger,
        egress: Arc::clone(&egress),
        token_handle: token_handle.clone(),
    });

    let resolver: Arc<dyn TelegramInstallationResolver> =
        Arc::new(DynamicTelegramInstallationResolver::new(
            Arc::clone(&setup_service),
            Arc::clone(&pairing),
            Arc::clone(&identity_lookup),
            Arc::clone(&revision_parts) as Arc<dyn TelegramRevisionWorkflowBuilder>,
        ));
    let events = telegram_updates_route_mount(TelegramUpdatesRouteState::from_resolver(resolver));

    // Proactive delivery INTO Telegram: the caller-scoped personal-DM target
    // provider (WebUI delivery defaults + per-trigger targets), keyed by host
    // config so a second mounts build for the same config is a no-op. Mirrors
    // the Slack runtime-setup registration.
    let outbound_delivery_target_provider: Arc<dyn OutboundDeliveryTargetProvider> =
        Arc::new(TelegramOutboundTargetProvider::new(
            config.tenant_id.clone(),
            Arc::clone(&setup_service),
            Arc::clone(&dm_target_store),
        ));
    let provider_key = telegram_outbound_delivery_target_provider_key(&config);
    let provider_already_registered = runtime
        .outbound_delivery_target_provider_key_registered(&provider_key)
        .map_err(
            |error| TelegramHostBuildError::OutboundDeliveryTargetRegistration {
                reason: error.to_string(),
            },
        )?;
    if !provider_already_registered {
        match runtime
            .register_outbound_delivery_target_provider(
                provider_key,
                Arc::clone(&outbound_delivery_target_provider),
            )
            .map_err(
                |error| TelegramHostBuildError::OutboundDeliveryTargetRegistration {
                    reason: error.to_string(),
                },
            )? {
            OutboundDeliveryTargetRegistrationOutcome::Registered => {}
            OutboundDeliveryTargetRegistrationOutcome::Replaced => {
                return Err(TelegramHostBuildError::OutboundDeliveryTargetRegistration {
                    reason:
                        "Telegram outbound delivery target provider was concurrently registered"
                            .to_string(),
                });
            }
        }
    }

    // Triggered-run delivery into Telegram DMs: appended to the trigger
    // poller's composite post-submit hook (Slack's mounts fill the same slot
    // under their own key when both hosts are enabled). Mirrors the Slack
    // duplicate-config guard: a second mounts build for the SAME config is
    // tolerated (hook key already present, provider already registered), a
    // DIFFERENT config fails closed.
    let trigger_delivery_hook: Arc<dyn PostSubmitDeliveryHook> =
        Arc::new(DynamicTelegramTriggeredRunDeliveryHook::new(
            Arc::clone(&revision_parts),
            Arc::clone(&setup_service),
            Arc::clone(&parts.local_runtime.triggered_run_delivery)
                as Arc<dyn TriggeredRunDeliveryStore>,
            Arc::clone(&outbound_delivery_target_provider),
        ));
    let hook_added = runtime.add_trigger_post_submit_hook(
        crate::runtime::TELEGRAM_TRIGGER_POST_SUBMIT_HOOK_KEY,
        trigger_delivery_hook,
    );
    if !hook_added && runtime.trigger_post_submit_hook_is_set() && !provider_already_registered {
        return Err(TelegramHostBuildError::OutboundDeliveryTargetRegistration {
            reason:
                "Telegram triggered-run delivery hook is already wired for a different Telegram host config"
                    .to_string(),
        });
    }

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

/// Revision-independent services the per-revision workflow assembly reuses:
/// resolved once in [`build_telegram_host_runtime_mounts`], then consulted by
/// [`DynamicTelegramInstallationResolver`] (workflow + observer per setup
/// revision) and by the triggered-run delivery hook (driver per revision).
struct TelegramRevisionWorkflowParts {
    config: TelegramHostRuntimeConfig,
    parts: TelegramHostRuntimeParts,
    conversation_port: Arc<dyn ironclaw_conversations::ConversationBindingService>,
    actor_pairings: Arc<dyn ironclaw_conversations::ConversationActorPairingService>,
    actor_user_resolver: Arc<dyn ProductActorUserResolver>,
    idempotency_ledger:
        Arc<RebornFilesystemIdempotencyLedger<crate::factory::LocalDevRootFilesystem>>,
    egress: Arc<dyn ProtocolHttpEgress>,
    token_handle: EgressCredentialHandle,
}

impl TelegramRevisionWorkflowParts {
    /// The Telegram rendering adapter for one setup revision (installation id
    /// + group trigger policy come from the setup record).
    fn adapter_for_setup(
        &self,
        setup: &TelegramInstallationSetup,
        installation_id: AdapterInstallationId,
    ) -> Result<Arc<dyn ProductAdapter>, TelegramHostBuildError> {
        let adapter_id = ProductAdapterId::new(TELEGRAM_V2_ADAPTER_ID)
            .map_err(|reason| invalid_config("adapter_id", reason.to_string()))?;
        Ok(Arc::new(TelegramV2Adapter::new(TelegramV2AdapterConfig {
            adapter_id,
            installation_id,
            group_trigger_policy: GroupTriggerPolicy {
                bot_username: setup.bot_username.clone(),
                bot_user_id: setup.bot_id,
                recognized_commands: vec![],
            },
            egress_credential_handle: self.token_handle.clone(),
            auth_requirement: AuthRequirement::SharedSecretHeader {
                header_name: TELEGRAM_SECRET_TOKEN_HEADER.into(),
            },
            progress_push_enabled: false,
        })))
    }
}

impl TelegramRevisionWorkflowBuilder for TelegramRevisionWorkflowParts {
    fn build_revision_workflow(
        &self,
        setup: &TelegramInstallationSetup,
    ) -> Result<TelegramRevisionWorkflow, TelegramRevisionWorkflowBuildError> {
        let installation_id = setup
            .installation_id()
            .map_err(revision_workflow_build_error)?;

        // DM-only scope: every conversation binds to the paired actor via the
        // resolver; no channel routes and no subject route resolver (unrouted
        // shared conversations fall back to the operator subject, which
        // Telegram group support would revisit). Keyed by THIS revision's
        // installation id, so a bot swap re-scopes conversations by design.
        let scope = ProductInstallationScope::with_default_scope(
            self.config.tenant_id.clone(),
            self.config.agent_id.clone(),
            self.config.project_id.clone(),
        )
        .with_default_subject_user_id(self.config.operator_user_id.clone())
        .with_actor_user_resolver(
            Arc::clone(&self.actor_user_resolver),
            Arc::clone(&self.actor_pairings),
        );
        let adapter_id =
            ProductAdapterId::new(TELEGRAM_V2_ADAPTER_ID).map_err(revision_workflow_build_error)?;
        let installation_resolver = StaticProductInstallationResolver::new([(
            ProductInstallationKey::new(adapter_id, installation_id.clone()),
            scope,
        )]);
        let binding = ProductConversationBindingService::new(
            Arc::clone(&self.conversation_port),
            installation_resolver,
        );
        let inbound = Arc::new(DefaultInboundTurnService::new(
            binding.clone(),
            Arc::clone(&self.parts.thread_service),
            Arc::clone(&self.parts.turn_coordinator),
        ));
        let route_store = Arc::clone(&self.parts.local_runtime.delivered_gate_routes);
        let idempotency_ledger = Arc::clone(&self.idempotency_ledger);
        let workflow = Arc::new(
            DefaultProductWorkflow::new(inbound, idempotency_ledger, Arc::new(binding.clone()))
                .with_approval_interaction_service(Arc::clone(
                    &self.parts.approval_interaction_service,
                ))
                .with_auth_interaction_service(Arc::clone(&self.parts.auth_interaction_service))
                .with_delivered_gate_routes(Arc::clone(&route_store)),
        );

        // Generic final-reply delivery machinery reused from the Slack host
        // (see the import comment); adapter/egress/sink here are Telegram's,
        // and the adapter carries THIS revision's installation identity.
        let adapter = self
            .adapter_for_setup(setup, installation_id)
            .map_err(revision_workflow_build_error)?;
        let observer = Arc::new(FinalReplyDeliveryObserver::with_settings(
            FinalReplyDeliveryServices {
                channel_protocol: Arc::new(
                    crate::telegram::telegram_outbound_targets::TelegramDeliveryProtocol,
                ),
                binding_service: Arc::new(binding),
                thread_service: Arc::clone(&self.parts.thread_service),
                turn_coordinator: Arc::clone(&self.parts.turn_coordinator),
                outbound_store: Arc::clone(&self.parts.local_runtime.outbound_state),
                route_store,
                communication_preferences: Arc::clone(
                    &self.parts.local_runtime.outbound_preferences,
                ),
                adapter,
                egress: Arc::clone(&self.egress),
                delivery_sink: Arc::new(NoopTelegramDeliverySink),
                auth_challenges: self.parts.auth_challenge_provider.clone(),
                auth_flow_canceller: self.parts.auth_flow_canceller.clone(),
                approval_requests: Some(Arc::clone(&self.parts.local_runtime.approval_requests)
                    as Arc<dyn ironclaw_run_state::ApprovalRequestStore>),
            },
            FinalReplyDeliverySettings::default(),
        ));

        Ok(TelegramRevisionWorkflow {
            workflow,
            workflow_observer: Some(observer),
        })
    }
}

fn revision_workflow_build_error(
    error: impl std::fmt::Display,
) -> TelegramRevisionWorkflowBuildError {
    TelegramRevisionWorkflowBuildError::new(error.to_string())
}

/// Deterministic per-host-config registry key for the Telegram outbound
/// delivery target provider (mirror of the Slack runtime-setup key).
fn telegram_outbound_delivery_target_provider_key(config: &TelegramHostRuntimeConfig) -> String {
    let mut hasher = Sha256::new();
    hash_provider_key_field(&mut hasher, config.tenant_id.as_str());
    hash_provider_key_field(&mut hasher, config.agent_id.as_str());
    hash_provider_key_field(
        &mut hasher,
        config.project_id.as_ref().map_or("", ProjectId::as_str),
    );
    hash_provider_key_field(&mut hasher, config.operator_user_id.as_str());

    let digest = hasher.finalize();
    let mut suffix = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        #[allow(clippy::let_underscore_must_use)] // writing to a String is infallible
        let _ = write!(&mut suffix, "{byte:02x}");
    }
    format!("{TELEGRAM_OUTBOUND_PROVIDER_KEY_PREFIX}:{suffix}")
}

fn hash_provider_key_field(hasher: &mut Sha256, value: &str) {
    hasher.update(value.len().to_be_bytes());
    hasher.update(value.as_bytes());
}

/// Setup-revision-cached triggered-run delivery hook: builds a
/// [`TriggeredRunDeliveryDriver`] over the Telegram adapter/egress for the
/// CURRENT setup revision (mirror of `DynamicSlackTriggeredRunDeliveryHook`),
/// so first-configure and bot swaps re-key trigger delivery without a
/// restart. No setup record => the delivery is skipped with a debug log; the
/// driver's own delivery/outcome semantics are untouched.
struct DynamicTelegramTriggeredRunDeliveryHook {
    revision_parts: Arc<TelegramRevisionWorkflowParts>,
    setup_service: Arc<TelegramSetupService>,
    delivery_store: Arc<dyn TriggeredRunDeliveryStore>,
    outbound_target_provider: Arc<dyn OutboundDeliveryTargetProvider>,
    cached_driver: Mutex<Option<CachedTelegramTriggeredRunDriver>>,
}

struct CachedTelegramTriggeredRunDriver {
    revision: u64,
    driver: Arc<TriggeredRunDeliveryDriver>,
}

impl DynamicTelegramTriggeredRunDeliveryHook {
    fn new(
        revision_parts: Arc<TelegramRevisionWorkflowParts>,
        setup_service: Arc<TelegramSetupService>,
        delivery_store: Arc<dyn TriggeredRunDeliveryStore>,
        outbound_target_provider: Arc<dyn OutboundDeliveryTargetProvider>,
    ) -> Self {
        Self {
            revision_parts,
            setup_service,
            delivery_store,
            outbound_target_provider,
            cached_driver: Mutex::new(None),
        }
    }

    async fn current_driver(&self) -> Result<Option<Arc<TriggeredRunDeliveryDriver>>, String> {
        let Some(setup) = self
            .setup_service
            .current_setup()
            .await
            .map_err(|error| error.to_string())?
        else {
            return Ok(None);
        };
        let revision = setup.revision;

        {
            let cached_driver = self.cached_driver.lock().await;
            if let Some(cached) = cached_driver
                .as_ref()
                .filter(|cached| cached.revision == revision)
            {
                return Ok(Some(Arc::clone(&cached.driver)));
            }
        }

        let driver = self
            .build_driver(&setup)
            .map_err(|error| error.to_string())?;

        let mut cached_driver = self.cached_driver.lock().await;
        if let Some(cached) = cached_driver
            .as_ref()
            .filter(|cached| cached.revision >= revision)
        {
            return Ok(Some(Arc::clone(&cached.driver)));
        }
        *cached_driver = Some(CachedTelegramTriggeredRunDriver {
            revision,
            driver: Arc::clone(&driver),
        });
        Ok(Some(driver))
    }

    fn build_driver(
        &self,
        setup: &TelegramInstallationSetup,
    ) -> Result<Arc<TriggeredRunDeliveryDriver>, TelegramHostBuildError> {
        let installation_id = setup
            .installation_id()
            .map_err(|reason| invalid_config("installation_id", reason.to_string()))?;
        let adapter = self
            .revision_parts
            .adapter_for_setup(setup, installation_id)?;
        let parts = &self.revision_parts.parts;
        let route_store: Arc<dyn DeliveredGateRouteStore> =
            Arc::clone(&parts.local_runtime.delivered_gate_routes);
        let services = FinalReplyDeliveryServices {
            channel_protocol: Arc::new(
                crate::telegram::telegram_outbound_targets::TelegramDeliveryProtocol,
            ),
            binding_service: Arc::new(NoopTelegramConversationBindingService),
            thread_service: Arc::clone(&parts.thread_service),
            turn_coordinator: Arc::clone(&parts.turn_coordinator),
            outbound_store: Arc::clone(&parts.local_runtime.outbound_state),
            route_store: Arc::clone(&route_store),
            communication_preferences: Arc::clone(&parts.local_runtime.outbound_preferences),
            adapter,
            egress: Arc::clone(&self.revision_parts.egress),
            delivery_sink: Arc::new(NoopTelegramDeliverySink),
            auth_challenges: parts.auth_challenge_provider.clone(),
            auth_flow_canceller: parts.auth_flow_canceller.clone(),
            approval_requests: Some(Arc::clone(&parts.local_runtime.approval_requests)
                as Arc<dyn ironclaw_run_state::ApprovalRequestStore>),
        };
        Ok(Arc::new(
            TriggeredRunDeliveryDriver::new(
                services,
                Arc::clone(&self.delivery_store),
                route_store,
                self.revision_parts.config.agent_id.clone(),
            )
            .with_outbound_target_provider(Arc::clone(&self.outbound_target_provider)),
        ))
    }
}

#[async_trait::async_trait]
impl PostSubmitDeliveryHook for DynamicTelegramTriggeredRunDeliveryHook {
    async fn on_trigger_submitted(&self, fire: TriggerFire, run_id: TurnRunId, scope: TurnScope) {
        match self.current_driver().await {
            Ok(Some(driver)) => driver.on_trigger_submitted(fire, run_id, scope).await,
            Ok(None) => {
                tracing::debug!(
                    %run_id,
                    "Telegram triggered-run delivery skipped: Telegram setup is not configured"
                );
            }
            Err(error) => {
                tracing::warn!(
                    %run_id,
                    %error,
                    "Telegram triggered-run delivery skipped: delivery hook unavailable"
                );
            }
        }
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
        telegram_runtime_with(|input| input).await
    }

    async fn telegram_runtime_with(
        customize: impl FnOnce(RebornRuntimeInput) -> RebornRuntimeInput,
    ) -> (crate::RebornRuntime, tempfile::TempDir) {
        let root = tempfile::tempdir().expect("tempdir");
        let input = RebornRuntimeInput::from_services(
            RebornBuildInput::local_dev("telegram-host-owner", root.path().join("local-dev"))
                .with_runtime_policy(local_dev_runtime_policy().expect("local policy")),
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
}
