//! `outbound_delivery_*` synthetic-capability test support (C-SYNTH outbound
//! seam).

/// Capability id of the local-dev synthetic `outbound_delivery_targets_list`
/// capability. Single owner is the production constant in
/// `outbound_delivery_capability_surface`; the harness references this so its
/// `outbound_target_tools()` constructor and assertions never hardcode the
/// string.
#[cfg(feature = "test-support")]
pub const OUTBOUND_DELIVERY_TARGETS_LIST_CAPABILITY_ID: &str =
    crate::outbound_delivery_capability_surface::OUTBOUND_DELIVERY_TARGETS_LIST_CAPABILITY_ID;
/// Capability id of the local-dev synthetic `outbound_delivery_target_set`
/// capability. See [`OUTBOUND_DELIVERY_TARGETS_LIST_CAPABILITY_ID`].
#[cfg(feature = "test-support")]
pub const OUTBOUND_DELIVERY_TARGET_SET_CAPABILITY_ID: &str =
    crate::outbound_delivery_capability_surface::OUTBOUND_DELIVERY_TARGET_SET_CAPABILITY_ID;

/// Typed bundle of the settings/store parts needed to wrap the two
/// `outbound_delivery_*` synthetic capabilities for tests (C-SYNTH outbound
/// seam). Passed by value through the `test_support` -> `runtime` ->
/// `runtime::local_dev::outbound_delivery` wrap chain so none of the three
/// layers needs `#[allow(clippy::too_many_arguments)]`.
#[cfg(feature = "test-support")]
pub struct OutboundDeliveryCapabilityTestParts {
    /// Injected facade double the production wrap dispatches through.
    pub facade: std::sync::Arc<dyn ironclaw_product_workflow::OutboundPreferencesProductFacade>,
    /// User id used when a run has no explicit actor.
    pub fallback_user_id: ironclaw_host_api::UserId,
    /// Approval-request store backing the target-set approval gate.
    pub approval_requests: std::sync::Arc<dyn ironclaw_run_state::ApprovalRequestStore>,
    /// Capability-lease store backing approved-dispatch leases.
    pub capability_leases: std::sync::Arc<dyn ironclaw_authorization::CapabilityLeaseStore>,
    /// Per-tool approval-setting overrides.
    pub tool_permission_overrides:
        std::sync::Arc<dyn ironclaw_approvals::ToolPermissionOverrideStore>,
    /// Auto-approve setting store.
    pub auto_approve: std::sync::Arc<dyn ironclaw_approvals::AutoApproveSettingStore>,
    /// Persistent approval-policy store.
    pub persistent_policies: std::sync::Arc<dyn ironclaw_approvals::PersistentApprovalPolicyStore>,
    /// Whether the target-set capability requires approval.
    pub target_set_requires_approval: bool,
    /// Run context the wrapped capabilities execute under.
    pub run_context: ironclaw_turns::run_profile::LoopRunContext,
    /// Input resolver for the wrapped capability port.
    pub input_resolver: std::sync::Arc<dyn ironclaw_loop_support::LoopCapabilityInputResolver>,
    /// Result writer for the wrapped capability port.
    pub result_writer: std::sync::Arc<dyn ironclaw_loop_support::LoopCapabilityResultWriter>,
}

/// Test-support entry point for the two `outbound_delivery_*`
/// synthetic-capability wraps (C-SYNTH outbound seam). Lets the
/// integration-test harness inject the synthetic capabilities onto its
/// host-runtime capability port via the real production wrap
/// (`wrap_local_dev_synthetic_capabilities` + `outbound_delivery_capabilities`)
/// over an injected [`OutboundPreferencesProductFacade`] double, so the
/// dispatch, settings-decision, and approval path never drift from production.
/// Mirrors `wrap_project_create_capability_for_test`.
#[cfg(feature = "test-support")]
pub fn wrap_outbound_delivery_capabilities_for_test(
    inner: std::sync::Arc<dyn ironclaw_turns::run_profile::LoopCapabilityPort>,
    parts: OutboundDeliveryCapabilityTestParts,
) -> Result<
    std::sync::Arc<dyn ironclaw_turns::run_profile::LoopCapabilityPort>,
    ironclaw_turns::run_profile::AgentLoopHostError,
> {
    crate::runtime::wrap_outbound_delivery_capabilities_for_test(inner, parts)
}
