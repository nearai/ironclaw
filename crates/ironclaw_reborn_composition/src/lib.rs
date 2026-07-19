#![forbid(unsafe_code)]

//! Reborn composition root.
//!
//! Two entry points:
//!
//! - [`build_reborn_services`] — substrate/product facades (host runtime,
//!   turn coordinator, product auth). Useful when an outer harness wires the loop
//!   drivers / turn-runner itself (e.g. v1 `AppBuilder`).
//! - [`build_reborn_runtime`] — full runtime assembly: substrate + loop
//!   driver registry + LLM model gateway (under `root-llm-provider`) +
//!   turn-runner worker, spawned as one unit. This is the single entry
//!   point used by the standalone `ironclaw-reborn` binary and any
//!   future Reborn ingress.
//!
//! Downstream callers should not name internal Reborn types directly:
//! [`RebornRuntime`] exposes only task-level methods, so callers never
//! import `TurnCoordinator`, `SessionThreadService`, `HostManagedModel
//! Gateway`, etc.

use std::sync::Arc;

#[cfg(feature = "webui-v2-beta")]
mod admin_secrets;
mod admin_token;
#[cfg(feature = "webui-v2-beta")]
mod admin_user_directory;
#[cfg(test)]
mod approval_test_support;
mod automation;
mod blocked_auth_resume;
mod builtin_capability_policy;
mod deployment;
mod error;
mod extension_host;
mod factory;
mod google_oauth_secret_store;
mod input;
mod lifecycle_auth_continuation;
mod llm_admin;
mod local_dev_authorization;
mod local_dev_mounts;
mod local_runtime_profile;
mod observability;
mod outbound;
mod product_auth;
#[cfg(any(feature = "libsql", feature = "postgres"))]
mod production_runtime_policy;
mod profile_approval_authorization;
mod projection;
mod slack;
mod telegram;
pub use ironclaw_product_workflow::{
    AuthChallengeProvider, AuthChallengeView, BlockedAuthFlowCanceller,
};
#[cfg(any(feature = "slack-v2-host-beta", feature = "telegram-v2-host-beta"))]
mod delivered_gate_routing;
mod readiness;
mod root;
mod runtime;
mod runtime_input;
mod runtime_profile_approval_policy;
mod support;
#[cfg(feature = "test-support")]
pub mod test_support;
mod turn_run_snapshot;
mod web_access;
mod webui;

