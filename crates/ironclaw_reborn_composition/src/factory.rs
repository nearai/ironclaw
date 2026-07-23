// arch-exempt: large_file, needs Reborn composition helper extraction, plan #4469
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    sync::Arc,
    sync::atomic::AtomicBool,
};

use crate::RebornProductAuthServicePorts;
#[cfg(any(test, feature = "test-support"))]
use crate::builtin_capability_policy::BuiltinCapabilityPolicy;
use crate::builtin_capability_policy::builtin_capability_policy;
use crate::deployment::TrafficPolicy;
use crate::extension_host::host_api_contracts::product_extension_host_api_contract_registry;
use crate::extension_host::lifecycle::RebornLocalSkillManagementPort;
use crate::extension_host::mcp::hosted_http_mcp_runtime;
use crate::extension_host::{
    admin_configuration::ComposedAdminConfigurationService,
    admin_configuration_capability::{
        extend_builtin_first_party_package as extend_builtin_admin_configuration_package,
        insert_handler as insert_admin_configuration_handler,
    },
    available_extensions::{AdminConfigurationCatalogUse, AvailableExtensionCatalog},
    extension_lifecycle::{
        ActiveExtensionPublisher, ExtensionCredentialCleanup, RebornLocalExtensionManagementPort,
        restore_extension_lifecycle_state,
    },
    extension_lifecycle_capabilities::{
        extend_builtin_first_party_package, insert_handlers as insert_extension_lifecycle_handlers,
    },
    extension_removal_cleanup::{ExtensionRemovalCleanupAdapter, ExtensionRemovalCleanupRegistry},
    first_party::{FirstPartyRegistrarContext, first_party_reserved_extension_ids},
    operator_config_capability::{
        extend_builtin_first_party_package as extend_builtin_operator_config_package,
        insert_handler as insert_operator_config_handler,
    },
    provider_instance_readiness::{
        ProviderInstanceReadinessInput, provider_instance_readiness_map,
    },
    skill_auto_activate_capability::{
        extend_builtin_first_party_package as extend_builtin_skill_auto_activate_package,
        insert_handler as insert_skill_auto_activate_handler,
    },
};
use crate::input::{
    LibsqlConnectionConfig, PostgresPoolSource, RebornLocalRuntimeIdentity,
    RebornRuntimeProcessBinding, RebornStorageInput,
};
use crate::local_dev_authorization::{StoreApprovalSettingsProvider, local_dev_authorizer};
use crate::local_dev_mounts::{
    ambient_workspace_mount_view, memory_mount_view, scoped_skill_context_mount_view,
    skill_management_mount_view, workspace_mount_view,
};
use crate::operator_tool_catalog::ActiveRegistryOperatorToolCatalog;
use crate::outbound::outbound_preferences_capability::{
    extend_builtin_first_party_package as extend_builtin_outbound_preferences_package,
    insert_handler as insert_outbound_preferences_handler,
};
use crate::outbound::{
    outbound_delivery_synthetic_provider, outbound_delivery_target_set_operator_tool_info,
};
use crate::product_auth::credentials::product_auth_providers::{
    OAuthProviderComposition, compose_provider_client,
};
use crate::product_auth::credentials::runtime_credentials::ProductAuthRuntimeCredentialResolver;
use crate::product_auth::durable::{FilesystemAuthProductServices, UnavailableAuthProviderClient};
use crate::root::default_system_prompt::seed_default_system_prompt;
use crate::runtime_input::RebornRuntimeIdentity;
use crate::storage_catalog::validate_reborn_runtime_storage;
use crate::support::fs::RebornProjectService;
use crate::{
    RebornAuthContinuationDispatcher, RebornBuildError, RebornCompositionProfile,
    RebornFacadeReadiness, RebornHostBindings, RebornProductAuthServices, RebornReadiness,
    RebornWorkerReadiness,
};
use ironclaw_approvals::{
    FilesystemAutoApproveSettingStore, FilesystemPersistentApprovalPolicyStore,
    FilesystemToolPermissionOverrideStore,
};
use ironclaw_auth::{AuthProductScope, AuthSurface};
use ironclaw_authorization::FilesystemCapabilityLeaseStore;
use ironclaw_authorization::GrantAuthorizer;
use ironclaw_conversations::RebornFilesystemConversationServices;
use ironclaw_conversations::{
    AdapterInstallationId, AdapterKind, ConversationActorPairingService, ExternalActorRef,
};
use ironclaw_events::{DurableAuditLog, DurableEventLog};
use ironclaw_extension_host::{AdminConfigurationService, FilesystemAdminConfigurationStore};
use ironclaw_extensions::{
    ExtensionInstallationStore, ExtensionLifecycleService, ExtensionRegistry,
    FilesystemExtensionInstallationStore, SharedExtensionRegistry,
};
use ironclaw_filesystem::LibSqlRootFilesystem;
use ironclaw_filesystem::PostgresRootFilesystem;
use ironclaw_filesystem::{
    BackendCapabilities, BackendId, BackendKind, CompositeRootFilesystem, ContentKind, IndexPolicy,
    MountDescriptor, RootFilesystem, StorageClass,
};
use ironclaw_filesystem::{DiskFilesystem, ScopedFilesystem};
use ironclaw_host_api::runtime_policy::{
    DeploymentMode, EffectiveRuntimePolicy, FilesystemBackendKind, NetworkMode, ProcessBackendKind,
    SecretMode,
};
use ironclaw_host_api::{HostApiError, MountAlias, MountGrant};
use ironclaw_host_api::{
    HostPath, InvocationId, MountPermissions, MountView, PackageId, ResourceScope,
    RuntimeHttpEgress, UserId, VendorId, VirtualPath, sha256_digest_token,
};
use ironclaw_host_runtime::{
    CapabilitySurfaceVersion, FirstPartyCapabilityRegistry, HostProcessPort, HostRuntimeServices,
    PostEditCheckConfig, ProductAuthProviderRuntimePorts, TriggerCreateHook,
    builtin_first_party_package,
};
use ironclaw_host_runtime::{
    builtin_first_party_handlers_with_trigger_create_hook_for_process_backend,
    builtin_first_party_package_for_process_backend,
};
use ironclaw_loop_host::FilesystemCheckpointStateStore;
use ironclaw_outbound::CommunicationPreferenceRepository;
use ironclaw_outbound::FilesystemOutboundStateStore;
use ironclaw_outbound::{DeliveredGateRouteStore, OutboundStateStore, TriggeredRunDeliveryStore};
use ironclaw_processes::ProcessServices;
use ironclaw_product_workflow::{
    ExtensionAccountSetupRegistry, LifecycleProductSurfaceContext,
    OutboundPreferencesProductFacade, ProductAuthTurnGateResumeDispatcher, ProjectService,
};
use ironclaw_projects::ProjectRepository;
use ironclaw_resources::FilesystemBudgetGateStore;
use ironclaw_resources::InMemoryResourceGovernor;
use ironclaw_resources::{
    BroadcastBudgetEventSink, BudgetGateStore, FilesystemResourceGovernor, ResourceGovernor,
};
use ironclaw_run_state::FilesystemApprovalRequestStore;
use ironclaw_secrets::FilesystemCredentialBroker;
use ironclaw_secrets::FilesystemSecretStore;
use ironclaw_secrets::SecretStore;
use ironclaw_threads::FilesystemSessionThreadService;
use ironclaw_threads::SessionThreadService;
use ironclaw_triggers::{
    TRIGGER_TRUSTED_ADAPTER_INSTALLATION_ID, TRIGGER_TRUSTED_ADAPTER_KIND,
    TRIGGER_TRUSTED_EXTERNAL_ACTOR_NAMESPACE, TriggerActiveRunLookup, TriggerError, TriggerRecord,
    TriggerRepository,
};
use ironclaw_trust::{AdminConfig, AdminEntry, HostTrustAssignment, HostTrustPolicy};
use ironclaw_turns::FilesystemTurnStateRowStore;
use ironclaw_turns::InMemoryRunProfileResolver;
use ironclaw_turns::{
    CheckpointStateStore, ExternalToolCatalog, InMemoryExternalToolCatalog, LoopCheckpointStore,
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
    LibSql(Arc<libsql::Database>),
    Postgres(deadpool_postgres::Pool),
}

enum StorageBackendInput {
    LocalDefault,
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
pub const LOCAL_DEV_SECRETS_MASTER_KEY_PATH: &str = ".reborn-local-dev-secrets-master-key";

/// The ONE construction seam for host HTTP egress: policy enforcement over
/// the reqwest transport, honoring the env-gated test-only host rewrite map
/// ([`ironclaw_network::TEST_HTTP_REWRITE_MAP_ENV`]). Every composition path
/// builds its vendor egress here so test runs redirect ALL vendor calls
/// identically. Fail-closed: a set-but-invalid map refuses composition.
fn default_host_http_egress() -> Result<
    ironclaw_network::PolicyNetworkHttpEgress<
        ironclaw_network::RewriteNetworkTransport<ironclaw_network::ReqwestNetworkTransport>,
    >,
    RebornBuildError,
> {
    ironclaw_network::default_policy_http_egress().map_err(|error| {
        RebornBuildError::InvalidConfig {
            reason: error.to_string(),
        }
    })
}

/// Test-support pass-through so a `#[cfg]`-gated injected
/// `Arc<dyn NetworkHttpEgress>` (there is no blanket `NetworkHttpEgress` impl on
/// `Arc<dyn …>`) satisfies the generic `try_with_host_http_egress_with_body_store`
/// bound. Consumes `RebornHostBindings::network_http_egress_for_test`, letting a
/// unit/integration test drive hosted-MCP discovery and any host HTTP egress
/// over a fake transport instead of the real network. Restores the consumer
/// dropped in commit 975bcd2ce ("Unify reborn runtime assembly"), which
/// collapsed the two build paths and left the injected egress unread.
#[cfg(any(test, feature = "test-support"))]
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

type ComposedResourceGovernor = FilesystemResourceGovernor<CompositeRootFilesystem>;

pub(crate) type ComposedApprovalRequestStore =
    FilesystemApprovalRequestStore<CompositeRootFilesystem>;

pub(crate) type ComposedCapabilityLeaseStore =
    FilesystemCapabilityLeaseStore<CompositeRootFilesystem>;

pub(crate) type ComposedPersistentApprovalPolicyStore =
    FilesystemPersistentApprovalPolicyStore<CompositeRootFilesystem>;

pub(crate) type ComposedToolPermissionOverrideStore =
    FilesystemToolPermissionOverrideStore<CompositeRootFilesystem>;

pub(crate) type ComposedAutoApproveSettingStore =
    FilesystemAutoApproveSettingStore<CompositeRootFilesystem>;

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

pub(crate) struct RebornRuntimeStores {
    pub(crate) host_runtime: Arc<dyn ironclaw_host_runtime::HostRuntime>,
    #[allow(dead_code)]
    pub(crate) turn_coordinator: Arc<dyn ironclaw_turns::TurnCoordinator>,
    pub(crate) product_auth: Arc<RebornProductAuthServices>,
    pub(crate) readiness: RebornReadiness,
    pub(crate) skill_management: Arc<RebornLocalSkillManagementPort>,
    pub(crate) extension_lifecycle_surface_context: LifecycleProductSurfaceContext,
    pub(crate) owner_user_id: UserId,
    pub(crate) approval_requests: Arc<ComposedApprovalRequestStore>,
    pub(crate) capability_leases: Arc<ComposedCapabilityLeaseStore>,
    pub(crate) external_tool_catalog: Arc<dyn ExternalToolCatalog>,
    pub(crate) runtime_policy: Option<EffectiveRuntimePolicy>,
    pub(crate) persistent_approval_policies: Arc<ComposedPersistentApprovalPolicyStore>,
    pub(crate) tool_permission_overrides: Arc<ComposedToolPermissionOverrideStore>,
    pub(crate) auto_approve_settings: Arc<ComposedAutoApproveSettingStore>,
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) capability_policy: Arc<BuiltinCapabilityPolicy>,
    pub(crate) outbound_preferences: Arc<dyn CommunicationPreferenceRepository>,
    pub(crate) outbound_delivery_targets:
        Arc<crate::outbound::MutableOutboundDeliveryTargetRegistry>,
    pub(crate) skill_auto_activate_learned: Arc<AtomicBool>,
    pub(crate) outbound_state: Arc<dyn OutboundStateStore>,
    pub(crate) delivered_gate_routes: Arc<dyn DeliveredGateRouteStore>,
    pub(crate) triggered_run_delivery: Arc<dyn TriggeredRunDeliveryStore>,
    pub(crate) extension_management: Arc<RebornLocalExtensionManagementPort>,
    pub(crate) channel_config: Arc<crate::extension_host::channel_config::ChannelConfigService>,
    pub(crate) admin_configuration: Arc<ComposedAdminConfigurationService>,
    pub(crate) admin_configuration_uses: Arc<Vec<AdminConfigurationCatalogUse>>,
    pub(crate) channel_identity_store:
        Arc<crate::extension_host::channel_identity_store::FilesystemChannelIdentityStore>,
    pub(crate) channel_dm_target_store:
        Arc<crate::extension_host::channel_dm_targets::FilesystemChannelDmTargetStore>,
    pub(crate) channel_disconnect_slot:
        Arc<std::sync::OnceLock<Arc<dyn ironclaw_product_workflow::ChannelConnectionFacade>>>,
    pub(crate) runtime_http_egress: Option<Arc<dyn RuntimeHttpEgress>>,
    pub(crate) skill_mounts: MountView,
    pub(crate) memory_mounts: MountView,
    pub(crate) system_extensions_lifecycle_mounts: MountView,
    pub(crate) skill_filesystem: Arc<ScopedFilesystem<CompositeRootFilesystem>>,
    pub(crate) workspace_filesystem: Arc<ScopedFilesystem<CompositeRootFilesystem>>,
    pub(crate) extension_filesystem: Arc<CompositeRootFilesystem>,
    pub(crate) workspace_mounts: MountView,
    pub(crate) local_dev_storage_root: Option<PathBuf>,
    pub(crate) default_system_prompt_path: Option<PathBuf>,
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) in_memory_budget_event_sink: Arc<ironclaw_resources::InMemoryBudgetEventSink>,
    pub(crate) extension_registry: Arc<ExtensionRegistry>,
    pub(crate) shared_extension_registry: Arc<SharedExtensionRegistry>,
    pub(crate) scoped_filesystem: Arc<ScopedFilesystem<CompositeRootFilesystem>>,
    pub(crate) turn_state: Arc<FilesystemTurnStateRowStore<CompositeRootFilesystem>>,
    pub(crate) checkpoint_state_store: Arc<dyn CheckpointStateStore>,
    pub(crate) loop_checkpoint_store: Arc<dyn LoopCheckpointStore>,
    pub(crate) thread_service: Arc<dyn SessionThreadService>,
    pub(crate) trigger_repository: Arc<dyn TriggerRepository>,
    pub(crate) resource_governor: Arc<dyn ResourceGovernor>,
    pub(crate) budget_gate_store: Arc<dyn BudgetGateStore>,
    pub(crate) broadcast_budget_event_sink: Arc<BroadcastBudgetEventSink>,
    pub(crate) event_log: Arc<dyn DurableEventLog>,
    pub(crate) audit_log: Arc<dyn DurableAuditLog>,
    pub(crate) admin_secret_provisioner: Arc<dyn crate::admin_secrets::AdminSecretProvisioner>,
    pub(crate) project_service: Arc<dyn ProjectService>,
    pub(crate) trigger_conversation_services: RebornFilesystemConversationServices,
    /// Pre-minted scheduler wake wiring for the production composition path.
    /// Minted in `build_production_shaped` so the notifier can satisfy
    /// `HostRuntimeServices.with_turn_run_wake_notifier_dyn` before
    /// `build_default_planned_runtime` runs; consumed by `build_reborn_runtime`
    /// via `DefaultPlannedRuntimeParts.scheduler_wake_wiring` so the scheduler
    /// loop driven by that function shares the exact same channel.
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
    pub(crate) credential_refresh_worker: CredentialRefreshWorkerReady,
    /// The binary-assembled channel-extension bindings (extension-runtime
    /// DEL-7): adapters were handed to the generic host at build; the extras
    /// are consumed by `build_reborn_runtime` when the channel host assembly
    /// starts.
    pub(crate) channel_extension_bindings: Vec<crate::input::ChannelExtensionBinding>,
    /// Manifest-declared deployment channel surfaces, independent of user
    /// installation/activation state.
    pub(crate) deployment_channels: Arc<ironclaw_extension_host::DeploymentChannelRegistry>,
    /// The composed generic channel ingress (extension-runtime P4): the
    /// deployment-first router plus its active-snapshot compatibility lane and
    /// per-extension registration surface. `None` on composition paths that do
    /// not build the generic extension host.
    pub(crate) extension_ingress:
        Option<crate::extension_host::extension_ingress::ExtensionIngressParts>,
    /// Pairing services for `WebGeneratedCode` channel extensions, built
    /// from the binary-assembled account-setup descriptors; the channel host
    /// assembly consumes it for sink gates and actor resolution.
    pub(crate) channel_pairing:
        Option<Arc<crate::extension_host::channel_pairing::ChannelPairingRegistry>>,
    /// The generic delivery coordinator (extension-runtime §5.4): the sole
    /// writer of outbound delivery state, resolving channel adapters +
    /// policy egress from deployment bindings or the active compatibility
    /// snapshot. `None` when the composition path builds no channel egress
    /// transport.
    pub(crate) delivery_coordinator: Option<Arc<ironclaw_product_workflow::DeliveryCoordinator>>,
    /// The deployment-first channel delivery resolver behind the coordinator,
    /// exposed separately for host flows (e.g. DM target provisioning) that
    /// need one stable adapter + egress read outside a delivery.
    // Consumed by the DM-provisioning re-point in the deletion slice.
    #[allow(dead_code)]
    pub(crate) channel_delivery_resolver:
        Option<Arc<dyn ironclaw_product_workflow::ChannelDeliveryResolver>>,
    /// Registry of beta-era channel credential bridges (§11 compatibility):
    /// channel hosts whose secrets predate the extension-config store
    /// register resolution ports here.
    #[cfg(feature = "test-support")]
    #[allow(dead_code)]
    pub(crate) channel_egress_credential_bridges:
        Option<Arc<crate::extension_host::channel_egress::BridgedChannelEgressCredentials>>,
}

