use std::path::PathBuf;
use std::sync::Arc;

use ironclaw_extensions::ExtensionPackage;
use ironclaw_host_api::{
    AgentId, CapabilityId, EffectKind, ExtensionId, MountView, NetworkPolicy, SecretHandle,
    TenantId, UserId,
};
use ironclaw_host_runtime::BUILTIN_FIRST_PARTY_PROVIDER;
use ironclaw_network::NetworkHttpEgress;

use super::{HarnessResult, HostRuntimeCapabilityHarness};

#[derive(Default)]
pub(crate) struct HostRuntimeHarnessOptions {
    pub(crate) mounts: MountView,
    pub(crate) runtime_policy: Option<ironclaw_host_api::runtime_policy::EffectiveRuntimePolicy>,
    /// Override the local runtime tenant/agent before the composed runtime is
    /// built. Group harnesses set this to the canonical product scope so
    /// runtime-owned facades keyed by tenant (for example channel connections)
    /// match the turns that dispatch through the group.
    pub(crate) local_runtime_identity: Option<(TenantId, AgentId)>,
    pub(crate) seed_extension_credentials: bool,
    /// Tenant the E-SKILL skill context source is constructed under, when this
    /// harness surfaces the synthetic `skill_activate` capability. Only
    /// `skill_activation_tools()` sets this (via
    /// `with_skill_activation_tenant`), passing the SAME tenant the caller's
    /// group run scope resolved (`group.rs` `build_base`'s
    /// `canonical_binding.tenant_id`) — never a separately hardcoded literal —
    /// so `skill_activate` resolves the seeded user skill against the same
    /// tenant the turn runs under. `None` for every other harness variant.
    pub(crate) skill_activation_tenant: Option<TenantId>,
    /// System-skill fixtures copied into the local-dev storage root before
    /// runtime construction. The runtime warms the system-skill descriptor cache
    /// during build, so system fixtures must exist before `build_runtime`.
    pub(crate) system_skill_fixtures: Vec<SystemSkillFixture>,
    /// Injected outbound-delivery facade double + `target_set` approval flag
    /// for the synthetic list/set test seam. The current-run router is a normal
    /// first-party capability and is not backed by this double. Only
    /// `outbound_target_tools()` sets this. `new_with_options` pairs the facade
    /// with the local-dev settings stores captured from `RebornServices` to
    /// build `OutboundTargetToolsParts`.
    pub(crate) outbound_target_facade: Option<(
        Arc<super::super::outbound_preferences::FakeOutboundPreferencesFacade>,
        bool,
    )>,
    /// C-JOURNEY: override the local-dev host network HTTP egress
    /// (`RebornHostBindings::with_network_http_egress_for_test`). Without this,
    /// `build_local_runtime` defaults to a REAL `ReqwestNetworkTransport`
    /// (`factory.rs`), so any harness dispatching a bundled WASM capability
    /// that crosses HTTP (e.g. `github.*`) on the `new_with_options` path MUST
    /// set this to stay hermetic. `None` for every harness that surfaces no
    /// such capability.
    pub(crate) network_http_egress_for_test: Option<Arc<dyn NetworkHttpEgress>>,
    /// C-JOURNEY: bundled first-party WASM packages (e.g. github) to publish
    /// directly into the local-dev active-extension registry at construction
    /// time, via `RebornServices::publish_bundled_extension_for_test`
    /// (reaches the SAME `ActiveExtensionPublisher::publish` step
    /// `builtin.extension_install` calls). Without this, a bundled package's
    /// capabilities are granted/trusted at the harness-authority layer
    /// (`capability_ids`/`additional_provider_trust`) but NOT present in the
    /// runtime's own dispatchable registry, so dispatch silently no-ops (the
    /// tool call never reaches `invoke_capability`). Empty for every harness
    /// that surfaces no bundled WASM capability.
    pub(crate) activate_bundled_extensions_for_test: Vec<(
        ExtensionPackage,
        Option<ironclaw_extensions::ResolvedExtensionManifest>,
    )>,
    /// Fixture extension asset directories copied into the harness storage
    /// root's `/system/extensions/{id}` BEFORE composition builds, so the
    /// available-extension catalog discovers them like installed packages
    /// (the invented-vendor fixture, overview §8).
    pub(crate) fixture_extension_dirs: Vec<(std::path::PathBuf, String)>,
    /// `first_party` extension factories the harness assembles into the
    /// composition input (`RebornHostBindings::with_native_extension_factories`
    /// — the same seam the binary uses).
    pub(crate) native_extension_factories:
        Vec<Arc<dyn ironclaw_extension_host::NativeExtensionFactory>>,
    /// Channel-adapter bindings for non-`first_party`-runtime channel
    /// extensions (`RebornHostBindings::with_channel_extension_bindings` — the
    /// same seam the binary uses for Slack's WASM-runtime package).
    pub(crate) channel_extension_bindings:
        Vec<ironclaw_reborn_composition::ChannelExtensionBinding>,
    /// Typed handle for the recording network egress when the profile wants
    /// `captured_network_requests` assertions (the dyn seam alone loses the
    /// recorder type).
    pub(crate) recording_network_egress:
        Option<Arc<super::super::doubles::RecordingNetworkHttpEgress>>,
    /// C-SYNTH `project_create` fault-injection seam: wrap the real
    /// `Arc<dyn ProjectService>` (`services.local_dev_project_service_for_test()`)
    /// in `FaultInjectingProjectService` before it reaches the capability-port
    /// test parts' `project_service` field, so a `create_project` call naming
    /// `FAULT_INJECT_DENIED_PROJECT_NAME` returns `ProjectServiceError::Denied`
    /// instead of reaching the real store. Only
    /// `project_tools_with_fault_injection()` sets this; every other harness
    /// leaves the real service unwrapped.
    pub(crate) project_service_fault_injection: bool,
    /// Durable tool-result projection seam (issue #5838): when `true`, the
    /// harness backs its capability io with the REAL `StagedCapabilityIo`
    /// (via `ironclaw_reborn_composition::test_support::staged_capability_io_for_test`,
    /// wired over this harness's own local-dev `thread_service`) instead of
    /// the ephemeral `ProductLiveCapabilityIo` test double. Opt-in and
    /// explicit rather than a profile default, so the ~100 other
    /// `HostRuntimeCapabilityHarness`-based integration tests stay
    /// byte-identical.
    pub(crate) durable_capability_io: bool,
    /// #5886 harness-wiring seam: when `true`, the harness's `builtin.trigger_list`
    /// dispatch is later re-routed (post-construction, once the caller's real
    /// shared turn-state store exists) to a REAL `TriggerActiveRunLookup`
    /// instead of the harness's own baked-in lookup, which is scoped to a
    /// turn-state store group-based tests never write real runs into. See
    /// `HostRuntimeCapabilityHarness::install_trigger_active_run_lookup_for_test`.
    /// Opt-in; every other harness stays byte-identical.
    pub(crate) trigger_active_run_lookup_requested: bool,
    /// Provider-instance readiness map, "config set" + restart arm: when
    /// `true`, registers a dummy Google OAuth backend on the `RebornHostBindings`
    /// via the SAME generic production builder
    /// (`RebornHostBindings::with_vendor_oauth_client`)
    /// that `ironclaw config set google.client_id`/`client_secret` feeds in
    /// production — proving the readiness-map check clears once an operator
    /// configures the instance, with no test-only bypass. `false` (the
    /// default) matches every pre-existing harness: no Google OAuth backend,
    /// i.e. this instance is "unconfigured".
    pub(crate) google_oauth_backend_for_test: bool,
}

