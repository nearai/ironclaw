use std::{
    collections::BTreeMap,
    fmt,
    path::{Path, PathBuf},
    sync::Arc,
    sync::atomic::AtomicBool,
};

use crate::backend_store_assembly::{
    ProductionStoreBundle, SecretCredentialStores, build_filesystem_secret_credential_stores,
    filesystem_resource_governor, resolve_explicit_or_keychain_master_key,
    trigger_repository_for_durable_backend,
};
#[cfg(any(test, feature = "test-support"))]
use crate::builtin_capability_policy::BuiltinCapabilityPolicy;
use crate::builtin_capability_policy::builtin_capability_policy;
use crate::capability_authorization::{StoreApprovalSettingsProvider, capability_authorizer};
use crate::deployment::TrafficPolicy;
use crate::extension_host_assembly::{
    BackendChannelPairingAssemblyInput, BackendExtensionHostAssemblyInput,
    build_backend_channel_pairing, build_backend_extension_host,
};
#[cfg(any(test, feature = "test-support"))]
use crate::filesystem_assembly::build_default_database_roots;
#[cfg(test)]
use crate::filesystem_assembly::mount_descriptor;
use crate::filesystem_assembly::{
    DurableBackend, DurableStorageInput, build_filesystem, open_standalone_libsql_database,
    production_database_root_filesystem, standalone_db_path,
};
#[cfg(test)]
use crate::host_access_assembly::validate_workspace_skill_isolation;
use crate::host_access_assembly::{WorkspaceFilesystems, build_host_access};
use crate::input::{
    OAuthDcrCallbackConfig, OAuthProviderBackendConfig, PostgresPoolSource,
    RebornLocalRuntimeIdentity, RebornRuntimeProcessBinding, RebornStorageInput,
};
use crate::operator_tool_catalog::ActiveRegistryOperatorToolCatalog;
use crate::outbound::outbound_preferences_capability::{
    extend_builtin_first_party_package as extend_builtin_outbound_preferences_package,
    insert_handler as insert_outbound_preferences_handler,
};
use crate::outbound::{
    outbound_delivery_synthetic_provider, outbound_delivery_target_set_operator_tool_info,
};
use crate::outbound_store_assembly::build_outbound_stores;
use crate::runtime_input::RebornRuntimeIdentity;
use crate::runtime_mounts::{
    ambient_workspace_mount_view, memory_mount_view, scoped_skill_context_mount_view,
    skill_management_mount_view, workspace_mount_view,
};
#[cfg(all(test, unix))]
use crate::standalone_bootstrap_assembly::LEGACY_SKILLS_BACKFILL_MARKER;
#[cfg(test)]
use crate::standalone_bootstrap_assembly::backfill_legacy_user_skills;
use crate::standalone_bootstrap_assembly::bootstrap_standalone_host;
use crate::{
    RebornBuildError, RebornCompositionProfile, RebornHostBindings, RebornReadiness,
    RebornServiceReadiness, RebornWorkerReadiness,
};
use ironclaw_approvals::ApprovalRequestStore;
use ironclaw_approvals::{
    AutoApproveSettingStore, PersistentApprovalPolicyStore, ToolPermissionOverrideStore,
};
use ironclaw_auth::RebornProductAuthServicePorts;
use ironclaw_auth::product_auth::durable::{
    FilesystemAuthProductServices, UnavailableAuthProviderClient,
};
use ironclaw_auth::product_auth::oauth::oauth_gate::OAuthGateFlowDriver;
use ironclaw_auth::{
    AuthEngine, AuthEngineDeps, AuthProductError, AuthProductScope, AuthProviderClient,
    AuthRecipeResolver, AuthSurface, CredentialAccountStatus, EngineCallbackBase,
    EngineClientCredentialsSource, EngineOAuthClientMaterial, OAuthClientId,
    RebornAuthContinuationDispatcher, RebornProductAuthServices,
    RuntimeCredentialAccountRefreshService, RuntimeCredentialAccountSelectionService,
    StaticAuthRecipeResolver, map_account_error, runtime_credential_account_selection_request,
};
use ironclaw_authorization::CapabilityLeaseStore;
use ironclaw_authorization::GrantAuthorizer;
use ironclaw_capabilities::{
    CapabilityObligationAbortRequest, CapabilityObligationHandler, CapabilityObligationOutcome,
    CapabilityObligationPhase, CapabilityObligationRequest,
};
use ironclaw_conversations::RebornFilesystemConversationServices;
use ironclaw_conversations::{
    AdapterInstallationId, AdapterKind, ConversationActorPairingService, ExternalActorRef,
};
use ironclaw_events::{DurableAuditLog, DurableEventLog};
use ironclaw_extension_host::channel_pairing::ChannelPairingRegistry;
use ironclaw_extension_host::{
    ActiveExtensionPublisher, AdminConfigurationCatalogUse, AdminConfigurationService,
    AvailableExtensionCatalog, ChannelConfigService, ExtensionRemovalCleanupAdapter,
    ExtensionRemovalCleanupRegistry, FilesystemAdminConfigurationStore, FirstPartyRegistrarContext,
    ProviderInstanceReadinessInput, first_party_reserved_extension_ids, hosted_http_mcp_runtime,
    product_extension_host_api_contract_registry, provider_instance_readiness_map,
    restore_extension_lifecycle_state,
};
use ironclaw_extension_host::{
    admin_configuration::{
        ComposedAdminConfigurationService, ComposedExtensionAdminConfigurationResolver,
    },
    admin_configuration_capability::{
        extend_builtin_first_party_package as extend_builtin_admin_configuration_package,
        insert_handler as insert_admin_configuration_handler,
    },
    extension_lifecycle::{
        ExtensionCredentialCleanup, RebornLocalExtensionManagementPort,
        RebornProductAuthCredentialCleanup,
    },
    extension_lifecycle_capabilities::{
        extend_builtin_first_party_package, insert_handlers as insert_extension_lifecycle_handlers,
    },
    ironhub::{
        extend_builtin_first_party_package as extend_builtin_ironhub_package,
        insert_handlers as insert_ironhub_handlers,
    },
    operator_config_capability::{
        extend_builtin_first_party_package as extend_builtin_operator_config_package,
        insert_handler as insert_operator_config_handler,
    },
    skill_auto_activate_capability::{
        extend_builtin_first_party_package as extend_builtin_skill_auto_activate_package,
        insert_handler as insert_skill_auto_activate_handler,
    },
};
use ironclaw_extensions::{
    ExtensionInstallationStore, ExtensionInstallationStorePort, ExtensionLifecycleService,
    ExtensionRegistry, ManifestSource, SharedExtensionRegistry,
};
use ironclaw_filesystem::ScopedFilesystem;
#[cfg(test)]
use ironclaw_filesystem::{
    BackendCapabilities, BackendKind, ContentKind, DiskFilesystem, IndexPolicy, StorageClass,
};
use ironclaw_filesystem::{CompositeRootFilesystem, LibSqlRootFilesystem, RootFilesystem};
use ironclaw_host_api::runtime_policy::{
    DeploymentMode, EffectiveRuntimePolicy, FilesystemBackendKind, NetworkMode, ProcessBackendKind,
    SecretMode,
};
use ironclaw_host_api::{
    CapabilitySet, CorrelationId, CredentialStageError, ExtensionId, HostApiError, InvocationId,
    MountAlias, MountGrant, MountPermissions, MountView, NetworkPolicy, Obligation, PackageId,
    RecipeClientCredentials, ResourceEstimate, ResourceScope, RunId, RuntimeHttpEgress,
    RuntimeHttpEgressError, RuntimeHttpEgressRequest, RuntimeHttpEgressResponse, RuntimeKind,
    TrustClass, UserId, VendorId, VirtualPath, sha256_digest_token,
};
use ironclaw_host_runtime::memory_provider::MemoryServiceResolver;
use ironclaw_host_runtime::{
    CapabilitySurfaceVersion, FirstPartyCapabilityRegistry, HostProcessPort, HostRuntimeServices,
    PostEditCheckConfig, ProductAuthProviderRuntimePorts, RuntimeCredentialAccessSecret,
    RuntimeCredentialAccountRequest, RuntimeCredentialAccountResolver, TriggerCreateHook,
    builtin_first_party_package,
};
use ironclaw_host_runtime::{
    builtin_first_party_handlers_with_trigger_create_hook_for_process_backend,
    builtin_first_party_package_for_process_backend,
};
use ironclaw_outbound::{CommunicationPreferenceRepository, ReplyAttachmentIntentPort};
use ironclaw_outbound::{
    DeliveredGateRouteStore, OutboundStateStorePort, TriggeredRunDeliveryStore,
};
use ironclaw_processes::{ProcessConcurrencyLimits, ProcessJournalStore, ProcessServices};
use ironclaw_product::RebornProjectService;
use ironclaw_product::{
    ChannelConnectionNoticePolicy, ChannelConnectionRequirement, ExtensionAccountSetupDescriptor,
    ExtensionAccountSetupRegistry, LifecycleProductSurfaceContext,
    OutboundPreferencesProductService, ProductAuthTurnGateResumeDispatcher, ProjectService,
};
use ironclaw_projects::ProjectRepository;
use ironclaw_resources::InMemoryResourceGovernor;
use ironclaw_resources::{
    BroadcastBudgetEventSink, BudgetGateStore, BudgetGateStorePort, FilesystemResourceGovernor,
    ResourceGovernor,
};
use ironclaw_runner::runtime::ProcessRuntimeSystem;
use ironclaw_secrets::{SecretStore, SecretStorePort};
use ironclaw_skills::ScopedSkillManagementPort;
use ironclaw_threads::FilesystemSessionThreadService;
use ironclaw_threads::SessionThreadService;
use ironclaw_triggers::{
    TRIGGER_TRUSTED_ADAPTER_INSTALLATION_ID, TRIGGER_TRUSTED_ADAPTER_KIND,
    TRIGGER_TRUSTED_EXTERNAL_ACTOR_NAMESPACE, TriggerActiveRunLookup, TriggerError, TriggerRecord,
    TriggerRepository,
};
use ironclaw_trust::{AdminConfig, AdminEntry, HostTrustAssignment, HostTrustPolicy};
use ironclaw_turns::{
    AgentTurnRuntimePort, GetRunStateRequest, InMemoryRunProfileResolver, TurnScope,
};
use ironclaw_turns::{ExternalToolCatalog, InMemoryExternalToolCatalog};
use secrecy::SecretString;