pub use admin_token::AdminApiTokenMinter;
pub use automation::facade::RebornAutomationProductFacade;
pub use error::RebornBuildError;
pub use extension_host::extension_lifecycle_command::{
    RebornExtensionLifecycleCommand, RebornExtensionLifecycleCommandError,
    execute_reborn_extension_lifecycle_command, render_reborn_extension_lifecycle_response,
};
pub use extension_host::gsuite::{
    bundled_gsuite_extension_packages, bundled_gsuite_first_party_handlers,
};
pub use extension_host::skill_listing::{RebornSkillListError, list_reborn_local_skills};
#[cfg(feature = "test-support")]
pub use factory::AttachmentTestSupport;
#[cfg(any(feature = "libsql", feature = "postgres"))]
pub use factory::LOCAL_DEV_SECRETS_MASTER_KEY_PATH;
#[cfg(feature = "test-support")]
pub use factory::RebornApprovalTestParts;
#[cfg(feature = "migration-support")]
pub use factory::extension_installation_store_for_migration;
#[cfg(any(feature = "libsql", feature = "test-support"))]
pub use factory::local_dev_db_path;
#[cfg(feature = "libsql")]
pub use factory::open_local_dev_secret_store;
#[cfg(any(feature = "libsql", feature = "postgres"))]
pub use factory::{KeychainMasterKeyOutcome, provision_local_dev_keychain_master_key};
pub use factory::{RebornServices, build_reborn_services, builtin_first_party_trust_policy};
pub use google_oauth_secret_store::{GoogleOauthSecretStore, GoogleOauthSecretStoreError};
pub use input::{OAuthClientConfig, RebornBuildInput, RebornRuntimeProcessBinding};
#[cfg(feature = "webui-v2-beta")]
pub use ironclaw_auth::GoogleOAuthRouteConfig;
/// OAuth redirect-URI newtype re-exported so the `ironclaw_reborn_cli` binary
/// can name it without a direct `ironclaw_auth` dependency. Its
/// `runtime/mod.rs` parses `IRONCLAW_REBORN_SLACK_PERSONAL_OAUTH_REDIRECT_URI`
/// and the Google OAuth redirect URI from env into `OAuthRedirectUri` when
/// building the runtime input / OAuth client config. The
/// `reborn_cli_binary_crate_stays_separate_from_v1_root` boundary test (in
/// `ironclaw_architecture`) pins the CLI's workspace dependencies to exactly
/// the composition-facade set, so adding `ironclaw_auth` there would fail that
/// test — the type must travel through this facade instead.
pub use ironclaw_auth::OAuthRedirectUri;
pub use ironclaw_product_workflow::{
    LifecycleExtensionSource, LifecycleExtensionSummary, LifecyclePhase, LifecycleProductPayload,
    LifecycleProductResponse, LifecycleSearchExtensionSummary,
};
pub use ironclaw_runner::runtime::DEFAULT_TURN_RUNNER_WORKER_COUNT;
// Re-exported for `ironclaw_reborn_cli` (`runtime/mod.rs` turn-failure display):
// the CLI consumes composition as its facade and must not grow a direct
// `ironclaw_runner` edge for one summary helper. All other run-failure
// classifier items moved to `ironclaw_runner::{failure_lane, failure_summary,
// retry_disposition}` with consumers repointed (no path-preservation shims).
pub use ironclaw_runner::failure_summary::reborn_failure_summary_for_category;
#[cfg(any(feature = "libsql", feature = "postgres"))]
pub use ironclaw_runtime_policy::{
    ResolveRequest as RuntimePolicyResolveRequest, resolve as resolve_runtime_policy,
};
pub use ironclaw_skills::{
    ManagedSkillSource as RebornSkillSource, SkillSummary as RebornSkillSummary,
    skill_summary_json as reborn_skill_summary_json,
};
pub use ironclaw_triggers::TriggerId;
pub use ironclaw_turns::TurnStatus;
#[cfg(feature = "root-llm-provider")]
pub use llm_admin::llm_catalog::{
    ProviderCatalogValidationError, RebornLlmCatalogError, resolve_against_registry,
    resolve_llm_selection_against_catalog, resolve_llm_selection_allow_missing_key,
    resolve_reborn_runtime_llm, validate_reborn_provider_catalog_contents,
};
#[cfg(feature = "root-llm-provider")]
pub use llm_admin::llm_config_service::{LlmReloadTrigger, RebornLlmConfigService};
#[cfg(feature = "root-llm-provider")]
pub use llm_admin::llm_key_store::{LlmKeyStore, LlmKeyStoreError};
pub use llm_admin::nearai_mcp::{
    NearAiMcpBootstrapConfig, NearAiMcpBootstrapConfigError, nearai_mcp_bootstrap_config_from_env,
};
#[cfg(feature = "openai-compat-beta")]
pub use llm_admin::openai_compat_serve::build_openai_compat_route_mount;
// Re-exported for the host-owned `ironclaw_webui::webui_v2_app`
// (hoisted up from this crate): its bearer-auth middleware mints tenant-scoped
// verified-bearer evidence for protected OpenAI-compatible mounts. Ingress must
// not depend on `ironclaw_product_adapters` directly (architecture boundary), so
// it reaches this helper through composition's facade.
#[cfg(feature = "openai-compat-beta")]
pub use ironclaw_product_adapters::mark_bearer_token_verified_for_tenant;
#[cfg(feature = "root-llm-provider")]
pub use llm_admin::provider_admin::{
    DetectedEnvLlm, EXAMPLE_OVERLAY_PROVIDER_ID, ProviderMenuEntry, ProviderProbeOutcome,
    RebornModelRoutesState, RebornProviderAdmin, RebornProviderAdminError, RebornProviderInfo,
    RebornProviderList, RebornProviderMetadata, RebornProviderSelection, RebornProviderStatus,
    RebornProviderWriteOutcome, RebornV1State,
};
#[cfg(feature = "root-llm-provider")]
pub use llm_admin::provider_admin_product_command::RebornProviderAdminProductCommandService;
#[cfg(feature = "root-llm-provider")]
pub use llm_admin::provider_repo::{ProviderRepo, ProviderRepoError};
pub use local_runtime_profile::{
    RebornRuntimeProfileError, RebornRuntimeProfileOptions, hosted_single_tenant_runtime_policy,
    hosted_single_tenant_volume_runtime_policy, local_dev_runtime_policy,
    local_dev_yolo_runtime_policy, local_runtime_build_input,
    local_runtime_build_input_with_options,
};
pub use observability::budget::build_default_budget_accountant;
pub use observability::budget_events::{BudgetEventObserver, TracingBudgetEventObserver};
pub use observability::hooks::{
    HOOKS_ENABLED_ENV, HOOKS_THIRD_PARTY_ENABLED_ENV, HookDispatcherBuilderFactory,
    HookProjectionRegistry, HooksActivationConfig, MAX_INSTALLED_EXTENSIONS_CONSIDERED,
    MAX_TOTAL_HOOKS_PER_TENANT, ThirdPartyDiscoveryInput, build_hook_dispatcher_builder_factory,
    build_hook_dispatcher_builder_factory_for_tenant, build_hook_projection_registry,
    tenant_extension_root,
};
pub use observability::operator_logs::{
    OperatorLogLayer, capture_tracing_log, operator_log_buffer,
};
pub use observability::trajectory_observer::RebornTrajectoryObserver;
// The continuation-dispatch port lives in ironclaw_channel_host so channel
// host crates can hold it without a composition dependency; composition's
// facade re-exports it for its own downstream consumers (root test suites,
// the CLI) alongside the product-auth service surface that produces it.
pub use ironclaw_channel_host::auth_continuation::RebornAuthContinuationDispatcher;
#[cfg(feature = "slack-v2-host-beta")]
pub use ironclaw_channel_host::identity::{
    RebornUserIdentityLookup, RebornUserIdentityLookupError,
};
pub use product_auth::api::auth::{
    RebornAuthProductError, RebornCredentialLifecycleError, RebornManualTokenChallenge,
    RebornManualTokenError, RebornManualTokenSetupRequest, RebornManualTokenSubmitRequest,
    RebornManualTokenSubmitResponse, RebornOAuthCallbackError, RebornOAuthCallbackOutcome,
    RebornOAuthCallbackRequest, RebornOAuthCallbackResponse, RebornProductAuthServicePorts,
    RebornProductAuthServices,
};
#[cfg(feature = "slack-v2-host-beta")]
pub use product_auth::serve::SlackPersonalOAuthBindingConfig;
// Product-auth WebUI route-mount builders, exposed so the host-owned
// `ironclaw_webui::webui_v2_app` (moved up from this crate) can
// compose the Reborn-native product-auth surface into the WebChat v2 router.
#[cfg(feature = "webui-v2-beta")]
pub use product_auth::serve::{
    ProductAuthRouteMount, ProductAuthRouteState, product_auth_route_mount,
};
#[cfg(any(feature = "libsql", feature = "postgres"))]
pub use production_runtime_policy::RebornProductionRuntimePolicy;
pub use readiness::{
    RebornFacadeReadiness, RebornReadiness, RebornReadinessDiagnostic,
    RebornReadinessDiagnosticComponent, RebornReadinessDiagnosticReason,
    RebornReadinessDiagnosticStatus, RebornReadinessState, RebornWorkerReadiness,
};
pub use root::product_live_adapters::{
    ProductLiveCapabilityAuthorityResolver, ProductLiveCapabilityIo, ProductLiveModelRouteSettings,
    ProductLivePlannedRuntimeAdapterConfig, ProductLivePlannedRuntimeAdapterError,
    ProductLivePlannedRuntimeAdapters, ProductLiveVisibleCapabilityRequestConfig,
    capability_allowlist, visible_capability_request_for_run,
};
pub use root::profile::{RebornCompositionProfile, RebornCompositionProfileParseError};
#[cfg(any(test, feature = "test-support"))]
pub use runtime::RebornTurnDriveOutcome;
pub use runtime::{
    AssistantReply, ConversationId, RebornRuntime, RebornRuntimeError, RebornSkillActivation,
    RebornSkillActivationMode, RebornSkillAsset, RebornSkillBundle, RebornSkillExecutionPlan,
    RebornSkillExecutionResult, RebornSkillSourceKind, build_reborn_runtime,
};
pub use runtime_input::{
    CredentialRefreshSettings, DEFAULT_TURN_RUNNER_HEARTBEAT_INTERVAL,
    DEFAULT_TURN_RUNNER_POLL_INTERVAL, PollSettings, RebornRuntimeIdentity, RebornRuntimeInput,
    TriggerFireAccessCheck, TriggerFireAccessChecker, TriggerFireAccessDecision,
    TriggerFireAccessError, TriggerPollerSettings, TurnRunnerSettings,
};
#[cfg(feature = "root-llm-provider")]
pub use runtime_input::{RebornProviderFactory, ResolvedRebornLlm};
#[cfg(feature = "slack-v2-host-beta")]
pub use slack::slack_actor_identity::{
    SlackUserIdentityActorResolver, slack_user_identity_provider_user_id,
};
#[cfg(feature = "slack-v2-host-beta")]
pub use slack::slack_channel_routes::{
    SlackChannelRouteAdminRouteConfig, SlackChannelRouteAdminRouteMount,
    WEBUI_V2_CHANNELS_SLACK_ALLOWED_PATH, WEBUI_V2_CHANNELS_SLACK_ROUTES_PATH,
    WEBUI_V2_CHANNELS_SLACK_SUBJECTS_PATH, slack_channel_route_admin_route_mount,
};
#[cfg(feature = "slack-v2-host-beta")]
pub use slack::slack_connectable_channel::{
    SlackOperatorRouteVisibility, build_webui_services_with_slack_host_beta_mounts,
};
#[cfg(all(feature = "slack-v2-host-beta", feature = "telegram-v2-host-beta"))]
pub use webui::facade::build_webui_services_with_slack_and_telegram_host_mounts;
// Exported under either channel-host feature: the delivery observer and the
// triggered-run driver are adapter-generic machinery in
// `ironclaw_channel_delivery`; each channel host injects its own
// adapter/egress/sink plus a `ChannelDeliveryProtocol`.
#[cfg(any(feature = "slack-v2-host-beta", feature = "telegram-v2-host-beta"))]
pub use ironclaw_channel_delivery::{
    FinalReplyDeliveryObserver, FinalReplyDeliveryServices, FinalReplyDeliverySettings,
};
#[cfg(any(feature = "slack-v2-host-beta", feature = "telegram-v2-host-beta"))]
pub use ironclaw_channel_delivery::{
    NoopPostSubmitDeliveryHook, PostSubmitDeliveryHook, TriggeredRunDeliveryDriver,
};
#[cfg(feature = "telegram-v2-host-beta")]
pub use ironclaw_telegram_extension::channel_routes::{
    WEBUI_V2_CHANNELS_TELEGRAM_PAIRING_PATH, WEBUI_V2_CHANNELS_TELEGRAM_SETUP_PATH,
};
#[cfg(feature = "slack-v2-host-beta")]
pub use slack::slack_egress::{
    SlackEgressCredential, SlackEgressCredentialError, SlackEgressCredentialProvider,
    SlackProtocolHttpEgress, StaticSlackEgressCredentialProvider,
};
#[cfg(feature = "slack-v2-host-beta")]
pub use slack::slack_host_beta::{
    SlackHostBetaBuildError, SlackHostBetaChannelRoute, SlackHostBetaConfig,
    SlackHostBetaConfigInput, SlackHostBetaLegacySetup, SlackHostBetaMounts,
    SlackHostBetaRuntimeConfig, build_slack_events_route_mount,
    build_slack_events_route_mount_with_actor_user_resolver, build_slack_host_beta_mounts,
    build_slack_host_beta_runtime_mounts, build_triggered_run_delivery_hook,
};
#[cfg(feature = "slack-v2-host-beta")]
pub use slack::slack_serve;
#[cfg(feature = "slack-v2-host-beta")]
pub use slack::slack_serve::{
    SLACK_EVENTS_PATH, SlackEventsRouteState, SlackEventsWebhookDispatcher,
    SlackInstallationSelector, SlackTeamId, slack_events_route_descriptors,
    slack_events_route_mount,
};
#[cfg(feature = "slack-v2-host-beta")]
pub use slack::slack_setup::SlackPersonalSetupServiceSlot;
#[cfg(feature = "telegram-v2-host-beta")]
pub use telegram::telegram_host_beta::{
    TelegramHostBuildError, TelegramHostMounts, TelegramHostRuntimeConfig,
    build_telegram_host_runtime_mounts,
};
pub use web_access::register_bundled_web_access_first_party_handlers;
#[cfg(feature = "telegram-v2-host-beta")]
pub use webui::facade::build_webui_services_with_telegram_host_mounts;
pub use webui::facade::{RebornWebuiBundle, build_webui_services};
// Host-supplied route-mount vocabulary shared with composition's own route
// builders (nearai login, OpenAI-compat) and the host-owned gateway assembly
// in `ironclaw_webui`. The `WebuiServeConfig` / `webui_v2_app`
// / `WebuiAuthenticator` surface moved up into that ingress crate.
#[cfg(feature = "webui-v2-beta")]
pub use webui::route_mounts::{
    ProtectedRouteMount, PublicRouteDrain, PublicRouteDrains, PublicRouteMount,
};

