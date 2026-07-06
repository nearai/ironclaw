//! `HostRuntimeCapabilityHarness` — the integration-tier harness that
//! assembles real host-runtime capability wiring (a genuine `HostRuntime`,
//! real mounts, real capability dispatch) over recorded test doubles
//! substituted at the port boundaries (HTTP/network egress, process,
//! approval). See `tests/integration/CLAUDE.md` for the `harness/` file
//! split and the single-fake-at-the-vendor-SDK-seam contract it implements.

#![allow(dead_code)] // Shared by staged Reborn binary-E2E validation ports.

pub(crate) mod assembly;
pub(crate) mod options;
pub(crate) mod profiles;
pub(crate) mod recorder;

pub(crate) use options::HostRuntimeHarnessOptions;
pub(crate) use recorder::{HarnessCapabilityRecorder, RecordedCapabilityResult};

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use super::{filesystem::BlockingTurnStatePutFilesystem, product_workflow::resource_scope};
use ironclaw_approvals::{ApprovalResolver, AutoApproveSettingInput, DenyApproval, LeaseApproval};
use ironclaw_auth::{AuthProductScope, AuthProviderId, AuthSurface, CredentialAccountLabel};
use ironclaw_filesystem::{
    BackendKind, CompositeRootFilesystem, ContentKind, InMemoryBackend, IndexPolicy,
    RootFilesystem, ScopedFilesystem, StorageClass,
};
use ironclaw_host_api::{
    Action, AgentId, ApprovalRequestId, CapabilityGrant, CapabilityGrantId, CapabilityId,
    CapabilitySet, EffectKind, ExtensionId, GrantConstraints, InvocationId, MountAlias, MountGrant,
    MountPermissions, MountView, NetworkPolicy, Principal, ProjectId, ResourceScope,
    RuntimeHttpEgressRequest, RuntimeKind, SecretHandle, TenantId, TrustClass, UserId, VirtualPath,
};
use ironclaw_host_runtime::{CapabilitySurfacePolicy, HostRuntime, SurfaceKind};
use ironclaw_loop_support::{
    CapabilityAllowSet, CapabilitySurfaceProfileResolver, HostRuntimeLoopCapabilityPortFactory,
    LoopCapabilityPortFactory, LoopCapabilityResultWriter,
};
use ironclaw_network::NetworkHttpRequest;
use ironclaw_product_workflow::{ProjectService, ResolvedBinding};
use ironclaw_reborn_composition::test_support::SkillActivationTestSource;
use ironclaw_reborn_composition::{
    ProductLiveCapabilityIo, ProductLiveVisibleCapabilityRequestConfig, RebornBuildInput,
    RebornLocalDevApprovalTestParts, RebornProductAuthServices, build_reborn_services,
    visible_capability_request_for_run,
};
use ironclaw_trust::EffectiveTrustClass;
use ironclaw_turns::{
    GateRef,
    run_profile::{
        AgentLoopHostError, AgentLoopHostErrorKind, CapabilityInvocation, LoopCapabilityPort,
        LoopHostMilestoneSink, LoopRunContext,
    },
};

