//! QA use-case coverage for channel (Slack- and Telegram-shaped) inbound
//! flows:
//!
//! - "In Slack, in a DM with IronClaw, ask a detailed strategy question"
//!   → Slack reply that answers the question.
//! - "In Slack, send a message starting with 'bug:'" → the logging action
//!   runs and the bug is acknowledged.
//! - "In Telegram, ask IronClaw 'summarize the latest BTC news'" → the
//!   summary lands back in the same Telegram thread (UC1).
//! - "In Telegram, ask IronClaw 'Every 5 minutes, send me a telegram
//!   message with a summary of the latest BTC news'" → the routine is
//!   acknowledged in the same Telegram thread (UC1).
//!
//! Inbound Slack/Telegram traffic is driven through the binary-e2e harness.
//! Outbound reply-target delivery coverage moved to the `ChannelAdapter`
//! contract in P7b (DEL-5) — see the removed-test note at the bottom of
//! this file.
//!
//! **Seam split (why the Telegram cases assert a thread-bound reply and not
//! a real fetch/trigger).** The installation-scoped constructor these tests
//! need in order to bind a channel thread
//! (`with_model_gateway_scope_installation_shared_storage`) takes a
//! `RecordingTestCapabilityPort`, not a host runtime, so a channel-shaped
//! harness cannot also dispatch the real `builtin.http` /
//! `builtin.trigger_create` capabilities. Each QA row is therefore pinned
//! in two halves at this tier: the channel half here (inbound normalize →
//! turn → reply persisted in the bound thread, capability dispatched
//! exactly once) and the capability half in the host-runtime suites
//! (`reborn_qa_web_fetch`, `reborn_qa_routines`). Each half names its
//! sibling. The existing Slack cases already work this way.

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
const TELEGRAM_ADAPTER_ID: &str = "telegram-v2";
const TELEGRAM_INSTALLATION_ID: &str = "install-qa-telegram";

async fn slack_shaped_harness(
    room: &str,
    model_gateway: RebornTraceReplayModelGateway,
) -> RebornBinaryE2EHarness {
    channel_shaped_harness(
        room,
        model_gateway,
        "tenant-qa-slack",
        SLACK_ADAPTER_ID,
        SLACK_INSTALLATION_ID,
    )
    .await
}

async fn telegram_shaped_harness(
    room: &str,
    model_gateway: RebornTraceReplayModelGateway,
) -> RebornBinaryE2EHarness {
    channel_shaped_harness(
        room,
        model_gateway,
        "tenant-qa-telegram",
        TELEGRAM_ADAPTER_ID,
        TELEGRAM_INSTALLATION_ID,
    )
    .await
}