/// Re-exported identity vocabulary host binaries need to construct
/// public runtime/WebUI types whose signatures mention a host-api identity.
/// Kept narrow on purpose — the composition CLAUDE.md says "Expose
/// facade-shaped handles only"; these host-api identity types are the
/// host-identity facade.
pub mod host_api {
    pub use ironclaw_host_api::{
        AgentId, InvocationId, ProjectId, ResourceScope, SecretHandle, TenantId, UserId,
    };
}

#[cfg(all(feature = "webui-v2-beta", feature = "postgres"))]
pub use ironclaw_runner::local_trigger_access::RebornFilesystemLocalTriggerAccessStore;
/// Reborn-owned local trigger-fire access store, re-exported so host
/// binaries reach it through this composition facade instead of taking a
/// direct `ironclaw_runner` dependency (the
/// `reborn_cli_binary_crate_stays_separate_from_v1_root` architecture
/// boundary forbids that). The store is a reborn-owned repository. Local-dev
/// callers use [`open_local_trigger_access_store`]; hosted-single-tenant
/// callers use the filesystem-backed store through the host filesystem
/// abstraction.
#[cfg(feature = "webui-v2-beta")]
pub use ironclaw_runner::local_trigger_access::{
    LocalTriggerAccessReconciliation, LocalTriggerAccessRole, LocalTriggerAccessSeed,
    LocalTriggerAccessSource, LocalTriggerAccessStore, RebornLibSqlLocalTriggerAccessStore,
    RebornLocalTriggerAccessStoreError,
};

#[cfg(feature = "webui-v2-beta")]
struct LocalTriggerAccessFireChecker {
    store: std::sync::Arc<dyn LocalTriggerAccessStore>,
}

#[cfg(feature = "webui-v2-beta")]
impl LocalTriggerAccessFireChecker {
    fn new(store: std::sync::Arc<dyn LocalTriggerAccessStore>) -> Self {
        Self { store }
    }
}

/// Wrap a backend-neutral local trigger access store as the runtime fire-time
/// authorizer.
#[cfg(feature = "webui-v2-beta")]
pub fn local_trigger_access_fire_checker(
    store: std::sync::Arc<dyn LocalTriggerAccessStore>,
) -> std::sync::Arc<dyn runtime_input::TriggerFireAccessChecker> {
    std::sync::Arc::new(LocalTriggerAccessFireChecker::new(store))
}

#[cfg(feature = "webui-v2-beta")]
#[async_trait::async_trait]
impl runtime_input::TriggerFireAccessChecker for LocalTriggerAccessFireChecker {
    async fn check_trigger_fire_access(
        &self,
        request: runtime_input::TriggerFireAccessCheck,
    ) -> Result<runtime_input::TriggerFireAccessDecision, runtime_input::TriggerFireAccessError>
    {
        self.store
            .has_active_local_access(
                &request.tenant_id,
                &request.creator_user_id,
                request.agent_id.as_ref(),
                request.project_id.as_ref(),
            )
            .await
            .map_err(|error| runtime_input::TriggerFireAccessError::Unavailable {
                reason: error.to_string(),
            })
            .map(|allowed| {
                if allowed {
                    runtime_input::TriggerFireAccessDecision::Allowed
                } else {
                    runtime_input::TriggerFireAccessDecision::Denied {
                        reason: "trigger creator does not have active local access for this scope"
                            .to_string(),
                    }
                }
            })
    }
}

/// Canonical Reborn identity resolver vocabulary (issue #4381): the one
/// boundary that maps every external identity — WebUI OAuth logins and
/// external channel/product actors — to a stable `UserId` before runtime
/// state is touched. Only the resolver trait, request, surface, and error
/// types are re-exported so host wiring (`ironclaw-reborn serve`, the CLI
/// `UserDirectory` adapter) depends on the facade vocabulary, never on
/// `ironclaw_reborn_identity` directly. The concrete filesystem-backed store
/// stays private to this composition layer (composition CLAUDE.md: "keep
/// lower substrate handles private").
#[cfg(feature = "webui-v2-beta")]
pub use ironclaw_reborn_identity::{
    ExternalSubjectId, IdentityKeyError, ProviderInstanceId, ProviderKind, RebornIdentityError,
    RebornIdentityResolver, ResolveExternalIdentity, SurfaceKind,
};

/// Test-support: build a standalone canonical Reborn identity resolver on an
/// in-memory host filesystem under `tenant_id`.
///
/// This mirrors the production path
/// [`RebornRuntime::open_reborn_identity_resolver`](crate::RebornRuntime::open_reborn_identity_resolver),
/// which builds the same filesystem-backed store on the runtime's durable
/// scoped filesystem. Production callers must use that accessor; this free
/// function exists only so tests (and downstream integration crates via
/// `test-support`) can build a resolver without standing up a full runtime.
/// Gated so it ships zero bytes in production binaries.
#[cfg(all(feature = "webui-v2-beta", any(test, feature = "test-support")))]
pub fn open_reborn_identity_resolver(
    tenant_id: &ironclaw_host_api::TenantId,
) -> std::sync::Arc<dyn RebornIdentityResolver> {
    use ironclaw_host_api::{
        AgentId, MountAlias, MountGrant, MountPermissions, MountView, UserId, VirtualPath,
    };

    let root = std::sync::Arc::new(ironclaw_filesystem::InMemoryBackend::default());
    let view = MountView::new(vec![MountGrant::new(
        MountAlias::new("/tenant-shared").expect("mount alias"),
        VirtualPath::new("/tenants/test/shared").expect("virtual path"),
        MountPermissions::read_write_list_delete(),
    )])
    .expect("mount view");
    let filesystem = std::sync::Arc::new(ironclaw_filesystem::ScopedFilesystem::with_fixed_view(
        root, view,
    ));
    std::sync::Arc::new(
        ironclaw_reborn_identity::FilesystemRebornIdentityStore::new(
            filesystem,
            tenant_id.clone(),
            UserId::new("test-owner").expect("user"),
            AgentId::new("test-agent").expect("agent"),
            None,
        ),
    )
}

