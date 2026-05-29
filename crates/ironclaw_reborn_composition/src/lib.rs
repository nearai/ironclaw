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

mod auth;
mod available_extensions;
mod bundled_skills;
mod default_system_prompt;
mod error;
mod extension_installation_store;
mod extension_lifecycle;
mod extension_lifecycle_capabilities;
mod extension_lifecycle_command;
mod factory;
mod google_oauth;
mod gsuite;
mod input;
mod lifecycle;
#[cfg(feature = "root-llm-provider")]
mod llm_catalog;
mod local_dev_capability_policy;
mod local_dev_mounts;
mod local_runtime_profile;
#[cfg(feature = "webui-v2-beta")]
mod product_auth_serve;
mod product_live_adapters;
#[cfg(any(feature = "libsql", feature = "postgres"))]
mod production_runtime_policy;
mod profile;
mod projection;
#[cfg(feature = "root-llm-provider")]
mod provider_admin;
#[cfg(feature = "root-llm-provider")]
mod provider_admin_product_command;
mod readiness;
mod runtime;
mod runtime_input;
mod skill_listing;
mod webui;
#[cfg(feature = "webui-v2-beta")]
mod webui_body_limit;
#[cfg(feature = "webui-v2-beta")]
mod webui_rate_limit;
#[cfg(feature = "webui-v2-beta")]
mod webui_route_match;
#[cfg(feature = "webui-v2-beta")]
mod webui_serve;
#[cfg(feature = "webui-v2-beta")]
mod webui_ws_origin;

pub use auth::{
    RebornAuthContinuationDispatcher, RebornAuthProductError, RebornCredentialLifecycleError,
    RebornManualTokenChallenge, RebornManualTokenError, RebornManualTokenSetupRequest,
    RebornManualTokenSubmitRequest, RebornManualTokenSubmitResponse, RebornOAuthCallbackError,
    RebornOAuthCallbackOutcome, RebornOAuthCallbackRequest, RebornOAuthCallbackResponse,
    RebornProductAuthServicePorts, RebornProductAuthServices,
};
pub use error::RebornBuildError;
pub use extension_lifecycle_command::{
    RebornExtensionLifecycleCommand, RebornExtensionLifecycleCommandError,
    execute_reborn_extension_lifecycle_command, render_reborn_extension_lifecycle_response,
};
pub use factory::{RebornServices, build_reborn_services};
pub use gsuite::{bundled_gsuite_extension_packages, bundled_gsuite_first_party_handlers};
pub use input::{OAuthClientConfig, RebornBuildInput, RebornRuntimeProcessBinding};
pub use ironclaw_product_workflow::{
    LifecycleExtensionSource, LifecycleExtensionSummary, LifecyclePhase, LifecycleProductPayload,
    LifecycleProductResponse,
};
pub use ironclaw_skills::{
    ManagedSkillSource as RebornSkillSource, SkillSummary as RebornSkillSummary,
    skill_summary_json as reborn_skill_summary_json,
};
#[cfg(feature = "root-llm-provider")]
pub use llm_catalog::{
    RebornLlmCatalogError, resolve_against_registry, resolve_llm_selection_against_catalog,
    resolve_reborn_runtime_llm,
};
pub use local_runtime_profile::{
    RebornLocalRuntimeProfileError, RebornLocalRuntimeProfileOptions, local_dev_runtime_policy,
    local_dev_yolo_runtime_policy, local_runtime_build_input,
    local_runtime_build_input_with_options,
};
pub use product_live_adapters::{
    ProductLiveCapabilityAuthorityResolver, ProductLiveCapabilityIo, ProductLiveModelRouteSettings,
    ProductLivePlannedRuntimeAdapterConfig, ProductLivePlannedRuntimeAdapterError,
    ProductLivePlannedRuntimeAdapters, ProductLiveVisibleCapabilityRequestConfig,
    capability_allowlist, visible_capability_request_for_run,
};
#[cfg(any(feature = "libsql", feature = "postgres"))]
pub use production_runtime_policy::RebornProductionRuntimePolicy;
pub use profile::{RebornCompositionProfile, RebornCompositionProfileParseError};
#[cfg(feature = "root-llm-provider")]
pub use provider_admin::{
    RebornModelRoutesState, RebornProviderAdmin, RebornProviderAdminError, RebornProviderInfo,
    RebornProviderList, RebornProviderMetadata, RebornProviderSelection, RebornProviderStatus,
    RebornProviderWriteOutcome, RebornV1State,
};
#[cfg(feature = "root-llm-provider")]
pub use provider_admin_product_command::RebornProviderAdminProductCommandService;
pub use readiness::{RebornFacadeReadiness, RebornReadiness, RebornReadinessState};
pub use runtime::{
    AssistantReply, ConversationId, RebornRuntime, RebornRuntimeError, RebornSkillActivation,
    RebornSkillActivationMode, RebornSkillAsset, RebornSkillBundle, RebornSkillExecutionPlan,
    RebornSkillExecutionResult, RebornSkillSourceKind, build_reborn_runtime,
};
#[cfg(feature = "root-llm-provider")]
pub use runtime_input::ResolvedRebornLlm;
pub use runtime_input::{
    DEFAULT_TURN_RUNNER_HEARTBEAT_INTERVAL, DEFAULT_TURN_RUNNER_POLL_INTERVAL, PollSettings,
    RebornRuntimeIdentity, RebornRuntimeInput, TurnRunnerSettings,
};
pub use skill_listing::{RebornSkillListError, list_reborn_local_skills};
pub use webui::{RebornWebuiBundle, build_webui_services};
#[cfg(feature = "webui-v2-beta")]
pub use webui_rate_limit::RateLimitConfigError;
#[cfg(feature = "webui-v2-beta")]
pub use webui_serve::{
    PublicRouteMount, WebuiAuthenticator, WebuiServeConfig, WebuiServeConfigError, WebuiServeError,
    webui_v2_app,
};

