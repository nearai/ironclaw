//! Production `LocalDevCapabilityIo` test support (durable tool-result
//! projection seam, issue #5838).
//!
//! Drives the REAL constructor production's `capability_wiring`
//! (`runtime/local_dev.rs`) uses to build the shared input-resolver /
//! result-writer object, so an integration-test harness that opts in
//! exercises durable tool-result persistence (`put_tool_result_record`) and
//! `result_read` continuation instead of the ephemeral `ProductLiveCapabilityIo`
//! test double.

/// Real `LocalDevCapabilityIo`, wired like production's `capability_wiring`
/// (`new_with_durable_previews`). Returns two `Arc` clones of ONE underlying
/// io object -- input resolver and result writer MUST share the same object
/// (see `RefreshingLocalDevCapabilityPortTestParts::input_resolver`'s doc for
/// why: input-ref/result-ref correlation by `call_id` depends on it).
///
/// For tests only -- gated behind `test-support`, ships zero bytes in
/// production builds.
#[cfg(feature = "test-support")]
pub fn local_dev_capability_io_for_test(
    thread_service: std::sync::Arc<dyn ironclaw_threads::SessionThreadService>,
    fallback_user_id: ironclaw_host_api::UserId,
) -> (
    std::sync::Arc<dyn ironclaw_loop_support::LoopCapabilityInputResolver>,
    std::sync::Arc<dyn ironclaw_loop_support::LoopCapabilityResultWriter>,
) {
    crate::runtime::local_dev_capability_io_for_test(thread_service, fallback_user_id)
}
