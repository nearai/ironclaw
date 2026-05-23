use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use ironclaw_authorization::GrantAuthorizer;
use ironclaw_extensions::ExtensionRegistry;
use ironclaw_filesystem::{LocalFilesystem, ScopedFilesystem};
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_host_api::runtime_policy::EffectiveRuntimePolicy;
use ironclaw_host_api::{
    EffectKind, MountAlias, MountGrant, MountPermissions, MountView, PackageId, VirtualPath,
};
use ironclaw_host_runtime::{
    CapabilitySurfaceVersion, FirstPartyCapabilityRegistry, HostRuntimeServices,
    builtin_first_party_handlers, builtin_first_party_package,
};
use ironclaw_processes::ProcessServices;
use ironclaw_resources::InMemoryResourceGovernor;
use ironclaw_run_state::{InMemoryApprovalRequestStore, InMemoryRunStateStore};
use ironclaw_threads::InMemorySessionThreadService;
use ironclaw_trust::{AdminConfig, AdminEntry, HostTrustAssignment, HostTrustPolicy};
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_turns::InMemoryRunProfileResolver;
use ironclaw_turns::{
    DefaultTurnCoordinator, InMemoryCheckpointStateStore, InMemoryLoopCheckpointStore,
    InMemoryTurnStateStore,
};

use crate::input::{RebornStorageInput, TenantSandboxProcessPortInput};
use crate::{
    RebornBuildError, RebornBuildInput, RebornCompositionProfile, RebornFacadeReadiness,
    RebornReadiness, RebornReadinessState,
};

pub struct RebornServices {
    pub host_runtime: Option<Arc<dyn ironclaw_host_runtime::HostRuntime>>,
    pub turn_coordinator: Option<Arc<dyn ironclaw_turns::TurnCoordinator>>,
    pub readiness: RebornReadiness,
    pub(crate) local_runtime: Option<Arc<RebornLocalRuntimeServices>>,
}

pub(crate) struct RebornLocalRuntimeServices {
    pub(crate) turn_state: Arc<InMemoryTurnStateStore>,
    pub(crate) checkpoint_state_store: Arc<InMemoryCheckpointStateStore>,
    pub(crate) loop_checkpoint_store: Arc<InMemoryLoopCheckpointStore>,
    pub(crate) thread_service: Arc<InMemorySessionThreadService>,
    pub(crate) skill_filesystem: Arc<ScopedFilesystem<LocalFilesystem>>,
}

impl std::fmt::Debug for RebornServices {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RebornServices")
            .field("host_runtime", &self.host_runtime.is_some())
            .field("turn_coordinator", &self.turn_coordinator.is_some())
            .field("readiness", &self.readiness)
            .field("local_runtime", &self.local_runtime.is_some())
            .finish()
    }
}