/// Open the reborn-owned local trigger access store on the substrate DB at
/// `path`, creating the parent directory and running its idempotent
/// migrations.
#[cfg(feature = "webui-v2-beta")]
pub async fn open_local_trigger_access_store(
    path: &std::path::Path,
) -> Result<std::sync::Arc<RebornLibSqlLocalTriggerAccessStore>, RebornLocalTriggerAccessStoreError>
{
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| RebornLocalTriggerAccessStoreError::Backend(err.to_string()))?;
    }
    let db = std::sync::Arc::new(
        libsql::Builder::new_local(path)
            .build()
            .await
            .map_err(|err| RebornLocalTriggerAccessStoreError::Backend(err.to_string()))?,
    );
    Ok(std::sync::Arc::new(
        RebornLibSqlLocalTriggerAccessStore::open(db).await?,
    ))
}

#[cfg(all(test, feature = "webui-v2-beta"))]
mod webui_user_access_checker_tests {
    use super::*;
    use crate::runtime_input::{TriggerFireAccessCheck, TriggerFireAccessDecision};
    use ironclaw_host_api::{AgentId, ProjectId, TenantId, UserId};

    #[tokio::test]
    async fn user_store_trigger_fire_checker_uses_exact_seeded_scope() {
        let root = tempfile::tempdir().expect("tempdir");
        let store = open_local_trigger_access_store(&root.path().join("reborn-local-dev.db"))
            .await
            .expect("open local trigger access store");
        let tenant_id = TenantId::new("checker-tenant").expect("tenant id");
        let user_id = UserId::new("checker-user").expect("user id");
        let other_user_id = UserId::new("checker-other-user").expect("user id");
        let agent_id = AgentId::new("checker-agent").expect("agent id");
        let project_id = ProjectId::new("checker-project").expect("project id");

        store
            .seed_local_access(LocalTriggerAccessSeed {
                tenant_id: &tenant_id,
                user_id: &user_id,
                agent_id: Some(&agent_id),
                project_id: Some(&project_id),
                role: LocalTriggerAccessRole::Owner,
                source: LocalTriggerAccessSource::LocalDevEnvBootstrap,
            })
            .await
            .expect("seed local access");

        let checker = local_trigger_access_fire_checker(store);

        let allowed = checker
            .check_trigger_fire_access(TriggerFireAccessCheck {
                tenant_id: tenant_id.clone(),
                creator_user_id: user_id,
                agent_id: Some(agent_id.clone()),
                project_id: Some(project_id.clone()),
                trigger_id: TriggerId::new(),
                fire_slot: chrono::Utc::now(),
            })
            .await
            .expect("check access");
        assert_eq!(allowed, TriggerFireAccessDecision::Allowed);

        let denied = checker
            .check_trigger_fire_access(TriggerFireAccessCheck {
                tenant_id,
                creator_user_id: other_user_id,
                agent_id: Some(agent_id),
                project_id: Some(project_id),
                trigger_id: TriggerId::new(),
                fire_slot: chrono::Utc::now(),
            })
            .await
            .expect("check access");
        assert!(matches!(
            denied,
            TriggerFireAccessDecision::Denied { reason }
                if reason.contains("does not have active local access")
        ));
    }
}

