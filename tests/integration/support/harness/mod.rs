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
    collections::{BTreeMap, HashMap},
    path::PathBuf,
    sync::{Arc, Mutex},
};

use super::{filesystem::BlockingTurnStatePutFilesystem, product_workflow::resource_scope};
use ironclaw_approvals::{ApprovalResolver, AutoApproveSettingInput, DenyApproval, LeaseApproval};
use ironclaw_auth::{
    AuthProductScope, AuthProviderId, AuthSurface, CredentialAccountLabel, CredentialAccountStatus,
    CredentialOwnership, NewCredentialAccount, ProviderScope,
};
use ironclaw_filesystem::{
    BackendKind, CompositeRootFilesystem, ContentKind, InMemoryBackend, IndexPolicy,
    RootFilesystem, ScopedFilesystem, StorageClass,
};
use ironclaw_host_api::{
    Action, AgentId, ApprovalRequestId, CapabilityGrant, CapabilityGrantId, CapabilityId,
    EffectKind, ExtensionId, GrantConstraints, InvocationId, MountAlias, MountGrant,
    MountPermissions, MountView, NetworkPolicy, Principal, ProjectId, ResourceScope,
    RuntimeHttpEgressRequest, RuntimeKind, SecretHandle, TenantId, UserId, VirtualPath,
};
use ironclaw_host_runtime::HostRuntime;
use ironclaw_loop_host::{
    CapabilityAllowSet, CapabilityResolveError, CapabilitySurfaceProfileResolver,
    LoopCapabilityPortFactory, LoopCapabilityResultWriter,
};
use ironclaw_network::{NetworkHttpRequest, NetworkTransportRequest};
use ironclaw_product_workflow::{ProjectService, ResolvedBinding};
use ironclaw_reborn_composition::test_support::SkillActivationTestSource;
use ironclaw_reborn_composition::{
    ProductLiveCapabilityIo, RebornApprovalTestParts, RebornBuildInput, RebornProductAuthServices,
    build_reborn_services,
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
    HostRuntimeHarnessCapabilityPortFactory, ParkingCapabilityGate, ParkingHostRuntime,
    RecordingCapabilityResultWriter, RecordingDelegatingCapabilityPort, RecordingHostRuntime,
    RecordingNetworkHttpEgress, RecordingNetworkHttpTransport, RecordingRuntimeHttpEgress,
    RecordingTestCapabilityPort, StaticCapabilitySurfaceProfileResolver,
    TriggerActiveRunLookupHostRuntime,
};
pub(crate) use assembly::{
    LocalDevRootMounts, bundled_extension_provider_trust, capability_ids_from_strs,
    copy_dir_recursive, default_capability_io_pair, host_runtime_storage_roots, http_test_policy,
    local_dev_all_effects, local_dev_host_runtime_with_http_egress,
    local_dev_host_runtime_with_live_http_egress, local_dev_host_runtime_with_real_egress_pipeline,
    local_dev_host_runtime_with_registry_and_egress, local_dev_mount_descriptor,
    local_dev_root_filesystem, memory_mounts, qa_smoke_mounts, skill_mounts, wildcard_test_policy,
    workspace_mounts,
};

