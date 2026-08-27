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
use reborn_support::reply::RebornScriptedReply;
use serde_json::json;

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
    "reactions:read",
    "reactions:write",
    "im:write",
];

fn github_webhook_normalization_call() -> RebornScriptedReply {
    RebornScriptedReply::tool_call(
        "github.handle_webhook",
        json!({
            "webhook": {
                "headers": {
                    "X-GitHub-Event": "pull_request",
                    "X-GitHub-Delivery": "delivery-capability-evidence"
                },
                "body_json": {
                    "action": "opened",
                    "repository": {
                        "full_name": "nearai/ironclaw",
                        "owner": {"login": "nearai"}
                    },
                    "pull_request": {
                        "number": 6573,
                        "state": "open",
                        "base": {"ref": "main"},
                        "head": {"ref": "codex/provider-evidence"}
                    },
                    "sender": {"login": "serrrfirat"}
                }
            }
        }),
    )
}

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
        .with_durable_capability_io_builtin_http_tools()
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
    h.assert_latest_result_json_round_trips("builtin.http")
        .await
        .expect("first-party output round-trips through durable result_read");
}

/// `github.handle_webhook` is local normalization rather than a provider API
/// call. Drive it through the real bundled GitHub WASM capability and assert
/// the emitted event plus the absence of network egress.
#[tokio::test]
async fn github_webhook_normalization_dispatches_through_bundled_wasm() {
    let h = RebornIntegrationHarness::test_default()
        .with_github_issue_tools()
        .script([
            github_webhook_normalization_call(),
            RebornScriptedReply::text("webhook normalized"),
        ])
        .build()
        .await
        .expect("harness builds");

    h.submit_turn("normalize this GitHub webhook")
        .await
        .expect("turn completes");
    h.assert_tool_invoked("github.handle_webhook")
        .await
        .expect("bundled GitHub WASM capability ran");
    h.assert_tool_result_contains(r#""event_type":"pr.opened""#)
        .await
        .expect("normalized event type reached the model-facing result");
    h.assert_tool_result_contains(r#""delivery_id":"delivery-capability-evidence""#)
        .await
        .expect("delivery identity survived normalization");
    h.assert_latest_result_json_round_trips("github.handle_webhook")
        .await
        .expect("WASM output round-trips through durable result_read");
    h.assert_network_egress_count(0)
        .await
        .expect("local webhook normalization made no provider request");
}

const HTTP_TOOL_URL: &str = "https://api.example.test/v1/items";

/// A prior assistant refusal is conversation history, not capability truth.
/// Once Slack is installed and ready, the refreshed tool definitions must
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
            RebornScriptedReply::text("Slack is ready."),
        ])
        .build()
        .await
        .expect("Slack lifecycle thread builds");
    lifecycle
        .seed_capability_credential_account("slack", "itest Slack personal", SLACK_PERSONAL_SCOPES)
        .await
        .expect("Slack personal credential is seeded with real test material");
    lifecycle
        .submit_turn("Install Slack")
        .await
        .expect("Slack lifecycle turn completes");
    lifecycle
        .assert_tool_result_contains("\"phase\":\"active\"")
        .await
        .expect("Slack install publishes its capability surface once ready");

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
        .expect("current model request advertises the active Slack tool");
    caller
        .assert_system_prompt_contains(
            "The current tool definitions are authoritative for this turn",
        )
        .await
        .expect("system guidance makes current capability truth outrank stale history");
    caller
        .assert_tool_invoked("slack.list_conversations")
        .await
        .expect("active Slack capability dispatches through the real runtime");
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
async fn runs_http_save_tool_call_through_real_egress_and_persists_body() {
    let h = RebornIntegrationHarness::test_default()
        .with_real_egress_pipeline()
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
    h.assert_workspace_file_contains("response.json", r#"{"ok":true}"#)
        .await
        .expect("http.save persisted the response body");
    h.assert_reply_contains("saved")
        .await
        .expect("final reply finalized");
}

fn transcript_json_fixture() -> Vec<u8> {
    let source = serde_json::json!({
        "schema": "ironclaw.thread.export.v1",
        "messages": [
            {
                "sequence": 23,
                "tool_call": {
                    "capability_id": "builtin.json",
                    "arguments": {
                        "data": "/workspace/market-data.json",
                        "operation": "parse"
                    }
                }
            },
            {
                "sequence": 64,
                "tool_call": {
                    "capability_id": "builtin.json",
                    "arguments": {
                        "data": "{\"end\":1.7405,\"start\":2.5528}",
                        "operation": "query",
                        "path": "$"
                    }
                }
            },
            {
                "sequence": 180,
                "tool_call": {
                    "capability_id": "builtin.json",
                    "arguments": {
                        "data": "{\"change\":((1.74-1.70)/1.70)*100}",
                        "operation": "stringify"
                    }
                }
            }
        ],
        "nodes": [
            null,
            null,
            {"data": (0..16).map(|index| vec![format!("value-{index}")]).collect::<Vec<_>>()}
        ],
        "redaction": {"applied": true},
        "synthetic_padding": "x".repeat(470_000)
    });
    let source_bytes = serde_json::to_vec(&source).expect("JSON fixture serializes");
    assert!(
        (470_000..480_000).contains(&source_bytes.len()),
        "sanitized fixture should preserve the supplied export's size class"
    );
    source_bytes
}

async fn run_scoped_file_json_queries() -> RebornIntegrationHarness {
    let h = RebornIntegrationHarness::test_default()
        .with_real_egress_pipeline()
        .with_real_egress_response_bodies([transcript_json_fixture()])
        .script([
            RebornScriptedReply::tool_call(
                "builtin.http.save",
                json!({"url": HTTP_TOOL_URL, "save_to": "/workspace/source.json"}),
            ),
            RebornScriptedReply::tool_call(
                "builtin.json",
                json!({
                    "operation": "parse",
                    "data": "/workspace/source.json"
                }),
            ),
            RebornScriptedReply::tool_call(
                "builtin.json",
                json!({
                    "operation": "query",
                    "file_path": "/workspace/source.json",
                    "path": "$.messages[1].tool_call.arguments.path"
                }),
            ),
            RebornScriptedReply::tool_call(
                "builtin.json",
                json!({
                    "operation": "query",
                    "file_path": "/workspace/source.json",
                    "path": "$.nodes[2].data[15][0]"
                }),
            ),
            RebornScriptedReply::text("queried"),
        ])
        .build()
        .await
        .expect("harness builds");
    h.submit_turn("save the response and query its JSON paths")
        .await
        .expect("turn completes");
    h
}

async fn run_inline_jsonpath_queries() -> RebornIntegrationHarness {
    let h = RebornIntegrationHarness::test_default()
        .with_real_egress_pipeline()
        .script([
            RebornScriptedReply::tool_call(
                "builtin.json",
                json!({
                    "operation": "query",
                    "data": "{\"end\":1.7404796872364274,\"start\":2.5528116140825894}",
                    "path": "$"
                }),
            ),
            RebornScriptedReply::tool_call(
                "builtin.json",
                json!({
                    "operation": "query",
                    "data": [["zero"], ["root-array-value"]],
                    "path": "$[1][0]"
                }),
            ),
            RebornScriptedReply::tool_call(
                "builtin.json",
                json!({
                    "operation": "query",
                    "data": {"items": [{"name": "jsonpath-root-value"}]},
                    "path": "$.items[0].name"
                }),
            ),
            RebornScriptedReply::text("queried"),
        ])
        .build()
        .await
        .expect("harness builds");
    h.submit_turn("query inline JSON with JSONPath-style roots")
        .await
        .expect("turn completes");
    h
}

async fn run_invalid_json_query() -> RebornIntegrationHarness {
    let h = RebornIntegrationHarness::test_default()
        .with_real_egress_pipeline()
        .script([
            RebornScriptedReply::tool_call(
                "builtin.json",
                json!({
                    "operation": "stringify",
                    "data": "{\"change\":((1.74-1.70)/1.70)*100}"
                }),
            ),
            RebornScriptedReply::text("queried"),
        ])
        .build()
        .await
        .expect("harness builds");
    h.submit_turn("stringify invalid JSON")
        .await
        .expect("turn completes");
    h
}

async fn run_json_collection_analysis() -> RebornIntegrationHarness {
    let h = RebornIntegrationHarness::test_default()
        .with_real_egress_pipeline()
        .with_real_egress_response_bodies([transcript_json_fixture()])
        .script([
            RebornScriptedReply::tool_call(
                "builtin.http.save",
                json!({"url": HTTP_TOOL_URL, "save_to": "/workspace/source.json"}),
            ),
            RebornScriptedReply::tool_call(
                "builtin.json",
                json!({
                    "operation": "last",
                    "file_path": "/workspace/source.json",
                    "path": "$.nodes[2].data"
                }),
            ),
            RebornScriptedReply::text("selected"),
            RebornScriptedReply::tool_call(
                "builtin.json",
                json!({
                    "operation": "aggregate",
                    "data": {"prices": [[1, 10.0], [2, 20.0], [3, 30.0]]},
                    "path": "prices",
                    "function": "average",
                    "value_index": 1
                }),
            ),
            RebornScriptedReply::text("analyzed"),
        ])
        .build()
        .await
        .expect("harness builds");
    h.submit_turn("select the last saved JSON row")
        .await
        .expect("file-backed collection turn completes");
    h.submit_turn("average the inline JSON price rows")
        .await
        .expect("aggregate turn completes");
    h
}

async fn run_json_schema_disclosure() -> RebornIntegrationHarness {
    let h = RebornIntegrationHarness::test_default()
        .with_real_egress_pipeline()
        .script([RebornScriptedReply::text("described")])
        .build()
        .await
        .expect("harness builds");
    h.submit_turn("describe the JSON capability")
        .await
        .expect("turn completes");
    h
}

fn assert_json_schema_disclosure(h: &RebornIntegrationHarness) {
    let definitions = h.scripted_llm.captured_tool_definitions();
    let definition = definitions
        .iter()
        .flatten()
        .find(|definition| definition.name == "builtin__json")
        .expect("JSON capability reaches the model");
    assert_eq!(
        definition.parameters["properties"]["operation"]["enum"],
        json!([
            "parse",
            "stringify",
            "query",
            "validate",
            "length",
            "last",
            "slice",
            "aggregate"
        ])
    );
    assert!(
        definition.parameters["properties"]["file_path"]["description"]
            .as_str()
            .is_some_and(|description| description.contains("/workspace"))
    );
    assert!(
        definition.parameters["properties"]["path"]["description"]
            .as_str()
            .is_some_and(|description| description.contains("$.nodes")),
        "model-visible schema must advertise optional root markers"
    );
    assert!(
        definition.parameters["properties"]["path"]["description"]
            .as_str()
            .is_some_and(|description| description.contains("not supported")),
        "model-visible schema must distinguish traversal paths from full JSONPath"
    );
    assert!(
        definition.parameters["oneOf"]
            .as_array()
            .is_some_and(|branches| branches.iter().any(|branch| {
                branch["required"] == json!(["operation", "file_path", "path"])
                    && branch["not"]["required"] == json!(["data"])
            })),
        "model-visible schema must disclose the file-backed query alternative unambiguously"
    );
}

/// A realistic-size response saved under `/workspace` remains queryable without
/// shell or inline copying, including repeated adjacent array indices.
#[tokio::test]
async fn json_queries_scoped_file_and_adjacent_array_indices() {
    let h = run_scoped_file_json_queries().await;
    h.assert_tool_invoked("builtin.http.save")
        .await
        .expect("http.save tool ran");
    h.assert_workspace_file_contains("source.json", "value-15")
        .await
        .expect("http.save persisted the JSON source");
    h.assert_tool_result_contains("\"$\"")
        .await
        .expect("the transcript-derived file query returns its nested root marker");
    h.assert_tool_result_contains("value-15")
        .await
        .expect("file-backed query traversed repeated adjacent indices");
}

/// `$`-rooted inline queries support object roots, array roots, and traversal.
#[tokio::test]
async fn json_queries_inline_jsonpath_roots() {
    let h = run_inline_jsonpath_queries().await;
    h.assert_tool_result_contains("1.7404796872364274")
        .await
        .expect("the transcript-derived inline root query returns the full object");
    h.assert_tool_result_contains("root-array-value")
        .await
        .expect("inline compatibility includes JSONPath-style root-array queries");
    h.assert_tool_result_contains("jsonpath-root-value")
        .await
        .expect("JSONPath-style object roots resolve through the real capability path");
}

/// Invalid JSON produces actionable, model-visible correction guidance.
#[tokio::test]
async fn invalid_json_is_recoverable() {
    let h = run_invalid_json_query().await;
    h.assert_tool_error_summary_contains("JSON input is not valid JSON")
        .await
        .expect("invalid JSON is explained to the model with an actionable safe summary");
}

/// Bounded collection operations work for both scoped files and inline rows.
#[tokio::test]
async fn json_runs_bounded_collection_operations() {
    // This debug workflow future exceeds libtest's default 2 MiB thread stack;
    // box it before awaiting. CI sets no `RUST_MIN_STACK`, and assertions remain
    // unchanged.
    let h = Box::pin(run_json_collection_analysis()).await;
    h.assert_tool_result_contains("value-15")
        .await
        .expect("last selects the final item from the scoped JSON array");
    h.assert_tool_result_contains(r#""function":"average""#)
        .await
        .expect("aggregate reports the selected numeric operation");
    h.assert_tool_result_contains(r#""value":20.0"#)
        .await
        .expect("aggregate selects and averages the numeric row values");
}

/// The model-visible schema advertises operations and file-backed JSONPath queries.
#[tokio::test]
async fn json_schema_discloses_file_queries() {
    let h = run_json_schema_disclosure().await;
    assert_json_schema_disclosure(&h);
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
/// the OUTERMOST `CapabilitySurfacePolicyFilter` in
/// `build_default_planned_runtime_inner` — see that function's doc comments)
/// must never reach the model-facing tool list, whichever port would
/// otherwise have surfaced it: the flavor-aware `SubagentSpawnCapabilityDecorator`
/// (always wired, independent of any harness extension registry) or the
/// host-runtime first-party manifest stub (`builtin_first_party_package()` in
/// `crates/kernel/ironclaw_host_runtime/src/first_party_tools/mod.rs`, included in
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
/// at the gateway (`CapabilitySurfacePolicyFilter`, before
/// `register_provider_tool_call` ever stages an invocation). The gateway must
/// return precise batch-rejection feedback to the model, let it repair the
/// response on the next call, and complete without ever dispatching or
/// reporting the rejected call as successful.
#[tokio::test]
async fn disabled_spawn_subagent_capability_call_recovers_without_dispatch() {
    let h = RebornIntegrationHarness::test_default()
        .with_builtin_http_tools()
        .script([
            RebornScriptedReply::tool_call("builtin.spawn_subagent", json!({"goal": "test"})),
            RebornScriptedReply::text(
                "I cannot use that capability, so I will continue without it.",
            ),
        ])
        .build()
        .await
        .expect("harness builds");

    h.submit_turn("spawn a subagent")
        .await
        .expect("run recovers from the disabled capability call");
    h.assert_reply_contains("continue without it")
        .await
        .expect("repaired reply is finalized");
    h.assert_model_message_content_in_order(&[
        "Tool call batch rejected by host:",
        "model returned a tool call outside the advertised capability surface",
        "None of this response's tool calls were executed.",
        "Retry with an available capability",
    ])
    .await
    .expect("the gateway tells the model precisely why its tool-call batch was rejected");

    h.assert_tool_not_invoked("builtin.spawn_subagent")
        .await
        .expect("the rejected capability must never be dispatched");
    h.assert_capability_result_count("builtin.spawn_subagent", 0)
        .await
        .expect("the rejected call must not produce a successful capability result");
}

/// Regression for the host-policy propagation seam: capability metadata
/// lookup must resolve against the same already-narrowed host-visible surface
/// that disclosure and the outer gate use. A denied target is therefore
/// indistinguishable from an unknown target and returns a normal tool failure;
/// it must not escape registration as terminal invalid model output.
#[tokio::test]
async fn capability_info_for_disabled_spawn_is_opaque_and_model_recoverable() {
    let h = RebornIntegrationHarness::test_default()
        .with_builtin_http_tools()
        .script([
            RebornScriptedReply::tool_call(
                "capability_info",
                json!({"name": "builtin__spawn_subagent", "detail": "schema"}),
            ),
            RebornScriptedReply::text("continued without the disabled capability"),
        ])
        .build()
        .await
        .expect("harness builds");

    h.submit_turn("inspect the disabled spawn capability")
        .await
        .expect("run recovers from opaque capability_info failure");
    h.assert_tool_error_summary_contains("capability_info target is not on the visible surface")
        .await
        .expect("denied target is reported through the model-visible tool-result path");
    h.assert_reply_contains("continued without the disabled capability")
        .await
        .expect("the model can continue after the tool failure");
    h.assert_tool_not_invoked("builtin.spawn_subagent")
        .await
        .expect("metadata lookup never dispatches the denied target");
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

fn durable_file_content_with_suppressed_continuation_preview() -> String {
    (0..1500)
        .map(|i| {
            if i == 800 {
                "line-0800 secret filler filler filler".to_string()
            } else {
                format!("line-{i:04} filler filler filler filler")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn large_nested_json_capability_result() -> serde_json::Value {
    let items = (0..4_000)
        .map(|index| {
            json!({
                "id": index,
                "label": format!("nested-item-{index}"),
                "payload": "x".repeat(8),
            })
        })
        .collect::<Vec<_>>();
    let credential_rows = (0..100)
        .map(|row| {
            serde_json::Value::Object(
                (0..24)
                    .map(|field| (format!("password-{row}-{field}"), json!(0)))
                    .collect(),
            )
        })
        .collect::<Vec<_>>();
    let result = json!({
        "metadata": {"marker": "large-nested-json"},
        "payload": {
            "credential_rows": credential_rows.clone(),
            "items": items,
            "rows": credential_rows,
            "secret": "sensitive-value".repeat(200),
            "tail": {"marker": "tail-survives-selection"},
        },
    });
    let serialized = serde_json::to_vec(&result).expect("nested JSON fixture serializes");
    assert!(
        serialized.len() > 100 * 1024,
        "nested JSON fixture must exceed 100 KiB"
    );
    result
}

/// Durable tool-result projection (issue #5838 / PR #5902): a `read_file`
/// result routed through the REAL `StagedCapabilityIo`
/// (`.with_durable_capability_io_file_tools()`, which wires
/// `new_with_durable_previews` over this harness's own local-dev session
/// thread service — mirrors production's `capability_wiring`) must reach the
/// model as a bounded `ResultReference` preview
/// (`standalone_result_reference_observation`), never the raw payload.
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
async fn durable_large_read_file_result_reaches_model_as_bounded_json_view() {
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
    // carries) contains the host-authored structured-view summary...
    h.assert_conversation_history_role_contains(
        MessageKind::ToolResultReference,
        "bounded JSON view",
    )
    .await
    .expect("model-visible history carries the bounded JSON-view summary");
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

/// Secret labels in structured file output must redact their paired values
/// before the preview vocabulary hides the label itself. Otherwise the next
/// model request sees `"[redacted]": "opaque-value"` and no later scanner can
/// recover that the surviving value was credential material.
#[tokio::test]
async fn durable_read_file_result_redacts_structured_credential_value() {
    let canary = "never-before-uploaded-canary-integration";
    let marker = "structured-credential-redaction-marker";
    let content = serde_json::json!({
        "marker": marker,
        "password": canary,
        "secretary": "Treasury contact",
    })
    .to_string();
    let h = RebornIntegrationHarness::test_default()
        .with_durable_capability_io_file_tools()
        .script([
            RebornScriptedReply::tool_call(
                "builtin.read_file",
                json!({"path": "/workspace/structured-secret.json"}),
            ),
            RebornScriptedReply::text("read it"),
        ])
        .build()
        .await
        .expect("harness builds");

    let workspace_path = h
        .capability_recorder
        .workspace_file_path("structured-secret.json")
        .expect("durable file-tools harness exposes its workspace");
    std::fs::write(&workspace_path, content)
        .expect("structured fixture is seeded outside the model");

    h.submit_turn("read the structured file")
        .await
        .expect("turn completes");
    h.assert_tool_invoked("builtin.read_file")
        .await
        .expect("read_file ran");
    h.assert_conversation_history_role_contains(MessageKind::ToolResultReference, marker)
        .await
        .expect("benign structured content survives in the model-visible result");
    h.assert_conversation_history_role_contains(
        MessageKind::ToolResultReference,
        "Treasury contact",
    )
    .await
    .expect("credential substrings in benign keys do not cause false-positive redaction");
    assert!(
        h.assert_conversation_history_role_contains(MessageKind::ToolResultReference, canary)
            .await
            .is_err(),
        "the credential value must not survive in the model-visible tool result"
    );
    assert!(
        h.assert_model_request_contains(canary).await.is_err(),
        "the credential value must never reach a model request"
    );
}

/// `result_read` continuation (issue #5838): two subsequent scripted turns on
/// the SAME thread page the durable `read_file` result in legacy byte mode.
/// Page two is invoked
/// exclusively with the `result_ref` and `next_offset` surfaced by page one,
/// proving that model-visible continuation metadata retains the original
/// pageable identity instead of exposing the fresh `InlineOnly` write ref.
/// Both chunks continue byte-exactly through the SAME canonical serialization
/// `tool_result_output` returns for `read_file` — no gap, no overlap — and
/// report the durable record's true `total_bytes`. Page one's chunk contains a
/// credential marker, so its inline preview is suppressed; the continuation
/// identity and offset must survive independently of preview content. The
/// first explicit byte read starts at the historical 24 KiB boundary because
/// the automatic first look now carries continuation inside its JSON page.
#[test]
fn result_read_continues_a_durable_result_byte_exactly() {
    run_async_test_with_stack(
        "result_read_continues_a_durable_result_byte_exactly",
        result_read_continues_a_durable_result_byte_exactly_impl,
    );
}

async fn result_read_continues_a_durable_result_byte_exactly_impl() {
    let h = RebornIntegrationHarness::test_default()
        .with_durable_capability_io_file_tools()
        .script([
            RebornScriptedReply::tool_call(
                "builtin.write_file",
                json!({
                    "path": "/workspace/durable.txt",
                    "content": durable_file_content_with_suppressed_continuation_preview()
                }),
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
    let next_offset = ironclaw_threads::TOOL_RESULT_RECORD_READ_MAX_BYTES as u64;
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
    assert!(
        chunk_content.contains("secret"),
        "fixture must put the rejected marker inside the requested chunk"
    );

    let surfaced_result_ref = h
        .latest_tool_result_ref()
        .await
        .expect("page one surfaces a continuation result_ref");
    let page_two_offset = h
        .latest_tool_result_next_offset()
        .await
        .expect("page one surfaces a continuation offset");
    assert_eq!(
        surfaced_result_ref, result_ref,
        "page one must surface the original durable result ref, not its inline-only write ref"
    );

    h.push_script([
        RebornScriptedReply::tool_call(
            "builtin.result_read",
            json!({
                "result_ref": surfaced_result_ref,
                "offset": page_two_offset,
                "max_bytes": ironclaw_threads::TOOL_RESULT_RECORD_READ_MAX_BYTES,
            }),
        ),
        RebornScriptedReply::text("continued again"),
    ]);
    h.submit_turn("continue reading the next page")
        .await
        .expect("third turn completes");

    let page_two = h
        .tool_result_output("builtin.result_read")
        .await
        .expect("second result_read result recorded");
    let page_two_content = page_two["content"]
        .as_str()
        .expect("second chunk content is text");
    let page_two_start = page_two_offset as usize;
    let page_two_expected = &serialized[page_two_start..page_two_start + page_two_content.len()];
    assert_eq!(
        page_two_content.as_bytes(),
        page_two_expected,
        "second result_read chunk must continue from page one's surfaced next_offset"
    );
    assert_eq!(
        page_two["total_bytes"].as_u64(),
        Some(serialized.len() as u64),
        "every page must report the same durable total byte length"
    );

    let envelopes = h
        .persisted_tool_result_envelopes()
        .await
        .expect("tool-result envelopes persist");
    let result_read_envelopes = envelopes
        .iter()
        .filter(|envelope| {
            envelope.result_ref == result_ref
                && envelope
                    .model_observation
                    .as_ref()
                    .and_then(|observation| observation["summary"].as_str())
                    == Some("Requested tool-result chunk returned.")
        })
        .collect::<Vec<_>>();
    assert_eq!(
        result_read_envelopes.len(),
        2,
        "exactly the two ordered result_read envelopes must be selected"
    );
    let page_one_envelope = result_read_envelopes[0];
    let page_two_envelope = result_read_envelopes[1];
    let page_one_observation = page_one_envelope
        .model_observation
        .as_ref()
        .expect("metadata-only first-page observation survives");
    let page_one_detail = &page_one_observation["detail"];
    assert_eq!(
        page_one_envelope.result_ref, result_ref,
        "first-page replay must retain the original pageable result ref"
    );
    assert_eq!(
        page_one_detail["result_ref"].as_str(),
        Some(result_ref.as_str()),
        "continuation authority remains the durable source ref"
    );
    assert!(
        page_one_detail.get("preview").is_none(),
        "credential-bearing preview remains suppressed"
    );
    assert_eq!(
        page_one_detail["total_bytes"].as_u64(),
        Some(serialized.len() as u64)
    );
    assert_eq!(
        page_one_detail["next_offset"].as_u64(),
        Some(page_two_offset),
        "first-page replay must retain the offset fed into page two"
    );

    let page_two_observation = page_two_envelope
        .model_observation
        .as_ref()
        .expect("second-page observation survives");
    let page_two_detail = &page_two_observation["detail"];
    assert_eq!(
        page_two_envelope.result_ref, result_ref,
        "second-page replay must retain the original pageable result ref"
    );
    assert_eq!(
        page_two_detail["result_ref"].as_str(),
        Some(result_ref.as_str())
    );
    assert_eq!(
        page_two_detail["total_bytes"].as_u64(),
        Some(serialized.len() as u64)
    );
    assert_eq!(
        page_two_detail["next_offset"].as_u64(),
        page_two["next_offset"].as_u64(),
        "second-page replay metadata must match the second page output"
    );
}

struct LargeNestedResultFixture {
    harness: RebornIntegrationHarness,
    serialized: Vec<u8>,
    result_ref: String,
}

async fn large_nested_result_fixture() -> LargeNestedResultFixture {
    let result = large_nested_json_capability_result();
    let serialized = serde_json::to_vec(&result).expect("nested result serializes");
    let h = RebornIntegrationHarness::test_default()
        .with_durable_capability_io_file_tools()
        .script([
            RebornScriptedReply::tool_call(
                "builtin.json",
                json!({
                    "operation": "query",
                    "file_path": "/workspace/large-nested.json",
                    "path": "$",
                }),
            ),
            RebornScriptedReply::text("inspected"),
        ])
        .build()
        .await
        .expect("harness builds");
    let workspace_path = h
        .capability_recorder
        .workspace_file_path("large-nested.json")
        .expect("durable file-tools harness exposes its workspace");
    std::fs::write(
        &workspace_path,
        serde_json::to_vec(&result).expect("nested fixture serializes"),
    )
    .expect("nested JSON fixture is seeded outside the model");

    h.submit_turn("inspect the large nested result")
        .await
        .expect("first turn completes");
    let result_ref = h
        .latest_tool_result_ref()
        .await
        .expect("large result has a durable result reference");

    LargeNestedResultFixture {
        harness: h,
        serialized,
        result_ref,
    }
}

async fn read_large_nested_result(
    h: &RebornIntegrationHarness,
    result_ref: &str,
    offset: u64,
    max_bytes: usize,
    json_pointer: Option<&str>,
    limit: Option<usize>,
) -> serde_json::Value {
    let mut arguments = json!({
        "result_ref": result_ref,
        "offset": offset,
        "max_bytes": max_bytes,
    });
    if let Some(json_pointer) = json_pointer {
        arguments["json_pointer"] = json!(json_pointer);
    }
    if let Some(limit) = limit {
        arguments["limit"] = json!(limit);
    }
    h.push_script([
        RebornScriptedReply::tool_call("builtin.result_read", arguments),
        RebornScriptedReply::text("result read complete"),
    ]);
    h.submit_turn("read the requested result view")
        .await
        .expect("result_read turn completes");
    h.tool_result_output("builtin.result_read")
        .await
        .expect("result_read output is recorded")
}

/// A large structured capability result must get a bounded, parseable first
/// look before the model asks for a more specific JSON view.
#[test]
fn result_read_large_nested_result_first_look_is_bounded_and_parseable() {
    run_async_test_with_stack(
        "result_read_large_nested_result_first_look_is_bounded_and_parseable",
        result_read_large_nested_result_first_look_is_bounded_and_parseable_impl,
    );
}

async fn result_read_large_nested_result_first_look_is_bounded_and_parseable_impl() {
    let fixture = large_nested_result_fixture().await;
    let envelopes = fixture
        .harness
        .persisted_tool_result_envelopes()
        .await
        .expect("tool-result envelopes persist");
    let observation = envelopes
        .last()
        .and_then(|envelope| envelope.model_observation.as_ref())
        .expect("large result observation survives");
    let detail = &observation["detail"];
    assert_eq!(
        detail["total_bytes"].as_u64(),
        Some(fixture.serialized.len() as u64)
    );
    let preview = detail["preview"]
        .as_str()
        .expect("first look has a preview");
    assert!(
        preview.len() <= 3 * 1024,
        "automatic first look must stay within its independent 3 KiB budget"
    );
    assert!(
        serde_json::to_vec(observation)
            .expect("complete observation serializes")
            .len()
            <= 4 * 1024,
        "complete automatic observation must stay within 4 KiB"
    );
    let preview_value = serde_json::from_str::<serde_json::Value>(preview)
        .expect("first look must be a complete JSON value, not a blind prefix");
    assert_eq!(
        preview_value["total_bytes"],
        fixture.serialized.len() as u64
    );
    assert!(
        preview_value["total_bytes"]
            .as_u64()
            .expect("total bytes present")
            > 100 * 1024
    );
    assert_eq!(preview_value["json_pointer"], "");
    assert_eq!(preview_value["node_type"], "object");
    assert_eq!(
        preview_value["content"]["metadata"]["marker"],
        "large-nested-json"
    );
    assert_eq!(preview_value["omitted"][0]["json_pointer"], "/payload");
}

/// A nested JSON pointer selects an object or a scalar without returning the
/// surrounding large result.
#[test]
fn result_read_selects_nested_json_node_and_scalar() {
    run_async_test_with_stack(
        "result_read_selects_nested_json_node_and_scalar",
        result_read_selects_nested_json_node_and_scalar_impl,
    );
}

async fn result_read_selects_nested_json_node_and_scalar_impl() {
    let fixture = large_nested_result_fixture().await;

    let selected = read_large_nested_result(
        &fixture.harness,
        &fixture.result_ref,
        0,
        ironclaw_threads::TOOL_RESULT_RECORD_READ_MAX_BYTES,
        Some("/payload/items/2345"),
        None,
    )
    .await;
    assert_eq!(
        selected["result_ref"].as_str(),
        Some(fixture.result_ref.as_str())
    );
    assert_eq!(
        selected["json_pointer"].as_str(),
        Some("/payload/items/2345")
    );
    assert_eq!(
        selected["total_bytes"].as_u64(),
        Some(fixture.serialized.len() as u64)
    );
    let selected_value = &selected["content"];
    assert_eq!(selected_value["id"], 2345);
    assert_eq!(selected_value["label"], "nested-item-2345");
    assert!(selected["next_offset"].is_null());

    let scalar_page = read_large_nested_result(
        &fixture.harness,
        &fixture.result_ref,
        0,
        ironclaw_host_api::model_result_preview::MODEL_RESULT_PREVIEW_MAX_BYTES,
        Some("/payload/items/2345/id"),
        None,
    )
    .await;
    assert_eq!(scalar_page["node_type"], "number");
    assert_eq!(scalar_page["content"], 2345);
    assert!(scalar_page["next_offset"].is_null());
}

/// Collection pointers page by item index while preserving the selected JSON
/// node across continuation reads.
#[test]
fn result_read_pages_nested_json_collection() {
    run_async_test_with_stack(
        "result_read_pages_nested_json_collection",
        result_read_pages_nested_json_collection_impl,
    );
}

async fn result_read_pages_nested_json_collection_impl() {
    let fixture = large_nested_result_fixture().await;
    let array_page = read_large_nested_result(
        &fixture.harness,
        &fixture.result_ref,
        2345,
        ironclaw_host_api::model_result_preview::MODEL_RESULT_PREVIEW_MAX_BYTES,
        Some("/payload/items"),
        Some(2),
    )
    .await;
    assert_eq!(array_page["content"][0]["id"], 2345);
    assert_eq!(array_page["content"][1]["id"], 2346);
    let array_next_offset = array_page["next_offset"]
        .as_u64()
        .expect("limited collection page has a continuation");

    let next_array_page = read_large_nested_result(
        &fixture.harness,
        &fixture.result_ref,
        array_next_offset,
        ironclaw_host_api::model_result_preview::MODEL_RESULT_PREVIEW_MAX_BYTES,
        Some("/payload/items"),
        Some(2),
    )
    .await;
    assert_eq!(next_array_page["content"][0]["id"], array_next_offset);
}

/// Credential-labeled selections remain usable while provider credential
/// values are masked and the caller's post-redaction byte budget is honored.
#[test]
fn result_read_redacts_credential_json_within_requested_budget() {
    run_async_test_with_stack(
        "result_read_redacts_credential_json_within_requested_budget",
        result_read_redacts_credential_json_within_requested_budget_impl,
    );
}

async fn result_read_redacts_credential_json_within_requested_budget_impl() {
    let fixture = large_nested_result_fixture().await;
    let h = &fixture.harness;
    let result_ref = &fixture.result_ref;

    h.push_script([
        RebornScriptedReply::tool_call(
            "builtin.result_read",
            json!({
                "result_ref": result_ref,
                "offset": 0,
                "max_bytes": ironclaw_host_api::model_result_preview::MODEL_RESULT_PREVIEW_MAX_BYTES,
                "json_pointer": "/payload/secret",
            }),
        ),
        RebornScriptedReply::text("sensitive field handled"),
    ]);
    h.submit_turn("select the credential-labeled field")
        .await
        .expect("credential-labeled JSON selection remains readable");
    let secret_envelopes = h
        .persisted_tool_result_envelopes()
        .await
        .expect("credential-labeled result observation persists");
    let secret_preview = secret_envelopes
        .last()
        .and_then(|envelope| envelope.model_observation.as_ref())
        .and_then(|observation| observation["detail"]["preview"].as_str())
        .expect("credential-labeled result keeps a safe preview");
    let secret_page: serde_json::Value =
        serde_json::from_str(secret_preview).expect("credential-labeled preview remains JSON");
    assert_eq!(secret_page["json_pointer"], "/payload/secret");
    assert_eq!(secret_page["content"], "[redacted]");
    assert!(secret_page["next"].is_null());

    h.push_script([
        RebornScriptedReply::tool_call(
            "builtin.result_read",
            json!({
                "result_ref": result_ref,
                "offset": 0,
                "max_bytes": ironclaw_host_api::model_result_preview::MODEL_RESULT_PREVIEW_MAX_BYTES,
                "json_pointer": "/payload/credential_rows",
                "limit": 100,
            }),
        ),
        RebornScriptedReply::text("credential rows inspected"),
    ]);
    h.submit_turn("inspect the credential-shaped rows")
        .await
        .expect("redaction growth remains a recoverable result read");
    let credential_envelopes = h
        .persisted_tool_result_envelopes()
        .await
        .expect("credential-shaped result observation persists");
    let credential_preview = credential_envelopes
        .last()
        .and_then(|envelope| envelope.model_observation.as_ref())
        .and_then(|observation| observation["detail"]["preview"].as_str())
        .expect("credential-shaped result keeps a safe preview");
    let credential_page: serde_json::Value =
        serde_json::from_str(credential_preview).expect("credential-shaped preview remains JSON");
    assert_eq!(credential_page["json_pointer"], "/payload/credential_rows");
    assert!(credential_page["next"].is_null());
    h.assert_conversation_history_role_contains(MessageKind::ToolResultReference, "[redacted]")
        .await
        .expect("credential-shaped values are redacted without terminalizing the turn");

    let redacted_page_budget = 1024usize;
    h.push_script([
        RebornScriptedReply::tool_call(
            "builtin.result_read",
            json!({
                "result_ref": result_ref,
                "offset": 0,
                "max_bytes": redacted_page_budget,
                "json_pointer": "/payload/rows",
                "limit": 100,
            }),
        ),
        RebornScriptedReply::text("bounded redacted rows inspected"),
    ]);
    h.submit_turn("inspect redacted rows within the requested byte budget")
        .await
        .expect("redaction growth is retried within the requested budget");
    let bounded_redacted_page = h
        .tool_result_output("builtin.result_read")
        .await
        .expect("bounded redacted JSON page is recorded");
    assert!(
        serde_json::to_vec(&bounded_redacted_page)
            .expect("bounded redacted page serializes")
            .len()
            <= redacted_page_budget,
        "post-redaction output must honor the caller's max_bytes"
    );
    assert!(
        bounded_redacted_page.to_string().contains("[redacted]"),
        "the bounded page must expose redacted provider content"
    );
}

/// Invalid pointer, offset, and collection-limit combinations remain typed,
/// model-correctable tool outcomes rather than terminal host failures.
#[test]
fn invalid_json_result_selections_remain_model_correctable() {
    run_async_test_with_stack(
        "invalid_json_result_selections_remain_model_correctable",
        invalid_json_result_selections_remain_model_correctable_impl,
    );
}

async fn invalid_json_result_selections_remain_model_correctable_impl() {
    let fixture = large_nested_result_fixture().await;
    let h = &fixture.harness;
    let result_ref = &fixture.result_ref;

    h.push_script([
        RebornScriptedReply::tool_call(
            "builtin.result_read",
            json!({
                "result_ref": result_ref,
                "offset": 0,
                "max_bytes": ironclaw_host_api::model_result_preview::MODEL_RESULT_PREVIEW_MAX_BYTES,
                "json_pointer": "/payload/items/2345/id",
                "limit": 5,
            }),
        ),
        RebornScriptedReply::text("recovered from the incompatible limit"),
    ]);
    h.submit_turn("apply a collection limit to the selected scalar")
        .await
        .expect("incompatible limit remains model-correctable");
    h.assert_conversation_history_role_contains(MessageKind::ToolResultReference, "invalid_input")
        .await
        .expect("collection limit on a scalar is a structured model-visible input failure");

    h.push_script([
        RebornScriptedReply::tool_call(
            "builtin.result_read",
            json!({
                "result_ref": result_ref,
                "offset": 0,
                "max_bytes": ironclaw_threads::TOOL_RESULT_RECORD_READ_MAX_BYTES,
                "json_pointer": "/payload/items/does-not-exist",
            }),
        ),
        RebornScriptedReply::text("recovered"),
    ]);
    h.submit_turn("select the missing nested item")
        .await
        .expect("missing pointer remains model-correctable");
    h.assert_conversation_history_role_contains(MessageKind::ToolResultReference, "invalid_input")
        .await
        .expect("missing JSON pointer is a structured model-visible input failure");

    h.push_script([
        RebornScriptedReply::tool_call(
            "builtin.result_read",
            json!({
                "result_ref": result_ref,
                "offset": 99_999,
                "max_bytes": ironclaw_host_api::model_result_preview::MODEL_RESULT_PREVIEW_MAX_BYTES,
                "json_pointer": "/payload/items",
            }),
        ),
        RebornScriptedReply::text("recovered from the invalid offset"),
    ]);
    h.submit_turn("continue past the end of the selected array")
        .await
        .expect("invalid JSON offset remains model-correctable");
    h.assert_conversation_history_role_contains(MessageKind::ToolResultReference, "invalid_input")
        .await
        .expect("invalid JSON offset is a structured model-visible input failure");
}

/// JSON selection is additive: omitting `json_pointer` keeps the exact legacy
/// byte reader and its durable total-size continuation contract.
#[test]
fn result_read_preserves_exact_legacy_byte_reads() {
    run_async_test_with_stack(
        "result_read_preserves_exact_legacy_byte_reads",
        result_read_preserves_exact_legacy_byte_reads_impl,
    );
}

async fn result_read_preserves_exact_legacy_byte_reads_impl() {
    let fixture = large_nested_result_fixture().await;
    let h = &fixture.harness;
    let result_ref = &fixture.result_ref;
    let serialized = &fixture.serialized;

    h.push_script([
        RebornScriptedReply::tool_call(
            "builtin.result_read",
            json!({
                "result_ref": result_ref,
                "offset": 0,
                "max_bytes": 32,
            }),
        ),
        RebornScriptedReply::text("raw bytes"),
    ]);
    h.submit_turn("read the raw result bytes")
        .await
        .expect("legacy byte read turn completes");
    let raw_page = h
        .tool_result_output("builtin.result_read")
        .await
        .expect("legacy result_read output is recorded");
    assert_eq!(
        raw_page["content"]
            .as_str()
            .expect("raw page has text")
            .as_bytes(),
        &serialized[..32]
    );
    assert_eq!(
        raw_page["total_bytes"].as_u64(),
        Some(serialized.len() as u64)
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

/// A structured array first look carries the full count in its summary and
/// keeps continuation inside the page. The outer `item_count` stays absent
/// because that legacy field is valid only alongside an outer byte offset.
/// `builtin.json` `parse` is the granted capability whose output is a
/// top-level JSON array.
#[test]
fn structured_array_result_persists_count_summary_and_redacted_page() {
    run_async_test_with_stack(
        "structured_array_result_persists_count_summary_and_redacted_page",
        structured_array_result_persists_count_summary_and_redacted_page_impl,
    );
}

async fn structured_array_result_persists_count_summary_and_redacted_page_impl() {
    let mut items: Vec<String> = (0..4000).map(|i| format!("item-{i:04}")).collect();
    items[0] = "secret".to_string();
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

    h.assert_conversation_history_role_contains(MessageKind::ToolResultReference, "4000 items")
        .await
        .expect("persisted summary names the array's element count");
    let envelopes = h
        .persisted_tool_result_envelopes()
        .await
        .expect("tool-result envelopes persist");
    let observation = envelopes
        .last()
        .and_then(|envelope| envelope.model_observation.as_ref())
        .expect("metadata-only array observation survives");
    assert!(observation["detail"].get("item_count").is_none());
    let preview = observation["detail"]["preview"]
        .as_str()
        .expect("structured array preview survives with redaction");
    assert!(
        !preview.contains("secret"),
        "credential marker must not survive structured preview redaction"
    );
    let page = serde_json::from_str::<serde_json::Value>(preview)
        .expect("redacted structured preview remains parseable JSON");
    assert_eq!(page["node_type"], "array");
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

/// #6284 item 1, at the seam that matters: a **caller-shaped capability port
/// error ends the tool call, not the run**.
///
/// Before the capability-stage fix, `capability_host_error` mapped every
/// non-`Cancelled` `AgentLoopHostError` from the port to a terminal
/// `HostUnavailable{Capability}` — so an expired credential, a scope
/// mismatch, or a malformed invocation killed a run the model could have
/// recovered from. The executor now splits the port-error kinds exhaustively:
/// caller-shaped ones (`InvalidInvocation` here) surface as a model-visible tool
/// error and the loop continues; genuine host faults stay terminal.
///
/// Asserted at the durable seam — the persisted `ToolResultReference` envelope
/// and the finalized reply — not on a completed status, so it proves the model
/// actually saw the failure *and* kept working. Crate-tier coverage of the same
/// split lives in `ironclaw_agent_loop`'s executor tests; this pins it through
/// the production composition.
#[tokio::test]
async fn caller_shaped_capability_port_error_is_a_tool_error_not_a_dead_run() {
    let h = RebornIntegrationHarness::test_default()
        .with_recoverable_port_error_for_test()
        .script([
            RebornScriptedReply::tool_call("test_echo", json!({"message": "hi"})),
            RebornScriptedReply::text("the tool was refused, so here is what I can say instead"),
        ])
        .build()
        .await
        .expect("harness builds");

    h.submit_turn("use the echo tool")
        .await
        .expect("turn completes");

    // The model was told, in the durable envelope the next turn reads from.
    h.assert_tool_error(
        reborn_support::assertions::ToolErrorClass::Failed,
        "input_encode",
    )
    .await
    .expect("a caller-shaped port error reaches the model as a recoverable tool error");

    // …and the run kept going rather than dying on the port error.
    h.assert_reply_contains("here is what I can say instead")
        .await
        .expect("the run continues past a recoverable port error");
}
