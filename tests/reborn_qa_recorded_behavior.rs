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
//!    When `ANTHROPIC_API_KEY` is unset the recorder takes the NEAR AI path
//!    (`NEARAI_API_KEY`). Its default model `deepseek-ai/DeepSeek-V4-Flash`
//!    loops on multi-step tasks and dies `Failed(driver_protocol_violation)`;
//!    override it with a strong model served through NEAR AI, e.g.:
//!
//!    ```bash
//!    IRONCLAW_REBORN_QA_CREDENTIAL_SOURCE_USER=me \
//!    IRONCLAW_QA_RECORD_MODEL=anthropic/claude-sonnet-4-6 \
//!    RUST_MIN_STACK=67108864 \
//!      cargo test --test reborn_qa_recorded_behavior record_investigate_ci_job \
//!        -- --ignored --test-threads=1 --nocapture
//!    ```
//!
//!    Two of those are non-obvious on a DB-backed local-dev store: the stored
//!    product-auth accounts live under `user_id = "me"` (not the `reborn-cli`
//!    default), so `IRONCLAW_REBORN_QA_CREDENTIAL_SOURCE_USER=me` is required or
//!    credential import fails with "Visible accounts: <none>"; and the recorder
//!    builds two runtimes plus a live turn, whose combined debug async frame
//!    overflows the default test-thread stack without a larger `RUST_MIN_STACK`.
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
#[path = "support/reborn_parity_qa/mod.rs"]
mod parity_qa_support;
#[allow(dead_code)]
#[path = "integration/support/mod.rs"]
mod reborn_support;
mod support;

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use chrono::Utc;
use ironclaw_host_api::TenantId;
use ironclaw_triggers::{TriggerRunStatus, TriggerState};
use parity_qa_support::model_replay::RebornTraceReplayModelGateway;
use parity_qa_support::qa_trace::{
    build_qa_trace_runtime_with_http_exchanges,
    build_qa_trace_runtime_with_http_exchanges_and_trigger_poller, canonical_recorded_tool_name,
    load_qa_trace, qa_fixture_path, qa_trace_tenant_id, record_qa_phrase, recorded_tool_calls,
    send_qa_phrase, strip_expected_tool_results,
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
// A github task with no credential seeded: the agent should onboard the github
// extension with the single install action and reach the auth gate. Deterministic and
// state-independent — no live PR or CI run involved.
const GITHUB_NOTIFICATIONS: QaPhrase = QaPhrase {
    fixture: "github_notifications",
    phrase: "Check my GitHub notifications and give me a short summary of what needs my attention.",
};

// Investigate one specific, already-completed GitHub Actions job. The job URL is
// pinned to an immutable historical run (conclusion is frozen `failure`) whose
// failure is self-contained in the log (a cargo dependency-resolution conflict),
// so the scenario needs no repository access and does not depend on any open
// PR's live CI state.
const INVESTIGATE_CI_JOB: QaPhrase = QaPhrase {
    fixture: "investigate_ci_job",
    phrase: "Use the github extension to read the logs of this failing GitHub Actions job, then \
             explain in a few sentences what caused it to fail (the reason is in the job's log \
             output): \
             https://github.com/nearai/holonear/actions/runs/29182450888/job/86622570037 . Do not \
             clone the repository, run shell commands, or edit any files.",
};

const SLACK_CHANNEL_MEMBERSHIP_FIXTURE: &str = "slack_channel_membership";
const SLACK_RECENT_MESSAGE_FIXTURE: &str = "slack_recent_message";
const SLACK_MENTION_ENCODING_FIXTURE: &str = "slack_mention_encoding";
const SLACK_ENTITY_HYGIENE_FIXTURE: &str = "slack_entity_hygiene";
const SLACK_SELF_ATTRIBUTION_FIXTURE: &str = "slack_self_attribution";
const SLACK_OOO_STATUS_FIXTURE: &str = "slack_ooo_status";
const SLACK_THREAD_REPLIES_FIXTURE: &str = "slack_thread_replies";
#[derive(serde::Deserialize)]
struct LiveCanaryManifest {
    schema_version: u64,
    selected_cases: Vec<String>,
    no_model_cases: Vec<String>,
    quarantined_model_cases: Vec<String>,
}

fn load_live_canary_manifest() -> LiveCanaryManifest {
    let path = qa_fixture_path("live_canary/case-manifest");
    let contents = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read live-canary manifest {}: {error}", path.display()));
    serde_json::from_str(&contents)
        .unwrap_or_else(|error| panic!("parse live-canary manifest {}: {error}", path.display()))
}

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
recorder_test!(record_github_notifications, GITHUB_NOTIFICATIONS);
recorder_test!(record_investigate_ci_job, INVESTIGATE_CI_JOB);

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

fn assert_tool_sequence(trace: &LlmTrace, expected: &[&str]) {
    let calls = recorded_tool_calls(trace);
    let actual = calls
        .iter()
        .map(|(name, _)| name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(actual, expected, "recorded tool sequence changed");
}

fn assert_tool_not_called(trace: &LlmTrace, forbidden: &str) {
    let calls = recorded_tool_calls(trace);
    assert!(
        calls.iter().all(|(name, _)| name != forbidden),
        "recorded fixture must not call {forbidden}; recorded calls: {calls:#?}"
    );
}

fn assert_tool_call_groups(trace: &LlmTrace, expected: &[&[&str]]) {
    let (user_input, model_responses) = trace
        .steps
        .split_first()
        .expect("recorded fixture should contain a user-input step");
    assert!(
        matches!(user_input.response, TraceResponse::UserInput { .. }),
        "recorded fixture should begin with one user-input step"
    );
    assert_eq!(
        model_responses.len(),
        expected.len() + 1,
        "recorded fixture should contain the expected tool-call response groups followed by one final text response"
    );

    for (index, (step, expected_group)) in model_responses
        .iter()
        .take(expected.len())
        .zip(expected.iter())
        .enumerate()
    {
        let TraceResponse::ToolCalls { tool_calls, .. } = &step.response else {
            panic!("model response {index} should be a tool-call group");
        };
        let actual_group = tool_calls
            .iter()
            .map(|call| canonical_recorded_tool_name(&call.name))
            .collect::<Vec<_>>();
        let expected_group = expected_group
            .iter()
            .map(|name| (*name).to_string())
            .collect::<Vec<_>>();
        assert_eq!(
            actual_group, expected_group,
            "recorded tool-call grouping changed at model response {index}"
        );
    }

    match &model_responses[expected.len()].response {
        TraceResponse::Text { content, .. } => assert!(
            !content.is_empty(),
            "recorded fixture should end with a non-empty final text response"
        ),
        _ => panic!("recorded fixture should end with exactly one final text response"),
    }
}

fn assert_tool_argument_string_field_eq(trace: &LlmTrace, tool: &str, field: &str, expected: &str) {
    let matching_calls = trace
        .steps
        .iter()
        .filter_map(|step| match &step.response {
            TraceResponse::ToolCalls { tool_calls, .. } => Some(tool_calls.iter()),
            _ => None,
        })
        .flatten()
        .filter(|call| canonical_recorded_tool_name(&call.name) == tool)
        .collect::<Vec<_>>();
    assert_eq!(
        matching_calls.len(),
        1,
        "expected exactly one recorded {tool} call before checking its arguments"
    );

    let arguments = matching_calls[0]
        .arguments
        .as_object()
        .unwrap_or_else(|| panic!("recorded {tool} arguments should be a JSON object"));
    assert_eq!(
        arguments.get(field),
        Some(&serde_json::Value::String(expected.to_string())),
        "recorded {tool} argument {field:?} changed"
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
    assert_tool_not_called(&gmail, "builtin.extension_activate");
}

#[tokio::test]
async fn contract_github_notifications_onboards_the_github_extension() {
    // A github task with no credential seeded routes through extension
    // onboarding rather than failing outright.
    let trace = load_qa_trace(GITHUB_NOTIFICATIONS.fixture);
    assert_tool_called_with(&trace, "builtin.extension_install", &["github"]);
    assert_tool_not_called(&trace, "builtin.extension_activate");
}

#[tokio::test]
async fn contract_investigate_ci_job_reads_the_pinned_job_logs() {
    let trace = load_qa_trace(INVESTIGATE_CI_JOB.fixture);
    // Investigation routes through the first-party GitHub extension...
    assert_tool_called_with(&trace, "builtin.extension_install", &["github"]);
    assert_tool_not_called(&trace, "builtin.extension_activate");
    // ...and reads the pinned failing job's logs via the new capability (host
    // follows GitHub's 302 -> blob-storage redirect, stripping the
    // api.github.com Bearer token on the cross-host hop). The plain-text log is
    // delivered to the model as a string (see the wasm_execution output-decode
    // coercion) rather than failing the call.
    assert_tool_called_with(&trace, "github.get_job_logs", &["86622570037"]);
    // Read-only investigation: it must not commit a change to any repo.
    assert_tool_not_called(&trace, "github.create_or_update_file");
    // The root-cause explanation lands in a non-empty final assistant reply.
    let reply = final_text_reply(&trace).expect("investigation phrase should finalize a reply");
    assert!(
        !reply.is_empty(),
        "investigation reply explaining the failure should be non-empty"
    );
}

#[test]
fn canonical_tool_name_folds_provider_escape_to_dot() {
    // NEAR-AI-recorded extension calls escape the dot; the direct-Anthropic path
    // keeps it. Both must canonicalize to one capability-style name.
    assert_eq!(
        canonical_recorded_tool_name("github__get_job_logs"),
        "github.get_job_logs"
    );
    assert_eq!(
        canonical_recorded_tool_name("builtin__extension_install"),
        "builtin.extension_install"
    );
    // Already-dotted names and inner underscores are preserved.
    assert_eq!(canonical_recorded_tool_name("slack.whoami"), "slack.whoami");
    assert_eq!(
        canonical_recorded_tool_name("builtin__get_file_content"),
        "builtin.get_file_content"
    );
}

#[tokio::test]
async fn contract_slack_channel_membership_lists_joined_conversations() {
    let trace = load_qa_trace(SLACK_CHANNEL_MEMBERSHIP_FIXTURE);
    assert_tool_sequence(
        &trace,
        &[
            "builtin.extension_search",
            "builtin.extension_install",
            "slack.list_conversations",
        ],
    );
    assert_tool_call_groups(
        &trace,
        &[
            &["builtin.extension_search"][..],
            &["builtin.extension_install"][..],
            &["slack.list_conversations"][..],
        ],
    );
}

#[tokio::test]
async fn contract_slack_recent_message_reads_the_synthetic_conversation() {
    let trace = load_qa_trace(SLACK_RECENT_MESSAGE_FIXTURE);
    assert_tool_sequence(
        &trace,
        &[
            "builtin.extension_search",
            "builtin.extension_install",
            "slack.whoami",
            "slack.get_conversation_history",
        ],
    );
    assert_tool_call_groups(
        &trace,
        &[
            &["builtin.extension_search"][..],
            &["builtin.extension_install"][..],
            &["slack.whoami"][..],
            &["slack.get_conversation_history"][..],
        ],
    );
    assert_tool_argument_string_field_eq(
        &trace,
        "slack.get_conversation_history",
        "channel",
        "D0CANARY",
    );
    assert_tool_not_called(&trace, "slack.search_messages");
    assert_tool_not_called(&trace, "builtin.outbound_delivery_targets_list");
}

#[tokio::test]
async fn contract_slack_mention_encoding_uses_exact_conversation_lookup() {
    let trace = load_qa_trace(SLACK_MENTION_ENCODING_FIXTURE);
    assert_tool_sequence(
        &trace,
        &[
            "builtin.extension_search",
            "builtin.extension_install",
            "slack.get_conversation_info",
            "slack.send_message",
        ],
    );
    assert_tool_call_groups(
        &trace,
        &[
            &["builtin.extension_search"][..],
            &["builtin.extension_install"][..],
            &["slack.get_conversation_info"][..],
            &["slack.send_message"][..],
        ],
    );
    assert_tool_argument_string_field_eq(
        &trace,
        "slack.get_conversation_info",
        "channel",
        "D0CANARY",
    );
    assert_tool_argument_string_field_eq(&trace, "slack.send_message", "channel", "D0CANARY");
    assert_tool_called_with(
        &trace,
        "slack.send_message",
        &["<@U0CANARY>", "MENTION_CANARY"],
    );
    assert_tool_not_called(&trace, "slack.list_conversations");
}

#[tokio::test]
async fn contract_slack_entity_hygiene_humanizes_the_chained_user_id() {
    let trace = load_qa_trace(SLACK_ENTITY_HYGIENE_FIXTURE);
    assert_tool_sequence(
        &trace,
        &[
            "builtin.extension_search",
            "builtin.extension_install",
            "slack.search_messages",
            "slack.search_messages",
            "slack.search_messages",
            "slack.get_conversation_history",
            "slack.get_user_info",
        ],
    );
    assert_tool_call_groups(
        &trace,
        &[
            &["builtin.extension_search"][..],
            &["builtin.extension_install"][..],
            &["slack.search_messages"][..],
            &["slack.search_messages"][..],
            &["slack.search_messages"][..],
            &["slack.get_conversation_history"][..],
            &["slack.get_user_info"][..],
        ],
    );
    assert_tool_argument_string_field_eq(
        &trace,
        "slack.get_conversation_history",
        "channel",
        "D0CANARY",
    );
    assert_tool_argument_string_field_eq(&trace, "slack.get_user_info", "user_id", "U0CANARY");
    assert_tool_not_called(&trace, "builtin.outbound_delivery_targets_list");

    let reply = final_text_reply(&trace).expect("entity-hygiene fixture should end in text");
    assert!(
        reply.ends_with("Canary User"),
        "entity-hygiene reply should end with the synthetic display name; reply: {reply:?}"
    );
    assert!(
        !reply.contains("U0CANARY"),
        "entity-hygiene reply leaked the synthetic raw user id: {reply:?}"
    );
    assert!(
        !reply.contains("D0CANARY"),
        "entity-hygiene reply leaked the synthetic raw conversation id: {reply:?}"
    );
}

#[tokio::test]
async fn contract_slack_self_attribution_filters_other_senders() {
    let trace = load_qa_trace(SLACK_SELF_ATTRIBUTION_FIXTURE);
    assert_tool_sequence(&trace, &["slack.get_conversation_history", "slack.whoami"]);
    assert_tool_call_groups(
        &trace,
        &[["slack.get_conversation_history", "slack.whoami"].as_slice()],
    );
    assert_tool_argument_string_field_eq(
        &trace,
        "slack.get_conversation_history",
        "channel",
        "D0CANARY",
    );

    let reply = final_text_reply(&trace).expect("self-attribution fixture should end in text");
    assert!(
        reply.contains("SELFMSG_A_1784640084808") && reply.contains("SELFMSG_B_1784640084808"),
        "self-attribution reply should include both current-user markers; reply: {reply:?}"
    );
    assert!(
        !reply.contains("OTHERMSG_C_1784640084808") && !reply.contains("OTHERMSG_D_1784640084808"),
        "self-attribution reply should exclude other-sender markers; reply: {reply:?}"
    );
}

#[tokio::test]
async fn contract_slack_ooo_status_reads_the_connected_user() {
    let trace = load_qa_trace(SLACK_OOO_STATUS_FIXTURE);
    assert_tool_sequence(&trace, &["slack.whoami", "slack.get_user_info"]);
    assert_tool_call_groups(
        &trace,
        &[&["slack.whoami"][..], &["slack.get_user_info"][..]],
    );
    assert_tool_argument_string_field_eq(&trace, "slack.get_user_info", "user_id", "U0CANARY");

    let reply = final_text_reply(&trace).expect("OOO-status fixture should end in text");
    assert!(
        reply.contains("OOO-CANARY-FIXTURE back July 20"),
        "OOO-status reply should preserve the exact synthetic status text; reply: {reply:?}"
    );
}

#[tokio::test]
async fn contract_slack_thread_replies_expands_the_recent_thread() {
    let trace = load_qa_trace(SLACK_THREAD_REPLIES_FIXTURE);
    assert_tool_sequence(
        &trace,
        &[
            "slack.get_conversation_history",
            "builtin.time",
            "slack.get_thread_replies",
        ],
    );
    assert_tool_call_groups(
        &trace,
        &[
            &["slack.get_conversation_history"][..],
            &["builtin.time"][..],
            &["slack.get_thread_replies"][..],
        ],
    );
    assert_tool_argument_string_field_eq(
        &trace,
        "slack.get_conversation_history",
        "channel",
        "D0CANARY",
    );
    assert_tool_argument_string_field_eq(
        &trace,
        "slack.get_thread_replies",
        "thread_ts",
        "1700000000.000000",
    );

    let reply = final_text_reply(&trace).expect("thread-replies fixture should end in text");
    for marker in [
        "REPLY_ONE_1784640131932",
        "REPLY_TWO_1784640131932",
        "REPLY_THREE_1784640131932",
    ] {
        assert!(
            reply.contains(marker),
            "thread-replies reply should include {marker}; reply: {reply:?}"
        );
    }
}

#[test]
fn contract_live_canary_harvested_traces_cover_active_and_quarantined_model_cases() {
    let manifest = load_live_canary_manifest();
    assert_eq!(
        manifest.schema_version, 2,
        "live-canary manifest schema must explicitly account for quarantined traces"
    );
    let selected = manifest
        .selected_cases
        .iter()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        selected.len(),
        manifest.selected_cases.len(),
        "live-canary manifest must not contain duplicate cases"
    );
    let no_model = manifest
        .no_model_cases
        .iter()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    assert!(
        no_model.is_subset(&selected),
        "every no-model case must belong to the selected live-QA inventory"
    );
    let quarantined = manifest
        .quarantined_model_cases
        .iter()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        quarantined.len(),
        manifest.quarantined_model_cases.len(),
        "live-canary manifest must not contain duplicate quarantined cases"
    );
    assert!(
        quarantined.is_subset(&selected),
        "every quarantined case must belong to the selected live-QA inventory"
    );
    assert!(
        quarantined.is_disjoint(&no_model),
        "a case cannot both have no model trace and quarantine a model trace"
    );

    let fixture_dir = qa_fixture_path("live_canary/case-manifest")
        .parent()
        .expect("live-canary fixture directory")
        .to_path_buf();
    let actual_model_cases = std::fs::read_dir(&fixture_dir)
        .expect("read live-canary fixture directory")
        .map(|entry| entry.expect("read live-canary fixture entry").path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .filter_map(|path| {
            let case = path.file_stem()?.to_str()?.to_string();
            (case != "case-manifest").then_some(case)
        })
        .collect::<std::collections::BTreeSet<_>>();
    let expected_model_cases = selected
        .difference(&no_model)
        .filter(|case| !quarantined.contains(*case))
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        actual_model_cases, expected_model_cases,
        "fixture files must exactly match manifest cases that reached the model"
    );

    for case in expected_model_cases {
        let trace = load_qa_trace(&format!("live_canary/{case}"));
        assert!(
            matches!(
                trace.steps.first().map(|step| &step.response),
                Some(TraceResponse::UserInput { .. })
            ),
            "{case} should begin with the harvested user input"
        );
        assert!(
            !trace.expects.tools_used.is_empty(),
            "{case} must declare its required tool contract in the fixture"
        );

        let calls = recorded_tool_calls(&trace);
        assert!(
            calls
                .iter()
                .all(|(name, _)| name != "builtin.extension_activate"),
            "{case} invokes retired builtin.extension_activate and must be quarantined"
        );
        for required_tool in &trace.expects.tools_used {
            assert!(
                calls.iter().any(|(name, _)| name == required_tool),
                "{case} should call {required_tool}; recorded calls: {calls:#?}"
            );
        }
    }

    for case in no_model {
        assert!(
            !qa_fixture_path(&format!("live_canary/{case}")).exists(),
            "{case} is a preflight/connect probe and should not invent a model trace"
        );
    }

    let quarantine_dir = fixture_dir.join("quarantined_retired_activation");
    let actual_quarantined_cases = std::fs::read_dir(&quarantine_dir)
        .expect("read quarantined live-canary fixture directory")
        .map(|entry| entry.expect("read quarantined fixture entry").path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .filter_map(|path| path.file_stem()?.to_str().map(ToString::to_string))
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        actual_quarantined_cases, quarantined,
        "quarantined fixture files must exactly match the promoted manifest"
    );

    for case in quarantined {
        assert!(
            !qa_fixture_path(&format!("live_canary/{case}")).exists(),
            "{case} is quarantined and must not remain in the active fixture directory"
        );
        let trace = load_qa_trace(&format!(
            "live_canary/quarantined_retired_activation/{case}"
        ));
        let calls = recorded_tool_calls(&trace);
        assert!(
            calls
                .iter()
                .any(|(name, _)| name == "builtin.extension_activate"),
            "{case} must contain the retired call that justifies its quarantine"
        );
    }
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

    let repo = runtime.trigger_repository();
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

    let repo = runtime.trigger_repository();
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