mod auth_engine_assembly;
#[cfg(any(test, feature = "test-support"))]
pub(crate) use auth_engine_assembly::auth_continuation_dispatcher;
use auth_engine_assembly::{
    AdminConfigurationCredentialSlot, ProductAuthRuntimeCredentialResolver,
    ProductAuthServicesCompositionInput, compose_product_auth_services, compose_provider_client,
};
mod trigger_creation_assembly;
use trigger_creation_assembly::TriggerCreatorPairingHook;
pub(crate) use trigger_creation_assembly::{
    TriggerSourceReplyTarget, TurnStateTriggerSourceReplyTarget,
};
#[cfg(test)]
use trigger_creation_assembly::{
    pair_trigger_creator, validate_trigger_delivery_target_against_registry,
};
mod production_backend_assembly;
mod production_build_assembly;
mod runtime_lane_assembly;
#[cfg(any(test, feature = "test-support"))]
use production_backend_assembly::build_libsql_production;
#[cfg(test)]
use production_backend_assembly::ensure_libsql_resource_governor_authority_for_build;
use production_backend_assembly::{
    build_backend_production, build_postgres_production,
    ensure_postgres_resource_governor_authority_for_build,
};
pub(crate) use production_backend_assembly::{
    build_libsql_production_host_runtime_services, build_postgres_production_host_runtime_services,
};
#[cfg(test)]
pub(crate) use production_backend_assembly::{
    production_skill_management_mount_view, production_system_extensions_lifecycle_mount_view,
};
use production_build_assembly::{
    FilesystemProductionHostRuntimeServices, RebornProductionBuildContext, build_production_shaped,
    planned_run_profile_resolver,
};
pub(crate) use runtime_lane_assembly::apply_production_runtime_process_binding;
use runtime_lane_assembly::{
    apply_post_edit_check_from_env, attach_hosted_mcp_runtime, attach_wasm_runtime,
    default_host_http_egress, require_product_auth_runtime_ports,
};

