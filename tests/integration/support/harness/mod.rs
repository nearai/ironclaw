//! Reborn binary-E2E harness.
//!
//! This harness drives the product caller path used by the #3702 validation
//! ports:
//!
//! inbound bytes -> ProductAdapter -> DefaultProductWorkflow ->
//! DefaultInboundTurnService -> DefaultTurnCoordinator -> TurnRunScheduler ->
//! Reborn planned agent loop -> model/capability/transcript evidence.
//!
//! Documented test-support substitutions:
//! - the model gateway is scripted trace replay;
//! - the capability port is a local recording echo/approval port;
//! - external internet, delivery, and OAuth are not exercised by this harness.

#![allow(dead_code)] // Shared by staged Reborn binary-E2E validation ports.

// arch-exempt: large_file, Reborn binary-E2E + host-runtime capability harness; the
// mock-MCP scaffolding has been split into `harness_mcp.rs`, further focused splits
// (auth, hooks) are tracked in `tests/support/reborn/CLAUDE.md`.

pub(crate) mod assembly;
pub(crate) mod options;
pub(crate) mod recorder;

pub(crate) use options::HostRuntimeHarnessOptions;
pub(crate) use recorder::{HarnessCapabilityRecorder, RecordedCapabilityResult};

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use super::{
    extension_surface::{BUNDLED_EXTENSION_CAPABILITY_IDS, EXTENSION_LIFECYCLE_CAPABILITY_IDS},
    filesystem::BlockingTurnStatePutFilesystem,
    github as github_support,
    harness_mcp::{
        build_loopback_mcp_runtime, local_dev_host_runtime_with_registry_egress_and_mcp,
        mcp_loopback_network_policy, mock_mcp_extension_package,
    },
    harness_web_access,
    product_workflow::resource_scope,
};
use ironclaw_approvals::{ApprovalResolver, AutoApproveSettingInput, DenyApproval, LeaseApproval};
use ironclaw_auth::{
    AuthProductScope, AuthProviderId, AuthSurface, CredentialAccountLabel, CredentialAccountStatus,
    CredentialOwnership, NewCredentialAccount, ProviderScope,
};
use ironclaw_extensions::ExtensionRegistry;
use ironclaw_filesystem::{
    BackendKind, CompositeRootFilesystem, ContentKind, InMemoryBackend, IndexPolicy,
    RootFilesystem, ScopedFilesystem, StorageClass,
};
use ironclaw_first_party_extensions::{WEB_GET_CONTENT_CAPABILITY_ID, WEB_SEARCH_CAPABILITY_ID};
use ironclaw_host_api::{
    Action, AgentId, ApprovalRequestId, CapabilityId, CredentialStageError, EffectKind,
    ExtensionId, GrantConstraints, InvocationId, MountAlias, MountGrant, MountPermissions,
    MountView, NetworkPolicy, Principal, ProjectId, ResourceScope, RuntimeHttpEgressRequest,
    RuntimeKind, SecretHandle, TenantId, UserId, VirtualPath,
};
use ironclaw_host_runtime::{
    APPLY_PATCH_CAPABILITY_ID, BUILTIN_FIRST_PARTY_PROVIDER, ECHO_CAPABILITY_ID,
    GLOB_CAPABILITY_ID, GREP_CAPABILITY_ID, HTTP_CAPABILITY_ID, HTTP_SAVE_CAPABILITY_ID,
    HostRuntime, JSON_CAPABILITY_ID, LIST_DIR_CAPABILITY_ID, MEMORY_READ_CAPABILITY_ID,
    MEMORY_SEARCH_CAPABILITY_ID, MEMORY_TREE_CAPABILITY_ID, MEMORY_WRITE_CAPABILITY_ID,
    PROFILE_SET_CAPABILITY_ID, READ_FILE_CAPABILITY_ID, RuntimeProcessPort, SHELL_CAPABILITY_ID,
    SKILL_INSTALL_CAPABILITY_ID, SKILL_LIST_CAPABILITY_ID, SKILL_REMOVE_CAPABILITY_ID,
    SPAWN_SUBAGENT_CAPABILITY_ID, TIME_CAPABILITY_ID, TRACE_COMMONS_CREDITS_CAPABILITY_ID,
    TRACE_COMMONS_ONBOARD_CAPABILITY_ID, TRACE_COMMONS_PROFILE_SET_CAPABILITY_ID,
    TRACE_COMMONS_PROFILE_TOKEN_CAPABILITY_ID, TRACE_COMMONS_STATUS_CAPABILITY_ID,
    TRIGGER_CREATE_CAPABILITY_ID, TRIGGER_LIST_CAPABILITY_ID, TRIGGER_PAUSE_CAPABILITY_ID,
    TRIGGER_REMOVE_CAPABILITY_ID, TRIGGER_RESUME_CAPABILITY_ID, WRITE_FILE_CAPABILITY_ID,
};
use ironclaw_loop_support::{
    CapabilityAllowSet, CapabilitySurfaceProfileResolver, LoopCapabilityPortFactory,
    LoopCapabilityResultWriter,
};
use ironclaw_network::{NetworkHttpEgress, NetworkHttpRequest};
use ironclaw_product_workflow::{ProjectService, ResolvedBinding};
use ironclaw_reborn_composition::test_support::SkillActivationTestSource;
use ironclaw_reborn_composition::{
    ProductLiveCapabilityIo, RebornBuildInput, RebornLocalDevApprovalTestParts,
    RebornProductAuthServices, build_reborn_services,
};
use ironclaw_turns::{
    GateRef,
    run_profile::{
        AgentLoopHostError, AgentLoopHostErrorKind, CapabilityInvocation, LoopCapabilityPort,
        LoopRunContext,
    },
};

pub(crate) use super::doubles::{
    EmptyIdentityContextSource, HarnessCapabilityPortFactory,
    HostRuntimeHarnessCapabilityPortFactory, RecordingApprovalRequestStore,
    RecordingCapabilityResultWriter, RecordingHostRuntime, RecordingNetworkHttpEgress,
    RecordingRuntimeHttpEgress, RecordingTestCapabilityPort,
    StaticCapabilitySurfaceProfileResolver,
};
pub(crate) use assembly::{
    LocalDevRootMounts, bundled_extension_provider_trust, capability_ids_from_strs,
    copy_dir_recursive, host_runtime_storage_roots, http_test_policy, local_dev_all_effects,
    local_dev_host_runtime_with_http_egress, local_dev_host_runtime_with_live_http_egress,
    local_dev_host_runtime_with_registry_and_egress, local_dev_mount_descriptor,
    local_dev_root_filesystem, memory_mounts, qa_smoke_mounts, skill_mounts, wildcard_test_policy,
    workspace_mounts,
};

pub(crate) type HarnessResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;
pub(crate) type HarnessCapabilityParts = (
    Arc<dyn LoopCapabilityPortFactory>,
    Arc<dyn CapabilitySurfaceProfileResolver>,
    Arc<dyn ironclaw_loop_support::LoopCapabilityInputResolver>,
    Arc<dyn LoopCapabilityResultWriter>,
    HarnessCapabilityRecorder,
);
pub(crate) type HarnessTurnStorageBackend = BlockingTurnStatePutFilesystem<InMemoryBackend>;
pub(crate) type HarnessTurnBackend = CompositeRootFilesystem;

pub(crate) enum HarnessCapabilityMode {
    Recording(RecordingTestCapabilityPort),
    HostRuntime(Arc<HostRuntimeCapabilityHarness>),
}

impl HarnessCapabilityMode {
    pub(crate) fn into_parts(
        self,
        milestone_sink: Arc<ironclaw_turns::run_profile::InMemoryLoopHostMilestoneSink>,
    ) -> HarnessResult<HarnessCapabilityParts> {
        match self {
            Self::Recording(port) => {
                let port = Arc::new(port);
                let capability_io = Arc::new(ProductLiveCapabilityIo::default());
                Ok((
                    Arc::new(HarnessCapabilityPortFactory {
                        port: Arc::clone(&port),
                    }),
                    Arc::new(StaticCapabilitySurfaceProfileResolver {
                        allow_set: CapabilityAllowSet::allowlist(port.capability_allowlist()),
                    }),
                    capability_io.clone(),
                    capability_io,
                    HarnessCapabilityRecorder::Recording(port),
                ))
            }
            Self::HostRuntime(harness) => Ok((
                harness.capability_factory(milestone_sink),
                Arc::new(StaticCapabilitySurfaceProfileResolver {
                    allow_set: CapabilityAllowSet::allowlist(harness.capability_ids.clone()),
                }),
                harness.io.clone(),
                harness.capability_result_writer(),
                HarnessCapabilityRecorder::HostRuntime(harness),
            )),
        }
    }
}

/// Backing handles for the two synthetic `outbound_delivery_*` capabilities
/// (C-SYNTH outbound seam). `Some` only for `outbound_target_tools()`. Bundles
/// the injected facade double + the settings stores the production
/// `outbound_delivery_capabilities` wiring consumes, so the harness struct
/// widens by ONE field instead of four. The auto-approve store and
/// approval-request/lease stores are already held as sibling harness fields
/// (`auto_approve_settings` / `approval_parts`) and re-used, not duplicated here.
struct OutboundTargetToolsParts {
    /// Concrete double (not the trait object) so tests can read `set` calls back;
    /// upcast to `Arc<dyn OutboundPreferencesProductFacade>` at wrap time.
    facade: Arc<super::outbound_preferences::FakeOutboundPreferencesFacade>,
    requires_approval: bool,
    tool_permission_overrides: Arc<dyn ironclaw_approvals::ToolPermissionOverrideStore>,
    persistent_approval_policies: Arc<dyn ironclaw_approvals::PersistentApprovalPolicyStore>,
}

pub(crate) struct HostRuntimeCapabilityHarness {
    pub(crate) runtime: Arc<dyn HostRuntime>,
    approval_parts: Option<RebornLocalDevApprovalTestParts>,
    auto_approve_settings: Option<Arc<dyn ironclaw_approvals::AutoApproveSettingStore>>,
    pending_approval_scopes: Arc<Mutex<HashMap<ApprovalRequestId, ResourceScope>>>,
    pub(crate) io: Arc<ProductLiveCapabilityIo>,
    root: Arc<tempfile::TempDir>,
    workspace_root: PathBuf,
    pub(crate) mounts: MountView,
    pub(crate) capability_mount_overrides: Vec<(CapabilityId, MountView)>,
    pub(crate) capability_ids: Vec<CapabilityId>,
    pub(crate) runtime_kind: RuntimeKind,
    pub(crate) effect_kinds: Vec<EffectKind>,
    pub(crate) network_policy: NetworkPolicy,
    pub(crate) secrets: Vec<SecretHandle>,
    pub(crate) provider_id: ExtensionId,
    pub(crate) additional_provider_trust: Vec<(ExtensionId, Vec<EffectKind>)>,
    user_id: UserId,
    pub(crate) invocations: Arc<Mutex<Vec<CapabilityInvocation>>>,
    pub(crate) results: Arc<Mutex<Vec<RecordedCapabilityResult>>>,
    http_egress: Option<Arc<RecordingRuntimeHttpEgress>>,
    network_egress: Option<Arc<RecordingNetworkHttpEgress>>,
    /// Inert recording process port (slice 5). `Some` when the harness injected
    /// a `RecordingProcessPort`; `None` when the live `LocalHostProcessPort` was
    /// used (`.with_live_shell()` path) or the harness predates slice 5.
    process_port: Option<Arc<super::process::RecordingProcessPort>>,
    /// Raw local-dev memory filesystem backing the user-profile source
    /// (E-PROFILE seam). `Some` only for `new_with_options`-built harnesses (which
    /// flow through `RebornServices`); `None` for the lower-level constructors and
    /// the Echo backend. Read back via `profile_filesystem_for_test`.
    profile_filesystem: Option<Arc<dyn RootFilesystem>>,
    /// Project service for the local-dev synthetic `project_create` capability
    /// (E-PROJ seam). `Some` for every `new_with_options`-built local-dev harness;
    /// `None` for the lower-level constructors and the Echo backend.
    /// `create_capability_port` wraps the port with the synthetic project-create
    /// capability ONLY when `PROJECT_CREATE_CAPABILITY_ID` is also in
    /// `capability_ids` (i.e. only `project_tools()` surfaces it), so other
    /// local-dev groups are unaffected. Tests read projects back via `project_service`.
    project_service: Option<Arc<dyn ProjectService>>,
    /// Local-dev skill context source for the synthetic `skill_activate`
    /// capability and runtime prompt injection (E-SKILL seam). `Some` only for
    /// `skill_activation_tools()`; `None` otherwise. `create_capability_port`
    /// wraps the port with the synthetic `skill_activate` capability ONLY when
    /// `SKILL_ACTIVATE_CAPABILITY_ID` is also in `capability_ids`, and its
    /// `context_source()` is wired as the runtime's `skill_context_source` in
    /// `into_group`. Held as the opaque test-support handle so this crate never
    /// names the crate-private source type.
    skill_activation_source: Option<SkillActivationTestSource>,
    /// Attachment read port + inbound lander backing the C-ATTACH seam. `Some`
    /// only for `new_with_options`-built harnesses (which flow through
    /// `RebornServices`, and thus have a local-dev workspace filesystem to build
    /// both over); `None` for the lower-level constructors and the Echo backend.
    /// Read back via `attachment_test_support_for_test`.
    attachment_test_support: Option<ironclaw_reborn_composition::AttachmentTestSupport>,
    /// Backing handles for the synthetic `outbound_delivery_*` capabilities
    /// (C-SYNTH outbound seam). `Some` only for `outbound_target_tools()`;
    /// `create_capability_port` wraps the port with the two capabilities via
    /// `apply_synthetic_capability_wrappers` when this is `Some`.
    outbound_target_tools: Option<OutboundTargetToolsParts>,
    /// C-MULTIUSER seam: when `true`, [`create_capability_port`] resolves the
    /// capability-execution user from the RUN's owner/actor (mirroring
    /// production `local_dev_visible_capability_request`,
    /// `crates/ironclaw_reborn_composition/src/runtime/local_dev.rs`) instead of
    /// this harness's single fixed `user_id`. That is what lets two distinct
    /// actors dispatching over the group's ONE shared capability backend run
    /// under DISTINCT `(tenant, user)` scopes, so memory, auto-approve, and
    /// approval-settings isolate per actor — the real production behavior.
    /// Defaults `false` so every existing fixed-user harness is byte-identical;
    /// only the multiuser group constructors flip it on via
    /// [`with_run_owner_scoped_capability_dispatch`].
    scope_capability_by_run_owner: bool,
    /// Local-dev product-auth services (C-JOURNEY convergence seam). `Some`
    /// only for `new_with_options`-built harnesses (which flow through
    /// `RebornServices`); `None` for the lower-level constructors and the Echo
    /// backend. `seed_github_credential_account` reads this to create a real
    /// credential account through `credential_account_service()`, letting a
    /// parked `github.*` auth gate's `ProductAuthRuntimeCredentialResolver`
    /// lookup resolve on re-dispatch.
    product_auth: Option<Arc<RebornProductAuthServices>>,
    /// W4-ASK-EACH-ONCE: local-dev per-tool permission override store (mirrors
    /// `auto_approve_settings`). `Some` only for `new_with_options`-built
    /// harnesses (which flow through `RebornServices`); `None` for the
    /// lower-level constructors and the Echo backend. Lets a test install a
    /// dynamic `ToolPermissionOverride::AskEachTime` override on any capability
    /// via `set_ask_each_time_override_for_test`, independent of the
    /// `outbound_target_tools()`-only `OutboundTargetToolsParts` copy.
    tool_permission_overrides: Option<Arc<dyn ironclaw_approvals::ToolPermissionOverrideStore>>,
}

