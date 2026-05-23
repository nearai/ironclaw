use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

#[cfg(feature = "libsql")]
use ironclaw_authorization::FilesystemCapabilityLeaseStore;
use ironclaw_authorization::GrantAuthorizer;
#[cfg(feature = "libsql")]
use ironclaw_events::DurableEventLog;
#[cfg(not(feature = "libsql"))]
use ironclaw_events::{DurableEventLog, InMemoryDurableEventLog};
use ironclaw_extensions::ExtensionRegistry;
#[cfg(feature = "libsql")]
use ironclaw_filesystem::{
    BackendCapabilities, BackendId, BackendKind, Capability, CompositeRootFilesystem, ContentKind,
    IndexPolicy, LibSqlRootFilesystem, MountDescriptor, RootFilesystem, StorageClass,
};
use ironclaw_filesystem::{LocalFilesystem, ScopedFilesystem};
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_host_api::runtime_policy::EffectiveRuntimePolicy;
use ironclaw_host_api::{
    EffectKind, HostPath, MountAlias, MountGrant, MountPermissions, MountView, PackageId,
    VirtualPath,
};
use ironclaw_host_runtime::{
    CapabilitySurfaceVersion, FirstPartyCapabilityRegistry, HostRuntimeServices,
    builtin_first_party_handlers, builtin_first_party_package,
};
#[cfg(feature = "libsql")]
use ironclaw_loop_support::FilesystemCheckpointStateStore;
use ironclaw_processes::ProcessServices;
use ironclaw_resources::InMemoryResourceGovernor;
#[cfg(feature = "libsql")]
use ironclaw_resources::{FilesystemResourceGovernorStore, PersistentResourceGovernor};
#[cfg(feature = "libsql")]
use ironclaw_run_state::{FilesystemApprovalRequestStore, FilesystemRunStateStore};
#[cfg(not(feature = "libsql"))]
use ironclaw_run_state::{InMemoryApprovalRequestStore, InMemoryRunStateStore};
#[cfg(feature = "libsql")]
use ironclaw_threads::FilesystemSessionThreadService;
#[cfg(not(feature = "libsql"))]
use ironclaw_threads::InMemorySessionThreadService;
use ironclaw_threads::SessionThreadService;
use ironclaw_trust::{AdminConfig, AdminEntry, HostTrustAssignment, HostTrustPolicy};
#[cfg(feature = "libsql")]
use ironclaw_turns::FilesystemTurnStateStore;
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_turns::InMemoryRunProfileResolver;
use ironclaw_turns::{CheckpointStateStore, DefaultTurnCoordinator, LoopCheckpointStore};
#[cfg(not(feature = "libsql"))]
use ironclaw_turns::{
    InMemoryCheckpointStateStore, InMemoryLoopCheckpointStore, InMemoryTurnStateStore,
};

use crate::input::RebornStorageInput;
use crate::{
    RebornBuildError, RebornBuildInput, RebornCompositionProfile, RebornFacadeReadiness,
    RebornReadiness, RebornReadinessState,
};

#[cfg(feature = "libsql")]
pub(crate) type LocalDevRootFilesystem = CompositeRootFilesystem;
#[cfg(not(feature = "libsql"))]
pub(crate) type LocalDevRootFilesystem = LocalFilesystem;

#[cfg(feature = "libsql")]
pub(crate) type LocalDevTurnStateStore = FilesystemTurnStateStore<LocalDevRootFilesystem>;
#[cfg(not(feature = "libsql"))]
pub(crate) type LocalDevTurnStateStore = InMemoryTurnStateStore;

#[cfg(feature = "libsql")]
type LocalDevResourceGovernor =
    PersistentResourceGovernor<FilesystemResourceGovernorStore<LocalDevRootFilesystem>>;
#[cfg(not(feature = "libsql"))]
type LocalDevResourceGovernor = InMemoryResourceGovernor;

#[cfg(feature = "libsql")]
type LocalDevRunStateStore = FilesystemRunStateStore<LocalDevRootFilesystem>;
#[cfg(not(feature = "libsql"))]
type LocalDevRunStateStore = InMemoryRunStateStore;

