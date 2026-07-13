//! Production capability-port assembly test support (harness-port-seam P1).
//!
//! Lets the Reborn integration-test harness assemble its capability port
//! through the REAL production factory
//! (`create_refreshing_local_dev_capability_port`,
//! `runtime::local_dev::refreshing_capability_port.rs:75`) instead of hand-
//! rebuilding the wrap order, so every current and future production layer
//! (surface disclosure, external tools, StaleSurface refresh, the shared
//! `LocalDevCapabilityIo`) is automatically exercised by the harness too.

/// Typed bundle of the parts the harness controls, passed by value through
/// the `test_support` -> `runtime` -> `runtime::local_dev` forwarding chain
/// so no layer needs `#[allow(clippy::too_many_arguments)]`. Mirrors
/// `RefreshingLocalDevCapabilityPortConfig` minus the no-op-by-default parts
/// (`extension_surface_source`, `external_tool_catalog`, `policy`).
#[cfg(feature = "test-support")]
pub struct RefreshingLocalDevCapabilityPortTestParts {
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
    /// `LocalDevCapabilityIo` to both roles so input-ref/result-ref
    /// correlation by `call_id` works; never source them independently.
    pub input_resolver: std::sync::Arc<dyn ironclaw_loop_support::LoopCapabilityInputResolver>,
    pub result_writer: std::sync::Arc<dyn ironclaw_loop_support::LoopCapabilityResultWriter>,
    pub milestone_sink: std::sync::Arc<dyn ironclaw_turns::run_profile::LoopHostMilestoneSink>,
    /// Opaque handle built by
    /// `test_support::build_local_dev_skill_context_source_for_test`. Wraps
    /// the crate-private `LocalDevSelectableSkillContextSource` so it never
    /// appears in this (public, `test-support`-gated) struct's field types;
    /// the private type is recovered internally via
    /// `SkillActivationTestSource::activation_source` when forwarding to the
    /// production factory.
    pub skill_activation_source:
        Option<std::sync::Arc<crate::test_support::SkillActivationTestSource>>,
    pub project_service: std::sync::Arc<dyn ironclaw_product_workflow::ProjectService>,
    /// Backs the `result_read` synthetic capability's durable tool-result
    /// reads; production wires the runtime's session thread service
    /// (`local_dev.rs` `create_capability_port`).
    pub thread_service: std::sync::Arc<dyn ironclaw_threads::SessionThreadService>,
    pub trajectory_observer: Option<std::sync::Arc<dyn crate::RebornTrajectoryObserver>>,
    pub outbound_preferences_facade:
        Option<std::sync::Arc<dyn ironclaw_product_workflow::OutboundPreferencesProductFacade>>,
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
    /// Test-only config extension (empty = production behavior). See
    /// `RefreshingLocalDevCapabilityPortConfig::capability_execution_mount_overrides`.
    pub capability_execution_mount_overrides:
        std::collections::HashMap<ironclaw_host_api::CapabilityId, ironclaw_host_api::MountView>,
    /// Test-only config extension (empty = production behavior). See
    /// `RefreshingLocalDevCapabilityPortConfig::additional_provider_trust`.
    pub additional_provider_trust:
        std::collections::BTreeMap<ironclaw_host_api::ExtensionId, ironclaw_trust::TrustDecision>,
    /// Test-only config extension (empty = production behavior, i.e. no
    /// filtering). See `RefreshingLocalDevCapabilityPortConfig::capability_id_filter`.
    pub capability_id_filter: std::collections::HashSet<ironclaw_host_api::CapabilityId>,
}

/// Test-support entry point that drives the real
/// `create_refreshing_local_dev_capability_port` (production's sole port
/// factory) with the harness's injectable parts, supplying the same no-op
/// defaults production uses for the rest. Never hand-rebuilds the wrap order.
#[cfg(feature = "test-support")]
pub async fn create_refreshing_local_dev_capability_port_for_test(
    parts: RefreshingLocalDevCapabilityPortTestParts,
) -> Result<
    std::sync::Arc<dyn ironclaw_turns::run_profile::LoopCapabilityPort>,
    ironclaw_turns::run_profile::AgentLoopHostError,
> {
    crate::runtime::create_refreshing_local_dev_capability_port_for_test(parts).await
}
