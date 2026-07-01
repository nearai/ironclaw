//! Reborn integration-test framework — slice 4: URL/method-keyed HTTP matcher
//! + richer egress assertions over a multi-step tool-HTTP flow.
//!
//! Builds on slice 2's `RecordingRuntimeHttpEgress` (FIFO scripted body +
//! substring `assert_egress_request_matching`). Here the scripted model makes
//! TWO `builtin.http` calls to DIFFERENT URLs; the keyed matcher returns a
//! DIFFERENT scripted body per URL (and can key on method); the test asserts
//! each keyed body surfaced as a tool result and asserts the egress request log
//! (count / URL order / method order / per-URL body). Same single LLM seam as
//! slices 1–2 (scripted `TraceLlm` beneath the real decorator chain); no
//! network, no services, no keys, no Docker, no `integration` feature.

#[allow(dead_code)]
#[path = "support/reborn/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
mod support;

use reborn_support::assertions::ToolErrorClass;
use reborn_support::builder::RebornIntegrationHarness;
use reborn_support::http_matcher::ScriptedHttpResponse;
use reborn_support::reply::RebornScriptedReply;
use serde_json::json;

const ITEMS_URL: &str = "https://api.example.test/v1/items";
const ORDERS_URL: &str = "https://api.example.test/v1/orders";