/// Re-exported identity vocabulary host binaries need to construct
/// [`WebuiServeConfig`] (and any other public type on this crate whose
/// signature mentions a host-api identity). Kept narrow on purpose —
/// the composition CLAUDE.md says "Expose facade-shaped handles only";
/// these four newtypes are the WebUI gateway's host-identity facade.
#[cfg(feature = "webui-v2-beta")]
pub mod host_api {
    pub use ironclaw_host_api::{AgentId, ProjectId, TenantId, UserId};
}

/// Reborn model purpose slot names exposed for diagnostic callers.
///
/// This keeps CLI diagnostics on the composition boundary instead of making
/// the CLI mirror `ironclaw_reborn::model_routes::ModelSlot`.
pub fn reborn_model_slot_names() -> Vec<&'static str> {
    ironclaw_reborn::model_routes::ModelSlot::all()
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
    let mut registry = ironclaw_reborn::driver_registry::DriverRegistry::new();
    let text_only_driver = RebornRuntimeComponentStatus::from_result(
        ironclaw_reborn::planned_driver_factory::register_default_text_only_driver(
            &mut registry,
            ironclaw_reborn::text_loop_driver::TextOnlyModelReplyDriverConfig::default(),
        ),
    );
    let family_registry = ironclaw_reborn::app_loop_family::build_loop_family_registry();
    let planned_driver = match &family_registry {
        Ok(family_registry) => RebornRuntimeComponentStatus::from_result(
            ironclaw_reborn::planned_driver_factory::register_default_planned_driver(
                &mut registry,
                Arc::clone(family_registry),
            ),
        ),
        Err(error) => RebornRuntimeComponentStatus::Failed(error.to_string()),
    };
    let subagent_planned_driver = match family_registry {
        Ok(family_registry) => RebornRuntimeComponentStatus::from_result(
            ironclaw_reborn::planned_driver_factory::register_subagent_planned_driver(
                &mut registry,
                family_registry,
            ),
        ),
        Err(error) => RebornRuntimeComponentStatus::Failed(error.to_string()),
    };
    let planned_default_profile = RebornRuntimeComponentStatus::from_result(
        ironclaw_reborn::planned_driver_factory::default_planned_run_profile_resolver(),
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
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_filesystem::{RootFilesystem, ScopedFilesystem};
use ironclaw_host_api::ProcessBackendKind;
#[cfg(any(feature = "libsql", feature = "postgres"))]
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
use ironclaw_resources::ResourceError;
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_resources::{FilesystemResourceGovernorStore, PersistentResourceGovernor};
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
    PersistentResourceGovernor<FilesystemResourceGovernorStore<LibSqlRootFilesystem>>,
    FilesystemProcessStore<LibSqlRootFilesystem>,
    FilesystemProcessResultStore<LibSqlRootFilesystem>,
>;

#[cfg(feature = "postgres")]
pub type PostgresProductionHostRuntimeServices = HostRuntimeServices<
    PostgresRootFilesystem,
    PersistentResourceGovernor<FilesystemResourceGovernorStore<PostgresRootFilesystem>>,
    FilesystemProcessStore<PostgresRootFilesystem>,
    FilesystemProcessResultStore<PostgresRootFilesystem>,
>;

/// Consumer-store mount aliases that are tenant-rewritten by
/// [`invocation_mount_view`]. Each alias resolves to
/// `/tenants/<tenant>/users/<user>/<alias>` for the caller's scope, so
/// two tenants sharing one underlying [`RootFilesystem`] cannot collide
/// on identically-shaped paths.
#[cfg(any(feature = "libsql", feature = "postgres"))]
const PER_USER_ALIASES: &[&str] = &[
    "/processes",
    "/secrets",
    "/authorization",
    "/outbound",
    "/run-state",
    "/approvals",
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
#[cfg(any(feature = "libsql", feature = "postgres"))]
pub fn invocation_mount_view(
    scope: &ResourceScope,
) -> Result<MountView, ironclaw_host_api::HostApiError> {
    invocation_mount_view_for_segments(
        resource_scope_path_segment(scope.tenant_id.as_str()),
        resource_scope_path_segment(scope.user_id.as_str()),
    )
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
fn resource_scope_path_segment(value: &str) -> &str {
    if value == SYSTEM_RESERVED_ID {
        "__system__"
    } else {
        value
    }
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
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
        MountPermissions::read_write(),
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

/// Wrap `root` in a tenant-aware [`ScopedFilesystem`] whose resolver is
/// [`invocation_mount_view`]. The returned filesystem is the single
/// production handle — every consumer-store call routes per-scope
/// through this one instance.
#[cfg(any(feature = "libsql", feature = "postgres"))]
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
    pub secret_master_key: Option<SecretMaterial>,
    pub trust_policy: Arc<TPolicy>,
    pub runtime_policy: RebornProductionRuntimePolicy,
    pub turn_run_wake_notifier: Arc<TWake>,
    pub surface_version: CapabilitySurfaceVersion,
}

#[derive(Debug, Error)]
pub enum RebornCompositionError {
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
            )
            .await
            .unwrap();
        store
            .put(
                scope_b.clone(),
                handle.clone(),
                SecretMaterial::from("bob-secret".to_string()),
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
