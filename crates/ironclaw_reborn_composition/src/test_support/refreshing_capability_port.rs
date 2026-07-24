//! Production capability-port assembly test support (harness-port-seam P1).
//!
//! Lets the Reborn integration-test harness assemble its capability port
//! through the REAL production factory
//! (`create_refreshing_capability_port`,
//! `runtime::local_dev::refreshing_capability_port.rs:75`) instead of hand-
//! rebuilding the wrap order, so every current and future production layer
//! (surface disclosure, external tools, StaleSurface refresh, the shared
//! `StagedCapabilityIo`) is automatically exercised by the harness too.

/// Typed bundle of the parts the harness controls, passed by value through
/// the `test_support` -> `runtime` -> `runtime::local_dev` forwarding chain
/// so no layer needs `#[allow(clippy::too_many_arguments)]`. Mirrors
/// `RefreshingCapabilityPortConfig` minus the no-op-by-default parts
/// (`external_tool_catalog`, `policy`). `extension_surface_source` itself
/// stays no-op-by-default too — the harness supplies the raw
/// `extension_management` port below (Change 3, harness-port-seam P1
/// follow-up) and `create_refreshing_capability_port_for_test`
/// wraps it in `ExtensionCapabilitySurfaceSource::new(..)` internally, the SAME
/// constructor production's `capability_wiring` calls
/// (`runtime/local_dev.rs:132-133`) — this crate is the only place that can
/// name the `pub(in crate::runtime)` wrapper type.
#[cfg(feature = "test-support")]
pub struct RefreshingCapabilityPortTestParts {
    /// Host runtime the assembled port dispatches builtin capabilities
    /// through (harness passes a recording double).
    pub runtime: std::sync::Arc<dyn ironclaw_host_runtime::HostRuntime>,
    pub run_context: ironclaw_turns::run_profile::LoopRunContext,
    pub fallback_user_id: ironclaw_host_api::UserId,
    pub workspace_mounts: ironclaw_host_api::MountView,
    pub skill_mounts: ironclaw_host_api::MountView,
    pub memory_mounts: ironclaw_host_api::MountView,
    pub system_extensions_lifecycle_mounts: ironclaw_host_api::MountView,
    /// Input resolver AND [`result_writer`](Self::result_writer) must be two
    /// `Arc::clone`s of the SAME shared io object — production assigns one
    /// `StagedCapabilityIo` to both roles so input-ref/result-ref
    /// correlation by `call_id` works; never source them independently.
    pub input_resolver: std::sync::Arc<dyn ironclaw_loop_host::LoopCapabilityInputResolver>,
    pub result_writer: std::sync::Arc<dyn ironclaw_loop_host::LoopCapabilityResultWriter>,
    pub milestone_sink: std::sync::Arc<dyn ironclaw_turns::run_profile::LoopHostMilestoneSink>,
    /// Opaque handle built by
    /// `test_support::build_skill_context_source_for_test`. Wraps
    /// the crate-private `ComposedSelectableSkillContextSource` so it never
    /// appears in this (public, `test-support`-gated) struct's field types;
    /// the private type is recovered internally via
    /// `SkillActivationTestSource::activation_source` when forwarding to the
    /// production factory.
    pub skill_activation_source:
        Option<std::sync::Arc<crate::test_support::SkillActivationTestSource>>,
    pub project_service: std::sync::Arc<dyn ironclaw_product::ProjectService>,
    /// Backs the `result_read` synthetic capability's durable tool-result
    /// reads; production wires the runtime's session thread service
    /// (`local_dev.rs` `create_capability_port`).
    pub thread_service: std::sync::Arc<dyn ironclaw_threads::SessionThreadService>,
    /// Opaque handle built by
    /// [`build_extension_management_for_test`]. Wraps the
    /// crate-private (`pub(crate)`) `ExtensionManagementPort` so it
    /// never appears in this (public, `test-support`-gated) struct's field
    /// types; mirrors `skill_activation_source` above. Active-extension
    /// registry (installed/activated extensions like `github`, `gmail`, MCP
    /// servers) whose capabilities and provider trust get folded into the
    /// visible-capability grants on every refresh — mirrors production
    /// `capability_wiring`'s
    /// `ExtensionCapabilitySurfaceSource::new(runtime_surfaces.extension_management.clone())`
    /// (`runtime/local_dev.rs:132-133`). `None` (the default a harness gets by
    /// simply omitting extension setup) reproduces the no-op surface this
    /// struct always had before this field existed — extension-lane
    /// capabilities are only visible when the harness actually installs and
    /// activates them AND passes the resulting handle here.
    pub extension_management: Option<ExtensionManagementTestHandle>,
    pub trajectory_observer: Option<std::sync::Arc<dyn crate::RebornTrajectoryObserver>>,
    pub outbound_preferences_facade:
        Option<std::sync::Arc<dyn ironclaw_product::OutboundPreferencesProductFacade>>,
    pub outbound_delivery_target_set_requires_approval: bool,
    /// Per-tool approval-setting overrides; wrapped into the same
    /// `StoreApprovalSettingsProvider` production wires (`local_dev.rs:1002`).
    pub tool_permission_overrides:
        std::sync::Arc<dyn ironclaw_approvals::ToolPermissionOverrideStore>,
    pub auto_approve_settings: std::sync::Arc<dyn ironclaw_approvals::AutoApproveSettingStore>,
    pub persistent_approval_policies:
        std::sync::Arc<dyn ironclaw_approvals::PersistentApprovalPolicyStore>,
    pub approval_requests: std::sync::Arc<dyn ironclaw_run_state::ApprovalRequestStore>,
    pub capability_leases: std::sync::Arc<dyn ironclaw_authorization::CapabilityLeaseStore>,
    /// Durable model-visible gate-record store the built capability port persists
    /// pending-gate records into (§5.2.9).
    pub gate_record_store: std::sync::Arc<dyn ironclaw_run_state::GateRecordStore>,
    /// Durable host-private replay-payload store the built capability port
    /// persists gate/auth replay payloads into and reconstitutes on resume
    /// (§5.3 Stage 2a-i). Must be shared across the harness's turns/threads so a
    /// resume finds the payload its raise persisted.
    pub replay_payload_store: std::sync::Arc<dyn ironclaw_capabilities::ReplayPayloadStore>,
    /// Test-only config extension (empty = production behavior). See
    /// `RefreshingCapabilityPortConfig::capability_execution_mount_overrides`.
    pub capability_execution_mount_overrides:
        std::collections::HashMap<ironclaw_host_api::CapabilityId, ironclaw_host_api::MountView>,
    /// Test-only config extension (empty = production behavior). See
    /// `RefreshingCapabilityPortConfig::additional_provider_trust`.
    pub additional_provider_trust:
        std::collections::BTreeMap<ironclaw_host_api::ExtensionId, ironclaw_trust::TrustDecision>,
    /// Test-only config extension (`None` = production behavior, i.e. no
    /// filtering). See `RefreshingCapabilityPortConfig::capability_id_filter`.
    pub capability_id_filter: Option<std::collections::HashSet<ironclaw_host_api::CapabilityId>>,
    /// Test-only config extension (empty = production behavior). See
    /// `RefreshingCapabilityPortConfig::additional_capability_grants`
    /// — hand-minted grants for capability ids an ad-hoc test-only
    /// `HostRuntime` backend (mock MCP, GitHub/web-access WASM) dispatches
    /// without a real extension activation.
    pub additional_capability_grants: Vec<ironclaw_host_api::CapabilityGrant>,
}