impl HostRuntimeHarnessOptions {
    pub(crate) fn new(
        mounts: MountView,
        runtime_policy: Option<ironclaw_host_api::runtime_policy::EffectiveRuntimePolicy>,
    ) -> Self {
        Self {
            mounts,
            runtime_policy,
            local_runtime_identity: None,
            seed_extension_credentials: false,
            skill_activation_tenant: None,
            system_skill_fixtures: Vec::new(),
            outbound_target_facade: None,
            network_http_egress_for_test: None,
            activate_bundled_extensions_for_test: Vec::new(),
            fixture_extension_dirs: Vec::new(),
            native_extension_factories: Vec::new(),
            channel_extension_bindings: Vec::new(),
            recording_network_egress: None,
            project_service_fault_injection: false,
            durable_capability_io: false,
            trigger_active_run_lookup_requested: false,
            google_oauth_backend_for_test: false,
        }
    }

    /// Request the post-construction real `TriggerActiveRunLookup` rewire
    /// (#5886) — see the field doc on
    /// [`Self::trigger_active_run_lookup_requested`].
    pub(crate) fn with_trigger_active_run_lookup_for_test(mut self) -> Self {
        self.trigger_active_run_lookup_requested = true;
        self
    }

    pub(crate) fn with_seed_extension_credentials(mut self) -> Self {
        self.seed_extension_credentials = true;
        self
    }

