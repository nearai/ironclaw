//! Reborn integration-test framework — first-party `web-access.*` coverage
//! (C-WEBACCESS).
//!
//! `web-access.search` / `web-access.get_content` are `RuntimeKind::FirstParty`
//! capabilities dispatched through the real `WebAccessExecutor`, which speaks
//! MCP JSON-RPC by hand over three sequential `RuntimeHttpEgress` calls
//! (`initialize` → `notifications/initialized` → `tools/call`) to the Exa MCP
//! endpoint. `.with_web_access_tools([..])` wires the real executor behind a
//! thin test adapter and scripts the three-leg handshake onto the recording
//! egress's FIFO queue at build time (all three legs share one URL, so the
//! keyed HTTP matcher can't tell them apart). Same single LLM seam as every
//! other Reborn integration test; no real network, services, keys, or Docker.

#[allow(dead_code)]
#[path = "support/reborn/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
mod support;

use reborn_support::assertions::ToolErrorClass;
use reborn_support::builder::RebornIntegrationHarness;
use reborn_support::reply::RebornScriptedReply;
use serde_json::json;

const MCP_INIT_BODY: &[u8] =
    br#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05","capabilities":{}}}"#;
const MCP_NOTIF_BODY: &[u8] = br#"{"accepted":true}"#;

/// `web-access.search` dispatches through the real `WebAccessExecutor` over
/// the scripted Exa MCP handshake; the search result content surfaces back to
/// the model as a tool result.
#[tokio::test]
async fn web_search_dispatches_through_scripted_exa_mcp() {
    let harness = RebornIntegrationHarness::test_default()
        .with_web_access_tools([
            MCP_INIT_BODY.to_vec(),
            MCP_NOTIF_BODY.to_vec(),
            b"{\"result\":{\"content\":[{\"type\":\"text\",\"text\":\"Title: Tokio Async Runtime\\nURL: https://tokio.rs\\nText: Tokio is an async runtime for Rust providing IO, networking, and scheduling.\"}]}}".to_vec(),
        ])
        .script([
            RebornScriptedReply::tool_call(
                "web-access.search",
                json!({"query": "rust async runtimes"}),
            ),
            RebornScriptedReply::text("done"),
        ])
        .build()
        .await
        .expect("harness builds");

    harness
        .submit_turn("search the web for rust async runtimes")
        .await
        .expect("turn completes");

    harness
        .assert_tool_invoked("web-access.search")
        .await
        .expect("capability dispatched through the real executor");
    harness
        .assert_egress_body_contains_any(
            ironclaw_first_party_extensions::EXA_MCP_HOST,
            "web_search_exa",
        )
        .await
        .expect("outbound MCP tools/call carried the exa search tool name");
    harness
        .assert_egress_body_contains_any(
            ironclaw_first_party_extensions::EXA_MCP_HOST,
            "rust async runtimes",
        )
        .await
        .expect("outbound MCP tools/call carried the search query");
    harness
        .assert_tool_result_contains(
            "Tokio is an async runtime for Rust providing IO, networking, and scheduling.",
        )
        .await
        .expect("scripted MCP search result surfaced back to the model");
}

/// `web-access.get_content` dispatches through the real `WebAccessExecutor`
/// (the `web_fetch_exa` MCP tool) over the same scripted handshake shape.
#[tokio::test]
async fn get_content_dispatches_through_scripted_exa_mcp() {
    let harness = RebornIntegrationHarness::test_default()
        .with_web_access_tools([
            MCP_INIT_BODY.to_vec(),
            MCP_NOTIF_BODY.to_vec(),
            b"{\"result\":{\"content\":[{\"type\":\"text\",\"text\":\"# Example Domain\\nURL: https://example.com\\n\\nThis domain is for illustrative examples in documents.\"}]}}".to_vec(),
        ])
        .script([
            RebornScriptedReply::tool_call(
                "web-access.get_content",
                json!({"url": "https://example.com"}),
            ),
            RebornScriptedReply::text("done"),
        ])
        .build()
        .await
        .expect("harness builds");

    harness
        .submit_turn("fetch the content at https://example.com")
        .await
        .expect("turn completes");

    harness
        .assert_tool_invoked("web-access.get_content")
        .await
        .expect("capability dispatched through the real executor");
    harness
        .assert_egress_body_contains_any(
            ironclaw_first_party_extensions::EXA_MCP_HOST,
            "web_fetch_exa",
        )
        .await
        .expect("outbound MCP tools/call carried the exa fetch tool name");
    harness
        .assert_egress_body_contains_any(
            ironclaw_first_party_extensions::EXA_MCP_HOST,
            "https://example.com",
        )
        .await
        .expect("outbound MCP tools/call carried the requested URL");
    harness
        .assert_tool_result_contains("This domain is for illustrative examples in documents.")
        .await
        .expect("scripted MCP fetch result surfaced back to the model");
}

