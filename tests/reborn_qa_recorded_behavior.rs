//! Recorded-trace coverage for the QA workflow phrases.
//!
//! Three tiers, all over the same committed fixtures in
//! `tests/fixtures/llm_traces/reborn_qa/`:
//!
//! 1. **Recorder tests** (`#[ignore]`, run manually with `ANTHROPIC_API_KEY`
//!    set): drive each QA phrase through a local-dev Reborn runtime backed by
//!    the real Anthropic API and flush the recorded `LlmTrace` fixture. These
//!    are the only tests that spend tokens; everything else is hermetic.
//!
//!    ```bash
//!    ANTHROPIC_API_KEY=... cargo test --test reborn_qa_recorded_behavior \
//!        record_ -- --ignored --test-threads=1 --nocapture
//!    ```
//!
//!    Recording executes the model's chosen capabilities for real under the
//!    local-dev yolo surface (including shell and outbound HTTP) — run it
//!    attended, then review/scrub the fixture per
//!    `tests/support/LIVE_TESTING.md` before committing.
//!
//! 2. **Contract tests**: parse the committed fixture and pin the agent's
//!    tool choices for the phrase — which capability, with which key
//!    arguments. A prompt or tool-surface change that alters behavior shows
//!    up as a contract failure at the next re-record.
//!
//! 3. **Replay tests**: replay the fixture through a real Reborn runtime via
//!    `RebornTraceReplayModelGateway::from_trace` (with recorded
//!    `expected_tool_results` stripped — live tool output contains fresh ids)
//!    and assert the end state, e.g. the routine actually exists with the
//!    right cron after the routine phrases.
//!
//! Contract and replay tests are `#[ignore]`d until the first recording run
//! lands fixtures; flip them on in the same commit that adds the fixtures.

#[allow(dead_code)]
#[path = "support/reborn/mod.rs"]
mod reborn_support;
mod support;

use std::sync::Arc;

use ironclaw_host_api::TenantId;
use reborn_support::{
    model_replay::RebornTraceReplayModelGateway,
    qa_trace::{
        build_qa_trace_runtime, load_qa_trace, qa_trace_tenant_id, record_qa_phrase,
        send_qa_phrase, strip_expected_tool_results,
    },
};
use support::trace_llm::{LlmTrace, TraceResponse};

struct QaPhrase {
    fixture: &'static str,
    phrase: &'static str,
}

const ROUTINE_HEALTH_PING: QaPhrase = QaPhrase {
    fixture: "routine_health_ping",
    phrase: "Every 5 minutes, ping https://cloud-api.near.ai/health and send me a dm in Slack if it does not return a 200.",
};
const ROUTINE_MEETING_PREP: QaPhrase = QaPhrase {
    fixture: "routine_meeting_prep",
    phrase: "Every 30 minutes, send me an email with a summary about the company from my Google Drive and the latest news about the company that I will meet.",
};
const ROUTINE_RELEASE_WATCH: QaPhrase = QaPhrase {
    fixture: "routine_release_watch",
    phrase: "Every 5 minutes, check https://github.com/nearai/ironclaw for latest releases and send me a Slack message summarizing any new ones.",
};
const ROUTINE_CRM_INBOX: QaPhrase = QaPhrase {
    fixture: "routine_crm_inbox",
    phrase: "Every 30 minutes, check my inbox and add any new emails from a near.ai address to my Google Sheet called ABC.",
};
const ROUTINE_HN_MONITOR: QaPhrase = QaPhrase {
    fixture: "routine_hn_monitor",
    phrase: "Every hour, check Hacker News for new posts mentioning 'IronClaw' or 'NEAR AI' and send a summary to Slack.",
};
const WEB_STATUS_CHECK: QaPhrase = QaPhrase {
    fixture: "web_status_check",
    phrase: "check if api.github.com returns a 200 status",
};
const WEB_RELEASE_SUMMARY: QaPhrase = QaPhrase {
    fixture: "web_release_summary",
    phrase: "summarize the latest release from https://github.com/nearai/ironclaw",
};
const WEB_HN_SEARCH: QaPhrase = QaPhrase {
    fixture: "web_hn_search",
    phrase: "search Hacker News for any recent posts mentioning 'IronClaw' or 'NEAR AI'",
};
const CONNECT_TELEGRAM: QaPhrase = QaPhrase {
    fixture: "connect_telegram",
    phrase: "connect to Telegram",
};
const CONNECT_GMAIL: QaPhrase = QaPhrase {
    fixture: "connect_gmail",
    phrase: "connect to Gmail",
};

