//! Reborn integration tests for typed model-error recovery observations.
//!
//! These scenarios cross the production turn scheduler, canonical loop,
//! provider gateway, checkpoint, compaction, and transcript seams.

#[allow(dead_code)]
#[path = "support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

use ironclaw_turns::{TurnEventKind, TurnStatus};
use reborn_support::builder::RebornIntegrationHarness;
use reborn_support::doubles::TRANSCRIPT_FAILURE_SECRET;
use reborn_support::http_matcher::ScriptedHttpResponse;
use reborn_support::reply::RebornScriptedReply;
use reborn_support::scripted_provider::CONTEXT_OVERFLOW_USED_TOKENS;
use serde_json::json;

const UNPERSISTED_ASSISTANT_REPLY: &str =
    "raw assistant transcript that must never be reported as a reply";
const UNPERSISTED_TOOL_RESULT: &str =
    "raw tool result that must never enter the transcript failure";
const TRANSCRIPT_FAILURE_TOOL_URL: &str = "https://transcript-failure.example.test/result";

#[tokio::test]
async fn content_filtered_completion_recovers_with_model_visible_observation() {
    let harness = RebornIntegrationHarness::test_default()
        .content_filter_model_once()
        .script([RebornScriptedReply::text("recovered after content filter")])
        .build()
        .await
        .expect("harness builds");
    harness
        .submit_turn("answer safely")
        .await
        .expect("turn recovers after the provider filters one completion");
    harness
        .assert_reply_contains("recovered after content filter")
        .await
        .expect("recovered reply persisted");
    harness
        .assert_model_message_content_contains(
            "model error observation: completion refused by content filter; provide a policy compliant alternative without reproducing blocked content",
        )
        .await
        .expect("retry request carries the typed model-error observation");
    harness
        .assert_interactive_model_provider_call_count(2)
        .await
        .expect("content filtering receives exactly one recovery call");
    harness
        .assert_model_message_content_occurrences("model error observation", 1)
        .await
        .expect("content filtering injects exactly one recovery observation");
    harness
        .assert_model_message_content_not_contains("model response was blocked by provider policy")
        .await
        .expect("gateway summaries do not enter the recovery prompt");
}

#[tokio::test]
async fn context_overflow_recovers_with_model_visible_observation() {
    // Seed one oversized user message so forced compaction exercises the real
    // compactor instead of taking its safe "nothing eligible" skip path.
    let oversized_setup_turn = format!("third setup turn {}", "history ".repeat(5_000));
    let harness = RebornIntegrationHarness::test_default()
        .context_overflow_model_after(3, 3)
        .script([
            RebornScriptedReply::text("first setup reply"),
            RebornScriptedReply::text("second setup reply"),
            RebornScriptedReply::text("third setup reply"),
            RebornScriptedReply::text("compacted recovery history"),
            RebornScriptedReply::text("recovered after context overflow"),
        ])
        .build()
        .await
        .expect("harness builds");
    harness
        .submit_turn("first setup turn")
        .await
        .expect("first setup turn establishes compactable history");
    harness
        .submit_turn("second setup turn")
        .await
        .expect("second setup turn establishes compactable history");
    harness
        .submit_turn(&oversized_setup_turn)
        .await
        .expect("third setup turn establishes compactable history");
    harness
        .submit_turn("answer after compacting")
        .await
        .expect("turn recovers after context overflow exhausts normal retries");
    harness
        .assert_reply_contains("recovered after context overflow")
        .await
        .expect("recovered reply persisted");
    harness
        .assert_model_message_content_contains(
            "model error observation: context overflowed; use the available context and continue",
        )
        .await
        .expect("recovery request carries the typed context-overflow observation");
    harness
        .assert_model_message_content_contains("compacted recovery history")
        .await
        .expect("the final recovery request carries the persisted compaction summary");
    harness
        .assert_interactive_model_provider_call_count(7)
        .await
        .expect("setup and context-overflow recovery use the bounded interactive budget");
    harness
        .assert_text_model_provider_call_count_at_least(1)
        .await
        .expect("context overflow performs a real text-only compaction inference");
    harness
        .assert_model_message_content_occurrences("model error observation", 1)
        .await
        .expect("context overflow injects exactly one recovery observation");
    harness
        .assert_model_message_content_not_contains(&CONTEXT_OVERFLOW_USED_TOKENS.to_string())
        .await
        .expect("provider diagnostics do not enter the recovery prompt");
}

