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
async fn reborn_adapter_installation_scope_isolation_parity() {
    const ROOM: &str = "room-installation-shared";
    const EVENT: &str = "event-installation-shared";

    let shared_storage = RebornHarnessSharedStorage::new().expect("shared storage");
    let scope = test_product_scope(
        "tenant-install-e2e",
        "host-user",
        "agent-e2e",
        Some("project-e2e"),
    );

    let mut install_a =
        RebornBinaryE2EHarness::with_model_gateway_scope_installation_shared_storage(
            ROOM,
            RebornTraceReplayModelGateway::with_responses([
                HostManagedModelResponse::assistant_reply("installation alpha isolated reply"),
            ]),
            RecordingTestCapabilityPort::echo(),
            scope.clone(),
            "reborn-test",
            "install-alpha",
            shared_storage.clone(),
        )
        .await
        .expect("install A harness");
    let mut install_b =
        RebornBinaryE2EHarness::with_model_gateway_scope_installation_shared_storage(
            ROOM,
            RebornTraceReplayModelGateway::with_responses([
                HostManagedModelResponse::assistant_reply("installation beta isolated reply"),
            ]),
            RecordingTestCapabilityPort::echo(),
            scope,
            "reborn-test",
            "install-beta",
            shared_storage,
        )
        .await
        .expect("install B harness");

    let alpha = install_a
        .submit_text_for(ROOM, "alice", EVENT, "installation alpha turn")
        .await
        .expect("submit installation alpha turn");
    install_a.start();
    install_a
        .wait_for_submitted_status(&alpha, TurnStatus::Completed)
        .await
        .expect("install alpha completed");
    install_a.shutdown().await;

    let beta = install_b
        .submit_text_for(ROOM, "alice", EVENT, "installation beta turn")
        .await
        .expect("submit installation beta turn with same external event id");
    install_b.start();
    install_b
        .wait_for_submitted_status(&beta, TurnStatus::Completed)
        .await
        .expect("install beta completed");

    assert_ne!(
        alpha.thread_id, beta.thread_id,
        "same external conversation under different adapter installations must bind to distinct threads"
    );
    assert_ne!(
        alpha.run_id, beta.run_id,
        "same external event id under different adapter installations must not replay the same run"
    );

    let alpha_history = install_a
        .history_for_submitted_thread(&alpha)
        .await
        .expect("install alpha history");
    let beta_history = install_b
        .history_for_submitted_thread(&beta)
        .await
        .expect("install beta history");

    assert_history_contains_user(&alpha_history, "installation alpha turn");
    assert_history_contains_assistant(&alpha_history, "installation alpha isolated reply");
    assert_history_excludes(&alpha_history, "installation beta turn");
    assert_history_excludes(&alpha_history, "installation beta isolated reply");

    assert_history_contains_user(&beta_history, "installation beta turn");
    assert_history_contains_assistant(&beta_history, "installation beta isolated reply");
    assert_history_excludes(&beta_history, "installation alpha turn");
    assert_history_excludes(&beta_history, "installation alpha isolated reply");

    install_a.assert_model_exhausted();
    install_b.assert_model_exhausted();

    install_b.shutdown().await;
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
        "thread history should not contain message from another adapter installation: {text:?}"
    );
}