impl RebornServices {
    pub fn disabled() -> Self {
        Self {
            host_runtime: None,
            turn_coordinator: None,
            readiness: RebornReadiness::disabled(),
            local_runtime: None,
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

#[cfg(any(feature = "libsql", feature = "postgres"))]
fn production_config(
    required_runtime_backends: Vec<ironclaw_host_api::RuntimeKind>,
    require_runtime_http_egress: bool,
    require_wasm_credentials: bool,
) -> ironclaw_host_runtime::ProductionWiringConfig {
    let mut config = ironclaw_host_runtime::ProductionWiringConfig::new(required_runtime_backends);
    if require_runtime_http_egress {
        config = config.require_runtime_http_egress();
    }
    if require_wasm_credentials {
        config = config.require_wasm_credentials();
    }
    config
}

fn apply_tenant_sandbox_process_port<F, G, S, R>(
    services: HostRuntimeServices<F, G, S, R>,
    process_port: Option<TenantSandboxProcessPortInput>,
) -> HostRuntimeServices<F, G, S, R>
where
    F: ironclaw_filesystem::RootFilesystem + 'static,
    G: ironclaw_resources::ResourceGovernor + 'static,
    S: ironclaw_processes::ProcessStore + 'static,
    R: ironclaw_processes::ProcessResultStore + 'static,
{
    match process_port {
        Some(TenantSandboxProcessPortInput::ProductionCandidate(process_port)) => {
            services.with_production_tenant_sandbox_process_port(process_port)
        }
        Some(TenantSandboxProcessPortInput::Unverified(process_port)) => {
            services.with_tenant_sandbox_process_port_dyn(process_port)
        }
        None => services,
    }
}

async fn build_local_dev(input: RebornBuildInput) -> Result<RebornServices, RebornBuildError> {
    let RebornStorageInput::LocalDev {
        root,
        workspace_root,
    } = input.storage
    else {
        return Err(RebornBuildError::InvalidConfig {
            reason: "local-dev profile requires local-dev storage input".to_string(),
        });
    };
    std::fs::create_dir_all(&root).map_err(|_| RebornBuildError::InvalidConfig {
        reason: "local-dev storage root could not be initialized".to_string(),
    })?;
    let workspace_root = workspace_root.unwrap_or_else(|| root.join("workspace"));
    std::fs::create_dir_all(&workspace_root).map_err(|_| RebornBuildError::InvalidConfig {
        reason: "local-dev workspace root could not be initialized".to_string(),
    })?;
    let root = canonicalize_local_dev_path(&root, "storage root")?;
    let workspace_root = canonicalize_local_dev_path(&workspace_root, "workspace root")?;
    validate_local_dev_workspace_skill_isolation(&root, &workspace_root)?;
    let mut filesystem = LocalFilesystem::new();
    let projects_root = ironclaw_host_api::VirtualPath::new("/projects").map_err(|error| {
        RebornBuildError::InvalidConfig {
            reason: error.to_string(),
        }
    })?;
    let workspace_virtual_root = ironclaw_host_api::VirtualPath::new("/projects/workspace")
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: error.to_string(),
        })?;
    filesystem.mount_local(
        projects_root,
        ironclaw_host_api::HostPath::from_path_buf(root),
    )?;
    filesystem.mount_local(
        workspace_virtual_root,
        ironclaw_host_api::HostPath::from_path_buf(workspace_root),
    )?;

    let filesystem = Arc::new(filesystem);
    let skill_filesystem = Arc::new(ScopedFilesystem::with_fixed_view(
        Arc::clone(&filesystem),
        local_dev_skill_mount_view()?,
    ));

    let run_state = Arc::new(InMemoryRunStateStore::new());
    let approval_requests = Arc::new(InMemoryApprovalRequestStore::new());
    let turn_state = Arc::new(InMemoryTurnStateStore::default());
    let local_runtime = Arc::new(RebornLocalRuntimeServices {
        turn_state: Arc::clone(&turn_state),
        checkpoint_state_store: Arc::new(InMemoryCheckpointStateStore::default()),
        loop_checkpoint_store: Arc::new(InMemoryLoopCheckpointStore::default()),
        thread_service: Arc::new(InMemorySessionThreadService::default()),
        skill_filesystem,
    });

    let mut services = HostRuntimeServices::new(
        Arc::new(builtin_extension_registry()?),
        filesystem,
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ProcessServices::in_memory(),
        CapabilitySurfaceVersion::new("reborn-app-v1")?,
    )
    .with_first_party_capabilities(Arc::new(builtin_first_party_registry()?))
    .with_trust_policy(Arc::new(local_dev_first_party_trust_policy()?))
    .with_secret_store(Arc::new(ironclaw_secrets::InMemorySecretStore::new()))
    .try_with_host_http_egress(ironclaw_network::PolicyNetworkHttpEgress::new(
        ironclaw_network::ReqwestNetworkTransport::default(),
    ))?
    .with_run_state(Arc::clone(&run_state))
    .with_approval_requests(Arc::clone(&approval_requests))
    .with_turn_state(Arc::clone(&turn_state));
    if let Some(runtime_policy) = input.runtime_policy {
        services = services.with_runtime_policy(runtime_policy);
    }
    // Local-dev deliberately accepts injected process ports so composition tests
    // can exercise tenant-sandbox routing without requiring Docker.
    services = apply_tenant_sandbox_process_port(services, input.tenant_sandbox_process_port);

    let host_runtime: Arc<dyn ironclaw_host_runtime::HostRuntime> =
        Arc::new(services.host_runtime_for_local_testing());
    let turn_coordinator: Arc<dyn ironclaw_turns::TurnCoordinator> =
        Arc::new(DefaultTurnCoordinator::new(turn_state));

    Ok(RebornServices {
        host_runtime: Some(host_runtime),
        turn_coordinator: Some(turn_coordinator),
        readiness: readiness_for(input.profile, true, true),
        local_runtime: Some(local_runtime),
    })
}

fn canonicalize_local_dev_path(path: &Path, label: &str) -> Result<PathBuf, RebornBuildError> {
    std::fs::canonicalize(path).map_err(|_| RebornBuildError::InvalidConfig {
        reason: format!("local-dev {label} could not be resolved"),
    })
}

fn validate_local_dev_workspace_skill_isolation(
    storage_root: &Path,
    workspace_root: &Path,
) -> Result<(), RebornBuildError> {
    for (label, skill_root) in [
        ("/skills", storage_root.join("skills")),
        (
            "/tenant-shared/skills",
            storage_root.join("tenant-shared/skills"),
        ),
        ("/system/skills", storage_root.join("system/skills")),
    ] {
        if paths_overlap(workspace_root, &skill_root) {
            return Err(RebornBuildError::InvalidConfig {
                reason: format!(
                    "local-dev workspace root must not overlap default skill root {label}"
                ),
            });
        }
    }
    Ok(())
}

fn paths_overlap(left: &Path, right: &Path) -> bool {
    left == right || left.starts_with(right) || right.starts_with(left)
}

fn local_dev_skill_mount_view() -> Result<MountView, RebornBuildError> {
    let grant = |alias: &str, target: &str| -> Result<MountGrant, RebornBuildError> {
        Ok(MountGrant::new(
            MountAlias::new(alias).map_err(|error| RebornBuildError::InvalidConfig {
                reason: error.to_string(),
            })?,
            VirtualPath::new(target).map_err(|error| RebornBuildError::InvalidConfig {
                reason: error.to_string(),
            })?,
            MountPermissions::read_only(),
        ))
    };
    MountView::new(vec![
        grant("/skills", "/projects/skills")?,
        grant("/tenant-shared/skills", "/projects/tenant-shared/skills")?,
        grant("/system/skills", "/projects/system/skills")?,
    ])
    .map_err(|error| RebornBuildError::InvalidConfig {
        reason: error.to_string(),
    })
}

fn builtin_extension_registry() -> Result<ExtensionRegistry, RebornBuildError> {
    // Shared by local-dev and production composition so host-owned first-party
    // capabilities expose the same built-in package contract in both profiles.
    let mut registry = ExtensionRegistry::new();
    registry
        .insert(
            builtin_first_party_package().map_err(|error| RebornBuildError::InvalidConfig {
                reason: format!("built-in first-party package is invalid: {error}"),
            })?,
        )
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("built-in first-party registry is invalid: {error}"),
        })?;
    Ok(registry)
}

