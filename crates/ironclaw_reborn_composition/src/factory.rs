use std::sync::Arc;

use ironclaw_authorization::GrantAuthorizer;
use ironclaw_extensions::ExtensionRegistry;
use ironclaw_filesystem::LocalFilesystem;
use ironclaw_host_runtime::{
    CapabilitySurfaceVersion, HostRuntimeServices, ProductionWiringConfig,
};
use ironclaw_processes::ProcessServices;
use ironclaw_resources::InMemoryResourceGovernor;
use ironclaw_run_state::{InMemoryApprovalRequestStore, InMemoryRunStateStore};
use ironclaw_trust::HostTrustPolicy;
use ironclaw_turns::{
    DefaultTurnCoordinator, InMemoryTurnStateStore, TurnRunWake, TurnRunWakeNotifier,
    TurnRunWakeNotifyError,
};

use crate::input::RebornStorageInput;
use crate::{
    RebornBuildError, RebornBuildInput, RebornCompositionProfile, RebornFacadeReadiness,
    RebornReadiness, RebornReadinessState,
};

pub struct RebornServices {
    pub host_runtime: Option<Arc<dyn ironclaw_host_runtime::HostRuntime>>,
    pub turn_coordinator: Option<Arc<dyn ironclaw_turns::TurnCoordinator>>,
    pub readiness: RebornReadiness,
}

impl std::fmt::Debug for RebornServices {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RebornServices")
            .field("host_runtime", &self.host_runtime.is_some())
            .field("turn_coordinator", &self.turn_coordinator.is_some())
            .field("readiness", &self.readiness)
            .finish()
    }
}

impl RebornServices {
    pub fn disabled() -> Self {
        Self {
            host_runtime: None,
            turn_coordinator: None,
            readiness: RebornReadiness::disabled(),
        }
    }
}

pub async fn build_reborn_services(
    input: RebornBuildInput,
) -> Result<RebornServices, RebornBuildError> {
    tracing::debug!(
        profile = %input.profile,
        owner_id = %input.owner_id,
        "building Reborn composition facades"
    );
    match input.profile {
        RebornCompositionProfile::Disabled => Ok(RebornServices::disabled()),
        RebornCompositionProfile::LocalDev => build_local_dev(input).await,
        RebornCompositionProfile::Production | RebornCompositionProfile::MigrationDryRun => {
            build_production_shaped(input).await
        }
    }
}

#[derive(Debug, Default)]
struct RebornCompositionWakeNotifier;

