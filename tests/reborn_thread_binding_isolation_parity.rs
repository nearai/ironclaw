#[allow(dead_code)]
#[path = "support/reborn/mod.rs"]
mod reborn_support;
mod support;

use ironclaw_loop_support::HostManagedModelResponse;
use ironclaw_threads::{MessageKind, MessageStatus, ThreadMessageRecord};
use ironclaw_turns::TurnStatus;
use reborn_support::harness::{RebornBinaryE2EHarness, RecordingTestCapabilityPort};
use reborn_support::model_replay::RebornTraceReplayModelGateway;

#[tokio::test]
async fn reborn_thread_binding_isolation_parity() {
    let model_gateway = RebornTraceReplayModelGateway::with_responses([
        HostManagedModelResponse::assistant_reply("alpha isolated reply"),
        HostManagedModelResponse::assistant_reply("beta isolated reply"),
    ]);
    let mut harness = RebornBinaryE2EHarness::with_model_gateway_unscoped_worker(
        "room-thread-alpha",
        model_gateway,
        RecordingTestCapabilityPort::echo(),
    )
    .await
    .expect("harness");
    harness.start();

    let alpha = harness
        .submit_text_for(
            "room-thread-alpha",
            "alice",
            "event-thread-alpha",
            "alpha turn",
        )
        .await
        .expect("submit alpha turn");
    harness
        .wait_for_submitted_status(&alpha, TurnStatus::Completed)
        .await
        .expect("alpha completed");

    let beta = harness
        .submit_text_for(
            "room-thread-beta",
            "alice",
            "event-thread-beta",
            "beta turn",
        )
        .await
        .expect("submit beta turn");
    harness
        .wait_for_submitted_status(&beta, TurnStatus::Completed)
        .await
        .expect("beta completed");

    assert_ne!(
        alpha.thread_id, beta.thread_id,
        "distinct external conversations must resolve to distinct canonical threads"
    );
    assert_ne!(
        alpha.scope.thread_id, beta.scope.thread_id,
        "submitted turn scopes should remain thread-isolated"
    );

    let alpha_history = harness
        .history_for_thread(alpha.thread_id.clone())
        .await
        .expect("alpha history");
    let beta_history = harness
        .history_for_thread(beta.thread_id.clone())
        .await
        .expect("beta history");

    assert_history_contains_user(&alpha_history, "alpha turn");
    assert_history_contains_assistant(&alpha_history, "alpha isolated reply");
    assert_history_excludes(&alpha_history, "beta turn");
    assert_history_excludes(&alpha_history, "beta isolated reply");

    assert_history_contains_user(&beta_history, "beta turn");
    assert_history_contains_assistant(&beta_history, "beta isolated reply");
    assert_history_excludes(&beta_history, "alpha turn");
    assert_history_excludes(&beta_history, "alpha isolated reply");

    assert_eq!(harness.model_requests().len(), 2);
    harness.assert_model_exhausted();

    harness.shutdown().await;
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
        "thread history should not contain message from another conversation: {text:?}"
    );
}
