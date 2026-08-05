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

    const WARNING: &str = "loop control repeated capability call detected";
    h.assert_model_request_contains(WARNING)
        .await
        .expect("the repeated-call warning must still reach the model");
    h.assert_system_prompt_excludes(WARNING)
        .await
        .expect("the warning must not be folded into the cached system prefix");
    // Position and role, not just presence: a nudge emitted ahead of the
    // thread messages, or at Role::User, passes the two assertions above and
    // still breaks caching (or hijacks the last-user-message consumers).
    h.assert_rides_conversation_tail(WARNING)
        .await
        .expect("the warning must ride the tail as a host reminder");
    h.assert_system_prompts_identical()
        .await
        .expect("the system prefix must stay byte-identical across all model calls in the run");
    h.assert_prompt_cache_prefix_stable()
        .await
        .expect("no model call may rewrite the cached prefix without a tool-surface change");
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

    const CLOCK: &str = "Current date/time at loop start";
    h.assert_model_request_contains(CLOCK)
        .await
        .expect("runtime context must still reach the model");
    h.assert_system_prompt_excludes(CLOCK)
        .await
        .expect("the clock must not sit in the cached system prefix");
    h.assert_rides_conversation_tail(CLOCK)
        .await
        .expect("the clock must ride the tail as a host reminder, after the turn's messages");
    h.assert_system_prompts_identical()
        .await
        .expect("system prompts across turns of one conversation must be byte-identical");
    h.assert_prompt_cache_prefix_stable()
        .await
        .expect("no model call may rewrite the cached prefix without a tool-surface change");
}

/// The clock reaching the model must never come at the cost of hijacking the
/// last-user-message consumers.
///
/// `unavailable_requested_capability_guard`, `SmartRoutingProvider::classify`
/// and the trace-replay hint all answer "what did the user last ask?" by
/// scanning back for `Role::User`. The tail reminder is the final message on
/// nearly every call, so if it carried the user role those consumers would read
/// the runtime clock instead of the request — the guard would stop inspecting
/// the real ask, and every turn would score identically for routing (#6985).
#[tokio::test]
async fn tail_reminder_does_not_displace_the_real_user_message() {
    let h = RebornIntegrationHarness::test_default()
        .script([RebornScriptedReply::text("answered")])
        .build()
        .await
        .expect("harness builds");
    h.submit_turn("summarize the quarterly report")
        .await
        .expect("turn completes");

    h.assert_last_user_message_is("summarize the quarterly report")
        .await
        .expect("the last Role::User message must be the user's actual ask, not the reminder");
}
