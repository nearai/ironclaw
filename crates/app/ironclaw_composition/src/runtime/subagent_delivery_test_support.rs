//! Test-only access to the production-composed subagent delivery subsystem.

use std::sync::Arc;

pub(crate) async fn parts(
    runtime: &super::RebornRuntime,
) -> crate::test_support::SubagentDeliveryTestParts {
    runtime.turn_scheduler.stop_for_test().await;
    crate::test_support::SubagentDeliveryTestParts {
        turn_tree_store: Arc::clone(&runtime.turn_tree_store),
        resolver: Arc::clone(&runtime.subagent_delivery.resolver),
        store: Arc::clone(&runtime.subagent_delivery.store),
        input_queue: Arc::clone(&runtime.subagent_delivery.input_queue),
    }
}