pub(crate) use super::doubles::{
    EmptyIdentityContextSource, HarnessCapabilityPortFactory,
    HostRuntimeHarnessCapabilityPortFactory, RecordingApprovalRequestStore,
    RecordingCapabilityResultWriter, RecordingDelegatingCapabilityPort, RecordingHostRuntime,
    RecordingNetworkHttpEgress, RecordingRuntimeHttpEgress, RecordingTestCapabilityPort,
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
    pub(crate) fn exposes_spawn_subagent(&self) -> bool {
        match self {
            Self::Recording(port) => port.exposes_spawn_subagent(),
            Self::HostRuntime(_) => false,
        }
    }

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
    runtime: Arc<dyn HostRuntime>,
    approval_parts: Option<RebornLocalDevApprovalTestParts>,
    auto_approve_settings: Option<Arc<dyn ironclaw_approvals::AutoApproveSettingStore>>,
    pending_approval_scopes: Arc<Mutex<HashMap<ApprovalRequestId, ResourceScope>>>,
    io: Arc<ProductLiveCapabilityIo>,
    root: Arc<tempfile::TempDir>,
    workspace_root: PathBuf,
    mounts: MountView,
    capability_mount_overrides: Vec<(CapabilityId, MountView)>,
    capability_ids: Vec<CapabilityId>,
    runtime_kind: RuntimeKind,
    effect_kinds: Vec<EffectKind>,
    network_policy: NetworkPolicy,
    secrets: Vec<SecretHandle>,
    provider_id: ExtensionId,
    additional_provider_trust: Vec<(ExtensionId, Vec<EffectKind>)>,
    user_id: UserId,
    invocations: Arc<Mutex<Vec<CapabilityInvocation>>>,
    results: Arc<Mutex<Vec<RecordedCapabilityResult>>>,
    http_egress: Option<Arc<RecordingRuntimeHttpEgress>>,
    network_egress: Option<Arc<RecordingNetworkHttpEgress>>,
    /// Inert recording process port. `Some` when the harness injected a
    /// `RecordingProcessPort`; `None` when the live `LocalHostProcessPort` was
    /// used (`.with_live_shell()` path).
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
    /// WebUI-facing `InboundAttachmentReader` view (Enabler C) — a different
    /// trait than `attachment_test_support`'s `LoopAttachmentReadPort`, though
    /// the same concrete reader implements both. `Some` only for
    /// `new_with_options`-built harnesses.
    inbound_attachment_reader: Option<Arc<dyn ironclaw_product_workflow::InboundAttachmentReader>>,
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
    /// Local-dev persistent approval-policy store, captured unconditionally
    /// like `tool_permission_overrides`/`auto_approve_settings` above. `Some`
    /// only for `new_with_options`-built harnesses.
    persistent_approval_policies:
        Option<Arc<dyn ironclaw_approvals::PersistentApprovalPolicyStore>>,
    /// SAME live trigger repository this harness's capability dispatch uses
    /// (Enabler B.3). `Some` only for `new_with_options`-built harnesses.
    /// Read via `trigger_repository_for_test` to wire
    /// `RebornAutomationProductFacade` over the same repo a prior turn used.
    trigger_repository: Option<Arc<dyn ironclaw_triggers::TriggerRepository>>,
}

