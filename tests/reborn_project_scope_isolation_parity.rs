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
async fn reborn_project_scope_isolation_parity() {
    const ROOM: &str = "room-project-shared";
    const EVENT: &str = "event-project-shared";

    let shared_storage = RebornHarnessSharedStorage::new().expect("shared storage");
    let project_a_scope = test_product_scope(
        "tenant-project-e2e",
        "host-user",
        "agent-e2e",
        Some("project-alpha-e2e"),
    );
    let project_b_scope = test_product_scope(
        "tenant-project-e2e",
        "host-user",
        "agent-e2e",
        Some("project-beta-e2e"),
    );

    let mut project_a =
        RebornBinaryE2EHarness::with_model_gateway_scope_shared_storage_unscoped_worker(
            ROOM,
            RebornTraceReplayModelGateway::with_responses([
                HostManagedModelResponse::assistant_reply("project alpha isolated reply"),
            ]),
            RecordingTestCapabilityPort::echo(),
            project_a_scope,
            shared_storage.clone(),
        )
        .await
        .expect("project A harness");
    let mut project_b =
        RebornBinaryE2EHarness::with_model_gateway_scope_shared_storage_unscoped_worker(
            ROOM,
            RebornTraceReplayModelGateway::with_responses([
                HostManagedModelResponse::assistant_reply("project beta isolated reply"),
            ]),
            RecordingTestCapabilityPort::echo(),
            project_b_scope,
            shared_storage,
        )
        .await
        .expect("project B harness");

    let alpha = project_a
        .submit_text_for(ROOM, "alice", EVENT, "project alpha turn")
        .await
        .expect("submit project A turn");
    project_a.start();
    project_a
        .wait_for_submitted_status(&alpha, TurnStatus::Completed)
        .await
        .expect("project A completed");
    project_a.shutdown().await;

    let beta = project_b
        .submit_text_for(ROOM, "alice", EVENT, "project beta turn")
        .await
        .expect("submit project B turn with same external event id");
    project_b.start();
    project_b
        .wait_for_submitted_status(&beta, TurnStatus::Completed)
        .await
        .expect("project B completed");

    assert_ne!(alpha.scope.project_id, beta.scope.project_id);
    assert_ne!(
        alpha.thread_id, beta.thread_id,
        "same external conversation under different projects must bind to distinct threads"
    );
    assert_ne!(
        alpha.run_id, beta.run_id,
        "same external event id under different projects must not replay the same run"
    );

    let alpha_history = project_a
        .history_for_submitted_thread(&alpha)
        .await
        .expect("project A history");
    let beta_history = project_b
        .history_for_submitted_thread(&beta)
        .await
        .expect("project B history");

    assert_history_contains_user(&alpha_history, "project alpha turn");
    assert_history_contains_assistant(&alpha_history, "project alpha isolated reply");
    assert_history_excludes(&alpha_history, "project beta turn");
    assert_history_excludes(&alpha_history, "project beta isolated reply");

    assert_history_contains_user(&beta_history, "project beta turn");
    assert_history_contains_assistant(&beta_history, "project beta isolated reply");
    assert_history_excludes(&beta_history, "project alpha turn");
    assert_history_excludes(&beta_history, "project alpha isolated reply");

    project_a.assert_model_exhausted();
    project_b.assert_model_exhausted();
    project_b.shutdown().await;
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
        "thread history should not contain message from another project: {text:?}"
    );
}
