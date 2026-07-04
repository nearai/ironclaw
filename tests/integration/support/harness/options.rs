use std::sync::Arc;

use ironclaw_extensions::ExtensionPackage;
use ironclaw_host_api::{MountView, TenantId};
use ironclaw_network::NetworkHttpEgress;

pub(crate) struct HostRuntimeHarnessOptions {
    pub(crate) mounts: MountView,
    pub(crate) runtime_policy: Option<ironclaw_host_api::runtime_policy::EffectiveRuntimePolicy>,
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
    /// Injected outbound-delivery facade double + `target_set` approval flag,
    /// when this harness surfaces the synthetic `outbound_delivery_*`
    /// capabilities (C-SYNTH outbound seam). Only `outbound_target_tools()` sets
    /// this. `new_with_options` pairs the facade with the local-dev settings
    /// stores captured from `RebornServices` to build `OutboundTargetToolsParts`.
    pub(crate) outbound_target_facade: Option<(
        Arc<super::super::outbound_preferences::FakeOutboundPreferencesFacade>,
        bool,
    )>,
    /// C-JOURNEY: override the local-dev host network HTTP egress
    /// (`RebornBuildInput::with_network_http_egress_for_test`). Without this,
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
    /// `builtin.extension_activate` calls). Without this, a bundled package's
    /// capabilities are granted/trusted at the harness-authority layer
    /// (`capability_ids`/`additional_provider_trust`) but NOT present in the
    /// runtime's own dispatchable registry, so dispatch silently no-ops (the
    /// tool call never reaches `invoke_capability`). Empty for every harness
    /// that surfaces no bundled WASM capability.
    pub(crate) activate_bundled_extensions_for_test: Vec<ExtensionPackage>,
    /// C-SYNTH `project_create` fault-injection seam: wrap the real
    /// `Arc<dyn ProjectService>` (`services.local_dev_project_service_for_test()`)
    /// in `FaultInjectingProjectService` before it reaches
    /// `wrap_project_create_capability_for_test`, so a `create_project` call
    /// naming `FAULT_INJECT_DENIED_PROJECT_NAME` returns
    /// `ProjectServiceError::Denied` instead of reaching the real store.
    /// Only `project_tools_with_fault_injection()` sets this; every other
    /// harness leaves the real service unwrapped.
    pub(crate) project_service_fault_injection: bool,
}

impl HostRuntimeHarnessOptions {
    pub(crate) fn new(
        mounts: MountView,
        runtime_policy: Option<ironclaw_host_api::runtime_policy::EffectiveRuntimePolicy>,
    ) -> Self {
        Self {
            mounts,
            runtime_policy,
            seed_extension_credentials: false,
            skill_activation_tenant: None,
            outbound_target_facade: None,
            network_http_egress_for_test: None,
            activate_bundled_extensions_for_test: Vec::new(),
            project_service_fault_injection: false,
        }
    }

    pub(crate) fn with_seed_extension_credentials(mut self) -> Self {
        self.seed_extension_credentials = true;
        self
    }

    pub(crate) fn with_skill_activation_tenant(mut self, tenant: TenantId) -> Self {
        self.skill_activation_tenant = Some(tenant);
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

    pub(crate) fn with_activated_bundled_extension(mut self, package: ExtensionPackage) -> Self {
        self.activate_bundled_extensions_for_test.push(package);
        self
    }

    pub(crate) fn with_project_service_fault_injection(mut self) -> Self {
        self.project_service_fault_injection = true;
        self
    }
}