/// Guards `assert_egress_body_contains_any` against vacuous pass: when the
/// substring is genuinely absent from every captured egress request body to
/// the matching URL, the assertion must return `Err`, not silently succeed.
#[tokio::test]
async fn assert_egress_body_contains_any_fails_when_substring_absent() {
    let harness = RebornIntegrationHarness::test_default()
        .with_web_access_tools([
            MCP_INIT_BODY.to_vec(),
            MCP_NOTIF_BODY.to_vec(),
            b"{\"result\":{\"content\":[{\"type\":\"text\",\"text\":\"Title: Tokio Async Runtime\\nURL: https://tokio.rs\\nText: Tokio is an async runtime for Rust providing IO, networking, and scheduling.\"}]}}".to_vec(),
        ])
        .script([
            RebornScriptedReply::tool_call(
                "web-access.search",
                json!({"query": "rust async runtimes"}),
            ),
            RebornScriptedReply::text("done"),
        ])
        .build()
        .await
        .expect("harness builds");

    harness
        .submit_turn("search the web for rust async runtimes")
        .await
        .expect("turn completes");

    let result = harness
        .assert_egress_body_contains_any(
            ironclaw_first_party_extensions::EXA_MCP_HOST,
            "a substring that never appears in any scripted leg",
        )
        .await;
    assert!(
        result.is_err(),
        "assert_egress_body_contains_any must fail when no captured egress body to the \
         matching URL contains the substring"
    );
}

/// Error path — `web-access.get_content` requests a URL the scripted Exa MCP
/// response reports as failed to fetch. `WebAccessExecutor::fetch_content`
/// treats an `"Error fetching <requested url>"` line in the MCP tool-call
/// text as a hard failure (`parse_fetch_results` -> `operation_error()` ->
/// `RuntimeDispatchErrorKind::OperationFailed`), proving
/// `WebAccessTestHandler`'s error-mapping path (`dispatch` ->
/// `web_access_test_error` -> `FirstPartyCapabilityError`) surfaces a real
/// `WebAccessDispatchError` as a model-visible `Failed` tool error — the run
/// reaches `Completed`, not a terminal `driver_unavailable` — rather than a
/// dropped `Err` or a mis-mapped error class.
#[tokio::test]
async fn get_content_fetch_error_surfaces_recoverable_failed() {
    let harness = RebornIntegrationHarness::test_default()
        .with_web_access_tools([
            MCP_INIT_BODY.to_vec(),
            MCP_NOTIF_BODY.to_vec(),
            br#"{"result":{"content":[{"type":"text","text":"Error fetching https://example.com: 404 not found"}]}}"#.to_vec(),
        ])
        .script([
            RebornScriptedReply::tool_call(
                "web-access.get_content",
                json!({"url": "https://example.com"}),
            ),
            RebornScriptedReply::text("done"),
        ])
        .build()
        .await
        .expect("harness builds");

    harness
        .submit_turn("fetch the content at https://example.com")
        .await
        .expect("turn completes");

    harness
        .assert_tool_error(ToolErrorClass::Failed, "operation_failed")
        .await
        .expect("Exa fetch-error content surfaced as a model-visible Failed tool error");
    harness
        .assert_reply_contains("done")
        .await
        .expect("run recovered and finalized (not terminal driver_unavailable)");
}
