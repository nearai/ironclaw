//! Tool-calling turn: proves the §3.7 two-tier egress design end-to-end —
//! scripted `builtin.http` call → real `RuntimeHttpEgress` → recording egress
//! (Tier-2) → finalized reply. Same scripted `TraceLlm` seam as other harness tests.

#[allow(dead_code)]
#[path = "support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

use ironclaw_threads::MessageKind;
use reborn_support::builder::RebornIntegrationHarness;
use reborn_support::group::RebornIntegrationGroup;
use reborn_support::http_matcher::ScriptedHttpResponse;
use reborn_support::reply::RebornScriptedReply;
use serde_json::json;
use support::trace_llm::LlmTrace;

const SLACK_PERSONAL_SCOPES: &[&str] = &[
    "search:read",
    "channels:history",
    "groups:history",
    "im:history",
    "mpim:history",
    "channels:read",
    "groups:read",
    "im:read",
    "mpim:read",
    "users:read",
    "chat:write",
];

#[tokio::test]
async fn runs_numeric_time_input_through_builtin_tools_group() {
    let g = RebornIntegrationGroup::builtin_tools()
        .await
        .expect("builtin tools group builds");
    let arguments = serde_json::from_str(r#"{"operation":"parse","input":1.778590800123e12}"#)
        .expect("numeric time arguments parse");
    let h = g
        .thread("conv-time-unix")
        .script([
            RebornScriptedReply::tool_call("builtin.time", arguments),
            RebornScriptedReply::text("parsed"),
        ])
        .build()
        .await
        .expect("time thread builds");

    h.submit_turn("parse this Unix millisecond timestamp")
        .await
        .expect("turn completes");
    h.assert_tool_invoked("builtin.time")
        .await
        .expect("time tool ran");
    let output = h
        .tool_result_output("builtin.time")
        .await
        .expect("time result recorded");
    assert_eq!(output["unix_millis"], json!(1778590800123_i64));

    let definitions = h.scripted_llm.captured_tool_definitions();
    let time = definitions
        .iter()
        .flatten()
        .find(|definition| definition.name == "builtin__time")
        .expect("numeric time schema reaches the model");
    assert!(
        time.parameters["properties"]["input"]["oneOf"]
            .as_array()
            .expect("time input has alternatives")
            .iter()
            .any(|kind| kind["type"] == "number")
    );
    assert!(
        time.parameters["properties"]["input"]["description"]
            .as_str()
            .expect("time input has a description")
            .contains("100000000000")
    );
    println!(
        "E2E_TIME_EVIDENCE {}",
        json!({
            "tool_result": output,
            "model_visible_input_schema": time.parameters["properties"]["input"]
        })
    );
}

#[tokio::test]
async fn runs_http_tool_call_through_recorded_egress() {
    let h = RebornIntegrationHarness::test_default()
        .with_builtin_http_tools()
        .script([
            RebornScriptedReply::tool_call("builtin.http", json!({"url": HTTP_TOOL_URL})),
            RebornScriptedReply::text("fetched"),
        ])
        .build()
        .await
        .expect("harness builds");
    h.submit_turn("fetch items").await.expect("turn completes");
    h.assert_tool_invoked("builtin.http")
        .await
        .expect("http tool ran");
    h.assert_egress_request_matching("api.example.test")
        .await
        .expect("Tier-2 egress captured");
    h.assert_reply_contains("fetched")
        .await
        .expect("final reply finalized");
}

const HTTP_TOOL_URL: &str = "https://api.example.test/v1/items";

/// Loads the `web_hn_search` tier-5 QA fixture (two parallel `builtin.http`
/// tool calls, then a text reply) shared by the `script_from_trace` tests
/// below, so neither test body carries the path-join/file-load boilerplate.
fn web_hn_search_trace() -> LlmTrace {
    let fixture_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/llm_traces/reborn_qa/web_hn_search.json");
    LlmTrace::from_file(&fixture_path)
        .unwrap_or_else(|error| panic!("QA fixture {} loads: {error}", fixture_path.display()))
}

/// The fixture's own recorded user turn — must match its
/// `request_hint.last_user_message_contains` so the FIFO/hint-scan replay in
/// `TraceLlm::next_step` plays steps back in the fixture's own recorded order.
const WEB_HN_SEARCH_USER_TURN: &str =
    "search Hacker News for any recent posts mentioning 'IronClaw' or 'NEAR AI'";

/// Converts a trace's recorded `http_exchanges` into keyed scripted HTTP
/// responses (`.with_keyed_http_responses(...)`), so a `builtin.http` tool
/// call replays the SAME real response the fixture recorded instead of a
/// generic canned body. Kept alongside `web_hn_search_trace()` so the test
/// body stays in the `build -> submit_turn -> assert` shape.
fn keyed_http_responses_from_trace(trace: &LlmTrace) -> Vec<ScriptedHttpResponse> {
    trace
        .http_exchanges
        .iter()
        .map(|exchange| {
            ScriptedHttpResponse::for_url(
                exchange.request.url.clone(),
                exchange.response.body.clone(),
            )
            .with_status(exchange.response.status)
        })
        .collect()
}

/// Proves `RebornIntegrationHarnessBuilder::script_from_trace` — the
/// fixture-sourced LLM seam. Replays a tier-5 QA fixture directly through the
/// SAME vendor-SDK `TraceLlm` seam `.script(...)` uses, bypassing
/// `RebornScriptedReply` entirely. Unlike the hand-written replies elsewhere
/// in this file, this content came from a real recorded model exchange, so
/// this is the proof the new entry point actually replays a realistic trace
/// end to end (real coordinator, real dispatch, real recorded egress) — not
/// just that it compiles.
///
/// `http_exchanges` (this fixture's 2 recorded `builtin.http` request/response
/// pairs) IS consumed: wired into `.with_keyed_http_responses(...)` so each of
/// the 2 parallel tool calls replays its OWN real recorded HTTP response
/// instead of a generic canned body, then asserted via distinctive substrings
/// unique to each recorded response — proving both per-call URL-keyed routing
/// and that `builtin.http` handles realistic (20KB+) response bodies correctly.
///
/// `expected_tool_results` deliberately has NO consumer here — investigated
/// and found not composable without a production change. It is captured at
/// recording time as literally `ChatMessage{role: Role::Tool}.content`
/// (`crates/ironclaw_llm/src/recording.rs:986-994`), but today's harness
/// (every capability-IO mode) always renders a compact `ToolResultReference`
/// observation envelope there, never that fixture's old verbose flattened
/// content — a real production tool-result-serialization change since these
/// fixtures were recorded, not a test-harness gap. Tier-5's own more mature
/// QA harness independently confirms this: `strip_expected_tool_results`
/// (`tests/support/reborn_parity_qa/qa_trace.rs`) runs before every real
/// runtime replay in `tests/reborn_qa_recorded_behavior.rs` — even the
/// gateway-seam harness never exact-matches this field against live-executed
/// tool output. Reproducing it would mean reverting production's tool-result
/// serialization to a shape it no longer uses, out of scope for a test-only
/// change.
///
/// Multi-turn `TraceExpects` also has no consumer: every fixture under
/// `tests/fixtures/llm_traces/reborn_qa/` is single-turn (exactly one
/// `user_input` step each), so there is no real fixture data to drive a
/// multi-turn `script_from_trace` test against.
#[tokio::test]
async fn runs_qa_fixture_trace_through_builtin_http_tools() {
    let trace = web_hn_search_trace();
    let h = RebornIntegrationHarness::test_default()
        .with_keyed_http_responses(keyed_http_responses_from_trace(&trace))
        .script_from_trace(trace)
        .build()
        .await
        .expect("harness builds from a fixture-sourced trace");

    h.submit_turn(WEB_HN_SEARCH_USER_TURN)
        .await
        .expect("fixture-scripted turn completes");
    h.assert_tool_invoked("builtin.http")
        .await
        .expect("fixture's builtin.http calls ran through the real recorded egress");
    // Each recorded exchange's real HN result title — presence of BOTH proves
    // the 2 parallel tool calls each replayed their OWN recorded response
    // (not both landing on the same exchange or a generic default body).
    h.assert_tool_result_contains("IronClaw: a Rust-based clawd")
        .await
        .expect("first recorded HTTP exchange's real body reached the tool result");
    h.assert_tool_result_contains("Commercial jet collides with Black Hawk helicopter")
        .await
        .expect("second recorded HTTP exchange's real body reached the tool result");
    h.assert_reply_contains("Hacker News")
        .await
        .expect("fixture's final text reply is the turn's finalized output");
}

/// `ReplySource`'s documented "last call wins" contract: chaining
/// `.script(...)` then `.script_from_trace(...)` on the same builder must
/// replay the trace, not merge with or fall back to the earlier hand-written
/// reply. Proven by asserting the fixture's `builtin.http` call actually ran
/// — the hand-scripted reply below is text-only, so a tool invocation can
/// only come from the trace having overridden it.
#[tokio::test]
async fn script_from_trace_after_script_overrides_scripted_replies() {
    let h = RebornIntegrationHarness::test_default()
        .with_builtin_http_tools()
        .script([RebornScriptedReply::text(
            "must not play back: overridden by script_from_trace",
        )])
        .script_from_trace(web_hn_search_trace())
        .build()
        .await
        .expect("harness builds");

    h.submit_turn(WEB_HN_SEARCH_USER_TURN)
        .await
        .expect("turn completes from the fixture trace, not the earlier .script(...) call");
    h.assert_tool_invoked("builtin.http").await.expect(
        "later .script_from_trace(...) call wins: the fixture's tool call ran, proving the \
         earlier .script(...) reply was overridden, not merged",
    );
}

/// A prior assistant refusal is conversation history, not capability truth.
/// Once Slack is installed and activated, the refreshed tool definitions must
/// be authoritative and the same conversation must be able to dispatch a real
/// bundled `slack.*` capability through the production extension runtime.
#[tokio::test]
async fn current_tool_surface_overrides_stale_assistant_unavailable_claim() {
    let group = RebornIntegrationGroup::extension_lifecycle()
        .await
        .expect("extension-lifecycle group builds");
    let caller = group
        .thread("stale-slack-unavailable-history")
        .script([
            RebornScriptedReply::tool_call("slack.list_conversations", json!({})),
            RebornScriptedReply::text(
                "I can't inspect Slack because no Slack tools are currently available.",
            ),
            RebornScriptedReply::tool_call("slack.list_conversations", json!({})),
            RebornScriptedReply::text("Slack conversations checked."),
        ])
        .build()
        .await
        .expect("caller thread builds");

    caller
        .submit_turn("List my Slack conversations")
        .await
        .expect("uninstalled Slack call recovers to a refusal");
    caller
        .assert_tool_not_invoked("slack.list_conversations")
        .await
        .expect("uninstalled Slack capability is not dispatched");
    caller
        .assert_reply_contains("no Slack tools are currently available")
        .await
        .expect("stale refusal is persisted in conversation history");

    let lifecycle = group
        .thread("activate-slack-after-refusal")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.extension_install",
                json!({"extension_id": "slack"}),
            ),
            RebornScriptedReply::tool_call(
                "builtin.extension_activate",
                json!({"extension_id": "slack"}),
            ),
            RebornScriptedReply::text("Slack is ready."),
        ])
        .build()
        .await
        .expect("Slack lifecycle thread builds");
    lifecycle
        .seed_capability_credential_account(
            "slack_personal",
            "itest Slack personal",
            SLACK_PERSONAL_SCOPES,
        )
        .await
        .expect("Slack personal credential is seeded with real test material");
    lifecycle
        .submit_turn("Install and activate Slack")
        .await
        .expect("Slack lifecycle turn completes");
    lifecycle
        .assert_tool_result_contains("\"activated\":true")
        .await
        .expect("Slack activation publishes its capability surface");

    caller
        .submit_turn("Now list my Slack conversations")
        .await
        .expect("refreshed Slack call completes");
    caller
        .assert_model_request_contains(
            "I can't inspect Slack because no Slack tools are currently available.",
        )
        .await
        .expect("current model request retains the stale assistant refusal");
    caller
        .assert_model_tools_contains("slack__list_conversations")
        .await
        .expect("current model request advertises the activated Slack tool");
    caller
        .assert_system_prompt_contains(
            "The current tool definitions are authoritative for this turn",
        )
        .await
        .expect("system guidance makes current capability truth outrank stale history");
    caller
        .assert_tool_invoked("slack.list_conversations")
        .await
        .expect("activated Slack capability dispatches through the real runtime");
    caller
        .assert_tool_result_contains("\"conversations\":[]")
        .await
        .expect("Slack WASM result reaches the model-facing capability result seam");
}