impl HostRuntimeCapabilityHarness {
    /// C-JOURNEY: seed a real GitHub credential account — WITH real secret
    /// material — through the PRODUCTION manual-token flow
    /// (`request_manual_token_setup` → `submit_manual_token`), so a parked
    /// `github.*` auth gate resolves on re-dispatch AND the WASM capability's
    /// credential obligation can actually stage the token (a bare
    /// `create_account(..)` with a dangling handle clears the gate but then
    /// fails at `stage_credential_material`).
    ///
    /// `scope` MUST be the run's actual dispatch-time `(tenant, user, agent,
    /// project)` — a mismatch silently seeds an account the dispatch-time
    /// lookup never finds, leaving the run stuck at `BlockedAuth` (see
    /// `RebornIntegrationHarness::resolve_auth_gate` for how callers derive it).
    ///
    /// Continuation is `AuthContinuationRef::SetupOnly`: the harness's
    /// `resolve_auth_gate` performs the run resume itself, so this must not
    /// ALSO dispatch a `TurnGateResume` continuation.
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
            profiles::extension::seed_extension_lifecycle_credentials(&services, &user_id).await?;
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
        // W5-WEBUI-API-1 (attachments scenario): capture the WebUI-facing
        // reader view alongside the model-injection one above.
        let inbound_attachment_reader = services.local_dev_inbound_attachment_reader_for_test();
        // W5-WEBUI-API-1 Enabler B.3: capture the SAME live, shared trigger
        // repository the capability dispatch path uses, before
        // `services.host_runtime` is moved out below.
        let trigger_repository = services.local_dev_shared_trigger_repository_for_test();
        // W4-ASK-EACH-ONCE: capture the local-dev per-tool permission override
        // store unconditionally (mirrors `auto_approve_settings` above), not just
        // for `outbound_target_tools()`'s narrower `Some((facade, ..))` arm below
        // -- any host-runtime-backed harness/group can now install a per-capability
        // `AskEachTime` override via `set_ask_each_time_override_for_test`.
        let tool_permission_overrides = services.local_dev_tool_permission_overrides_for_test();
        // W5-WEBUI-API-1 (settings scenario): capture unconditionally, mirroring
        // `tool_permission_overrides` above.
        let persistent_approval_policies =
            services.local_dev_persistent_approval_policies_for_test();
        // C-SYNTH outbound: pair the injected facade double with the local-dev
        // settings stores production's `outbound_delivery_capabilities` consumes,
        // captured from `RebornServices` before the `host_runtime` move. Only
        // `outbound_target_tools()` supplies the facade.
        let outbound_target_tools = match outbound_target_facade {
            Some((facade, requires_approval)) => {
                let tool_permission_overrides = tool_permission_overrides
                    .clone()
                    .ok_or("outbound_target_tools requires a local-dev tool-override store")?;
                let persistent_approval_policies = persistent_approval_policies
                    .clone()
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
            inbound_attachment_reader,
            outbound_target_tools,
            scope_capability_by_run_owner: false,
            product_auth,
            tool_permission_overrides,
            persistent_approval_policies,
            trigger_repository,
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

    /// Snapshot of every command string recorded by the inert process port.
    /// Empty when the harness uses the live `LocalHostProcessPort`
    /// (`.with_live_shell()` path).
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
    /// local-dev per-tool permission override store — generalizes
    /// `disable_outbound_target_set_tool`'s shape to any host-runtime-backed
    /// harness/group. Errors if this harness wired no such store (i.e. not
    /// built via `new_with_options`).
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
    /// `<storage_root>/tenants/<tenant>/users/<user>/skills/<name>/SKILL.md`)
    /// so a name shared with a system-scoped skill
    /// (`seed_system_skill_for_test`) resolves to TWO Trusted candidates
    /// (`System` and `User` roots both default `Trusted`), triggering
    /// `SkillActivationSelectionError::AmbiguousSkill`. `tenant`/`user` must
    /// match the driving thread's run scope (`harness.binding`), or the user
    /// root never matches the run's own `/skills` mount. Tests only.
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

    /// W5-WEBUI-API-1 (attachments cold-GET scenario): the WebUI-facing
    /// `InboundAttachmentReader` view, for wiring
    /// `RebornServices::with_inbound_attachment_reader`.
    pub(crate) fn inbound_attachment_reader_for_test(
        &self,
    ) -> Option<Arc<dyn ironclaw_product_workflow::InboundAttachmentReader>> {
        self.inbound_attachment_reader.clone()
    }

    /// W5-WEBUI-API-1 Enabler B.3: the SAME live, shared trigger repository
    /// this harness's `trigger_create`/`trigger_list` capability dispatch
    /// already uses. `Some` only for `new_with_options`-built harnesses.
    pub(crate) fn trigger_repository_for_test(
        &self,
    ) -> Option<Arc<dyn ironclaw_triggers::TriggerRepository>> {
        self.trigger_repository.clone()
    }

    /// W5-WEBUI-API-1 (settings scenario): local-dev per-tool permission
    /// override store, for wiring `RebornServices::with_operator_approval_config`.
    pub(crate) fn tool_permission_overrides_for_test(
        &self,
    ) -> Option<Arc<dyn ironclaw_approvals::ToolPermissionOverrideStore>> {
        self.tool_permission_overrides.clone()
    }

    /// W5-WEBUI-API-1 (settings scenario): local-dev auto-approve setting
    /// store, for wiring `RebornServices::with_operator_approval_config`.
    pub(crate) fn auto_approve_settings_for_test(
        &self,
    ) -> Option<Arc<dyn ironclaw_approvals::AutoApproveSettingStore>> {
        self.auto_approve_settings.clone()
    }

    /// W5-WEBUI-API-1 (settings scenario): local-dev persistent
    /// approval-policy store, for wiring
    /// `RebornServices::with_operator_approval_config`.
    pub(crate) fn persistent_approval_policies_for_test(
        &self,
    ) -> Option<Arc<dyn ironclaw_approvals::PersistentApprovalPolicyStore>> {
        self.persistent_approval_policies.clone()
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
    fn apply_synthetic_capability_wrappers(
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

    /// Assembles the recording `LoopCapabilityPort` for one run: builds the
    /// authority/grant/visible-capability-request chain over this harness's
    /// fields, wraps it in the real `HostRuntimeLoopCapabilityPortFactory`,
    /// layers the synthetic capability wrappers, and wraps the result in the
    /// invocation-recording port. Owned here (rather than in the
    /// `HostRuntimeHarnessCapabilityPortFactory` test double) because the
    /// assembly reads this harness's fields directly — see
    /// `tests/integration/support/doubles/host_runtime_harness_capability_port_factory.rs`
    /// for the thin `LoopCapabilityPortFactory` delegating wrapper that calls
    /// this method.
    pub(crate) async fn create_recording_capability_port(
        &self,
        run_context: &LoopRunContext,
        milestone_sink: &Arc<ironclaw_turns::run_profile::InMemoryLoopHostMilestoneSink>,
    ) -> Result<Arc<dyn LoopCapabilityPort>, AgentLoopHostError> {
        // C-MULTIUSER: resolve the execution user per run (owner/actor) when the
        // harness opts in, else the fixed harness user. Both the authority scope
        // and the grant grantee MUST use the SAME user so the lease is
        // self-consistent (grantee == execution user) — matching production.
        let dispatch_user = self.dispatch_user_for_run(run_context);
        let mut authority = ProductLiveVisibleCapabilityRequestConfig::new(
            dispatch_user.clone(),
            self.runtime_kind,
            TrustClass::FirstParty,
            SurfaceKind::new("agent_loop").map_err(host_runtime_harness_error)?,
            CapabilitySurfacePolicy::allow_all(),
        )
        .with_mounts(self.mounts.clone())
        .with_grants(capability_grants(
            Principal::User(dispatch_user.clone()),
            &self.capability_ids,
            self.effect_kinds.clone(),
            self.mounts.clone(),
            &self.capability_mount_overrides,
            self.network_policy.clone(),
            self.secrets.clone(),
        ))
        .with_provider_trust_for_effects(
            self.provider_id.clone(),
            EffectiveTrustClass::user_trusted(),
            self.effect_kinds.clone(),
        );
        for (provider, effects) in &self.additional_provider_trust {
            authority = authority.with_provider_trust_for_effects(
                provider.clone(),
                EffectiveTrustClass::user_trusted(),
                effects.clone(),
            );
        }
        let execution_mounts = self.mounts.clone();
        let visible_request = visible_capability_request_for_run(run_context, authority)
            .map_err(host_runtime_harness_error)?;
        let milestone_sink: Arc<dyn LoopHostMilestoneSink> = milestone_sink.clone();
        let result_writer = Arc::new(RecordingCapabilityResultWriter {
            inner: self.io.clone(),
            results: Arc::clone(&self.results),
        });
        let mut factory = HostRuntimeLoopCapabilityPortFactory::new(
            Arc::clone(&self.runtime),
            visible_request,
            self.io.clone(),
            result_writer.clone(),
            milestone_sink,
        )
        .with_execution_mounts(execution_mounts);
        for (capability_id, mounts) in &self.capability_mount_overrides {
            factory =
                factory.with_capability_execution_mount(capability_id.clone(), mounts.clone());
        }
        let port = factory.for_run_context(run_context.clone());
        // E-PROJ: see `apply_synthetic_capability_wrappers`'s doc comment.
        let port = self.apply_synthetic_capability_wrappers(
            port,
            run_context,
            self.io.clone(),
            result_writer,
        )?;
        Ok(Arc::new(RecordingDelegatingCapabilityPort {
            inner: port,
            invocations: Arc::clone(&self.invocations),
        }))
    }

    /// Override the user this capability harness executes first-party tools
    /// under. Dispatch scope, approval persistence, auto-approve keying, and
    /// gate-evidence lookup are ALL keyed on this user. The integration harness
    /// sets it to the run's binding owner so dispatch and the turn share one
    /// `(tenant, user)`, matching production — without this, a
    /// `BlockedApproval` gate's evidence lookup uses the turn owner while the
    /// request persists under the capability user, and the run never verifies.
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
    fn dispatch_user_for_run(&self, run_context: &LoopRunContext) -> UserId {
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

fn capability_grants(
    grantee: Principal,
    capabilities: &[CapabilityId],
    allowed_effects: Vec<EffectKind>,
    mounts: MountView,
    mount_overrides: &[(CapabilityId, MountView)],
    network: NetworkPolicy,
    secrets: Vec<SecretHandle>,
) -> CapabilitySet {
    CapabilitySet {
        grants: capabilities
            .iter()
            .map(|capability| {
                let mounts = mount_overrides
                    .iter()
                    .find(|(override_capability, _mounts)| override_capability == capability)
                    .map(|(_capability, mounts)| mounts.clone())
                    .unwrap_or_else(|| mounts.clone());
                CapabilityGrant {
                    id: CapabilityGrantId::new(),
                    capability: capability.clone(),
                    grantee: grantee.clone(),
                    issued_by: Principal::HostRuntime,
                    constraints: GrantConstraints {
                        allowed_effects: allowed_effects.clone(),
                        mounts,
                        network: network.clone(),
                        secrets: secrets.clone(),
                        resource_ceiling: None,
                        expires_at: None,
                        max_invocations: None,
                    },
                }
            })
            .collect(),
    }
}

fn host_runtime_harness_error(error: impl std::fmt::Display) -> AgentLoopHostError {
    AgentLoopHostError::new(AgentLoopHostErrorKind::InvalidInvocation, error.to_string())
}

fn approval_request_id_from_gate_ref(gate_ref: &GateRef) -> HarnessResult<ApprovalRequestId> {
    const APPROVAL_GATE_PREFIX: &str = "gate:approval-";
    let value = gate_ref
        .as_str()
        .strip_prefix(APPROVAL_GATE_PREFIX)
        .ok_or("gate ref is not a local-dev approval gate")?;
    Ok(ApprovalRequestId::parse(value)?)
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
