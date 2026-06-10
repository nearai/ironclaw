// arch-exempt: large_file, needs Reborn composition helper extraction, plan #4469
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::Arc,
};

#[cfg(any(feature = "libsql", feature = "postgres"))]
use crate::product_auth_durable::{FilesystemAuthProductServices, UnavailableAuthProviderClient};
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_auth::AuthProviderClient;
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_authorization::FilesystemCapabilityLeaseStore;
use ironclaw_authorization::GrantAuthorizer;
#[cfg(not(feature = "libsql"))]
use ironclaw_authorization::InMemoryCapabilityLeaseStore;
#[cfg(not(any(feature = "libsql", feature = "postgres")))]
use ironclaw_conversations::InMemoryConversationServices;
use ironclaw_conversations::{
    AdapterInstallationId, AdapterKind, ConversationActorPairingService, ExternalActorRef,
};
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_conversations::{InboundTurnError, RebornFilesystemConversationServices};
#[cfg(feature = "libsql")]
use ironclaw_events::{DurableAuditLog, DurableEventLog};
#[cfg(not(feature = "libsql"))]
use ironclaw_events::{
    DurableAuditLog, DurableEventLog, InMemoryDurableAuditLog, InMemoryDurableEventLog,
};
use ironclaw_extensions::{
    ExtensionInstallationStore, ExtensionLifecycleService, ExtensionRegistry,
};
#[cfg(not(feature = "libsql"))]
use ironclaw_filesystem::InMemoryBackend;
#[cfg(feature = "libsql")]
use ironclaw_filesystem::LibSqlRootFilesystem;
use ironclaw_filesystem::{
    BackendCapabilities, BackendId, BackendKind, CompositeRootFilesystem, ContentKind, IndexPolicy,
    MountDescriptor, RootFilesystem, StorageClass,
};
use ironclaw_filesystem::{LocalFilesystem, ScopedFilesystem};
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_host_api::runtime_policy::EffectiveRuntimePolicy;
use ironclaw_host_api::runtime_policy::{FilesystemBackendKind, ProcessBackendKind, SecretMode};
use ironclaw_host_api::{
    EffectKind, ExtensionId, HostPath, MountPermissions, MountView, PackageId, RuntimeHttpEgress,
    UserId, VirtualPath,
};
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_host_api::{HostApiError, MountAlias, MountGrant, ResourceScope};
use ironclaw_host_runtime::{
    CapabilitySurfaceVersion, FirstPartyCapabilityRegistry, HostRuntimeHttpEgressPort,
    HostRuntimeServices, LocalHostProcessPort, ProductAuthProviderRuntimePorts, TriggerCreateHook,
    builtin_first_party_handlers_with_trigger_create_hook, builtin_first_party_package,
};
#[cfg(feature = "libsql")]
use ironclaw_loop_support::FilesystemCheckpointStateStore;
use ironclaw_outbound::CommunicationPreferenceRepository;
#[cfg(feature = "libsql")]
use ironclaw_outbound::FilesystemOutboundStateStore;
#[cfg(not(feature = "libsql"))]
use ironclaw_outbound::InMemoryOutboundStateStore;
use ironclaw_processes::ProcessServices;
use ironclaw_product_workflow::ProductAuthTurnGateResumeDispatcher;
use ironclaw_resources::InMemoryResourceGovernor;
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_resources::{FilesystemResourceGovernorStore, PersistentResourceGovernor};
#[cfg(feature = "libsql")]
use ironclaw_run_state::{FilesystemApprovalRequestStore, FilesystemRunStateStore};
#[cfg(not(feature = "libsql"))]
use ironclaw_run_state::{InMemoryApprovalRequestStore, InMemoryRunStateStore};
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_secrets::FilesystemCredentialBroker;
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_secrets::FilesystemSecretStore;
use ironclaw_secrets::SecretStore;
#[cfg(feature = "libsql")]
use ironclaw_threads::FilesystemSessionThreadService;
#[cfg(not(feature = "libsql"))]
use ironclaw_threads::InMemorySessionThreadService;
use ironclaw_threads::SessionThreadService;
use ironclaw_triggers::{
    TRIGGER_TRUSTED_ADAPTER_INSTALLATION_ID, TRIGGER_TRUSTED_ADAPTER_KIND,
    TRIGGER_TRUSTED_EXTERNAL_ACTOR_NAMESPACE, TriggerError, TriggerRecord, TriggerRepository,
};
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

use crate::RebornProductAuthServicePorts;
use crate::default_system_prompt::seed_default_system_prompt;
use crate::input::{RebornRuntimeProcessBinding, RebornStorageInput};
use crate::lifecycle::{RebornLocalSkillManagementPort, build_local_skill_management_port};
use crate::local_dev_capability_policy::local_dev_capability_policy;
use crate::local_dev_mounts::{
    ambient_workspace_mount_view, memory_mount_view, scoped_skill_context_mount_view,
    skill_management_mount_view, workspace_mount_view,
};
use crate::mcp::hosted_http_mcp_runtime;
use crate::product_auth_providers::{OAuthProviderComposition, compose_provider_client};
use crate::product_auth_runtime_credentials::ProductAuthRuntimeCredentialResolver;
use crate::{
    RebornAuthContinuationDispatcher, RebornBuildError, RebornBuildInput, RebornCompositionProfile,
    RebornFacadeReadiness, RebornProductAuthServices, RebornReadiness, RebornReadinessState,
    RebornWorkerReadiness,
};
use crate::{
    available_extensions::{
        AvailableExtensionCatalog, gmail_manifest_digest, google_calendar_manifest_digest,
        google_docs_manifest_digest, google_drive_manifest_digest, google_sheets_manifest_digest,
        google_slides_manifest_digest, notion_mcp_manifest_digest, web_access_manifest_digest,
    },
    extension_installation_store::FilesystemExtensionInstallationStore,
    extension_lifecycle::{
        ActiveExtensionPublisher, RebornLocalExtensionManagementPort,
        restore_extension_lifecycle_state,
    },
    extension_lifecycle_capabilities::{
        extend_builtin_first_party_package, insert_handlers as insert_extension_lifecycle_handlers,
    },
    gsuite::{
        ProductAuthRuntimeGsuiteCredentialStager, register_bundled_gsuite_first_party_handlers,
    },
    web_access::register_bundled_web_access_first_party_handlers,
};

pub(crate) type LocalDevRootFilesystem = CompositeRootFilesystem;

/// Output of [`build_local_dev_root_filesystem`]: the composed local-dev
/// root filesystem and, when libSQL is the substrate, a clone of the raw
/// libSQL handle. The handle backs both the local-dev trigger repository
/// and the canonical Reborn identity store, so each rides the same
/// `reborn-local-dev.db` rather than opening a second handle to the file
/// (see `RebornRuntime::open_reborn_identity_resolver`).
struct LocalDevRootFilesystemBundle {
    filesystem: Arc<LocalDevRootFilesystem>,
    #[cfg(feature = "libsql")]
    database: Arc<libsql::Database>,
}

type LocalDevWorkspaceFilesystems = (
    Arc<ScopedFilesystem<LocalDevRootFilesystem>>,
    Arc<ScopedFilesystem<LocalDevRootFilesystem>>,
    MountView,
);

const LOCAL_DEV_DEFAULT_SYSTEM_PROMPT_PATH: &str = "system/prompts/default-system.md";
const LOCAL_DEV_LEGACY_SKILLS_BACKFILL_MARKER: &str = ".legacy-skills-backfilled";
const LOCAL_DEV_LEGACY_SKILLS_BACKFILL_MAX_DEPTH: usize = 64;
#[cfg(any(feature = "libsql", feature = "postgres"))]
const LOCAL_DEV_SECRETS_MASTER_KEY_PATH: &str = ".reborn-local-dev-secrets-master-key";

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

fn local_dev_process_port_for_policy(
    runtime_policy: &Option<ironclaw_host_api::runtime_policy::EffectiveRuntimePolicy>,
    workspace_root: &Path,
    host_home_root: Option<&LocalDevHostHomeRoot>,
) -> Option<LocalHostProcessPort> {
    let runtime_policy = runtime_policy.as_ref()?;
    if runtime_policy.process_backend != ProcessBackendKind::LocalHost {
        return None;
    }
    let mut process_port = if runtime_policy.secret_mode == SecretMode::InheritedEnv {
        LocalHostProcessPort::new_inherited_env()
    } else {
        LocalHostProcessPort::new()
    }
    .with_workdir_alias("/workspace", workspace_root);
    if let Some(host_home_root) = host_home_root {
        process_port =
            process_port.with_workdir_alias("/host", host_home_root.canonical_root.clone());
        for alias in host_home_root.aliases() {
            let alias_str = match alias.to_str() {
                Some(s) => s,
                None => {
                    tracing::debug!(alias = ?alias, "skipping non-UTF-8 host home alias");
                    continue;
                }
            };
            process_port = process_port.with_workdir_alias(alias_str, alias.to_path_buf());
        }
    }
    Some(process_port)
}

fn require_product_auth_runtime_ports<F, G, S, R>(
    services: &HostRuntimeServices<F, G, S, R>,
) -> Result<ProductAuthProviderRuntimePorts, RebornBuildError>
where
    F: ironclaw_filesystem::RootFilesystem + 'static,
    G: ironclaw_resources::ResourceGovernor + 'static,
    S: ironclaw_processes::ProcessStore + 'static,
    R: ironclaw_processes::ProcessResultStore + 'static,
{
    services
        .product_auth_provider_runtime_ports()
        .ok_or_else(|| RebornBuildError::InvalidConfig {
            reason: "product auth runtime ports unavailable; host runtime must be configured with HTTP egress and a secret store".to_string(),
        })
}

fn attach_hosted_mcp_runtime<F, G, S, R>(
    services: HostRuntimeServices<F, G, S, R>,
) -> Result<HostRuntimeServices<F, G, S, R>, RebornBuildError>
where
    F: ironclaw_filesystem::RootFilesystem + 'static,
    G: ironclaw_resources::ResourceGovernor + 'static,
    S: ironclaw_processes::ProcessStore + 'static,
    R: ironclaw_processes::ProcessResultStore + 'static,
{
    // Soft-disable when host runtime HTTP egress is absent. Builds without
    // egress — in-memory test services, minimal compositions — must still
    // succeed; only hosted MCP capabilities go dark.
    let Some(runtime_ports) = services.product_auth_provider_runtime_ports() else {
        tracing::debug!(
            "skipping hosted MCP runtime: host runtime HTTP egress absent \
             (only affects hosted MCP extensions, e.g. Notion, NEAR AI)"
        );
        return Ok(services);
    };
    let runtime_http_egress = runtime_ports.runtime_http_egress();
    let registry = services.shared_extension_registry();

    Ok(services.with_mcp_runtime(Arc::new(hosted_http_mcp_runtime(
        registry,
        runtime_http_egress,
    ))))
}

fn attach_wasm_runtime<F, G, S, R>(
    services: HostRuntimeServices<F, G, S, R>,
) -> Result<HostRuntimeServices<F, G, S, R>, RebornBuildError>
where
    F: ironclaw_filesystem::RootFilesystem + 'static,
    G: ironclaw_resources::ResourceGovernor + 'static,
    S: ironclaw_processes::ProcessStore + 'static,
    R: ironclaw_processes::ProcessResultStore + 'static,
{
    services
        .try_with_default_wasm_runtime()
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("WASM runtime could not be initialized: {error}"),
        })
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
    pub(crate) skill_management: Option<Arc<RebornLocalSkillManagementPort>>,
    pub(crate) local_runtime: Option<Arc<RebornLocalRuntimeServices>>,
    /// Shared scoped secret store. Exposed so runtime-level features (e.g.
    /// operator LLM-key storage) can reuse the same instance product-auth uses
    /// rather than standing up a second authority.
    #[cfg(feature = "root-llm-provider")]
    pub(crate) secret_store: Arc<dyn SecretStore>,
}

impl RebornServices {
    /// The shared scoped secret store backing this composition.
    #[cfg(feature = "root-llm-provider")]
    pub(crate) fn secret_store(&self) -> Arc<dyn SecretStore> {
        Arc::clone(&self.secret_store)
    }
}

pub(crate) struct RebornLocalRuntimeServices {
    pub(crate) approval_requests: Arc<LocalDevApprovalRequestStore>,
    pub(crate) capability_leases: Arc<LocalDevCapabilityLeaseStore>,
    pub(crate) turn_state: Arc<LocalDevTurnStateStore>,
    pub(crate) trigger_repository: Arc<dyn TriggerRepository>,
    pub(crate) outbound_preferences: Arc<dyn CommunicationPreferenceRepository>,
    #[cfg(not(any(feature = "libsql", feature = "postgres")))]
    pub(crate) trigger_conversation_services: InMemoryConversationServices,
    #[cfg(any(feature = "libsql", feature = "postgres"))]
    pub(crate) trigger_conversation_services:
        tokio::sync::OnceCell<RebornFilesystemConversationServices>,
    pub(crate) checkpoint_state_store: Arc<dyn CheckpointStateStore>,
    pub(crate) loop_checkpoint_store: Arc<dyn LoopCheckpointStore>,
    pub(crate) thread_service: Arc<dyn SessionThreadService>,
    /// Scoped filesystem backing the canonical Reborn identity store, so it
    /// rides the host `RootFilesystem` abstraction like every other durable
    /// Reborn store rather than a raw DB handle. Only the WebUI v2 SSO surface
    /// reads it today, hence `dead_code` when that feature is off.
    #[cfg(feature = "libsql")]
    #[allow(dead_code)]
    pub(crate) identity_filesystem: Arc<ScopedFilesystem<LocalDevRootFilesystem>>,
    /// Raw libSQL substrate handle backing `reborn-local-dev.db`. Carried ONLY
    /// for the one-time legacy WebUI `user_identities` fold (a substrate-level
    /// read that belongs in this host layer, not the identity crate); the
    /// steady-state identity store goes through `identity_filesystem` above.
    #[cfg(feature = "libsql")]
    #[allow(dead_code)]
    pub(crate) identity_substrate_db: Arc<libsql::Database>,
    /// Resource governor handle used by the budget accountant. Kept here
    /// separately from the type-erased `dyn HostRuntime` so the runtime
    /// composer can construct a `GovernorBackedAccountant` without losing
    /// the concrete governor type. Wired through #3841 follow-up "A1: wire
    /// GovernorBackedAccountant into production composition".
    pub(crate) resource_governor: Arc<dyn ironclaw_resources::ResourceGovernor>,
    /// Sink that receives `BudgetEvent`s from the governor. Composition
    /// hands this to downstream consumers (audit log, SSE projection)
    /// without forcing the governor to know about them. Wired through
    /// #3841 follow-up "A2: project BudgetEvent into the gateway event
    /// stream".
    #[allow(dead_code)]
    pub(crate) budget_event_sink: Arc<dyn ironclaw_resources::BudgetEventSink>,
    /// Same sink as `budget_event_sink` but typed as the concrete
    /// `InMemoryBudgetEventSink` so the runtime can expose `drain()` /
    /// `snapshot()` to tests without leaking the concrete type into the
    /// production `BudgetEventSink` boundary.
    #[allow(dead_code)]
    pub(crate) in_memory_budget_event_sink: Arc<ironclaw_resources::InMemoryBudgetEventSink>,
    /// Broadcast sink production callers can subscribe against once a
    /// real projection caller lands (review feedback Thermo-Nuclear
    /// #3: the speculative `src/bridge/budget_events.rs` helper plus
    /// `AppEvent::Budget` variant were removed pending an owner that
    /// actually spawns a projection task with shutdown cancellation).
    /// Composition fans every BudgetEvent through this alongside the
    /// in-memory sink so tests can still inspect history.
    pub(crate) broadcast_budget_event_sink: Arc<ironclaw_resources::BroadcastBudgetEventSink>,
    /// Approval-gate store used to surface `BudgetApprovalRequired` to a
    /// user. Stays in-memory in local-dev; production composition will
    /// swap in the filesystem-backed `FilesystemBudgetGateStore`.
    #[allow(dead_code)]
    pub(crate) budget_gate_store: Arc<dyn ironclaw_resources::BudgetGateStore>,
    pub(crate) skill_management: Arc<RebornLocalSkillManagementPort>,
    // LocalSingleUser-only for now. Production and multi-tenant lifecycle
    // wiring need scoped storage/registry ownership before this is reused
    // outside local-dev composition. Tracked in #4091.
    pub(crate) extension_management: Option<Arc<RebornLocalExtensionManagementPort>>,
    pub(crate) runtime_http_egress: Option<Arc<dyn RuntimeHttpEgress>>,
    pub(crate) host_runtime_http_egress: Option<HostRuntimeHttpEgressPort>,
    pub(crate) skill_mounts: MountView,
    pub(crate) memory_mounts: MountView,
    pub(crate) skill_filesystem: Arc<ScopedFilesystem<LocalDevRootFilesystem>>,
    pub(crate) workspace_filesystem: Arc<ScopedFilesystem<LocalDevRootFilesystem>>,
    #[cfg(all(
        any(feature = "libsql", feature = "postgres"),
        feature = "slack-v2-host-beta"
    ))]
    pub(crate) host_state_filesystem: Arc<ScopedFilesystem<LocalDevRootFilesystem>>,
    #[cfg(any(feature = "libsql", feature = "postgres"))]
    pub(crate) subagent_goal_filesystem: Arc<ScopedFilesystem<LocalDevRootFilesystem>>,
    /// Tenant-scoped root filesystem used for third-party extension hook
    /// discovery (`/system/extensions/<tenant>`). The runtime derives the
    /// discovery root from the authenticated tenant id; this is the same
    /// backend the rest of local-dev composition uses.
    pub(crate) extension_filesystem: Arc<LocalDevRootFilesystem>,
    pub(crate) workspace_mounts: MountView,
    pub(crate) local_dev_storage_root: PathBuf,
    pub(crate) default_system_prompt_path: PathBuf,
    pub(crate) event_log: Arc<dyn DurableEventLog>,
    pub(crate) audit_log: Arc<dyn DurableAuditLog>,
    /// Canonical registry shared by capability dispatch and hook activation.
    pub(crate) extension_registry: Arc<ExtensionRegistry>,
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
impl RebornLocalRuntimeServices {
    pub(crate) async fn durable_trigger_conversation_services(
        &self,
    ) -> Result<RebornFilesystemConversationServices, InboundTurnError> {
        let filesystem = Arc::clone(&self.subagent_goal_filesystem);
        self.trigger_conversation_services
            .get_or_try_init(|| async move {
                RebornFilesystemConversationServices::new(filesystem).await
            })
            .await
            .cloned()
    }
}