impl HostRuntimeCapabilityHarness {
    pub(crate) async fn file_tools() -> HarnessResult<Self> {
        let harness = Self::file_tools_with_runtime_policy(Some(
            ironclaw_reborn_composition::local_dev_yolo_runtime_policy(true)?,
        ))
        .await?;
        harness
            .enable_global_auto_approve_for_product_and_harness_users()
            .await?;
        Ok(harness)
    }

    pub(crate) async fn file_tools_requiring_approval() -> HarnessResult<Self> {
        let harness = Self::file_tools_with_runtime_policy(None).await?;
        // Global auto-approve now defaults ON, so disable it explicitly to keep
        // this constructor's per-tool approval gate behavior.
        harness
            .disable_global_auto_approve_for_product_and_harness_users()
            .await?;
        Ok(harness)
    }

    async fn file_tools_with_runtime_policy(
        runtime_policy: Option<ironclaw_host_api::runtime_policy::EffectiveRuntimePolicy>,
    ) -> HarnessResult<Self> {
        Self::new(
            "reborn-e2e-builtin-tools",
            vec![
                CapabilityId::new(WRITE_FILE_CAPABILITY_ID)?,
                CapabilityId::new(READ_FILE_CAPABILITY_ID)?,
            ],
            vec![EffectKind::ReadFilesystem, EffectKind::WriteFilesystem],
            Vec::new(),
            ExtensionId::new(BUILTIN_FIRST_PARTY_PROVIDER)?,
            UserId::new("reborn-e2e-builtin-user")?,
            runtime_policy,
        )
        .await
    }

    pub(crate) async fn write_only() -> HarnessResult<Self> {
        Self::new(
            "reborn-e2e-write-only",
            vec![CapabilityId::new(WRITE_FILE_CAPABILITY_ID)?],
            vec![EffectKind::WriteFilesystem],
            Vec::new(),
            ExtensionId::new(BUILTIN_FIRST_PARTY_PROVIDER)?,
            UserId::new("reborn-e2e-write-only-user")?,
            None,
        )
        .await
    }

    pub(crate) async fn coding_read_tools() -> HarnessResult<Self> {
        let harness = Self::new(
            "reborn-e2e-coding-read-tools",
            vec![
                CapabilityId::new(LIST_DIR_CAPABILITY_ID)?,
                CapabilityId::new(GLOB_CAPABILITY_ID)?,
                CapabilityId::new(GREP_CAPABILITY_ID)?,
            ],
            vec![EffectKind::ReadFilesystem],
            Vec::new(),
            ExtensionId::new(BUILTIN_FIRST_PARTY_PROVIDER)?,
            UserId::new("reborn-e2e-coding-read-user")?,
            None,
        )
        .await?;
        harness
            .enable_global_auto_approve_for_product_and_harness_users()
            .await?;
        Ok(harness)
    }

    pub(crate) async fn process_tools() -> HarnessResult<Self> {
        let harness = Self::new_with_options(
            "reborn-e2e-process-tools",
            vec![
                CapabilityId::new(ECHO_CAPABILITY_ID)?,
                CapabilityId::new(SHELL_CAPABILITY_ID)?,
                CapabilityId::new(SPAWN_SUBAGENT_CAPABILITY_ID)?,
            ],
            vec![
                EffectKind::DispatchCapability,
                EffectKind::ReadFilesystem,
                EffectKind::WriteFilesystem,
                EffectKind::Network,
                EffectKind::SpawnProcess,
                EffectKind::ExecuteCode,
            ],
            Vec::new(),
            ExtensionId::new(BUILTIN_FIRST_PARTY_PROVIDER)?,
            UserId::new("reborn-e2e-process-user")?,
            HostRuntimeHarnessOptions::new(MountView::default(), None),
        )
        .await?;
        harness
            .enable_global_auto_approve_for_product_and_harness_users()
            .await?;
        Ok(harness)
    }

    pub(crate) async fn qa_smoke_tools() -> HarnessResult<Self> {
        let (root, storage_root, workspace_root) = host_runtime_storage_roots()?;
        std::fs::create_dir_all(storage_root.join("skills"))?;
        std::fs::create_dir_all(storage_root.join("system/skills"))?;
        let runtime = local_dev_host_runtime_with_http_egress(
            storage_root,
            Arc::new(RecordingRuntimeHttpEgress::with_body(
                br#"{"accepted":true,"source":"qa-smoke"}"#.to_vec(),
            )),
            // qa_smoke_tools exercises real process execution (SpawnProcess effect);
            // leave the default LocalHostProcessPort in place.
            None,
        )?;
        let mounts = qa_smoke_mounts()?;
        let memory_mounts = memory_mounts(MountPermissions::read_write_list_delete())?;
        let memory_capability_ids = [
            CapabilityId::new(MEMORY_SEARCH_CAPABILITY_ID)?,
            CapabilityId::new(MEMORY_WRITE_CAPABILITY_ID)?,
            CapabilityId::new(MEMORY_READ_CAPABILITY_ID)?,
            CapabilityId::new(MEMORY_TREE_CAPABILITY_ID)?,
        ];
        Ok(Self {
            runtime,
            approval_parts: None,
            auto_approve_settings: None,
            pending_approval_scopes: Arc::new(Mutex::new(HashMap::new())),
            io: Arc::new(ProductLiveCapabilityIo::default()),
            root,
            workspace_root,
            mounts,
            capability_mount_overrides: memory_capability_ids
                .iter()
                .cloned()
                .map(|capability_id| (capability_id, memory_mounts.clone()))
                .collect(),
            capability_ids: vec![
                CapabilityId::new(ECHO_CAPABILITY_ID)?,
                CapabilityId::new(TIME_CAPABILITY_ID)?,
                CapabilityId::new(JSON_CAPABILITY_ID)?,
                CapabilityId::new(HTTP_CAPABILITY_ID)?,
                CapabilityId::new(HTTP_SAVE_CAPABILITY_ID)?,
                CapabilityId::new(MEMORY_SEARCH_CAPABILITY_ID)?,
                CapabilityId::new(MEMORY_WRITE_CAPABILITY_ID)?,
                CapabilityId::new(MEMORY_READ_CAPABILITY_ID)?,
                CapabilityId::new(MEMORY_TREE_CAPABILITY_ID)?,
                CapabilityId::new(READ_FILE_CAPABILITY_ID)?,
                CapabilityId::new(WRITE_FILE_CAPABILITY_ID)?,
                CapabilityId::new(LIST_DIR_CAPABILITY_ID)?,
                CapabilityId::new(GLOB_CAPABILITY_ID)?,
                CapabilityId::new(GREP_CAPABILITY_ID)?,
                CapabilityId::new(APPLY_PATCH_CAPABILITY_ID)?,
                CapabilityId::new(SHELL_CAPABILITY_ID)?,
                CapabilityId::new(SPAWN_SUBAGENT_CAPABILITY_ID)?,
                CapabilityId::new(SKILL_LIST_CAPABILITY_ID)?,
                CapabilityId::new(SKILL_INSTALL_CAPABILITY_ID)?,
                CapabilityId::new(SKILL_REMOVE_CAPABILITY_ID)?,
                CapabilityId::new(TRIGGER_CREATE_CAPABILITY_ID)?,
                CapabilityId::new(TRIGGER_LIST_CAPABILITY_ID)?,
                CapabilityId::new(TRIGGER_PAUSE_CAPABILITY_ID)?,
                CapabilityId::new(TRIGGER_RESUME_CAPABILITY_ID)?,
                CapabilityId::new(TRIGGER_REMOVE_CAPABILITY_ID)?,
            ],
            runtime_kind: RuntimeKind::FirstParty,
            effect_kinds: vec![
                EffectKind::DispatchCapability,
                EffectKind::ReadFilesystem,
                EffectKind::WriteFilesystem,
                EffectKind::DeleteFilesystem,
                EffectKind::Network,
                EffectKind::SpawnProcess,
                EffectKind::ExecuteCode,
                EffectKind::ExternalWrite,
            ],
            network_policy: http_test_policy(),
            secrets: Vec::new(),
            provider_id: ExtensionId::new(BUILTIN_FIRST_PARTY_PROVIDER)?,
            additional_provider_trust: Vec::new(),
            user_id: UserId::new("reborn-e2e-qa-smoke-user")?,
            invocations: Arc::new(Mutex::new(Vec::new())),
            results: Arc::new(Mutex::new(Vec::new())),
            http_egress: None,
            network_egress: None,
            process_port: None,
            profile_filesystem: None,
            project_service: None,
            skill_activation_source: None,
            attachment_test_support: None,
            outbound_target_tools: None,
            scope_capability_by_run_owner: false,
            product_auth: None,
            tool_permission_overrides: None,
        })
    }

    pub(crate) async fn extension_lifecycle_tools() -> HarnessResult<Self> {
        let mut capability_ids = capability_ids_from_strs(EXTENSION_LIFECYCLE_CAPABILITY_IDS)?;
        capability_ids.extend(capability_ids_from_strs(BUNDLED_EXTENSION_CAPABILITY_IDS)?);
        let mut harness = Self::new_with_options(
            "reborn-e2e-extension-lifecycle-tools",
            capability_ids,
            local_dev_all_effects(),
            Vec::new(),
            ExtensionId::new(BUILTIN_FIRST_PARTY_PROVIDER)?,
            UserId::new("reborn-e2e-extension-lifecycle-user")?,
            HostRuntimeHarnessOptions::new(
                MountView::default(),
                Some(ironclaw_reborn_composition::local_dev_yolo_runtime_policy(
                    true,
                )?),
            )
            .with_seed_extension_credentials(),
        )
        .await?;
        harness.network_policy = wildcard_test_policy();
        harness.additional_provider_trust = bundled_extension_provider_trust()?;
        harness
            .enable_global_auto_approve_for_product_and_harness_users()
            .await?;
        Ok(harness)
    }

