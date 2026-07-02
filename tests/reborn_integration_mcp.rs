//! Reborn integration-test framework — slice 6: MCP mock.
//!
//! Proves the MCP tool path end-to-end with a real in-process HTTP server:
//! scripted model emits a `mock-mcp.search` tool call → the real MCP runtime
//! sends `tools/call` over HTTP to the loopback mock server → the call is
//! captured → the model finalizes a text reply. Same SDK seam as slices 1–2;
//! no real network, no services, no API keys, no Docker, no `integration` feature.

#[allow(dead_code)]
#[path = "support/reborn/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
mod support;

use reborn_support::assertions::ToolErrorClass;
use reborn_support::builder::RebornIntegrationHarness;
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
async fn mcp_tool_call_error_surfaces_recoverable_failed() {
    let server = start_mock_mcp_server(vec![MockToolResponse {
        name: "search".to_string(),
        content: serde_json::json!({"results": []}),
    }])
    .await;
    server.set_tool_call_error(-32602, "unknown tool");

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

// MCP "auth mismatch" (HTTP 401/403) is deliberately NOT covered here: unlike
// the two cases above, a 401/403 maps to `McpClientError::AuthRequired` — an
// auth *gate* (park-and-resume), not a model-visible `Failed`/`Denied` tool
// error. Exercising a credentialed-backend 401 through to a raised auth gate
// requires a capability-backend credential stub that this tier does not yet
// have; it is the same "live-401 re-auth arm" already deferred in
// `reborn_integration_auth_failure.rs`. Track with that follow-up.