/// Filename of the cached standalone secrets master-key dotfile under a
/// Reborn home / standalone root directory. `pub` (re-exported from `lib.rs`)
/// so onboarding (`ironclaw_reborn_cli::commands::onboard`) can check for its
/// presence without duplicating the literal.
pub const STANDALONE_SECRETS_MASTER_KEY_PATH: &str = ".reborn-local-dev-secrets-master-key";

pub(crate) type ComposedResourceGovernor = FilesystemResourceGovernor<CompositeRootFilesystem>;

pub(crate) type ComposedApprovalRequestStore = ApprovalRequestStore<CompositeRootFilesystem>;

pub(crate) type ComposedCapabilityLeaseStore = CapabilityLeaseStore<CompositeRootFilesystem>;

pub(crate) type ComposedPersistentApprovalPolicyStore =
    PersistentApprovalPolicyStore<CompositeRootFilesystem>;

pub(crate) type ComposedToolPermissionOverrideStore =
    ToolPermissionOverrideStore<CompositeRootFilesystem>;

pub(crate) type ComposedAutoApproveSettingStore = AutoApproveSettingStore<CompositeRootFilesystem>;

pub(crate) struct RebornRuntimeStores {
    pub(crate) host_runtime: Arc<dyn ironclaw_host_runtime::HostRuntime>,
    #[cfg(test)]
    pub(crate) turn_coordinator: Arc<dyn ironclaw_turns::TurnCoordinator>,
    pub(crate) product_auth: Arc<RebornProductAuthServices>,
    pub(crate) readiness: RebornReadiness,
    pub(crate) skill_management: Arc<ScopedSkillManagementPort>,
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
    pub(crate) outbound_state: Arc<dyn OutboundStateStorePort>,
    pub(crate) reply_attachment_intents: Arc<dyn ReplyAttachmentIntentPort>,
    pub(crate) delivered_gate_routes: Arc<dyn DeliveredGateRouteStore>,
    pub(crate) triggered_run_delivery: Arc<dyn TriggeredRunDeliveryStore>,
    pub(crate) process_gate_query_source:
        Arc<dyn ironclaw_processes::ProcessGateQuerySource<Error = ironclaw_turns::TurnError>>,
    /// Late-rebindable process lifecycle source the trigger active-run lookup
    /// reads. Production points it at this runtime's process journal; a
    /// `test-support` harness can repoint it at its own process runtime.
    #[cfg(any(test, feature = "test-support"))]
    #[allow(
        dead_code,
        reason = "held for test-support rebinding after runtime construction"
    )]
    pub(crate) trigger_process_lifecycle_source: Arc<
        std::sync::RwLock<
            Arc<
                dyn ironclaw_processes::ProcessLifecycleLookupSource<
                        Error = ironclaw_turns::TurnError,
                    >,
            >,
        >,
    >,
    /// Sibling read-only reply-target projection; repointed with the lifecycle
    /// source by test-support harnesses.
    #[cfg(any(test, feature = "test-support"))]
    #[allow(
        dead_code,
        reason = "held for test-support rebinding after runtime construction"
    )]
    pub(crate) trigger_source_reply_target:
        Arc<std::sync::RwLock<Arc<dyn TriggerSourceReplyTarget>>>,
    pub(crate) extension_management: Arc<RebornLocalExtensionManagementPort>,
    pub(crate) admin_configuration: Arc<ComposedAdminConfigurationService>,
    pub(crate) admin_configuration_uses: Arc<Vec<AdminConfigurationCatalogUse>>,
    /// Deployment-first current delivery-target resolver (extension-runtime
    /// §5.4): the run-delivery observer half reads it to route a run's final
    /// reply to the caller's active channel target.
    pub(crate) channel_config_service: Arc<ChannelConfigService>,
    pub(crate) channel_identity_store: Arc<ironclaw_extension_host::FilesystemChannelIdentityStore>,
    pub(crate) channel_dm_target_store:
        Arc<ironclaw_extension_host::FilesystemChannelDmTargetStore>,
    pub(crate) channel_disconnect_slot:
        Arc<std::sync::OnceLock<Arc<dyn ironclaw_product::ChannelConnectionService>>>,
    pub(crate) runtime_http_egress: Option<Arc<dyn RuntimeHttpEgress>>,
    pub(crate) host_runtime_http_egress: Option<ironclaw_host_runtime::HostRuntimeHttpEgressPort>,
    pub(crate) skill_mounts: MountView,
    pub(crate) memory_mounts: MountView,
    pub(crate) system_extensions_lifecycle_mounts: MountView,
    pub(crate) skill_filesystem: Arc<ScopedFilesystem<CompositeRootFilesystem>>,
    pub(crate) workspace_filesystem: Arc<ScopedFilesystem<CompositeRootFilesystem>>,
    pub(crate) extension_filesystem: Arc<CompositeRootFilesystem>,
    /// Single memory provider resolver (issue #3537). Both the memory tools and
    /// the standalone profile source build their `MemoryService` through this, so
    /// profile reads and tools agree on the bound provider (native, or
    /// degrade-to-empty for disabled/third-party).
    pub(crate) memory_service_resolver: MemoryServiceResolver,
    /// Lifecycle hooks declared by the bound memory provider. Host-initiated
    /// retrieval, recording, and profile reads are wired only when declared.
    pub(crate) memory_lifecycle: ironclaw_host_api::MemoryDescriptor,
    pub(crate) workspace_mounts: MountView,
    pub(crate) standalone_storage_root: Option<PathBuf>,
    pub(crate) default_system_prompt_path: Option<PathBuf>,
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) in_memory_budget_event_sink: Arc<ironclaw_resources::InMemoryBudgetEventSink>,
    pub(crate) extension_registry: Arc<ExtensionRegistry>,
    pub(crate) shared_extension_registry: Arc<SharedExtensionRegistry>,
    pub(crate) scoped_filesystem: Arc<ScopedFilesystem<CompositeRootFilesystem>>,
    pub(crate) processes: ProcessRuntimeSystem,
    pub(crate) thread_service: Arc<dyn SessionThreadService>,
    pub(crate) trigger_repository: Arc<dyn TriggerRepository>,
    pub(crate) resource_governor: Arc<dyn ResourceGovernor>,
    pub(crate) budget_gate_store: Arc<dyn BudgetGateStorePort>,
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
    pub(crate) secret_store: Arc<dyn SecretStorePort>,
    #[cfg(test)]
    pub(crate) standalone_wasm_runtime_credential_provider_captured: bool,
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
        Option<ironclaw_extension_host::extension_ingress::ExtensionIngressParts>,
    /// Pairing services for `WebGeneratedCode` channel extensions, built
    /// from the binary-assembled account-setup descriptors; the channel host
    /// assembly consumes it for sink gates and actor resolution.
    pub(crate) channel_pairing: Option<Arc<ChannelPairingRegistry>>,
    /// The generic delivery coordinator (extension-runtime §5.4): the sole
    /// writer of outbound delivery state, resolving channel adapters +
    /// policy egress from deployment bindings or the active compatibility
    /// snapshot. `None` when the composition path builds no channel egress
    /// transport.
    pub(crate) delivery_coordinator: Option<Arc<ironclaw_product::DeliveryCoordinator>>,
    /// The deployment-first channel delivery resolver behind the coordinator,
    /// exposed separately for host flows (e.g. DM target provisioning) that
    /// need one stable adapter + egress read outside a delivery.
    pub(crate) channel_delivery_resolver:
        Option<Arc<dyn ironclaw_product::ChannelDeliveryResolver>>,
    /// Registry of beta-era channel credential bridges (§11 compatibility):
    /// channel hosts whose secrets predate the extension-config store
    /// register resolution ports here.
    #[cfg(feature = "test-support")]
    pub(crate) channel_egress_credential_bridges:
        Option<Arc<ironclaw_extension_host::channel_egress::BridgedChannelEgressCredentials>>,
}