#[cfg(feature = "libsql")]
type LocalDevApprovalRequestStore = FilesystemApprovalRequestStore<LocalDevRootFilesystem>;
#[cfg(not(feature = "libsql"))]
type LocalDevApprovalRequestStore = InMemoryApprovalRequestStore;

#[cfg(feature = "libsql")]
type LocalDevProcessServices = ProcessServices<
    ironclaw_processes::FilesystemProcessStore<LocalDevRootFilesystem>,
    ironclaw_processes::FilesystemProcessResultStore<LocalDevRootFilesystem>,
>;
#[cfg(not(feature = "libsql"))]
type LocalDevProcessServices = ProcessServices<
    ironclaw_processes::InMemoryProcessStore,
    ironclaw_processes::InMemoryProcessResultStore,
>;

pub struct RebornServices {
    pub host_runtime: Option<Arc<dyn ironclaw_host_runtime::HostRuntime>>,
    pub turn_coordinator: Option<Arc<dyn ironclaw_turns::TurnCoordinator>>,
    pub readiness: RebornReadiness,
    pub(crate) local_runtime: Option<Arc<RebornLocalRuntimeServices>>,
}

pub(crate) struct RebornLocalRuntimeServices {
    pub(crate) turn_state: Arc<LocalDevTurnStateStore>,
    pub(crate) checkpoint_state_store: Arc<dyn CheckpointStateStore>,
    pub(crate) loop_checkpoint_store: Arc<dyn LoopCheckpointStore>,
    pub(crate) thread_service: Arc<dyn SessionThreadService>,
    pub(crate) skill_filesystem: Arc<ScopedFilesystem<LocalDevRootFilesystem>>,
    pub(crate) event_log: Arc<dyn DurableEventLog>,
}

struct RebornLocalDevStoreGraph {
    run_state: Arc<LocalDevRunStateStore>,
    approval_requests: Arc<LocalDevApprovalRequestStore>,
    turn_state: Arc<LocalDevTurnStateStore>,
    local_runtime: Arc<RebornLocalRuntimeServices>,
    resource_governor: Arc<LocalDevResourceGovernor>,
    process_services: LocalDevProcessServices,
    #[cfg(feature = "libsql")]
    capability_leases: Arc<FilesystemCapabilityLeaseStore<LocalDevRootFilesystem>>,
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
    let filesystem = build_local_dev_root_filesystem(&root, &workspace_root).await?;
    let skill_filesystem = Arc::new(ScopedFilesystem::with_fixed_view(
        Arc::clone(&filesystem),
        local_dev_skill_mount_view()?,
    ));
    let store_graph = build_local_dev_store_graph(Arc::clone(&filesystem), skill_filesystem)?;

    let mut services = HostRuntimeServices::new(
        Arc::new(builtin_extension_registry()?),
        filesystem,
        Arc::clone(&store_graph.resource_governor),
        Arc::new(GrantAuthorizer::new()),
        store_graph.process_services.clone(),
        CapabilitySurfaceVersion::new("reborn-app-v1")?,
    )
    .with_first_party_capabilities(Arc::new(builtin_first_party_registry()?))
    .with_trust_policy(Arc::new(local_dev_first_party_trust_policy()?))
    .with_secret_store(Arc::new(ironclaw_secrets::InMemorySecretStore::new()))
    .try_with_host_http_egress(ironclaw_network::PolicyNetworkHttpEgress::new(
        ironclaw_network::ReqwestNetworkTransport::default(),
    ))?
    .with_run_state(Arc::clone(&store_graph.run_state))
    .with_approval_requests(Arc::clone(&store_graph.approval_requests))
    .with_turn_state_and_transition_port(Arc::clone(&store_graph.turn_state));
    #[cfg(feature = "libsql")]
    {
        services = services.with_capability_leases(Arc::clone(&store_graph.capability_leases));
    }
    if let Some(runtime_policy) = input.runtime_policy {
        services = services.with_runtime_policy(runtime_policy);
    }
    // TODO(process-port): local-dev intentionally uses the default
    // LocalHostProcessPort until a non-local process backend is composed.