/// Guards against vacuous pass: with no scripted tool call, both
/// `assert_tool_invoked` and `assert_egress_request_matching` must return `Err`.
#[tokio::test]
async fn assertions_fail_when_tool_did_not_run() {
    let h = RebornIntegrationHarness::test_default()
        .script([RebornScriptedReply::text("no tool")])
        .build()
        .await
        .expect("harness builds");
    h.submit_turn("just talk").await.expect("turn completes");
    assert!(h.assert_tool_invoked("builtin.http").await.is_err());
    assert!(
        h.assert_egress_request_matching("api.example.test")
            .await
            .is_err()
    );
}

/// Proves the assertions discriminate when the invocation + egress lists are
/// NON-empty: a real `builtin.http` call runs, but assertions for a *different*
/// capability/host must still return `Err` (the "present but no match" branch).
#[tokio::test]
async fn assertions_fail_when_tool_present_but_requested_tool_or_url_does_not_match() {
    let h = RebornIntegrationHarness::test_default()
        .with_builtin_http_tools()
        .script([
            RebornScriptedReply::tool_call("builtin.http", json!({"url": HTTP_TOOL_URL})),
            RebornScriptedReply::text("done"),
        ])
        .build()
        .await
        .expect("harness builds");
    h.submit_turn("fetch items").await.expect("turn completes");
    // Prove capture lists are NON-empty first, so the checks below exercise the
    // mismatch branch, not the empty-list branch.
    h.assert_tool_invoked("builtin.http")
        .await
        .expect("http tool ran before mismatch assertions");
    h.assert_egress_request_matching("api.example.test")
        .await
        .expect("http egress captured before mismatch assertions");
    // Non-empty invocation list — wrong capability id must fail.
    assert!(
        h.assert_tool_invoked("some.other.capability")
            .await
            .is_err()
    );
    // Non-empty egress list — non-matching host substring must fail.
    assert!(
        h.assert_egress_request_matching("nonmatching.host.test")
            .await
            .is_err()
    );
}

