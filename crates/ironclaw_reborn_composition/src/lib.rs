#![forbid(unsafe_code)]

//! Reborn composition root.
//!
//! Main entry point:
//!
//! - [`build_runtime`] — full runtime assembly: deployment config + loop
//!   driver registry + LLM model gateway + turn-runner worker, spawned
//!   as one unit. This is the single entry
//!   point used by the standalone `ironclaw-reborn` binary and any
//!   future Reborn ingress.
//!
//! Downstream callers should not name internal Reborn types directly:
//! [`RebornRuntime`] exposes only task-level methods, so callers never
//! import `TurnCoordinator`, `SessionThreadService`, `HostManagedModel
//! Gateway`, etc.

use std::sync::Arc;

mod admin_secrets;
#[cfg(test)]
mod approval_test_support;
mod automation;
mod backend_store_assembly;
mod builtin_capability_policy;
mod capability_authorization;
#[cfg(test)]
#[path = "extension_lifecycle_capabilities_auth_tests.rs"]
mod composition_extension_lifecycle_auth_tests;
pub mod deployment;
mod error;
mod extension_host_assembly;
mod factory;
mod filesystem_assembly;
mod google_oauth_secret_store;
mod host_access_assembly;
mod input;
mod ironhub_link_serve;
mod llm_admin;
mod memory_binding;
mod memory_provider_factory;
mod model_gateway_assembly;
mod observability;
mod operator_secret_store;
mod operator_tool_catalog;
mod outbound;
mod outbound_store_assembly;
mod product_capability;
mod product_surface;
mod production_runtime_policy;
mod readiness;
mod root;
mod runtime;
mod runtime_input;
mod runtime_mounts;
mod sandbox;
mod standalone_bootstrap_assembly;
mod storage_catalog;
mod support;
#[cfg(feature = "test-support")]
pub mod test_support;
mod trigger_fire_access;
mod trigger_poller_assembly;