    /// C-JOURNEY convergence seam: surfaces the file-tool approval-gate
    /// capabilities (`write_file`/`read_file`, `PermissionMode::Ask` — same
    /// grant shape as `file_tools_requiring_approval`) AND a single GitHub
    /// capability (`github.get_repo`) on the SAME `build_reborn_services`
    /// local-dev runtime — the one wired with the
    /// `run_state`/`approval_requests`/`capability_leases` stores BOTH gate
    /// classes' resume paths need (`new_with_options` -> `build_reborn_services`).
    ///
    /// Distinct from `github_issue_tools_auth_required` (a separate,
    /// lower-level `HostRuntimeServices` build with a hardcoded
    /// `FixedRuntimeCredentialAccountResolver` and no run_state store — see
    /// that constructor's doc comment): this harness's `github.*` credential
    /// resolves through the REAL `ProductAuthRuntimeCredentialResolver`
    /// (`factory.rs`, wired unconditionally by `build_reborn_services`). No
    /// GitHub credential account is seeded at construction (unlike
    /// `extension_lifecycle_tools`, which seeds all four bundled providers via
    /// `.with_seed_extension_credentials()`).
    ///
    /// **Gate chaining (empirically verified, not assumed):** the global
    /// auto-approve toggle this harness disables (for the file-tool arm) is
    /// NOT capability-scoped, so `github.get_repo` first raises a real
    /// `TurnStatus::BlockedApproval` too. Approving re-dispatches the
    /// still-uncredentialed capability, which blocks AGAIN at a real
    /// `TurnStatus::BlockedAuth` (`CredentialStageError::AuthRequired`).
    /// `RebornIntegrationHarness::resolve_auth_gate` seeds the account
    /// (`seed_github_credential_account`) and resumes, letting the SAME
    /// parked capability re-dispatch and complete — the happy-path auth
    /// resume the `github_issue_tools_auth_required` fixture cannot do. See
    /// `scenario_auth_then_approval_journey`'s module doc for the full
    /// approval->auth chain a caller must drive.
    ///
    /// **Making `github.*` genuinely dispatchable (not just granted) needed
    /// two additive test-support seams, both required together:**
    /// 1. `capability_ids`/`additional_provider_trust` alone are NOT enough —
    ///    they only populate the harness-authority grant layer. The runtime's
    ///    OWN dispatchable registry (`build_local_runtime`'s
    ///    `local_dev_builtin_extension_registry()`) contains only first-party
    ///    builtins + the four lifecycle capabilities; bundled packages
    ///    (github, gmail, …) live in a SEPARATE `AvailableExtensionCatalog`
    ///    used for search only. Without registry presence, a scripted
    ///    `github.*` call silently never reaches `invoke_capability` (the run
    ///    completes with zero recorded invocations). Fixed via
    ///    `RebornServices::publish_bundled_extension_for_test`
    ///    (`factory.rs`, new `#[cfg(feature = "test-support")]` accessor) —
    ///    reaches the SAME `ActiveExtensionPublisher::publish` step
    ///    `builtin.extension_activate` calls, called directly at harness
    ///    construction instead of via a scripted install/activate handshake.
    /// 2. Registry presence alone still isn't sufficient: `build_local_runtime`
    ///    mounts `/system/extensions` at an EMPTY per-harness tempdir, so the
    ///    runtime fails to compile `wasm/github_tool.wasm` at dispatch time
    ///    (`Failed{host_creation_failed}`) even once the package metadata is
    ///    registered. Fixed by copying the REAL asset directory
    ///    (`github_support::asset_root()`, already used by the
    ///    `github_issue_tools_*` harnesses) into this harness's own tempdir
    ///    mount (`copy_dir_recursive`) — no new fixtures, reuses the existing
    ///    on-disk asset tree.
    ///
    /// Runtime policy is left at `None` (like `file_tools_requiring_approval`,
    /// NOT the `LocalDevYolo` policy `extension_lifecycle_tools` uses) so the
    /// file tools' real `PermissionMode::Ask` gate is preserved; the two seams
    /// above are independent of the runtime-policy profile.
    pub(crate) async fn file_and_github_auth_tools() -> HarnessResult<Self> {
        // Hermetic guard: `new_with_options`'s `build_local_runtime` defaults to
        // a REAL `ReqwestNetworkTransport` when no test egress is supplied
        // (`factory.rs`). This harness surfaces a `github.*` WASM capability
        // that crosses HTTP, so it MUST override the network egress or the
        // post-resume dispatch would attempt a live network call.
        let github_fixture_response =
            br#"{"id":1,"full_name":"octocat/hello-world","private":false}"#.to_vec();
        let network_egress: Arc<dyn NetworkHttpEgress> = Arc::new(
            RecordingNetworkHttpEgress::with_body(github_fixture_response),
        );
        let mut harness = Self::new_with_options(
            "reborn-e2e-file-github-auth-tools",
            vec![
                CapabilityId::new(WRITE_FILE_CAPABILITY_ID)?,
                CapabilityId::new(READ_FILE_CAPABILITY_ID)?,
                CapabilityId::new("github.get_repo")?,
            ],
            local_dev_all_effects(),
            Vec::new(),
            ExtensionId::new(BUILTIN_FIRST_PARTY_PROVIDER)?,
            UserId::new("reborn-e2e-file-github-auth-user")?,
            HostRuntimeHarnessOptions::new(
                workspace_mounts(MountPermissions::read_write_list_delete())?,
                None,
            )
            .with_network_http_egress_for_test(network_egress)
            .with_activated_bundled_extension(github_support::extension_package()?),
        )
        .await?;
        harness.network_policy = wildcard_test_policy();
        harness.additional_provider_trust = bundled_extension_provider_trust()?;
        // See point 2 of this constructor's doc comment: registry presence
        // alone isn't enough, the WASM asset bytes must be copied into this
        // harness's own tempdir mount too.
        copy_dir_recursive(
            &github_support::asset_root(),
            &harness
                .root
                .path()
                .join("local-dev/system/extensions/github"),
        )?;
        // Global auto-approve now defaults ON; disable it so write_file/read_file
        // raise real `BlockedApproval` gates (mirrors `file_tools_requiring_approval`).
        // The GitHub auth gate is a separate mechanism (credential resolution, not
        // approval mode) and is unaffected by this toggle.
        harness
            .disable_global_auto_approve_for_product_and_harness_users()
            .await?;
        Ok(harness)
    }

    /// C-JOURNEY: seed a real GitHub credential account — WITH real secret
    /// material — through the PRODUCTION manual-token flow
    /// (`request_manual_token_setup` → `submit_manual_token`, the same
    /// two-step path the real "user pastes a token in the UI" flow drives), so
    /// a parked `github.*` auth gate's `ProductAuthRuntimeCredentialResolver`
    /// lookup resolves on re-dispatch AND the re-dispatched WASM capability's
    /// credential obligation can actually stage the token. A bare
    /// `credential_account_service().create_account(..)` carrying a dangling
    /// `SecretHandle` is NOT enough: it clears the auth gate, but the
    /// re-dispatched execution then fails at `stage_credential_material`
    /// (no material behind the handle) and the run still completes with a
    /// model-visible failure — caught by `assert_tool_result_contains`.
    ///
    /// `scope` MUST be the run's actual dispatch-time `(tenant, user, agent,
    /// project)` — `ProductAuthRuntimeCredentialResolver::resolve_access_secret`'s
    /// `account_visible_from_runtime_scope` check matches on all four, so a
    /// mismatched scope (e.g. a fixed literal that doesn't match the calling
    /// group/harness's real run scope) silently seeds an account the
    /// dispatch-time lookup never finds, leaving the run stuck at
    /// `BlockedAuth`. Callers build this from their own resolved run scope
    /// (see `RebornIntegrationHarness::resolve_auth_gate`, which uses
    /// `self.turn_scope` + `self.binding.actor_user_id` — the SAME fields
    /// `resume_run` uses).
    ///
    /// Continuation is `AuthContinuationRef::SetupOnly`: the harness's
    /// `resolve_auth_gate` performs the run resume itself (mirroring the
    /// approval-gate helpers), so the flow must not ALSO dispatch a
    /// `TurnGateResume` continuation.
    pub(crate) async fn seed_github_credential_account(
        &self,
        scope: &ResourceScope,
    ) -> HarnessResult<()> {
        let product_auth = self
            .product_auth
            .as_ref()
            .ok_or("harness missing local-dev product auth (not built via new_with_options)")?;
        let scope = AuthProductScope::credential_owner(scope, AuthSurface::Api);
        let challenge = product_auth
            .request_manual_token_setup(
                ironclaw_reborn_composition::RebornManualTokenSetupRequest::new(
                    scope.clone(),
                    AuthProviderId::new("github")?,
                    CredentialAccountLabel::new("journey github")?,
                    ironclaw_auth::AuthContinuationRef::SetupOnly,
                    chrono::Utc::now() + chrono::Duration::minutes(10),
                ),
            )
            .await
            .map_err(|error| format!("manual token setup failed: {error:?}"))?;
        product_auth
            .submit_manual_token(
                ironclaw_reborn_composition::RebornManualTokenSubmitRequest::new(
                    scope.clone(),
                    challenge.interaction_id,
                    secrecy::SecretString::from("journey-github-token"),
                ),
            )
            .await
            .map_err(|error| format!("manual token submit failed: {error:?}"))?;
        Ok(())
    }

    /// E-PROJ: harness surfacing the local-dev synthetic `project_create`
    /// capability. `create_capability_port` injects the synthetic capability via
    /// `apply_synthetic_capability_wrappers` because `PROJECT_CREATE_CAPABILITY_ID`
    /// is in the allowlist. Auto-approve is enabled so the capability dispatches
    /// without a gate.
    pub(crate) async fn project_tools() -> HarnessResult<Self> {
        let harness = Self::new_with_options(
            "reborn-e2e-project-tools",
            vec![CapabilityId::new(
                ironclaw_reborn_composition::test_support::PROJECT_CREATE_CAPABILITY_ID,
            )?],
            vec![
                EffectKind::DispatchCapability,
                EffectKind::ReadFilesystem,
                EffectKind::WriteFilesystem,
            ],
            Vec::new(),
            ExtensionId::new(BUILTIN_FIRST_PARTY_PROVIDER)?,
            UserId::new("reborn-e2e-project-tools-user")?,
            HostRuntimeHarnessOptions::new(
                MountView::default(),
                Some(ironclaw_reborn_composition::local_dev_yolo_runtime_policy(
                    true,
                )?),
            ),
        )
        .await?;
        harness
            .enable_global_auto_approve_for_product_and_harness_users()
            .await?;
        Ok(harness)
    }

    /// C-SYNTH `project_create` fault-injection arm: same surface as
    /// `project_tools()`, but the real `Arc<dyn ProjectService>` is wrapped in
    /// `FaultInjectingProjectService`
    /// (`with_project_service_fault_injection`) so a `create_project` call
    /// naming `FAULT_INJECT_DENIED_PROJECT_NAME` returns
    /// `ProjectServiceError::Denied`/`PolicyDenied` and proves the real
    /// capability dispatch's recoverable `Failed` behavior. This is *not*
    /// the `project_service_outcome` `Unavailable` / internal-retry path.
    /// Any other `create_project` name still reaches the real store.
    pub(crate) async fn project_tools_with_fault_injection() -> HarnessResult<Self> {
        let harness = Self::new_with_options(
            "reborn-e2e-project-tools-fault-injection",
            vec![CapabilityId::new(
                ironclaw_reborn_composition::test_support::PROJECT_CREATE_CAPABILITY_ID,
            )?],
            vec![
                EffectKind::DispatchCapability,
                EffectKind::ReadFilesystem,
                EffectKind::WriteFilesystem,
            ],
            Vec::new(),
            ExtensionId::new(BUILTIN_FIRST_PARTY_PROVIDER)?,
            UserId::new("reborn-e2e-project-tools-fault-injection-user")?,
            HostRuntimeHarnessOptions::new(
                MountView::default(),
                Some(ironclaw_reborn_composition::local_dev_yolo_runtime_policy(
                    true,
                )?),
            )
            .with_project_service_fault_injection(),
        )
        .await?;
        harness
            .enable_global_auto_approve_for_product_and_harness_users()
            .await?;
        Ok(harness)
    }

    /// C-SYNTH outbound: harness surfacing the two local-dev synthetic
    /// `outbound_delivery_*` capabilities over an injected
    /// [`FakeOutboundPreferencesFacade`] double.
    /// `create_capability_port` injects them via
    /// `apply_synthetic_capability_wrappers` because
    /// `outbound_target_tools` is `Some`. `target_set` runs with
    /// `requires_approval = true`, so its settings decision is exercised for
    /// real: global auto-approve (default ON) → `Allow`; a `Disabled` tool
    /// override (`disable_outbound_target_set_tool`) → `Deny`; auto-approve
    /// disabled → `Ask` (approval gate). The RETURNED harness leaves global
    /// auto-approve at its default-ON state so the happy/`NotFound` arms
    /// dispatch through `Allow`; the gate arm disables it per-test.
    pub(crate) async fn outbound_target_tools() -> HarnessResult<Self> {
        let facade =
            super::outbound_preferences::FakeOutboundPreferencesFacade::with_default_targets();
        Self::new_with_options(
            "reborn-e2e-outbound-target-tools",
            vec![
                CapabilityId::new(
                    ironclaw_reborn_composition::test_support::OUTBOUND_DELIVERY_TARGETS_LIST_CAPABILITY_ID,
                )?,
                CapabilityId::new(
                    ironclaw_reborn_composition::test_support::OUTBOUND_DELIVERY_TARGET_SET_CAPABILITY_ID,
                )?,
            ],
            vec![
                EffectKind::DispatchCapability,
                EffectKind::ExternalWrite,
                EffectKind::ReadFilesystem,
                EffectKind::WriteFilesystem,
            ],
            Vec::new(),
            ExtensionId::new(BUILTIN_FIRST_PARTY_PROVIDER)?,
            UserId::new("reborn-e2e-outbound-target-user")?,
            HostRuntimeHarnessOptions::new(
                MountView::default(),
                Some(ironclaw_reborn_composition::local_dev_yolo_runtime_policy(
                    true,
                )?),
            )
            .with_outbound_target_tools(facade, true),
        )
        .await
    }

    /// Group whose ONLY capability is `builtin.profile_set` (E-PROFILE seam).
    /// Uses `new_with_options` (not `core_builtin_tools_from_runtime`), so
    /// `profile_filesystem` is populated from `services.local_dev_profile_filesystem_for_test()`
    /// — the read-back half of the round trip a `RebornIntegrationGroup::profile_tools()`
    /// scenario needs. Base mounts are `/memory` directly (this harness's only
    /// capability needs it; no per-capability mount override required, unlike
    /// `core_builtin_tools_from_runtime`'s multi-capability surface).
    pub(crate) async fn profile_tools() -> HarnessResult<Self> {
        let harness = Self::new_with_options(
            "reborn-e2e-profile-tools",
            vec![CapabilityId::new(PROFILE_SET_CAPABILITY_ID)?],
            vec![
                EffectKind::DispatchCapability,
                EffectKind::ReadFilesystem,
                EffectKind::WriteFilesystem,
            ],
            Vec::new(),
            ExtensionId::new(BUILTIN_FIRST_PARTY_PROVIDER)?,
            UserId::new("reborn-e2e-profile-tools-user")?,
            HostRuntimeHarnessOptions::new(
                memory_mounts(MountPermissions::read_write_list_delete())?,
                Some(ironclaw_reborn_composition::local_dev_yolo_runtime_policy(
                    true,
                )?),
            ),
        )
        .await?;
        harness
            .enable_global_auto_approve_for_product_and_harness_users()
            .await?;
        Ok(harness)
    }