pub(crate) type HarnessResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;
pub(crate) type HarnessCapabilityParts = (
    Arc<dyn LoopCapabilityPortFactory>,
    Arc<dyn CapabilitySurfaceProfileResolver>,
    Arc<dyn ironclaw_loop_host::LoopCapabilityInputResolver>,
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

    /// `turn_thread_service` is the REAL `SessionThreadService` the caller's
    /// turns dispatch against (`group.rs`'s `group_thread_harness.service`).
    /// Only consumed by the `HostRuntime` arm, and only when that harness was
    /// built with `.with_durable_capability_io()` -- see
    /// `HostRuntimeCapabilityHarness::install_durable_capability_io`'s doc
    /// for why the durable-io swap must happen here rather than at harness
    /// construction (issue #5838).
    ///
    /// `turn_store` is the caller's REAL shared turn-state store
    /// (`group.rs`'s `GroupSharedStorage.turn_store`), consumed only by the
    /// `HostRuntime` arm and only when that harness opted into
    /// `.with_trigger_active_run_lookup_for_test()` — see
    /// `HostRuntimeCapabilityHarness::install_trigger_active_run_lookup_for_test`'s
    /// doc (#5886).
    pub(crate) fn into_parts(
        self,
        milestone_sink: Arc<ironclaw_turns::run_profile::InMemoryLoopHostMilestoneSink>,
        turn_thread_service: Arc<dyn ironclaw_threads::SessionThreadService>,
        turn_store: Arc<ironclaw_turns::FilesystemTurnStateStore<HarnessTurnBackend>>,
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
            Self::HostRuntime(harness) => {
                if harness.durable_capability_io_requested {
                    harness.install_durable_capability_io(turn_thread_service);
                }
                if harness.trigger_active_run_lookup_requested {
                    harness.install_trigger_active_run_lookup_for_test(turn_store)?;
                }
                Ok((
                    harness.capability_factory(milestone_sink),
                    Arc::new(HostRuntimeHarnessSurfaceResolver),
                    harness.input_resolver(),
                    harness.capability_result_writer(),
                    HarnessCapabilityRecorder::HostRuntime(harness),
                ))
            }
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
    /// Interior-mutable (not a plain `Arc`) so `install_trigger_active_run_lookup_for_test`
    /// (#5886) can re-wrap it with `TriggerActiveRunLookupHostRuntime` AFTER
    /// this harness is already `Arc`'d — the same chicken-and-egg constraint
    /// `io`/`result_writer_io` document above `install_durable_capability_io`.
    runtime: Mutex<Arc<dyn HostRuntime>>,
    approval_parts: Option<RebornApprovalTestParts>,
    auto_approve_settings: Option<Arc<dyn ironclaw_approvals::AutoApproveSettingStore>>,
    pending_approval_scopes: Arc<Mutex<HashMap<ApprovalRequestId, ResourceScope>>>,
    /// Input-resolver half of this harness's capability io. Default (every
    /// existing constructor): the ephemeral `ProductLiveCapabilityIo` test
    /// double, both halves coerced from ONE shared object (see
    /// `default_capability_io_pair`). Interior-mutable (not a builder field)
    /// because the durable swap (`install_durable_capability_io`, issue
    /// #5838) can only run once the REAL group thread service exists, which
    /// is after this harness is already constructed and `Arc`'d -- see that
    /// method's doc for why.
    io: Mutex<Arc<dyn ironclaw_loop_host::LoopCapabilityInputResolver>>,
    /// Result-writer half; see `io`'s doc -- always the SAME underlying
    /// object as `io`, coerced to the other trait.
    result_writer_io: Mutex<Arc<dyn LoopCapabilityResultWriter>>,
    /// Set by `install_durable_capability_io`: the session thread service
    /// backing `io`/`result_writer_io`. `None` until then (or forever, for
    /// harnesses that never opt in). Also used to wrap the synthetic
    /// `result_read` capability (`apply_synthetic_capability_wrappers`) so a
    /// scripted `result_read` call can page through the durable record `io`
    /// just persisted.
    durable_capability_io_thread_service:
        Mutex<Option<Arc<dyn ironclaw_threads::SessionThreadService>>>,
    /// Set from `HostRuntimeHarnessOptions::with_durable_capability_io()` at
    /// construction; read by the capability-mode assembly (`into_parts`) to
    /// decide whether to call `install_durable_capability_io` once the real
    /// thread service is available.
    durable_capability_io_requested: bool,
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
    /// S1 seam: wire-level transport recorder installed only by
    /// `.with_real_egress_pipeline()`. Sits BELOW the real
    /// `PolicyNetworkHttpEgress` (network-policy enforcement) and the real
    /// `HostHttpEgressService` (leak scan) — both run for real before a
    /// request reaches this double. `None` for every other construction.
    real_egress_transport: Option<Arc<RecordingNetworkHttpTransport>>,
    /// Inert recording process port. `Some` when the harness injected a
    /// `RecordingProcessPort`; `None` when the live `HostProcessPort` was
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
    skill_activation_source: Option<Arc<SkillActivationTestSource>>,
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
    /// production `visible_capability_request`,
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
    /// The full `RebornServices` bundle this harness's `new_with_options` built
    /// (`build_reborn_services`), retained so a group can build the REAL
    /// approval/auth interaction services over it instead of the harness's
    /// piecewise test-support accessors. `Some` only for `new_with_options`-built
    /// harnesses; `None` for the lower-level constructors and the Echo backend.
    /// Read via `reborn_services_for_test`.
    reborn_services: Option<ironclaw_reborn_composition::RebornServices>,
    /// Set from `HostRuntimeHarnessOptions::with_trigger_active_run_lookup_for_test()`
    /// (#5886) at construction; read by `HarnessCapabilityMode::into_parts` to
    /// decide whether to call `install_trigger_active_run_lookup_for_test` once
    /// the caller's real shared turn-state store is available.
    trigger_active_run_lookup_requested: bool,
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
        self.seed_credential_account_with_material(scope, "github", "journey github", &[])
            .await
    }

    /// Provider-generic core of [`Self::seed_github_credential_account`]:
    /// seeds a Configured credential account WITH real secret material through
    /// the production manual-token flow, under `scope`. The same
    /// dispatch-scope caveat applies: `scope` must match the run's actual
    /// `(tenant, user, agent, project)` or dispatch-time selection never finds
    /// the account.
    ///
    /// When `provider_scopes` is non-empty, a second Configured account
    /// carrying those scopes is created over the SAME secret handle the
    /// manual-token flow minted: OAuth-setup requirements only select accounts
    /// whose stored scopes cover the requirement, and manual-token accounts
    /// store none — the scoped twin satisfies selection while the shared
    /// handle keeps dispatch-time staging backed by real material.
    pub(crate) async fn seed_credential_account_with_material(
        &self,
        scope: &ResourceScope,
        provider: &str,
        label: &str,
        provider_scopes: &[&str],
    ) -> HarnessResult<()> {
        let product_auth = self
            .product_auth
            .as_ref()
            .ok_or("harness missing local-dev product auth (not built via new_with_options)")?;
        let scope = AuthProductScope::credential_owner(scope, AuthSurface::Api);
        let provider_id = AuthProviderId::new(provider)?;
        let challenge = product_auth
            .request_manual_token_setup(
                ironclaw_reborn_composition::RebornManualTokenSetupRequest::new(
                    scope.clone(),
                    provider_id.clone(),
                    CredentialAccountLabel::new(label)?,
                    ironclaw_auth::AuthContinuationRef::SetupOnly,
                    chrono::Utc::now() + chrono::Duration::minutes(10),
                ),
            )
            .await
            .map_err(|error| format!("manual token setup failed: {error:?}"))?;
        let submitted = product_auth
            .submit_manual_token(
                ironclaw_reborn_composition::RebornManualTokenSubmitRequest::new(
                    scope.clone(),
                    challenge.interaction_id,
                    secrecy::SecretString::from(format!("itest-{provider}-token")),
                ),
            )
            .await
            .map_err(|error| format!("manual token submit failed: {error:?}"))?;
        if provider_scopes.is_empty() {
            return Ok(());
        }
        let record_source = product_auth.credential_account_record_source_for_test();
        // Match on the account THIS call minted (`submitted.account_id`), not
        // the first provider match: a scope with multiple existing accounts
        // for the same provider (e.g. two Google accounts) would otherwise
        // silently pick an unrelated account's secret handle.
        let minted_handle = record_source
            .accounts_for_owner(&scope)
            .await
            .map_err(|error| format!("account read-back after manual token failed: {error:?}"))?
            .into_iter()
            .find(|account| account.id == submitted.account_id)
            .and_then(|account| account.access_secret)
            .ok_or("manual-token account with a secret handle not found on read-back")?;
        product_auth
            .credential_account_service()
            .create_account(NewCredentialAccount {
                scope,
                provider: provider_id,
                label: CredentialAccountLabel::new(format!("{label} scoped"))?,
                status: CredentialAccountStatus::Configured,
                ownership: CredentialOwnership::UserReusable,
                owner_extension: None,
                granted_extensions: Vec::new(),
                access_secret: Some(minted_handle),
                refresh_secret: None,
                scopes: provider_scopes
                    .iter()
                    .map(|scope| ProviderScope::new(*scope))
                    .collect::<Result<Vec<_>, _>>()?,
            })
            .await
            .map_err(|error| format!("scoped credential account creation failed: {error:?}"))?;
        Ok(())
    }

    /// Flip every non-revoked credential account for `provider` under
    /// `scope`'s credential-owner view to `Revoked`, returning how many were
    /// flipped. Models an EXTERNAL revocation (the user revoked the grant on
    /// the provider's side) through the same production transition the
    /// refresh sweep's `report_terminal_refresh_status` performs:
    /// `CredentialAccountService::update_status(.., Revoked)`. Companion to
    /// [`Self::seed_credential_account_with_material`] for the #5878
    /// activation-with-revoked-credential shape.
    pub(crate) async fn revoke_credential_accounts_for_provider(
        &self,
        scope: &ResourceScope,
        provider: &str,
    ) -> HarnessResult<usize> {
        let product_auth = self
            .product_auth
            .as_ref()
            .ok_or("harness missing local-dev product auth (not built via new_with_options)")?;
        let scope = AuthProductScope::credential_owner(scope, AuthSurface::Api);
        let provider_id = AuthProviderId::new(provider)?;
        let accounts = product_auth
            .credential_account_record_source_for_test()
            .accounts_for_owner(&scope)
            .await
            .map_err(|error| format!("account enumeration for revoke failed: {error:?}"))?;
        let mut revoked = 0;
        for account in accounts {
            if account.provider != provider_id || account.status == CredentialAccountStatus::Revoked
            {
                continue;
            }
            // Pass the account's OWN stored scope: `update_status` requires
            // full scope equality, and the stored scope carries the minting
            // invocation id no reconstructed scope can reproduce (same
            // pattern as the refresh sweep's terminal-status write).
            product_auth
                .credential_account_service()
                .update_status(&account.scope, account.id, CredentialAccountStatus::Revoked)
                .await
                .map_err(|error| format!("revoking account {} failed: {error:?}", account.id))?;
            revoked += 1;
        }
        Ok(revoked)
    }

    /// The fixed user this harness dispatches first-party capabilities under
    /// (see [`Self::with_user_id`]). Credential seeding aimed at a capability's
    /// dispatch-time account selection must use THIS user — for groups that do
    /// not align the capability user to the binding subject, it differs from
    /// the thread's binding actor.
    pub(crate) fn capability_user_id(&self) -> &UserId {
        &self.user_id
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
            durable_capability_io,
            trigger_active_run_lookup_requested,
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
                ironclaw_reborn_composition::RebornRuntimeProfileOptions {
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
            .map(Arc::new)
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
        // Durable tool-result projection seam (issue #5838): capability io
        // still defaults to the ephemeral `ProductLiveCapabilityIo` test
        // double at construction, like every other harness -- the real swap
        // (`install_durable_capability_io`) happens later, once the caller's
        // ACTUAL turn thread service exists (see that method's doc for why
        // it cannot happen here).
        let (io, result_writer_io) = default_capability_io_pair();
        // `.clone()` (not a move) so `services` survives intact below —
        // `reborn_services_for_test` needs the WHOLE `RebornServices` value,
        // not just the pieces already extracted above.
        let runtime = services
            .host_runtime
            .clone()
            .ok_or("local-dev Reborn services missing host runtime")?;
        let runtime = Arc::new(RecordingHostRuntime::new(
            runtime,
            Arc::clone(&pending_approval_scopes),
        ));
        Ok(Self {
            runtime: Mutex::new(runtime),
            approval_parts,
            auto_approve_settings,
            pending_approval_scopes,
            io: Mutex::new(io),
            result_writer_io: Mutex::new(result_writer_io),
            durable_capability_io_thread_service: Mutex::new(None),
            durable_capability_io_requested: durable_capability_io,
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
            real_egress_transport: None,
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
            reborn_services: Some(services),
            trigger_active_run_lookup_requested,
        })
    }

    /// Park this harness's tool/capability dispatch until released
    /// (`ParkingCapabilityGate`) — the tool-path analog of
    /// `RebornThreadBuilder::park_model`, for lease-expiry-under-a-wedged-tool
    /// coverage (see `tests/integration/lease_wedge.rs`). Wraps whatever
    /// `self.runtime` already is (e.g. `RecordingHostRuntime` over the real
    /// runtime), so parking sits outside the existing recorder at the same
    /// `HostRuntime` trait-object seam.
    pub(crate) fn park_capability_dispatch(mut self, gate: ParkingCapabilityGate) -> Self {
        let inner = self
            .runtime
            .into_inner()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.runtime = Mutex::new(Arc::new(ParkingHostRuntime::new(inner, gate)));
        self
    }

    /// The full `RebornServices` bundle this harness was built from, if built
    /// via `new_with_options`. Lets a caller build the REAL approval/auth
    /// interaction services over this harness's own local-dev composition
    /// (`RebornServices::local_dev_approval_interaction_service_with_turn_state_for_test`
    /// et al.), e.g. so a group can wire genuine `submit_inbound`-driven
    /// gate dispatch instead of the harness's direct-resume test shortcut.
    pub(crate) fn reborn_services_for_test(
        &self,
    ) -> Option<&ironclaw_reborn_composition::RebornServices> {
        self.reborn_services.as_ref()
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
            input_resolver: self.input_resolver(),
            result_writer: self.result_writer_io(),
            results: Arc::clone(&self.results),
        })
    }

    /// Current input-resolver half of this harness's capability io. See the
    /// `io` field's doc for the default-vs-durable shape.
    fn input_resolver(&self) -> Arc<dyn ironclaw_loop_host::LoopCapabilityInputResolver> {
        self.io.lock().unwrap().clone()
    }

    /// Current result-writer half; see `input_resolver`.
    fn result_writer_io(&self) -> Arc<dyn LoopCapabilityResultWriter> {
        self.result_writer_io.lock().unwrap().clone()
    }

    /// Durable tool-result projection seam (issue #5838): swap this
    /// harness's capability io for the REAL `StagedCapabilityIo`
    /// (`ironclaw_reborn_composition::test_support::staged_capability_io_for_test`,
    /// which mirrors production's `capability_wiring`), wired over
    /// `thread_service`.
    ///
    /// `thread_service` MUST be the SAME `SessionThreadService` the calling
    /// group's turns actually dispatch against (`group.rs`'s `into_group`
    /// builds it as `group_thread_harness.service`, BEFORE this harness's
    /// capability parts are assembled) -- `StagedCapabilityIo::write_capability_result`
    /// resolves the run's thread scope and appends the durable record keyed
    /// by `run_context.thread_id`; a different (e.g. this harness's own
    /// local-dev-only) thread service would never have that thread and every
    /// durable append would fail closed as `UnknownThread` ->
    /// `AgentLoopHostErrorKind::Unavailable` -> a terminal
    /// `HostUnavailable { stage: Capability }` for the whole run (see
    /// `.claude/rules/agent-loop-capabilities.md`).
    ///
    /// Interior mutability (not a builder field) because that real thread
    /// service is only constructed in `group.rs`'s `into_group`, AFTER
    /// `RebornCapabilityBackend::install` has already produced this harness
    /// (already `Arc`'d by then) -- called once, from
    /// `HarnessCapabilityMode::into_parts`, before any run uses this
    /// harness's capability port.
    fn install_durable_capability_io(
        &self,
        thread_service: Arc<dyn ironclaw_threads::SessionThreadService>,
    ) {
        let (io, result_writer_io) =
            ironclaw_reborn_composition::test_support::staged_capability_io_for_test(
                thread_service.clone(),
                self.user_id.clone(),
            );
        *self.io.lock().unwrap() = io;
        *self.result_writer_io.lock().unwrap() = result_writer_io;
        *self.durable_capability_io_thread_service.lock().unwrap() = Some(thread_service);
    }

    /// #5886: re-wire `builtin.trigger_list` dispatch to a REAL
    /// `TriggerActiveRunLookup` built over `turn_store` — the caller's actual
    /// shared turn-state store, where the group's real triggered runs (and
    /// their `BlockedApproval` gates) are recorded. This harness's own
    /// baked-in lookup (from `new_with_options` -> `build_reborn_services`)
    /// is scoped to a DIFFERENT, disjoint turn-state store this harness's
    /// capability dispatch never actually writes through in group-based
    /// tests (`group.rs`'s `into_group` routes real dispatch over its own
    /// shared coordinator/turn_store), so it can never see the run and
    /// `trigger_list` always omits `active_hold`.
    ///
    /// `turn_store` MUST be the SAME store the caller's runs actually persist
    /// to (`group.rs`'s `GroupSharedStorage.turn_store`) — mirrors
    /// `install_durable_capability_io`'s same interior-mutability constraint:
    /// that store only exists AFTER this harness is already built and `Arc`'d,
    /// so this runs post-construction, from `HarnessCapabilityMode::into_parts`,
    /// gated on `trigger_active_run_lookup_requested`.
    fn install_trigger_active_run_lookup_for_test(
        &self,
        turn_store: Arc<ironclaw_turns::FilesystemTurnStateStore<HarnessTurnBackend>>,
    ) -> HarnessResult<()> {
        let repo = self
            .trigger_repository_for_test()
            .ok_or("trigger_active_run_lookup wiring requires a captured trigger repository")?;
        let active_run_lookup =
            ironclaw_reborn_composition::test_support::local_dev_trigger_active_run_lookup_for_test(
                turn_store,
            );
        let trigger_lookup_storage_root = self.root.path().join("trigger-active-run-lookup");
        std::fs::create_dir_all(&trigger_lookup_storage_root)?;
        let trigger_runtime = assembly::local_dev_trigger_only_host_runtime(
            trigger_lookup_storage_root,
            repo,
            active_run_lookup,
        )?;
        let inner = self.runtime.lock().unwrap().clone();
        *self.runtime.lock().unwrap() = Arc::new(TriggerActiveRunLookupHostRuntime::new(
            inner,
            trigger_runtime,
            CapabilityId::new(ironclaw_host_runtime::TRIGGER_LIST_CAPABILITY_ID)?,
        ));
        Ok(())
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
    /// Empty when the harness uses the live `HostProcessPort`
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

    /// Every request that reached the S1 wire-level transport recorder, in
    /// call order. Empty (not an error) for every harness that did not opt
    /// into `.with_real_egress_pipeline()`.
    pub(crate) fn real_egress_transport_requests(&self) -> Vec<NetworkTransportRequest> {
        self.real_egress_transport
            .as_ref()
            .map(|transport| transport.requests())
            .unwrap_or_default()
    }

    /// Install FIFO scripted response bodies (S1 seam) onto the real-egress
    /// transport recorder, consumed ahead of its default body. Errors if this
    /// harness did not wire the real-egress pipeline.
    pub(crate) fn install_real_egress_response_bodies(
        &self,
        bodies: impl IntoIterator<Item = Vec<u8>>,
    ) -> HarnessResult<()> {
        let transport = self
            .real_egress_transport
            .as_ref()
            .ok_or("host runtime harness has no real-egress transport to script")?;
        for body in bodies {
            transport.push_response_body(body);
        }
        Ok(())
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
    ) -> Option<Arc<dyn ironclaw_loop_host::HostSkillContextSource>> {
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

    /// Assembles the recording `LoopCapabilityPort` for one run by driving the
    /// REAL production capability-port factory
    /// (`create_refreshing_capability_port`, harness-port-seam P1
    /// Change 2) with this harness's fields, then wrapping the returned port
    /// in the invocation-recording port (outermost, unchanged). Every
    /// production wrap layer (synthetic capabilities, surface disclosure,
    /// external tools, StaleSurface refresh, the shared `StagedCapabilityIo`)
    /// is now exercised automatically — this method no longer hand-rebuilds
    /// any of them. Owned here (rather than in the
    /// `HostRuntimeHarnessCapabilityPortFactory` test double) because the
    /// assembly reads this harness's fields directly — see
    /// `tests/integration/support/doubles/host_runtime_harness_capability_port_factory.rs`
    /// for the thin `LoopCapabilityPortFactory` delegating wrapper that calls
    /// this method.
    ///
    /// The returned port refreshes itself internally
    /// (`RefreshingCapabilityPort`'s own `StaleSurface` recovery), so
    /// there is no harness-level refresh wrapper layered on top anymore — one
    /// refresh mechanism (production's), not two.
    pub(crate) async fn create_recording_capability_port(
        self: &Arc<Self>,
        run_context: &LoopRunContext,
        milestone_sink: &Arc<ironclaw_turns::run_profile::InMemoryLoopHostMilestoneSink>,
    ) -> Result<Arc<dyn LoopCapabilityPort>, AgentLoopHostError> {
        // C-MULTIUSER: resolve the execution user per run (owner/actor) when
        // the harness opts in, else the fixed harness user — see
        // `dispatch_user_for_run`'s doc comment. `run_context` (and thus the
        // resolved user) is captured once for the lifetime of the returned
        // port, matching the per-run construction this method already had.
        let dispatch_user = self.dispatch_user_for_run(run_context);
        // ONE shared io, both roles: production assigns a single
        // `StagedCapabilityIo` to both `input_resolver` and `result_writer`
        // so input-ref/result-ref correlation by `call_id` works.
        // `RecordingCapabilityResultWriter` implements both traits over
        // `self.io` (recording only result writes) — `Arc::clone` the SAME
        // object into both config fields below, never two
        // independently-sourced io objects.
        let shared_io = Arc::new(RecordingCapabilityResultWriter {
            input_resolver: self.input_resolver(),
            result_writer: self.result_writer_io(),
            results: Arc::clone(&self.results),
        });
        let input_resolver: Arc<dyn ironclaw_loop_host::LoopCapabilityInputResolver> =
            Arc::clone(&shared_io) as Arc<dyn ironclaw_loop_host::LoopCapabilityInputResolver>;
        let result_writer: Arc<dyn LoopCapabilityResultWriter> =
            shared_io as Arc<dyn LoopCapabilityResultWriter>;
        // Parts this harness has no opinion on get a fresh in-memory no-op
        // default rather than a `None`/panic — production's config takes
        // plain required arguments, not `Option<Arc<...>>` fields (Change 1).
        let project_service = self
            .project_service
            .clone()
            .unwrap_or_else(|| Arc::new(super::doubles::UnavailableProjectService));
        // Wrapped in `RecordingApprovalRequestStore`: port-level synthetic
        // capabilities (e.g. `outbound_delivery_target_set`) persist approval
        // requests directly to this store rather than through the host
        // runtime, so `RecordingHostRuntime` never sees their scope — the
        // wrapper restores the `pending_approval_scopes` bookkeeping
        // `approve_local_dev_gate` / `deny_local_dev_gate` depend on while
        // delegating every method to the inner store (single source of truth).
        let inner_approval_requests: Arc<dyn ironclaw_run_state::ApprovalRequestStore> = self
            .approval_parts
            .as_ref()
            .map(|parts| Arc::clone(&parts.approval_requests))
            .unwrap_or_else(|| {
                Arc::new(ironclaw_run_state::in_memory_backed_approval_request_store())
            });
        let approval_requests: Arc<dyn ironclaw_run_state::ApprovalRequestStore> =
            Arc::new(super::doubles::RecordingApprovalRequestStore {
                inner: inner_approval_requests,
                pending_approval_scopes: Arc::clone(&self.pending_approval_scopes),
            });
        let capability_leases: Arc<dyn ironclaw_authorization::CapabilityLeaseStore> = self
            .approval_parts
            .as_ref()
            .map(|parts| Arc::clone(&parts.capability_leases))
            .unwrap_or_else(|| {
                Arc::new(ironclaw_authorization::in_memory_backed_capability_lease_store())
            });
        let tool_permission_overrides: Arc<dyn ironclaw_approvals::ToolPermissionOverrideStore> =
            self.tool_permission_overrides.clone().unwrap_or_else(|| {
                Arc::new(
                    ironclaw_approvals::test_support::in_memory_backed_capability_permission_override_store(),
                )
            });
        let auto_approve_settings: Arc<dyn ironclaw_approvals::AutoApproveSettingStore> =
            self.auto_approve_settings.clone().unwrap_or_else(|| {
                Arc::new(
                    ironclaw_approvals::test_support::in_memory_backed_auto_approve_setting_store(),
                )
            });
        let persistent_approval_policies: Arc<
            dyn ironclaw_approvals::PersistentApprovalPolicyStore,
        > = self
            .persistent_approval_policies
            .clone()
            .unwrap_or_else(|| {
                Arc::new(
                    ironclaw_approvals::test_support::in_memory_backed_persistent_approval_policy_store(),
                )
            });
        let outbound_preferences_facade = self.outbound_target_tools.as_ref().map(|parts| {
            Arc::clone(&parts.facade)
                as Arc<dyn ironclaw_product_workflow::OutboundPreferencesProductFacade>
        });
        let outbound_delivery_target_set_requires_approval = self
            .outbound_target_tools
            .as_ref()
            .map(|parts| parts.requires_approval)
            .unwrap_or(false);
        // Grantee = the loop-driver execution extension — the SAME principal
        // production's `visible_capability_request` executes under
        // and mints extension grants for (`ExtensionCapabilitySurface::grants`);
        // a `Principal::User` grantee (the pre-seam harness's choice, matched
        // to its removed hand-built User-principal authority) is skipped by
        // authorization under the production request. Computed early (moved
        // up from its original spot below) because
        // `build_additional_provider_trust`'s per-provider activation check
        // needs it too.
        let execution_extension =
            ironclaw_loop_host::loop_driver_execution_extension_id(run_context)?;
        // Which providers ALREADY have a production trust decision arriving
        // through the real activation handshake (`extension_surface_source`'s
        // `ExtensionCapabilitySurface::provider_trust()`, same
        // `extension_management` + grantee production's factory reads) --
        // queried the SAME way `build_local_dev_extension_management_for_test`
        // -> `extension_surface_source` will read it downstream in
        // `create_refreshing_capability_port_for_test`. NOT the same
        // as "this harness has `reborn_services` wired": a harness can have
        // `reborn_services` (so `new_with_options`) while only ever calling
        // the `publish_bundled_extension_for_test` SHORTCUT (registers the
        // package in the active registry but creates no enabled
        // installation), which `active_model_visible_capabilities()` requires
        // -- such a provider has NO production trust decision to protect, so
        // this must be an empty set for it, not treated as activation-backed.
        let activation_backed_providers: std::collections::HashSet<ExtensionId> =
            if let Some(services) = self.reborn_services.as_ref() {
                match services
                    .local_dev_active_extension_authority_for_test(&execution_extension)
                    .await
                {
                    Some(active_authority) => active_authority
                        .map_err(host_runtime_harness_error)?
                        .provider_trust
                        .into_iter()
                        .map(|(provider, _decision)| provider)
                        .collect(),
                    None => std::collections::HashSet::new(),
                }
            } else {
                std::collections::HashSet::new()
            };
        // Two config extensions Change 1 added (plus `capability_id_filter`
        // below = three total): per-capability execution-mount overrides and
        // additional provider-trust entries. Both inert/empty at production's
        // sole call site; this harness is the ONLY populator. Pulled into a
        // pure helper (`build_additional_provider_trust`) so the CodeRabbit
        // PR #6026 invariant it encodes is directly unit-testable — see that
        // function's doc comment and the `harness_trust_tests` module below.
        let additional_provider_trust = Self::build_additional_provider_trust(
            &self.provider_id,
            &self.effect_kinds,
            &self.additional_provider_trust,
            &activation_backed_providers,
        );
        // Hand-mint a grant for every id in this harness's `capability_ids`
        // allowlist (ad-hoc test-only `HostRuntime` backends never get a real
        // builtin/extension grant otherwise). Excludes the synthetic-capability
        // ids, which are surfaced by wrapping the port directly. See
        // `additional_capability_grants` doc for the invariant.
        let synthetic_capability_ids: std::collections::HashSet<&str> = [
            ironclaw_reborn_composition::test_support::PROJECT_CREATE_CAPABILITY_ID,
            ironclaw_reborn_composition::test_support::SKILL_ACTIVATE_CAPABILITY_ID,
            ironclaw_reborn_composition::test_support::OUTBOUND_DELIVERY_TARGET_SET_CAPABILITY_ID,
            ironclaw_reborn_composition::test_support::OUTBOUND_DELIVERY_TARGETS_LIST_CAPABILITY_ID,
        ]
        .into_iter()
        .collect();
        let additional_capability_grants: Vec<CapabilityGrant> = self
            .capability_ids
            .iter()
            .filter(|capability| !synthetic_capability_ids.contains(capability.as_str()))
            .map(|capability| {
                let mounts = self
                    .capability_mount_overrides
                    .iter()
                    .find(|(override_capability, _mounts)| override_capability == capability)
                    .map(|(_capability, mounts)| mounts.clone())
                    .unwrap_or_else(|| self.mounts.clone());
                CapabilityGrant {
                    id: CapabilityGrantId::new(),
                    capability: capability.clone(),
                    grantee: Principal::Extension(execution_extension.clone()),
                    issued_by: Principal::HostRuntime,
                    constraints: GrantConstraints {
                        allowed_effects: self.effect_kinds.clone(),
                        mounts,
                        network: self.network_policy.clone(),
                        secrets: self.secrets.clone(),
                        resource_ceiling: None,
                        expires_at: None,
                        max_invocations: None,
                    },
                }
            })
            .collect();
        let parts =
            ironclaw_reborn_composition::test_support::RefreshingCapabilityPortTestParts {
                runtime: self.runtime.lock().unwrap().clone(),
                run_context: run_context.clone(),
                fallback_user_id: dispatch_user,
                // All four mount views = this harness's single `mounts` view.
                // Production splits skill/memory/system-extensions mounts off
                // the local-dev workspace root, but this harness has ONE
                // profile-built view and the pre-seam behavior was "every
                // capability executes (and is granted) under `self.mounts`
                // unless `capability_mount_overrides` says otherwise" — a
                // `MountView::default()` here would instead OVERRIDE the
                // memory/skill capability families down to an empty view via
                // `build_inner`'s per-domain `with_capability_execution_mount`
                // special-cases, silently blinding `builtin.memory_*`.
                workspace_mounts: self.mounts.clone(),
                skill_mounts: self.mounts.clone(),
                memory_mounts: self.mounts.clone(),
                system_extensions_lifecycle_mounts: self.mounts.clone(),
                input_resolver,
                result_writer,
                milestone_sink: milestone_sink.clone() as Arc<dyn LoopHostMilestoneSink>,
                skill_activation_source: self.skill_activation_source.clone(),
                project_service,
                // result_read (durable tool-result projection seam, issue
                // #5838): production always wires the run's session thread
                // service into the synthetic `result_read` capability, so
                // this is a required (non-`Option`) field. Harnesses that
                // opted into `.with_durable_capability_io()` populate
                // `durable_capability_io_thread_service` with the REAL group
                // thread service; every other harness gets a fresh in-memory
                // no-op service, mirroring the `project_service`/
                // `tool_permission_overrides` "no opinion -> default" pattern
                // above -- `result_read` is simply never granted for those
                // harnesses (not in `capability_ids`), so the default is
                // never actually read.
                thread_service: self
                    .durable_capability_io_thread_service
                    .lock()
                    .unwrap()
                    .clone()
                    .unwrap_or_else(|| {
                        Arc::new(ironclaw_threads::InMemorySessionThreadService::default())
                    }),
                trajectory_observer: None,
                // Feeds the same active-extension authority (installed +
                // activated extensions like `github`, `gmail`, MCP servers)
                // production's `capability_wiring` folds into every refresh
                // (`runtime/local_dev.rs:132-133`); `None` when this harness
                // was built without `RebornServices` (mirrors the old
                // `local_dev_active_extension_authority_for_test` early-return).
                extension_management: self.reborn_services.as_ref().and_then(|services| {
                    ironclaw_reborn_composition::test_support::build_local_dev_extension_management_for_test(
                        services,
                    )
                }),
                outbound_preferences_facade,
                outbound_delivery_target_set_requires_approval,
                tool_permission_overrides,
                auto_approve_settings,
                persistent_approval_policies,
                approval_requests,
                capability_leases,
                // `self.capability_mount_overrides` is the SAME per-capability
                // mount-override list production's `skill_mounts`/`memory_mounts`/
                // `system_extensions_lifecycle_mounts` special-cases apply
                // automatically for their own fixed capability-id lists; passing
                // it here too (rather than splitting it across those three
                // fields, which this harness has no per-domain tracking for)
                // reaches the SAME final per-capability mount via the override
                // map, applied after those defaults in `build_inner`.
                capability_execution_mount_overrides: self
                    .capability_mount_overrides
                    .iter()
                    .cloned()
                    .collect(),
                additional_provider_trust,
                // Whole-set narrowing over the FULL granted-capability set to
                // this harness's exhaustive `capability_ids` allowlist,
                // including the empty case (zero grants). See
                // `additional_capability_grants` doc for the invariant.
                capability_id_filter: Some(self.capability_ids.iter().cloned().collect()),
                additional_capability_grants,
            };
        let port =
            ironclaw_reborn_composition::test_support::create_refreshing_capability_port_for_test(
                parts,
            )
            .await?;
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
    /// `visible_capability_request`'s owner→actor→fallback resolution
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

    /// Builds the composition-facing `additional_provider_trust` map this
    /// harness forwards to `RefreshingCapabilityPortTestParts` on
    /// every capability-port refresh (`create_recording_capability_port`
    /// calls this and passes the result straight through).
    ///
    /// Two independent entries, minted under two independent conditions:
    ///
    /// 1. `provider_id`'s own entry (this harness's primary provider,
    ///    `effect_kinds` its full authority) — restores the old harness's
    ///    primary-provider trust entry
    ///    (`ProductLiveVisibleCapabilityRequestConfig::new(...)
    ///    .with_provider_trust_for_effects(...)`, the very first authority
    ///    builder call pre-seam). Skipped for the `builtin` provider:
    ///    `additional_provider_trust` OVERWRITES same-id baseline entries,
    ///    and a profile's narrow `effect_kinds` (e.g. file tools'
    ///    filesystem-only list) would clobber the production builtin ceiling
    ///    and break builtin dispatch — the baseline already trusts `builtin`
    ///    with the full production effect set.
    /// 2. `additional_provider_trust`'s ad-hoc list (e.g.
    ///    `bundled_extension_provider_trust()`) — an ad-hoc test-only
    ///    `HostRuntime` backend (mock MCP, standalone GitHub/web-access WASM
    ///    — none of which ever activate a real extension) would otherwise
    ///    never get its provider trusted, and `tool_definitions()` silently
    ///    omits every capability from an untrusted provider.
    ///
    /// Security (CodeRabbit, PR #6026): entry (2) is ONLY safe to mint for a
    /// provider that does NOT already have a production trust decision --
    /// `additional_provider_trust` OVERWRITES same-id entries
    /// (`additional_provider_trust_is_forwarded_to_visible_request` in
    /// `refreshing_capability_port_test_support.rs`), so minting one for a
    /// provider `extension_surface_source` already trusts would silently
    /// clobber that (potentially narrower) production ceiling.
    ///
    /// The gate is PER-PROVIDER (`activation_backed_providers`), not a
    /// harness-wide `reborn_services.is_some()` boolean: a harness can have
    /// `reborn_services` wired (`new_with_options`) while its provider only
    /// ever reached the registry through the
    /// `publish_bundled_extension_for_test` SHORTCUT (upserts the package
    /// into the active registry but creates no ENABLED INSTALLATION) --
    /// `active_model_visible_capabilities()` requires an enabled
    /// installation, so such a provider has NO production trust decision at
    /// all and this entry is the ONLY thing that ever trusts it (see
    /// `file_and_github_auth_tools_profile` / `extension_visibility_probe_tools_profile`,
    /// neither of which runs a real install→activate handshake). A provider
    /// IS activation-backed (must be excluded here) only once
    /// `local_dev_active_extension_authority_for_test` actually reports a
    /// trust entry for it -- e.g. `extension_lifecycle_tools_profile`'s real
    /// credentialed install+activate flow. See `harness_trust_tests` below
    /// for the regression pin covering both shapes.
    fn build_additional_provider_trust(
        provider_id: &ExtensionId,
        effect_kinds: &[EffectKind],
        additional_provider_trust: &[(ExtensionId, Vec<EffectKind>)],
        activation_backed_providers: &std::collections::HashSet<ExtensionId>,
    ) -> BTreeMap<ExtensionId, ironclaw_trust::TrustDecision> {
        let mut result = BTreeMap::new();
        if provider_id.as_str() != ironclaw_host_runtime::BUILTIN_FIRST_PARTY_PROVIDER
            && !activation_backed_providers.contains(provider_id)
        {
            result.insert(
                provider_id.clone(),
                Self::admin_config_trust_decision(effect_kinds.to_vec()),
            );
        }
        for (provider, effects) in additional_provider_trust {
            if activation_backed_providers.contains(provider) {
                continue;
            }
            result.insert(
                provider.clone(),
                Self::admin_config_trust_decision(effects.clone()),
            );
        }
        result
    }

    fn admin_config_trust_decision(
        allowed_effects: Vec<EffectKind>,
    ) -> ironclaw_trust::TrustDecision {
        ironclaw_trust::TrustDecision {
            effective_trust: EffectiveTrustClass::user_trusted(),
            authority_ceiling: ironclaw_trust::AuthorityCeiling {
                allowed_effects,
                max_resource_ceiling: None,
            },
            provenance: ironclaw_trust::TrustProvenance::AdminConfig,
            evaluated_at: chrono::Utc::now(),
        }
    }
}

struct HostRuntimeHarnessSurfaceResolver;

#[async_trait::async_trait]
impl CapabilitySurfaceProfileResolver for HostRuntimeHarnessSurfaceResolver {
    async fn resolve(
        &self,
        _run_context: &LoopRunContext,
    ) -> Result<CapabilityAllowSet, CapabilityResolveError> {
        Ok(CapabilityAllowSet::All)
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

/// Regression coverage for the CodeRabbit PR #6026 security finding:
/// `HostRuntimeCapabilityHarness::build_additional_provider_trust` must never
/// mint a synthetic `additional_provider_trust` entry for a provider that a
/// wired `extension_management` could supply a real (potentially narrower)
/// production trust decision for — such an entry would silently OVERWRITE
/// that decision (`additional_provider_trust` extends the base map AFTER
/// `extension_surface.provider_trust()`, last-writer-wins). Pure unit tests
/// against the extracted helper, independent of the full async harness/tokio
/// runtime, so the invariant is pinned even without a live extension-activation
/// scenario.
#[cfg(test)]
mod harness_trust_tests {
    use super::*;

    /// The exact shape `extension_lifecycle_tools_profile_for_user` builds:
    /// a blanket `bundled_extension_provider_trust()`-style entry for
    /// `gmail` (the provider CodeRabbit's finding named), where `gmail` IS in
    /// `activation_backed_providers` (its real credentialed install+activate
    /// handshake already produced a production trust decision). Before the
    /// fix this test would have observed an unbounded `gmail` entry here; the
    /// fix must make it absent so the activation-backed production decision
    /// (computed downstream, not visible to this pure helper) is free to pass
    /// through untouched.
    #[test]
    fn activation_backed_provider_gets_no_synthetic_trust_entry() {
        let builtin_provider =
            ExtensionId::new(ironclaw_host_runtime::BUILTIN_FIRST_PARTY_PROVIDER)
                .expect("builtin provider id");
        let gmail_provider = ExtensionId::new("gmail").expect("gmail provider id");
        let blanket_additional = vec![(gmail_provider.clone(), local_dev_all_effects())];
        let activation_backed_providers: std::collections::HashSet<ExtensionId> =
            [gmail_provider.clone()].into_iter().collect();

        let result = HostRuntimeCapabilityHarness::build_additional_provider_trust(
            &builtin_provider,
            &local_dev_all_effects(),
            &blanket_additional,
            &activation_backed_providers,
        );

        assert!(
            !result.contains_key(&gmail_provider),
            "a provider already reported by `local_dev_active_extension_authority_for_test` \
             must never get a synthetic trust entry -- production's \
             `extension_surface.provider_trust()` must be the sole source of `gmail`'s \
             ceiling once it is activated, and this entry would silently overwrite it: \
             {result:?}"
        );
    }

    /// The genuinely ad-hoc case (mock-mcp / standalone github / standalone
    /// web-access harnesses): the provider is absent from
    /// `activation_backed_providers` (no `reborn_services` wired at all, so
    /// there is no production decision to ever clobber), and the entry IS
    /// needed (no other path trusts these providers at all).
    #[test]
    fn ad_hoc_provider_without_reborn_services_still_gets_trust_entry() {
        let builtin_provider =
            ExtensionId::new(ironclaw_host_runtime::BUILTIN_FIRST_PARTY_PROVIDER)
                .expect("builtin provider id");
        let mock_mcp_provider = ExtensionId::new("mock-mcp").expect("mock-mcp provider id");
        let effects = vec![EffectKind::DispatchCapability, EffectKind::Network];
        let additional = vec![(mock_mcp_provider.clone(), effects.clone())];

        let result = HostRuntimeCapabilityHarness::build_additional_provider_trust(
            &builtin_provider,
            &effects,
            &additional,
            &std::collections::HashSet::new(),
        );

        assert_eq!(
            result
                .get(&mock_mcp_provider)
                .map(|decision| &decision.authority_ceiling.allowed_effects),
            Some(&effects),
            "a harness with no activation-backed decision for this provider must still mint \
             its ad-hoc provider's trust entry, or that provider's capabilities become \
             invisible: {result:?}"
        );
    }

    /// The gap this fix closes (CI diagnostician finding, post-#5902
    /// rebase): `file_and_github_auth_tools_profile` /
    /// `extension_visibility_probe_tools_profile` build through
    /// `new_with_options` (so `reborn_services` IS wired) but only ever call
    /// the `publish_bundled_extension_for_test` shortcut for their provider
    /// -- no enabled installation, so `local_dev_active_extension_authority_for_test`
    /// never reports a trust entry for it. The OLD `has_reborn_services: bool`
    /// gate treated "reborn_services wired" as "activation-backed" and
    /// wrongly suppressed this provider's only trust source. The per-provider
    /// `activation_backed_providers` set must NOT contain such a
    /// shortcut-only provider, so its synthetic entry is still minted.
    #[test]
    fn shortcut_published_provider_without_activation_still_gets_trust_entry() {
        let builtin_provider =
            ExtensionId::new(ironclaw_host_runtime::BUILTIN_FIRST_PARTY_PROVIDER)
                .expect("builtin provider id");
        let visprobe_provider = ExtensionId::new("visprobe").expect("visprobe provider id");
        let effects = local_dev_all_effects();
        let additional = vec![(visprobe_provider.clone(), effects.clone())];
        // reborn_services is wired for this harness shape (`new_with_options`)
        // but `visprobe` was only ever `publish_bundled_extension_for_test`'d,
        // never installed+activated -- so it must be ABSENT from
        // `activation_backed_providers`, not folded in via a blanket
        // `reborn_services.is_some()` check.
        let activation_backed_providers: std::collections::HashSet<ExtensionId> =
            std::collections::HashSet::new();

        let result = HostRuntimeCapabilityHarness::build_additional_provider_trust(
            &builtin_provider,
            &effects,
            &additional,
            &activation_backed_providers,
        );

        assert_eq!(
            result
                .get(&visprobe_provider)
                .map(|decision| &decision.authority_ceiling.allowed_effects),
            Some(&effects),
            "a publish-only (never installed+activated) provider must still get its \
             synthetic trust entry even when the harness has `reborn_services` wired, \
             or its capabilities become invisible: {result:?}"
        );
    }
}
