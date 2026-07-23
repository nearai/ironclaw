//! C-WEBACCESS: first-party `web-access.*` capabilities dispatched through the
//! real `WebAccessExecutor`, which speaks MCP JSON-RPC over three sequential
//! `RuntimeHttpEgress` calls (initialize → notifications/initialized →
//! tools/call) to the Exa MCP endpoint. `.with_web_access_tools([..])` scripts
//! the three-leg handshake onto the recording egress's FIFO queue (all three
//! legs share one URL, so the keyed HTTP matcher can't tell them apart).

#[allow(dead_code)]
#[path = "support/mod.rs"]
mod ironclaw_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

use ironclaw_support::assertions::ToolErrorClass;
use ironclaw_support::builder::IronClawIntegrationHarness;
use ironclaw_support::reply::IronClawScriptedReply;
use serde_json::json;

const MCP_INIT_BODY: &[u8] =
    br#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05","capabilities":{}}}"#;
const MCP_NOTIF_BODY: &[u8] = br#"{"accepted":true}"#;

/// SSE-framed `initialize` result with a leading keepalive `ping` event before
/// the real `message` event — legal because `web_access` sends `Accept:
/// application/json, text/event-stream`. Mirrors the unit fixture in
/// `web_access.rs::extracts_text_from_sse_mcp_response`.
const MCP_INIT_BODY_SSE: &[u8] = b"event: ping\ndata:\n\nevent: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"protocolVersion\":\"2024-11-05\",\"capabilities\":{}}}\n\n";

/// SSE-framed `tools/call` result wrapping the same `{"result":{"content":..}}`
/// shape `mcp_tool_call_result_body` emits, as a single `data:` event.
fn mcp_tool_call_result_body_sse(content_text: &str) -> Vec<u8> {
    let json = json!({
        "result": {
            "content": [
                {"type": "text", "text": content_text}
            ]
        }
    })
    .to_string();
    format!("event: message\ndata: {json}\n\n").into_bytes()
}