struct ChannelHostWiring {
    extension_ingress: Option<ironclaw_extension_host::extension_ingress::ExtensionIngressParts>,
    delivery_coordinator: Option<Arc<ironclaw_product::DeliveryCoordinator>>,
    channel_delivery_resolver: Option<Arc<dyn ironclaw_product::ChannelDeliveryResolver>>,
    #[cfg(feature = "test-support")]
    channel_egress_credential_bridges:
        Option<Arc<ironclaw_extension_host::channel_egress::BridgedChannelEgressCredentials>>,
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
        leader_lock: ironclaw_auth::CredentialRefreshLeaderLock,
        refresh_port: Arc<RebornProductAuthServices>,
    },
    /// Deps intentionally absent: standalone (single-user, no cross-owner
    /// enumeration), or a caller-supplied `product_auth_ports` override/test
    /// path. The sweep never starts.
    Absent,
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
    mount_default_database_roots, open_standalone_approval_request_store_for_test,
    open_standalone_approval_settings_stores_for_test,
    open_standalone_extension_installation_store_for_test,
    open_standalone_outbound_preferences_store_for_test, open_standalone_root_filesystem_for_test,
    open_standalone_trigger_repository_for_test,
};

impl std::fmt::Debug for RebornRuntimeStores {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut debug = formatter.debug_struct("RebornRuntimeStores");
        debug
            .field("host_runtime", &"Arc<dyn HostRuntime>")
            .field("turn_coordinator", &cfg!(test))
            .field("product_auth", &"Arc<RebornProductAuthServices>")
            .field("readiness", &self.readiness)
            .field("extension_management", &true)
            .field("scoped_filesystem", &"Arc<ScopedFilesystem>")
            .field("turn_state", &"Arc<TurnStateRowStore>");
        debug.finish()
    }
}

