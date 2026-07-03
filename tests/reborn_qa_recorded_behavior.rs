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
//!    ANTHROPIC_API_KEY=... \
//!    IRONCLAW_REBORN_QA_CREDENTIAL_SOURCE_ROOT=/path/to/reborn/local-dev \
//!      cargo test --test reborn_qa_recorded_behavior record_ \
//!        -- --ignored --test-threads=1 --nocapture
//!    ```
//!
//!    Fixtures that exercise auth-gated Google integrations import the
//!    configured Google product-auth account from the local Reborn store.
//!    By default the source is `$IRONCLAW_REBORN_HOME/local-dev` (or
//!    `~/.ironclaw/reborn/local-dev`) using `[identity]` from
//!    `$IRONCLAW_REBORN_HOME/config.toml`; override with
//!    `IRONCLAW_REBORN_QA_CREDENTIAL_SOURCE_ROOT`,
//!    `IRONCLAW_REBORN_QA_CREDENTIAL_SOURCE_TENANT`,
//!    `IRONCLAW_REBORN_QA_CREDENTIAL_SOURCE_USER`, or
//!    `IRONCLAW_REBORN_QA_CREDENTIAL_SOURCE_AGENT` for non-default local
//!    setups.
//!
//!    Recording executes the model's chosen capabilities for real under the
//!    local-dev yolo surface (including shell and outbound HTTP) — run it
//!    attended, then review/scrub the fixture per
//!    `tests/support/LIVE_TESTING.md` before committing.
//!
//!    Before committing updated fixtures, run:
//!
//!    ```bash
//!    scripts/ci/check-reborn-qa-fixtures.sh
//!    ```
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
//! Contract and replay tests are hermetic and run in CI. Recorder tests stay
//! `#[ignore]` because they spend tokens and may import live credentials.

#[allow(dead_code)]
#[path = "support/reborn/mod.rs"]
mod reborn_support;
mod support;

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use chrono::Utc;
use ironclaw_host_api::TenantId;
use ironclaw_triggers::{TriggerRunStatus, TriggerState};
use reborn_support::{
    model_replay::RebornTraceReplayModelGateway,
    qa_trace::{
        build_qa_trace_runtime_with_http_exchanges,
        build_qa_trace_runtime_with_http_exchanges_and_trigger_poller, load_qa_trace,
        qa_trace_tenant_id, record_qa_phrase, recorded_tool_calls, send_qa_phrase,
        strip_expected_tool_results,
    },
};
use support::trace_llm::{LlmTrace, TraceExpects, TraceResponse, TraceStep, TraceTurn};

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
    phrase: "Every 30 minutes in UTC, use my Google Calendar to find the company for my next upcoming meeting, then send the configured Email delivery target a meeting-prep summary from matching Google Drive files and the latest web news about that company.",
};
const ROUTINE_RELEASE_WATCH: QaPhrase = QaPhrase {
    fixture: "routine_release_watch",
    phrase: "Every 5 minutes in UTC, create a routine that checks the public GitHub releases API for https://github.com/nearai/ironclaw and sends me a Slack message summarizing any new releases. Do not require GitHub account authorization.",
};
const ROUTINE_CRM_INBOX: QaPhrase = QaPhrase {
    fixture: "routine_crm_inbox",
    phrase: "Every 30 minutes in UTC, create a routine that checks my Gmail inbox and adds any new emails from a near.ai address to my Google Sheet called ABC. Do not run the inbox check now.",
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
recorder_test!(record_connect_gmail, CONNECT_GMAIL);

// --- Tier 2: fixture contracts (hermetic) -----------------------------------

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
async fn contract_routine_health_ping_creates_5_minute_trigger() {
    assert_routine_contract(&ROUTINE_HEALTH_PING, "*/5 * * * *");
}

#[tokio::test]
async fn contract_routine_meeting_prep_creates_30_minute_trigger() {
    assert_routine_contract(&ROUTINE_MEETING_PREP, "*/30 * * * *");
}

#[tokio::test]
async fn contract_routine_release_watch_creates_5_minute_trigger() {
    assert_routine_contract(&ROUTINE_RELEASE_WATCH, "*/5 * * * *");
}

#[tokio::test]
async fn contract_routine_crm_inbox_creates_30_minute_trigger() {
    assert_routine_contract(&ROUTINE_CRM_INBOX, "*/30 * * * *");
}

#[tokio::test]
async fn contract_routine_hn_monitor_creates_hourly_trigger() {
    assert_routine_contract(&ROUTINE_HN_MONITOR, "0 * * * *");
}

#[tokio::test]
async fn contract_web_status_check_fetches_target_endpoint() {
    let trace = load_qa_trace(WEB_STATUS_CHECK.fixture);
    assert_tool_called_with(&trace, "builtin.http", &["api.github.com"]);
}

#[tokio::test]
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
async fn contract_connect_gmail_routes_through_extension_tools() {
    let gmail = load_qa_trace(CONNECT_GMAIL.fixture);
    assert_tool_called_with(&gmail, "builtin.extension_install", &["gmail"]);
    assert_tool_called_with(&gmail, "builtin.extension_activate", &["gmail"]);
}

// --- Tier 3: runtime replay (hermetic) ---------------------------------------

/// Replay a routine-creation fixture through a real local-dev runtime and
/// assert the routine actually exists afterwards with the expected schedule.
async fn replay_routine_phrase(case: &QaPhrase, cron_fragment: &str) {
    let mut trace = load_qa_trace(case.fixture);
    let http_exchanges = trace.http_exchanges.clone();
    strip_expected_tool_results(&mut trace);
    let gateway =
        RebornTraceReplayModelGateway::from_trace(trace).expect("replay gateway from fixture");

    let root = tempfile::tempdir().expect("tempdir");
    let runtime = build_qa_trace_runtime_with_http_exchanges(
        &root,
        Arc::new(gateway.clone()),
        http_exchanges,
    )
    .await;
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
            matches!(
                &record.schedule,
                ironclaw_triggers::TriggerSchedule::Cron { expression, .. }
                    if expression.contains(cron_fragment)
            )
        }),
        "replayed {} should create a routine scheduled {cron_fragment}; triggers: {triggers:#?}",
        case.fixture
    );

    runtime.shutdown().await.expect("runtime shutdown");
}