/// Reborn model purpose slot names exposed for diagnostic callers.
///
/// This keeps CLI diagnostics on the composition boundary instead of making
/// the CLI mirror `ironclaw_runner::model_routes::ModelSlot`.
pub fn reborn_model_slot_names() -> Vec<&'static str> {
    ironclaw_runner::model_routes::ModelSlot::all()
        .iter()
        .map(|slot| slot.as_str())
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebornRuntimeReadinessSnapshot {
    pub text_only_driver: RebornRuntimeComponentStatus,
    pub planned_driver: RebornRuntimeComponentStatus,
    pub subagent_planned_driver: RebornRuntimeComponentStatus,
    pub planned_default_profile: RebornRuntimeComponentStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RebornRuntimeComponentStatus {
    Initialized,
    Failed(String),
}

impl RebornRuntimeComponentStatus {
    pub fn from_result<T, E: std::fmt::Display>(result: Result<T, E>) -> Self {
        match result {
            Ok(_) => Self::Initialized,
            Err(error) => Self::Failed(error.to_string()),
        }
    }

    pub fn is_initialized(&self) -> bool {
        matches!(self, Self::Initialized)
    }

    pub fn render(&self, ok_label: &str) -> String {
        match self {
            Self::Initialized => ok_label.to_string(),
            Self::Failed(reason) => format!("unavailable: {reason}"),
        }
    }
}

/// Side-effect-free runtime readiness snapshot for diagnostic callers.
pub fn reborn_runtime_readiness_snapshot() -> RebornRuntimeReadinessSnapshot {
    let mut registry = ironclaw_runner::driver_registry::DriverRegistry::new();
    let text_only_driver = RebornRuntimeComponentStatus::from_result(
        ironclaw_runner::planned_driver_factory::register_default_text_only_driver(
            &mut registry,
            ironclaw_runner::text_loop_driver::TextOnlyModelReplyDriverConfig::default(),
        ),
    );
    let family_registry = ironclaw_runner::app_loop_family::build_loop_family_registry();
    let planned_driver = match &family_registry {
        Ok(family_registry) => RebornRuntimeComponentStatus::from_result(
            ironclaw_runner::planned_driver_factory::register_default_planned_driver(
                &mut registry,
                Arc::clone(family_registry),
            ),
        ),
        Err(error) => RebornRuntimeComponentStatus::Failed(error.to_string()),
    };
    let subagent_planned_driver = match family_registry {
        Ok(family_registry) => RebornRuntimeComponentStatus::from_result(
            ironclaw_runner::planned_driver_factory::register_subagent_planned_driver(
                &mut registry,
                family_registry,
            ),
        ),
        Err(error) => RebornRuntimeComponentStatus::Failed(error.to_string()),
    };
    let planned_default_profile = RebornRuntimeComponentStatus::from_result(
        ironclaw_runner::planned_driver_factory::default_planned_run_profile_resolver(),
    );
    RebornRuntimeReadinessSnapshot {
        text_only_driver,
        planned_driver,
        subagent_planned_driver,
        planned_default_profile,
    }
}

use ironclaw_authorization::CapabilityLeaseError;
#[cfg(feature = "libsql")]
use ironclaw_filesystem::LibSqlRootFilesystem;
#[cfg(feature = "postgres")]
use ironclaw_filesystem::PostgresRootFilesystem;
use ironclaw_filesystem::{RootFilesystem, ScopedFilesystem};
use ironclaw_host_api::ProcessBackendKind;
use ironclaw_host_api::{
    MountAlias, MountGrant, MountPermissions, MountView, ResourceScope, SYSTEM_RESERVED_ID,
    VirtualPath,
};
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_host_runtime::{CapabilitySurfaceVersion, HostRuntimeServices};
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_processes::{FilesystemProcessResultStore, FilesystemProcessStore};
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_reborn_event_store::RebornEventStoreConfig;
use ironclaw_reborn_event_store::RebornEventStoreError;
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_resources::FilesystemResourceGovernor;
use ironclaw_resources::ResourceError;
use ironclaw_run_state::RunStateError;
use ironclaw_secrets::SecretError;
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_secrets::SecretMaterial;
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_trust::TrustPolicy;
use ironclaw_turns::TurnError;
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_turns::TurnRunWakeNotifier;
use thiserror::Error;

#[cfg(feature = "libsql")]
pub type LibSqlProductionHostRuntimeServices = HostRuntimeServices<
    LibSqlRootFilesystem,
    FilesystemResourceGovernor<LibSqlRootFilesystem>,
    FilesystemProcessStore<LibSqlRootFilesystem>,
    FilesystemProcessResultStore<LibSqlRootFilesystem>,
>;

#[cfg(feature = "postgres")]
pub type PostgresProductionHostRuntimeServices = HostRuntimeServices<
    PostgresRootFilesystem,
    FilesystemResourceGovernor<PostgresRootFilesystem>,
    FilesystemProcessStore<PostgresRootFilesystem>,
    FilesystemProcessResultStore<PostgresRootFilesystem>,
>;

/// Consumer-store mount aliases that are tenant-rewritten by
/// [`invocation_mount_view`]. Each alias resolves to
/// `/tenants/<tenant>/users/<user>/<alias>` for the caller's scope, so
/// two tenants sharing one underlying [`RootFilesystem`] cannot collide
/// on identically-shaped paths.
const PER_USER_ALIASES: &[&str] = &[
    "/processes",
    "/secrets",
    "/authorization",
    "/outbound",
    "/run-state",
    "/approvals",
    "/gate-records",
    "/threads",
    "/conversations",
    "/turns",
    "/checkpoint-state",
    "/resources",
    "/engine",
    "/skills",
    "/workspace",
];

/// Per-invocation [`MountView`] used as the production resolver.
///
/// Every call rebuilds the alias→VirtualPath table for the caller's
/// scope so consumer-store records land under
/// `/tenants/<tenant>/users/<user>/<alias>` virtual paths — cross-tenant
/// isolation is structural rather than a convention. `/tenant-shared`
/// resolves to `/tenants/<tenant>/shared`; `/system/{settings,
/// extensions, skills}` route globally as read-only. See
/// `docs/plans/2026-05-16-scoped-filesystem-tenant-isolation.md`.
///
/// The system sentinel scope (see
/// [`ironclaw_host_api::ResourceScope::system`]) routes records under
/// `/tenants/__system__/users/__system__/<alias>`. Production code uses
/// it for process-global records whose paths already encode per-tenant
/// identity (event-log stream keys, conversation singleton state).
pub fn invocation_mount_view(
    scope: &ResourceScope,
) -> Result<MountView, ironclaw_host_api::HostApiError> {
    invocation_mount_view_for_segments(
        resource_scope_path_segment(scope.tenant_id.as_str()),
        resource_scope_path_segment(scope.user_id.as_str()),
    )
}

fn resource_scope_path_segment(value: &str) -> &str {
    if value == SYSTEM_RESERVED_ID {
        "__system__"
    } else {
        value
    }
}

fn invocation_mount_view_for_segments(
    tenant_id: &str,
    user_id: &str,
) -> Result<MountView, ironclaw_host_api::HostApiError> {
    let tenant_user_prefix = format!("/tenants/{tenant_id}/users/{user_id}");
    let mut grants = Vec::with_capacity(PER_USER_ALIASES.len() + 2);
    for alias in PER_USER_ALIASES {
        let target = format!("{tenant_user_prefix}{alias}");
        grants.push(MountGrant::new(
            MountAlias::new(*alias)?,
            VirtualPath::new(target)?,
            MountPermissions::read_write_list_delete(),
        ));
    }
    grants.push(MountGrant::new(
        MountAlias::new("/tenant-shared")?,
        VirtualPath::new(format!("/tenants/{tenant_id}/shared"))?,
        // Broad tenant-shared storage gets read + write + list, but NOT delete:
        // no tenant-shared consumer other than the identity store needs to
        // remove records, so withholding delete here keeps the blast radius of
        // a compromised writer from spanning every tenant-shared subtree.
        MountPermissions::read_write(),
    ));
    grants.push(MountGrant::new(
        // Delete authority is scoped to the identity subtree specifically: the
        // Reborn identity store's admin user-directory needs it for the delete
        // cascade (removing a user's identity / verified-email records) that
        // lives under `/tenant-shared/reborn-identity/…`. Longest-prefix mount
        // matching routes identity paths here and everything else to the
        // delete-less grant above.
        MountAlias::new("/tenant-shared/reborn-identity")?,
        VirtualPath::new(format!("/tenants/{tenant_id}/shared/reborn-identity"))?,
        MountPermissions::read_write_list_delete(),
    ));
    #[cfg(feature = "slack-v2-host-beta")]
    grants.push(MountGrant::new(
        MountAlias::new("/tenant-shared/slack-channel-routes")?,
        VirtualPath::new(format!("/tenants/{tenant_id}/shared/slack-channel-routes"))?,
        MountPermissions::read_only(),
    ));
    for system_subroot in ["/system/settings", "/system/extensions", "/system/skills"] {
        grants.push(MountGrant::new(
            MountAlias::new(system_subroot)?,
            VirtualPath::new(system_subroot)?,
            MountPermissions::read_only(),
        ));
    }
    MountView::new(grants)
}

#[cfg(all(
    any(feature = "libsql", feature = "postgres"),
    feature = "slack-v2-host-beta"
))]
pub(crate) fn slack_host_state_mount_view(
    scope: &ResourceScope,
) -> Result<MountView, ironclaw_host_api::HostApiError> {
    let tenant_id = resource_scope_path_segment(scope.tenant_id.as_str());
    MountView::new(vec![
        MountGrant::new(
            MountAlias::new("/tenant-shared/slack-personal-binding")?,
            VirtualPath::new(format!(
                "/tenants/{tenant_id}/shared/slack-personal-binding"
            ))?,
            MountPermissions::read_write_list_delete(),
        ),
        MountGrant::new(
            MountAlias::new("/tenant-shared/slack-channel-routes")?,
            VirtualPath::new(format!("/tenants/{tenant_id}/shared/slack-channel-routes"))?,
            MountPermissions::read_write_list_delete(),
        ),
        MountGrant::new(
            MountAlias::new("/tenant-shared/slack-setup")?,
            VirtualPath::new(format!("/tenants/{tenant_id}/shared/slack-setup"))?,
            MountPermissions::read_write_list_delete(),
        ),
        MountGrant::new(
            MountAlias::new("/engine/product_workflow/idempotency")?,
            VirtualPath::new(format!(
                "/tenants/{tenant_id}/shared/slack-product-workflow/idempotency"
            ))?,
            MountPermissions::read_write_list_delete(),
        ),
        // Durable Slack conversation-binding store: RebornFilesystemConversationServices
        // persists `/conversations/state.json`. Without this alias the ScopedFilesystem
        // cannot resolve that path, the conversation store fails to open, and every
        // inbound Slack event (e.g. a DM to the bot) is dropped with a 503.
        MountGrant::new(
            MountAlias::new("/conversations")?,
            VirtualPath::new(format!("/tenants/{tenant_id}/shared/slack-conversations"))?,
            MountPermissions::read_write_list_delete(),
        ),
    ])
}

#[cfg(all(
    any(feature = "libsql", feature = "postgres"),
    feature = "telegram-v2-host-beta"
))]
pub(crate) fn telegram_host_state_mount_view(
    scope: &ResourceScope,
) -> Result<MountView, ironclaw_host_api::HostApiError> {
    let tenant_id = resource_scope_path_segment(scope.tenant_id.as_str());
    MountView::new(vec![
        MountGrant::new(
            MountAlias::new("/tenant-shared/telegram-setup")?,
            VirtualPath::new(format!("/tenants/{tenant_id}/shared/telegram-setup"))?,
            MountPermissions::read_write_list_delete(),
        ),
        MountGrant::new(
            MountAlias::new("/tenant-shared/telegram-pairing")?,
            VirtualPath::new(format!("/tenants/{tenant_id}/shared/telegram-pairing"))?,
            MountPermissions::read_write_list_delete(),
        ),
        MountGrant::new(
            MountAlias::new("/tenant-shared/telegram-binding")?,
            VirtualPath::new(format!("/tenants/{tenant_id}/shared/telegram-binding"))?,
            MountPermissions::read_write_list_delete(),
        ),
        MountGrant::new(
            MountAlias::new("/tenant-shared/telegram-dm-targets")?,
            VirtualPath::new(format!("/tenants/{tenant_id}/shared/telegram-dm-targets"))?,
            MountPermissions::read_write_list_delete(),
        ),
        MountGrant::new(
            MountAlias::new("/engine/product_workflow/idempotency")?,
            VirtualPath::new(format!(
                "/tenants/{tenant_id}/shared/telegram-product-workflow/idempotency"
            ))?,
            MountPermissions::read_write_list_delete(),
        ),
        // Durable Telegram conversation-binding store: RebornFilesystemConversationServices
        // persists `/conversations/state.json`. Without this alias the ScopedFilesystem
        // cannot resolve that path, the conversation store fails to open, and every
        // inbound Telegram update (e.g. a DM to the bot) is dropped with a 503.
        MountGrant::new(
            MountAlias::new("/conversations")?,
            VirtualPath::new(format!(
                "/tenants/{tenant_id}/shared/telegram-conversations"
            ))?,
            MountPermissions::read_write_list_delete(),
        ),
    ])
}