impl TurnRunWakeNotifier for RebornCompositionWakeNotifier {
    fn notify_queued_run(&self, wake: TurnRunWake) -> Result<(), TurnRunWakeNotifyError> {
        tracing::debug!(
            run_id = %wake.run_id,
            status = ?wake.status,
            "queued Reborn turn without live scheduler; channel cutover is not enabled"
        );
        Err(TurnRunWakeNotifyError::DeliveryUnavailable)
    }
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
fn production_config(
    required_runtime_backends: Vec<ironclaw_host_api::RuntimeKind>,
    require_runtime_http_egress: bool,
    require_wasm_credentials: bool,
) -> ProductionWiringConfig {
    let mut config = ProductionWiringConfig::new(required_runtime_backends);
    if require_runtime_http_egress {
        config = config.require_runtime_http_egress();
    }
    if require_wasm_credentials {
        config = config.require_wasm_credentials();
    }
    config
}

async fn build_local_dev(input: RebornBuildInput) -> Result<RebornServices, RebornBuildError> {
    let RebornStorageInput::LocalDev { root } = input.storage else {
        return Err(RebornBuildError::InvalidConfig {
            reason: "local-dev profile requires local-dev storage input".to_string(),
        });
    };
    std::fs::create_dir_all(&root).map_err(|_| RebornBuildError::InvalidConfig {
        reason: "local-dev storage root could not be initialized".to_string(),
    })?;
    let mut filesystem = LocalFilesystem::new();
    let projects_root = ironclaw_host_api::VirtualPath::new("/projects").map_err(|error| {
        RebornBuildError::InvalidConfig {
            reason: error.to_string(),
        }
    })?;
    filesystem.mount_local(
        projects_root,
        ironclaw_host_api::HostPath::from_path_buf(root),
    )?;

    let run_state = Arc::new(InMemoryRunStateStore::new());
    let approval_requests = Arc::new(InMemoryApprovalRequestStore::new());
    let turn_state = Arc::new(InMemoryTurnStateStore::default());

    let services = HostRuntimeServices::new(
        Arc::new(ExtensionRegistry::new()),
        Arc::new(filesystem),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("reborn-app-v1")?,
    )
    .with_trust_policy(Arc::new(HostTrustPolicy::empty()))
    .with_run_state(Arc::clone(&run_state))
    .with_approval_requests(Arc::clone(&approval_requests))
    .with_turn_state(Arc::clone(&turn_state));

    let host_runtime: Arc<dyn ironclaw_host_runtime::HostRuntime> =
        Arc::new(services.host_runtime_for_local_testing());
    let turn_coordinator: Arc<dyn ironclaw_turns::TurnCoordinator> =
        Arc::new(DefaultTurnCoordinator::new(turn_state));

    Ok(RebornServices {
        host_runtime: Some(host_runtime),
        turn_coordinator: Some(turn_coordinator),
        readiness: readiness_for(input.profile, true, true),
    })
}

async fn build_production_shaped(
    input: RebornBuildInput,
) -> Result<RebornServices, RebornBuildError> {
    let RebornBuildInput {
        profile,
        owner_id: _,
        storage,
        required_runtime_backends,
        require_runtime_http_egress,
        require_wasm_credentials,
    } = input;
    #[cfg(any(feature = "libsql", feature = "postgres"))]
    let wiring_config = production_config(
        required_runtime_backends,
        require_runtime_http_egress,
        require_wasm_credentials,
    );
    #[cfg(not(any(feature = "libsql", feature = "postgres")))]
    let _ = (
        required_runtime_backends,
        require_runtime_http_egress,
        require_wasm_credentials,
    );

    match storage {
        RebornStorageInput::Disabled | RebornStorageInput::LocalDev { .. } => {
            Err(RebornBuildError::InvalidConfig {
                reason: format!(
                    "profile={} requires durable database-backed Reborn storage",
                    profile
                ),
            })
        }
        #[cfg(feature = "libsql")]
        RebornStorageInput::Libsql {
            db,
            path_or_url,
            auth_token,
            secret_master_key,
        } => {
            build_libsql_production(
                profile,
                wiring_config,
                db,
                path_or_url,
                auth_token,
                secret_master_key,
            )
            .await
        }
        #[cfg(feature = "postgres")]
        RebornStorageInput::Postgres {
            pool,
            url,
            secret_master_key,
        } => build_postgres_production(profile, wiring_config, pool, url, secret_master_key).await,
    }
}

#[cfg(feature = "libsql")]
async fn build_libsql_production(
    profile: RebornCompositionProfile,
    wiring_config: ProductionWiringConfig,
    db: Arc<libsql::Database>,
    path_or_url: String,
    auth_token: Option<ironclaw_secrets::SecretMaterial>,
    secret_master_key: ironclaw_secrets::SecretMaterial,
) -> Result<RebornServices, RebornBuildError> {
    use ironclaw_authorization::LibSqlCapabilityLeaseStore;
    use ironclaw_filesystem::LibSqlRootFilesystem;
    use ironclaw_secrets::{LibSqlSecretsStore, ScopedSecretsStoreAdapter};

    let filesystem = Arc::new(LibSqlRootFilesystem::new(Arc::clone(&db)));
    filesystem.run_migrations().await?;

    let leases = Arc::new(LibSqlCapabilityLeaseStore::new(Arc::clone(&db)));
    leases.run_migrations().await?;

    let secret_crypto = Arc::new(ironclaw_secrets::SecretsCrypto::new(secret_master_key)?);
    let legacy_secret_store = Arc::new(LibSqlSecretsStore::new(
        Arc::clone(&db),
        Arc::clone(&secret_crypto),
    ));
    legacy_secret_store.run_migrations().await?;
    legacy_secret_store
        .verify_can_decrypt_existing_secrets()
        .await?;
    let secret_store = Arc::new(ScopedSecretsStoreAdapter::new(legacy_secret_store));

    let event_store = ironclaw_reborn_event_store::RebornEventStoreConfig::Libsql {
        path_or_url,
        auth_token,
    };

    let services = HostRuntimeServices::new(
        Arc::new(ExtensionRegistry::new()),
        Arc::clone(&filesystem),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ProcessServices::filesystem(Arc::clone(&filesystem)),
        CapabilitySurfaceVersion::new("reborn-app-v1")?,
    )
    .with_trust_policy(Arc::new(HostTrustPolicy::empty()))
    .with_capability_leases(leases)
    .with_secret_store(secret_store)
    .with_libsql_resource_governor(Arc::clone(&db))
    .await?
    .with_reborn_event_store_config(profile.to_event_store_profile(), event_store)
    .await?
    .with_libsql_run_state_approval_store(Arc::clone(&db))
    .await?
    .with_libsql_turn_state_store(db)
    .await?
    .with_turn_run_wake_notifier(Arc::new(RebornCompositionWakeNotifier));

    let turn_coordinator: Arc<dyn ironclaw_turns::TurnCoordinator> =
        Arc::new(services.turn_coordinator_for_production()?);
    let host_runtime: Arc<dyn ironclaw_host_runtime::HostRuntime> =
        Arc::new(services.host_runtime_for_production(&wiring_config)?);

    Ok(RebornServices {
        host_runtime: Some(host_runtime),
        turn_coordinator: Some(turn_coordinator),
        readiness: readiness_for(profile, true, true),
    })
}

#[cfg(feature = "postgres")]
async fn build_postgres_production(
    profile: RebornCompositionProfile,
    wiring_config: ProductionWiringConfig,
    pool: deadpool_postgres::Pool,
    url: ironclaw_secrets::SecretMaterial,
    secret_master_key: ironclaw_secrets::SecretMaterial,
) -> Result<RebornServices, RebornBuildError> {
    use ironclaw_authorization::PostgresCapabilityLeaseStore;
    use ironclaw_filesystem::PostgresRootFilesystem;
    use ironclaw_secrets::{PostgresSecretsStore, ScopedSecretsStoreAdapter};

    let filesystem = Arc::new(PostgresRootFilesystem::new(pool.clone()));
    filesystem.run_migrations().await?;

    let leases = Arc::new(PostgresCapabilityLeaseStore::new(pool.clone()));
    leases.run_migrations().await?;

    let secret_crypto = Arc::new(ironclaw_secrets::SecretsCrypto::new(secret_master_key)?);
    let legacy_secret_store = Arc::new(PostgresSecretsStore::new(pool.clone(), secret_crypto));
    legacy_secret_store.run_migrations().await?;
    legacy_secret_store
        .verify_can_decrypt_existing_secrets()
        .await?;
    let secret_store = Arc::new(ScopedSecretsStoreAdapter::new(legacy_secret_store));

    let event_store = ironclaw_reborn_event_store::RebornEventStoreConfig::Postgres { url };

    let services = HostRuntimeServices::new(
        Arc::new(ExtensionRegistry::new()),
        Arc::clone(&filesystem),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ProcessServices::filesystem(Arc::clone(&filesystem)),
        CapabilitySurfaceVersion::new("reborn-app-v1")?,
    )
    .with_trust_policy(Arc::new(HostTrustPolicy::empty()))
    .with_capability_leases(leases)
    .with_secret_store(secret_store)
    .with_postgres_resource_governor(pool.clone())
    .await?
    .with_reborn_event_store_config(profile.to_event_store_profile(), event_store)
    .await?
    .with_postgres_run_state_approval_store(pool.clone())
    .await?
    .with_postgres_turn_state_store(pool)
    .await?
    .with_turn_run_wake_notifier(Arc::new(RebornCompositionWakeNotifier));

    let turn_coordinator: Arc<dyn ironclaw_turns::TurnCoordinator> =
        Arc::new(services.turn_coordinator_for_production()?);
    let host_runtime: Arc<dyn ironclaw_host_runtime::HostRuntime> =
        Arc::new(services.host_runtime_for_production(&wiring_config)?);

    Ok(RebornServices {
        host_runtime: Some(host_runtime),
        turn_coordinator: Some(turn_coordinator),
        readiness: readiness_for(profile, true, true),
    })
}

fn readiness_for(
    profile: RebornCompositionProfile,
    host_runtime: bool,
    turn_coordinator: bool,
) -> RebornReadiness {
    let state = match profile {
        RebornCompositionProfile::Disabled => RebornReadinessState::Disabled,
        RebornCompositionProfile::LocalDev => RebornReadinessState::DevOnly,
        RebornCompositionProfile::Production => RebornReadinessState::ProductionValidated,
        RebornCompositionProfile::MigrationDryRun => RebornReadinessState::MigrationDryRunValidated,
    };
    RebornReadiness {
        profile,
        state,
        facades: RebornFacadeReadiness {
            host_runtime,
            turn_coordinator,
        },
    }
}
