//! QA use-case coverage for channel (Slack-shaped) inbound flows:
//!
//! - "In Slack, in a DM with IronClaw, ask a detailed strategy question"
//!   → Slack reply that answers the question.
//! - "In Slack, send a message starting with 'bug:'" → the logging action
//!   runs and the bug is acknowledged.
//!
//! Inbound Slack traffic is driven through the binary-e2e harness. Outbound
//! reply-target delivery coverage moved to the `ChannelAdapter` contract in
//! P7b (DEL-5) — see the removed-test note at the bottom of this file.

#[allow(dead_code)]
#[path = "support/reborn_parity_qa/mod.rs"]
mod parity_qa_support;
#[allow(dead_code)]
#[path = "integration/support/mod.rs"]
mod reborn_support;
mod support;

use ironclaw_loop_host::HostManagedModelResponse;
use ironclaw_threads::{MessageKind, MessageStatus};
use ironclaw_turns::TurnStatus;
use parity_qa_support::binary_e2e::{
    RebornBinaryE2EHarness, RebornHarnessSharedStorage, trace_tool_call_response,
};
use parity_qa_support::model_replay::RebornTraceReplayModelGateway;
use reborn_support::harness::{RecordingTestCapabilityPort, test_product_scope};

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

// The retired `ProductAdapter::render_outbound` outbound-delivery test
// (`reborn_qa_slack_outbound_reply_delivers_to_bound_reply_target`) was removed
// in P7b (DEL-5). Live coverage of outbound reply-target delivery and
// per-installation routing lives on the `ChannelAdapter` contract:
// `run_channel_adapter_conformance` (deliver drives the vendor server, every
// part Sent), the `DeliveryCoordinator` suite in
// `crates/ironclaw_product_workflow/tests/outbound_delivery_contract.rs`
// (`coordinator_notice_is_source_routed_and_persists_before_egress` et al.),
// and `tests/reborn_adapter_installation_scope_isolation_parity.rs`.