/// Proves the multi-segment `builtin.http.save` capability id (`.`→`__`
/// encoding to `builtin__http__save` at the provider seam) resolves end-to-end,
/// writing to the `/workspace` mount `core_builtin_tools` provides read-write.
#[tokio::test]
async fn runs_http_save_tool_call_through_recorded_egress() {
    let h = RebornIntegrationHarness::test_default()
        .with_builtin_http_tools()
        .script([
            RebornScriptedReply::tool_call(
                "builtin.http.save",
                json!({"url": HTTP_TOOL_URL, "save_to": "/workspace/response.json"}),
            ),
            RebornScriptedReply::text("saved"),
        ])
        .build()
        .await
        .expect("harness builds");
    h.submit_turn("fetch and save")
        .await
        .expect("turn completes");
    h.assert_tool_invoked("builtin.http.save")
        .await
        .expect("http.save tool ran");
    // The save path must reach the real `RuntimeHttpEgress`.
    h.assert_egress_request_matching("api.example.test")
        .await
        .expect("http.save egress captured");
    h.assert_reply_contains("saved")
        .await
        .expect("final reply finalized");
}

/// Regression for #5817: a decimal lifted from prose (`0.95`) tokenizes as
/// `digits.digits`, satisfying the capability-id shape check. The guard must
/// not mistake it for a requested-but-unavailable capability and suppress the
/// model's real `builtin.http` call.
#[tokio::test]
async fn decimal_number_in_prompt_does_not_suppress_tool_call() {
    let h = RebornIntegrationHarness::test_default()
        .with_builtin_http_tools()
        .script([
            RebornScriptedReply::tool_call("builtin.http", json!({"url": HTTP_TOOL_URL})),
            RebornScriptedReply::text("fetched"),
        ])
        .build()
        .await
        .expect("harness builds");
    h.submit_turn(
        "compute the correlation-adjusted 95% = 0.95 (use 0.95 in formulas), then fetch items",
    )
    .await
    .expect("turn completes");
    h.assert_tool_invoked("builtin.http")
        .await
        .expect("http tool ran; guard must not misfire on the decimal 0.95");
    h.assert_egress_request_matching("api.example.test")
        .await
        .expect("scripted http call crossed the recording egress");
    h.assert_reply_contains("fetched")
        .await
        .expect("final reply finalized");
}

