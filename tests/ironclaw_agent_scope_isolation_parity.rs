#[allow(dead_code)]
#[path = "integration/support/mod.rs"]
mod ironclaw_support;
#[allow(dead_code)]
#[path = "support/ironclaw_parity_qa/mod.rs"]
mod parity_qa_support;
mod support;

use ironclaw_loop_host::HostManagedModelResponse;
use ironclaw_support::harness::{RecordingTestCapabilityPort, test_product_scope};
use ironclaw_threads::{MessageKind, MessageStatus, ThreadMessageRecord};
use ironclaw_turns::TurnStatus;
use parity_qa_support::binary_e2e::{IronClawBinaryE2EHarness, IronClawHarnessSharedStorage};
use parity_qa_support::model_replay::IronClawTraceReplayModelGateway;

#[tokio::test]
async fn ironclaw_agent_scope_isolation_parity() {
    const ROOM: &str = "room-agent-shared";
    const EVENT: &str = "event-agent-shared";

    let shared_storage = IronClawHarnessSharedStorage::new().expect("shared storage");
    let agent_a_scope = test_product_scope(
        "tenant-agent-e2e",
        "host-user",
        "agent-alpha-e2e",
        Some("project-e2e"),
    );
    let agent_b_scope = test_product_scope(
        "tenant-agent-e2e",
        "host-user",
        "agent-beta-e2e",
        Some("project-e2e"),
    );

    let mut agent_a = IronClawBinaryE2EHarness::with_model_gateway_scope_shared_storage(
        ROOM,
        IronClawTraceReplayModelGateway::with_responses([
            HostManagedModelResponse::assistant_reply("agent alpha isolated reply"),
        ]),
        RecordingTestCapabilityPort::echo(),
        agent_a_scope,
        shared_storage.clone(),
    )
    .await
    .expect("agent A harness");
    let mut agent_b = IronClawBinaryE2EHarness::with_model_gateway_scope_shared_storage(
        ROOM,
        IronClawTraceReplayModelGateway::with_responses([
            HostManagedModelResponse::assistant_reply("agent beta isolated reply"),
        ]),
        RecordingTestCapabilityPort::echo(),
        agent_b_scope,
        shared_storage,
    )
    .await
    .expect("agent B harness");

    let alpha = agent_a
        .submit_text_for(ROOM, "alice", EVENT, "agent alpha turn")
        .await
        .expect("submit agent A turn");
    agent_a.start();
    agent_a
        .wait_for_submitted_status(&alpha, TurnStatus::Completed)
        .await
        .expect("agent A completed");
    agent_a.shutdown().await;

    let beta = agent_b
        .submit_text_for(ROOM, "alice", EVENT, "agent beta turn")
        .await
        .expect("submit agent B turn with same external event id");
    agent_b.start();
    agent_b
        .wait_for_submitted_status(&beta, TurnStatus::Completed)
        .await
        .expect("agent B completed");

    assert_ne!(alpha.scope.agent_id, beta.scope.agent_id);
    assert_ne!(
        alpha.thread_id, beta.thread_id,
        "same external conversation under different agents must bind to distinct threads"
    );
    assert_ne!(
        alpha.run_id, beta.run_id,
        "same external event id under different agents must not replay the same run"
    );

    let alpha_history = agent_a
        .history_for_submitted_thread(&alpha)
        .await
        .expect("agent A history");
    let beta_history = agent_b
        .history_for_submitted_thread(&beta)
        .await
        .expect("agent B history");

    assert_history_contains_user(&alpha_history, "agent alpha turn");
    assert_history_contains_assistant(&alpha_history, "agent alpha isolated reply");
    assert_history_excludes(&alpha_history, "agent beta turn");
    assert_history_excludes(&alpha_history, "agent beta isolated reply");

    assert_history_contains_user(&beta_history, "agent beta turn");
    assert_history_contains_assistant(&beta_history, "agent beta isolated reply");
    assert_history_excludes(&beta_history, "agent alpha turn");
    assert_history_excludes(&beta_history, "agent alpha isolated reply");

    agent_a.assert_model_exhausted();
    agent_b.assert_model_exhausted();
    agent_b.shutdown().await;
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
        "thread history should not contain message from another agent: {text:?}"
    );
}
