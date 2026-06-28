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

use reborn_support::builder::RebornIntegrationHarness;
use reborn_support::reply::RebornScriptedReply;
use support::mock_mcp_server::{MockToolResponse, start_mock_mcp_server};

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
            RebornScriptedReply::tool_call("mock-mcp.search", serde_json::json!({})),
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