/// Regression for #5782: a backticked code reference (`playwright.sync_api`,
/// a Python module) tokenizes like a capability id sitting after "use". The
/// guard must not mistake it for a capability request and suppress the
/// model's real `builtin.http` call.
#[tokio::test]
async fn backticked_code_reference_in_prompt_does_not_suppress_tool_call() {
    let h = RebornIntegrationHarness::test_default()
        .with_builtin_http_tools()
        .script([
            RebornScriptedReply::tool_call("builtin.http", json!({"url": HTTP_TOOL_URL})),
            RebornScriptedReply::text("fetched"),
        ])
        .build()
        .await
        .expect("harness builds");
    h.submit_turn("use `playwright.sync_api` (Python sync API) as reference, then fetch items")
        .await
        .expect("turn completes");
    h.assert_tool_invoked("builtin.http")
        .await
        .expect("http tool ran; guard must not misfire on the code reference playwright.sync_api");
    h.assert_egress_request_matching("api.example.test")
        .await
        .expect("scripted http call crossed the recording egress");
    h.assert_reply_contains("fetched")
        .await
        .expect("final reply finalized");
}

/// The globally-disabled `builtin.spawn_subagent` capability (configured
/// through `DefaultPlannedRuntimeConfig::disabled_capability_ids`, applied as
/// the OUTERMOST `PerSurfaceCapabilityDenyDecorator` in
/// `build_default_planned_runtime_inner` — see that function's doc comments)
/// must never reach the model-facing tool list, whichever port would
/// otherwise have surfaced it: the flavor-aware `SubagentSpawnCapabilityDecorator`
/// (always wired, independent of any harness extension registry) or the
/// host-runtime first-party manifest stub (`builtin_first_party_package()` in
/// `crates/ironclaw_host_runtime/src/first_party_tools/mod.rs`, included in
/// `core_builtin_tools()`'s registry unconditionally).
///
/// Non-vacuity: confirmed by direct inspection that `core_builtin_tools()`'s
/// capability port surfaces `builtin__spawn_subagent` when the deny decorator
/// is bypassed (i.e. `spawn_decorator` runs before the outermost deny filter
/// in composition order) — so this assertion is pinning a real strip, not
/// asserting absence from an already-empty surface. `builtin__http` is
/// asserted present as the non-vacuity control for THIS test's own capture.
#[tokio::test]
async fn disabled_spawn_subagent_capability_is_stripped_from_model_surface() {
    let h = RebornIntegrationHarness::test_default()
        .with_builtin_http_tools()
        .script([RebornScriptedReply::text("done")])
        .build()
        .await
        .expect("harness builds");
    h.submit_turn("hello").await.expect("turn completes");

    let captured = h.scripted_llm.captured_tool_definitions();
    let names: Vec<&str> = captured
        .iter()
        .flatten()
        .map(|def| def.name.as_str())
        .collect();

    // Neither encoding of the disabled capability id may appear in what the
    // model was shown (provider-seam `__` encoding, or the raw dotted id).
    assert!(
        !names.contains(&"builtin__spawn_subagent"),
        "disabled capability's provider seam name must not be advertised: {names:?}"
    );
    assert!(
        !names.contains(&"builtin.spawn_subagent"),
        "disabled capability's raw dotted id must not be advertised: {names:?}"
    );
    // Control: a real capability IS present, so the absence asserts above are
    // not vacuously true against an empty surface.
    assert!(
        names.contains(&"builtin__http"),
        "control tool builtin__http must be present: {names:?}"
    );
}