#[tokio::test]
async fn invalid_output_recovers_with_model_visible_observation() {
    let harness = RebornIntegrationHarness::test_default()
        .invalid_output_model_times(3)
        .script([RebornScriptedReply::text("recovered after invalid output")])
        .build()
        .await
        .expect("harness builds");
    harness
        .submit_turn("return a valid answer")
        .await
        .expect("turn recovers after invalid output exhausts normal retries");
    harness
        .assert_reply_contains("recovered after invalid output")
        .await
        .expect("recovered reply persisted");
    harness
        .assert_model_message_content_contains(
            "model error observation: invalid_output reason=empty_assistant_response; repair the response and continue",
        )
        .await
        .expect("recovery request carries the typed invalid-output observation");
    harness
        .assert_interactive_model_provider_call_count(4)
        .await
        .expect("invalid output uses the bounded recovery budget");
    harness
        .assert_model_message_content_occurrences("model error observation", 1)
        .await
        .expect("invalid output injects exactly one recovery observation");
    harness
        .assert_model_message_content_not_contains("model returned an empty assistant response")
        .await
        .expect("gateway summaries do not enter the recovery prompt");
}

/// Regression for #6700: a provider's output-token ceiling is not an input
/// context overflow. The real gateway caller must preserve that distinction,
/// skip context compaction, and give the model an actionable finalization turn.
#[tokio::test]
async fn output_truncation_recovers_without_shrinking_input_context() {
    let harness = RebornIntegrationHarness::test_default()
        .output_truncated_model_times(1)
        .script([RebornScriptedReply::text(
            "concise complete answer after truncation",
        )])
        .build()
        .await
        .expect("harness builds");

    harness
        .submit_turn("give a complete answer")
        .await
        .expect("run survives a truncated provider response");
    harness
        .assert_reply_contains("concise complete answer after truncation")
        .await
        .expect("the recovery turn is durably finalized");
    harness
        .assert_model_message_content_contains(
            "model error observation: output was truncated; continue without repeating if prior partial text is available, otherwise provide a concise complete answer",
        )
        .await
        .expect("the real gateway caller injects continue-or-condense guidance");
    harness
        .assert_interactive_model_provider_call_count(2)
        .await
        .expect("truncation receives exactly one recovery call");
    harness
        .assert_text_model_provider_call_count(0)
        .await
        .expect("output truncation must not consume a context-compaction attempt");
    harness
        .assert_conversation_history_lacks("partial response that must not be reported as complete")
        .await
        .expect("partial provider output is never durably reported as a successful reply");
}

#[tokio::test]
async fn transcript_write_failure_stops_without_another_model_or_tool_side_effect() {
    let harness = RebornIntegrationHarness::test_default()
        .with_keyed_http_responses([ScriptedHttpResponse::for_url(
            TRANSCRIPT_FAILURE_TOOL_URL,
            UNPERSISTED_TOOL_RESULT,
        )])
        .record_model_calls_for_test()
        .fail_append_finalized_assistant_message_for_test()
        .with_turn_event_sink()
        .script([
            RebornScriptedReply::tool_call(
                "builtin.http",
                json!({"url": TRANSCRIPT_FAILURE_TOOL_URL}),
            ),
            RebornScriptedReply::text(UNPERSISTED_ASSISTANT_REPLY),
        ])
        .build()
        .await
        .expect("harness builds");

    let run_id = harness
        .submit_turn_async("produce one reply")
        .await
        .expect("turn submitted");
    let state = harness
        .wait_for_status(run_id, TurnStatus::Failed)
        .await
        .expect("transcript persistence failure reaches a terminal failed state");
    let failure = state
        .failure
        .as_ref()
        .expect("failed transcript persistence carries a durable failure");

    assert_eq!(
        failure.category(),
        "transcript_write_failed",
        "the transcript cause must survive the executor and runner"
    );
    assert_eq!(
        failure.detail(),
        Some("assistant transcript write failed"),
        "only the bounded host-authored safe cause may be persisted"
    );
    assert!(
        !format!("{failure:?}").contains(TRANSCRIPT_FAILURE_SECRET),
        "the backend secret must not enter durable turn state"
    );
    assert!(
        !format!("{failure:?}").contains(UNPERSISTED_ASSISTANT_REPLY),
        "raw unpersisted assistant content must not enter durable failure state"
    );

    harness
        .assert_conversation_history_lacks(UNPERSISTED_ASSISTANT_REPLY)
        .await
        .expect("no draft or fabricated finalized reply is persisted");
    harness
        .assert_interactive_model_provider_call_count(2)
        .await
        .expect("a failed transcript boundary must not trigger a third model call");
    harness
        .assert_text_model_provider_call_count(0)
        .await
        .expect("a failed transcript boundary must not trigger model inference");
    harness
        .assert_tool_invocation_count("builtin.http", 1)
        .await
        .expect("the prior capability is not repeated after final reply persistence fails");
    harness
        .assert_capability_result_count("builtin.http", 1)
        .await
        .expect("the prior capability effect is not repeated");
    harness
        .assert_egress_count(1)
        .await
        .expect("the prior external side effect is issued exactly once");
    harness
        .assert_model_message_content_occurrences("model error observation", 0)
        .await
        .expect("no model-visible observation is fabricated after persistence fails");
    harness
        .assert_turn_event_recorded(TurnEventKind::Failed)
        .await
        .expect("the terminal transcript failure is published durably");
}

