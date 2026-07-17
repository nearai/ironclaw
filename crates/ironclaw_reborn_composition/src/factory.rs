// arch-exempt: large_file, needs Reborn composition helper extraction, plan #4469
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
    sync::{Arc, OnceLock},
};

#[cfg(any(feature = "libsql", feature = "postgres"))]
use crate::product_auth::durable::{FilesystemAuthProductServices, UnavailableAuthProviderClient};
use crate::support::fs::RebornProjectService;
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_approvals::{
    FilesystemAutoApproveSettingStore, FilesystemPersistentApprovalPolicyStore,
    FilesystemToolPermissionOverrideStore,
};
#[cfg(not(any(feature = "libsql", feature = "postgres")))]
use ironclaw_approvals::{
    InMemoryAutoApproveSettingStore, InMemoryPersistentApprovalPolicyStore,
    InMemoryToolPermissionOverrideStore,
};
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_auth::AuthProviderClient;
use ironclaw_auth::{AuthProductScope, AuthSurface};
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_authorization::FilesystemCapabilityLeaseStore;
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_authorization::GrantAuthorizer;
#[cfg(not(any(feature = "libsql", feature = "postgres")))]
use ironclaw_authorization::InMemoryCapabilityLeaseStore;
#[cfg(not(any(feature = "libsql", feature = "postgres")))]
use ironclaw_conversations::InMemoryConversationServices;
use ironclaw_conversations::{
    AdapterInstallationId, AdapterKind, ConversationActorPairingService, ExternalActorRef,
};
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_conversations::{InboundTurnError, RebornFilesystemConversationServices};
use ironclaw_events::{DurableAuditLog, DurableEventLog};
#[cfg(not(any(feature = "libsql", feature = "postgres")))]
use ironclaw_events::{InMemoryDurableAuditLog, InMemoryDurableEventLog};
use ironclaw_extensions::{
    ExtensionInstallationStore, ExtensionLifecycleService, ExtensionRegistry,
    SharedExtensionRegistry,
};
#[cfg(not(feature = "libsql"))]
use ironclaw_filesystem::InMemoryBackend;
#[cfg(feature = "libsql")]
use ironclaw_filesystem::LibSqlRootFilesystem;
#[cfg(feature = "postgres")]
use ironclaw_filesystem::PostgresRootFilesystem;
use ironclaw_filesystem::{
    BackendCapabilities, BackendId, BackendKind, CompositeRootFilesystem, ContentKind, IndexPolicy,
    MountDescriptor, RootFilesystem, StorageClass,
};
use ironclaw_filesystem::{LocalFilesystem, ScopedFilesystem};
#[cfg(feature = "test-support")]
use ironclaw_first_party_extensions::{
    EXA_MCP_HOST, NETWORK_EGRESS_LIMIT, WEB_ACCESS_EXTENSION_ID, WEB_GET_CONTENT_CAPABILITY_ID,
    WEB_SEARCH_CAPABILITY_ID, gsuite_network_policy_for,
};
use ironclaw_host_api::runtime_policy::{
    EffectiveRuntimePolicy, FilesystemBackendKind, ProcessBackendKind, SecretMode,
};
#[cfg(feature = "test-support")]
use ironclaw_host_api::{
    CapabilityGrant, CapabilityGrantId, GrantConstraints, NetworkPolicy, NetworkTargetPattern,
    Principal,
};
use ironclaw_host_api::{
    EffectKind, ExtensionId, HostPath, InvocationId, MountPermissions, MountView, PackageId,
    ResourceScope, RuntimeHttpEgress, UserId, VirtualPath,
};
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_host_api::{HostApiError, MountAlias, MountGrant};
use ironclaw_host_runtime::{
    CapabilitySurfaceVersion, FirstPartyCapabilityRegistry, HostRuntimeHttpEgressPort,
    HostRuntimeServices, LocalHostProcessPort, ProductAuthProviderRuntimePorts, TriggerCreateHook,
    builtin_first_party_handlers_with_trigger_create_hook, builtin_first_party_package,
};
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_host_runtime::{
    builtin_first_party_handlers_with_trigger_create_hook_for_process_backend,
    builtin_first_party_package_for_process_backend,
};
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_loop_host::FilesystemCheckpointStateStore;
use ironclaw_outbound::CommunicationPreferenceRepository;
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_outbound::FilesystemOutboundStateStore;
#[cfg(not(any(feature = "libsql", feature = "postgres")))]
use ironclaw_outbound::InMemoryOutboundStateStore;
#[cfg(feature = "slack-v2-host-beta")]
use ironclaw_outbound::{DeliveredGateRouteStore, OutboundStateStore, TriggeredRunDeliveryStore};
#[cfg(all(
    not(any(feature = "libsql", feature = "postgres")),
    feature = "slack-v2-host-beta"
))]
use ironclaw_outbound::{InMemoryDeliveredGateRouteStore, InMemoryTriggeredRunDeliveryStore};
use ironclaw_processes::ProcessServices;
#[cfg(any(feature = "slack-v2-host-beta", test))]
use ironclaw_product_workflow::ChannelConnectionFacade;
use ironclaw_product_workflow::{
    LifecycleProductSurfaceContext, ProductAuthTurnGateResumeDispatcher, ProjectService,
};
use ironclaw_projects::ProjectRepository;
use ironclaw_resources::InMemoryResourceGovernor;
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_resources::{
    BroadcastBudgetEventSink, BudgetGateStore, FilesystemBudgetGateStore,
    FilesystemResourceGovernor, ResourceGovernor,
};
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_run_state::{FilesystemApprovalRequestStore, FilesystemRunStateStore};
#[cfg(not(any(feature = "libsql", feature = "postgres")))]
use ironclaw_run_state::{InMemoryApprovalRequestStore, InMemoryRunStateStore};
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_secrets::FilesystemCredentialBroker;
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_secrets::FilesystemSecretStore;
use ironclaw_secrets::SecretStore;
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_threads::FilesystemSessionThreadService;
#[cfg(not(any(feature = "libsql", feature = "postgres")))]
use ironclaw_threads::InMemorySessionThreadService;
use ironclaw_threads::SessionThreadService;
use ironclaw_triggers::{
    TRIGGER_TRUSTED_ADAPTER_INSTALLATION_ID, TRIGGER_TRUSTED_ADAPTER_KIND,
    TRIGGER_TRUSTED_EXTERNAL_ACTOR_NAMESPACE, TriggerActiveRunLookup, TriggerError, TriggerRecord,
    TriggerRepository,
};
use ironclaw_trust::{AdminConfig, AdminEntry, HostTrustAssignment, HostTrustPolicy};
#[cfg(feature = "test-support")]
use ironclaw_trust::{AuthorityCeiling, EffectiveTrustClass, TrustDecision, TrustProvenance};
#[cfg(all(
    feature = "inmemory-turn-state",
    any(feature = "libsql", feature = "postgres")
))]
use ironclaw_turns::FilesystemTurnStateBlockPersistence;
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_turns::FilesystemTurnStateStoreKind;
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_turns::InMemoryRunProfileResolver;
#[cfg(any(
    feature = "inmemory-turn-state",
    not(any(feature = "libsql", feature = "postgres"))
))]
use ironclaw_turns::InMemoryTurnStateStore;
use ironclaw_turns::{
    CheckpointStateStore, DefaultTurnCoordinator, ExternalToolCatalog, InMemoryExternalToolCatalog,
    LoopCheckpointStore,
};
#[cfg(not(any(feature = "libsql", feature = "postgres")))]
use ironclaw_turns::{InMemoryCheckpointStateStore, InMemoryLoopCheckpointStore};

use crate::RebornProductAuthServicePorts;
#[cfg(feature = "slack-v2-host-beta")]
use crate::extension_host::available_extensions::{
    slack_bot_manifest_digest, slack_manifest_digest,
};
#[cfg(feature = "slack-v2-host-beta")]
use crate::extension_host::extension_removal_cleanup::SlackPersonalConnectionCleanupAdapter;
use crate::extension_host::lifecycle::{
    RebornLocalLifecycleFacade, RebornLocalSkillManagementPort, build_local_skill_management_port,
};
use crate::extension_host::mcp::hosted_http_mcp_runtime;
use crate::extension_host::{
    available_extensions::{
        AvailableExtensionCatalog, gmail_manifest_digest, google_calendar_manifest_digest,
        google_docs_manifest_digest, google_drive_manifest_digest, google_sheets_manifest_digest,
        google_slides_manifest_digest, notion_mcp_manifest_digest, web_access_manifest_digest,
    },
    extension_installation_store::FilesystemExtensionInstallationStore,
    extension_lifecycle::{
        ActiveExtensionPublisher, ExtensionCredentialCleanup, RebornLocalExtensionManagementPort,
        restore_extension_lifecycle_state,
    },
    extension_lifecycle_capabilities::{
        extend_builtin_first_party_package, insert_handlers as insert_extension_lifecycle_handlers,
    },
    extension_removal_cleanup::{ExtensionRemovalCleanupAdapter, ExtensionRemovalCleanupRegistry},
    gsuite::{
        ProductAuthRuntimeGsuiteCredentialStager, register_bundled_gsuite_first_party_handlers,
    },
};
use crate::input::{RebornLocalRuntimeIdentity, RebornRuntimeProcessBinding, RebornStorageInput};
use crate::lifecycle_auth_continuation::{
    LifecycleAuthContinuationDispatcher, LifecycleProductFacadeSlot,
};
use crate::local_dev_authorization::{StoreApprovalSettingsProvider, local_dev_authorizer};
use crate::local_dev_capability_policy::{LocalDevCapabilityPolicy, local_dev_capability_policy};
use crate::local_dev_mounts::{
    ambient_workspace_mount_view, memory_mount_view, scoped_skill_context_mount_view,
    skill_management_mount_view, system_extensions_lifecycle_mount_view, workspace_mount_view,
};
use crate::product_auth::credentials::product_auth_providers::{
    OAuthProviderComposition, compose_provider_client,
};
use crate::product_auth::credentials::runtime_credentials::ProductAuthRuntimeCredentialResolver;
use crate::root::default_system_prompt::seed_default_system_prompt;
use crate::runtime_input::RebornRuntimeIdentity;
use crate::web_access::register_bundled_web_access_first_party_handlers;
use crate::{
    RebornAuthContinuationDispatcher, RebornBuildError, RebornBuildInput, RebornCompositionProfile,
    RebornFacadeReadiness, RebornProductAuthServices, RebornReadiness, RebornWorkerReadiness,
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
    durable_backend: LocalDevDurableBackend,
}

// `pub(crate)` to match `build_default_local_dev_database_roots` (also
// `pub(crate)` for the `test_support` accessor): a `pub(crate)` fn returning a
// private enum trips `private_interfaces`. The enum stays crate-internal.
pub(crate) enum LocalDevDurableBackend {
    #[cfg(feature = "libsql")]
    LibSql(Arc<libsql::Database>),
    #[cfg(feature = "postgres")]
    Postgres(deadpool_postgres::Pool),
    #[cfg(not(feature = "libsql"))]
    Ephemeral,
}

enum LocalDevStorageBackendInput {
    LocalDefault,
    #[cfg(feature = "postgres")]
    Postgres(deadpool_postgres::Pool),
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

#[cfg(any(test, feature = "test-support"))]
#[derive(Clone)]
struct TestNetworkHttpEgress(Arc<dyn ironclaw_network::NetworkHttpEgress>);

#[cfg(any(test, feature = "test-support"))]
#[async_trait::async_trait]
impl ironclaw_network::NetworkHttpEgress for TestNetworkHttpEgress {
    async fn execute(
        &self,
        request: ironclaw_network::NetworkHttpRequest,
    ) -> Result<ironclaw_network::NetworkHttpResponse, ironclaw_network::NetworkHttpError> {
        self.0.execute(request).await
    }
}

// The in-memory turn-state authority wins whenever `inmemory-turn-state` is on
// (the runtime-wedge fix), and is also the only option in pure-memory builds.
// Otherwise the durable per-user filesystem row store is used.
#[cfg(all(
    not(feature = "inmemory-turn-state"),
    any(feature = "libsql", feature = "postgres")
))]
pub(crate) type LocalDevTurnStateStore = FilesystemTurnStateStoreKind<LocalDevRootFilesystem>;
#[cfg(any(
    feature = "inmemory-turn-state",
    not(any(feature = "libsql", feature = "postgres"))
))]
pub(crate) type LocalDevTurnStateStore = InMemoryTurnStateStore;

#[cfg(any(feature = "libsql", feature = "postgres"))]
type LocalDevResourceGovernor = FilesystemResourceGovernor<LocalDevRootFilesystem>;
#[cfg(not(any(feature = "libsql", feature = "postgres")))]
type LocalDevResourceGovernor = InMemoryResourceGovernor;

#[cfg(any(feature = "libsql", feature = "postgres"))]
type LocalDevRunStateStore = FilesystemRunStateStore<LocalDevRootFilesystem>;
#[cfg(not(any(feature = "libsql", feature = "postgres")))]
type LocalDevRunStateStore = InMemoryRunStateStore;

#[cfg(any(feature = "libsql", feature = "postgres"))]
pub(crate) type LocalDevApprovalRequestStore =
    FilesystemApprovalRequestStore<LocalDevRootFilesystem>;
#[cfg(not(any(feature = "libsql", feature = "postgres")))]
pub(crate) type LocalDevApprovalRequestStore = InMemoryApprovalRequestStore;

#[cfg(any(feature = "libsql", feature = "postgres"))]
pub(crate) type LocalDevCapabilityLeaseStore =
    FilesystemCapabilityLeaseStore<LocalDevRootFilesystem>;
#[cfg(not(any(feature = "libsql", feature = "postgres")))]
pub(crate) type LocalDevCapabilityLeaseStore = InMemoryCapabilityLeaseStore;

#[cfg(any(feature = "libsql", feature = "postgres"))]
pub(crate) type LocalDevPersistentApprovalPolicyStore =
    FilesystemPersistentApprovalPolicyStore<LocalDevRootFilesystem>;
#[cfg(not(any(feature = "libsql", feature = "postgres")))]
pub(crate) type LocalDevPersistentApprovalPolicyStore = InMemoryPersistentApprovalPolicyStore;

#[cfg(any(feature = "libsql", feature = "postgres"))]
pub(crate) type LocalDevToolPermissionOverrideStore =
    FilesystemToolPermissionOverrideStore<LocalDevRootFilesystem>;
#[cfg(not(any(feature = "libsql", feature = "postgres")))]
pub(crate) type LocalDevToolPermissionOverrideStore = InMemoryToolPermissionOverrideStore;

#[cfg(any(feature = "libsql", feature = "postgres"))]
pub(crate) type LocalDevAutoApproveSettingStore =
    FilesystemAutoApproveSettingStore<LocalDevRootFilesystem>;
#[cfg(not(any(feature = "libsql", feature = "postgres")))]
pub(crate) type LocalDevAutoApproveSettingStore = InMemoryAutoApproveSettingStore;

#[cfg(any(feature = "libsql", feature = "postgres"))]
type LocalDevProcessServices = ProcessServices<
    ironclaw_processes::FilesystemProcessStore<LocalDevRootFilesystem>,
    ironclaw_processes::FilesystemProcessResultStore<LocalDevRootFilesystem>,
>;
#[cfg(not(any(feature = "libsql", feature = "postgres")))]
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
    #[cfg(any(feature = "libsql", feature = "postgres"))]
    // arch-exempt: optional_arc, local-dev vs production split pending RebornServices split, plan #4471
    pub(crate) production_runtime: Option<RebornProductionRuntimeServices>,
    /// Pre-minted scheduler wake wiring for the production composition path.
    /// Minted in `build_production_shaped` so the notifier can satisfy
    /// `HostRuntimeServices.with_turn_run_wake_notifier_dyn` before
    /// `build_default_planned_runtime` runs; consumed by `build_reborn_runtime`
    /// via `DefaultPlannedRuntimeParts.scheduler_wake_wiring` so the scheduler
    /// loop driven by that function shares the exact same channel.
    #[cfg(any(feature = "libsql", feature = "postgres"))]
    pub(crate) production_scheduler_wake: Option<ironclaw_runner::runtime::SchedulerWakeWiring>,
    /// Shared scoped secret store. Exposed so runtime-level features (e.g.
    /// operator LLM-key storage) can reuse the same instance product-auth uses
    /// rather than standing up a second authority.
    #[cfg_attr(
        not(any(feature = "root-llm-provider", feature = "slack-v2-host-beta")),
        allow(dead_code)
    )]
    pub(crate) secret_store: Arc<dyn SecretStore>,
    #[cfg(any(test, feature = "test-support"))]
    #[allow(dead_code)]
    pub(crate) local_dev_wasm_runtime_credential_provider_captured: bool,
    /// Readiness of the background credential keepalive worker (B1). Carries the
    /// worker's dependencies together so "both deps present or neither" is a type
    /// invariant rather than a runtime check. MUST stay private — the worker is
    /// the only consumer; this field must never leak through any public facade.
    #[cfg(any(feature = "libsql", feature = "postgres"))]
    pub(crate) credential_refresh_worker: CredentialRefreshWorkerReady,
}

/// Whether the background credential keepalive worker can be started, with its
/// dependencies bundled so they cannot be partially wired.
///
/// The dependencies (cross-owner candidate enumeration + deployment-wide leader
/// lock + refresh port) are only ever produced together on the durable
/// production path. Bundling them into one `Ready` variant makes the
/// half-configured state — which would silently disable proactive refresh —
/// unrepresentable, so the runtime spawn site is a clean two-arm match with no
/// "enabled but deps missing" branch to forget about.
#[cfg(any(feature = "libsql", feature = "postgres"))]
pub(crate) enum CredentialRefreshWorkerReady {
    /// Deps fully wired (durable production path). The only state that can start
    /// the worker; the `enabled` policy flag still gates the actual spawn.
    Ready {
        candidate_source:
            Arc<dyn crate::product_auth::credentials::credential_refresh_worker::CredentialRefreshCandidateSource>,
        leader_lock: crate::product_auth::credentials::product_auth_refresh_lock::CredentialRefreshLeaderLock,
        refresh_port: Arc<RebornProductAuthServices>,
    },
    /// Deps intentionally absent: local-dev (single-user, no cross-owner
    /// enumeration), `disabled()`, or a caller-supplied `product_auth_ports`
    /// override/test path. The worker never starts.
    Absent,
}

impl RebornServices {
    /// The shared scoped secret store backing this composition.
    #[cfg_attr(
        not(any(feature = "root-llm-provider", feature = "slack-v2-host-beta")),
        allow(dead_code)
    )]
    pub(crate) fn secret_store(&self) -> Arc<dyn SecretStore> {
        Arc::clone(&self.secret_store)
    }

    /// Test-support access to the shared scoped secret store backing the
    /// composed runtime.
    #[cfg(feature = "test-support")]
    pub fn secret_store_for_test(&self) -> Arc<dyn SecretStore> {
        Arc::clone(&self.secret_store)
    }

    /// Read-write project-scoped workspace filesystem, built over
    /// `local_runtime.extension_filesystem` + `local_runtime.workspace_mounts`.
    /// `None` when no local runtime is composed.
    ///
    /// This deliberately does NOT reuse `local_runtime.workspace_filesystem`:
    /// that handle is intentionally read-only (it backs setup-marker reads —
    /// see `local_dev_setup_marker_workspace_filesystem_is_read_only`), so
    /// writing through it fails closed with `PermissionDenied`.
    ///
    /// Single owner of this recipe — both `RebornRuntime::webui_workspace_filesystem`
    /// (production attachment landing) and `local_dev_attachment_test_support_for_test`
    /// (C-ATTACH test seam) call this rather than each rebuilding the view, so the
    /// two can never drift apart.
    pub(crate) fn read_write_workspace_filesystem(
        &self,
    ) -> Option<Arc<ScopedFilesystem<LocalDevRootFilesystem>>> {
        let local_runtime = self.local_runtime.as_ref()?;
        Some(Arc::new(ScopedFilesystem::with_fixed_view(
            Arc::clone(&local_runtime.extension_filesystem),
            local_runtime.workspace_mounts.clone(),
        )))
    }

    #[cfg(feature = "test-support")]
    pub fn local_dev_approval_test_parts(&self) -> Option<RebornLocalDevApprovalTestParts> {
        let local_runtime = self.local_runtime.as_ref()?;
        let approval_requests: Arc<dyn ironclaw_run_state::ApprovalRequestStore> =
            local_runtime.approval_requests.clone();
        let capability_leases: Arc<dyn ironclaw_authorization::CapabilityLeaseStore> =
            local_runtime.capability_leases.clone();
        Some(RebornLocalDevApprovalTestParts {
            approval_requests,
            capability_leases,
        })
    }

    #[cfg(feature = "test-support")]
    pub fn local_dev_auto_approve_settings_for_test(
        &self,
    ) -> Option<Arc<dyn ironclaw_approvals::AutoApproveSettingStore>> {
        let local_runtime = self.local_runtime.as_ref()?;
        let auto_approve_settings: Arc<dyn ironclaw_approvals::AutoApproveSettingStore> =
            local_runtime.auto_approve_settings.clone();
        Some(auto_approve_settings)
    }

    /// Test-support access to the extension installation store for this
    /// composition. Returns `None` for production-profile compositions that did
    /// not wire up local-dev extension management.
    ///
    /// Mirrors the `installation_store` that `build_local_runtime` wires into
    /// `RebornLocalExtensionManagementPort`. For tests only — zero bytes
    /// shipped in production builds.
    #[cfg(feature = "test-support")]
    pub fn extension_installation_store_for_test(
        &self,
    ) -> Option<Arc<dyn ExtensionInstallationStore>> {
        self.local_runtime
            .as_ref()
            .and_then(|rt| rt.extension_management.as_ref())
            .map(|em| em.installation_store_for_test())
    }

    /// Test-support access to the local-dev memory filesystem that backs the
    /// user-profile source (E-PROFILE seam). This is the raw `RootFilesystem`
    /// that `MemoryBackedUserProfileSource` reads `context/profile.json` from and
    /// that the `profile_set` capability writes through, enabling a profile
    /// write→read-back round-trip at the integration tier. Returns `None` for
    /// production-profile compositions without a local-dev runtime.
    #[cfg(feature = "test-support")]
    pub fn local_dev_profile_filesystem_for_test(
        &self,
    ) -> Option<Arc<dyn ironclaw_filesystem::RootFilesystem>> {
        let local_runtime = self.local_runtime.as_ref()?;
        Some(Arc::clone(&local_runtime.extension_filesystem)
            as Arc<dyn ironclaw_filesystem::RootFilesystem>)
    }

    /// Test-support access to the local-dev project service backing the synthetic
    /// `project_create` capability (E-PROJ seam). Returns `None` for
    /// production-profile compositions without a local-dev runtime.
    #[cfg(feature = "test-support")]
    pub fn local_dev_project_service_for_test(&self) -> Option<Arc<dyn ProjectService>> {
        let local_runtime = self.local_runtime.as_ref()?;
        Some(Arc::clone(&local_runtime.project_service))
    }

    /// Test-support access to the local-dev session thread service (durable
    /// tool-result projection seam, issue #5838). This is the SAME `Arc`
    /// production's `capability_wiring` passes to
    /// `LocalDevCapabilityIo::new_with_durable_previews` and to the
    /// `result_read` synthetic capability, so a harness built over this
    /// `RebornServices` can drive its own real `LocalDevCapabilityIo` through
    /// `local_dev_capability_io_for_test`. Returns `None` for production-profile
    /// compositions without a local-dev runtime.
    #[cfg(feature = "test-support")]
    pub fn local_dev_thread_service_for_test(
        &self,
    ) -> Option<Arc<dyn ironclaw_threads::SessionThreadService>> {
        let local_runtime = self.local_runtime.as_ref()?;
        Some(Arc::clone(&local_runtime.thread_service))
    }

    /// Test-support access to the local-dev communication-preference repository
    /// (W6-COLD-SPOTS seam). This is the SAME `Arc` that `build_local_dev_store_graph`
    /// wires into `RebornLocalRuntimeServices::outbound_preferences` via
    /// `local_dev_outbound_store`, for tests only. Returns `None` for
    /// production-profile compositions without a local-dev runtime.
    #[cfg(feature = "test-support")]
    pub fn local_dev_outbound_preferences_for_test(
        &self,
    ) -> Option<Arc<dyn CommunicationPreferenceRepository>> {
        let local_runtime = self.local_runtime.as_ref()?;
        Some(Arc::clone(&local_runtime.outbound_preferences))
    }

    /// Test-support access to the on-disk local-dev storage root (W6-COLD-SPOTS
    /// seam), for tests only — mirrors the same `local_runtime.local_dev_storage_root`
    /// that `build_local_dev_store_graph` establishes in production. Used to reopen
    /// a fresh outbound-preferences store at the same root (see
    /// `open_local_dev_outbound_preferences_store_for_test`). Returns `None` for
    /// production-profile compositions without a local-dev runtime.
    #[cfg(feature = "test-support")]
    pub fn local_dev_storage_root_for_test(&self) -> Option<PathBuf> {
        let local_runtime = self.local_runtime.as_ref()?;
        Some(local_runtime.local_dev_storage_root.clone())
    }

    /// Single owner of the `ProjectScopedAttachmentReader` construction recipe
    /// over `local_runtime.workspace_filesystem` (mirrors the
    /// `read_write_workspace_filesystem` "single owner" pattern above). The
    /// concrete reader implements both `LoopAttachmentReadPort` and
    /// `InboundAttachmentReader`, so callers cast the same `Arc` into whichever
    /// trait object they need instead of re-deriving the recipe. Test-support
    /// only; zero bytes shipped in production builds.
    #[cfg(feature = "test-support")]
    fn local_dev_workspace_attachment_reader_for_test(
        &self,
    ) -> Option<Arc<crate::support::fs::ProjectScopedAttachmentReader<LocalDevRootFilesystem>>>
    {
        let local_runtime = self.local_runtime.as_ref()?;
        Some(Arc::new(
            crate::support::fs::ProjectScopedAttachmentReader::new(Arc::clone(
                &local_runtime.workspace_filesystem,
            )),
        ))
    }

    /// Test-support access to the attachment read port + inbound lander backing
    /// the C-ATTACH seam. The read port is built over `local_runtime.workspace_filesystem`,
    /// exactly like production's `attachment_read_port` (`runtime.rs` ~line 3328) —
    /// that handle is intentionally read-only (it backs setup-marker reads), which
    /// is fine for reading. The lander is built over the SAME read-write view
    /// `RebornRuntime::webui_workspace_filesystem` uses in production, via the
    /// shared [`Self::read_write_workspace_filesystem`] helper — landing through
    /// the read-only `workspace_filesystem` handle fails closed with
    /// `PermissionDenied`. Bundled into one accessor (rather than two, mirroring
    /// `local_dev_profile_filesystem_for_test` / `local_dev_project_service_for_test`
    /// above) because the two are always populated together. Returns `None` for
    /// production-profile compositions without a local-dev runtime.
    #[cfg(feature = "test-support")]
    pub fn local_dev_attachment_test_support_for_test(&self) -> Option<AttachmentTestSupport> {
        let read_port = self.local_dev_workspace_attachment_reader_for_test()?
            as Arc<dyn ironclaw_loop_host::LoopAttachmentReadPort>;
        let read_write_workspace_filesystem = self.read_write_workspace_filesystem()?;
        Some(AttachmentTestSupport {
            read_port,
            lander: Arc::new(crate::support::fs::ProjectScopedAttachmentLander::new(
                read_write_workspace_filesystem,
            )),
        })
    }

    /// Test-support access to the local-dev per-tool permission override store
    /// (C-SYNTH outbound seam). Backs `StoreApprovalSettingsProvider::tool_override`,
    /// which the synthetic `outbound_delivery_target_set` capability consults for
    /// its settings decision — a `Disabled` override drives the `policy_denied`
    /// route. Mirrors `local_dev_auto_approve_settings_for_test`; `None` for
    /// production-profile compositions without a local-dev runtime.
    #[cfg(feature = "test-support")]
    pub fn local_dev_tool_permission_overrides_for_test(
        &self,
    ) -> Option<Arc<dyn ironclaw_approvals::ToolPermissionOverrideStore>> {
        let local_runtime = self.local_runtime.as_ref()?;
        let overrides: Arc<dyn ironclaw_approvals::ToolPermissionOverrideStore> =
            local_runtime.tool_permission_overrides.clone();
        Some(overrides)
    }

    /// Test-support access to the local-dev persistent approval-policy store
    /// (C-SYNTH outbound seam). Backs `StoreApprovalSettingsProvider::tool_always_allow`.
    /// Mirrors `local_dev_auto_approve_settings_for_test`; `None` for
    /// production-profile compositions without a local-dev runtime.
    #[cfg(feature = "test-support")]
    pub fn local_dev_persistent_approval_policies_for_test(
        &self,
    ) -> Option<Arc<dyn ironclaw_approvals::PersistentApprovalPolicyStore>> {
        let local_runtime = self.local_runtime.as_ref()?;
        let policies: Arc<dyn ironclaw_approvals::PersistentApprovalPolicyStore> =
            local_runtime.persistent_approval_policies.clone();
        Some(policies)
    }

    /// SAME live trigger repository `local_dev_trigger_repository` builds and
    /// capability dispatch uses (the `trigger_repository` binding in
    /// `build_local_runtime`, above) — not a fresh reopen. Contrast
    /// [`open_local_dev_trigger_repository_for_test`] (independent reopened
    /// repo, for persistence/reopen tests). Backs the cold-LIST scenario
    /// (W5-WEBUI-API-1 Enabler B.1). Test-support only; zero bytes shipped in
    /// production builds. `None` w/o local-dev runtime.
    #[cfg(feature = "test-support")]
    pub fn local_dev_shared_trigger_repository_for_test(
        &self,
    ) -> Option<Arc<dyn ironclaw_triggers::TriggerRepository>> {
        let local_runtime = self.local_runtime.as_ref()?;
        Some(Arc::clone(&local_runtime.trigger_repository))
    }

    /// WebUI-facing `InboundAttachmentReader` view over the local-dev
    /// workspace filesystem, mirroring production's `webui.rs`
    /// (`ProjectScopedAttachmentReader` construction at `webui.rs` ~line 153).
    /// Shares [`Self::local_dev_workspace_attachment_reader_for_test`]'s
    /// construction recipe with [`Self::local_dev_attachment_test_support_for_test`]
    /// rather than re-deriving it. Test-support only; zero bytes shipped in
    /// production builds. `None` w/o a local-dev runtime.
    #[cfg(feature = "test-support")]
    pub fn local_dev_inbound_attachment_reader_for_test(
        &self,
    ) -> Option<Arc<dyn ironclaw_product_workflow::InboundAttachmentReader>> {
        Some(self.local_dev_workspace_attachment_reader_for_test()?
            as Arc<dyn ironclaw_product_workflow::InboundAttachmentReader>)
    }

    /// C-JOURNEY: publish a bundled first-party WASM extension package (e.g.
    /// github) directly into the local-dev active-extension registry + trust
    /// policy, bypassing the multi-turn `builtin.extension_install` →
    /// `builtin.extension_activate` capability handshake. Reaches the SAME
    /// `ActiveExtensionPublisher::publish` step `activate()` calls
    /// (`extension_lifecycle.rs`) — the model-visible dispatchable surface —
    /// so a harness that needs a bundled capability (like `github.*`)
    /// reachable for dispatch without scripting install/activate turns can
    /// seed it at construction time. Returns `None` for production-profile
    /// compositions without a local-dev runtime (mirrors
    /// `extension_installation_store_for_test`).
    #[cfg(feature = "test-support")]
    pub fn publish_bundled_extension_for_test(
        &self,
        package: &ironclaw_extensions::ExtensionPackage,
    ) -> Option<Result<(), ironclaw_product_workflow::ProductWorkflowError>> {
        let extension_management = self.local_runtime.as_ref()?.extension_management.as_ref()?;
        Some(
            extension_management
                .active_extensions_for_test()
                .publish(package),
        )
    }

    /// Test-support authority snapshot for active local-dev extensions.
    ///
    /// Binary-E2E harnesses build capability ports at the host-runtime boundary
    /// instead of going through `LocalDevLoopCapabilityPortFactory`, so they need
    /// the same active-extension grants and provider trust that production
    /// local-dev recomputes whenever the model-visible surface is refreshed.
    #[cfg(feature = "test-support")]
    pub async fn local_dev_active_extension_authority_for_test(
        &self,
        grantee: &ExtensionId,
    ) -> Option<
        Result<
            LocalDevActiveExtensionAuthorityForTest,
            ironclaw_product_workflow::ProductWorkflowError,
        >,
    > {
        let extension_management = self.local_runtime.as_ref()?.extension_management.as_ref()?;
        Some(active_extension_authority_for_test(extension_management, grantee).await)
    }
}