    pub(crate) fn with_local_runtime_identity(
        mut self,
        tenant_id: TenantId,
        agent_id: AgentId,
    ) -> Self {
        self.local_runtime_identity = Some((tenant_id, agent_id));
        self
    }

    pub(crate) fn with_skill_activation_tenant(mut self, tenant: TenantId) -> Self {
        self.skill_activation_tenant = Some(tenant);
        self
    }

    pub(crate) fn with_system_skill_fixture(
        mut self,
        name: impl Into<String>,
        description: impl Into<String>,
        prompt: impl Into<String>,
    ) -> Self {
        self.system_skill_fixtures.push(SystemSkillFixture {
            name: name.into(),
            description: description.into(),
            prompt: prompt.into(),
        });
        self
    }

    pub(crate) fn with_outbound_target_tools(
        mut self,
        facade: Arc<super::super::outbound_preferences::FakeOutboundPreferencesFacade>,
        target_set_requires_approval: bool,
    ) -> Self {
        self.outbound_target_facade = Some((facade, target_set_requires_approval));
        self
    }

    pub(crate) fn with_network_http_egress_for_test(
        mut self,
        egress: Arc<dyn NetworkHttpEgress>,
    ) -> Self {
        self.network_http_egress_for_test = Some(egress);
        self
    }

    pub(crate) fn with_fixture_extension_dir(
        mut self,
        source: std::path::PathBuf,
        extension_id: &str,
    ) -> Self {
        self.fixture_extension_dirs
            .push((source, extension_id.to_string()));
        self
    }

    /// Install a RECORDING network egress: wires the dyn transport seam AND
    /// retains the typed handle so `captured_network_requests` works.
    pub(crate) fn with_recording_network_egress(
        mut self,
        egress: Arc<super::super::doubles::RecordingNetworkHttpEgress>,
    ) -> Self {
        self.network_http_egress_for_test = Some(egress.clone() as Arc<dyn NetworkHttpEgress>);
        self.recording_network_egress = Some(egress);
        self
    }

    pub(crate) fn with_native_extension_factory(
        mut self,
        factory: Arc<dyn ironclaw_extension_host::NativeExtensionFactory>,
    ) -> Self {
        self.native_extension_factories.push(factory);
        self
    }

    /// Binary-parity channel-adapter binding for channel extensions whose
    /// runtime is NOT `first_party` (extension-runtime P6): mirrors
    /// `RebornHostBindings::with_channel_extension_bindings` the same way the
    /// native factories mirror the CLI assembly. Without it, composition
    /// binds the transitional `HostServedChannelBridge`, whose `inbound`
    /// rejects every verified request with `ChannelError::Unsupported`.
    pub(crate) fn with_channel_extension_binding(
        mut self,
        binding: ironclaw_reborn_composition::ChannelExtensionBinding,
    ) -> Self {
        self.channel_extension_bindings.push(binding);
        self
    }

    pub(crate) fn with_activated_bundled_extension(mut self, package: ExtensionPackage) -> Self {
        self.activate_bundled_extensions_for_test
            .push((package, None));
        self
    }

    /// Variant for in-code fixture packages with no catalog entry: the
    /// caller supplies the resolved contract the generic-host mirror
    /// publishes.
    pub(crate) fn with_activated_bundled_extension_resolved(
        mut self,
        package: ExtensionPackage,
        resolved: ironclaw_extensions::ResolvedExtensionManifest,
    ) -> Self {
        self.activate_bundled_extensions_for_test
            .push((package, Some(resolved)));
        self
    }