struct RebornLocalDevStoreGraph {
    run_state: Arc<LocalDevRunStateStore>,
    approval_requests: Arc<LocalDevApprovalRequestStore>,
    capability_leases: Arc<LocalDevCapabilityLeaseStore>,
    turn_state: Arc<LocalDevTurnStateStore>,
    local_runtime: Arc<RebornLocalRuntimeServices>,
    resource_governor: Arc<LocalDevResourceGovernor>,
    process_services: LocalDevProcessServices,
    trigger_repository: Arc<dyn TriggerRepository>,
}

struct RebornLocalDevStoreGraphInput {
    filesystem: Arc<LocalDevRootFilesystem>,
    owner_user_id: UserId,
    skill_filesystem: Arc<ScopedFilesystem<LocalDevRootFilesystem>>,
    workspace_filesystem: Arc<ScopedFilesystem<LocalDevRootFilesystem>>,
    workspace_mounts: MountView,
    local_dev_storage_root: PathBuf,
    default_system_prompt_path: PathBuf,
    trigger_repository: Arc<dyn TriggerRepository>,
    /// Raw libSQL substrate handle, carried so the canonical Reborn identity
    /// store rides the same `reborn-local-dev.db` instead of opening a second
    /// handle (see `RebornRuntime::open_reborn_identity_resolver`).
    #[cfg(feature = "libsql")]
    identity_substrate_db: Arc<libsql::Database>,
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
            skill_management: None,
            local_runtime: None,
            #[cfg(feature = "root-llm-provider")]
            secret_store: Arc::new(ironclaw_secrets::InMemorySecretStore::new()),
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
    provider_composition: OAuthProviderComposition,
) -> Arc<RebornProductAuthServices> {
    let ports = match provider_composition.client {
        Some(provider_client) => ports.with_provider_client(provider_client),
        None => ports,
    };
    let mut services = ports.into_services(auth_continuation_dispatcher(turn_coordinator));
    if let Some(registry) = provider_composition.dcr_registry {
        services = services.with_dcr_oauth_registry(registry);
    }
    if let Some(registry) = provider_composition.gate_registry {
        services = services.with_oauth_gate_registry(registry);
    }
    Arc::new(services)
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
    config.require_credential_broker()
}

async fn build_local_dev(input: RebornBuildInput) -> Result<RebornServices, RebornBuildError> {
    #[cfg(all(test, feature = "slack-v2-host-beta"))]
    let host_runtime_http_egress_for_test = input.host_runtime_http_egress_for_test.clone();
    let RebornBuildInput {
        profile,
        storage,
        runtime_policy,
        runtime_process_binding,
        product_auth_ports,
        oauth_provider_configs,
        oauth_dcr_provider_configs,
        owner_id,
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
    std::fs::create_dir_all(root.join("system/extensions")).map_err(|_| {
        RebornBuildError::InvalidConfig {
            reason: "local-dev system extensions root could not be initialized".to_string(),
        }
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
    let owner_user_id = UserId::new(owner_id).map_err(|error| RebornBuildError::InvalidConfig {
        reason: error.to_string(),
    })?;
    let backfill_root = root.clone();
    let backfill_owner_user_id = owner_user_id.clone();
    tokio::task::spawn_blocking(move || {
        backfill_local_dev_legacy_user_skills(&backfill_root, &backfill_owner_user_id)
    })
    .await
    .map_err(|error| RebornBuildError::InvalidConfig {
        reason: format!("local-dev legacy skill backfill task failed: {error}"),
    })??;
    let default_system_prompt_path = local_dev_default_system_prompt_path(&root);
    seed_default_system_prompt(&root, &default_system_prompt_path).map_err(|error| {
        RebornBuildError::InvalidConfig {
            reason: error.to_string(),
        }
    })?;
    crate::bundled_skills::ensure_bundled_reborn_skills_installed(&root).await?;
    let filesystem_bundle =
        build_local_dev_root_filesystem(&root, &workspace_root, host_home_root.as_ref()).await?;
    // Clone the raw libSQL handle for the canonical identity store before
    // `filesystem` moves out of the bundle, so the resolver rides the same
    // substrate DB the runtime owns rather than a second handle.
    #[cfg(feature = "libsql")]
    let identity_substrate_db = Arc::clone(&filesystem_bundle.database);
    let filesystem = filesystem_bundle.filesystem;
    #[cfg(feature = "libsql")]
    let trigger_repository =
        local_dev_trigger_repository(Arc::clone(&filesystem_bundle.database)).await?;
    #[cfg(not(feature = "libsql"))]
    let trigger_repository = local_dev_trigger_repository();
    let (skill_filesystem, workspace_filesystem, runtime_workspace_mounts) =
        build_workspace_filesystems(
            Arc::clone(&filesystem),
            &workspace_root,
            host_home_root.as_ref(),
        )?;
    let http_body_filesystem = Arc::new(ScopedFilesystem::with_fixed_view(
        Arc::clone(&filesystem),
        runtime_workspace_mounts.clone(),
    ));
    let mut store_graph = build_local_dev_store_graph(RebornLocalDevStoreGraphInput {
        filesystem: Arc::clone(&filesystem),
        owner_user_id,
        skill_filesystem,
        workspace_filesystem,
        workspace_mounts: runtime_workspace_mounts,
        local_dev_storage_root: root.clone(),
        default_system_prompt_path,
        trigger_repository,
        #[cfg(feature = "libsql")]
        identity_substrate_db,
    })?;

    let turn_coordinator: Arc<dyn ironclaw_turns::TurnCoordinator> = Arc::new(
        DefaultTurnCoordinator::new(Arc::clone(&store_graph.turn_state)),
    );
    #[cfg(any(feature = "libsql", feature = "postgres"))]
    let local_dev_product_auth_filesystem = local_dev_scoped_filesystem(Arc::clone(&filesystem));
    #[cfg(any(feature = "libsql", feature = "postgres"))]
    let local_dev_secret_store =
        build_local_dev_secret_store(&root, Arc::clone(&local_dev_product_auth_filesystem))?;
    #[cfg(any(feature = "libsql", feature = "postgres"))]
    let secret_store: Arc<dyn SecretStore> = local_dev_secret_store.clone();
    #[cfg(not(any(feature = "libsql", feature = "postgres")))]
    let secret_store: Arc<dyn SecretStore> = Arc::new(ironclaw_secrets::InMemorySecretStore::new());
    let local_dev_trust_policy = Arc::new(local_dev_first_party_trust_policy()?);
    let local_dev_trust_invalidation_bus = Arc::new(ironclaw_trust::InvalidationBus::new());
    let extension_registry = Arc::new(local_dev_builtin_extension_registry()?);
    let mut services = HostRuntimeServices::new(
        Arc::clone(&extension_registry),
        Arc::clone(&filesystem),
        Arc::clone(&store_graph.resource_governor),
        Arc::new(GrantAuthorizer::new()),
        store_graph.process_services.clone(),
        CapabilitySurfaceVersion::new("reborn-app-v1")?,
    )
    .with_trust_policy(Arc::clone(&local_dev_trust_policy))
    .with_secret_store_dyn(Arc::clone(&secret_store))
    .try_with_host_http_egress_with_body_store(
        ironclaw_network::PolicyNetworkHttpEgress::new(
            ironclaw_network::ReqwestNetworkTransport::default(),
        ),
        http_body_filesystem,
    )?
    .with_run_state(Arc::clone(&store_graph.run_state))
    .with_approval_requests(Arc::clone(&store_graph.approval_requests))
    .with_capability_leases(Arc::clone(&store_graph.capability_leases))
    .with_turn_state_and_transition_port(Arc::clone(&store_graph.turn_state));
    let local_dev_process_port = local_dev_process_port_for_policy(
        &runtime_policy,
        &workspace_root,
        host_home_root.as_ref(),
    );
    if let Some(runtime_policy) = runtime_policy {
        services = services.with_runtime_policy(runtime_policy);
    }
    if let Some(process_port) = local_dev_process_port {
        services = services.with_runtime_process_port(Arc::new(process_port));
    }
    services = apply_runtime_process_binding(services, runtime_process_binding);
    services = attach_hosted_mcp_runtime(services)?;
    services = attach_wasm_runtime(services)?;
    let product_auth_runtime_ports = require_product_auth_runtime_ports(&services)?;
    let provider_composition = compose_provider_client(
        oauth_provider_configs,
        oauth_dcr_provider_configs,
        Arc::clone(&secret_store),
        product_auth_runtime_ports.clone(),
    )?;
    let product_auth = match product_auth_ports {
        Some(ports) => {
            compose_product_auth_services(ports, turn_coordinator.clone(), provider_composition)
        }
        None => {
            #[cfg(any(feature = "libsql", feature = "postgres"))]
            {
                let durable_services = Arc::new(FilesystemAuthProductServices::new(
                    local_dev_product_auth_filesystem,
                    Arc::clone(&secret_store),
                ));
                let provider_client: Arc<dyn AuthProviderClient> = provider_composition
                    .client
                    .clone()
                    .unwrap_or_else(|| Arc::new(UnavailableAuthProviderClient));
                let services = RebornProductAuthServicePorts::from_shared_with_provider(
                    Arc::clone(&durable_services),
                    provider_client,
                )
                .into_services(auth_continuation_dispatcher(turn_coordinator.clone()))
                .with_flow_record_source(durable_services);
                let services = match provider_composition.dcr_registry.clone() {
                    Some(registry) => services.with_dcr_oauth_registry(registry),
                    None => services,
                };
                let services = match provider_composition.gate_registry.clone() {
                    Some(registry) => services.with_oauth_gate_registry(registry),
                    None => services,
                };
                Arc::new(services)
            }
            #[cfg(not(any(feature = "libsql", feature = "postgres")))]
            {
                let services = RebornProductAuthServices::local_dev_in_memory(
                    auth_continuation_dispatcher(turn_coordinator.clone()),
                );
                let services = match provider_composition.client.clone() {
                    Some(provider_client) => services.with_provider_client(provider_client),
                    None => services,
                };
                let services = match provider_composition.dcr_registry.clone() {
                    Some(registry) => services.with_dcr_oauth_registry(registry),
                    None => services,
                };
                Arc::new(match provider_composition.gate_registry.clone() {
                    Some(registry) => services.with_oauth_gate_registry(registry),
                    None => services,
                })
            }
        }
    };
    services = services.with_runtime_credential_account_resolver(Arc::new(
        ProductAuthRuntimeCredentialResolver::new(
            product_auth.runtime_credential_account_selection_service(),
        ),
    ));
    let mut available_extensions = AvailableExtensionCatalog::from_filesystem_root(
        filesystem.as_ref(),
        &VirtualPath::new("/system/extensions")?,
    )
    .await
    .map_err(|error| RebornBuildError::InvalidConfig {
        reason: format!("available extension catalog could not be loaded: {error}"),
    })?;
    available_extensions.extend(
        AvailableExtensionCatalog::from_first_party_assets().map_err(|error| {
            RebornBuildError::InvalidConfig {
                reason: format!("first-party extension catalog could not be loaded: {error}"),
            }
        })?,
    );
    let extension_filesystem: Arc<dyn RootFilesystem> = filesystem.clone();
    let extension_installation_store: Arc<dyn ExtensionInstallationStore> = Arc::new(
        FilesystemExtensionInstallationStore::load(extension_filesystem.clone())
            .await
            .map_err(|error| RebornBuildError::InvalidConfig {
                reason: format!("extension installation state could not be loaded: {error}"),
            })?,
    );
    let extension_lifecycle_service = Arc::new(tokio::sync::Mutex::new(
        ExtensionLifecycleService::new(services.shared_extension_registry().snapshot_owned()),
    ));
    let active_registry = services.shared_extension_registry();
    let active_extensions = ActiveExtensionPublisher::new(
        active_registry,
        local_dev_trust_policy,
        local_dev_trust_invalidation_bus,
    );
    restore_extension_lifecycle_state(
        &available_extensions,
        &extension_filesystem,
        &extension_installation_store,
        &extension_lifecycle_service,
        &active_extensions,
    )
    .await
    .map_err(|error| RebornBuildError::InvalidConfig {
        reason: format!("extension lifecycle state could not be restored: {error}"),
    })?;
    let extension_management = Arc::new(RebornLocalExtensionManagementPort::new(
        extension_filesystem,
        available_extensions,
        extension_installation_store,
        extension_lifecycle_service,
        active_extensions,
    ));
    if let Some(local_runtime) = Arc::get_mut(&mut store_graph.local_runtime) {
        local_runtime.extension_management = Some(Arc::clone(&extension_management));
        local_runtime.runtime_http_egress = Some(product_auth_runtime_ports.runtime_http_egress());
        local_runtime.extension_registry = Arc::clone(&extension_registry);
        let host_runtime_http_egress = services.host_runtime_http_egress_port();
        #[cfg(all(test, feature = "slack-v2-host-beta"))]
        let host_runtime_http_egress =
            host_runtime_http_egress_for_test.unwrap_or(host_runtime_http_egress);
        local_runtime.host_runtime_http_egress = host_runtime_http_egress;
    } else {
        return Err(RebornBuildError::InvalidConfig {
            reason: "local-dev extension lifecycle facade could not be attached".to_string(),
        });
    }
    let trigger_create_hook = local_dev_trigger_create_hook(&store_graph.local_runtime);
    let mut first_party_registry = builtin_first_party_registry_with_trigger_create_hook(
        Arc::clone(&store_graph.trigger_repository),
        trigger_create_hook,
    )?;
    register_bundled_gsuite_first_party_handlers(
        &mut first_party_registry,
        product_auth.credential_account_service(),
        product_auth.credential_account_record_source(),
        Arc::new(ProductAuthRuntimeGsuiteCredentialStager::new(
            product_auth_runtime_ports.clone(),
        )),
    )
    .map_err(|error| RebornBuildError::InvalidConfig {
        reason: format!("GSuite first-party handlers are invalid: {error}"),
    })?;
    register_bundled_web_access_first_party_handlers(&mut first_party_registry).map_err(
        |error| RebornBuildError::InvalidConfig {
            reason: format!("web access first-party handlers are invalid: {error}"),
        },
    )?;
    insert_extension_lifecycle_handlers(&mut first_party_registry, extension_management).map_err(
        |error| RebornBuildError::InvalidConfig {
            reason: format!("local-dev extension lifecycle handlers are invalid: {error}"),
        },
    )?;
    services = services.with_first_party_capabilities(Arc::new(first_party_registry));

    let host_runtime: Arc<dyn ironclaw_host_runtime::HostRuntime> =
        Arc::new(services.host_runtime_for_local_testing());

    Ok(RebornServices {
        host_runtime: Some(host_runtime),
        turn_coordinator: Some(turn_coordinator),
        // Local-dev always composes a safe in-memory product-auth boundary when
        // the caller does not inject one; readiness tracks the assembled facade.
        product_auth: Some(product_auth),
        readiness: readiness_for(profile, true, true, true),
        skill_management: Some(Arc::clone(&store_graph.local_runtime.skill_management)),
        local_runtime: Some(store_graph.local_runtime),
        #[cfg(feature = "root-llm-provider")]
        secret_store,
    })
}

fn backfill_local_dev_legacy_user_skills(
    storage_root: &Path,
    owner_user_id: &UserId,
) -> Result<(), RebornBuildError> {
    let legacy_root = storage_root.join("skills");
    if !legacy_root.is_dir() {
        return Ok(());
    }

    for tenant_id in ["default", "reborn-cli"] {
        backfill_local_dev_legacy_user_skills_for_tenant(
            &legacy_root,
            storage_root,
            tenant_id,
            owner_user_id,
        )?;
    }
    Ok(())
}

fn backfill_local_dev_legacy_user_skills_for_tenant(
    legacy_root: &Path,
    storage_root: &Path,
    tenant_id: &str,
    owner_user_id: &UserId,
) -> Result<(), RebornBuildError> {
    let scoped_root = storage_root
        .join("tenants")
        .join(tenant_id)
        .join("users")
        .join(owner_user_id.as_str())
        .join("skills");
    let marker = scoped_root.join(LOCAL_DEV_LEGACY_SKILLS_BACKFILL_MARKER);
    if marker.exists() {
        return Ok(());
    }

    std::fs::create_dir_all(&scoped_root).map_err(|error| RebornBuildError::InvalidConfig {
        reason: format!("local-dev scoped skill root could not be initialized: {error}"),
    })?;

    for entry in
        std::fs::read_dir(legacy_root).map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!(
                "local-dev legacy skills root '{}' could not be inspected: {error}",
                legacy_root.display()
            ),
        })?
    {
        let entry = entry.map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!(
                "local-dev legacy skills root '{}' could not be inspected: {error}",
                legacy_root.display()
            ),
        })?;
        let source = entry.path();
        let destination = scoped_root.join(entry.file_name());
        if destination.exists() {
            continue;
        }
        copy_local_dev_legacy_skill_entry(&source, &destination)?;
    }
    std::fs::write(&marker, b"").map_err(|error| RebornBuildError::InvalidConfig {
        reason: format!(
            "local-dev legacy skill migration marker '{}' could not be written: {error}",
            marker.display()
        ),
    })?;
    Ok(())
}

