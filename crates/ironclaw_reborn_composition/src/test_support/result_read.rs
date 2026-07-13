//! `result_read` synthetic-capability test support (durable tool-result
//! projection seam, issue #5838).

/// Capability id of the local-dev synthetic `result_read` capability. Single
/// owner is the production constant in `runtime::local_dev::result_read`; the
/// harness references this so its durable-io scenarios and assertions never
/// hardcode the string.
#[cfg(feature = "test-support")]
pub const RESULT_READ_CAPABILITY_ID: &str = crate::runtime::RESULT_READ_CAPABILITY_ID_FOR_TEST;

/// Test-support entry point for the `result_read` synthetic-capability wrap.
/// Lets the integration-test harness inject the synthetic `result_read`
/// capability onto its host-runtime capability port via the real production
/// wrap (`wrap_local_dev_synthetic_capabilities` + `result_read_capability`),
/// so the dispatch path never drifts from production's unconditional wire-in
/// (`refreshing_capability_port.rs`'s `build_inner`).
#[cfg(feature = "test-support")]
pub fn wrap_result_read_capability_for_test(
    inner: std::sync::Arc<dyn ironclaw_turns::run_profile::LoopCapabilityPort>,
    thread_service: std::sync::Arc<dyn ironclaw_threads::SessionThreadService>,
    fallback_user_id: ironclaw_host_api::UserId,
    run_context: ironclaw_turns::run_profile::LoopRunContext,
    input_resolver: std::sync::Arc<dyn ironclaw_loop_support::LoopCapabilityInputResolver>,
    result_writer: std::sync::Arc<dyn ironclaw_loop_support::LoopCapabilityResultWriter>,
) -> Result<
    std::sync::Arc<dyn ironclaw_turns::run_profile::LoopCapabilityPort>,
    ironclaw_turns::run_profile::AgentLoopHostError,
> {
    crate::runtime::wrap_result_read_capability_for_test(
        inner,
        thread_service,
        fallback_user_id,
        run_context,
        input_resolver,
        result_writer,
    )
}
