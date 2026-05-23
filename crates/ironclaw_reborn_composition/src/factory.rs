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

use crate::input::RebornStorageInput;
use crate::{
    RebornBuildError, RebornBuildInput, RebornCompositionProfile, RebornFacadeReadiness,
    RebornProductAuthServices, RebornReadiness, RebornReadinessState,
};

pub struct RebornServices {
    pub host_runtime: Option<Arc<dyn ironclaw_host_runtime::HostRuntime>>,
    pub turn_coordinator: Option<Arc<dyn ironclaw_turns::TurnCoordinator>>,
    pub product_auth: Option<Arc<RebornProductAuthServices>>,
    pub readiness: RebornReadiness,
    pub(crate) local_runtime: Option<Arc<RebornLocalRuntimeServices>>,
}

pub(crate) struct RebornLocalRuntimeServices {
    pub(crate) turn_state: Arc<InMemoryTurnStateStore>,
    pub(crate) checkpoint_state_store: Arc<InMemoryCheckpointStateStore>,
    pub(crate) loop_checkpoint_store: Arc<InMemoryLoopCheckpointStore>,
    pub(crate) thread_service: Arc<InMemorySessionThreadService>,
    pub(crate) skill_filesystem: Arc<ScopedFilesystem<LocalFilesystem>>,
    /// Tenant-scoped root filesystem used for third-party extension hook
    /// discovery (`/system/extensions/<tenant>`). The runtime derives the
    /// discovery root from the authenticated tenant id; this is the same
    /// backend the rest of local-dev composition uses.
    pub(crate) extension_filesystem: Arc<LocalFilesystem>,
}

