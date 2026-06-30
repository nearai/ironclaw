//! QA use-case coverage for channel (Slack-shaped) inbound/outbound flows:
//!
//! - "In Slack, in a DM with IronClaw, ask a detailed strategy question"
//!   → Slack reply that answers the question.
//! - "In Slack, send a message starting with 'bug:'" → the logging action
//!   runs and the bug is acknowledged.
//! - Outbound replies must deliver to the reply target bound to the Slack
//!   installation that received the inbound message.
//!
//! Inbound Slack traffic is simulated through the harness product adapter
//! (verified protocol auth evidence, Slack-shaped adapter installation id);
//! outbound delivery is asserted through the recording delivery sink.

#[allow(dead_code)]
#[path = "support/reborn/mod.rs"]
mod reborn_support;
mod support;

use ironclaw_loop_support::HostManagedModelResponse;
use ironclaw_product_adapters::{
    DeliveryStatus, ExternalConversationRef, FakeProtocolHttpEgress, FinalReplyView,
    ProductAdapter, ProductOutboundEnvelope, ProductOutboundPayload, ProductOutboundTarget,
    ProductRenderOutcome, ProjectionCursor,
};
use ironclaw_threads::{MessageKind, MessageStatus};
use ironclaw_turns::{ReplyTargetBindingRef, TurnRunId, TurnStatus};
use reborn_support::{
    delivery::RecordingOutboundDeliverySink,
    harness::{
        RebornBinaryE2EHarness, RebornHarnessSharedStorage, RecordingTestCapabilityPort,
        test_product_scope, trace_tool_call_response,
    },
    model_replay::RebornTraceReplayModelGateway,
    test_adapter::RebornTestProductAdapter,
};

const SLACK_ADAPTER_ID: &str = "slack-v2";
const SLACK_INSTALLATION_ID: &str = "install-qa-slack";

async fn slack_shaped_harness(
    room: &str,
    model_gateway: RebornTraceReplayModelGateway,
) -> RebornBinaryE2EHarness {
    RebornBinaryE2EHarness::with_model_gateway_scope_installation_shared_storage(
        room,
        model_gateway,
        RecordingTestCapabilityPort::echo(),
        test_product_scope("tenant-qa-slack", "host-user", "agent-qa", None),
        SLACK_ADAPTER_ID,
        SLACK_INSTALLATION_ID,
        RebornHarnessSharedStorage::new().expect("shared storage"),
    )
    .await
    .expect("slack-shaped harness")
}

#[tokio::test]
async fn reborn_qa_slack_dm_strategy_question_gets_reply_in_same_thread() {
    const ROOM: &str = "slack-dm-qa-strategy";
    const QUESTION: &str =
        "What is the NEAR AI strategy on user-owned agents? See the strategy doc.";
    const ANSWER: &str = "Per the NEAR AI Strategy doc, user-owned agents are the core pillar: users keep custody of credentials and data.";

    let mut harness = slack_shaped_harness(
        ROOM,
        RebornTraceReplayModelGateway::with_responses([HostManagedModelResponse::assistant_reply(
            ANSWER,
        )]),
    )
    .await;
    harness.start();

    let submitted = harness
        .submit_text_for(ROOM, "alice", "event-qa-slack-strategy-dm", QUESTION)
        .await
        .expect("submit slack DM question");
    harness
        .wait_for_submitted_status(&submitted, TurnStatus::Completed)
        .await
        .expect("completed run");

    let history = harness
        .history_for_submitted_thread(&submitted)
        .await
        .expect("slack thread history");
    assert!(
        history
            .iter()
            .any(|message| message.kind == MessageKind::User
                && message.status == MessageStatus::Submitted
                && message.content.as_deref() == Some(QUESTION)),
        "inbound Slack DM should land in the bound thread"
    );
    assert!(
        history
            .iter()
            .any(|message| message.kind == MessageKind::Assistant
                && message.status == MessageStatus::Finalized
                && message.content.as_deref() == Some(ANSWER)),
        "the strategy answer should be finalized in the same Slack thread"
    );
    harness.assert_model_exhausted();

    harness.shutdown().await;
}

#[tokio::test]
async fn reborn_qa_slack_bug_prefix_message_runs_logging_action() {
    const ROOM: &str = "slack-dm-qa-bug-logger";
    const BUG_MESSAGE: &str = "bug: login button unresponsive on Safari";
    const ACK: &str = "Added the bug to your bug logging Google Sheet";

    let mut harness = slack_shaped_harness(
        ROOM,
        RebornTraceReplayModelGateway::with_responses([
            trace_tool_call_response(),
            HostManagedModelResponse::assistant_reply(ACK),
        ]),
    )
    .await;
    harness.start();

    let submitted = harness
        .submit_text_for(ROOM, "alice", "event-qa-slack-bug-prefix", BUG_MESSAGE)
        .await
        .expect("submit slack bug message");
    harness
        .wait_for_submitted_status(&submitted, TurnStatus::Completed)
        .await
        .expect("completed run");

    assert_eq!(
        harness.capability_invocations().len(),
        1,
        "the bug-logging action should run exactly once for the bug: message"
    );

    let history = harness
        .history_for_submitted_thread(&submitted)
        .await
        .expect("slack thread history");
    assert!(
        history
            .iter()
            .any(|message| message.kind == MessageKind::User
                && message.content.as_deref() == Some(BUG_MESSAGE)),
        "the bug: message should land in the bound thread"
    );
    assert!(
        history
            .iter()
            .any(|message| message.kind == MessageKind::Assistant
                && message.status == MessageStatus::Finalized
                && message.content.as_deref() == Some(ACK)),
        "the bug-logging acknowledgement should be finalized in the same thread"
    );
    harness.assert_model_exhausted();

    harness.shutdown().await;
}

#[tokio::test]
async fn reborn_qa_slack_outbound_reply_delivers_to_bound_reply_target() {
    let adapter = RebornTestProductAdapter::new(SLACK_ADAPTER_ID, SLACK_INSTALLATION_ID)
        .expect("slack-shaped adapter");
    let target =
        ReplyTargetBindingRef::new("reply:install-qa-slack:dm-alice").expect("reply target");
    let sink = RecordingOutboundDeliverySink::new();
    let egress = FakeProtocolHttpEgress::new(["slack.example.test".to_string()]);

    let envelope = ProductOutboundEnvelope::new(
        adapter.adapter_id().clone(),
        adapter.installation_id().clone(),
        ProductOutboundTarget::new(
            target.clone(),
            ExternalConversationRef::new(None, "dm-alice", None, None).expect("conversation ref"),
            None,
        ),
        ProjectionCursor::new("cursor:qa-slack-outbound").expect("projection cursor"),
        ProductOutboundPayload::FinalReply(FinalReplyView {
            turn_run_id: TurnRunId::new(),
            text: "Here is the summary you asked for".to_string(),
            generated_at: chrono::Utc::now(),
        }),
    );

    let outcome = adapter
        .render_outbound(envelope, &egress, &sink)
        .await
        .expect("render slack outbound");
    assert_eq!(outcome, ProductRenderOutcome::DeliveryRecorded);

    let statuses = sink.statuses();
    assert_eq!(statuses.len(), 1, "exactly one delivery should be recorded");
    assert!(
        matches!(
            &statuses[0],
            DeliveryStatus::Delivered { target: delivered, .. } if delivered == &target
        ),
        "the Slack reply must deliver to the reply target bound to the inbound DM; statuses={statuses:?}"
    );
}