/// Keyed matcher routes a distinct scripted body per URL across a 2-step
/// `builtin.http` flow; the egress log is asserted on count, URL order, method
/// order, and per-URL body.
#[tokio::test]
async fn keyed_matcher_routes_distinct_bodies_per_url_in_multi_step_flow() {
    let h = RebornIntegrationHarness::test_default()
        .with_keyed_http_responses([
            ScriptedHttpResponse::for_url(ITEMS_URL, br#"{"marker":"items-body"}"#),
            ScriptedHttpResponse::for_url(ORDERS_URL, br#"{"marker":"orders-body"}"#)
                .with_method("post"),
        ])
        .script([
            RebornScriptedReply::tool_call("builtin.http", json!({"url": ITEMS_URL})),
            RebornScriptedReply::tool_call(
                "builtin.http",
                json!({"url": ORDERS_URL, "method": "post", "body": {"qty": 1}}),
            ),
            RebornScriptedReply::text("done"),
        ])
        .build()
        .await
        .expect("harness builds");
    h.submit_turn("fetch then order")
        .await
        .expect("turn completes");
    h.assert_tool_result_contains("items-body")
        .await
        .expect("items keyed body surfaced");
    h.assert_tool_result_contains("orders-body")
        .await
        .expect("orders keyed body surfaced");
    h.assert_egress_count(2).await.expect("two egress calls");
    h.assert_egress_url_order(&[ITEMS_URL, ORDERS_URL])
        .await
        .expect("egress URLs in order");
    h.assert_egress_method_order(&["get", "post"])
        .await
        .expect("egress methods in order");
    h.assert_egress_body_contains(ORDERS_URL, "qty")
        .await
        .expect("post body captured");
}

/// Guards the new egress assertions against passing vacuously: with the same
/// real 2-call flow, a wrong count / wrong URL order / wrong method order /
/// wrong body must each return `Err`.
#[tokio::test]
async fn egress_assertions_discriminate_on_mismatch() {
    let h = RebornIntegrationHarness::test_default()
        .with_keyed_http_responses([ScriptedHttpResponse::for_url(ITEMS_URL, br#"{"ok":true}"#)])
        .script([
            RebornScriptedReply::tool_call("builtin.http", json!({"url": ITEMS_URL})),
            RebornScriptedReply::text("done"),
        ])
        .build()
        .await
        .expect("harness builds");
    h.submit_turn("fetch").await.expect("turn completes");
    h.assert_egress_count(1).await.expect("one egress call");
    assert!(h.assert_egress_count(2).await.is_err());
    assert!(h.assert_egress_url_order(&[ORDERS_URL]).await.is_err());
    assert!(h.assert_egress_method_order(&["post"]).await.is_err());
    assert!(
        h.assert_egress_body_contains(ITEMS_URL, "nonmatching")
            .await
            .is_err()
    );
}

const CAP_URL: &str = "https://api.example.test/v1/cap-keyed";

/// Capability-keyed matcher: two responses scripted for the SAME URL but with
/// DIFFERENT `with_capability` keys. The first entry is keyed to a capability
/// that `builtin.http` does NOT carry, so the request falls through to the
/// second entry (keyed to `"builtin.http"`), which is the fallback that actually
/// matches. Proves first-match-wins fallthrough on a capability mismatch.
#[tokio::test]
async fn capability_keyed_response_matches_and_mismatch_falls_through_to_second_entry() {
    let h = RebornIntegrationHarness::test_default()
        .with_keyed_http_responses([
            // First entry: same URL, capability "builtin.http.wrong" — does NOT match
            // a request whose capability_id is "builtin.http".
            ScriptedHttpResponse::for_url(CAP_URL, br#"{"marker":"wrong-cap-body"}"#)
                .with_capability("builtin.http.wrong"),
            // Second entry: same URL, capability "builtin.http" — the fallback that
            // matches after the first entry fails the capability check.
            ScriptedHttpResponse::for_url(CAP_URL, br#"{"marker":"http-cap-body"}"#)
                .with_capability("builtin.http"),
        ])
        .script([
            RebornScriptedReply::tool_call("builtin.http", json!({"url": CAP_URL})),
            RebornScriptedReply::text("done"),
        ])
        .build()
        .await
        .expect("harness builds");
    h.submit_turn("fetch").await.expect("turn completes");
    // The builtin.http call fell through the wrong-cap entry and matched the http-cap entry.
    h.assert_tool_result_contains("http-cap-body")
        .await
        .expect("capability-matched body returned after fallthrough");
    // The wrong-cap entry was NOT matched — its body never surfaced.
    assert!(
        h.assert_tool_result_contains("wrong-cap-body")
            .await
            .is_err(),
        "wrong-capability entry must not match a builtin.http call"
    );
}

const ERR_URL: &str = "https://api.example.test/v1/err";

/// Error path — HTTP 5xx status. A scripted `500` is NOT an egress error: the
/// `builtin.http` tool surfaces it as a *successful* (Completed) tool result
/// carrying `"status":500`, so the run completes and the model can react. Proves
/// a server-error status is model-visible, not a terminal driver failure.
#[tokio::test]
async fn http_5xx_status_surfaces_as_completed_result_with_status() {
    let h = RebornIntegrationHarness::test_default()
        .with_keyed_http_responses([
            ScriptedHttpResponse::for_url(ERR_URL, br#"{"error":"boom"}"#).with_status(500),
        ])
        .script([
            RebornScriptedReply::tool_call("builtin.http", json!({"url": ERR_URL})),
            RebornScriptedReply::text("done"),
        ])
        .build()
        .await
        .expect("harness builds");
    h.submit_turn("fetch").await.expect("turn completes");
    h.assert_tool_result_contains("\"status\":500")
        .await
        .expect("5xx status surfaced in the model-visible tool result");
    h.assert_reply_contains("done")
        .await
        .expect("run recovered and finalized");
}

/// Error path — network-policy-denied egress. The scripted egress `Err` maps
/// (`policy_denied` reason) to a model-visible `Denied` capability outcome, so
/// the run continues to completion rather than dying with `driver_unavailable`.
/// Asserts the `denied` category surfaced (not merely that the run completed).
#[tokio::test]
async fn http_network_policy_denied_surfaces_recoverable_denied() {
    let h = RebornIntegrationHarness::test_default()
        .with_keyed_http_responses([ScriptedHttpResponse::network_error(
            ERR_URL,
            "policy_denied",
        )])
        .script([
            RebornScriptedReply::tool_call("builtin.http", json!({"url": ERR_URL})),
            RebornScriptedReply::text("done"),
        ])
        .build()
        .await
        .expect("harness builds");
    h.submit_turn("fetch").await.expect("turn completes");
    h.assert_tool_error(ToolErrorClass::Denied, "policy_denied")
        .await
        .expect("policy-denied surfaced as a model-visible Denied tool error");
    h.assert_reply_contains("done")
        .await
        .expect("run recovered and finalized (not terminal driver_unavailable)");
}

/// Error path — oversize response body. The scripted egress
/// `response_body_limit_exceeded` error maps to a model-visible
/// `Failed{OutputTooLarge}` capability outcome; the run recovers to completion.
#[tokio::test]
async fn http_oversize_response_surfaces_recoverable_failed() {
    use ironclaw_host_api::RUNTIME_HTTP_REASON_RESPONSE_BODY_LIMIT_EXCEEDED;
    let h = RebornIntegrationHarness::test_default()
        .with_keyed_http_responses([ScriptedHttpResponse::response_error(
            ERR_URL,
            RUNTIME_HTTP_REASON_RESPONSE_BODY_LIMIT_EXCEEDED,
        )])
        .script([
            RebornScriptedReply::tool_call("builtin.http", json!({"url": ERR_URL})),
            RebornScriptedReply::text("done"),
        ])
        .build()
        .await
        .expect("harness builds");
    h.submit_turn("fetch").await.expect("turn completes");
    h.assert_tool_error(ToolErrorClass::Failed, "output_too_large")
        .await
        .expect("oversize response surfaced as a model-visible Failed tool error");
    h.assert_reply_contains("done")
        .await
        .expect("run recovered and finalized (not terminal driver_unavailable)");
}

/// Guards `assert_tool_error` against a vacuous pass, mirroring the sibling
/// negative guards (`shell_assertions_fail_when_no_shell_call_ran`,
/// `assert_mcp_tool_called_fails_when_no_mcp_call_ran`). Three ways it must
/// return `Err`: (a) a completed turn that persisted NO tool-error reference at
/// all, (b) a real `Denied` turn probed with the wrong reason, and (c) that same
/// `Denied` turn probed with the WRONG CLASS but the right reason token —
/// proving the class discriminates structurally, not just the reason.
#[tokio::test]
async fn tool_error_assertion_fails_without_matching_tool_error() {
    // (a) Plain text turn — no tool call, so no `ToolResultReference` is persisted.
    let clean = RebornIntegrationHarness::test_default()
        .script([RebornScriptedReply::text("no tool")])
        .build()
        .await
        .expect("harness builds");
    clean
        .submit_turn("just talk")
        .await
        .expect("turn completes");
    assert!(
        clean
            .assert_tool_error(ToolErrorClass::Denied, "policy_denied")
            .await
            .is_err(),
        "assertion must reject a turn that persisted no tool-error reference"
    );

    // A real `Denied{policy_denied}` turn for cases (b) and (c).
    let denied = RebornIntegrationHarness::test_default()
        .with_keyed_http_responses([ScriptedHttpResponse::network_error(
            ERR_URL,
            "policy_denied",
        )])
        .script([
            RebornScriptedReply::tool_call("builtin.http", json!({"url": ERR_URL})),
            RebornScriptedReply::text("done"),
        ])
        .build()
        .await
        .expect("harness builds");
    denied.submit_turn("fetch").await.expect("turn completes");
    // (b) Right class, wrong reason.
    assert!(
        denied
            .assert_tool_error(ToolErrorClass::Denied, "backend")
            .await
            .is_err(),
        "assertion must reject a reason that is absent from the persisted summary"
    );
    // (c) Wrong class, right reason token — the crux of the class-discrimination
    // fix: `policy_denied` is present, but the outcome is `Denied`, not `Failed`.
    assert!(
        denied
            .assert_tool_error(ToolErrorClass::Failed, "policy_denied")
            .await
            .is_err(),
        "assertion must discriminate the outcome class, not just the reason token"
    );
}