#[cfg(feature = "test-support")]
pub struct LocalDevActiveExtensionAuthorityForTest {
    pub grants: Vec<CapabilityGrant>,
    pub provider_trust: Vec<(ExtensionId, TrustDecision)>,
}

#[cfg(feature = "test-support")]
async fn active_extension_authority_for_test(
    extension_management: &RebornLocalExtensionManagementPort,
    grantee: &ExtensionId,
) -> Result<LocalDevActiveExtensionAuthorityForTest, ironclaw_product_workflow::ProductWorkflowError>
{
    let active_capabilities = extension_management
        .active_model_visible_capabilities()
        .await?;
    let grants = active_capabilities
        .iter()
        .map(|capability| CapabilityGrant {
            id: CapabilityGrantId::new(),
            capability: capability.id.clone(),
            grantee: Principal::Extension(grantee.clone()),
            issued_by: Principal::HostRuntime,
            constraints: active_extension_grant_constraints_for_test(capability),
        })
        .collect();
    let mut effects_by_provider: std::collections::BTreeMap<ExtensionId, Vec<EffectKind>> =
        std::collections::BTreeMap::new();
    for capability in &active_capabilities {
        let effects = effects_by_provider
            .entry(capability.provider.clone())
            .or_default();
        for effect in &capability.effects {
            if !effects.contains(effect) {
                effects.push(*effect);
            }
        }
    }
    let provider_trust = effects_by_provider
        .into_iter()
        .map(|(provider, allowed_effects)| {
            (
                provider,
                TrustDecision {
                    effective_trust: EffectiveTrustClass::user_trusted(),
                    authority_ceiling: AuthorityCeiling {
                        allowed_effects,
                        max_resource_ceiling: None,
                    },
                    provenance: TrustProvenance::AdminConfig,
                    evaluated_at: chrono::Utc::now(),
                },
            )
        })
        .collect();
    Ok(LocalDevActiveExtensionAuthorityForTest {
        grants,
        provider_trust,
    })
}

#[cfg(feature = "test-support")]
fn active_extension_grant_constraints_for_test(
    capability: &crate::extension_host::extension_lifecycle::ActiveExtensionCapability,
) -> GrantConstraints {
    GrantConstraints {
        allowed_effects: capability.effects.clone(),
        mounts: MountView::default(),
        network: active_extension_network_policy_for_test(capability),
        secrets: {
            let mut handles = Vec::new();
            for credential in &capability.runtime_credentials {
                if !handles.contains(&credential.handle) {
                    handles.push(credential.handle.clone());
                }
            }
            handles
        },
        resource_ceiling: None,
        expires_at: None,
        max_invocations: None,
    }
}

#[cfg(feature = "test-support")]
fn active_extension_network_policy_for_test(
    capability: &crate::extension_host::extension_lifecycle::ActiveExtensionCapability,
) -> NetworkPolicy {
    if let Some(policy) = gsuite_network_policy_for(&capability.provider) {
        return policy;
    }

    let mut targets = Vec::new();
    for credential in &capability.runtime_credentials {
        if !targets.contains(&credential.audience) {
            targets.push(credential.audience.clone());
        }
    }
    let is_web_access_exa_mcp = capability.provider.as_str() == WEB_ACCESS_EXTENSION_ID
        && matches!(
            capability.id.as_str(),
            WEB_SEARCH_CAPABILITY_ID | WEB_GET_CONTENT_CAPABILITY_ID
        );
    if is_web_access_exa_mcp
        && !targets
            .iter()
            .any(|target| target.host_pattern == EXA_MCP_HOST)
    {
        targets.push(NetworkTargetPattern {
            scheme: Some(ironclaw_host_api::NetworkScheme::Https),
            host_pattern: EXA_MCP_HOST.to_string(),
            port: None,
        });
    }
    NetworkPolicy {
        allowed_targets: targets,
        deny_private_ip_ranges: true,
        max_egress_bytes: is_web_access_exa_mcp.then_some(NETWORK_EGRESS_LIMIT),
    }
}

/// Bundle returned by [`RebornServices::local_dev_attachment_test_support_for_test`]
/// (C-ATTACH seam). Test-support only — zero bytes shipped in production builds.
#[cfg(feature = "test-support")]
#[derive(Clone)]
pub struct AttachmentTestSupport {
    pub read_port: Arc<dyn ironclaw_loop_host::LoopAttachmentReadPort>,
    pub lander: Arc<dyn ironclaw_product_workflow::InboundAttachmentLander>,
}

#[cfg(feature = "test-support")]
#[derive(Clone)]
pub struct RebornLocalDevApprovalTestParts {
    pub approval_requests: Arc<dyn ironclaw_run_state::ApprovalRequestStore>,
    pub capability_leases: Arc<dyn ironclaw_authorization::CapabilityLeaseStore>,
}

pub(crate) struct RebornLocalRuntimeServices {
    pub(crate) extension_lifecycle_surface_context: LifecycleProductSurfaceContext,
    pub(crate) owner_user_id: UserId,
    pub(crate) approval_requests: Arc<LocalDevApprovalRequestStore>,
    pub(crate) capability_leases: Arc<LocalDevCapabilityLeaseStore>,
    /// Per-runtime catalog of client-supplied ("external") tools. Shared between
    /// the loop capability host (which offers them to the model and parks calls)
    /// and the OpenAI-compatible Responses surface (which registers tool specs
    /// and submits client outputs), so both see the same run-scoped state.
    pub(crate) external_tool_catalog: Arc<dyn ExternalToolCatalog>,
    pub(crate) runtime_policy: Option<EffectiveRuntimePolicy>,
    // Used in approval_test_support (cfg(test) only); suppress the dead-code
    // lint on non-test builds where that module is not compiled in.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) capability_policy: Arc<LocalDevCapabilityPolicy>,
    pub(crate) persistent_approval_policies: Arc<LocalDevPersistentApprovalPolicyStore>,
    pub(crate) tool_permission_overrides: Arc<LocalDevToolPermissionOverrideStore>,
    pub(crate) auto_approve_settings: Arc<LocalDevAutoApproveSettingStore>,
    pub(crate) turn_state: Arc<LocalDevTurnStateStore>,
    pub(crate) trigger_repository: Arc<dyn TriggerRepository>,
    /// Facade-shaped handle (not the raw `ProjectRepository`): composition
    /// modules wire the access-controlled service, never the substrate repo.
    pub(crate) project_service: Arc<dyn ProjectService>,
    pub(crate) outbound_preferences: Arc<dyn CommunicationPreferenceRepository>,
    /// The one mutable outbound delivery target registry for this runtime.
    /// Runtime composition wraps it into the outbound preferences facade and
    /// product hosts (Slack host beta) register their providers into it; the
    /// trigger-create hook validates per-trigger `delivery_target_id`s against
    /// the same instance, so an id accepted at creation is one the delivery
    /// layer can resolve at fire time.
    pub(crate) outbound_delivery_targets:
        Arc<crate::outbound::MutableOutboundDeliveryTargetRegistry>,
    /// Global default criteria-based skill auto-activation master switch,
    /// shared by reference between the skill activation selector (reads it per
    /// turn) and the WebUI skills facade (toggles it). Defaults to `true`; a
    /// Settings write flips it and the next turn's selection honors the new
    /// value without a restart.
    pub(crate) skill_auto_activate_learned: Arc<AtomicBool>,
    #[cfg(feature = "slack-v2-host-beta")]
    pub(crate) outbound_state: Arc<dyn OutboundStateStore>,
    #[cfg(feature = "slack-v2-host-beta")]
    pub(crate) delivered_gate_routes: Arc<dyn DeliveredGateRouteStore>,
    #[cfg(feature = "slack-v2-host-beta")]
    pub(crate) triggered_run_delivery: Arc<dyn TriggeredRunDeliveryStore>,
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
    #[cfg(any(feature = "libsql", feature = "postgres"))]
    #[allow(dead_code)]
    pub(crate) identity_filesystem: Arc<ScopedFilesystem<LocalDevRootFilesystem>>,
    /// Admin per-user secret provisioner (target-user-scoped secret store over
    /// the shared root + crypto). `None` when no filesystem secret store was
    /// built. Read only by the WebUI v2 admin surface.
    #[cfg(feature = "webui-v2-beta")]
    pub(crate) admin_secret_provisioner:
        Option<Arc<dyn crate::admin_secrets::AdminSecretProvisioner>>,
    /// Raw libSQL substrate handle backing `reborn-local-dev.db`. Carried ONLY
    /// for the one-time legacy WebUI `user_identities` fold (a substrate-level
    /// read that belongs in this host layer, not the identity crate); the
    /// steady-state identity store goes through `identity_filesystem` above.
    #[cfg(feature = "libsql")]
    #[allow(dead_code)]
    pub(crate) identity_substrate_db: Option<Arc<libsql::Database>>,
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
    /// Late-binding slot for the per-caller channel-connection facade. Created
    /// empty here and shared with the extension-lifecycle capability handler so
    /// an inbound-channel activation can check whether the caller has already
    /// connected the channel. Filled after runtime build by the Slack host-beta
    /// composition (`build_webui_services_with_slack_host_beta_mounts` →
    /// `RebornRuntime::set_channel_connection_facade`); stays empty in
    /// deployments without a connectable channel, in which case the handler
    /// fails closed (blocks) for any channel that declares a connection
    /// requirement. Mirrors the `post_submit_hook_slot` deferred-wiring pattern.
    #[cfg(any(feature = "slack-v2-host-beta", test))]
    pub(crate) channel_connection_facade_slot:
        Arc<std::sync::OnceLock<Arc<dyn ChannelConnectionFacade>>>,
    pub(crate) runtime_http_egress: Option<Arc<dyn RuntimeHttpEgress>>,
    pub(crate) host_runtime_http_egress: Option<HostRuntimeHttpEgressPort>,
    pub(crate) skill_mounts: MountView,
    pub(crate) memory_mounts: MountView,
    pub(crate) system_extensions_lifecycle_mounts: MountView,
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
    pub(crate) shared_extension_registry: Option<Arc<SharedExtensionRegistry>>,
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
pub(crate) enum RebornProductionRuntimeServices {
    #[cfg(feature = "libsql")]
    LibSql(Arc<RebornProductionRuntimeStoreGraph<LibSqlRootFilesystem>>),
    #[cfg(feature = "postgres")]
    Postgres(Arc<RebornProductionRuntimeStoreGraph<PostgresRootFilesystem>>),
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
pub(crate) struct RebornProductionRuntimeStoreGraph<F>
where
    F: RootFilesystem + 'static,
{
    pub(crate) scoped_filesystem: Arc<ScopedFilesystem<F>>,
    /// Registry used by the production host runtime for extension descriptors.
    #[allow(dead_code)]
    pub(crate) extension_registry: Arc<ExtensionRegistry>,
    pub(crate) turn_state: Arc<FilesystemTurnStateStoreKind<F>>,
    pub(crate) checkpoint_state_store: Arc<dyn CheckpointStateStore>,
    pub(crate) thread_service: Arc<dyn SessionThreadService>,
    pub(crate) trigger_repository: Arc<dyn TriggerRepository>,
    pub(crate) resource_governor: Arc<dyn ResourceGovernor>,
    pub(crate) budget_gate_store: Arc<dyn BudgetGateStore>,
    pub(crate) broadcast_budget_event_sink: Arc<BroadcastBudgetEventSink>,
    pub(crate) event_log: Arc<dyn DurableEventLog>,
    pub(crate) audit_log: Arc<dyn DurableAuditLog>,
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
impl RebornProductionRuntimeServices {
    /// Returns the trigger repository from whichever production store graph is
    /// active. Backs the WebUI automations facade for production profiles
    /// (libSQL / Postgres) where `local_runtime` is None.
    pub(crate) fn trigger_repository(&self) -> Arc<dyn TriggerRepository> {
        match self {
            #[cfg(feature = "libsql")]
            Self::LibSql(graph) => Arc::clone(&graph.trigger_repository),
            #[cfg(feature = "postgres")]
            Self::Postgres(graph) => Arc::clone(&graph.trigger_repository),
        }
    }

    /// Turn-state snapshot source from the active production store graph.
    /// Pairs with [`Self::trigger_repository`] so the automations facade
    /// derives active-hold projections from the same runtime's run state
    /// (#5886).
    pub(crate) fn turn_run_snapshot_source(
        &self,
    ) -> Arc<dyn crate::turn_run_snapshot::TurnRunSnapshotSource> {
        match self {
            #[cfg(feature = "libsql")]
            Self::LibSql(graph) => Arc::clone(&graph.turn_state) as _,
            #[cfg(feature = "postgres")]
            Self::Postgres(graph) => Arc::clone(&graph.turn_state) as _,
        }
    }
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
    persistent_approval_policies: Arc<LocalDevPersistentApprovalPolicyStore>,
    turn_state: Arc<LocalDevTurnStateStore>,
    local_runtime: Arc<RebornLocalRuntimeServices>,
    resource_governor: Arc<LocalDevResourceGovernor>,
    process_services: LocalDevProcessServices,
    trigger_repository: Arc<dyn TriggerRepository>,
}

struct RebornLocalDevStoreGraphInput {
    filesystem: Arc<LocalDevRootFilesystem>,
    owner_user_id: UserId,
    local_runtime_identity: Option<RebornLocalRuntimeIdentity>,
    runtime_policy: Option<EffectiveRuntimePolicy>,
    skill_filesystem: Arc<ScopedFilesystem<LocalDevRootFilesystem>>,
    workspace_filesystem: Arc<ScopedFilesystem<LocalDevRootFilesystem>>,
    workspace_mounts: MountView,
    local_dev_storage_root: PathBuf,
    default_system_prompt_path: PathBuf,
    trigger_repository: Arc<dyn TriggerRepository>,
    project_repository: Arc<dyn ProjectRepository>,
    /// Concurrency limits for the in-memory (or filesystem-backed) turn-state store.
    turn_state_store_limits: ironclaw_turns::InMemoryTurnStateStoreLimits,
    #[cfg(feature = "postgres")]
    postgres_resource_governor_singleton: Option<bool>,
    /// Raw libSQL substrate handle, carried so the canonical Reborn identity
    /// store rides the same `reborn-local-dev.db` instead of opening a second
    /// handle (see `RebornRuntime::open_reborn_identity_resolver`).
    #[cfg(feature = "libsql")]
    identity_substrate_db: Option<Arc<libsql::Database>>,
}

impl std::fmt::Debug for RebornServices {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut debug = formatter.debug_struct("RebornServices");
        debug
            .field("host_runtime", &self.host_runtime.is_some())
            .field("turn_coordinator", &self.turn_coordinator.is_some())
            .field("product_auth", &self.product_auth.is_some())
            .field("readiness", &self.readiness)
            .field("local_runtime", &self.local_runtime.is_some());
        #[cfg(any(feature = "libsql", feature = "postgres"))]
        debug.field("production_runtime", &self.production_runtime.is_some());
        debug.finish()
    }
}

// arch-exempt: optional_arc, RebornServices fields are Optional because disabled()/local-dev paths don't wire all production services; proper factories always set them, plan #4469

impl RebornServices {
    pub fn disabled() -> Self {
        Self {
            host_runtime: None,
            turn_coordinator: None,
            product_auth: None,
            readiness: RebornReadiness::disabled(),
            skill_management: None,
            local_runtime: None,
            #[cfg(any(feature = "libsql", feature = "postgres"))]
            production_runtime: None,
            #[cfg(any(feature = "libsql", feature = "postgres"))]
            production_scheduler_wake: None,
            secret_store: Arc::new(ironclaw_secrets::InMemorySecretStore::new()),
            #[cfg(any(test, feature = "test-support"))]
            local_dev_wasm_runtime_credential_provider_captured: false,
            #[cfg(any(feature = "libsql", feature = "postgres"))]
            credential_refresh_worker: CredentialRefreshWorkerReady::Absent,
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
        RebornCompositionProfile::LocalDev
        | RebornCompositionProfile::LocalDevYolo
        | RebornCompositionProfile::HostedSingleTenant
        | RebornCompositionProfile::HostedSingleTenantVolume => build_local_runtime(input).await,
        RebornCompositionProfile::Production | RebornCompositionProfile::MigrationDryRun => {
            build_production_shaped(input).await
        }
    }
}

fn auth_continuation_dispatcher(
    turn_coordinator: Arc<dyn ironclaw_turns::TurnCoordinator>,
    blocked_auth_snapshot_source: Option<
        Arc<dyn crate::blocked_auth_resume::BlockedAuthSnapshotSource>,
    >,
    lifecycle: LifecycleProductFacadeSlot,
) -> Arc<dyn RebornAuthContinuationDispatcher> {
    let single_run: Arc<dyn RebornAuthContinuationDispatcher> = Arc::new(
        ProductAuthTurnGateResumeDispatcher::new(Arc::clone(&turn_coordinator)),
    );
    let turn_dispatcher = match blocked_auth_snapshot_source {
        // Local paths fan a completed flow out to the caller's other
        // provider-blocked runs (pair/authorize once, all waiting chats
        // continue). Production-shaped builders pass None until their
        // turn-state snapshot source is wired.
        Some(snapshot_source) => {
            Arc::new(crate::blocked_auth_resume::BlockedAuthResumeFanout::new(
                single_run,
                snapshot_source,
                turn_coordinator,
            ))
        }
        None => single_run,
    };
    Arc::new(LifecycleAuthContinuationDispatcher::new(
        turn_dispatcher,
        lifecycle,
    ))
}

struct ProductAuthServicesCompositionInput {
    ports: RebornProductAuthServicePorts,
    turn_coordinator: Arc<dyn ironclaw_turns::TurnCoordinator>,
    blocked_auth_snapshot_source:
        Option<Arc<dyn crate::blocked_auth_resume::BlockedAuthSnapshotSource>>,
    lifecycle: LifecycleProductFacadeSlot,
    provider_composition: OAuthProviderComposition,
    security_audit_sink: Option<Arc<dyn ironclaw_events::SecurityAuditSink>>,
    secret_store: Arc<dyn SecretStore>,
    nearai_mcp_host_managed_scope: Option<AuthProductScope>,
}

fn compose_product_auth_services(
    input: ProductAuthServicesCompositionInput,
) -> Result<Arc<RebornProductAuthServices>, RebornBuildError> {
    let ProductAuthServicesCompositionInput {
        ports,
        turn_coordinator,
        blocked_auth_snapshot_source,
        lifecycle,
        provider_composition,
        security_audit_sink,
        secret_store,
        nearai_mcp_host_managed_scope,
    } = input;
    let ports = match provider_composition.client {
        Some(provider_client) => ports.with_provider_client(provider_client),
        None => ports,
    };
    let mut services = ports.into_services(
        auth_continuation_dispatcher(turn_coordinator, blocked_auth_snapshot_source, lifecycle),
        secret_store,
    );
    if let Some(sink) = security_audit_sink {
        services = services.with_security_audit_sink(sink);
    }
    if let Some(registry) = provider_composition.dcr_registry {
        services = services.with_dcr_oauth_registry(registry);
    }
    if let Some(registry) = provider_composition.gate_registry {
        services = services.with_oauth_gate_registry(registry);
    }
    if let Some(scope) = nearai_mcp_host_managed_scope {
        services = services.with_host_managed_nearai_credential_scope(scope)?;
    }
    Ok(Arc::new(services))
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

/// Build the safe single-tenant runtime surface used by local-dev and
/// hosted-single-tenant. Hosted single-tenant supplies a durable Postgres
/// backend through `RebornStorageInput::HostedSingleTenantPostgres`; local-dev
/// keeps its historical local filesystem/libSQL default.
async fn build_local_runtime(input: RebornBuildInput) -> Result<RebornServices, RebornBuildError> {
    #[cfg(all(test, feature = "slack-v2-host-beta"))]
    let host_runtime_http_egress_for_test = input.host_runtime_http_egress_for_test.clone();
    #[cfg(any(test, feature = "test-support"))]
    let network_http_egress_for_test = input.network_http_egress_for_test.clone();
    let RebornBuildInput {
        profile,
        storage,
        runtime_policy,
        runtime_process_binding,
        product_auth_ports,
        oauth_provider_configs,
        oauth_dcr_provider_configs,
        #[cfg(feature = "slack-v2-host-beta")]
        slack_personal_oauth_lazy_slot,
        nearai_mcp_bootstrap_config,
        owner_id,
        local_runtime_identity,
        turn_state_store_limits,
        ..
    } = input;
    let local_runtime_identity_for_nearai_mcp = local_runtime_identity.clone();
    let (
        root,
        workspace_root,
        host_home_root,
        storage_backend_input,
        secret_master_key,
        postgres_resource_governor_singleton,
    ) = match storage {
        RebornStorageInput::LocalDev { .. }
            if profile == RebornCompositionProfile::HostedSingleTenant =>
        {
            return Err(RebornBuildError::InvalidConfig {
                    reason: "profile=hosted-single-tenant requires hosted single-tenant Postgres storage input"
                        .to_string(),
                });
        }
        RebornStorageInput::LocalDev {
            root,
            workspace_root,
            host_home_root,
        } => (
            root,
            workspace_root,
            host_home_root,
            LocalDevStorageBackendInput::LocalDefault,
            None::<ironclaw_secrets::SecretMaterial>,
            None::<bool>,
        ),
        #[cfg(feature = "postgres")]
        RebornStorageInput::HostedSingleTenantPostgres { .. }
            if profile != RebornCompositionProfile::HostedSingleTenant =>
        {
            return Err(RebornBuildError::InvalidConfig {
                reason: format!("{profile} profile requires local-runtime storage input"),
            });
        }
        #[cfg(feature = "postgres")]
        RebornStorageInput::HostedSingleTenantPostgres {
            root,
            workspace_root,
            host_home_root,
            pool,
            secret_master_key,
            process_local_resource_governor_singleton,
        } => (
            root,
            workspace_root,
            host_home_root,
            LocalDevStorageBackendInput::Postgres(pool),
            Some(secret_master_key),
            Some(process_local_resource_governor_singleton),
        ),
        _ => {
            return Err(RebornBuildError::InvalidConfig {
                reason: format!("{profile} profile requires local-runtime storage input"),
            });
        }
    };
    #[cfg(not(any(feature = "libsql", feature = "postgres")))]
    let _ = secret_master_key;
    #[cfg(not(feature = "postgres"))]
    let _ = postgres_resource_governor_singleton;
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
    crate::extension_host::bundled_skills::ensure_bundled_reborn_skills_installed(&root).await?;
    let filesystem_bundle = build_local_dev_root_filesystem(
        &root,
        &workspace_root,
        host_home_root.as_ref(),
        storage_backend_input,
    )
    .await?;
    let extension_installation_state_path =
        local_dev_extension_installation_state_path(profile, local_runtime_identity.as_ref())?;
    // Clone the raw libSQL handle for the canonical identity store before
    // `filesystem` moves out of the bundle, so the resolver rides the same
    // substrate DB the runtime owns rather than a second handle.
    #[cfg(all(feature = "libsql", feature = "postgres"))]
    let identity_substrate_db = match &filesystem_bundle.durable_backend {
        LocalDevDurableBackend::LibSql(database) => Some(Arc::clone(database)),
        LocalDevDurableBackend::Postgres(_) => None,
    };
    #[cfg(all(feature = "libsql", not(feature = "postgres")))]
    let identity_substrate_db = {
        let LocalDevDurableBackend::LibSql(database) = &filesystem_bundle.durable_backend;
        Some(Arc::clone(database))
    };
    let trigger_repository =
        local_dev_trigger_repository(&filesystem_bundle.durable_backend).await?;
    let filesystem = filesystem_bundle.filesystem;
    // Projects persist over the control-plane `ScopedFilesystem` substrate (no
    // SQL in the crate); the backend is whatever the local-dev root filesystem
    // dispatches to. Tenant is supplied per call, so the scope carries only the
    // control-plane user/agent identity. Without a durable backend the runtime
    // has no scoped substrate, so projects ride an ephemeral in-memory backend —
    // parity with the in-memory trigger repository.
    let project_agent_id = ironclaw_host_api::AgentId::new("reborn-projects").map_err(|error| {
        RebornBuildError::InvalidConfig {
            reason: format!("invalid project agent id: {error}"),
        }
    })?;
    #[cfg(any(feature = "libsql", feature = "postgres"))]
    let project_repository: Arc<dyn ProjectRepository> =
        Arc::new(ironclaw_projects::FilesystemProjectRepository::new(
            crate::wrap_scoped(Arc::clone(&filesystem)),
            owner_user_id.clone(),
            project_agent_id,
        ));
    #[cfg(not(any(feature = "libsql", feature = "postgres")))]
    let project_repository: Arc<dyn ProjectRepository> = {
        use ironclaw_host_api::{MountAlias, MountGrant, MountPermissions, MountView, VirtualPath};
        let view = MountView::new(vec![MountGrant::new(
            MountAlias::new("/tenant-shared").map_err(|error| RebornBuildError::InvalidConfig {
                reason: format!("invalid project mount alias: {error}"),
            })?,
            VirtualPath::new("/tenants/local/shared").map_err(|error| {
                RebornBuildError::InvalidConfig {
                    reason: format!("invalid project virtual path: {error}"),
                }
            })?,
            MountPermissions::read_write_list_delete(),
        )])
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("invalid project mount view: {error}"),
        })?;
        let scoped = Arc::new(ScopedFilesystem::with_fixed_view(
            Arc::new(ironclaw_filesystem::InMemoryBackend::default()),
            view,
        ));
        Arc::new(ironclaw_projects::FilesystemProjectRepository::new(
            scoped,
            owner_user_id.clone(),
            project_agent_id,
        ))
    };
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
    let nearai_mcp_owner_scope = local_dev_nearai_mcp_owner_scope(
        owner_user_id.clone(),
        local_runtime_identity_for_nearai_mcp.as_ref(),
    )?;
    let mut store_graph = build_local_dev_store_graph(RebornLocalDevStoreGraphInput {
        filesystem: Arc::clone(&filesystem),
        owner_user_id,
        local_runtime_identity,
        runtime_policy: runtime_policy.clone(),
        skill_filesystem,
        workspace_filesystem,
        workspace_mounts: runtime_workspace_mounts,
        local_dev_storage_root: root.clone(),
        default_system_prompt_path,
        trigger_repository,
        project_repository,
        turn_state_store_limits,
        #[cfg(feature = "postgres")]
        postgres_resource_governor_singleton,
        #[cfg(feature = "libsql")]
        identity_substrate_db,
    })
    .await?;