struct ChannelHostWiring {
    extension_ingress: Option<crate::extension_host::extension_ingress::ExtensionIngressParts>,
    delivery_coordinator: Option<Arc<ironclaw_product_workflow::DeliveryCoordinator>>,
    channel_delivery_resolver: Option<Arc<dyn ironclaw_product_workflow::ChannelDeliveryResolver>>,
    #[cfg(feature = "test-support")]
    channel_egress_credential_bridges:
        Option<Arc<crate::extension_host::channel_egress::BridgedChannelEgressCredentials>>,
}

/// Whether the engine-owned credential keepalive sweep
/// (`ironclaw_auth::keepalive`) can be started, with its dependencies bundled
/// so they cannot be partially wired.
///
/// The dependencies (cross-owner candidate enumeration + recipe data +
/// deployment-wide leader lock + refresh port) are only ever produced together
/// on the durable production path. Bundling them into one `Ready` variant
/// makes the half-configured state — which would silently disable proactive
/// refresh — unrepresentable, so the runtime spawn site is a clean two-arm
/// match with no "enabled but deps missing" branch to forget about.
pub(crate) enum CredentialRefreshWorkerReady {
    /// Deps fully wired (durable production path). The only state that can start
    /// the sweep; the `enabled` policy flag still gates the actual spawn.
    Ready {
        candidate_source: Arc<dyn ironclaw_auth::KeepaliveCandidateSource>,
        /// Active recipe data — declares which vendors carry an idle lifetime
        /// (`refresh.keepalive_idle_seconds`).
        recipes: Arc<dyn ironclaw_auth::AuthRecipeResolver>,
        leader_lock: crate::product_auth::credentials::product_auth_refresh_lock::CredentialRefreshLeaderLock,
        refresh_port: Arc<RebornProductAuthServices>,
    },
    /// Deps intentionally absent: local-dev (single-user, no cross-owner
    /// enumeration), or a caller-supplied `product_auth_ports` override/test
    /// path. The sweep never starts.
    Absent,
}

/// Production wiring for [`RebornRuntimeStores::start_channel_host_assembly`]:
/// the run-world services and identity the per-extension channel workflows
/// bind under, plus the prompt-enrichment ports for the run-delivery
/// observer half.
pub(crate) struct ChannelHostAssemblyWiring {
    pub(crate) thread_service: Arc<dyn SessionThreadService>,
    pub(crate) turn_coordinator: Arc<dyn ironclaw_turns::TurnCoordinator>,
    pub(crate) approval_interaction:
        Option<Arc<dyn ironclaw_product_workflow::ApprovalInteractionService>>,
    pub(crate) auth_interaction: Option<Arc<dyn ironclaw_product_workflow::AuthInteractionService>>,
    pub(crate) identity: crate::extension_host::channel_host::ChannelHostIdentity,
    pub(crate) approval_context:
        Option<Arc<dyn ironclaw_product_workflow::ApprovalPromptContextSource>>,
    pub(crate) blocked_auth_prompts:
        Option<Arc<dyn ironclaw_product_workflow::BlockedAuthPromptSource>>,
    pub(crate) auth_flow_cancel:
        Option<Arc<dyn ironclaw_product_workflow::BlockedAuthFlowCanceller>>,
    pub(crate) run_delivery_settings: ironclaw_product_workflow::RunDeliverySettings,
}

impl RebornRuntimeStores {
    /// Start the generic channel host assembly (extension-runtime P6 S2):
    /// the per-extension inbound-channel reconcile loop over deployment
    /// bindings and the generic host's active compatibility snapshot. `None`
    /// when this composition path has no
    /// generic host, no ingress registry, or no `[channel.config]` service
    /// — there is nothing to reconcile against. The run-delivery observer
    /// half follows the delivery coordinator's availability: without a
    /// coordinator, registrations are ingress-only.
    pub(crate) fn start_channel_host_assembly(
        &self,
        wiring: ChannelHostAssemblyWiring,
    ) -> Option<Arc<crate::extension_host::channel_host::GenericChannelHostAssembly>> {
        use crate::extension_host::channel_host::{
            FilesystemChannelWorkflowStateFactory, GenericChannelHostDeps,
        };

        let ChannelHostAssemblyWiring {
            thread_service,
            turn_coordinator,
            approval_interaction,
            auth_interaction,
            identity,
            approval_context,
            blocked_auth_prompts,
            auth_flow_cancel,
            run_delivery_settings,
        } = wiring;
        let generic_host = self.extension_management.generic_host()?;
        let ingress = self.extension_ingress.as_ref()?;
        let channel_config = self.channel_config.clone();
        let workflow_state = Arc::new(FilesystemChannelWorkflowStateFactory::new(Arc::clone(
            &self.extension_filesystem,
        )));
        let outbound_state = Arc::clone(&self.outbound_state);
        let delivered_gate_routes = Arc::clone(&self.delivered_gate_routes);
        let outbound_preferences = Arc::clone(&self.outbound_preferences);
        let delivery = self.delivery_coordinator.clone().map(|coordinator| {
            crate::extension_host::channel_host::ChannelHostDeliveryDeps {
                coordinator,
                outbound_store: Arc::clone(&outbound_state),
                route_store: Arc::clone(&delivered_gate_routes),
                communication_preferences: Arc::clone(&outbound_preferences),
                approval_context,
                blocked_auth_prompts,
                auth_flow_cancel,
                settings: run_delivery_settings,
            }
        });

        let identity_lookup = Some(Arc::clone(&self.channel_identity_store)
            as Arc<dyn crate::provider_identity::RebornUserIdentityLookup>);
        Some(
            crate::extension_host::channel_host::GenericChannelHostAssembly::start(
                GenericChannelHostDeps {
                    watch: generic_host.snapshot_watch(),
                    deployment_channels: Arc::clone(&self.deployment_channels),
                    registry: Arc::clone(&ingress.registry),
                    channel_config,
                    workflow_state,
                    thread_service,
                    turn_coordinator,
                    approval_interaction,
                    auth_interaction,
                    identity,
                    identity_lookup,
                    delivery,
                    channel_pairing: self.channel_pairing.clone(),
                },
            ),
        )
    }
}

#[cfg(any(test, feature = "test-support"))]
pub(crate) mod test_support;

#[cfg(feature = "test-support")]
pub use test_support::RebornApprovalTestParts;
#[cfg(feature = "test-support")]
pub(crate) use test_support::{
    ActiveExtensionAuthorityForTest, active_extension_authority_for_test,
};
#[cfg(any(test, feature = "test-support"))]
pub use test_support::{AttachmentTestSupport, ChannelHostAssemblyTestWiring};

#[cfg(feature = "test-support")]
pub(crate) use test_support::{
    mount_default_local_dev_database_roots, open_local_dev_approval_request_store_for_test,
    open_local_dev_approval_settings_stores_for_test,
    open_local_dev_extension_installation_store_for_test,
    open_local_dev_outbound_preferences_store_for_test, open_local_dev_root_filesystem_for_test,
    open_local_dev_trigger_repository_for_test,
};

impl std::fmt::Debug for RebornRuntimeStores {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut debug = formatter.debug_struct("RebornRuntimeStores");
        debug
            .field("host_runtime", &"Arc<dyn HostRuntime>")
            .field("turn_coordinator", &"Arc<dyn TurnCoordinator>")
            .field("product_auth", &"Arc<RebornProductAuthServices>")
            .field("readiness", &self.readiness)
            .field("extension_management", &true)
            .field("scoped_filesystem", &"Arc<ScopedFilesystem>")
            .field("turn_state", &"Arc<FilesystemTurnStateRowStore>");
        debug.finish()
    }
}

pub(crate) fn filesystem_reborn_identity_store<F>(
    scoped_filesystem: Arc<ScopedFilesystem<F>>,
    tenant_id: ironclaw_host_api::TenantId,
    actor_user_id: UserId,
    agent_id: ironclaw_host_api::AgentId,
    project_id: Option<ironclaw_host_api::ProjectId>,
) -> Arc<ironclaw_reborn_identity::FilesystemRebornIdentityStore<F>>
where
    F: RootFilesystem + 'static,
{
    Arc::new(
        ironclaw_reborn_identity::FilesystemRebornIdentityStore::new(
            scoped_filesystem,
            tenant_id,
            actor_user_id,
            agent_id,
            project_id,
        ),
    )
}

pub(crate) async fn build_runtime_substrate(
    input: RebornHostBindings,
) -> Result<RebornRuntimeStores, RebornBuildError> {
    tracing::debug!(
        profile = %input.profile(),
        owner_id = %input.owner_id(),
        "building Reborn composition facades"
    );
    // Substrate selection is deployment *data* (§4.4/§5.6), not a profile
    // match: the config says which substrate to assemble and this dispatches
    // on that value.
    let substrate = input.deployment().substrate();
    match substrate {
        crate::deployment::RuntimeSubstrate::None => Err(RebornBuildError::InvalidConfig {
            reason: format!(
                "profile={} does not configure a Reborn runtime substrate",
                input.profile()
            ),
        }),
        crate::deployment::RuntimeSubstrate::ProductionShaped => {
            build_production_shaped(input).await
        }
    }
}

pub(crate) fn auth_continuation_dispatcher(
    turn_coordinator: Arc<dyn ironclaw_turns::TurnCoordinator>,
    blocked_auth_snapshot_source: Option<
        Arc<dyn crate::blocked_auth_resume::BlockedAuthSnapshotSource>,
    >,
) -> Arc<dyn RebornAuthContinuationDispatcher> {
    let single_run: Arc<dyn RebornAuthContinuationDispatcher> = Arc::new(
        ProductAuthTurnGateResumeDispatcher::new(Arc::clone(&turn_coordinator)),
    );
    match blocked_auth_snapshot_source {
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
    }
}

struct ProductAuthServicesCompositionInput {
    ports: RebornProductAuthServicePorts,
    turn_coordinator: Arc<dyn ironclaw_turns::TurnCoordinator>,
    blocked_auth_snapshot_source:
        Option<Arc<dyn crate::blocked_auth_resume::BlockedAuthSnapshotSource>>,
    provider_composition: OAuthProviderComposition,
    security_audit_sink: Option<Arc<dyn ironclaw_events::SecurityAuditSink>>,
    secret_store: Arc<dyn SecretStore>,
    nearai_mcp_host_managed_scope: Option<AuthProductScope>,
    credential_account_visibility_policy: Option<
        Arc<dyn crate::product_auth::credentials::runtime_credentials::RuntimeCredentialAccountVisibilityPolicy>,
    >,
    /// Durable auth-flow record projection wired for the builder's OWN durable
    /// product-auth service (filesystem-backed local-dev / production-shaped
    /// path). `None` when a caller supplied its own product-auth bundle — that
    /// path intentionally leaves the WebUI auth interaction surface unavailable
    /// (see `runtime/tests/auth_interaction.rs`
    /// `..._are_unavailable_without_flow_record_source`). Restores wiring dropped
    /// in commit 975bcd2ce ("Unify reborn runtime assembly"), which collapsed the
    /// old two-branch builder and lost the local-dev `.with_flow_record_source`.
    flow_record_source: Option<Arc<dyn ironclaw_auth::AuthFlowRecordSource>>,
}