// The public re-export wall — a *documented* surface (PROPOSAL §6.10, CHECKLIST WS6
// "`RebornRuntime` slimmed"). An entry earns its place only when the consumer cannot
// reach the symbol at its owner: no dependency on that crate (the app tier has none by
// design), or a private home module here. Anything importable from its owner must be.
// `pinned by` = the test that fails if the entry goes, else the build that does. Gated by
// `reborn_composition_boundaries.rs`: `..._surface_matches_snapshot` pins the set,
// `..._entries_name_their_consumer` pins these annotations.
// consumer: `ironclaw_conversations::inbound`, `tests/integration/support/triggered_submit.rs` · pinned by: `ironclaw_conversations/tests/inbound_contract.rs`
pub use automation::conversation_turn_submitter::conversation_turn_submitter;
// consumer: every `build_*` caller in the app tier and the test tiers · pinned by: `composition/tests/service_factory.rs`
pub use error::RebornBuildError;
// consumer: `tests/integration/support/harness` recorder · pinned by: `tests/integration/support/harness/recorder.rs`
#[cfg(feature = "test-support")]
pub use factory::AttachmentTestSupport;
// consumer: root integration harness · pinned by: `tests/integration/extension_delivery.rs`
#[cfg(feature = "test-support")]
pub use factory::ChannelHostAssemblyTestWiring;
// consumer: root integration harness · pinned by: `tests/integration/support/harness/mod.rs`
#[cfg(feature = "test-support")]
pub use factory::RebornApprovalTestParts;
// consumer: `ironclaw_reborn_cli` onboard + runtime + status · pinned by: `ironclaw_reborn_cli/tests/smoke.rs`
pub use factory::STANDALONE_SECRETS_MASTER_KEY_PATH;
/// Crate-root alias for composition's own `#[cfg(test)]` trust-policy builders.
#[cfg(test)]
pub(crate) use factory::builtin_first_party_trust_policy;
// consumer: `ironclaw_reborn_cli` config/set + onboard + runtime · pinned by: `ironclaw_reborn_cli/tests/smoke.rs`
pub use factory::open_standalone_secret_store;
/// Production first-party trust-policy builder over the neutral injected bundle set,
/// public so integration tests build the same policy the binary composes.
// consumer: composition's own contract tests · pinned by: `composition/tests/support/first_party.rs`
pub use factory::production_first_party_trust_policy;
// consumer: `ironclaw_reborn_cli` onboard/master_key · pinned by: `ironclaw_reborn_cli` build (the outcome is the fn's return type; `factory` is private)
pub use factory::{KeychainMasterKeyOutcome, provision_standalone_keychain_master_key};
// consumer: `ironclaw_reborn_cli` status + runtime · pinned by: `ironclaw_reborn_cli` build
pub use filesystem_assembly::standalone_db_path;
// consumer: `ironclaw_reborn_cli` config/set · pinned by: `ironclaw_reborn_cli` build (the error is the store's; module is private)
pub use google_oauth_secret_store::{GoogleOauthSecretStore, GoogleOauthSecretStoreError};
// consumer: `ironclaw_reborn_cli` serve/runtime/native_extensions, `harness/latency/runner` · pinned by: `composition/tests/admin_api_e2e.rs`
pub use input::{
    ChannelExtensionBinding, OAuthClientConfig, RebornHostBindings, RebornRuntimeProcessBinding,
};
// WS1.4 deleted the `extension_contracts::channel_adapter` second import path; WS6 did
// the same for the `auth`/`host_api`/`host_runtime`/`product_contracts`/`failure_lane`/
// `runtime_policy`/`triggers`/`provider_identity` pass-throughs.
// consumer: `ironclaw_reborn_cli` extension command (no `ironclaw_product` dep) · pinned by: `ironclaw_reborn_cli` build
pub use ironclaw_product::LifecycleProductResponse;
// consumer: `ironclaw_reborn_cli` runtime (no `ironclaw_runner` dep) · pinned by: `ironclaw_reborn_cli` build
pub use ironclaw_runner::runtime::DEFAULT_TURN_RUNNER_WORKER_COUNT;
// consumer: `ironclaw_reborn_cli` skills command (no `ironclaw_skills` dep) · pinned by: `ironclaw_reborn_cli` build
pub use ironclaw_skills::{
    SkillSummary as RebornSkillSummary, skill_summary_json as reborn_skill_summary_json,
};
// consumer: `ironclaw_reborn_cli` runtime (no `ironclaw_turns` dep) · pinned by: `ironclaw_reborn_cli` build
pub use ironclaw_turns::TurnStatus;
// consumer: `ironclaw_reborn_cli` serve wiring · pinned by: `ironclaw_reborn_cli` build
pub use llm_admin::openai_compat_serve::build_openai_compat_route_mount;
// consumer: `ironclaw_reborn_cli` runtime · pinned by: `composition/tests/memory_mem0_swap.rs`
pub use memory_binding::{memory_binding_diagnostics, resolve_memory_binding_policy};
// consumer: `ironclaw_reborn_cli` runtime, `tests/integration/group_memory` · pinned by: `composition/tests/memory_mem0_swap.rs` (`MemoryLifecycleConsumers` is the fn's return type)
pub use memory_provider_factory::{
    Mem0ConnectionConfig, MemoryLifecycleConsumers, MemoryProviderDeps, ResolvedMemoryProvider,
    memory_lifecycle_consumers, resolve_memory_provider,
};
// consumer: composition's operator LLM-key wiring test · pinned by: `composition/tests/operator_llm_key_store_wiring.rs`
pub use operator_secret_store::RuntimeOperatorSecretValueStore;
// consumer: `ironclaw_reborn_cli` explicit sandbox-profile boot wiring · pinned by: `sandbox::tests`
pub use sandbox::UserSandboxFactory;
// consumer: `ironclaw_reborn_cli` Railway profile config · pinned by: `ironclaw_sandbox::sandbox_process::railway::tests`
pub use ironclaw_sandbox::RailwayPreviewSandboxConfig;
// consumer: `ironclaw_reborn_cli` serve + runtime, `harness/latency/runner`, root QA suites · pinned by: `composition/tests/profile_acceptance.rs`
// (`RebornRuntimeProfileError` left: `deployment` is a `pub mod`, so it stays nameable there.)
pub use deployment::{
    RebornRuntimeProfileOptions, hosted_single_tenant_runtime_policy,
    hosted_single_tenant_volume_runtime_policy,
    hosted_single_tenant_volume_sandboxed_runtime_policy, local_runtime_build_input,
    local_runtime_build_input_with_options, standalone_runtime_policy,
    standalone_unrestricted_runtime_policy,
};
// consumer: `ironclaw_product/tests/support/planned_agent_loop.rs`, root integration harness · pinned by: `composition/tests/budget_e2e.rs`
#[cfg(any(test, feature = "test-support"))]
pub use deployment::{local_filesystem_build_input, local_filesystem_build_input_with_profile};
// consumer: `ironclaw_reborn_cli` serve wiring · pinned by: `composition/tests/webui_v2_serve.rs`
pub use ironhub_link_serve::{
    IRONHUB_REGISTER_PATH, IronhubRegisterRouteState, ironhub_register_route_mount,
};
// consumer: root integration harness group wiring · pinned by: `tests/integration/support/group.rs`
pub use observability::budget::build_default_budget_accountant;
// consumer: composition budget contract tests · pinned by: `composition/tests/budget_e2e.rs`
pub use observability::budget_events::BudgetEventObserver;
// consumer: composition hook-projection tests · pinned by: `composition/tests/third_party_hook_projection.rs` (the factory type is the builder fn's return type; `observability` is private)
pub use observability::hooks::{
    HookDispatcherBuilderFactory, HookProjectionRegistry, HooksActivationConfig,
    MAX_INSTALLED_EXTENSIONS_CONSIDERED, ThirdPartyDiscoveryInput,
    build_hook_dispatcher_builder_factory, build_hook_projection_registry,
};
// consumer: root integration harness hook suites · pinned by: `tests/integration/hooks.rs`
pub use observability::trajectory_observer::RebornTrajectoryObserver;
// consumer: `harness/latency/runner`, composition substrate suites · pinned by: `composition/tests/libsql_substrate.rs`
pub use production_runtime_policy::RebornProductionRuntimePolicy;
// consumer: `ironclaw_reborn_cli` serve readiness reporting · pinned by: `composition/tests/profile_acceptance.rs`
pub use readiness::{
    RebornReadiness, RebornReadinessDiagnostic, RebornReadinessDiagnosticComponent,
    RebornReadinessDiagnosticReason, RebornReadinessDiagnosticStatus, RebornReadinessState,
    RebornServiceReadiness, RebornWorkerReadiness,
};
// consumer: `ironclaw_product` test support + root integration harness · pinned by: `composition/tests/product_live_adapters.rs`
// Reached through the `test-support`-featured dev-dependency in `ironclaw_product/Cargo.toml`.
// CHECKLIST WS6 called this block dead and asked for its deletion; it is NOT — deleting
// it strands a sibling crate's test support. See the row's recorded refutation.
#[cfg(any(test, feature = "test-support"))]
pub use root::product_live_adapters::{
    ProductLiveCapabilityAuthorityResolver, ProductLiveCapabilityIo, ProductLiveModelRouteSettings,
    ProductLivePlannedRuntimeAdapterConfig, ProductLivePlannedRuntimeAdapterError,
    ProductLivePlannedRuntimeAdapters, ProductLiveVisibleCapabilityRequestConfig,
    capability_allowlist, visible_capability_request_for_run,
};
// consumer: `ironclaw_reborn_cli` serve + runtime, `harness/latency/runner`, root QA suites · pinned by: `composition/tests/admin_api_e2e.rs` (the parse error is `FromStr::Err`; `root` is private)
pub use root::profile::{RebornCompositionProfile, RebornCompositionProfileParseError};
// consumer: composition + root QA turn-drive suites · pinned by: `composition/tests/runtime.rs`
#[cfg(any(test, feature = "test-support"))]
pub use runtime::RebornTurnDriveOutcome;
// consumer: `ironclaw_reborn_cli` (extension/ironhub/serve/runtime) · pinned by: `composition/tests/runtime.rs`
// Also `harness/latency/runner`, `ironclaw_product` test support, root integration + QA suites.
// The `RebornSkill*` types are `RebornRuntime`'s public skill signatures; `runtime` is private.
pub use runtime::{
    AssistantReply, ConversationId, RebornRuntime, RebornRuntimeError, RebornSkillActivation,
    RebornSkillActivationMode, RebornSkillActivationSource, RebornSkillAsset, RebornSkillBundle,
    RebornSkillExecutionPlan, RebornSkillExecutionResult, build_reborn_runtime, build_runtime,
    product_auth_challenge_provider,
};
// consumer: `ironclaw_reborn_cli` runtime input construction · pinned by: `composition/tests/admin_api_e2e.rs`
// Also `harness/latency/runner`, `ironclaw_product` test support, root integration + QA suites.
// `TriggerFireAccess*` is `TriggerFireAccessPolicy`'s vocabulary; `runtime_input` is private.
pub use runtime_input::{
    KeepaliveSweepSettings, PollSettings, RebornRuntimeIdentity, RebornRuntimeInput,
    TriggerFireAccessCheck, TriggerFireAccessChecker, TriggerFireAccessDecision,
    TriggerFireAccessError, TriggerFireAccessGrant, TriggerFireAccessPolicy, TriggerPollerSettings,
    TurnRunnerSettings,
};

