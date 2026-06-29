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
