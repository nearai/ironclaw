#[allow(dead_code)]
#[path = "support/reborn_parity_qa/mod.rs"]
mod parity_qa_support;
#[allow(dead_code)]
#[path = "integration/support/mod.rs"]
mod reborn_support;
mod support;

use std::time::Duration;

use ironclaw_assistant::ProductInboundAck;
use ironclaw_loop_host::HostManagedModelResponse;
use ironclaw_turns::TurnStatus;
use parity_qa_support::binary_e2e::{
    HarnessWaitConfig, RebornBinaryE2EHarness, RebornHarnessSharedStorage,
};
use parity_qa_support::model_replay::RebornTraceReplayModelGateway;
use reborn_support::harness::{RecordingTestCapabilityPort, test_product_scope};

#[tokio::test]
async fn reborn_user_submit_completes_while_another_turn_state_write_is_blocked() {
    const BLOCKED_ROOM: &str = "room-turn-state-lock-free-submit-blocked";
    const LIVE_ROOM: &str = "room-turn-state-lock-free-submit-live";
    // The live submit is still awaited before releasing the blocked writer, so
    // a real lock regression times out here; the wider window only absorbs CI
    // scheduler/build-host jitter around the binary-E2E harness.
    const LOCK_FREE_SUBMIT_TIMEOUT: Duration = Duration::from_secs(5);
    const BLOCKED_SUBMIT_RELEASE_TIMEOUT: Duration = Duration::from_secs(5);

    let shared_storage = RebornHarnessSharedStorage::new().expect("shared storage");
    let scope = test_product_scope(
        "tenant-turn-state-lock-free-submit",
        "host-user",
        "agent-e2e",
        Some("project-e2e"),
    );
    // Both schedulers can claim either durable run from the shared storage.
    // Share the replay queue too, so whichever scheduler wins sees the same
    // two model responses instead of exhausting a harness-local queue.
    let model_gateway = RebornTraceReplayModelGateway::with_responses([
        HostManagedModelResponse::assistant_reply("first submit completed"),
        HostManagedModelResponse::assistant_reply("second submit completed"),
    ]);

    let mut blocked_harness =
        RebornBinaryE2EHarness::with_model_gateway_scope_initial_actor_installation_shared_storage(
            BLOCKED_ROOM,
            "alice",
            model_gateway.clone(),
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
            LIVE_ROOM,
            "alice",
            model_gateway,
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
            .submit_text_for(
                BLOCKED_ROOM,
                "alice",
                "event-turn-state-blocked",
                "blocked writer",
            )
            .await;
        blocked_harness.shutdown().await;
        result
    });

    tokio::time::timeout(
        LOCK_FREE_SUBMIT_TIMEOUT,
        shared_storage.wait_for_blocked_turn_state_put(),
    )
    .await
    .expect("first inbound submit should reach the delayed turn-state write");

    let live = tokio::time::timeout(
        LOCK_FREE_SUBMIT_TIMEOUT,
        live_harness.submit_text_for(LIVE_ROOM, "alice", "event-turn-state-live", "live writer"),
    )
    .await
    .expect("same-user inbound submit must not wait behind the blocked writer")
    .expect("live submit");
    assert!(matches!(live.ack, ProductInboundAck::Accepted { .. }));

    live_harness
        .wait_for_status_in_scope_with_config(
            live.scope.clone(),
            live.run_id,
            TurnStatus::Completed,
            HarnessWaitConfig {
                timeout: LOCK_FREE_SUBMIT_TIMEOUT,
                poll_interval: Duration::from_millis(10),
            },
        )
        .await
        .expect("live run should complete while the first writer remains blocked");

    shared_storage.release_blocked_turn_state_put();
    let blocked = tokio::time::timeout(BLOCKED_SUBMIT_RELEASE_TIMEOUT, blocked_submit)
        .await
        .expect("blocked submit should finish after release")
        .expect("blocked submit task")
        .expect("blocked submit");
    assert!(matches!(blocked.ack, ProductInboundAck::Accepted { .. }));

    live_harness.shutdown().await;
}
