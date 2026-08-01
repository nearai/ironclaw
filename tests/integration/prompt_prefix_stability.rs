//! C-CACHE-PREFIX (#6985): the coalesced system prompt must stay byte-stable
//! across model calls so provider prompt caching can hit. Loop-control nudges
//! and the runtime clock ride the conversation tail — they must never rewrite
//! the cached system prefix, neither mid-run (nudges render on a later
//! iteration) nor across turns of one conversation (the clock advances).

#[allow(dead_code)]
#[path = "support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

use reborn_support::builder::RebornIntegrationHarness;
use reborn_support::reply::RebornScriptedReply;
use serde_json::json;

/// Repeating one capability call with identical arguments until the loop's
/// repeated-call warning renders must NOT rewrite the system prompt: the
/// warning reaches the model in the conversation tail while every captured
/// system prompt in the run stays byte-identical.
#[tokio::test]
async fn nudge_rides_the_tail_not_the_system_prefix() {
    let call = || {
        RebornScriptedReply::tool_call("builtin.http", json!({"url": "https://example.com/probe"}))
    };
    let h = RebornIntegrationHarness::test_default()
        .with_builtin_http_tools()
        .script([
            call(),
            call(),
            call(),
            RebornScriptedReply::text("answered from current evidence"),
        ])
        .build()
        .await
        .expect("harness builds");
    h.submit_turn("probe the endpoint")
        .await
        .expect("turn completes");

    h.assert_model_request_contains("loop control repeated capability call detected")
        .await
        .expect("the repeated-call warning must still reach the model");
    h.assert_system_prompt_excludes("loop control repeated capability call detected")
        .await
        .expect("the warning must not be folded into the cached system prefix");
    h.assert_system_prompts_identical()
        .await
        .expect("the system prefix must stay byte-identical across all model calls in the run");
}

/// The runtime clock must not live in the cached system prefix: two turns on
/// one conversation produce byte-identical system prompts, with the date/time
/// context still reaching the model in the conversation tail.
#[tokio::test]
async fn runtime_clock_rides_the_tail_and_prefix_is_stable_across_turns() {
    let h = RebornIntegrationHarness::test_default()
        .script([
            RebornScriptedReply::text("first"),
            RebornScriptedReply::text("second"),
        ])
        .build()
        .await
        .expect("harness builds");
    h.submit_turn("first question")
        .await
        .expect("turn one completes");
    h.submit_turn("second question")
        .await
        .expect("turn two completes");

    h.assert_model_request_contains("Current date/time at loop start")
        .await
        .expect("runtime context must still reach the model");
    h.assert_system_prompt_excludes("Current date/time at loop start")
        .await
        .expect("the clock must not sit in the cached system prefix");
    h.assert_system_prompts_identical()
        .await
        .expect("system prompts across turns of one conversation must be byte-identical");
}
