#[allow(dead_code)]
#[path = "support/reborn/mod.rs"]
mod reborn_support;
mod support;

use ironclaw_loop_support::HostManagedModelResponse;
use ironclaw_threads::{MessageKind, MessageStatus, ThreadMessageRecord};
use ironclaw_turns::TurnStatus;
use reborn_support::harness::{
    RebornBinaryE2EHarness, RebornHarnessSharedStorage, RecordingTestCapabilityPort,
    test_product_scope,
};
use reborn_support::model_replay::RebornTraceReplayModelGateway;

#[tokio::test]
async fn reborn_direct_chat_user_scope_isolation_parity() {
    const DIRECT_ROOM: &str = "room-direct-user-shared-id";

    let shared_storage = RebornHarnessSharedStorage::new().expect("shared storage");
    let scope = test_product_scope(
        "tenant-direct-user-e2e",
        "host-user",
        "agent-e2e",
        Some("project-e2e"),
    );

    let mut alice_harness = RebornBinaryE2EHarness::with_model_gateway_scope_initial_actor_installation_shared_storage_unscoped_worker(
        DIRECT_ROOM,
        "alice",
        RebornTraceReplayModelGateway::with_responses([HostManagedModelResponse::assistant_reply(
            "alice direct isolated reply",
        )]),
        RecordingTestCapabilityPort::echo(),
        scope.clone(),
        "reborn-test",
        "install-1",
        shared_storage.clone(),
    )
    .await
    .expect("alice harness");
    let mut bob_harness = RebornBinaryE2EHarness::with_model_gateway_scope_initial_actor_installation_shared_storage_unscoped_worker(
        DIRECT_ROOM,
        "bob",
        RebornTraceReplayModelGateway::with_responses([HostManagedModelResponse::assistant_reply(
            "bob direct isolated reply",
        )]),
        RecordingTestCapabilityPort::echo(),
        scope,
        "reborn-test",
        "install-1",
        shared_storage,
    )
    .await
    .expect("bob harness");

    alice_harness.start();
    bob_harness.start();

    let alice = alice_harness
        .submit_text_for(
            DIRECT_ROOM,
            "alice",
            "event-direct-alice",
            "alice direct turn",
        )
        .await
        .expect("submit alice direct turn");
    alice_harness
        .wait_for_submitted_status(&alice, TurnStatus::Completed)
        .await
        .expect("alice completed");

    let bob = bob_harness
        .submit_text_for(DIRECT_ROOM, "bob", "event-direct-bob", "bob direct turn")
        .await
        .expect("submit bob direct turn");
    bob_harness
        .wait_for_submitted_status(&bob, TurnStatus::Completed)
        .await
        .expect("bob completed");

    assert_ne!(
        alice.thread_scope, bob.thread_scope,
        "direct chat thread scopes must remain owner-user isolated"
    );
    assert_ne!(
        alice.thread_scope.owner_user_id, bob.thread_scope.owner_user_id,
        "direct chat owner user controls thread history isolation"
    );

    let alice_history = alice_harness
        .history_for_submitted_thread(&alice)
        .await
        .expect("alice history");
    let bob_history = bob_harness
        .history_for_submitted_thread(&bob)
        .await
        .expect("bob history");

    assert_history_contains_user(&alice_history, "alice direct turn");
    assert_history_contains_assistant(&alice_history, "alice direct isolated reply");
    assert_history_excludes(&alice_history, "bob direct turn");
    assert_history_excludes(&alice_history, "bob direct isolated reply");

    assert_history_contains_user(&bob_history, "bob direct turn");
    assert_history_contains_assistant(&bob_history, "bob direct isolated reply");
    assert_history_excludes(&bob_history, "alice direct turn");
    assert_history_excludes(&bob_history, "alice direct isolated reply");

    alice_harness.assert_model_exhausted();
    bob_harness.assert_model_exhausted();
    alice_harness.shutdown().await;
    bob_harness.shutdown().await;
}

fn assert_history_contains_user(history: &[ThreadMessageRecord], text: &str) {
    assert!(
        history
            .iter()
            .any(|message| message.kind == MessageKind::User
                && message.status == MessageStatus::Submitted
                && message.content.as_deref() == Some(text)),
        "thread history should contain submitted user message {text:?}"
    );
}

fn assert_history_contains_assistant(history: &[ThreadMessageRecord], text: &str) {
    assert!(
        history
            .iter()
            .any(|message| message.kind == MessageKind::Assistant
                && message.status == MessageStatus::Finalized
                && message.content.as_deref() == Some(text)),
        "thread history should contain finalized assistant reply {text:?}"
    );
}

fn assert_history_excludes(history: &[ThreadMessageRecord], text: &str) {
    assert!(
        history
            .iter()
            .all(|message| message.content.as_deref() != Some(text)),
        "thread history should not contain message from another direct-chat user: {text:?}"
    );
}