    let turn_coordinator: Arc<dyn ironclaw_turns::TurnCoordinator> = Arc::new(
        DefaultTurnCoordinator::new(Arc::clone(&store_graph.turn_state)),
    );
    #[cfg(any(feature = "libsql", feature = "postgres"))]
    let local_dev_product_auth_filesystem = local_dev_scoped_filesystem(Arc::clone(&filesystem));
    #[cfg(any(feature = "libsql", feature = "postgres"))]
    let local_dev_secret_bundle = build_local_dev_secret_store(
        &root,
        Arc::clone(&local_dev_product_auth_filesystem),
        secret_master_key,
    )?;
    #[cfg(any(feature = "libsql", feature = "postgres"))]
    let secret_store: Arc<dyn SecretStore> = local_dev_secret_bundle.0.clone();
    #[cfg(not(any(feature = "libsql", feature = "postgres")))]
    let secret_store: Arc<dyn SecretStore> = Arc::new(ironclaw_secrets::InMemorySecretStore::new());
    // Admin per-user secret provisioner over the shared root + the SAME crypto
    // as the runtime's own secret store (webui-v2-beta implies libsql, so the
    // bundle above always exists here).
    #[cfg(feature = "webui-v2-beta")]
    let admin_secret_provisioner: Option<
        Arc<dyn crate::admin_secrets::AdminSecretProvisioner>,
    > = Some(Arc::new(
        crate::admin_secrets::FilesystemAdminSecretProvisioner::new(
            Arc::clone(&filesystem),
            local_dev_secret_bundle.1,
        ),
    ));
    let local_dev_trust_policy = Arc::new(builtin_first_party_trust_policy()?);
    let local_dev_trust_invalidation_bus = Arc::new(ironclaw_trust::InvalidationBus::new());
    let extension_registry = Arc::new(local_dev_builtin_extension_registry()?);
    // Per-(tenant,user) approval settings resolved live at each dispatch gate
    // so a WebUI change applies without a restart (#4959). Reuse the local
    // runtime stores exactly: in-memory builds must not accidentally fork UI
    // writes away from the authorizer.
    let tool_permission_overrides: Arc<dyn ironclaw_approvals::ToolPermissionOverrideStore> =
        store_graph.local_runtime.tool_permission_overrides.clone();
    let auto_approve_settings: Arc<dyn ironclaw_approvals::AutoApproveSettingStore> =
        store_graph.local_runtime.auto_approve_settings.clone();
    let approval_settings_provider = Arc::new(StoreApprovalSettingsProvider::new(
        tool_permission_overrides,
        auto_approve_settings,
        store_graph
            .local_runtime
            .persistent_approval_policies
            .clone(),
    ));
    let authorizer = local_dev_authorizer(
        runtime_policy.as_ref(),
        Arc::clone(&store_graph.local_runtime.capability_policy),
        approval_settings_provider,
    );
    let services = HostRuntimeServices::new(
        Arc::clone(&extension_registry),
        Arc::clone(&filesystem),
        Arc::clone(&store_graph.resource_governor),
        authorizer,
        store_graph.process_services.clone(),
        CapabilitySurfaceVersion::new("reborn-app-v1")?,
    )
    .with_trust_policy(Arc::clone(&local_dev_trust_policy))
    .with_secret_store_dyn(Arc::clone(&secret_store));
    #[cfg(any(test, feature = "test-support"))]
    let services = if let Some(network_http_egress) = network_http_egress_for_test {
        services.try_with_host_http_egress_with_body_store(
            TestNetworkHttpEgress(network_http_egress),
            http_body_filesystem,
        )?
    } else {
        services.try_with_host_http_egress_with_body_store(
            ironclaw_network::PolicyNetworkHttpEgress::new(
                ironclaw_network::ReqwestNetworkTransport::default(),
            ),
            http_body_filesystem,
        )?
    };
    #[cfg(not(any(test, feature = "test-support")))]
    let services = services.try_with_host_http_egress_with_body_store(
        ironclaw_network::PolicyNetworkHttpEgress::new(
            ironclaw_network::ReqwestNetworkTransport::default(),
        ),
        http_body_filesystem,
    )?;
    let mut services = services
        .with_run_state(Arc::clone(&store_graph.run_state))
        .with_approval_requests(Arc::clone(&store_graph.approval_requests))
        .with_capability_leases(Arc::clone(&store_graph.capability_leases))
        .with_persistent_approval_policies(Arc::clone(&store_graph.persistent_approval_policies))
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
    let product_auth_runtime_ports = require_product_auth_runtime_ports(&services)?;
    let provider_composition = compose_provider_client(
        oauth_provider_configs,
        oauth_dcr_provider_configs,
        Arc::clone(&secret_store),
        product_auth_runtime_ports.clone(),
        #[cfg(feature = "slack-v2-host-beta")]
        slack_personal_oauth_lazy_slot,
        #[cfg(not(feature = "slack-v2-host-beta"))]
        None,
    )?;
    let security_audit_sink = services.security_audit_sink();
    let nearai_mcp_host_managed_scope =
        AuthProductScope::new(nearai_mcp_owner_scope.clone(), AuthSurface::Api);
    let lifecycle_auth_continuation_slot: LifecycleProductFacadeSlot = Arc::new(OnceLock::new());
    let product_auth = match product_auth_ports {
        Some(ports) => compose_product_auth_services(ProductAuthServicesCompositionInput {
            ports,
            turn_coordinator: turn_coordinator.clone(),
            blocked_auth_snapshot_source: Some(Arc::clone(&store_graph.turn_state)
                as Arc<dyn crate::blocked_auth_resume::BlockedAuthSnapshotSource>),
            lifecycle: Arc::clone(&lifecycle_auth_continuation_slot),
            provider_composition,
            security_audit_sink: security_audit_sink.clone(),
            secret_store: Arc::clone(&secret_store),
            nearai_mcp_host_managed_scope: Some(nearai_mcp_host_managed_scope.clone()),
        })?,
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
                // Wrap the credential-account service in
                // `ProviderBackedCredentialAccountService` (via `with_provider_client`) so the
                // runtime token-refresh path (`refresh_account`) routes through the OAuth
                // provider client. `from_shared_with_provider` stores the provider client in a
                // separate field but does NOT wrap the account service, so without this the
                // durable `FilesystemAuthProductServices::refresh_account` stub returns
                // `BackendUnavailable` — Google OAuth access tokens are never refreshed and every
                // capability call reauths once the 1h access token expires. The sibling branch
                // routes through `compose_product_auth_services`, which applies the same wrap.
                let services = RebornProductAuthServicePorts::from_shared_with_provider(
                    Arc::clone(&durable_services),
                    Arc::clone(&provider_client),
                )
                .with_provider_client(Arc::clone(&provider_client))
                .into_services(
                    auth_continuation_dispatcher(
                        turn_coordinator.clone(),
                        Some(Arc::clone(&store_graph.turn_state)
                            as Arc<
                                dyn crate::blocked_auth_resume::BlockedAuthSnapshotSource,
                            >),
                        Arc::clone(&lifecycle_auth_continuation_slot),
                    ),
                    Arc::clone(&secret_store),
                )
                .with_provider_client(Arc::clone(&provider_client))
                .with_flow_record_source(durable_services);
                let services = match provider_composition.dcr_registry.clone() {
                    Some(registry) => services.with_dcr_oauth_registry(registry),
                    None => services,
                };
                let services = match provider_composition.gate_registry.clone() {
                    Some(registry) => services.with_oauth_gate_registry(registry),
                    None => services,
                };
                let services = match security_audit_sink.clone() {
                    Some(sink) => services.with_security_audit_sink(sink),
                    None => services,
                };
                Arc::new(services.with_host_managed_nearai_credential_scope(
                    nearai_mcp_host_managed_scope.clone(),
                )?)
            }
            #[cfg(not(any(feature = "libsql", feature = "postgres")))]
            {
                let services =
                    RebornProductAuthServices::local_dev_in_memory(auth_continuation_dispatcher(
                        turn_coordinator.clone(),
                        Some(Arc::clone(&store_graph.turn_state)
                            as Arc<
                                dyn crate::blocked_auth_resume::BlockedAuthSnapshotSource,
                            >),
                        Arc::clone(&lifecycle_auth_continuation_slot),
                    ));
                let services = match provider_composition.client.clone() {
                    Some(provider_client) => services.with_provider_client(provider_client),
                    None => services,
                };
                let services = match provider_composition.dcr_registry.clone() {
                    Some(registry) => services.with_dcr_oauth_registry(registry),
                    None => services,
                };
                let services = match security_audit_sink.clone() {
                    Some(sink) => services.with_security_audit_sink(sink),
                    None => services,
                };
                let services = match provider_composition.gate_registry.clone() {
                    Some(registry) => services.with_oauth_gate_registry(registry),
                    None => services,
                };
                #[cfg(feature = "slack-v2-host-beta")]
                let services = match provider_composition.slack_gate_registry.clone() {
                    Some(registry) => services.with_slack_oauth_gate_registry(registry),
                    None => services,
                };
                Arc::new(services.with_host_managed_nearai_credential_scope(
                    nearai_mcp_host_managed_scope.clone(),
                )?)
            }
        }
    };
    services = services.with_runtime_credential_account_resolver(Arc::new(
        ProductAuthRuntimeCredentialResolver::new_with_refresh(
            product_auth.runtime_credential_account_selection_service(),
            product_auth.runtime_credential_account_refresh_service(),
        ),
    ));
    services = attach_wasm_runtime(services)?;
    let mut available_extensions = AvailableExtensionCatalog::from_filesystem_root(
        filesystem.as_ref(),
        &VirtualPath::new("/system/extensions")?,
    )
    .await
    .map_err(|error| RebornBuildError::InvalidConfig {
        reason: format!("available extension catalog could not be loaded: {error}"),
    })?;
    available_extensions.extend(
        AvailableExtensionCatalog::from_first_party_assets_with_nearai_mcp_config(
            nearai_mcp_bootstrap_config.as_ref(),
        )
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("first-party extension catalog could not be loaded: {error}"),
        })?,
    );
    let extension_filesystem: Arc<dyn RootFilesystem> = filesystem.clone();
    let extension_installation_store: Arc<dyn ExtensionInstallationStore> = Arc::new(
        FilesystemExtensionInstallationStore::load_at(
            extension_filesystem.clone(),
            extension_installation_state_path,
        )
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
    #[cfg(feature = "slack-v2-host-beta")]
    let removal_cleanup_adapters: Vec<Arc<dyn ExtensionRemovalCleanupAdapter>> = vec![Arc::new(
        SlackPersonalConnectionCleanupAdapter::new(Arc::clone(
            &store_graph.local_runtime.channel_connection_facade_slot,
        ))
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("Slack extension removal cleanup could not be built: {error}"),
        })?,
    )];
    #[cfg(not(feature = "slack-v2-host-beta"))]
    let removal_cleanup_adapters: Vec<Arc<dyn ExtensionRemovalCleanupAdapter>> = Vec::new();
    let removal_cleanup = Arc::new(
        ExtensionRemovalCleanupRegistry::try_from_adapters(removal_cleanup_adapters).map_err(
            |error| RebornBuildError::InvalidConfig {
                reason: format!("extension removal cleanup registry could not be built: {error}"),
            },
        )?,
    );
    let extension_management = Arc::new(
        RebornLocalExtensionManagementPort::new(
            extension_filesystem,
            available_extensions,
            extension_installation_store,
            extension_lifecycle_service,
            active_extensions,
            Some(Arc::clone(&product_auth) as Arc<dyn ExtensionCredentialCleanup>),
            // #5459 P1: the base owner is the tenant operator in local-dev —
            // their installs are tenant-shared, everyone else's are private.
            nearai_mcp_owner_scope.user_id.clone(),
        )
        .with_removal_cleanup_registry(removal_cleanup),
    );
    let lifecycle_facade =
        RebornLocalLifecycleFacade::new(store_graph.local_runtime.skill_management.clone())
            .with_extension_management(Arc::clone(&extension_management))
            .with_runtime_http_egress(product_auth_runtime_ports.runtime_http_egress())
            .with_runtime_credential_accounts(
                product_auth.runtime_credential_account_selection_service(),
            );
    let lifecycle_facade: Arc<dyn ironclaw_product_workflow::LifecycleProductFacade> =
        Arc::new(lifecycle_facade);
    lifecycle_auth_continuation_slot
        .set(lifecycle_facade)
        .map_err(|_| RebornBuildError::InvalidConfig {
            reason: "extension lifecycle auth continuation facade was already attached".to_string(),
        })?;
    let nearai_mcp_bootstrap_outcome = crate::llm_admin::nearai_mcp::bootstrap_nearai_mcp(
        nearai_mcp_bootstrap_config,
        &product_auth,
        &extension_management,
        nearai_mcp_owner_scope,
    )
    .await?;
    nearai_mcp_bootstrap_outcome.log_completion();
    if let Some(local_runtime) = Arc::get_mut(&mut store_graph.local_runtime) {
        local_runtime.extension_management = Some(Arc::clone(&extension_management));
        local_runtime.runtime_http_egress = Some(product_auth_runtime_ports.runtime_http_egress());
        local_runtime.extension_registry = Arc::clone(&extension_registry);
        local_runtime.shared_extension_registry = Some(services.shared_extension_registry());
        let host_runtime_http_egress = services.host_runtime_http_egress_port();
        #[cfg(all(test, feature = "slack-v2-host-beta"))]
        let host_runtime_http_egress =
            host_runtime_http_egress_for_test.unwrap_or(host_runtime_http_egress);
        local_runtime.host_runtime_http_egress = host_runtime_http_egress;
        // Attach the admin secret provisioner now the secret-store crypto is
        // built (the store graph was constructed before it existed).
        #[cfg(feature = "webui-v2-beta")]
        {
            local_runtime.admin_secret_provisioner = admin_secret_provisioner;
        }
    } else {
        return Err(RebornBuildError::InvalidConfig {
            reason: "local-dev extension lifecycle facade could not be attached".to_string(),
        });
    }
    let trigger_create_hook = local_dev_trigger_create_hook(&store_graph.local_runtime);
    // Built from the same turn-state store the WebUI automations panel reads
    // (`crate::webui::facade`), so both `trigger_list` and the panel agree on
    // which fires are blocked (#5886).
    let trigger_active_run_lookup: Arc<dyn TriggerActiveRunLookup> = Arc::new(
        crate::automation::trigger_poller::SnapshotActiveRunLookup::new(Arc::clone(
            &store_graph.local_runtime.turn_state,
        )
            as Arc<dyn crate::turn_run_snapshot::TurnRunSnapshotSource>),
    );
    let mut first_party_registry = builtin_first_party_registry_with_trigger_create_hook(
        Arc::clone(&store_graph.trigger_repository),
        trigger_create_hook,
        trigger_active_run_lookup,
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
    insert_extension_lifecycle_handlers(
        &mut first_party_registry,
        extension_management,
        product_auth.runtime_credential_account_selection_service(),
    )
    .map_err(|error| RebornBuildError::InvalidConfig {
        reason: format!("local-dev extension lifecycle handlers are invalid: {error}"),
    })?;
    services = services.with_first_party_capabilities(Arc::new(first_party_registry));

    #[cfg(any(test, feature = "test-support"))]
    let local_dev_wasm_runtime_credential_provider_captured =
        services.wasm_runtime_credential_provider_captured_for_test();
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
        #[cfg(any(feature = "libsql", feature = "postgres"))]
        production_runtime: None,
        #[cfg(any(feature = "libsql", feature = "postgres"))]
        production_scheduler_wake: None,
        secret_store,
        #[cfg(any(test, feature = "test-support"))]
        local_dev_wasm_runtime_credential_provider_captured,
        // Local-dev is single-user; no cross-owner enumeration or leader lock needed.
        #[cfg(any(feature = "libsql", feature = "postgres"))]
        credential_refresh_worker: CredentialRefreshWorkerReady::Absent,
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

fn local_dev_extension_lifecycle_surface_context(
    owner_user_id: UserId,
    local_runtime_identity: Option<&RebornLocalRuntimeIdentity>,
) -> Result<LifecycleProductSurfaceContext, RebornBuildError> {
    let default_identity = RebornRuntimeIdentity::reborn_cli();
    let default_tenant_id =
        ironclaw_host_api::TenantId::new(default_identity.tenant_id).map_err(|error| {
            RebornBuildError::InvalidConfig {
                reason: error.to_string(),
            }
        })?;
    let default_agent_id =
        ironclaw_host_api::AgentId::new(default_identity.agent_id).map_err(|error| {
            RebornBuildError::InvalidConfig {
                reason: error.to_string(),
            }
        })?;
    let tenant_id = local_runtime_identity
        .map(|identity| identity.tenant_id.clone())
        .unwrap_or(default_tenant_id);
    let agent_id = local_runtime_identity
        .map(|identity| identity.agent_id.clone())
        .unwrap_or(default_agent_id);
    Ok(LifecycleProductSurfaceContext {
        tenant_id,
        user_id: owner_user_id,
        agent_id: Some(agent_id),
        project_id: None,
    })
}

fn local_dev_nearai_mcp_owner_scope(
    owner_user_id: UserId,
    local_runtime_identity: Option<&RebornLocalRuntimeIdentity>,
) -> Result<ResourceScope, RebornBuildError> {
    let context =
        local_dev_extension_lifecycle_surface_context(owner_user_id, local_runtime_identity)?;
    Ok(ResourceScope {
        tenant_id: context.tenant_id,
        user_id: context.user_id,
        agent_id: context.agent_id,
        project_id: context.project_id,
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    })
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
fn owner_scope_from_runtime_identity(
    owner_user_id: UserId,
    tenant_id: ironclaw_host_api::TenantId,
    agent_id: ironclaw_host_api::AgentId,
) -> ResourceScope {
    ResourceScope {
        tenant_id,
        user_id: owner_user_id,
        agent_id: Some(agent_id),
        project_id: None,
        mission_id: None,
        thread_id: None,
        invocation_id: InvocationId::new(),
    }
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
fn default_runtime_owner_scope(
    owner_user_id: UserId,
) -> Result<ResourceScope, ironclaw_host_api::HostApiError> {
    let identity = RebornRuntimeIdentity::reborn_cli();
    let tenant_id = ironclaw_host_api::TenantId::new(identity.tenant_id)?;
    let agent_id = ironclaw_host_api::AgentId::new(identity.agent_id)?;
    Ok(owner_scope_from_runtime_identity(
        owner_user_id,
        tenant_id,
        agent_id,
    ))
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
fn configured_runtime_owner_scope(
    owner_user_id: UserId,
    local_runtime_identity: &RebornLocalRuntimeIdentity,
) -> ResourceScope {
    owner_scope_from_runtime_identity(
        owner_user_id,
        local_runtime_identity.tenant_id.clone(),
        local_runtime_identity.agent_id.clone(),
    )
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
fn owner_turn_state_filesystem<F>(
    filesystem: Arc<F>,
    owner_scope: &ResourceScope,
) -> Result<Arc<ScopedFilesystem<F>>, ironclaw_host_api::HostApiError>
where
    F: RootFilesystem + 'static,
{
    let view = crate::invocation_mount_view(owner_scope)?;
    Ok(Arc::new(ScopedFilesystem::with_fixed_view(
        filesystem, view,
    )))
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
fn production_turn_state_store<F>(
    filesystem: Arc<ScopedFilesystem<F>>,
    limits: ironclaw_turns::InMemoryTurnStateStoreLimits,
) -> FilesystemTurnStateStoreKind<F>
where
    F: RootFilesystem + 'static,
{
    FilesystemTurnStateStoreKind::row(filesystem).with_limits(limits)
}

fn local_dev_extension_installation_state_path(
    profile: RebornCompositionProfile,
    local_runtime_identity: Option<&RebornLocalRuntimeIdentity>,
) -> Result<VirtualPath, RebornBuildError> {
    if !profile.uses_hosted_extension_installation_state() {
        return FilesystemExtensionInstallationStore::default_state_path().map_err(|error| {
            RebornBuildError::InvalidConfig {
                reason: format!("extension installation state path is invalid: {error}"),
            }
        });
    }

    let default_identity = RebornRuntimeIdentity::reborn_cli();
    let default_tenant_id =
        ironclaw_host_api::TenantId::new(default_identity.tenant_id).map_err(|error| {
            RebornBuildError::InvalidConfig {
                reason: error.to_string(),
            }
        })?;
    let tenant_id = local_runtime_identity
        .map(|identity| identity.tenant_id.clone())
        .unwrap_or(default_tenant_id);
    VirtualPath::new(format!(
        "/tenants/{}/system/extensions/.installations/state.json",
        tenant_id.as_str()
    ))
    .map_err(|error| RebornBuildError::InvalidConfig {
        reason: format!("hosted extension installation state path is invalid: {error}"),
    })
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
async fn build_local_dev_store_graph(
    input: RebornLocalDevStoreGraphInput,
) -> Result<RebornLocalDevStoreGraph, RebornBuildError> {
    let RebornLocalDevStoreGraphInput {
        filesystem,
        owner_user_id,
        local_runtime_identity,
        runtime_policy,
        skill_filesystem,
        workspace_filesystem,
        workspace_mounts,
        local_dev_storage_root,
        default_system_prompt_path,
        trigger_repository,
        project_repository,
        turn_state_store_limits,
        #[cfg(feature = "postgres")]
        postgres_resource_governor_singleton,
        #[cfg(feature = "libsql")]
        identity_substrate_db,
    } = input;
    let scoped_filesystem = local_dev_scoped_filesystem(Arc::clone(&filesystem));
    // The turn-state filesystem is needed by both backends: the durable
    // filesystem store persists every transition to it, and the in-memory
    // authority persists only its gate-blocked snapshot to it (persist-on-block
    // durability, so a restart can recover turns parked on a human gate).
    let turn_state_scope =
        local_dev_nearai_mcp_owner_scope(owner_user_id.clone(), local_runtime_identity.as_ref())?;
    let turn_state_filesystem =
        owner_turn_state_filesystem(Arc::clone(&filesystem), &turn_state_scope)
            .map_err(RebornBuildError::Mount)?;
    let event_log = local_dev_event_log(Arc::clone(&filesystem))?;
    let audit_log = local_dev_audit_log(Arc::clone(&filesystem))?;
    let run_state = Arc::new(FilesystemRunStateStore::new(Arc::clone(&scoped_filesystem)));
    let approval_requests = Arc::new(FilesystemApprovalRequestStore::new(Arc::clone(
        &scoped_filesystem,
    )));
    let capability_leases = Arc::new(FilesystemCapabilityLeaseStore::new(Arc::clone(
        &scoped_filesystem,
    )));
    let persistent_approval_policies = Arc::new(FilesystemPersistentApprovalPolicyStore::new(
        Arc::clone(&scoped_filesystem),
    ));
    // Runtime-wedge fix: with `inmemory-turn-state`, the whole local-dev/hosted
    // runtime family (incl. hosted-single-tenant-volume) coordinates turn state
    // in one in-process authority — no per-user `state.json` CAS livelock.
    // Otherwise the durable filesystem row store is used. `LocalDevTurnStateStore`
    // resolves to the matching concrete type via the same feature cfg, so both
    // arms satisfy every downstream consumer (coordinator, `LoopCheckpointStore`,
    // `RuntimeTurnStateStore`) with no trait-object plumbing.
    #[cfg(feature = "inmemory-turn-state")]
    let turn_state = {
        // Persist-on-block: the in-memory authority owns turn state in-process
        // (no per-user `state.json` CAS livelock) but is otherwise volatile.
        // Attach a durable sink that snapshots only when the gate-blocked set
        // changes, and rehydrate from the last such snapshot on startup so a
        // deploy never silently drops a turn parked on approval/auth.
        let block_persistence = Arc::new(FilesystemTurnStateBlockPersistence::new(Arc::clone(
            &turn_state_filesystem,
        )));
        // Fail loud on a real recovery fault. `load()` returns an empty snapshot
        // for the normal missing-snapshot case (fresh volume), so an `Err` here is
        // a genuine read/deserialization/integrity failure — falling back to an
        // empty store would silently drop persisted blocked approvals/auth and
        // defeat the recovery guarantee. Surface it as a build error instead
        // (`RebornBuildError: Turn`), so an operator sees the failure rather than
        // losing gate-parked turns.
        let restored = block_persistence.load().await?;
        let store =
            InMemoryTurnStateStore::from_persistence_snapshot(restored, turn_state_store_limits)?;
        Arc::new(store.with_block_persistence(block_persistence))
    };
    #[cfg(not(feature = "inmemory-turn-state"))]
    let turn_state = Arc::new(production_turn_state_store(
        Arc::clone(&turn_state_filesystem),
        turn_state_store_limits,
    ));
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
    } = build_budget_sinks();
    let budget_gate_store: Arc<dyn BudgetGateStore> = Arc::new(FilesystemBudgetGateStore::new(
        Arc::clone(&scoped_filesystem),
    ));
    #[cfg(feature = "postgres")]
    if let Some(singleton) = postgres_resource_governor_singleton {
        ensure_postgres_resource_governor_authority_for_build(singleton)?;
    }
    let resource_governor = FilesystemResourceGovernor::new(Arc::clone(&scoped_filesystem))
        .with_event_sink(Arc::clone(&budget_event_sink));
    resource_governor.warm_authority()?;
    let resource_governor: Arc<LocalDevResourceGovernor> = Arc::new(resource_governor);
    let skill_mounts =
        skill_management_mount_view().map_err(|error| RebornBuildError::InvalidConfig {
            reason: error.to_string(),
        })?;
    let capability_policy = Arc::new(local_dev_capability_policy().map_err(|error| {
        RebornBuildError::InvalidConfig {
            reason: format!("local-dev capability policy is invalid: {error}"),
        }
    })?);
    let tool_permission_overrides = Arc::new(LocalDevToolPermissionOverrideStore::new(Arc::clone(
        &scoped_filesystem,
    )));
    let auto_approve_settings = Arc::new(LocalDevAutoApproveSettingStore::new(Arc::clone(
        &scoped_filesystem,
    )));
    let memory_mounts =
        memory_mount_view(MountPermissions::read_write_list_delete()).map_err(|error| {
            RebornBuildError::InvalidConfig {
                reason: error.to_string(),
            }
        })?;
    let system_extensions_lifecycle_mounts =
        system_extensions_lifecycle_mount_view().map_err(|error| {
            RebornBuildError::InvalidConfig {
                reason: error.to_string(),
            }
        })?;
    #[cfg(feature = "slack-v2-host-beta")]
    let host_state_filesystem = local_dev_slack_host_state_filesystem(Arc::clone(&filesystem));
    let extension_lifecycle_surface_context = local_dev_extension_lifecycle_surface_context(
        owner_user_id.clone(),
        local_runtime_identity.as_ref(),
    )?;
    let skill_management =
        build_local_skill_management_port(owner_user_id.clone(), Arc::clone(&filesystem))?;
    let outbound_stores = local_dev_outbound_store(Arc::clone(&filesystem));
    let local_runtime = Arc::new(RebornLocalRuntimeServices {
        extension_lifecycle_surface_context,
        owner_user_id: owner_user_id.clone(),
        approval_requests: Arc::clone(&approval_requests),
        capability_leases: Arc::clone(&capability_leases),
        external_tool_catalog: Arc::new(InMemoryExternalToolCatalog::new()),
        runtime_policy,
        capability_policy: Arc::clone(&capability_policy),
        persistent_approval_policies: Arc::clone(&persistent_approval_policies),
        tool_permission_overrides: Arc::clone(&tool_permission_overrides),
        auto_approve_settings: Arc::clone(&auto_approve_settings),
        turn_state: Arc::clone(&turn_state),
        trigger_repository: Arc::clone(&trigger_repository),
        project_service: Arc::new(RebornProjectService::new(Arc::clone(&project_repository))),
        outbound_preferences: outbound_stores.outbound_preferences,
        outbound_delivery_targets: Arc::new(
            crate::outbound::MutableOutboundDeliveryTargetRegistry::default(),
        ),
        skill_auto_activate_learned: Arc::new(AtomicBool::new(true)),
        #[cfg(feature = "slack-v2-host-beta")]
        outbound_state: outbound_stores.outbound_state,
        #[cfg(feature = "slack-v2-host-beta")]
        delivered_gate_routes: outbound_stores.delivered_gate_routes,
        #[cfg(feature = "slack-v2-host-beta")]
        triggered_run_delivery: outbound_stores.triggered_run_delivery,
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
        #[cfg(any(feature = "slack-v2-host-beta", test))]
        channel_connection_facade_slot: Arc::new(std::sync::OnceLock::new()),
        runtime_http_egress: None,
        host_runtime_http_egress: None,
        skill_mounts,
        memory_mounts,
        system_extensions_lifecycle_mounts,
        skill_filesystem,
        workspace_filesystem,
        #[cfg(feature = "slack-v2-host-beta")]
        host_state_filesystem,
        subagent_goal_filesystem: Arc::clone(&scoped_filesystem),
        identity_filesystem: Arc::clone(&scoped_filesystem),
        // Set later in `build_local_runtime`, once the secret-store crypto
        // exists, via `Arc::get_mut` on this services value.
        #[cfg(feature = "webui-v2-beta")]
        admin_secret_provisioner: None,
        #[cfg(feature = "libsql")]
        identity_substrate_db,
        extension_filesystem: Arc::clone(&filesystem),
        workspace_mounts,
        local_dev_storage_root,
        default_system_prompt_path,
        event_log,
        audit_log,
        extension_registry: Arc::new(ExtensionRegistry::new()),
        shared_extension_registry: None,
    });
    let process_services = ProcessServices::filesystem(Arc::clone(&scoped_filesystem));

    Ok(RebornLocalDevStoreGraph {
        run_state,
        approval_requests,
        capability_leases,
        persistent_approval_policies,
        turn_state,
        local_runtime,
        resource_governor,
        process_services,
        trigger_repository,
    })
}

#[cfg(not(any(feature = "libsql", feature = "postgres")))]
async fn build_local_dev_store_graph(
    input: RebornLocalDevStoreGraphInput,
) -> Result<RebornLocalDevStoreGraph, RebornBuildError> {
    let RebornLocalDevStoreGraphInput {
        filesystem,
        owner_user_id,
        local_runtime_identity,
        runtime_policy,
        skill_filesystem,
        workspace_filesystem,
        workspace_mounts,
        local_dev_storage_root,
        default_system_prompt_path,
        trigger_repository,
        project_repository,
        turn_state_store_limits,
    } = input;
    let event_log = local_dev_event_log(Arc::clone(&filesystem))?;
    let audit_log = local_dev_audit_log(Arc::clone(&filesystem))?;
    let run_state = Arc::new(InMemoryRunStateStore::new());
    let approval_requests = Arc::new(InMemoryApprovalRequestStore::new());
    let capability_leases = Arc::new(InMemoryCapabilityLeaseStore::new());
    let persistent_approval_policies = Arc::new(InMemoryPersistentApprovalPolicyStore::new());
    let turn_state = Arc::new(InMemoryTurnStateStore::with_limits(turn_state_store_limits));
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
    } = build_budget_sinks();
    let budget_gate_store: Arc<dyn ironclaw_resources::BudgetGateStore> =
        Arc::new(ironclaw_resources::InMemoryBudgetGateStore::new());
    let resource_governor: Arc<LocalDevResourceGovernor> =
        Arc::new(InMemoryResourceGovernor::new().with_event_sink(Arc::clone(&budget_event_sink)));
    let skill_mounts =
        skill_management_mount_view().map_err(|error| RebornBuildError::InvalidConfig {
            reason: error.to_string(),
        })?;
    let capability_policy = Arc::new(local_dev_capability_policy().map_err(|error| {
        RebornBuildError::InvalidConfig {
            reason: format!("local-dev capability policy is invalid: {error}"),
        }
    })?);
    let tool_permission_overrides = Arc::new(LocalDevToolPermissionOverrideStore::new());
    let auto_approve_settings = Arc::new(LocalDevAutoApproveSettingStore::new());
    let memory_mounts =
        memory_mount_view(MountPermissions::read_write_list_delete()).map_err(|error| {
            RebornBuildError::InvalidConfig {
                reason: error.to_string(),
            }
        })?;
    let system_extensions_lifecycle_mounts =
        system_extensions_lifecycle_mount_view().map_err(|error| {
            RebornBuildError::InvalidConfig {
                reason: error.to_string(),
            }
        })?;
    #[cfg(all(feature = "postgres", feature = "slack-v2-host-beta"))]
    let host_state_filesystem = local_dev_slack_host_state_filesystem(Arc::clone(&filesystem));
    let extension_lifecycle_surface_context = local_dev_extension_lifecycle_surface_context(
        owner_user_id.clone(),
        local_runtime_identity.as_ref(),
    )?;
    let skill_management =
        build_local_skill_management_port(owner_user_id.clone(), Arc::clone(&filesystem))?;
    #[cfg(not(any(feature = "libsql", feature = "postgres")))]
    let trigger_conversation_services = local_dev_trigger_conversation_services();
    let outbound_stores = local_dev_outbound_store(Arc::clone(&filesystem));
    let local_runtime = Arc::new(RebornLocalRuntimeServices {
        extension_lifecycle_surface_context,
        owner_user_id: owner_user_id.clone(),
        approval_requests: Arc::clone(&approval_requests),
        capability_leases: Arc::clone(&capability_leases),
        external_tool_catalog: Arc::new(InMemoryExternalToolCatalog::new()),
        runtime_policy,
        capability_policy: Arc::clone(&capability_policy),
        persistent_approval_policies: Arc::clone(&persistent_approval_policies),
        tool_permission_overrides: Arc::clone(&tool_permission_overrides),
        auto_approve_settings: Arc::clone(&auto_approve_settings),
        turn_state: Arc::clone(&turn_state),
        trigger_repository: Arc::clone(&trigger_repository),
        project_service: Arc::new(RebornProjectService::new(Arc::clone(&project_repository))),
        outbound_preferences: outbound_stores.outbound_preferences,
        outbound_delivery_targets: Arc::new(
            crate::outbound::MutableOutboundDeliveryTargetRegistry::default(),
        ),
        skill_auto_activate_learned: Arc::new(AtomicBool::new(true)),
        #[cfg(feature = "slack-v2-host-beta")]
        outbound_state: outbound_stores.outbound_state,
        #[cfg(feature = "slack-v2-host-beta")]
        delivered_gate_routes: outbound_stores.delivered_gate_routes,
        #[cfg(feature = "slack-v2-host-beta")]
        triggered_run_delivery: outbound_stores.triggered_run_delivery,
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
        #[cfg(any(feature = "slack-v2-host-beta", test))]
        channel_connection_facade_slot: Arc::new(std::sync::OnceLock::new()),
        runtime_http_egress: None,
        host_runtime_http_egress: None,
        skill_mounts,
        memory_mounts,
        system_extensions_lifecycle_mounts,
        skill_filesystem,
        workspace_filesystem,
        extension_filesystem: Arc::clone(&filesystem),
        workspace_mounts,
        local_dev_storage_root,
        default_system_prompt_path,
        event_log,
        audit_log,
        extension_registry: Arc::new(ExtensionRegistry::new()),
        shared_extension_registry: None,
    });
    let process_services = ProcessServices::in_memory();

    Ok(RebornLocalDevStoreGraph {
        run_state,
        approval_requests,
        capability_leases,
        persistent_approval_policies,
        turn_state,
        local_runtime,
        resource_governor,
        process_services,
        trigger_repository,
    })
}

async fn local_dev_trigger_repository(
    backend: &LocalDevDurableBackend,
) -> Result<Arc<dyn TriggerRepository>, RebornBuildError> {
    match backend {
        #[cfg(feature = "libsql")]
        LocalDevDurableBackend::LibSql(database) => {
            let repository = ironclaw_triggers::LibSqlTriggerRepository::new(Arc::clone(database));
            repository
                .run_migrations()
                .await
                .map_err(|error| RebornBuildError::InvalidConfig {
                    reason: format!("local-dev trigger repository migrations failed: {error}"),
                })?;
            Ok(Arc::new(repository))
        }
        #[cfg(feature = "postgres")]
        LocalDevDurableBackend::Postgres(pool) => {
            let repository = ironclaw_triggers::PostgresTriggerRepository::new(pool.clone());
            repository
                .run_migrations()
                .await
                .map_err(|error| RebornBuildError::InvalidConfig {
                    reason: format!("PostgreSQL trigger repository migrations failed: {error}"),
                })?;
            Ok(Arc::new(repository))
        }
        #[cfg(not(feature = "libsql"))]
        LocalDevDurableBackend::Ephemeral => Ok(Arc::new(
            ironclaw_triggers::InMemoryTriggerRepository::default(),
        )),
    }
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
            outbound_delivery_targets: Arc::clone(&local_runtime.outbound_delivery_targets),
        })
    }
}

/// Validate a per-trigger delivery target against the runtime's outbound
/// delivery target registry: the id must resolve for the trigger creator (the
/// same ownership check the delivery layer applies at fire time). Fails
/// closed when no provider is registered or the id is unknown/foreign.
async fn validate_trigger_delivery_target_against_registry(
    registry: &crate::outbound::MutableOutboundDeliveryTargetRegistry,
    scope: &ironclaw_host_api::ResourceScope,
    target: &ironclaw_triggers::TriggerDeliveryTargetId,
) -> Result<(), TriggerError> {
    let invalid = |reason: String| TriggerError::InvalidRecord {
        kind: ironclaw_triggers::TriggerRecordValidationKind::DeliveryTargetInvalid,
        reason,
    };
    let target_id = ironclaw_product_workflow::RebornOutboundDeliveryTargetId::new(target.as_str())
        .map_err(|error| {
            tracing::debug!(
                target = "ironclaw::reborn::trigger_create",
                %error,
                "per-trigger delivery target id failed outbound target id validation"
            );
            invalid("delivery target id is not a valid outbound target id".to_string())
        })?;
    let caller = ironclaw_product_workflow::WebUiAuthenticatedCaller::new(
        scope.tenant_id.clone(),
        scope.user_id.clone(),
        scope.agent_id.clone(),
        scope.project_id.clone(),
    );
    use crate::outbound::OutboundDeliveryTargetProvider as _;
    match registry
        .resolve_outbound_delivery_target(&caller, &target_id)
        .await
    {
        Ok(Some(_)) => Ok(()),
        Ok(None) => Err(invalid(
            "delivery target is not available to this caller".to_string(),
        )),
        Err(error) => {
            tracing::warn!(
                target = "ironclaw::reborn::trigger_create",
                %error,
                "outbound delivery target lookup failed during trigger create validation"
            );
            Err(TriggerError::Backend {
                reason: "outbound delivery target lookup unavailable".to_string(),
            })
        }
    }
}

#[cfg(not(any(feature = "libsql", feature = "postgres")))]
struct InMemoryTriggerCreatorPairingHook {
    conversations: InMemoryConversationServices,
    outbound_delivery_targets: Arc<crate::outbound::MutableOutboundDeliveryTargetRegistry>,
}

#[cfg(not(any(feature = "libsql", feature = "postgres")))]
#[async_trait::async_trait]
impl TriggerCreateHook for InMemoryTriggerCreatorPairingHook {
    async fn validate_delivery_target(
        &self,
        scope: &ironclaw_host_api::ResourceScope,
        target: &ironclaw_triggers::TriggerDeliveryTargetId,
    ) -> Result<(), TriggerError> {
        validate_trigger_delivery_target_against_registry(
            &self.outbound_delivery_targets,
            scope,
            target,
        )
        .await
    }

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
    async fn validate_delivery_target(
        &self,
        scope: &ironclaw_host_api::ResourceScope,
        target: &ironclaw_triggers::TriggerDeliveryTargetId,
    ) -> Result<(), TriggerError> {
        validate_trigger_delivery_target_against_registry(
            &self.runtime.outbound_delivery_targets,
            scope,
            target,
        )
        .await
    }

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
    BudgetSinks {
        budget_event_sink,
        in_memory_budget_event_sink,
        broadcast_budget_event_sink,
    }
}

async fn build_local_dev_root_filesystem(
    root: &Path,
    workspace_root: &Path,
    host_home_root: Option<&LocalDevHostHomeRoot>,
    storage_backend_input: LocalDevStorageBackendInput,
) -> Result<LocalDevRootFilesystemBundle, RebornBuildError> {
    let local = Arc::new(local_dev_project_filesystem(
        root,
        workspace_root,
        host_home_root,
    )?);
    let mut composite = CompositeRootFilesystem::new();
    let durable_backend = match storage_backend_input {
        #[cfg(feature = "postgres")]
        LocalDevStorageBackendInput::Postgres(pool) => {
            let database = Arc::new(PostgresRootFilesystem::new(pool.clone()));
            database.run_migrations().await?;
            mount_local_dev_database_roots(&mut composite, database)?;
            LocalDevDurableBackend::Postgres(pool)
        }
        LocalDevStorageBackendInput::LocalDefault => {
            build_default_local_dev_database_roots(root, &mut composite).await?
        }
    };
    mount_local_dev_project_roots(&mut composite, local)?;
    Ok(LocalDevRootFilesystemBundle {
        filesystem: Arc::new(composite),
        durable_backend,
    })
}

/// Filename of the local-dev libSQL database within the per-user root directory.
/// One owner for the string — production factory, integration-test framework, and
/// any on-disk path assertion all derive from this constant.
#[cfg(any(feature = "libsql", feature = "test-support"))]
pub(crate) const LOCAL_DEV_DB_FILENAME: &str = "reborn-local-dev.db";

/// Open (or create) the local-dev libSQL database file at `root` — just the
/// connection, no migrations/mount. One owner for the `libsql::Builder::new_local`
/// sequence: [`build_default_local_dev_database_roots`] (production) and the
/// C-DURABLE test-support trigger-repository reopen
/// (`open_local_dev_trigger_repository_for_test`) both call this rather than
/// each opening their own connection to the same file.
#[cfg(feature = "libsql")]
async fn open_local_dev_libsql_database(
    root: &Path,
) -> Result<Arc<libsql::Database>, RebornBuildError> {
    let db_path = root.join(LOCAL_DEV_DB_FILENAME);
    Ok(Arc::new(
        libsql::Builder::new_local(&db_path)
            .build()
            .await
            .map_err(|error| RebornBuildError::InvalidConfig {
                reason: format!("local-dev libSQL database could not be opened: {error}"),
            })?,
    ))
}

// `pub(crate)` so the `test_support` accessor
// (`build_default_local_dev_database_roots_for_test`) can call this
// without duplicating the 4-step libSQL setup sequence (Builder →
// LibSqlRootFilesystem → run_migrations → mount). Production callers
// stay inside this module (`build_local_dev_root_filesystem`).
pub(crate) async fn build_default_local_dev_database_roots(
    root: &Path,
    composite: &mut CompositeRootFilesystem,
) -> Result<LocalDevDurableBackend, RebornBuildError> {
    #[cfg(feature = "libsql")]
    {
        let db = open_local_dev_libsql_database(root).await?;
        let database = Arc::new(LibSqlRootFilesystem::new(Arc::clone(&db)));
        database.run_migrations().await?;
        mount_local_dev_database_roots(composite, database)?;
        Ok(LocalDevDurableBackend::LibSql(db))
    }
    #[cfg(not(feature = "libsql"))]
    {
        let _ = root;
        tracing::debug!(
            "local-dev: control-plane filesystem roots are backed by InMemoryBackend; runtime state is ephemeral and will be lost on restart"
        );
        mount_local_dev_database_roots(composite, Arc::new(InMemoryBackend::new()))?;
        Ok(LocalDevDurableBackend::Ephemeral)
    }
}

/// Thin void wrapper over [`build_default_local_dev_database_roots`] for
/// `#[cfg(feature = "test-support")]` callers that need to mount the local-dev
/// database roots but don't need the opaque `LocalDevDurableBackend` handle
/// (which is private to this module).
///
/// Used by `test_support::build_default_local_dev_database_roots_for_test`.
#[cfg(feature = "test-support")]
pub(crate) async fn mount_default_local_dev_database_roots(
    root: &Path,
    composite: &mut CompositeRootFilesystem,
) -> Result<(), RebornBuildError> {
    build_default_local_dev_database_roots(root, composite)
        .await
        .map(|_| ())
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

/// Test-only (C-SLACK-LIFECYCLE restart seam, issue #6105 T5): reopen the
/// composed Slack host-state filesystem at an existing local-dev
/// `storage_root` — a FRESH root filesystem (fresh durable-backend handles,
/// independent of the live runtime's `Arc`s) wrapped with the SAME
/// `slack_host_state_mount_view` production wraps it with. Mirrors the
/// production construction in [`build_reborn_services`]
/// (`local_dev_slack_host_state_filesystem` over the local-dev root), so a
/// restart-survival test proves durable Slack host state (identity bindings,
/// DM targets) is reconstructible the way a real process restart
/// reconstructs it. Tests only; zero bytes in production builds.
///
/// `libsql`-only (not `any(libsql, postgres)`): the body reopens via
/// `LocalDevStorageBackendInput::LocalDefault`, whose non-libsql arm mounts a
/// fresh `InMemoryBackend` — under a postgres-composed runtime that would be
/// a brand-new empty store, not the live Postgres-backed host state, so a
/// postgres gate here would compile a probe that can only report absence.
#[cfg(all(
    feature = "test-support",
    feature = "slack-v2-host-beta",
    feature = "libsql"
))]
pub(crate) async fn open_local_dev_slack_host_state_filesystem_for_test(
    storage_root: &Path,
) -> Result<Arc<ScopedFilesystem<LocalDevRootFilesystem>>, RebornBuildError> {
    let workspace_root = storage_root.join("workspace");
    let bundle = build_local_dev_root_filesystem(
        storage_root,
        &workspace_root,
        None,
        LocalDevStorageBackendInput::LocalDefault,
    )
    .await?;
    Ok(local_dev_slack_host_state_filesystem(bundle.filesystem))
}

