#![forbid(unsafe_code)]

//! Reborn composition root.
//!
//! Main entry point:
//!
//! - [`build_runtime`] — full runtime assembly: deployment config + loop
//!   driver registry + LLM model gateway + turn-runner worker, spawned
//!   as one unit. This is the single entry
//!   point used by the standalone `ironclaw-reborn` binary and any
//!   future Reborn ingress.
//!
//! Downstream callers should not name internal Reborn types directly:
//! [`RebornRuntime`] exposes only task-level methods, so callers never
//! import `TurnCoordinator`, `SessionThreadService`, `HostManagedModel
//! Gateway`, etc.

use std::sync::Arc;

mod admin_secrets;
mod admin_token;
mod admin_user_directory;
#[cfg(test)]
mod approval_test_support;
mod automation;
mod blocked_auth_resume;
mod builtin_capability_policy;
pub mod deployment;
mod error;
mod extension_host;
mod factory;
mod google_oauth_secret_store;
mod input;
mod llm_admin;
mod local_dev_authorization;
mod local_dev_mounts;
mod observability;
mod operator_tool_catalog;
mod outbound;
mod product_auth;
mod production_runtime_policy;
mod profile_approval_authorization;
mod projection;
mod provider_identity;
mod readiness;
mod root;
mod runtime;
mod runtime_input;
mod runtime_profile_approval_policy;
mod storage_catalog;
mod support;
#[cfg(feature = "test-support")]
pub mod test_support;
mod trigger_fire_access;
mod turn_run_snapshot;
mod webui;

