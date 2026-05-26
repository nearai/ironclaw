use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_authorization::FilesystemCapabilityLeaseStore;
use ironclaw_authorization::GrantAuthorizer;
#[cfg(not(feature = "libsql"))]
use ironclaw_authorization::InMemoryCapabilityLeaseStore;
#[cfg(feature = "libsql")]
use ironclaw_events::{DurableAuditLog, DurableEventLog};
#[cfg(not(feature = "libsql"))]
use ironclaw_events::{
    DurableAuditLog, DurableEventLog, InMemoryDurableAuditLog, InMemoryDurableEventLog,
};
use ironclaw_extensions::ExtensionRegistry;
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_filesystem::RootFilesystem;
#[cfg(feature = "libsql")]
use ironclaw_filesystem::{
    BackendCapabilities, BackendId, BackendKind, Capability, CompositeRootFilesystem, ContentKind,
    IndexPolicy, LibSqlRootFilesystem, MountDescriptor, StorageClass,
};
use ironclaw_filesystem::{LocalFilesystem, ScopedFilesystem};
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_host_api::runtime_policy::EffectiveRuntimePolicy;
use ironclaw_host_api::runtime_policy::FilesystemBackendKind;
use ironclaw_host_api::{
    EffectKind, HostPath, MountPermissions, MountView, PackageId, VirtualPath,
};
#[cfg(feature = "libsql")]
use ironclaw_host_api::{MountAlias, MountGrant};
use ironclaw_host_runtime::{
    CapabilitySurfaceVersion, FirstPartyCapabilityRegistry, HostRuntimeServices,
    builtin_first_party_handlers, builtin_first_party_package,
};
#[cfg(feature = "libsql")]
use ironclaw_loop_support::FilesystemCheckpointStateStore;
use ironclaw_processes::ProcessServices;
use ironclaw_product_workflow::ProductAuthTurnGateResumeDispatcher;
use ironclaw_resources::InMemoryResourceGovernor;
#[cfg(feature = "libsql")]
use ironclaw_resources::{FilesystemResourceGovernorStore, PersistentResourceGovernor};
#[cfg(feature = "libsql")]
use ironclaw_run_state::{FilesystemApprovalRequestStore, FilesystemRunStateStore};
#[cfg(not(feature = "libsql"))]
use ironclaw_run_state::{InMemoryApprovalRequestStore, InMemoryRunStateStore};
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_secrets::FilesystemSecretStore;
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

use crate::input::{RebornRuntimeProcessBinding, RebornStorageInput};
use crate::local_dev_mounts::{skill_context_mount_view, workspace_mount_view};
use crate::{
    RebornAuthContinuationDispatcher, RebornBuildError, RebornBuildInput, RebornCompositionProfile,
    RebornFacadeReadiness, RebornProductAuthServicePorts, RebornProductAuthServices,
    RebornReadiness, RebornReadinessState,
};

#[cfg(feature = "libsql")]
pub(crate) type LocalDevRootFilesystem = CompositeRootFilesystem;
#[cfg(not(feature = "libsql"))]
pub(crate) type LocalDevRootFilesystem = LocalFilesystem;

type LocalDevWorkspaceFilesystems = (
    Arc<ScopedFilesystem<LocalDevRootFilesystem>>,
    Arc<ScopedFilesystem<LocalDevRootFilesystem>>,
    MountView,
);

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
pub(crate) type LocalDevApprovalRequestStore =
    FilesystemApprovalRequestStore<LocalDevRootFilesystem>;
#[cfg(not(feature = "libsql"))]
pub(crate) type LocalDevApprovalRequestStore = InMemoryApprovalRequestStore;

#[cfg(feature = "libsql")]
pub(crate) type LocalDevCapabilityLeaseStore =
    FilesystemCapabilityLeaseStore<LocalDevRootFilesystem>;
#[cfg(not(feature = "libsql"))]
pub(crate) type LocalDevCapabilityLeaseStore = InMemoryCapabilityLeaseStore;

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