    /// Group with NO first-party capability dispatch — the test drives the
    /// C-ATTACH seam purely through the attachment read port + inbound lander,
    /// never a tool call. Uses `new_with_options` (mirrors `profile_tools()`),
    /// so `attachment_test_support` is populated from
    /// `services.local_dev_attachment_test_support_for_test()`. No mounts needed:
    /// attachment landing/reading goes through `local_runtime.workspace_filesystem`
    /// directly, not the capability-dispatch `MountView` (mirrors
    /// `trigger_management_tools()`'s `MountView::default()`, which also has no
    /// filesystem capability to gate).
    pub(crate) async fn attachment_tools() -> HarnessResult<Self> {
        Self::new_with_options(
            "reborn-e2e-attachment-tools",
            Vec::new(),
            vec![EffectKind::ReadFilesystem, EffectKind::WriteFilesystem],
            Vec::new(),
            ExtensionId::new(BUILTIN_FIRST_PARTY_PROVIDER)?,
            UserId::new("reborn-e2e-attachment-tools-user")?,
            HostRuntimeHarnessOptions::new(
                MountView::default(),
                Some(ironclaw_reborn_composition::local_dev_yolo_runtime_policy(
                    true,
                )?),
            ),
        )
        .await
    }

    /// `pub(crate)`: also used by `RebornIntegrationGroupBuilder::skill_management_tools`
    /// (`group_constructors.rs`, C-SKILL) to wire the SAME preset onto the
    /// int-tier group, so the QA/trace-tier smoke test and the int-tier group
    /// never drift on capability ids / mounts / policy.
    pub(crate) async fn skill_management_tools() -> HarnessResult<Self> {
        let mut harness = Self::new_with_options(
            "reborn-e2e-skill-management-tools",
            vec![
                CapabilityId::new(SKILL_LIST_CAPABILITY_ID)?,
                CapabilityId::new(SKILL_INSTALL_CAPABILITY_ID)?,
                CapabilityId::new(SKILL_REMOVE_CAPABILITY_ID)?,
            ],
            vec![
                EffectKind::DispatchCapability,
                EffectKind::ReadFilesystem,
                EffectKind::WriteFilesystem,
                EffectKind::DeleteFilesystem,
                EffectKind::Network,
            ],
            Vec::new(),
            ExtensionId::new(BUILTIN_FIRST_PARTY_PROVIDER)?,
            UserId::new("reborn-e2e-skill-management-user")?,
            HostRuntimeHarnessOptions::new(
                skill_mounts()?,
                Some(ironclaw_reborn_composition::local_dev_yolo_runtime_policy(
                    true,
                )?),
            ),
        )
        .await?;
        harness.network_policy = http_test_policy();
        harness
            .enable_global_auto_approve_for_product_and_harness_users()
            .await?;
        Ok(harness)
    }

    /// Harness surfacing the local-dev synthetic `skill_activate` capability
    /// (E-SKILL seam). `new_with_options` builds the `skill_activation_source`
    /// (because `SKILL_ACTIVATE_CAPABILITY_ID` is in the allowlist) under
    /// `tenant` — the caller's ACTUAL group run-scope tenant, passed through
    /// rather than re-hardcoded here — which `create_capability_port` wraps
    /// onto the port and `into_group` wires as the runtime's
    /// `skill_context_source`. The skill file the model activates is seeded as
    /// a system-scoped skill by `RebornIntegrationGroup::skill_activation_tools`.
    /// Mirrors `skill_management_tools`/`project_tools`.
    pub(crate) async fn skill_activation_tools(tenant: &TenantId) -> HarnessResult<Self> {
        let mut harness = Self::new_with_options(
            "reborn-e2e-skill-activation-tools",
            vec![CapabilityId::new(
                ironclaw_reborn_composition::test_support::SKILL_ACTIVATE_CAPABILITY_ID,
            )?],
            vec![
                EffectKind::DispatchCapability,
                EffectKind::ReadFilesystem,
                EffectKind::WriteFilesystem,
                EffectKind::Network,
            ],
            Vec::new(),
            ExtensionId::new(BUILTIN_FIRST_PARTY_PROVIDER)?,
            UserId::new("reborn-e2e-skill-activation-user")?,
            HostRuntimeHarnessOptions::new(
                skill_mounts()?,
                Some(ironclaw_reborn_composition::local_dev_yolo_runtime_policy(
                    true,
                )?),
            )
            .with_skill_activation_tenant(tenant.clone()),
        )
        .await?;
        harness.network_policy = http_test_policy();
        harness
            .enable_global_auto_approve_for_product_and_harness_users()
            .await?;
        Ok(harness)
    }

    pub(crate) async fn trigger_management_tools() -> HarnessResult<Self> {
        let harness = Self::new_with_options(
            "reborn-e2e-trigger-management-tools",
            vec![
                CapabilityId::new(TRIGGER_CREATE_CAPABILITY_ID)?,
                CapabilityId::new(TRIGGER_LIST_CAPABILITY_ID)?,
                CapabilityId::new(TRIGGER_PAUSE_CAPABILITY_ID)?,
                CapabilityId::new(TRIGGER_RESUME_CAPABILITY_ID)?,
                CapabilityId::new(TRIGGER_REMOVE_CAPABILITY_ID)?,
            ],
            vec![EffectKind::DispatchCapability, EffectKind::ExternalWrite],
            Vec::new(),
            ExtensionId::new(BUILTIN_FIRST_PARTY_PROVIDER)?,
            UserId::new("reborn-e2e-trigger-management-user")?,
            HostRuntimeHarnessOptions::new(
                MountView::default(),
                Some(ironclaw_reborn_composition::local_dev_yolo_runtime_policy(
                    true,
                )?),
            ),
        )
        .await?;
        harness
            .enable_global_auto_approve_for_product_and_harness_users()
            .await?;
        Ok(harness)
    }

    async fn enable_global_auto_approve_for_product_and_harness_users(&self) -> HarnessResult<()> {
        let product_scope = product_scope();
        self.enable_global_auto_approve(product_scope.clone())
            .await?;
        let mut harness_user_scope = product_scope;
        harness_user_scope.user_id = self.user_id.clone();
        self.enable_global_auto_approve(harness_user_scope).await?;
        Ok(())
    }

    pub(crate) async fn enable_global_auto_approve(
        &self,
        scope: ResourceScope,
    ) -> HarnessResult<()> {
        let store = self
            .auto_approve_settings
            .as_ref()
            .ok_or("host runtime harness missing local-dev auto-approve settings")?;
        store
            .set(AutoApproveSettingInput {
                updated_by: Principal::User(scope.user_id.clone()),
                scope,
                enabled: true,
            })
            .await?;
        Ok(())
    }

    /// Global auto-approve now defaults ON. A test that needs to exercise the
    /// per-tool approval gate must flip it OFF for the product and harness-user
    /// scopes the run authorizes against, as an explicit precondition.
    pub async fn disable_global_auto_approve_for_product_and_harness_users(
        &self,
    ) -> HarnessResult<()> {
        let product_scope = product_scope();
        self.disable_global_auto_approve(product_scope.clone())
            .await?;
        let mut harness_user_scope = product_scope;
        harness_user_scope.user_id = self.user_id.clone();
        self.disable_global_auto_approve(harness_user_scope).await?;
        Ok(())
    }

    pub(crate) async fn disable_global_auto_approve(
        &self,
        scope: ResourceScope,
    ) -> HarnessResult<()> {
        let store = self
            .auto_approve_settings
            .as_ref()
            .ok_or("host runtime harness missing local-dev auto-approve settings")?;
        store
            .set(AutoApproveSettingInput {
                updated_by: Principal::User(scope.user_id.clone()),
                scope,
                enabled: false,
            })
            .await?;
        Ok(())
    }

    pub(crate) async fn trace_commons_tools() -> HarnessResult<Self> {
        let mut harness = Self::new_with_options(
            "reborn-e2e-trace-commons-tools",
            vec![
                CapabilityId::new(TRACE_COMMONS_ONBOARD_CAPABILITY_ID)?,
                CapabilityId::new(TRACE_COMMONS_STATUS_CAPABILITY_ID)?,
                CapabilityId::new(TRACE_COMMONS_CREDITS_CAPABILITY_ID)?,
                CapabilityId::new(TRACE_COMMONS_PROFILE_TOKEN_CAPABILITY_ID)?,
                CapabilityId::new(TRACE_COMMONS_PROFILE_SET_CAPABILITY_ID)?,
            ],
            vec![
                EffectKind::DispatchCapability,
                EffectKind::ReadFilesystem,
                // onboard persists device-key material (Ed25519 keypair +
                // policy.json) and profile_token writes profile_token.jwt, so
                // the harness allow-set must grant WriteFilesystem or those
                // capabilities are filtered out of the model-visible surface.
                EffectKind::WriteFilesystem,
                EffectKind::Network,
                EffectKind::ExternalWrite,
            ],
            Vec::new(),
            ExtensionId::new(BUILTIN_FIRST_PARTY_PROVIDER)?,
            UserId::new("reborn-e2e-trace-commons-user")?,
            // The Trace Commons write/network capabilities are
            // PermissionMode::Ask (onboard, profile_token, profile_set) — like
            // the skill/trigger harnesses, the scripted run enables global
            // auto-approve so it is not gated.
            HostRuntimeHarnessOptions::new(
                MountView::default(),
                Some(ironclaw_reborn_composition::local_dev_yolo_runtime_policy(
                    true,
                )?),
            ),
        )
        .await?;
        // onboard declares EffectKind::Network, so the lease must carry a
        // non-empty network policy or the obligation check rejects dispatch
        // before the consent gate runs.
        harness.network_policy = http_test_policy();
        harness
            .enable_global_auto_approve_for_product_and_harness_users()
            .await?;
        Ok(harness)
    }

    async fn new(
        service_label: &'static str,
        capability_ids: Vec<CapabilityId>,
        effect_kinds: Vec<EffectKind>,
        secrets: Vec<SecretHandle>,
        provider_id: ExtensionId,
        user_id: UserId,
        runtime_policy: Option<ironclaw_host_api::runtime_policy::EffectiveRuntimePolicy>,
    ) -> HarnessResult<Self> {
        Self::new_with_options(
            service_label,
            capability_ids,
            effect_kinds,
            secrets,
            provider_id,
            user_id,
            HostRuntimeHarnessOptions::new(
                workspace_mounts(MountPermissions::read_write_list_delete())?,
                runtime_policy,
            ),
        )
        .await
    }