impl std::fmt::Debug for RebornServices {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RebornServices")
            .field("host_runtime", &self.host_runtime.is_some())
            .field("turn_coordinator", &self.turn_coordinator.is_some())
            .field("product_auth", &self.product_auth.is_some())
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
            product_auth: None,
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

async fn build_local_dev(input: RebornBuildInput) -> Result<RebornServices, RebornBuildError> {
    let RebornBuildInput {
        profile,
        storage,
        runtime_policy,
        product_auth_services,
        ..
    } = input;
    let RebornStorageInput::LocalDev {
        root,
        workspace_root,
    } = storage
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
    // Mount `/system/extensions` to a per-owner host subtree under the storage
    // root. This is the discovery root for third-party extension hooks. Because
    // the whole local-dev runtime (and this filesystem) is constructed once per
    // owner/identity in `build_reborn_services`, this mount is per-identity by
    // construction — it is NOT a process-global, cross-tenant-shared mount. That
    // per-identity construction is the tenant-isolation boundary the hook-only
    // projection model relies on (see `crate::hooks::tenant_extension_root`).
    // In multi-tenant production a tenant-scoped backend must supply this mount;
    // `build_reborn_runtime` only wires local-dev today. Computed before the
    // `/projects` mount consumes `root`.
    let extensions_host_root = root.join("system/extensions");
    std::fs::create_dir_all(&extensions_host_root).map_err(|_| {
        RebornBuildError::InvalidConfig {
            reason: "local-dev extensions root could not be initialized".to_string(),
        }
    })?;
    let extensions_virtual_root = ironclaw_host_api::VirtualPath::new("/system/extensions")
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
    filesystem.mount_local(
        extensions_virtual_root,
        ironclaw_host_api::HostPath::from_path_buf(extensions_host_root),
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
        extension_filesystem: Arc::clone(&filesystem),
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
    if let Some(runtime_policy) = runtime_policy {
        services = services.with_runtime_policy(runtime_policy);
    }
    // TODO(process-port): local-dev intentionally uses the default
    // LocalHostProcessPort until a non-local process backend is composed.

    let host_runtime: Arc<dyn ironclaw_host_runtime::HostRuntime> =
        Arc::new(services.host_runtime_for_local_testing());
    let turn_coordinator: Arc<dyn ironclaw_turns::TurnCoordinator> =
        Arc::new(DefaultTurnCoordinator::new(turn_state));
    let product_auth = Some(
        product_auth_services
            .unwrap_or_else(|| Arc::new(RebornProductAuthServices::local_dev_in_memory())),
    );
    let product_auth_ready = product_auth.is_some();

    Ok(RebornServices {
        host_runtime: Some(host_runtime),
        turn_coordinator: Some(turn_coordinator),
        // Local-dev always composes a safe in-memory product-auth boundary when
        // the caller does not inject one; readiness tracks the assembled facade.
        product_auth,
        readiness: readiness_for(profile, true, true, product_auth_ready),
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

pub(crate) fn builtin_extension_registry() -> Result<ExtensionRegistry, RebornBuildError> {
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
        required_runtime_backends,
        require_runtime_http_egress,
        require_wasm_credentials,
        product_auth_services,
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
        required_runtime_backends,
        require_runtime_http_egress,
        require_wasm_credentials,
        product_auth_services,
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
            )?;
            // TODO(process-port): if production enables FirstParty runtime,
            // HostRuntimeServices must be given a production process port;
            // otherwise the LocalHostProcessPort default is rejected.
            let context = RebornProductionBuildContext {
                profile,
                wiring_config,
                production_wiring,
                product_auth_services,
            };
            build_libsql_production(context, db, path_or_url, auth_token, secret_master_key).await
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
            )?;
            // TODO(process-port): if production enables FirstParty runtime,
            // HostRuntimeServices must be given a production process port;
            // otherwise the LocalHostProcessPort default is rejected.
            let context = RebornProductionBuildContext {
                profile,
                wiring_config,
                production_wiring,
                product_auth_services,
            };
            build_postgres_production(context, pool, url, secret_master_key).await
        }
    }
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
struct RebornProductionWiring {
    trust_policy: Arc<HostTrustPolicy>,
    runtime_policy: EffectiveRuntimePolicy,
    turn_run_wake_notifier: Arc<ironclaw_host_runtime::SchedulerTurnRunWakeNotifier>,
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
struct RebornProductionBuildContext {
    profile: RebornCompositionProfile,
    wiring_config: ironclaw_host_runtime::ProductionWiringConfig,
    production_wiring: RebornProductionWiring,
    product_auth_services: Option<Arc<RebornProductAuthServices>>,
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
fn production_wiring(
    trust_policy: Option<Arc<HostTrustPolicy>>,
    runtime_policy: Option<EffectiveRuntimePolicy>,
    turn_run_wake_notifier: Option<Arc<ironclaw_host_runtime::SchedulerTurnRunWakeNotifier>>,
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
    context: RebornProductionBuildContext,
    db: Arc<libsql::Database>,
    path_or_url: String,
    auth_token: Option<ironclaw_secrets::SecretMaterial>,
    secret_master_key: ironclaw_secrets::SecretMaterial,
) -> Result<RebornServices, RebornBuildError> {
    use ironclaw_authorization::FilesystemCapabilityLeaseStore;
    use ironclaw_filesystem::LibSqlRootFilesystem;
    use ironclaw_secrets::FilesystemSecretStore;

    let RebornProductionBuildContext {
        profile,
        wiring_config,
        production_wiring,
        product_auth_services,
    } = context;

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

    let services = HostRuntimeServices::new(
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

    let turn_coordinator: Arc<dyn ironclaw_turns::TurnCoordinator> =
        Arc::new(services.turn_coordinator_for_production()?);
    let host_runtime: Arc<dyn ironclaw_host_runtime::HostRuntime> =
        Arc::new(services.host_runtime_for_production(&wiring_config)?);
    let product_auth_ready = product_auth_services.is_some();

    Ok(RebornServices {
        host_runtime: Some(host_runtime),
        turn_coordinator: Some(turn_coordinator),
        readiness: readiness_for(profile, true, true, product_auth_ready),
        product_auth: product_auth_services,
        local_runtime: None,
    })
}

#[cfg(feature = "postgres")]
async fn build_postgres_production(
    context: RebornProductionBuildContext,
    pool: deadpool_postgres::Pool,
    url: ironclaw_secrets::SecretMaterial,
    secret_master_key: ironclaw_secrets::SecretMaterial,
) -> Result<RebornServices, RebornBuildError> {
    use ironclaw_authorization::FilesystemCapabilityLeaseStore;
    use ironclaw_filesystem::PostgresRootFilesystem;
    use ironclaw_secrets::FilesystemSecretStore;

    let RebornProductionBuildContext {
        profile,
        wiring_config,
        production_wiring,
        product_auth_services,
    } = context;

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

    let services = HostRuntimeServices::new(
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

    let turn_coordinator: Arc<dyn ironclaw_turns::TurnCoordinator> =
        Arc::new(services.turn_coordinator_for_production()?);
    let host_runtime: Arc<dyn ironclaw_host_runtime::HostRuntime> =
        Arc::new(services.host_runtime_for_production(&wiring_config)?);
    let product_auth_ready = product_auth_services.is_some();

    Ok(RebornServices {
        host_runtime: Some(host_runtime),
        turn_coordinator: Some(turn_coordinator),
        readiness: readiness_for(profile, true, true, product_auth_ready),
        product_auth: product_auth_services,
        local_runtime: None,
    })
}

fn readiness_for(
    profile: RebornCompositionProfile,
    host_runtime: bool,
    turn_coordinator: bool,
    product_auth: bool,
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
            product_auth,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(services.product_auth.is_some());
        assert!(services.local_runtime.is_some());
        assert_eq!(services.readiness.state, RebornReadinessState::DevOnly);
    }

    #[test]
    fn disabled_services_do_not_include_repl_runtime_substrate() {
        let services = RebornServices::disabled();

        assert!(services.host_runtime.is_none());
        assert!(services.turn_coordinator.is_none());
        assert!(services.product_auth.is_none());
        assert!(services.local_runtime.is_none());
        assert_eq!(services.readiness.state, RebornReadinessState::Disabled);
    }

    #[test]
    fn production_readiness_reflects_product_auth_presence() {
        let without_auth = readiness_for(RebornCompositionProfile::Production, true, true, false);
        assert_eq!(
            without_auth.state,
            RebornReadinessState::ProductionValidated
        );
        assert!(!without_auth.facades.product_auth);

        let with_auth = readiness_for(RebornCompositionProfile::Production, true, true, true);
        assert_eq!(with_auth.state, RebornReadinessState::ProductionValidated);
        assert!(with_auth.facades.product_auth);
    }
}