pub use admin_token::AdminApiTokenMinter;
pub use automation::facade::RebornAutomationProductFacade;
pub use automation::trigger_poller::PostSubmitDeliveryHook;
pub use error::RebornBuildError;
pub use extension_host::channel_host::{ChannelHostIdentity, GenericChannelHostAssembly};
pub use extension_host::channel_identity::{
    ChannelIdentityBindingConfig, channel_identity_binding_hook_factory,
};
pub use extension_host::extension_ingress::{
    ChannelInboundSinkConfig, ChannelIngressDrain, ChannelIngressRegistration,
    ExtensionIngressParts, ExtensionIngressRegistry, GenericChannelInboundSink,
    PostAdmissionObserver, StaticIngressSecrets, VerifiedEvidenceMint,
};
pub use extension_host::extension_ingress::{
    EXTENSION_INGRESS_ROUTE_PATTERN, extension_ingress_route_mount,
};
pub use extension_host::extension_lifecycle_command::{
    RebornExtensionLifecycleCommand, RebornExtensionLifecycleCommandError,
    execute_reborn_extension_lifecycle_command, render_reborn_extension_lifecycle_response,
};
pub use extension_host::first_party::{
    FirstPartyHandlerRegistrar, FirstPartyPackageAsset, FirstPartyPackageBundle,
    FirstPartyPackageOAuthSetup, FirstPartyPackageOnboarding, FirstPartyRegistrarContext,
};
pub use extension_host::skill_listing::{RebornSkillListError, list_reborn_local_skills};
#[cfg(feature = "test-support")]
pub use factory::AttachmentTestSupport;
#[cfg(feature = "test-support")]
pub use factory::ChannelHostAssemblyTestWiring;
pub use factory::LOCAL_DEV_SECRETS_MASTER_KEY_PATH;
#[cfg(feature = "test-support")]
pub use factory::RebornApprovalTestParts;
/// Crate-root alias for composition's own unit tests (the src `#[cfg(test)]`
/// modules that build a production trust policy from the concrete inventory).
#[cfg(test)]
pub(crate) use factory::builtin_first_party_trust_policy;
pub use factory::local_dev_db_path;
pub use factory::open_local_dev_secret_store;
/// Production first-party trust-policy builder over the neutral injected bundle
/// set. Public so integration tests (which convert the concrete first-party
/// inventory via the dev-dependency) can build the same trust policy the
/// production binary composes at build time.
pub use factory::production_first_party_trust_policy;
pub use factory::{KeychainMasterKeyOutcome, provision_local_dev_keychain_master_key};
pub use google_oauth_secret_store::{GoogleOauthSecretStore, GoogleOauthSecretStoreError};
pub use input::{
    ChannelExtensionBinding, OAuthClientConfig, RebornHostBindings, RebornRuntimeProcessBinding,
};
/// OAuth redirect-URI newtype re-exported so the `ironclaw_reborn_cli` binary
/// can name it without a direct `ironclaw_auth` dependency. Its
/// `runtime/mod.rs` parses the Google OAuth redirect URI from env into
/// `OAuthRedirectUri` when building the runtime input / OAuth client config. The
/// `reborn_cli_binary_crate_stays_separate_from_v1_root` boundary test (in
/// `ironclaw_architecture`) pins the CLI's workspace dependencies to exactly
/// the composition-facade set, so adding `ironclaw_auth` there would fail that
/// test — the type must travel through this facade instead.
pub use ironclaw_auth::OAuthRedirectUri;
#[cfg(any(test, feature = "test-support"))]
pub use ironclaw_auth::{
    AuthProductScope, AuthProviderId, AuthSurface, CredentialAccountId, CredentialAccountLabel,
    CredentialAccountStatus, CredentialOwnership, Timestamp,
};
/// First-party capability-wiring vocabulary re-exported so the assembling
/// binary (`ironclaw_reborn_cli`) can build the concrete GSuite / web tooling
/// [`FirstPartyHandlerRegistrar`]s and the credential-account visibility policy
/// without depending on `ironclaw_host_api` / `ironclaw_host_runtime` /
/// `ironclaw_auth` directly (extension-runtime DEL-7). The CLI's exact-deps
/// allow-list is frozen to the composition facade, so these types travel
/// through here.
pub use ironclaw_auth::{CredentialAccount, CredentialAccountSelectionRequest};
pub use ironclaw_host_api::{
    CapabilityId, HostApiError, NetworkScheme, NetworkTargetPattern, RuntimeCredentialRequirement,
    RuntimeCredentialRequirementSource, RuntimeCredentialTarget, RuntimeDispatchErrorKind,
    SecretHandle,
};
pub use ironclaw_host_api::{
    ExtensionId, RuntimeCredentialAccountSetup, RuntimeCredentialAuthRequirement, VendorId,
};
pub use ironclaw_host_runtime::{
    FirstPartyCapabilityError, FirstPartyCapabilityHandler, FirstPartyCapabilityRegistry,
    FirstPartyCapabilityRequest, FirstPartyCapabilityResult, ProductAuthProviderRuntimePorts,
};
pub use ironclaw_product::PreferenceTargetCodec;
/// Channel-adapter and codec contracts re-exported for the assembling
/// binary's [`ChannelExtensionBinding`] construction.
pub use ironclaw_product::{ChannelAdapter, NormalizedInboundMessage};
pub use ironclaw_product::{
    ChannelConnectionNoticePolicy, ChannelConnectionRequirement, ExtensionAccountSetupDescriptor,
    RebornChannelConnectStrategy,
};
pub use ironclaw_product::{
    LifecycleExtensionSource, LifecycleExtensionSummary, LifecycleProductPayload,
    LifecycleProductResponse, LifecycleSearchExtensionSummary,
};
pub use ironclaw_runner::failure_lane::{ALL_RUN_FAILURE_CATEGORIES, FailureLane, failure_lane};
pub use ironclaw_runner::runtime::DEFAULT_TURN_RUNNER_WORKER_COUNT;
pub use product_auth::credentials::runtime_credentials::RuntimeCredentialAccountVisibilityPolicy;
// Re-exported for `ironclaw_reborn_cli` (`runtime/mod.rs` turn-failure display):
// the CLI consumes composition as its facade and must not grow a direct
// `ironclaw_runner` edge for one summary helper. All other run-failure
// classifier items moved to `ironclaw_runner::{failure_lane, failure_summary,
// retry_disposition}` with consumers repointed (no path-preservation shims).
pub use ironclaw_runner::failure_summary::reborn_failure_summary_for_category;
pub use ironclaw_runtime_policy::{
    ResolveRequest as RuntimePolicyResolveRequest, resolve as resolve_runtime_policy,
};
pub use ironclaw_skills::{
    ManagedSkillSource as RebornSkillSource, SkillSummary as RebornSkillSummary,
    skill_summary_json as reborn_skill_summary_json,
};
pub use ironclaw_triggers::TriggerId;
pub use ironclaw_turns::TurnStatus;
pub use llm_admin::llm_catalog::{
    ProviderCatalogValidationError, RebornLlmCatalogError, resolve_against_registry,
    resolve_llm_selection_against_catalog, resolve_llm_selection_allow_missing_key,
    resolve_reborn_runtime_llm, validate_reborn_provider_catalog_contents,
};
pub use llm_admin::llm_config_service::{LlmReloadTrigger, RebornLlmConfigService};
pub use llm_admin::llm_key_store::{LlmKeyStore, LlmKeyStoreError};
pub use llm_admin::nearai_mcp::{
    NearAiMcpBootstrapConfig, NearAiMcpBootstrapConfigError, nearai_mcp_bootstrap_config_from_env,
};
pub use llm_admin::openai_compat_serve::build_openai_compat_route_mount;
// Re-exported for the host-owned `ironclaw_webui::webui_v2_app`
// (hoisted up from this crate): its bearer-auth middleware mints tenant-scoped
// verified-bearer evidence for protected OpenAI-compatible mounts. Ingress must
// not depend on `ironclaw_product` directly (architecture boundary), so
// it reaches this helper through composition's facade.
pub use deployment::{
    RebornRuntimeProfileError, RebornRuntimeProfileOptions, hosted_single_tenant_runtime_policy,
    hosted_single_tenant_volume_runtime_policy, local_dev_runtime_policy,
    local_dev_yolo_runtime_policy, local_runtime_build_input,
    local_runtime_build_input_with_options,
};
#[cfg(any(test, feature = "test-support"))]
pub use deployment::{local_dev_build_input, local_dev_build_input_with_profile};
pub use ironclaw_product::mark_bearer_token_verified_for_tenant;
pub use llm_admin::provider_admin::{
    DetectedEnvLlm, EXAMPLE_OVERLAY_PROVIDER_ID, ProviderMenuEntry, ProviderProbeOutcome,
    RebornModelRoutesState, RebornProviderAdmin, RebornProviderAdminError, RebornProviderInfo,
    RebornProviderList, RebornProviderMetadata, RebornProviderSelection, RebornProviderStatus,
    RebornProviderWriteOutcome, RebornV1State,
};
pub use llm_admin::provider_admin_product_command::RebornProviderAdminProductCommandService;
pub use llm_admin::provider_repo::{ProviderRepo, ProviderRepoError};
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
// Composition's facade re-exports the continuation dispatcher for its own
// downstream consumers (root test suites, the CLI) alongside the
// product-auth service surface that produces it.
pub use product_auth::api::auth::RebornAuthContinuationDispatcher;
pub use product_auth::api::auth::{
    RebornAuthProductError, RebornCredentialLifecycleError, RebornManualTokenChallenge,
    RebornManualTokenError, RebornManualTokenSetupRequest, RebornManualTokenSubmitRequest,
    RebornManualTokenSubmitResponse, RebornOAuthCallbackError, RebornOAuthCallbackOutcome,
    RebornOAuthCallbackRequest, RebornOAuthCallbackResponse, RebornProductAuthServicePorts,
    RebornProductAuthServices,
};
// Product-auth WebUI route-mount builders, exposed so the host-owned
// `ironclaw_webui::webui_v2_app` (moved up from this crate) can
// compose the Reborn-native product-auth surface into the WebChat v2 router.
pub use product_auth::serve::{
    ProductAuthRouteMount, ProductAuthRouteState, product_auth_route_mount,
};
pub use production_runtime_policy::RebornProductionRuntimePolicy;
pub use provider_identity::{
    ProviderIdentityActorResolver, RebornIdentityProviderId, RebornIdentityProviderUserId,
    RebornUserIdentityBinding, RebornUserIdentityBindingDeleteStore,
    RebornUserIdentityBindingError, RebornUserIdentityBindingStore, RebornUserIdentityLookup,
    RebornUserIdentityLookupError, installation_scoped_provider_user_id,
};
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
    RebornSkillExecutionResult, RebornSkillSourceKind, build_reborn_runtime, build_runtime,
};
pub use runtime_input::{
    DEFAULT_TURN_RUNNER_HEARTBEAT_INTERVAL, DEFAULT_TURN_RUNNER_POLL_INTERVAL,
    KeepaliveSweepSettings, PollSettings, RebornRuntimeIdentity, RebornRuntimeInput,
    TriggerFireAccessCheck, TriggerFireAccessChecker, TriggerFireAccessDecision,
    TriggerFireAccessError, TriggerFireAccessGrant, TriggerFireAccessPolicy, TriggerPollerSettings,
    TurnRunnerSettings,
};
pub use runtime_input::{RebornProviderFactory, ResolvedRebornLlm};
pub use webui::facade::{RebornWebuiBundle, build_webui_services};
// Host-supplied route-mount vocabulary shared with composition's own route
// builders (nearai login, OpenAI-compat) and the host-owned gateway assembly
// in `ironclaw_webui`. The `WebuiServeConfig` / `webui_v2_app`
// / `WebuiAuthenticator` surface moved up into that ingress crate.
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