/// Opaque handle (harness-port-seam P1 Change 3) carrying the crate-private
/// `ExtensionManagementPort`. Hides the type from the
/// integration-test crate, which cannot name it (it is only `pub(crate)`
/// inside `ironclaw_reborn_composition`); the private type is recovered
/// internally via [`ExtensionManagementTestHandle::extension_management`]
/// when forwarding to the production factory. Mirrors `SkillActivationTestSource`.
#[cfg(feature = "test-support")]
pub struct ExtensionManagementTestHandle {
    readiness_source: std::sync::Arc<crate::extension_host::lifecycle::LifecycleFacade>,
}

#[cfg(feature = "test-support")]
impl ExtensionManagementTestHandle {
    /// Crate-internal accessor for the caller-scoped readiness facade. Kept
    /// `pub(crate)` (never `pub`) so the crate-private
    /// `LifecycleFacade` type never appears in this crate's public
    /// API; only `runtime::local_dev`'s test-support constructor (which
    /// already names the type) may call this. For tests only -- gated behind
    /// `test-support`, ships zero bytes in production builds.
    pub(crate) fn readiness_source(
        &self,
    ) -> std::sync::Arc<crate::extension_host::lifecycle::LifecycleFacade> {
        self.readiness_source.clone()
    }
}

/// Reads the same `runtime_surfaces.extension_management` handle production's
/// `capability_wiring` reads (`runtime/local_dev.rs:132-133`) off a built
/// `RebornRuntimeStores`, for wiring
/// [`RefreshingCapabilityPortTestParts::extension_management`].
/// `None` when the services were built without a local-dev runtime (mirrors
/// `local_dev_active_extension_authority_for_test`'s `None`-propagation
/// shape), OR when no extension is currently active (matches production:
/// `ExtensionCapabilitySurfaceSource::new` accepts the port either way and
/// `snapshot()` just returns an empty surface); tests that never
/// install/activate an extension can also just omit this call and leave the
/// field `None` for the same no-op surface.
#[cfg(feature = "test-support")]
pub fn build_extension_management_for_test(
    runtime: &crate::RebornRuntime,
) -> Option<ExtensionManagementTestHandle> {
    let mut facade =
        crate::extension_host::lifecycle::LifecycleFacade::new(runtime.skill_management.clone())
            .with_extension_management(runtime.extension_management.clone())
            .with_admin_configuration_resolver(runtime.admin_configuration_resolver.clone())
            .with_runtime_credential_accounts(
                runtime
                    .product_auth
                    .runtime_credential_account_selection_service(),
            );
    if let Some(egress) = runtime.runtime_http_egress.as_ref() {
        facade = facade.with_runtime_http_egress(egress.clone());
    }
    Some(ExtensionManagementTestHandle {
        readiness_source: std::sync::Arc::new(facade),
    })
}

/// Test-support entry point that drives the real
/// `create_refreshing_capability_port` (production's sole port
/// factory) with the harness's injectable parts, supplying the same no-op
/// defaults production uses for the rest. Never hand-rebuilds the wrap order.
#[cfg(feature = "test-support")]
pub async fn create_refreshing_capability_port_for_test(
    parts: RefreshingCapabilityPortTestParts,
) -> Result<
    std::sync::Arc<dyn ironclaw_turns::run_profile::LoopCapabilityPort>,
    ironclaw_turns::run_profile::AgentLoopHostError,
> {
    crate::runtime::create_refreshing_capability_port_for_test(parts).await
}