fn copy_local_dev_legacy_skill_entry(
    source: &Path,
    destination: &Path,
) -> Result<(), RebornBuildError> {
    let mut pending = VecDeque::from([(source.to_path_buf(), destination.to_path_buf(), 0usize)]);

    while let Some((source, destination, depth)) = pending.pop_front() {
        if depth > LOCAL_DEV_LEGACY_SKILLS_BACKFILL_MAX_DEPTH {
            return Err(RebornBuildError::InvalidConfig {
                reason: format!(
                    "local-dev legacy skill entry '{}' exceeds max copy depth {}",
                    source.display(),
                    LOCAL_DEV_LEGACY_SKILLS_BACKFILL_MAX_DEPTH
                ),
            });
        }

        let metadata = std::fs::symlink_metadata(&source).map_err(|error| {
            RebornBuildError::InvalidConfig {
                reason: format!(
                    "local-dev legacy skill entry '{}' could not be inspected: {error}",
                    source.display()
                ),
            }
        })?;
        if metadata.file_type().is_symlink() {
            tracing::warn!(
                path = %source.display(),
                "Skipping symlinked local-dev legacy skill entry during backfill"
            );
            continue;
        }
        if metadata.is_dir() {
            std::fs::create_dir_all(&destination).map_err(|error| {
                RebornBuildError::InvalidConfig {
                    reason: format!(
                        "local-dev scoped skill directory '{}' could not be initialized: {error}",
                        destination.display()
                    ),
                }
            })?;
            for entry in
                std::fs::read_dir(&source).map_err(|error| RebornBuildError::InvalidConfig {
                    reason: format!(
                        "local-dev legacy skill directory '{}' could not be inspected: {error}",
                        source.display()
                    ),
                })?
            {
                let entry = entry.map_err(|error| RebornBuildError::InvalidConfig {
                    reason: format!(
                        "local-dev legacy skill directory '{}' could not be inspected: {error}",
                        source.display()
                    ),
                })?;
                pending.push_back((
                    entry.path(),
                    destination.join(entry.file_name()),
                    depth.saturating_add(1),
                ));
            }
            continue;
        }

        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent).map_err(|error| RebornBuildError::InvalidConfig {
                reason: format!(
                    "local-dev scoped skill directory '{}' could not be initialized: {error}",
                    parent.display()
                ),
            })?;
        }
        std::fs::copy(&source, &destination).map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!(
                "local-dev legacy skill file '{}' could not be migrated to '{}': {error}",
                source.display(),
                destination.display()
            ),
        })?;
    }
    Ok(())
}

#[cfg(feature = "libsql")]
fn build_local_dev_store_graph(
    input: RebornLocalDevStoreGraphInput,
) -> Result<RebornLocalDevStoreGraph, RebornBuildError> {
    let RebornLocalDevStoreGraphInput {
        filesystem,
        owner_user_id,
        skill_filesystem,
        workspace_filesystem,
        workspace_mounts,
        local_dev_storage_root,
        default_system_prompt_path,
        trigger_repository,
        identity_substrate_db,
    } = input;
    let scoped_filesystem = local_dev_scoped_filesystem(Arc::clone(&filesystem));
    let event_log = local_dev_event_log(Arc::clone(&filesystem))?;
    let audit_log = local_dev_audit_log(Arc::clone(&filesystem))?;
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
    let BudgetSinks {
        budget_event_sink,
        in_memory_budget_event_sink,
        broadcast_budget_event_sink,
        budget_gate_store,
    } = build_budget_sinks();
    let resource_governor: Arc<LocalDevResourceGovernor> = Arc::new(
        PersistentResourceGovernor::new(FilesystemResourceGovernorStore::new(Arc::clone(
            &scoped_filesystem,
        )))
        .with_event_sink(Arc::clone(&budget_event_sink)),
    );
    let skill_mounts =
        skill_management_mount_view().map_err(|error| RebornBuildError::InvalidConfig {
            reason: error.to_string(),
        })?;
    let memory_mounts =
        memory_mount_view(MountPermissions::read_write_list_delete()).map_err(|error| {
            RebornBuildError::InvalidConfig {
                reason: error.to_string(),
            }
        })?;
    #[cfg(feature = "slack-v2-host-beta")]
    let host_state_filesystem = local_dev_slack_host_state_filesystem(Arc::clone(&filesystem));
    let skill_management =
        build_local_skill_management_port(owner_user_id, Arc::clone(&filesystem))?;
    let outbound_preferences = local_dev_outbound_preferences(Arc::clone(&filesystem));
    let local_runtime = Arc::new(RebornLocalRuntimeServices {
        approval_requests: Arc::clone(&approval_requests),
        capability_leases: Arc::clone(&capability_leases),
        turn_state: Arc::clone(&turn_state),
        trigger_repository: Arc::clone(&trigger_repository),
        outbound_preferences,
        #[cfg(not(any(feature = "libsql", feature = "postgres")))]
        trigger_conversation_services,
        #[cfg(any(feature = "libsql", feature = "postgres"))]
        trigger_conversation_services: tokio::sync::OnceCell::new(),
        checkpoint_state_store,
        loop_checkpoint_store,
        thread_service,
        resource_governor: Arc::clone(&resource_governor)
            as Arc<dyn ironclaw_resources::ResourceGovernor>,
        budget_event_sink,
        in_memory_budget_event_sink,
        broadcast_budget_event_sink,
        budget_gate_store,
        skill_management,
        extension_management: None,
        runtime_http_egress: None,
        host_runtime_http_egress: None,
        skill_mounts,
        memory_mounts,
        skill_filesystem,
        workspace_filesystem,
        #[cfg(feature = "slack-v2-host-beta")]
        host_state_filesystem,
        subagent_goal_filesystem: Arc::clone(&scoped_filesystem),
        identity_filesystem: Arc::clone(&scoped_filesystem),
        identity_substrate_db,
        extension_filesystem: Arc::clone(&filesystem),
        workspace_mounts,
        local_dev_storage_root,
        default_system_prompt_path,
        event_log,
        audit_log,
        extension_registry: Arc::new(ExtensionRegistry::new()),
    });
    let process_services = ProcessServices::filesystem(Arc::clone(&scoped_filesystem));

    Ok(RebornLocalDevStoreGraph {
        run_state,
        approval_requests,
        capability_leases,
        turn_state,
        local_runtime,
        resource_governor,
        process_services,
        trigger_repository,
    })
}

#[cfg(not(feature = "libsql"))]
fn build_local_dev_store_graph(
    input: RebornLocalDevStoreGraphInput,
) -> Result<RebornLocalDevStoreGraph, RebornBuildError> {
    let RebornLocalDevStoreGraphInput {
        filesystem,
        owner_user_id,
        skill_filesystem,
        workspace_filesystem,
        workspace_mounts,
        local_dev_storage_root,
        default_system_prompt_path,
        trigger_repository,
    } = input;
    #[cfg(feature = "postgres")]
    let subagent_goal_filesystem = local_dev_scoped_filesystem(Arc::clone(&filesystem));
    let event_log = local_dev_event_log(Arc::clone(&filesystem))?;
    let audit_log = local_dev_audit_log(Arc::clone(&filesystem))?;
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
    let BudgetSinks {
        budget_event_sink,
        in_memory_budget_event_sink,
        broadcast_budget_event_sink,
        budget_gate_store,
    } = build_budget_sinks();
    let resource_governor: Arc<LocalDevResourceGovernor> =
        Arc::new(InMemoryResourceGovernor::new().with_event_sink(Arc::clone(&budget_event_sink)));
    let skill_mounts =
        skill_management_mount_view().map_err(|error| RebornBuildError::InvalidConfig {
            reason: error.to_string(),
        })?;
    let memory_mounts =
        memory_mount_view(MountPermissions::read_write_list_delete()).map_err(|error| {
            RebornBuildError::InvalidConfig {
                reason: error.to_string(),
            }
        })?;
    #[cfg(all(feature = "postgres", feature = "slack-v2-host-beta"))]
    let host_state_filesystem = local_dev_slack_host_state_filesystem(Arc::clone(&filesystem));
    let skill_management =
        build_local_skill_management_port(owner_user_id, Arc::clone(&filesystem))?;
    #[cfg(not(any(feature = "libsql", feature = "postgres")))]
    let trigger_conversation_services = local_dev_trigger_conversation_services();
    let outbound_preferences = local_dev_outbound_preferences(Arc::clone(&filesystem));
    let local_runtime = Arc::new(RebornLocalRuntimeServices {
        approval_requests: Arc::clone(&approval_requests),
        capability_leases: Arc::clone(&capability_leases),
        turn_state: Arc::clone(&turn_state),
        trigger_repository: Arc::clone(&trigger_repository),
        outbound_preferences,
        #[cfg(not(any(feature = "libsql", feature = "postgres")))]
        trigger_conversation_services,
        #[cfg(any(feature = "libsql", feature = "postgres"))]
        trigger_conversation_services: tokio::sync::OnceCell::new(),
        checkpoint_state_store,
        loop_checkpoint_store,
        thread_service,
        resource_governor: Arc::clone(&resource_governor)
            as Arc<dyn ironclaw_resources::ResourceGovernor>,
        budget_event_sink,
        in_memory_budget_event_sink,
        broadcast_budget_event_sink,
        budget_gate_store,
        skill_management,
        extension_management: None,
        runtime_http_egress: None,
        host_runtime_http_egress: None,
        skill_mounts,
        memory_mounts,
        skill_filesystem,
        workspace_filesystem,
        #[cfg(all(feature = "postgres", feature = "slack-v2-host-beta"))]
        host_state_filesystem,
        #[cfg(feature = "postgres")]
        subagent_goal_filesystem,
        extension_filesystem: Arc::clone(&filesystem),
        workspace_mounts,
        local_dev_storage_root,
        default_system_prompt_path,
        event_log,
        audit_log,
        extension_registry: Arc::new(ExtensionRegistry::new()),
    });
    let process_services = ProcessServices::in_memory();

    Ok(RebornLocalDevStoreGraph {
        run_state,
        approval_requests,
        capability_leases,
        turn_state,
        local_runtime,
        resource_governor,
        process_services,
        trigger_repository,
    })
}

#[cfg(feature = "libsql")]
async fn local_dev_trigger_repository(
    database: Arc<libsql::Database>,
) -> Result<Arc<dyn TriggerRepository>, RebornBuildError> {
    let repository = ironclaw_triggers::LibSqlTriggerRepository::new(database);
    repository
        .run_migrations()
        .await
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("local-dev trigger repository migrations failed: {error}"),
        })?;
    Ok(Arc::new(repository))
}

#[cfg(not(feature = "libsql"))]
fn local_dev_trigger_repository() -> Arc<dyn TriggerRepository> {
    Arc::new(ironclaw_triggers::InMemoryTriggerRepository::default())
}

#[cfg(not(any(feature = "libsql", feature = "postgres")))]
fn local_dev_trigger_conversation_services() -> InMemoryConversationServices {
    InMemoryConversationServices::default()
}

fn local_dev_trigger_create_hook(
    local_runtime: &Arc<RebornLocalRuntimeServices>,
) -> Arc<dyn TriggerCreateHook> {
    #[cfg(any(feature = "libsql", feature = "postgres"))]
    {
        Arc::new(LocalRuntimeTriggerCreatorPairingHook {
            runtime: Arc::clone(local_runtime),
        })
    }
    #[cfg(not(any(feature = "libsql", feature = "postgres")))]
    {
        Arc::new(InMemoryTriggerCreatorPairingHook {
            conversations: local_runtime.trigger_conversation_services.clone(),
        })
    }
}

#[cfg(not(any(feature = "libsql", feature = "postgres")))]
struct InMemoryTriggerCreatorPairingHook {
    conversations: InMemoryConversationServices,
}