// --- Tier 1: recorders (live API, manual) ----------------------------------

macro_rules! recorder_test {
    ($name:ident, $case:expr) => {
        #[tokio::test]
        #[ignore = "records against the live Anthropic API; set ANTHROPIC_API_KEY and run explicitly"]
        async fn $name() {
            record_qa_phrase($case.fixture, $case.phrase).await;
        }
    };
}

recorder_test!(record_routine_health_ping, ROUTINE_HEALTH_PING);
recorder_test!(record_routine_meeting_prep, ROUTINE_MEETING_PREP);
recorder_test!(record_routine_release_watch, ROUTINE_RELEASE_WATCH);
recorder_test!(record_routine_crm_inbox, ROUTINE_CRM_INBOX);
recorder_test!(record_routine_hn_monitor, ROUTINE_HN_MONITOR);
recorder_test!(record_web_status_check, WEB_STATUS_CHECK);
recorder_test!(record_web_release_summary, WEB_RELEASE_SUMMARY);
recorder_test!(record_web_hn_search, WEB_HN_SEARCH);
recorder_test!(record_connect_telegram, CONNECT_TELEGRAM);
recorder_test!(record_connect_gmail, CONNECT_GMAIL);

// --- Tier 2: fixture contracts (hermetic) -----------------------------------

/// All tool calls in the fixture as (name, serialized arguments) pairs.
fn recorded_tool_calls(trace: &LlmTrace) -> Vec<(String, String)> {
    trace
        .turns
        .iter()
        .flat_map(|turn| turn.steps.iter())
        .filter_map(|step| match &step.response {
            TraceResponse::ToolCalls { tool_calls, .. } => Some(tool_calls.iter().map(|call| {
                (
                    call.name.clone(),
                    serde_json::to_string(&call.arguments).unwrap_or_default(),
                )
            })),
            _ => None,
        })
        .flatten()
        .collect()
}

fn final_text_reply(trace: &LlmTrace) -> Option<String> {
    trace
        .turns
        .iter()
        .flat_map(|turn| turn.steps.iter())
        .rev()
        .find_map(|step| match &step.response {
            TraceResponse::Text { content, .. } => Some(content.clone()),
            _ => None,
        })
}

fn assert_tool_called_with(trace: &LlmTrace, tool: &str, argument_fragments: &[&str]) {
    let calls = recorded_tool_calls(trace);
    let matched = calls.iter().any(|(name, arguments)| {
        name == tool
            && argument_fragments
                .iter()
                .all(|fragment| arguments.contains(fragment))
    });
    assert!(
        matched,
        "expected a recorded {tool} call with arguments containing {argument_fragments:?}; \
         recorded calls: {calls:#?}"
    );
}

fn assert_routine_contract(case: &QaPhrase, cron_fragment: &str) {
    let trace = load_qa_trace(case.fixture);
    assert_tool_called_with(&trace, "builtin.trigger_create", &[cron_fragment]);
    assert!(
        final_text_reply(&trace).is_some(),
        "routine phrase should end with a finalized assistant reply"
    );
}

#[tokio::test]
#[ignore = "enable once QA fixtures are recorded"]
async fn contract_routine_health_ping_creates_5_minute_trigger() {
    assert_routine_contract(&ROUTINE_HEALTH_PING, "*/5 * * * *");
}

#[tokio::test]
#[ignore = "enable once QA fixtures are recorded"]
async fn contract_routine_meeting_prep_creates_30_minute_trigger() {
    assert_routine_contract(&ROUTINE_MEETING_PREP, "*/30 * * * *");
}

#[tokio::test]
#[ignore = "enable once QA fixtures are recorded"]
async fn contract_routine_release_watch_creates_5_minute_trigger() {
    assert_routine_contract(&ROUTINE_RELEASE_WATCH, "*/5 * * * *");
}

#[tokio::test]
#[ignore = "enable once QA fixtures are recorded"]
async fn contract_routine_crm_inbox_creates_30_minute_trigger() {
    assert_routine_contract(&ROUTINE_CRM_INBOX, "*/30 * * * *");
}

