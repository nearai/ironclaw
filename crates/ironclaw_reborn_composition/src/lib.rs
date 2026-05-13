//! Minimal Reborn production composition root.
//!
//! This crate intentionally wires substrate services only. Product/AppBuilder
//! integration belongs in later slices.

#[cfg(any(feature = "libsql", feature = "postgres"))]
use std::sync::Arc;

#[cfg(any(feature = "libsql", feature = "postgres"))]
use async_trait::async_trait;
use ironclaw_authorization::CapabilityLeaseError;
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_authorization::GrantAuthorizer;
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_extensions::ExtensionRegistry;
#[cfg(feature = "libsql")]
use ironclaw_filesystem::LibSqlRootFilesystem;
#[cfg(feature = "postgres")]
use ironclaw_filesystem::PostgresRootFilesystem;
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_host_api::{ResourceScope, SecretHandle};
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_host_runtime::{CapabilitySurfaceVersion, HostHttpEgressService, HostRuntimeServices};
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
#[cfg(feature = "libsql")]
use ironclaw_secrets::LibSqlSecretsStore;
#[cfg(feature = "postgres")]
use ironclaw_secrets::PostgresSecretsStore;
use ironclaw_secrets::SecretError;
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_secrets::{
    ScopedSecretsStoreAdapter, SecretLease, SecretLeaseId, SecretMaterial, SecretMetadata,
    SecretStore, SecretStoreError, SecretsCrypto,
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
    FilesystemProcessStore<'static, LibSqlRootFilesystem>,
    FilesystemProcessResultStore<'static, LibSqlRootFilesystem>,
>;

#[cfg(feature = "postgres")]
pub type PostgresProductionHostRuntimeServices = HostRuntimeServices<
    PostgresRootFilesystem,
    PersistentResourceGovernor<PostgresResourceGovernorStore>,
    FilesystemProcessStore<'static, PostgresRootFilesystem>,
    FilesystemProcessResultStore<'static, PostgresRootFilesystem>,
>;

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
    let secret_store =
        build_libsql_secret_store(Arc::clone(&config.database), config.secret_master_key).await?;

    let filesystem = Arc::new(LibSqlRootFilesystem::new(Arc::clone(&config.database)));
    filesystem.run_migrations().await?;

    let process_services = ProcessServices::filesystem(Arc::clone(&filesystem));

    let resource_store = LibSqlResourceGovernorStore::new(Arc::clone(&config.database));
    resource_store.run_migrations().await?;
    let governor = Arc::new(PersistentResourceGovernor::new(resource_store));

    let capability_leases = Arc::new(ironclaw_authorization::LibSqlCapabilityLeaseStore::new(
        Arc::clone(&config.database),
    ));
    capability_leases.run_migrations().await?;

    let mut services = HostRuntimeServices::new(
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
    .with_libsql_run_state_approval_store(Arc::clone(&config.database))
    .await?
    .with_libsql_turn_state_store(Arc::clone(&config.database))
    .await?
    .with_reborn_event_store_config(RebornProfile::Production, config.event_store)
    .await?;

    let http_egress = Arc::new(
        HostHttpEgressService::new(
            PolicyNetworkHttpEgress::new(ReqwestNetworkTransport::default()),
            (*secret_store).clone(),
        )
        .with_network_policy_store(services.network_policy_store())
        .with_secret_injection_store(services.secret_injection_store()),
    );
    services = services.with_host_http_egress_service(http_egress);

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
    let secret_store =
        build_postgres_secret_store(config.pool.clone(), config.secret_master_key).await?;

    let filesystem = Arc::new(PostgresRootFilesystem::new(config.pool.clone()));
    filesystem.run_migrations().await?;

    let process_services = ProcessServices::filesystem(Arc::clone(&filesystem));

    let resource_store = PostgresResourceGovernorStore::new(config.pool.clone());
    resource_store.run_migrations().await?;
    let governor = Arc::new(PersistentResourceGovernor::new(resource_store));

    let capability_leases = Arc::new(ironclaw_authorization::PostgresCapabilityLeaseStore::new(
        config.pool.clone(),
    ));
    capability_leases.run_migrations().await?;

    let mut services = HostRuntimeServices::new(
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
    .with_postgres_run_state_approval_store(config.pool.clone())
    .await?
    .with_postgres_turn_state_store(config.pool.clone())
    .await?
    .with_reborn_event_store_config(RebornProfile::Production, config.event_store)
    .await?;

    let http_egress = Arc::new(
        HostHttpEgressService::new(
            PolicyNetworkHttpEgress::new(ReqwestNetworkTransport::default()),
            (*secret_store).clone(),
        )
        .with_network_policy_store(services.network_policy_store())
        .with_secret_injection_store(services.secret_injection_store()),
    );
    services = services.with_host_http_egress_service(http_egress);

    Ok(services)
}

#[cfg(feature = "libsql")]
async fn build_libsql_secret_store(
    database: Arc<libsql::Database>,
    master_key: Option<SecretMaterial>,
) -> Result<Arc<SharedSecretStore>, RebornCompositionError> {
    let crypto = secrets_crypto(master_key)?;
    let backend = Arc::new(LibSqlSecretsStore::new(database, crypto));
    backend.run_migrations().await?;
    backend.verify_can_decrypt_existing_secrets().await?;
    let store: Arc<dyn SecretStore> = Arc::new(ScopedSecretsStoreAdapter::new(backend));
    Ok(Arc::new(SharedSecretStore::new(store)))
}

#[cfg(feature = "postgres")]
async fn build_postgres_secret_store(
    pool: deadpool_postgres::Pool,
    master_key: Option<SecretMaterial>,
) -> Result<Arc<SharedSecretStore>, RebornCompositionError> {
    let crypto = secrets_crypto(master_key)?;
    let backend = Arc::new(PostgresSecretsStore::new(pool, crypto));
    backend.run_migrations().await?;
    backend.verify_can_decrypt_existing_secrets().await?;
    let store: Arc<dyn SecretStore> = Arc::new(ScopedSecretsStoreAdapter::new(backend));
    Ok(Arc::new(SharedSecretStore::new(store)))
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
fn secrets_crypto(
    master_key: Option<SecretMaterial>,
) -> Result<Arc<SecretsCrypto>, RebornCompositionError> {
    let master_key = master_key.ok_or(RebornCompositionError::MissingSecretMasterKey)?;
    Ok(Arc::new(SecretsCrypto::new(master_key)?))
}

// TODO(#3571): remove this adapter when HostHttpEgressService accepts Arc<dyn SecretStore>.
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
