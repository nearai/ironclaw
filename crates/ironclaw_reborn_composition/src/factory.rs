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
use ironclaw_approvals::{
    FilesystemAutoApproveSettingStore, FilesystemPersistentApprovalPolicyStore,
    FilesystemToolPermissionOverrideStore,
};
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_auth::AuthProviderClient;
use ironclaw_auth::{AuthProductScope, AuthSurface};
// Used by both the durable (`<CompositeRootFilesystem>`) and no-durable
// (`<InMemoryBackend>`) capability-lease aliases/builders, so the import is
// unconditional (arch-simplification §4.3).
use ironclaw_authorization::FilesystemCapabilityLeaseStore;
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_authorization::GrantAuthorizer;
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
use ironclaw_filesystem::{DiskFilesystem, ScopedFilesystem};
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
    CapabilitySurfaceVersion, FirstPartyCapabilityRegistry, HostProcessPort,
    HostRuntimeHttpEgressPort, HostRuntimeServices, PostEditCheckConfig,
    ProductAuthProviderRuntimePorts, TriggerCreateHook,
    builtin_first_party_handlers_with_trigger_create_hook, builtin_first_party_package,
};
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_host_runtime::{
    builtin_first_party_handlers_with_trigger_create_hook_for_process_backend,
    builtin_first_party_package_for_process_backend,
};
// §4.3: the deleted `InMemoryCheckpointStateStore` is gone — both durable and
// no-durable checkpoint-state wiring share the one
// `FilesystemCheckpointStateStore` (over the composite root filesystem or a
// volatile in-memory backend), so this import is unconditional.
use ironclaw_loop_host::FilesystemCheckpointStateStore;
use ironclaw_outbound::CommunicationPreferenceRepository;
// §4.3: the deleted `InMemoryOutboundStateStore` is gone — both durable and
// no-durable outbound wiring now share the one `FilesystemOutboundStateStore`
// (over a libsql/postgres or in-memory backend), so this import is
// unconditional, not gated behind the durable-backend features.
use ironclaw_outbound::FilesystemOutboundStateStore;
use ironclaw_outbound::{DeliveredGateRouteStore, OutboundStateStore, TriggeredRunDeliveryStore};
use ironclaw_processes::ProcessServices;
use ironclaw_product_workflow::ChannelConnectionFacade;
use ironclaw_product_workflow::{
    ExtensionAccountSetupRegistry, LifecycleProductSurfaceContext,
    ProductAuthTurnGateResumeDispatcher, ProjectService,
};
use ironclaw_projects::ProjectRepository;
use ironclaw_resources::InMemoryResourceGovernor;
// `FilesystemBudgetGateStore` backs both the durable and the no-durable
// (`<InMemoryBackend>`) budget-gate wiring — the deleted `InMemoryBudgetGateStore`
// had no cfg gate either — so its import must be unconditional, not gated behind
// the durable-backend features (arch-simplification §4.3).
use ironclaw_resources::FilesystemBudgetGateStore;
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_resources::{
    BroadcastBudgetEventSink, BudgetGateStore, FilesystemResourceGovernor, ResourceGovernor,
};
// Used by both the durable (`<CompositeRootFilesystem>`) and no-durable
// (`<InMemoryBackend>`) run-state/approval aliases + builders, so the import is
// unconditional (arch-simplification §4.3).
use crate::RebornProductAuthServicePorts;
use crate::builtin_capability_policy::{BuiltinCapabilityPolicy, builtin_capability_policy};
use crate::extension_host::available_extensions::telegram_manifest_digest;
use crate::extension_host::available_extensions::{
    slack_bot_manifest_digest, slack_manifest_digest,
};
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
    provider_instance_readiness::{
        ProviderInstanceReadinessInputs, provider_instance_readiness_map,
    },
};
use crate::input::{RebornLocalRuntimeIdentity, RebornRuntimeProcessBinding, RebornStorageInput};
use crate::lifecycle_auth_continuation::{
    LifecycleAuthContinuationDispatcher, LifecycleProductFacadeSlot,
};
use crate::local_dev_authorization::{StoreApprovalSettingsProvider, local_dev_authorizer};
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
use ironclaw_run_state::{FilesystemApprovalRequestStore, FilesystemRunStateStore};
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
use ironclaw_turns::FilesystemTurnStateRowStore;
#[cfg(any(feature = "libsql", feature = "postgres"))]
use ironclaw_turns::InMemoryRunProfileResolver;
use ironclaw_turns::{
    CheckpointStateStore, DefaultTurnCoordinator, ExternalToolCatalog, InMemoryExternalToolCatalog,
    LoopCheckpointStore,
};

/// Output of [`build_local_runtime_root_filesystem`]: the composed local-dev
/// root filesystem and, when libSQL is the substrate, a clone of the raw
/// libSQL handle. The handle backs both the local-dev trigger repository
/// and the canonical Reborn identity store, so each rides the same
/// `reborn-local-dev.db` rather than opening a second handle to the file
/// (see `RebornRuntime::open_reborn_identity_resolver`).
struct RootFilesystemBundle {
    filesystem: Arc<CompositeRootFilesystem>,
    durable_backend: DurableBackend,
}

// `pub(crate)` to match `build_default_local_dev_database_roots` (also
// `pub(crate)` for the `test_support` accessor): a `pub(crate)` fn returning a
// private enum trips `private_interfaces`. The enum stays crate-internal.
pub(crate) enum DurableBackend {
    #[cfg(feature = "libsql")]
    LibSql(Arc<libsql::Database>),
    #[cfg(feature = "postgres")]
    Postgres(deadpool_postgres::Pool),
    #[cfg(not(feature = "libsql"))]
    Ephemeral,
}

enum StorageBackendInput {
    LocalDefault,
    #[cfg(feature = "postgres")]
    Postgres(deadpool_postgres::Pool),
}

type WorkspaceFilesystems = (
    Arc<ScopedFilesystem<CompositeRootFilesystem>>,
    Arc<ScopedFilesystem<CompositeRootFilesystem>>,
    MountView,
);

const LOCAL_DEV_DEFAULT_SYSTEM_PROMPT_PATH: &str = "system/prompts/default-system.md";
const LOCAL_DEV_LEGACY_SKILLS_BACKFILL_MARKER: &str = ".legacy-skills-backfilled";
const LOCAL_DEV_LEGACY_SKILLS_BACKFILL_MAX_DEPTH: usize = 64;
/// Filename of the cached local-dev secrets master-key dotfile under a
/// Reborn home / local-dev root directory. `pub` (re-exported from `lib.rs`)
/// so onboarding (`ironclaw_reborn_cli::commands::onboard`) can check for its
/// presence without duplicating the literal.
#[cfg(any(feature = "libsql", feature = "postgres"))]
pub const LOCAL_DEV_SECRETS_MASTER_KEY_PATH: &str = ".reborn-local-dev-secrets-master-key";

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

// One turn-state store, backend-injected — the production
// `FilesystemTurnStateRowStore<F>` (row layout) every deployment uses
// unconditionally, at its single write-behind durability mode (arch-simplification
// §4.3 / #6263 Step 5b — there is no longer a durability-mode or store-type
// choice). The no-durable-features build backs it with `InMemoryBackend` directly
// (volatile, `LocalOnly`), matching the sibling run-state/approval/lease stores.
#[cfg(any(feature = "libsql", feature = "postgres"))]
pub(crate) type ComposedTurnStateStore = FilesystemTurnStateRowStore<CompositeRootFilesystem>;
#[cfg(not(any(feature = "libsql", feature = "postgres")))]
pub(crate) type ComposedTurnStateStore = FilesystemTurnStateRowStore<InMemoryBackend>;

#[cfg(any(feature = "libsql", feature = "postgres"))]
type ComposedResourceGovernor = FilesystemResourceGovernor<CompositeRootFilesystem>;
#[cfg(not(any(feature = "libsql", feature = "postgres")))]
type ComposedResourceGovernor = InMemoryResourceGovernor;

// One run-state / approval-request store, backend-injected — the production
// `Filesystem*Store<F>` every deployment uses, never a bespoke `InMemory*Store`
// (arch-simplification §4.3). The no-durable-features build backs them with
// `InMemoryBackend` directly, so the concrete type is `<InMemoryBackend>`, which
// the host-runtime production-wiring guard classifies `LocalOnly`.
#[cfg(any(feature = "libsql", feature = "postgres"))]
type ComposedRunStateStore = FilesystemRunStateStore<CompositeRootFilesystem>;
#[cfg(not(any(feature = "libsql", feature = "postgres")))]
type ComposedRunStateStore = FilesystemRunStateStore<InMemoryBackend>;

#[cfg(any(feature = "libsql", feature = "postgres"))]
pub(crate) type ComposedApprovalRequestStore =
    FilesystemApprovalRequestStore<CompositeRootFilesystem>;