pub(crate) fn filesystem_reborn_identity_store<F>(
    scoped_filesystem: Arc<ScopedFilesystem<F>>,
    tenant_id: ironclaw_host_api::TenantId,
    actor_user_id: UserId,
    agent_id: ironclaw_host_api::AgentId,
    project_id: Option<ironclaw_host_api::ProjectId>,
) -> Arc<ironclaw_reborn_identity::RebornIdentityStore<F>>
where
    F: RootFilesystem + 'static,
{
    Arc::new(ironclaw_reborn_identity::RebornIdentityStore::new(
        scoped_filesystem,
        tenant_id,
        actor_user_id,
        agent_id,
        project_id,
    ))
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

/// Build the safe single-tenant runtime surface used by standalone and
/// hosted-single-tenant. Hosted single-tenant supplies a durable Postgres
/// backend through `RebornStorageInput::HostedSingleTenantPostgres`; standalone
/// keeps its historical local filesystem/libSQL default.
fn extension_lifecycle_surface_context(
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

/// The `HostRuntimeServices` wiring shared by the standalone and production
/// build paths (F4): the shared `.with_*` setters both paths always apply, plus
/// the fixed `TracingSecurityAuditSink`. Single-sourced as a macro because the
/// builder is generic over backend type params and the setters are
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
        process_runtime = $process_runtime:expr,
        approval_filesystem = $fs:expr,
        turn_state = $turn_state:expr,
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
            .with_process_journal_invocation_state($process_runtime, $fs)
            .with_turn_state($turn_state)
            .with_run_profile_resolver($resolver)
    };
}
pub(super) use with_shared_host_runtime_wiring;

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

