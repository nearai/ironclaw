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
async fn reborn_tenant_binding_scope_isolation_parity() {
    let shared_storage = RebornHarnessSharedStorage::new().expect("shared storage");
    let tenant_a_scope = test_product_scope(
        "tenant-alpha-e2e",
        "host-user",
        "agent-e2e",
        Some("project-e2e"),
    );
    let tenant_b_scope = test_product_scope(
        "tenant-beta-e2e",
        "host-user",
        "agent-e2e",
        Some("project-e2e"),
    );

    let mut tenant_a =
        RebornBinaryE2EHarness::with_model_gateway_scope_shared_storage_unscoped_worker(
            "room-shared-tenant",
            RebornTraceReplayModelGateway::with_responses([
                HostManagedModelResponse::assistant_reply("tenant alpha isolated reply"),
            ]),
            RecordingTestCapabilityPort::echo(),
            tenant_a_scope,
            shared_storage.clone(),
        )
        .await
        .expect("tenant A harness");
    let mut tenant_b =
        RebornBinaryE2EHarness::with_model_gateway_scope_shared_storage_unscoped_worker(
            "room-shared-tenant",
            RebornTraceReplayModelGateway::with_responses([
                HostManagedModelResponse::assistant_reply("tenant beta isolated reply"),
            ]),
            RecordingTestCapabilityPort::echo(),
            tenant_b_scope,
            shared_storage,
        )
        .await
        .expect("tenant B harness");

    tenant_a.start();
    tenant_b.start();

    let alpha = tenant_a
        .submit_text_for(
            "room-shared-tenant",
            "alice",
            "event-same-external-id",
            "tenant alpha turn",
        )
        .await
        .expect("submit tenant A turn");
    tenant_a
        .wait_for_submitted_status(&alpha, TurnStatus::Completed)
        .await
        .expect("tenant A completed");

    let beta = tenant_b
        .submit_text_for(
            "room-shared-tenant",
            "alice",
            "event-same-external-id",
            "tenant beta turn",
        )
        .await
        .expect("submit tenant B turn with same external event id");
    tenant_b
        .wait_for_submitted_status(&beta, TurnStatus::Completed)
        .await
        .expect("tenant B completed");

    assert_ne!(
        alpha.scope.tenant_id, beta.scope.tenant_id,
        "test must exercise distinct tenant scopes"
    );
    assert_ne!(
        alpha.thread_id, beta.thread_id,
        "same external conversation under different tenants must resolve to distinct canonical threads"
    );
    assert_ne!(
        alpha.run_id, beta.run_id,
        "same external event id under different tenants must not replay the same run"
    );

    let alpha_history = tenant_a
        .history_for_submitted_thread(&alpha)
        .await
        .expect("tenant A history");
    let beta_history = tenant_b
        .history_for_submitted_thread(&beta)
        .await
        .expect("tenant B history");

    assert_history_contains_user(&alpha_history, "tenant alpha turn");
    assert_history_contains_assistant(&alpha_history, "tenant alpha isolated reply");
    assert_history_excludes(&alpha_history, "tenant beta turn");
    assert_history_excludes(&alpha_history, "tenant beta isolated reply");

    assert_history_contains_user(&beta_history, "tenant beta turn");
    assert_history_contains_assistant(&beta_history, "tenant beta isolated reply");
    assert_history_excludes(&beta_history, "tenant alpha turn");
    assert_history_excludes(&beta_history, "tenant alpha isolated reply");

    tenant_a.assert_model_exhausted();
    tenant_b.assert_model_exhausted();

    tenant_a.shutdown().await;
    tenant_b.shutdown().await;
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
        "thread history should not contain message from another tenant: {text:?}"
    );
}