/// Test-only (E-DURABLE seam): open a FRESH, independent
/// [`ExtensionInstallationStore`] at an existing local-dev `storage_root`,
/// paralleling how `assert_reply_persists_after_reopen` opens a fresh libsql
/// handle rather than reusing the live one. Reuses the production
/// [`local_dev_project_filesystem`] mounts and [`FilesystemExtensionInstallationStore::default_state_path`]
/// so the reopen reads the exact on-disk `/system/extensions` state the running
/// harness wrote (mirrors the production install-store load in
/// [`build_reborn_services`], above at the `extension_installation_store` binding).
/// The store's virtual state path has no identity dependency for local-dev
/// profiles, so no tenant/user context is needed. Tests only; zero bytes in
/// production builds.
#[cfg(feature = "test-support")]
pub(crate) async fn open_local_dev_extension_installation_store_for_test(
    storage_root: &Path,
) -> Result<Arc<dyn ExtensionInstallationStore>, RebornBuildError> {
    let workspace_root = storage_root.join("workspace");
    let filesystem: Arc<dyn RootFilesystem> = Arc::new(local_dev_project_filesystem(
        storage_root,
        &workspace_root,
        None,
    )?);
    let state_path =
        FilesystemExtensionInstallationStore::default_state_path().map_err(|error| {
            RebornBuildError::InvalidConfig {
                reason: format!("extension installation state path invalid: {error}"),
            }
        })?;
    let store = FilesystemExtensionInstallationStore::load_at(filesystem, state_path)
        .await
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("extension installation state could not be reopened: {error}"),
        })?;
    Ok(Arc::new(store))
}

/// Migration seam: open the extension installation store over a caller-supplied
/// [`RootFilesystem`] at either the legacy default path or a hosted
/// tenant-qualified path, returning the boxed trait object so the migration
/// tool never touches the concrete
/// `pub(crate)` `FilesystemExtensionInstallationStore`. Mirrors the production
/// binding in [`build_reborn_services`] (the `extension_installation_store`
/// construction via `FilesystemExtensionInstallationStore::load_at`); gated
/// behind `migration-support` so it ships zero bytes in a default production
/// binary, exactly like the `test-support` seams above.
///
/// The migration tool owns the cross-stack bridge (it depends on the legacy
/// `ironclaw` crate); keeping this narrow accessor here lets composition retain
/// sole ownership of the installation store's construction without composition
/// itself taking any legacy dependency.
#[cfg(feature = "migration-support")]
pub async fn extension_installation_store_for_migration(
    filesystem: Arc<dyn RootFilesystem>,
    tenant_id: Option<&ironclaw_host_api::TenantId>,
) -> Result<Arc<dyn ExtensionInstallationStore>, RebornBuildError> {
    let state_path = match tenant_id {
        Some(tenant_id) => VirtualPath::new(format!(
            "/tenants/{}/system/extensions/.installations/state.json",
            tenant_id.as_str()
        ))
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("extension installation state path invalid: {error}"),
        })?,
        None => FilesystemExtensionInstallationStore::default_state_path().map_err(|error| {
            RebornBuildError::InvalidConfig {
                reason: format!("extension installation state path invalid: {error}"),
            }
        })?,
    };
    let store = FilesystemExtensionInstallationStore::load_at(filesystem, state_path)
        .await
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("extension installation state could not be loaded: {error}"),
        })?;
    Ok(Arc::new(store))
}

/// Test-only (C-DURABLE seam): open a FRESH, independent
/// [`ironclaw_run_state::ApprovalRequestStore`] at an existing local-dev
/// `storage_root`, paralleling [`open_local_dev_extension_installation_store_for_test`]
/// (same on-disk root; a sibling capability store). Reuses
/// [`mount_default_local_dev_database_roots`] + the production [`crate::wrap_scoped`]
/// so the reopen mounts + scopes the SAME way `build_local_runtime` does when it
/// first builds `approval_requests` — the reopen path never drifts from
/// production. Tests only; zero bytes in production builds.
#[cfg(all(feature = "test-support", feature = "libsql"))]
pub(crate) async fn open_local_dev_approval_request_store_for_test(
    storage_root: &Path,
) -> Result<Arc<dyn ironclaw_run_state::ApprovalRequestStore>, RebornBuildError> {
    let mut composite = CompositeRootFilesystem::new();
    mount_default_local_dev_database_roots(storage_root, &mut composite).await?;
    let scoped = crate::wrap_scoped(Arc::new(composite));
    Ok(Arc::new(FilesystemApprovalRequestStore::new(scoped)))
}

/// W6-COLD-SPOTS: fresh `CommunicationPreferenceRepository` reopen, mirrors
/// [`open_local_dev_approval_request_store_for_test`]. Reuses
/// [`local_dev_outbound_store`] — the same composition-owned construction the
/// production `build_local_dev_store_graph` path uses — so the reopen path
/// never drifts from production and needs no `disallowed_methods` exception.
/// Tests only.
#[cfg(all(feature = "test-support", feature = "libsql"))]
pub(crate) async fn open_local_dev_outbound_preferences_store_for_test(
    storage_root: &Path,
) -> Result<Arc<dyn CommunicationPreferenceRepository>, RebornBuildError> {
    let mut composite = CompositeRootFilesystem::new();
    mount_default_local_dev_database_roots(storage_root, &mut composite).await?;
    Ok(local_dev_outbound_store(Arc::new(composite)).outbound_preferences)
}

/// Test-only (W5-WEBUI-API-1 seam): open FRESH, independent
/// [`ironclaw_approvals::ToolPermissionOverrideStore`] /
/// [`ironclaw_approvals::AutoApproveSettingStore`] /
/// [`ironclaw_approvals::PersistentApprovalPolicyStore`] handles at an
/// existing local-dev `storage_root`, paralleling
/// [`open_local_dev_approval_request_store_for_test`] (same on-disk root;
/// sibling capability stores). Reuses [`mount_default_local_dev_database_roots`]
/// plus the production [`crate::wrap_scoped`] so the reopen mounts and scopes
/// the SAME way `build_local_dev_store_graph` does when it first builds
/// `tool_permission_overrides` / `auto_approve_settings` /
/// `persistent_approval_policies` (above) — the reopen path never drifts from
/// production. Tests only; zero bytes in production builds.
#[cfg(all(feature = "test-support", feature = "libsql"))]
pub(crate) async fn open_local_dev_approval_settings_stores_for_test(
    storage_root: &Path,
) -> Result<
    (
        Arc<dyn ironclaw_approvals::ToolPermissionOverrideStore>,
        Arc<dyn ironclaw_approvals::AutoApproveSettingStore>,
        Arc<dyn ironclaw_approvals::PersistentApprovalPolicyStore>,
    ),
    RebornBuildError,
> {
    let mut composite = CompositeRootFilesystem::new();
    mount_default_local_dev_database_roots(storage_root, &mut composite).await?;
    let scoped = crate::wrap_scoped(Arc::new(composite));
    let tool_permission_overrides: Arc<dyn ironclaw_approvals::ToolPermissionOverrideStore> =
        Arc::new(LocalDevToolPermissionOverrideStore::new(Arc::clone(
            &scoped,
        )));
    let auto_approve_settings: Arc<dyn ironclaw_approvals::AutoApproveSettingStore> =
        Arc::new(LocalDevAutoApproveSettingStore::new(Arc::clone(&scoped)));
    let persistent_approval_policies: Arc<dyn ironclaw_approvals::PersistentApprovalPolicyStore> =
        Arc::new(FilesystemPersistentApprovalPolicyStore::new(scoped));
    Ok((
        tool_permission_overrides,
        auto_approve_settings,
        persistent_approval_policies,
    ))
}