fn compose_product_auth_services(
    input: ProductAuthServicesCompositionInput,
) -> Result<Arc<RebornProductAuthServices>, RebornBuildError> {
    let ProductAuthServicesCompositionInput {
        ports,
        turn_coordinator,
        blocked_auth_snapshot_source,
        provider_composition,
        security_audit_sink,
        secret_store,
        nearai_mcp_host_managed_scope,
        credential_account_visibility_policy,
        flow_record_source,
    } = input;
    let builder_owned_durable_auth = flow_record_source.is_some();
    let ports = match provider_composition.client {
        Some(provider_client) => ports.with_provider_client(provider_client),
        None if builder_owned_durable_auth => ports.with_current_provider_client(),
        None => ports,
    };
    let mut services = ports.into_services(
        auth_continuation_dispatcher(turn_coordinator, blocked_auth_snapshot_source),
        secret_store,
    );
    if let Some(sink) = security_audit_sink {
        services = services.with_security_audit_sink(sink);
    }
    if let Some(policy) = credential_account_visibility_policy {
        services = services.with_credential_account_visibility_policy(policy);
    }
    if let Some(engine) = provider_composition.engine {
        services = services.with_auth_engine(engine);
    }
    if let Some(driver) = provider_composition.gate_driver {
        services = services.with_oauth_gate_driver(driver);
    }
    if let Some(scope) = nearai_mcp_host_managed_scope {
        services = services.with_host_managed_nearai_credential_scope(scope)?;
    }
    if let Some(source) = flow_record_source {
        services = services.with_flow_record_source(source);
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
        .any(|config| config.vendor == ironclaw_auth::GOOGLE_PROVIDER_ID)
}

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

fn production_turn_state_store<F>(
    filesystem: Arc<ScopedFilesystem<F>>,
    limits: ironclaw_turns::TurnStateStoreLimits,
) -> FilesystemTurnStateRowStore<F>
where
    F: RootFilesystem + 'static,
{
    FilesystemTurnStateRowStore::new(filesystem).with_limits(limits)
}

async fn local_dev_trigger_repository(
    backend: &DurableBackend,
) -> Result<Arc<dyn TriggerRepository>, RebornBuildError> {
    match backend {
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
    let target_id =
        crate::outbound::OutboundDeliveryTargetId::new(target.as_str()).map_err(|error| {
            tracing::debug!(
                target = "ironclaw::reborn::trigger_create",
                %error,
                "per-trigger delivery target id failed outbound target id validation"
            );
            invalid("delivery target id is not a valid outbound target id".to_string())
        })?;
    let caller = crate::outbound::OutboundDeliveryTargetScope::new(
        scope.tenant_id.clone(),
        scope.user_id.clone(),
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

struct LocalRuntimeTriggerCreatorPairingHook {
    outbound_delivery_targets: Arc<crate::outbound::MutableOutboundDeliveryTargetRegistry>,
    scoped_filesystem: Arc<ScopedFilesystem<CompositeRootFilesystem>>,
    conversations: tokio::sync::OnceCell<RebornFilesystemConversationServices>,
}

#[async_trait::async_trait]
impl TriggerCreateHook for LocalRuntimeTriggerCreatorPairingHook {
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
        let filesystem = Arc::clone(&self.scoped_filesystem);
        let conversations = self
            .conversations
            .get_or_try_init(|| async move {
                RebornFilesystemConversationServices::new(filesystem).await
            })
            .await
            .map_err(|error| {
                trigger_pairing_error(TriggerPairingFailureSource::ConversationInit, error)
            })?;
        pair_trigger_creator(conversations, record).await
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
    ConversationInit,
    ActorPairing,
}

impl TriggerPairingFailureSource {
    fn as_str(&self) -> &'static str {
        match self {
            Self::TypedIdentity => "typed_identity",
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
    #[cfg(any(test, feature = "test-support"))]
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
        #[cfg(any(test, feature = "test-support"))]
        in_memory_budget_event_sink,
        broadcast_budget_event_sink,
    }
}

/// Single source for the resource-governor recipe every substrate build path
/// uses: a `FilesystemResourceGovernor` over the invocation-scoped view of the
/// composed root filesystem.
fn filesystem_resource_governor<F>(filesystem: &Arc<F>) -> FilesystemResourceGovernor<F>
where
    F: RootFilesystem + 'static,
{
    FilesystemResourceGovernor::new(crate::wrap_scoped(Arc::clone(filesystem)))
}

/// The `HostRuntimeServices` wiring shared by the local-dev and production
/// build paths (F4): the ten `.with_*` setters both paths always apply, plus
/// the fixed `TracingSecurityAuditSink`. Single-sourced as a macro because the
/// builder is generic over four backend type params and the setters are
/// value-generic (e.g. `with_trust_policy<T>`), so a function would have to
/// thread all of them; the macro defers typing to each expansion site.
/// Backend-specific setters (approval requests, resource governor, event
/// stores, the wake-notifier variant) are appended by the caller after this —
/// order is irrelevant because each setter writes an independent field.
macro_rules! with_shared_host_runtime_wiring {
    (
        $services:expr,
        trust_policy = $trust:expr,
        runtime_policy = $runtime_policy:expr,
        capability_leases = $leases:expr,
        persistent_approval_policies = $policies:expr,
        secret_store = $secret:expr,
        credential_broker = $broker:expr,
        filesystem_run_state = $fs:expr,
        turn_state_and_transition_port = $turn_state:expr,
        run_profile_resolver = $resolver:expr $(,)?
    ) => {
        $services
            .with_trust_policy($trust)
            .with_runtime_policy($runtime_policy)
            .with_capability_leases($leases)
            .with_persistent_approval_policies($policies)
            .with_security_audit_sink(::std::sync::Arc::new(
                ironclaw_events::TracingSecurityAuditSink,
            ))
            .with_secret_store($secret)
            .with_credential_broker($broker)
            .with_filesystem_run_state($fs)
            .with_turn_state_and_transition_port($turn_state)
            .with_run_profile_resolver($resolver)
    };
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
pub(crate) const LOCAL_DEV_DB_FILENAME: &str = "reborn-local-dev.db";

/// Full path to the local-dev libSQL database file within `root`. The single
/// public accessor for [`LOCAL_DEV_DB_FILENAME`]; callers outside this crate
/// (`ironclaw_reborn_cli`) must use this instead of hardcoding the filename.
pub fn local_dev_db_path(root: &Path) -> PathBuf {
    root.join(LOCAL_DEV_DB_FILENAME)
}

/// Open (or create) the local-dev libSQL database file at `root` — just the
/// connection, no migrations/mount. One owner for the `libsql::Builder::new_local`
/// sequence: [`build_default_local_dev_database_roots`] (production) and the
/// C-DURABLE test-support trigger-repository reopen
/// (`open_local_dev_trigger_repository_for_test`) both call this rather than
/// each opening their own connection to the same file.
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

/// Open a PostgreSQL pool from a build-time [`PostgresPoolSource`] (Phase B).
///
/// Production (`*_from_config_and_env`) carries `Config` and the pool is opened
/// here, at build time, from declarative connection config — construction no
/// longer performs database I/O. The `Prebuilt` arm is the caller-supplied
/// test escape hatch and is preferred verbatim when present.
fn open_postgres_pool_from_source(
    source: PostgresPoolSource,
) -> Result<deadpool_postgres::Pool, RebornBuildError> {
    match source {
        PostgresPoolSource::Prebuilt(pool) => Ok(pool),
        PostgresPoolSource::Config(connection) => Ok(
            ironclaw_reborn_event_store::open_postgres_pool_with_tls_options(
                connection.url,
                connection.pool_max_size,
                connection.tls_options,
            )?,
        ),
    }
}

/// Open a libSQL database from a build-time [`LibsqlConnectionConfig`]
/// (Phase B). Scheme detection mirrors
/// `ironclaw_reborn_event_store`'s libsql backend: recognised remote schemes
/// (`libsql://`, `https://`, `http://`, case-insensitive) route through
/// `Builder::new_remote` with the auth token; everything else is a local file.
async fn open_libsql_database_from_connection(
    connection: &LibsqlConnectionConfig,
) -> Result<Arc<libsql::Database>, RebornBuildError> {
    use secrecy::ExposeSecret;

    let path_or_url = connection.path_or_url.as_str();
    let build_result = if is_remote_libsql_target(path_or_url) {
        libsql::Builder::new_remote(
            path_or_url.to_string(),
            connection
                .auth_token
                .as_ref()
                .map(|token| token.expose_secret().to_string())
                .unwrap_or_default(),
        )
        .build()
        .await
    } else {
        libsql::Builder::new_local(path_or_url).build().await
    };
    build_result
        .map(Arc::new)
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("libSQL database could not be opened: {error}"),
        })
}

/// Detect a remote libSQL endpoint by recognised URL scheme, case-insensitively
/// (mirrors `ironclaw_reborn_event_store::libsql_backed::is_remote_libsql`).
fn is_remote_libsql_target(path_or_url: &str) -> bool {
    let Some(scheme_end) = path_or_url.find("://") else {
        return false;
    };
    let scheme = &path_or_url[..scheme_end];
    scheme.eq_ignore_ascii_case("libsql")
        || scheme.eq_ignore_ascii_case("https")
        || scheme.eq_ignore_ascii_case("http")
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
    {
        let db = open_local_dev_libsql_database(root).await?;
        let database = Arc::new(LibSqlRootFilesystem::new(Arc::clone(&db)));
        database.run_migrations().await?;
        mount_local_dev_database_roots(composite, database)?;
        Ok(DurableBackend::LibSql(db))
    }
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
    filesystem.mount_local(
        VirtualPath::new("/system/skills")?,
        HostPath::from_path_buf(root.join("system/skills")),
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
    root.mount(
        local_dev_mount_descriptor(
            "/system/extensions/.installations",
            "local-dev-extension-installation-state",
            BackendKind::DatabaseFilesystem,
            StorageClass::StructuredRecords,
            ContentKind::SystemState,
            IndexPolicy::BackendDefined,
            database.capabilities(),
        )?,
        Arc::clone(&database),
    )?;
    root.mount(
        local_dev_mount_descriptor(
            "/system/settings",
            "local-dev-system-settings",
            BackendKind::DatabaseFilesystem,
            StorageClass::StructuredRecords,
            ContentKind::SystemState,
            IndexPolicy::BackendDefined,
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

fn production_database_root_filesystem<F>(
    backend: Arc<F>,
    backend_id: &str,
) -> Result<Arc<CompositeRootFilesystem>, RebornBuildError>
where
    F: RootFilesystem + 'static,
{
    let mut root = CompositeRootFilesystem::new();
    for virtual_root in [
        "/tenants",
        "/events",
        "/memory",
        "/projects",
        "/system/extensions",
        "/system/settings",
        "/system/skills",
    ] {
        let mount_id = format!(
            "{backend_id}-{}",
            virtual_root
                .trim_start_matches('/')
                .replace(['/', '.'], "-")
        );
        root.mount(
            local_dev_mount_descriptor(
                virtual_root,
                &mount_id,
                BackendKind::DatabaseFilesystem,
                StorageClass::StructuredRecords,
                ContentKind::StructuredRecord,
                IndexPolicy::BackendDefined,
                backend.capabilities(),
            )?,
            Arc::clone(&backend),
        )?;
    }
    Ok(Arc::new(root))
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
        Arc::clone(&local),
    )?;
    root.mount(
        local_dev_mount_descriptor(
            "/system/skills",
            "local-dev-system-skills",
            BackendKind::DiskFilesystem,
            StorageClass::FileContent,
            ContentKind::GenericFile,
            IndexPolicy::NotIndexed,
            BackendCapabilities::bytes_only(),
        )?,
        local,
    )?;
    Ok(())
}

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

/// Unified bundle of outbound store handles returned by [`local_dev_outbound_store`].
///
/// All four trait roles must be satisfied on construction.  Every role is an
/// `Arc` clone of a single `FilesystemOutboundStateStore` — which implements all
/// four outbound-store traits — so the WebUI delivery-defaults facade and the
/// Slack delivery path share one backing tree.
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
    // facade and the Slack delivery path share the same backing tree.
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

fn production_builtin_extension_registry(
    process_backend: ProcessBackendKind,
) -> Result<ExtensionRegistry, RebornBuildError> {
    let mut registry = ExtensionRegistry::new();
    let package =
        builtin_first_party_package_for_process_backend(process_backend).map_err(|error| {
            RebornBuildError::InvalidConfig {
                reason: format!("built-in first-party package is invalid: {error}"),
            }
        })?;
    let package = extend_builtin_first_party_package(package).map_err(|error| {
        RebornBuildError::InvalidConfig {
            reason: format!("extension lifecycle package is invalid: {error}"),
        }
    })?;
    let package = extend_builtin_admin_configuration_package(package).map_err(|error| {
        RebornBuildError::InvalidConfig {
            reason: format!("administrator configuration package is invalid: {error}"),
        }
    })?;
    let package = extend_builtin_operator_config_package(package).map_err(|error| {
        RebornBuildError::InvalidConfig {
            reason: format!("operator configuration package is invalid: {error}"),
        }
    })?;
    let package = extend_builtin_outbound_preferences_package(package).map_err(|error| {
        RebornBuildError::InvalidConfig {
            reason: format!("outbound preferences package is invalid: {error}"),
        }
    })?;
    let package = extend_builtin_skill_auto_activate_package(package).map_err(|error| {
        RebornBuildError::InvalidConfig {
            reason: format!("skill auto-activation package is invalid: {error}"),
        }
    })?;
    registry
        .insert(package)
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("built-in first-party registry is invalid: {error}"),
        })?;
    Ok(registry)
}

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

/// Build the production first-party trust policy from the binary-injected
/// neutral bundle set (extension-runtime DEL-7). The provider entry comes from
/// `builtin_capability_policy` (no first-party dependency); each package's host
/// authority grant is sourced from its injected `trust_effects` instead of a
/// direct `ironclaw_first_party_extensions` call. Every entry is byte-identical
/// to the one the inventory-driven builder produced — same id, local-manifest
/// path, manifest digest, and effect list — so behavior is preserved exactly.
pub fn production_first_party_trust_policy(
    bundles: &[crate::extension_host::first_party::FirstPartyPackageBundle],
) -> Result<HostTrustPolicy, RebornBuildError> {
    let policy = builtin_capability_policy().map_err(|error| RebornBuildError::InvalidConfig {
        reason: format!("local-dev capability policy is invalid: {error}"),
    })?;
    let mut entries = vec![AdminEntry::for_local_manifest(
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
    )];
    // Packages supply their own trust grant as data (`trust_effects`);
    // composition still owns the decision (`first_party`) and the policy
    // construction. Packages with `None` (WASM tools, channel-only) draw trust
    // from the extension registry instead and are skipped here.
    for bundle in bundles {
        let Some(effects) = bundle.trust_effects.clone() else {
            continue;
        };
        entries.push(AdminEntry::for_local_manifest(
            PackageId::new(bundle.id.as_str()).map_err(|error| {
                RebornBuildError::InvalidConfig {
                    reason: format!("first-party package id '{}' is invalid: {error}", bundle.id),
                }
            })?,
            format!("/system/extensions/{}/manifest.toml", bundle.id),
            Some(sha256_digest_token(bundle.manifest_toml.as_bytes())),
            HostTrustAssignment::first_party(),
            effects,
            None,
        ));
    }
    HostTrustPolicy::new(vec![Box::new(AdminConfig::with_entries(entries))]).map_err(|error| {
        RebornBuildError::InvalidConfig {
            reason: format!("built-in first-party trust policy is invalid: {error}"),
        }
    })
}

/// Inventory-driven trust policy for composition's own unit tests (mirrors the
/// production builder, sourcing the neutral bundle set from the concrete
/// inventory). Gated `#[cfg(test)]` because it names
/// `ironclaw_first_party_extensions`, a dev-dependency; integration tests build
/// their trust policy from `production_first_party_trust_policy` plus bundles
/// they convert themselves (see `tests/support/first_party.rs`).
#[cfg(test)]
pub(crate) fn builtin_first_party_trust_policy() -> Result<HostTrustPolicy, RebornBuildError> {
    production_first_party_trust_policy(
        &crate::extension_host::first_party::first_party_bundles_from_inventory(),
    )
}

#[cfg(test)]
fn nearai_allowed_effects() -> Vec<ironclaw_host_api::EffectKind> {
    vec![
        ironclaw_host_api::EffectKind::DispatchCapability,
        ironclaw_host_api::EffectKind::Network,
        ironclaw_host_api::EffectKind::UseSecret,
    ]
}

async fn build_production_shaped(
    input: RebornHostBindings,
) -> Result<RebornRuntimeStores, RebornBuildError> {
    let RebornHostBindings {
        deployment,
        storage,
        production_trust_policy,
        // The notifier field on `RebornHostBindings` is kept for backward
        // compatibility with test callers that pre-mint one, but the
        // production-shaped build now mints its own notifier internally so the
        // coordinator and scheduler always share the exact same channel.
        turn_run_wake_notifier: _,
        runtime_process_binding,
        product_auth_ports,
        native_extension_factories,
        channel_extension_bindings,
        first_party_registrars,
        credential_account_visibility_policy,
        #[cfg(any(test, feature = "test-support"))]
        network_http_egress_for_test,
        #[cfg(any(test, feature = "test-support"))]
        trust_fixture_extensions_for_test,
        ..
    } = input;
    // The declarative DATA now lives on the deployment (Phase A). Clone the
    // fields this build path consumes by value; `deployment` stays in scope for
    // its substrate/traffic/readiness axes below.
    let owner_id = deployment.owner_id.clone();
    let local_runtime_identity = deployment.local_runtime_identity.clone();
    let runtime_policy = deployment.runtime_policy.clone();
    let account_setup_descriptors = deployment.account_setup_descriptors.clone();
    let oauth_provider_configs = deployment.oauth_provider_configs.clone();
    let oauth_dcr_callback = deployment.oauth_dcr_callback.clone();
    let nearai_mcp_bootstrap_config = deployment.nearai_mcp_bootstrap_config.clone();
    let turn_state_store_limits = deployment.turn_state_store_limits;
    let first_party_bundles = deployment.first_party_bundles.clone();
    let traffic_policy = deployment.traffic();
    // Label for logging/errors; behaviour reads `deployment`'s axes.
    let profile = deployment.profile();
    let wiring_config = production_config(
        deployment.required_runtime_backends.clone(),
        deployment.require_runtime_http_egress,
        deployment.require_wasm_credentials,
    );
    // The built-in first-party trust policy is composed here, at BUILD time,
    // from the binary-injected neutral bundle set (extension-runtime DEL-7) when
    // the caller did not pre-supply one — construction time (input.rs) predates
    // bundle injection. Same grants as the inventory-driven builder, sourced
    // from injected data instead of a direct `ironclaw_first_party_extensions`
    // call.
    let production_trust_policy = match production_trust_policy {
        Some(policy) => Some(policy),
        None => Some(Arc::new(production_first_party_trust_policy(
            &first_party_bundles,
        )?)),
    };
    match storage {
        RebornStorageInput::Disabled => Err(RebornBuildError::InvalidConfig {
            reason: format!(
                "profile={} requires durable database-backed Reborn storage",
                profile
            ),
        }),
        RebornStorageInput::LocalDev {
            root,
            workspace_root,
            host_home_root,
        } => {
            let scheduler_wake_wiring = ironclaw_runner::runtime::SchedulerWakeWiring::channel();
            let runtime_policy_for_local_process = runtime_policy.clone();
            let production_wiring = production_wiring(
                traffic_policy,
                production_trust_policy,
                runtime_policy,
                scheduler_wake_wiring.notifier(),
                runtime_process_binding,
            )?;
            let context = RebornProductionBuildContext {
                profile,
                wiring_config,
                production_wiring,
                local_process_port: None,
                product_auth_ports,
                oauth_provider_configs,
                oauth_dcr_callback,
                owner_id,
                local_runtime_identity,
                turn_state_store_limits,
                scheduler_wake_wiring,
                account_setup_descriptors,
                nearai_mcp_bootstrap_config,
                native_extension_factories,
                channel_extension_bindings,
                first_party_bundles,
                first_party_registrars,
                credential_account_visibility_policy,
                workspace_filesystems: None,
                local_dev_storage_root: None,
                default_system_prompt_path: None,
                #[cfg(any(test, feature = "test-support"))]
                network_http_egress_for_test: network_http_egress_for_test.clone(),
                #[cfg(any(test, feature = "test-support"))]
                trust_fixture_extensions_for_test,
            };
            build_local_storage_production_shaped(
                context,
                LocalStorageProductionInput {
                    root,
                    workspace_root,
                    host_home_root,
                    storage_backend_input: StorageBackendInput::LocalDefault,
                    explicit_secret_master_key: None,
                    runtime_policy_for_local_process,
                    postgres_resource_governor_singleton: None,
                },
            )
            .await
        }
        RebornStorageInput::HostedSingleTenantPostgres {
            root,
            workspace_root,
            host_home_root,
            pool_source,
            secret_master_key,
            process_local_resource_governor_singleton,
        } => {
            // Phase B: open (or accept the test-supplied) pool at build time.
            let pool = open_postgres_pool_from_source(pool_source)?;
            let scheduler_wake_wiring = ironclaw_runner::runtime::SchedulerWakeWiring::channel();
            let runtime_policy_for_local_process = runtime_policy.clone();
            let production_wiring = production_wiring(
                traffic_policy,
                production_trust_policy,
                runtime_policy,
                scheduler_wake_wiring.notifier(),
                runtime_process_binding,
            )?;
            let context = RebornProductionBuildContext {
                profile,
                wiring_config,
                production_wiring,
                local_process_port: None,
                product_auth_ports,
                oauth_provider_configs,
                oauth_dcr_callback,
                owner_id,
                local_runtime_identity,
                turn_state_store_limits,
                scheduler_wake_wiring,
                account_setup_descriptors,
                nearai_mcp_bootstrap_config,
                native_extension_factories,
                channel_extension_bindings,
                first_party_bundles,
                first_party_registrars,
                credential_account_visibility_policy,
                workspace_filesystems: None,
                local_dev_storage_root: None,
                default_system_prompt_path: None,
                #[cfg(any(test, feature = "test-support"))]
                network_http_egress_for_test: network_http_egress_for_test.clone(),
                #[cfg(any(test, feature = "test-support"))]
                trust_fixture_extensions_for_test,
            };
            build_local_storage_production_shaped(
                context,
                LocalStorageProductionInput {
                    root,
                    workspace_root,
                    host_home_root,
                    storage_backend_input: StorageBackendInput::Postgres(pool),
                    explicit_secret_master_key: Some(secret_master_key),
                    runtime_policy_for_local_process,
                    postgres_resource_governor_singleton: Some(
                        process_local_resource_governor_singleton,
                    ),
                },
            )
            .await
        }
        RebornStorageInput::Libsql {
            connection,
            prebuilt_db,
            secret_master_key,
            process_local_resource_governor_singleton,
        } => {
            // Mint the scheduler wake wiring here, before building the coordinator, so:
            // 1. The notifier can satisfy `HostRuntimeServices.with_turn_run_wake_notifier_dyn`
            //    (required by `validate_production_wiring` / `turn_coordinator_for_production`).
            // 2. The wiring is threaded through `RebornRuntimeStores` →
            //    `DefaultPlannedRuntimeParts.scheduler_wake_wiring` so the
            //    `build_default_planned_runtime` scheduler loop consumes the exact same channel,
            //    ensuring the coordinator's notifier and the scheduler share a live queue.
            let scheduler_wake_wiring = ironclaw_runner::runtime::SchedulerWakeWiring::channel();
            let production_wiring = production_wiring(
                traffic_policy,
                production_trust_policy,
                runtime_policy,
                scheduler_wake_wiring.notifier(),
                runtime_process_binding,
            )?;
            let secret_master_key = resolve_secret_master_key(secret_master_key).await?;
            // Phase B: prefer the test-supplied handle; otherwise open the
            // database from the declarative connection config at build time.
            let db = match prebuilt_db {
                Some(db) => db,
                None => open_libsql_database_from_connection(&connection).await?,
            };
            let context = RebornProductionBuildContext {
                profile,
                wiring_config,
                production_wiring,
                local_process_port: None,
                product_auth_ports,
                oauth_provider_configs,
                oauth_dcr_callback,
                owner_id,
                local_runtime_identity,
                turn_state_store_limits,
                scheduler_wake_wiring,
                account_setup_descriptors,
                nearai_mcp_bootstrap_config,
                native_extension_factories,
                channel_extension_bindings,
                first_party_bundles,
                first_party_registrars,
                credential_account_visibility_policy,
                workspace_filesystems: None,
                local_dev_storage_root: None,
                default_system_prompt_path: None,
                #[cfg(any(test, feature = "test-support"))]
                network_http_egress_for_test: network_http_egress_for_test.clone(),
                #[cfg(any(test, feature = "test-support"))]
                trust_fixture_extensions_for_test,
            };
            build_libsql_production(
                context,
                db,
                connection.path_or_url,
                connection.auth_token,
                secret_master_key,
                process_local_resource_governor_singleton,
            )
            .await
        }
        RebornStorageInput::Postgres {
            pool_source,
            secret_master_key,
            process_local_resource_governor_singleton,
        } => {
            // Phase B: open (or accept the test-supplied) pool at build time.
            let pool = open_postgres_pool_from_source(pool_source)?;
            // Mint the scheduler wake wiring here, before building the coordinator, so:
            // 1. The notifier can satisfy `HostRuntimeServices.with_turn_run_wake_notifier_dyn`
            //    (required by `validate_production_wiring` / `turn_coordinator_for_production`).
            // 2. The wiring is threaded through `RebornRuntimeStores` →
            //    `DefaultPlannedRuntimeParts.scheduler_wake_wiring` so the
            //    `build_default_planned_runtime` scheduler loop consumes the exact same channel,
            //    ensuring the coordinator's notifier and the scheduler share a live queue.
            let scheduler_wake_wiring = ironclaw_runner::runtime::SchedulerWakeWiring::channel();
            let production_wiring = production_wiring(
                traffic_policy,
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
                local_process_port: None,
                product_auth_ports,
                oauth_provider_configs,
                oauth_dcr_callback,
                owner_id,
                local_runtime_identity,
                turn_state_store_limits,
                scheduler_wake_wiring,
                account_setup_descriptors,
                nearai_mcp_bootstrap_config,
                native_extension_factories,
                channel_extension_bindings,
                first_party_bundles,
                first_party_registrars,
                credential_account_visibility_policy,
                workspace_filesystems: None,
                local_dev_storage_root: None,
                default_system_prompt_path: None,
                #[cfg(any(test, feature = "test-support"))]
                network_http_egress_for_test: network_http_egress_for_test.clone(),
                #[cfg(any(test, feature = "test-support"))]
                trust_fixture_extensions_for_test,
            };
            build_postgres_production(
                context,
                pool,
                secret_master_key,
                process_local_resource_governor_singleton,
            )
            .await
        }
    }
}

async fn resolve_secret_master_key(
    explicit: Option<ironclaw_secrets::SecretMaterial>,
) -> Result<ironclaw_secrets::SecretMaterial, RebornBuildError> {
    resolve_explicit_or_keychain_master_key(explicit)
        .await?
        .ok_or(RebornBuildError::MissingSecretMasterKey)
}

/// Local-storage bring-up inputs for [`build_local_storage_production_shaped`],
/// bundled so the builder keeps a two-argument shape (`context` + these) rather
/// than a positional-argument sprawl.
struct LocalStorageProductionInput {
    root: PathBuf,
    workspace_root: Option<PathBuf>,
    host_home_root: Option<PathBuf>,
    storage_backend_input: StorageBackendInput,
    explicit_secret_master_key: Option<ironclaw_secrets::SecretMaterial>,
    runtime_policy_for_local_process: Option<EffectiveRuntimePolicy>,
    postgres_resource_governor_singleton: Option<bool>,
}

async fn build_local_storage_production_shaped(
    mut context: RebornProductionBuildContext,
    input: LocalStorageProductionInput,
) -> Result<RebornRuntimeStores, RebornBuildError> {
    let LocalStorageProductionInput {
        root,
        workspace_root,
        host_home_root,
        storage_backend_input,
        explicit_secret_master_key,
        runtime_policy_for_local_process,
        postgres_resource_governor_singleton,
    } = input;
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
    let include_host_home = runtime_policy_for_local_process
        .as_ref()
        .is_some_and(|policy| {
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
    let owner_user_id =
        UserId::new(context.owner_id.clone()).map_err(|error| RebornBuildError::InvalidConfig {
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

    context.local_process_port = local_dev_process_port_for_policy(
        &runtime_policy_for_local_process,
        &workspace_root,
        host_home_root.as_ref(),
    );
    let filesystem_bundle = build_local_runtime_root_filesystem(
        &root,
        &workspace_root,
        host_home_root.as_ref(),
        storage_backend_input,
    )
    .await?;
    let trigger_repository =
        local_dev_trigger_repository(&filesystem_bundle.durable_backend).await?;
    let refresh_lock_pool = match &filesystem_bundle.durable_backend {
        DurableBackend::LibSql(_) => None,
        DurableBackend::Postgres(pool) => Some(pool.clone()),
    };
    let event_store = match &filesystem_bundle.durable_backend {
        DurableBackend::LibSql(_) => ironclaw_reborn_event_store::RebornEventStoreConfig::Libsql {
            path_or_url: local_dev_db_path(&root).to_string_lossy().into_owned(),
            auth_token: None,
        },
        DurableBackend::Postgres(pool) => {
            ironclaw_reborn_event_store::RebornEventStoreConfig::PostgresPool { pool: pool.clone() }
        }
    };
    let filesystem = filesystem_bundle.filesystem;
    context.workspace_filesystems = Some(build_workspace_filesystems(
        Arc::clone(&filesystem),
        &workspace_root,
        host_home_root.as_ref(),
    )?);
    context.local_dev_storage_root = Some(root.clone());
    context.default_system_prompt_path = Some(default_system_prompt_path);
    let scoped_filesystem = crate::wrap_scoped(Arc::clone(&filesystem));
    let (_secret_store, crypto) = build_secret_store(
        &root,
        Arc::clone(&scoped_filesystem),
        explicit_secret_master_key,
    )
    .await?;
    let secret_credentials = FilesystemSecretCredentialStores::new(scoped_filesystem, crypto);
    let resource_governor = filesystem_resource_governor(&filesystem);
    if let Some(singleton) = postgres_resource_governor_singleton {
        ensure_postgres_resource_governor_authority_for_build(singleton)?;
    }
    let stores = ProductionStoreBundle::with_secret_credentials(
        filesystem,
        resource_governor,
        secret_credentials,
        event_store,
    )
    .await?;
    build_backend_production(
        context,
        stores,
        trigger_repository,
        crate::product_auth::credentials::product_auth_refresh_lock::CredentialRefreshLeaderLock::new(
            refresh_lock_pool,
        ),
    )
    .await
}

struct RebornProductionWiring {
    trust_policy: Arc<HostTrustPolicy>,
    runtime_policy: EffectiveRuntimePolicy,
    turn_run_wake_notifier: Arc<dyn ironclaw_turns::TurnRunWakeNotifier>,
    runtime_process_binding: RebornRuntimeProcessBinding,
}

struct RebornProductionBuildContext {
    profile: RebornCompositionProfile,
    wiring_config: ironclaw_host_runtime::ProductionWiringConfig,
    production_wiring: RebornProductionWiring,
    local_process_port: Option<HostProcessPort>,
    product_auth_ports: Option<RebornProductAuthServicePorts>,
    oauth_provider_configs: Vec<crate::input::OAuthProviderBackendConfig>,
    oauth_dcr_callback: Option<crate::input::OAuthDcrCallbackConfig>,
    owner_id: String,
    local_runtime_identity: Option<RebornLocalRuntimeIdentity>,
    turn_state_store_limits: ironclaw_turns::TurnStateStoreLimits,
    /// The pre-minted scheduler wake wiring to carry to `RebornRuntimeStores` so
    /// `build_reborn_runtime` can hand it to `build_default_planned_runtime` via
    /// `DefaultPlannedRuntimeParts.scheduler_wake_wiring`.
    scheduler_wake_wiring: ironclaw_runner::runtime::SchedulerWakeWiring,
    account_setup_descriptors: Vec<ironclaw_product_workflow::ExtensionAccountSetupDescriptor>,
    nearai_mcp_bootstrap_config: Option<crate::llm_admin::nearai_mcp::NearAiMcpBootstrapConfig>,
    native_extension_factories: Vec<Arc<dyn ironclaw_extension_host::NativeExtensionFactory>>,
    channel_extension_bindings: Vec<crate::input::ChannelExtensionBinding>,
    /// Binary-injected neutral first-party bundle set (extension-runtime DEL-7):
    /// feeds the available-extension catalog, vendor auth recipes, and the
    /// reserved host-bundled id set.
    first_party_bundles: Vec<crate::extension_host::first_party::FirstPartyPackageBundle>,
    /// Binary-injected first-party capability handler registrars (GSuite,
    /// web tooling).
    first_party_registrars:
        Vec<Arc<dyn crate::extension_host::first_party::FirstPartyHandlerRegistrar>>,
    /// Injected credential-account visibility policy (see the build-input field).
    credential_account_visibility_policy: Option<
        Arc<
            dyn crate::product_auth::credentials::runtime_credentials::RuntimeCredentialAccountVisibilityPolicy,
        >,
    >,
    workspace_filesystems: Option<WorkspaceFilesystems>,
    local_dev_storage_root: Option<PathBuf>,
    default_system_prompt_path: Option<PathBuf>,
    /// Test-support host HTTP egress override (see `TestNetworkHttpEgress`).
    /// Carried from `RebornHostBindings::network_http_egress_for_test` so the
    /// unified production-shaped build honors an injected fake transport.
    #[cfg(any(test, feature = "test-support"))]
    network_http_egress_for_test: Option<Arc<dyn ironclaw_network::NetworkHttpEgress>>,
    /// Test-support only: allow trusted fixture packages copied into
    /// `/system/extensions` to validate as host-bundled.
    #[cfg(any(test, feature = "test-support"))]
    trust_fixture_extensions_for_test: bool,
}

fn production_wiring(
    traffic_policy: TrafficPolicy,
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
    if traffic_policy.requires_production_runtime_policy_preflight() {
        validate_production_runtime_policy(&runtime_policy)?;
    }
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

fn validate_production_runtime_policy(
    runtime_policy: &EffectiveRuntimePolicy,
) -> Result<(), RebornBuildError> {
    let mut issues = Vec::new();
    if let Some(reason) = local_only_runtime_policy_reason(runtime_policy) {
        issues.push(ironclaw_host_runtime::ProductionWiringIssue::new(
            ironclaw_host_runtime::ProductionWiringComponent::RuntimePolicy,
            ironclaw_host_runtime::ProductionWiringIssueKind::LocalOnlyImplementation,
            Some(reason),
        ));
    }
    if runtime_policy.process_backend == ProcessBackendKind::LocalHost {
        issues.push(ironclaw_host_runtime::ProductionWiringIssue::new(
            ironclaw_host_runtime::ProductionWiringComponent::RuntimeProcessPort,
            ironclaw_host_runtime::ProductionWiringIssueKind::LocalOnlyImplementation,
            Some("local_host_process"),
        ));
    }
    if issues.is_empty() {
        Ok(())
    } else {
        Err(RebornBuildError::ProductionWiring {
            report: ironclaw_host_runtime::ProductionWiringReport::new(issues),
        })
    }
}

fn local_only_runtime_policy_reason(policy: &EffectiveRuntimePolicy) -> Option<&'static str> {
    if matches!(policy.deployment, DeploymentMode::LocalSingleUser) {
        return Some("local_single_user_deployment");
    }
    if matches!(
        policy.filesystem_backend,
        FilesystemBackendKind::HostWorkspace | FilesystemBackendKind::HostWorkspaceAndHome
    ) {
        return Some("host_workspace_filesystem");
    }
    if matches!(policy.process_backend, ProcessBackendKind::LocalHost) {
        return Some("local_host_process");
    }
    if matches!(policy.network_mode, NetworkMode::Direct) {
        return Some("direct_network");
    }
    if matches!(
        policy.secret_mode,
        SecretMode::ScrubbedEnv | SecretMode::InheritedEnv
    ) {
        return Some("local_secret_environment");
    }
    None
}

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

fn planned_run_profile_resolver() -> Result<Arc<InMemoryRunProfileResolver>, RebornBuildError> {
    Ok(Arc::new(
        ironclaw_runner::planned_driver_factory::default_planned_run_profile_resolver().map_err(
            |error| RebornBuildError::PlannedRunProfileResolver {
                reason: error.to_string(),
            },
        )?,
    ))
}

type FilesystemProductionHostRuntimeServices<F> = HostRuntimeServices<
    F,
    FilesystemResourceGovernor<F>,
    ironclaw_processes::FilesystemProcessStore<F>,
    ironclaw_processes::FilesystemProcessResultStore<F>,
>;

fn substrate_only_default_owner_id() -> Result<UserId, crate::RebornCompositionError> {
    let identity = RebornRuntimeIdentity::reborn_cli();
    // The substrate-only builders do not receive app/runtime owner input.
    // Preserve their legacy location under the default `reborn-cli` owner.
    UserId::new(identity.tenant_id).map_err(crate::RebornCompositionError::Mount)
}

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
    let resource_governor = filesystem_resource_governor(&filesystem);
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

enum FilesystemProductionEventStoresInput {
    Config(ironclaw_reborn_event_store::RebornEventStoreConfig),
    Prebuilt(ironclaw_reborn_event_store::RebornEventStores),
}

fn ensure_postgres_event_store_config(
    config: &ironclaw_reborn_event_store::RebornEventStoreConfig,
) -> Result<(), crate::RebornCompositionError> {
    match config {
        ironclaw_reborn_event_store::RebornEventStoreConfig::Postgres { .. } => Ok(()),
        ironclaw_reborn_event_store::RebornEventStoreConfig::PostgresPool { .. } => Ok(()),
        _ => Err(crate::RebornCompositionError::InvalidConfig {
            reason: "PostgreSQL production substrate requires a PostgreSQL event store".to_string(),
        }),
    }
}

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

    let services = with_shared_host_runtime_wiring!(
        HostRuntimeServices::new(
            Arc::new(ExtensionRegistry::new()),
            filesystem,
            governor,
            Arc::new(GrantAuthorizer::new()),
            process_services,
            surface_version,
        ),
        trust_policy = trust_policy,
        runtime_policy = runtime_policy,
        capability_leases = capability_leases,
        persistent_approval_policies = persistent_approval_policies,
        secret_store = Arc::clone(&secret_credentials.secret_store),
        credential_broker = secret_credentials.credential_broker,
        filesystem_run_state = Arc::clone(&scoped_filesystem),
        turn_state_and_transition_port = turn_state,
        run_profile_resolver = Arc::new(
            ironclaw_runner::planned_driver_factory::default_planned_run_profile_resolver()?,
        ),
    )
    .with_turn_run_wake_notifier(turn_run_wake_notifier);
    let services = match event_store {
        FilesystemProductionEventStoresInput::Config(config) => {
            services
                .with_reborn_event_store_config(
                    ironclaw_reborn_event_store::RebornProfile::Production,
                    config,
                )
                .await?
        }
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
            default_host_http_egress().map_err(|error| {
                crate::RebornCompositionError::InvalidConfig {
                    reason: error.to_string(),
                }
            })?,
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

struct ProductionStoreBundle {
    filesystem: Arc<CompositeRootFilesystem>,
    scoped_filesystem: Arc<ScopedFilesystem<CompositeRootFilesystem>>,
    resource_governor: ComposedResourceGovernor,
    leases: Arc<ComposedCapabilityLeaseStore>,
    persistent_approval_policies: Arc<ComposedPersistentApprovalPolicyStore>,
    secret_credentials: FilesystemSecretCredentialStores<CompositeRootFilesystem>,
    event_store: ironclaw_reborn_event_store::RebornEventStoreConfig,
}

impl ProductionStoreBundle {
    async fn new(
        filesystem: Arc<CompositeRootFilesystem>,
        resource_governor: ComposedResourceGovernor,
        secret_master_key: ironclaw_secrets::SecretMaterial,
        event_store: ironclaw_reborn_event_store::RebornEventStoreConfig,
    ) -> Result<Self, RebornBuildError> {
        validate_reborn_runtime_storage(&filesystem).await?;
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

    async fn with_secret_credentials(
        filesystem: Arc<CompositeRootFilesystem>,
        resource_governor: ComposedResourceGovernor,
        secret_credentials: FilesystemSecretCredentialStores<CompositeRootFilesystem>,
        event_store: ironclaw_reborn_event_store::RebornEventStoreConfig,
    ) -> Result<Self, RebornBuildError> {
        validate_reborn_runtime_storage(&filesystem).await?;
        let scoped_filesystem = crate::wrap_scoped(Arc::clone(&filesystem));
        let leases = Arc::new(FilesystemCapabilityLeaseStore::new(Arc::clone(
            &scoped_filesystem,
        )));
        let persistent_approval_policies = Arc::new(FilesystemPersistentApprovalPolicyStore::new(
            Arc::clone(&scoped_filesystem),
        ));
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

pub(crate) fn production_skill_management_mount_view(
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

pub(crate) fn production_system_extensions_lifecycle_mount_view() -> Result<MountView, HostApiError>
{
    MountView::new(vec![MountGrant::new(
        MountAlias::new("/system/extensions")?,
        VirtualPath::new("/system/extensions")?,
        MountPermissions::read_write_list_delete(),
    )])
}

async fn build_backend_production(
    context: RebornProductionBuildContext,
    stores: ProductionStoreBundle,
    trigger_repository: Arc<dyn TriggerRepository>,
    // Leader lock for the background credential keepalive worker. The worker
    // uses this to elect one process per tick as the sweep leader. `None`
    // pool → always-leader (libsql / single-process). Stays private.
    leader_lock: crate::product_auth::credentials::product_auth_refresh_lock::CredentialRefreshLeaderLock,
) -> Result<RebornRuntimeStores, RebornBuildError> {
    let RebornProductionBuildContext {
        profile,
        wiring_config,
        production_wiring,
        local_process_port,
        product_auth_ports,
        oauth_provider_configs,
        oauth_dcr_callback,
        owner_id,
        local_runtime_identity,
        turn_state_store_limits,
        scheduler_wake_wiring,
        account_setup_descriptors,
        nearai_mcp_bootstrap_config,
        native_extension_factories,
        channel_extension_bindings,
        first_party_bundles,
        first_party_registrars,
        credential_account_visibility_policy,
        workspace_filesystems,
        local_dev_storage_root,
        default_system_prompt_path,
        #[cfg(any(test, feature = "test-support"))]
        network_http_egress_for_test,
        #[cfg(any(test, feature = "test-support"))]
        trust_fixture_extensions_for_test,
    } = context;
    // Select the non-validating local-testing host runtime for a local-dev
    // deployment. The pre-`975bcd2ce` dedicated local-dev builder always used
    // `host_runtime_for_local_testing()`; the unified path keyed only on a wired
    // local host process port (`local_process_port.is_some()`), which is `None`
    // whenever the local-dev deployment uses a non-`LocalHost` process backend
    // (e.g. an injected `TenantSandbox` port — the multi-user-safe default). That
    // wrongly routed such local-dev builds through `host_runtime_for_production`,
    // whose `validate_production_wiring` rejects the `LocalSingleUser` deployment
    // mode. Key the choice on the deployment mode too: a `LocalSingleUser` policy
    // is exactly the shape production validation would reject, so it must use the
    // local-testing runtime regardless of process backend. (Production
    // deployments never resolve to `LocalSingleUser` — see
    // `.claude/rules/safety-and-sandbox.md`.)
    let deployment_is_local_single_user = matches!(
        production_wiring.runtime_policy.deployment,
        DeploymentMode::LocalSingleUser
    );
    let uses_local_host_runtime = local_process_port.is_some() || deployment_is_local_single_user;
    // The reserved host-bundled id set consulted during filesystem catalog
    // load and by the upload-import path, sourced from the injected bundles.
    let first_party_reserved_ids = first_party_reserved_extension_ids(&first_party_bundles);
    // Computed before `oauth_provider_configs` is consumed by
    // `compose_provider_client` below — see `google_oauth_configured`.
    let google_oauth_configured = google_oauth_configured(&oauth_provider_configs);
    let google_provider = VendorId::new(ironclaw_auth::GOOGLE_PROVIDER_ID).map_err(|error| {
        RebornBuildError::InvalidConfig {
            reason: format!("provider instance readiness map could not be built: {error}"),
        }
    })?;
    let provider_instance_readiness =
        provider_instance_readiness_map([ProviderInstanceReadinessInput {
            provider: google_provider,
            configured: google_oauth_configured,
            remediation: ironclaw_reborn_config::google_setup_steps_text(),
        }]);
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
    let extension_lifecycle_surface_context = local_dev_extension_lifecycle_surface_context(
        owner_user_id.clone(),
        local_runtime_identity.as_ref(),
    )?;
    let channel_egress_scope = turn_state_scope.clone();
    let (skill_filesystem, workspace_filesystem, runtime_workspace_mounts) =
        match workspace_filesystems {
            Some(filesystems) => filesystems,
            None => {
                let read_only_workspace_mounts =
                    workspace_mount_view(MountPermissions::read_only(), &[]).map_err(|error| {
                        RebornBuildError::InvalidConfig {
                            reason: error.to_string(),
                        }
                    })?;
                let runtime_workspace_mounts =
                    ambient_workspace_mount_view(MountPermissions::read_write(), &[], &[])
                        .map_err(|error| RebornBuildError::InvalidConfig {
                            reason: error.to_string(),
                        })?;
                (
                    Arc::new(ScopedFilesystem::new(
                        Arc::clone(&stores.filesystem),
                        scoped_skill_context_mount_view,
                    )),
                    Arc::new(ScopedFilesystem::with_fixed_view(
                        Arc::clone(&stores.filesystem),
                        read_only_workspace_mounts,
                    )),
                    runtime_workspace_mounts,
                )
            }
        };
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
    let system_extensions_lifecycle_mounts = production_system_extensions_lifecycle_mount_view()
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: error.to_string(),
        })?;
    let approval_requests = Arc::new(FilesystemApprovalRequestStore::new(Arc::clone(
        &stores.scoped_filesystem,
    )));
    let capability_policy =
        Arc::new(
            builtin_capability_policy().map_err(|error| RebornBuildError::InvalidConfig {
                reason: format!("capability policy is invalid: {error}"),
            })?,
        );
    let tool_permission_overrides = Arc::new(ComposedToolPermissionOverrideStore::new(Arc::clone(
        &stores.scoped_filesystem,
    )));
    let auto_approve_settings = Arc::new(ComposedAutoApproveSettingStore::new(Arc::clone(
        &stores.scoped_filesystem,
    )));
    let persistent_approval_policies_for_settings: Arc<
        dyn ironclaw_approvals::PersistentApprovalPolicyStore,
    > = Arc::clone(&stores.persistent_approval_policies)
        as Arc<dyn ironclaw_approvals::PersistentApprovalPolicyStore>;
    let approval_settings_provider = Arc::new(StoreApprovalSettingsProvider::new(
        Arc::clone(&tool_permission_overrides)
            as Arc<dyn ironclaw_approvals::ToolPermissionOverrideStore>,
        Arc::clone(&auto_approve_settings) as Arc<dyn ironclaw_approvals::AutoApproveSettingStore>,
        persistent_approval_policies_for_settings,
    ));
    let runtime_policy = production_wiring.runtime_policy.clone();
    let runtime_policy_for_return = Some(runtime_policy.clone());
    let authorizer = local_dev_authorizer(
        Some(&runtime_policy),
        Arc::clone(&capability_policy),
        approval_settings_provider,
    );
    let outbound_stores = local_dev_outbound_store(Arc::clone(&stores.filesystem));
    let outbound_delivery_targets =
        Arc::new(crate::outbound::MutableOutboundDeliveryTargetRegistry::default());
    let skill_auto_activate_learned = Arc::new(AtomicBool::new(true));
    let trigger_create_hook = Arc::new(LocalRuntimeTriggerCreatorPairingHook {
        outbound_delivery_targets: Arc::clone(&outbound_delivery_targets),
        scoped_filesystem: Arc::clone(&stores.scoped_filesystem),
        conversations: tokio::sync::OnceCell::new(),
    });
    let process_backend = production_wiring.runtime_policy.process_backend;
    let extension_registry = production_builtin_extension_registry(process_backend)?;
    let extension_registry = Arc::new(extension_registry);
    let BudgetSinks {
        budget_event_sink,
        #[cfg(any(test, feature = "test-support"))]
        in_memory_budget_event_sink,
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
    // exactly as the local substrate builds them — see the local runtime stores'
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
    // Same store-backed lookup the WebUI automations panel builds from the
    // runtime's turn-state snapshot source (#5886).
    let trigger_active_run_lookup: Arc<dyn TriggerActiveRunLookup> = Arc::new(
        crate::automation::trigger_poller::SnapshotActiveRunLookup::new(
            Arc::clone(&turn_state) as Arc<dyn crate::turn_run_snapshot::TurnRunSnapshotSource>
        ),
    );
    let mut first_party_registry = production_first_party_registry_with_trigger_create_hook(
        Arc::clone(&trigger_repository),
        trigger_create_hook,
        trigger_active_run_lookup,
        process_backend,
    )?;
    let product_auth_filesystem = Arc::clone(&stores.scoped_filesystem);
    let services = with_shared_host_runtime_wiring!(
        HostRuntimeServices::new(
            Arc::clone(&extension_registry),
            Arc::clone(&stores.filesystem),
            Arc::new(InMemoryResourceGovernor::new()),
            authorizer,
            ProcessServices::filesystem(Arc::clone(&stores.scoped_filesystem)),
            CapabilitySurfaceVersion::new("reborn-app-v1")?,
        ),
        trust_policy = Arc::clone(&production_wiring.trust_policy),
        runtime_policy = runtime_policy,
        capability_leases = Arc::clone(&stores.leases),
        persistent_approval_policies = Arc::clone(&stores.persistent_approval_policies),
        secret_store = Arc::clone(&stores.secret_credentials.secret_store),
        credential_broker = stores.secret_credentials.credential_broker,
        filesystem_run_state = Arc::clone(&stores.scoped_filesystem),
        turn_state_and_transition_port = Arc::clone(&turn_state),
        run_profile_resolver = planned_run_profile_resolver()?,
    )
    .with_approval_requests(Arc::clone(&approval_requests))
    .with_resource_governor(Arc::clone(&resource_governor))
    .with_production_reborn_event_stores(event_stores)
    .with_turn_run_wake_notifier_dyn(production_wiring.turn_run_wake_notifier);
    // Honor an injected test egress (hosted-MCP discovery / DM provisioning over
    // a fake transport) when present; otherwise the real policy egress. Restores
    // the consumer dropped in commit 975bcd2ce — without it every local-dev test
    // reaches the real network. `TestNetworkHttpEgress` adapts the injected
    // `Arc<dyn NetworkHttpEgress>` to the generic method bound.
    #[cfg(any(test, feature = "test-support"))]
    let services = match network_http_egress_for_test {
        Some(test_egress) => {
            services.try_with_host_http_egress(TestNetworkHttpEgress(test_egress))?
        }
        None => services.try_with_host_http_egress(default_host_http_egress()?)?,
    };
    #[cfg(not(any(test, feature = "test-support")))]
    let services = services.try_with_host_http_egress(default_host_http_egress()?)?;
    let product_auth_runtime_ports = require_product_auth_runtime_ports(&services)?;
    let services = attach_hosted_mcp_runtime(services)?;
    let channel_config_credential_slot =
        crate::product_auth::credentials::product_auth_providers::ChannelConfigCredentialSlot::default();
    let provider_composition = compose_provider_client(
        oauth_provider_configs,
        oauth_dcr_callback,
        Arc::clone(&secret_store),
        product_auth_runtime_ports.clone(),
        channel_config_credential_slot.clone(),
        &first_party_bundles,
    )?;
    let services = if let Some(process_port) = local_process_port {
        services.with_runtime_process_port(Arc::new(process_port))
    } else {
        services
    };
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
    // B1: track the durable FilesystemAuthProductServices so the engine
    // keepalive sweep can enumerate candidates across all owners. When a
    // caller pre-supplies product_auth_ports, we do not create a durable
    // instance here, so the candidate source is None (sweep finds no
    // candidates, which is safe for override/test callers).
    let credential_refresh_candidate_source: Option<
        Arc<dyn ironclaw_auth::KeepaliveCandidateSource>,
    >;
    // The durable auth-flow record projection this builder wires for its own
    // durable service (`None` arm). Left `None` for a caller-supplied bundle so
    // that path's WebUI auth interaction surface stays explicitly unavailable
    // (restores wiring dropped in commit 975bcd2ce).
    let product_auth_flow_record_source: Option<Arc<dyn ironclaw_auth::AuthFlowRecordSource>>;
    let product_auth_ports = match product_auth_ports {
        Some(ports) => {
            credential_refresh_candidate_source = None;
            product_auth_flow_record_source = None;
            ports
        }
        None => {
            let durable = Arc::new(FilesystemAuthProductServices::new_with_root(
                product_auth_filesystem,
                Arc::clone(&stores.filesystem),
                Arc::clone(&secret_store),
            ));
            credential_refresh_candidate_source =
                Some(Arc::clone(&durable) as Arc<dyn ironclaw_auth::KeepaliveCandidateSource>);
            product_auth_flow_record_source =
                Some(Arc::clone(&durable) as Arc<dyn ironclaw_auth::AuthFlowRecordSource>);
            RebornProductAuthServicePorts::from_shared_with_provider(
                durable,
                provider_composition
                    .client
                    .clone()
                    .unwrap_or_else(|| Arc::new(UnavailableAuthProviderClient)),
            )
        }
    };
    // The sweep resolves per-vendor idle lifetimes through the same recipe
    // data the auth engine executes; capture it before `provider_composition`
    // moves into `compose_product_auth_services`.
    let keepalive_recipes = provider_composition
        .engine
        .as_ref()
        .map(|engine| Arc::clone(engine.recipes()));
    let product_auth_services =
        compose_product_auth_services(ProductAuthServicesCompositionInput {
            ports: product_auth_ports,
            turn_coordinator: turn_coordinator.clone(),
            // Blocked-auth fan-out over this builder's own durable turn-state
            // store: a completed connect resumes every run the same owner has
            // parked on the same provider, matching the local-dev builder. The
            // blanket `TurnRunSnapshotSource` impl covers the generic
            // filesystem store directly.
            blocked_auth_snapshot_source: Some(Arc::clone(&turn_state)
                as Arc<dyn crate::blocked_auth_resume::BlockedAuthSnapshotSource>),
            // This production builder wires no lifecycle-activation facade, so an
            // empty slot leaves lifecycle-activation auth continuations unsupported
            // here, preserving this builder's prior behavior.
            provider_composition,
            security_audit_sink,
            secret_store: Arc::clone(&secret_store),
            nearai_mcp_host_managed_scope: Some(AuthProductScope::new(
                channel_egress_scope.clone(),
                AuthSurface::Api,
            )),
            credential_account_visibility_policy,
            flow_record_source: product_auth_flow_record_source,
        })?;
    // Bundle the keepalive sweep deps so they are wired all-or-nothing. The
    // candidate source is present only when this path built a durable instance
    // (no caller-supplied product_auth_ports); recipes are present only when
    // the auth engine was composed; the leader lock and refresh port are
    // always available here.
    let credential_refresh_worker = match (credential_refresh_candidate_source, keepalive_recipes) {
        (Some(candidate_source), Some(recipes)) => CredentialRefreshWorkerReady::Ready {
            candidate_source,
            recipes,
            leader_lock,
            refresh_port: Arc::clone(&product_auth_services),
        },
        _ => CredentialRefreshWorkerReady::Absent,
    };
    let product_auth_ready = true;
    // Wire ProductAuthAccount runtime credential resolver before
    // host_runtime_for_production so WASM extensions whose manifest declares a
    // ProductAuthAccount runtime credential source resolve through
    // CredentialAccountService. Unconditional in production: product_auth_services
    // always exists (durable filesystem fallback from #4234).
    let mut services = services.with_runtime_credential_account_resolver(Arc::new(
        ProductAuthRuntimeCredentialResolver::new_with_refresh(
            product_auth_services.runtime_credential_account_selection_service(),
            product_auth_services.runtime_credential_account_refresh_service(),
        ),
    ));
    services = attach_wasm_runtime(services)?;
    // Install every binary-assembled first-party capability handler (GSuite,
    // web tooling) through the generic registrar seam (extension-runtime DEL-7).
    // Composition owns the loop and the shared context; the concrete executors
    // live in the assembling binary.
    let first_party_registrar_context = FirstPartyRegistrarContext {
        credential_account_service: product_auth_services.credential_account_service(),
        credential_account_record_source: product_auth_services.credential_account_record_source(),
        product_auth_runtime_ports: product_auth_runtime_ports.clone(),
        google_oauth_configured,
    };
    for registrar in &first_party_registrars {
        registrar
            .register(&mut first_party_registry, &first_party_registrar_context)
            .map_err(|error| RebornBuildError::InvalidConfig {
                reason: format!("first-party capability handlers are invalid: {error}"),
            })?;
    }
    let extensions_root = VirtualPath::new("/system/extensions")?;
    #[cfg(any(test, feature = "test-support"))]
    let filesystem_catalog = if trust_fixture_extensions_for_test {
        AvailableExtensionCatalog::from_trusted_fixture_filesystem_root(
            stores.filesystem.as_ref(),
            &extensions_root,
            &first_party_reserved_ids,
        )
        .await
    } else {
        AvailableExtensionCatalog::from_filesystem_root(
            stores.filesystem.as_ref(),
            &extensions_root,
            &first_party_reserved_ids,
        )
        .await
    };
    #[cfg(not(any(test, feature = "test-support")))]
    let filesystem_catalog = AvailableExtensionCatalog::from_filesystem_root(
        stores.filesystem.as_ref(),
        &extensions_root,
        &first_party_reserved_ids,
    )
    .await;
    let mut available_extensions =
        filesystem_catalog.map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("available extension catalog could not be loaded: {error}"),
        })?;
    available_extensions.extend(
        AvailableExtensionCatalog::from_first_party_assets_with_nearai_mcp_config(
            nearai_mcp_bootstrap_config.as_ref(),
            &first_party_bundles,
        )
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("first-party extension catalog could not be loaded: {error}"),
        })?,
    );
    // Carry the reserved first-party id set onto the composed catalog so the
    // upload-import path can reject reserved ids without re-deriving the
    // inventory.
    available_extensions =
        available_extensions.with_reserved_bundled_ids(first_party_reserved_ids.clone());
    let admin_configuration_uses = available_extensions.admin_configuration_uses();
    let available_manifests = available_extensions.resolved_manifests();
    let deployment_bindings = available_manifests
        .iter()
        .filter(|manifest| {
            manifest
                .channel
                .as_ref()
                .is_some_and(|channel| channel.inbound && channel.ingress.is_some())
        })
        .filter_map(|manifest| {
            channel_extension_bindings
                .iter()
                .find(|binding| binding.extension_id == manifest.id.as_str())
                .map(|binding| {
                    ironclaw_extension_host::DeploymentChannelBinding::new(
                        Arc::clone(manifest),
                        Arc::clone(&binding.adapter),
                    )
                })
        })
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("deployment channel registry could not be built: {error}"),
        })?;
    let deployment_channels = Arc::new(
        ironclaw_extension_host::DeploymentChannelRegistry::try_new(deployment_bindings).map_err(
            |error| RebornBuildError::InvalidConfig {
                reason: format!("deployment channel registry could not be built: {error}"),
            },
        )?,
    );
    let admin_configuration_filesystem: Arc<dyn RootFilesystem> = stores.filesystem.clone();
    let admin_configuration = Arc::new(
        AdminConfigurationService::new(
            FilesystemAdminConfigurationStore::new(Arc::new(ScopedFilesystem::new(
                admin_configuration_filesystem,
                crate::invocation_mount_view,
            ))),
            Arc::clone(&secret_store),
            admin_configuration_uses
                .iter()
                .map(|usage| usage.descriptor.clone()),
        )
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("admin configuration service could not be built: {error}"),
        })?,
    );
    let extension_filesystem: Arc<dyn RootFilesystem> = stores.filesystem.clone();
    let extension_host_ports =
        ironclaw_host_runtime::default_host_port_catalog().map_err(|error| {
            RebornBuildError::InvalidConfig {
                reason: format!("extension host port catalog could not be loaded: {error}"),
            }
        })?;
    let extension_host_api_contracts =
        product_extension_host_api_contract_registry().map_err(|error| {
            RebornBuildError::InvalidConfig {
                reason: format!("extension host API contracts could not be loaded: {error}"),
            }
        })?;
    let extension_installation_state_path =
        FilesystemExtensionInstallationStore::default_state_path().map_err(|error| {
            RebornBuildError::InvalidConfig {
                reason: format!("extension installation state path is invalid: {error}"),
            }
        })?;
    let extension_installation_store: Arc<dyn ExtensionInstallationStore> = Arc::new(
        FilesystemExtensionInstallationStore::load_at(
            extension_filesystem.clone(),
            extension_installation_state_path,
            extension_host_ports,
            extension_host_api_contracts,
        )
        .await
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("extension installation state could not be loaded: {error}"),
        })?,
    );
    let extension_lifecycle_service = Arc::new(tokio::sync::Mutex::new(
        ExtensionLifecycleService::new(services.shared_extension_registry().snapshot_owned()),
    ));
    let active_extensions = ActiveExtensionPublisher::new(
        services.shared_extension_registry(),
        Arc::clone(&production_wiring.trust_policy),
        Arc::new(ironclaw_trust::InvalidationBus::new()),
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
    let removal_cleanup_adapters: Vec<Arc<dyn ExtensionRemovalCleanupAdapter>> = Vec::new();
    let removal_cleanup = Arc::new(
        ExtensionRemovalCleanupRegistry::try_from_adapters(removal_cleanup_adapters).map_err(
            |error| RebornBuildError::InvalidConfig {
                reason: format!("extension removal cleanup registry could not be built: {error}"),
            },
        )?,
    );
    let account_setups = ExtensionAccountSetupRegistry::default();
    let channel_disconnect_slot: Arc<
        std::sync::OnceLock<Arc<dyn ironclaw_product_workflow::ChannelConnectionFacade>>,
    > = Arc::new(std::sync::OnceLock::new());
    let extension_management = Arc::new(
        RebornLocalExtensionManagementPort::new(
            extension_filesystem,
            available_extensions,
            extension_installation_store,
            extension_lifecycle_service,
            active_extensions,
            Some(Arc::clone(&product_auth_services) as Arc<dyn ExtensionCredentialCleanup>),
            channel_egress_scope.user_id.clone(),
        )
        .with_account_setup_registry(account_setups.clone())
        .with_removal_cleanup_registry(removal_cleanup)
        .with_provider_instance_readiness(provider_instance_readiness)
        .with_channel_disconnect_slot(Arc::clone(&channel_disconnect_slot)),
    );
    let nearai_mcp_bootstrap_outcome = crate::llm_admin::nearai_mcp::bootstrap_nearai_mcp(
        nearai_mcp_bootstrap_config,
        &product_auth_services,
        &extension_management,
        channel_egress_scope.clone(),
    )
    .await?;
    nearai_mcp_bootstrap_outcome.log_completion();
    let channel_config_service = Arc::new(
        crate::extension_host::channel_config::ChannelConfigService::new(
            extension_management.installation_store_handle(),
            Arc::clone(&secret_store),
            channel_egress_scope.clone(),
            Arc::clone(&extension_management)
                as Arc<dyn crate::extension_host::channel_config::ChannelConfigReactivation>,
        )
        .with_admin_configuration(
            Arc::clone(&admin_configuration),
            channel_egress_scope.clone(),
        )
        .with_available_manifests(available_manifests.clone()),
    );
    extension_management.attach_channel_config(&channel_config_service);
    channel_config_credential_slot.fill(Arc::clone(&channel_config_service));
    let fold_filesystem: Arc<dyn RootFilesystem> = stores.filesystem.clone();
    let channel_identity_store = Arc::new(
        crate::extension_host::channel_identity_store::FilesystemChannelIdentityStore::new(
            Arc::clone(&fold_filesystem),
            channel_egress_scope.tenant_id.clone(),
            channel_egress_scope.user_id.clone(),
        ),
    );
    let channel_dm_target_store = Arc::new(
        crate::extension_host::channel_dm_targets::FilesystemChannelDmTargetStore::new(
            Arc::clone(&fold_filesystem),
            channel_egress_scope.tenant_id.clone(),
            channel_egress_scope.user_id.clone(),
        ),
    );
    let runtime_http_egress = Some(product_auth_runtime_ports.runtime_http_egress());
    let host_runtime_http_egress = services.host_runtime_http_egress_port();
    // The first-party capability handlers were installed above through the
    // binary-supplied `first_party_registrars` loop (extension-runtime DEL-7);
    // composition names no concrete first-party executor here.
    insert_extension_lifecycle_handlers(
        &mut first_party_registry,
        Arc::clone(&extension_management),
        product_auth_services.runtime_credential_account_selection_service(),
    )
    .map_err(|error| RebornBuildError::InvalidConfig {
        reason: format!("extension lifecycle handlers are invalid: {error}"),
    })?;
    insert_admin_configuration_handler(
        &mut first_party_registry,
        Arc::clone(&admin_configuration),
        channel_egress_scope.user_id.clone(),
    )
    .map_err(|error| RebornBuildError::InvalidConfig {
        reason: format!("admin configuration handler is invalid: {error}"),
    })?;
    let operator_auto_approve_settings: Arc<dyn ironclaw_approvals::AutoApproveSettingStore> =
        Arc::clone(&auto_approve_settings) as Arc<dyn ironclaw_approvals::AutoApproveSettingStore>;
    let operator_tool_permission_overrides: Arc<
        dyn ironclaw_approvals::ToolPermissionOverrideStore,
    > = Arc::clone(&tool_permission_overrides)
        as Arc<dyn ironclaw_approvals::ToolPermissionOverrideStore>;
    let operator_persistent_approval_policies: Arc<
        dyn ironclaw_approvals::PersistentApprovalPolicyStore,
    > = Arc::clone(&stores.persistent_approval_policies)
        as Arc<dyn ironclaw_approvals::PersistentApprovalPolicyStore>;
    let operator_synthetic_tools = {
        let provider = outbound_delivery_synthetic_provider().map_err(|error| {
            RebornBuildError::InvalidConfig {
                reason: format!("outbound delivery synthetic provider id is invalid: {error}"),
            }
        })?;
        vec![
            outbound_delivery_target_set_operator_tool_info(provider).map_err(|error| {
                RebornBuildError::InvalidConfig {
                    reason: format!("outbound delivery operator tool is invalid: {error}"),
                }
            })?,
        ]
    };
    let operator_tool_catalog: Arc<dyn ironclaw_product_workflow::RebornOperatorToolCatalog> =
        Arc::new(ActiveRegistryOperatorToolCatalog::new(
            services.shared_extension_registry(),
            operator_synthetic_tools,
            Some(Arc::clone(&extension_management)),
        ));
    insert_operator_config_handler(
        &mut first_party_registry,
        operator_auto_approve_settings,
        operator_tool_permission_overrides,
        operator_persistent_approval_policies,
        operator_tool_catalog,
    )
    .map_err(|error| RebornBuildError::InvalidConfig {
        reason: format!("operator configuration handler is invalid: {error}"),
    })?;
    let outbound_target_provider = Arc::clone(&outbound_delivery_targets)
        as Arc<dyn crate::outbound::OutboundDeliveryTargetProvider>;
    let outbound_preferences_facade: Arc<dyn OutboundPreferencesProductFacade> =
        Arc::new(crate::outbound::RebornOutboundPreferencesFacade::new(
            Arc::clone(&outbound_stores.outbound_preferences),
            outbound_target_provider,
        ));
    insert_outbound_preferences_handler(&mut first_party_registry, outbound_preferences_facade)
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("outbound preferences handler is invalid: {error}"),
        })?;
    insert_skill_auto_activate_handler(
        &mut first_party_registry,
        Arc::clone(&skill_auto_activate_learned),
    )
    .map_err(|error| RebornBuildError::InvalidConfig {
        reason: format!("skill auto-activation handler is invalid: {error}"),
    })?;
    services = services.with_first_party_capabilities(Arc::new(first_party_registry));
    let channel_config_for_generic = Arc::clone(&channel_config_service);
    let channel_pairing_registry;
    let channel_host_wiring = {
        let reserved_capability_ids: std::collections::BTreeSet<_> = services
            .shared_extension_registry()
            .snapshot()
            .capabilities()
            .filter(|descriptor| {
                descriptor.provider.as_str() == ironclaw_host_runtime::BUILTIN_FIRST_PARTY_PROVIDER
            })
            .map(|descriptor| descriptor.id.clone())
            .collect();
        let channel_egress_credentials = Arc::new(
            crate::extension_host::channel_egress::ChannelConfigEgressCredentials::new(Arc::clone(
                &channel_config_for_generic,
            )),
        );
        #[cfg(feature = "test-support")]
        let channel_egress_credentials = Arc::new(
            crate::extension_host::channel_egress::BridgedChannelEgressCredentials::new(
                channel_egress_credentials,
            ),
        );
        #[cfg(feature = "test-support")]
        let channel_egress_credential_bridges = Arc::clone(&channel_egress_credentials);
        let channel_egress_transport = host_runtime_http_egress.clone().map(|port| {
            Arc::new(
                crate::extension_host::channel_egress::HostRuntimeChannelEgressTransport::new(
                    port,
                    channel_egress_credentials,
                    channel_egress_scope.clone(),
                ),
            ) as Arc<dyn ironclaw_extension_host::egress::ChannelEgressTransport>
        });
        let generic_installation_store = extension_management.installation_store_handle();
        let pairing_installation_store = Arc::clone(&generic_installation_store);
        let generic = crate::extension_host::generic_host::build_generic_extension_host(
            crate::extension_host::generic_host::GenericExtensionHostParams {
                binder: services.extension_lane_tool_binder(),
                native_factories: native_extension_factories,
                channel_adapters: channel_extension_bindings
                    .iter()
                    .map(|binding| (binding.extension_id.clone(), Arc::clone(&binding.adapter)))
                    .collect(),
                installation_store: generic_installation_store,
                channel_config: Some(Arc::clone(&channel_config_for_generic)),
                governor: Arc::clone(&resource_governor)
                    as Arc<dyn ironclaw_resources::ResourceGovernor>,
                reserved_capability_ids,
                reserved_ingress_routes:
                    crate::extension_host::extension_ingress::reserved_fixed_ingress_routes(),
                channel_egress_transport: channel_egress_transport.clone(),
            },
        )
        .await?;
        extension_management.attach_generic_host(Arc::clone(&generic.host));
        if let Some(ports) = services.product_auth_provider_runtime_ports() {
            extension_management.attach_discovery_runtime_ports(ports);
        }
        services.set_extension_tool_resolver(generic.resolver);
        let ingress_parts = crate::extension_host::extension_ingress::build_extension_ingress(
            generic.host.snapshot_watch(),
            Arc::clone(&deployment_channels),
            Arc::new(
                crate::extension_host::reply_contexts::FilesystemReplyContextStore::new(
                    Arc::clone(&fold_filesystem),
                    channel_egress_scope.tenant_id.clone(),
                    channel_egress_scope.user_id.clone(),
                ),
            ),
        );
        let channel_pairing_registry_built = {
            let registry =
                Arc::new(crate::extension_host::channel_pairing::ChannelPairingRegistry::default());
            for descriptor in &account_setup_descriptors {
                if !account_setups.declare(descriptor.clone()) {
                    return Err(RebornBuildError::InvalidConfig {
                        reason: format!(
                            "duplicate account-setup descriptor for extension `{}`",
                            descriptor.extension_id.as_str()
                        ),
                    });
                }
                if descriptor.connection_requirement.strategy
                    != ironclaw_product_workflow::RebornChannelConnectStrategy::WebGeneratedCode
                {
                    continue;
                }
                let extension_id = descriptor.extension_id.clone();
                let pairing_store = Arc::new(
                    crate::extension_host::channel_pairing::FilesystemChannelPairingStore::new(
                        Arc::clone(&fold_filesystem),
                        channel_egress_scope.tenant_id.clone(),
                        channel_egress_scope.user_id.clone(),
                        extension_id.clone(),
                    ),
                );
                let installation = Arc::new(
                    crate::extension_host::channel_pairing::StoredPairingInstallationSource::new(
                        Arc::clone(&pairing_installation_store),
                        extension_id.clone(),
                    ),
                );
                let template_values = Arc::new(
                    crate::extension_host::channel_pairing::ChannelConfigPairingTemplateValues::new(
                        Arc::clone(&channel_config_for_generic),
                        extension_id.clone(),
                        descriptor.pairing_deep_link_template.as_deref(),
                    ),
                );
                let roots =
                    crate::extension_host::channel_host::default_channel_workflow_storage_roots(
                        &channel_egress_scope.tenant_id,
                        extension_id.as_str(),
                    )
                    .map_err(|reason| RebornBuildError::InvalidConfig { reason })?;
                let workflow_state_factory =
                    crate::extension_host::channel_host::FilesystemChannelWorkflowStateFactory::new(
                        Arc::clone(&stores.filesystem),
                    );
                let workflow_state =
                    crate::extension_host::channel_host::ChannelWorkflowStateFactory::build(
                        &workflow_state_factory,
                        &roots,
                        channel_egress_scope.clone(),
                    )
                    .await
                    .map_err(|reason| RebornBuildError::InvalidConfig { reason })?;
                let continuation = auth_continuation_dispatcher(
                    turn_coordinator.clone(),
                    Some(Arc::clone(&turn_state)
                        as Arc<
                            dyn crate::blocked_auth_resume::BlockedAuthSnapshotSource,
                        >),
                );
                let agent_id = match channel_egress_scope.agent_id.clone() {
                    Some(agent_id) => agent_id,
                    None => ironclaw_host_api::AgentId::new("reborn").map_err(|error| {
                        RebornBuildError::InvalidConfig {
                            reason: format!(
                                "fallback channel pairing agent id is invalid: {error}"
                            ),
                        }
                    })?,
                };
                let service = Arc::new(
                    crate::extension_host::channel_pairing::ChannelPairingService::new(
                        crate::extension_host::channel_pairing::ChannelPairingServiceParts {
                            tenant_id: channel_egress_scope.tenant_id.clone(),
                            agent_id,
                            project_id: channel_egress_scope.project_id.clone(),
                            extension_id: extension_id.clone(),
                            connection_notices: descriptor.connection_notices.clone(),
                            deep_link_template: descriptor.pairing_deep_link_template.clone(),
                            store: pairing_store,
                            installation,
                            template_values,
                            identity_bind: Arc::clone(&channel_identity_store)
                                as Arc<dyn crate::provider_identity::RebornUserIdentityBindingStore>,
                            identity_lookup: Arc::clone(&channel_identity_store)
                                as Arc<dyn crate::provider_identity::RebornUserIdentityLookup>,
                            identity_delete: Arc::clone(&channel_identity_store)
                                as Arc<dyn crate::provider_identity::RebornUserIdentityBindingDeleteStore>,
                            continuation,
                            conversation_actor_pairings: Arc::clone(&workflow_state.conversations)
                                as Arc<dyn ironclaw_conversations::ConversationActorPairingService>,
                            dm_targets: Arc::clone(&channel_dm_target_store),
                        },
                    ),
                );
                if !account_setups.connect(
                    &descriptor.extension_id,
                    Arc::clone(&service)
                        as Arc<dyn ironclaw_product_workflow::AccountConnectionStatusSource>,
                ) {
                    return Err(RebornBuildError::InvalidConfig {
                        reason: format!(
                            "account-setup status source for `{}` was already connected",
                            descriptor.extension_id.as_str()
                        ),
                    });
                }
                registry.register(service);
            }
            registry
        };
        channel_pairing_registry = Some(channel_pairing_registry_built);
        let (delivery_coordinator, channel_delivery_resolver) = match channel_egress_transport {
            Some(transport) => {
                let resolver: Arc<dyn ironclaw_product_workflow::ChannelDeliveryResolver> =
                    Arc::new(
                        crate::extension_host::channel_delivery::SnapshotChannelDeliveryResolver::new(
                            generic.host.snapshot_watch(),
                            transport,
                        )
                        .with_deployment_channels(Arc::clone(&deployment_channels)),
                    );
                let coordinator = Arc::new(ironclaw_product_workflow::DeliveryCoordinator::new(
                    Arc::clone(&outbound_stores.outbound_state)
                        as Arc<dyn ironclaw_outbound::OutboundStateStore>,
                    Arc::clone(&resolver),
                    Arc::new(
                        crate::extension_host::channel_delivery::IngressReplyContextSource::new(
                            Arc::clone(&ingress_parts.reply_context),
                        ),
                    ),
                    ironclaw_product_workflow::DeliveryRetryPolicy::default(),
                ));
                (Some(coordinator), Some(resolver))
            }
            None => (None, None),
        };
        ChannelHostWiring {
            extension_ingress: Some(ingress_parts),
            delivery_coordinator,
            channel_delivery_resolver,
            #[cfg(feature = "test-support")]
            channel_egress_credential_bridges: Some(channel_egress_credential_bridges),
        }
    };
    let shared_extension_registry = services.shared_extension_registry();

    #[cfg(any(test, feature = "test-support"))]
    let local_dev_wasm_runtime_credential_provider_captured =
        services.wasm_runtime_credential_provider_captured_for_test();
    let host_runtime: Arc<dyn ironclaw_host_runtime::HostRuntime> = if uses_local_host_runtime {
        Arc::new(services.host_runtime_for_local_testing())
    } else {
        Arc::new(services.host_runtime_for_production(&wiring_config)?)
    };

    Ok(RebornRuntimeStores {
        host_runtime,
        turn_coordinator,
        readiness: readiness_for(profile, true, true, product_auth_ready),
        product_auth: product_auth_services,
        skill_management,
        extension_lifecycle_surface_context,
        owner_user_id,
        approval_requests: Arc::clone(&approval_requests),
        capability_leases: Arc::clone(&stores.leases),
        external_tool_catalog: Arc::new(InMemoryExternalToolCatalog::new()),
        runtime_policy: runtime_policy_for_return,
        persistent_approval_policies: Arc::clone(&stores.persistent_approval_policies),
        tool_permission_overrides: Arc::clone(&tool_permission_overrides),
        auto_approve_settings: Arc::clone(&auto_approve_settings),
        #[cfg(any(test, feature = "test-support"))]
        capability_policy: Arc::clone(&capability_policy),
        outbound_preferences: outbound_stores.outbound_preferences,
        outbound_delivery_targets: Arc::clone(&outbound_delivery_targets),
        skill_auto_activate_learned: Arc::clone(&skill_auto_activate_learned),
        outbound_state: outbound_stores.outbound_state,
        delivered_gate_routes: outbound_stores.delivered_gate_routes,
        triggered_run_delivery: outbound_stores.triggered_run_delivery,
        extension_management,
        channel_config: channel_config_service,
        admin_configuration,
        admin_configuration_uses: Arc::new(admin_configuration_uses),
        channel_identity_store,
        channel_dm_target_store,
        channel_disconnect_slot,
        runtime_http_egress,
        skill_mounts,
        memory_mounts,
        system_extensions_lifecycle_mounts,
        skill_filesystem,
        workspace_filesystem,
        extension_filesystem: Arc::clone(&stores.filesystem),
        workspace_mounts: runtime_workspace_mounts,
        local_dev_storage_root,
        default_system_prompt_path,
        #[cfg(any(test, feature = "test-support"))]
        in_memory_budget_event_sink,
        extension_registry: Arc::clone(&extension_registry),
        shared_extension_registry,
        scoped_filesystem: Arc::clone(&stores.scoped_filesystem),
        turn_state: Arc::clone(&turn_state),
        checkpoint_state_store,
        loop_checkpoint_store: Arc::clone(&turn_state) as Arc<dyn LoopCheckpointStore>,
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
        production_scheduler_wake: Some(scheduler_wake_wiring),
        secret_store,
        #[cfg(any(test, feature = "test-support"))]
        local_dev_wasm_runtime_credential_provider_captured,
        // `Ready` only when this path built a durable candidate source (i.e. no
        // caller-supplied product_auth_ports override); `Absent` otherwise. The
        // leader lock is always available on this production path.
        credential_refresh_worker,
        channel_extension_bindings,
        deployment_channels,
        extension_ingress: channel_host_wiring.extension_ingress,
        channel_pairing: channel_pairing_registry,
        delivery_coordinator: channel_host_wiring.delivery_coordinator,
        channel_delivery_resolver: channel_host_wiring.channel_delivery_resolver,
        #[cfg(feature = "test-support")]
        channel_egress_credential_bridges: channel_host_wiring.channel_egress_credential_bridges,
    })
}

/// Common tail of the libsql/postgres production build paths. After each
/// backend assembles its unified `CompositeRootFilesystem`, trigger repository,
/// event-store config, and refresh leader lock, this single-sources the
/// resource-governor + `ProductionStoreBundle` + backend build so the two paths
/// cannot drift on the store-assembly recipe.
async fn finish_production_backend(
    context: RebornProductionBuildContext,
    filesystem: Arc<CompositeRootFilesystem>,
    trigger_repository: Arc<dyn TriggerRepository>,
    secret_master_key: ironclaw_secrets::SecretMaterial,
    event_store_config: ironclaw_reborn_event_store::RebornEventStoreConfig,
    leader_lock: crate::product_auth::credentials::product_auth_refresh_lock::CredentialRefreshLeaderLock,
) -> Result<RebornRuntimeStores, RebornBuildError> {
    let resource_governor = filesystem_resource_governor(&filesystem);
    let stores = ProductionStoreBundle::new(
        filesystem,
        resource_governor,
        secret_master_key,
        event_store_config,
    )
    .await?;
    build_backend_production(context, stores, trigger_repository, leader_lock).await
}

async fn build_libsql_production(
    context: RebornProductionBuildContext,
    db: Arc<libsql::Database>,
    path_or_url: String,
    auth_token: Option<ironclaw_secrets::SecretMaterial>,
    secret_master_key: ironclaw_secrets::SecretMaterial,
    process_local_resource_governor_singleton: bool,
) -> Result<RebornRuntimeStores, RebornBuildError> {
    use ironclaw_filesystem::LibSqlRootFilesystem;

    ensure_libsql_resource_governor_authority_for_build(process_local_resource_governor_singleton)?;
    let database_filesystem = Arc::new(LibSqlRootFilesystem::new(Arc::clone(&db)));
    database_filesystem.run_migrations().await?;
    let trigger_repository = Arc::new(ironclaw_triggers::LibSqlTriggerRepository::new(db));
    trigger_repository
        .run_migrations()
        .await
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("libSQL trigger repository migrations failed: {error}"),
        })?;
    let filesystem =
        production_database_root_filesystem(database_filesystem, "production-libsql-reborn-state")?;
    finish_production_backend(
        context,
        filesystem,
        trigger_repository,
        secret_master_key,
        ironclaw_reborn_event_store::RebornEventStoreConfig::Libsql {
            path_or_url,
            auth_token,
        },
        crate::product_auth::credentials::product_auth_refresh_lock::CredentialRefreshLeaderLock::new(None),
    )
    .await
}