    let host_runtime: Arc<dyn ironclaw_host_runtime::HostRuntime> =
        Arc::new(services.host_runtime_for_local_testing());
    let turn_coordinator: Arc<dyn ironclaw_turns::TurnCoordinator> = Arc::new(
        DefaultTurnCoordinator::new(Arc::clone(&store_graph.turn_state)),
    );

    Ok(RebornServices {
        host_runtime: Some(host_runtime),
        turn_coordinator: Some(turn_coordinator),
        readiness: readiness_for(input.profile, true, true),
        local_runtime: Some(store_graph.local_runtime),
    })
}

#[cfg(feature = "libsql")]
fn build_local_dev_store_graph(
    filesystem: Arc<LocalDevRootFilesystem>,
    skill_filesystem: Arc<ScopedFilesystem<LocalDevRootFilesystem>>,
) -> Result<RebornLocalDevStoreGraph, RebornBuildError> {
    let scoped_filesystem = local_dev_scoped_filesystem(Arc::clone(&filesystem));
    let event_log = local_dev_event_log(filesystem)?;
    let run_state = Arc::new(FilesystemRunStateStore::new(Arc::clone(&scoped_filesystem)));
    let approval_requests = Arc::new(FilesystemApprovalRequestStore::new(Arc::clone(
        &scoped_filesystem,
    )));
    let turn_state = Arc::new(FilesystemTurnStateStore::new(Arc::clone(
        &scoped_filesystem,
    )));
    let checkpoint_state_store: Arc<dyn CheckpointStateStore> = Arc::new(
        FilesystemCheckpointStateStore::new(Arc::clone(&scoped_filesystem)),
    );
    let loop_checkpoint_store: Arc<dyn LoopCheckpointStore> = turn_state.clone();
    let thread_service: Arc<dyn SessionThreadService> = Arc::new(
        FilesystemSessionThreadService::new(Arc::clone(&scoped_filesystem)),
    );
    let local_runtime = Arc::new(RebornLocalRuntimeServices {
        turn_state: Arc::clone(&turn_state),
        checkpoint_state_store,
        loop_checkpoint_store,
        thread_service,
        skill_filesystem,
        event_log,
    });
    let resource_governor: Arc<LocalDevResourceGovernor> =
        Arc::new(PersistentResourceGovernor::new(
            FilesystemResourceGovernorStore::new(Arc::clone(&scoped_filesystem)),
        ));
    let process_services = ProcessServices::filesystem(Arc::clone(&scoped_filesystem));
    let capability_leases = Arc::new(FilesystemCapabilityLeaseStore::new(scoped_filesystem));

    Ok(RebornLocalDevStoreGraph {
        run_state,
        approval_requests,
        turn_state,
        local_runtime,
        resource_governor,
        process_services,
        capability_leases,
    })
}

#[cfg(not(feature = "libsql"))]
fn build_local_dev_store_graph(
    filesystem: Arc<LocalDevRootFilesystem>,
    skill_filesystem: Arc<ScopedFilesystem<LocalDevRootFilesystem>>,
) -> Result<RebornLocalDevStoreGraph, RebornBuildError> {
    let event_log = local_dev_event_log(filesystem)?;
    let run_state = Arc::new(InMemoryRunStateStore::new());
    let approval_requests = Arc::new(InMemoryApprovalRequestStore::new());
    let turn_state = Arc::new(InMemoryTurnStateStore::default());
    let checkpoint_state_store: Arc<dyn CheckpointStateStore> =
        Arc::new(InMemoryCheckpointStateStore::default());
    let loop_checkpoint_store: Arc<dyn LoopCheckpointStore> =
        Arc::new(InMemoryLoopCheckpointStore::default());
    let thread_service: Arc<dyn SessionThreadService> =
        Arc::new(InMemorySessionThreadService::default());
    let local_runtime = Arc::new(RebornLocalRuntimeServices {
        turn_state: Arc::clone(&turn_state),
        checkpoint_state_store,
        loop_checkpoint_store,
        thread_service,
        skill_filesystem,
        event_log,
    });
    let resource_governor: Arc<LocalDevResourceGovernor> =
        Arc::new(InMemoryResourceGovernor::new());
    let process_services = ProcessServices::in_memory();

    Ok(RebornLocalDevStoreGraph {
        run_state,
        approval_requests,
        turn_state,
        local_runtime,
        resource_governor,
        process_services,
    })
}

