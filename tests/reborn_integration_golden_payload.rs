//! Reborn integration — golden inference-payload coverage.
//!
//! Exact-matches the FULL model-visible inference payload (system prompt +
//! conversation turns + tool-call/tool-result messages + ordered tool surface)
//! per inference iteration against a committed `insta` snapshot, plus the exact
//! final user-visible reply. Where `assert_system_prompt_contains` proves a
//! substring reached the model, this pins end-to-end prompt construction byte
//! for byte — catching silent drift in prompt assembly, history accumulation,
//! and tool-result feed-back. See `tests/support/reborn/golden.rs` for the
//! canonicalization + single-filter normalization rationale. Regenerate drift
//! with `cargo insta review` (or `INSTA_UPDATE=always cargo test`).

#[allow(dead_code)]
#[path = "support/reborn/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
mod support;

use reborn_support::builder::RebornIntegrationHarness;
use reborn_support::reply::RebornScriptedReply;
use serde_json::json;

const HTTP_TOOL_URL: &str = "https://api.example.test/v1/items";

/// (a) Single-turn greeting: the one inference call's full payload + the exact
/// final reply. Pins the base system-prompt construction and text-turn shape.
#[tokio::test]
async fn golden_single_turn_greeting() {
    let h = RebornIntegrationHarness::test_default()
        .script([RebornScriptedReply::text("Hello! How can I help?")])
        .build()
        .await
        .expect("harness builds");
    h.submit_turn("hi there").await.expect("turn completes");
    h.assert_golden_payload("greeting");
    h.assert_reply_eq("Hello! How can I help?")
        .await
        .expect("final reply matches exactly");
}

/// (b) Tool-call turn: BOTH inference iterations exact-matched — the initial
/// call and the post-tool-result call. Pins tool-result feed-back construction
/// (the assistant `tool_calls[].id` and the following `tool` message's
/// `tool_call_id` must match).
#[tokio::test]
async fn golden_tool_call_feedback() {
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
    h.assert_golden_payload("tool_call");
    h.assert_reply_eq("fetched")
        .await
        .expect("final reply matches exactly");
}

/// (c) Multi-turn (two user turns): the second turn's inference call carries the
/// accumulated history (turn-1 user + assistant reply + turn-2 user). Golden
/// pins history/turns accumulation across turns.
#[tokio::test]
async fn golden_multi_turn_history() {
    let h = RebornIntegrationHarness::test_default()
        .script([
            RebornScriptedReply::text("First reply"),
            RebornScriptedReply::text("Second reply"),
        ])
        .build()
        .await
        .expect("harness builds");
    h.submit_turn("first question")
        .await
        .expect("turn 1 completes");
    h.submit_turn("second question")
        .await
        .expect("turn 2 completes");
    // Structural pin alongside the golden snapshot (see assert messages below
    // for the exact shape).
    let requests = h.scripted_llm.captured_requests();
    assert_eq!(requests.len(), 2, "one inference call per turn");
    assert_eq!(
        requests[1].len(),
        4,
        "second call carries system prompt + turn-1 user/assistant + turn-2 user"
    );
    h.assert_golden_payload("multi_turn");
    h.assert_reply_eq("Second reply")
        .await
        .expect("final reply matches exactly");
}
