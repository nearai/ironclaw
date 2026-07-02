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

use ironclaw_threads::MessageKind;
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

const ERR_A_URL: &str = "https://api.example.test/v1/err-a";
const ERR_B_URL: &str = "https://api.example.test/v1/err-b";

/// Demo + regression for the multi-turn (baseline-sliced) thread-history
/// assertions: `history_len` / `assert_tool_error_since` /
/// `assert_no_tool_error_since` / `assert_conversation_history_contains{,_since}`
/// / `assert_conversation_history_role_contains`.
///
/// One harness runs TWO turns on the same thread. Turn 1 raises a `Denied`
/// (`policy_denied`) tool error; turn 2 raises a `Failed` (`output_too_large`)
/// one. Capturing `history_len` between the turns lets the `*_since` asserts
/// scope to only the turn under test — the gap the full-history `assert_tool_error`
/// (documented single-turn-only) cannot close. The negative branches double as
/// the fail-checks: the slice must EXCLUDE turn 1's error and turn 1's prompt,
/// and the role filter must not match a `User` prompt as an `Assistant` reply.
#[tokio::test]
async fn multi_turn_baseline_sliced_history_assertions() {
    use ironclaw_host_api::RUNTIME_HTTP_REASON_RESPONSE_BODY_LIMIT_EXCEEDED;
    let h = RebornIntegrationHarness::test_default()
        .with_keyed_http_responses([
            ScriptedHttpResponse::network_error(ERR_A_URL, "policy_denied"),
            ScriptedHttpResponse::response_error(
                ERR_B_URL,
                RUNTIME_HTTP_REASON_RESPONSE_BODY_LIMIT_EXCEEDED,
            ),
        ])
        .script([
            RebornScriptedReply::tool_call("builtin.http", json!({"url": ERR_A_URL})),
            RebornScriptedReply::text("done one"),
            RebornScriptedReply::tool_call("builtin.http", json!({"url": ERR_B_URL})),
            RebornScriptedReply::text("done two"),
        ])
        .build()
        .await
        .expect("harness builds");

    // Turn 1: Denied{policy_denied}.
    h.submit_turn("first fetch")
        .await
        .expect("turn 1 completes");

    // Baseline captured BETWEEN turns — everything before this is turn 1.
    let after_turn_one = h.history_len().await.expect("history len readable");

    // Turn 2: Failed{output_too_large}.
    h.submit_turn("second fetch")
        .await
        .expect("turn 2 completes");

    // --- tool-error slicing ---
    // Turn 2's Failed IS in the slice.
    h.assert_tool_error_since(after_turn_one, ToolErrorClass::Failed, "output_too_large")
        .await
        .expect("turn 2 Failed error is in the post-baseline slice");
    // Turn 1's Denied is NOT in the slice (the whole point of the baseline).
    h.assert_no_tool_error_since(after_turn_one, ToolErrorClass::Denied, "policy_denied")
        .await
        .expect("turn 1 Denied error is excluded by the baseline slice");
    // Fail-check: the sliced assert must REJECT turn 1's error (content absent
    // from the slice) — proves the slice genuinely excludes prior turns.
    assert!(
        h.assert_tool_error_since(after_turn_one, ToolErrorClass::Denied, "policy_denied")
            .await
            .is_err(),
        "post-baseline slice must not see turn 1's Denied error"
    );
    // From baseline 0 the whole history is in scope, so turn 1's Denied IS seen.
    h.assert_tool_error_since(0, ToolErrorClass::Denied, "policy_denied")
        .await
        .expect("turn 1 Denied error is visible from baseline 0");
    // Full-history (un-sliced) asserts still see BOTH turns' errors — backward compat.
    h.assert_tool_error(ToolErrorClass::Denied, "policy_denied")
        .await
        .expect("full-history sees turn 1 Denied");
    h.assert_tool_error(ToolErrorClass::Failed, "output_too_large")
        .await
        .expect("full-history sees turn 2 Failed");

    // --- conversation-history containment ---
    h.assert_conversation_history_contains("first fetch")
        .await
        .expect("turn 1 user prompt persisted");
    h.assert_conversation_history_contains("second fetch")
        .await
        .expect("turn 2 user prompt persisted");
    h.assert_conversation_history_contains("done two")
        .await
        .expect("turn 2 assistant reply persisted");
    // Multi-turn: only turn 2's prompt is in the post-baseline slice.
    h.assert_conversation_history_contains_since(after_turn_one, "second fetch")
        .await
        .expect("turn 2 prompt is in the post-baseline slice");
    // Fail-check: turn 1's prompt is before the baseline → excluded.
    assert!(
        h.assert_conversation_history_contains_since(after_turn_one, "first fetch")
            .await
            .is_err(),
        "post-baseline slice must not see turn 1's user prompt"
    );
    // Role filter: "second fetch" is a User message, matched under User…
    h.assert_conversation_history_role_contains(MessageKind::User, "second fetch")
        .await
        .expect("user-role filter matches the user prompt");
    // …but NOT under Assistant (fail-check: role filter discriminates).
    assert!(
        h.assert_conversation_history_role_contains(MessageKind::Assistant, "second fetch")
            .await
            .is_err(),
        "assistant-role filter must not match a user prompt"
    );
    // Fail-check: absent text is never found.
    assert!(
        h.assert_conversation_history_contains("never-said-this")
            .await
            .is_err(),
        "containment assert must reject text absent from the whole transcript"
    );
    // Fail-check: an out-of-range baseline (stale/foreign-thread value) is a
    // loud error, not an empty slice — otherwise `assert_no_tool_error_since`
    // would pass vacuously on a caller bug.
    let past_end = h.history_len().await.expect("history len readable") + 1;
    assert!(
        h.assert_no_tool_error_since(past_end, ToolErrorClass::Denied, "policy_denied")
            .await
            .is_err(),
        "out-of-range baseline must fail loud, not vacuously pass"
    );
}