/// Wrap `root` in a tenant-aware [`ScopedFilesystem`] whose resolver is
/// [`invocation_mount_view`]. The returned filesystem is the single
/// production handle — every consumer-store call routes per-scope
/// through this one instance.
pub fn wrap_scoped<F>(root: Arc<F>) -> Arc<ScopedFilesystem<F>>
where
    F: RootFilesystem,
{
    Arc::new(ScopedFilesystem::new(root, invocation_mount_view))
}

/// libSQL substrate handles needed to build production host-runtime services.
#[cfg(feature = "libsql")]
pub struct LibSqlProductionSubstrateConfig<TPolicy, TWake>
where
    TPolicy: TrustPolicy + 'static,
    TWake: TurnRunWakeNotifier + 'static,
{
    pub database: Arc<libsql::Database>,
    pub event_store: RebornEventStoreConfig,
    /// Set this only when deployment guarantees exactly one runtime process, or
    /// one elected runtime owner, is allowed to enforce resource quotas for this
    /// database. The filesystem governor keeps in-process tallies as authority.
    pub process_local_resource_governor_singleton: bool,
    pub secret_master_key: Option<SecretMaterial>,
    pub trust_policy: Arc<TPolicy>,
    pub runtime_policy: RebornProductionRuntimePolicy,
    pub turn_run_wake_notifier: Arc<TWake>,
    pub surface_version: CapabilitySurfaceVersion,
}

/// PostgreSQL substrate handles needed to build production host-runtime services.
#[cfg(feature = "postgres")]
pub struct PostgresProductionSubstrateConfig<TPolicy, TWake>
where
    TPolicy: TrustPolicy + 'static,
    TWake: TurnRunWakeNotifier + 'static,
{
    pub pool: deadpool_postgres::Pool,
    pub event_store: RebornEventStoreConfig,
    /// Set this only when deployment guarantees exactly one runtime process, or
    /// one elected runtime owner, is allowed to enforce resource quotas for this
    /// database. The filesystem governor keeps in-process tallies as authority.
    pub process_local_resource_governor_singleton: bool,
    pub secret_master_key: Option<SecretMaterial>,
    pub trust_policy: Arc<TPolicy>,
    pub runtime_policy: RebornProductionRuntimePolicy,
    pub turn_run_wake_notifier: Arc<TWake>,
    pub surface_version: CapabilitySurfaceVersion,
}