fn builtin_first_party_registry() -> Result<FirstPartyCapabilityRegistry, RebornBuildError> {
    builtin_first_party_handlers().map_err(|error| RebornBuildError::InvalidConfig {
        reason: format!("built-in first-party handlers are invalid: {error}"),
    })
}

fn local_dev_first_party_trust_policy() -> Result<HostTrustPolicy, RebornBuildError> {
    HostTrustPolicy::new(vec![Box::new(AdminConfig::with_entries(vec![
        AdminEntry::for_local_manifest(
            PackageId::new("builtin").map_err(|error| RebornBuildError::InvalidConfig {
                reason: format!("built-in first-party package id is invalid: {error}"),
            })?,
            "/system/extensions/builtin/manifest.toml".to_string(),
            None,
            HostTrustAssignment::first_party(),
            vec![
                EffectKind::DispatchCapability,
                EffectKind::ReadFilesystem,
                EffectKind::WriteFilesystem,
                EffectKind::Network,
                EffectKind::SpawnProcess,
                EffectKind::ExecuteCode,
            ],
            None,
        ),
    ]))])
    .map_err(|error| RebornBuildError::InvalidConfig {
        reason: format!("built-in first-party trust policy is invalid: {error}"),
    })
}

async fn build_production_shaped(
    input: RebornBuildInput,
) -> Result<RebornServices, RebornBuildError> {
    let RebornBuildInput {
        profile,
        owner_id: _,
        storage,
        production_trust_policy,
        runtime_policy,
        turn_run_wake_notifier,
        tenant_sandbox_process_port,
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
        production_trust_policy,
        runtime_policy,
        turn_run_wake_notifier,
        tenant_sandbox_process_port,
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
            let production_wiring = production_wiring(
                production_trust_policy,
                runtime_policy,
                turn_run_wake_notifier,
                tenant_sandbox_process_port,
            )?;
            build_libsql_production(
                profile,
                wiring_config,
                production_wiring,
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
        } => {
            let production_wiring = production_wiring(
                production_trust_policy,
                runtime_policy,
                turn_run_wake_notifier,
                tenant_sandbox_process_port,
            )?;
            build_postgres_production(
                profile,
                wiring_config,
                production_wiring,
                pool,
                url,
                secret_master_key,
            )
            .await
        }
    }
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
struct RebornProductionWiring {
    trust_policy: Arc<HostTrustPolicy>,
    runtime_policy: EffectiveRuntimePolicy,
    turn_run_wake_notifier: Arc<ironclaw_host_runtime::SchedulerTurnRunWakeNotifier>,
    tenant_sandbox_process_port: Option<TenantSandboxProcessPortInput>,
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
fn production_wiring(
    trust_policy: Option<Arc<HostTrustPolicy>>,
    runtime_policy: Option<EffectiveRuntimePolicy>,
    turn_run_wake_notifier: Option<Arc<ironclaw_host_runtime::SchedulerTurnRunWakeNotifier>>,
    tenant_sandbox_process_port: Option<TenantSandboxProcessPortInput>,
) -> Result<RebornProductionWiring, RebornBuildError> {
    let trust_policy = trust_policy.ok_or(RebornBuildError::MissingProductionTrustPolicy)?;
    if !trust_policy.has_sources() {
        return Err(RebornBuildError::EmptyProductionTrustPolicy);
    }
    let runtime_policy = runtime_policy.ok_or(RebornBuildError::MissingRuntimePolicy)?;
    let turn_run_wake_notifier =
        turn_run_wake_notifier.ok_or(RebornBuildError::MissingTurnRunWakeNotifier)?;
    Ok(RebornProductionWiring {
        trust_policy,
        runtime_policy,
        turn_run_wake_notifier,
        tenant_sandbox_process_port,
    })
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
fn planned_run_profile_resolver() -> Result<Arc<InMemoryRunProfileResolver>, RebornBuildError> {
    Ok(Arc::new(
        ironclaw_reborn::planned_driver_factory::default_planned_run_profile_resolver().map_err(
            |error| RebornBuildError::PlannedRunProfileResolver {
                reason: error.to_string(),
            },
        )?,
    ))
}

#[cfg(feature = "libsql")]
async fn build_libsql_production(
    profile: RebornCompositionProfile,
    wiring_config: ironclaw_host_runtime::ProductionWiringConfig,
    production_wiring: RebornProductionWiring,
    db: Arc<libsql::Database>,
    path_or_url: String,
    auth_token: Option<ironclaw_secrets::SecretMaterial>,
    secret_master_key: ironclaw_secrets::SecretMaterial,
) -> Result<RebornServices, RebornBuildError> {
    use ironclaw_authorization::FilesystemCapabilityLeaseStore;
    use ironclaw_filesystem::LibSqlRootFilesystem;
    use ironclaw_secrets::FilesystemSecretStore;

    let filesystem = Arc::new(LibSqlRootFilesystem::new(Arc::clone(&db)));
    filesystem.run_migrations().await?;

    let scoped_filesystem = crate::wrap_scoped(Arc::clone(&filesystem));
    let leases = Arc::new(FilesystemCapabilityLeaseStore::new(Arc::clone(
        &scoped_filesystem,
    )));

    let scoped_filesystem = crate::wrap_scoped(Arc::clone(&filesystem));

    let secret_crypto = Arc::new(ironclaw_secrets::SecretsCrypto::new(secret_master_key)?);
    let secret_store = Arc::new(FilesystemSecretStore::new(
        Arc::clone(&scoped_filesystem),
        Arc::clone(&secret_crypto),
    ));

    let event_store = ironclaw_reborn_event_store::RebornEventStoreConfig::Libsql {
        path_or_url,
        auth_token,
    };

    let mut services = HostRuntimeServices::new(
        Arc::new(builtin_extension_registry()?),
        Arc::clone(&filesystem),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ProcessServices::filesystem(Arc::clone(&scoped_filesystem)),
        CapabilitySurfaceVersion::new("reborn-app-v1")?,
    )
    .with_trust_policy(production_wiring.trust_policy)
    .with_runtime_policy(production_wiring.runtime_policy)
    .with_first_party_capabilities(Arc::new(builtin_first_party_registry()?))
    .with_capability_leases(leases)
    .with_secret_store(secret_store)
    .try_with_host_http_egress(ironclaw_network::PolicyNetworkHttpEgress::new(
        ironclaw_network::ReqwestNetworkTransport::default(),
    ))?
    .with_filesystem_resource_governor(Arc::clone(&scoped_filesystem))
    .with_reborn_event_store_config(profile.to_event_store_profile(), event_store)
    .await?
    .with_filesystem_run_state(Arc::clone(&scoped_filesystem))
    .with_filesystem_turn_state_store(scoped_filesystem)
    .with_run_profile_resolver(planned_run_profile_resolver()?)
    .with_turn_run_wake_notifier(production_wiring.turn_run_wake_notifier);
    services =
        apply_tenant_sandbox_process_port(services, production_wiring.tenant_sandbox_process_port);

    let turn_coordinator: Arc<dyn ironclaw_turns::TurnCoordinator> =
        Arc::new(services.turn_coordinator_for_production()?);
    let host_runtime: Arc<dyn ironclaw_host_runtime::HostRuntime> =
        Arc::new(services.host_runtime_for_production(&wiring_config)?);

    Ok(RebornServices {
        host_runtime: Some(host_runtime),
        turn_coordinator: Some(turn_coordinator),
        readiness: readiness_for(profile, true, true),
        local_runtime: None,
    })
}

#[cfg(feature = "postgres")]
async fn build_postgres_production(
    profile: RebornCompositionProfile,
    wiring_config: ironclaw_host_runtime::ProductionWiringConfig,
    production_wiring: RebornProductionWiring,
    pool: deadpool_postgres::Pool,
    url: ironclaw_secrets::SecretMaterial,
    secret_master_key: ironclaw_secrets::SecretMaterial,
) -> Result<RebornServices, RebornBuildError> {
    use ironclaw_authorization::FilesystemCapabilityLeaseStore;
    use ironclaw_filesystem::PostgresRootFilesystem;
    use ironclaw_secrets::FilesystemSecretStore;

    let filesystem = Arc::new(PostgresRootFilesystem::new(pool.clone()));
    filesystem.run_migrations().await?;

    let scoped_filesystem = crate::wrap_scoped(Arc::clone(&filesystem));
    let leases = Arc::new(FilesystemCapabilityLeaseStore::new(Arc::clone(
        &scoped_filesystem,
    )));

    let scoped_filesystem = crate::wrap_scoped(Arc::clone(&filesystem));

    let secret_crypto = Arc::new(ironclaw_secrets::SecretsCrypto::new(secret_master_key)?);
    let secret_store = Arc::new(FilesystemSecretStore::new(
        Arc::clone(&scoped_filesystem),
        Arc::clone(&secret_crypto),
    ));

    let event_store = ironclaw_reborn_event_store::RebornEventStoreConfig::Postgres { url };

    let mut services = HostRuntimeServices::new(
        Arc::new(builtin_extension_registry()?),
        Arc::clone(&filesystem),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ProcessServices::filesystem(Arc::clone(&scoped_filesystem)),
        CapabilitySurfaceVersion::new("reborn-app-v1")?,
    )
    .with_trust_policy(production_wiring.trust_policy)
    .with_runtime_policy(production_wiring.runtime_policy)
    .with_first_party_capabilities(Arc::new(builtin_first_party_registry()?))
    .with_capability_leases(leases)
    .with_secret_store(secret_store)
    .try_with_host_http_egress(ironclaw_network::PolicyNetworkHttpEgress::new(
        ironclaw_network::ReqwestNetworkTransport::default(),
    ))?
    .with_filesystem_resource_governor(Arc::clone(&scoped_filesystem))
    .with_reborn_event_store_config(profile.to_event_store_profile(), event_store)
    .await?
    .with_filesystem_run_state(Arc::clone(&scoped_filesystem))
    .with_filesystem_turn_state_store(scoped_filesystem)
    .with_run_profile_resolver(planned_run_profile_resolver()?)
    .with_turn_run_wake_notifier(production_wiring.turn_run_wake_notifier);
    services =
        apply_tenant_sandbox_process_port(services, production_wiring.tenant_sandbox_process_port);

    let turn_coordinator: Arc<dyn ironclaw_turns::TurnCoordinator> =
        Arc::new(services.turn_coordinator_for_production()?);
    let host_runtime: Arc<dyn ironclaw_host_runtime::HostRuntime> =
        Arc::new(services.host_runtime_for_production(&wiring_config)?);

    Ok(RebornServices {
        host_runtime: Some(host_runtime),
        turn_coordinator: Some(turn_coordinator),
        readiness: readiness_for(profile, true, true),
        local_runtime: None,
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

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use ironclaw_host_api::{
        ApprovalPolicy, AuditMode, CapabilityGrant, CapabilityGrantId, CapabilityId, CapabilitySet,
        DeploymentMode, ExecutionContext, ExtensionId, FilesystemBackendKind, GrantConstraints,
        NetworkMode, NetworkPolicy, NetworkTargetPattern, Principal, ProcessBackendKind,
        ResourceEstimate, RuntimeKind, RuntimeProfile, SecretMode, TrustClass, UserId,
    };
    use ironclaw_host_runtime::{
        CommandExecutionOutput, CommandExecutionRequest, RuntimeCapabilityOutcome,
        RuntimeCapabilityRequest, RuntimeProcessError, RuntimeProcessPort, SHELL_CAPABILITY_ID,
    };
    use ironclaw_trust::{AuthorityCeiling, EffectiveTrustClass, TrustDecision, TrustProvenance};
    use serde_json::json;
    use std::sync::Mutex;
    use std::time::Duration;

    #[tokio::test]
    async fn local_dev_services_include_repl_runtime_substrate() {
        let dir = tempfile::tempdir().expect("tempdir");
        let services = build_reborn_services(RebornBuildInput::local_dev(
            "local-dev-substrate-owner",
            dir.path().join("local-dev"),
        ))
        .await
        .expect("local-dev services build");

        assert!(services.host_runtime.is_some());
        assert!(services.turn_coordinator.is_some());
        assert!(services.local_runtime.is_some());
        assert_eq!(services.readiness.state, RebornReadinessState::DevOnly);
    }

    #[test]
    fn disabled_services_do_not_include_repl_runtime_substrate() {
        let services = RebornServices::disabled();

        assert!(services.host_runtime.is_none());
        assert!(services.turn_coordinator.is_none());
        assert!(services.local_runtime.is_none());
        assert_eq!(services.readiness.state, RebornReadinessState::Disabled);
    }

    #[tokio::test]
    async fn local_dev_composes_injected_tenant_sandbox_process_port() {
        let dir = tempfile::tempdir().expect("tempdir");
        let process_port = Arc::new(RecordingProcessPort::default());
        let process_port_dyn: Arc<dyn RuntimeProcessPort> = process_port.clone();
        let services = build_reborn_services(
            RebornBuildInput::local_dev("sandbox-port-owner", dir.path().join("local-dev"))
                .with_runtime_policy(tenant_sandbox_process_policy())
                .with_unverified_tenant_sandbox_process_port_dyn(process_port_dyn),
        )
        .await
        .expect("local-dev services build");
        let host_runtime = services.host_runtime.expect("host runtime");

        let outcome = host_runtime
            .invoke_capability(RuntimeCapabilityRequest::new(
                shell_execution_context(),
                CapabilityId::new(SHELL_CAPABILITY_ID).unwrap(),
                ResourceEstimate::default(),
                json!({"command": "echo composed sandbox", "timeout": 9}),
                trust_decision(),
            ))
            .await
            .expect("capability invoke");

        let RuntimeCapabilityOutcome::Completed(completed) = outcome else {
            panic!("expected completed shell invocation, got {outcome:?}");
        };
        assert_eq!(completed.output["sandboxed"], json!(true));
        assert_eq!(
            completed.output["output"],
            json!("sandbox port: echo composed sandbox")
        );
        let requests = process_port.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].command, "echo composed sandbox");
        assert_eq!(requests[0].timeout_secs, Some(9));
    }

    #[derive(Debug, Default)]
    struct RecordingProcessPort {
        requests: Mutex<Vec<CommandExecutionRequest>>,
    }

    #[async_trait]
    impl RuntimeProcessPort for RecordingProcessPort {
        async fn run_command(
            &self,
            request: CommandExecutionRequest,
        ) -> Result<CommandExecutionOutput, RuntimeProcessError> {
            let command = request.command.clone();
            self.requests.lock().unwrap().push(request);
            Ok(CommandExecutionOutput {
                output: format!("sandbox port: {command}"),
                exit_code: 0,
                sandboxed: true,
                duration: Duration::from_millis(5),
            })
        }
    }

    fn tenant_sandbox_process_policy() -> ironclaw_host_api::EffectiveRuntimePolicy {
        ironclaw_host_api::EffectiveRuntimePolicy {
            deployment: DeploymentMode::LocalSingleUser,
            requested_profile: RuntimeProfile::LocalDev,
            resolved_profile: RuntimeProfile::LocalDev,
            filesystem_backend: FilesystemBackendKind::HostWorkspace,
            process_backend: ProcessBackendKind::TenantSandbox,
            network_mode: NetworkMode::DirectLogged,
            secret_mode: SecretMode::ScrubbedEnv,
            approval_policy: ApprovalPolicy::AskDestructive,
            audit_mode: AuditMode::LocalMinimal,
        }
    }

    fn shell_execution_context() -> ExecutionContext {
        let network = NetworkPolicy {
            allowed_targets: vec![NetworkTargetPattern {
                scheme: None,
                host_pattern: "*".to_string(),
                port: None,
            }],
            deny_private_ip_ranges: false,
            max_egress_bytes: None,
        };
        let grant = CapabilityGrant {
            id: CapabilityGrantId::new(),
            capability: CapabilityId::new(SHELL_CAPABILITY_ID).unwrap(),
            grantee: Principal::Extension(ExtensionId::new("caller").unwrap()),
            issued_by: Principal::HostRuntime,
            constraints: GrantConstraints {
                allowed_effects: builtin_effects(),
                mounts: MountView::default(),
                network,
                secrets: Vec::new(),
                resource_ceiling: None,
                expires_at: None,
                max_invocations: None,
            },
        };
        ExecutionContext::local_default(
            UserId::new("user").unwrap(),
            ExtensionId::new("caller").unwrap(),
            RuntimeKind::FirstParty,
            TrustClass::FirstParty,
            CapabilitySet {
                grants: vec![grant],
            },
            MountView::default(),
        )
        .unwrap()
    }

    fn builtin_effects() -> Vec<EffectKind> {
        vec![
            EffectKind::DispatchCapability,
            EffectKind::ReadFilesystem,
            EffectKind::WriteFilesystem,
            EffectKind::Network,
            EffectKind::SpawnProcess,
            EffectKind::ExecuteCode,
        ]
    }

    fn trust_decision() -> TrustDecision {
        TrustDecision {
            effective_trust: EffectiveTrustClass::user_trusted(),
            authority_ceiling: AuthorityCeiling {
                allowed_effects: builtin_effects(),
                max_resource_ceiling: None,
            },
            provenance: TrustProvenance::Default,
            evaluated_at: chrono::Utc::now(),
        }
    }
}