    pub(crate) fn with_project_service_fault_injection(mut self) -> Self {
        self.project_service_fault_injection = true;
        self
    }

    /// Opt into the real `StagedCapabilityIo` (durable tool-result
    /// projection seam, issue #5838) instead of the ephemeral
    /// `ProductLiveCapabilityIo` test double.
    pub(crate) fn with_durable_capability_io(mut self) -> Self {
        self.durable_capability_io = true;
        self
    }
    /// Opt into a composition-time Google OAuth backend. See
    /// `google_oauth_backend_for_test`'s doc.
    pub(crate) fn with_google_oauth_backend_for_test(mut self) -> Self {
        self.google_oauth_backend_for_test = true;
        self
    }
}

#[derive(Clone)]
pub(crate) struct SystemSkillFixture {
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) prompt: String,
}

/// Typed capture of a `HostRuntimeCapabilityHarness::new_with_options(..)` call
/// shape plus the post-construct steps a domain constructor applies to the
/// built harness. Each `harness/profiles/<domain>.rs` module constructs one
/// `ToolsProfile` and calls `.build()` instead of an inline
/// `new_with_options(..)` + ad-hoc post-construct mutation.
///
/// Mirrors `new_with_options`'s parameter list plus four ADDITIONAL fields
/// capturing post-construct steps (`network_policy_override`,
/// `provider_trust_override`, `post_construct_asset_copy`,
/// `auto_approve_default`) — see `build()`'s doc comment for the fixed
/// application order.
pub(crate) struct ToolsProfile {
    pub(crate) service_label: &'static str,
    pub(crate) capability_ids: Vec<CapabilityId>,
    pub(crate) effect_kinds: Vec<EffectKind>,
    pub(crate) secrets: Vec<SecretHandle>,
    pub(crate) provider_id: ExtensionId,
    pub(crate) user_id: UserId,
    pub(crate) options: HostRuntimeHarnessOptions,
    /// Mirrors constructors that overwrite `harness.network_policy` after
    /// `new_with_options` returns (e.g. `extension_lifecycle_tools`,
    /// `skill_management_tools`, `trace_commons_tools`). `None` leaves the
    /// harness's default `NetworkPolicy::default()`.
    pub(crate) network_policy_override: Option<NetworkPolicy>,
    /// Mirrors constructors that overwrite `harness.additional_provider_trust`
    /// (e.g. `extension_lifecycle_tools`, `file_and_github_auth_tools`).
    /// `None` leaves the harness's default empty trust list.
    pub(crate) provider_trust_override: Option<Vec<(ExtensionId, Vec<EffectKind>)>>,
    /// Mirrors `file_and_github_auth_tools`'s post-construct
    /// `copy_dir_recursive(&github_support::asset_root(), &harness.root.path().join(..))`
    /// step: `(source_dir, relative_dest_under_harness_root)`. The destination
    /// is captured as a path RELATIVE to the harness's tempdir root because the
    /// root itself is created inside `new_with_options` and does not exist yet
    /// when a `ToolsProfile` is assembled. Plain data (no closure) — the copy
    /// is a fixed filesystem operation, not caller-specific logic.
    pub(crate) post_construct_asset_copy: Option<(PathBuf, PathBuf)>,
    /// Mirrors the `enable_global_auto_approve_for_product_and_harness_users` /
    /// `disable_global_auto_approve_for_product_and_harness_users` post-construct
    /// calls: `Some(true)` enables, `Some(false)` disables, `None` touches
    /// nothing (e.g. `attachment_tools`, `write_only`).
    pub(crate) auto_approve_default: Option<bool>,
}