    async fn new_with_options(
        service_label: &'static str,
        capability_ids: Vec<CapabilityId>,
        effect_kinds: Vec<EffectKind>,
        secrets: Vec<SecretHandle>,
        provider_id: ExtensionId,
        user_id: UserId,
        options: HostRuntimeHarnessOptions,
    ) -> HarnessResult<Self> {
        let HostRuntimeHarnessOptions {
            mounts,
            runtime_policy,
            seed_extension_credentials,
            skill_activation_tenant,
            outbound_target_facade,
            network_http_egress_for_test,
            activate_bundled_extensions_for_test,
            project_service_fault_injection,
        } = options;
        let root = Arc::new(tempfile::tempdir()?);
        let storage_root = root.path().join("local-dev");
        let workspace_root = storage_root.join("workspace");
        std::fs::create_dir_all(&workspace_root)?;
        let mut input = if runtime_policy.as_ref().is_some_and(|policy| {
            policy.resolved_profile == ironclaw_host_api::runtime_policy::RuntimeProfile::LocalYolo
        }) {
            let host_home_root = root.path().join("host-home");
            std::fs::create_dir_all(&host_home_root)?;
            ironclaw_reborn_composition::local_runtime_build_input_with_options(
                ironclaw_reborn_composition::RebornCompositionProfile::LocalDevYolo,
                service_label,
                storage_root,
                ironclaw_reborn_composition::RebornLocalRuntimeProfileOptions {
                    confirm_host_access: true,
                },
            )?
            .with_local_dev_confirmed_host_home_root(host_home_root)
        } else {
            RebornBuildInput::local_dev(service_label, storage_root)
        };
        if let Some(runtime_policy) = runtime_policy {
            input = input.with_runtime_policy(runtime_policy);
        }
        if let Some(egress) = network_http_egress_for_test {
            input = input.with_network_http_egress_for_test(egress);
        }
        let services = build_reborn_services(input).await?;
        if seed_extension_credentials {
            seed_extension_lifecycle_credentials(&services, &user_id).await?;
        }
        // C-JOURNEY: publish bundled WASM packages into the active-extension
        // registry directly (see `HostRuntimeHarnessOptions::activate_bundled_extensions_for_test`
        // doc) so their capabilities are genuinely dispatchable, not merely
        // granted at the harness-authority layer.
        for package in &activate_bundled_extensions_for_test {
            services
                .publish_bundled_extension_for_test(package)
                .ok_or(
                    "local-dev Reborn services missing extension management for test publish",
                )??;
        }
        let approval_parts = services.local_dev_approval_test_parts();
        let auto_approve_settings = services.local_dev_auto_approve_settings_for_test();
        // Capture the profile filesystem + project service + attachment support
        // before `services.host_runtime` is moved out below (E-PROFILE / E-PROJ /
        // C-ATTACH seams).
        let profile_filesystem = services.local_dev_profile_filesystem_for_test();
        // C-SYNTH `project_create` fault-injection seam: wrap the real service
        // in `FaultInjectingProjectService` only when the harness opted in
        // (`with_project_service_fault_injection`) — every other harness keeps
        // the real service unwrapped and behaves exactly as before.
        let project_service: Option<Arc<dyn ProjectService>> =
            services.local_dev_project_service_for_test().map(|inner| {
                if project_service_fault_injection {
                    super::project_service_fault::FaultInjectingProjectService::wrapping(inner)
                        as Arc<dyn ProjectService>
                } else {
                    inner
                }
            });
        // C-JOURNEY: capture product-auth before `services.host_runtime` is
        // moved out below, so `seed_github_credential_account` can create a
        // real credential account later (auth-gate happy-path resume).
        let product_auth = services.product_auth.clone();
        // E-SKILL: build the local-dev skill context source only when this
        // harness surfaces the synthetic `skill_activate` capability (i.e.
        // `skill_activation_tools`). Built with the caller-supplied tenant
        // (`HostRuntimeHarnessOptions::with_skill_activation_tenant`, sourced
        // from the group's actual run-scope tenant) so activation visibility
        // matches the turn's scope. Must precede the `services.host_runtime`
        // move (it borrows `&services`).
        let skill_activation_source = if capability_ids.iter().any(|id| {
            id.as_str() == ironclaw_reborn_composition::test_support::SKILL_ACTIVATE_CAPABILITY_ID
        }) {
            let tenant = skill_activation_tenant
                .ok_or("skill_activation_tools harness requires with_skill_activation_tenant")?;
            ironclaw_reborn_composition::test_support::build_local_dev_skill_context_source_for_test(
                &services, &tenant, true,
            )
        } else {
            None
        };
        let attachment_test_support = services.local_dev_attachment_test_support_for_test();
        // W4-ASK-EACH-ONCE: capture the local-dev per-tool permission override
        // store unconditionally (mirrors `auto_approve_settings` above), not just
        // for `outbound_target_tools()`'s narrower `Some((facade, ..))` arm below
        // -- any host-runtime-backed harness/group can now install a per-capability
        // `AskEachTime` override via `set_ask_each_time_override_for_test`.
        let tool_permission_overrides = services.local_dev_tool_permission_overrides_for_test();
        // C-SYNTH outbound: pair the injected facade double with the local-dev
        // settings stores production's `outbound_delivery_capabilities` consumes,
        // captured from `RebornServices` before the `host_runtime` move. Only
        // `outbound_target_tools()` supplies the facade.
        let outbound_target_tools = match outbound_target_facade {
            Some((facade, requires_approval)) => {
                let tool_permission_overrides = services
                    .local_dev_tool_permission_overrides_for_test()
                    .ok_or("outbound_target_tools requires a local-dev tool-override store")?;
                let persistent_approval_policies = services
                    .local_dev_persistent_approval_policies_for_test()
                    .ok_or("outbound_target_tools requires a local-dev persistent-policy store")?;
                Some(OutboundTargetToolsParts {
                    facade,
                    requires_approval,
                    tool_permission_overrides,
                    persistent_approval_policies,
                })
            }
            None => None,
        };
        let pending_approval_scopes = Arc::new(Mutex::new(HashMap::new()));
        let runtime = services
            .host_runtime
            .ok_or("local-dev Reborn services missing host runtime")?;
        let runtime = Arc::new(RecordingHostRuntime::new(
            runtime,
            Arc::clone(&pending_approval_scopes),
        ));
        Ok(Self {
            runtime,
            approval_parts,
            auto_approve_settings,
            pending_approval_scopes,
            io: Arc::new(ProductLiveCapabilityIo::default()),
            root,
            workspace_root,
            mounts,
            capability_mount_overrides: Vec::new(),
            capability_ids,
            runtime_kind: RuntimeKind::FirstParty,
            effect_kinds,
            network_policy: NetworkPolicy::default(),
            secrets,
            provider_id,
            additional_provider_trust: Vec::new(),
            user_id,
            invocations: Arc::new(Mutex::new(Vec::new())),
            results: Arc::new(Mutex::new(Vec::new())),
            http_egress: None,
            network_egress: None,
            process_port: None,
            profile_filesystem,
            project_service,
            skill_activation_source,
            attachment_test_support,
            outbound_target_tools,
            scope_capability_by_run_owner: false,
            product_auth,
            tool_permission_overrides,
        })
    }

    pub(crate) async fn core_builtin_tools() -> HarnessResult<Self> {
        Self::core_builtin_tools_with_network_policy(http_test_policy()).await
    }

    pub(crate) async fn core_builtin_tools_with_network_policy(
        network_policy: NetworkPolicy,
    ) -> HarnessResult<Self> {
        Self::core_builtin_tools_with_network_policy_and_process_port(network_policy, true).await
    }

    /// Variant used by `.with_live_shell()`: same as `core_builtin_tools_with_network_policy`
    /// but opts out of the recording process port so the real `LocalHostProcessPort`
    /// executes shell commands on the host.
    pub(crate) async fn core_builtin_tools_with_live_shell() -> HarnessResult<Self> {
        Self::core_builtin_tools_with_network_policy_and_process_port(http_test_policy(), false)
            .await
    }

    async fn core_builtin_tools_with_network_policy_and_process_port(
        network_policy: NetworkPolicy,
        recording_process: bool,
    ) -> HarnessResult<Self> {
        let (root, storage_root, workspace_root) = host_runtime_storage_roots()?;
        let runtime_http_egress = Arc::new(RecordingRuntimeHttpEgress::with_body(
            br#"{"accepted":true}"#.to_vec(),
        ));
        // Slice 5: inject the inert recording port by default so `builtin.shell`
        // invocations in tests never spawn a real OS process. The `.with_live_shell()`
        // opt-in passes `recording_process = false`, which skips injection and lets
        // `HostRuntimeServices` default to the real `LocalHostProcessPort`.
        let recording_process_port = if recording_process {
            Some(Arc::new(super::process::RecordingProcessPort::new()))
        } else {
            None
        };
        let process_port_dyn: Option<Arc<dyn RuntimeProcessPort>> = recording_process_port
            .as_ref()
            .map(|p| Arc::clone(p) as Arc<dyn RuntimeProcessPort>);
        let runtime = local_dev_host_runtime_with_http_egress(
            storage_root.clone(),
            Arc::clone(&runtime_http_egress),
            process_port_dyn,
        )?;
        let mut harness = Self::core_builtin_tools_from_runtime(
            root,
            workspace_root,
            runtime,
            network_policy,
            UserId::new("reborn-e2e-core-builtins-user")?,
        )?;
        harness.http_egress = Some(runtime_http_egress);
        harness.process_port = recording_process_port;
        Ok(harness)
    }

    pub(crate) async fn core_builtin_tools_with_live_http_egress(
        network_policy: NetworkPolicy,
    ) -> HarnessResult<Self> {
        let (root, storage_root, workspace_root) = host_runtime_storage_roots()?;
        let runtime = local_dev_host_runtime_with_live_http_egress(storage_root.clone())?;
        Self::core_builtin_tools_from_runtime(
            root,
            workspace_root,
            runtime,
            network_policy,
            UserId::new("reborn-e2e-core-builtins-live-http-user")?,
        )
    }

    fn core_builtin_tools_from_runtime(
        root: Arc<tempfile::TempDir>,
        workspace_root: PathBuf,
        runtime: Arc<dyn HostRuntime>,
        network_policy: NetworkPolicy,
        user_id: UserId,
    ) -> HarnessResult<Self> {
        let mounts = workspace_mounts(MountPermissions::read_write_list_delete())?;
        let memory_mounts = memory_mounts(MountPermissions::read_write_list_delete())?;
        let memory_capability_ids = [
            CapabilityId::new(MEMORY_SEARCH_CAPABILITY_ID)?,
            CapabilityId::new(MEMORY_WRITE_CAPABILITY_ID)?,
            CapabilityId::new(MEMORY_READ_CAPABILITY_ID)?,
            CapabilityId::new(MEMORY_TREE_CAPABILITY_ID)?,
            // profile_set writes to the memory mount (context/profile.json under
            // the user-scoped scope), so it needs the memory mount override just
            // like the four memory_* capabilities above.
            CapabilityId::new(PROFILE_SET_CAPABILITY_ID)?,
        ];
        Ok(Self {
            runtime,
            approval_parts: None,
            auto_approve_settings: None,
            pending_approval_scopes: Arc::new(Mutex::new(HashMap::new())),
            io: Arc::new(ProductLiveCapabilityIo::default()),
            root,
            workspace_root,
            mounts,
            capability_mount_overrides: memory_capability_ids
                .iter()
                .cloned()
                .map(|capability_id| (capability_id, memory_mounts.clone()))
                .collect(),
            capability_ids: vec![
                CapabilityId::new(TIME_CAPABILITY_ID)?,
                CapabilityId::new(JSON_CAPABILITY_ID)?,
                CapabilityId::new(HTTP_CAPABILITY_ID)?,
                CapabilityId::new(HTTP_SAVE_CAPABILITY_ID)?,
                CapabilityId::new(MEMORY_SEARCH_CAPABILITY_ID)?,
                CapabilityId::new(MEMORY_WRITE_CAPABILITY_ID)?,
                CapabilityId::new(MEMORY_READ_CAPABILITY_ID)?,
                CapabilityId::new(MEMORY_TREE_CAPABILITY_ID)?,
                CapabilityId::new(PROFILE_SET_CAPABILITY_ID)?,
                CapabilityId::new(READ_FILE_CAPABILITY_ID)?,
                CapabilityId::new(APPLY_PATCH_CAPABILITY_ID)?,
                // slice 5: `builtin.shell` on the surface so scripted shell calls
                // route through the process port (recording by default, live via
                // `.with_live_shell()`).
                CapabilityId::new(SHELL_CAPABILITY_ID)?,
            ],
            runtime_kind: RuntimeKind::FirstParty,
            effect_kinds: vec![
                EffectKind::DispatchCapability,
                EffectKind::ReadFilesystem,
                EffectKind::WriteFilesystem,
                EffectKind::Network,
                EffectKind::SpawnProcess,
                // slice 5: `builtin.shell` declares ExecuteCode; the grant's
                // allowed_effects must include it or the authorizer denies the
                // capability before it reaches the process port.
                EffectKind::ExecuteCode,
            ],
            network_policy,
            secrets: Vec::new(),
            provider_id: ExtensionId::new(BUILTIN_FIRST_PARTY_PROVIDER)?,
            additional_provider_trust: Vec::new(),
            user_id,
            invocations: Arc::new(Mutex::new(Vec::new())),
            results: Arc::new(Mutex::new(Vec::new())),
            http_egress: None,
            network_egress: None,
            process_port: None,
            profile_filesystem: None,
            project_service: None,
            skill_activation_source: None,
            attachment_test_support: None,
            outbound_target_tools: None,
            scope_capability_by_run_owner: false,
            product_auth: None,
            tool_permission_overrides: None,
        })
    }

    /// Wires the GitHub first-party WASM capabilities behind `GithubHarnessAuthorizer`.
    /// See `github_issue_tools_with_credential_result` for the credential-injection
    /// coupling this relies on (T0-SECRET-INJECT).
    pub(crate) async fn github_issue_tools() -> HarnessResult<Self> {
        // Credential account resolves to a real handle → capability dispatches.
        Self::github_issue_tools_with_credential_result(Ok(SecretHandle::new(
            "github_manual_access",
        )?))
    }

    /// E-AUTHGATE: the GitHub extension wired so its credential account resolver
    /// returns `AuthRequired`, raising a `TurnStatus::BlockedAuth` gate when a
    /// `github.*` capability is dispatched. Used by `RebornIntegrationGroup::live_auth_gate`.
    pub(crate) async fn github_issue_tools_auth_required() -> HarnessResult<Self> {
        Self::github_issue_tools_with_credential_result(Err(CredentialStageError::AuthRequired))
    }