fn append_fired_routine_reply(trace: &mut LlmTrace) {
    trace.turns.push(TraceTurn {
        user_input: "qa trigger fire".to_string(),
        steps: vec![TraceStep {
            request_hint: None,
            response: TraceResponse::Text {
                content: "qa fired routine ok".to_string(),
                input_tokens: 1,
                output_tokens: 1,
            },
            expected_tool_results: Vec::new(),
        }],
        expects: TraceExpects::default(),
    });
}

/// Replay a routine-creation fixture, make the persisted trigger due, and
/// assert the poller submits a real fired turn carrying the recorded prompt.
async fn replay_routine_phrase_fires(case: &QaPhrase, cron_fragment: &str) {
    let mut trace = load_qa_trace(case.fixture);
    let http_exchanges = trace.http_exchanges.clone();
    strip_expected_tool_results(&mut trace);
    append_fired_routine_reply(&mut trace);
    let gateway =
        RebornTraceReplayModelGateway::from_trace(trace).expect("replay gateway from fixture");

    let root = tempfile::tempdir().expect("tempdir");
    let runtime = build_qa_trace_runtime_with_http_exchanges_and_trigger_poller(
        &root,
        Arc::new(gateway.clone()),
        http_exchanges,
    )
    .await;
    let reply = send_qa_phrase(&runtime, case.phrase).await;
    assert!(
        reply.is_successful_final_reply(),
        "replayed {} should finalize creation before firing; status {:?}",
        case.fixture,
        reply.status
    );

    let repo = runtime
        .trigger_repository()
        .expect("local-dev runtime exposes trigger repository");
    let tenant_id = TenantId::new(qa_trace_tenant_id()).expect("tenant id");
    let triggers = repo
        .list_triggers(tenant_id.clone())
        .await
        .expect("list triggers after replay");
    let mut trigger = triggers
        .iter()
        .find(|record| {
            matches!(
                &record.schedule,
                ironclaw_triggers::TriggerSchedule::Cron { expression, .. }
                    if expression.contains(cron_fragment)
            )
        })
        .cloned()
        .unwrap_or_else(|| {
            panic!(
                "replayed {} should create a routine scheduled {cron_fragment}; triggers: {triggers:#?}",
                case.fixture
            )
        });
    let trigger_id = trigger.trigger_id;
    let trigger_prompt = trigger.prompt.clone();
    assert!(
        !trigger_prompt.trim().is_empty(),
        "replayed {} should persist a non-empty routine prompt",
        case.fixture
    );

    trigger.next_run_at = Utc::now() - chrono::Duration::try_seconds(120).expect("duration");
    repo.upsert_trigger(trigger)
        .await
        .expect("make replayed routine due");

    let deadline = Instant::now() + Duration::from_secs(15);
    let mut settled = repo
        .get_trigger(tenant_id.clone(), trigger_id)
        .await
        .expect("get trigger")
        .expect("record present");
    let mut prompt_seen = false;
    while Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(100)).await;
        settled = repo
            .get_trigger(tenant_id.clone(), trigger_id)
            .await
            .expect("get trigger")
            .expect("record present");
        prompt_seen = gateway.requests().iter().any(|request| {
            request
                .messages
                .iter()
                .any(|message| message.content.contains(&trigger_prompt))
        });
        if prompt_seen && settled.last_status == Some(TriggerRunStatus::Ok) {
            break;
        }
    }

    runtime.shutdown().await.expect("runtime shutdown");

    let captured_requests = gateway.requests();
    assert!(
        prompt_seen,
        "replayed {} fired routine never submitted a turn carrying the persisted prompt; \
         prompt: {trigger_prompt:?}; captured: {:?}",
        case.fixture,
        captured_requests
            .iter()
            .map(|request| request
                .messages
                .iter()
                .map(|message| message.content.as_str())
                .collect::<Vec<_>>())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        settled.last_status,
        Some(TriggerRunStatus::Ok),
        "replayed {} fired routine should settle Ok; record: {settled:?}",
        case.fixture
    );
    assert_eq!(
        settled.state,
        TriggerState::Scheduled,
        "replayed {} recurring routine should remain scheduled; record: {settled:?}",
        case.fixture
    );
    assert!(
        settled.last_fired_slot.is_some() && settled.last_run_at.is_some(),
        "replayed {} fired routine should record fire metadata; record: {settled:?}",
        case.fixture
    );
    gateway.assert_exhausted();
}

#[tokio::test]
async fn replay_routine_health_ping_creates_real_trigger() {
    replay_routine_phrase(&ROUTINE_HEALTH_PING, "*/5 * * * *").await;
}

#[tokio::test]
async fn replay_routine_hn_monitor_creates_real_trigger() {
    replay_routine_phrase(&ROUTINE_HN_MONITOR, "0 * * * *").await;
}

#[tokio::test]
async fn replay_routine_health_ping_fires_recorded_automation() {
    replay_routine_phrase_fires(&ROUTINE_HEALTH_PING, "*/5 * * * *").await;
}

#[tokio::test]
async fn replay_routine_hn_monitor_fires_recorded_automation() {
    replay_routine_phrase_fires(&ROUTINE_HN_MONITOR, "0 * * * *").await;
}