fn apply_runtime_process_binding<F, G, S, R>(
    services: HostRuntimeServices<F, G, S, R>,
    binding: RebornRuntimeProcessBinding,
) -> HostRuntimeServices<F, G, S, R>
where
    F: ironclaw_filesystem::RootFilesystem + 'static,
    G: ironclaw_resources::ResourceGovernor + 'static,
    S: ironclaw_processes::ProcessStore + 'static,
    R: ironclaw_processes::ProcessResultStore + 'static,
{
    match binding {
        RebornRuntimeProcessBinding::None => services,
        RebornRuntimeProcessBinding::TenantSandbox { process_port } => {
            services.with_tenant_sandbox_process_port(process_port)
        }
    }
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
pub(crate) fn apply_production_runtime_process_binding<F, G, S, R>(
    services: HostRuntimeServices<F, G, S, R>,
    binding: RebornRuntimeProcessBinding,
) -> HostRuntimeServices<F, G, S, R>
where
    F: ironclaw_filesystem::RootFilesystem + 'static,
    G: ironclaw_resources::ResourceGovernor + 'static,
    S: ironclaw_processes::ProcessStore + 'static,
    R: ironclaw_processes::ProcessResultStore + 'static,
{
    match binding {
        RebornRuntimeProcessBinding::None => services,
        RebornRuntimeProcessBinding::TenantSandbox { process_port } => {
            services.with_production_tenant_sandbox_process_port(process_port)
        }
    }
}

pub struct RebornServices {
    pub host_runtime: Option<Arc<dyn ironclaw_host_runtime::HostRuntime>>,
    pub turn_coordinator: Option<Arc<dyn ironclaw_turns::TurnCoordinator>>,
    pub product_auth: Option<Arc<RebornProductAuthServices>>,
    pub readiness: RebornReadiness,
    pub(crate) local_runtime: Option<Arc<RebornLocalRuntimeServices>>,
}

pub(crate) struct RebornLocalRuntimeServices {
    pub(crate) approval_requests: Arc<LocalDevApprovalRequestStore>,
    pub(crate) capability_leases: Arc<LocalDevCapabilityLeaseStore>,
    pub(crate) turn_state: Arc<LocalDevTurnStateStore>,
    pub(crate) checkpoint_state_store: Arc<dyn CheckpointStateStore>,
    pub(crate) loop_checkpoint_store: Arc<dyn LoopCheckpointStore>,
    pub(crate) thread_service: Arc<dyn SessionThreadService>,
    pub(crate) skill_mounts: MountView,
    pub(crate) skill_filesystem: Arc<ScopedFilesystem<LocalDevRootFilesystem>>,
    pub(crate) workspace_filesystem: Arc<ScopedFilesystem<LocalDevRootFilesystem>>,
    pub(crate) workspace_mounts: MountView,
    pub(crate) event_log: Arc<dyn DurableEventLog>,
    pub(crate) audit_log: Arc<dyn DurableAuditLog>,
}

struct RebornLocalDevStoreGraph {
    run_state: Arc<LocalDevRunStateStore>,
    approval_requests: Arc<LocalDevApprovalRequestStore>,
    capability_leases: Arc<LocalDevCapabilityLeaseStore>,
    turn_state: Arc<LocalDevTurnStateStore>,
    local_runtime: Arc<RebornLocalRuntimeServices>,
    resource_governor: Arc<LocalDevResourceGovernor>,
    process_services: LocalDevProcessServices,
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
        storage,
        runtime_policy,
        runtime_process_binding,
        product_auth_ports,
        ..
    } = input;
    let RebornStorageInput::LocalDev {
        root,
        workspace_root,
        host_home_root,
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
    let include_host_home = runtime_policy.as_ref().is_some_and(|policy| {
        policy.filesystem_backend == FilesystemBackendKind::HostWorkspaceAndHome
    });
    let host_home_root = match (include_host_home, host_home_root) {
        (true, Some(path)) => Some(LocalDevHostHomeRoot {
            canonical_root: canonicalize_local_dev_host_home_root(&path)?,
            raw_alias: path,
        }),
        (true, None) => {
            return Err(RebornBuildError::InvalidConfig {
                reason: "local-dev-yolo host home access requires a confirmed host home root"
                    .to_string(),
            });
        }
        (false, Some(_)) => {
            return Err(RebornBuildError::InvalidConfig {
                reason:
                    "confirmed host home root was supplied but the resolved runtime policy does not allow host home access"
                        .to_string(),
            });
        }
        (false, None) => None,
    };
    validate_local_dev_workspace_skill_isolation(&root, &workspace_root)?;
    let filesystem =
        build_local_dev_root_filesystem(&root, &workspace_root, host_home_root.as_ref()).await?;
    let (skill_filesystem, workspace_filesystem, runtime_workspace_mounts) =
        build_workspace_filesystems(Arc::clone(&filesystem), host_home_root.as_ref())?;
    let store_graph = build_local_dev_store_graph(
        Arc::clone(&filesystem),
        skill_filesystem,
        workspace_filesystem,
        runtime_workspace_mounts,
    )?;

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
    .with_capability_leases(Arc::clone(&store_graph.capability_leases))
    .with_turn_state_and_transition_port(Arc::clone(&store_graph.turn_state));
    if let Some(runtime_policy) = runtime_policy {
        services = services.with_runtime_policy(runtime_policy);
    }
    services = apply_runtime_process_binding(services, runtime_process_binding);

    let host_runtime: Arc<dyn ironclaw_host_runtime::HostRuntime> =
        Arc::new(services.host_runtime_for_local_testing());
    let turn_coordinator: Arc<dyn ironclaw_turns::TurnCoordinator> = Arc::new(
        DefaultTurnCoordinator::new(Arc::clone(&store_graph.turn_state)),
    );
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
        local_runtime: Some(store_graph.local_runtime),
    })
}

