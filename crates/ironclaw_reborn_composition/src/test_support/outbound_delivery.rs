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

/// Test-support entry point for the two `outbound_delivery_*`
/// synthetic-capability wraps (C-SYNTH outbound seam). Lets the
/// integration-test harness inject the synthetic capabilities onto its
/// host-runtime capability port via the real production wrap
/// (`wrap_local_dev_synthetic_capabilities` + `outbound_delivery_capabilities`)
/// over an injected [`OutboundPreferencesProductFacade`] double, so the
/// dispatch, settings-decision, and approval path never drift from production.
/// Mirrors `wrap_project_create_capability_for_test`.
#[cfg(feature = "test-support")]
#[allow(clippy::too_many_arguments)]
pub fn wrap_outbound_delivery_capabilities_for_test(
    inner: std::sync::Arc<dyn ironclaw_turns::run_profile::LoopCapabilityPort>,
    facade: std::sync::Arc<dyn ironclaw_product_workflow::OutboundPreferencesProductFacade>,
    fallback_user_id: ironclaw_host_api::UserId,
    approval_requests: std::sync::Arc<dyn ironclaw_run_state::ApprovalRequestStore>,
    capability_leases: std::sync::Arc<dyn ironclaw_authorization::CapabilityLeaseStore>,
    tool_permission_overrides: std::sync::Arc<dyn ironclaw_approvals::ToolPermissionOverrideStore>,
    auto_approve: std::sync::Arc<dyn ironclaw_approvals::AutoApproveSettingStore>,
    persistent_policies: std::sync::Arc<dyn ironclaw_approvals::PersistentApprovalPolicyStore>,
    target_set_requires_approval: bool,
    run_context: ironclaw_turns::run_profile::LoopRunContext,
    input_resolver: std::sync::Arc<dyn ironclaw_loop_support::LoopCapabilityInputResolver>,
    result_writer: std::sync::Arc<dyn ironclaw_loop_support::LoopCapabilityResultWriter>,
) -> Result<
    std::sync::Arc<dyn ironclaw_turns::run_profile::LoopCapabilityPort>,
    ironclaw_turns::run_profile::AgentLoopHostError,
> {
    crate::runtime::wrap_outbound_delivery_capabilities_for_test(
        inner,
        facade,
        fallback_user_id,
        approval_requests,
        capability_leases,
        tool_permission_overrides,
        auto_approve,
        persistent_policies,
        target_set_requires_approval,
        run_context,
        input_resolver,
        result_writer,
    )
}
