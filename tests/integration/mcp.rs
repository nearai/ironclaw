//! Reborn integration-test framework — slice 6: MCP mock.
//!
//! Proves the MCP tool path end-to-end with a real in-process HTTP server:
//! scripted model emits a `mock-mcp.search` tool call → the real MCP runtime
//! sends `tools/call` over HTTP to the loopback mock server → the call is
//! captured → the model finalizes a text reply. Same SDK seam as slices 1–2;
//! no real network, no services, no API keys, no Docker, no `integration` feature.

#[allow(dead_code)]
#[path = "support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

use reborn_support::assertions::ToolErrorClass;
use reborn_support::builder::RebornIntegrationHarness;
use reborn_support::group::RebornIntegrationGroup;
use reborn_support::reply::RebornScriptedReply;
use support::mock_mcp_server::{MockMcpServer, MockToolResponse, start_mock_mcp_server};

/// Asserts the mock MCP server actually received a `tools/call` request naming
/// `expected_tool` with a `query` argument equal to `expected_query`. Mirrors
/// the inline server-side check in `mcp_tool_call_reaches_mock_server`, so
/// error-path tests also prove the loopback HTTP boundary was reached, not
/// just that the harness's own capability-invocation recorder fired.
fn assert_recorded_tools_call(server: &MockMcpServer, expected_tool: &str, expected_query: &str) {
    let recorded = server.recorded_requests();
    let tools_call = recorded
        .iter()
        .find(|r| r.method == "tools/call")
        .unwrap_or_else(|| panic!("no tools/call recorded; saw: {:?}", recorded));

    assert_eq!(
        tools_call
            .params
            .as_ref()
            .and_then(|p| p.get("name"))
            .and_then(|n| n.as_str()),
        Some(expected_tool)
    );
    assert_eq!(
        tools_call
            .params
            .as_ref()
            .and_then(|p| p.get("arguments"))
            .and_then(|a| a.get("query"))
            .and_then(|q| q.as_str()),
        Some(expected_query)
    );
}

