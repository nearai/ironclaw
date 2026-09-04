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
async fn repeated_call_warning_reaches_model_and_recovers() {
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
        .assert_model_message_content_contains("repeated capability call detected")
        .await
        .expect("warning reaches the model");
    harness
        .assert_model_tools_contains("test_echo")
        .await
        .expect("warning turn retains the normal tool surface");
}

#[tokio::test]
async fn repeated_calls_after_warning_remain_advisory() {
    let repeated = || RebornScriptedReply::tool_call("test_echo", json!({"message": "same"}));
    let harness = RebornIntegrationHarness::test_default()
        .with_no_progress_echo_for_test()
        .record_model_calls_for_test()
        .script([
            repeated(),
            repeated(),
            repeated(),
            repeated(),
            RebornScriptedReply::text("finished after another repeated call"),
        ])
        .build()
        .await
        .expect("harness builds");

    harness
        .submit_turn("make progress")
        .await
        .expect("repeated calls remain non-terminal");
    harness
        .assert_reply_contains("finished after another repeated call")
        .await
        .expect("the final reply persists after the repeated calls");
    harness
        .assert_tool_invocation_count("test.echo", 4)
        .await
        .expect("the capability still runs after the advisory warning");
    harness
        .assert_model_message_content_occurrences("repeated capability call detected", 1)
        .await
        .expect("the warning is rendered only once for one uninterrupted streak");
}

#[tokio::test]
async fn repeated_identical_call_terminates_as_no_progress_after_second_strike() {
    // Byte-identical scripted output each call. Two strikes at threshold 8
    // need 16 calls minimum (8 + reset + 8); 20 leaves margin.
    let repeated = || RebornScriptedReply::tool_call("test_echo", json!({"message": "same"}));
    let harness = RebornIntegrationHarness::test_default()
        .with_no_progress_echo_for_test()
        .with_turn_event_sink()
        .script(std::iter::repeat_with(repeated).take(20))
        .build()
        .await
        .expect("harness builds");
    let run_id = harness
        .submit_turn_async("stuck task")
        .await
        .expect("turn submitted");
    let state = harness
        .wait_for_status(run_id, TurnStatus::Failed)
        .await
        .expect("run reaches Failed after two strikes");
    let failure = state
        .failure
        .as_ref()
        .expect("failed run carries a durable failure");
    assert_eq!(failure.category(), "no_progress_detected");
}

#[tokio::test]
async fn alternating_signatures_with_byte_identical_outputs_terminate_as_no_progress() {
    // Strict A/B alternation, each side byte-identical across repeats. Needs
    // the leading signature to reach 8 twice with a reset between: ~30 calls
    // minimum; 40 (20 pairs) gives >30% margin.
    let a = || RebornScriptedReply::tool_call("test_echo", json!({"message": "a"}));
    let b = || RebornScriptedReply::tool_call("test_echo", json!({"message": "b"}));
    let harness = RebornIntegrationHarness::test_default()
        .with_no_progress_echo_for_test()
        .with_turn_event_sink()
        .script(
            std::iter::repeat_with(move || [a(), b()])
                .take(20)
                .flatten(),
        )
        .build()
        .await
        .expect("harness builds");
    let run_id = harness
        .submit_turn_async("alternate forever")
        .await
        .expect("turn submitted");
    let state = harness
        .wait_for_status(run_id, TurnStatus::Failed)
        .await
        .expect("alternating pattern reaches Failed");
    let failure = state
        .failure
        .as_ref()
        .expect("failed run carries a durable failure");
    assert_eq!(failure.category(), "no_progress_detected");
}

#[tokio::test]
async fn same_call_with_changing_output_completes_instead_of_terminating() {
    // Negative control: the SAME call repeated 40 times, output different
    // every time (a red/green loop, or a log tail) — must never trip the
    // check no matter how many times the call itself repeats.
    let changing = std::iter::repeat_with(|| {
        RebornScriptedReply::tool_call("test_echo", json!({"message": "changing-output"}))
    })
    .take(40);
    let harness = RebornIntegrationHarness::test_default()
        .with_no_progress_echo_for_test()
        .with_turn_event_sink()
        .script(changing.chain(std::iter::once(RebornScriptedReply::text("done"))))
        .build()
        .await
        .expect("harness builds");
    let run_id = harness
        .submit_turn_async("iterate until done")
        .await
        .expect("turn submitted");
    harness
        .wait_for_status(run_id, TurnStatus::Completed)
        .await
        .expect("changing-output repetition must complete, not terminate as no-progress");
    harness
        .assert_model_tool_result_content_occurrences("echo: changing-output-", 40)
        .await
        .expect("each fixed-signature call must return its own changing output");
}
