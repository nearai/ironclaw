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

use ironclaw_turns::run_profile::LoopRecoveryClass;
use ironclaw_turns::{TurnEventKind, TurnStatus};
use reborn_support::builder::{RebornIntegrationHarness, StorageMode};
use reborn_support::reply::RebornScriptedReply;
use reborn_support::scripted_provider::{CONTEXT_OVERFLOW_USED_TOKENS, RecoverableModelFailure};

#[tokio::test]
async fn provider_outage_advances_real_fallback_chain_and_persists_reply() {
    let harness = RebornIntegrationHarness::test_default()
        .storage(StorageMode::LibSql)
        .with_turn_event_sink()
        .advance_fallback_after_unavailable()
        .script([RebornScriptedReply::text("fallback recovered the turn")])
        .build()
        .await
        .expect("harness builds");

    harness
        .submit_turn("recover through the configured provider fallback")
        .await
        .expect("whole turn completes through fallback index one");
    harness
        .assert_ordered_fallback_vendor_calls()
        .await
        .expect("primary is called once and fallback index one is authoritative");
    harness
        .assert_reply_persists_after_reopen("fallback recovered the turn")
        .await
        .expect("final fallback reply survives a fresh libSQL connection");
    harness
        .assert_turn_event_recorded(TurnEventKind::Completed)
        .await
        .expect("the recovered turn emits its durable completed event");
    harness
        .assert_no_turn_event_recorded(TurnEventKind::Failed)
        .await
        .expect("the recoverable primary outage emits no false terminal failure");
}

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

/// Regression for #6897: malformed, JSON-invalid, and empty completed provider
/// responses use the bounded invalid-output lane, then persist a durable
/// user-visible failure category and scrubbed provider cause.
#[tokio::test]
async fn deterministic_provider_response_errors_use_bounded_invalid_output_recovery() {
    let cases = [
        (RecoverableModelFailure::ProviderJson, "JSON error:", "json"),
        (
            RecoverableModelFailure::ProviderInvalidResponse,
            "malformed response envelope",
            "invalid_response",
        ),
        (
            RecoverableModelFailure::ProviderEmptyResponse,
            "Empty response",
            "empty_response",
        ),
    ];

    for (provider_failure, expected_detail, label) in cases {
        let harness = RebornIntegrationHarness::test_default()
            .with_turn_event_sink()
            .provider_response_error_model_times(provider_failure, usize::MAX)
            .build()
            .await
            .expect("harness builds");
        let run_id = harness
            .submit_turn_async("return a usable response")
            .await
            .expect("turn submitted");
        let state = harness
            .wait_for_status(run_id, TurnStatus::Failed)
            .await
            .expect("deterministic provider failure reaches Failed");
        let failure = state
            .failure
            .as_ref()
            .expect("failed run carries a durable failure");

        assert_eq!(
            failure.category(),
            "model_invalid_output",
            "{label} must not use the provider-unavailable category"
        );
        assert!(
            failure
                .detail()
                .is_some_and(|detail| detail.contains(expected_detail)),
            "{label} must retain its scrubbed provider cause: {failure:?}"
        );
        harness
            .assert_interactive_model_provider_call_count(4)
            .await
            .expect("invalid output uses two retries, one observation, then aborts");
        harness
            .assert_model_recovery_class(
                LoopRecoveryClass::ModelInvalidOutput,
                LoopRecoveryClass::ModelUnavailable,
            )
            .await
            .expect("recovery remains in the bounded invalid-output lane");
        harness
            .assert_failed_turn_event("model_invalid_output", expected_detail)
            .await
            .expect("durable failed event carries user-visible category and safe detail");
        harness
            .assert_no_turn_event_recorded(TurnEventKind::Completed)
            .await
            .expect("terminal invalid output emits no false completion");
    }
}

/// Regression for #6700: a provider's output-token ceiling is not an input
/// context overflow. The real gateway caller must preserve that distinction,
/// skip context compaction, and give the model an actionable finalization turn.
#[tokio::test]
async fn output_truncation_recovers_without_shrinking_input_context() {
    let harness = RebornIntegrationHarness::test_default()
        .with_builtin_http_tools()
        .with_budget_accounting()
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
    harness
        .assert_tool_not_invoked("builtin.http")
        .await
        .expect("a truncated textual tool call must never reach capability dispatch");
    harness
        .assert_egress_count(0)
        .await
        .expect("a truncated textual tool call must never reach HTTP egress");
    harness
        .assert_budget_spent_tokens(11, 7)
        .await
        .expect("truncated provider usage must still be durably charged");
}