pub(crate) async fn build_secret_store<F>(
    root: &Path,
    scoped_filesystem: Arc<ScopedFilesystem<F>>,
    explicit_master_key: Option<ironclaw_secrets::SecretMaterial>,
) -> Result<(Arc<SecretStore<F>>, Arc<ironclaw_secrets::SecretsCrypto>), RebornBuildError>
where
    F: RootFilesystem + 'static,
{
    let master_key = match explicit_master_key {
        Some(master_key) => master_key,
        None => resolve_standalone_secret_master_key(root).await?,
    };
    // The crypto is returned alongside the store so the admin secret
    // provisioner (`admin_secrets.rs`) can build per-target-user stores that
    // share the SAME master key — secrets written admin-side decrypt under the
    // user's own store and vice versa.
    let crypto = Arc::new(ironclaw_secrets::SecretsCrypto::new(master_key)?);
    let store = Arc::new(SecretStore::new(scoped_filesystem, Arc::clone(&crypto)));
    Ok((store, crypto))
}

/// Open the `/secrets` store alone, without building the rest of the
/// standalone [`CompositeRootFilesystem`] (project mounts, extension mounts,
/// trigger/project repositories, …).
///
/// - Pre-composition entry point `ironclaw-reborn onboard` needs: it must
///   write a provider API key before a full build-input-driven build exists,
///   and reconstructing the whole composite just to reach one mount is
///   heavy and risks silently diverging from `serve`'s copy.
/// - `/secrets`'s physical backing is the same standalone libSQL file
///   `build_standalone_root_filesystem` opens for `/tenants` in production —
///   a key written here is immediately visible to `serve`, no extra
///   coordination needed.
/// - Uses the same resolver chain as production (env -> cached dotfile ->
///   OS keychain -> generate-and-cache, via [`build_secret_store`]).
/// - `run_migrations()` here and again on `serve`'s later open is safe —
///   already relied on as idempotent elsewhere in this module's tests.
pub async fn open_standalone_secret_store(
    root: &Path,
) -> Result<Arc<dyn SecretStorePort>, RebornBuildError> {
    let db = open_standalone_libsql_database(root).await?;
    let filesystem = Arc::new(LibSqlRootFilesystem::new(db)?);
    filesystem.run_migrations().await?;
    let scoped = crate::wrap_scoped(filesystem);
    let (store, _crypto) = build_secret_store(root, scoped, None).await?;
    Ok(store as Arc<dyn SecretStorePort>)
}

/// Where a resolved standalone master key came from, used to name the source in
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
                "standalone secrets master key from {location} is malformed: {error}; \
                 it must be at least 32 bytes with at least 8 distinct byte values. \
                 Remove or replace it and retry."
            ),
        }
    })
}

async fn resolve_standalone_secret_master_key(
    root: &Path,
) -> Result<ironclaw_secrets::SecretMaterial, RebornBuildError> {
    // Fail closed on an explicitly-set-but-unusable master key: only an
    // *absent* env var is "not configured". A non-Unicode value must not be
    // silently dropped (via `.ok()`) and fall through to generating a fresh
    // key, which would encrypt standalone secrets under an unintended key the
    // operator never chose.
    let env_key = match std::env::var(ironclaw_secrets::keychain::SECRETS_MASTER_KEY_ENV) {
        Ok(value) => Some(value),
        Err(std::env::VarError::NotPresent) => None,
        Err(std::env::VarError::NotUnicode(_)) => {
            return Err(RebornBuildError::InvalidConfig {
                reason: format!(
                    "standalone secrets master key env var {} is set but not valid UTF-8",
                    ironclaw_secrets::keychain::SECRETS_MASTER_KEY_ENV
                ),
            });
        }
    };
    resolve_standalone_secret_master_key_with_env(root, env_key).await
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
async fn resolve_standalone_secret_master_key_with_env(
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
                        "standalone secrets master key env var {} is set but empty",
                        ironclaw_secrets::keychain::SECRETS_MASTER_KEY_ENV
                    ),
                });
            }
            validate_resolved_master_key(&trimmed, &MasterKeySource::Env)?;
            Some(trimmed)
        }
        None => None,
    };

    let key_path = root.join(STANDALONE_SECRETS_MASTER_KEY_PATH);
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
                    "standalone secrets master key at {} could not be read: {error}",
                    key_path.display()
                ),
            });
        }
    }

    // No cached file. Prefer the explicit (already-validated) env key.
    if let Some(key) = env_key {
        write_standalone_secret_master_key(&key_path, &key)?;
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
    write_standalone_secret_master_key(&key_path, &key)?;
    Ok(ironclaw_secrets::SecretMaterial::from(key))
}

