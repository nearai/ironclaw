//! Whole-turn coverage for checkpointed pre-termination warning recovery.

#[allow(dead_code)]
#[path = "support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

use std::num::NonZeroU32;

use ironclaw_turns::TurnStatus;
use reborn_support::builder::RebornIntegrationHarness;
use reborn_support::reply::RebornScriptedReply;
use serde_json::json;

#[tokio::test]
async fn iteration_limit_warning_reaches_model_with_tools_and_recovers() {
    let harness = RebornIntegrationHarness::test_default()
        .with_iteration_limit_for_test(NonZeroU32::new(1).expect("nonzero"))
        .script([
            RebornScriptedReply::tool_call("test_echo", json!({"message": "work"})),
            RebornScriptedReply::text("recovered at the iteration limit"),
        ])
        .build()
        .await
        .expect("harness builds");

    harness
        .submit_turn("finish this task")
        .await
        .expect("turn recovers");
    harness
        .assert_reply_contains("recovered at the iteration limit")
        .await
        .expect("recovered reply persisted");
    harness
        .assert_model_message_content_contains("final recovery iteration")
        .await
        .expect("warning reaches the model");
    harness
        .assert_model_tools_contains("test_echo")
        .await
        .expect("warning turn retains the normal tool surface");
}

#[tokio::test]
async fn no_progress_warning_reaches_model_and_recovers() {
    let repeated = || RebornScriptedReply::tool_call("test_echo", json!({"message": "same"}));
    let harness = RebornIntegrationHarness::test_default()
        .with_no_progress_echo_for_test()
        .script([
            repeated(),
            repeated(),
            repeated(),
            RebornScriptedReply::text("recovered after changing approach"),
        ])
        .build()
        .await
        .expect("harness builds");

    harness
        .submit_turn("make progress")
        .await
        .expect("turn recovers");
    harness
        .assert_reply_contains("recovered after changing approach")
        .await
        .expect("recovered reply persisted");
    harness
        .assert_model_message_content_contains("no progress detected")
        .await
        .expect("warning reaches the model");
    harness
        .assert_model_tools_contains("test_echo")
        .await
        .expect("warning turn retains the normal tool surface");
}

#[tokio::test]
async fn repeated_no_progress_after_warning_fails_without_extra_capability_turns() {
    let repeated = || RebornScriptedReply::tool_call("test_echo", json!({"message": "same"}));
    let harness = RebornIntegrationHarness::test_default()
        .with_no_progress_echo_for_test()
        .script([repeated(), repeated(), repeated(), repeated()])
        .build()
        .await
        .expect("harness builds");

    let run_id = harness
        .submit_turn_async("make progress")
        .await
        .expect("turn submitted");
    let state = harness
        .wait_for_status(run_id, TurnStatus::Failed)
        .await
        .expect("the repeated warning action reaches the typed failure");
    let failure = state
        .failure
        .as_ref()
        .expect("a failed run carries failure evidence");
    assert_eq!(
        failure.category(),
        "no_progress_detected",
        "the first repeated no-change action after the warning must terminalize"
    );
    harness
        .assert_tool_invocation_count("test.echo", 4)
        .await
        .expect("the warning turn runs once and no fifth capability turn is granted");
}
