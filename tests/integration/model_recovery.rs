//! IronClaw integration tests for typed model-error recovery observations.
//!
//! These scenarios cross the production turn scheduler, canonical loop,
//! provider gateway, checkpoint, compaction, and transcript seams.

#[allow(dead_code)]
#[path = "support/mod.rs"]
mod ironclaw_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

use ironclaw_support::builder::IronClawIntegrationHarness;
use ironclaw_support::reply::IronClawScriptedReply;
use ironclaw_support::scripted_provider::CONTEXT_OVERFLOW_USED_TOKENS;

#[tokio::test]
async fn content_filtered_completion_recovers_with_model_visible_observation() {
    let harness = IronClawIntegrationHarness::test_default()
        .content_filter_model_once()
        .script([IronClawScriptedReply::text(
            "recovered after content filter",
        )])
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
    let harness = IronClawIntegrationHarness::test_default()
        .context_overflow_model_after(3, 3)
        .script([
            IronClawScriptedReply::text("first setup reply"),
            IronClawScriptedReply::text("second setup reply"),
            IronClawScriptedReply::text("third setup reply"),
            IronClawScriptedReply::text("compacted recovery history"),
            IronClawScriptedReply::text("recovered after context overflow"),
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
    let harness = IronClawIntegrationHarness::test_default()
        .invalid_output_model_times(3)
        .script([IronClawScriptedReply::text(
            "recovered after invalid output",
        )])
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