/// Builds the `tools/call` JSON-RPC result body shared by `web-access.search`
/// and `web-access.get_content` responses (only the text content differs).
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
    let harness = IronClawIntegrationHarness::test_default()
        .with_web_access_tools([
            MCP_INIT_BODY.to_vec(),
            MCP_NOTIF_BODY.to_vec(),
            mcp_tool_call_result_body(
                "Title: Tokio Async Runtime\nURL: https://tokio.rs\nText: Tokio is an async \
                 runtime for Rust providing IO, networking, and scheduling.",
            ),
        ])
        .script([
            IronClawScriptedReply::tool_call(
                "web-access.search",
                json!({"query": "rust async runtimes"}),
            ),
            IronClawScriptedReply::text("done"),
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
    let harness = IronClawIntegrationHarness::test_default()
        .with_web_access_tools([
            MCP_INIT_BODY.to_vec(),
            MCP_NOTIF_BODY.to_vec(),
            mcp_tool_call_result_body(
                "# Example Domain\nURL: https://example.com\n\nThis domain is for illustrative \
                 examples in documents.",
            ),
        ])
        .script([
            IronClawScriptedReply::tool_call(
                "web-access.get_content",
                json!({"url": "https://example.com"}),
            ),
            IronClawScriptedReply::text("done"),
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

/// Format-matrix regression (C-WIREFMT): the Exa MCP server answers
/// `initialize` with SSE framing; `web-access.search` must still round-trip
/// end-to-end. Int-tier twin of the crate-tier
/// `search_accepts_sse_framed_initialize_response`.
#[tokio::test]
async fn web_search_over_sse_framed_initialize_dispatches_through_exa_mcp() {
    let harness = IronClawIntegrationHarness::test_default()
        .with_web_access_tools([
            MCP_INIT_BODY_SSE.to_vec(),
            MCP_NOTIF_BODY.to_vec(),
            mcp_tool_call_result_body(
                "Title: Tokio Async Runtime\nURL: https://tokio.rs\nText: Tokio is an async \
                 runtime for Rust providing IO, networking, and scheduling.",
            ),
        ])
        .script([
            IronClawScriptedReply::tool_call(
                "web-access.search",
                json!({"query": "rust async runtimes"}),
            ),
            IronClawScriptedReply::text("done"),
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
        .expect("capability dispatched through the real executor over SSE-framed initialize");
    harness
        .assert_egress_count(3)
        .await
        .expect("all three MCP handshake legs dispatched");
    harness
        .assert_tool_result_contains(
            "Tokio is an async runtime for Rust providing IO, networking, and scheduling.",
        )
        .await
        .expect("SSE-framed initialize accepted so the handshake reached tools/call");
}

/// Sibling parity (C-WIREFMT): both body-parsing legs (`initialize` and
/// `tools/call`) are SSE-framed on the same handshake, proving each leg
/// inherits the other's framing matrix rather than only the tested one.
#[tokio::test]
async fn web_access_handshake_over_sse_framed_both_legs() {
    let harness = IronClawIntegrationHarness::test_default()
        .with_web_access_tools([
            MCP_INIT_BODY_SSE.to_vec(),
            MCP_NOTIF_BODY.to_vec(),
            mcp_tool_call_result_body_sse(
                "# Example Domain\nURL: https://example.com\n\nThis domain is for illustrative \
                 examples in documents.",
            ),
        ])
        .script([
            IronClawScriptedReply::tool_call(
                "web-access.get_content",
                json!({"url": "https://example.com"}),
            ),
            IronClawScriptedReply::text("done"),
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
        .expect("capability dispatched with both handshake legs SSE-framed");
    harness
        .assert_egress_count(3)
        .await
        .expect("all three MCP handshake legs dispatched");
    harness
        .assert_tool_result_contains("This domain is for illustrative examples in documents.")
        .await
        .expect("SSE-framed tools/call result surfaced back to the model");
}

/// Guards `assert_egress_body_contains_any` against vacuous pass: when the
/// substring is genuinely absent from every captured egress request body to
/// the matching URL, the assertion must return `Err`, not silently succeed.
#[tokio::test]
async fn assert_egress_body_contains_any_fails_when_substring_absent() {
    let harness = IronClawIntegrationHarness::test_default()
        .with_web_access_tools([
            MCP_INIT_BODY.to_vec(),
            MCP_NOTIF_BODY.to_vec(),
            mcp_tool_call_result_body(
                "Title: Tokio Async Runtime\nURL: https://tokio.rs\nText: Tokio is an async \
                 runtime for Rust providing IO, networking, and scheduling.",
            ),
        ])
        .script([
            IronClawScriptedReply::tool_call(
                "web-access.search",
                json!({"query": "rust async runtimes"}),
            ),
            IronClawScriptedReply::text("done"),
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

/// Guards the OTHER error branch of `assert_egress_body_contains_any`: no
/// captured request's URL matches `url_substr` at all (vs. matching the URL
/// but missing the body substring, covered by the sibling test above).
#[tokio::test]
async fn assert_egress_body_contains_any_fails_when_url_absent() {
    let harness = IronClawIntegrationHarness::test_default()
        .with_web_access_tools([
            MCP_INIT_BODY.to_vec(),
            MCP_NOTIF_BODY.to_vec(),
            mcp_tool_call_result_body(
                "Title: Tokio Async Runtime\nURL: https://tokio.rs\nText: Tokio is an async \
                 runtime for Rust providing IO, networking, and scheduling.",
            ),
        ])
        .script([
            IronClawScriptedReply::tool_call(
                "web-access.search",
                json!({"query": "rust async runtimes"}),
            ),
            IronClawScriptedReply::text("done"),
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
    // Pins the specific "no matching URL" branch, not just any `Err` — the
    // sibling substring-absent branch returns a differently worded message.
    assert!(
        err.to_string()
            .contains("no captured egress request matching url"),
        "expected the no-matching-URL branch's error message, got: {err}"
    );
}

/// Error path: a scripted Exa fetch-error response surfaces as a model-visible
/// `Failed`/`operation_failed` tool error. The next real model request sees the
/// failure, changes strategy to a search, observes that successful result, and
/// only then finalizes the run rather than terminalizing it.
#[tokio::test]
async fn get_content_fetch_error_surfaces_recoverable_failed() {
    let harness = IronClawIntegrationHarness::test_default()
        .with_web_access_tools([
            MCP_INIT_BODY.to_vec(),
            MCP_NOTIF_BODY.to_vec(),
            mcp_tool_call_result_body("Error fetching https://example.com: 404 not found"),
            MCP_INIT_BODY.to_vec(),
            MCP_NOTIF_BODY.to_vec(),
            mcp_tool_call_result_body(
                "Title: Example Domain\nURL: https://example.com\nText: fallback search succeeded",
            ),
        ])
        .script([
            IronClawScriptedReply::tool_call(
                "web-access.get_content",
                json!({"url": "https://example.com"}),
            ),
            IronClawScriptedReply::tool_call(
                "web-access.search",
                json!({"query": "Example Domain"}),
            ),
            IronClawScriptedReply::text("Recovered with the fallback search result."),
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
    let model_requests = harness.scripted_llm.captured_requests();
    assert_eq!(
        model_requests.len(),
        3,
        "failure, corrected action, and final reply each require a model turn"
    );
    let recovery_request =
        serde_json::to_string(&model_requests[1]).expect("recovery model request serializes");
    for expected in [
        "operation_failed",
        "respect_failure_constraint",
        "same_call_retry",
    ] {
        assert!(
            recovery_request.contains(expected),
            "the immediate recovery request must contain {expected:?}: {recovery_request}"
        );
    }
    harness
        .assert_tool_invoked("web-access.search")
        .await
        .expect("the model changed strategy after observing the failure");
    harness
        .assert_tool_result_contains("fallback search succeeded")
        .await
        .expect("the corrected fallback action succeeded");
    harness
        .assert_reply_contains("Recovered with the fallback search result.")
        .await
        .expect("run recovered and finalized (not terminal driver_unavailable)");
}