/// Canonical Reborn identity resolver vocabulary (issue #4381): the one
/// boundary that maps every external identity — WebUI OAuth logins and
/// external channel/product actors — to a stable `UserId` before runtime
/// state is touched. Only the resolver trait, request, surface, and error
/// types are re-exported so host wiring (`ironclaw-reborn serve`, the CLI
/// `UserDirectory` adapter) depends on the facade vocabulary, never on
/// `ironclaw_reborn_identity` directly. The concrete filesystem-backed store
/// stays private to this composition layer (composition CLAUDE.md: "keep
/// lower substrate handles private").
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
#[cfg(any(test, feature = "test-support"))]
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
use ironclaw_filesystem::LibSqlRootFilesystem;
use ironclaw_filesystem::PostgresRootFilesystem;
use ironclaw_filesystem::{RootFilesystem, ScopedFilesystem};
use ironclaw_host_api::ProcessBackendKind;
use ironclaw_host_api::{
    MountAlias, MountGrant, MountPermissions, MountView, ResourceScope, VirtualPath,
};
use ironclaw_host_runtime::{CapabilitySurfaceVersion, HostRuntimeServices};
use ironclaw_processes::{FilesystemProcessResultStore, FilesystemProcessStore};
use ironclaw_reborn_event_store::RebornEventStoreConfig;
use ironclaw_reborn_event_store::RebornEventStoreError;
use ironclaw_resources::FilesystemResourceGovernor;
use ironclaw_resources::ResourceError;
use ironclaw_run_state::RunStateError;
use ironclaw_secrets::SecretError;
use ironclaw_secrets::SecretMaterial;
use ironclaw_trust::TrustPolicy;
use ironclaw_turns::TurnError;
use ironclaw_turns::TurnRunWakeNotifier;
use thiserror::Error;