#[cfg(not(any(feature = "libsql", feature = "postgres")))]
#[async_trait::async_trait]
impl TriggerCreateHook for InMemoryTriggerCreatorPairingHook {
    async fn after_trigger_persisted(&self, record: &TriggerRecord) -> Result<(), TriggerError> {
        pair_trigger_creator(&self.conversations, record).await
    }
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
struct LocalRuntimeTriggerCreatorPairingHook {
    runtime: Arc<RebornLocalRuntimeServices>,
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
#[async_trait::async_trait]
impl TriggerCreateHook for LocalRuntimeTriggerCreatorPairingHook {
    async fn after_trigger_persisted(&self, record: &TriggerRecord) -> Result<(), TriggerError> {
        let conversations = self
            .runtime
            .durable_trigger_conversation_services()
            .await
            .map_err(|error| {
                trigger_pairing_error(TriggerPairingFailureSource::ConversationInit, error)
            })?;
        pair_trigger_creator(&conversations, record).await
    }
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
struct ScopedFilesystemTriggerCreatorPairingHook<F>
where
    F: RootFilesystem + 'static,
{
    filesystem: Arc<ScopedFilesystem<F>>,
    conversations: tokio::sync::OnceCell<RebornFilesystemConversationServices>,
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
impl<F> ScopedFilesystemTriggerCreatorPairingHook<F>
where
    F: RootFilesystem + 'static,
{
    fn new(filesystem: Arc<ScopedFilesystem<F>>) -> Self {
        Self {
            filesystem,
            conversations: tokio::sync::OnceCell::new(),
        }
    }
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
#[async_trait::async_trait]
impl<F> TriggerCreateHook for ScopedFilesystemTriggerCreatorPairingHook<F>
where
    F: RootFilesystem + 'static,
{
    async fn after_trigger_persisted(&self, record: &TriggerRecord) -> Result<(), TriggerError> {
        let filesystem = Arc::clone(&self.filesystem);
        let conversations = self
            .conversations
            .get_or_try_init(|| async move {
                RebornFilesystemConversationServices::new(filesystem)
                    .await
                    .map_err(|error| {
                        trigger_pairing_error(TriggerPairingFailureSource::ConversationInit, error)
                    })
            })
            .await
            .cloned()?;
        pair_trigger_creator(&conversations, record).await
    }
}

async fn pair_trigger_creator(
    pairing: &dyn ConversationActorPairingService,
    record: &TriggerRecord,
) -> Result<(), TriggerError> {
    let adapter_kind = AdapterKind::new(TRIGGER_TRUSTED_ADAPTER_KIND).map_err(|error| {
        trigger_pairing_error(TriggerPairingFailureSource::TypedIdentity, error)
    })?;
    let adapter_installation_id =
        AdapterInstallationId::new(TRIGGER_TRUSTED_ADAPTER_INSTALLATION_ID).map_err(|error| {
            trigger_pairing_error(TriggerPairingFailureSource::TypedIdentity, error)
        })?;
    let external_actor_ref = ExternalActorRef::new(
        TRIGGER_TRUSTED_EXTERNAL_ACTOR_NAMESPACE,
        record.creator_user_id.as_str(),
    )
    .map_err(|error| trigger_pairing_error(TriggerPairingFailureSource::TypedIdentity, error))?;
    pairing
        .pair_external_actor(
            record.tenant_id.clone(),
            adapter_kind,
            adapter_installation_id,
            external_actor_ref,
            record.creator_user_id.clone(),
        )
        .await
        .map_err(|error| trigger_pairing_error(TriggerPairingFailureSource::ActorPairing, error))
}

enum TriggerPairingFailureSource {
    TypedIdentity,
    #[cfg(any(feature = "libsql", feature = "postgres"))]
    ConversationInit,
    ActorPairing,
}

impl TriggerPairingFailureSource {
    fn as_str(&self) -> &'static str {
        match self {
            Self::TypedIdentity => "typed_identity",
            #[cfg(any(feature = "libsql", feature = "postgres"))]
            Self::ConversationInit => "conversation_init",
            Self::ActorPairing => "actor_pairing",
        }
    }
}

fn trigger_pairing_error(
    source: TriggerPairingFailureSource,
    _error: impl std::fmt::Display,
) -> TriggerError {
    tracing::debug!(
        error_kind = "pairing_failure",
        error_source = source.as_str(),
        "trigger creator actor pairing failed"
    );
    TriggerError::Backend {
        reason: "trigger creator actor pairing failed".to_string(),
    }
}

struct BudgetSinks {
    budget_event_sink: Arc<dyn ironclaw_resources::BudgetEventSink>,
    in_memory_budget_event_sink: Arc<ironclaw_resources::InMemoryBudgetEventSink>,
    broadcast_budget_event_sink: Arc<ironclaw_resources::BroadcastBudgetEventSink>,
    budget_gate_store: Arc<dyn ironclaw_resources::BudgetGateStore>,
}

fn build_budget_sinks() -> BudgetSinks {
    let in_memory_budget_event_sink = Arc::new(ironclaw_resources::InMemoryBudgetEventSink::new());
    let broadcast_budget_event_sink =
        Arc::new(ironclaw_resources::BroadcastBudgetEventSink::default());
    let budget_event_sink: Arc<dyn ironclaw_resources::BudgetEventSink> =
        Arc::new(ironclaw_resources::CompositeBudgetEventSink::new(vec![
            Arc::clone(&in_memory_budget_event_sink)
                as Arc<dyn ironclaw_resources::BudgetEventSink>,
            Arc::clone(&broadcast_budget_event_sink)
                as Arc<dyn ironclaw_resources::BudgetEventSink>,
        ]));
    let budget_gate_store: Arc<dyn ironclaw_resources::BudgetGateStore> =
        Arc::new(ironclaw_resources::InMemoryBudgetGateStore::new());
    BudgetSinks {
        budget_event_sink,
        in_memory_budget_event_sink,
        broadcast_budget_event_sink,
        budget_gate_store,
    }
}

#[cfg(feature = "libsql")]
async fn build_local_dev_root_filesystem(
    root: &Path,
    workspace_root: &Path,
    host_home_root: Option<&LocalDevHostHomeRoot>,
) -> Result<LocalDevRootFilesystemBundle, RebornBuildError> {
    let db_path = root.join("reborn-local-dev.db");
    let db = Arc::new(
        libsql::Builder::new_local(&db_path)
            .build()
            .await
            .map_err(|error| RebornBuildError::InvalidConfig {
                reason: format!("local-dev libSQL database could not be opened: {error}"),
            })?,
    );
    let database = Arc::new(LibSqlRootFilesystem::new(Arc::clone(&db)));
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
    mount_local_dev_memory_root(&mut root, Arc::clone(&database))?;
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
    mount_local_dev_project_roots(&mut root, local)?;
    Ok(LocalDevRootFilesystemBundle {
        filesystem: Arc::new(root),
        database: db,
    })
}

#[cfg(not(feature = "libsql"))]
async fn build_local_dev_root_filesystem(
    root: &Path,
    workspace_root: &Path,
    host_home_root: Option<&LocalDevHostHomeRoot>,
) -> Result<LocalDevRootFilesystemBundle, RebornBuildError> {
    let local = Arc::new(local_dev_project_filesystem(
        root,
        workspace_root,
        host_home_root,
    )?);
    tracing::warn!(
        "local-dev: /memory is backed by InMemoryBackend; memory documents are ephemeral and will be lost on restart"
    );
    let mut composite = CompositeRootFilesystem::new();
    mount_local_dev_memory_root(&mut composite, Arc::new(InMemoryBackend::new()))?;
    mount_local_dev_project_roots(&mut composite, local)?;
    Ok(LocalDevRootFilesystemBundle {
        filesystem: Arc::new(composite),
    })
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
    filesystem.mount_local(
        VirtualPath::new("/system/extensions")?,
        HostPath::from_path_buf(root.join("system/extensions")),
    )?;
    if let Some(host_home_root) = host_home_root {
        filesystem.mount_local(
            VirtualPath::new("/projects/host")?,
            HostPath::from_path_buf(host_home_root.canonical_root.clone()),
        )?;
    }
    Ok(filesystem)
}

fn mount_local_dev_memory_root<F>(
    root: &mut CompositeRootFilesystem,
    backend: Arc<F>,
) -> Result<(), RebornBuildError>
where
    F: RootFilesystem + 'static,
{
    root.mount(
        local_dev_mount_descriptor(
            "/memory",
            "local-dev-memory",
            BackendKind::MemoryDocuments,
            StorageClass::StructuredRecords,
            ContentKind::MemoryDocument,
            IndexPolicy::FullTextAndVector,
            backend.capabilities(),
        )?,
        backend,
    )?;
    Ok(())
}

fn mount_local_dev_project_roots(
    root: &mut CompositeRootFilesystem,
    local: Arc<LocalFilesystem>,
) -> Result<(), RebornBuildError> {
    root.mount(
        local_dev_mount_descriptor(
            "/projects",
            "local-dev-project-files",
            BackendKind::LocalFilesystem,
            StorageClass::FileContent,
            ContentKind::ProjectFile,
            IndexPolicy::NotIndexed,
            BackendCapabilities::bytes_only(),
        )?,
        Arc::clone(&local),
    )?;
    root.mount(
        local_dev_mount_descriptor(
            "/system/extensions",
            "local-dev-system-extensions",
            BackendKind::LocalFilesystem,
            StorageClass::FileContent,
            ContentKind::ExtensionPackage,
            IndexPolicy::NotIndexed,
            BackendCapabilities::bytes_only(),
        )?,
        local,
    )?;
    Ok(())
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
fn build_local_dev_secret_store<F>(
    root: &Path,
    scoped_filesystem: Arc<ScopedFilesystem<F>>,
) -> Result<Arc<FilesystemSecretStore<F>>, RebornBuildError>
where
    F: RootFilesystem + 'static,
{
    let master_key = resolve_local_dev_secret_master_key(root)?;
    let crypto = Arc::new(ironclaw_secrets::SecretsCrypto::new(master_key)?);
    Ok(Arc::new(FilesystemSecretStore::new(
        scoped_filesystem,
        crypto,
    )))
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
fn resolve_local_dev_secret_master_key(
    root: &Path,
) -> Result<ironclaw_secrets::SecretMaterial, RebornBuildError> {
    let key_path = root.join(LOCAL_DEV_SECRETS_MASTER_KEY_PATH);
    match std::fs::read_to_string(&key_path) {
        Ok(existing) => {
            return Ok(ironclaw_secrets::SecretMaterial::from(
                existing.trim().to_string(),
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(RebornBuildError::InvalidConfig {
                reason: format!("local-dev secrets master key could not be read: {error}"),
            });
        }
    }

    let key = std::env::var(ironclaw_secrets::keychain::SECRETS_MASTER_KEY_ENV)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(ironclaw_secrets::keychain::generate_master_key_hex);
    write_local_dev_secret_master_key(&key_path, &key)?;
    Ok(ironclaw_secrets::SecretMaterial::from(key))
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
fn write_local_dev_secret_master_key(path: &Path, key: &str) -> Result<(), RebornBuildError> {
    #[cfg(unix)]
    {
        use std::io::Write as _;
        use std::os::unix::fs::OpenOptionsExt as _;

        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
            .map_err(|error| RebornBuildError::InvalidConfig {
                reason: format!("local-dev secrets master key could not be created: {error}"),
            })?;
        file.write_all(key.as_bytes())
            .and_then(|_| file.write_all(b"\n"))
            .map_err(|error| RebornBuildError::InvalidConfig {
                reason: format!("local-dev secrets master key could not be written: {error}"),
            })
    }
    #[cfg(windows)]
    {
        use std::io::Write as _;

        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|error| RebornBuildError::InvalidConfig {
                reason: format!("local-dev secrets master key could not be created: {error}"),
            })?;
        let account = std::env::var("USERDOMAIN")
            .ok()
            .filter(|domain| !domain.trim().is_empty())
            .zip(
                std::env::var("USERNAME")
                    .ok()
                    .filter(|user| !user.trim().is_empty()),
            )
            .map(|(domain, user)| format!("{domain}\\{user}"))
            .or_else(|| std::env::var("USERNAME").ok())
            .ok_or_else(|| RebornBuildError::InvalidConfig {
                reason: "local-dev secrets master key could not be restricted: USERNAME is unset"
                    .to_string(),
            })?;
        let status = std::process::Command::new("icacls")
            .arg(path)
            .arg("/inheritance:r")
            .arg("/grant:r")
            .arg(format!("{account}:F"))
            .status()
            .map_err(|error| RebornBuildError::InvalidConfig {
                reason: format!(
                    "local-dev secrets master key permissions could not be set: {error}"
                ),
            })?;
        if !status.success() {
            let _ = std::fs::remove_file(path);
            return Err(RebornBuildError::InvalidConfig {
                reason: format!(
                    "local-dev secrets master key permissions could not be set: icacls exited with {status}"
                ),
            });
        }
        file.write_all(key.as_bytes())
            .and_then(|_| file.write_all(b"\n"))
            .map_err(|error| RebornBuildError::InvalidConfig {
                reason: format!("local-dev secrets master key could not be written: {error}"),
            })
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        let _ = key;
        Err(RebornBuildError::InvalidConfig {
            reason:
                "local-dev filesystem secret persistence requires Unix permissions or Windows ACLs"
                    .to_string(),
        })
    }
}

// Intentionally uncfg'd: called from both libsql and no-libsql local-dev root
// filesystem paths.
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

#[cfg(any(feature = "libsql", feature = "postgres"))]
fn local_dev_scoped_filesystem(
    filesystem: Arc<LocalDevRootFilesystem>,
) -> Arc<ScopedFilesystem<LocalDevRootFilesystem>> {
    crate::wrap_scoped(filesystem)
}

#[cfg(feature = "libsql")]
fn local_dev_outbound_preferences(
    filesystem: Arc<LocalDevRootFilesystem>,
) -> Arc<dyn CommunicationPreferenceRepository> {
    Arc::new(FilesystemOutboundStateStore::new(
        local_dev_scoped_filesystem(filesystem),
    ))
}

#[cfg(not(feature = "libsql"))]
fn local_dev_outbound_preferences(
    _filesystem: Arc<LocalDevRootFilesystem>,
) -> Arc<dyn CommunicationPreferenceRepository> {
    Arc::new(InMemoryOutboundStateStore::default())
}

#[cfg(all(
    any(feature = "libsql", feature = "postgres"),
    feature = "slack-v2-host-beta"
))]
fn local_dev_slack_host_state_filesystem(
    filesystem: Arc<LocalDevRootFilesystem>,
) -> Arc<ScopedFilesystem<LocalDevRootFilesystem>> {
    Arc::new(ScopedFilesystem::new(
        filesystem,
        crate::slack_host_state_mount_view,
    ))
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
/// When `host_home_root` is present, the runtime view is the local-dev-yolo
/// ambient coding-tool view: it grants raw workspace and host-home aliases so
/// real local paths resolve through the same virtual roots as `/workspace` and
/// `/host`.
fn build_workspace_filesystems(
    filesystem: Arc<LocalDevRootFilesystem>,
    workspace_root: &Path,
    host_home_root: Option<&LocalDevHostHomeRoot>,
) -> Result<LocalDevWorkspaceFilesystems, RebornBuildError> {
    let read_only_workspace_mounts = workspace_mount_view(MountPermissions::read_only(), &[])
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: error.to_string(),
        })?;
    let host_home_aliases = host_home_root
        .map(|root| root.aliases())
        .unwrap_or_default();
    let workspace_aliases = if host_home_root.is_some() {
        vec![workspace_root]
    } else {
        Vec::new()
    };
    let runtime_workspace_mounts = ambient_workspace_mount_view(
        MountPermissions::read_write(),
        &workspace_aliases,
        &host_home_aliases,
    )
    .map_err(|error| RebornBuildError::InvalidConfig {
        reason: error.to_string(),
    })?;
    let skill_filesystem = Arc::new(ScopedFilesystem::new(
        Arc::clone(&filesystem),
        scoped_skill_context_mount_view,
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
        ("/system/extensions", storage_root.join("system/extensions")),
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

fn local_dev_default_system_prompt_path(storage_root: &Path) -> PathBuf {
    storage_root.join(LOCAL_DEV_DEFAULT_SYSTEM_PROMPT_PATH)
}

fn paths_overlap(left: &Path, right: &Path) -> bool {
    left == right || left.starts_with(right) || right.starts_with(left)
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

fn builtin_first_party_registry_with_trigger_create_hook(
    trigger_repository: Arc<dyn TriggerRepository>,
    trigger_create_hook: Arc<dyn TriggerCreateHook>,
) -> Result<FirstPartyCapabilityRegistry, RebornBuildError> {
    builtin_first_party_handlers_with_trigger_create_hook(trigger_repository, trigger_create_hook)
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("built-in first-party handlers are invalid: {error}"),
        })
}

fn local_dev_builtin_extension_registry() -> Result<ExtensionRegistry, RebornBuildError> {
    let mut registry = builtin_extension_registry()?;
    let builtin_id =
        ExtensionId::new("builtin").map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("built-in first-party package id is invalid: {error}"),
        })?;
    let package = registry
        .remove(&builtin_id)
        .ok_or_else(|| RebornBuildError::InvalidConfig {
            reason: "built-in first-party package is missing".to_string(),
        })?;
    let package = extend_builtin_first_party_package(package).map_err(|error| {
        RebornBuildError::InvalidConfig {
            reason: format!("local-dev extension lifecycle package is invalid: {error}"),
        }
    })?;
    registry
        .insert(package)
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("local-dev built-in first-party registry is invalid: {error}"),
        })?;
    Ok(registry)
}

fn local_dev_first_party_trust_policy() -> Result<HostTrustPolicy, RebornBuildError> {
    let policy =
        local_dev_capability_policy().map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("local-dev capability policy is invalid: {error}"),
        })?;
    HostTrustPolicy::new(vec![Box::new(AdminConfig::with_entries(vec![
        AdminEntry::for_local_manifest(
            policy.provider.id,
            policy.provider.manifest_path,
            None,
            HostTrustAssignment::first_party(),
            policy.provider.authority_effects,
            None,
        ),
        AdminEntry::for_local_manifest(
            PackageId::new("web-access").map_err(|error| RebornBuildError::InvalidConfig {
                reason: format!("Web Access first-party package id is invalid: {error}"),
            })?,
            "/system/extensions/web-access/manifest.toml".to_string(),
            Some(web_access_manifest_digest()),
            HostTrustAssignment::first_party(),
            web_access_allowed_effects(),
            None,
        ),
        AdminEntry::for_local_manifest(
            PackageId::new("google-calendar").map_err(|error| RebornBuildError::InvalidConfig {
                reason: format!("Google Calendar first-party package id is invalid: {error}"),
            })?,
            "/system/extensions/google-calendar/manifest.toml".to_string(),
            Some(google_calendar_manifest_digest()),
            HostTrustAssignment::first_party(),
            gsuite_allowed_effects(),
            None,
        ),
        AdminEntry::for_local_manifest(
            PackageId::new("google-docs").map_err(|error| RebornBuildError::InvalidConfig {
                reason: format!("Google Docs first-party package id is invalid: {error}"),
            })?,
            "/system/extensions/google-docs/manifest.toml".to_string(),
            Some(google_docs_manifest_digest()),
            HostTrustAssignment::first_party(),
            gsuite_allowed_effects(),
            None,
        ),
        AdminEntry::for_local_manifest(
            PackageId::new("google-drive").map_err(|error| RebornBuildError::InvalidConfig {
                reason: format!("Google Drive first-party package id is invalid: {error}"),
            })?,
            "/system/extensions/google-drive/manifest.toml".to_string(),
            Some(google_drive_manifest_digest()),
            HostTrustAssignment::first_party(),
            gsuite_allowed_effects(),
            None,
        ),
        AdminEntry::for_local_manifest(
            PackageId::new("google-sheets").map_err(|error| RebornBuildError::InvalidConfig {
                reason: format!("Google Sheets first-party package id is invalid: {error}"),
            })?,
            "/system/extensions/google-sheets/manifest.toml".to_string(),
            Some(google_sheets_manifest_digest()),
            HostTrustAssignment::first_party(),
            gsuite_allowed_effects(),
            None,
        ),
        AdminEntry::for_local_manifest(
            PackageId::new("google-slides").map_err(|error| RebornBuildError::InvalidConfig {
                reason: format!("Google Slides first-party package id is invalid: {error}"),
            })?,
            "/system/extensions/google-slides/manifest.toml".to_string(),
            Some(google_slides_manifest_digest()),
            HostTrustAssignment::first_party(),
            gsuite_allowed_effects(),
            None,
        ),
        AdminEntry::for_local_manifest(
            PackageId::new("gmail").map_err(|error| RebornBuildError::InvalidConfig {
                reason: format!("Gmail first-party package id is invalid: {error}"),
            })?,
            "/system/extensions/gmail/manifest.toml".to_string(),
            Some(gmail_manifest_digest()),
            HostTrustAssignment::first_party(),
            gsuite_allowed_effects(),
            None,
        ),
        AdminEntry::for_local_manifest(
            PackageId::new("notion").map_err(|error| RebornBuildError::InvalidConfig {
                reason: format!("Notion MCP first-party package id is invalid: {error}"),
            })?,
            "/system/extensions/notion/manifest.toml".to_string(),
            Some(notion_mcp_manifest_digest()),
            HostTrustAssignment::first_party(),
            notion_mcp_allowed_effects(),
            None,
        ),
    ]))])
    .map_err(|error| RebornBuildError::InvalidConfig {
        reason: format!("built-in first-party trust policy is invalid: {error}"),
    })
}