async fn build_postgres_production(
    context: RebornProductionBuildContext,
    pool: deadpool_postgres::Pool,
    secret_master_key: ironclaw_secrets::SecretMaterial,
    process_local_resource_governor_singleton: bool,
) -> Result<RebornRuntimeStores, RebornBuildError> {
    use ironclaw_filesystem::PostgresRootFilesystem;

    ensure_postgres_resource_governor_authority_for_build(
        process_local_resource_governor_singleton,
    )?;
    // A4: Clone the pool before it is moved into PostgresTriggerRepository so we
    // can thread it to the credential keepalive worker as a leader-lock for
    // sweep serialization.
    // This clone stays PRIVATE — it is never exposed through any public facade.
    let pool_for_refresh_lock = pool.clone();
    let database_filesystem = Arc::new(PostgresRootFilesystem::new(pool.clone()));
    database_filesystem.run_migrations().await?;
    let trigger_repository = Arc::new(ironclaw_triggers::PostgresTriggerRepository::new(
        pool.clone(),
    ));
    trigger_repository
        .run_migrations()
        .await
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("PostgreSQL trigger repository migrations failed: {error}"),
        })?;
    let filesystem = production_database_root_filesystem(
        database_filesystem,
        "production-postgres-reborn-state",
    )?;
    finish_production_backend(
        context,
        filesystem,
        trigger_repository,
        secret_master_key,
        ironclaw_reborn_event_store::RebornEventStoreConfig::PostgresPool { pool },
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