#[cfg(not(any(feature = "libsql", feature = "postgres")))]
pub(crate) type ComposedApprovalRequestStore = FilesystemApprovalRequestStore<InMemoryBackend>;

#[cfg(any(feature = "libsql", feature = "postgres"))]
pub(crate) type ComposedCapabilityLeaseStore =
    FilesystemCapabilityLeaseStore<CompositeRootFilesystem>;
// One capability-lease store, backend-injected — the production
// `FilesystemCapabilityLeaseStore<F>` every deployment uses, never a bespoke
// `InMemory*Store` (arch-simplification §4.3). The no-durable-features build
// backs it with `InMemoryBackend` directly, so the concrete type is
// `<InMemoryBackend>`, which the host-runtime production-wiring guard classifies
// `LocalOnly`.
#[cfg(not(any(feature = "libsql", feature = "postgres")))]
pub(crate) type ComposedCapabilityLeaseStore = FilesystemCapabilityLeaseStore<InMemoryBackend>;

// One store per approval domain, backend-injected — the production
// `Filesystem*Store<F>` every deployment uses, never a bespoke `InMemory*Store`
// (arch-simplification §4.3). The store's backend type encodes its durability:
// the no-durable-features build backs them with `InMemoryBackend` directly, so
// the concrete type is `<InMemoryBackend>` — which the host-runtime
// production-wiring guard classifies `LocalOnly` (the same way the volatile
// `<InMemoryBackend>`-backed run-state/approval/lease stores are flagged).
// Durable builds use the libSQL/Postgres-backed composite root filesystem, whose
// type is distinct and correctly classifies as a production candidate.
#[cfg(any(feature = "libsql", feature = "postgres"))]
pub(crate) type ComposedPersistentApprovalPolicyStore =
    FilesystemPersistentApprovalPolicyStore<CompositeRootFilesystem>;
#[cfg(not(any(feature = "libsql", feature = "postgres")))]
pub(crate) type ComposedPersistentApprovalPolicyStore =
    FilesystemPersistentApprovalPolicyStore<InMemoryBackend>;

#[cfg(any(feature = "libsql", feature = "postgres"))]
pub(crate) type ComposedToolPermissionOverrideStore =
    FilesystemToolPermissionOverrideStore<CompositeRootFilesystem>;
#[cfg(not(any(feature = "libsql", feature = "postgres")))]
pub(crate) type ComposedToolPermissionOverrideStore =
    FilesystemToolPermissionOverrideStore<InMemoryBackend>;

#[cfg(any(feature = "libsql", feature = "postgres"))]
pub(crate) type ComposedAutoApproveSettingStore =
    FilesystemAutoApproveSettingStore<CompositeRootFilesystem>;
#[cfg(not(any(feature = "libsql", feature = "postgres")))]
pub(crate) type ComposedAutoApproveSettingStore =
    FilesystemAutoApproveSettingStore<InMemoryBackend>;

#[cfg(any(feature = "libsql", feature = "postgres"))]
type ComposedProcessServices = ProcessServices<
    ironclaw_processes::FilesystemProcessStore<CompositeRootFilesystem>,
    ironclaw_processes::FilesystemProcessResultStore<CompositeRootFilesystem>,
>;
// One process store pair, backend-injected — the production
// `FilesystemProcess*Store<F>` every deployment uses, never a bespoke
// `InMemory*Store` (arch-simplification §4.3). The no-durable-features build
// backs it with `InMemoryBackend` directly, so the concrete type is
// `<InMemoryBackend>`, which the host-runtime production-wiring guard
// classifies `LocalOnly`.
#[cfg(not(any(feature = "libsql", feature = "postgres")))]
type ComposedProcessServices = ProcessServices<
    ironclaw_processes::FilesystemProcessStore<InMemoryBackend>,
    ironclaw_processes::FilesystemProcessResultStore<InMemoryBackend>,
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

/// Composition-layer optional-env seam for the coding post-edit check
/// (`IRONCLAW_POST_EDIT_CHECK` / `IRONCLAW_POST_EDIT_CHECK_TIMEOUT_SECS`).
/// Parsing lives in the module-owned `PostEditCheckConfig::from_env`; this
/// only threads the resolved config into host runtime services. The feature
/// stays off when the command env is unset or blank.
fn apply_post_edit_check_from_env<F, G, S, R>(
    services: HostRuntimeServices<F, G, S, R>,
) -> Result<HostRuntimeServices<F, G, S, R>, RebornBuildError>
where
    F: ironclaw_filesystem::RootFilesystem + 'static,
    G: ironclaw_resources::ResourceGovernor + 'static,
    S: ironclaw_processes::ProcessStore + 'static,
    R: ironclaw_processes::ProcessResultStore + 'static,
{
    match PostEditCheckConfig::from_env() {
        Ok(Some(post_edit_check)) => Ok(services.with_post_edit_check(post_edit_check)),
        Ok(None) => Ok(services),
        Err(error) => Err(RebornBuildError::InvalidConfig {
            reason: error.to_string(),
        }),
    }
}