/// Re-exported IronHub command vocabulary for the `ironclaw` binary's
/// `ironhub` subcommand and serve wiring. This facade keeps runtime input
/// construction independent of the manager's wider public surface.
pub mod ironhub {
    pub use ironclaw_extension_manager::ironhub::{
        IronHubCommand, IronHubEntryKind, IronHubInstallOptions, IronHubResponse,
        IronhubManifestUrl, IronhubSharedKey, IronhubSharedKeyError,
        execute_reborn_ironhub_command, render_reborn_ironhub_response, validated_manifest_url,
    };
}

/// Re-exported identity vocabulary host binaries need to construct
/// public runtime/WebUI types whose signatures mention a host-api identity.
/// Kept narrow on purpose — the composition CLAUDE.md says "Expose
/// facade-shaped handles only"; these host-api identity types are the
/// host-identity facade.
pub mod host_api {
    pub use ironclaw_host_api::{
        ids::{AgentId, InvocationId, ProjectId, SecretHandle, TenantId, UserId},
        resource::ResourceScope,
    };
}

/// Canonical Reborn identity resolver vocabulary (issue #4381): the one
/// boundary that maps every external identity — WebUI OAuth logins and
/// external channel/product actors — to a stable `UserId` before runtime
/// state is touched. Only the resolver trait, request, surface, and error
/// types are re-exported so host wiring (`ironclaw-reborn serve`, the CLI
/// `UserDirectory` adapter) depends on the facade vocabulary, never on
/// `ironclaw_reborn_identity` directly. The concrete filesystem-backed store
/// stays private to this composition layer (composition CLAUDE.md: "keep
/// lower substrate handles private").
// consumer: `ironclaw_reborn_cli` user_directory + webui_auth (no `ironclaw_reborn_identity` dep) · pinned by: `composition/tests/production_runtime_identity.rs`
pub use ironclaw_reborn_identity::{
    ExternalSubjectId, ProviderKind, RebornIdentityError, RebornIdentityResolver,
    ResolveExternalIdentity, SurfaceKind,
};

/// Test-support: build a standalone canonical Reborn identity resolver on an
/// in-memory host filesystem under `tenant_id`.
///
/// This mirrors the production path
/// [`RebornRuntime::open_reborn_identity_resolver`](crate::RebornRuntime::open_reborn_identity_resolver),
/// which builds the same filesystem-backed store on the runtime's durable
/// scoped filesystem. Production callers must use that accessor; this free
/// function exists only so tests (and downstream integration crates via
/// `test-support`) can build a resolver without standing up a full runtime.
/// Gated so it ships zero bytes in production binaries.
#[cfg(any(test, feature = "test-support"))]
pub fn open_reborn_identity_resolver(
    tenant_id: &ironclaw_host_api::ids::TenantId,
) -> std::sync::Arc<dyn RebornIdentityResolver> {
    use ironclaw_host_api::{
        ids::{AgentId, UserId},
        mount::{MountGrant, MountPermissions, MountView},
        path::{MountAlias, VirtualPath},
    };

    let root = std::sync::Arc::new(ironclaw_filesystem::InMemoryBackend::default());
    let view = MountView::new(vec![MountGrant::new(
        MountAlias::new("/tenant-shared").expect("mount alias"),
        VirtualPath::new("/tenants/test/shared").expect("virtual path"),
        MountPermissions::read_write_list_delete(),
    )])
    .expect("mount view");
    let filesystem = std::sync::Arc::new(ironclaw_filesystem::ScopedFilesystem::with_fixed_view(
        root, view,
    ));
    std::sync::Arc::new(ironclaw_reborn_identity::RebornIdentityStore::new(
        filesystem,
        tenant_id.clone(),
        UserId::new("test-owner").expect("user"), // safety: test-support-only static valid ID
        AgentId::new("test-agent").expect("agent"), // safety: test-support-only static valid ID
        None,
    ))
}