#[cfg(feature = "libsql")]
fn build_local_dev_store_graph(
    filesystem: Arc<LocalDevRootFilesystem>,
    skill_filesystem: Arc<ScopedFilesystem<LocalDevRootFilesystem>>,
    workspace_filesystem: Arc<ScopedFilesystem<LocalDevRootFilesystem>>,
    workspace_mounts: MountView,
) -> Result<RebornLocalDevStoreGraph, RebornBuildError> {
    let scoped_filesystem = local_dev_scoped_filesystem(Arc::clone(&filesystem));
    let event_log = local_dev_event_log(Arc::clone(&filesystem))?;
    let audit_log = local_dev_audit_log(filesystem)?;
    let run_state = Arc::new(FilesystemRunStateStore::new(Arc::clone(&scoped_filesystem)));
    let approval_requests = Arc::new(FilesystemApprovalRequestStore::new(Arc::clone(
        &scoped_filesystem,
    )));
    let capability_leases = Arc::new(FilesystemCapabilityLeaseStore::new(Arc::clone(
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
    let skill_mounts =
        skill_context_mount_view().map_err(|error| RebornBuildError::InvalidConfig {
            reason: error.to_string(),
        })?;
    let local_runtime = Arc::new(RebornLocalRuntimeServices {
        approval_requests: Arc::clone(&approval_requests),
        capability_leases: Arc::clone(&capability_leases),
        turn_state: Arc::clone(&turn_state),
        checkpoint_state_store,
        loop_checkpoint_store,
        thread_service,
        skill_mounts,
        skill_filesystem,
        workspace_filesystem,
        workspace_mounts,
        event_log,
        audit_log,
    });
    let resource_governor: Arc<LocalDevResourceGovernor> =
        Arc::new(PersistentResourceGovernor::new(
            FilesystemResourceGovernorStore::new(Arc::clone(&scoped_filesystem)),
        ));
    let process_services = ProcessServices::filesystem(Arc::clone(&scoped_filesystem));

    Ok(RebornLocalDevStoreGraph {
        run_state,
        approval_requests,
        capability_leases,
        turn_state,
        local_runtime,
        resource_governor,
        process_services,
    })
}

#[cfg(not(feature = "libsql"))]
fn build_local_dev_store_graph(
    filesystem: Arc<LocalDevRootFilesystem>,
    skill_filesystem: Arc<ScopedFilesystem<LocalDevRootFilesystem>>,
    workspace_filesystem: Arc<ScopedFilesystem<LocalDevRootFilesystem>>,
    workspace_mounts: MountView,
) -> Result<RebornLocalDevStoreGraph, RebornBuildError> {
    let event_log = local_dev_event_log(Arc::clone(&filesystem))?;
    let audit_log = local_dev_audit_log(filesystem)?;
    let run_state = Arc::new(InMemoryRunStateStore::new());
    let approval_requests = Arc::new(InMemoryApprovalRequestStore::new());
    let capability_leases = Arc::new(InMemoryCapabilityLeaseStore::new());
    let turn_state = Arc::new(InMemoryTurnStateStore::default());
    let checkpoint_state_store: Arc<dyn CheckpointStateStore> =
        Arc::new(InMemoryCheckpointStateStore::default());
    let loop_checkpoint_store: Arc<dyn LoopCheckpointStore> =
        Arc::new(InMemoryLoopCheckpointStore::default());
    let thread_service: Arc<dyn SessionThreadService> =
        Arc::new(InMemorySessionThreadService::default());
    let skill_mounts =
        skill_context_mount_view().map_err(|error| RebornBuildError::InvalidConfig {
            reason: error.to_string(),
        })?;
    let local_runtime = Arc::new(RebornLocalRuntimeServices {
        approval_requests: Arc::clone(&approval_requests),
        capability_leases: Arc::clone(&capability_leases),
        turn_state: Arc::clone(&turn_state),
        checkpoint_state_store,
        loop_checkpoint_store,
        thread_service,
        skill_mounts,
        skill_filesystem,
        workspace_filesystem,
        workspace_mounts,
        event_log,
        audit_log,
    });
    let resource_governor: Arc<LocalDevResourceGovernor> =
        Arc::new(InMemoryResourceGovernor::new());
    let process_services = ProcessServices::in_memory();

    Ok(RebornLocalDevStoreGraph {
        run_state,
        approval_requests,
        capability_leases,
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
    host_home_root: Option<&LocalDevHostHomeRoot>,
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

    let local = Arc::new(local_dev_project_filesystem(
        root,
        workspace_root,
        host_home_root,
    )?);
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
    host_home_root: Option<&LocalDevHostHomeRoot>,
) -> Result<Arc<LocalDevRootFilesystem>, RebornBuildError> {
    Ok(Arc::new(local_dev_project_filesystem(
        root,
        workspace_root,
        host_home_root,
    )?))
}

fn local_dev_project_filesystem(
    root: &Path,
    workspace_root: &Path,
    host_home_root: Option<&LocalDevHostHomeRoot>,
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
    if let Some(host_home_root) = host_home_root {
        filesystem.mount_local(
            VirtualPath::new("/projects/host")?,
            HostPath::from_path_buf(host_home_root.canonical_root.clone()),
        )?;
    }
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

#[cfg(feature = "libsql")]
fn local_dev_audit_log(
    filesystem: Arc<LocalDevRootFilesystem>,
) -> Result<Arc<dyn DurableAuditLog>, RebornBuildError> {
    let scoped = Arc::new(ScopedFilesystem::with_fixed_view(
        filesystem,
        MountView::new(vec![MountGrant::new(
            MountAlias::new("/events")?,
            VirtualPath::new("/events")?,
            MountPermissions::read_write_list_delete(),
        )])?,
    ));
    Ok(Arc::new(
        ironclaw_reborn_event_store::FilesystemDurableAuditLog::new(scoped),
    ))
}

#[cfg(not(feature = "libsql"))]
fn local_dev_event_log(
    _filesystem: Arc<LocalDevRootFilesystem>,
) -> Result<Arc<dyn DurableEventLog>, RebornBuildError> {
    Ok(Arc::new(InMemoryDurableEventLog::new()))
}

#[cfg(not(feature = "libsql"))]
fn local_dev_audit_log(
    _filesystem: Arc<LocalDevRootFilesystem>,
) -> Result<Arc<dyn DurableAuditLog>, RebornBuildError> {
    Ok(Arc::new(InMemoryDurableAuditLog::new()))
}

fn canonicalize_local_dev_path(path: &Path, label: &str) -> Result<PathBuf, RebornBuildError> {
    std::fs::canonicalize(path).map_err(|_| RebornBuildError::InvalidConfig {
        reason: format!("local-dev {label} could not be resolved"),
    })
}

struct LocalDevHostHomeRoot {
    canonical_root: PathBuf,
    raw_alias: PathBuf,
}

impl LocalDevHostHomeRoot {
    fn aliases(&self) -> Vec<&Path> {
        vec![self.raw_alias.as_path(), self.canonical_root.as_path()]
    }
}

/// Build the two ScopedFilesystem views used by local-dev: a read-only workspace view
/// for skill context, and a read-write workspace view for runtime operations.
///
/// When `host_home_root` is present, the runtime view also grants raw host-home aliases
/// so `/Users/alice`-style paths resolve through the `/host` mount.
fn build_workspace_filesystems(
    filesystem: Arc<LocalDevRootFilesystem>,
    host_home_root: Option<&LocalDevHostHomeRoot>,
) -> Result<LocalDevWorkspaceFilesystems, RebornBuildError> {
    let read_only_workspace_mounts = workspace_mount_view(MountPermissions::read_only(), &[])
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: error.to_string(),
        })?;
    let host_home_aliases = host_home_root
        .map(|root| root.aliases())
        .unwrap_or_default();
    let runtime_workspace_mounts =
        workspace_mount_view(MountPermissions::read_write(), &host_home_aliases).map_err(
            |error| RebornBuildError::InvalidConfig {
                reason: error.to_string(),
            },
        )?;
    let skill_filesystem = Arc::new(ScopedFilesystem::with_fixed_view(
        Arc::clone(&filesystem),
        skill_context_mount_view().map_err(|error| RebornBuildError::InvalidConfig {
            reason: error.to_string(),
        })?,
    ));
    let workspace_filesystem = Arc::new(ScopedFilesystem::with_fixed_view(
        filesystem,
        read_only_workspace_mounts,
    ));
    Ok((
        skill_filesystem,
        workspace_filesystem,
        runtime_workspace_mounts,
    ))
}

fn canonicalize_local_dev_existing_dir(
    path: &Path,
    label: &str,
) -> Result<PathBuf, RebornBuildError> {
    let path = canonicalize_local_dev_path(path, label)?;
    let metadata = std::fs::metadata(&path).map_err(|_| RebornBuildError::InvalidConfig {
        reason: format!("local-dev {label} could not be inspected"),
    })?;
    if metadata.is_dir() {
        Ok(path)
    } else {
        Err(RebornBuildError::InvalidConfig {
            reason: format!("local-dev {label} must be an existing directory"),
        })
    }
}

fn canonicalize_local_dev_host_home_root(path: &Path) -> Result<PathBuf, RebornBuildError> {
    let path = canonicalize_local_dev_existing_dir(path, "host home root")?;
    if path.parent().is_none() {
        return Err(RebornBuildError::InvalidConfig {
            reason: "local-dev host home root must not be a filesystem root".to_string(),
        });
    }
    Ok(path)
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
        runtime_process_binding,
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
        runtime_process_binding,
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
                runtime_process_binding,
            )?;
            let secret_master_key = resolve_secret_master_key(secret_master_key).await?;
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
                runtime_process_binding,
            )?;
            let secret_master_key = resolve_secret_master_key(secret_master_key).await?;
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
async fn resolve_secret_master_key(
    explicit: Option<ironclaw_secrets::SecretMaterial>,
) -> Result<ironclaw_secrets::SecretMaterial, RebornBuildError> {
    if let Some(master_key) = explicit {
        Ok(master_key)
    } else if let Some(master_key) =
        ironclaw_secrets::keychain::resolve_master_key_material().await?
    {
        Ok(master_key)
    } else {
        Err(RebornBuildError::MissingSecretMasterKey)
    }
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
struct RebornProductionWiring {
    trust_policy: Arc<HostTrustPolicy>,
    runtime_policy: EffectiveRuntimePolicy,
    turn_run_wake_notifier: Arc<ironclaw_host_runtime::SchedulerTurnRunWakeNotifier>,
    runtime_process_binding: RebornRuntimeProcessBinding,
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
    runtime_process_binding: RebornRuntimeProcessBinding,
) -> Result<RebornProductionWiring, RebornBuildError> {
    let trust_policy = trust_policy.ok_or(RebornBuildError::MissingProductionTrustPolicy)?;
    if !trust_policy.has_sources() {
        return Err(RebornBuildError::EmptyProductionTrustPolicy);
    }
    let runtime_policy = runtime_policy.ok_or(RebornBuildError::MissingRuntimePolicy)?;
    validate_production_process_binding(&runtime_policy, &runtime_process_binding)?;
    let turn_run_wake_notifier =
        turn_run_wake_notifier.ok_or(RebornBuildError::MissingTurnRunWakeNotifier)?;
    Ok(RebornProductionWiring {
        trust_policy,
        runtime_policy,
        turn_run_wake_notifier,
        runtime_process_binding,
    })
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
fn validate_production_process_binding(
    runtime_policy: &EffectiveRuntimePolicy,
    binding: &RebornRuntimeProcessBinding,
) -> Result<(), RebornBuildError> {
    binding
        .validate_for_production_policy(runtime_policy)
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: error.to_string(),
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
    let services = apply_production_runtime_process_binding(
        services,
        production_wiring.runtime_process_binding,
    );

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
        ExecutionContext, ExtensionId, GrantConstraints, InvocationId, MountAlias, MountGrant,
        NetworkPolicy, Principal, ResourceEstimate, ResourceScope, RuntimeKind, ScopedPath,
        TrustClass, UserId, VirtualPath,
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
#[cfg(test)]
mod local_dev_host_tests;