/// Test-only (C-DURABLE seam): open a FRESH, independent
/// [`ironclaw_triggers::TriggerRepository`] at an existing local-dev
/// `storage_root`, paralleling [`open_local_dev_extension_installation_store_for_test`].
/// Reuses [`open_local_dev_libsql_database`] (the same libSQL-open sequence
/// production uses) AND delegates to [`local_dev_trigger_repository`] for
/// repository construction + migrations, so the reopen path shares the SAME
/// construction code as production local-dev wiring — never a second place to
/// update if trigger repository setup changes. Tests only; zero bytes in
/// production builds.
#[cfg(all(feature = "test-support", feature = "libsql"))]
pub(crate) async fn open_local_dev_trigger_repository_for_test(
    storage_root: &Path,
) -> Result<Arc<dyn TriggerRepository>, RebornBuildError> {
    let db = open_local_dev_libsql_database(storage_root).await?;
    local_dev_trigger_repository(&LocalDevDurableBackend::LibSql(db)).await
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

// `pub(crate)` (not private) so the `test_support` accessor
// (`mount_local_dev_database_roots_for_test`) can forward to it across the
// crate boundary for downstream integration tests without a second copy of the
// mount truth. Production callers stay inside this module
// (`build_local_dev_root_filesystem` / `build_default_local_dev_database_roots`).
pub(crate) fn mount_local_dev_database_roots<F>(
    root: &mut CompositeRootFilesystem,
    database: Arc<F>,
) -> Result<(), RebornBuildError>
where
    F: RootFilesystem + 'static,
{
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
    mount_local_dev_memory_root(root, Arc::clone(&database))?;
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
pub(crate) fn build_local_dev_secret_store<F>(
    root: &Path,
    scoped_filesystem: Arc<ScopedFilesystem<F>>,
    explicit_master_key: Option<ironclaw_secrets::SecretMaterial>,
) -> Result<
    (
        Arc<FilesystemSecretStore<F>>,
        Arc<ironclaw_secrets::SecretsCrypto>,
    ),
    RebornBuildError,
>
where
    F: RootFilesystem + 'static,
{
    let master_key = match explicit_master_key {
        Some(master_key) => master_key,
        None => resolve_local_dev_secret_master_key(root)?,
    };
    // The crypto is returned alongside the store so the admin secret
    // provisioner (`admin_secrets.rs`) can build per-target-user stores that
    // share the SAME master key — secrets written admin-side decrypt under the
    // user's own store and vice versa.
    let crypto = Arc::new(ironclaw_secrets::SecretsCrypto::new(master_key)?);
    let store = Arc::new(FilesystemSecretStore::new(
        scoped_filesystem,
        Arc::clone(&crypto),
    ));
    Ok((store, crypto))
}

/// Where a resolved local-dev master key came from, used to name the source in
/// fail-loud error messages.
#[cfg(any(feature = "libsql", feature = "postgres"))]
enum MasterKeySource {
    File(PathBuf),
    Env,
}

/// Validate a resolved master key against the same rules `SecretsCrypto::new`
/// enforces, mapping a rejection to a `RebornBuildError` that names *where the
/// key came from* and the offending path/env var.
///
/// Without this, a corrupt cached key file or a malformed `SECRETS_MASTER_KEY`
/// env value surfaces only as the opaque "Invalid master key" raised several
/// layers deep in `SecretsCrypto::new`, with no pointer to the file the
/// operator must fix. See `.claude/rules/error-handling.md` (fail loud, name
/// the operation).
#[cfg(any(feature = "libsql", feature = "postgres"))]
fn validate_resolved_master_key(
    key: &str,
    source: &MasterKeySource,
) -> Result<(), RebornBuildError> {
    ironclaw_secrets::validate_master_key_material(key.as_bytes()).map_err(|error| {
        let location = match source {
            MasterKeySource::File(path) => format!("file {}", path.display()),
            MasterKeySource::Env => format!(
                "env var {}",
                ironclaw_secrets::keychain::SECRETS_MASTER_KEY_ENV
            ),
        };
        RebornBuildError::InvalidConfig {
            reason: format!(
                "local-dev secrets master key from {location} is malformed: {error}; \
                 it must be at least 32 bytes with at least 8 distinct byte values. \
                 Remove or replace it and retry."
            ),
        }
    })
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
fn resolve_local_dev_secret_master_key(
    root: &Path,
) -> Result<ironclaw_secrets::SecretMaterial, RebornBuildError> {
    // Fail closed on an explicitly-set-but-unusable master key: only an
    // *absent* env var is "not configured". A non-Unicode value must not be
    // silently dropped (via `.ok()`) and fall through to generating a fresh
    // key, which would encrypt local-dev secrets under an unintended key the
    // operator never chose.
    let env_key = match std::env::var(ironclaw_secrets::keychain::SECRETS_MASTER_KEY_ENV) {
        Ok(value) => Some(value),
        Err(std::env::VarError::NotPresent) => None,
        Err(std::env::VarError::NotUnicode(_)) => {
            return Err(RebornBuildError::InvalidConfig {
                reason: format!(
                    "local-dev secrets master key env var {} is set but not valid UTF-8",
                    ironclaw_secrets::keychain::SECRETS_MASTER_KEY_ENV
                ),
            });
        }
    };
    resolve_local_dev_secret_master_key_with_env(root, env_key)
}

/// Inner resolver that takes the `SECRETS_MASTER_KEY` env value as a parameter
/// so the write-before-validate invariant can be exercised through this real
/// caller in tests without mutating process-global env (which is racy under
/// `cargo test`'s parallel harness).
#[cfg(any(feature = "libsql", feature = "postgres"))]
fn resolve_local_dev_secret_master_key_with_env(
    root: &Path,
    env_key: Option<String>,
) -> Result<ironclaw_secrets::SecretMaterial, RebornBuildError> {
    // Fully resolve and VALIDATE an explicitly-set env value UP FRONT, before
    // the cached file read. Otherwise a rebuild where
    // `.reborn-local-dev-secrets-master-key` already exists returns the cached
    // key and silently ignores the operator's bad explicit env config — whether
    // it is empty OR a malformed non-empty value (e.g. `0000...`). Validating
    // here means any explicit-but-unusable env key fails closed regardless of
    // cached state.
    let env_key = match env_key {
        Some(value) => {
            let trimmed = value.trim().to_string();
            if trimmed.is_empty() {
                return Err(RebornBuildError::InvalidConfig {
                    reason: format!(
                        "local-dev secrets master key env var {} is set but empty",
                        ironclaw_secrets::keychain::SECRETS_MASTER_KEY_ENV
                    ),
                });
            }
            validate_resolved_master_key(&trimmed, &MasterKeySource::Env)?;
            Some(trimmed)
        }
        None => None,
    };

    let key_path = root.join(LOCAL_DEV_SECRETS_MASTER_KEY_PATH);
    match std::fs::read_to_string(&key_path) {
        Ok(existing) => {
            let key = existing.trim().to_string();
            validate_resolved_master_key(&key, &MasterKeySource::File(key_path.clone()))?;
            return Ok(ironclaw_secrets::SecretMaterial::from(key));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(RebornBuildError::InvalidConfig {
                reason: format!(
                    "local-dev secrets master key at {} could not be read: {error}",
                    key_path.display()
                ),
            });
        }
    }

    // No cached file. Prefer the explicit (already-validated) env key; otherwise
    // generate a fresh one.
    match env_key {
        Some(key) => {
            write_local_dev_secret_master_key(&key_path, &key)?;
            Ok(ironclaw_secrets::SecretMaterial::from(key))
        }
        None => {
            let key = ironclaw_secrets::keychain::generate_master_key_hex();
            write_local_dev_secret_master_key(&key_path, &key)?;
            Ok(ironclaw_secrets::SecretMaterial::from(key))
        }
    }
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

/// Unified bundle of outbound store handles returned by both cfg variants of
/// [`local_dev_outbound_store`].
///
/// All four trait roles must be satisfied on construction.  In the durable
/// build (libsql or postgres) every role is an `Arc` clone of a single
/// `FilesystemOutboundStateStore`, so the WebUI delivery-defaults facade and
/// the Slack delivery path share one backing tree.  In the non-durable build
/// `InMemoryOutboundStateStore` covers the preference and state roles;
/// `DeliveredGateRouteStore` and `TriggeredRunDeliveryStore` use separate
/// in-memory instances — the cross-store invariant that matters (WebUI-written
/// preferences visible to the Slack triggered-delivery hook) only involves the
/// preference role.
/// See docs/plans/2026-05-29-trigger-loop-delivery-resolution-implementation.md.
pub(crate) struct LocalDevOutboundStores {
    pub(crate) outbound_preferences: Arc<dyn CommunicationPreferenceRepository>,
    #[cfg(feature = "slack-v2-host-beta")]
    pub(crate) outbound_state: Arc<dyn OutboundStateStore>,
    #[cfg(feature = "slack-v2-host-beta")]
    pub(crate) delivered_gate_routes: Arc<dyn DeliveredGateRouteStore>,
    #[cfg(feature = "slack-v2-host-beta")]
    pub(crate) triggered_run_delivery: Arc<dyn TriggeredRunDeliveryStore>,
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
fn local_dev_outbound_store(filesystem: Arc<LocalDevRootFilesystem>) -> LocalDevOutboundStores {
    // One store instance over the composition-owned per-user scoped filesystem
    // (`/outbound` → `/tenants/<t>/users/<u>/outbound`). All four outbound
    // roles — preferences, state, delivered-gate routes, triggered-run delivery
    // — are Arc-cloned from this single instance so the WebUI delivery-defaults
    // facade and the Slack delivery path share the same backing tree.
    // composition-owned construction site, the only one allowed.
    #[allow(clippy::disallowed_methods)]
    let store: Arc<FilesystemOutboundStateStore<LocalDevRootFilesystem>> = Arc::new(
        FilesystemOutboundStateStore::new(local_dev_scoped_filesystem(filesystem)),
    );
    LocalDevOutboundStores {
        outbound_preferences: Arc::clone(&store) as Arc<dyn CommunicationPreferenceRepository>,
        #[cfg(feature = "slack-v2-host-beta")]
        outbound_state: Arc::clone(&store) as Arc<dyn OutboundStateStore>,
        #[cfg(feature = "slack-v2-host-beta")]
        delivered_gate_routes: Arc::clone(&store) as Arc<dyn DeliveredGateRouteStore>,
        #[cfg(feature = "slack-v2-host-beta")]
        triggered_run_delivery: store as Arc<dyn TriggeredRunDeliveryStore>,
    }
}

#[cfg(not(any(feature = "libsql", feature = "postgres")))]
fn local_dev_outbound_store(_filesystem: Arc<LocalDevRootFilesystem>) -> LocalDevOutboundStores {
    // In the non-filesystem (no libsql/postgres) profile, InMemoryOutboundStateStore
    // implements both CommunicationPreferenceRepository and OutboundStateStore, so a
    // single Arc covers both roles.  The other two roles (DeliveredGateRouteStore and
    // TriggeredRunDeliveryStore) are not implemented by InMemoryOutboundStateStore and
    // therefore use separate in-memory instances; this is acceptable because the
    // cross-store invariant that matters — WebUI-written preferences being visible to
    // the Slack triggered-delivery hook — only involves the preference role.  The durable
    // build (libsql or postgres) avoids this gap entirely by sharing one
    // FilesystemOutboundStateStore across all four roles.
    let outbound = Arc::new(InMemoryOutboundStateStore::default());
    LocalDevOutboundStores {
        outbound_preferences: Arc::clone(&outbound) as Arc<dyn CommunicationPreferenceRepository>,
        #[cfg(feature = "slack-v2-host-beta")]
        outbound_state: outbound as Arc<dyn OutboundStateStore>,
        #[cfg(feature = "slack-v2-host-beta")]
        delivered_gate_routes: Arc::new(InMemoryDeliveredGateRouteStore::default()),
        #[cfg(feature = "slack-v2-host-beta")]
        triggered_run_delivery: Arc::new(InMemoryTriggeredRunDeliveryStore::default()),
    }
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

#[cfg(any(feature = "libsql", feature = "postgres"))]
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

#[cfg(any(feature = "libsql", feature = "postgres"))]
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

#[cfg(not(any(feature = "libsql", feature = "postgres")))]
fn local_dev_event_log(
    _filesystem: Arc<LocalDevRootFilesystem>,
) -> Result<Arc<dyn DurableEventLog>, RebornBuildError> {
    Ok(Arc::new(InMemoryDurableEventLog::new()))
}

#[cfg(not(any(feature = "libsql", feature = "postgres")))]
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

#[cfg(any(feature = "libsql", feature = "postgres"))]
fn production_builtin_extension_registry(
    process_backend: ProcessBackendKind,
) -> Result<ExtensionRegistry, RebornBuildError> {
    let mut registry = ExtensionRegistry::new();
    registry
        .insert(
            builtin_first_party_package_for_process_backend(process_backend).map_err(|error| {
                RebornBuildError::InvalidConfig {
                    reason: format!("built-in first-party package is invalid: {error}"),
                }
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
    active_run_lookup: Arc<dyn TriggerActiveRunLookup>,
) -> Result<FirstPartyCapabilityRegistry, RebornBuildError> {
    builtin_first_party_handlers_with_trigger_create_hook(
        trigger_repository,
        trigger_create_hook,
        active_run_lookup,
    )
    .map_err(|error| RebornBuildError::InvalidConfig {
        reason: format!("built-in first-party handlers are invalid: {error}"),
    })
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
fn production_first_party_registry_with_trigger_create_hook(
    trigger_repository: Arc<dyn TriggerRepository>,
    trigger_create_hook: Arc<dyn TriggerCreateHook>,
    active_run_lookup: Arc<dyn TriggerActiveRunLookup>,
    process_backend: ProcessBackendKind,
) -> Result<FirstPartyCapabilityRegistry, RebornBuildError> {
    builtin_first_party_handlers_with_trigger_create_hook_for_process_backend(
        trigger_repository,
        trigger_create_hook,
        active_run_lookup,
        process_backend,
    )
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

pub fn builtin_first_party_trust_policy() -> Result<HostTrustPolicy, RebornBuildError> {
    let policy =
        local_dev_capability_policy().map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("local-dev capability policy is invalid: {error}"),
        })?;
    #[cfg_attr(not(feature = "slack-v2-host-beta"), allow(unused_mut))]
    let mut entries = vec![
        AdminEntry::for_local_manifest(
            policy.provider.id,
            policy.provider.manifest_path,
            None,
            HostTrustAssignment::first_party(),
            // Sourced from local_dev_capability_policy.toml `[provider]
            // authority_effects`, which includes `external_write` — required by
            // builtin.trace_commons.onboard (operator-invite enrollment posts to
            // an external onboarding server).
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
    ];
    #[cfg(feature = "slack-v2-host-beta")]
    entries.push(AdminEntry::for_local_manifest(
        PackageId::new("slack_bot").map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("Slack first-party package id is invalid: {error}"),
        })?,
        "/system/extensions/slack_bot/manifest.toml".to_string(),
        Some(slack_bot_manifest_digest()),
        HostTrustAssignment::first_party(),
        Vec::new(),
        None,
    ));
    #[cfg(feature = "slack-v2-host-beta")]
    entries.push(AdminEntry::for_local_manifest(
        PackageId::new("slack").map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("Slack personal first-party package id is invalid: {error}"),
        })?,
        "/system/extensions/slack/manifest.toml".to_string(),
        Some(slack_manifest_digest()),
        HostTrustAssignment::first_party(),
        slack_user_allowed_effects(),
        None,
    ));
    HostTrustPolicy::new(vec![Box::new(AdminConfig::with_entries(entries))]).map_err(|error| {
        RebornBuildError::InvalidConfig {
            reason: format!("built-in first-party trust policy is invalid: {error}"),
        }
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

#[cfg(feature = "slack-v2-host-beta")]
fn slack_user_allowed_effects() -> Vec<EffectKind> {
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

#[cfg(all(test, any(feature = "libsql", feature = "postgres")))]
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
        local_runtime_identity,
        storage,
        production_trust_policy,
        runtime_policy,
        // The notifier field on `RebornBuildInput` is kept for backward
        // compatibility with test callers that pre-mint one, but the
        // production-shaped build now mints its own notifier internally so the
        // coordinator and scheduler always share the exact same channel.
        turn_run_wake_notifier: _,
        runtime_process_binding,
        required_runtime_backends,
        require_runtime_http_egress,
        require_wasm_credentials,
        #[cfg(all(test, feature = "slack-v2-host-beta"))]
            host_runtime_http_egress_for_test: _,
        #[cfg(any(test, feature = "test-support"))]
            network_http_egress_for_test: _,
        product_auth_ports,
        oauth_provider_configs,
        oauth_dcr_provider_configs,
        #[cfg(feature = "slack-v2-host-beta")]
        slack_personal_oauth_lazy_slot,
        nearai_mcp_bootstrap_config: _,
        turn_state_store_limits,
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
        runtime_process_binding,
        owner_id,
        required_runtime_backends,
        require_runtime_http_egress,
        require_wasm_credentials,
        local_runtime_identity,
        product_auth_ports,
        oauth_provider_configs,
        oauth_dcr_provider_configs,
        turn_state_store_limits,
    );
    #[cfg(feature = "slack-v2-host-beta")]
    let _ = slack_personal_oauth_lazy_slot;

    match storage {
        RebornStorageInput::Disabled | RebornStorageInput::LocalDev { .. } => {
            Err(RebornBuildError::InvalidConfig {
                reason: format!(
                    "profile={} requires durable database-backed Reborn storage",
                    profile
                ),
            })
        }
        #[cfg(feature = "postgres")]
        RebornStorageInput::HostedSingleTenantPostgres { .. } => {
            Err(RebornBuildError::InvalidConfig {
                reason: format!(
                    "profile={} requires production-shaped Reborn storage, not hosted single-tenant Postgres storage",
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
            process_local_resource_governor_singleton,
        } => {
            // Mint the scheduler wake wiring here, before building the coordinator, so:
            // 1. The notifier can satisfy `HostRuntimeServices.with_turn_run_wake_notifier_dyn`
            //    (required by `validate_production_wiring` / `turn_coordinator_for_production`).
            // 2. The wiring is threaded through `RebornServices` →
            //    `DefaultPlannedRuntimeParts.scheduler_wake_wiring` so the
            //    `build_default_planned_runtime` scheduler loop consumes the exact same channel,
            //    ensuring the coordinator's notifier and the scheduler share a live queue.
            let scheduler_wake_wiring = ironclaw_runner::runtime::SchedulerWakeWiring::channel();
            let production_wiring = production_wiring(
                production_trust_policy,
                runtime_policy,
                scheduler_wake_wiring.notifier(),
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
                #[cfg(feature = "slack-v2-host-beta")]
                slack_personal_oauth_lazy_slot,
                owner_id,
                local_runtime_identity,
                turn_state_store_limits,
                scheduler_wake_wiring,
            };
            build_libsql_production(
                context,
                db,
                path_or_url,
                auth_token,
                secret_master_key,
                process_local_resource_governor_singleton,
            )
            .await
        }
        #[cfg(feature = "postgres")]
        RebornStorageInput::Postgres {
            pool,
            url,
            tls_options,
            secret_master_key,
            process_local_resource_governor_singleton,
        } => {
            // Mint the scheduler wake wiring here, before building the coordinator, so:
            // 1. The notifier can satisfy `HostRuntimeServices.with_turn_run_wake_notifier_dyn`
            //    (required by `validate_production_wiring` / `turn_coordinator_for_production`).
            // 2. The wiring is threaded through `RebornServices` →
            //    `DefaultPlannedRuntimeParts.scheduler_wake_wiring` so the
            //    `build_default_planned_runtime` scheduler loop consumes the exact same channel,
            //    ensuring the coordinator's notifier and the scheduler share a live queue.
            let scheduler_wake_wiring = ironclaw_runner::runtime::SchedulerWakeWiring::channel();
            let production_wiring = production_wiring(
                production_trust_policy,
                runtime_policy,
                scheduler_wake_wiring.notifier(),
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
                #[cfg(feature = "slack-v2-host-beta")]
                slack_personal_oauth_lazy_slot,
                owner_id,
                local_runtime_identity,
                turn_state_store_limits,
                scheduler_wake_wiring,
            };
            build_postgres_production(
                context,
                pool,
                url,
                tls_options,
                secret_master_key,
                process_local_resource_governor_singleton,
            )
            .await
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
    turn_run_wake_notifier: Arc<dyn ironclaw_turns::TurnRunWakeNotifier>,
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
    #[cfg(feature = "slack-v2-host-beta")]
    slack_personal_oauth_lazy_slot:
        Option<crate::slack::slack_setup::SlackPersonalSetupServiceSlot>,
    owner_id: String,
    local_runtime_identity: Option<RebornLocalRuntimeIdentity>,
    turn_state_store_limits: ironclaw_turns::InMemoryTurnStateStoreLimits,
    /// The pre-minted scheduler wake wiring to carry to `RebornServices` so
    /// `build_reborn_runtime` can hand it to `build_default_planned_runtime` via
    /// `DefaultPlannedRuntimeParts.scheduler_wake_wiring`.
    scheduler_wake_wiring: ironclaw_runner::runtime::SchedulerWakeWiring,
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
fn production_wiring(
    trust_policy: Option<Arc<HostTrustPolicy>>,
    runtime_policy: Option<EffectiveRuntimePolicy>,
    turn_run_wake_notifier: Arc<ironclaw_runner::turn_scheduler::SchedulerTurnRunWakeNotifier>,
    runtime_process_binding: RebornRuntimeProcessBinding,
) -> Result<RebornProductionWiring, RebornBuildError> {
    let trust_policy = trust_policy.ok_or(RebornBuildError::MissingProductionTrustPolicy)?;
    if !trust_policy.has_sources() {
        return Err(RebornBuildError::EmptyProductionTrustPolicy);
    }
    let runtime_policy = runtime_policy.ok_or(RebornBuildError::MissingRuntimePolicy)?;
    validate_production_process_binding(&runtime_policy, &runtime_process_binding)?;
    let turn_run_wake_notifier: Arc<dyn ironclaw_turns::TurnRunWakeNotifier> =
        turn_run_wake_notifier;
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
        ironclaw_runner::planned_driver_factory::default_planned_run_profile_resolver().map_err(
            |error| RebornBuildError::PlannedRunProfileResolver {
                reason: error.to_string(),
            },
        )?,
    ))
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
type FilesystemProductionHostRuntimeServices<F> = HostRuntimeServices<
    F,
    FilesystemResourceGovernor<F>,
    ironclaw_processes::FilesystemProcessStore<F>,
    ironclaw_processes::FilesystemProcessResultStore<F>,
>;

#[cfg(any(feature = "libsql", feature = "postgres"))]
fn substrate_only_default_owner_id() -> Result<UserId, crate::RebornCompositionError> {
    let identity = RebornRuntimeIdentity::reborn_cli();
    // The substrate-only builders do not receive app/runtime owner input.
    // Preserve their legacy location under the default `reborn-cli` owner.
    UserId::new(identity.tenant_id).map_err(crate::RebornCompositionError::Mount)
}

#[cfg(feature = "libsql")]
pub(crate) async fn build_libsql_production_host_runtime_services<TPolicy, TWake>(
    config: crate::LibSqlProductionSubstrateConfig<TPolicy, TWake>,
) -> Result<crate::LibSqlProductionHostRuntimeServices, crate::RebornCompositionError>
where
    TPolicy: ironclaw_trust::TrustPolicy + 'static,
    TWake: ironclaw_turns::TurnRunWakeNotifier + 'static,
{
    ensure_libsql_resource_governor_authority(config.process_local_resource_governor_singleton)?;
    let filesystem = Arc::new(LibSqlRootFilesystem::new(Arc::clone(&config.database)));
    filesystem.run_migrations().await?;
    let scoped_filesystem = crate::wrap_scoped(Arc::clone(&filesystem));
    let resource_governor = FilesystemResourceGovernor::new(scoped_filesystem);
    build_filesystem_production_host_runtime_services(
        FilesystemProductionHostRuntimeServicesInput {
            filesystem,
            resource_governor,
            event_store: FilesystemProductionEventStoresInput::Config(config.event_store),
            secret_master_key: config.secret_master_key,
            trust_policy: config.trust_policy,
            runtime_policy: config.runtime_policy,
            turn_run_wake_notifier: config.turn_run_wake_notifier,
            surface_version: config.surface_version,
        },
    )
    .await
}

#[cfg(feature = "libsql")]
fn ensure_libsql_resource_governor_authority(
    process_local_singleton: bool,
) -> Result<(), crate::RebornCompositionError> {
    if process_local_singleton {
        return Ok(());
    }
    Err(crate::RebornCompositionError::InvalidConfig {
        reason: "libSQL production FilesystemResourceGovernor uses process-local tallies; configure a singleton or elected resource-governor owner before sharing one database across runtime processes".to_string(),
    })
}

#[cfg(feature = "libsql")]
fn ensure_libsql_resource_governor_authority_for_build(
    process_local_singleton: bool,
) -> Result<(), RebornBuildError> {
    if process_local_singleton {
        return Ok(());
    }
    Err(RebornBuildError::InvalidConfig {
        reason: "libSQL FilesystemResourceGovernor uses process-local tallies; configure a singleton or elected resource-governor owner before sharing one database across runtime processes".to_string(),
    })
}

#[cfg(feature = "postgres")]
pub(crate) async fn build_postgres_production_host_runtime_services<TPolicy, TWake>(
    config: crate::PostgresProductionSubstrateConfig<TPolicy, TWake>,
) -> Result<crate::PostgresProductionHostRuntimeServices, crate::RebornCompositionError>
where
    TPolicy: ironclaw_trust::TrustPolicy + 'static,
    TWake: ironclaw_turns::TurnRunWakeNotifier + 'static,
{
    let pool = config.pool;
    ensure_postgres_resource_governor_authority(config.process_local_resource_governor_singleton)?;
    let filesystem = Arc::new(ironclaw_filesystem::PostgresRootFilesystem::new(
        pool.clone(),
    ));
    ensure_postgres_event_store_config(&config.event_store)?;
    filesystem.run_migrations().await?;
    let resource_governor =
        FilesystemResourceGovernor::new(crate::wrap_scoped(Arc::clone(&filesystem)));
    let event_store = ironclaw_reborn_event_store::build_reborn_event_stores_from_root_filesystem(
        Arc::clone(&filesystem),
    )?;
    build_filesystem_production_host_runtime_services(
        FilesystemProductionHostRuntimeServicesInput {
            filesystem,
            resource_governor,
            event_store: FilesystemProductionEventStoresInput::Prebuilt(event_store),
            secret_master_key: config.secret_master_key,
            trust_policy: config.trust_policy,
            runtime_policy: config.runtime_policy,
            turn_run_wake_notifier: config.turn_run_wake_notifier,
            surface_version: config.surface_version,
        },
    )
    .await
}

#[cfg(feature = "postgres")]
fn ensure_postgres_resource_governor_authority(
    process_local_singleton: bool,
) -> Result<(), crate::RebornCompositionError> {
    if process_local_singleton {
        return Ok(());
    }
    Err(crate::RebornCompositionError::InvalidConfig {
        reason: "Postgres production FilesystemResourceGovernor uses process-local tallies; configure a singleton or elected resource-governor owner before sharing one database across runtime processes".to_string(),
    })
}

#[cfg(feature = "postgres")]
fn ensure_postgres_resource_governor_authority_for_build(
    process_local_singleton: bool,
) -> Result<(), RebornBuildError> {
    if process_local_singleton {
        return Ok(());
    }
    Err(RebornBuildError::InvalidConfig {
        reason: "Postgres FilesystemResourceGovernor uses process-local tallies; configure a singleton or elected resource-governor owner before sharing one database across runtime processes".to_string(),
    })
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
struct FilesystemProductionHostRuntimeServicesInput<F, TPolicy, TWake>
where
    F: RootFilesystem + 'static,
{
    filesystem: Arc<F>,
    resource_governor: FilesystemResourceGovernor<F>,
    event_store: FilesystemProductionEventStoresInput,
    secret_master_key: Option<ironclaw_secrets::SecretMaterial>,
    trust_policy: Arc<TPolicy>,
    runtime_policy: crate::RebornProductionRuntimePolicy,
    turn_run_wake_notifier: Arc<TWake>,
    surface_version: CapabilitySurfaceVersion,
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
enum FilesystemProductionEventStoresInput {
    #[cfg(feature = "libsql")]
    Config(ironclaw_reborn_event_store::RebornEventStoreConfig),
    #[cfg(feature = "postgres")]
    Prebuilt(ironclaw_reborn_event_store::RebornEventStores),
}

#[cfg(feature = "postgres")]
fn ensure_postgres_event_store_config(
    config: &ironclaw_reborn_event_store::RebornEventStoreConfig,
) -> Result<(), crate::RebornCompositionError> {
    match config {
        ironclaw_reborn_event_store::RebornEventStoreConfig::Postgres { .. } => Ok(()),
        #[cfg(feature = "postgres")]
        ironclaw_reborn_event_store::RebornEventStoreConfig::PostgresPool { .. } => Ok(()),
        _ => Err(crate::RebornCompositionError::InvalidConfig {
            reason: "PostgreSQL production substrate requires a PostgreSQL event store".to_string(),
        }),
    }
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
async fn warm_resource_governor_with_error<F, E, J>(
    resource_governor: FilesystemResourceGovernor<F>,
    map_join_error: J,
) -> Result<FilesystemResourceGovernor<F>, E>
where
    F: RootFilesystem + 'static,
    E: From<ironclaw_resources::ResourceError>,
    J: FnOnce(tokio::task::JoinError) -> E,
{
    let resource_governor = tokio::task::spawn_blocking(move || {
        resource_governor.warm_authority()?;
        Ok::<_, ironclaw_resources::ResourceError>(resource_governor)
    })
    .await
    .map_err(map_join_error)??;
    Ok(resource_governor)
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
async fn warm_resource_governor_for_composition<F>(
    resource_governor: FilesystemResourceGovernor<F>,
) -> Result<FilesystemResourceGovernor<F>, crate::RebornCompositionError>
where
    F: RootFilesystem + 'static,
{
    warm_resource_governor_with_error(resource_governor, |error| {
        crate::RebornCompositionError::InvalidConfig {
            reason: format!("resource governor warm-up task failed: {error}"),
        }
    })
    .await
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
async fn build_filesystem_production_host_runtime_services<F, TPolicy, TWake>(
    input: FilesystemProductionHostRuntimeServicesInput<F, TPolicy, TWake>,
) -> Result<FilesystemProductionHostRuntimeServices<F>, crate::RebornCompositionError>
where
    F: RootFilesystem + 'static,
    TPolicy: ironclaw_trust::TrustPolicy + 'static,
    TWake: ironclaw_turns::TurnRunWakeNotifier + 'static,
{
    let FilesystemProductionHostRuntimeServicesInput {
        filesystem,
        resource_governor,
        event_store,
        secret_master_key,
        trust_policy,
        runtime_policy,
        turn_run_wake_notifier,
        surface_version,
    } = input;
    let scoped_filesystem = crate::wrap_scoped(Arc::clone(&filesystem));
    let owner_user_id = substrate_only_default_owner_id()?;
    let owner_scope =
        default_runtime_owner_scope(owner_user_id).map_err(crate::RebornCompositionError::Mount)?;
    let turn_state_filesystem = owner_turn_state_filesystem(Arc::clone(&filesystem), &owner_scope)
        .map_err(crate::RebornCompositionError::Mount)?;
    let turn_state = Arc::new(production_turn_state_store(
        Arc::clone(&turn_state_filesystem),
        ironclaw_turns::InMemoryTurnStateStoreLimits::default(),
    ));
    let process_services = ProcessServices::filesystem(Arc::clone(&scoped_filesystem));
    let secret_credentials = build_filesystem_secret_credential_stores(
        Arc::clone(&scoped_filesystem),
        secret_master_key,
    )
    .await?;
    let resource_governor = warm_resource_governor_for_composition(resource_governor).await?;
    let governor = Arc::new(resource_governor);
    let capability_leases = Arc::new(FilesystemCapabilityLeaseStore::new(Arc::clone(
        &scoped_filesystem,
    )));
    let persistent_approval_policies = Arc::new(FilesystemPersistentApprovalPolicyStore::new(
        Arc::clone(&scoped_filesystem),
    ));
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
    .with_persistent_approval_policies(persistent_approval_policies)
    .with_security_audit_sink(Arc::new(ironclaw_events::TracingSecurityAuditSink))
    .with_secret_store(Arc::clone(&secret_credentials.secret_store))
    .with_credential_broker(secret_credentials.credential_broker)
    .with_turn_run_wake_notifier(turn_run_wake_notifier)
    .with_filesystem_run_state(Arc::clone(&scoped_filesystem))
    .with_turn_state_and_transition_port(turn_state)
    .with_run_profile_resolver(Arc::new(
        ironclaw_runner::planned_driver_factory::default_planned_run_profile_resolver()?,
    ));
    let services = match event_store {
        #[cfg(feature = "libsql")]
        FilesystemProductionEventStoresInput::Config(config) => {
            services
                .with_reborn_event_store_config(
                    ironclaw_reborn_event_store::RebornProfile::Production,
                    config,
                )
                .await?
        }
        #[cfg(feature = "postgres")]
        FilesystemProductionEventStoresInput::Prebuilt(stores) => {
            services.with_production_reborn_event_stores(stores)
        }
    };
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
    resource_governor: FilesystemResourceGovernor<F>,
    leases: Arc<FilesystemCapabilityLeaseStore<F>>,
    persistent_approval_policies: Arc<FilesystemPersistentApprovalPolicyStore<F>>,
    secret_credentials: FilesystemSecretCredentialStores<F>,
    event_store: ironclaw_reborn_event_store::RebornEventStoreConfig,
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
impl<F> ProductionStoreBundle<F>
where
    F: RootFilesystem + 'static,
{
    async fn new(
        filesystem: Arc<F>,
        resource_governor: FilesystemResourceGovernor<F>,
        secret_master_key: ironclaw_secrets::SecretMaterial,
        event_store: ironclaw_reborn_event_store::RebornEventStoreConfig,
    ) -> Result<Self, RebornBuildError> {
        let scoped_filesystem = crate::wrap_scoped(Arc::clone(&filesystem));
        let leases = Arc::new(FilesystemCapabilityLeaseStore::new(Arc::clone(
            &scoped_filesystem,
        )));
        let persistent_approval_policies = Arc::new(FilesystemPersistentApprovalPolicyStore::new(
            Arc::clone(&scoped_filesystem),
        ));
        let secret_credentials = FilesystemSecretCredentialStores::from_master_key(
            Arc::clone(&scoped_filesystem),
            secret_master_key,
        )?;
        let resource_governor = warm_resource_governor_for_build(resource_governor).await?;

        Ok(Self {
            filesystem,
            scoped_filesystem,
            resource_governor,
            leases,
            persistent_approval_policies,
            secret_credentials,
            event_store,
        })
    }
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
async fn warm_resource_governor_for_build<F>(
    resource_governor: FilesystemResourceGovernor<F>,
) -> Result<FilesystemResourceGovernor<F>, RebornBuildError>
where
    F: RootFilesystem + 'static,
{
    warm_resource_governor_with_error(resource_governor, |error| RebornBuildError::InvalidConfig {
        reason: format!("resource governor warm-up task failed: {error}"),
    })
    .await
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
    production_runtime_services: impl FnOnce(
        Arc<RebornProductionRuntimeStoreGraph<F>>,
    ) -> RebornProductionRuntimeServices,
    // Leader lock for the background credential keepalive worker. The worker
    // uses this to elect one process per tick as the sweep leader. `None`
    // pool → always-leader (libsql / single-process). Stays private.
    leader_lock: crate::product_auth::credentials::product_auth_refresh_lock::CredentialRefreshLeaderLock,
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
        #[cfg(feature = "slack-v2-host-beta")]
        slack_personal_oauth_lazy_slot,
        owner_id,
        local_runtime_identity,
        turn_state_store_limits,
        scheduler_wake_wiring,
    } = context;
    let owner_user_id = UserId::new(owner_id).map_err(|error| RebornBuildError::InvalidConfig {
        reason: error.to_string(),
    })?;
    let turn_state_scope = match local_runtime_identity.as_ref() {
        Some(identity) => configured_runtime_owner_scope(owner_user_id.clone(), identity),
        None => {
            default_runtime_owner_scope(owner_user_id.clone()).map_err(RebornBuildError::Mount)?
        }
    };
    let turn_state_filesystem =
        owner_turn_state_filesystem(Arc::clone(&stores.filesystem), &turn_state_scope)
            .map_err(RebornBuildError::Mount)?;
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
    let process_backend = production_wiring.runtime_policy.process_backend;
    let extension_registry = production_builtin_extension_registry(process_backend)?;
    let extension_registry = Arc::new(extension_registry);
    let BudgetSinks {
        budget_event_sink,
        broadcast_budget_event_sink,
        ..
    } = build_budget_sinks();
    let turn_state = Arc::new(production_turn_state_store(
        Arc::clone(&turn_state_filesystem),
        turn_state_store_limits,
    ));
    let checkpoint_state_store: Arc<dyn CheckpointStateStore> = Arc::new(
        FilesystemCheckpointStateStore::new(Arc::clone(&stores.scoped_filesystem)),
    );
    let thread_service: Arc<dyn SessionThreadService> = Arc::new(
        FilesystemSessionThreadService::new(Arc::clone(&stores.scoped_filesystem)),
    );
    let resource_governor = Arc::new(
        stores
            .resource_governor
            .with_event_sink(Arc::clone(&budget_event_sink)),
    );
    let production_resource_governor: Arc<dyn ResourceGovernor> = resource_governor.clone();
    let budget_gate_store: Arc<dyn BudgetGateStore> = Arc::new(FilesystemBudgetGateStore::new(
        Arc::clone(&stores.scoped_filesystem),
    ));
    let event_stores = ironclaw_reborn_event_store::build_reborn_event_stores(
        profile.to_event_store_profile(),
        stores.event_store,
    )
    .await?;
    let event_log = Arc::clone(&event_stores.events);
    let audit_log = Arc::clone(&event_stores.audit);
    let production_runtime_graph = Arc::new(RebornProductionRuntimeStoreGraph {
        scoped_filesystem: Arc::clone(&stores.scoped_filesystem),
        extension_registry: Arc::clone(&extension_registry),
        turn_state: Arc::clone(&turn_state),
        checkpoint_state_store: Arc::clone(&checkpoint_state_store),
        thread_service,
        trigger_repository: Arc::clone(&trigger_repository),
        resource_governor: production_resource_governor,
        budget_gate_store,
        broadcast_budget_event_sink,
        event_log,
        audit_log,
    });
    let production_runtime = production_runtime_services(production_runtime_graph);
    // Same store-backed lookup the WebUI automations panel builds via
    // `RebornProductionRuntimeServices::turn_run_snapshot_source` (#5886).
    let trigger_active_run_lookup: Arc<dyn TriggerActiveRunLookup> = Arc::new(
        crate::automation::trigger_poller::SnapshotActiveRunLookup::new(
            production_runtime.turn_run_snapshot_source(),
        ),
    );
    let mut first_party_registry = production_first_party_registry_with_trigger_create_hook(
        trigger_repository,
        trigger_create_hook,
        trigger_active_run_lookup,
        process_backend,
    )?;
    let product_auth_filesystem = Arc::clone(&stores.scoped_filesystem);
    let services = HostRuntimeServices::new(
        Arc::clone(&extension_registry),
        Arc::clone(&stores.filesystem),
        Arc::new(InMemoryResourceGovernor::new()),
        Arc::new(ironclaw_authorization::GrantAuthorizer::new()),
        ProcessServices::filesystem(Arc::clone(&stores.scoped_filesystem)),
        CapabilitySurfaceVersion::new("reborn-app-v1")?,
    )
    .with_trust_policy(production_wiring.trust_policy)
    .with_runtime_policy(production_wiring.runtime_policy)
    .with_capability_leases(stores.leases)
    .with_persistent_approval_policies(stores.persistent_approval_policies)
    .with_secret_store(Arc::clone(&stores.secret_credentials.secret_store))
    .with_credential_broker(stores.secret_credentials.credential_broker)
    .with_security_audit_sink(Arc::new(ironclaw_events::TracingSecurityAuditSink))
    .try_with_host_http_egress_with_body_store(
        ironclaw_network::PolicyNetworkHttpEgress::new(
            ironclaw_network::ReqwestNetworkTransport::default(),
        ),
        Arc::clone(&stores.scoped_filesystem),
    )?
    .with_resource_governor(Arc::clone(&resource_governor))
    .with_production_reborn_event_stores(event_stores)
    .with_filesystem_run_state(Arc::clone(&stores.scoped_filesystem))
    .with_turn_state_and_transition_port(Arc::clone(&turn_state))
    .with_run_profile_resolver(planned_run_profile_resolver()?)
    .with_turn_run_wake_notifier_dyn(production_wiring.turn_run_wake_notifier);
    let product_auth_runtime_ports = require_product_auth_runtime_ports(&services)?;
    let services = attach_hosted_mcp_runtime(services)?;
    let provider_composition = compose_provider_client(
        oauth_provider_configs,
        oauth_dcr_provider_configs,
        Arc::clone(&secret_store),
        product_auth_runtime_ports.clone(),
        #[cfg(feature = "slack-v2-host-beta")]
        slack_personal_oauth_lazy_slot,
        #[cfg(not(feature = "slack-v2-host-beta"))]
        None,
    )?;
    let services = apply_production_runtime_process_binding(
        services,
        production_wiring.runtime_process_binding,
    );
    let security_audit_sink = services.security_audit_sink();

    let turn_coordinator: Arc<dyn ironclaw_turns::TurnCoordinator> =
        Arc::new(services.turn_coordinator_for_production()?);
    // B1: track the durable FilesystemAuthProductServices so the credential-
    // refresh worker can enumerate candidates across all owners.  When a
    // caller pre-supplies product_auth_ports, we do not create a durable
    // instance here, so the candidate source is None (worker finds no
    // candidates, which is safe for override/test callers).
    let credential_refresh_candidate_source: Option<
        Arc<dyn crate::product_auth::credentials::credential_refresh_worker::CredentialRefreshCandidateSource>,
    >;
    let product_auth_ports = match product_auth_ports {
        Some(ports) => {
            credential_refresh_candidate_source = None;
            ports
        }
        None => {
            let durable = Arc::new(FilesystemAuthProductServices::new_with_root(
                product_auth_filesystem,
                Arc::clone(&stores.filesystem),
                Arc::clone(&secret_store),
            ));
            credential_refresh_candidate_source = Some(Arc::clone(&durable)
                as Arc<dyn crate::product_auth::credentials::credential_refresh_worker::CredentialRefreshCandidateSource>);
            RebornProductAuthServicePorts::from_shared_with_provider(
                durable,
                provider_composition
                    .client
                    .clone()
                    .unwrap_or_else(|| Arc::new(UnavailableAuthProviderClient)),
            )
        }
    };
    let product_auth_services =
        compose_product_auth_services(ProductAuthServicesCompositionInput {
            ports: product_auth_ports,
            turn_coordinator: turn_coordinator.clone(),
            blocked_auth_snapshot_source: Some(Arc::clone(&turn_state)
                as Arc<dyn crate::blocked_auth_resume::BlockedAuthSnapshotSource>),
            lifecycle: Arc::new(OnceLock::new()),
            provider_composition,
            security_audit_sink,
            secret_store: Arc::clone(&secret_store),
            // Host-managed NEAR AI MCP fallback is wired only by
            // `build_local_runtime`'s local-dev/hosted-single-tenant path today;
            // preserves this builder's prior behavior of never attaching it.
            nearai_mcp_host_managed_scope: None,
        })?;
    // Bundle the keepalive worker deps so they are wired all-or-nothing. The
    // candidate source is present only when this path built a durable instance
    // (no caller-supplied product_auth_ports); the leader lock and refresh port
    // are always available here.
    let credential_refresh_worker = match credential_refresh_candidate_source {
        Some(candidate_source) => CredentialRefreshWorkerReady::Ready {
            candidate_source,
            leader_lock,
            refresh_port: Arc::clone(&product_auth_services),
        },
        None => CredentialRefreshWorkerReady::Absent,
    };
    let product_auth_ready = true;
    // Wire ProductAuthAccount runtime credential resolver before
    // host_runtime_for_production so WASM extensions whose manifest declares a
    // ProductAuthAccount runtime credential source resolve through
    // CredentialAccountService. Unconditional in production: product_auth_services
    // always exists (durable filesystem fallback from #4234).
    let services = services.with_runtime_credential_account_resolver(Arc::new(
        ProductAuthRuntimeCredentialResolver::new_with_refresh(
            product_auth_services.runtime_credential_account_selection_service(),
            product_auth_services.runtime_credential_account_refresh_service(),
        ),
    ));
    let services = attach_wasm_runtime(services)?;
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

    #[cfg(any(test, feature = "test-support"))]
    let local_dev_wasm_runtime_credential_provider_captured =
        services.wasm_runtime_credential_provider_captured_for_test();
    let host_runtime: Arc<dyn ironclaw_host_runtime::HostRuntime> =
        Arc::new(services.host_runtime_for_production(&wiring_config)?);

    Ok(RebornServices {
        host_runtime: Some(host_runtime),
        turn_coordinator: Some(turn_coordinator),
        readiness: readiness_for(profile, true, true, product_auth_ready),
        product_auth: Some(product_auth_services),
        skill_management: Some(skill_management),
        local_runtime: None,
        #[cfg(any(feature = "libsql", feature = "postgres"))]
        production_runtime: Some(production_runtime),
        #[cfg(any(feature = "libsql", feature = "postgres"))]
        production_scheduler_wake: Some(scheduler_wake_wiring),
        secret_store,
        #[cfg(any(test, feature = "test-support"))]
        local_dev_wasm_runtime_credential_provider_captured,
        // `Ready` only when this path built a durable candidate source (i.e. no
        // caller-supplied product_auth_ports override); `Absent` otherwise. The
        // leader lock is always available on this production path.
        credential_refresh_worker,
    })
}

#[cfg(feature = "libsql")]
async fn build_libsql_production(
    context: RebornProductionBuildContext,
    db: Arc<libsql::Database>,
    path_or_url: String,
    auth_token: Option<ironclaw_secrets::SecretMaterial>,
    secret_master_key: ironclaw_secrets::SecretMaterial,
    process_local_resource_governor_singleton: bool,
) -> Result<RebornServices, RebornBuildError> {
    use ironclaw_filesystem::LibSqlRootFilesystem;

    ensure_libsql_resource_governor_authority_for_build(process_local_resource_governor_singleton)?;
    let filesystem = Arc::new(LibSqlRootFilesystem::new(Arc::clone(&db)));
    filesystem.run_migrations().await?;
    let trigger_repository = Arc::new(ironclaw_triggers::LibSqlTriggerRepository::new(db));
    trigger_repository
        .run_migrations()
        .await
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("libSQL trigger repository migrations failed: {error}"),
        })?;
    let resource_governor =
        FilesystemResourceGovernor::new(crate::wrap_scoped(Arc::clone(&filesystem)));
    let stores = ProductionStoreBundle::new(
        filesystem,
        resource_governor,
        secret_master_key,
        ironclaw_reborn_event_store::RebornEventStoreConfig::Libsql {
            path_or_url,
            auth_token,
        },
    )
    .await?;

    build_backend_production(
        context,
        stores,
        trigger_repository,
        RebornProductionRuntimeServices::LibSql,
        {
            #[cfg(feature = "postgres")]
            {
                crate::product_auth::credentials::product_auth_refresh_lock::CredentialRefreshLeaderLock::new(None)
            }
            #[cfg(not(feature = "postgres"))]
            {
                crate::product_auth::credentials::product_auth_refresh_lock::CredentialRefreshLeaderLock::always_leader()
            }
        },
    )
    .await
}

#[cfg(feature = "postgres")]
async fn build_postgres_production(
    context: RebornProductionBuildContext,
    pool: deadpool_postgres::Pool,
    _url: ironclaw_secrets::SecretMaterial,
    _tls_options: ironclaw_reborn_event_store::PostgresPoolTlsOptions,
    secret_master_key: ironclaw_secrets::SecretMaterial,
    process_local_resource_governor_singleton: bool,
) -> Result<RebornServices, RebornBuildError> {
    use ironclaw_filesystem::PostgresRootFilesystem;

    ensure_postgres_resource_governor_authority_for_build(
        process_local_resource_governor_singleton,
    )?;
    // A4: Clone the pool before it is moved into PostgresTriggerRepository so we
    // can thread it to the credential keepalive worker as a leader-lock for
    // sweep serialization.
    // This clone stays PRIVATE — it is never exposed through any public facade.
    let pool_for_refresh_lock = pool.clone();
    let filesystem = Arc::new(PostgresRootFilesystem::new(pool.clone()));
    filesystem.run_migrations().await?;
    let trigger_repository = Arc::new(ironclaw_triggers::PostgresTriggerRepository::new(
        pool.clone(),
    ));
    trigger_repository
        .run_migrations()
        .await
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("PostgreSQL trigger repository migrations failed: {error}"),
        })?;
    let resource_governor =
        FilesystemResourceGovernor::new(crate::wrap_scoped(Arc::clone(&filesystem)));
    let stores = ProductionStoreBundle::new(
        filesystem,
        resource_governor,
        secret_master_key,
        ironclaw_reborn_event_store::RebornEventStoreConfig::PostgresPool { pool },
    )
    .await?;

    build_backend_production(
        context,
        stores,
        trigger_repository,
        RebornProductionRuntimeServices::Postgres,
        crate::product_auth::credentials::product_auth_refresh_lock::CredentialRefreshLeaderLock::new(Some(
            pool_for_refresh_lock,
        )),
    )
    .await
}

fn readiness_for(
    profile: RebornCompositionProfile,
    host_runtime: bool,
    turn_coordinator: bool,
    product_auth: bool,
) -> RebornReadiness {
    let (state, diagnostics) = crate::readiness::readiness_contract_for_profile(profile);

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
        diagnostics,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_approvals::{AutoApproveSettingInput, AutoApproveSettingStore};
    use ironclaw_auth::{
        AuthProductScope, AuthSurface, CredentialAccountLabel, CredentialAccountStatus,
        CredentialOwnership, GOOGLE_CALENDAR_EVENTS_SCOPE, GOOGLE_GMAIL_SEND_SCOPE,
        NewCredentialAccount, ProviderScope,
    };
    use ironclaw_authorization::{CapabilityLeaseStatus, CapabilityLeaseStore, GrantAuthorizer};
    use ironclaw_filesystem::FilesystemError;
    #[cfg(any(feature = "libsql", feature = "postgres"))]
    use ironclaw_filesystem::{
        DirEntry, FileStat, FilesystemOperation, RootFilesystem, VersionedEntry,
    };
    use ironclaw_host_api::{
        CapabilityGrant, CapabilityGrantId, CapabilityId, CapabilitySet, EffectKind,
        ExecutionContext, ExtensionId, GrantConstraints, InvocationId, MountAlias, MountGrant,
        MountPermissions, NetworkPolicy, NetworkScheme, NetworkTargetPattern, Principal,
        ResourceEstimate, ResourceScope, ResourceUsage, RuntimeKind, ScopedPath, SecretHandle,
        TenantId, TrustClass, UserId, VirtualPath,
    };
    #[cfg(any(feature = "libsql", feature = "postgres"))]
    use ironclaw_host_api::{
        RuntimeCredentialAccountProviderId, RuntimeCredentialAccountSetup,
        RuntimeCredentialRequirementSource,
    };
    use ironclaw_host_runtime::{
        MEMORY_SEARCH_CAPABILITY_ID, MEMORY_TREE_CAPABILITY_ID, MEMORY_WRITE_CAPABILITY_ID,
        RuntimeCapabilityOutcome, RuntimeCapabilityRequest, RuntimeFailureKind,
        SKILL_INSTALL_CAPABILITY_ID, SKILL_LIST_CAPABILITY_ID, SKILL_REMOVE_CAPABILITY_ID,
        TRIGGER_CREATE_CAPABILITY_ID, TRIGGER_LIST_CAPABILITY_ID, TRIGGER_REMOVE_CAPABILITY_ID,
    };
    #[cfg(any(feature = "libsql", feature = "postgres"))]
    use ironclaw_host_runtime::{
        RuntimeCredentialAccountRequest, RuntimeCredentialAccountResolver,
    };
    use ironclaw_product_workflow::{LifecyclePackageKind, LifecyclePackageRef, LifecyclePhase};
    use ironclaw_trust::{AuthorityCeiling, EffectiveTrustClass, TrustDecision, TrustProvenance};
    #[cfg(any(feature = "libsql", feature = "postgres"))]
    use rust_decimal_macros::dec;
    #[cfg(feature = "libsql")]
    use secrecy::ExposeSecret;

    use crate::extension_host::extension_lifecycle::ExtensionActivationMode;
    use crate::local_dev_capability_policy::{
        LocalDevApprovalPolicyAction, LocalDevCapabilityPolicyError,
    };
    use crate::{
        RebornReadinessDiagnostic, RebornReadinessState, runtime::SKILL_ACTIVATE_CAPABILITY_ID,
    };

    #[cfg(feature = "libsql")]
    #[test]
    fn libsql_build_resource_governor_guard_requires_singleton_authority() {
        assert!(ensure_libsql_resource_governor_authority_for_build(true).is_ok());
        assert!(matches!(
            ensure_libsql_resource_governor_authority_for_build(false),
            Err(RebornBuildError::InvalidConfig { reason })
                if reason.contains("libSQL FilesystemResourceGovernor uses process-local tallies")
        ));
    }

    #[cfg(any(feature = "libsql", feature = "postgres"))]
    #[tokio::test]
    async fn production_turn_state_store_uses_row_layout() {
        let view = MountView::new(vec![MountGrant::new(
            MountAlias::new("/turns").expect("turns mount alias"),
            VirtualPath::new("/turns").expect("turns virtual path"),
            MountPermissions::read_write_list_delete(),
        )])
        .expect("mount view");
        let filesystem = Arc::new(ScopedFilesystem::with_fixed_view(
            Arc::new(ironclaw_filesystem::InMemoryBackend::new()),
            view,
        ));

        let store = production_turn_state_store(
            filesystem,
            ironclaw_turns::InMemoryTurnStateStoreLimits::default(),
        );

        assert!(matches!(store, FilesystemTurnStateStoreKind::Row(_)));
    }

    #[cfg(any(feature = "libsql", feature = "postgres"))]
    #[test]
    fn build_reborn_services_uses_filesystem_resource_governor() {
        let dir = tempfile::tempdir().expect("tempdir");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");

        let services = runtime
            .block_on(build_reborn_services(RebornBuildInput::local_dev(
                "resource-governor-enabled-env-owner",
                dir.path().join("local-dev"),
            )))
            .expect("local-dev services build");
        let local_runtime = services.local_runtime.as_ref().expect("local runtime");
        let scope = ResourceScope {
            tenant_id: TenantId::new("resource-governor-tenant").expect("tenant"),
            user_id: UserId::new("resource-governor-user").expect("user"),
            agent_id: None,
            project_id: None,
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        };
        let account = ironclaw_resources::ResourceAccount::tenant(scope.tenant_id.clone());

        let reservation = local_runtime
            .resource_governor
            .reserve(scope, ResourceEstimate::default().set_usd(dec!(0.10)))
            .expect("reservation");
        local_runtime
            .resource_governor
            .reconcile(reservation.id, ResourceUsage::default().set_usd(dec!(0.10)))
            .expect("reconcile");

        assert_eq!(
            local_runtime
                .resource_governor
                .usage_for(&account)
                .expect("usage")
                .usd,
            dec!(0.10)
        );
    }

    #[test]
    fn extension_installation_state_path_stays_legacy_for_local_dev() {
        let path =
            local_dev_extension_installation_state_path(RebornCompositionProfile::LocalDev, None)
                .expect("state path");

        assert_eq!(
            path.as_str(),
            "/system/extensions/.installations/state.json"
        );
    }

    #[test]
    fn extension_installation_state_path_uses_durable_tenant_root_for_hosted() {
        let identity = RebornLocalRuntimeIdentity {
            tenant_id: TenantId::new("acme").expect("tenant id"),
            agent_id: ironclaw_host_api::AgentId::new("agent").expect("agent id"),
        };
        let path = local_dev_extension_installation_state_path(
            RebornCompositionProfile::HostedSingleTenant,
            Some(&identity),
        )
        .expect("state path");

        assert_eq!(
            path.as_str(),
            "/tenants/acme/system/extensions/.installations/state.json"
        );
    }

    #[test]
    fn extension_installation_state_path_uses_durable_tenant_root_for_hosted_volume() {
        let identity = RebornLocalRuntimeIdentity {
            tenant_id: TenantId::new("acme").expect("tenant id"),
            agent_id: ironclaw_host_api::AgentId::new("agent").expect("agent id"),
        };
        let path = local_dev_extension_installation_state_path(
            RebornCompositionProfile::HostedSingleTenantVolume,
            Some(&identity),
        )
        .expect("state path");

        assert_eq!(
            path.as_str(),
            "/tenants/acme/system/extensions/.installations/state.json"
        );
    }

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

        async fn pair_external_actor_with_epoch(
            &self,
            _tenant_id: TenantId,
            _adapter_kind: AdapterKind,
            _adapter_installation_id: AdapterInstallationId,
            _external_actor_ref: ExternalActorRef,
            _user_id: UserId,
            _binding_epoch: ironclaw_conversations::ExternalActorBindingEpoch,
        ) -> Result<(), ironclaw_conversations::InboundTurnError> {
            Err(ironclaw_conversations::InboundTurnError::DurableState {
                reason: "raw durable store error".to_string(),
            })
        }

        async fn unpair_external_actor(
            &self,
            _tenant_id: TenantId,
            _adapter_kind: AdapterKind,
            _adapter_installation_id: AdapterInstallationId,
            _external_actor_ref: ExternalActorRef,
        ) -> Result<(), ironclaw_conversations::InboundTurnError> {
            Err(ironclaw_conversations::InboundTurnError::DurableState {
                reason: "raw durable store error".to_string(),
            })
        }

        async fn unpair_external_actor_if_owned_by(
            &self,
            _tenant_id: &TenantId,
            _adapter_kind: &AdapterKind,
            _adapter_installation_id: &AdapterInstallationId,
            _external_actor_ref: &ExternalActorRef,
            _expected: &ironclaw_conversations::ExpectedExternalActorOwner,
        ) -> Result<
            ironclaw_conversations::ConditionalUnpairOutcome,
            ironclaw_conversations::InboundTurnError,
        > {
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

    /// Per-trigger delivery targets validate against the SAME registry the
    /// outbound target surface publishes from: an id a provider resolves for
    /// the caller is accepted; an unknown id (or an empty registry) fails
    /// closed as `DeliveryTargetInvalid`.
    #[tokio::test]
    async fn trigger_delivery_target_validation_resolves_through_the_outbound_registry() {
        use crate::outbound::outbound_preferences::OutboundDeliveryTargetEntry;
        use crate::outbound::{
            MutableOutboundDeliveryTargetRegistry, OutboundDeliveryTargetProvider,
        };
        use ironclaw_product_workflow::{
            RebornOutboundDeliveryTargetCapabilities, RebornOutboundDeliveryTargetId,
            RebornOutboundDeliveryTargetSummary, RebornServicesError, WebUiAuthenticatedCaller,
        };

        struct OneTargetProvider {
            entry: OutboundDeliveryTargetEntry,
        }

        #[async_trait::async_trait]
        impl OutboundDeliveryTargetProvider for OneTargetProvider {
            async fn list_outbound_delivery_targets(
                &self,
                _caller: &WebUiAuthenticatedCaller,
            ) -> Result<Vec<OutboundDeliveryTargetEntry>, RebornServicesError> {
                Ok(vec![OutboundDeliveryTargetEntry {
                    summary: self.entry.summary.clone(),
                    capabilities: self.entry.capabilities.clone(),
                    reply_target_binding_ref: self.entry.reply_target_binding_ref.clone(),
                }])
            }
        }

        let scope = ironclaw_host_api::ResourceScope {
            tenant_id: TenantId::new("registry-validation-tenant").expect("tenant"),
            user_id: UserId::new("registry-validation-user").expect("user"),
            agent_id: None,
            project_id: None,
            mission_id: None,
            thread_id: None,
            invocation_id: ironclaw_host_api::InvocationId::new(),
        };
        let target = ironclaw_triggers::TriggerDeliveryTargetId::new("slack:personal-dm:T1:me")
            .expect("target id");

        let registry = MutableOutboundDeliveryTargetRegistry::default();
        // Empty registry → fail closed.
        let rejected =
            validate_trigger_delivery_target_against_registry(&registry, &scope, &target)
                .await
                .expect_err("empty registry must reject");
        assert!(matches!(
            rejected,
            TriggerError::InvalidRecord {
                kind: ironclaw_triggers::TriggerRecordValidationKind::DeliveryTargetInvalid,
                ..
            }
        ));

        // Registered provider that resolves the id for the caller → accept.
        let entry = OutboundDeliveryTargetEntry {
            summary: RebornOutboundDeliveryTargetSummary::new(
                RebornOutboundDeliveryTargetId::new("slack:personal-dm:T1:me").expect("id"),
                "slack",
                "Slack DM".to_string(),
                None,
            )
            .expect("summary"),
            capabilities: RebornOutboundDeliveryTargetCapabilities {
                final_replies: true,
                gate_prompts: true,
                auth_prompts: true,
            },
            reply_target_binding_ref: ironclaw_turns::ReplyTargetBindingRef::new(
                "reply:registry-validation",
            )
            .expect("binding ref"),
        };
        registry
            .register_provider("test", Arc::new(OneTargetProvider { entry }))
            .expect("register");
        validate_trigger_delivery_target_against_registry(&registry, &scope, &target)
            .await
            .expect("registered target must validate");

        // A different id still fails closed.
        let other = ironclaw_triggers::TriggerDeliveryTargetId::new("slack:personal-dm:T1:other")
            .expect("target id");
        validate_trigger_delivery_target_against_registry(&registry, &scope, &other)
            .await
            .expect_err("unknown target must reject");
    }

    fn trigger_record_for_pairing_test() -> TriggerRecord {
        TriggerRecord {
            trigger_id: ironclaw_triggers::TriggerId::new(),
            tenant_id: TenantId::new("pairing-test-tenant").expect("tenant id"),
            creator_user_id: UserId::new("pairing-test-user").expect("user id"),
            agent_id: None,
            project_id: None,
            name: "pairing test".to_string(),
            source: ironclaw_triggers::TriggerSourceKind::Schedule,
            schedule: ironclaw_triggers::TriggerSchedule::cron("* * * * *")
                .expect("valid cron expression"),
            prompt: "pairing test prompt".to_string(),
            delivery_target: None,
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
            extension_lifecycle_surface_context: base_runtime
                .extension_lifecycle_surface_context
                .clone(),
            owner_user_id: base_runtime.owner_user_id.clone(),
            approval_requests: Arc::clone(&base_runtime.approval_requests),
            capability_leases: Arc::clone(&base_runtime.capability_leases),
            external_tool_catalog: Arc::clone(&base_runtime.external_tool_catalog),
            runtime_policy: base_runtime.runtime_policy.clone(),
            capability_policy: Arc::clone(&base_runtime.capability_policy),
            persistent_approval_policies: Arc::clone(&base_runtime.persistent_approval_policies),
            tool_permission_overrides: Arc::clone(&base_runtime.tool_permission_overrides),
            outbound_delivery_targets: Arc::clone(&base_runtime.outbound_delivery_targets),
            auto_approve_settings: Arc::clone(&base_runtime.auto_approve_settings),
            turn_state: Arc::clone(&base_runtime.turn_state),
            trigger_repository: Arc::clone(&base_runtime.trigger_repository),
            project_service: Arc::clone(&base_runtime.project_service),
            outbound_preferences: Arc::clone(&base_runtime.outbound_preferences),
            skill_auto_activate_learned: Arc::clone(&base_runtime.skill_auto_activate_learned),
            #[cfg(feature = "slack-v2-host-beta")]
            outbound_state: Arc::clone(&base_runtime.outbound_state),
            #[cfg(feature = "slack-v2-host-beta")]
            delivered_gate_routes: Arc::clone(&base_runtime.delivered_gate_routes),
            #[cfg(feature = "slack-v2-host-beta")]
            triggered_run_delivery: Arc::clone(&base_runtime.triggered_run_delivery),
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
            #[cfg(any(feature = "slack-v2-host-beta", test))]
            channel_connection_facade_slot: Arc::clone(
                &base_runtime.channel_connection_facade_slot,
            ),
            runtime_http_egress: base_runtime.runtime_http_egress.clone(),
            host_runtime_http_egress: base_runtime.host_runtime_http_egress.clone(),
            skill_mounts: base_runtime.skill_mounts.clone(),
            memory_mounts: base_runtime.memory_mounts.clone(),
            system_extensions_lifecycle_mounts: base_runtime
                .system_extensions_lifecycle_mounts
                .clone(),
            skill_filesystem: Arc::clone(&base_runtime.skill_filesystem),
            workspace_filesystem: Arc::clone(&base_runtime.workspace_filesystem),
            #[cfg(feature = "slack-v2-host-beta")]
            host_state_filesystem: Arc::clone(&base_runtime.host_state_filesystem),
            #[cfg(any(feature = "libsql", feature = "postgres"))]
            identity_filesystem: Arc::clone(&base_runtime.identity_filesystem),
            #[cfg(feature = "webui-v2-beta")]
            admin_secret_provisioner: base_runtime.admin_secret_provisioner.clone(),
            #[cfg(feature = "libsql")]
            identity_substrate_db: base_runtime.identity_substrate_db.clone(),
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
            shared_extension_registry: base_runtime.shared_extension_registry.clone(),
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
    async fn hosted_single_tenant_rejects_local_dev_storage_input() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut input = RebornBuildInput::local_dev(
            "hosted-single-tenant-local-storage-owner",
            dir.path().join("local-dev"),
        );
        input.profile = RebornCompositionProfile::HostedSingleTenant;

        let error = match build_reborn_services(input).await {
            Ok(_) => {
                panic!("hosted single-tenant must use hosted single-tenant Postgres storage")
            }
            Err(error) => error,
        };
        let RebornBuildError::InvalidConfig { reason } = error else {
            panic!("expected invalid config, got {error:?}");
        };
        assert!(
            reason.contains("hosted single-tenant Postgres storage input"),
            "reason: {reason}"
        );
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
        invoke_json(
            &services,
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
            &services,
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
            &services,
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
        invoke_json(
            &services,
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

        let tree = invoke_json(
            &rebuilt,
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
            &rebuilt,
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
            LocalDevStorageBackendInput::LocalDefault,
        )
        .await
        .expect("local-dev filesystem rebuild")
        .filesystem;
        let (rebuilt_secret_store, _rebuilt_secret_crypto) = build_local_dev_secret_store(
            &local_dev_root,
            local_dev_scoped_filesystem(rebuilt_filesystem),
            None,
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

    /// Caller-level regression for the Slack durable conversation store mount
    /// alias. `slack_host_state_mount_view` has a unit test for the grant set,
    /// but the grant only matters through the composed path: production wraps
    /// the local-dev root filesystem with that view via
    /// `local_dev_slack_host_state_filesystem`, then opens the store with
    /// `RebornFilesystemConversationServices::new`, whose init reads
    /// `/conversations/state.json`. Without the `/conversations` alias the
    /// ScopedFilesystem rejects that path ("no mount alias matches scoped
    /// path"), init fails, and every inbound Slack DM is silently dropped (the
    /// bug fixed in 7917cf89f). This drives that exact composition so a future
    /// edit that drops the alias fails here, not just in the mount-view unit
    /// test the composition never consults directly.
    #[cfg(all(
        any(feature = "libsql", feature = "postgres"),
        feature = "slack-v2-host-beta"
    ))]
    #[tokio::test]
    async fn slack_durable_conversation_store_initializes_through_composed_host_state_mount() {
        let dir = tempfile::tempdir().expect("tempdir");
        let local_dev_root = dir.path().join("local-dev");
        // `local_dev_project_filesystem` mounts these host paths; they must
        // exist before the root filesystem is built.
        std::fs::create_dir_all(local_dev_root.join("workspace")).expect("workspace dir");
        std::fs::create_dir_all(local_dev_root.join("system/extensions"))
            .expect("system extensions dir");

        let root_filesystem = build_local_dev_root_filesystem(
            &local_dev_root,
            &local_dev_root.join("workspace"),
            None,
            LocalDevStorageBackendInput::LocalDefault,
        )
        .await
        .expect("local-dev root filesystem")
        .filesystem;

        // Exactly how production composes the Slack host-state filesystem.
        let host_state_filesystem = local_dev_slack_host_state_filesystem(root_filesystem);

        let conversations = RebornFilesystemConversationServices::new(host_state_filesystem).await;

        assert!(
            conversations.is_ok(),
            "durable conversation store must open `/conversations/state.json` \
             through the composed Slack host-state mount view; got {:?}",
            conversations.err()
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

    /// A corrupt local-dev key file must fail loud with a path-naming error,
    /// not the opaque "Invalid master key" that surfaces when the unvalidated
    /// material reaches `SecretsCrypto::new` several layers deep. Mirrors the
    /// real all-zeros key an `[env] SECRETS_MASTER_KEY = "000...0"` cargo
    /// override writes into the cached key file.
    #[cfg(any(feature = "libsql", feature = "postgres"))]
    #[test]
    fn resolve_local_dev_secret_master_key_rejects_malformed_file_with_path_context() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        let key_path = root.join(LOCAL_DEV_SECRETS_MASTER_KEY_PATH);
        // 64 zero chars: passes the length floor but has a single distinct
        // byte, which `SecretsCrypto::new` rejects on the entropy check.
        std::fs::write(&key_path, "0".repeat(64)).expect("write malformed key");

        let error = resolve_local_dev_secret_master_key(root)
            .expect_err("malformed local-dev master key must be rejected");

        match error {
            RebornBuildError::InvalidConfig { reason } => {
                assert!(
                    reason.contains(&key_path.display().to_string()),
                    "error must name the offending key file path, got: {reason}"
                );
                assert!(
                    reason.contains("master key"),
                    "error must mention the master key, got: {reason}"
                );
            }
            other => panic!("expected InvalidConfig, got {other:?}"),
        }
    }

    /// An explicit but malformed `SECRETS_MASTER_KEY` env value (the actual
    /// root cause of the original report) must fail loud and name the env var.
    /// Driven through the real caller `resolve_local_dev_secret_master_key`
    /// (via its env-parameterized inner) so this also guards the
    /// write-before-validate invariant: a rejected env key must never be
    /// persisted to the cached `.reborn-local-dev-secrets-master-key` file.
    #[cfg(any(feature = "libsql", feature = "postgres"))]
    #[test]
    fn resolve_local_dev_secret_master_key_rejects_malformed_env_without_persisting() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        let key_path = root.join(LOCAL_DEV_SECRETS_MASTER_KEY_PATH);
        assert!(
            !key_path.exists(),
            "precondition: cached key file must not exist yet"
        );

        // 64 zero chars: passes the length floor but has a single distinct byte,
        // so the entropy check rejects it.
        let error = resolve_local_dev_secret_master_key_with_env(root, Some("0".repeat(64)))
            .expect_err("malformed env master key must be rejected");

        match error {
            RebornBuildError::InvalidConfig { reason } => {
                assert!(
                    reason.contains(ironclaw_secrets::keychain::SECRETS_MASTER_KEY_ENV),
                    "error must name the env var, got: {reason}"
                );
                assert!(
                    reason.contains("master key"),
                    "error must mention the master key, got: {reason}"
                );
            }
            other => panic!("expected InvalidConfig, got {other:?}"),
        }

        // Write-before-validate regression guard: the rejected key must NOT have
        // been persisted to the cached file.
        assert!(
            !key_path.exists(),
            "rejected env master key must not be persisted to {}",
            key_path.display()
        );
    }

    #[test]
    #[cfg(any(feature = "libsql", feature = "postgres"))]
    fn resolve_local_dev_secret_master_key_rejects_set_but_empty_env_without_persisting() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        let key_path = root.join(LOCAL_DEV_SECRETS_MASTER_KEY_PATH);

        // A set-but-empty (or whitespace-only) env value is explicit-but-unusable
        // configuration: it must fail closed, NOT collapse to "absent" and
        // generate + persist a fresh key the operator never chose.
        for empty in ["", "   ", "\n\t "] {
            let error = resolve_local_dev_secret_master_key_with_env(root, Some(empty.to_string()))
                .expect_err("set-but-empty env master key must be rejected");
            match error {
                RebornBuildError::InvalidConfig { reason } => assert!(
                    reason.contains(ironclaw_secrets::keychain::SECRETS_MASTER_KEY_ENV),
                    "error must name the env var, got: {reason}"
                ),
                other => panic!("expected InvalidConfig, got {other:?}"),
            }
            assert!(
                !key_path.exists(),
                "a set-but-empty env master key must not generate/persist a key at {}",
                key_path.display()
            );
        }
    }

    #[test]
    #[cfg(any(feature = "libsql", feature = "postgres"))]
    fn resolve_local_dev_secret_master_key_rejects_empty_env_even_with_cached_file() {
        // Regression: the empty-env rejection must run BEFORE the cached-file
        // read, so an explicitly-set-but-empty SECRETS_MASTER_KEY fails closed
        // on a rebuild even when `.reborn-local-dev-secrets-master-key` already
        // exists — it must not be silently ignored in favor of the cached key.
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        let key_path = root.join(LOCAL_DEV_SECRETS_MASTER_KEY_PATH);

        // Seed a valid cached key first (no env value -> generated + persisted).
        resolve_local_dev_secret_master_key_with_env(root, None)
            .expect("seed a valid cached master key");
        assert!(key_path.exists(), "precondition: cached key file exists");
        let cached_before = std::fs::read_to_string(&key_path).expect("read cached key");

        let error = resolve_local_dev_secret_master_key_with_env(root, Some("   ".to_string()))
            .expect_err("empty env must fail closed even with a cached file");
        match error {
            RebornBuildError::InvalidConfig { reason } => assert!(
                reason.contains(ironclaw_secrets::keychain::SECRETS_MASTER_KEY_ENV),
                "error must name the env var, got: {reason}"
            ),
            other => panic!("expected InvalidConfig, got {other:?}"),
        }
        // The cached key is left untouched (not silently returned, not rewritten).
        assert_eq!(
            std::fs::read_to_string(&key_path).expect("read cached key"),
            cached_before,
            "the cached key must be left unchanged when the env value is rejected"
        );
    }

    #[test]
    #[cfg(any(feature = "libsql", feature = "postgres"))]
    fn resolve_local_dev_secret_master_key_rejects_malformed_env_even_with_cached_file() {
        // A non-empty-but-malformed env value must also fail closed BEFORE the
        // cached-file read, so `SECRETS_MASTER_KEY=0000...` is not silently
        // ignored in favor of a valid cached key on a rebuild.
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        let key_path = root.join(LOCAL_DEV_SECRETS_MASTER_KEY_PATH);

        resolve_local_dev_secret_master_key_with_env(root, None)
            .expect("seed a valid cached master key");
        let cached_before = std::fs::read_to_string(&key_path).expect("read cached key");

        // 64 zero chars: passes the length floor but fails the entropy check.
        let error = resolve_local_dev_secret_master_key_with_env(root, Some("0".repeat(64)))
            .expect_err("malformed env must fail closed even with a cached file");
        match error {
            RebornBuildError::InvalidConfig { reason } => assert!(
                reason.contains(ironclaw_secrets::keychain::SECRETS_MASTER_KEY_ENV),
                "error must name the env var, got: {reason}"
            ),
            other => panic!("expected InvalidConfig, got {other:?}"),
        }
        assert_eq!(
            std::fs::read_to_string(&key_path).expect("read cached key"),
            cached_before,
            "the cached key must be left unchanged when a malformed env value is rejected"
        );
    }

    /// A well-formed cached key file passes through unchanged.
    #[cfg(any(feature = "libsql", feature = "postgres"))]
    #[test]
    fn resolve_local_dev_secret_master_key_accepts_valid_cached_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        let valid = ironclaw_secrets::keychain::generate_master_key_hex();
        std::fs::write(root.join(LOCAL_DEV_SECRETS_MASTER_KEY_PATH), &valid)
            .expect("write valid key");

        resolve_local_dev_secret_master_key(root).expect("valid cached key must be accepted");
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
            .install(
                gmail_ref.clone(),
                extension_management.tenant_operator_user_id_for_test(),
            )
            .await
            .expect("install Gmail");
        extension_management
            .activate_with_prechecked_credentials_for_test(
                gmail_ref,
                ExtensionActivationMode::Static,
            )
            .await
            .expect("activate Gmail");
        extension_management
            .install(
                calendar_ref.clone(),
                extension_management.tenant_operator_user_id_for_test(),
            )
            .await
            .expect("install Google Calendar");
        extension_management
            .activate_with_prechecked_credentials_for_test(
                calendar_ref,
                ExtensionActivationMode::Static,
            )
            .await
            .expect("activate Google Calendar");

        let gmail_context = gsuite_context("gmail.send_message");
        let gmail_scope = gmail_context.resource_scope.clone();
        let gmail_capability =
            CapabilityId::new("gmail.send_message").expect("valid Gmail capability id");
        assert!(matches!(
            local_runtime.capability_policy.lease_approval_for(
                LocalDevApprovalPolicyAction::Dispatch {
                    capability: &gmail_capability,
                },
                &local_runtime.workspace_mounts,
                &local_runtime.skill_mounts,
                &local_runtime.memory_mounts,
                &local_runtime.system_extensions_lifecycle_mounts,
            ),
            Err(LocalDevCapabilityPolicyError::MissingGrant { .. })
        ));
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

        disable_global_auto_approve(local_runtime, &gmail_context).await;
        let failure = invoke_json(
            &services,
            "gmail.send_message",
            gmail_context,
            serde_json::json!({ "message": { "raw": "base64url-rfc822" } }),
        )
        .await
        .expect_err("missing token should fail after approval resume");
        assert_ne!(failure, RuntimeFailureKind::Authorization);
        assert_ne!(failure, RuntimeFailureKind::MissingRuntime);
        let gmail_leases = local_runtime
            .capability_leases
            .leases_for_scope(&gmail_scope)
            .await;
        assert_eq!(gmail_leases.len(), 1);
        assert_eq!(gmail_leases[0].grant.issued_by, Principal::HostRuntime);
        assert_eq!(gmail_leases[0].grant.constraints.max_invocations, Some(1));
        assert_eq!(gmail_leases[0].status, CapabilityLeaseStatus::Revoked);

        let calendar_context = gsuite_context("google-calendar.create_event");
        disable_global_auto_approve(local_runtime, &calendar_context).await;
        let failure = invoke_json(
            &services,
            "google-calendar.create_event",
            calendar_context,
            serde_json::json!({
                "calendar_id": "primary",
                "event": { "summary": "Review" }
            }),
        )
        .await
        .expect_err("missing token should fail after approval resume");
        assert_ne!(failure, RuntimeFailureKind::Authorization);
        assert_ne!(failure, RuntimeFailureKind::MissingRuntime);
    }

    #[tokio::test]
    async fn local_dev_notion_mcp_installs_activates_and_reaches_auth_gate() {
        let dir = tempfile::tempdir().expect("tempdir");
        let services = build_reborn_services(
            RebornBuildInput::local_dev_with_profile(
                RebornCompositionProfile::LocalDevYolo,
                "local-dev-notion-mcp-owner",
                dir.path().join("local-dev"),
            )
            .with_runtime_policy(local_dev_minimal_approval_policy()),
        )
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
            .install(
                notion_ref.clone(),
                extension_management.tenant_operator_user_id_for_test(),
            )
            .await
            .expect("install Notion MCP");
        extension_management
            .activate_with_prechecked_credentials_for_test(
                notion_ref,
                ExtensionActivationMode::Static,
            )
            .await
            .expect("activate Notion MCP");

        let context = notion_mcp_context("notion.notion-search");
        enable_global_auto_approve_for_context(local_runtime, &context).await;
        let outcome = services
            .host_runtime
            .as_ref()
            .expect("host runtime")
            .invoke_capability(RuntimeCapabilityRequest::new(
                context,
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
        let services = build_reborn_services(
            RebornBuildInput::local_dev_with_profile(
                RebornCompositionProfile::LocalDevYolo,
                "local-dev-web-access-owner",
                dir.path().join("local-dev"),
            )
            .with_runtime_policy(local_dev_minimal_approval_policy()),
        )
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
            .install(
                web_access_ref.clone(),
                extension_management.tenant_operator_user_id_for_test(),
            )
            .await
            .expect("install Web Access");
        extension_management
            .activate_with_prechecked_credentials_for_test(
                web_access_ref,
                ExtensionActivationMode::Static,
            )
            .await
            .expect("activate Web Access");

        let context = web_access_context("web-access.search");
        enable_global_auto_approve_for_context(local_runtime, &context).await;
        let outcome = services
            .host_runtime
            .as_ref()
            .expect("host runtime")
            .invoke_capability(RuntimeCapabilityRequest::new(
                context,
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
        // A capability the model named with no registered first-party handler
        // is a model-fixable, model-visible failure (#5389 reclassified the
        // missing-handler dispatch failure from Backend to InvalidInput so it
        // does not burn the retry budget on a call that can never resolve). The
        // capability still fails closed — only the disposition changed.
        assert_eq!(failure.kind, RuntimeFailureKind::InvalidInput);
    }

    fn nearai_bootstrap_input_with_base(
        owner: &str,
        root: PathBuf,
        base_url: &str,
        api_key: &str,
    ) -> RebornBuildInput {
        RebornBuildInput::local_dev(owner, root).with_nearai_mcp_bootstrap_config(
            crate::llm_admin::nearai_mcp::NearAiMcpBootstrapConfig::new(
                base_url,
                secrecy::SecretString::from(api_key.to_string()),
            )
            .expect("valid NEAR AI MCP bootstrap config"),
        )
    }

    fn nearai_bootstrap_input(owner: &str, root: PathBuf, api_key: &str) -> RebornBuildInput {
        nearai_bootstrap_input_with_base(owner, root, "https://private.near.ai", api_key)
    }

    #[test]
    fn hosted_single_tenant_nearai_mcp_bootstrap_scope_uses_runtime_identity() {
        let owner = UserId::new("hosted-nearai-owner").expect("owner");
        let identity = RebornLocalRuntimeIdentity {
            tenant_id: ironclaw_host_api::TenantId::new("hosted-nearai-tenant").expect("tenant"),
            agent_id: ironclaw_host_api::AgentId::new("hosted-nearai-agent").expect("agent"),
        };

        let scope = local_dev_nearai_mcp_owner_scope(owner.clone(), Some(&identity))
            .expect("hosted NEAR AI bootstrap scope");

        assert_eq!(scope.tenant_id, identity.tenant_id);
        assert_eq!(scope.user_id, owner);
        assert_eq!(scope.agent_id, Some(identity.agent_id));
        assert!(scope.project_id.is_none());
    }

    #[cfg(any(feature = "libsql", feature = "postgres"))]
    #[test]
    fn turn_state_filesystem_routes_global_store_ops_to_owner_turns_path() {
        let root = Arc::new(ironclaw_filesystem::InMemoryBackend::default());
        let owner_scope = ResourceScope {
            tenant_id: TenantId::new("tenant-alpha").expect("tenant"),
            user_id: UserId::new("owner-alpha").expect("owner"),
            agent_id: Some(ironclaw_host_api::AgentId::new("agent-alpha").expect("agent")),
            project_id: None,
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        };
        let scoped =
            owner_turn_state_filesystem(root, &owner_scope).expect("owner turn-state filesystem");
        let path = ScopedPath::new("/turns/state.json").expect("turn state path");
        let resolved = scoped
            .resolve(&ResourceScope::system(), &path)
            .expect("fixed view should resolve global store operation");

        assert_eq!(
            resolved.as_str(),
            "/tenants/tenant-alpha/users/owner-alpha/turns/state.json"
        );
    }

    #[cfg(any(feature = "libsql", feature = "postgres"))]
    #[test]
    fn runtime_owner_scope_uses_configured_runtime_identity_for_turn_state() {
        let owner = UserId::new("configured-owner").expect("owner");
        let identity = RebornLocalRuntimeIdentity {
            tenant_id: TenantId::new("configured-tenant").expect("tenant"),
            agent_id: ironclaw_host_api::AgentId::new("configured-agent").expect("agent"),
        };
        let scope = configured_runtime_owner_scope(owner.clone(), &identity);

        assert_eq!(scope.tenant_id, identity.tenant_id);
        assert_eq!(scope.user_id, owner);
        assert_eq!(scope.agent_id, Some(identity.agent_id));
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn production_libsql_turn_state_uses_configured_runtime_identity() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Arc::new(
            libsql::Builder::new_local(dir.path().join("reborn.db").display().to_string())
                .build()
                .await
                .expect("build libsql database"),
        );
        let assertion_filesystem = LibSqlRootFilesystem::new(Arc::clone(&db));
        let owner = UserId::new("configured-owner").expect("owner");
        let tenant = TenantId::new("configured-tenant").expect("tenant");
        let agent = ironclaw_host_api::AgentId::new("configured-agent").expect("agent");
        let services = build_reborn_services(
            RebornBuildInput::libsql(
                RebornCompositionProfile::Production,
                owner.as_str(),
                db,
                dir.path().join("events.db").display().to_string(),
                None,
                ironclaw_secrets::SecretMaterial::from("01234567890123456789012345678901"),
            )
            .with_local_runtime_identity(tenant.clone(), agent.clone())
            .with_production_trust_policy(Arc::new(
                builtin_first_party_trust_policy().expect("builtin trust policy"),
            ))
            .with_runtime_policy(EffectiveRuntimePolicy {
                deployment: ironclaw_host_api::DeploymentMode::HostedMultiTenant,
                requested_profile: ironclaw_host_api::RuntimeProfile::HostedSafe,
                resolved_profile: ironclaw_host_api::RuntimeProfile::HostedSafe,
                filesystem_backend: FilesystemBackendKind::TenantWorkspace,
                process_backend: ProcessBackendKind::None,
                network_mode: ironclaw_host_api::NetworkMode::Brokered,
                secret_mode: SecretMode::TenantBroker,
                approval_policy: ironclaw_host_api::runtime_policy::ApprovalPolicy::AskAlways,
                audit_mode: ironclaw_host_api::AuditMode::Standard,
            }),
        )
        .await
        .expect("production libsql services build");

        let production_runtime = services
            .production_runtime
            .as_ref()
            .expect("production runtime");
        #[cfg(not(feature = "postgres"))]
        let RebornProductionRuntimeServices::LibSql(graph) = production_runtime;
        #[cfg(feature = "postgres")]
        let graph = match production_runtime {
            RebornProductionRuntimeServices::LibSql(graph) => graph,
            RebornProductionRuntimeServices::Postgres(_) => {
                panic!("expected libsql production runtime")
            }
        };
        let scope = ironclaw_turns::TurnScope::new_with_owner(
            tenant,
            Some(agent),
            None,
            ironclaw_host_api::ThreadId::new("configured-thread").expect("thread"),
            Some(owner.clone()),
        );
        let submit = ironclaw_turns::SubmitTurnRequest {
            requested_model: None,
            scope,
            actor: ironclaw_turns::TurnActor::new(owner),
            accepted_message_ref: ironclaw_turns::AcceptedMessageRef::new("configured-message-ref")
                .expect("message ref"),
            source_binding_ref: ironclaw_turns::SourceBindingRef::new("source-web")
                .expect("source binding"),
            reply_target_binding_ref: ironclaw_turns::ReplyTargetBindingRef::new("reply-web")
                .expect("reply binding"),
            requested_run_profile: Some(
                ironclaw_turns::RunProfileRequest::new("default").expect("run profile"),
            ),
            idempotency_key: ironclaw_turns::IdempotencyKey::new("configured-turn")
                .expect("idempotency key"),
            received_at: chrono::Utc::now(),
            requested_run_id: None,
            parent_run_id: None,
            subagent_depth: 0,
            spawn_tree_root_run_id: None,
            product_context: None,
        };
        ironclaw_turns::TurnStateStore::submit_turn(
            graph.turn_state.as_ref(),
            submit,
            &ironclaw_turns::AllowAllTurnAdmissionPolicy,
            &InMemoryRunProfileResolver::default(),
        )
        .await
        .expect("submit through production turn-state store");

        let configured_path = VirtualPath::new(
            "/tenants/configured-tenant/users/configured-owner/turns/rows/v1/deltas/log",
        )
        .expect("configured turn-state row delta log path");
        let system_path =
            VirtualPath::new("/tenants/__system__/users/__system__/turns/rows/v1/deltas/log")
                .expect("system turn-state row delta log path");

        assert!(
            append_log_has_entries(
                &assertion_filesystem,
                &configured_path,
                "configured turn-state row delta log read"
            )
            .await
        );
        assert!(
            !append_log_has_entries(
                &assertion_filesystem,
                &system_path,
                "system turn-state row delta log read"
            )
            .await
        );
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn production_libsql_turn_state_uses_default_runtime_identity_when_unconfigured() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Arc::new(
            libsql::Builder::new_local(dir.path().join("reborn.db").display().to_string())
                .build()
                .await
                .expect("build libsql database"),
        );
        let assertion_filesystem = LibSqlRootFilesystem::new(Arc::clone(&db));
        let owner = UserId::new("default-owner").expect("owner");
        let services = build_reborn_services(
            RebornBuildInput::libsql(
                RebornCompositionProfile::Production,
                owner.as_str(),
                db,
                dir.path().join("events.db").display().to_string(),
                None,
                ironclaw_secrets::SecretMaterial::from("01234567890123456789012345678901"),
            )
            .with_production_trust_policy(Arc::new(
                builtin_first_party_trust_policy().expect("builtin trust policy"),
            ))
            .with_runtime_policy(EffectiveRuntimePolicy {
                deployment: ironclaw_host_api::DeploymentMode::HostedMultiTenant,
                requested_profile: ironclaw_host_api::RuntimeProfile::HostedSafe,
                resolved_profile: ironclaw_host_api::RuntimeProfile::HostedSafe,
                filesystem_backend: FilesystemBackendKind::TenantWorkspace,
                process_backend: ProcessBackendKind::None,
                network_mode: ironclaw_host_api::NetworkMode::Brokered,
                secret_mode: SecretMode::TenantBroker,
                approval_policy: ironclaw_host_api::runtime_policy::ApprovalPolicy::AskAlways,
                audit_mode: ironclaw_host_api::AuditMode::Standard,
            }),
        )
        .await
        .expect("production libsql services build");

        let production_runtime = services
            .production_runtime
            .as_ref()
            .expect("production runtime");
        #[cfg(not(feature = "postgres"))]
        let RebornProductionRuntimeServices::LibSql(graph) = production_runtime;
        #[cfg(feature = "postgres")]
        let graph = match production_runtime {
            RebornProductionRuntimeServices::LibSql(graph) => graph,
            RebornProductionRuntimeServices::Postgres(_) => {
                panic!("expected libsql production runtime")
            }
        };
        let default_path =
            VirtualPath::new("/tenants/reborn-cli/users/default-owner/turns/rows/v1/deltas/log")
                .expect("default turn-state row delta log path");
        let system_path =
            VirtualPath::new("/tenants/__system__/users/__system__/turns/rows/v1/deltas/log")
                .expect("system turn-state row delta log path");
        let default_identity = RebornRuntimeIdentity::reborn_cli();
        let default_tenant = TenantId::new(default_identity.tenant_id).expect("default tenant");
        let scope = ironclaw_turns::TurnScope::new_with_owner(
            default_tenant,
            None,
            None,
            ironclaw_host_api::ThreadId::new("default-thread").expect("thread"),
            Some(owner.clone()),
        );
        let submit = ironclaw_turns::SubmitTurnRequest {
            requested_model: None,
            scope,
            actor: ironclaw_turns::TurnActor::new(owner),
            accepted_message_ref: ironclaw_turns::AcceptedMessageRef::new("default-message-ref")
                .expect("message ref"),
            source_binding_ref: ironclaw_turns::SourceBindingRef::new("source-web")
                .expect("source binding"),
            reply_target_binding_ref: ironclaw_turns::ReplyTargetBindingRef::new("reply-web")
                .expect("reply binding"),
            requested_run_profile: Some(
                ironclaw_turns::RunProfileRequest::new("default").expect("run profile"),
            ),
            idempotency_key: ironclaw_turns::IdempotencyKey::new("default-turn")
                .expect("idempotency key"),
            received_at: chrono::Utc::now(),
            requested_run_id: None,
            parent_run_id: None,
            subagent_depth: 0,
            spawn_tree_root_run_id: None,
            product_context: None,
        };
        ironclaw_turns::TurnStateStore::submit_turn(
            graph.turn_state.as_ref(),
            submit,
            &ironclaw_turns::AllowAllTurnAdmissionPolicy,
            &InMemoryRunProfileResolver::default(),
        )
        .await
        .expect("submit through production turn-state store");

        assert!(
            append_log_has_entries(
                &assertion_filesystem,
                &default_path,
                "default turn-state row delta log read"
            )
            .await
        );
        assert!(
            !append_log_has_entries(
                &assertion_filesystem,
                &system_path,
                "system turn-state row delta log read"
            )
            .await
        );
    }

    #[cfg(feature = "libsql")]
    async fn append_log_has_entries<F>(filesystem: &F, path: &VirtualPath, label: &str) -> bool
    where
        F: RootFilesystem,
    {
        match filesystem
            .tail(path, ironclaw_filesystem::SeqNo::ZERO)
            .await
        {
            Ok(entries) => !entries.is_empty(),
            Err(ironclaw_filesystem::FilesystemError::NotFound { .. }) => false,
            Err(error) => panic!("{label}: {error}"),
        }
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn production_libsql_builder_rejects_invalid_owner_id_at_composition_boundary() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = Arc::new(
            libsql::Builder::new_local(dir.path().join("reborn.db").display().to_string())
                .build()
                .await
                .expect("build libsql database"),
        );

        let result = build_reborn_services(
            RebornBuildInput::libsql(
                RebornCompositionProfile::Production,
                "",
                db,
                dir.path().join("events.db").display().to_string(),
                None,
                ironclaw_secrets::SecretMaterial::from("01234567890123456789012345678901"),
            )
            .with_production_trust_policy(Arc::new(
                builtin_first_party_trust_policy().expect("builtin trust policy"),
            ))
            .with_runtime_policy(EffectiveRuntimePolicy {
                deployment: ironclaw_host_api::DeploymentMode::HostedMultiTenant,
                requested_profile: ironclaw_host_api::RuntimeProfile::HostedSafe,
                resolved_profile: ironclaw_host_api::RuntimeProfile::HostedSafe,
                filesystem_backend: FilesystemBackendKind::TenantWorkspace,
                process_backend: ProcessBackendKind::None,
                network_mode: ironclaw_host_api::NetworkMode::Brokered,
                secret_mode: SecretMode::TenantBroker,
                approval_policy: ironclaw_host_api::runtime_policy::ApprovalPolicy::AskAlways,
                audit_mode: ironclaw_host_api::AuditMode::Standard,
            }),
        )
        .await;

        assert!(
            matches!(result, Err(RebornBuildError::InvalidConfig { ref reason }) if reason.contains("must not be empty")),
            "expected invalid owner id error, got {result:?}"
        );
    }

    #[cfg(any(feature = "libsql", feature = "postgres"))]
    #[tokio::test]
    async fn local_dev_nearai_mcp_auto_bootstraps_from_injected_config() {
        let dir = tempfile::tempdir().expect("tempdir");
        let owner = "local-dev-nearai-mcp-owner";
        let services = build_reborn_services(nearai_bootstrap_input_with_base(
            owner,
            dir.path().join("local-dev"),
            "https://nearai-db.example.test:9443/v1",
            "nearai-test-key",
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

        let projection = extension_management
            .project(
                nearai_ref,
                extension_management.tenant_operator_user_id_for_test(),
            )
            .await
            .expect("NEAR AI MCP projected");
        assert_eq!(projection.phase, LifecyclePhase::Active);

        let capabilities = extension_management
            .active_model_visible_capabilities()
            .await
            .expect("active capabilities");
        let search = capabilities
            .iter()
            .find(|capability| capability.id.as_str() == "nearai.web_search")
            .expect("nearai.web_search active");

        assert_eq!(search.provider.as_str(), "nearai");
        assert_eq!(search.effects, nearai_allowed_effects());
        assert_eq!(search.runtime_credentials.len(), 1);
        assert_eq!(
            search.runtime_credentials[0].handle,
            SecretHandle::new("llm_nearai_api_key").unwrap()
        );
        assert_eq!(
            search.runtime_credentials[0].source,
            RuntimeCredentialRequirementSource::ProductAuthAccount {
                provider: RuntimeCredentialAccountProviderId::new("nearai").unwrap(),
                setup: Default::default(),
            }
        );
        assert_eq!(
            search.runtime_credentials[0].audience.host_pattern,
            "nearai-db.example.test"
        );
        assert_eq!(search.runtime_credentials[0].audience.port, Some(9443));

        let auth_scope = AuthProductScope::new(
            local_dev_nearai_mcp_owner_scope(UserId::new(owner).unwrap(), None)
                .expect("NEAR AI MCP owner scope"),
            AuthSurface::Api,
        );
        let accounts = services
            .product_auth
            .as_ref()
            .expect("product auth")
            .credential_account_record_source()
            .accounts_for_owner(&auth_scope)
            .await
            .expect("credential accounts load");
        let nearai_account = accounts
            .iter()
            .find(|account| account.provider.as_str() == "nearai")
            .expect("NEAR AI product-auth account");
        assert_eq!(nearai_account.status, CredentialAccountStatus::Configured);
        assert!(nearai_account.access_secret.is_some());
        let nearai_access_secret = nearai_account
            .access_secret
            .clone()
            .expect("NEAR AI product-auth access secret");
        let nearai_account_scope = nearai_account.scope.resource.clone();
        let resolver = ProductAuthRuntimeCredentialResolver::new_with_refresh(
            services
                .product_auth
                .as_ref()
                .expect("product auth")
                .runtime_credential_account_selection_service(),
            services
                .product_auth
                .as_ref()
                .expect("product auth")
                .runtime_credential_account_refresh_service(),
        );
        let sso_scope = ResourceScope {
            tenant_id: nearai_account_scope.tenant_id.clone(),
            user_id: UserId::new("local-dev-nearai-mcp-sso-user").unwrap(),
            agent_id: nearai_account_scope.agent_id.clone(),
            project_id: nearai_account_scope.project_id.clone(),
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        };
        let resolved = resolver
            .resolve_access_secret(RuntimeCredentialAccountRequest {
                scope: &sso_scope,
                provider: &RuntimeCredentialAccountProviderId::new("nearai").unwrap(),
                setup: &RuntimeCredentialAccountSetup::ManualToken,
                provider_scopes: &[],
                requester_extension: &ExtensionId::new("nearai").unwrap(),
            })
            .await
            .expect("SSO user should resolve host-managed NEAR AI credential");
        assert_eq!(resolved.handle, nearai_access_secret);
        assert_eq!(resolved.scope, nearai_account_scope);
    }

    #[cfg(not(any(feature = "libsql", feature = "postgres")))]
    #[tokio::test]
    async fn local_dev_nearai_mcp_skips_auto_activation_without_durable_product_auth() {
        let dir = tempfile::tempdir().expect("tempdir");
        let owner = "local-dev-nearai-mcp-no-durable-owner";
        let services = build_reborn_services(nearai_bootstrap_input_with_base(
            owner,
            dir.path().join("local-dev"),
            "http://private.near.ai",
            "nearai-test-key",
        ))
        .await
        .expect("local-dev services build should ignore invalid NEAR AI MCP endpoint without durable product auth");
        let local_runtime = services.local_runtime.as_ref().expect("local runtime");
        let extension_management = local_runtime
            .extension_management
            .as_ref()
            .expect("extension management");
        let nearai_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "nearai").expect("valid ref");

        let projection = extension_management
            .project(
                nearai_ref,
                extension_management.tenant_operator_user_id_for_test(),
            )
            .await
            .expect("NEAR AI MCP projected");
        assert_eq!(projection.phase, LifecyclePhase::Discovered);

        let capabilities = extension_management
            .active_model_visible_capabilities()
            .await
            .expect("active capabilities");
        assert!(
            capabilities
                .iter()
                .all(|capability| capability.id.as_str() != "nearai.web_search")
        );

        let auth_scope = AuthProductScope::new(
            local_dev_nearai_mcp_owner_scope(UserId::new(owner).unwrap(), None)
                .expect("NEAR AI MCP owner scope"),
            AuthSurface::Api,
        );
        let accounts = services
            .product_auth
            .as_ref()
            .expect("product auth")
            .credential_account_record_source()
            .accounts_for_owner(&auth_scope)
            .await
            .expect("credential accounts load");
        assert!(
            accounts
                .iter()
                .all(|account| account.provider.as_str() != "nearai")
        );
    }

    #[cfg(any(feature = "libsql", feature = "postgres"))]
    #[tokio::test]
    async fn local_dev_nearai_mcp_rebootstrap_reuses_existing_account() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("local-dev");
        let owner = "local-dev-nearai-mcp-idempotent-owner";
        let auth_scope = AuthProductScope::new(
            local_dev_nearai_mcp_owner_scope(UserId::new(owner).unwrap(), None)
                .expect("NEAR AI MCP owner scope"),
            AuthSurface::Api,
        );

        let first = build_reborn_services(nearai_bootstrap_input(owner, root, "nearai-first-key"))
            .await
            .expect("first local-dev services build");
        let first_account = first
            .product_auth
            .as_ref()
            .expect("product auth")
            .credential_account_record_source()
            .accounts_for_owner(&auth_scope)
            .await
            .expect("credential accounts load")
            .into_iter()
            .find(|account| account.provider.as_str() == "nearai")
            .expect("NEAR AI product-auth account");
        let extension_management = first
            .local_runtime
            .as_ref()
            .expect("local runtime")
            .extension_management
            .as_ref()
            .expect("extension management");
        let outcome = crate::llm_admin::nearai_mcp::bootstrap_nearai_mcp(
            Some(
                crate::llm_admin::nearai_mcp::NearAiMcpBootstrapConfig::new(
                    "https://private.near.ai",
                    secrecy::SecretString::from("nearai-second-key"),
                )
                .expect("valid NEAR AI MCP bootstrap config"),
            ),
            first.product_auth.as_ref().expect("product auth"),
            extension_management,
            auth_scope.resource.clone(),
        )
        .await
        .expect("second NEAR AI MCP bootstrap");
        assert_eq!(
            outcome,
            crate::llm_admin::nearai_mcp::NearAiMcpBootstrapOutcome::ReusedCredential
        );
        let accounts = first
            .product_auth
            .as_ref()
            .expect("product auth")
            .credential_account_record_source()
            .accounts_for_owner(&auth_scope)
            .await
            .expect("credential accounts load");
        let nearai_accounts = accounts
            .iter()
            .filter(|account| account.provider.as_str() == "nearai")
            .collect::<Vec<_>>();

        assert_eq!(nearai_accounts.len(), 1);
        assert_eq!(nearai_accounts[0].id, first_account.id);
        assert_eq!(
            nearai_accounts[0].access_secret,
            first_account.access_secret
        );
        assert_eq!(nearai_accounts[0].updated_at, first_account.updated_at);
        assert_eq!(
            nearai_accounts[0].status,
            CredentialAccountStatus::Configured
        );
    }

    #[cfg(any(feature = "libsql", feature = "postgres"))]
    #[tokio::test]
    async fn local_dev_nearai_mcp_bootstrap_reinstalls_discovered_reused_credential() {
        let dir = tempfile::tempdir().expect("tempdir");
        let owner = "local-dev-nearai-mcp-discovered-owner";
        let nearai_ref =
            LifecyclePackageRef::new(LifecyclePackageKind::Extension, "nearai").expect("valid ref");

        let services = build_reborn_services(nearai_bootstrap_input(
            owner,
            dir.path().join("local-dev"),
            "nearai-test-key",
        ))
        .await
        .expect("local-dev services build");
        let extension_management = services
            .local_runtime
            .as_ref()
            .expect("local runtime")
            .extension_management
            .as_ref()
            .expect("extension management");
        let removal_scope = ironclaw_host_api::ResourceScope::local_default(
            ironclaw_host_api::UserId::new(owner).expect("valid user"),
            ironclaw_host_api::InvocationId::new(),
        )
        .expect("valid scope");
        extension_management
            .remove(
                nearai_ref.clone(),
                &removal_scope,
                Some(&removal_scope.user_id),
            )
            .await
            .expect("disable NEAR AI MCP extension");
        let outcome = crate::llm_admin::nearai_mcp::bootstrap_nearai_mcp(
            Some(
                crate::llm_admin::nearai_mcp::NearAiMcpBootstrapConfig::new(
                    "https://private.near.ai",
                    secrecy::SecretString::from("nearai-test-key"),
                )
                .expect("valid NEAR AI MCP bootstrap config"),
            ),
            services.product_auth.as_ref().expect("product auth"),
            extension_management,
            local_dev_nearai_mcp_owner_scope(UserId::new(owner).unwrap(), None)
                .expect("NEAR AI MCP owner scope"),
        )
        .await
        .expect("bootstrap should reinstall discovered extension");
        assert_eq!(
            outcome,
            crate::llm_admin::nearai_mcp::NearAiMcpBootstrapOutcome::Activated
        );
        let projection = extension_management
            .project(
                nearai_ref,
                extension_management.tenant_operator_user_id_for_test(),
            )
            .await
            .expect("NEAR AI MCP projected");
        assert_eq!(projection.phase, LifecyclePhase::Active);

        let capabilities = extension_management
            .active_model_visible_capabilities()
            .await
            .expect("active capabilities");
        assert!(
            capabilities
                .iter()
                .any(|capability| capability.id.as_str() == "nearai.web_search")
        );
    }

    #[cfg(any(feature = "libsql", feature = "postgres"))]
    #[tokio::test]
    async fn local_dev_nearai_mcp_invalid_base_url_fails_build() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config = crate::llm_admin::nearai_mcp::NearAiMcpBootstrapConfig::new(
            "http://private.near.ai",
            secrecy::SecretString::from("nearai-test-key"),
        )
        .expect("config shape");
        let error = build_reborn_services(
            RebornBuildInput::local_dev(
                "local-dev-nearai-mcp-invalid-owner",
                dir.path().join("local-dev"),
            )
            .with_nearai_mcp_bootstrap_config(config),
        )
        .await
        .expect_err("invalid endpoint should fail build");

        let RebornBuildError::InvalidConfig { reason } = error else {
            panic!("expected invalid config");
        };
        assert!(reason.contains("NEARAI_BASE_URL must use https"));
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

        let install_output = invoke_json(
            &services,
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
            &services,
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
            &services,
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

        let failure = invoke_json(
            &services,
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
        assert!(without_auth.diagnostics.is_empty());

        let with_auth = readiness_for(RebornCompositionProfile::Production, true, true, true);
        assert_eq!(with_auth.state, RebornReadinessState::ProductionValidated);
        assert!(with_auth.facades.product_auth);
        assert!(with_auth.diagnostics.is_empty());
    }

    #[test]
    fn readiness_for_profile_diagnostics_cover_cutover_states() {
        let migration = readiness_for(RebornCompositionProfile::MigrationDryRun, true, true, true);
        assert_eq!(
            migration.state,
            RebornReadinessState::MigrationDryRunValidated
        );
        assert!(migration.diagnostics.is_empty());

        let yolo = readiness_for(RebornCompositionProfile::LocalDevYolo, true, true, true);
        assert_eq!(yolo.state, RebornReadinessState::DevOnly);
        assert_eq!(
            yolo.diagnostics,
            vec![RebornReadinessDiagnostic::local_dev_yolo()]
        );

        let hosted_volume = readiness_for(
            RebornCompositionProfile::HostedSingleTenantVolume,
            true,
            true,
            true,
        );
        assert_eq!(
            hosted_volume.state,
            RebornReadinessState::HostedSingleTenantVolumePreviewValidated
        );
        assert_eq!(
            hosted_volume.diagnostics,
            vec![RebornReadinessDiagnostic::hosted_single_tenant_volume()]
        );
    }

    async fn invoke_json(
        services: &RebornServices,
        capability_id: &str,
        context: ExecutionContext,
        input: serde_json::Value,
    ) -> Result<serde_json::Value, RuntimeFailureKind> {
        crate::approval_test_support::invoke_json_with_local_dev_approval(
            services,
            capability_id,
            context,
            input,
            trust_decision(),
        )
        .await
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

    /// Turn on the global auto-approve switch for `context`'s actor scope so a
    /// host-runtime dispatch exercises the tool path instead of stopping at the
    /// per-tool approval gate. The Tools-settings switch is authoritative for
    /// first-party tool dispatch; enabling it here mirrors the operator
    /// having flipped it on before letting the agent run tools.
    async fn enable_global_auto_approve_for_context(
        local_runtime: &RebornLocalRuntimeServices,
        context: &ExecutionContext,
    ) {
        local_runtime
            .auto_approve_settings
            .set(AutoApproveSettingInput {
                updated_by: Principal::User(context.resource_scope.user_id.clone()),
                scope: context.resource_scope.clone(),
                enabled: true,
            })
            .await
            .expect("enabling global auto-approve should succeed");
    }

    use crate::approval_test_support::disable_global_auto_approve;

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

    fn local_dev_minimal_approval_policy()
    -> ironclaw_host_api::runtime_policy::EffectiveRuntimePolicy {
        let mut policy = crate::local_dev_runtime_policy().expect("local-dev policy resolves");
        policy.requested_profile = ironclaw_host_api::runtime_policy::RuntimeProfile::LocalYolo;
        policy.resolved_profile = ironclaw_host_api::runtime_policy::RuntimeProfile::LocalYolo;
        policy.approval_policy = ironclaw_host_api::runtime_policy::ApprovalPolicy::Minimal;
        policy
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

    /// Verify that the durable `local_dev_outbound_store` bundle (libsql or postgres)
    /// shares a single `FilesystemOutboundStateStore` allocation across all four
    /// trait-object roles.
    ///
    /// The assertion reads the four trait-object pointers from the built
    /// `RebornLocalRuntimeServices` and compares their data halves via
    /// `std::ptr::addr_eq` (trait objects of different traits cannot be compared
    /// with `Arc::ptr_eq` directly).
    #[cfg(all(
        any(feature = "libsql", feature = "postgres"),
        feature = "slack-v2-host-beta"
    ))]
    #[tokio::test]
    async fn local_dev_outbound_store_durable_shares_one_allocation_across_all_roles() {
        let dir = tempfile::tempdir().expect("tempdir");
        let services = build_reborn_services(RebornBuildInput::local_dev(
            "outbound-store-alloc-owner",
            dir.path().join("local-dev"),
        ))
        .await
        .expect("local-dev services build");

        let local_runtime = services.local_runtime.as_ref().expect("local runtime");

        // Cast each fat-pointer's data half to *const () for cross-trait comparison.
        let pref_ptr = Arc::as_ptr(&local_runtime.outbound_preferences) as *const ();
        let state_ptr = Arc::as_ptr(&local_runtime.outbound_state) as *const ();
        let gate_ptr = Arc::as_ptr(&local_runtime.delivered_gate_routes) as *const ();
        let delivery_ptr = Arc::as_ptr(&local_runtime.triggered_run_delivery) as *const ();

        assert!(
            std::ptr::addr_eq(pref_ptr, state_ptr),
            "outbound_preferences and outbound_state must share one allocation"
        );
        assert!(
            std::ptr::addr_eq(pref_ptr, gate_ptr),
            "outbound_preferences and delivered_gate_routes must share one allocation"
        );
        assert!(
            std::ptr::addr_eq(pref_ptr, delivery_ptr),
            "outbound_preferences and triggered_run_delivery must share one allocation"
        );
    }

    #[cfg(feature = "slack-v2-host-beta")]
    fn slack_identity(
        manifest_path: &str,
        digest: Option<String>,
    ) -> ironclaw_host_api::PackageIdentity {
        ironclaw_host_api::PackageIdentity::new(
            ironclaw_host_api::PackageId::new("slack_bot").expect("slack package id"),
            ironclaw_host_api::PackageSource::LocalManifest {
                path: manifest_path.to_string(),
            },
            digest,
            None,
        )
    }

    #[cfg(feature = "slack-v2-host-beta")]
    #[test]
    fn builtin_first_party_trust_policy_includes_slack_local_manifest_entry() {
        let policy = builtin_first_party_trust_policy().expect("trust policy");
        let expected_digest = slack_bot_manifest_digest();

        let matching = ironclaw_trust::TrustPolicy::evaluate(
            &policy,
            &ironclaw_trust::TrustPolicyInput {
                identity: slack_identity(
                    "/system/extensions/slack_bot/manifest.toml",
                    Some(expected_digest.clone()),
                ),
                requested_trust: ironclaw_host_api::RequestedTrustClass::FirstPartyRequested,
                requested_authority: Default::default(),
            },
        )
        .expect("matching slack identity should evaluate");

        assert_eq!(matching.effective_trust.class(), TrustClass::FirstParty);
        assert_eq!(
            matching.provenance,
            ironclaw_trust::TrustProvenance::AdminConfig
        );

        let wrong_digest = ironclaw_trust::TrustPolicy::evaluate(
            &policy,
            &ironclaw_trust::TrustPolicyInput {
                identity: slack_identity(
                    "/system/extensions/slack_bot/manifest.toml",
                    Some(
                        "sha256:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"
                            .to_string(),
                    ),
                ),
                requested_trust: ironclaw_host_api::RequestedTrustClass::FirstPartyRequested,
                requested_authority: Default::default(),
            },
        )
        .expect("wrong digest slack identity should evaluate");

        assert_eq!(wrong_digest.effective_trust.class(), TrustClass::Sandbox);
        assert_eq!(
            wrong_digest.provenance,
            ironclaw_trust::TrustProvenance::Default
        );

        let wrong_path = ironclaw_trust::TrustPolicy::evaluate(
            &policy,
            &ironclaw_trust::TrustPolicyInput {
                identity: slack_identity(
                    "/system/extensions/slack_bot/other-manifest.toml",
                    Some(expected_digest),
                ),
                requested_trust: ironclaw_host_api::RequestedTrustClass::FirstPartyRequested,
                requested_authority: Default::default(),
            },
        )
        .expect("wrong path slack identity should evaluate");

        assert_eq!(wrong_path.effective_trust.class(), TrustClass::Sandbox);
        assert_eq!(
            wrong_path.provenance,
            ironclaw_trust::TrustProvenance::Default
        );
    }
}

#[cfg(test)]
mod auth_tests;
#[cfg(test)]
mod local_dev_host_tests;
