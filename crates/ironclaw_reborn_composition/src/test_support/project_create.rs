//! `project_create` synthetic-capability test support (E-PROJ seam).

/// Capability id of the local-dev synthetic `project_create` capability
/// (E-PROJ seam). Single owner is the production constant in
/// `runtime::local_dev::project_create`; the harness references this so its
/// `project_tools()` constructor and assertions never hardcode the string.
#[cfg(feature = "test-support")]
pub const PROJECT_CREATE_CAPABILITY_ID: &str = crate::runtime::PROJECT_CREATE_CAPABILITY_ID;
/// Test-support entry point for the `project_create` synthetic-capability wrap
/// (E-PROJ seam). Lets the integration-test harness inject the synthetic
/// `project_create` capability onto its host-runtime capability port via the
/// real production wrap (`wrap_local_dev_synthetic_capabilities` +
/// `project_create_capability`), so the dispatch path never drifts from
/// production.
#[cfg(feature = "test-support")]
pub fn wrap_project_create_capability_for_test(
    inner: std::sync::Arc<dyn ironclaw_turns::run_profile::LoopCapabilityPort>,
    project_service: std::sync::Arc<dyn ironclaw_product_workflow::ProjectService>,
    fallback_user_id: ironclaw_host_api::UserId,
    run_context: ironclaw_turns::run_profile::LoopRunContext,
    input_resolver: std::sync::Arc<dyn ironclaw_loop_support::LoopCapabilityInputResolver>,
    result_writer: std::sync::Arc<dyn ironclaw_loop_support::LoopCapabilityResultWriter>,
) -> Result<
    std::sync::Arc<dyn ironclaw_turns::run_profile::LoopCapabilityPort>,
    ironclaw_turns::run_profile::AgentLoopHostError,
> {
    crate::runtime::wrap_project_create_capability_for_test(
        inner,
        project_service,
        fallback_user_id,
        run_context,
        input_resolver,
        result_writer,
    )
}
