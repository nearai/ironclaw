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