/// Reborn model purpose slot names exposed for diagnostic callers.
///
/// This keeps CLI diagnostics on the composition boundary instead of making
/// the CLI mirror `ironclaw_loop_host::ModelSlot`.
pub fn reborn_model_slot_names() -> Vec<&'static str> {
    ironclaw_loop_host::ModelSlot::all()
        .iter()
        .map(|slot| slot.as_str())
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RebornRuntimeReadinessSnapshot {
    pub text_only_driver: RebornRuntimeComponentStatus,
    pub planned_driver: RebornRuntimeComponentStatus,
    pub subagent_planned_driver: RebornRuntimeComponentStatus,
    pub planned_default_profile: RebornRuntimeComponentStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RebornRuntimeComponentStatus {
    Initialized,
    Failed(String),
}

impl RebornRuntimeComponentStatus {
    pub fn from_result<T, E: std::fmt::Display>(result: Result<T, E>) -> Self {
        match result {
            Ok(_) => Self::Initialized,
            Err(error) => Self::Failed(error.to_string()),
        }
    }

    pub fn is_initialized(&self) -> bool {
        matches!(self, Self::Initialized)
    }

    pub fn render(&self, ok_label: &str) -> String {
        match self {
            Self::Initialized => ok_label.to_string(),
            Self::Failed(reason) => format!("unavailable: {reason}"),
        }
    }
}

/// Side-effect-free runtime readiness snapshot for diagnostic callers.
pub fn reborn_runtime_readiness_snapshot() -> RebornRuntimeReadinessSnapshot {
    let mut registry = ironclaw_runner::driver_registry::DriverRegistry::new();
    let text_only_driver = RebornRuntimeComponentStatus::from_result(
        ironclaw_runner::planned_driver_factory::register_default_text_only_driver(
            &mut registry,
            ironclaw_runner::text_loop_driver::TextOnlyModelReplyDriverConfig::default(),
        ),
    );
    let family_registry = ironclaw_runner::app_loop_family::build_loop_family_registry();
    let planned_driver = match &family_registry {
        Ok(family_registry) => RebornRuntimeComponentStatus::from_result(
            ironclaw_runner::planned_driver_factory::register_default_planned_driver(
                &mut registry,
                Arc::clone(family_registry),
            ),
        ),
        Err(error) => RebornRuntimeComponentStatus::Failed(error.to_string()),
    };
    let subagent_planned_driver = match family_registry {
        Ok(family_registry) => RebornRuntimeComponentStatus::from_result(
            ironclaw_runner::planned_driver_factory::register_subagent_planned_driver(
                &mut registry,
                family_registry,
            ),
        ),
        Err(error) => RebornRuntimeComponentStatus::Failed(error.to_string()),
    };
    let planned_default_profile = RebornRuntimeComponentStatus::from_result(
        ironclaw_runner::planned_driver_factory::default_planned_run_profile_resolver(),
    );
    RebornRuntimeReadinessSnapshot {
        text_only_driver,
        planned_driver,
        subagent_planned_driver,
        planned_default_profile,
    }
}

use ironclaw_approvals::ApprovalStoreError;
use ironclaw_authorization::CapabilityLeaseError;
use ironclaw_filesystem::LibSqlRootFilesystem;
use ironclaw_filesystem::PostgresRootFilesystem;
use ironclaw_filesystem::{RootFilesystem, ScopedFilesystem};
use ironclaw_host_api::runtime_policy::ProcessBackendKind;
use ironclaw_host_api::{
    mount::{MountGrant, MountPermissions, MountView},
    path::{MountAlias, VirtualPath},
    resource::ResourceScope,
};
use ironclaw_host_runtime::{CapabilitySurfaceVersion, HostRuntimeServices};
use ironclaw_reborn_event_store::RebornEventStoreConfig;
use ironclaw_reborn_event_store::RebornEventStoreError;
use ironclaw_resources::FilesystemResourceGovernor;
use ironclaw_resources::ResourceError;
use ironclaw_secrets::SecretError;
use ironclaw_secrets::SecretMaterial;
use ironclaw_trust::TrustPolicy;
use ironclaw_turns::TurnError;
use ironclaw_turns::TurnRunWakeNotifier;
use thiserror::Error;

pub type LibSqlProductionHostRuntimeServices =
    HostRuntimeServices<LibSqlRootFilesystem, FilesystemResourceGovernor<LibSqlRootFilesystem>>;

pub type PostgresProductionHostRuntimeServices =
    HostRuntimeServices<PostgresRootFilesystem, FilesystemResourceGovernor<PostgresRootFilesystem>>;

/// Consumer-store mount aliases that are tenant-rewritten by
/// [`invocation_mount_view`]. Each alias resolves to
/// `/tenants/<tenant>/users/<user>/<alias>` for the caller's scope, so
/// two tenants sharing one underlying [`RootFilesystem`] cannot collide
/// on identically-shaped paths.
const PER_USER_ALIASES: &[&str] = &[
    "/product-results",
    "/processes",
    "/secrets",
    "/authorization",
    "/outbound",
    "/run-state",
    "/checkpoint-state",
    "/approvals",
    "/gate-records",
    "/replay-payloads",
    "/threads",
    "/conversations",
    "/turns",
    "/resources",
    "/engine",
    "/skills",
    "/workspace",
];

/// The canonical global `/system` subroots, each exposed as its own read-only
/// alias resolving to the same tenant-independent `VirtualPath`. Single source
/// for the mount-grant wiring and its resolution test so the two cannot drift.
const SYSTEM_SUBROOTS: [&str; 3] = ["/system/settings", "/system/extensions", "/system/skills"];

/// Per-invocation [`MountView`] used as the production resolver.
///
/// Every call rebuilds the alias→VirtualPath table for the caller's
/// scope so consumer-store records land under
/// `/tenants/<tenant>/users/<user>/<alias>` virtual paths — cross-tenant
/// isolation is structural rather than a convention. `/tenant-shared`
/// resolves to `/tenants/<tenant>/shared`; `/system/{settings,
/// extensions, skills}` route globally as read-only. See
/// `docs/plans/2026-05-16-scoped-filesystem-tenant-isolation.md`.
///
/// The system sentinel scope (see
/// [`ironclaw_host_api::resource::ResourceScope::system`]) routes records under
/// `/tenants/__system__/users/__system__/<alias>`. Production code uses
/// it for process-global records whose paths already encode per-tenant
/// identity (event-log stream keys, conversation singleton state).
pub fn invocation_mount_view(
    scope: &ResourceScope,
) -> Result<MountView, ironclaw_host_api::error::HostApiError> {
    invocation_mount_view_for_segments(
        resource_scope_path_segment(scope.tenant_id.as_str()),
        resource_scope_path_segment(scope.user_id.as_str()),
    )
}

pub(crate) fn resource_scope_path_segment(value: &str) -> &str {
    if value == ironclaw_host_api::resource::SYSTEM_RESERVED_ID {
        "__system__"
    } else {
        value
    }
}

fn invocation_mount_view_for_segments(
    tenant_id: &str,
    user_id: &str,
) -> Result<MountView, ironclaw_host_api::error::HostApiError> {
    let tenant_user_prefix = format!("/tenants/{tenant_id}/users/{user_id}");
    let mut grants = Vec::with_capacity(PER_USER_ALIASES.len() + 4);
    for alias in PER_USER_ALIASES {
        let target = format!("{tenant_user_prefix}{alias}");
        grants.push(MountGrant::new(
            MountAlias::new(*alias)?,
            VirtualPath::new(target)?,
            MountPermissions::read_write_list_delete(),
        ));
    }
    grants.push(MountGrant::new(
        MountAlias::new("/tenant-shared")?,
        VirtualPath::new(format!("/tenants/{tenant_id}/shared"))?,
        // Broad tenant-shared storage gets read + write + list, but NOT delete.
        // Consumers that own revocable records receive delete authority only
        // through the narrow longest-prefix grants below.
        MountPermissions::read_write(),
    ));
    grants.push(MountGrant::new(
        // Project deletion and membership revocation remove durable records.
        // Keep that authority confined to the project repository subtree.
        MountAlias::new("/tenant-shared/reborn-projects")?,
        VirtualPath::new(format!("/tenants/{tenant_id}/shared/reborn-projects"))?,
        MountPermissions::read_write_list_delete(),
    ));
    grants.push(MountGrant::new(
        // Delete authority is scoped to the identity subtree specifically: the
        // Reborn identity store's admin user-directory needs it for the delete
        // cascade (removing a user's identity / verified-email records) that
        // lives under `/tenant-shared/reborn-identity/…`. Longest-prefix mount
        // matching routes identity paths here and everything else to the
        // delete-less grant above.
        MountAlias::new("/tenant-shared/reborn-identity")?,
        VirtualPath::new(format!("/tenants/{tenant_id}/shared/reborn-identity"))?,
        MountPermissions::read_write_list_delete(),
    ));
    grants.push(MountGrant::new(
        MountAlias::new("/extension-admin-configuration")?,
        VirtualPath::new(format!(
            "/tenants/{tenant_id}/shared/extension-admin-configuration"
        ))?,
        MountPermissions::read_write_list_delete(),
    ));
    for system_subroot in SYSTEM_SUBROOTS {
        grants.push(MountGrant::new(
            MountAlias::new(system_subroot)?,
            VirtualPath::new(system_subroot)?,
            MountPermissions::read_only(),
        ));
    }
    MountView::new(grants)
}

/// Wrap `root` in a tenant-aware [`ScopedFilesystem`] whose resolver is
/// [`invocation_mount_view`]. The returned filesystem is the single
/// production handle — every consumer-store call routes per-scope
/// through this one instance.
pub fn wrap_scoped<F>(root: Arc<F>) -> Arc<ScopedFilesystem<F>>
where
    F: RootFilesystem,
{
    Arc::new(ScopedFilesystem::new(root, invocation_mount_view))
}

/// Process-journal filesystem handle with read-only access to deployed
/// per-user legacy authorities during the explicit one-time migration.
///
/// The extra alias exists only for the system sentinel and only on this
/// process-store-specific handle. Ordinary consumers cannot enumerate another
/// tenant's filesystem tree.
pub(crate) fn wrap_process_journal_scoped<F>(root: Arc<F>) -> Arc<ScopedFilesystem<F>>
where
    F: RootFilesystem,
{
    Arc::new(ScopedFilesystem::new(root, |scope| {
        let mut view = invocation_mount_view(scope)?;
        if scope.is_system() {
            view.mounts.push(MountGrant::new(
                MountAlias::new("/legacy-tenants")?,
                VirtualPath::new("/tenants")?,
                MountPermissions::read_only(),
            ));
            view.validate()?;
        }
        Ok(view)
    }))
}

/// libSQL substrate handles needed to build production host-runtime services.
///
/// State, event, and audit persistence all use `runtime`. A second libSQL
/// connection target is deliberately not configurable here: one physical
/// database has one composition-owned runtime and one writer admission lane.
pub struct LibSqlProductionSubstrateConfig<TPolicy, TWake>
where
    TPolicy: TrustPolicy + 'static,
    TWake: TurnRunWakeNotifier + 'static,
{
    pub runtime: Arc<ironclaw_libsql_runtime::LibSqlRuntime>,
    /// The exact target from which `runtime` was opened through
    /// [`ironclaw_libsql_runtime::LibSqlRuntime::open`]. Caller-supplied
    /// database handles do not carry this provenance and are rejected.
    ///
    /// Retained only for production durability and transport-policy
    /// validation; it is never reopened by the event store.
    pub database_path_or_url: String,
    /// Set this only when deployment guarantees exactly one runtime process, or
    /// one elected runtime owner, is allowed to enforce resource quotas for this
    /// database. The filesystem governor keeps in-process tallies as authority.
    pub process_local_resource_governor_singleton: bool,
    pub secret_master_key: Option<SecretMaterial>,
    pub trust_policy: Arc<TPolicy>,
    pub runtime_policy: RebornProductionRuntimePolicy,
    pub turn_run_wake_notifier: Arc<TWake>,
    pub surface_version: CapabilitySurfaceVersion,
}

/// PostgreSQL substrate handles needed to build production host-runtime services.
pub struct PostgresProductionSubstrateConfig<TPolicy, TWake>
where
    TPolicy: TrustPolicy + 'static,
    TWake: TurnRunWakeNotifier + 'static,
{
    pub pool: deadpool_postgres::Pool,
    pub event_store: RebornEventStoreConfig,
    /// Set this only when deployment guarantees exactly one runtime process, or
    /// one elected runtime owner, is allowed to enforce resource quotas for this
    /// database. The filesystem governor keeps in-process tallies as authority.
    pub process_local_resource_governor_singleton: bool,
    pub secret_master_key: Option<SecretMaterial>,
    pub trust_policy: Arc<TPolicy>,
    pub runtime_policy: RebornProductionRuntimePolicy,
    pub turn_run_wake_notifier: Arc<TWake>,
    pub surface_version: CapabilitySurfaceVersion,
}

#[derive(Debug, Error)]
pub enum RebornCompositionError {
    #[error("invalid reborn production configuration: {reason}")]
    InvalidConfig { reason: String },
    #[error(
        "reborn production composition requires a configured or keychain-resolvable secret master key"
    )]
    MissingSecretMasterKey,
    #[error("reborn mount view construction failed: {0}")]
    Mount(#[from] ironclaw_host_api::error::HostApiError),
    #[error("reborn filesystem substrate failed: {0}")]
    Filesystem(#[from] ironclaw_filesystem::FilesystemError),
    #[error("reborn resource governor substrate failed: {0}")]
    Resource(#[from] ResourceError),
    #[error("reborn approval store substrate failed: {0}")]
    ApprovalStore(#[from] ApprovalStoreError),
    #[error("reborn capability lease substrate failed: {0}")]
    CapabilityLease(#[from] CapabilityLeaseError),
    #[error("reborn secret substrate failed: {0}")]
    Secret(#[from] SecretError),
    #[error("reborn event store substrate failed: {0}")]
    EventStore(#[from] RebornEventStoreError),
    #[error("reborn turn substrate failed: {0}")]
    Turn(#[from] TurnError),
    #[error("reborn run-profile resolver substrate failed: {0}")]
    RunProfile(#[from] ironclaw_loop_contracts::RunProfileRegistryError),
    #[error("production user-sandbox process backend requires a user sandbox process binding")]
    MissingUserSandboxProcessPort,
    #[error(
        "production runtime policy uses {process_backend:?} but a user sandbox process binding was supplied"
    )]
    UnexpectedUserSandboxProcessPort { process_backend: ProcessBackendKind },
    #[error("reborn production wiring failed: {report:?}")]
    ProductionWiring {
        report: ironclaw_host_runtime::ProductionWiringReport,
    },
}

/// Build production-wired host-runtime services over libSQL-backed substrates.
///
/// This is deliberately substrate-only: no app/web setup, no runtime adapter
/// registration, and no product loop construction.
///
/// Initialization runs substrate migrations and secret decryptability checks
/// sequentially against the shared database. Earlier successful migrations are
/// not rolled back if a later substrate fails; each migration is expected to be
/// idempotent so callers can fix the underlying failure and retry composition.
pub async fn build_libsql_production_host_runtime_services<TPolicy, TWake>(
    config: LibSqlProductionSubstrateConfig<TPolicy, TWake>,
) -> Result<LibSqlProductionHostRuntimeServices, RebornCompositionError>
where
    TPolicy: TrustPolicy + 'static,
    TWake: TurnRunWakeNotifier + 'static,
{
    factory::build_libsql_production_host_runtime_services(config).await
}

/// Build production-wired host-runtime services over PostgreSQL-backed substrates.
///
/// Initialization runs substrate migrations and secret decryptability checks
/// sequentially against the shared database. Earlier successful migrations are
/// not rolled back if a later substrate fails; each migration is expected to be
/// idempotent so callers can fix the underlying failure and retry composition.
pub async fn build_postgres_production_host_runtime_services<TPolicy, TWake>(
    config: PostgresProductionSubstrateConfig<TPolicy, TWake>,
) -> Result<PostgresProductionHostRuntimeServices, RebornCompositionError>
where
    TPolicy: TrustPolicy + 'static,
    TWake: TurnRunWakeNotifier + 'static,
{
    factory::build_postgres_production_host_runtime_services(config).await
}

#[cfg(test)]
mod mount_view_tests {
    use super::*;
    use ironclaw_filesystem::{FilesystemError, FilesystemOperation, InMemoryBackend};
    use ironclaw_host_api::{
        ids::{AgentId, InvocationId, MissionId, ProjectId, TenantId, ThreadId, UserId},
        path::ScopedPath,
    };

    fn sample_scope() -> ResourceScope {
        ResourceScope {
            tenant_id: TenantId::new("tenant-a").unwrap(),
            user_id: UserId::new("user-1").unwrap(),
            agent_id: Some(AgentId::new("agent-x").unwrap()),
            project_id: Some(ProjectId::new("project-y").unwrap()),
            mission_id: Some(MissionId::new("mission-w").unwrap()),
            thread_id: Some(ThreadId::new("thread-z").unwrap()),
            invocation_id: InvocationId::new(),
        }
    }

    fn other_tenant_scope() -> ResourceScope {
        ResourceScope {
            tenant_id: TenantId::new("tenant-b").unwrap(),
            ..sample_scope()
        }
    }

    #[test]
    fn invocation_mount_view_rewrites_per_user_aliases_to_tenant_user_paths() {
        let scope = sample_scope();
        let view = invocation_mount_view(&scope).unwrap();
        for alias in PER_USER_ALIASES {
            let resolved = view
                .resolve(&ScopedPath::new(format!("{alias}/foo")).unwrap())
                .unwrap();
            assert_eq!(
                resolved.as_str(),
                &format!(
                    "/tenants/{}/users/{}{alias}/foo",
                    scope.tenant_id.as_str(),
                    scope.user_id.as_str()
                )
            );
        }
    }

    #[tokio::test]
    async fn process_journal_migration_mount_is_system_only_and_read_only() {
        let root = Arc::new(InMemoryBackend::new());
        let scoped = wrap_process_journal_scoped(root);
        let legacy = ScopedPath::new("/legacy-tenants/tenant-a/users/user-a/run-state")
            .expect("legacy path");
        assert!(
            scoped.resolve(&sample_scope(), &legacy).is_err(),
            "ordinary user scopes must not enumerate other tenant roots"
        );
        assert_eq!(
            scoped
                .resolve(&ResourceScope::system(), &legacy)
                .expect("system migration mount")
                .as_str(),
            "/tenants/tenant-a/users/user-a/run-state"
        );
        assert!(matches!(
            scoped
                .write_bytes(&ResourceScope::system(), &legacy, b"forbidden".to_vec())
                .await
                .expect_err("migration mount must not mutate legacy authorities"),
            FilesystemError::PermissionDenied { .. }
        ));
    }

    #[test]
    fn invocation_mount_view_isolates_tenants_with_same_user() {
        let view_a = invocation_mount_view(&sample_scope()).unwrap();
        let view_b = invocation_mount_view(&other_tenant_scope()).unwrap();
        let path = ScopedPath::new("/engine/threads/x").unwrap();
        let a = view_a.resolve(&path).unwrap();
        let b = view_b.resolve(&path).unwrap();
        assert_ne!(a.as_str(), b.as_str());
        assert!(a.as_str().contains("tenant-a"));
        assert!(b.as_str().contains("tenant-b"));
    }

    #[test]
    fn invocation_mount_view_routes_tenant_shared_to_tenant_root() {
        let scope = sample_scope();
        let view = invocation_mount_view(&scope).unwrap();
        let resolved = view
            .resolve(&ScopedPath::new("/tenant-shared/foo").unwrap())
            .unwrap();
        assert_eq!(
            resolved.as_str(),
            &format!("/tenants/{}/shared/foo", scope.tenant_id.as_str())
        );
    }

    #[tokio::test]
    async fn invocation_mount_view_limits_tenant_shared_delete_to_owned_subtrees() {
        let scope = sample_scope();
        let view = invocation_mount_view(&scope).unwrap();
        let broad_scoped_path = ScopedPath::new("/tenant-shared/other/state.json").unwrap();
        let (_, broad_grant) = view.resolve_with_grant(&broad_scoped_path).unwrap();
        let project_scoped_path =
            ScopedPath::new("/tenant-shared/reborn-projects/tenant-a/record.json").unwrap();
        let (project_path, project_grant) = view.resolve_with_grant(&project_scoped_path).unwrap();

        assert!(!broad_grant.permissions.delete);
        assert!(project_grant.permissions.delete);
        assert_eq!(
            project_path.as_str(),
            "/tenants/tenant-a/shared/reborn-projects/tenant-a/record.json"
        );

        let scoped = wrap_scoped(Arc::new(InMemoryBackend::new()));
        scoped
            .write_bytes(&scope, &broad_scoped_path, b"shared".to_vec())
            .await
            .unwrap();
        assert!(matches!(
            scoped
                .delete(&scope, &broad_scoped_path)
                .await
                .expect_err("broad tenant-shared grant must deny delete"),
            FilesystemError::PermissionDenied {
                operation: FilesystemOperation::Delete,
                ..
            }
        ));
        assert!(
            scoped
                .get(&scope, &broad_scoped_path)
                .await
                .unwrap()
                .is_some()
        );

        scoped
            .write_bytes(&scope, &project_scoped_path, b"project".to_vec())
            .await
            .unwrap();
        scoped.delete(&scope, &project_scoped_path).await.unwrap();
        assert!(
            scoped
                .get(&scope, &project_scoped_path)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn invocation_mount_view_routes_admin_configuration_to_tenant_shared_storage() {
        let scope = sample_scope();
        let view = invocation_mount_view(&scope).unwrap();
        let resolved = view
            .resolve(
                &ScopedPath::new("/extension-admin-configuration/groups/extension.slack.json")
                    .unwrap(),
            )
            .unwrap();
        assert_eq!(
            resolved.as_str(),
            &format!(
                "/tenants/{}/shared/extension-admin-configuration/groups/extension.slack.json",
                scope.tenant_id.as_str(),
            ),
        );
    }

    #[test]
    fn invocation_mount_view_sanitizes_system_scope_segments() {
        let view = invocation_mount_view(&ResourceScope::system()).unwrap();
        let resolved = view
            .resolve(&ScopedPath::new("/turns/state.json").unwrap())
            .unwrap();
        assert_eq!(
            resolved.as_str(),
            "/tenants/__system__/users/__system__/turns/state.json"
        );
    }

    #[test]
    fn invocation_mount_view_routes_system_globally() {
        let scope = sample_scope();
        let view = invocation_mount_view(&scope).unwrap();
        // Each canonical /system subroot is exposed as its own
        // read-only alias and resolves to the same VirtualPath
        // regardless of tenant — system data is global, not
        // per-tenant.
        for system_subroot in SYSTEM_SUBROOTS {
            let resolved = view
                .resolve(&ScopedPath::new(format!("{system_subroot}/foo")).unwrap())
                .unwrap();
            assert_eq!(resolved.as_str(), &format!("{system_subroot}/foo"));
        }
    }

    #[test]
    fn invocation_mount_view_routes_user_skills_to_tenant_user_root() {
        let scope = sample_scope();
        let view = invocation_mount_view(&scope).unwrap();
        let (resolved, grant) = view
            .resolve_with_grant(&ScopedPath::new("/skills/code-review/SKILL.md").unwrap())
            .unwrap();
        assert_eq!(
            resolved.as_str(),
            &format!(
                "/tenants/{}/users/{}/skills/code-review/SKILL.md",
                scope.tenant_id.as_str(),
                scope.user_id.as_str()
            )
        );
        assert!(grant.permissions.read);
        assert!(grant.permissions.write);
        assert!(grant.permissions.list);
        assert!(grant.permissions.delete);
        assert!(!grant.permissions.execute);
    }

    #[test]
    fn invocation_mount_view_keeps_user_skills_isolated_from_system_skills() {
        let scope = sample_scope();
        let view = invocation_mount_view(&scope).unwrap();
        let user_skill = view
            .resolve(&ScopedPath::new("/skills/code-review/SKILL.md").unwrap())
            .unwrap();
        let system_skill = view
            .resolve(&ScopedPath::new("/system/skills/code-review/SKILL.md").unwrap())
            .unwrap();
        assert_ne!(user_skill.as_str(), system_skill.as_str());
        assert!(
            user_skill
                .as_str()
                .starts_with("/tenants/tenant-a/users/user-1/skills/")
        );
        assert_eq!(system_skill.as_str(), "/system/skills/code-review/SKILL.md");
    }

    #[test]
    fn invocation_mount_view_isolates_user_skills_between_tenants() {
        let view_a = invocation_mount_view(&sample_scope()).unwrap();
        let view_b = invocation_mount_view(&other_tenant_scope()).unwrap();
        let path = ScopedPath::new("/skills/code-review/SKILL.md").unwrap();
        let a = view_a.resolve(&path).unwrap();
        let b = view_b.resolve(&path).unwrap();
        assert_ne!(a.as_str(), b.as_str());
        assert!(a.as_str().contains("tenant-a"));
        assert!(b.as_str().contains("tenant-b"));
    }

    #[tokio::test]
    async fn scoped_filesystem_rejects_system_skill_writes_but_allows_user_skill_writes() {
        let root = Arc::new(InMemoryBackend::default());
        let scoped = wrap_scoped(root);
        let scope = sample_scope();
        let system_path = ScopedPath::new("/system/skills/code-review/SKILL.md").unwrap();
        let user_path = ScopedPath::new("/skills/code-review/SKILL.md").unwrap();

        let error = scoped
            .write_bytes(&scope, &system_path, b"system skill".to_vec())
            .await
            .expect_err("system skills must remain read-only");
        assert!(matches!(
            error,
            FilesystemError::PermissionDenied {
                operation: FilesystemOperation::WriteFile,
                ..
            }
        ));

        scoped
            .write_bytes(&scope, &user_path, b"user skill".to_vec())
            .await
            .expect("user skills should be writable through the scoped alias");
        let content = scoped
            .read_bytes(&scope, &user_path)
            .await
            .expect("user skill should be readable");
        assert_eq!(content, b"user skill");
    }
}

#[cfg(test)]
mod two_tenant_isolation_tests {
    //! Regression test for the cross-tenant collision finding from the
    //! 2026-05-17 serrrfirat review.
    //!
    //! Drives the public `SecretStorePort` surface from two distinct
    //! `(tenant, user)` scopes that share identical agent/project/handle,
    //! against the production-shape `wrap_scoped`/`invocation_mount_view`
    //! wiring over an `InMemoryBackend`. Without per-tenant path
    //! rewriting both `put`s would land at the same backend row;
    //! Alice's `consume` would then decrypt to Bob's ciphertext (or
    //! fail with DecryptionFailed via AAD mismatch). The resolver in
    //! place gives each tenant their own subtree — both reads succeed
    //! with their own plaintext.
    //!
    //! A regression that puts the old singleton (identity-mapping)
    //! resolver back into production wiring trips this test directly.
    use super::*;
    use ironclaw_filesystem::InMemoryBackend;
    use ironclaw_host_api::ids::{
        AgentId, InvocationId, ProjectId, SecretHandle, TenantId, UserId,
    };
    use ironclaw_secrets::{SecretMaterial, SecretStore, SecretStorePort, SecretsCrypto};
    use secrecy::ExposeSecret;

    fn scope(tenant: &str, user: &str) -> ResourceScope {
        ResourceScope {
            tenant_id: TenantId::new(tenant).unwrap(), // safety: fixed-valid test fixture
            user_id: UserId::new(user).unwrap(),       // safety: fixed-valid test fixture
            agent_id: Some(AgentId::new("github").unwrap()),
            project_id: Some(ProjectId::new("default").unwrap()), // safety: fixed-valid test fixture
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        }
    }

    fn test_crypto() -> Arc<SecretsCrypto> {
        Arc::new(
            SecretsCrypto::new(SecretMaterial::from(
                "test-master-key-32-bytes-aaaaaaaaa".to_string(),
            ))
            .expect("crypto"),
        )
    }

    #[tokio::test]
    async fn two_tenants_with_same_agent_project_handle_do_not_collide_on_put() {
        let backend = Arc::new(InMemoryBackend::new());
        let scoped = wrap_scoped(Arc::clone(&backend));
        let store = SecretStore::new(Arc::clone(&scoped), test_crypto());

        let handle = SecretHandle::new("oauth_token").unwrap();
        let scope_a = scope("tenant_a", "alice");
        let scope_b = scope("tenant_b", "bob");

        store
            .put(
                scope_a.clone(),
                handle.clone(),
                SecretMaterial::from("alice-secret".to_string()),
                None,
            )
            .await
            .unwrap();
        store
            .put(
                scope_b.clone(),
                handle.clone(),
                SecretMaterial::from("bob-secret".to_string()),
                None,
            )
            .await
            .unwrap();

        let lease_a = store.lease_once(&scope_a, &handle).await.unwrap();
        let material_a = store.consume(&scope_a, lease_a.id).await.unwrap();
        assert_eq!(material_a.expose_secret(), "alice-secret");

        let lease_b = store.lease_once(&scope_b, &handle).await.unwrap();
        let material_b = store.consume(&scope_b, lease_b.id).await.unwrap();
        assert_eq!(material_b.expose_secret(), "bob-secret");
    }
}

#[cfg(test)]
mod gate_record_production_mount_tests {
    //! Production-shape mount coverage for the `/gate-records` alias: drives the
    //! `GateRecordStorePort` seam over the real `wrap_scoped`/`invocation_mount_view`
    //! wiring. Pins two things: the alias is actually registered in
    //! [`PER_USER_ALIASES`] (an unregistered alias fails every save with
    //! `MountNotFound`, making the store unusable in production), and the
    //! per-tenant path rewriting keeps identically-shaped refs from colliding
    //! across tenants.
    use super::*;
    use ironclaw_approvals::{GateRecordStore, GateRecordStorePort};
    use ironclaw_filesystem::InMemoryBackend;
    use ironclaw_host_api::{
        gate_record::GateRecord,
        ids::{GateRef, InvocationId, ProjectId, TenantId, UserId},
        safe_summary::SafeSummary,
    };

    fn scope(tenant: &str, user: &str) -> ResourceScope {
        ResourceScope {
            tenant_id: TenantId::new(tenant).unwrap(), // safety: fixed-valid test fixture
            user_id: UserId::new(user).unwrap(),       // safety: fixed-valid test fixture
            agent_id: None,
            project_id: Some(ProjectId::new("default").unwrap()), // safety: fixed-valid test fixture
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        }
    }

    #[tokio::test]
    async fn gate_records_save_and_load_through_the_production_mount_view() {
        let scoped = wrap_scoped(Arc::new(InMemoryBackend::new()));
        let store = GateRecordStore::new(scoped);
        let record = GateRecord::Approval {
            summary: SafeSummary::new("awaiting decision").unwrap(), // safety: fixed-valid test fixture
        };
        let gate_ref = GateRef::new();
        let scope_a = scope("tenant_a", "alice");

        // The alias must resolve (a missing PER_USER_ALIASES entry fails here
        // with MountNotFound), and the owner must read the record back.
        store
            .save(scope_a.clone(), gate_ref, record.clone())
            .await
            .unwrap(); // safety: test assertion on an in-memory store
        assert_eq!(store.load(&scope_a, gate_ref).await.unwrap(), Some(record)); // safety: test assertion

        // Structural tenant isolation: same ref, different tenant → unknown.
        let scope_b = scope("tenant_b", "bob");
        assert_eq!(store.load(&scope_b, gate_ref).await.unwrap(), None); // safety: test assertion
    }
}
