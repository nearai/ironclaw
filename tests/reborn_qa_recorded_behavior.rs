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
use ironclaw_composition::{AssistantReply, RebornRuntime};
use ironclaw_host_api::ids::{AgentId, TenantId, UserId};
use ironclaw_threads::{MessageKind, MessageStatus, ThreadHistoryRequest, ThreadScope};
use ironclaw_triggers::{TriggerExecutionSpec, TriggerRunStatus, TriggerState};
use ironclaw_turns::{GetRunStateRequest, TurnScope};
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

const ROUTINE_CRM_INBOX: QaPhrase = QaPhrase {
    fixture: "routine_crm_inbox",
    phrase: "Every 30 minutes in UTC, create a routine that checks my Gmail inbox and adds any new emails from a near.ai address to my Google Sheet called ABC. Do not run the inbox check now.",
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

// Source-channel-default matrix cells (the routing UX agreed 2026-08-06: a
// bare "send me" defaults to the channel the request came from; the web app
// default is no delivery step — results are already in the run thread).
// These two are LIVE-recordable today because they are web-app-origin
// conversations. The origin-channel cell (bare "send me" asked from a
// Slack/Telegram conversation pins that channel's target) needs a
// channel-bound conversation in this harness before it can be recorded;
// until then the trigger_create description pin carries the guidance and the
// deterministic delivery journeys prove the fire machinery.
const ROUTINE_BARE_SEND_ME_WEBUI: QaPhrase = QaPhrase {
    fixture: "routine_bare_send_me_webui",
    phrase: "Every morning at 9 in UTC, send me a one-line status of my workspace. Do not run it now.",
};
const ROUTINE_MULTI_CHANNEL_DELIVERY: QaPhrase = QaPhrase {
    fixture: "routine_multi_channel_delivery",
    phrase: "Every morning at 9 in UTC, send me the workspace status to Slack and Telegram. Do not run it now.",
};

const SLACK_CHANNEL_MEMBERSHIP_FIXTURE: &str = "slack_channel_membership";
const SLACK_RECENT_MESSAGE_FIXTURE: &str = "slack_recent_message";
const SLACK_MENTION_ENCODING_FIXTURE: &str = "slack_mention_encoding";
const SLACK_ENTITY_HYGIENE_FIXTURE: &str = "slack_entity_hygiene";
const SLACK_SELF_ATTRIBUTION_FIXTURE: &str = "slack_self_attribution";
const SLACK_OOO_STATUS_FIXTURE: &str = "slack_ooo_status";
const SLACK_THREAD_REPLIES_FIXTURE: &str = "slack_thread_replies";

// Explicit-delivery tool-choice fixtures (design doc
// docs/superpowers/specs/2026-07-27-channel-delivery-tool-design.md §13/14,
// Task 16 of docs/superpowers/plans/2026-07-27-channel-delivery-tool.md):
// prove the model reaches for the model-initiated `builtin.outbound_deliver`
// capability -- never the act-as-user `slack.send_message` -- both for an
// interactive "send me X on Slack" ask and for a routine's own fire when its
// stored prompt carries an explicit delivery step written at creation time.
// No live LLM credentials are available in this environment, so these are
// hand-authored scripted traces (same convention as `SLACK_RECENT_MESSAGE_FIXTURE`
// and its siblings above -- see that fixture's "synthetic-..." `model_name`),
// not `record_qa_phrase` recordings; there is deliberately no `recorder_test!`
// registration for them, matching the existing scripted fixtures' precedent.
const SLACK_SUMMARY_DELIVERY_FIXTURE: &str = "slack_summary_delivery";
const ROUTINE_STATUS_DIGEST_DELIVERY_FIXTURE: &str = "routine_status_digest_delivery";
// Task 11 carry: successor for the four `outbound_delivery_target_set` replay
// fixtures deleted with that capability -- "set my notification channels to
// X and Y" must call `builtin.notification_channels_set` with both ids.
// Both X and Y must be real channels: Slack and Telegram are the only
// first-party extensions that declare a `[channel]` capability surface
// (`crates/extensions/packages/{slack,telegram}/manifest.toml`);
// Gmail has none, so an `email:*` id can never appear in the real
// `outbound_delivery_targets_list` catalog and the real service would reject
// it as an unknown target -- an earlier revision of this fixture used
// `email:qa-trace-inbox` and was fixed in review (see the Fix Report in
// task-16-report.md). `"telegram:qa-trace-dm"` mirrors the exact model-facing
// catalog-id convention `tests/integration/delivery_user_journeys.rs` uses
// for a real Telegram delivery target (`CROSS_TELEGRAM_TARGET_ID =
// "telegram:journey-dm"`) -- not to be confused with the internal
// `tg:<chat_id>:_:_` binding-ref encoding that same file documents, which
// the model never sees.
const NOTIFICATION_CHANNELS_SET_SLACK_AND_TELEGRAM_FIXTURE: &str =
    "notification_channels_set_slack_and_telegram";

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

/// Every `live_canary/quarantined_*/` subdirectory is one quarantine group: a
/// directory of unmodified traces plus the specific recorded-call condition
/// that justifies removing that case from active dispatch. The inventory is
/// discovered from disk so a newly added `quarantined_*` directory cannot be
/// invisible to the manifest-coverage check; it must also gain a matching arm
/// in `recorded_calls_justify_quarantine` or the contract fails loudly.
fn quarantine_group_names(root: &std::path::Path) -> std::collections::BTreeSet<String> {
    std::fs::read_dir(root)
        .unwrap_or_else(|error| {
            panic!(
                "read live-canary fixture directory {}: {error}",
                root.display()
            )
        })
        .map(|entry| entry.expect("read live-canary fixture entry").path())
        .filter(|path| path.is_dir())
        .filter_map(|path| {
            let name = path.file_name()?.to_str()?;
            name.starts_with("quarantined_").then(|| name.to_string())
        })
        .collect()
}

/// File stems (case IDs) of every `.json` trace directly inside `group_dir`.
fn quarantine_group_cases(group_dir: &std::path::Path) -> std::collections::BTreeSet<String> {
    std::fs::read_dir(group_dir)
        .unwrap_or_else(|error| {
            panic!(
                "read quarantined live-canary fixture directory {}: {error}",
                group_dir.display()
            )
        })
        .map(|entry| entry.expect("read quarantined fixture entry").path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .filter_map(|path| path.file_stem()?.to_str().map(ToString::to_string))
        .collect()
}

/// The recorded-call condition that justifies each quarantine group's cause.
fn recorded_calls_justify_quarantine(group_name: &str, calls: &[(String, String)]) -> bool {
    match group_name {
        "quarantined_retired_activation" => calls
            .iter()
            .any(|(name, _)| name == "builtin.extension_activate"),
        "quarantined_stale_slack_canonicalization" => {
            // Same retired-field markers the standardized messaging framework
            // canonicalized away (`channel`, `thread_ts`, `user_id`, `types`,
            // `count`, `sort`); `arguments` is the JSON-serialized call
            // arguments produced by `normalized_argument_text`.
            const RETIRED_ARGUMENT_KEYS: [&str; 6] =
                ["channel", "thread_ts", "user_id", "types", "count", "sort"];
            calls.iter().any(|(name, arguments)| {
                if !name.starts_with("slack.") {
                    return false;
                }
                serde_json::from_str::<serde_json::Value>(arguments)
                    .ok()
                    .and_then(|value| value.as_object().cloned())
                    .is_some_and(|object| {
                        RETIRED_ARGUMENT_KEYS
                            .iter()
                            .any(|key| object.contains_key(*key))
                    })
            })
        }
        other => panic!("no quarantine justification defined for group {other}"),
    }
}

#[test]
fn stale_slack_quarantine_matches_argument_keys_not_values() {
    let canonical = vec![(
        "slack.list_conversations".to_string(),
        r#"{"kinds":["channel"]}"#.to_string(),
    )];
    assert!(!recorded_calls_justify_quarantine(
        "quarantined_stale_slack_canonicalization",
        &canonical
    ));

    let retired = vec![(
        "slack.get_conversation_history".to_string(),
        r#"{"channel":"C1"}"#.to_string(),
    )];
    assert!(recorded_calls_justify_quarantine(
        "quarantined_stale_slack_canonicalization",
        &retired
    ));
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

recorder_test!(record_routine_crm_inbox, ROUTINE_CRM_INBOX);
recorder_test!(
    record_routine_bare_send_me_webui,
    ROUTINE_BARE_SEND_ME_WEBUI
);
recorder_test!(
    record_routine_multi_channel_delivery,
    ROUTINE_MULTI_CHANNEL_DELIVERY
);
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
    assert_structured_trigger_create(&trace);
    assert!(
        final_text_reply(&trace).is_some(),
        "routine phrase should end with a finalized assistant reply"
    );
}

fn assert_structured_trigger_create(trace: &LlmTrace) {
    // Every recorded creation call must satisfy the versioned contract —
    // substring checks on the first call would let a later legacy call or a
    // malformed `execution_contract: null` slip through.
    let creates = recorded_tool_calls(trace)
        .into_iter()
        .filter(|(name, _)| name == "builtin.trigger_create")
        .map(|(_, arguments)| arguments)
        .collect::<Vec<_>>();
    assert!(!creates.is_empty(), "routine phrase must create a trigger");
    for arguments in creates {
        let parsed: serde_json::Value = serde_json::from_str(&arguments).unwrap_or_else(|error| {
            panic!("trigger_create arguments must be a JSON object ({error}): {arguments}")
        });
        let object = parsed
            .as_object()
            .unwrap_or_else(|| panic!("trigger_create arguments must be an object: {arguments}"));
        assert!(
            !object.contains_key("prompt"),
            "routine creation fixtures must not use the retired raw prompt field: {arguments}"
        );
        let contract = object.get("execution_contract").unwrap_or_else(|| {
            panic!(
                "routine creation fixtures must exercise the structured execution contract: {arguments}"
            )
        });
        let spec: TriggerExecutionSpec =
            serde_json::from_value(contract.clone()).unwrap_or_else(|error| {
                panic!(
                    "execution_contract must deserialize as the versioned TriggerExecutionSpec ({error}): {arguments}"
                )
            });
        spec.validate().unwrap_or_else(|error| {
            panic!("execution_contract must pass contract validation ({error}): {arguments}")
        });
    }
}

#[tokio::test]
async fn contract_routine_crm_inbox_creates_30_minute_trigger() {
    assert_routine_contract(&ROUTINE_CRM_INBOX, "*/30 * * * *");
}

#[tokio::test]
async fn contract_routine_bare_send_me_from_web_app_pins_no_delivery_step() {
    let trace = load_qa_trace(ROUTINE_BARE_SEND_ME_WEBUI.fixture);
    assert_structured_trigger_create(&trace);
    // Web-app half of the source-channel default: results are already in the
    // run thread the user is looking at, so a bare "send me" writes NO
    // delivery step and the creation turn performs no delivery itself.
    assert_tool_called_with(&trace, "builtin.trigger_create", &["0 9 * * *"]);
    assert!(
        !recorded_tool_calls(&trace)
            .iter()
            .any(|(name, arguments)| name == "builtin.trigger_create"
                && arguments.contains("outbound_deliver")),
        "a web-app bare send-me routine must not embed a delivery step"
    );
    assert_tool_not_called(&trace, "builtin.outbound_deliver");
    assert!(
        final_text_reply(&trace).is_some(),
        "routine phrase should end with a finalized assistant reply"
    );
}

#[tokio::test]
async fn contract_routine_multi_channel_delivery_pins_both_targets_in_prompt() {
    let trace = load_qa_trace(ROUTINE_MULTI_CHANNEL_DELIVERY.fixture);
    assert_structured_trigger_create(&trace);
    // "to Slack and Telegram" resolves BOTH destinations while the user is
    // present and pins each in the routine's own prompt as an explicit
    // delivery step — one delivery call per channel at fire time.
    assert_tool_called_with(&trace, "builtin.outbound_delivery_targets_list", &[]);
    let create_arguments = recorded_tool_calls(&trace)
        .iter()
        .find(|(name, _)| name == "builtin.trigger_create")
        .map(|(_, arguments)| arguments.clone())
        .expect("multi-channel routine phrase must create a trigger");
    for needle in [
        "outbound_deliver",
        "slack:qa-trace-dm",
        "telegram:qa-trace-dm",
        "0 9 * * *",
    ] {
        assert!(
            create_arguments.contains(needle),
            "trigger_create must pin {needle} in the routine prompt; arguments: {create_arguments}"
        );
    }
    assert_tool_not_called(&trace, "slack.send_message");
    assert!(
        final_text_reply(&trace).is_some(),
        "routine phrase should end with a finalized assistant reply"
    );
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
    assert_tool_called_with(&trace, "builtin.skill_activate", &["github"]);
    assert_tool_called_with(&trace, "builtin.extension_install", &["github"]);
    assert_tool_not_called(&trace, "builtin.extension_activate");
}

#[tokio::test]
async fn contract_investigate_ci_job_reads_the_pinned_job_logs() {
    let trace = load_qa_trace(INVESTIGATE_CI_JOB.fixture);
    // The recorded model selected the listed GitHub skill by its exact name
    // before using its extension workflow.
    assert_tool_called_with(&trace, "builtin.skill_activate", &["github"]);
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

#[tokio::test]
async fn contract_slack_summary_delivery_uses_outbound_deliver_not_send_message() {
    let trace = load_qa_trace(SLACK_SUMMARY_DELIVERY_FIXTURE);
    // An interactive "send me a summary of X on Slack" ask resolves the
    // destination, then delivers through the model-initiated capability --
    // never the act-as-user Slack tool, which would DM from the user's own
    // account instead of the bot (see prompts/slack/send_message.md).
    assert_tool_sequence(
        &trace,
        &[
            "builtin.outbound_delivery_targets_list",
            "builtin.outbound_deliver",
        ],
    );
    assert_tool_argument_string_field_eq(
        &trace,
        "builtin.outbound_deliver",
        "target_id",
        "slack:qa-trace-dm",
    );
    assert_tool_called_with(
        &trace,
        "builtin.outbound_deliver",
        &["slack:qa-trace-dm", "SLACK_SUMMARY_DELIVERY_CANARY"],
    );
    assert_tool_not_called(&trace, "slack.send_message");

    let reply = final_text_reply(&trace).expect("delivery fixture should end with a reply");
    assert!(
        reply.contains("SLACK_SUMMARY_DELIVERY_CANARY"),
        "reply should confirm the delivery; reply: {reply:?}"
    );
}

#[tokio::test]
async fn contract_routine_status_digest_delivery_fire_calls_outbound_deliver_not_send_message() {
    let trace = load_qa_trace(ROUTINE_STATUS_DIGEST_DELIVERY_FIXTURE);
    // Creation turn: the destination is resolved and pinned into the
    // routine's own prompt while the user is present; `trigger_create`
    // itself never carries a destination -- the retired `delivery_target_id`
    // field is gone from the schema, so a fresh call cannot smuggle it back in.
    assert_tool_called_with(&trace, "builtin.trigger_create", &["0 9 * * *"]);
    assert!(
        !recorded_tool_calls(&trace)
            .iter()
            .any(|(name, arguments)| name == "builtin.trigger_create"
                && arguments.contains("delivery_target_id")),
        "trigger_create must not carry the retired delivery_target_id field"
    );
    // Fire turn: the model reads its own persisted prompt (which carries the
    // explicit delivery step) and calls the delivery capability itself --
    // nothing delivers a fire's reply automatically in the explicit-delivery
    // world, and the act-as-user Slack tool must not be used for it either.
    assert_tool_argument_string_field_eq(
        &trace,
        "builtin.outbound_deliver",
        "target_id",
        "slack:qa-trace-dm",
    );
    assert_tool_called_with(
        &trace,
        "builtin.outbound_deliver",
        &[
            "slack:qa-trace-dm",
            "ROUTINE_STATUS_DIGEST_DELIVERED_CANARY",
        ],
    );
    assert_tool_not_called(&trace, "slack.send_message");

    let reply = final_text_reply(&trace).expect("fired routine should end with a reply");
    assert!(
        reply.contains("ROUTINE_STATUS_DIGEST_DELIVERED_CANARY"),
        "fired routine's reply should confirm delivery; reply: {reply:?}"
    );
}

#[tokio::test]
async fn contract_notification_channels_set_slack_and_telegram_sets_both_targets() {
    let trace = load_qa_trace(NOTIFICATION_CHANNELS_SET_SLACK_AND_TELEGRAM_FIXTURE);
    // Task 11 carry: successor for the four `outbound_delivery_target_set`
    // replay fixtures deleted with that capability. Both targets are real
    // channels (Slack and Telegram both declare a `[channel]` manifest
    // surface; Gmail does not, so an `email:*` id could never reach this
    // catalog in production -- see the const-level comment above).
    assert_tool_called_with(
        &trace,
        "builtin.notification_channels_set",
        &[
            "\"target_ids\"",
            "slack:qa-trace-dm",
            "telegram:qa-trace-dm",
        ],
    );
    // Setting notification channels is not a delivery act: it must not also
    // reach for the explicit delivery tool or an act-as-user send tool.
    assert_tool_not_called(&trace, "builtin.outbound_deliver");
    assert_tool_not_called(&trace, "slack.send_message");
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

    let mut actual_quarantined_cases = std::collections::BTreeSet::new();
    for group_name in quarantine_group_names(&fixture_dir) {
        let cases = quarantine_group_cases(&fixture_dir.join(&group_name));
        assert!(
            actual_quarantined_cases.is_disjoint(&cases),
            "{group_name} contains a case already claimed by another quarantine group"
        );
        for case in &cases {
            assert!(
                !qa_fixture_path(&format!("live_canary/{case}")).exists(),
                "{case} is quarantined and must not remain in the active fixture directory"
            );
            let trace = load_qa_trace(&format!("live_canary/{group_name}/{case}"));
            let calls = recorded_tool_calls(&trace);
            assert!(
                recorded_calls_justify_quarantine(&group_name, &calls),
                "{case} in {group_name} must contain the call that justifies its quarantine"
            );
        }
        actual_quarantined_cases.extend(cases);
    }
    assert_eq!(
        actual_quarantined_cases, quarantined,
        "quarantined fixture files (across every quarantined_*/ group directory) must \
         exactly match the promoted manifest"
    );
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
    let failure_detail = if reply.is_successful_final_reply() {
        None
    } else {
        replay_failure_detail(&runtime, &reply).await
    };
    assert!(
        reply.is_successful_final_reply(),
        "replayed {} should finalize a reply; status {:?}, failure_category {:?}, text {:?}, failure_detail {:?}",
        case.fixture,
        reply.status,
        reply.failure_category,
        reply.text,
        failure_detail
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
                content: "qa fired routine ok.".to_string(),
                input_tokens: 1,
                output_tokens: 1,
            },
            expected_tool_results: Vec::new(),
        }],
        expects: TraceExpects::default(),
    });
}

