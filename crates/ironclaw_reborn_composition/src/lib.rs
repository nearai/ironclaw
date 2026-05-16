#![forbid(unsafe_code)]

//! Reborn composition root.
//!
//! Two entry points:
//!
//! - [`build_reborn_services`] — substrate-only facades (host runtime,
//!   turn coordinator). Useful when an outer harness wires the loop
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

mod error;
mod factory;
mod input;
mod product_live_adapters;
mod profile;
mod readiness;
mod runtime;
mod runtime_input;

pub use error::RebornBuildError;
pub use factory::{RebornServices, build_reborn_services};
pub use input::RebornBuildInput;
pub use product_live_adapters::{
    ProductLiveCapabilityIo, ProductLiveModelRouteSettings, ProductLivePlannedRuntimeAdapterConfig,
    ProductLivePlannedRuntimeAdapterError, ProductLivePlannedRuntimeAdapters,
    ProductLiveVisibleCapabilityRequestConfig, capability_allowlist,
    visible_capability_request_for_run,
};
pub use profile::{RebornCompositionProfile, RebornCompositionProfileParseError};
pub use readiness::{RebornFacadeReadiness, RebornReadiness, RebornReadinessState};
pub use runtime::{
    AssistantReply, ConversationId, RebornRuntime, RebornRuntimeError, build_reborn_runtime,
};
#[cfg(feature = "root-llm-provider")]
pub use runtime_input::RebornLlmConfig;
pub use runtime_input::{RebornRuntimeIdentity, RebornRuntimeInput, TurnRunnerSettings};

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
    .with_libsql_run_state_approval_store(Arc::clone(&config.database))
    .await?
    .with_libsql_turn_state_store(Arc::clone(&config.database))
    .await?
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
    .with_postgres_run_state_approval_store(config.pool.clone())
    .await?
    .with_postgres_turn_state_store(config.pool.clone())
    .await?
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