fn write_standalone_secret_master_key(path: &Path, key: &str) -> Result<(), RebornBuildError> {
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
                reason: format!("standalone secrets master key could not be created: {error}"),
            })?;
        file.write_all(key.as_bytes())
            .and_then(|_| file.write_all(b"\n"))
            .map_err(|error| RebornBuildError::InvalidConfig {
                reason: format!("standalone secrets master key could not be written: {error}"),
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
                reason: format!("standalone secrets master key could not be created: {error}"),
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
                reason: "standalone secrets master key could not be restricted: USERNAME is unset"
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
                    "standalone secrets master key permissions could not be set: {error}"
                ),
            })?;
        if !status.success() {
            let _ = std::fs::remove_file(path);
            return Err(RebornBuildError::InvalidConfig {
                reason: format!(
                    "standalone secrets master key permissions could not be set: icacls exited with {status}"
                ),
            });
        }
        file.write_all(key.as_bytes())
            .and_then(|_| file.write_all(b"\n"))
            .map_err(|error| RebornBuildError::InvalidConfig {
                reason: format!("standalone secrets master key could not be written: {error}"),
            })
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        let _ = key;
        Err(RebornBuildError::InvalidConfig {
            reason:
                "standalone filesystem secret persistence requires Unix permissions or Windows ACLs"
                    .to_string(),
        })
    }
}

/// Outcome of provisioning a standalone secrets master key directly into the
/// OS keychain (as opposed to `resolve_standalone_secret_master_key_with_env`'s
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
///   matching `resolve_standalone_secret_master_key_with_env`'s env/dotfile fallback.
pub async fn provision_standalone_keychain_master_key() -> KeychainMasterKeyOutcome {
    // `has_master_key()` collapses "no key yet" and "backend/permission/locked
    // error probing the keychain" into the same `false` — a false negative
    // here falls through to `generate` + `store` below, which overwrites
    // whatever key the keychain actually holds. Same accepted-risk class as
    // the TOCTOU documented on this function's only caller
    // (`ironclaw_reborn_cli::commands::onboard::master_key::provision_master_key`):
    // Standalone, single-operator, run-once-by-hand; worst case is a
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
                "OS keychain store of standalone secrets master key failed during onboarding; \
                 falling back to env/dotfile resolution"
            );
            KeychainMasterKeyOutcome::Suppressed
        }
    }
}

/// The host-owned outbound target registry always exposes the WebApp
/// final-reply destination (#6520 run-scoped delivery): channel extensions add
/// their targets at activation, but "store the answer in run history" is a
/// host affordance that must exist even with zero channels active.
fn host_owned_outbound_delivery_target_registry()
-> Result<Arc<crate::outbound::MutableOutboundDeliveryTargetRegistry>, RebornBuildError> {
    let registry = Arc::new(crate::outbound::MutableOutboundDeliveryTargetRegistry::default());
    let web_app = ironclaw_outbound::OutboundDeliveryTargetSummary::new(
        ironclaw_outbound::OutboundDeliveryTargetId::new(
            ironclaw_outbound::WEB_APP_OUTBOUND_DELIVERY_TARGET_ID,
        )
        .map_err(|reason| RebornBuildError::InvalidConfig {
            reason: format!("host-owned WebApp target id is invalid: {reason}"),
        })?,
        "web_app",
        "Web app only",
        Some("Store the final answer in run history without external delivery.".to_string()),
    )
    .map_err(|reason| RebornBuildError::InvalidConfig {
        reason: format!("host-owned WebApp delivery target is invalid: {reason}"),
    })?;
    registry
        .register_provider(
            ironclaw_outbound::WEB_APP_OUTBOUND_DELIVERY_TARGET_ID,
            Arc::new(
                ironclaw_outbound::HostOwnedOutboundDeliveryTargetProvider::new(
                    web_app,
                    ironclaw_outbound::DeliveryTargetCapabilities {
                        final_replies: true,
                        progress: false,
                        gate_prompts: false,
                        auth_prompts: false,
                        modalities: Vec::new(),
                    },
                    ironclaw_outbound::RunFinalReplyDestination::WebApp,
                ),
            ),
        )
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("host-owned WebApp delivery target registration failed: {error}"),
        })?;
    Ok(registry)
}

