//! Reborn integration-test framework — slice 2 tool-calling turn.
//!
//! Proves the tool path + the §3.7 two-tier egress design end-to-end: the
//! scripted model emits a `builtin.http` tool call → the real first-party tool
//! runtime executes it through `RuntimeHttpEgress` → the call is captured by the
//! recording egress (Tier-2) → the model finalizes a text reply. Same single
//! LLM seam as slice 1 (scripted `TraceLlm` beneath the real decorator chain);
//! no network, no services, no keys, no Docker, no `integration` feature.

#[allow(dead_code)]
#[path = "support/reborn/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
mod support;

use reborn_support::builder::RebornIntegrationHarness;
use reborn_support::reply::RebornScriptedReply;
use serde_json::json;

#[tokio::test]
async fn runs_http_tool_call_through_recorded_egress() {
    let h = RebornIntegrationHarness::test_default()
        .with_builtin_http_tools()
        .script([
            RebornScriptedReply::tool_call("builtin.http", json!({"url": HTTP_TOOL_URL})),
            RebornScriptedReply::text("fetched"),
        ])
        .build()
        .await
        .expect("harness builds");
    h.submit_turn("fetch items").await.expect("turn completes");
    h.assert_tool_invoked("builtin.http")
        .await
        .expect("http tool ran");
    h.assert_egress_request_matching("api.example.test")
        .await
        .expect("Tier-2 egress captured");
    h.assert_reply_contains("fetched")
        .await
        .expect("final reply finalized");
}

const HTTP_TOOL_URL: &str = "https://api.example.test/v1/items";

/// Guards the assertion helpers against silently passing: when the scripted tool
/// call is *absent* (a plain text turn on the default echo backend, which runs no
/// tool and captures no egress), both `assert_tool_invoked` and
/// `assert_egress_request_matching` must return `Err`.
#[tokio::test]
async fn assertions_fail_when_tool_did_not_run() {
    let h = RebornIntegrationHarness::test_default()
        .script([RebornScriptedReply::text("no tool")])
        .build()
        .await
        .expect("harness builds");
    h.submit_turn("just talk").await.expect("turn completes");
    assert!(h.assert_tool_invoked("builtin.http").await.is_err());
    assert!(
        h.assert_egress_request_matching("api.example.test")
            .await
            .is_err()
    );
}

/// Proves the assertion helpers discriminate when the invocation + egress lists
/// are NON-empty: a real `builtin.http` call runs, but assertions for a
/// *different* capability / host must return `Err` — exercising the
/// "present but no match" branch, not the empty-list branch (builder.rs:331).
#[tokio::test]
async fn assertions_fail_when_tool_present_but_requested_tool_or_url_does_not_match() {
    let h = RebornIntegrationHarness::test_default()
        .with_builtin_http_tools()
        .script([
            RebornScriptedReply::tool_call("builtin.http", json!({"url": HTTP_TOOL_URL})),
            RebornScriptedReply::text("done"),
        ])
        .build()
        .await
        .expect("harness builds");
    h.submit_turn("fetch items").await.expect("turn completes");
    // Prove the capture lists are NON-empty first, so the negative checks below
    // exercise the mismatch branch rather than passing vacuously on empty lists.
    h.assert_tool_invoked("builtin.http")
        .await
        .expect("http tool ran before mismatch assertions");
    h.assert_egress_request_matching("api.example.test")
        .await
        .expect("http egress captured before mismatch assertions");
    // Non-empty invocation list — wrong capability id must fail.
    assert!(
        h.assert_tool_invoked("some.other.capability")
            .await
            .is_err()
    );
    // Non-empty egress list — non-matching host substring must fail.
    assert!(
        h.assert_egress_request_matching("nonmatching.host.test")
            .await
            .is_err()
    );
}

/// Proves the multi-segment `builtin.http.save` capability id — whose
/// `.`→`__` encoding produces `builtin__http__save` at the provider seam
/// (reply.rs:33) — resolves end-to-end through the real runtime.
///
/// Args: `url` (same constant the existing test uses) + `save_to` under the
/// `/workspace` mount that `core_builtin_tools` provides with read-write
/// permissions. The recording egress returns a fixed `{"accepted":true}` body;
/// no network is touched.
#[tokio::test]
async fn runs_http_save_tool_call_through_recorded_egress() {
    let h = RebornIntegrationHarness::test_default()
        .with_builtin_http_tools()
        .script([
            RebornScriptedReply::tool_call(
                "builtin.http.save",
                json!({"url": HTTP_TOOL_URL, "save_to": "/workspace/response.json"}),
            ),
            RebornScriptedReply::text("saved"),
        ])
        .build()
        .await
        .expect("harness builds");
    h.submit_turn("fetch and save")
        .await
        .expect("turn completes");
    h.assert_tool_invoked("builtin.http.save")
        .await
        .expect("http.save tool ran");
    // The save path must reach the real `RuntimeHttpEgress`; assert the recorded
    // egress so a regression that bypasses it cannot pass this test.
    h.assert_egress_request_matching("api.example.test")
        .await
        .expect("http.save egress captured");
    h.assert_reply_contains("saved")
        .await
        .expect("final reply finalized");
}