fn local_dev_process_port_for_policy(
    runtime_policy: &Option<ironclaw_host_api::runtime_policy::EffectiveRuntimePolicy>,
    workspace_root: &Path,
    host_home_root: Option<&HostHomeRoot>,
) -> Option<HostProcessPort> {
    let runtime_policy = runtime_policy.as_ref()?;
    if runtime_policy.process_backend != ProcessBackendKind::LocalHost {
        return None;
    }
    let mut process_port = if runtime_policy.secret_mode == SecretMode::InheritedEnv {
        HostProcessPort::new_inherited_env()
    } else {
        HostProcessPort::new()
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
    pub(crate) local_runtime: Option<Arc<RebornRuntimeSubstrate>>,
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
    ) -> Option<Arc<ScopedFilesystem<CompositeRootFilesystem>>> {
        let local_runtime = self.local_runtime.as_ref()?;
        Some(Arc::new(ScopedFilesystem::with_fixed_view(
            Arc::clone(&local_runtime.extension_filesystem),
            local_runtime.workspace_mounts.clone(),
        )))
    }

    #[cfg(feature = "test-support")]
    pub fn local_dev_approval_test_parts(&self) -> Option<RebornApprovalTestParts> {
        let local_runtime = self.local_runtime.as_ref()?;
        let approval_requests: Arc<dyn ironclaw_run_state::ApprovalRequestStore> =
            local_runtime.approval_requests.clone();
        let capability_leases: Arc<dyn ironclaw_authorization::CapabilityLeaseStore> =
            local_runtime.capability_leases.clone();
        // Build over the same shared composite root production `capability_wiring`
        // uses, so these test-support stores persist across the group's
        // threads/turns and round-trip identically to production.
        let capability_store_filesystem =
            crate::wrap_scoped(Arc::clone(&local_runtime.extension_filesystem));
        let gate_record_store: Arc<dyn ironclaw_run_state::GateRecordStore> =
            Arc::new(ironclaw_run_state::FilesystemGateRecordStore::new(
                Arc::clone(&capability_store_filesystem),
            ));
        let replay_payload_store: Arc<dyn ironclaw_capabilities::ReplayPayloadStore> = Arc::new(
            ironclaw_capabilities::FilesystemReplayPayloadStore::new(capability_store_filesystem),
        );
        Some(RebornApprovalTestParts {
            approval_requests,
            capability_leases,
            gate_record_store,
            replay_payload_store,
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
    /// `StagedCapabilityIo::new_with_durable_previews` and to the
    /// `result_read` synthetic capability, so a harness built over this
    /// `RebornServices` can drive its own real `StagedCapabilityIo` through
    /// `staged_capability_io_for_test`. Returns `None` for production-profile
    /// compositions without a local-dev runtime.
    #[cfg(feature = "test-support")]
    pub fn local_dev_thread_service_for_test(
        &self,
    ) -> Option<Arc<dyn ironclaw_threads::SessionThreadService>> {
        let local_runtime = self.local_runtime.as_ref()?;
        Some(Arc::clone(&local_runtime.thread_service))
    }

    /// Test-support access to the local-dev communication-preference repository
    /// (W6-COLD-SPOTS seam). This is the SAME `Arc` that `build_local_runtime_store_graph`
    /// wires into `RebornRuntimeSubstrate::outbound_preferences` via
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
    /// that `build_local_runtime_store_graph` establishes in production. Used to reopen
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
    ) -> Option<Arc<crate::support::fs::ProjectScopedAttachmentReader<CompositeRootFilesystem>>>
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
    /// instead of going through `RefreshingLoopCapabilityPortFactory`, so they need
    /// the same active-extension grants and provider trust that production
    /// local-dev recomputes whenever the model-visible surface is refreshed.
    #[cfg(feature = "test-support")]
    pub async fn local_dev_active_extension_authority_for_test(
        &self,
        grantee: &ExtensionId,
    ) -> Option<
        Result<ActiveExtensionAuthorityForTest, ironclaw_product_workflow::ProductWorkflowError>,
    > {
        let extension_management = self.local_runtime.as_ref()?.extension_management.as_ref()?;
        Some(active_extension_authority_for_test(extension_management, grantee).await)
    }
}

#[cfg(feature = "test-support")]
pub struct ActiveExtensionAuthorityForTest {
    pub grants: Vec<CapabilityGrant>,
    pub provider_trust: Vec<(ExtensionId, TrustDecision)>,
}

#[cfg(feature = "test-support")]
async fn active_extension_authority_for_test(
    extension_management: &RebornLocalExtensionManagementPort,
    grantee: &ExtensionId,
) -> Result<ActiveExtensionAuthorityForTest, ironclaw_product_workflow::ProductWorkflowError> {
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
    Ok(ActiveExtensionAuthorityForTest {
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
pub struct RebornApprovalTestParts {
    pub approval_requests: Arc<dyn ironclaw_run_state::ApprovalRequestStore>,
    pub capability_leases: Arc<dyn ironclaw_authorization::CapabilityLeaseStore>,
    /// Durable model-visible gate-record store, shared across the group's threads
    /// so a gate raised on one thread can be read back on another.
    pub gate_record_store: Arc<dyn ironclaw_run_state::GateRecordStore>,
    /// Durable host-private replay-payload store (§5.3 Stage 2a-i), shared across
    /// the group's threads/turns so a gate/auth resume reconstitutes the input the
    /// original raise persisted. Backed by the same composite root as production
    /// `capability_wiring`, so the harness store round-trips identically.
    pub replay_payload_store: Arc<dyn ironclaw_capabilities::ReplayPayloadStore>,
}

pub(crate) struct RebornRuntimeSubstrate {
    pub(crate) extension_lifecycle_surface_context: LifecycleProductSurfaceContext,
    pub(crate) owner_user_id: UserId,
    pub(crate) approval_requests: Arc<ComposedApprovalRequestStore>,
    pub(crate) capability_leases: Arc<ComposedCapabilityLeaseStore>,
    /// Per-runtime catalog of client-supplied ("external") tools. Shared between
    /// the loop capability host (which offers them to the model and parks calls)
    /// and the OpenAI-compatible Responses surface (which registers tool specs
    /// and submits client outputs), so both see the same run-scoped state.
    pub(crate) external_tool_catalog: Arc<dyn ExternalToolCatalog>,
    pub(crate) runtime_policy: Option<EffectiveRuntimePolicy>,
    // Used in approval_test_support (cfg(test) only); suppress the dead-code
    // lint on non-test builds where that module is not compiled in.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) capability_policy: Arc<BuiltinCapabilityPolicy>,
    pub(crate) persistent_approval_policies: Arc<ComposedPersistentApprovalPolicyStore>,
    pub(crate) tool_permission_overrides: Arc<ComposedToolPermissionOverrideStore>,
    pub(crate) auto_approve_settings: Arc<ComposedAutoApproveSettingStore>,
    pub(crate) turn_state: Arc<ComposedTurnStateStore>,
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
    pub(crate) outbound_state: Arc<dyn OutboundStateStore>,
    pub(crate) delivered_gate_routes: Arc<dyn DeliveredGateRouteStore>,
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
    #[allow(dead_code)]
    pub(crate) identity_filesystem: Arc<ScopedFilesystem<CompositeRootFilesystem>>,
    /// Admin per-user secret provisioner (target-user-scoped secret store over
    /// the shared root + crypto). `None` when no filesystem secret store was
    /// built. Read only by the WebUI v2 admin surface.
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
    pub(crate) channel_connection_facade_slot:
        Arc<std::sync::OnceLock<Arc<dyn ChannelConnectionFacade>>>,
    pub(crate) runtime_http_egress: Option<Arc<dyn RuntimeHttpEgress>>,
    pub(crate) host_runtime_http_egress: Option<HostRuntimeHttpEgressPort>,
    pub(crate) skill_mounts: MountView,
    pub(crate) memory_mounts: MountView,
    pub(crate) system_extensions_lifecycle_mounts: MountView,
    pub(crate) skill_filesystem: Arc<ScopedFilesystem<CompositeRootFilesystem>>,
    pub(crate) workspace_filesystem: Arc<ScopedFilesystem<CompositeRootFilesystem>>,
    pub(crate) host_state_filesystem: Arc<ScopedFilesystem<CompositeRootFilesystem>>,
    /// Telegram analog of `host_state_filesystem`: a `ScopedFilesystem` whose
    /// fixed resolver is [`crate::telegram_host_state_mount_view`], backing the
    /// durable Telegram setup/pairing/binding/DM-target stores plus the
    /// telegram-scoped idempotency ledger and conversation-binding store.
    pub(crate) telegram_host_state_filesystem: Arc<ScopedFilesystem<dyn RootFilesystem>>,
    pub(crate) subagent_goal_filesystem: Arc<ScopedFilesystem<CompositeRootFilesystem>>,
    /// Tenant-scoped root filesystem used for third-party extension hook
    /// discovery (`/system/extensions/<tenant>`). The runtime derives the
    /// discovery root from the authenticated tenant id; this is the same
    /// backend the rest of local-dev composition uses.
    pub(crate) extension_filesystem: Arc<CompositeRootFilesystem>,
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
    pub(crate) turn_state: Arc<FilesystemTurnStateRowStore<F>>,
    pub(crate) checkpoint_state_store: Arc<dyn CheckpointStateStore>,
    pub(crate) thread_service: Arc<dyn SessionThreadService>,
    pub(crate) trigger_repository: Arc<dyn TriggerRepository>,
    pub(crate) resource_governor: Arc<dyn ResourceGovernor>,
    pub(crate) budget_gate_store: Arc<dyn BudgetGateStore>,
    pub(crate) broadcast_budget_event_sink: Arc<BroadcastBudgetEventSink>,
    pub(crate) event_log: Arc<dyn DurableEventLog>,
    pub(crate) audit_log: Arc<dyn DurableAuditLog>,
    /// Admin per-user secret provisioner over the production secret substrate
    /// (raw root + the runtime's own crypto). Backs the WebUI admin
    /// user-management surface for production profiles where `local_runtime` is
    /// None; mirrors the local substrate's `admin_secret_provisioner`.
    pub(crate) admin_secret_provisioner: Arc<dyn crate::admin_secrets::AdminSecretProvisioner>,
    /// First-class projects + membership (ACL) facade over the production scoped
    /// filesystem. Backs the WebUI project surface for production profiles where
    /// `local_runtime` is None; mirrors the local substrate's `project_service`.
    pub(crate) project_service: Arc<dyn ProjectService>,
    /// Trigger conversation services over the production scoped filesystem.
    /// Mirrors the local substrate's `trigger_conversation_services`: it backs
    /// the production trigger poller's prompt materializer and trusted-ingress
    /// submitter (binding + session-thread + actor-pairing roles). Built eagerly
    /// in `build_backend_production` — production is always durable, so there is
    /// no `OnceCell` lazy-init arm like the local substrate carries.
    pub(crate) trigger_conversation_services:
        ironclaw_conversations::RebornFilesystemConversationServices,
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

// `trigger_conversation_services` is a `OnceCell<RebornFilesystemConversationServices>`
// under a durable backend and an `InMemoryConversationServices` without one, so this
// accessor only exists in the durable shape.
#[cfg(any(feature = "libsql", feature = "postgres"))]
impl RebornRuntimeSubstrate {
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

struct RebornStoreGraph {
    run_state: Arc<ComposedRunStateStore>,
    approval_requests: Arc<ComposedApprovalRequestStore>,
    capability_leases: Arc<ComposedCapabilityLeaseStore>,
    persistent_approval_policies: Arc<ComposedPersistentApprovalPolicyStore>,
    turn_state: Arc<ComposedTurnStateStore>,
    local_runtime: Arc<RebornRuntimeSubstrate>,
    resource_governor: Arc<ComposedResourceGovernor>,
    process_services: ComposedProcessServices,
    trigger_repository: Arc<dyn TriggerRepository>,
}

struct RebornStoreGraphInput {
    filesystem: Arc<CompositeRootFilesystem>,
    owner_user_id: UserId,
    local_runtime_identity: Option<RebornLocalRuntimeIdentity>,
    runtime_policy: Option<EffectiveRuntimePolicy>,
    skill_filesystem: Arc<ScopedFilesystem<CompositeRootFilesystem>>,
    workspace_filesystem: Arc<ScopedFilesystem<CompositeRootFilesystem>>,
    workspace_mounts: MountView,
    local_dev_storage_root: PathBuf,
    default_system_prompt_path: PathBuf,
    trigger_repository: Arc<dyn TriggerRepository>,
    project_repository: Arc<dyn ProjectRepository>,
    /// Concurrency limits for the in-memory (or filesystem-backed) turn-state store.
    turn_state_store_limits: ironclaw_turns::TurnStateStoreLimits,
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
            // §4.3: the deleted `FilesystemSecretStore::ephemeral()` is gone — the disabled
            // bundle wires the one production encrypted secret store over an
            // in-memory backend with an ephemeral master key.
            secret_store: Arc::new(ironclaw_secrets::FilesystemSecretStore::ephemeral()),
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
        profile = %input.profile(),
        owner_id = %input.owner_id,
        "building Reborn composition facades"
    );
    // Substrate selection is deployment *data* (§4.4/§5.6), not a profile
    // match: the config says which substrate to assemble and this dispatches
    // on that value.
    let substrate = input.deployment().substrate();
    match substrate {
        crate::deployment::RuntimeSubstrate::None => Ok(RebornServices::disabled()),
        crate::deployment::RuntimeSubstrate::Local => build_local_runtime(input).await,
        crate::deployment::RuntimeSubstrate::ProductionShaped => {
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

/// Whether a Google OAuth backend is configured, from the composition-side
/// signal `GsuiteFirstPartyHandler` uses to short-circuit dispatch with a
/// "not configured" tool result instead of reaching credential resolution.
/// Shared by `build_local_runtime` and its production-build-context
/// counterpart so the check doesn't drift between the two call sites.
fn google_oauth_configured(
    oauth_provider_configs: &[crate::input::OAuthProviderBackendConfig],
) -> bool {
    oauth_provider_configs
        .iter()
        .any(|config| config.spec.provider_id == ironclaw_auth::GOOGLE_PROVIDER_ID)
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
    #[cfg(test)]
    let host_runtime_http_egress_for_test = input.host_runtime_http_egress_for_test.clone();
    #[cfg(any(test, feature = "test-support"))]
    let network_http_egress_for_test = input.network_http_egress_for_test.clone();
    let RebornBuildInput {
        deployment,
        storage,
        runtime_policy,
        runtime_process_binding,
        product_auth_ports,
        oauth_provider_configs,
        oauth_dcr_provider_configs,
        slack_personal_oauth_lazy_slot,
        slack_host_beta_enabled,
        slack_personal_oauth_redirect_uri_configured,
        nearai_mcp_bootstrap_config,
        owner_id,
        local_runtime_identity,
        turn_state_store_limits,
        ..
    } = input;
    // Label for logging/errors; behaviour reads `deployment`'s axes.
    let profile = deployment.profile();
    // Computed before `oauth_provider_configs` is consumed by
    // `compose_provider_client` below — see `google_oauth_configured`.
    let google_oauth_configured = google_oauth_configured(&oauth_provider_configs);
    // Do NOT "simplify" this to `slack_personal_oauth_lazy_slot.is_some()`.
    // The CLI resolves both from the same env var, but the slot also switches
    // the Slack provider client to lazy setup-service credential resolution —
    // so deriving readiness from it makes every fixture that just wants
    // "configured" opt into lazy credentials it never fills. See the field doc
    // on `RebornBuildInput::slack_personal_oauth_redirect_uri_configured`.
    let provider_instance_readiness =
        provider_instance_readiness_map(ProviderInstanceReadinessInputs {
            google_oauth_configured,
            slack_host_beta_enabled,
            slack_personal_oauth_redirect_uri_configured,
        })
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("provider instance readiness map could not be built: {error}"),
        })?;
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
            if deployment.storage_shape()
                == crate::deployment::StorageShape::HostedSingleTenantPool =>
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
            StorageBackendInput::LocalDefault,
            None::<ironclaw_secrets::SecretMaterial>,
            None::<bool>,
        ),
        #[cfg(feature = "postgres")]
        RebornStorageInput::HostedSingleTenantPostgres { .. }
            if deployment.storage_shape()
                != crate::deployment::StorageShape::HostedSingleTenantPool =>
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
            StorageBackendInput::Postgres(pool),
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
        (true, Some(path)) => Some(HostHomeRoot {
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
    let filesystem_bundle = build_local_runtime_root_filesystem(
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
        DurableBackend::LibSql(database) => Some(Arc::clone(database)),
        DurableBackend::Postgres(_) => None,
    };
    #[cfg(all(feature = "libsql", not(feature = "postgres")))]
    let identity_substrate_db = {
        let DurableBackend::LibSql(database) = &filesystem_bundle.durable_backend;
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
    let mut store_graph = build_local_runtime_store_graph(RebornStoreGraphInput {
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
    let local_dev_secret_bundle = build_secret_store(
        &root,
        Arc::clone(&local_dev_product_auth_filesystem),
        secret_master_key,
    )
    .await?;
    #[cfg(any(feature = "libsql", feature = "postgres"))]
    let secret_store: Arc<dyn SecretStore> = local_dev_secret_bundle.0.clone();
    // §4.3: the deleted `FilesystemSecretStore::ephemeral()` is gone — the no-durable build
    // wires the one production encrypted `FilesystemSecretStore` over an
    // `InMemoryBackend` with an ephemeral master key and the tenant-rewriting
    // `/secrets` mount resolver.
    #[cfg(not(any(feature = "libsql", feature = "postgres")))]
    let secret_store: Arc<dyn SecretStore> =
        Arc::new(ironclaw_secrets::FilesystemSecretStore::ephemeral());
    // Admin per-user secret provisioner over the shared root + the SAME crypto
    // as the runtime's own secret store. Only a durable backend produces the
    // secret bundle this reuses, so the no-storage build has no provisioner —
    // `None` is an ordinary value here (see the in-memory runtime below).
    #[cfg(any(feature = "libsql", feature = "postgres"))]
    let admin_secret_provisioner: Option<
        Arc<dyn crate::admin_secrets::AdminSecretProvisioner>,
    > = Some(Arc::new(
        crate::admin_secrets::FilesystemAdminSecretProvisioner::new(
            Arc::clone(&filesystem),
            local_dev_secret_bundle.1,
        ),
    ));
    #[cfg(not(any(feature = "libsql", feature = "postgres")))]
    let admin_secret_provisioner: Option<
        Arc<dyn crate::admin_secrets::AdminSecretProvisioner>,
    > = None;
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
    services = apply_post_edit_check_from_env(services)?;
    services = attach_hosted_mcp_runtime(services)?;
    let product_auth_runtime_ports = require_product_auth_runtime_ports(&services)?;
    let provider_composition = compose_provider_client(
        oauth_provider_configs,
        oauth_dcr_provider_configs,
        Arc::clone(&secret_store),
        product_auth_runtime_ports.clone(),
        slack_personal_oauth_lazy_slot,
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
    let mut removal_cleanup_adapters: Vec<Arc<dyn ExtensionRemovalCleanupAdapter>> = Vec::new();
    removal_cleanup_adapters.push(Arc::new(
        SlackPersonalConnectionCleanupAdapter::new(Arc::clone(
            &store_graph.local_runtime.channel_connection_facade_slot,
        ))
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("Slack extension removal cleanup could not be built: {error}"),
        })?,
    ));
    removal_cleanup_adapters.push(Arc::new(
        crate::extension_host::extension_removal_cleanup::TelegramPairingConnectionCleanupAdapter::new(
            Arc::clone(&store_graph.local_runtime.channel_connection_facade_slot),
        )
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("Telegram extension removal cleanup could not be built: {error}"),
        })?,
    ));
    let removal_cleanup = Arc::new(
        ExtensionRemovalCleanupRegistry::try_from_adapters(removal_cleanup_adapters).map_err(
            |error| RebornBuildError::InvalidConfig {
                reason: format!("extension removal cleanup registry could not be built: {error}"),
            },
        )?,
    );
    let account_setups = ExtensionAccountSetupRegistry::default();
    {
        let descriptor =
            ironclaw_telegram_extension::telegram_account_setup_descriptor().map_err(|error| {
                RebornBuildError::InvalidConfig {
                    reason: format!("Telegram account setup could not be declared: {error}"),
                }
            })?;
        if !account_setups.declare(descriptor) {
            return Err(RebornBuildError::InvalidConfig {
                reason: "Telegram account setup was declared more than once".to_string(),
            });
        }
    }
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
        .with_account_setup_registry(account_setups)
        .with_removal_cleanup_registry(removal_cleanup)
        .with_provider_instance_readiness(provider_instance_readiness),
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
        #[cfg(test)]
        let host_runtime_http_egress =
            host_runtime_http_egress_for_test.unwrap_or(host_runtime_http_egress);
        local_runtime.host_runtime_http_egress = host_runtime_http_egress;
        // Attach the admin secret provisioner now the secret-store crypto is
        // built (the store graph was constructed before it existed).
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
        google_oauth_configured,
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
    limits: ironclaw_turns::TurnStateStoreLimits,
) -> FilesystemTurnStateRowStore<F>
where
    F: RootFilesystem + 'static,
{
    FilesystemTurnStateRowStore::new(filesystem).with_limits(limits)
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
async fn build_local_runtime_store_graph(
    input: RebornStoreGraphInput,
) -> Result<RebornStoreGraph, RebornBuildError> {
    let RebornStoreGraphInput {
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
    // #6263 Step 5b — every deployment composes the durable filesystem ROW store
    // (typed journal/delta rows + a hot in-process snapshot cache), unconditionally
    // and with no durability-mode choice: `FilesystemTurnStateRowStore` has exactly
    // one behavior (write-behind, with gate-park/terminal/new-run transitions on a
    // synchronous durability barrier — see `filesystem_store/row_store.rs`). The
    // read-after-submit gap that used to justify pinning to a stricter mode
    // (`get_run_state` et al. returning `ScopeNotFound` for an async-materializing
    // run) was closed in #6263 Step 3.5/read-your-writes: those query paths now
    // serve from the hot cache, so write-behind's query paths are cache-aware. The
    // row store is crash-recoverable (rehydrates from its own rows on boot) and has
    // no per-user `state.json` CAS livelock (journal/row model, not whole-snapshot
    // CAS). Existing deployments migrate automatically: their on-disk
    // block-persistence snapshot at `/turns/state.json` is imported as the row
    // store's first delta on an empty-rows boot
    // (`FilesystemTurnStateRowStore::migrate_legacy_blob_if_needed` reads the SAME
    // path/format the block-persistence sink wrote), so no gate-parked/approval turn
    // is lost on first boot after the flip.
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
    let resource_governor: Arc<ComposedResourceGovernor> = Arc::new(resource_governor);
    let skill_mounts =
        skill_management_mount_view().map_err(|error| RebornBuildError::InvalidConfig {
            reason: error.to_string(),
        })?;
    let capability_policy =
        Arc::new(
            builtin_capability_policy().map_err(|error| RebornBuildError::InvalidConfig {
                reason: format!("local-dev capability policy is invalid: {error}"),
            })?,
        );
    let tool_permission_overrides = Arc::new(ComposedToolPermissionOverrideStore::new(Arc::clone(
        &scoped_filesystem,
    )));
    let auto_approve_settings = Arc::new(ComposedAutoApproveSettingStore::new(Arc::clone(
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
    let host_state_filesystem = local_dev_slack_host_state_filesystem(Arc::clone(&filesystem));
    let telegram_host_state_filesystem =
        local_dev_telegram_host_state_filesystem(Arc::clone(&filesystem));
    let extension_lifecycle_surface_context = local_dev_extension_lifecycle_surface_context(
        owner_user_id.clone(),
        local_runtime_identity.as_ref(),
    )?;
    let skill_management =
        build_local_skill_management_port(owner_user_id.clone(), Arc::clone(&filesystem))?;
    let outbound_stores = local_dev_outbound_store(Arc::clone(&filesystem));
    let local_runtime = Arc::new(RebornRuntimeSubstrate {
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
        outbound_state: outbound_stores.outbound_state,
        delivered_gate_routes: outbound_stores.delivered_gate_routes,
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
        channel_connection_facade_slot: Arc::new(std::sync::OnceLock::new()),
        runtime_http_egress: None,
        host_runtime_http_egress: None,
        skill_mounts,
        memory_mounts,
        system_extensions_lifecycle_mounts,
        skill_filesystem,
        workspace_filesystem,
        host_state_filesystem,
        telegram_host_state_filesystem,
        subagent_goal_filesystem: Arc::clone(&scoped_filesystem),
        identity_filesystem: Arc::clone(&scoped_filesystem),
        // Set later in `build_local_runtime`, once the secret-store crypto
        // exists, via `Arc::get_mut` on this services value.
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

    Ok(RebornStoreGraph {
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
async fn build_local_runtime_store_graph(
    input: RebornStoreGraphInput,
) -> Result<RebornStoreGraph, RebornBuildError> {
    let RebornStoreGraphInput {
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
    // Approval stores run the production `Filesystem*Store<F>` over a dedicated
    // `InMemoryBackend` (volatile) — no bespoke `InMemory*Store`
    // (arch-simplification §4.3). Backing them with `InMemoryBackend` *directly*
    // (rather than the composite root filesystem) keeps the store's concrete type
    // `<InMemoryBackend>`, so the host-runtime production-wiring guard classifies it
    // `LocalOnly` — matching the volatile run-state/lease stores in this build.
    let approvals_filesystem = crate::wrap_scoped(Arc::new(InMemoryBackend::new()));
    let event_log = local_dev_event_log(Arc::clone(&filesystem))?;
    let audit_log = local_dev_audit_log(Arc::clone(&filesystem))?;
    // Run-state and approval-request records live under sibling aliases on the
    // same volatile in-memory backend (§4.3), so both share `approvals_filesystem`
    // (the full-alias in-memory scoped filesystem) — a blocked run and its approval
    // record resolve against one consistent view.
    let run_state = Arc::new(FilesystemRunStateStore::new(Arc::clone(
        &approvals_filesystem,
    )));
    let approval_requests = Arc::new(FilesystemApprovalRequestStore::new(Arc::clone(
        &approvals_filesystem,
    )));
    let capability_leases = Arc::new(FilesystemCapabilityLeaseStore::new(crate::wrap_scoped(
        Arc::new(InMemoryBackend::new()),
    )));
    let persistent_approval_policies = Arc::new(FilesystemPersistentApprovalPolicyStore::new(
        Arc::clone(&approvals_filesystem),
    ));
    // Turn state runs the production `FilesystemTurnStateRowStore` over a
    // dedicated volatile `InMemoryBackend` (§4.3) — no bespoke private
    // turn-state engine standalone authority. Matches the sibling
    // run-state/approval stores in this build (volatile, `LocalOnly`). The row
    // store still has a write-behind tail (only gate-park/terminal/new-run
    // transitions flush synchronously), but the backend is volatile — a restart
    // discards it entirely — so draining that tail at shutdown would persist
    // nothing recoverable; this build intentionally skips the shutdown drain.
    let turn_state = Arc::new(
        FilesystemTurnStateRowStore::new(crate::wrap_scoped(Arc::new(InMemoryBackend::new())))
            .with_limits(turn_state_store_limits),
    );
    // §4.3: checkpoint payloads run the production `FilesystemCheckpointStateStore`
    // over a dedicated volatile `InMemoryBackend`, and checkpoint metadata lives in
    // the turn-state store — the same `LoopCheckpointStore` wiring the durable
    // build path uses (`turn_state.clone()`).
    let checkpoint_state_store: Arc<dyn CheckpointStateStore> = Arc::new(
        FilesystemCheckpointStateStore::new(crate::wrap_scoped(Arc::new(InMemoryBackend::new()))),
    );
    let loop_checkpoint_store: Arc<dyn LoopCheckpointStore> = turn_state.clone();
    let thread_service: Arc<dyn SessionThreadService> =
        Arc::new(InMemorySessionThreadService::default());
    let BudgetSinks {
        budget_event_sink,
        in_memory_budget_event_sink,
        broadcast_budget_event_sink,
    } = build_budget_sinks();
    // §4.3: the deleted `InMemoryBudgetGateStore` is replaced by the one
    // production `FilesystemBudgetGateStore` over an in-memory backend. Unlike
    // the old scope-ignoring HashMap, this scopes each gate under the caller's
    // tenant/user mount — strictly more correct for multi-tenant no-durable
    // deployments — while remaining volatile (fresh `InMemoryBackend`).
    let budget_gate_store: Arc<dyn ironclaw_resources::BudgetGateStore> = Arc::new(
        FilesystemBudgetGateStore::new(crate::wrap_scoped(Arc::new(InMemoryBackend::new()))),
    );
    let resource_governor: Arc<ComposedResourceGovernor> =
        Arc::new(InMemoryResourceGovernor::new().with_event_sink(Arc::clone(&budget_event_sink)));
    let skill_mounts =
        skill_management_mount_view().map_err(|error| RebornBuildError::InvalidConfig {
            reason: error.to_string(),
        })?;
    let capability_policy =
        Arc::new(
            builtin_capability_policy().map_err(|error| RebornBuildError::InvalidConfig {
                reason: format!("local-dev capability policy is invalid: {error}"),
            })?,
        );
    let tool_permission_overrides = Arc::new(ComposedToolPermissionOverrideStore::new(Arc::clone(
        &approvals_filesystem,
    )));
    let auto_approve_settings = Arc::new(ComposedAutoApproveSettingStore::new(Arc::clone(
        &approvals_filesystem,
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
    #[cfg(feature = "postgres")]
    let host_state_filesystem = local_dev_slack_host_state_filesystem(Arc::clone(&filesystem));
    #[cfg(feature = "postgres")]
    let telegram_host_state_filesystem =
        local_dev_telegram_host_state_filesystem(Arc::clone(&filesystem));
    let extension_lifecycle_surface_context = local_dev_extension_lifecycle_surface_context(
        owner_user_id.clone(),
        local_runtime_identity.as_ref(),
    )?;
    let skill_management =
        build_local_skill_management_port(owner_user_id.clone(), Arc::clone(&filesystem))?;
    #[cfg(not(any(feature = "libsql", feature = "postgres")))]
    let trigger_conversation_services = local_dev_trigger_conversation_services();
    let outbound_stores = local_dev_outbound_store(Arc::clone(&filesystem));
    let local_runtime = Arc::new(RebornRuntimeSubstrate {
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
        outbound_state: outbound_stores.outbound_state,
        delivered_gate_routes: outbound_stores.delivered_gate_routes,
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
        channel_connection_facade_slot: Arc::new(std::sync::OnceLock::new()),
        runtime_http_egress: None,
        host_runtime_http_egress: None,
        skill_mounts,
        memory_mounts,
        system_extensions_lifecycle_mounts,
        skill_filesystem,
        workspace_filesystem,
        host_state_filesystem: local_dev_slack_host_state_filesystem(Arc::clone(&filesystem)),
        telegram_host_state_filesystem: local_dev_telegram_host_state_filesystem(Arc::clone(
            &filesystem,
        )),
        subagent_goal_filesystem: local_dev_scoped_filesystem(Arc::clone(&filesystem)),
        identity_filesystem: local_dev_scoped_filesystem(Arc::clone(&filesystem)),
        // Set later in `build_local_runtime`, once secret-store crypto exists.
        admin_secret_provisioner: None,
        extension_filesystem: Arc::clone(&filesystem),
        workspace_mounts,
        local_dev_storage_root,
        default_system_prompt_path,
        event_log,
        audit_log,
        extension_registry: Arc::new(ExtensionRegistry::new()),
        shared_extension_registry: None,
    });
    let process_services =
        ProcessServices::filesystem(crate::wrap_scoped(Arc::new(InMemoryBackend::new())));

    Ok(RebornStoreGraph {
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
    backend: &DurableBackend,
) -> Result<Arc<dyn TriggerRepository>, RebornBuildError> {
    match backend {
        #[cfg(feature = "libsql")]
        DurableBackend::LibSql(database) => {
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
        DurableBackend::Postgres(pool) => {
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
        DurableBackend::Ephemeral => Ok(Arc::new(
            ironclaw_triggers::InMemoryTriggerRepository::default(),
        )),
    }
}

#[cfg(not(any(feature = "libsql", feature = "postgres")))]
fn local_dev_trigger_conversation_services() -> InMemoryConversationServices {
    InMemoryConversationServices::default()
}

fn local_dev_trigger_create_hook(
    local_runtime: &Arc<RebornRuntimeSubstrate>,
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
    runtime: Arc<RebornRuntimeSubstrate>,
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

async fn build_local_runtime_root_filesystem(
    root: &Path,
    workspace_root: &Path,
    host_home_root: Option<&HostHomeRoot>,
    storage_backend_input: StorageBackendInput,
) -> Result<RootFilesystemBundle, RebornBuildError> {
    let local = Arc::new(local_dev_project_filesystem(
        root,
        workspace_root,
        host_home_root,
    )?);
    let mut composite = CompositeRootFilesystem::new();
    let durable_backend = match storage_backend_input {
        #[cfg(feature = "postgres")]
        StorageBackendInput::Postgres(pool) => {
            let database = Arc::new(PostgresRootFilesystem::new(pool.clone()));
            database.run_migrations().await?;
            mount_local_dev_database_roots(&mut composite, database)?;
            DurableBackend::Postgres(pool)
        }
        StorageBackendInput::LocalDefault => {
            build_default_local_dev_database_roots(root, &mut composite).await?
        }
    };
    mount_local_dev_project_roots(&mut composite, local)?;
    Ok(RootFilesystemBundle {
        filesystem: Arc::new(composite),
        durable_backend,
    })
}

/// Filename of the local-dev libSQL database within the per-user root directory.
/// One owner for the string — production factory, integration-test framework, and
/// any on-disk path assertion all derive from this constant.
#[cfg(any(feature = "libsql", feature = "test-support"))]
pub(crate) const LOCAL_DEV_DB_FILENAME: &str = "reborn-local-dev.db";

/// Full path to the local-dev libSQL database file within `root`. The single
/// public accessor for [`LOCAL_DEV_DB_FILENAME`]; callers outside this crate
/// (`ironclaw_reborn_cli`) must use this instead of hardcoding the filename.
#[cfg(any(feature = "libsql", feature = "test-support"))]
pub fn local_dev_db_path(root: &Path) -> PathBuf {
    root.join(LOCAL_DEV_DB_FILENAME)
}

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
    let db_path = local_dev_db_path(root);
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
// stay inside this module (`build_local_runtime_root_filesystem`).
pub(crate) async fn build_default_local_dev_database_roots(
    root: &Path,
    composite: &mut CompositeRootFilesystem,
) -> Result<DurableBackend, RebornBuildError> {
    #[cfg(feature = "libsql")]
    {
        let db = open_local_dev_libsql_database(root).await?;
        let database = Arc::new(LibSqlRootFilesystem::new(Arc::clone(&db)));
        database.run_migrations().await?;
        mount_local_dev_database_roots(composite, database)?;
        Ok(DurableBackend::LibSql(db))
    }
    #[cfg(not(feature = "libsql"))]
    {
        let _ = root;
        tracing::debug!(
            "local-dev: control-plane filesystem roots are backed by InMemoryBackend; runtime state is ephemeral and will be lost on restart"
        );
        mount_local_dev_database_roots(composite, Arc::new(InMemoryBackend::new()))?;
        Ok(DurableBackend::Ephemeral)
    }
}

/// Thin void wrapper over [`build_default_local_dev_database_roots`] for
/// `#[cfg(feature = "test-support")]` callers that need to mount the local-dev
/// database roots but don't need the opaque `DurableBackend` handle
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
    host_home_root: Option<&HostHomeRoot>,
) -> Result<DiskFilesystem, RebornBuildError> {
    let mut filesystem = DiskFilesystem::new();
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
/// `StorageBackendInput::LocalDefault`, whose non-libsql arm mounts a
/// fresh `InMemoryBackend` — under a postgres-composed runtime that would be
/// a brand-new empty store, not the live Postgres-backed host state, so a
/// postgres gate here would compile a probe that can only report absence.
#[cfg(all(feature = "test-support", feature = "libsql"))]
pub(crate) async fn open_local_dev_slack_host_state_filesystem_for_test(
    storage_root: &Path,
) -> Result<Arc<ScopedFilesystem<CompositeRootFilesystem>>, RebornBuildError> {
    let workspace_root = storage_root.join("workspace");
    let bundle = build_local_runtime_root_filesystem(
        storage_root,
        &workspace_root,
        None,
        StorageBackendInput::LocalDefault,
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
/// production `build_local_runtime_store_graph` path uses — so the reopen path
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
/// the SAME way `build_local_runtime_store_graph` does when it first builds
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
        Arc::new(ComposedToolPermissionOverrideStore::new(Arc::clone(
            &scoped,
        )));
    let auto_approve_settings: Arc<dyn ironclaw_approvals::AutoApproveSettingStore> =
        Arc::new(ComposedAutoApproveSettingStore::new(Arc::clone(&scoped)));
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
    local_dev_trigger_repository(&DurableBackend::LibSql(db)).await
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
// (`build_local_runtime_root_filesystem` / `build_default_local_dev_database_roots`).
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
    local: Arc<DiskFilesystem>,
) -> Result<(), RebornBuildError> {
    root.mount(
        local_dev_mount_descriptor(
            "/projects",
            "local-dev-project-files",
            BackendKind::DiskFilesystem,
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
            BackendKind::DiskFilesystem,
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
pub(crate) async fn build_secret_store<F>(
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
        None => resolve_local_dev_secret_master_key(root).await?,
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

/// Open the `/secrets` store alone, without building the rest of the
/// local-dev [`CompositeRootFilesystem`] (project mounts, extension mounts,
/// trigger/project repositories, …).
///
/// - Pre-composition entry point `ironclaw-reborn onboard` needs: it must
///   write a provider API key before a full build-input-driven build exists,
///   and reconstructing the whole composite just to reach one mount is
///   heavy and risks silently diverging from `serve`'s copy.
/// - `/secrets`'s physical backing is the same local-dev libSQL file
///   `build_local_runtime_root_filesystem` opens for `/tenants` in production —
///   a key written here is immediately visible to `serve`, no extra
///   coordination needed.
/// - Uses the same resolver chain as production (env -> cached dotfile ->
///   OS keychain -> generate-and-cache, via [`build_secret_store`]).
/// - `run_migrations()` here and again on `serve`'s later open is safe —
///   already relied on as idempotent elsewhere in this module's tests.
#[cfg(feature = "libsql")]
pub async fn open_local_dev_secret_store(
    root: &Path,
) -> Result<Arc<dyn SecretStore>, RebornBuildError> {
    let db = open_local_dev_libsql_database(root).await?;
    let filesystem = Arc::new(LibSqlRootFilesystem::new(db));
    filesystem.run_migrations().await?;
    let scoped = crate::wrap_scoped(filesystem);
    let (store, _crypto) = build_secret_store(root, scoped, None).await?;
    Ok(store as Arc<dyn SecretStore>)
}

/// Where a resolved local-dev master key came from, used to name the source in
/// fail-loud error messages.
#[cfg(any(feature = "libsql", feature = "postgres"))]
enum MasterKeySource {
    File(PathBuf),
    Env,
    Keychain,
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
            MasterKeySource::Keychain => "the OS keychain".to_string(),
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
async fn resolve_local_dev_secret_master_key(
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
    resolve_local_dev_secret_master_key_with_env(root, env_key).await
}

/// Inner resolver that takes the `SECRETS_MASTER_KEY` env value as a parameter
/// so the write-before-validate invariant can be exercised through this real
/// caller in tests without mutating process-global env (which is racy under
/// `cargo test`'s parallel harness).
///
/// Resolution order: cached dotfile -> explicit/env key -> OS keychain
/// (suppressed under test/CI, see
/// `ironclaw_secrets::keychain::get_master_key`) -> generate a fresh key and
/// persist it to the dotfile. The env key is VALIDATED up front so a bad
/// explicit value fails closed regardless of cached state, but a valid cached
/// dotfile deliberately wins over it: the existing secret store is encrypted
/// under the cached key, and silently switching to a different env key would
/// make that store undecryptable. A keychain hit is returned as-is and never
/// written to the dotfile — the dotfile and keychain are alternative sources
/// for the same secret, not layered, so writing both would mean the two
/// copies must agree forever.
#[cfg(any(feature = "libsql", feature = "postgres"))]
async fn resolve_local_dev_secret_master_key_with_env(
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

    // No cached file. Prefer the explicit (already-validated) env key.
    if let Some(key) = env_key {
        write_local_dev_secret_master_key(&key_path, &key)?;
        return Ok(ironclaw_secrets::SecretMaterial::from(key));
    }

    // No env key either. Try the OS keychain next (suppressed under test/CI —
    // see `ironclaw_secrets::keychain::get_master_key`, which returns
    // `NotFound` when suppressed so this falls through exactly as it would
    // for a genuinely empty keychain). Deliberately calling `get_master_key`
    // directly rather than `resolve_master_key_material`: this resolver
    // already owns the env-var branch above, and `resolve_master_key_material`
    // re-checks the env var itself — calling it here would mean two
    // independent env-precedence implementations that could disagree.
    match ironclaw_secrets::keychain::get_master_key().await {
        Ok(key_bytes) => {
            let key_hex = key_bytes
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>();
            validate_resolved_master_key(&key_hex, &MasterKeySource::Keychain)?;
            // Keychain hit: return as-is, do not also write the dotfile — the
            // dotfile and keychain are alternative sources, not layered.
            return Ok(ironclaw_secrets::SecretMaterial::from(key_hex));
        }
        Err(_) => {
            // Miss or error (including suppressed-under-test): fall through
            // to generating a fresh key, unchanged from prior behavior.
            //
            // Accepted risk: intentionally blanket — this collapses "no key
            // in the keychain yet" and "keychain unreachable" into the same
            // fallback. Headless containers (e.g. Railway) have no
            // secret-service daemon at all, so `get_master_key` returns a
            // generic `SecretError::KeychainError` there, not a distinguishable
            // `NotFound`; narrowing this match to only fall through on
            // `NotFound` would make every container boot fail closed instead
            // of falling back to the dotfile. Worst case of the current
            // broad match: a transient keychain error on a real desktop
            // causes a wrongly-regenerated dotfile key, which just means
            // re-entering one API key on the next `onboard`/`serve` run.
        }
    }

    // No cached file, no env key, no keychain hit. Generate a fresh key.
    let key = ironclaw_secrets::keychain::generate_master_key_hex();
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

/// Outcome of provisioning a local-dev secrets master key directly into the
/// OS keychain (as opposed to `resolve_local_dev_secret_master_key_with_env`'s
/// full resolution chain, which is only consulted at boot time). Used by
/// `onboard`'s standalone keychain-provisioning step.
#[cfg(any(feature = "libsql", feature = "postgres"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeychainMasterKeyOutcome {
    /// The OS keychain already has a master key from a prior onboarding run.
    AlreadyPresent,
    /// A fresh key was generated and stored in the OS keychain.
    Provisioned,
    /// The OS keychain is unavailable (suppressed under test/CI, or the OS
    /// denied the write).
    Suppressed,
}

/// Facade over `ironclaw_secrets::keychain` for onboarding's OS-keychain
/// master-key provisioning step.
///
/// - Lets callers outside this crate (`ironclaw_reborn_cli`) avoid their own
///   `ironclaw_secrets` dependency — pinned by
///   `reborn_dependency_boundaries.rs::reborn_cli_binary_crate_stays_separate_from_v1_root`.
/// - No key yet -> generate + store; already populated -> no-op `AlreadyPresent`.
/// - Never returns an error: unavailable/denied keychain reports `Suppressed`,
///   matching `resolve_local_dev_secret_master_key_with_env`'s env/dotfile fallback.
#[cfg(any(feature = "libsql", feature = "postgres"))]
pub async fn provision_local_dev_keychain_master_key() -> KeychainMasterKeyOutcome {
    // `has_master_key()` collapses "no key yet" and "backend/permission/locked
    // error probing the keychain" into the same `false` — a false negative
    // here falls through to `generate` + `store` below, which overwrites
    // whatever key the keychain actually holds. Same accepted-risk class as
    // the TOCTOU documented on this function's only caller
    // (`ironclaw_reborn_cli::commands::onboard::master_key::provision_master_key`):
    // LocalDev, single-operator, run-once-by-hand; worst case is a
    // wrongly-regenerated key recoverable by re-entering one API key.
    if ironclaw_secrets::keychain::has_master_key().await {
        return KeychainMasterKeyOutcome::AlreadyPresent;
    }
    let key = ironclaw_secrets::keychain::generate_master_key();
    match ironclaw_secrets::keychain::store_master_key(&key).await {
        Ok(()) => KeychainMasterKeyOutcome::Provisioned,
        Err(error) => {
            tracing::debug!(
                %error,
                "OS keychain store of local-dev secrets master key failed during onboarding; \
                 falling back to env/dotfile resolution"
            );
            KeychainMasterKeyOutcome::Suppressed
        }
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

fn local_dev_scoped_filesystem(
    filesystem: Arc<CompositeRootFilesystem>,
) -> Arc<ScopedFilesystem<CompositeRootFilesystem>> {
    crate::wrap_scoped(filesystem)
}

/// Unified bundle of outbound store handles returned by both cfg variants of
/// [`local_dev_outbound_store`].
///
/// All four trait roles must be satisfied on construction.  Every role is an
/// `Arc` clone of a single `FilesystemOutboundStateStore` — which implements all
/// four outbound-store traits — so the WebUI delivery-defaults facade and the
/// Slack delivery path share one backing tree.  The durable build (libsql or
/// postgres) and the non-durable build (in-memory backend) use the SAME wiring;
/// the arch-simplification §4.3 store consolidation deleted the parallel
/// `InMemoryOutboundStateStore`, closing the former non-durable cross-store gap
/// (where `DeliveredGateRouteStore`/`TriggeredRunDeliveryStore` used separate
/// in-memory instances not visible to the shared preference tree).
/// See docs/plans/2026-05-29-trigger-loop-delivery-resolution-implementation.md.
pub(crate) struct OutboundStores {
    pub(crate) outbound_preferences: Arc<dyn CommunicationPreferenceRepository>,
    pub(crate) outbound_state: Arc<dyn OutboundStateStore>,
    pub(crate) delivered_gate_routes: Arc<dyn DeliveredGateRouteStore>,
    pub(crate) triggered_run_delivery: Arc<dyn TriggeredRunDeliveryStore>,
}

fn local_dev_outbound_store(filesystem: Arc<CompositeRootFilesystem>) -> OutboundStores {
    // One store instance over the composition-owned per-user scoped filesystem
    // (`/outbound` → `/tenants/<t>/users/<u>/outbound`). All four outbound
    // roles — preferences, state, delivered-gate routes, triggered-run delivery
    // — are Arc-cloned from this single instance so the WebUI delivery-defaults
    // facade and the Slack delivery path share the same backing tree. Works in
    // both durable (libsql/postgres) and no-durable (in-memory backend) builds
    // because `CompositeRootFilesystem` is `CompositeRootFilesystem` in both.
    // composition-owned construction site, the only one allowed.
    #[allow(clippy::disallowed_methods)]
    let store: Arc<FilesystemOutboundStateStore<CompositeRootFilesystem>> = Arc::new(
        FilesystemOutboundStateStore::new(local_dev_scoped_filesystem(filesystem)),
    );
    OutboundStores {
        outbound_preferences: Arc::clone(&store) as Arc<dyn CommunicationPreferenceRepository>,
        outbound_state: Arc::clone(&store) as Arc<dyn OutboundStateStore>,
        delivered_gate_routes: Arc::clone(&store) as Arc<dyn DeliveredGateRouteStore>,
        triggered_run_delivery: store as Arc<dyn TriggeredRunDeliveryStore>,
    }
}

fn local_dev_slack_host_state_filesystem(
    filesystem: Arc<CompositeRootFilesystem>,
) -> Arc<ScopedFilesystem<CompositeRootFilesystem>> {
    Arc::new(ScopedFilesystem::new(
        filesystem,
        crate::slack_host_state_mount_view,
    ))
}

fn local_dev_telegram_host_state_filesystem(
    filesystem: Arc<CompositeRootFilesystem>,
) -> Arc<ScopedFilesystem<dyn RootFilesystem>> {
    let filesystem: Arc<dyn RootFilesystem> = filesystem;
    Arc::new(ScopedFilesystem::new(
        filesystem,
        crate::telegram_host_state_mount_view,
    ))
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
fn local_dev_event_log(
    filesystem: Arc<CompositeRootFilesystem>,
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
    filesystem: Arc<CompositeRootFilesystem>,
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
    _filesystem: Arc<CompositeRootFilesystem>,
) -> Result<Arc<dyn DurableEventLog>, RebornBuildError> {
    Ok(Arc::new(InMemoryDurableEventLog::new()))
}

#[cfg(not(any(feature = "libsql", feature = "postgres")))]
fn local_dev_audit_log(
    _filesystem: Arc<CompositeRootFilesystem>,
) -> Result<Arc<dyn DurableAuditLog>, RebornBuildError> {
    Ok(Arc::new(InMemoryDurableAuditLog::new()))
}

fn canonicalize_local_dev_path(path: &Path, label: &str) -> Result<PathBuf, RebornBuildError> {
    std::fs::canonicalize(path).map_err(|_| RebornBuildError::InvalidConfig {
        reason: format!("local-dev {label} could not be resolved"),
    })
}

struct HostHomeRoot {
    canonical_root: PathBuf,
    raw_alias: PathBuf,
}

impl HostHomeRoot {
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
    filesystem: Arc<CompositeRootFilesystem>,
    workspace_root: &Path,
    host_home_root: Option<&HostHomeRoot>,
) -> Result<WorkspaceFilesystems, RebornBuildError> {
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
    let policy = builtin_capability_policy().map_err(|error| RebornBuildError::InvalidConfig {
        reason: format!("local-dev capability policy is invalid: {error}"),
    })?;
    let mut entries = vec![
        AdminEntry::for_local_manifest(
            policy.provider.id,
            policy.provider.manifest_path,
            None,
            HostTrustAssignment::first_party(),
            // Sourced from builtin_capability_policy.toml `[provider]
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
    // Zero-tool channel package (like slack_bot): activation registers the
    // channel surface only, so no capability effects are granted.
    entries.push(AdminEntry::for_local_manifest(
        PackageId::new("telegram").map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("Telegram first-party package id is invalid: {error}"),
        })?,
        "/system/extensions/telegram/manifest.toml".to_string(),
        Some(telegram_manifest_digest()),
        HostTrustAssignment::first_party(),
        Vec::new(),
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
        deployment,
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
        #[cfg(test)]
            host_runtime_http_egress_for_test: _,
        #[cfg(any(test, feature = "test-support"))]
            network_http_egress_for_test: _,
        product_auth_ports,
        oauth_provider_configs,
        oauth_dcr_provider_configs,
        slack_personal_oauth_lazy_slot,
        // Build-time Slack host-beta signal only feeds
        // `provider_instance_readiness_map`, consumed exclusively by
        // `build_local_runtime`'s `RebornLocalExtensionManagementPort`
        // wiring — production composition has no extension lifecycle port
        // yet (#4091), so this build path has no consumer for it.
        slack_host_beta_enabled: _,
        slack_personal_oauth_redirect_uri_configured: _,
        nearai_mcp_bootstrap_config: _,
        turn_state_store_limits,
    } = input;
    // Label for logging/errors; behaviour reads `deployment`'s axes.
    let profile = deployment.profile();
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
    slack_personal_oauth_lazy_slot:
        Option<crate::slack::slack_setup::SlackPersonalSetupServiceSlot>,
    owner_id: String,
    local_runtime_identity: Option<RebornLocalRuntimeIdentity>,
    turn_state_store_limits: ironclaw_turns::TurnStateStoreLimits,
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
        ironclaw_turns::TurnStateStoreLimits::default(),
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
    // Wire the operator post-edit check in production too (off unless
    // IRONCLAW_POST_EDIT_CHECK is set). It runs isolated in the tenant sandbox
    // per the runtime process binding applied above; the resolver routes it to
    // the tenant-sandbox process port rather than the provider host.
    let services = match PostEditCheckConfig::from_env() {
        Ok(Some(config)) => services.with_post_edit_check(config),
        Ok(None) => services,
        Err(error) => {
            return Err(crate::RebornCompositionError::InvalidConfig {
                reason: error.to_string(),
            });
        }
    };

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
    /// Retained so `build_backend_production` can build the admin secret
    /// provisioner over the SAME crypto the runtime's own secret store uses —
    /// material written by the provisioner must decrypt under the user's own
    /// store and vice versa (mirrors the local `local_dev_secret_bundle.1`).
    crypto: Arc<ironclaw_secrets::SecretsCrypto>,
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
            credential_broker: Arc::new(FilesystemCredentialBroker::new(
                scoped_filesystem,
                Arc::clone(&crypto),
            )),
            crypto,
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
        slack_personal_oauth_lazy_slot,
        owner_id,
        local_runtime_identity,
        turn_state_store_limits,
        scheduler_wake_wiring,
    } = context;
    // Computed before `oauth_provider_configs` is consumed by
    // `compose_provider_client` below — see `google_oauth_configured`.
    let google_oauth_configured = google_oauth_configured(&oauth_provider_configs);
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
        owner_user_id.clone(),
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
    // Admin per-user secret provisioner over the raw production root and the
    // SAME crypto the runtime's own secret store uses, so material written for
    // a target user decrypts under that user's own store (mirrors the local
    // substrate's `admin_secret_provisioner`; see `admin_secrets.rs`).
    let admin_secret_provisioner: Arc<dyn crate::admin_secrets::AdminSecretProvisioner> =
        Arc::new(crate::admin_secrets::FilesystemAdminSecretProvisioner::new(
            Arc::clone(&stores.filesystem),
            Arc::clone(&stores.secret_credentials.crypto),
        ));
    // Projects persist over the production scoped filesystem (tenant supplied
    // per call; the scope carries only the control-plane owner/agent identity),
    // exactly as the local substrate builds them — see the `local_runtime`
    // project repository. Production is always durable, so there is no
    // in-memory fallback arm here.
    let project_agent_id = ironclaw_host_api::AgentId::new("reborn-projects").map_err(|error| {
        RebornBuildError::InvalidConfig {
            reason: format!("invalid project agent id: {error}"),
        }
    })?;
    let project_repository: Arc<dyn ProjectRepository> =
        Arc::new(ironclaw_projects::FilesystemProjectRepository::new(
            Arc::clone(&stores.scoped_filesystem),
            owner_user_id.clone(),
            project_agent_id,
        ));
    let project_service: Arc<dyn ProjectService> =
        Arc::new(RebornProjectService::new(project_repository));
    // Trigger conversation services over the production scoped filesystem —
    // the substrate-agnostic trigger poller (`runtime.rs`) sources the
    // materializer/submitter/pairing roles from here for production profiles,
    // exactly as the local substrate serves them from its own conversation
    // services. Built eagerly (production is always durable); the underlying
    // `InboundTurnError` cause is preserved in the mapped build error.
    let trigger_conversation_services =
        RebornFilesystemConversationServices::new(Arc::clone(&stores.scoped_filesystem))
            .await
            .map_err(|error| RebornBuildError::InvalidConfig {
                reason: format!("trigger conversation services unavailable: {error}"),
            })?;
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
        admin_secret_provisioner,
        project_service,
        trigger_conversation_services,
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
        slack_personal_oauth_lazy_slot,
    )?;
    let services = apply_production_runtime_process_binding(
        services,
        production_wiring.runtime_process_binding,
    );
    // Wire the operator post-edit check in production too (off unless
    // IRONCLAW_POST_EDIT_CHECK is set); it runs isolated in the tenant sandbox
    // per the process binding applied above.
    let services = apply_post_edit_check_from_env(services)?;
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
        google_oauth_configured,
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
mod tests;

#[cfg(test)]
mod auth_tests;
#[cfg(test)]
mod local_dev_host_tests;
