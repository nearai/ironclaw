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

/// Builds the `tools/call` JSON-RPC result body the scripted Exa MCP
/// handshake's third leg returns:
/// `{"result":{"content":[{"type":"text","text":"<content_text>"}]}}`. Both
/// `web-access.search` and `web-access.get_content` MCP responses share this
/// shape — only the text content differs — so every test scripts its third
/// leg with `mcp_tool_call_result_body(..)` instead of hand-writing the JSON.
fn mcp_tool_call_result_body(content_text: &str) -> Vec<u8> {
    json!({
        "result": {
            "content": [
                {"type": "text", "text": content_text}
            ]
        }
    })
    .to_string()
    .into_bytes()
}

/// `web-access.search` dispatches through the real `WebAccessExecutor` over
/// the scripted Exa MCP handshake; the search result content surfaces back to
/// the model as a tool result.
#[tokio::test]
async fn web_search_dispatches_through_scripted_exa_mcp() {
    let harness = RebornIntegrationHarness::test_default()
        .with_web_access_tools([
            MCP_INIT_BODY.to_vec(),
            MCP_NOTIF_BODY.to_vec(),
            mcp_tool_call_result_body(
                "Title: Tokio Async Runtime\nURL: https://tokio.rs\nText: Tokio is an async \
                 runtime for Rust providing IO, networking, and scheduling.",
            ),
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
            mcp_tool_call_result_body(
                "# Example Domain\nURL: https://example.com\n\nThis domain is for illustrative \
                 examples in documents.",
            ),
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
            mcp_tool_call_result_body(
                "Title: Tokio Async Runtime\nURL: https://tokio.rs\nText: Tokio is an async \
                 runtime for Rust providing IO, networking, and scheduling.",
            ),
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

/// Guards `assert_egress_body_contains_any` against its OTHER error branch —
/// when no captured egress request's URL matches `url_substr` at all (as
/// opposed to matching the URL but missing the body substring, covered by
/// `assert_egress_body_contains_any_fails_when_substring_absent` above).
#[tokio::test]
async fn assert_egress_body_contains_any_fails_when_url_absent() {
    let harness = RebornIntegrationHarness::test_default()
        .with_web_access_tools([
            MCP_INIT_BODY.to_vec(),
            MCP_NOTIF_BODY.to_vec(),
            mcp_tool_call_result_body(
                "Title: Tokio Async Runtime\nURL: https://tokio.rs\nText: Tokio is an async \
                 runtime for Rust providing IO, networking, and scheduling.",
            ),
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

    let err = harness
        .assert_egress_body_contains_any("a-host-that-was-never-contacted.example", "anything")
        .await
        .expect_err(
            "assert_egress_body_contains_any must fail when no captured egress request's URL \
             matches url_substr at all",
        );
    // Pins the specific "no matching URL" branch (not just any `Err`) — the
    // sibling substring-absent branch below it in the same function returns a
    // differently worded `Err`, so this message check is what actually
    // distinguishes the two failure paths.
    assert!(
        err.to_string()
            .contains("no captured egress request matching url"),
        "expected the no-matching-URL branch's error message, got: {err}"
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
/// is expected to reach `Completed` rather than a terminal `driver_unavailable`
/// (implied here by the presence of a final reply) — rather than a dropped
/// `Err` or a mis-mapped error class.
#[tokio::test]
async fn get_content_fetch_error_surfaces_recoverable_failed() {
    let harness = RebornIntegrationHarness::test_default()
        .with_web_access_tools([
            MCP_INIT_BODY.to_vec(),
            MCP_NOTIF_BODY.to_vec(),
            mcp_tool_call_result_body("Error fetching https://example.com: 404 not found"),
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