    /// Shared GitHub-extension constructor (E-AUTHGATE): the only difference
    /// between the happy-path and auth-blocked variants is the credential account
    /// resolver result, so the full `Self {..}` literal lives here once.
    ///
    /// **Credential injection runs through two mechanisms here, not one — worth
    /// knowing before you change either.** The authorizer's
    /// `InjectCredentialAccountOnce` obligation is one path. The
    /// `local_dev_host_runtime_with_registry_and_egress` helper this calls into
    /// separately auto-wires `SharedHostWasmRuntimeCredentials` with product-auth
    /// restaging via `try_with_wasm_runtime` (since both `.with_secret_store` and
    /// `.with_runtime_credential_account_resolver` are always set on that path),
    /// which independently resolves the GitHub manifest's declared
    /// `runtime_credentials` and stages the same secret. That staging path runs
    /// unconditionally on every WASM HTTP call (`WasmRuntimeHttpAdapter::request`)
    /// — it is not gated on the authorizer's `Decision`. So a test asserting on the
    /// injected header proves the *end-to-end* wire outcome, not that the
    /// authorizer's obligation specifically is the sole producer of the header.
    ///
    /// As currently wired (manually verified once, not re-checked by CI — treat as
    /// current-harness observation, not a guaranteed contract): removing the
    /// obligation does not make the call fall back to an unauthenticated request;
    /// the run instead hangs and never reaches `Completed`. That's why the
    /// mutation-verify in `reborn_integration_secret_injection.rs` proves the
    /// obligation's secret reaches the wire by flipping the secret *value* (a fast,
    /// specific assertion failure) rather than by removing the obligation (which
    /// would only yield a slow, ambiguous timeout — a poor mutation-test signal).
    fn github_issue_tools_with_credential_result(
        credential_account_result: Result<SecretHandle, CredentialStageError>,
    ) -> HarnessResult<Self> {
        let root = Arc::new(tempfile::tempdir()?);
        let storage_root = root.path().join("local-dev");
        let workspace_root = storage_root.join("workspace");
        std::fs::create_dir_all(&workspace_root)?;
        let github_fixture_response =
            br#"{"object":{"sha":"abc123def4567890abc123def4567890abc123de"},"ok":true}"#.to_vec();
        let runtime_http_egress = Arc::new(RecordingRuntimeHttpEgress::with_body(
            github_fixture_response.clone(),
        ));
        let network_egress = Arc::new(RecordingNetworkHttpEgress::with_body(
            github_fixture_response,
        ));
        let runtime = local_dev_host_runtime_with_registry_and_egress(
            storage_root.clone(),
            github_support::extension_registry()?,
            runtime_http_egress.clone(),
            network_egress.clone(),
            credential_account_result,
        )?;
        let mounts = workspace_mounts(MountPermissions::read_write_list_delete())?;
        Ok(Self {
            runtime,
            approval_parts: None,
            auto_approve_settings: None,
            pending_approval_scopes: Arc::new(Mutex::new(HashMap::new())),
            io: Arc::new(ProductLiveCapabilityIo::default()),
            root,
            workspace_root,
            mounts,
            capability_mount_overrides: Vec::new(),
            capability_ids: github_support::capability_ids()?,
            runtime_kind: RuntimeKind::Wasm,
            effect_kinds: github_support::effect_kinds(),
            network_policy: github_support::api_policy(),
            secrets: github_support::secret_handles()?,
            provider_id: github_support::provider_id()?,
            additional_provider_trust: Vec::new(),
            user_id: UserId::new("reborn-e2e-github-user")?,
            invocations: Arc::new(Mutex::new(Vec::new())),
            results: Arc::new(Mutex::new(Vec::new())),
            http_egress: Some(runtime_http_egress),
            network_egress: Some(network_egress),
            process_port: None,
            profile_filesystem: None,
            project_service: None,
            skill_activation_source: None,
            attachment_test_support: None,
            outbound_target_tools: None,
            scope_capability_by_run_owner: false,
            product_auth: None,
            tool_permission_overrides: None,
        })
    }

    /// Slice 6: wire a single MCP capability backed by the loopback mock server.
    ///
    /// `mcp_url`  — the mock server's MCP endpoint (e.g. `"http://127.0.0.1:PORT/mcp"`).
    /// `provider_id`   — extension id used in the registry (e.g. `"mock-mcp"`).
    /// `capability_id` — capability id surfaced to the model (e.g. `"mock-mcp.search"`).
    ///
    /// The harness (via the `harness_mcp` scaffolding) builds a loopback MCP
    /// egress that makes REAL HTTP connections to the mock server, injecting a
    /// fake Bearer token to satisfy the mock's auth gate. Production egress
    /// policy, network policy, and credential stores are bypassed — this path is
    /// test-only.
    pub(crate) async fn mock_mcp_tools(
        mcp_url: &str,
        provider_id: &str,
        capability_id: &str,
    ) -> HarnessResult<Self> {
        let (root, storage_root, workspace_root) = host_runtime_storage_roots()?;
        // Recording egress for any first-party tool paths (unused in MCP tests,
        // but HostRuntimeServices requires it when first_party_capabilities are wired).
        let first_party_egress = Arc::new(RecordingRuntimeHttpEgress::with_body(
            br#"{"accepted":true}"#.to_vec(),
        ));
        // Real loopback egress + MCP runtime for the mock MCP server; the
        // scaffolding (egress, adapter chain, runtime) lives in `harness_mcp`.
        let mcp_runtime = build_loopback_mcp_runtime(mcp_url)?;
        let mut registry = ExtensionRegistry::new();
        registry.insert(mock_mcp_extension_package(
            provider_id,
            mcp_url,
            capability_id,
        )?)?;
        let runtime = local_dev_host_runtime_with_registry_egress_and_mcp(
            storage_root,
            registry,
            Arc::clone(&first_party_egress),
            mcp_runtime,
            provider_id,
        )?;
        let mounts = workspace_mounts(MountPermissions::read_write_list_delete())?;
        Ok(Self {
            runtime,
            approval_parts: None,
            auto_approve_settings: None,
            pending_approval_scopes: Arc::new(Mutex::new(HashMap::new())),
            io: Arc::new(ProductLiveCapabilityIo::default()),
            root,
            workspace_root,
            mounts,
            capability_mount_overrides: Vec::new(),
            capability_ids: vec![CapabilityId::new(capability_id)?],
            runtime_kind: RuntimeKind::Mcp,
            effect_kinds: vec![EffectKind::DispatchCapability, EffectKind::Network],
            // The MCP capability declares `EffectKind::Network`, so authorization
            // attaches an `ApplyNetworkPolicy` obligation that the host runtime
            // rejects when `allowed_targets` is empty (a default `NetworkPolicy`).
            // The mock server lives at `http://127.0.0.1:<port>/mcp`, so permit the
            // loopback host (and disable the private-IP denial that would otherwise
            // block 127.0.0.1) so the MCP egress reaches the loopback server.
            network_policy: mcp_loopback_network_policy(),
            secrets: Vec::new(),
            provider_id: ExtensionId::new(provider_id)?,
            additional_provider_trust: Vec::new(),
            user_id: UserId::new("reborn-itest-mcp-user")?,
            invocations: Arc::new(Mutex::new(Vec::new())),
            results: Arc::new(Mutex::new(Vec::new())),
            http_egress: None,
            network_egress: None,
            process_port: None,
            profile_filesystem: None,
            project_service: None,
            skill_activation_source: None,
            attachment_test_support: None,
            outbound_target_tools: None,
            scope_capability_by_run_owner: false,
            product_auth: None,
            tool_permission_overrides: None,
        })
    }

    /// C-WEBACCESS: wires the real first-party `web-access.search` /
    /// `web-access.get_content` capabilities via the production
    /// `register_bundled_web_access_first_party_handlers` registration
    /// (`harness_web_access.rs`), which dispatches through the same
    /// `WebAccessExecutor` production composition uses. Unlike
    /// `github_issue_tools`, no credential-injecting authorizer is needed —
    /// web-access declares zero `runtime_credentials` — so this wires the
    /// plain default `GrantAuthorizer`.
    ///
    /// The three-leg Exa MCP handshake (`initialize` → `notifications/initialized`
    /// → `tools/call`) all target the same URL, so script it via
    /// `RecordingRuntimeHttpEgress::push_response_body` (FIFO), not the keyed
    /// matcher — see [`install_web_access_responses`](Self::install_web_access_responses),
    /// called from `RebornIntegrationHarnessBuilder::build` before the harness
    /// is returned.
    pub(crate) async fn web_access_tools() -> HarnessResult<Self> {
        let (root, storage_root, workspace_root) = host_runtime_storage_roots()?;
        let http_egress = Arc::new(RecordingRuntimeHttpEgress::with_body(
            br#"{"accepted":true}"#.to_vec(),
        ));
        let mut registry = ExtensionRegistry::new();
        registry.insert(harness_web_access::web_access_extension_package()?)?;
        let runtime = harness_web_access::local_dev_host_runtime_with_web_access(
            storage_root,
            registry,
            Arc::clone(&http_egress),
        )?;
        let mounts = workspace_mounts(MountPermissions::read_write_list_delete())?;
        Ok(Self {
            runtime,
            approval_parts: None,
            auto_approve_settings: None,
            pending_approval_scopes: Arc::new(Mutex::new(HashMap::new())),
            io: Arc::new(ProductLiveCapabilityIo::default()),
            root,
            workspace_root,
            mounts,
            capability_mount_overrides: Vec::new(),
            capability_ids: vec![
                CapabilityId::new(WEB_SEARCH_CAPABILITY_ID)?,
                CapabilityId::new(WEB_GET_CONTENT_CAPABILITY_ID)?,
            ],
            runtime_kind: RuntimeKind::FirstParty,
            effect_kinds: vec![EffectKind::DispatchCapability, EffectKind::Network],
            network_policy: harness_web_access::exa_mcp_test_network_policy(),
            secrets: Vec::new(),
            provider_id: ExtensionId::new(harness_web_access::WEB_ACCESS_PROVIDER_ID)?,
            additional_provider_trust: Vec::new(),
            user_id: UserId::new("reborn-itest-web-access-user")?,
            invocations: Arc::new(Mutex::new(Vec::new())),
            results: Arc::new(Mutex::new(Vec::new())),
            http_egress: Some(http_egress),
            network_egress: None,
            process_port: None,
            profile_filesystem: None,
            project_service: None,
            skill_activation_source: None,
            attachment_test_support: None,
            outbound_target_tools: None,
            scope_capability_by_run_owner: false,
            product_auth: None,
            tool_permission_overrides: None,
        })
    }

    pub(crate) fn capability_factory(
        self: &Arc<Self>,
        milestone_sink: Arc<ironclaw_turns::run_profile::InMemoryLoopHostMilestoneSink>,
    ) -> Arc<dyn LoopCapabilityPortFactory> {
        Arc::new(HostRuntimeHarnessCapabilityPortFactory {
            harness: Arc::clone(self),
            milestone_sink,
        })
    }

    pub(crate) fn capability_result_writer(
        self: &Arc<Self>,
    ) -> Arc<dyn LoopCapabilityResultWriter> {
        Arc::new(RecordingCapabilityResultWriter {
            inner: self.io.clone(),
            results: Arc::clone(&self.results),
        })
    }

    fn invocations(&self) -> Vec<CapabilityInvocation> {
        self.invocations.lock().unwrap().clone()
    }

    pub(crate) fn capability_results(&self) -> Vec<RecordedCapabilityResult> {
        self.results.lock().unwrap().clone()
    }

    pub(crate) fn runtime_http_requests(&self) -> Vec<RuntimeHttpEgressRequest> {
        self.http_egress
            .as_ref()
            .map(|egress| egress.requests())
            .unwrap_or_default()
    }

    /// Install FIFO response bodies (C-WEBACCESS) onto the recording runtime
    /// HTTP egress, consumed in call order ahead of the default body. Mirrors
    /// [`install_http_responses`](Self::install_http_responses)'s shape but for
    /// the web-access backend's `push_response_body` FIFO queue rather than the
    /// keyed matcher — the three-leg Exa MCP handshake (`initialize` →
    /// `notifications/initialized` → `tools/call`) all target the same
    /// URL/method/capability, so only the FIFO queue can script them
    /// independently. Errors if this harness wired no recording egress. Called
    /// from `RebornIntegrationHarnessBuilder::build` (build-time only — no
    /// post-build mutation).
    pub(crate) fn install_web_access_responses(
        &self,
        bodies: impl IntoIterator<Item = Vec<u8>>,
    ) -> HarnessResult<()> {
        let egress = self
            .http_egress
            .as_ref()
            .ok_or("web-access host runtime has no recording egress wired")?;
        for body in bodies {
            egress.push_response_body(body);
        }
        Ok(())
    }

    /// Snapshot of every command string recorded by the inert process port
    /// (slice 5). Empty when the harness uses the live `LocalHostProcessPort`
    /// (`.with_live_shell()` path) or predates slice 5.
    pub(crate) fn process_commands(&self) -> Vec<String> {
        self.process_port
            .as_ref()
            .map(|port| port.commands())
            .unwrap_or_default()
    }

    /// Install URL/method/capability-keyed scripted responses into the recording
    /// HTTP egress (§3.6 P1 ergonomics). Errors if this harness wired no
    /// recording egress (e.g. the live-HTTP variant).
    pub(crate) fn install_http_responses(
        &self,
        responses: impl IntoIterator<Item = super::http_matcher::ScriptedHttpResponse>,
    ) -> HarnessResult<()> {
        self.http_egress
            .as_ref()
            .ok_or("host runtime harness has no recording http egress to script")?
            .install_scripted(responses);
        Ok(())
    }

    /// W4-AUTHGATE-WIRE: enqueue a FIFO scripted status on the recording
    /// **network** HTTP egress (see `RecordingNetworkHttpEgress::push_status`).
    /// For `GithubIssueTools`-backed harnesses, the real WASM HTTP call flows
    /// through this lane (not `install_http_responses`'s runtime-egress
    /// matcher) — see `reborn_integration_secret_injection.rs`'s module doc.
    /// Errors if this harness wired no recording network egress.
    pub(crate) fn install_network_status_script(&self, status: u16) -> HarnessResult<()> {
        self.network_egress
            .as_ref()
            .ok_or("host runtime harness has no recording network egress to script")?
            .push_status(status);
        Ok(())
    }

    /// Install a sticky scripted `builtin.shell` process result on the inert
    /// recording process port (mirrors `install_http_responses`). Errors if the
    /// harness has no recording port (e.g. the `.with_live_shell()` path).
    pub(crate) fn install_process_script(
        &self,
        result: super::process::ScriptedProcessResult,
    ) -> HarnessResult<()> {
        self.process_port
            .as_ref()
            .ok_or("host runtime harness has no recording process port to script")?
            .set_scripted(result);
        Ok(())
    }

    pub(crate) fn network_http_requests(&self) -> Vec<NetworkHttpRequest> {
        self.network_egress
            .as_ref()
            .map(|egress| egress.requests())
            .unwrap_or_default()
    }

    pub(crate) fn workspace_file_path(&self, relative: &str) -> PathBuf {
        self.workspace_root.join(relative.trim_start_matches('/'))
    }

