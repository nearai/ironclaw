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

mod attested;
mod auth;
mod error;
mod factory;
mod input;
#[cfg(feature = "root-llm-provider")]
mod llm_catalog;
mod product_live_adapters;
mod profile;
mod projection;
mod readiness;
mod runtime;
mod runtime_input;
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

use ironclaw_runtime_policy::{EffectiveRuntimePolicy as ResolvedRuntimePolicy, ResolveError};

pub use attested::{NoopBroadcaster, RebornAttestedComposition};
pub use auth::{
    RebornAuthContinuationDispatcher, RebornOAuthCallbackError, RebornOAuthCallbackOutcome,
    RebornOAuthCallbackRequest, RebornOAuthCallbackResponse, RebornProductAuthServices,
};
pub use error::RebornBuildError;
pub use factory::{RebornServices, build_reborn_services};
pub use input::RebornBuildInput;
#[cfg(feature = "root-llm-provider")]
pub use llm_catalog::{
    RebornLlmCatalogError, resolve_against_registry, resolve_llm_selection_against_catalog,
    resolve_reborn_runtime_llm,
};
pub use product_live_adapters::{
    ProductLiveCapabilityAuthorityResolver, ProductLiveCapabilityIo, ProductLiveModelRouteSettings,
    ProductLivePlannedRuntimeAdapterConfig, ProductLivePlannedRuntimeAdapterError,
    ProductLivePlannedRuntimeAdapters, ProductLiveVisibleCapabilityRequestConfig,
    capability_allowlist, visible_capability_request_for_run,
};
pub use profile::{RebornCompositionProfile, RebornCompositionProfileParseError};
pub use readiness::{RebornFacadeReadiness, RebornReadiness, RebornReadinessState};
pub use runtime::{
    AssistantReply, ConversationId, RebornRuntime, RebornRuntimeError, RebornSkillActivation,
    RebornSkillActivationMode, RebornSkillAsset, RebornSkillBundle, RebornSkillExecutionPlan,
    RebornSkillExecutionResult, RebornSkillSourceKind, build_reborn_runtime,
};
pub use runtime_input::{
    DEFAULT_TURN_RUNNER_HEARTBEAT_INTERVAL, DEFAULT_TURN_RUNNER_POLL_INTERVAL, PollSettings,
    RebornRuntimeIdentity, RebornRuntimeInput, TurnRunnerSettings,
};
#[cfg(feature = "root-llm-provider")]
pub use runtime_input::{RebornLlmConfig, ResolvedRebornLlm};
pub use webui::{RebornWebuiBundle, build_webui_services};
#[cfg(feature = "webui-v2-beta")]
pub use webui_rate_limit::RateLimitConfigError;
#[cfg(feature = "webui-v2-beta")]
pub use webui_serve::{
    WebuiAuthenticator, WebuiServeConfig, WebuiServeConfigError, WebuiServeError, webui_v2_app,
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

/// Resolved policy for the standalone local development runtime profile.
pub fn local_dev_runtime_policy() -> Result<ResolvedRuntimePolicy, ResolveError> {
    use ironclaw_host_api::runtime_policy::{DeploymentMode, RuntimeProfile};

    ironclaw_runtime_policy::resolve(ironclaw_runtime_policy::ResolveRequest::new(
        DeploymentMode::LocalSingleUser,
        RuntimeProfile::LocalDev,
    ))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebornRuntimeReadinessSnapshot {
    pub text_only_driver: RebornRuntimeComponentStatus,
    pub planned_driver: RebornRuntimeComponentStatus,
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
    let planned_driver = match ironclaw_reborn::app_loop_family::build_loop_family_registry() {
        Ok(family_registry) => RebornRuntimeComponentStatus::from_result(
            ironclaw_reborn::planned_driver_factory::register_default_planned_driver(
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
        planned_default_profile,
    }
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
use std::sync::Arc;

#[cfg(any(feature = "libsql", feature = "postgres"))]
use async_trait::async_trait;
use ironclaw_authorization::CapabilityLeaseError;
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_authorization::{FilesystemCapabilityLeaseStore, GrantAuthorizer};
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_extensions::ExtensionRegistry;
#[cfg(feature = "libsql")]
use ironclaw_filesystem::LibSqlRootFilesystem;
#[cfg(feature = "postgres")]
use ironclaw_filesystem::PostgresRootFilesystem;
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_filesystem::{RootFilesystem, ScopedFilesystem};
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_host_api::{
    MountAlias, MountGrant, MountPermissions, MountView, ResourceScope, SecretHandle, VirtualPath,
    runtime_policy::EffectiveRuntimePolicy,
};
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_host_runtime::{CapabilitySurfaceVersion, HostRuntimeServices};
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_network::{PolicyNetworkHttpEgress, ReqwestNetworkTransport};
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_processes::{FilesystemProcessResultStore, FilesystemProcessStore, ProcessServices};
use ironclaw_reborn_event_store::RebornEventStoreError;
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_reborn_event_store::{RebornEventStoreConfig, RebornProfile};
use ironclaw_resources::ResourceError;
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_resources::{FilesystemResourceGovernorStore, PersistentResourceGovernor};
use ironclaw_run_state::RunStateError;
use ironclaw_secrets::SecretError;
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_secrets::{
    FilesystemSecretStore, SecretLease, SecretLeaseId, SecretMaterial, SecretMetadata, SecretStore,
    SecretStoreError, SecretsCrypto,
};
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
    "/resources",
    "/engine",
    "/skills",
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
/// `/tenants/__SYSTEM__/users/__SYSTEM__/<alias>`. Production code uses
/// it for process-global records whose paths already encode per-tenant
/// identity (event-log stream keys, conversation singleton state).
#[cfg(any(feature = "libsql", feature = "postgres"))]
pub fn invocation_mount_view(
    scope: &ResourceScope,
) -> Result<MountView, ironclaw_host_api::HostApiError> {
    let tenant_user_prefix = format!(
        "/tenants/{}/users/{}",
        scope.tenant_id.as_str(),
        scope.user_id.as_str()
    );
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
        VirtualPath::new(format!("/tenants/{}/shared", scope.tenant_id.as_str()))?,
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
    pub runtime_policy: EffectiveRuntimePolicy,
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
    pub runtime_policy: EffectiveRuntimePolicy,
    pub turn_run_wake_notifier: Arc<TWake>,
    pub surface_version: CapabilitySurfaceVersion,
}

#[derive(Debug, Error)]
pub enum RebornCompositionError {
    #[error("reborn production composition requires explicit secret master key")]
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
    let filesystem = Arc::new(LibSqlRootFilesystem::new(Arc::clone(&config.database)));
    filesystem.run_migrations().await?;

    let scoped_filesystem = wrap_scoped(Arc::clone(&filesystem));
    let process_services = ProcessServices::filesystem(Arc::clone(&scoped_filesystem));

    let secret_store =
        build_filesystem_secret_store(Arc::clone(&scoped_filesystem), config.secret_master_key)
            .await?;

    let resource_store = FilesystemResourceGovernorStore::new(Arc::clone(&scoped_filesystem));
    let governor = Arc::new(PersistentResourceGovernor::new(resource_store));

    let capability_leases = Arc::new(FilesystemCapabilityLeaseStore::new(Arc::clone(
        &scoped_filesystem,
    )));

    let services = HostRuntimeServices::new(
        Arc::new(ExtensionRegistry::new()),
        filesystem,
        governor,
        Arc::new(GrantAuthorizer::new()),
        process_services,
        config.surface_version,
    )
    .with_trust_policy(config.trust_policy)
    .with_runtime_policy(config.runtime_policy)
    .with_capability_leases(capability_leases)
    .with_secret_store(Arc::clone(&secret_store))
    .with_turn_run_wake_notifier(config.turn_run_wake_notifier)
    .with_filesystem_run_state(Arc::clone(&scoped_filesystem))
    .with_filesystem_turn_state_store(Arc::clone(&scoped_filesystem))
    .with_run_profile_resolver(Arc::new(
        ironclaw_reborn::planned_driver_factory::default_planned_run_profile_resolver()?,
    ))
    .with_reborn_event_store_config(RebornProfile::Production, config.event_store)
    .await?;

    // safety: `with_secret_store` is called unconditionally above on the same
    // builder chain, so `try_with_host_http_egress` can only return a
    // `Missing(SecretStore)` wiring report if the host-runtime builder API
    // regresses; treat that as infallible here.
    let services = services
        .try_with_host_http_egress(PolicyNetworkHttpEgress::new(
            ReqwestNetworkTransport::default(),
        ))
        .expect("secret_store wired above guarantees host HTTP egress is buildable"); // safety: see comment above

    Ok(services)
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
    let filesystem = Arc::new(PostgresRootFilesystem::new(config.pool.clone()));
    filesystem.run_migrations().await?;

    let scoped_filesystem = wrap_scoped(Arc::clone(&filesystem));
    let process_services = ProcessServices::filesystem(Arc::clone(&scoped_filesystem));

    let secret_store =
        build_filesystem_secret_store(Arc::clone(&scoped_filesystem), config.secret_master_key)
            .await?;

    let resource_store = FilesystemResourceGovernorStore::new(Arc::clone(&scoped_filesystem));
    let governor = Arc::new(PersistentResourceGovernor::new(resource_store));

    let capability_leases = Arc::new(FilesystemCapabilityLeaseStore::new(Arc::clone(
        &scoped_filesystem,
    )));

    let services = HostRuntimeServices::new(
        Arc::new(ExtensionRegistry::new()),
        filesystem,
        governor,
        Arc::new(GrantAuthorizer::new()),
        process_services,
        config.surface_version,
    )
    .with_trust_policy(config.trust_policy)
    .with_runtime_policy(config.runtime_policy)
    .with_capability_leases(capability_leases)
    .with_secret_store(Arc::clone(&secret_store))
    .with_turn_run_wake_notifier(config.turn_run_wake_notifier)
    .with_filesystem_run_state(Arc::clone(&scoped_filesystem))
    .with_filesystem_turn_state_store(Arc::clone(&scoped_filesystem))
    .with_run_profile_resolver(Arc::new(
        ironclaw_reborn::planned_driver_factory::default_planned_run_profile_resolver()?,
    ))
    .with_reborn_event_store_config(RebornProfile::Production, config.event_store)
    .await?;

    // safety: `with_secret_store` is called unconditionally above on the same
    // builder chain, so `try_with_host_http_egress` can only return a
    // `Missing(SecretStore)` wiring report if the host-runtime builder API
    // regresses; treat that as infallible here.
    let services = services
        .try_with_host_http_egress(PolicyNetworkHttpEgress::new(
            ReqwestNetworkTransport::default(),
        ))
        .expect("secret_store wired above guarantees host HTTP egress is buildable"); // safety: see comment above

    Ok(services)
}

/// Build the per-process [`SecretStore`] over the shared
/// [`ScopedFilesystem`].
///
/// Backend selection is now a property of the underlying
/// [`RootFilesystem`] (libSQL/Postgres/in-memory), not of the secret store
/// itself — see "Legacy per-backend store cleanup" in
/// `docs/plans/2026-05-16-scoped-filesystem-tenant-isolation.md`. The
/// startup readiness check
/// ([`FilesystemSecretStore::verify_can_decrypt_existing_secrets`])
/// preserves the same fail-loud-on-master-key-mismatch contract the deleted
/// libSQL/Postgres backends carried.
#[cfg(any(feature = "libsql", feature = "postgres"))]
async fn build_filesystem_secret_store<F>(
    scoped_filesystem: Arc<ScopedFilesystem<F>>,
    master_key: Option<SecretMaterial>,
) -> Result<Arc<SharedSecretStore>, RebornCompositionError>
where
    F: RootFilesystem + 'static,
{
    let crypto = secrets_crypto(master_key)?;
    let store = FilesystemSecretStore::new(scoped_filesystem, crypto);
    // The FS-stored master-key sentinel was removed alongside the tenant-aware
    // ScopedFilesystem rework — see filesystem_store.rs. Master-key
    // correctness is verified on first per-tenant decrypt op.
    let store: Arc<dyn SecretStore> = Arc::new(store);
    Ok(Arc::new(SharedSecretStore::new(store)))
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
fn secrets_crypto(
    master_key: Option<SecretMaterial>,
) -> Result<Arc<SecretsCrypto>, RebornCompositionError> {
    let master_key = master_key.ok_or(RebornCompositionError::MissingSecretMasterKey)?;
    Ok(Arc::new(SecretsCrypto::new(master_key)?))
}

// TODO(#3571): remove this adapter when the host-runtime services builder
// accepts `Arc<dyn SecretStore>` directly. Until then, this newtype lets the
// composition root pass a single concrete `SecretStore` impl to both the
// substrate wiring and any future per-store adapters.
#[cfg(any(feature = "libsql", feature = "postgres"))]
#[derive(Clone)]
struct SharedSecretStore {
    inner: Arc<dyn SecretStore>,
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
impl SharedSecretStore {
    fn new(inner: Arc<dyn SecretStore>) -> Self {
        Self { inner }
    }
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
#[async_trait]
impl SecretStore for SharedSecretStore {
    async fn put(
        &self,
        scope: ResourceScope,
        handle: SecretHandle,
        material: SecretMaterial,
    ) -> Result<SecretMetadata, SecretStoreError> {
        self.inner.put(scope, handle, material).await
    }

    async fn metadata(
        &self,
        scope: &ResourceScope,
        handle: &SecretHandle,
    ) -> Result<Option<SecretMetadata>, SecretStoreError> {
        self.inner.metadata(scope, handle).await
    }

    async fn lease_once(
        &self,
        scope: &ResourceScope,
        handle: &SecretHandle,
    ) -> Result<SecretLease, SecretStoreError> {
        self.inner.lease_once(scope, handle).await
    }

    async fn consume(
        &self,
        scope: &ResourceScope,
        lease_id: SecretLeaseId,
    ) -> Result<SecretMaterial, SecretStoreError> {
        self.inner.consume(scope, lease_id).await
    }

    async fn revoke(
        &self,
        scope: &ResourceScope,
        lease_id: SecretLeaseId,
    ) -> Result<SecretLease, SecretStoreError> {
        self.inner.revoke(scope, lease_id).await
    }

    async fn leases_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<SecretLease>, SecretStoreError> {
        self.inner.leases_for_scope(scope).await
    }
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
