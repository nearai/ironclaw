#[allow(dead_code)]
#[path = "support/reborn/mod.rs"]
mod reborn_support;
mod support;

use std::time::Duration;

use ironclaw_loop_support::HostManagedModelResponse;
use ironclaw_product_adapters::ProductInboundAck;
use ironclaw_turns::TurnStatus;
use reborn_support::harness::{
    RebornBinaryE2EHarness, RebornHarnessSharedStorage, RecordingTestCapabilityPort,
    test_product_scope,
};
use reborn_support::model_replay::RebornTraceReplayModelGateway;

#[tokio::test]
async fn reborn_user_submit_completes_while_another_turn_state_write_is_blocked() {
    const ROOM: &str = "room-turn-state-lock-free-submit";

    let shared_storage = RebornHarnessSharedStorage::new().expect("shared storage");
    let scope = test_product_scope(
        "tenant-turn-state-lock-free-submit",
        "host-user",
        "agent-e2e",
        Some("project-e2e"),
    );

    let mut blocked_harness =
        RebornBinaryE2EHarness::with_model_gateway_scope_initial_actor_installation_shared_storage(
            ROOM,
            "alice",
            RebornTraceReplayModelGateway::with_responses([
                HostManagedModelResponse::assistant_reply("blocked submit eventually completed"),
            ]),
            RecordingTestCapabilityPort::echo(),
            scope.clone(),
            "reborn-test",
            "install-1",
            shared_storage.clone(),
        )
        .await
        .expect("blocked harness");
    let mut live_harness =
        RebornBinaryE2EHarness::with_model_gateway_scope_initial_actor_installation_shared_storage(
            ROOM,
            "alice",
            RebornTraceReplayModelGateway::with_responses([
                HostManagedModelResponse::assistant_reply("live submit completed"),
            ]),
            RecordingTestCapabilityPort::echo(),
            scope,
            "reborn-test",
            "install-1",
            shared_storage.clone(),
        )
        .await
        .expect("live harness");

    blocked_harness.start();
    live_harness.start();

    shared_storage.block_next_turn_state_put();
    let blocked_submit = tokio::spawn(async move {
        let result = blocked_harness
            .submit_text_for(ROOM, "alice", "event-turn-state-blocked", "blocked writer")
            .await;
        blocked_harness.shutdown().await;
        result
    });

    tokio::time::timeout(
        Duration::from_secs(1),
        shared_storage.wait_for_blocked_turn_state_put(),
    )
    .await
    .expect("first inbound submit should reach the delayed turn-state write");

    let live = tokio::time::timeout(
        Duration::from_secs(1),
        live_harness.submit_text_for(ROOM, "alice", "event-turn-state-live", "live writer"),
    )
    .await
    .expect("same-user inbound submit must not wait behind the blocked writer")
    .expect("live submit");
    assert!(matches!(live.ack, ProductInboundAck::Accepted { .. }));

    live_harness
        .wait_for_submitted_status(&live, TurnStatus::Completed)
        .await
        .expect("live run should complete while the first writer remains blocked");

    shared_storage.release_blocked_turn_state_put();
    let blocked = tokio::time::timeout(Duration::from_secs(3), blocked_submit)
        .await
        .expect("blocked submit should finish after release")
        .expect("blocked submit task")
        .expect("blocked submit");
    assert!(matches!(blocked.ack, ProductInboundAck::Accepted { .. }));

    live_harness.shutdown().await;
}