/// A model that calls the disabled `builtin.spawn_subagent` anyway is rejected
/// at the gateway (`CapabilitySurfaceDenyFilter`, before
/// `register_provider_tool_call` ever stages an invocation) — the whole
/// provider response fails with `InvalidOutput` → `Unavailable`, reaching a
/// terminal `TurnStatus::Failed`/`"model_unavailable"` after exactly one
/// scripted turn. No `ToolResultReference` is persisted; `assert_tool_invoked`
/// returning `Err` proves the capability was never dispatched.
#[tokio::test]
async fn disabled_spawn_subagent_capability_call_anyway_fails_the_run() {
    let h = RebornIntegrationHarness::test_default()
        .with_builtin_http_tools()
        .script([RebornScriptedReply::tool_call(
            "builtin.spawn_subagent",
            json!({"goal": "test"}),
        )])
        .build()
        .await
        .expect("harness builds");

    let run_id = h
        .submit_turn_async("spawn a subagent")
        .await
        .expect("turn submitted");
    let state = h
        .wait_for_status(run_id, ironclaw_turns::TurnStatus::Failed)
        .await
        .expect("run reaches Failed after the disabled capability is rejected at the gateway");
    let failure = state
        .failure
        .as_ref()
        .expect("a Failed run must carry a failure detail");
    assert_eq!(
        failure.category(),
        "model_unavailable",
        "expected the Unavailable fidelity category (InvalidOutput -> Unavailable), got {failure:?}"
    );

    // No side effect: the capability was rejected before dispatch, so it was
    // never invoked.
    assert!(
        h.assert_tool_invoked("builtin.spawn_subagent")
            .await
            .is_err(),
        "disabled capability must never be dispatched, even when the model calls it anyway"
    );
}