pub(crate) fn builtin_extension_registry() -> Result<ExtensionRegistry, RebornBuildError> {
    // Shared by standalone and production composition so host-owned first-party
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

/// Insert the bound memory provider's package into a registry that already
/// holds the builtin package. A disabled or unconstructible binding registers
/// no memory tools.
fn insert_bound_memory_package(
    registry: &mut ExtensionRegistry,
    memory_package: Option<&ironclaw_extensions::ExtensionPackage>,
) -> Result<(), RebornBuildError> {
    let Some(package) = memory_package else {
        return Ok(());
    };
    registry
        .insert(package.clone())
        .map_err(|error| RebornBuildError::InvalidConfig {
            reason: format!("bound memory provider registry is invalid: {error}"),
        })
}

fn production_builtin_extension_registry(
    process_backend: ProcessBackendKind,
    memory_package: Option<&ironclaw_extensions::ExtensionPackage>,
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
    let package = extend_builtin_ironhub_package(package).map_err(|error| {
        RebornBuildError::InvalidConfig {
            reason: format!("IronHub package is invalid: {error}"),
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
    insert_bound_memory_package(&mut registry, memory_package)?;
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

fn manifest_channel_account_setup_descriptors(
    manifests: &[Arc<ironclaw_extensions::ResolvedExtensionManifest>],
) -> Vec<ExtensionAccountSetupDescriptor> {
    manifests
        .iter()
        .filter_map(|manifest| {
            let channel = manifest.channel.as_ref()?;
            let connection = channel.connection.as_ref()?;
            if connection.strategy != ironclaw_host_api::ChannelConnectionStrategy::WebGeneratedCode
            {
                return None;
            }
            Some(ExtensionAccountSetupDescriptor {
                extension_id: manifest.id.clone(),
                auth_requirement: ironclaw_host_api::RuntimeCredentialAuthRequirement {
                    provider: connection.provider.clone(),
                    setup: ironclaw_host_api::RuntimeCredentialAccountSetup::Pairing,
                    requester_extension: manifest.id.clone(),
                    provider_scopes: Vec::new(),
                },
                connection_requirement: ChannelConnectionRequirement {
                    channel: manifest.id.as_str().to_string(),
                    display_name: manifest.name.clone(),
                    strategy: ironclaw_product::RebornChannelConnectStrategy::WebGeneratedCode,
                    instructions: connection.instructions.clone(),
                    input_placeholder: connection.input_placeholder.clone(),
                    submit_label: connection.submit_label.clone(),
                    error_message: connection.error_message.clone(),
                },
                connection_notices: ChannelConnectionNoticePolicy {
                    connect_required: connection.notices.connect_required.clone(),
                    paired: connection.notices.paired.clone(),
                    already_paired_same_user: connection.notices.already_paired_same_user.clone(),
                    already_bound_to_other_user: connection
                        .notices
                        .already_bound_to_other_user
                        .clone(),
                    expired_or_unknown: connection.notices.expired_or_unknown.clone(),
                },
                activation_success_message: connection.connection_success_message.clone(),
                pairing_deep_link_template: connection.deep_link_template.clone(),
                inbound_code_prefixes: connection.inbound_code_prefixes.clone(),
            })
        })
        .collect()
}

/// Build the production first-party trust policy from the binary-injected
/// neutral bundle set (extension-runtime DEL-7). The provider entry comes from
/// `builtin_capability_policy` (no first-party dependency); each package's host
/// authority grant is sourced from its injected `trust_effects` instead of a
/// direct `ironclaw_first_party_extensions` call. Every entry is byte-identical
/// to the one the inventory-driven builder produced — same id, local-manifest
/// path, manifest digest, and effect list — so behavior is preserved exactly.
pub fn production_first_party_trust_policy(
    bundles: &[ironclaw_extension_host::FirstPartyPackageBundle],
) -> Result<HostTrustPolicy, RebornBuildError> {
    let policy = builtin_capability_policy().map_err(|error| RebornBuildError::InvalidConfig {
        reason: format!("standalone capability policy is invalid: {error}"),
    })?;
    let mut entries = vec![AdminEntry::for_local_manifest(
        policy.provider.id,
        policy.provider.manifest_path,
        None,
        HostTrustAssignment::first_party(),
        policy.provider.authority_effects,
        None,
    )];
    for provider in ironclaw_host_runtime::memory_native_extension::MEMORY_PROVIDER_PACKAGE_IDS {
        entries.push(AdminEntry::for_local_manifest(
            PackageId::new(*provider).map_err(|error| RebornBuildError::InvalidConfig {
                reason: format!("memory provider package id '{provider}' is invalid: {error}"),
            })?,
            format!("/system/extensions/{provider}/manifest.toml"),
            None,
            HostTrustAssignment::first_party(),
            vec![
                ironclaw_host_api::EffectKind::DispatchCapability,
                ironclaw_host_api::EffectKind::ReadFilesystem,
                ironclaw_host_api::EffectKind::WriteFilesystem,
            ],
            None,
        ));
    }
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
        &ironclaw_extension_host::test_support::first_party_bundles_from_inventory(),
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
        services: RebornServiceReadiness {
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
mod capability_host_tests;