    pub(crate) async fn approve_local_dev_gate(&self, gate_ref: &GateRef) -> HarnessResult<()> {
        let approval_parts = self
            .approval_parts
            .as_ref()
            .ok_or("host runtime harness has no local-dev approval stores")?;
        let request_id = approval_request_id_from_gate_ref(gate_ref)?;
        let scope = self
            .pending_approval_scopes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&request_id)
            .cloned()
            .ok_or("approval gate was not recorded by the host runtime harness")?;
        let record = approval_parts
            .approval_requests
            .get(&scope, request_id)
            .await?
            .ok_or("approval request was not persisted")?;
        let capability = match record.request.action.as_ref() {
            Action::Dispatch { capability, .. } | Action::SpawnCapability { capability, .. } => {
                capability.clone()
            }
            other => return Err(format!("unsupported approval action: {other:?}").into()),
        };
        let approval = self.lease_approval_for(&capability);
        let resolver = ApprovalResolver::new(
            approval_parts.approval_requests.as_ref(),
            approval_parts.capability_leases.as_ref(),
        );
        match record.request.action.as_ref() {
            Action::Dispatch { .. } => {
                resolver
                    .approve_dispatch(&scope, request_id, approval)
                    .await?;
            }
            Action::SpawnCapability { .. } => {
                resolver.approve_spawn(&scope, request_id, approval).await?;
            }
            other => return Err(format!("unsupported approval action: {other:?}").into()),
        }
        Ok(())
    }

    /// Deny a pending local-dev approval gate (the model-declined path). Mirrors
    /// [`approve_local_dev_gate`](Self::approve_local_dev_gate) but resolves the
    /// persisted request to `Denied` (no lease issued) via `ApprovalResolver::deny`.
    /// The caller then resumes the run with `GateResumeDisposition::Denied` so the
    /// executor surfaces a non-retryable authorization failure to the model.
    pub(crate) async fn deny_local_dev_gate(&self, gate_ref: &GateRef) -> HarnessResult<()> {
        let approval_parts = self
            .approval_parts
            .as_ref()
            .ok_or("host runtime harness has no local-dev approval stores")?;
        let request_id = approval_request_id_from_gate_ref(gate_ref)?;
        let scope = self
            .pending_approval_scopes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&request_id)
            .cloned()
            .ok_or("approval gate was not recorded by the host runtime harness")?;
        let resolver = ApprovalResolver::new(
            approval_parts.approval_requests.as_ref(),
            approval_parts.capability_leases.as_ref(),
        );
        resolver
            .deny(
                &scope,
                request_id,
                DenyApproval {
                    denied_by: Principal::User(scope.user_id.clone()),
                },
            )
            .await?;
        Ok(())
    }

    /// The persisted approval-request store, when this harness wires the real
    /// local-dev approval stores (`file_tools_requiring_approval`). The
    /// integration runtime builds an [`ApprovalGateEvidenceStore`] over it so a
    /// `BlockedApproval` run is verified at loop exit (mirrors production
    /// `runtime.rs:2799`) and genuinely pauses instead of failing.
    pub(crate) fn approval_requests_store(
        &self,
    ) -> Option<Arc<dyn ironclaw_run_state::ApprovalRequestStore>> {
        self.approval_parts
            .as_ref()
            .map(|parts| Arc::clone(&parts.approval_requests))
    }

    /// The user id this capability harness's first-party tools execute under.
    /// The dispatch-time auto-approve check is keyed `(tenant, user)` on THIS
    /// user (not the run's binding owner), so the group derives the auto-approve
    /// scope from it — see `GroupSharedStorage::auto_approve_scope`.
    pub(crate) fn user_id(&self) -> &UserId {
        &self.user_id
    }

    /// E-PROFILE: the raw local-dev memory filesystem backing the user-profile
    /// source, for write→read-back assertions on `context/profile.json`. `Some`
    /// only for `new_with_options`-built harnesses. Consumed by the E-PROFILE
    /// `profile_tools()` constructor and the `reborn_integration_profile` test.
    pub(crate) fn profile_filesystem_for_test(&self) -> Option<Arc<dyn RootFilesystem>> {
        self.profile_filesystem.clone()
    }

    /// E-PROJ: the project service backing the synthetic `project_create`
    /// capability, for write→read-back assertions — mirrors
    /// `profile_filesystem_for_test`'s role for E-PROFILE. `Some` only for
    /// `project_tools()`-built harnesses. Lets a test read a created project
    /// back through the SAME `Arc<dyn ProjectService>` instance
    /// `apply_synthetic_capability_wrappers` dispatches writes through, rather
    /// than reconstructing an equivalent (and possibly unwritten) one.
    pub(crate) fn project_service_for_test(&self) -> Option<Arc<dyn ProjectService>> {
        self.project_service.clone()
    }

    /// C-SYNTH outbound: the injected facade double, for read-back that a
    /// `target_set` actually reached the facade seam
    /// (`recorded_set_target_ids`). `Some` only for `outbound_target_tools()`.
    pub(crate) fn outbound_preferences_facade_for_test(
        &self,
    ) -> Option<Arc<super::outbound_preferences::FakeOutboundPreferencesFacade>> {
        self.outbound_target_tools
            .as_ref()
            .map(|parts| Arc::clone(&parts.facade))
    }

    /// C-SYNTH outbound: persist a `Disabled` per-tool permission override for
    /// `outbound_delivery_target_set` under `(tenant, user)`, driving the
    /// handler's settings decision to `Deny` → `Failed{policy_denied}`. The
    /// scope must be the run's EFFECTIVE dispatch user (the thread binding actor,
    /// `harness.binding.actor_user_id`) — the same `(tenant, user)`
    /// `StoreApprovalSettingsProvider::tool_override` reads it back under
    /// (`PersistentApprovalScope` = tenant+user, invocation-independent). `Some`
    /// only for `outbound_target_tools()`.
    pub(crate) async fn disable_outbound_target_set_tool(
        &self,
        tenant_id: TenantId,
        user_id: UserId,
    ) -> HarnessResult<()> {
        let parts = self
            .outbound_target_tools
            .as_ref()
            .ok_or("harness has no outbound_target_tools backing store")?;
        let scope = ResourceScope {
            tenant_id,
            user_id: user_id.clone(),
            agent_id: None,
            project_id: None,
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        };
        parts
            .tool_permission_overrides
            .set(ironclaw_approvals::CapabilityPermissionOverrideInput {
                scope,
                capability_id: CapabilityId::new(
                    ironclaw_reborn_composition::test_support::OUTBOUND_DELIVERY_TARGET_SET_CAPABILITY_ID,
                )?,
                state: ironclaw_approvals::CapabilityPermissionOverride::Disabled,
                updated_by: Principal::User(user_id),
            })
            .await?;
        Ok(())
    }

    /// W4-ASK-EACH-ONCE: install a `ToolPermissionOverride::AskEachTime`
    /// override for `capability_id` under `(tenant_id, user_id)` via the real
    /// local-dev per-tool permission override store -- generalizes
    /// `disable_outbound_target_set_tool`'s shape (same scope-key
    /// convention: `agent_id`/`project_id` unset, matching how
    /// `StoreApprovalSettingsProvider::tool_override` looks the override up)
    /// to any host-runtime-backed harness/group, not just
    /// `outbound_target_tools()`. Drives the SAME
    /// `require_approval_for_profile_policy` `tool_override` consultation
    /// the #5306 fix reordered relative to the one-shot approval-lease check.
    /// Errors if this harness wired no local-dev tool-permission-override
    /// store (i.e. not built via `new_with_options`).
    pub(crate) async fn set_ask_each_time_override_for_test(
        &self,
        capability_id: &CapabilityId,
        tenant_id: TenantId,
        user_id: UserId,
    ) -> HarnessResult<()> {
        let store = self
            .tool_permission_overrides
            .as_ref()
            .ok_or("harness has no local-dev tool-permission-override store")?;
        let scope = ResourceScope {
            tenant_id,
            user_id: user_id.clone(),
            agent_id: None,
            project_id: None,
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        };
        store
            .set(ironclaw_approvals::CapabilityPermissionOverrideInput {
                scope,
                capability_id: capability_id.clone(),
                state: ironclaw_approvals::CapabilityPermissionOverride::AskEachTime,
                updated_by: Principal::User(user_id),
            })
            .await?;
        Ok(())
    }

    /// E-SKILL: the `HostSkillContextSource` to wire as the runtime's
    /// `skill_context_source` in `into_group`, so activated-skill instructions
    /// inject into the model request. `Some` only for `skill_activation_tools()`.
    pub(crate) fn skill_context_source_for_test(
        &self,
    ) -> Option<Arc<dyn ironclaw_loop_support::HostSkillContextSource>> {
        self.skill_activation_source
            .as_ref()
            .map(|source| source.context_source())
    }

    /// E-DURABLE: the on-disk local-dev storage root this harness's capability
    /// stores persist under (`<tempdir>/local-dev`). Mirrors the `storage_root`
    /// computed inline in `new_with_options`. A durability test reopens a fresh,
    /// independent store at this path (see
    /// `open_local_dev_extension_installation_store_for_test`) to prove capability
    /// state survives a reopen, paralleling `assert_reply_persists_after_reopen`.
    /// Tests only.
    pub(crate) fn storage_root_for_test(&self) -> PathBuf {
        self.root.path().join("local-dev")
    }

    /// C-DURABLE: resolve `gate_ref` (a `"gate:approval-<id>"` local-dev
    /// approval gate) to the `(ApprovalRequestId, ResourceScope)` pair a fresh,
    /// independently-reopened `ApprovalRequestStore::get`/`read_versioned` call
    /// needs. Reuses the SAME private lookup `approve_local_dev_gate`/
    /// `deny_local_dev_gate` already use (`approval_request_id_from_gate_ref` +
    /// `pending_approval_scopes`) so a durability test's scope construction can
    /// never drift from the live approve/deny path. Tests only.
    pub(crate) fn approval_request_scope_for_test(
        &self,
        gate_ref: &GateRef,
    ) -> HarnessResult<(ApprovalRequestId, ResourceScope)> {
        let request_id = approval_request_id_from_gate_ref(gate_ref)?;
        let scope = self
            .pending_approval_scopes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&request_id)
            .cloned()
            .ok_or("approval gate was not recorded by the host runtime harness")?;
        Ok((request_id, scope))
    }

    /// E-SKILL: seed a system-scoped skill on this harness's on-disk skill
    /// filesystem so the model can activate it (`skill_activate`/`$name`). Writes
    /// `<storage_root>/system/skills/<name>/SKILL.md` — the system bundle root is
    /// always present in the skills extension's roots regardless of the run's
    /// tenant/user (`FirstPartySkillsExtensionHandles::bundle_roots`), so both
    /// `activate_skills_for_run` (the `skill_activate` capability) and the
    /// runtime's `skill_context_source` resolve it deterministically without
    /// depending on the harness's run-scope owner resolution. User-scoped skill
    /// filesystem resolution is already covered by the runtime.rs suite; this
    /// seam only needs the skill to exist so the capability + context wiring can
    /// be driven. Mirrors the runtime-test system-skill layout
    /// (runtime.rs `system/skills/<name>/SKILL.md`). Tests only.
    pub(crate) fn seed_system_skill_for_test(
        &self,
        name: &str,
        description: &str,
        prompt: &str,
    ) -> HarnessResult<()> {
        let dir = self
            .storage_root_for_test()
            .join("system")
            .join("skills")
            .join(name);
        std::fs::create_dir_all(&dir)?;
        let body = format!(
            "---\nname: {name}\ndescription: {description}\nactivation:\n  keywords: [\"{name}\"]\n---\n\n{prompt}"
        );
        std::fs::write(dir.join("SKILL.md"), body)?;
        Ok(())
    }

    /// C-SYNTH `skill_activate` `AmbiguousSkill` seeding arm: seed a
    /// USER-scoped skill (writes
    /// `<storage_root>/tenants/<tenant>/users/<user>/skills/<name>/SKILL.md`,
    /// mirroring the runtime.rs composition-test layout) so a name shared with
    /// a system-scoped skill (`seed_system_skill_for_test`) resolves to TWO
    /// Trusted candidates under different `SkillSourceKind`s — `System` root
    /// and `User` root both default to `SkillTrust::Trusted`
    /// (`FilesystemSkillBundleRoot::system`/`user`,
    /// `crates/ironclaw_loop_support/src/filesystem_skill_bundle_source.rs`),
    /// so `select_named_skill_activations`'s `active_candidates` filter
    /// (`trust == Trusted`) admits both, and
    /// `validate_explicit_mentions_are_unambiguous` then rejects the shared
    /// name as `SkillActivationSelectionError::AmbiguousSkill`. `tenant`/`user`
    /// must be the SAME `(tenant, actor_user_id)` the driving thread's run
    /// resolves under (`harness.binding`), or the user root never matches the
    /// run's own scoped `/skills` mount. Tests only.
    pub(crate) fn seed_user_skill_for_test(
        &self,
        tenant: &TenantId,
        user: &UserId,
        name: &str,
        description: &str,
        prompt: &str,
    ) -> HarnessResult<()> {
        let dir = self
            .storage_root_for_test()
            .join("tenants")
            .join(tenant.as_str())
            .join("users")
            .join(user.as_str())
            .join("skills")
            .join(name);
        std::fs::create_dir_all(&dir)?;
        let body = format!(
            "---\nname: {name}\ndescription: {description}\nactivation:\n  keywords: [\"{name}\"]\n---\n\n{prompt}"
        );
        std::fs::write(dir.join("SKILL.md"), body)?;
        Ok(())
    }

    /// C-ATTACH: the attachment read port + inbound lander over this harness's
    /// local-dev workspace filesystem, for wiring `DefaultPlannedRuntimeParts.attachment_read_port`
    /// and `DefaultInboundTurnService::with_inbound_attachments` — mirrors
    /// `profile_filesystem_for_test`'s role for E-PROFILE.
    pub(crate) fn attachment_test_support_for_test(
        &self,
    ) -> Option<ironclaw_reborn_composition::AttachmentTestSupport> {
        self.attachment_test_support.clone()
    }

    /// E-PROJ: wrap `port` with the local-dev synthetic capabilities this harness
    /// surfaces, in one linear step (keeps the capability-specific knowledge out
    /// of `create_capability_port`'s main assembly chain).
    ///
    /// Partial synthetic wrap: `project_create` (E-PROJ), `skill_activate`
    /// (E-SKILL), and the two `outbound_delivery_*` capabilities (C-SYNTH
    /// outbound), each layered independently when this harness holds the backing
    /// handle, so other local-dev groups are unaffected. See
    /// `LocalDevCapabilityPortFactory::build_inner()` for the full production set.
    pub(crate) fn apply_synthetic_capability_wrappers(
        &self,
        port: Arc<dyn LoopCapabilityPort>,
        run_context: &LoopRunContext,
        input_resolver: Arc<dyn ironclaw_loop_support::LoopCapabilityInputResolver>,
        result_writer: Arc<dyn LoopCapabilityResultWriter>,
    ) -> Result<Arc<dyn LoopCapabilityPort>, AgentLoopHostError> {
        let mut port = port;
        // project_create (E-PROJ): wrapped only for `project_tools`.
        if let Some(project_service) = &self.project_service
            && self.capability_ids.iter().any(|id| {
                id.as_str()
                    == ironclaw_reborn_composition::test_support::PROJECT_CREATE_CAPABILITY_ID
            })
        {
            port =
                ironclaw_reborn_composition::test_support::wrap_project_create_capability_for_test(
                    port,
                    Arc::clone(project_service),
                    self.user_id.clone(),
                    run_context.clone(),
                    input_resolver.clone(),
                    result_writer.clone(),
                )?;
        }
        // outbound_delivery_* (C-SYNTH outbound): wrapped only for
        // `outbound_target_tools`. The facade double is injected at the
        // production-wired trait seam; the settings/approval stores are the same
        // ones `outbound_delivery_capabilities` consumes in production (the
        // auto-approve + approval-request/lease stores are reused from the
        // sibling `auto_approve_settings` / `approval_parts` harness fields).
        if let Some(parts) = &self.outbound_target_tools {
            let approval = self.approval_parts.as_ref().ok_or_else(|| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Internal,
                    "outbound_target_tools requires local-dev approval stores",
                )
            })?;
            let auto_approve = self.auto_approve_settings.clone().ok_or_else(|| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Internal,
                    "outbound_target_tools requires the local-dev auto-approve store",
                )
            })?;
            // Record the synthetic capability's approval scope into
            // `pending_approval_scopes` (the host-runtime recorder can't see a
            // port-level synthetic gate) so `approve_local_dev_gate` /
            // `deny_local_dev_gate` resolve it; delegates to the inner store the
            // evidence/approve/deny paths read.
            let recording_approval_requests: Arc<dyn ironclaw_run_state::ApprovalRequestStore> =
                Arc::new(RecordingApprovalRequestStore {
                    inner: Arc::clone(&approval.approval_requests),
                    pending_approval_scopes: Arc::clone(&self.pending_approval_scopes),
                });
            port =
                ironclaw_reborn_composition::test_support::wrap_outbound_delivery_capabilities_for_test(
                    port,
                    ironclaw_reborn_composition::test_support::OutboundDeliveryCapabilityTestParts {
                        facade: Arc::clone(&parts.facade)
                            as Arc<dyn ironclaw_product_workflow::OutboundPreferencesProductFacade>,
                        fallback_user_id: self.user_id.clone(),
                        approval_requests: recording_approval_requests,
                        capability_leases: Arc::clone(&approval.capability_leases),
                        tool_permission_overrides: Arc::clone(&parts.tool_permission_overrides),
                        auto_approve,
                        persistent_policies: Arc::clone(&parts.persistent_approval_policies),
                        target_set_requires_approval: parts.requires_approval,
                        run_context: run_context.clone(),
                        input_resolver: input_resolver.clone(),
                        result_writer: result_writer.clone(),
                    },
                )?;
        }
        // skill_activate (E-SKILL): wrapped only for `skill_activation_tools`.
        if let Some(skill_source) = &self.skill_activation_source
            && self.capability_ids.iter().any(|id| {
                id.as_str()
                    == ironclaw_reborn_composition::test_support::SKILL_ACTIVATE_CAPABILITY_ID
            })
        {
            port =
                ironclaw_reborn_composition::test_support::wrap_skill_activation_capability_for_test(
                    port,
                    skill_source,
                    run_context.clone(),
                    input_resolver,
                    result_writer,
                )?;
        }
        Ok(port)
    }

    /// Override the user this capability harness executes first-party tools under.
    /// The dispatch ResourceScope, approval-request persistence, auto-approve
    /// keying, and the approval-gate-evidence lookup are ALL keyed on this user
    /// (`HostRuntimeHarnessCapabilityPortFactory` builds the authority from
    /// `self.user_id`). The integration harness sets it to the run's binding owner
    /// so capability dispatch and the turn run under the SAME `(tenant, user)` —
    /// matching production (where the run owner *is* the capability user) instead
    /// of the constructor's fixed test user. Without this, a `BlockedApproval`
    /// run's request persists under the capability user but the gate-evidence
    /// lookup uses the turn owner, so the gate is never verified and the run goes
    /// terminal `Failed`.
    pub(crate) fn with_user_id(mut self, user_id: UserId) -> Self {
        self.user_id = user_id;
        self
    }

    /// C-MULTIUSER: opt in to per-actor capability scoping. With this set,
    /// [`create_capability_port`] resolves the execution `(tenant, user)` from
    /// each run's OWN owner/actor rather than this harness's single fixed
    /// `user_id` — so N actors sharing one capability backend dispatch under N
    /// distinct scopes. See [`scope_capability_by_run_owner`] and
    /// [`dispatch_user_for_run`]. Enabled only by the multiuser group
    /// constructors; every other harness keeps the legacy fixed-user behavior.
    pub(crate) fn with_run_owner_scoped_capability_dispatch(mut self) -> Self {
        self.scope_capability_by_run_owner = true;
        self
    }

    /// The capability-execution `UserId` for one run. Mirrors production
    /// `local_dev_visible_capability_request`'s owner→actor→fallback resolution
    /// (`runtime/local_dev.rs`): when [`scope_capability_by_run_owner`] is set,
    /// prefer the run scope's explicit owner, then the run actor, then fall back
    /// to the fixed harness `user_id`. Without the flag, always the fixed
    /// `user_id` (legacy behavior — every existing test unaffected).
    pub(crate) fn dispatch_user_for_run(&self, run_context: &LoopRunContext) -> UserId {
        if self.scope_capability_by_run_owner {
            run_context
                .scope
                .explicit_owner_user_id()
                .cloned()
                .or_else(|| run_context.actor().map(|actor| actor.user_id.clone()))
                .unwrap_or_else(|| self.user_id.clone())
        } else {
            self.user_id.clone()
        }
    }

    fn lease_approval_for(&self, capability_id: &CapabilityId) -> LeaseApproval {
        let mounts = self
            .capability_mount_overrides
            .iter()
            .find(|(override_capability, _)| override_capability == capability_id)
            .map(|(_, mounts)| mounts.clone())
            .unwrap_or_else(|| self.mounts.clone());
        LeaseApproval {
            issued_by: Principal::HostRuntime,
            constraints: GrantConstraints {
                allowed_effects: self.effect_kinds.clone(),
                mounts,
                network: self.network_policy.clone(),
                secrets: self.secrets.clone(),
                resource_ceiling: None,
                expires_at: None,
                max_invocations: Some(1),
            },
        }
    }
}