pub type LibSqlProductionHostRuntimeServices = HostRuntimeServices<
    LibSqlRootFilesystem,
    FilesystemResourceGovernor<LibSqlRootFilesystem>,
    FilesystemProcessStore<LibSqlRootFilesystem>,
    FilesystemProcessResultStore<LibSqlRootFilesystem>,
>;

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
    "/product-results",
    "/processes",
    "/secrets",
    "/authorization",
    "/outbound",
    "/run-state",
    "/approvals",
    "/gate-records",
    "/replay-payloads",
    "/threads",
    "/conversations",
    "/turns",
    "/checkpoint-state",
    "/resources",
    "/engine",
    "/skills",
    "/workspace",
];

/// The canonical global `/system` subroots, each exposed as its own read-only
/// alias resolving to the same tenant-independent `VirtualPath`. Single source
/// for the mount-grant wiring and its resolution test so the two cannot drift.
const SYSTEM_SUBROOTS: [&str; 3] = ["/system/settings", "/system/extensions", "/system/skills"];

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

pub(crate) fn resource_scope_path_segment(value: &str) -> &str {
    if value == ironclaw_host_api::SYSTEM_RESERVED_ID {
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
    let mut grants = Vec::with_capacity(PER_USER_ALIASES.len() + 3);
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
    grants.push(MountGrant::new(
        MountAlias::new("/extension-admin-configuration")?,
        VirtualPath::new(format!(
            "/tenants/{tenant_id}/shared/extension-admin-configuration"
        ))?,
        MountPermissions::read_write_list_delete(),
    ));
    for system_subroot in SYSTEM_SUBROOTS {
        grants.push(MountGrant::new(
            MountAlias::new(system_subroot)?,
            VirtualPath::new(system_subroot)?,
            MountPermissions::read_only(),
        ));
    }
    MountView::new(grants)
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
pub fn open_reborn_postgres_pool(
    url: secrecy::SecretString,
) -> Result<deadpool_postgres::Pool, RebornCompositionError> {
    Ok(ironclaw_reborn_event_store::open_postgres_pool(url)?)
}

/// Open a PostgreSQL pool for Reborn production storage with an explicit
/// maximum connection count.
pub fn open_reborn_postgres_pool_with_max_size(
    url: secrecy::SecretString,
    max_size: usize,
) -> Result<deadpool_postgres::Pool, RebornCompositionError> {
    Ok(ironclaw_reborn_event_store::open_postgres_pool_with_max_size(url, max_size)?)
}

#[cfg(test)]
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

    #[test]
    fn invocation_mount_view_routes_admin_configuration_to_tenant_shared_storage() {
        let scope = sample_scope();
        let view = invocation_mount_view(&scope).unwrap();
        let resolved = view
            .resolve(
                &ScopedPath::new("/extension-admin-configuration/groups/extension.slack.json")
                    .unwrap(),
            )
            .unwrap();
        assert_eq!(
            resolved.as_str(),
            &format!(
                "/tenants/{}/shared/extension-admin-configuration/groups/extension.slack.json",
                scope.tenant_id.as_str(),
            ),
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
        for system_subroot in SYSTEM_SUBROOTS {
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

#[cfg(test)]
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
            tenant_id: TenantId::new(tenant).unwrap(), // safety: fixed-valid test fixture
            user_id: UserId::new(user).unwrap(),       // safety: fixed-valid test fixture
            agent_id: Some(AgentId::new("github").unwrap()),
            project_id: Some(ProjectId::new("default").unwrap()), // safety: fixed-valid test fixture
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
            tenant_id: TenantId::new(tenant).unwrap(), // safety: fixed-valid test fixture
            user_id: UserId::new(user).unwrap(),       // safety: fixed-valid test fixture
            agent_id: None,
            project_id: Some(ProjectId::new("default").unwrap()), // safety: fixed-valid test fixture
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
            summary: SafeSummary::new("awaiting decision").unwrap(), // safety: fixed-valid test fixture
        };
        let gate_ref = GateRef::new();
        let scope_a = scope("tenant_a", "alice");

        // The alias must resolve (a missing PER_USER_ALIASES entry fails here
        // with MountNotFound), and the owner must read the record back.
        store
            .save(scope_a.clone(), gate_ref, record.clone())
            .await
            .unwrap(); // safety: test assertion on an in-memory store
        assert_eq!(store.load(&scope_a, gate_ref).await.unwrap(), Some(record)); // safety: test assertion

        // Structural tenant isolation: same ref, different tenant → unknown.
        let scope_b = scope("tenant_b", "bob");
        assert_eq!(store.load(&scope_b, gate_ref).await.unwrap(), None); // safety: test assertion
    }
}