#[derive(Debug, Error)]
pub enum RebornCompositionError {
    #[error("invalid reborn production configuration: {reason}")]
    InvalidConfig { reason: String },
    #[error(
        "reborn production composition requires a configured or keychain-resolvable secret master key"
    )]
    MissingSecretMasterKey,
    #[error("reborn mount view construction failed: {0}")]
    Mount(#[from] ironclaw_host_api::HostApiError),
    #[error("reborn filesystem substrate failed: {0}")]
    Filesystem(#[from] ironclaw_filesystem::FilesystemError),
    #[error("reborn resource governor substrate failed: {0}")]
    Resource(#[from] ResourceError),
    #[error("reborn run-state substrate failed: {0}")]
    RunState(#[from] RunStateError),
    #[error("reborn capability lease substrate failed: {0}")]
    CapabilityLease(#[from] CapabilityLeaseError),
    #[error("reborn secret substrate failed: {0}")]
    Secret(#[from] SecretError),
    #[error("reborn event store substrate failed: {0}")]
    EventStore(#[from] RebornEventStoreError),
    #[error("reborn turn substrate failed: {0}")]
    Turn(#[from] TurnError),
    #[error("reborn run-profile resolver substrate failed: {0}")]
    RunProfile(#[from] ironclaw_turns::run_profile::RunProfileRegistryError),
    #[error("production tenant-sandbox process backend requires a tenant sandbox process binding")]
    MissingTenantSandboxProcessPort,
    #[error(
        "production runtime policy uses {process_backend:?} but a tenant sandbox process binding was supplied"
    )]
    UnexpectedTenantSandboxProcessPort { process_backend: ProcessBackendKind },
    #[error("reborn production wiring failed: {report:?}")]
    ProductionWiring {
        report: ironclaw_host_runtime::ProductionWiringReport,
    },
}

/// Build production-wired host-runtime services over libSQL-backed substrates.
///
/// This is deliberately substrate-only: no app/web setup, no runtime adapter
/// registration, and no product loop construction.
///
/// Initialization runs substrate migrations and secret decryptability checks
/// sequentially against the shared database. Earlier successful migrations are
/// not rolled back if a later substrate fails; each migration is expected to be
/// idempotent so callers can fix the underlying failure and retry composition.
#[cfg(feature = "libsql")]
pub async fn build_libsql_production_host_runtime_services<TPolicy, TWake>(
    config: LibSqlProductionSubstrateConfig<TPolicy, TWake>,
) -> Result<LibSqlProductionHostRuntimeServices, RebornCompositionError>
where
    TPolicy: TrustPolicy + 'static,
    TWake: TurnRunWakeNotifier + 'static,
{
    factory::build_libsql_production_host_runtime_services(config).await
}

/// Build production-wired host-runtime services over PostgreSQL-backed substrates.
///
/// Initialization runs substrate migrations and secret decryptability checks
/// sequentially against the shared database. Earlier successful migrations are
/// not rolled back if a later substrate fails; each migration is expected to be
/// idempotent so callers can fix the underlying failure and retry composition.
#[cfg(feature = "postgres")]
pub async fn build_postgres_production_host_runtime_services<TPolicy, TWake>(
    config: PostgresProductionSubstrateConfig<TPolicy, TWake>,
) -> Result<PostgresProductionHostRuntimeServices, RebornCompositionError>
where
    TPolicy: TrustPolicy + 'static,
    TWake: TurnRunWakeNotifier + 'static,
{
    factory::build_postgres_production_host_runtime_services(config).await
}

/// Open a PostgreSQL pool for Reborn production storage using the same
/// TLS/cleartext policy enforced by the production event-store backend.
///
/// Callers are responsible for validating that production boot selected the
/// PostgreSQL storage backend and that the URL came from an env-only config
/// reference before passing it here.
#[cfg(feature = "postgres")]
pub fn open_reborn_postgres_pool(
    url: secrecy::SecretString,
) -> Result<deadpool_postgres::Pool, RebornCompositionError> {
    Ok(ironclaw_reborn_event_store::open_postgres_pool(url)?)
}

/// Open a PostgreSQL pool for Reborn production storage with an explicit
/// maximum connection count.
#[cfg(feature = "postgres")]
pub fn open_reborn_postgres_pool_with_max_size(
    url: secrecy::SecretString,
    max_size: usize,
) -> Result<deadpool_postgres::Pool, RebornCompositionError> {
    Ok(ironclaw_reborn_event_store::open_postgres_pool_with_max_size(url, max_size)?)
}

#[cfg(all(test, any(feature = "libsql", feature = "postgres")))]
mod mount_view_tests {
    use super::*;
    use ironclaw_filesystem::{FilesystemError, FilesystemOperation, InMemoryBackend};
    use ironclaw_host_api::{
        AgentId, InvocationId, MissionId, ProjectId, ScopedPath, TenantId, ThreadId, UserId,
    };

    fn sample_scope() -> ResourceScope {
        ResourceScope {
            tenant_id: TenantId::new("tenant-a").unwrap(),
            user_id: UserId::new("user-1").unwrap(),
            agent_id: Some(AgentId::new("agent-x").unwrap()),
            project_id: Some(ProjectId::new("project-y").unwrap()),
            mission_id: Some(MissionId::new("mission-w").unwrap()),
            thread_id: Some(ThreadId::new("thread-z").unwrap()),
            invocation_id: InvocationId::new(),
        }
    }

    fn other_tenant_scope() -> ResourceScope {
        ResourceScope {
            tenant_id: TenantId::new("tenant-b").unwrap(),
            ..sample_scope()
        }
    }

    #[test]
    fn invocation_mount_view_rewrites_per_user_aliases_to_tenant_user_paths() {
        let scope = sample_scope();
        let view = invocation_mount_view(&scope).unwrap();
        for alias in PER_USER_ALIASES {
            let resolved = view
                .resolve(&ScopedPath::new(format!("{alias}/foo")).unwrap())
                .unwrap();
            assert_eq!(
                resolved.as_str(),
                &format!(
                    "/tenants/{}/users/{}{alias}/foo",
                    scope.tenant_id.as_str(),
                    scope.user_id.as_str()
                )
            );
        }
    }

    #[test]
    fn invocation_mount_view_isolates_tenants_with_same_user() {
        let view_a = invocation_mount_view(&sample_scope()).unwrap();
        let view_b = invocation_mount_view(&other_tenant_scope()).unwrap();
        let path = ScopedPath::new("/engine/threads/x").unwrap();
        let a = view_a.resolve(&path).unwrap();
        let b = view_b.resolve(&path).unwrap();
        assert_ne!(a.as_str(), b.as_str());
        assert!(a.as_str().contains("tenant-a"));
        assert!(b.as_str().contains("tenant-b"));
    }

    #[test]
    fn invocation_mount_view_routes_tenant_shared_to_tenant_root() {
        let scope = sample_scope();
        let view = invocation_mount_view(&scope).unwrap();
        let resolved = view
            .resolve(&ScopedPath::new("/tenant-shared/foo").unwrap())
            .unwrap();
        assert_eq!(
            resolved.as_str(),
            &format!("/tenants/{}/shared/foo", scope.tenant_id.as_str())
        );
    }

    #[cfg(feature = "slack-v2-host-beta")]
    #[test]
    fn invocation_mount_view_exposes_slack_channel_routes_read_only() {
        let scope = sample_scope();
        let view = invocation_mount_view(&scope).unwrap();
        let (resolved, grant) = view
            .resolve_with_grant(
                &ScopedPath::new("/tenant-shared/slack-channel-routes/install/team/route.json")
                    .unwrap(),
            )
            .unwrap();
        assert_eq!(
            resolved.as_str(),
            &format!(
                "/tenants/{}/shared/slack-channel-routes/install/team/route.json",
                scope.tenant_id.as_str()
            )
        );
        assert_eq!(grant.alias.as_str(), "/tenant-shared/slack-channel-routes");
        assert_eq!(grant.permissions, MountPermissions::read_only());
    }

    #[cfg(feature = "slack-v2-host-beta")]
    #[test]
    fn slack_host_state_mount_view_grants_delete_only_to_slack_state_roots() {
        let scope = sample_scope();
        let view = slack_host_state_mount_view(&scope).unwrap();
        for (alias, path, target) in [
            (
                "/tenant-shared/slack-channel-routes",
                "/tenant-shared/slack-channel-routes/install/team/route.json",
                "slack-channel-routes/install/team/route.json",
            ),
            (
                "/tenant-shared/slack-setup",
                "/tenant-shared/slack-setup/installation.json",
                "slack-setup/installation.json",
            ),
            (
                "/engine/product_workflow/idempotency",
                "/engine/product_workflow/idempotency/actions/action.json",
                "slack-product-workflow/idempotency/actions/action.json",
            ),
            // Regression: the durable conversation-binding store persists
            // `/conversations/state.json`; without this alias every inbound Slack
            // event (e.g. a DM to the bot) fails to open the store and is dropped.
            (
                "/conversations",
                "/conversations/state.json",
                "slack-conversations/state.json",
            ),
        ] {
            let (resolved, grant) = view
                .resolve_with_grant(&ScopedPath::new(path).unwrap())
                .unwrap();
            assert_eq!(
                resolved.as_str(),
                &format!("/tenants/{}/shared/{target}", scope.tenant_id.as_str())
            );
            assert_eq!(grant.alias.as_str(), alias);
            assert_eq!(
                grant.permissions,
                MountPermissions::read_write_list_delete()
            );
        }
        // /outbound is no longer in the slack-host-state mount; outbound state is
        // served via the composition-owned per-user scoped filesystem instead.
        assert!(
            view.resolve(&ScopedPath::new("/outbound/deliveries/delivery.json").unwrap())
                .is_err(),
            "/outbound must not resolve through the slack-host-state mount after store unification"
        );
        assert!(
            view.resolve(&ScopedPath::new("/tenant-shared/other.json").unwrap())
                .is_err()
        );
    }

    #[cfg(feature = "telegram-v2-host-beta")]
    #[test]
    fn telegram_host_state_mount_view_grants_delete_only_to_telegram_state_roots() {
        let scope = sample_scope();
        let view = telegram_host_state_mount_view(&scope).unwrap();
        for (alias, path, target) in [
            (
                "/tenant-shared/telegram-setup",
                "/tenant-shared/telegram-setup/installation.json",
                "telegram-setup/installation.json",
            ),
            (
                "/tenant-shared/telegram-pairing",
                "/tenant-shared/telegram-pairing/codes/code.json",
                "telegram-pairing/codes/code.json",
            ),
            (
                "/tenant-shared/telegram-binding",
                "/tenant-shared/telegram-binding/identities/identity.json",
                "telegram-binding/identities/identity.json",
            ),
            (
                "/tenant-shared/telegram-dm-targets",
                "/tenant-shared/telegram-dm-targets/target.json",
                "telegram-dm-targets/target.json",
            ),
            (
                "/engine/product_workflow/idempotency",
                "/engine/product_workflow/idempotency/actions/action.json",
                "telegram-product-workflow/idempotency/actions/action.json",
            ),
            // Regression: the durable conversation-binding store persists
            // `/conversations/state.json`; without this alias every inbound Telegram
            // update (e.g. a DM to the bot) fails to open the store and is dropped.
            (
                "/conversations",
                "/conversations/state.json",
                "telegram-conversations/state.json",
            ),
        ] {
            let (resolved, grant) = view
                .resolve_with_grant(&ScopedPath::new(path).unwrap())
                .unwrap();
            assert_eq!(
                resolved.as_str(),
                &format!("/tenants/{}/shared/{target}", scope.tenant_id.as_str())
            );
            assert_eq!(grant.alias.as_str(), alias);
            assert_eq!(
                grant.permissions,
                MountPermissions::read_write_list_delete()
            );
        }
        assert!(
            view.resolve(&ScopedPath::new("/tenant-shared/other.json").unwrap())
                .is_err(),
            "non-telegram tenant-shared paths must not resolve through the telegram host-state mount"
        );
    }

    #[test]
    fn invocation_mount_view_sanitizes_system_scope_segments() {
        let view = invocation_mount_view(&ResourceScope::system()).unwrap();
        let resolved = view
            .resolve(&ScopedPath::new("/turns/state.json").unwrap())
            .unwrap();
        assert_eq!(
            resolved.as_str(),
            "/tenants/__system__/users/__system__/turns/state.json"
        );
    }

    #[test]
    fn invocation_mount_view_routes_system_globally() {
        let scope = sample_scope();
        let view = invocation_mount_view(&scope).unwrap();
        // Each canonical /system subroot is exposed as its own
        // read-only alias and resolves to the same VirtualPath
        // regardless of tenant — system data is global, not
        // per-tenant.
        for system_subroot in ["/system/settings", "/system/extensions", "/system/skills"] {
            let resolved = view
                .resolve(&ScopedPath::new(format!("{system_subroot}/foo")).unwrap())
                .unwrap();
            assert_eq!(resolved.as_str(), &format!("{system_subroot}/foo"));
        }
    }

    #[test]
    fn invocation_mount_view_routes_user_skills_to_tenant_user_root() {
        let scope = sample_scope();
        let view = invocation_mount_view(&scope).unwrap();
        let (resolved, grant) = view
            .resolve_with_grant(&ScopedPath::new("/skills/code-review/SKILL.md").unwrap())
            .unwrap();
        assert_eq!(
            resolved.as_str(),
            &format!(
                "/tenants/{}/users/{}/skills/code-review/SKILL.md",
                scope.tenant_id.as_str(),
                scope.user_id.as_str()
            )
        );
        assert!(grant.permissions.read);
        assert!(grant.permissions.write);
        assert!(grant.permissions.list);
        assert!(grant.permissions.delete);
        assert!(!grant.permissions.execute);
    }

    #[test]
    fn invocation_mount_view_keeps_user_skills_isolated_from_system_skills() {
        let scope = sample_scope();
        let view = invocation_mount_view(&scope).unwrap();
        let user_skill = view
            .resolve(&ScopedPath::new("/skills/code-review/SKILL.md").unwrap())
            .unwrap();
        let system_skill = view
            .resolve(&ScopedPath::new("/system/skills/code-review/SKILL.md").unwrap())
            .unwrap();
        assert_ne!(user_skill.as_str(), system_skill.as_str());
        assert!(
            user_skill
                .as_str()
                .starts_with("/tenants/tenant-a/users/user-1/skills/")
        );
        assert_eq!(system_skill.as_str(), "/system/skills/code-review/SKILL.md");
    }

    #[test]
    fn invocation_mount_view_isolates_user_skills_between_tenants() {
        let view_a = invocation_mount_view(&sample_scope()).unwrap();
        let view_b = invocation_mount_view(&other_tenant_scope()).unwrap();
        let path = ScopedPath::new("/skills/code-review/SKILL.md").unwrap();
        let a = view_a.resolve(&path).unwrap();
        let b = view_b.resolve(&path).unwrap();
        assert_ne!(a.as_str(), b.as_str());
        assert!(a.as_str().contains("tenant-a"));
        assert!(b.as_str().contains("tenant-b"));
    }

    #[tokio::test]
    async fn scoped_filesystem_rejects_system_skill_writes_but_allows_user_skill_writes() {
        let root = Arc::new(InMemoryBackend::default());
        let scoped = wrap_scoped(root);
        let scope = sample_scope();
        let system_path = ScopedPath::new("/system/skills/code-review/SKILL.md").unwrap();
        let user_path = ScopedPath::new("/skills/code-review/SKILL.md").unwrap();

        let error = scoped
            .write_bytes(&scope, &system_path, b"system skill".to_vec())
            .await
            .expect_err("system skills must remain read-only");
        assert!(matches!(
            error,
            FilesystemError::PermissionDenied {
                operation: FilesystemOperation::WriteFile,
                ..
            }
        ));

        scoped
            .write_bytes(&scope, &user_path, b"user skill".to_vec())
            .await
            .expect("user skills should be writable through the scoped alias");
        let content = scoped
            .read_bytes(&scope, &user_path)
            .await
            .expect("user skill should be readable");
        assert_eq!(content, b"user skill");
    }
}

#[cfg(all(test, any(feature = "libsql", feature = "postgres")))]
mod two_tenant_isolation_tests {
    //! Regression test for the cross-tenant collision finding from the
    //! 2026-05-17 serrrfirat review.
    //!
    //! Drives the public `SecretStore` surface from two distinct
    //! `(tenant, user)` scopes that share identical agent/project/handle,
    //! against the production-shape `wrap_scoped`/`invocation_mount_view`
    //! wiring over an `InMemoryBackend`. Without per-tenant path
    //! rewriting both `put`s would land at the same backend row;
    //! Alice's `consume` would then decrypt to Bob's ciphertext (or
    //! fail with DecryptionFailed via AAD mismatch). The resolver in
    //! place gives each tenant their own subtree — both reads succeed
    //! with their own plaintext.
    //!
    //! A regression that puts the old singleton (identity-mapping)
    //! resolver back into production wiring trips this test directly.
    use super::*;
    use ironclaw_filesystem::InMemoryBackend;
    use ironclaw_host_api::{AgentId, InvocationId, ProjectId, SecretHandle, TenantId, UserId};
    use ironclaw_secrets::{FilesystemSecretStore, SecretMaterial, SecretStore, SecretsCrypto};
    use secrecy::ExposeSecret;

    fn scope(tenant: &str, user: &str) -> ResourceScope {
        ResourceScope {
            tenant_id: TenantId::new(tenant).unwrap(),
            user_id: UserId::new(user).unwrap(),
            agent_id: Some(AgentId::new("github").unwrap()),
            project_id: Some(ProjectId::new("default").unwrap()),
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        }
    }

    fn test_crypto() -> Arc<SecretsCrypto> {
        Arc::new(
            SecretsCrypto::new(SecretMaterial::from(
                "test-master-key-32-bytes-aaaaaaaaa".to_string(),
            ))
            .expect("crypto"),
        )
    }

    #[tokio::test]
    async fn two_tenants_with_same_agent_project_handle_do_not_collide_on_put() {
        let backend = Arc::new(InMemoryBackend::new());
        let scoped = wrap_scoped(Arc::clone(&backend));
        let store = FilesystemSecretStore::new(Arc::clone(&scoped), test_crypto());

        let handle = SecretHandle::new("oauth_token").unwrap();
        let scope_a = scope("tenant_a", "alice");
        let scope_b = scope("tenant_b", "bob");

        store
            .put(
                scope_a.clone(),
                handle.clone(),
                SecretMaterial::from("alice-secret".to_string()),
                None,
            )
            .await
            .unwrap();
        store
            .put(
                scope_b.clone(),
                handle.clone(),
                SecretMaterial::from("bob-secret".to_string()),
                None,
            )
            .await
            .unwrap();

        let lease_a = store.lease_once(&scope_a, &handle).await.unwrap();
        let material_a = store.consume(&scope_a, lease_a.id).await.unwrap();
        assert_eq!(material_a.expose_secret(), "alice-secret");

        let lease_b = store.lease_once(&scope_b, &handle).await.unwrap();
        let material_b = store.consume(&scope_b, lease_b.id).await.unwrap();
        assert_eq!(material_b.expose_secret(), "bob-secret");
    }
}

#[cfg(test)]
mod gate_record_production_mount_tests {
    //! Production-shape mount coverage for the `/gate-records` alias: drives the
    //! `GateRecordStore` seam over the real `wrap_scoped`/`invocation_mount_view`
    //! wiring. Pins two things: the alias is actually registered in
    //! [`PER_USER_ALIASES`] (an unregistered alias fails every save with
    //! `MountNotFound`, making the store unusable in production), and the
    //! per-tenant path rewriting keeps identically-shaped refs from colliding
    //! across tenants.
    use super::*;
    use ironclaw_filesystem::InMemoryBackend;
    use ironclaw_host_api::{
        GateRecord, GateRef, InvocationId, ProjectId, SafeSummary, TenantId, UserId,
    };
    use ironclaw_run_state::{FilesystemGateRecordStore, GateRecordStore};

    fn scope(tenant: &str, user: &str) -> ResourceScope {
        ResourceScope {
            tenant_id: TenantId::new(tenant).unwrap(),
            user_id: UserId::new(user).unwrap(),
            agent_id: None,
            project_id: Some(ProjectId::new("default").unwrap()),
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        }
    }

    #[tokio::test]
    async fn gate_records_save_and_load_through_the_production_mount_view() {
        let scoped = wrap_scoped(Arc::new(InMemoryBackend::new()));
        let store = FilesystemGateRecordStore::new(scoped);
        let record = GateRecord::Approval {
            summary: SafeSummary::new("awaiting decision").unwrap(),
        };
        let gate_ref = GateRef::new();
        let scope_a = scope("tenant_a", "alice");

        // The alias must resolve (a missing PER_USER_ALIASES entry fails here
        // with MountNotFound), and the owner must read the record back.
        store
            .save(scope_a.clone(), gate_ref, record.clone())
            .await
            .unwrap();
        assert_eq!(store.load(&scope_a, gate_ref).await.unwrap(), Some(record));

        // Structural tenant isolation: same ref, different tenant → unknown.
        let scope_b = scope("tenant_b", "bob");
        assert_eq!(store.load(&scope_b, gate_ref).await.unwrap(), None);
    }
}
