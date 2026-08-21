//! Production `StagedCapabilityIo` test support for durable tool-result
//! projection.
//!
//! Drives the same constructor as production's capability wiring, so an
//! integration harness can exercise durable previews and artifact-backed
//! result persistence instead of the ephemeral `ProductLiveCapabilityIo`
//! double.

/// Real `StagedCapabilityIo`, wired like production's `capability_wiring`
/// (`new_with_durable_previews`). Returns two `Arc` clones of ONE underlying
/// io object -- input resolver and result writer MUST share the same object
/// (see `RefreshingCapabilityPortTestParts::input_resolver`'s doc for
/// why: input-ref/result-ref correlation by `call_id` depends on it).
///
/// For tests only -- gated behind `test-support`, ships zero bytes in
/// production builds.
#[cfg(feature = "test-support")]
pub fn staged_capability_io_for_test(
    thread_service: std::sync::Arc<dyn ironclaw_threads::SessionThreadService>,
    fallback_user_id: ironclaw_host_api::ids::UserId,
) -> (
    std::sync::Arc<dyn ironclaw_loop_host::LoopCapabilityInputResolver>,
    std::sync::Arc<dyn ironclaw_loop_host::LoopCapabilityResultWriter>,
) {
    crate::runtime::staged_capability_io_for_test(thread_service, fallback_user_id)
}

#[cfg(feature = "test-support")]
pub fn staged_capability_io_with_observer_for_test(
    thread_service: std::sync::Arc<dyn ironclaw_threads::SessionThreadService>,
    fallback_user_id: ironclaw_host_api::ids::UserId,
    observer: Option<std::sync::Arc<dyn crate::RebornTrajectoryObserver>>,
) -> (
    std::sync::Arc<dyn ironclaw_loop_host::LoopCapabilityInputResolver>,
    std::sync::Arc<dyn ironclaw_loop_host::LoopCapabilityResultWriter>,
) {
    crate::runtime::staged_capability_io_with_observer_for_test(
        thread_service,
        fallback_user_id,
        observer,
    )
}
