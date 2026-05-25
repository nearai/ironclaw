use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_authorization::FilesystemCapabilityLeaseStore;
use ironclaw_authorization::GrantAuthorizer;
use ironclaw_extensions::ExtensionRegistry;
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_filesystem::RootFilesystem;
use ironclaw_filesystem::{LocalFilesystem, ScopedFilesystem};
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_host_api::runtime_policy::EffectiveRuntimePolicy;
use ironclaw_host_api::{
    EffectKind, MountAlias, MountGrant, MountPermissions, MountView, PackageId, UserId, VirtualPath,
};
use ironclaw_host_runtime::{
    CapabilitySurfaceVersion, FirstPartyCapabilityRegistry, HostRuntimeServices,
    builtin_first_party_handlers, builtin_first_party_package,
};
use ironclaw_processes::ProcessServices;
use ironclaw_product_workflow::ProductAuthTurnGateResumeDispatcher;
use ironclaw_resources::InMemoryResourceGovernor;
use ironclaw_run_state::{InMemoryApprovalRequestStore, InMemoryRunStateStore};
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_secrets::FilesystemSecretStore;
use ironclaw_threads::InMemorySessionThreadService;
use ironclaw_trust::{AdminConfig, AdminEntry, HostTrustAssignment, HostTrustPolicy};
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_turns::InMemoryRunProfileResolver;
use ironclaw_turns::{
    DefaultTurnCoordinator, InMemoryCheckpointStateStore, InMemoryLoopCheckpointStore,
    InMemoryTurnStateStore,
};

use crate::{
    RebornAuthContinuationDispatcher, RebornBuildError, RebornBuildInput, RebornCompositionProfile,
    RebornFacadeReadiness, RebornProductAuthServicePorts, RebornProductAuthServices,
    RebornReadiness, RebornReadinessState,
};
use crate::{input::RebornStorageInput, lifecycle::RebornLocalSkillManagementPort};

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
    pub(crate) skill_management: Arc<RebornLocalSkillManagementPort>,
    pub(crate) skill_filesystem: Arc<ScopedFilesystem<LocalFilesystem>>,
    pub(crate) workspace_filesystem: Arc<ScopedFilesystem<LocalFilesystem>>,
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
        RebornCompositionProfile::LocalDev | RebornCompositionProfile::LocalDevYolo => {
            build_local_dev(input).await
        }
        RebornCompositionProfile::Production | RebornCompositionProfile::MigrationDryRun => {
            build_production_shaped(input).await
        }
    }
}

fn auth_continuation_dispatcher(
    turn_coordinator: Arc<dyn ironclaw_turns::TurnCoordinator>,
) -> Arc<dyn RebornAuthContinuationDispatcher> {
    Arc::new(ProductAuthTurnGateResumeDispatcher::new(turn_coordinator))
}

