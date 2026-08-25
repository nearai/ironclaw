//! Test-only access to the production-composed subagent delivery subsystem.

use std::sync::Arc;

/// Permanently stops the test runtime's turn scheduler.
///
/// Callers must manually drive delivery after this function returns and must
/// not submit any further turns to the stopped scheduler.
pub(crate) async fn parts(
    runtime: &super::RebornRuntime,
) -> crate::test_support::SubagentDeliveryTestParts {
    runtime.turn_scheduler.stop_for_test().await;
    crate::test_support::SubagentDeliveryTestParts {
        turn_tree_store: Arc::clone(&runtime.turn_tree_store),
        resolver: Arc::clone(&runtime.subagent_delivery._resolver),
        store: Arc::clone(&runtime.subagent_delivery._store),
        input_queue: Arc::clone(&runtime.subagent_delivery.input_queue),
    }
}