fn gsuite_allowed_effects() -> Vec<EffectKind> {
    vec![
        EffectKind::DispatchCapability,
        EffectKind::Network,
        EffectKind::UseSecret,
        EffectKind::ExternalWrite,
    ]
}

fn web_access_allowed_effects() -> Vec<EffectKind> {
    vec![EffectKind::DispatchCapability, EffectKind::Network]
}

fn notion_mcp_allowed_effects() -> Vec<EffectKind> {
    vec![
        EffectKind::DispatchCapability,
        EffectKind::Network,
        EffectKind::UseSecret,
        EffectKind::ExternalWrite,
    ]
}

#[cfg(test)]
fn nearai_allowed_effects() -> Vec<EffectKind> {
    vec![
        EffectKind::DispatchCapability,
        EffectKind::Network,
        EffectKind::UseSecret,
    ]
}

async fn build_production_shaped(
    input: RebornBuildInput,
) -> Result<RebornServices, RebornBuildError> {
    let RebornBuildInput {
        profile,
        owner_id,
        storage,
        production_trust_policy,
        runtime_policy,
        turn_run_wake_notifier,
        runtime_process_binding,
        required_runtime_backends,
        require_runtime_http_egress,
        require_wasm_credentials,
        #[cfg(all(test, feature = "slack-v2-host-beta"))]
            host_runtime_http_egress_for_test: _,
        product_auth_ports,
        oauth_provider_configs,
        oauth_dcr_provider_configs,
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
        owner_id,
        required_runtime_backends,
        require_runtime_http_egress,
        require_wasm_credentials,
        product_auth_ports,
        oauth_provider_configs,
        oauth_dcr_provider_configs,
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
                oauth_provider_configs,
                oauth_dcr_provider_configs,
                owner_id,
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
                oauth_provider_configs,
                oauth_dcr_provider_configs,
                owner_id,
            };
            build_postgres_production(context, pool, url, secret_master_key).await
        }
    }
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
async fn resolve_secret_master_key(
    explicit: Option<ironclaw_secrets::SecretMaterial>,
) -> Result<ironclaw_secrets::SecretMaterial, RebornBuildError> {
    resolve_explicit_or_keychain_master_key(explicit)
        .await?
        .ok_or(RebornBuildError::MissingSecretMasterKey)
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
    oauth_provider_configs: Vec<crate::input::OAuthProviderBackendConfig>,
    oauth_dcr_provider_configs: Vec<crate::input::OAuthDcrProviderBackendConfig>,
    owner_id: String,
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
type FilesystemProductionHostRuntimeServices<F> = HostRuntimeServices<
    F,
    PersistentResourceGovernor<FilesystemResourceGovernorStore<F>>,
    ironclaw_processes::FilesystemProcessStore<F>,
    ironclaw_processes::FilesystemProcessResultStore<F>,
>;

#[cfg(feature = "libsql")]
pub(crate) async fn build_libsql_production_host_runtime_services<TPolicy, TWake>(
    config: crate::LibSqlProductionSubstrateConfig<TPolicy, TWake>,
) -> Result<crate::LibSqlProductionHostRuntimeServices, crate::RebornCompositionError>
where
    TPolicy: ironclaw_trust::TrustPolicy + 'static,
    TWake: ironclaw_turns::TurnRunWakeNotifier + 'static,
{
    let filesystem = Arc::new(LibSqlRootFilesystem::new(Arc::clone(&config.database)));
    filesystem.run_migrations().await?;
    build_filesystem_production_host_runtime_services(
        filesystem,
        config.event_store,
        config.secret_master_key,
        config.trust_policy,
        config.runtime_policy,
        config.turn_run_wake_notifier,
        config.surface_version,
    )
    .await
}

#[cfg(feature = "postgres")]
pub(crate) async fn build_postgres_production_host_runtime_services<TPolicy, TWake>(
    config: crate::PostgresProductionSubstrateConfig<TPolicy, TWake>,
) -> Result<crate::PostgresProductionHostRuntimeServices, crate::RebornCompositionError>
where
    TPolicy: ironclaw_trust::TrustPolicy + 'static,
    TWake: ironclaw_turns::TurnRunWakeNotifier + 'static,
{
    let filesystem = Arc::new(ironclaw_filesystem::PostgresRootFilesystem::new(
        config.pool,
    ));
    filesystem.run_migrations().await?;
    build_filesystem_production_host_runtime_services(
        filesystem,
        config.event_store,
        config.secret_master_key,
        config.trust_policy,
        config.runtime_policy,
        config.turn_run_wake_notifier,
        config.surface_version,
    )
    .await
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
async fn build_filesystem_production_host_runtime_services<F, TPolicy, TWake>(
    filesystem: Arc<F>,
    event_store: ironclaw_reborn_event_store::RebornEventStoreConfig,
    secret_master_key: Option<ironclaw_secrets::SecretMaterial>,
    trust_policy: Arc<TPolicy>,
    runtime_policy: crate::RebornProductionRuntimePolicy,
    turn_run_wake_notifier: Arc<TWake>,
    surface_version: CapabilitySurfaceVersion,
) -> Result<FilesystemProductionHostRuntimeServices<F>, crate::RebornCompositionError>
where
    F: RootFilesystem + 'static,
    TPolicy: ironclaw_trust::TrustPolicy + 'static,
    TWake: ironclaw_turns::TurnRunWakeNotifier + 'static,
{
    let scoped_filesystem = crate::wrap_scoped(Arc::clone(&filesystem));
    let process_services = ProcessServices::filesystem(Arc::clone(&scoped_filesystem));
    let secret_credentials = build_filesystem_secret_credential_stores(
        Arc::clone(&scoped_filesystem),
        secret_master_key,
    )
    .await?;
    let resource_store = FilesystemResourceGovernorStore::new(Arc::clone(&scoped_filesystem));
    let governor = Arc::new(PersistentResourceGovernor::new(resource_store));
    let capability_leases = Arc::new(FilesystemCapabilityLeaseStore::new(Arc::clone(
        &scoped_filesystem,
    )));
    let (runtime_policy, process_binding) = runtime_policy.into_parts();

    let services = HostRuntimeServices::new(
        Arc::new(ExtensionRegistry::new()),
        filesystem,
        governor,
        Arc::new(GrantAuthorizer::new()),
        process_services,
        surface_version,
    )
    .with_trust_policy(trust_policy)
    .with_runtime_policy(runtime_policy)
    .with_capability_leases(capability_leases)
    .with_security_audit_sink(Arc::new(ironclaw_events::TracingSecurityAuditSink))
    .with_secret_store(Arc::clone(&secret_credentials.secret_store))
    .with_credential_broker(secret_credentials.credential_broker)
    .with_turn_run_wake_notifier(turn_run_wake_notifier)
    .with_filesystem_run_state(Arc::clone(&scoped_filesystem))
    .with_filesystem_turn_state_store(Arc::clone(&scoped_filesystem))
    .with_run_profile_resolver(Arc::new(
        ironclaw_reborn::planned_driver_factory::default_planned_run_profile_resolver()?,
    ))
    .with_reborn_event_store_config(
        ironclaw_reborn_event_store::RebornProfile::Production,
        event_store,
    )
    .await?;
    let services = apply_production_runtime_process_binding(services, process_binding);

    let services = services
        .try_with_host_http_egress_with_body_store(
            ironclaw_network::PolicyNetworkHttpEgress::new(
                ironclaw_network::ReqwestNetworkTransport::default(),
            ),
            Arc::clone(&scoped_filesystem),
        )
        .map_err(crate::RebornCompositionError::from)?;

    Ok(services)
}

/// Central production secret/credential stores over the shared
/// [`ScopedFilesystem`].
///
/// Backend selection is now a property of the underlying
/// [`RootFilesystem`] (libSQL/Postgres/in-memory), not of each store itself.
/// The secret store and credential broker are deliberately built together from
/// one scoped filesystem and one crypto handle so production composition does
/// not grow parallel ad hoc secret/credential stores.
#[cfg(any(feature = "libsql", feature = "postgres"))]
struct FilesystemSecretCredentialStores<F>
where
    F: RootFilesystem + 'static,
{
    secret_store: Arc<FilesystemSecretStore<F>>,
    credential_broker: Arc<FilesystemCredentialBroker<F>>,
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
impl<F> FilesystemSecretCredentialStores<F>
where
    F: RootFilesystem + 'static,
{
    fn new(
        scoped_filesystem: Arc<ScopedFilesystem<F>>,
        crypto: Arc<ironclaw_secrets::SecretsCrypto>,
    ) -> Self {
        Self {
            secret_store: Arc::new(FilesystemSecretStore::new(
                Arc::clone(&scoped_filesystem),
                Arc::clone(&crypto),
            )),
            credential_broker: Arc::new(FilesystemCredentialBroker::new(scoped_filesystem, crypto)),
        }
    }

    fn from_master_key(
        scoped_filesystem: Arc<ScopedFilesystem<F>>,
        master_key: ironclaw_secrets::SecretMaterial,
    ) -> Result<Self, crate::RebornCompositionError> {
        Ok(Self::new(
            scoped_filesystem,
            Arc::new(ironclaw_secrets::SecretsCrypto::new(master_key)?),
        ))
    }
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
async fn build_filesystem_secret_credential_stores<F>(
    scoped_filesystem: Arc<ScopedFilesystem<F>>,
    master_key: Option<ironclaw_secrets::SecretMaterial>,
) -> Result<FilesystemSecretCredentialStores<F>, crate::RebornCompositionError>
where
    F: RootFilesystem + 'static,
{
    let master_key = resolve_explicit_or_keychain_master_key(master_key)
        .await?
        .ok_or(crate::RebornCompositionError::MissingSecretMasterKey)?;
    FilesystemSecretCredentialStores::from_master_key(scoped_filesystem, master_key)
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
async fn resolve_explicit_or_keychain_master_key(
    explicit: Option<ironclaw_secrets::SecretMaterial>,
) -> Result<Option<ironclaw_secrets::SecretMaterial>, ironclaw_secrets::SecretError> {
    if let Some(master_key) = explicit {
        Ok(Some(master_key))
    } else if let Some(master_key) =
        ironclaw_secrets::keychain::resolve_master_key_material().await?
    {
        Ok(Some(master_key))
    } else {
        Ok(None)
    }
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
struct ProductionStoreBundle<F>
where
    F: RootFilesystem + 'static,
{
    filesystem: Arc<F>,
    scoped_filesystem: Arc<ScopedFilesystem<F>>,
    leases: Arc<FilesystemCapabilityLeaseStore<F>>,
    secret_credentials: FilesystemSecretCredentialStores<F>,
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
        let secret_credentials = FilesystemSecretCredentialStores::from_master_key(
            Arc::clone(&scoped_filesystem),
            secret_master_key,
        )?;

        Ok(Self {
            filesystem,
            scoped_filesystem,
            leases,
            secret_credentials,
            event_store,
        })
    }
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
fn production_skill_management_mount_view(
    scope: &ResourceScope,
) -> Result<MountView, HostApiError> {
    MountView::new(vec![
        MountGrant::new(
            MountAlias::new("/skills")?,
            VirtualPath::new(format!(
                "/tenants/{}/users/{}/skills",
                scope.tenant_id.as_str(),
                scope.user_id.as_str()
            ))?,
            MountPermissions::read_write_list_delete(),
        ),
        MountGrant::new(
            MountAlias::new("/system/skills")?,
            VirtualPath::new("/system/skills")?,
            MountPermissions::read_only(),
        ),
    ])
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
async fn build_backend_production<F>(
    context: RebornProductionBuildContext,
    stores: ProductionStoreBundle<F>,
    trigger_repository: Arc<dyn TriggerRepository>,
) -> Result<RebornServices, RebornBuildError>
where
    F: RootFilesystem + 'static,
{
    let RebornProductionBuildContext {
        profile,
        wiring_config,
        production_wiring,
        product_auth_ports,
        oauth_provider_configs,
        oauth_dcr_provider_configs,
        owner_id,
    } = context;
    let owner_user_id = UserId::new(owner_id).map_err(|error| RebornBuildError::InvalidConfig {
        reason: error.to_string(),
    })?;
    let secret_store: Arc<dyn SecretStore> = stores.secret_credentials.secret_store.clone();
    let skill_management_filesystem: Arc<dyn RootFilesystem> = stores.filesystem.clone();
    let skill_management = Arc::new(RebornLocalSkillManagementPort::new_with_mount_resolver(
        owner_user_id,
        skill_management_filesystem,
        Arc::new(production_skill_management_mount_view),
    ));
    let trigger_create_hook = Arc::new(ScopedFilesystemTriggerCreatorPairingHook::new(Arc::clone(
        &stores.scoped_filesystem,
    )));
    let mut first_party_registry = builtin_first_party_registry_with_trigger_create_hook(
        trigger_repository,
        trigger_create_hook,
    )?;
    let product_auth_filesystem = Arc::clone(&stores.scoped_filesystem);
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
    .with_capability_leases(stores.leases)
    .with_secret_store(Arc::clone(&stores.secret_credentials.secret_store))
    .with_credential_broker(stores.secret_credentials.credential_broker)
    .with_security_audit_sink(Arc::new(ironclaw_events::TracingSecurityAuditSink))
    .try_with_host_http_egress_with_body_store(
        ironclaw_network::PolicyNetworkHttpEgress::new(
            ironclaw_network::ReqwestNetworkTransport::default(),
        ),
        Arc::clone(&stores.scoped_filesystem),
    )?
    .with_filesystem_resource_governor(Arc::clone(&stores.scoped_filesystem))
    .with_reborn_event_store_config(profile.to_event_store_profile(), stores.event_store)
    .await?
    .with_filesystem_run_state(Arc::clone(&stores.scoped_filesystem))
    .with_filesystem_turn_state_store(Arc::clone(&stores.scoped_filesystem))
    .with_run_profile_resolver(planned_run_profile_resolver()?)
    .with_turn_run_wake_notifier(production_wiring.turn_run_wake_notifier);
    let product_auth_runtime_ports = require_product_auth_runtime_ports(&services)?;
    let services = attach_hosted_mcp_runtime(services)?;
    let provider_composition = compose_provider_client(
        oauth_provider_configs,
        oauth_dcr_provider_configs,
        Arc::clone(&secret_store),
        product_auth_runtime_ports.clone(),
    )?;
    let services = apply_production_runtime_process_binding(
        services,
        production_wiring.runtime_process_binding,
    );
    let services = attach_wasm_runtime(services)?;

    let turn_coordinator: Arc<dyn ironclaw_turns::TurnCoordinator> =
        Arc::new(services.turn_coordinator_for_production()?);
    let product_auth_ports = product_auth_ports.unwrap_or_else(|| {
        let durable = Arc::new(FilesystemAuthProductServices::new(
            product_auth_filesystem,
            Arc::clone(&secret_store),
        ));
        RebornProductAuthServicePorts::from_shared_with_provider(
            durable,
            provider_composition
                .client
                .clone()
                .unwrap_or_else(|| Arc::new(UnavailableAuthProviderClient)),
        )
    });
    let product_auth_services = compose_product_auth_services(
        product_auth_ports,
        turn_coordinator.clone(),
        provider_composition,
    );
    let product_auth_ready = true;
    // Wire ProductAuthAccount runtime credential resolver before
    // host_runtime_for_production so WASM extensions whose manifest declares a
    // ProductAuthAccount runtime credential source resolve through
    // CredentialAccountService. Unconditional in production: product_auth_services
    // always exists (durable filesystem fallback from #4234).
    let services = services.with_runtime_credential_account_resolver(Arc::new(
        ProductAuthRuntimeCredentialResolver::new(
            product_auth_services.runtime_credential_account_selection_service(),
        ),
    ));
    register_bundled_gsuite_first_party_handlers(
        &mut first_party_registry,
        product_auth_services.credential_account_service(),
        product_auth_services.credential_account_record_source(),
        Arc::new(ProductAuthRuntimeGsuiteCredentialStager::new(
            product_auth_runtime_ports.clone(),
        )),
    )
    .map_err(|error| RebornBuildError::InvalidConfig {
        reason: format!("GSuite first-party handlers are invalid: {error}"),
    })?;
    let services = services.with_first_party_capabilities(Arc::new(first_party_registry));

    let host_runtime: Arc<dyn ironclaw_host_runtime::HostRuntime> =
        Arc::new(services.host_runtime_for_production(&wiring_config)?);

    Ok(RebornServices {
        host_runtime: Some(host_runtime),
        turn_coordinator: Some(turn_coordinator),
        readiness: readiness_for(profile, true, true, product_auth_ready),
        product_auth: Some(product_auth_services),
        skill_management: Some(skill_management),
        local_runtime: None,
        #[cfg(feature = "root-llm-provider")]
        secret_store,
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
    let trigger_repository = Arc::new(ironclaw_triggers::LibSqlTriggerRepository::new(db));
    trigger_repository
        .run_migrations()
        .await
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("libSQL trigger repository migrations failed: {error}"),
        })?;
    let stores = ProductionStoreBundle::new(
        filesystem,
        secret_master_key,
        ironclaw_reborn_event_store::RebornEventStoreConfig::Libsql {
            path_or_url,
            auth_token,
        },
    )?;

    build_backend_production(context, stores, trigger_repository).await
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
    let trigger_repository = Arc::new(ironclaw_triggers::PostgresTriggerRepository::new(pool));
    trigger_repository
        .run_migrations()
        .await
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("PostgreSQL trigger repository migrations failed: {error}"),
        })?;
    let stores = ProductionStoreBundle::new(
        filesystem,
        secret_master_key,
        ironclaw_reborn_event_store::RebornEventStoreConfig::Postgres { url },
    )?;

    build_backend_production(context, stores, trigger_repository).await
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
        workers: RebornWorkerReadiness {
            turn_runner: false,
            trigger_poller: false,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_auth::{
        AuthProductScope, AuthSurface, CredentialAccountLabel, CredentialAccountStatus,
        CredentialOwnership, GOOGLE_CALENDAR_EVENTS_SCOPE, GOOGLE_GMAIL_SEND_SCOPE,
        NewCredentialAccount, ProviderScope,
    };
    use ironclaw_filesystem::FilesystemError;
    #[cfg(any(feature = "libsql", feature = "postgres"))]
    use ironclaw_filesystem::{
        DirEntry, FileStat, FilesystemOperation, RootFilesystem, VersionedEntry,
    };
    use ironclaw_host_api::{
        CapabilityGrant, CapabilityGrantId, CapabilityId, CapabilitySet, EffectKind,
        ExecutionContext, ExtensionId, GrantConstraints, InvocationId, MountAlias, MountGrant,
        MountPermissions, NetworkPolicy, NetworkScheme, NetworkTargetPattern, Principal,
        ResourceEstimate, ResourceScope, RuntimeCredentialAccountProviderId,
        RuntimeCredentialRequirementSource, RuntimeKind, ScopedPath, SecretHandle, TenantId,
        TrustClass, UserId, VirtualPath,
    };
    use ironclaw_host_runtime::{
        MEMORY_SEARCH_CAPABILITY_ID, MEMORY_TREE_CAPABILITY_ID, MEMORY_WRITE_CAPABILITY_ID,
        RuntimeCapabilityOutcome, RuntimeCapabilityRequest, RuntimeFailureKind,
        SKILL_INSTALL_CAPABILITY_ID, SKILL_LIST_CAPABILITY_ID, SKILL_REMOVE_CAPABILITY_ID,
        TRIGGER_CREATE_CAPABILITY_ID, TRIGGER_LIST_CAPABILITY_ID, TRIGGER_REMOVE_CAPABILITY_ID,
    };
    use ironclaw_product_workflow::{LifecyclePackageKind, LifecyclePackageRef};
    use ironclaw_trust::{AuthorityCeiling, EffectiveTrustClass, TrustDecision, TrustProvenance};
    #[cfg(feature = "libsql")]
    use secrecy::ExposeSecret;

    use crate::{
        extension_lifecycle::ExtensionActivationMode, runtime::SKILL_ACTIVATE_CAPABILITY_ID,
    };

    struct FailingConversationActorPairingService;

    #[async_trait::async_trait]
    impl ConversationActorPairingService for FailingConversationActorPairingService {
        async fn pair_external_actor(
            &self,
            _tenant_id: TenantId,
            _adapter_kind: AdapterKind,
            _adapter_installation_id: AdapterInstallationId,
            _external_actor_ref: ExternalActorRef,
            _user_id: UserId,
        ) -> Result<(), ironclaw_conversations::InboundTurnError> {
            Err(ironclaw_conversations::InboundTurnError::DurableState {
                reason: "raw durable store error".to_string(),
            })
        }
    }

    #[cfg(any(feature = "libsql", feature = "postgres"))]
    struct FailingConversationStateFilesystem;

    #[cfg(any(feature = "libsql", feature = "postgres"))]
    #[async_trait::async_trait]
    impl RootFilesystem for FailingConversationStateFilesystem {
        async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
            Err(FilesystemError::Backend {
                path: path.clone(),
                operation: FilesystemOperation::ReadFile,
                reason: "conversation state load failed".to_string(),
            })
        }

        async fn list_dir(&self, _path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
            Ok(Vec::new())
        }

        async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
            Err(FilesystemError::NotFound {
                path: path.clone(),
                operation: FilesystemOperation::ReadFile,
            })
        }
    }

    fn trigger_record_for_pairing_test() -> TriggerRecord {
        TriggerRecord {
            trigger_id: ironclaw_triggers::TriggerId::new(),
            tenant_id: TenantId::new("pairing-test-tenant").expect("tenant id"),
            creator_user_id: UserId::new("pairing-test-user").expect("user id"),
            agent_id: None,
            project_id: None,
            ownership_scope: ironclaw_triggers::TriggerOwnershipScope::Personal,
            name: "pairing test".to_string(),
            source: ironclaw_triggers::TriggerSourceKind::Schedule,
            schedule: ironclaw_triggers::TriggerSchedule::cron("* * * * *")
                .expect("valid cron expression"),
            completion_policy: ironclaw_triggers::TriggerCompletionPolicy::Recurring,
            prompt: "pairing test prompt".to_string(),
            state: ironclaw_triggers::TriggerState::Scheduled,
            next_run_at: chrono::Utc::now(),
            last_run_at: None,
            last_fired_slot: None,
            last_status: None,
            active_fire_slot: None,
            active_run_ref: None,
            created_at: chrono::Utc::now(),
        }
    }

    #[tokio::test]
    async fn pair_trigger_creator_maps_pairing_failure_to_sanitized_backend_error() {
        let record = trigger_record_for_pairing_test();

        let error = pair_trigger_creator(&FailingConversationActorPairingService, &record)
            .await
            .expect_err("pairing failure should surface");

        let TriggerError::Backend { reason } = error else {
            panic!("expected backend trigger error");
        };
        assert_eq!(reason, "trigger creator actor pairing failed");
    }

    #[cfg(any(feature = "libsql", feature = "postgres"))]
    async fn local_runtime_with_failing_trigger_conversations() -> Arc<RebornLocalRuntimeServices> {
        let local_dev_root = tempfile::tempdir().expect("tempdir");
        let owner_user_id = "pairing-owner";
        let services = build_reborn_services(RebornBuildInput::local_dev(
            owner_user_id,
            local_dev_root.path().join("local-dev"),
        ))
        .await
        .expect("local-dev services build");

        let base_runtime = services.local_runtime.expect("local runtime");
        let mut failing_root = CompositeRootFilesystem::new();
        failing_root
            .mount(
                local_dev_mount_descriptor(
                    "/conversations",
                    "failing-conversation-state",
                    BackendKind::Custom("test".to_string()),
                    StorageClass::StructuredRecords,
                    ContentKind::StructuredRecord,
                    IndexPolicy::NotIndexed,
                    BackendCapabilities::default(),
                )
                .expect("mount descriptor"),
                Arc::new(FailingConversationStateFilesystem),
            )
            .expect("mount failing backend");
        Arc::new(RebornLocalRuntimeServices {
            approval_requests: Arc::clone(&base_runtime.approval_requests),
            capability_leases: Arc::clone(&base_runtime.capability_leases),
            turn_state: Arc::clone(&base_runtime.turn_state),
            trigger_repository: Arc::clone(&base_runtime.trigger_repository),
            outbound_preferences: Arc::clone(&base_runtime.outbound_preferences),
            #[cfg(not(any(feature = "libsql", feature = "postgres")))]
            trigger_conversation_services: base_runtime.trigger_conversation_services.clone(),
            #[cfg(any(feature = "libsql", feature = "postgres"))]
            trigger_conversation_services: tokio::sync::OnceCell::new(),
            checkpoint_state_store: Arc::clone(&base_runtime.checkpoint_state_store),
            loop_checkpoint_store: Arc::clone(&base_runtime.loop_checkpoint_store),
            thread_service: Arc::clone(&base_runtime.thread_service),
            resource_governor: Arc::clone(&base_runtime.resource_governor),
            budget_event_sink: Arc::clone(&base_runtime.budget_event_sink),
            in_memory_budget_event_sink: Arc::clone(&base_runtime.in_memory_budget_event_sink),
            broadcast_budget_event_sink: Arc::clone(&base_runtime.broadcast_budget_event_sink),
            budget_gate_store: Arc::clone(&base_runtime.budget_gate_store),
            skill_management: Arc::clone(&base_runtime.skill_management),
            extension_management: base_runtime.extension_management.clone(),
            runtime_http_egress: base_runtime.runtime_http_egress.clone(),
            host_runtime_http_egress: base_runtime.host_runtime_http_egress.clone(),
            skill_mounts: base_runtime.skill_mounts.clone(),
            memory_mounts: base_runtime.memory_mounts.clone(),
            skill_filesystem: Arc::clone(&base_runtime.skill_filesystem),
            workspace_filesystem: Arc::clone(&base_runtime.workspace_filesystem),
            #[cfg(feature = "slack-v2-host-beta")]
            host_state_filesystem: Arc::clone(&base_runtime.host_state_filesystem),
            #[cfg(feature = "libsql")]
            identity_filesystem: Arc::clone(&base_runtime.identity_filesystem),
            #[cfg(feature = "libsql")]
            identity_substrate_db: Arc::clone(&base_runtime.identity_substrate_db),
            subagent_goal_filesystem: Arc::new(ScopedFilesystem::with_fixed_view(
                Arc::new(failing_root),
                MountView::new(vec![MountGrant::new(
                    MountAlias::new("/conversations").expect("mount alias"),
                    VirtualPath::new("/conversations").expect("virtual path"),
                    MountPermissions::read_write_list_delete(),
                )])
                .expect("mount view"),
            )),
            extension_filesystem: Arc::clone(&base_runtime.extension_filesystem),
            workspace_mounts: base_runtime.workspace_mounts.clone(),
            local_dev_storage_root: base_runtime.local_dev_storage_root.clone(),
            default_system_prompt_path: base_runtime.default_system_prompt_path.clone(),
            event_log: Arc::clone(&base_runtime.event_log),
            audit_log: Arc::clone(&base_runtime.audit_log),
            extension_registry: Arc::clone(&base_runtime.extension_registry),
        })
    }

    #[cfg(any(feature = "libsql", feature = "postgres"))]
    #[tokio::test]
    async fn durable_trigger_conversation_services_propagates_init_error() {
        let runtime = local_runtime_with_failing_trigger_conversations().await;

        let error = match runtime.durable_trigger_conversation_services().await {
            Ok(_) => panic!("conversation service init should fail"),
            Err(error) => error,
        };

        assert!(matches!(
            error,
            ironclaw_conversations::InboundTurnError::DurableState { .. }
        ));
    }

    #[cfg(any(feature = "libsql", feature = "postgres"))]
    #[tokio::test]
    async fn local_runtime_trigger_create_hook_maps_conversation_init_error_to_backend() {
        let hook = LocalRuntimeTriggerCreatorPairingHook {
            runtime: local_runtime_with_failing_trigger_conversations().await,
        };
        let record = trigger_record_for_pairing_test();

        let error = hook
            .after_trigger_persisted(&record)
            .await
            .expect_err("conversation init failure should surface as trigger backend error");

        let TriggerError::Backend { reason } = error else {
            panic!("expected backend trigger error");
        };
        assert_eq!(reason, "trigger creator actor pairing failed");
    }

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
        assert!(
            services
                .local_runtime
                .as_ref()
                .expect("local runtime")
                .extension_management
                .is_some()
        );
        assert_eq!(services.readiness.state, RebornReadinessState::DevOnly);
    }

    #[tokio::test]
    async fn local_dev_memory_first_party_tools_use_mounted_memory_root() {
        let dir = tempfile::tempdir().expect("tempdir");
        let services = build_reborn_services(RebornBuildInput::local_dev(
            "local-dev-memory-owner",
            dir.path().join("local-dev"),
        ))
        .await
        .expect("local-dev services build");
        let runtime = services.host_runtime.expect("host runtime composed");

        invoke_json(
            runtime.as_ref(),
            MEMORY_WRITE_CAPABILITY_ID,
            memory_context(MEMORY_WRITE_CAPABILITY_ID),
            serde_json::json!({
                "target": "projects/alpha/notes.md",
                "content": "local dev mounted memory root search marker",
                "append": false
            }),
        )
        .await
        .expect("memory_write should use the mounted /memory root");

        let tree = invoke_json(
            runtime.as_ref(),
            MEMORY_TREE_CAPABILITY_ID,
            memory_context(MEMORY_TREE_CAPABILITY_ID),
            serde_json::json!({"path": "", "depth": 3}),
        )
        .await
        .expect("memory_tree should list the mounted /memory root");
        assert!(
            tree.to_string().contains("alpha/"),
            "memory_tree should include the written memory document: {tree}"
        );

        let search = invoke_json(
            runtime.as_ref(),
            MEMORY_SEARCH_CAPABILITY_ID,
            memory_context(MEMORY_SEARCH_CAPABILITY_ID),
            serde_json::json!({"query": "mounted memory root search marker", "limit": 5}),
        )
        .await
        .expect("memory_search should query the mounted /memory root");
        assert_eq!(search["result_count"], serde_json::json!(1));
        assert_eq!(
            search["results"][0]["path"],
            serde_json::json!("projects/alpha/notes.md")
        );
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn local_dev_memory_documents_persist_across_rebuilds() {
        let dir = tempfile::tempdir().expect("tempdir");
        let local_dev_root = dir.path().join("local-dev");
        let owner = "local-dev-durable-memory-owner";

        let services =
            build_reborn_services(RebornBuildInput::local_dev(owner, local_dev_root.clone()))
                .await
                .expect("first local-dev services build");
        let runtime = services
            .host_runtime
            .as_ref()
            .expect("host runtime composed");

        invoke_json(
            runtime.as_ref(),
            MEMORY_WRITE_CAPABILITY_ID,
            memory_context(MEMORY_WRITE_CAPABILITY_ID),
            serde_json::json!({
                "target": "projects/durable/notes.md",
                "content": "local dev durable mounted memory root search marker",
                "append": false
            }),
        )
        .await
        .expect("memory_write should persist through the libsql /memory root");
        drop(services);

        let rebuilt =
            build_reborn_services(RebornBuildInput::local_dev(owner, local_dev_root.clone()))
                .await
                .expect("rebuilt local-dev services");
        let runtime = rebuilt.host_runtime.as_ref().expect("rebuilt host runtime");

        let tree = invoke_json(
            runtime.as_ref(),
            MEMORY_TREE_CAPABILITY_ID,
            memory_context(MEMORY_TREE_CAPABILITY_ID),
            serde_json::json!({"path": "", "depth": 3}),
        )
        .await
        .expect("memory_tree should list rebuilt libsql memory documents");
        assert!(
            tree.to_string().contains("durable/"),
            "memory_tree should include the persisted memory document: {tree}"
        );

        let search = invoke_json(
            runtime.as_ref(),
            MEMORY_SEARCH_CAPABILITY_ID,
            memory_context(MEMORY_SEARCH_CAPABILITY_ID),
            serde_json::json!({"query": "durable mounted memory root search marker", "limit": 5}),
        )
        .await
        .expect("memory_search should query rebuilt libsql memory documents");
        assert_eq!(search["result_count"], serde_json::json!(1));
        assert_eq!(
            search["results"][0]["path"],
            serde_json::json!("projects/durable/notes.md")
        );
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn local_dev_default_product_auth_preserves_manual_token_across_rebuilds() {
        let dir = tempfile::tempdir().expect("tempdir");
        let local_dev_root = dir.path().join("local-dev");
        let owner = "local-dev-durable-auth-owner";
        let services =
            build_reborn_services(RebornBuildInput::local_dev(owner, local_dev_root.clone()))
                .await
                .expect("local-dev services build");
        let product_auth = services.product_auth.as_ref().expect("product auth");
        let scope = AuthProductScope::new(
            ResourceScope::local_default(UserId::new(owner).unwrap(), InvocationId::new()).unwrap(),
            AuthSurface::Callback,
        );
        let mut scope = scope;
        scope.resource.thread_id = Some(ironclaw_host_api::ThreadId::new("auth-thread").unwrap());

        let challenge = product_auth
            .request_manual_token_setup(crate::RebornManualTokenSetupRequest::new(
                scope.clone(),
                ironclaw_auth::AuthProviderId::new("github").unwrap(),
                CredentialAccountLabel::new("work github").unwrap(),
                ironclaw_auth::AuthContinuationRef::SetupOnly,
                chrono::Utc::now() + chrono::Duration::minutes(5),
            ))
            .await
            .unwrap();
        let submitted = product_auth
            .submit_manual_token(crate::RebornManualTokenSubmitRequest::new(
                scope.clone(),
                challenge.interaction_id,
                secrecy::SecretString::from("ghp_local_dev_pat"),
            ))
            .await
            .unwrap();

        let account = product_auth
            .credential_account_service()
            .get_account(ironclaw_auth::CredentialAccountLookupRequest::new(
                scope.clone(),
                submitted.account_id,
            ))
            .await
            .unwrap()
            .expect("manual-token submit should create account");
        let access_secret = account.access_secret.expect("manual token access secret");
        assert!(
            access_secret.as_str().starts_with("product-auth-manual-"),
            "local-dev default product-auth must create durable SecretStore-backed handles"
        );

        let rebuilt =
            build_reborn_services(RebornBuildInput::local_dev(owner, local_dev_root.clone()))
                .await
                .expect("local-dev services rebuild");
        let rebuilt_product_auth = rebuilt.product_auth.as_ref().expect("product auth");
        let rebuilt_account = rebuilt_product_auth
            .credential_account_service()
            .get_account(ironclaw_auth::CredentialAccountLookupRequest::new(
                scope.clone(),
                submitted.account_id,
            ))
            .await
            .unwrap()
            .expect("manual-token account should survive local-dev rebuild");
        assert_eq!(rebuilt_account.access_secret.as_ref(), Some(&access_secret));

        let rebuilt_filesystem = build_local_dev_root_filesystem(
            &local_dev_root,
            &local_dev_root.join("workspace"),
            None,
        )
        .await
        .expect("local-dev filesystem rebuild")
        .filesystem;
        let rebuilt_secret_store = build_local_dev_secret_store(
            &local_dev_root,
            local_dev_scoped_filesystem(rebuilt_filesystem),
        )
        .expect("local-dev secret store rebuild");
        let lease = rebuilt_secret_store
            .lease_once(&scope.resource, &access_secret)
            .await
            .expect("manual token secret should survive local-dev rebuild");
        let raw_secret = rebuilt_secret_store
            .consume(&scope.resource, lease.id)
            .await
            .expect("manual token secret should decrypt after local-dev rebuild");
        assert_eq!(raw_secret.expose_secret(), "ghp_local_dev_pat");

        let flows = product_auth
            .flow_record_source()
            .expect("local-dev product-auth flow source")
            .flows_for_owner(ironclaw_auth::AuthFlowOwnerScope {
                tenant_id: scope.resource.tenant_id.clone(),
                user_id: scope.resource.user_id.clone(),
                agent_id: scope.resource.agent_id.clone(),
                project_id: scope.resource.project_id.clone(),
                thread_id: scope.resource.thread_id.clone().unwrap(),
            })
            .await
            .unwrap();
        let completed_flow = flows
            .iter()
            .find(|flow| flow.credential_account_id == Some(submitted.account_id))
            .expect("manual-token completion should remain visible to auth gates");
        assert_eq!(
            completed_flow.status,
            ironclaw_auth::AuthFlowStatus::Completed
        );
    }

    /// Verify that `attach_hosted_mcp_runtime` is soft-disabled when the host
    /// runtime has no HTTP egress (e.g. in-memory-only test services). The
    /// function must not panic or return an error; it simply skips the MCP
    /// runtime attachment so the rest of the composition continues.
    #[test]
    fn attach_hosted_mcp_runtime_skips_services_without_http_egress() {
        let services = HostRuntimeServices::new(
            Arc::new(ExtensionRegistry::new()),
            Arc::new(LocalFilesystem::new()),
            Arc::new(InMemoryResourceGovernor::new()),
            Arc::new(GrantAuthorizer::new()),
            ProcessServices::in_memory(),
            CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        );
        // product_auth_provider_runtime_ports() is None without HTTP egress.
        assert!(services.product_auth_provider_runtime_ports().is_none());

        // attach_hosted_mcp_runtime must succeed (soft-skip) rather than error.
        let services = attach_hosted_mcp_runtime(services).expect("soft-disable must not error");

        // Runtime ports still absent — no egress was added by the attachment.
        assert!(services.product_auth_provider_runtime_ports().is_none());
    }

    #[tokio::test]
    async fn local_dev_gsuite_installs_activates_and_dispatches_through_host_runtime() {
        let dir = tempfile::tempdir().expect("tempdir");
        let services = build_reborn_services(RebornBuildInput::local_dev(
            "local-dev-gsuite-owner",
            dir.path().join("local-dev"),
        ))
        .await
        .expect("local-dev services build");
        let local_runtime = services.local_runtime.as_ref().expect("local runtime");
        let extension_management = local_runtime
            .extension_management
            .as_ref()
            .expect("extension management");
        let gmail_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "gmail").expect("valid ref");
        let calendar_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "google-calendar")
                .expect("valid ref");

        extension_management
            .install(gmail_ref.clone())
            .await
            .expect("install Gmail");
        extension_management
            .activate(gmail_ref, ExtensionActivationMode::Static)
            .await
            .expect("activate Gmail");
        extension_management
            .install(calendar_ref.clone())
            .await
            .expect("install Google Calendar");
        extension_management
            .activate(calendar_ref, ExtensionActivationMode::Static)
            .await
            .expect("activate Google Calendar");

        let gmail_context = gsuite_context("gmail.send_message");
        let auth_scope =
            AuthProductScope::new(gmail_context.resource_scope.clone(), AuthSurface::Api);
        services
            .product_auth
            .as_ref()
            .expect("product auth")
            .credential_account_service()
            .create_account(NewCredentialAccount {
                scope: auth_scope,
                provider: ironclaw_first_party_extensions::google_provider_id()
                    .expect("Google provider id"),
                label: CredentialAccountLabel::new("work google").expect("valid label"),
                status: CredentialAccountStatus::Configured,
                ownership: CredentialOwnership::UserReusable,
                owner_extension: None,
                granted_extensions: Vec::new(),
                access_secret: Some(SecretHandle::new("missing-google-access-token").unwrap()),
                refresh_secret: None,
                scopes: vec![
                    ProviderScope::new(GOOGLE_GMAIL_SEND_SCOPE).unwrap(),
                    ProviderScope::new(GOOGLE_CALENDAR_EVENTS_SCOPE).unwrap(),
                ],
            })
            .await
            .expect("create Google account");

        let outcome = services
            .host_runtime
            .as_ref()
            .expect("host runtime")
            .invoke_capability(RuntimeCapabilityRequest::new(
                gmail_context,
                CapabilityId::new("gmail.send_message").unwrap(),
                ResourceEstimate::default(),
                serde_json::json!({ "message": { "raw": "base64url-rfc822" } }),
                trust_decision(),
            ))
            .await
            .expect("runtime invocation completes");

        let RuntimeCapabilityOutcome::Failed(failure) = outcome else {
            panic!("expected fail-closed handler outcome, got {outcome:?}");
        };
        assert_eq!(failure.capability_id.as_str(), "gmail.send_message");
        assert_ne!(failure.kind, RuntimeFailureKind::Authorization);
        assert_ne!(failure.kind, RuntimeFailureKind::MissingRuntime);

        let calendar_context = gsuite_context("google-calendar.create_event");
        let outcome = services
            .host_runtime
            .as_ref()
            .expect("host runtime")
            .invoke_capability(RuntimeCapabilityRequest::new(
                calendar_context,
                CapabilityId::new("google-calendar.create_event").unwrap(),
                ResourceEstimate::default(),
                serde_json::json!({
                    "calendar_id": "primary",
                    "event": { "summary": "Review" }
                }),
                trust_decision(),
            ))
            .await
            .expect("runtime invocation completes");

        let RuntimeCapabilityOutcome::Failed(failure) = outcome else {
            panic!("expected fail-closed handler outcome, got {outcome:?}");
        };
        assert_eq!(
            failure.capability_id.as_str(),
            "google-calendar.create_event"
        );
        assert_ne!(failure.kind, RuntimeFailureKind::Authorization);
        assert_ne!(failure.kind, RuntimeFailureKind::MissingRuntime);
    }

    #[tokio::test]
    async fn local_dev_notion_mcp_installs_activates_and_reaches_auth_gate() {
        let dir = tempfile::tempdir().expect("tempdir");
        let services = build_reborn_services(RebornBuildInput::local_dev(
            "local-dev-notion-mcp-owner",
            dir.path().join("local-dev"),
        ))
        .await
        .expect("local-dev services build");
        let local_runtime = services.local_runtime.as_ref().expect("local runtime");
        let extension_management = local_runtime
            .extension_management
            .as_ref()
            .expect("extension management");
        let notion_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "notion").expect("valid ref");
        let catalog = AvailableExtensionCatalog::from_first_party_assets()
            .expect("first-party extensions load");
        let notion_package = catalog.resolve(&notion_ref).expect("Notion MCP is bundled");
        let capability_ids = notion_package
            .package
            .manifest
            .capabilities
            .iter()
            .map(|capability| capability.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(capability_ids.len(), 18);
        assert!(capability_ids.contains(&"notion.notion-create-pages"));
        assert!(capability_ids.contains(&"notion.notion-query-data-sources"));
        assert!(capability_ids.contains(&"notion.notion-create-comment"));
        assert!(capability_ids.contains(&"notion.notion-get-self"));

        extension_management
            .install(notion_ref.clone())
            .await
            .expect("install Notion MCP");
        extension_management
            .activate(notion_ref, ExtensionActivationMode::Static)
            .await
            .expect("activate Notion MCP");

        let outcome = services
            .host_runtime
            .as_ref()
            .expect("host runtime")
            .invoke_capability(RuntimeCapabilityRequest::new(
                notion_mcp_context("notion.notion-search"),
                CapabilityId::new("notion.notion-search").unwrap(),
                ResourceEstimate::default(),
                serde_json::json!({ "query": "project notes" }),
                notion_mcp_trust_decision(),
            ))
            .await
            .expect("runtime invocation completes");

        let RuntimeCapabilityOutcome::AuthRequired(gate) = outcome else {
            panic!("expected missing Notion token to open auth gate, got {outcome:?}");
        };
        assert_eq!(gate.capability_id.as_str(), "notion.notion-search");
    }

    #[tokio::test]
    async fn local_dev_web_access_installs_activates_and_dispatches_through_host_runtime() {
        let dir = tempfile::tempdir().expect("tempdir");
        let services = build_reborn_services(RebornBuildInput::local_dev(
            "local-dev-web-access-owner",
            dir.path().join("local-dev"),
        ))
        .await
        .expect("local-dev services build");
        let local_runtime = services.local_runtime.as_ref().expect("local runtime");
        let extension_management = local_runtime
            .extension_management
            .as_ref()
            .expect("extension management");
        let web_access_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "web-access")
                .expect("valid ref");

        extension_management
            .install(web_access_ref.clone())
            .await
            .expect("install Web Access");
        extension_management
            .activate(web_access_ref, ExtensionActivationMode::Static)
            .await
            .expect("activate Web Access");

        let outcome = services
            .host_runtime
            .as_ref()
            .expect("host runtime")
            .invoke_capability(RuntimeCapabilityRequest::new(
                web_access_context("web-access.search"),
                CapabilityId::new("web-access.search").unwrap(),
                ResourceEstimate::default(),
                serde_json::json!({
                    "provider": "brave",
                    "query": "ironclaw reborn"
                }),
                trust_decision(),
            ))
            .await
            .expect("runtime invocation completes");

        let RuntimeCapabilityOutcome::Failed(failure) = outcome else {
            panic!("expected fail-closed handler outcome, got {outcome:?}");
        };
        assert_eq!(failure.capability_id.as_str(), "web-access.search");
        assert_eq!(failure.kind, RuntimeFailureKind::Backend);
    }

    #[tokio::test]
    async fn local_dev_nearai_mcp_installs_and_activates_model_visible_capability() {
        let dir = tempfile::tempdir().expect("tempdir");
        let services = build_reborn_services(RebornBuildInput::local_dev(
            "local-dev-nearai-mcp-owner",
            dir.path().join("local-dev"),
        ))
        .await
        .expect("local-dev services build");
        let local_runtime = services.local_runtime.as_ref().expect("local runtime");
        let extension_management = local_runtime
            .extension_management
            .as_ref()
            .expect("extension management");
        let nearai_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "nearai").expect("valid ref");

        extension_management
            .install(nearai_ref.clone())
            .await
            .expect("install NEAR AI MCP");
        extension_management
            .activate(nearai_ref, ExtensionActivationMode::Static)
            .await
            .expect("activate NEAR AI MCP");

        let capabilities = extension_management
            .active_model_visible_capabilities()
            .await
            .expect("active capabilities");
        let search = capabilities
            .iter()
            .find(|capability| capability.id.as_str() == "nearai.search")
            .expect("nearai.search active");

        assert_eq!(search.provider.as_str(), "nearai");
        assert_eq!(search.effects, nearai_allowed_effects());
        assert_eq!(search.runtime_credentials.len(), 1);
        assert_eq!(
            search.runtime_credentials[0].handle,
            SecretHandle::new("llm_nearai_api_key").unwrap()
        );
        // NEAR AI MCP credential is sourced from a product-auth account so that the
        // user-facing setup flow is the manual-token product-auth surface (shared
        // with GitHub WASM), not an out-of-band SecretStore handle drop.
        // The 'handle' field remains the staging slot name the MCP egress planner
        // reads from RuntimeSecretInjectionStore after the obligation handler resolves
        // the access secret via RuntimeCredentialAccountResolver.
        assert_eq!(
            search.runtime_credentials[0].source,
            RuntimeCredentialRequirementSource::ProductAuthAccount {
                provider: RuntimeCredentialAccountProviderId::new("nearai").unwrap(),
                setup: Default::default(),
            }
        );
        assert_eq!(
            search.runtime_credentials[0].audience.host_pattern,
            "private.near.ai"
        );
    }

    #[test]
    fn attach_hosted_mcp_runtime_skips_services_without_runtime_http_egress() {
        let services = HostRuntimeServices::new(
            Arc::new(ExtensionRegistry::new()),
            Arc::new(LocalFilesystem::new()),
            Arc::new(InMemoryResourceGovernor::new()),
            Arc::new(GrantAuthorizer::new()),
            ProcessServices::in_memory(),
            CapabilitySurfaceVersion::new("surface-v1").unwrap(),
        );

        let services = attach_hosted_mcp_runtime(services).expect("attach is optional");

        assert!(services.product_auth_provider_runtime_ports().is_none());
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
                .join("tenants/default/users/local-dev-test-user/skills/runtime-sentinel/SKILL.md")
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
                .join("tenants/default/users/local-dev-test-user/skills/runtime-sentinel/SKILL.md")
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
        assert!(
            !storage_root
                .join("tenants/default/users/local-dev-test-user/skills/blocked/SKILL.md")
                .exists()
        );
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
    fn local_dev_legacy_skill_backfill_marker_preserves_deletions() {
        let dir = tempfile::tempdir().expect("tempdir");
        let storage_root = dir.path().join("local-dev");
        let legacy_skill_dir = storage_root.join("skills/legacy-skill");
        std::fs::create_dir_all(&legacy_skill_dir).expect("legacy skill dir");
        std::fs::write(legacy_skill_dir.join("SKILL.md"), "legacy skill").expect("legacy skill");
        let owner_user_id = UserId::new("owner").expect("owner");

        backfill_local_dev_legacy_user_skills(&storage_root, &owner_user_id)
            .expect("initial backfill");
        let scoped_skill_dir = storage_root.join("tenants/default/users/owner/skills/legacy-skill");
        let reborn_cli_skill_dir =
            storage_root.join("tenants/reborn-cli/users/owner/skills/legacy-skill");
        assert!(scoped_skill_dir.join("SKILL.md").exists());
        assert!(reborn_cli_skill_dir.join("SKILL.md").exists());

        std::fs::remove_dir_all(&scoped_skill_dir).expect("delete migrated skill");
        backfill_local_dev_legacy_user_skills(&storage_root, &owner_user_id)
            .expect("second backfill");
        assert!(
            !scoped_skill_dir.exists(),
            "one-time legacy backfill must not resurrect user-deleted migrated skills"
        );
    }

    #[cfg(unix)]
    #[test]
    fn local_dev_legacy_skill_backfill_skips_symlinks() {
        let dir = tempfile::tempdir().expect("tempdir");
        let storage_root = dir.path().join("local-dev");
        let legacy_root = storage_root.join("skills");
        let target_dir = storage_root.join("target-skill");
        std::fs::create_dir_all(&legacy_root).expect("legacy root");
        std::fs::create_dir_all(&target_dir).expect("target dir");
        std::os::unix::fs::symlink(&target_dir, legacy_root.join("linked-skill"))
            .expect("legacy symlink");
        let owner_user_id = UserId::new("owner").expect("owner");

        backfill_local_dev_legacy_user_skills(&storage_root, &owner_user_id)
            .expect("symlink should be skipped, not fail startup");
        assert!(
            !storage_root
                .join("tenants/default/users/owner/skills/linked-skill")
                .exists()
        );
        assert!(
            storage_root
                .join(format!(
                    "tenants/default/users/owner/skills/{LOCAL_DEV_LEGACY_SKILLS_BACKFILL_MARKER}"
                ))
                .exists(),
            "migration should still be marked complete after skipping symlinks"
        );
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
        assert!(!ids.contains(&SKILL_ACTIVATE_CAPABILITY_ID));
        assert!(ids.contains(&SKILL_INSTALL_CAPABILITY_ID));
        assert!(ids.contains(&SKILL_REMOVE_CAPABILITY_ID));
        assert!(ids.contains(&TRIGGER_CREATE_CAPABILITY_ID));
        assert!(ids.contains(&TRIGGER_LIST_CAPABILITY_ID));
        assert!(ids.contains(&TRIGGER_REMOVE_CAPABILITY_ID));

        let registry = ironclaw_host_runtime::builtin_first_party_handlers(Arc::new(
            ironclaw_triggers::InMemoryTriggerRepository::default(),
        ))
        .expect("built-in handlers build");
        for id in [
            SKILL_LIST_CAPABILITY_ID,
            SKILL_INSTALL_CAPABILITY_ID,
            SKILL_REMOVE_CAPABILITY_ID,
            TRIGGER_CREATE_CAPABILITY_ID,
            TRIGGER_LIST_CAPABILITY_ID,
            TRIGGER_REMOVE_CAPABILITY_ID,
        ] {
            assert!(registry.contains_handler(&ironclaw_host_api::CapabilityId::new(id).unwrap()));
        }
        assert!(!registry.contains_handler(
            &ironclaw_host_api::CapabilityId::new(SKILL_ACTIVATE_CAPABILITY_ID).unwrap()
        ));
    }

    #[cfg(any(feature = "libsql", feature = "postgres"))]
    #[test]
    fn production_skill_management_mounts_use_production_namespace() {
        let scope = ResourceScope {
            tenant_id: TenantId::new("tenant-alpha").expect("tenant"),
            user_id: UserId::new("alice").expect("user"),
            agent_id: None,
            project_id: None,
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        };

        let mounts = production_skill_management_mount_view(&scope).expect("mount view");
        let skills_mount = mounts
            .mounts
            .iter()
            .find(|mount| mount.alias.as_str() == "/skills")
            .expect("skills mount");
        assert_eq!(
            skills_mount.target.as_str(),
            "/tenants/tenant-alpha/users/alice/skills"
        );
        let system_mount = mounts
            .mounts
            .iter()
            .find(|mount| mount.alias.as_str() == "/system/skills")
            .expect("system skills mount");
        assert_eq!(system_mount.target.as_str(), "/system/skills");
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

    fn memory_context(capability_id: &str) -> ExecutionContext {
        execution_context(
            capability_id,
            memory_mount_view(MountPermissions::read_write_list_delete())
                .expect("valid memory mounts"),
        )
    }

    fn gsuite_context(capability_id: &str) -> ExecutionContext {
        let extension_id = ExtensionId::new("caller").expect("valid extension id");
        ExecutionContext::local_default(
            UserId::new("local-dev-test-user").expect("valid user id"),
            extension_id.clone(),
            RuntimeKind::FirstParty,
            TrustClass::FirstParty,
            CapabilitySet {
                grants: vec![CapabilityGrant {
                    id: CapabilityGrantId::new(),
                    capability: CapabilityId::new(capability_id).expect("valid capability id"),
                    grantee: Principal::Extension(extension_id),
                    issued_by: Principal::HostRuntime,
                    constraints: GrantConstraints {
                        allowed_effects: gsuite_allowed_effects(),
                        mounts: MountView::new(Vec::new()).expect("valid empty mount view"),
                        network: NetworkPolicy::default(),
                        secrets: vec![SecretHandle::new("missing-google-access-token").unwrap()],
                        resource_ceiling: None,
                        expires_at: None,
                        max_invocations: None,
                    },
                }],
            },
            MountView::new(Vec::new()).expect("valid empty mount view"),
        )
        .expect("valid execution context")
    }

    fn notion_mcp_context(capability_id: &str) -> ExecutionContext {
        let extension_id = ExtensionId::new("caller").expect("valid extension id");
        ExecutionContext::local_default(
            UserId::new("local-dev-test-user").expect("valid user id"),
            extension_id.clone(),
            RuntimeKind::Mcp,
            TrustClass::Sandbox,
            CapabilitySet {
                grants: vec![CapabilityGrant {
                    id: CapabilityGrantId::new(),
                    capability: CapabilityId::new(capability_id).expect("valid capability id"),
                    grantee: Principal::Extension(extension_id),
                    issued_by: Principal::HostRuntime,
                    constraints: GrantConstraints {
                        allowed_effects: notion_mcp_allowed_effects(),
                        mounts: MountView::new(Vec::new()).expect("valid empty mount view"),
                        network: notion_mcp_network_policy(),
                        secrets: vec![SecretHandle::new("mcp_notion_access_token").unwrap()],
                        resource_ceiling: None,
                        expires_at: None,
                        max_invocations: None,
                    },
                }],
            },
            MountView::new(Vec::new()).expect("valid empty mount view"),
        )
        .expect("valid execution context")
    }

    fn web_access_context(capability_id: &str) -> ExecutionContext {
        let extension_id = ExtensionId::new("caller").expect("valid extension id");
        ExecutionContext::local_default(
            UserId::new("local-dev-test-user").expect("valid user id"),
            extension_id.clone(),
            RuntimeKind::FirstParty,
            TrustClass::FirstParty,
            CapabilitySet {
                grants: vec![CapabilityGrant {
                    id: CapabilityGrantId::new(),
                    capability: CapabilityId::new(capability_id).expect("valid capability id"),
                    grantee: Principal::Extension(extension_id),
                    issued_by: Principal::HostRuntime,
                    constraints: GrantConstraints {
                        allowed_effects: web_access_allowed_effects(),
                        mounts: MountView::new(Vec::new()).expect("valid empty mount view"),
                        network: web_access_network_policy(),
                        secrets: Vec::new(),
                        resource_ceiling: None,
                        expires_at: None,
                        max_invocations: None,
                    },
                }],
            },
            MountView::new(Vec::new()).expect("valid empty mount view"),
        )
        .expect("valid execution context")
    }

    fn web_access_network_policy() -> NetworkPolicy {
        NetworkPolicy {
            allowed_targets: vec![NetworkTargetPattern {
                scheme: Some(ironclaw_host_api::NetworkScheme::Https),
                host_pattern: "mcp.exa.ai".to_string(),
                port: None,
            }],
            deny_private_ip_ranges: true,
            max_egress_bytes: None,
        }
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
                network: network_policy(),
                secrets: Vec::new(),
                resource_ceiling: None,
                expires_at: None,
                max_invocations: None,
            },
        }
    }

    fn skill_mounts() -> MountView {
        let scope = ironclaw_host_api::ResourceScope::local_default(
            UserId::new("local-dev-test-user").expect("valid user id"),
            ironclaw_host_api::InvocationId::new(),
        )
        .expect("valid resource scope");
        crate::local_dev_mounts::scoped_skill_management_mount_view(&scope)
            .expect("valid skill mounts")
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
            EffectKind::DeleteFilesystem,
            EffectKind::Network,
        ]
    }

    fn network_policy() -> NetworkPolicy {
        NetworkPolicy {
            allowed_targets: vec![NetworkTargetPattern {
                scheme: None,
                host_pattern: "*".to_string(),
                port: None,
            }],
            deny_private_ip_ranges: true,
            max_egress_bytes: None,
        }
    }

    fn notion_mcp_network_policy() -> NetworkPolicy {
        NetworkPolicy {
            allowed_targets: vec![NetworkTargetPattern {
                scheme: Some(NetworkScheme::Https),
                host_pattern: "mcp.notion.com".to_string(),
                port: None,
            }],
            deny_private_ip_ranges: true,
            max_egress_bytes: None,
        }
    }

    fn notion_mcp_allowed_effects() -> Vec<EffectKind> {
        vec![
            EffectKind::DispatchCapability,
            EffectKind::Network,
            EffectKind::UseSecret,
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

    fn notion_mcp_trust_decision() -> TrustDecision {
        TrustDecision {
            effective_trust: EffectiveTrustClass::user_trusted(),
            authority_ceiling: AuthorityCeiling {
                allowed_effects: notion_mcp_allowed_effects(),
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