/// One installation-scoped harness recipe for every channel QA case. The
/// vendor differs only by adapter/installation id and tenant, so the two
/// channels share this constructor rather than each growing their own —
/// which also keeps a Telegram case from silently diverging into a
/// different seam than the Slack case it is meant to parallel.
async fn channel_shaped_harness(
    room: &str,
    model_gateway: RebornTraceReplayModelGateway,
    tenant: &str,
    adapter_id: &str,
    installation_id: &str,
) -> RebornBinaryE2EHarness {
    RebornBinaryE2EHarness::with_model_gateway_scope_installation_shared_storage(
        room,
        model_gateway,
        RecordingTestCapabilityPort::echo(),
        test_product_scope(tenant, "host-user", "agent-qa", None),
        adapter_id,
        installation_id,
        RebornHarnessSharedStorage::new().expect("shared storage"),
    )
    .await
    .expect("channel-shaped harness")
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

/// UC1 (Daily news digest), channel half: the user asks for the BTC news
/// summary from a Telegram DM and the answer lands back in that same
/// Telegram thread. The fetch half — the real `builtin.http` call against a
/// live loopback server — is
/// `reborn_qa_web_fetch::reborn_qa_btc_news_summary_from_web_search`; see
/// the seam-split note in this file's module doc for why they are separate.
///
/// This is the first Telegram-shaped inbound case at this tier; every prior
/// channel QA case was Slack-shaped, so a Telegram-specific normalize/bind
/// regression had nowhere to fail.
#[tokio::test]
async fn reborn_qa_telegram_dm_btc_news_request_gets_reply_in_same_thread() {
    const ROOM: &str = "telegram-dm-qa-btc-news";
    const QUESTION: &str = "summarize the latest BTC news";
    const ANSWER: &str = "Latest BTC news: spot ETF inflows hit a monthly high, and a core dev proposal to soften the fee market is under review.";

    let mut harness = telegram_shaped_harness(
        ROOM,
        RebornTraceReplayModelGateway::with_responses([
            trace_tool_call_response(),
            HostManagedModelResponse::assistant_reply(ANSWER),
        ]),
    )
    .await;
    harness.start();

    let submitted = harness
        .submit_text_for(ROOM, "alice", "event-qa-telegram-btc-news", QUESTION)
        .await
        .expect("submit telegram BTC news request");
    harness
        .wait_for_submitted_status(&submitted, TurnStatus::Completed)
        .await
        .expect("completed run");

    assert_eq!(
        harness.capability_invocations().len(),
        1,
        "the news lookup should run exactly once for the Telegram request"
    );

    let history = harness
        .history_for_submitted_thread(&submitted)
        .await
        .expect("telegram thread history");
    assert!(
        history
            .iter()
            .any(|message| message.kind == MessageKind::User
                && message.content.as_deref() == Some(QUESTION)),
        "the Telegram question should land in the bound thread"
    );
    assert!(
        history
            .iter()
            .any(|message| message.kind == MessageKind::Assistant
                && message.status == MessageStatus::Finalized
                && message.content.as_deref() == Some(ANSWER)),
        "the BTC news summary should be finalized in the same Telegram thread"
    );
    harness.assert_model_exhausted();

    harness.shutdown().await;
}

/// UC1 (Daily news digest), channel half of the recurring ask: the user
/// sets up the routine from a Telegram DM and the acknowledgement lands
/// back in that same Telegram thread. Routine creation through the real
/// `builtin.trigger_create` is
/// `reborn_qa_routines::reborn_qa_routine_created_for_btc_news_telegram_every_5_minutes`.
#[tokio::test]
async fn reborn_qa_telegram_dm_routine_request_is_acknowledged_in_same_thread() {
    const ROOM: &str = "telegram-dm-qa-btc-routine";
    const REQUEST: &str =
        "Every 5 minutes, send me a telegram message with a summary of the latest BTC news.";
    const ACK: &str = "Routine created: BTC news digest to Telegram every 5 minutes";

    let mut harness = telegram_shaped_harness(
        ROOM,
        RebornTraceReplayModelGateway::with_responses([
            trace_tool_call_response(),
            HostManagedModelResponse::assistant_reply(ACK),
        ]),
    )
    .await;
    harness.start();

    let submitted = harness
        .submit_text_for(ROOM, "alice", "event-qa-telegram-btc-routine", REQUEST)
        .await
        .expect("submit telegram routine request");
    harness
        .wait_for_submitted_status(&submitted, TurnStatus::Completed)
        .await
        .expect("completed run");

    assert_eq!(
        harness.capability_invocations().len(),
        1,
        "routine creation should dispatch exactly once for the Telegram request"
    );

    let history = harness
        .history_for_submitted_thread(&submitted)
        .await
        .expect("telegram thread history");
    assert!(
        history
            .iter()
            .any(|message| message.kind == MessageKind::User
                && message.content.as_deref() == Some(REQUEST)),
        "the Telegram routine request should land in the bound thread"
    );
    assert!(
        history
            .iter()
            .any(|message| message.kind == MessageKind::Assistant
                && message.status == MessageStatus::Finalized
                && message.content.as_deref() == Some(ACK)),
        "the routine acknowledgement should be finalized in the same Telegram thread"
    );
    harness.assert_model_exhausted();

    harness.shutdown().await;
}

/// Cross-usage coverage: a FOLLOW-UP question in the same Telegram DM.
///
/// The QA script stops at the first answer, but the first thing a real
/// tester does is ask about that answer ("which of those is most
/// bullish?"). That second turn is where the cross-surface bugs live:
///
/// - #6349 "Telegram chat history rendered inconsistently in WebUI" — both
///   turns must accumulate in ONE bound thread, which is the same record
///   the WebUI timeline reads. Asserted by reading the thread back and
///   finding all four messages in order.
/// - #1993 "Agent falsely reports task completion after chat is closed and
///   reopened" — the second turn's model request must actually carry the
///   first turn's exchange, not start blank. Asserted against the captured
///   model request rather than the reply text, so a model that merely
///   sounds contextual cannot pass.
#[tokio::test]
async fn reborn_qa_telegram_follow_up_question_carries_thread_history() {
    const ROOM: &str = "telegram-dm-qa-btc-followup";
    const FIRST_ASK: &str = "summarize the latest BTC news";
    const FIRST_ANSWER: &str = "Two stories: spot ETF inflows hit a monthly high, and a core dev proposal to soften the fee market is under review.";
    const FOLLOW_UP: &str = "which of those is most bullish?";
    const FOLLOW_UP_ANSWER: &str = "The spot ETF inflows are the more bullish of the two, since they show sustained institutional demand.";

    let mut harness = telegram_shaped_harness(
        ROOM,
        RebornTraceReplayModelGateway::with_responses([
            HostManagedModelResponse::assistant_reply(FIRST_ANSWER),
            HostManagedModelResponse::assistant_reply(FOLLOW_UP_ANSWER),
        ]),
    )
    .await;
    harness.start();

    let first = harness
        .submit_text_for(ROOM, "alice", "event-qa-telegram-followup-1", FIRST_ASK)
        .await
        .expect("submit first telegram question");
    harness
        .wait_for_submitted_status(&first, TurnStatus::Completed)
        .await
        .expect("first turn completes");

    let second = harness
        .submit_text_for(ROOM, "alice", "event-qa-telegram-followup-2", FOLLOW_UP)
        .await
        .expect("submit telegram follow-up");
    harness
        .wait_for_submitted_status(&second, TurnStatus::Completed)
        .await
        .expect("follow-up turn completes");

    // #6349: one Telegram conversation, one thread — the record the WebUI
    // timeline renders. Both exchanges must be in it.
    let history = harness
        .history_for_submitted_thread(&second)
        .await
        .expect("telegram thread history");
    let transcript: Vec<(MessageKind, String)> = history
        .iter()
        .filter_map(|message| {
            message
                .content
                .as_ref()
                .map(|content| (message.kind, content.clone()))
        })
        .collect();
    for expected in [FIRST_ASK, FIRST_ANSWER, FOLLOW_UP, FOLLOW_UP_ANSWER] {
        assert!(
            transcript.iter().any(|(_, content)| content == expected),
            "the Telegram thread should carry the whole exchange; missing {expected:?} in {transcript:?}"
        );
    }
    assert!(
        history
            .iter()
            .any(|message| message.kind == MessageKind::Assistant
                && message.status == MessageStatus::Finalized
                && message.content.as_deref() == Some(FOLLOW_UP_ANSWER)),
        "the follow-up answer should be finalized in the same Telegram thread"
    );

    // #1993: the follow-up turn must have been ASKED with the prior
    // exchange in context — proving continuity at the model boundary, not
    // just in the persisted transcript.
    let requests = harness.model_requests();
    assert_eq!(requests.len(), 2, "one model call per turn");
    let follow_up_context = format!("{:?}", requests[1]);
    assert!(
        follow_up_context.contains(FIRST_ASK) && follow_up_context.contains(FIRST_ANSWER),
        "the follow-up model request should carry the first exchange, got {follow_up_context}"
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
// `crates/ironclaw_assistant/tests/outbound_delivery_contract.rs`
// (`coordinator_notice_is_source_routed_and_persists_before_egress` et al.),
// and `tests/reborn_adapter_installation_scope_isolation_parity.rs`.