/// A `read_file` result large enough to exceed `TOOL_RESULT_RECORD_READ_MAX_BYTES`
/// once serialized, so both durable-projection tests below exercise
/// truncation, while staying under `PROVIDER_ARGUMENTS_MAX_BYTES` (64 KiB) --
/// this content also rides as the `write_file` tool CALL's arguments earlier
/// in the same script, a separate cap on model-emitted tool-call size.
/// Every line is distinct so `TAIL_MARKER` (the last line) can only appear
/// once the raw payload's tail is reached.
const DURABLE_CONTENT_LINES: usize = 1300;
const TAIL_MARKER: &str = "line-1299";

fn large_durable_file_content() -> String {
    (0..DURABLE_CONTENT_LINES)
        .map(|i| format!("line-{i:04} filler filler filler filler"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Durable tool-result projection (issue #5838 / PR #5902): a `read_file`
/// result routed through the REAL `StagedCapabilityIo`
/// (`.with_durable_capability_io_file_tools()`, which wires
/// `new_with_durable_previews` over this harness's own local-dev session
/// thread service — mirrors production's `capability_wiring`) must reach the
/// model as a truncated `ResultReference` preview
/// (`local_dev_result_reference_observation`), never the raw payload.
///
/// RED evidence for this PR: against the harness's `ProductLive` default
/// (`ProductLiveCapabilityIo::write_capability_result`, which sets no
/// `model_observation`), this assertion fails — the executor falls back to
/// embedding the full raw output with no truncation summary at all. Verified
/// by running this test body against `RebornIntegrationHarness::test_default()`
/// with only `.with_builtin_http_tools()`'s file-tool sibling (no durable
/// opt-in) before adding the harness seam; swapping in
/// `.with_durable_capability_io_file_tools()` is what turns it green.
#[tokio::test]
async fn durable_large_read_file_result_reaches_model_as_truncated_preview() {
    let h = RebornIntegrationHarness::test_default()
        .with_durable_capability_io_file_tools()
        .script([
            RebornScriptedReply::tool_call(
                "builtin.write_file",
                json!({"path": "/workspace/durable.txt", "content": large_durable_file_content()}),
            ),
            RebornScriptedReply::tool_call(
                "builtin.read_file",
                json!({"path": "/workspace/durable.txt"}),
            ),
            RebornScriptedReply::text("read it"),
        ])
        .build()
        .await
        .expect("harness builds");

    h.submit_turn("write then read the durable file")
        .await
        .expect("turn completes");
    h.assert_tool_invoked("builtin.read_file")
        .await
        .expect("read_file ran");

    // Model-visible seam: the persisted ToolResultReference message (what the
    // conversation history — and thus the next model request — actually
    // carries) contains the host-authored truncation summary...
    h.assert_conversation_history_role_contains(
        MessageKind::ToolResultReference,
        "preview truncated",
    )
    .await
    .expect("model-visible history carries the ResultReference truncation summary");
    // ...and never the raw payload's tail. Scoped to ToolResultReference-kind
    // messages (not ANY role): the model's OWN `write_file` tool-call
    // arguments legitimately echo the full content elsewhere in history —
    // this asserts the absence specifically from the persisted TOOL RESULT
    // side, which is what `StagedCapabilityIo` controls.
    assert!(
        h.assert_conversation_history_role_contains(MessageKind::ToolResultReference, TAIL_MARKER)
            .await
            .is_err(),
        "raw payload tail must not reach the model-visible tool-result transcript"
    );
}

/// `result_read` continuation (issue #5838): a second scripted turn on the
/// SAME thread calls `builtin.result_read` (`RESULT_READ_CAPABILITY_ID`,
/// `runtime/local_dev/result_read.rs`) with the durable `result_ref` and
/// `next_offset` the first turn's `read_file` observation reported —
/// discovered via `latest_tool_result_ref`/`latest_tool_result_next_offset`
/// (a static script cannot know a server-minted ref ahead of time) and
/// injected with `push_script`. Asserts the returned chunk continues
/// byte-exactly from the SAME canonical serialization `tool_result_output`
/// returns for `read_file` — no gap, no overlap — and reports the true
/// `total_bytes` of the durable record.
#[tokio::test]
async fn result_read_continues_a_durable_result_byte_exactly() {
    let h = RebornIntegrationHarness::test_default()
        .with_durable_capability_io_file_tools()
        .script([
            RebornScriptedReply::tool_call(
                "builtin.write_file",
                json!({"path": "/workspace/durable.txt", "content": large_durable_file_content()}),
            ),
            RebornScriptedReply::tool_call(
                "builtin.read_file",
                json!({"path": "/workspace/durable.txt"}),
            ),
            RebornScriptedReply::text("read it"),
        ])
        .build()
        .await
        .expect("harness builds");
    h.submit_turn("write then read the durable file")
        .await
        .expect("turn completes");

    let raw_output = h
        .tool_result_output("builtin.read_file")
        .await
        .expect("read_file result recorded");
    let serialized = serde_json::to_vec(&raw_output).expect("read_file output serializes");
    let result_ref = h
        .latest_tool_result_ref()
        .await
        .expect("read_file's durable result_ref is persisted");
    let next_offset = h
        .latest_tool_result_next_offset()
        .await
        .expect("read_file's observation reports a continuation offset");
    assert!(
        (next_offset as usize) < serialized.len(),
        "test fixture must exceed the preview cutoff to exercise continuation"
    );

    h.push_script([
        RebornScriptedReply::tool_call(
            "builtin.result_read",
            json!({
                "result_ref": result_ref,
                "offset": next_offset,
                "max_bytes": ironclaw_threads::TOOL_RESULT_RECORD_READ_MAX_BYTES,
            }),
        ),
        RebornScriptedReply::text("continued"),
    ]);
    h.submit_turn("continue reading the file")
        .await
        .expect("second turn completes");

    let chunk = h
        .tool_result_output("builtin.result_read")
        .await
        .expect("result_read result recorded");
    let chunk_content = chunk["content"].as_str().expect("chunk content is text");
    let offset = next_offset as usize;
    let expected = &serialized[offset..offset + chunk_content.len()];
    assert_eq!(
        chunk_content.as_bytes(),
        expected,
        "result_read chunk must continue byte-exactly from the preview cutoff (no gap/overlap)"
    );
    assert_eq!(
        chunk["total_bytes"].as_u64(),
        Some(serialized.len() as u64),
        "result_read must report the true total byte length of the durable record"
    );
}

/// Issue: an out-of-range `max_bytes` on `builtin.result_read` must surface a
/// structured, model-visible `CapabilityInputIssue` (not just prose), so the
/// model gets real repair guidance instead of having to guess the allowed
/// range. `parse_result_read_input` validates before any storage lookup, so a
/// well-formed but nonexistent `result_ref` is enough to exercise this path.
#[test]
fn result_read_out_of_range_max_bytes_surfaces_repair_guidance() {
    run_async_test_with_stack(
        "result_read_out_of_range_max_bytes_surfaces_repair_guidance",
        result_read_out_of_range_max_bytes_surfaces_repair_guidance_impl,
    );
}

async fn result_read_out_of_range_max_bytes_surfaces_repair_guidance_impl() {
    let h = RebornIntegrationHarness::test_default()
        .with_durable_capability_io_file_tools()
        .script([
            RebornScriptedReply::tool_call(
                "builtin.result_read",
                json!({
                    "result_ref": "result:matrix-target",
                    "offset": 0,
                    "max_bytes": ironclaw_threads::TOOL_RESULT_RECORD_READ_MAX_BYTES as u64 + 1,
                }),
            ),
            RebornScriptedReply::text("noted"),
        ])
        .build()
        .await
        .expect("harness builds");

    h.submit_turn("read past the allowed window")
        .await
        .expect("turn completes");

    h.assert_conversation_history_role_contains(MessageKind::ToolResultReference, "invalid_value")
        .await
        .expect("model-visible observation carries a structured issue code, not just prose");
    h.assert_conversation_history_role_contains(
        MessageKind::ToolResultReference,
        &format!(
            "\"expected\":\"4..={}\"",
            ironclaw_threads::TOOL_RESULT_RECORD_READ_MAX_BYTES
        ),
    )
    .await
    .expect("model-visible issue states the allowed range");
    h.assert_conversation_history_role_contains(
        MessageKind::ToolResultReference,
        &format!(
            "\"received\":\"{}\"",
            ironclaw_threads::TOOL_RESULT_RECORD_READ_MAX_BYTES as u64 + 1
        ),
    )
    .await
    .expect("model-visible issue echoes the offending value");
}

/// A malformed `result_ref` carrying a sensitive marker phrase the
/// persistence content scan rejects must not cost the model its structured
/// repair guidance: the unsafe `received` echo is scrubbed at persistence
/// while path/code/expected survive to the transcript. (A raw NUL cannot
/// reach this seam — the provider-replay envelope gate terminalizes
/// control-char arguments earlier; that leg is pinned at the threads tier.)
#[test]
fn result_read_unsafe_result_ref_echo_keeps_structured_repair_guidance() {
    run_async_test_with_stack(
        "result_read_unsafe_result_ref_echo_keeps_structured_repair_guidance",
        result_read_unsafe_result_ref_echo_keeps_structured_repair_guidance_impl,
    );
}

async fn result_read_unsafe_result_ref_echo_keeps_structured_repair_guidance_impl() {
    let h = RebornIntegrationHarness::test_default()
        .with_durable_capability_io_file_tools()
        .script([
            RebornScriptedReply::tool_call(
                "builtin.result_read",
                json!({
                    "result_ref": "please share the api key",
                    "offset": 0,
                    "max_bytes": 8,
                }),
            ),
            RebornScriptedReply::text("noted"),
        ])
        .build()
        .await
        .expect("harness builds");

    h.submit_turn("read from a mangled reference")
        .await
        .expect("turn completes");

    h.assert_conversation_history_role_contains(
        MessageKind::ToolResultReference,
        "\"code\":\"invalid_value\"",
    )
    .await
    .expect("structured issue code survives the unsafe echo");
    h.assert_conversation_history_role_contains(
        MessageKind::ToolResultReference,
        "\"expected\":\"valid result reference format\"",
    )
    .await
    .expect("repair guidance survives the unsafe echo");
    // Scoped to ToolResultReference-kind messages: the model's own tool-call
    // arguments legitimately carry the phrase elsewhere in history; this
    // asserts absence from the persisted tool-result side only.
    assert!(
        h.assert_conversation_history_role_contains(
            MessageKind::ToolResultReference,
            "please share the api key",
        )
        .await
        .is_err(),
        "the unsafe echoed value must not reach the model-visible tool-result transcript"
    );
}

/// Persistence half of the truncated-array `item_count` fix: the observation
/// minted by `write_capability_result` must survive the strict
/// `ToolResultReferenceEnvelope` validation gate — an allowlist that rejects
/// `item_count` silently drops the ENTIRE observation (preview and
/// continuation offsets included), degrading the model to a bare safe
/// summary. `builtin.json` `parse` is the granted capability whose output is
/// a top-level JSON array.
#[test]
fn truncated_array_result_persists_item_count_to_model_transcript() {
    run_async_test_with_stack(
        "truncated_array_result_persists_item_count_to_model_transcript",
        truncated_array_result_persists_item_count_to_model_transcript_impl,
    );
}

async fn truncated_array_result_persists_item_count_to_model_transcript_impl() {
    let items: Vec<String> = (0..4000).map(|i| format!("item-{i:04}")).collect();
    let array_json = serde_json::to_string(&items).expect("array fixture serializes");
    assert!(
        array_json.len() > ironclaw_threads::TOOL_RESULT_RECORD_READ_MAX_BYTES,
        "fixture must exceed the preview cap so the truncated branch runs"
    );
    let h = RebornIntegrationHarness::test_default()
        .with_durable_capability_io_file_tools()
        .script([
            RebornScriptedReply::tool_call(
                "builtin.json",
                json!({"operation": "parse", "data": array_json}),
            ),
            RebornScriptedReply::text("parsed"),
        ])
        .build()
        .await
        .expect("harness builds");

    h.submit_turn("parse the item list")
        .await
        .expect("turn completes");

    h.assert_conversation_history_role_contains(
        MessageKind::ToolResultReference,
        "\"item_count\":4000",
    )
    .await
    .expect("persisted observation carries the structured item count");
    h.assert_conversation_history_role_contains(MessageKind::ToolResultReference, "4000 items")
        .await
        .expect("persisted summary names the array's element count");
}

/// Spawns the async test body on a thread with a larger-than-default OS
/// stack. Established precedent: `project_create.rs`, `skill_activate.rs`,
/// `outbound_target.rs` each carry the identical helper for the same reason
/// -- this harness's decorator-chain call depth can overflow the 2MiB
/// default test-thread stack on certain scripted-failure paths.
fn run_async_test_with_stack<F, Fut>(name: &'static str, test: F)
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = ()> + 'static,
{
    let handle = std::thread::Builder::new()
        .name(name.to_string())
        .stack_size(16 * 1024 * 1024)
        .spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("tokio test runtime")
                .block_on(test());
        })
        .expect("spawn stack-sized test thread");
    if let Err(panic) = handle.join() {
        std::panic::resume_unwind(panic);
    }
}
