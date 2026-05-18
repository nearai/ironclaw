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
mod profile;
mod readiness;
mod runtime;
mod runtime_input;
#[cfg(any(feature = "libsql", feature = "postgres"))]
mod secret_store;

pub use error::RebornBuildError;
pub use factory::{RebornServices, build_reborn_services};
pub use input::RebornBuildInput;
pub use profile::{RebornCompositionProfile, RebornCompositionProfileParseError};
pub use readiness::{RebornFacadeReadiness, RebornReadiness, RebornReadinessState};
pub use runtime::{
    AssistantReply, ConversationId, RebornRuntime, RebornRuntimeError, build_reborn_runtime,
};
#[cfg(feature = "root-llm-provider")]
pub use runtime_input::RebornLlmConfig;
pub use runtime_input::{
    PollSettings, RebornRuntimeIdentity, RebornRuntimeInput, TurnRunnerSettings,
};

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

use ironclaw_authorization::CapabilityLeaseError;
#[cfg(feature = "libsql")]
use ironclaw_filesystem::LibSqlRootFilesystem;
#[cfg(feature = "postgres")]
use ironclaw_filesystem::PostgresRootFilesystem;
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_host_runtime::{CapabilitySurfaceVersion, HostRuntimeServices};
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_processes::{FilesystemProcessResultStore, FilesystemProcessStore};
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_reborn_event_store::RebornEventStoreConfig;
use ironclaw_reborn_event_store::RebornEventStoreError;
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
    #[error("reborn run-profile resolver substrate failed: {0}")]
    RunProfile(#[from] ironclaw_turns::run_profile::RunProfileRegistryError),
    #[error("reborn production wiring failed")]
    ProductionWiring {
        report: ironclaw_host_runtime::ProductionWiringReport,
    },
}

impl From<ironclaw_host_runtime::ProductionWiringReport> for RebornCompositionError {
    fn from(report: ironclaw_host_runtime::ProductionWiringReport) -> Self {
        Self::ProductionWiring { report }
    }
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