impl ToolsProfile {
    /// Neutral baseline: empty capability/effect/secret lists, the universal
    /// `BUILTIN_FIRST_PARTY_PROVIDER` provider id (every existing
    /// `new_with_options`-based constructor passes this same value — the one
    /// "every caller agrees" exception to the empty/None/false default rule),
    /// default (empty) harness options, and no post-construct steps.
    ///
    /// `user_id` is explicit (no placeholder default): every profile has a
    /// fixed domain-specific user id, and a silently-valid fallback would let
    /// a forgotten override build a harness under the wrong user.
    pub(crate) fn new(service_label: &'static str, user_id: &str) -> HarnessResult<Self> {
        Ok(Self {
            service_label,
            capability_ids: Vec::new(),
            effect_kinds: Vec::new(),
            secrets: Vec::new(),
            provider_id: ExtensionId::new(BUILTIN_FIRST_PARTY_PROVIDER)?,
            user_id: UserId::new(user_id)?,
            options: HostRuntimeHarnessOptions::default(),
            network_policy_override: None,
            provider_trust_override: None,
            post_construct_asset_copy: None,
            auto_approve_default: None,
        })
    }

    pub(crate) fn with_capability_ids(mut self, capability_ids: Vec<CapabilityId>) -> Self {
        self.capability_ids = capability_ids;
        self
    }

    pub(crate) fn with_effect_kinds(mut self, effect_kinds: Vec<EffectKind>) -> Self {
        self.effect_kinds = effect_kinds;
        self
    }

    pub(crate) fn with_secrets(mut self, secrets: Vec<SecretHandle>) -> Self {
        self.secrets = secrets;
        self
    }

    pub(crate) fn with_provider_id(mut self, provider_id: ExtensionId) -> Self {
        self.provider_id = provider_id;
        self
    }

    pub(crate) fn with_user_id(mut self, user_id: UserId) -> Self {
        self.user_id = user_id;
        self
    }

    pub(crate) fn with_options(mut self, options: HostRuntimeHarnessOptions) -> Self {
        self.options = options;
        self
    }

    pub(crate) fn with_network_policy_override(mut self, policy: NetworkPolicy) -> Self {
        self.network_policy_override = Some(policy);
        self
    }

    pub(crate) fn with_provider_trust_override(
        mut self,
        trust: Vec<(ExtensionId, Vec<EffectKind>)>,
    ) -> Self {
        self.provider_trust_override = Some(trust);
        self
    }

    pub(crate) fn with_post_construct_asset_copy(
        mut self,
        source_dir: PathBuf,
        relative_dest_under_harness_root: PathBuf,
    ) -> Self {
        self.post_construct_asset_copy = Some((source_dir, relative_dest_under_harness_root));
        self
    }

    pub(crate) fn with_auto_approve_default(mut self, enabled: bool) -> Self {
        self.auto_approve_default = Some(enabled);
        self
    }

    /// THE one shared construction path domain profiles build on: calls
    /// `HostRuntimeCapabilityHarness::new_with_options(..)` with this profile's
    /// core fields, then applies the captured post-construct steps in the SAME
    /// fixed order every existing multi-step constructor applies them (verified
    /// against `extension_lifecycle_tools` and `file_and_github_auth_tools`,
    /// the only two constructors that combine more than one post-construct
    /// step):
    ///
    /// 1. `network_policy_override` (if set)
    /// 2. `provider_trust_override` (if set)
    /// 3. `post_construct_asset_copy` (if set)
    /// 4. `auto_approve_default` (enable/disable/neither)
    pub(crate) async fn build(self) -> HarnessResult<HostRuntimeCapabilityHarness> {
        let mut harness = HostRuntimeCapabilityHarness::new_with_options(
            self.service_label,
            self.capability_ids,
            self.effect_kinds,
            self.secrets,
            self.provider_id,
            self.user_id,
            self.options,
        )
        .await?;
        if let Some(policy) = self.network_policy_override {
            harness.network_policy = policy;
        }
        if let Some(trust) = self.provider_trust_override {
            harness.additional_provider_trust = trust;
        }
        if let Some((source_dir, relative_dest)) = self.post_construct_asset_copy {
            let dest = harness.root.path().join(relative_dest);
            super::copy_dir_recursive(&source_dir, &dest)?;
        }
        match self.auto_approve_default {
            Some(true) => {
                harness
                    .enable_global_auto_approve_for_product_and_harness_users()
                    .await?;
            }
            Some(false) => {
                harness
                    .disable_global_auto_approve_for_product_and_harness_users()
                    .await?;
            }
            None => {}
        }
        Ok(harness)
    }
}
