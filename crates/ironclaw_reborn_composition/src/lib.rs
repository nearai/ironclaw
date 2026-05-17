#![forbid(unsafe_code)]

//! Minimal Reborn production composition root.
//!
//! This crate intentionally wires substrate services only. Product/AppBuilder
//! integration belongs in later slices.

mod error;
mod factory;
mod input;
mod profile;
mod readiness;

pub use error::RebornBuildError;
pub use factory::{RebornServices, build_reborn_services};
pub use input::RebornBuildInput;
pub use profile::{RebornCompositionProfile, RebornCompositionProfileParseError};
pub use readiness::{RebornFacadeReadiness, RebornReadiness, RebornReadinessState};

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
#[cfg(feature = "libsql")]
use ironclaw_resources::LibSqlResourceGovernorStore;
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_resources::PersistentResourceGovernor;
#[cfg(feature = "postgres")]
use ironclaw_resources::PostgresResourceGovernorStore;
use ironclaw_resources::ResourceError;
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
    PersistentResourceGovernor<LibSqlResourceGovernorStore>,
    FilesystemProcessStore<LibSqlRootFilesystem>,
    FilesystemProcessResultStore<LibSqlRootFilesystem>,
>;

#[cfg(feature = "postgres")]
pub type PostgresProductionHostRuntimeServices = HostRuntimeServices<
    PostgresRootFilesystem,
    PersistentResourceGovernor<PostgresResourceGovernorStore>,
    FilesystemProcessStore<PostgresRootFilesystem>,
    FilesystemProcessResultStore<PostgresRootFilesystem>,
>;

/// Build the default single-tenant [`MountView`] for production composition.
///
/// Wires the canonical consumer-store aliases (`/processes`, `/secrets`,
/// `/authorization`, `/outbound`, `/engine`) to top-level
/// [`VirtualPath`] roots and grants full per-user-owner permissions.
///
/// This is the **single-tenant** default: every alias maps to the
/// root-level prefix with no `tenants/<tenant_id>/users/<user_id>/...`
/// rewriting. Multi-tenant deployments build a per-invocation MountView
/// that points each alias to a tenant/user-scoped subtree of the same
/// underlying [`RootFilesystem`] — see
/// `docs/plans/2026-05-16-scoped-filesystem-tenant-isolation.md`.
/// Per-user-owner consumer aliases (each mount gets full r/w/l/d).
/// Kept as a constant so both [`default_singleton_mount_view`] and
/// [`invocation_mount_view`] can share the alias list — adding a new
/// consumer crate is a single-line change here.
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
    "/engine",
];

/// Single-tenant default [`MountView`] used by long-lived production
/// composition. Every consumer-store alias resolves to a top-level
/// [`VirtualPath`] root (`/processes` → `/processes`) — no tenant
/// rewriting, just permission scoping and an alias→VirtualPath layer.
///
/// Use [`invocation_mount_view`] instead when constructing per-request
/// services that need cross-tenant isolation. The current long-lived
/// composition holds one set of consumer stores for the whole process
/// lifetime, which is correct for single-tenant deployments and a
/// known follow-up for multi-tenant ones (see
/// `docs/plans/2026-05-16-scoped-filesystem-tenant-isolation.md`
/// "Open Question 1").
#[cfg(any(feature = "libsql", feature = "postgres"))]
pub(crate) fn default_singleton_mount_view() -> Result<MountView, ironclaw_host_api::HostApiError> {
    let mut grants = Vec::with_capacity(PER_USER_ALIASES.len() + 2);
    for alias in PER_USER_ALIASES {
        grants.push(MountGrant::new(
            MountAlias::new(*alias)?,
            VirtualPath::new(*alias)?,
            MountPermissions::read_write_list_delete(),
        ));
    }
    // `/tenant-shared`: shared between users/agents in the same tenant.
    // In the singleton (non-tenant) view it points to a fixed root with
    // read+write+list (no delete — tenants can mutate but not erase).
    grants.push(MountGrant::new(
        MountAlias::new("/tenant-shared")?,
        VirtualPath::new("/tenant-shared")?,
        MountPermissions::read_write(),
    ));
    // `/system/{settings,extensions,skills}`: globally readable system data.
    // Each subroot is exposed as its own alias (rather than a unified
    // `/system` mount) because `VirtualPath` reserves the three canonical
    // subroots — see `docs/reborn/contracts/storage-placement.md`. ACL is
    // read-only at the `ScopedFilesystem` layer; writes are rejected
    // before any backend dispatch.
    for system_subroot in ["/system/settings", "/system/extensions", "/system/skills"] {
        grants.push(MountGrant::new(
            MountAlias::new(system_subroot)?,
            VirtualPath::new(system_subroot)?,
            MountPermissions::read_only(),
        ));
    }
    MountView::new(grants)
}