#[cfg(feature = "libsql")]
async fn build_local_dev_root_filesystem(
    root: &Path,
    workspace_root: &Path,
) -> Result<Arc<LocalDevRootFilesystem>, RebornBuildError> {
    let db_path = root.join("reborn-local-dev.db");
    let db = Arc::new(
        libsql::Builder::new_local(&db_path)
            .build()
            .await
            .map_err(|error| RebornBuildError::InvalidConfig {
                reason: format!("local-dev libSQL database could not be opened: {error}"),
            })?,
    );
    let database = Arc::new(LibSqlRootFilesystem::new(db));
    database.run_migrations().await?;

    let local = Arc::new(local_dev_project_filesystem(root, workspace_root)?);
    let mut root = CompositeRootFilesystem::new();
    root.mount(
        local_dev_mount_descriptor(
            "/tenants",
            "local-dev-reborn-state",
            BackendKind::DatabaseFilesystem,
            StorageClass::StructuredRecords,
            ContentKind::StructuredRecord,
            IndexPolicy::NotIndexed,
            database.capabilities(),
        )?,
        Arc::clone(&database),
    )?;
    root.mount(
        local_dev_mount_descriptor(
            "/events",
            "local-dev-events",
            BackendKind::DatabaseFilesystem,
            StorageClass::StructuredRecords,
            ContentKind::StructuredRecord,
            IndexPolicy::NotIndexed,
            database.capabilities(),
        )?,
        database,
    )?;
    root.mount(
        local_dev_mount_descriptor(
            "/projects",
            "local-dev-project-files",
            BackendKind::LocalFilesystem,
            StorageClass::FileContent,
            ContentKind::ProjectFile,
            IndexPolicy::NotIndexed,
            local_dev_bytes_capabilities(),
        )?,
        local,
    )?;
    Ok(Arc::new(root))
}

#[cfg(not(feature = "libsql"))]
async fn build_local_dev_root_filesystem(
    root: &Path,
    workspace_root: &Path,
) -> Result<Arc<LocalDevRootFilesystem>, RebornBuildError> {
    Ok(Arc::new(local_dev_project_filesystem(
        root,
        workspace_root,
    )?))
}

fn local_dev_project_filesystem(
    root: &Path,
    workspace_root: &Path,
) -> Result<LocalFilesystem, RebornBuildError> {
    let mut filesystem = LocalFilesystem::new();
    filesystem.mount_local(
        VirtualPath::new("/projects")?,
        HostPath::from_path_buf(root.to_path_buf()),
    )?;
    filesystem.mount_local(
        VirtualPath::new("/projects/workspace")?,
        HostPath::from_path_buf(workspace_root.to_path_buf()),
    )?;
    Ok(filesystem)
}

#[cfg(feature = "libsql")]
fn local_dev_mount_descriptor(
    virtual_root: &str,
    backend_id: &str,
    backend_kind: BackendKind,
    storage_class: StorageClass,
    content_kind: ContentKind,
    index_policy: IndexPolicy,
    capabilities: BackendCapabilities,
) -> Result<MountDescriptor, RebornBuildError> {
    Ok(MountDescriptor {
        virtual_root: VirtualPath::new(virtual_root)?,
        backend_id: BackendId::new(backend_id)?,
        backend_kind,
        storage_class,
        content_kind,
        index_policy,
        capabilities,
    })
}