#[tokio::test]
#[ignore = "enable once QA fixtures are recorded"]
async fn contract_routine_hn_monitor_creates_hourly_trigger() {
    assert_routine_contract(&ROUTINE_HN_MONITOR, "0 * * * *");
}

#[tokio::test]
#[ignore = "enable once QA fixtures are recorded"]
async fn contract_web_status_check_fetches_target_endpoint() {
    let trace = load_qa_trace(WEB_STATUS_CHECK.fixture);
    assert_tool_called_with(&trace, "builtin.http", &["api.github.com"]);
}

#[tokio::test]
#[ignore = "enable once QA fixtures are recorded"]
async fn contract_web_release_summary_fetches_release_data() {
    let trace = load_qa_trace(WEB_RELEASE_SUMMARY.fixture);
    assert_tool_called_with(&trace, "builtin.http", &["nearai/ironclaw"]);
    let reply = final_text_reply(&trace).expect("release summary reply");
    assert!(
        !reply.is_empty(),
        "release summary should produce a non-empty reply"
    );
}

#[tokio::test]
#[ignore = "enable once QA fixtures are recorded"]
async fn contract_web_hn_search_queries_for_keywords() {
    let trace = load_qa_trace(WEB_HN_SEARCH.fixture);
    let calls = recorded_tool_calls(&trace);
    assert!(
        calls.iter().any(|(name, arguments)| name == "builtin.http"
            && (arguments.contains("IronClaw") || arguments.contains("NEAR"))),
        "HN search should fetch with the requested keywords; recorded calls: {calls:#?}"
    );
}

#[tokio::test]
#[ignore = "enable once QA fixtures are recorded"]
async fn contract_connect_phrases_route_through_extension_tools() {
    for case in [&CONNECT_TELEGRAM, &CONNECT_GMAIL] {
        let trace = load_qa_trace(case.fixture);
        let calls = recorded_tool_calls(&trace);
        assert!(
            calls
                .iter()
                .any(|(name, _)| name.starts_with("builtin.extension_")),
            "{} should route through the extension lifecycle tools; recorded calls: {calls:#?}",
            case.fixture
        );
    }
}

// --- Tier 3: runtime replay (hermetic) ---------------------------------------

/// Replay a routine-creation fixture through a real local-dev runtime and
/// assert the routine actually exists afterwards with the expected schedule.
async fn replay_routine_phrase(case: &QaPhrase, cron_fragment: &str) {
    let mut trace = load_qa_trace(case.fixture);
    strip_expected_tool_results(&mut trace);
    let gateway =
        RebornTraceReplayModelGateway::from_trace(trace).expect("replay gateway from fixture");

    let root = tempfile::tempdir().expect("tempdir");
    let runtime = build_qa_trace_runtime(&root, Arc::new(gateway.clone())).await;
    let reply = send_qa_phrase(&runtime, case.phrase).await;
    assert!(
        reply.is_successful_final_reply(),
        "replayed {} should finalize a reply; status {:?}",
        case.fixture,
        reply.status
    );
    gateway.assert_exhausted();

    let repo = runtime
        .trigger_repository()
        .expect("local-dev runtime exposes trigger repository");
    let tenant_id = TenantId::new(qa_trace_tenant_id()).expect("tenant id");
    let triggers = repo
        .list_triggers(tenant_id)
        .await
        .expect("list triggers after replay");
    assert!(
        triggers.iter().any(|record| {
            let ironclaw_triggers::TriggerSchedule::Cron { expression, .. } = &record.schedule;
            expression.contains(cron_fragment)
        }),
        "replayed {} should create a routine scheduled {cron_fragment}; triggers: {triggers:#?}",
        case.fixture
    );

    runtime.shutdown().await.expect("runtime shutdown");
}

#[tokio::test]
#[ignore = "enable once QA fixtures are recorded"]
async fn replay_routine_health_ping_creates_real_trigger() {
    replay_routine_phrase(&ROUTINE_HEALTH_PING, "*/5 * * * *").await;
}

#[tokio::test]
#[ignore = "enable once QA fixtures are recorded"]
async fn replay_routine_hn_monitor_creates_real_trigger() {
    replay_routine_phrase(&ROUTINE_HN_MONITOR, "0 * * * *").await;
}