fn compose_product_auth_services(
    ports: RebornProductAuthServicePorts,
    turn_coordinator: Arc<dyn ironclaw_turns::TurnCoordinator>,
) -> Arc<RebornProductAuthServices> {
    Arc::new(ports.into_services(auth_continuation_dispatcher(turn_coordinator)))
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
        owner_id,
        storage,
        runtime_policy,
        product_auth_ports,
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
    filesystem.mount_local(
        projects_root,
        ironclaw_host_api::HostPath::from_path_buf(root),
    )?;
    filesystem.mount_local(
        workspace_virtual_root,
        ironclaw_host_api::HostPath::from_path_buf(workspace_root),
    )?;

    let filesystem = Arc::new(filesystem);
    let owner_user_id = UserId::new(owner_id).map_err(|error| RebornBuildError::InvalidConfig {
        reason: error.to_string(),
    })?;
    let skill_management = Arc::new(RebornLocalSkillManagementPort::new(
        owner_user_id,
        Arc::clone(&filesystem),
        local_dev_skill_management_mount_view()?,
    ));
    let skill_filesystem = Arc::new(ScopedFilesystem::with_fixed_view(
        Arc::clone(&filesystem),
        local_dev_skill_mount_view()?,
    ));
    let workspace_filesystem = Arc::new(ScopedFilesystem::with_fixed_view(
        Arc::clone(&filesystem),
        local_dev_workspace_mount_view()?,
    ));

    let run_state = Arc::new(InMemoryRunStateStore::new());
    let approval_requests = Arc::new(InMemoryApprovalRequestStore::new());
    let turn_state = Arc::new(InMemoryTurnStateStore::default());
    let local_runtime = Arc::new(RebornLocalRuntimeServices {
        turn_state: Arc::clone(&turn_state),
        checkpoint_state_store: Arc::new(InMemoryCheckpointStateStore::default()),
        loop_checkpoint_store: Arc::new(InMemoryLoopCheckpointStore::default()),
        thread_service: Arc::new(InMemorySessionThreadService::default()),
        skill_management,
        skill_filesystem,
        workspace_filesystem,
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
    // TODO(process-port): local runtime policies intentionally use the
    // LocalHostProcessPort until a non-local process backend is composed.

    let host_runtime: Arc<dyn ironclaw_host_runtime::HostRuntime> =
        Arc::new(services.host_runtime_for_local_testing());
    let turn_coordinator: Arc<dyn ironclaw_turns::TurnCoordinator> =
        Arc::new(DefaultTurnCoordinator::new(turn_state));
    let product_auth = Some(match product_auth_ports {
        Some(ports) => compose_product_auth_services(ports, turn_coordinator.clone()),
        None => Arc::new(RebornProductAuthServices::local_dev_in_memory(
            auth_continuation_dispatcher(turn_coordinator.clone()),
        )),
    });
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

fn local_dev_skill_management_mount_view() -> Result<MountView, RebornBuildError> {
    let grants = local_dev_skill_mount_grants(MountPermissions::read_write_list_delete())?;
    MountView::new(grants).map_err(|error| RebornBuildError::InvalidConfig {
        reason: error.to_string(),
    })
}

fn local_dev_skill_mount_view() -> Result<MountView, RebornBuildError> {
    let mut grants = local_dev_skill_mount_grants(MountPermissions::read_only())?;
    grants.push(mount_grant(
        "/tenant-shared/skills",
        "/projects/tenant-shared/skills",
        MountPermissions::read_only(),
    )?);
    MountView::new(grants).map_err(|error| RebornBuildError::InvalidConfig {
        reason: error.to_string(),
    })
}

fn local_dev_skill_mount_grants(
    skill_permissions: MountPermissions,
) -> Result<Vec<MountGrant>, RebornBuildError> {
    Ok(vec![
        mount_grant("/skills", "/projects/skills", skill_permissions)?,
        mount_grant(
            "/system/skills",
            "/projects/system/skills",
            MountPermissions::read_only(),
        )?,
    ])
}

fn mount_grant(
    alias: &str,
    target: &str,
    permissions: MountPermissions,
) -> Result<MountGrant, RebornBuildError> {
    Ok(MountGrant::new(
        MountAlias::new(alias).map_err(|error| RebornBuildError::InvalidConfig {
            reason: error.to_string(),
        })?,
        VirtualPath::new(target).map_err(|error| RebornBuildError::InvalidConfig {
            reason: error.to_string(),
        })?,
        permissions,
    ))
}

fn local_dev_workspace_mount_view() -> Result<MountView, RebornBuildError> {
    MountView::new(vec![MountGrant::new(
        MountAlias::new("/workspace").map_err(|error| RebornBuildError::InvalidConfig {
            reason: error.to_string(),
        })?,
        VirtualPath::new("/projects/workspace").map_err(|error| {
            RebornBuildError::InvalidConfig {
                reason: error.to_string(),
            }
        })?,
        MountPermissions::read_only(),
    )])
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
        required_runtime_backends,
        require_runtime_http_egress,
        require_wasm_credentials,
        product_auth_ports,
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
        product_auth_ports,
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
                product_auth_ports,
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
                product_auth_ports,
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
    product_auth_ports: Option<RebornProductAuthServicePorts>,
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

#[cfg(any(feature = "libsql", feature = "postgres"))]
struct ProductionStoreBundle<F>
where
    F: RootFilesystem + 'static,
{
    filesystem: Arc<F>,
    scoped_filesystem: Arc<ScopedFilesystem<F>>,
    leases: Arc<FilesystemCapabilityLeaseStore<F>>,
    secret_store: Arc<FilesystemSecretStore<F>>,
    event_store: ironclaw_reborn_event_store::RebornEventStoreConfig,
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
impl<F> ProductionStoreBundle<F>
where
    F: RootFilesystem + 'static,
{
    fn new(
        filesystem: Arc<F>,
        secret_master_key: ironclaw_secrets::SecretMaterial,
        event_store: ironclaw_reborn_event_store::RebornEventStoreConfig,
    ) -> Result<Self, RebornBuildError> {
        let scoped_filesystem = crate::wrap_scoped(Arc::clone(&filesystem));
        let leases = Arc::new(FilesystemCapabilityLeaseStore::new(Arc::clone(
            &scoped_filesystem,
        )));
        let secret_crypto = Arc::new(ironclaw_secrets::SecretsCrypto::new(secret_master_key)?);
        let secret_store = Arc::new(FilesystemSecretStore::new(
            Arc::clone(&scoped_filesystem),
            secret_crypto,
        ));

        Ok(Self {
            filesystem,
            scoped_filesystem,
            leases,
            secret_store,
            event_store,
        })
    }
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
async fn build_backend_production<F>(
    context: RebornProductionBuildContext,
    stores: ProductionStoreBundle<F>,
) -> Result<RebornServices, RebornBuildError>
where
    F: RootFilesystem + 'static,
{
    let RebornProductionBuildContext {
        profile,
        wiring_config,
        production_wiring,
        product_auth_ports,
    } = context;
    let services = HostRuntimeServices::new(
        Arc::new(builtin_extension_registry()?),
        Arc::clone(&stores.filesystem),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(GrantAuthorizer::new()),
        ProcessServices::filesystem(Arc::clone(&stores.scoped_filesystem)),
        CapabilitySurfaceVersion::new("reborn-app-v1")?,
    )
    .with_trust_policy(production_wiring.trust_policy)
    .with_runtime_policy(production_wiring.runtime_policy)
    .with_first_party_capabilities(Arc::new(builtin_first_party_registry()?))
    .with_capability_leases(stores.leases)
    .with_secret_store(stores.secret_store)
    .try_with_host_http_egress(ironclaw_network::PolicyNetworkHttpEgress::new(
        ironclaw_network::ReqwestNetworkTransport::default(),
    ))?
    .with_filesystem_resource_governor(Arc::clone(&stores.scoped_filesystem))
    .with_reborn_event_store_config(profile.to_event_store_profile(), stores.event_store)
    .await?
    .with_filesystem_run_state(Arc::clone(&stores.scoped_filesystem))
    .with_filesystem_turn_state_store(stores.scoped_filesystem)
    .with_run_profile_resolver(planned_run_profile_resolver()?)
    .with_turn_run_wake_notifier(production_wiring.turn_run_wake_notifier);

    let turn_coordinator: Arc<dyn ironclaw_turns::TurnCoordinator> =
        Arc::new(services.turn_coordinator_for_production()?);
    let host_runtime: Arc<dyn ironclaw_host_runtime::HostRuntime> =
        Arc::new(services.host_runtime_for_production(&wiring_config)?);
    let product_auth = product_auth_ports
        .map(|ports| compose_product_auth_services(ports, turn_coordinator.clone()));
    let product_auth_ready = product_auth.is_some();

    Ok(RebornServices {
        host_runtime: Some(host_runtime),
        turn_coordinator: Some(turn_coordinator),
        readiness: readiness_for(profile, true, true, product_auth_ready),
        product_auth,
        local_runtime: None,
    })
}

#[cfg(feature = "libsql")]
async fn build_libsql_production(
    context: RebornProductionBuildContext,
    db: Arc<libsql::Database>,
    path_or_url: String,
    auth_token: Option<ironclaw_secrets::SecretMaterial>,
    secret_master_key: ironclaw_secrets::SecretMaterial,
) -> Result<RebornServices, RebornBuildError> {
    use ironclaw_filesystem::LibSqlRootFilesystem;

    let filesystem = Arc::new(LibSqlRootFilesystem::new(Arc::clone(&db)));
    filesystem.run_migrations().await?;
    let stores = ProductionStoreBundle::new(
        filesystem,
        secret_master_key,
        ironclaw_reborn_event_store::RebornEventStoreConfig::Libsql {
            path_or_url,
            auth_token,
        },
    )?;

    build_backend_production(context, stores).await
}

#[cfg(feature = "postgres")]
async fn build_postgres_production(
    context: RebornProductionBuildContext,
    pool: deadpool_postgres::Pool,
    url: ironclaw_secrets::SecretMaterial,
    secret_master_key: ironclaw_secrets::SecretMaterial,
) -> Result<RebornServices, RebornBuildError> {
    use ironclaw_filesystem::PostgresRootFilesystem;

    let filesystem = Arc::new(PostgresRootFilesystem::new(pool.clone()));
    filesystem.run_migrations().await?;
    let stores = ProductionStoreBundle::new(
        filesystem,
        secret_master_key,
        ironclaw_reborn_event_store::RebornEventStoreConfig::Postgres { url },
    )?;

    build_backend_production(context, stores).await
}

fn readiness_for(
    profile: RebornCompositionProfile,
    host_runtime: bool,
    turn_coordinator: bool,
    product_auth: bool,
) -> RebornReadiness {
    let state = match profile {
        RebornCompositionProfile::Disabled => RebornReadinessState::Disabled,
        RebornCompositionProfile::LocalDev | RebornCompositionProfile::LocalDevYolo => {
            RebornReadinessState::DevOnly
        }
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
    use ironclaw_filesystem::FilesystemError;
    use ironclaw_host_api::{
        CapabilityGrant, CapabilityGrantId, CapabilityId, CapabilitySet, EffectKind,
        ExecutionContext, ExtensionId, GrantConstraints, InvocationId, NetworkPolicy, Principal,
        ResourceEstimate, ResourceScope, RuntimeKind, ScopedPath, TrustClass, UserId,
    };
    use ironclaw_host_runtime::{
        RuntimeCapabilityOutcome, RuntimeCapabilityRequest, RuntimeFailureKind,
        SKILL_INSTALL_CAPABILITY_ID, SKILL_LIST_CAPABILITY_ID, SKILL_REMOVE_CAPABILITY_ID,
    };
    use ironclaw_trust::{AuthorityCeiling, EffectiveTrustClass, TrustDecision, TrustProvenance};

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

    #[tokio::test]
    async fn local_dev_setup_marker_workspace_filesystem_is_read_only() {
        let dir = tempfile::tempdir().expect("tempdir");
        let storage_root = dir.path().join("local-dev");
        let marker_path = storage_root.join("workspace/markers/setup.done");
        std::fs::create_dir_all(marker_path.parent().expect("marker parent"))
            .expect("marker directory");
        std::fs::write(&marker_path, "done").expect("marker file");
        let services = build_reborn_services(RebornBuildInput::local_dev(
            "local-dev-marker-workspace-owner",
            storage_root,
        ))
        .await
        .expect("local-dev services build");
        let local_runtime = services
            .local_runtime
            .as_ref()
            .expect("local-dev runtime substrate");
        let scope = ResourceScope::local_default(
            UserId::new("local-dev-marker-user").expect("valid user"),
            InvocationId::new(),
        )
        .expect("valid resource scope");

        let stat = local_runtime
            .workspace_filesystem
            .stat(
                &scope,
                &ScopedPath::new("/workspace/markers/setup.done").expect("valid marker path"),
            )
            .await
            .expect("marker stat succeeds");
        assert_eq!(stat.len, 4);

        let error = local_runtime
            .workspace_filesystem
            .write_file(
                &scope,
                &ScopedPath::new("/workspace/markers/new.done").expect("valid marker path"),
                b"done",
            )
            .await
            .expect_err("setup marker workspace filesystem should be read-only");
        assert!(matches!(error, FilesystemError::PermissionDenied { .. }));
    }

    #[tokio::test]
    async fn local_dev_skill_management_invokes_through_first_party_runtime() {
        let dir = tempfile::tempdir().expect("tempdir");
        let storage_root = dir.path().join("local-dev");
        let services = build_reborn_services(RebornBuildInput::local_dev(
            "local-dev-skill-tools-owner",
            storage_root.clone(),
        ))
        .await
        .expect("local-dev services build");
        let runtime = services.host_runtime.expect("host runtime composed");

        let install_output = invoke_json(
            runtime.as_ref(),
            SKILL_INSTALL_CAPABILITY_ID,
            skill_context(SKILL_INSTALL_CAPABILITY_ID),
            serde_json::json!({
                "content": skill_md("runtime-sentinel", "runtime skill", "RUNTIME_SENTINEL")
            }),
        )
        .await
        .expect("skill install succeeds");
        assert_eq!(install_output["installed"], true);
        assert_eq!(install_output["name"], "runtime-sentinel");
        assert!(
            storage_root
                .join("skills/runtime-sentinel/SKILL.md")
                .exists()
        );

        let list_output = invoke_json(
            runtime.as_ref(),
            SKILL_LIST_CAPABILITY_ID,
            skill_context(SKILL_LIST_CAPABILITY_ID),
            serde_json::json!({}),
        )
        .await
        .expect("skill list succeeds");
        assert!(
            list_output["skills"]
                .as_array()
                .unwrap()
                .iter()
                .any(|skill| { skill["name"] == "runtime-sentinel" && skill["source"] == "user" })
        );

        let remove_output = invoke_json(
            runtime.as_ref(),
            SKILL_REMOVE_CAPABILITY_ID,
            skill_context(SKILL_REMOVE_CAPABILITY_ID),
            serde_json::json!({"name": "runtime-sentinel"}),
        )
        .await
        .expect("skill remove succeeds");
        assert_eq!(remove_output["removed"], true);
        assert!(
            !storage_root
                .join("skills/runtime-sentinel/SKILL.md")
                .exists()
        );
    }

    #[tokio::test]
    async fn local_dev_workspace_mounts_do_not_authorize_skill_writes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let storage_root = dir.path().join("local-dev");
        let services = build_reborn_services(RebornBuildInput::local_dev(
            "local-dev-workspace-skill-boundary-owner",
            storage_root.clone(),
        ))
        .await
        .expect("local-dev services build");
        let runtime = services.host_runtime.expect("host runtime composed");

        let failure = invoke_json(
            runtime.as_ref(),
            "builtin.write_file",
            workspace_context("builtin.write_file"),
            serde_json::json!({
                "path": "/skills/blocked/SKILL.md",
                "content": skill_md("blocked", "blocked skill", "BLOCKED")
            }),
        )
        .await
        .expect_err("workspace tool cannot write skill root");

        assert_eq!(failure, RuntimeFailureKind::Authorization);
        assert!(!storage_root.join("skills/blocked/SKILL.md").exists());
    }

    #[test]
    fn local_dev_workspace_root_overlapping_skill_root_is_rejected() {
        let dir = tempfile::tempdir().expect("tempdir");
        let storage_root = dir.path().join("local-dev");

        for skill_root in [
            storage_root.join("skills"),
            storage_root.join("tenant-shared/skills"),
            storage_root.join("system/skills"),
        ] {
            for workspace_root in [
                skill_root.clone(),
                skill_root
                    .parent()
                    .expect("skill root parent")
                    .to_path_buf(),
                skill_root.join("nested-workspace"),
            ] {
                let error =
                    validate_local_dev_workspace_skill_isolation(&storage_root, &workspace_root)
                        .expect_err("workspace root overlapping skill root should be rejected");
                assert!(
                    matches!(error, RebornBuildError::InvalidConfig { .. }),
                    "unexpected error: {error:?}"
                );
            }
        }
    }

    #[test]
    fn builtin_first_party_package_declares_skill_management_tools() {
        let package = builtin_first_party_package().expect("built-in package builds");
        let ids = package
            .capabilities
            .iter()
            .map(|capability| capability.id.as_str())
            .collect::<Vec<_>>();
        assert!(ids.contains(&SKILL_LIST_CAPABILITY_ID));
        assert!(ids.contains(&SKILL_INSTALL_CAPABILITY_ID));
        assert!(ids.contains(&SKILL_REMOVE_CAPABILITY_ID));

        let registry = builtin_first_party_registry().expect("built-in handlers build");
        for id in [
            SKILL_LIST_CAPABILITY_ID,
            SKILL_INSTALL_CAPABILITY_ID,
            SKILL_REMOVE_CAPABILITY_ID,
        ] {
            assert!(registry.contains_handler(&ironclaw_host_api::CapabilityId::new(id).unwrap()));
        }
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

    async fn invoke_json(
        runtime: &dyn ironclaw_host_runtime::HostRuntime,
        capability_id: &str,
        context: ExecutionContext,
        input: serde_json::Value,
    ) -> Result<serde_json::Value, RuntimeFailureKind> {
        let outcome = runtime
            .invoke_capability(RuntimeCapabilityRequest::new(
                context,
                CapabilityId::new(capability_id).expect("valid capability id"),
                ResourceEstimate::default(),
                input,
                trust_decision(),
            ))
            .await
            .expect("runtime invocation completes");
        match outcome {
            RuntimeCapabilityOutcome::Completed(completed) => Ok(completed.output),
            RuntimeCapabilityOutcome::Failed(failure) => Err(failure.kind),
            other => panic!("unexpected runtime outcome: {other:?}"),
        }
    }

    fn skill_context(capability_id: &str) -> ExecutionContext {
        execution_context(capability_id, skill_mounts())
    }

    fn workspace_context(capability_id: &str) -> ExecutionContext {
        execution_context(capability_id, workspace_mounts())
    }

    fn execution_context(capability_id: &str, mounts: MountView) -> ExecutionContext {
        let extension_id = ExtensionId::new("caller").expect("valid extension id");
        ExecutionContext::local_default(
            UserId::new("local-dev-test-user").expect("valid user id"),
            extension_id.clone(),
            RuntimeKind::FirstParty,
            TrustClass::FirstParty,
            CapabilitySet {
                grants: vec![capability_grant(
                    capability_id,
                    extension_id,
                    mounts.clone(),
                )],
            },
            mounts,
        )
        .expect("valid execution context")
    }

    fn capability_grant(
        capability_id: &str,
        grantee: ExtensionId,
        mounts: MountView,
    ) -> CapabilityGrant {
        CapabilityGrant {
            id: CapabilityGrantId::new(),
            capability: CapabilityId::new(capability_id).expect("valid capability id"),
            grantee: Principal::Extension(grantee),
            issued_by: Principal::HostRuntime,
            constraints: GrantConstraints {
                allowed_effects: allowed_effects(),
                mounts,
                network: NetworkPolicy::default(),
                secrets: Vec::new(),
                resource_ceiling: None,
                expires_at: None,
                max_invocations: None,
            },
        }
    }

    fn skill_mounts() -> MountView {
        MountView::new(vec![
            MountGrant::new(
                MountAlias::new("/skills").expect("valid mount alias"),
                VirtualPath::new("/projects/skills").expect("valid virtual path"),
                MountPermissions::read_write_list_delete(),
            ),
            MountGrant::new(
                MountAlias::new("/system/skills").expect("valid mount alias"),
                VirtualPath::new("/projects/system/skills").expect("valid virtual path"),
                MountPermissions::read_only(),
            ),
        ])
        .expect("valid mount view")
    }

    fn workspace_mounts() -> MountView {
        MountView::new(vec![MountGrant::new(
            MountAlias::new("/workspace").expect("valid mount alias"),
            VirtualPath::new("/projects/workspace").expect("valid virtual path"),
            MountPermissions::read_write(),
        )])
        .expect("valid mount view")
    }

    fn allowed_effects() -> Vec<EffectKind> {
        vec![
            EffectKind::DispatchCapability,
            EffectKind::ReadFilesystem,
            EffectKind::WriteFilesystem,
        ]
    }

    fn trust_decision() -> TrustDecision {
        TrustDecision {
            effective_trust: EffectiveTrustClass::user_trusted(),
            authority_ceiling: AuthorityCeiling {
                allowed_effects: allowed_effects(),
                max_resource_ceiling: None,
            },
            provenance: TrustProvenance::Default,
            evaluated_at: chrono::Utc::now(),
        }
    }

    fn skill_md(name: &str, description: &str, prompt: &str) -> String {
        format!("---\nname: {name}\ndescription: {description}\n---\n{prompt}\n")
    }
}

#[cfg(test)]
mod auth_tests;