#[cfg(feature = "libsql")]
fn local_dev_bytes_capabilities() -> BackendCapabilities {
    BackendCapabilities::empty()
        .with(Capability::Read)
        .with(Capability::Write)
        .with(Capability::Append)
        .with(Capability::List)
        .with(Capability::Stat)
        .with(Capability::Delete)
}

#[cfg(feature = "libsql")]
fn local_dev_scoped_filesystem(
    filesystem: Arc<LocalDevRootFilesystem>,
) -> Arc<ScopedFilesystem<LocalDevRootFilesystem>> {
    crate::wrap_scoped(filesystem)
}

#[cfg(feature = "libsql")]
fn local_dev_event_log(
    filesystem: Arc<LocalDevRootFilesystem>,
) -> Result<Arc<dyn DurableEventLog>, RebornBuildError> {
    let scoped = Arc::new(ScopedFilesystem::with_fixed_view(
        filesystem,
        MountView::new(vec![MountGrant::new(
            MountAlias::new("/events")?,
            VirtualPath::new("/events")?,
            MountPermissions::read_write_list_delete(),
        )])?,
    ));
    Ok(Arc::new(
        ironclaw_reborn_event_store::FilesystemDurableEventLog::new(scoped),
    ))
}

#[cfg(not(feature = "libsql"))]
fn local_dev_event_log(
    _filesystem: Arc<LocalDevRootFilesystem>,
) -> Result<Arc<dyn DurableEventLog>, RebornBuildError> {
    Ok(Arc::new(InMemoryDurableEventLog::new()))
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
            )?;
            // TODO(process-port): if production enables FirstParty runtime,
            // HostRuntimeServices must be given a production process port;
            // otherwise the LocalHostProcessPort default is rejected.
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
            )?;
            // TODO(process-port): if production enables FirstParty runtime,
            // HostRuntimeServices must be given a production process port;
            // otherwise the LocalHostProcessPort default is rejected.
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

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn local_dev_services_persist_thread_records_across_rebuilds() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("local-dev");
        let scope = ironclaw_threads::ThreadScope {
            tenant_id: ironclaw_host_api::TenantId::new("persist-tenant").unwrap(),
            agent_id: ironclaw_host_api::AgentId::new("persist-agent").unwrap(),
            project_id: None,
            owner_user_id: Some(ironclaw_host_api::UserId::new("persist-owner").unwrap()),
            mission_id: None,
        };
        let thread_id = ironclaw_host_api::ThreadId::new("persisted-thread").unwrap();

        let services =
            build_reborn_services(RebornBuildInput::local_dev("persist-owner", root.clone()))
                .await
                .expect("first local-dev services build");
        services
            .local_runtime
            .as_ref()
            .expect("local runtime")
            .thread_service
            .ensure_thread(ironclaw_threads::EnsureThreadRequest {
                scope: scope.clone(),
                thread_id: Some(thread_id.clone()),
                created_by_actor_id: "persist-owner".to_string(),
                title: Some("Persisted thread".to_string()),
                metadata_json: None,
            })
            .await
            .expect("persist thread");
        drop(services);

        let rebuilt =
            build_reborn_services(RebornBuildInput::local_dev("persist-owner", root.clone()))
                .await
                .expect("rebuilt local-dev services");
        let history = rebuilt
            .local_runtime
            .as_ref()
            .expect("rebuilt local runtime")
            .thread_service
            .list_thread_history(ironclaw_threads::ThreadHistoryRequest {
                scope,
                thread_id: thread_id.clone(),
            })
            .await
            .expect("read persisted thread");

        assert_eq!(history.thread.thread_id, thread_id);
        assert!(
            root.join("reborn-local-dev.db").exists(),
            "local-dev should use a libSQL database under the local-dev root"
        );
    }

    #[test]
    fn disabled_services_do_not_include_repl_runtime_substrate() {
        let services = RebornServices::disabled();

        assert!(services.host_runtime.is_none());
        assert!(services.turn_coordinator.is_none());
        assert!(services.local_runtime.is_none());
        assert_eq!(services.readiness.state, RebornReadinessState::Disabled);
    }
}