fn approval_request_id_from_gate_ref(gate_ref: &GateRef) -> HarnessResult<ApprovalRequestId> {
    const APPROVAL_GATE_PREFIX: &str = "gate:approval-";
    let value = gate_ref
        .as_str()
        .strip_prefix(APPROVAL_GATE_PREFIX)
        .ok_or("gate ref is not a local-dev approval gate")?;
    Ok(ApprovalRequestId::parse(value)?)
}

async fn seed_extension_lifecycle_credentials(
    services: &ironclaw_reborn_composition::RebornServices,
    user_id: &UserId,
) -> HarnessResult<()> {
    let product_auth = services
        .product_auth
        .as_ref()
        .ok_or("extension lifecycle harness missing product auth")?;
    let scope = AuthProductScope::credential_owner(
        &ResourceScope {
            tenant_id: TenantId::new("tenant-e2e")?,
            user_id: user_id.clone(),
            agent_id: Some(AgentId::new("agent-e2e")?),
            project_id: Some(ProjectId::new("project-e2e")?),
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        },
        AuthSurface::Api,
    );
    let accounts = product_auth.credential_account_service();
    for seed in extension_lifecycle_credential_seeds() {
        accounts
            .create_account(NewCredentialAccount {
                scope: scope.clone(),
                provider: AuthProviderId::new(seed.provider)?,
                label: CredentialAccountLabel::new(seed.label)?,
                status: CredentialAccountStatus::Configured,
                ownership: CredentialOwnership::UserReusable,
                owner_extension: None,
                granted_extensions: Vec::new(),
                access_secret: Some(SecretHandle::new(seed.secret_handle)?),
                refresh_secret: None,
                scopes: seed
                    .scopes
                    .iter()
                    .map(|scope| ProviderScope::new(*scope))
                    .collect::<Result<Vec<_>, _>>()?,
            })
            .await?;
    }
    Ok(())
}

struct ExtensionLifecycleCredentialSeed {
    provider: &'static str,
    label: &'static str,
    secret_handle: &'static str,
    scopes: &'static [&'static str],
}

fn extension_lifecycle_credential_seeds() -> &'static [ExtensionLifecycleCredentialSeed] {
    &[
        ExtensionLifecycleCredentialSeed {
            provider: "github",
            label: "qa github",
            secret_handle: "qa_github_access",
            scopes: &[],
        },
        ExtensionLifecycleCredentialSeed {
            provider: "google",
            label: "qa google",
            secret_handle: "qa_google_access",
            scopes: &[
                "https://www.googleapis.com/auth/calendar.events",
                "https://www.googleapis.com/auth/calendar.readonly",
                "https://www.googleapis.com/auth/documents",
                "https://www.googleapis.com/auth/documents.readonly",
                "https://www.googleapis.com/auth/drive",
                "https://www.googleapis.com/auth/drive.readonly",
                "https://www.googleapis.com/auth/gmail.modify",
                "https://www.googleapis.com/auth/gmail.readonly",
                "https://www.googleapis.com/auth/gmail.send",
                "https://www.googleapis.com/auth/presentations",
                "https://www.googleapis.com/auth/presentations.readonly",
                "https://www.googleapis.com/auth/spreadsheets",
                "https://www.googleapis.com/auth/spreadsheets.readonly",
            ],
        },
        ExtensionLifecycleCredentialSeed {
            provider: "nearai",
            label: "qa nearai",
            secret_handle: "qa_nearai_access",
            scopes: &[],
        },
        ExtensionLifecycleCredentialSeed {
            provider: "notion",
            label: "qa notion",
            secret_handle: "qa_notion_access",
            scopes: &[],
        },
    ]
}

pub(crate) fn product_scope() -> ResourceScope {
    test_product_scope("tenant-e2e", "host-user", "agent-e2e", Some("project-e2e"))
}

pub fn test_product_scope(
    tenant_id: &str,
    host_user_id: &str,
    agent_id: &str,
    project_id: Option<&str>,
) -> ResourceScope {
    resource_scope(
        TenantId::new(tenant_id).expect("valid tenant"),
        UserId::new(host_user_id).expect("valid user"),
        AgentId::new(agent_id).expect("valid agent"),
        project_id.map(|id| ProjectId::new(id).expect("valid project")),
    )
}

pub(crate) fn scoped_turns_fs(
    backend: Arc<HarnessTurnStorageBackend>,
    binding: &ResolvedBinding,
) -> HarnessResult<Arc<ScopedFilesystem<HarnessTurnBackend>>> {
    // Include agent_id and project_id in the path when present so that
    // distinct agents or projects stored under the same tenant/user
    // (e.g. shared-storage multi-harness tests) get isolated turn state
    // files and cannot cross-claim each other's queued runs.
    // The 4-arm match lives in `super::filesystem::turns_scope_path`; the
    // integration tier reuses it with a different prefix via
    // `scoped_turns_fs_composite` in builder.rs.
    let target = super::filesystem::turns_scope_path("/engine", binding);
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/turns").expect("valid turns alias"),
        VirtualPath::new(target).expect("valid turns target"),
        MountPermissions::read_write_list_delete(),
    )])?;
    Ok(Arc::new(ScopedFilesystem::with_fixed_view(
        turn_state_root_filesystem(backend)?,
        mounts,
    )))
}

fn turn_state_root_filesystem(
    backend: Arc<HarnessTurnStorageBackend>,
) -> HarnessResult<Arc<HarnessTurnBackend>> {
    let mut root = CompositeRootFilesystem::new();
    root.mount(
        local_dev_mount_descriptor(
            "/engine",
            "reborn-harness-turn-state",
            BackendKind::MemoryDocuments,
            StorageClass::StructuredRecords,
            ContentKind::StructuredRecord,
            IndexPolicy::NotIndexed,
            backend.capabilities(),
        )?,
        backend,
    )?;
    Ok(Arc::new(root))
}