#[tokio::test]
async fn tool_result_transcript_failure_stops_without_duplicate_model_or_tool_side_effect() {
    let harness = RebornIntegrationHarness::test_default()
        .with_keyed_http_responses([ScriptedHttpResponse::for_url(
            TRANSCRIPT_FAILURE_TOOL_URL,
            UNPERSISTED_TOOL_RESULT,
        )])
        .record_model_calls_for_test()
        .fail_append_tool_result_reference_for_test()
        .with_turn_event_sink()
        .script([
            RebornScriptedReply::tool_call(
                "builtin.http",
                json!({"url": TRANSCRIPT_FAILURE_TOOL_URL}),
            ),
            RebornScriptedReply::text("must not be called after transcript persistence fails"),
        ])
        .build()
        .await
        .expect("harness builds");

    let run_id = harness
        .submit_turn_async("use the echo tool once")
        .await
        .expect("turn submitted");
    let state = harness
        .wait_for_status(run_id, TurnStatus::Failed)
        .await
        .expect("tool-result transcript persistence failure reaches a terminal failed state");
    let failure = state
        .failure
        .as_ref()
        .expect("failed tool-result persistence carries a durable failure");

    assert_eq!(failure.category(), "transcript_write_failed");
    assert_eq!(
        failure.detail(),
        Some("assistant transcript write failed"),
        "the terminal projection retains only the fixed host-authored cause"
    );
    let durable_failure = format!("{failure:?}");
    assert!(!durable_failure.contains(TRANSCRIPT_FAILURE_SECRET));
    assert!(!durable_failure.contains(UNPERSISTED_TOOL_RESULT));

    harness
        .assert_conversation_history_lacks(TRANSCRIPT_FAILURE_SECRET)
        .await
        .expect("backend credentials do not enter conversation history");
    harness
        .assert_conversation_history_lacks(UNPERSISTED_TOOL_RESULT)
        .await
        .expect("unpersisted tool output does not enter conversation history");
    harness
        .assert_interactive_model_provider_call_count(1)
        .await
        .expect("tool-result persistence failure must not trigger another model call");
    harness
        .assert_text_model_provider_call_count(0)
        .await
        .expect("tool-result persistence failure must not trigger model inference");
    harness
        .assert_tool_invocation_count("builtin.http", 1)
        .await
        .expect("the capability executes exactly once");
    harness
        .assert_capability_result_count("builtin.http", 1)
        .await
        .expect("the capability effect is not repeated");
    harness
        .assert_egress_count(1)
        .await
        .expect("the external tool effect is issued exactly once");
    harness
        .assert_model_message_content_occurrences("model error observation", 0)
        .await
        .expect("no model-visible observation is fabricated after persistence fails");
    harness
        .assert_turn_event_recorded(TurnEventKind::Failed)
        .await
        .expect("the terminal transcript failure is published durably");
}