async fn replay_failure_detail(runtime: &RebornRuntime, reply: &AssistantReply) -> Option<String> {
    let scope = TurnScope::new_with_owner(
        TenantId::new(qa_trace_tenant_id()).ok()?,
        Some(AgentId::new("qa-trace-agent").ok()?),
        None,
        reply.conversation.0.clone(),
        Some(UserId::new("qa-trace-owner").ok()?),
    );
    runtime
        .turn_coordinator_for_test()
        .get_run_state(GetRunStateRequest {
            scope,
            run_id: reply.run_id,
        })
        .await
        .ok()
        .and_then(|state| state.failure.map(|failure| format!("{failure:?}")))
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
    let failure_detail = if reply.is_successful_final_reply() {
        None
    } else {
        replay_failure_detail(&runtime, &reply).await
    };
    assert!(
        reply.is_successful_final_reply(),
        "replayed {} should finalize creation before firing; status {:?}, failure_category {:?}, text {:?}, failure_detail {:?}",
        case.fixture,
        reply.status,
        reply.failure_category,
        reply.text,
        failure_detail
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

    let now = Utc::now();
    // Make the persisted slot due without moving it behind the current cron
    // boundary. If this used `now - 120s`, a `*/30` trigger started just after
    // `:00` or `:30` would reschedule to that already-passed boundary after
    // the first fire, letting the poller submit the same test routine twice.
    let already_due_or_fired =
        trigger.is_due_at(now) || trigger.has_active_fire() || trigger.last_fired_slot.is_some();
    if !already_due_or_fired {
        trigger.next_run_at = now;
        repo.upsert_trigger(trigger)
            .await
            .expect("make replayed routine due");
    }

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

    // Read the fired run's persisted reply while the runtime is still up; the
    // assertion happens after the clearer fire-progress asserts below.
    let fired_reply = fired_routine_finalized_reply(&runtime, &tenant_id, trigger_id).await;

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
    // A settled-Ok record alone does not prove the user-observable outcome:
    // the fired run's own thread must hold the finalized assistant reply the
    // scripted fire produced.
    let fired_reply = fired_reply.unwrap_or_else(|| {
        panic!(
            "replayed {} fired routine should persist a finalized assistant reply in its run thread",
            case.fixture
        )
    });
    assert!(
        fired_reply.contains("qa fired routine ok"),
        "replayed {} fired routine reply must carry the scripted fire output; reply: {fired_reply:?}",
        case.fixture
    );
    gateway.assert_exhausted();
}

/// Reads the fired routine run's finalized assistant reply from its canonical
/// run thread — the same `TriggerRunRecord.thread_id` path the WebUI
/// Automations panel uses to open the run.
async fn fired_routine_finalized_reply(
    runtime: &RebornRuntime,
    tenant_id: &TenantId,
    trigger_id: ironclaw_triggers::TriggerId,
) -> Option<String> {
    let runs = runtime
        .trigger_repository()
        .list_trigger_run_history(tenant_id.clone(), trigger_id, 8)
        .await
        .ok()?;
    let thread_id = runs.iter().find_map(|run| run.thread_id.clone())?;
    let thread_service = runtime.standalone_thread_service_for_test()?;
    let history = thread_service
        .list_thread_history(ThreadHistoryRequest {
            scope: ThreadScope {
                tenant_id: tenant_id.clone(),
                agent_id: AgentId::new("qa-trace-agent").ok()?,
                project_id: None,
                owner_user_id: Some(UserId::new("qa-trace-owner").ok()?),
                mission_id: None,
            },
            thread_id,
        })
        .await
        .ok()?;
    history
        .messages
        .iter()
        .rev()
        .find(|message| {
            message.kind == MessageKind::Assistant && message.status == MessageStatus::Finalized
        })
        .and_then(|message| message.content.clone())
}

// The runtime-replay lane previously ran on `routine_health_ping` /
// `routine_hn_monitor`. Both recorded traces called the retired
// `builtin.outbound_delivery_target_set` and were deleted with that capability,
// so the lane moved to `routine_crm_inbox` — the surviving routine fixture,
// whose recorded tool choice (`extension_search` → `extension_install` →
// `trigger_create`) never touched the delivery-preference tool. Successor
// delivery-tool-choice fixtures are recorded in a later task.
#[tokio::test]
async fn replay_routine_crm_inbox_creates_real_trigger() {
    replay_routine_phrase(&ROUTINE_CRM_INBOX, "*/30 * * * *").await;
}

#[tokio::test]
async fn replay_routine_bare_send_me_webui_creates_real_trigger() {
    replay_routine_phrase(&ROUTINE_BARE_SEND_ME_WEBUI, "0 9 * * *").await;
}

#[tokio::test]
async fn replay_routine_multi_channel_delivery_creates_real_trigger() {
    replay_routine_phrase(&ROUTINE_MULTI_CHANNEL_DELIVERY, "0 9 * * *").await;
}

#[tokio::test]
async fn replay_routine_crm_inbox_fires_recorded_automation() {
    replay_routine_phrase_fires(&ROUTINE_CRM_INBOX, "*/30 * * * *").await;
}