/// The bundled `nearai` package is hosted MCP rather than Emulate-backed.
/// Install and activate the real first-party manifest, then prove its static
/// search capability crosses the mediated MCP boundary with its credential
/// and returns the hermetic server result to the model.
#[tokio::test]
async fn nearai_web_search_dispatches_through_bundled_hosted_mcp() {
    let group = RebornIntegrationGroup::extension_lifecycle()
        .await
        .expect("extension-lifecycle group builds");
    let h = group
        .thread("nearai-hosted-mcp-lifecycle")
        .script([
            RebornScriptedReply::tool_call(
                "builtin.extension_install",
                serde_json::json!({"extension_id": "nearai"}),
            ),
            RebornScriptedReply::text("NEAR AI search is installed."),
            RebornScriptedReply::tool_call(
                "nearai.web_search",
                serde_json::json!({"query": "IronClaw capability evidence"}),
            ),
            RebornScriptedReply::text("search complete"),
        ])
        .build()
        .await
        .expect("NEAR AI lifecycle thread builds");

    // #6520 removed the public activate action: install owns readiness, so
    // the caller's nearai account must resolve BEFORE install for the
    // package to reconcile straight to active.
    h.seed_capability_credential_account("nearai", "NEAR AI integration account", &[])
        .await
        .expect("NEAR AI account is seeded under the dispatching user");
    h.submit_turn("install NEAR AI search")
        .await
        .expect("install turn completes");
    h.assert_tool_result_contains(r#""installed":true"#)
        .await
        .expect("NEAR AI package installed");
    h.submit_turn("search for IronClaw capability evidence")
        .await
        .expect("search turn completes");
    h.assert_model_tools_contains("nearai__web_search")
        .await
        .expect("activated NEAR AI capability is disclosed to the model");
    h.assert_tool_invoked("nearai.web_search")
        .await
        .expect("canonical NEAR AI capability dispatched");
    h.assert_tool_result_contains("REBORN_NEARAI_WEB_SEARCH_RESULT")
        .await
        .expect("hosted MCP response reached the model-facing result");

    let requests = h.captured_network_requests_for_test();
    let tools_call = requests
        .iter()
        .find(|request| {
            serde_json::from_slice::<serde_json::Value>(&request.body)
                .ok()
                .and_then(|body| body["method"].as_str().map(str::to_owned))
                .as_deref()
                == Some("tools/call")
        })
        .unwrap_or_else(|| {
            panic!(
                "no hosted MCP tools/call captured across {} redacted request(s)",
                requests.len()
            )
        });
    let body: serde_json::Value =
        serde_json::from_slice(&tools_call.body).expect("tools/call body is JSON");
    assert_eq!(body["params"]["name"], "web_search");
    assert_eq!(
        body["params"]["arguments"]["query"],
        "IronClaw capability evidence"
    );
    assert!(
        tools_call.headers.iter().any(|(name, value)| {
            name.eq_ignore_ascii_case("authorization")
                && value.starts_with("Bearer ")
                && value.len() > "Bearer ".len()
        }),
        "hosted MCP tools/call must carry a mediated bearer credential"
    );
}

/// Core slice-6 scenario: a scripted MCP tool call round-trips through the real
/// MCP runtime to the loopback mock server, and the invocation is recorded.
#[tokio::test]
async fn mcp_tool_call_reaches_mock_server() {
    let server = start_mock_mcp_server(vec![MockToolResponse {
        name: "search".to_string(),
        content: serde_json::json!({"results": ["mock result"]}),
    }])
    .await;

    let h = RebornIntegrationHarness::test_default()
        .script([
            RebornScriptedReply::tool_call(
                "mock-mcp.search",
                serde_json::json!({"query": "needle-xyz-42"}),
            ),
            RebornScriptedReply::text("done"),
        ])
        .with_mock_mcp(server.mcp_url())
        .build()
        .await
        .expect("harness builds");

    h.submit_turn("search for something")
        .await
        .expect("turn completes");
    h.assert_reply_contains("done")
        .await
        .expect("final reply finalized");
    h.assert_mcp_tool_called("search")
        .await
        .expect("MCP tool was invoked");
    // Confirm the mock server actually received the `tools/call` over HTTP with
    // the scripted arguments intact — proves the MCP runtime made a real HTTP
    // call to the loopback server, not just that the capability recorder fired
    // before the egress (the M4 gap).
    assert_recorded_tools_call(&server, "search", "needle-xyz-42");
}

/// Twin of `mcp_tool_call_reaches_mock_server`: same client `Accept:
/// application/json, text/event-stream` header (`crates/ironclaw_mcp/src/lib.rs`),
/// but here the mock server answers every leg with SSE framing instead of
/// plain JSON. Exercises `parse_mcp_response`/`response_is_sse` against a
/// real reqwest response's headers, not just the hand-built fixtures in the
/// crate-tier `parse_mcp_response_accepts_both_advertised_framings`.
#[tokio::test]
async fn mcp_tool_call_over_sse_framed_responses() {
    let server = start_mock_mcp_server(vec![MockToolResponse {
        name: "search".to_string(),
        content: serde_json::json!({"results": ["mock result"]}),
    }])
    .await;
    server.enable_sse_framing();

    let h = RebornIntegrationHarness::test_default()
        .script([
            RebornScriptedReply::tool_call(
                "mock-mcp.search",
                serde_json::json!({"query": "needle-sse-99"}),
            ),
            RebornScriptedReply::text("done"),
        ])
        .with_mock_mcp(server.mcp_url())
        .build()
        .await
        .expect("harness builds");

    h.submit_turn("search for something")
        .await
        .expect("turn completes");
    h.assert_reply_contains("done")
        .await
        .expect("final reply finalized over SSE-framed handshake");
    h.assert_mcp_tool_called("search")
        .await
        .expect("MCP tool invoked despite SSE framing on every leg");
    // Confirms the SSE-framed `initialize` leg round-tripped with the
    // scripted arguments intact, not just that the recorder fired.
    assert_recorded_tools_call(&server, "search", "needle-sse-99");
    // Distinct from the assert above: proves the tool result surfaced back to
    // the model over the SSE-framed wire, not just that the call was recorded.
    h.assert_tool_result_contains("mock result")
        .await
        .expect("scripted mock result surfaced through the SSE-framed response");
    // `LoopbackMcpRuntimeHttpEgress` pre-declares the capability schema
    // locally, so the client never sends a live `tools/list` (see
    // `with_mock_mcp` docs). Pins that `enable_sse_framing` covered exactly
    // this three-leg handshake, not a superset with an untested `tools/list`.
    let methods: Vec<String> = server
        .recorded_requests()
        .into_iter()
        .map(|r| r.method)
        .collect();
    assert_eq!(
        methods,
        vec!["initialize", "notifications/initialized", "tools/call"]
    );
}

/// Guards `assert_mcp_tool_called` against vacuous pass: when no MCP tool ran
/// (plain echo turn on the default backend), the assertion must return `Err`.
#[tokio::test]
async fn assert_mcp_tool_called_fails_when_no_mcp_call_ran() {
    let h = RebornIntegrationHarness::test_default()
        .script([RebornScriptedReply::text("no mcp")])
        .build()
        .await
        .expect("harness builds");

    h.submit_turn("just talk").await.expect("turn completes");
    assert!(h.assert_mcp_tool_called("search").await.is_err());
}

/// Error path — MCP `tools/call` returns a JSON-RPC `error` object. The client
/// surfaces this as `Failed{Backend}` (a recoverable, model-visible tool
/// error), so the run continues to completion rather than dying with
/// `driver_unavailable`. Distinct wire path from the 5xx case below: this trips
/// the client's JSON-RPC error-field guard, not its HTTP status gate.
#[tokio::test]
async fn mcp_tool_call_error_cause_reaches_next_model_request() {
    let server = start_mock_mcp_server(vec![MockToolResponse {
        name: "search".to_string(),
        content: serde_json::json!({"results": []}),
    }])
    .await;
    server.set_tool_call_error(-32602, "distinctive-mcp-cause-5965");

    let h = RebornIntegrationHarness::test_default()
        .script([
            RebornScriptedReply::tool_call("mock-mcp.search", serde_json::json!({"query": "x"})),
            RebornScriptedReply::text("done"),
        ])
        .with_mock_mcp(server.mcp_url())
        .build()
        .await
        .expect("harness builds");

    h.submit_turn("search").await.expect("turn completes");
    assert_recorded_tools_call(&server, "search", "x");
    h.assert_mcp_tool_called("search")
        .await
        .expect("MCP tool was invoked before the error");
    h.assert_tool_error(ToolErrorClass::Failed, "backend")
        .await
        .expect("JSON-RPC error surfaced as a model-visible Failed tool error");
    h.assert_model_request_contains("distinctive-mcp-cause-5965")
        .await
        .expect("MCP backend cause reached the next captured model request");
    h.assert_reply_contains("done")
        .await
        .expect("run recovered and finalized (not terminal driver_unavailable)");
}

/// Error path — MCP server returns HTTP 5xx on the tool call. The client
/// surfaces this as `Failed{Backend}` (recoverable, model-visible), and the run
/// completes. Distinct wire path from the JSON-RPC-error case above: this trips
/// the client's HTTP status gate, not its JSON-RPC error-field guard.
#[tokio::test]
async fn mcp_server_5xx_surfaces_recoverable_failed() {
    let server = start_mock_mcp_server(vec![MockToolResponse {
        name: "search".to_string(),
        content: serde_json::json!({"results": []}),
    }])
    .await;
    server.force_http_status(500);

    let h = RebornIntegrationHarness::test_default()
        .script([
            RebornScriptedReply::tool_call("mock-mcp.search", serde_json::json!({"query": "x"})),
            RebornScriptedReply::text("done"),
        ])
        .with_mock_mcp(server.mcp_url())
        .build()
        .await
        .expect("harness builds");

    h.submit_turn("search").await.expect("turn completes");
    assert_recorded_tools_call(&server, "search", "x");
    h.assert_mcp_tool_called("search")
        .await
        .expect("MCP tool call reached the server before the 5xx");
    h.assert_tool_error(ToolErrorClass::Failed, "backend")
        .await
        .expect("server 5xx surfaced as a model-visible Failed tool error");
    h.assert_reply_contains("done")
        .await
        .expect("run recovered and finalized (not terminal driver_unavailable)");
}

/// Error path — a valid, successful JSON-RPC response whose parsed `result`
/// exceeds the host's MCP output-size limit (`McpRuntimeConfig::default()
/// .max_output_bytes` = 1 MiB). Surfaces as `Failed{OutputTooLarge}`
/// (recoverable, model-visible), run completes. Distinct wire path from both
/// cases above: HTTP and JSON-RPC parse both succeed; this trips the
/// POST-parse `output_bytes > max_output_bytes` check in `McpRuntime::
/// execute_extension_json` — `LoopbackMcpRuntimeHttpEgress` never enforces
/// `response_body_limit` against real bytes, so this is the only size check
/// reachable at this test tier.
#[tokio::test]
async fn mcp_tool_call_output_too_large_surfaces_failed() {
    // `content` serializes (inside the mock server's `{"content":[{"type":
    // "text","text": <json-string>}]}` wrapper) to roughly `oversized_len + 60`
    // bytes — comfortably over the 1 MiB (1_048_576 byte) limit with a wide
    // margin so the exact escaping/wrapper overhead can't make this test flaky.
    let oversized_len = 1_200_000usize;
    let server = start_mock_mcp_server(vec![MockToolResponse {
        name: "search".to_string(),
        content: serde_json::json!({"results": ["x".repeat(oversized_len)]}),
    }])
    .await;

    let h = RebornIntegrationHarness::test_default()
        .script([
            RebornScriptedReply::tool_call("mock-mcp.search", serde_json::json!({"query": "x"})),
            RebornScriptedReply::text("done"),
        ])
        .with_mock_mcp(server.mcp_url())
        .build()
        .await
        .expect("harness builds");

    h.submit_turn("search").await.expect("turn completes");
    assert_recorded_tools_call(&server, "search", "x");
    h.assert_mcp_tool_called("search")
        .await
        .expect("MCP tool call reached the server before the output-size rejection");
    h.assert_tool_error(ToolErrorClass::Failed, "output_too_large")
        .await
        .expect("oversized MCP output surfaced as a model-visible Failed tool error");
    h.assert_reply_contains("done")
        .await
        .expect("run recovered and finalized (not terminal driver_unavailable)");
}

// MCP "auth mismatch" (HTTP 401/403) is deliberately NOT covered here: unlike
// the cases above, a 401/403 maps to `McpClientError::AuthRequired` — an auth
// *gate*, not a model-visible tool error — and requires a capability-backend
// credential stub this tier lacks. Same deferred "live-401 re-auth arm" as
// `reborn_integration_auth_failure.rs`.