/// Per-invocation [`MountView`] that rewrites consumer-store aliases to
/// `/tenants/<tenant>/users/<user>/<alias>` virtual paths.
///
/// Used by request handlers that need to construct tenant-scoped
/// consumer stores. The returned view, fed to
/// [`ScopedFilesystem::new`], makes every consumer crate's `put`/`get`
/// land under a tenant/user-private subtree of the same underlying
/// [`RootFilesystem`] — cross-tenant isolation is structural rather
/// than a convention.
///
/// `/tenant-shared` resolves to `/tenants/<tenant>/shared` (full
/// per-tenant r/w/l), `/system` to `/system` (globally read-only).
///
/// The single-tenant production composition uses
/// [`default_singleton_mount_view`] instead.
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

#[cfg(any(feature = "libsql", feature = "postgres"))]
pub(crate) fn wrap_scoped<F>(
    root: Arc<F>,
) -> Result<Arc<ScopedFilesystem<F>>, ironclaw_host_api::HostApiError>
where
    F: RootFilesystem,
{
    let view = default_singleton_mount_view()?;
    Ok(Arc::new(ScopedFilesystem::new(root, view)))
}

/// Wrap `root` in a per-invocation [`ScopedFilesystem`] that resolves
/// consumer-store aliases under `/tenants/<tenant>/users/<user>/…`.
///
/// Counterpart to [`wrap_scoped`] for request handlers that have a
/// `ResourceScope` in hand and need a tenant-isolated filesystem view.
/// The single underlying [`RootFilesystem`] is shared; per-tenant
/// separation comes from the rewritten target paths in the view.
#[cfg(any(feature = "libsql", feature = "postgres"))]
pub fn wrap_scoped_for_invocation<F>(
    root: Arc<F>,
    scope: &ResourceScope,
) -> Result<Arc<ScopedFilesystem<F>>, ironclaw_host_api::HostApiError>
where
    F: RootFilesystem,
{
    let view = invocation_mount_view(scope)?;
    Ok(Arc::new(ScopedFilesystem::new(root, view)))
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

    let scoped_filesystem = wrap_scoped(Arc::clone(&filesystem))?;
    let process_services = ProcessServices::filesystem(Arc::clone(&scoped_filesystem));

    let secret_store =
        build_filesystem_secret_store(Arc::clone(&scoped_filesystem), config.secret_master_key)
            .await?;

    let resource_store = LibSqlResourceGovernorStore::new(Arc::clone(&config.database));
    resource_store.run_migrations().await?;
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
    .with_capability_leases(capability_leases)
    .with_secret_store(Arc::clone(&secret_store))
    .with_turn_run_wake_notifier(config.turn_run_wake_notifier)
    .with_filesystem_run_state(Arc::clone(&scoped_filesystem))
    .with_filesystem_turn_state_store(Arc::clone(&scoped_filesystem))
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

    let scoped_filesystem = wrap_scoped(Arc::clone(&filesystem))?;
    let process_services = ProcessServices::filesystem(Arc::clone(&scoped_filesystem));

    let secret_store =
        build_filesystem_secret_store(Arc::clone(&scoped_filesystem), config.secret_master_key)
            .await?;

    let resource_store = PostgresResourceGovernorStore::new(config.pool.clone());
    resource_store.run_migrations().await?;
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
    .with_capability_leases(capability_leases)
    .with_secret_store(Arc::clone(&secret_store))
    .with_turn_run_wake_notifier(config.turn_run_wake_notifier)
    .with_filesystem_run_state(Arc::clone(&scoped_filesystem))
    .with_filesystem_turn_state_store(Arc::clone(&scoped_filesystem))
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
    store.verify_can_decrypt_existing_secrets().await?;
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
    fn default_singleton_mount_view_has_all_consumer_aliases() {
        let view = default_singleton_mount_view().unwrap();
        // Per-user aliases.
        for alias in PER_USER_ALIASES {
            let resolved = view
                .resolve(&ScopedPath::new(format!("{alias}/foo")).unwrap())
                .unwrap();
            assert_eq!(resolved.as_str(), &format!("{alias}/foo"));
        }
        // Shared carve-out + the three canonical /system subroots.
        view.resolve(&ScopedPath::new("/tenant-shared/foo").unwrap())
            .unwrap();
        for system_subroot in ["/system/settings", "/system/extensions", "/system/skills"] {
            view.resolve(&ScopedPath::new(format!("{system_subroot}/foo")).unwrap())
                .unwrap();
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
}
