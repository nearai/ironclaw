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

use ironclaw_loop_contracts::LoopRecoveryClass;
use ironclaw_threads::SessionThreadError;
use ironclaw_turns::{TurnEventKind, TurnStatus};
use reborn_support::builder::{RebornIntegrationHarness, StorageMode};
use reborn_support::http_matcher::ScriptedHttpResponse;
use reborn_support::reply::RebornScriptedReply;
use reborn_support::scripted_provider::{
    CONTEXT_OVERFLOW_USED_TOKENS, ModelProviderCallProbe, RecoverableModelFailure,
};
use serde_json::json;
use std::num::NonZeroU32;

const UNPERSISTED_ASSISTANT_REPLY: &str =
    "raw assistant transcript that must never be reported as a reply";
const UNPERSISTED_TOOL_RESULT: &str =
    "raw tool result that must never enter the transcript failure";
const TRANSCRIPT_FAILURE_TOOL_URL: &str = "https://transcript-failure.example.test/result";

#[test]
fn call_probe_distinguishes_missing_and_leaking_boundary_traffic() {
    let probe = ModelProviderCallProbe::default();
    assert_eq!(probe.text_message_content_contains("secret"), None);
    assert_eq!(
        probe.post_text_interactive_message_content_contains("secret"),
        None
    );

    probe.record_text_contents_for_test(&["safe input"]);
    assert_eq!(probe.text_message_content_contains("secret"), Some(false));
    assert_eq!(
        probe.post_text_interactive_message_content_contains("secret"),
        None
    );

    probe.record_interactive_contents_for_test(&["leaked secret"]);
    probe.record_interactive_contents_for_test(&["later clean request"]);
    assert_eq!(
        probe.post_text_interactive_message_content_contains("secret"),
        Some(true),
        "a later clean retry must not hide an earlier post-compaction leak"
    );

    let clean_probe = ModelProviderCallProbe::default();
    clean_probe.record_text_contents_for_test(&["safe input"]);
    clean_probe.record_interactive_contents_for_test(&["safe request"]);
    assert_eq!(
        clean_probe.post_text_interactive_message_content_contains("secret"),
        Some(false)
    );
}

#[test]
fn transcript_backend_error_classification_is_detail_free() {
    let error = SessionThreadError::Backend("storage credential sk-secret".to_string());

    assert_eq!(error.kind_name(), "backend");
    assert!(!error.kind_name().contains("sk-secret"));
}

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
async fn long_tool_run_keeps_the_original_task_after_raw_history_exceeds_window_limit() {
    const ORIGINAL_TASK: &str = "retain this exact original task through the long tool run";
    let mut script = Vec::new();
    for index in 0..130 {
        if index == 128 {
            script.push(RebornScriptedReply::text(format!(
                "durable window compaction summary before iteration {index}"
            )));
        }
        script.push(RebornScriptedReply::tool_call(
            "test_echo",
            json!({"message": format!("iteration {index}")}),
        ));
    }
    script.push(RebornScriptedReply::text("long tool run complete"));
    let harness = RebornIntegrationHarness::test_default()
        .with_iteration_limit_for_test(NonZeroU32::new(140).expect("non-zero test limit"))
        .script(script)
        .build()
        .await
        .expect("harness builds");

    harness
        .submit_turn(ORIGINAL_TASK)
        .await
        .expect("long tool run completes");
    harness
        .assert_conversation_history_message_count_at_least(132)
        .await
        .expect("durable tool results exceed the 128-message window");
    harness
        .assert_summary_artifact_count_at_least(1)
        .await
        .expect("window eviction produces a durable compaction summary");
    harness
        .assert_last_model_message_content_contains(ORIGINAL_TASK)
        .await
        .expect("the final interactive request still carries the accepted task");
    harness
        .assert_reply_contains("long tool run complete")
        .await
        .expect("the final reply persists");
}

#[tokio::test]
async fn context_overflow_recovers_with_model_visible_observation() {
    // Seed one oversized user message so forced compaction exercises the real
    // compactor instead of taking its safe "nothing eligible" skip path.
    let input_secret = concat!("AKIA", "IOSFODNN7EXAMPLE");
    let second_input_secret = concat!("ghp_", "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx");
    let first_setup_turn =
        format!("first setup turn credentials {input_secret} and {second_input_secret}");
    let oversized_setup_turn = format!("third setup turn {}", "history ".repeat(5_000));
    let output_secret = "OUTPUT_PRIVATE_KEY_MATERIAL";
    let compacted_summary = format!(
        "compacted recovery history\n-----BEGIN ENCRYPTED PRIVATE KEY-----\n{output_secret}\n-----END ENCRYPTED PRIVATE KEY-----\nretained"
    );
    let harness = RebornIntegrationHarness::test_default()
        .context_overflow_model_after(3, 3)
        .script([
            RebornScriptedReply::text("first setup reply"),
            RebornScriptedReply::text("second setup reply"),
            RebornScriptedReply::text("third setup reply"),
            RebornScriptedReply::text(compacted_summary),
            RebornScriptedReply::text("recovered after context overflow"),
        ])
        .build()
        .await
        .expect("harness builds");
    harness
        .submit_turn(&first_setup_turn)
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
    let before_recovery_milestones = harness
        .milestone_len()
        .await
        .expect("milestone len readable");
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
        .assert_text_model_message_content_not_contains(input_secret)
        .await
        .expect("compaction inference input redacts the transcript secret");
    harness
        .assert_text_model_message_content_not_contains(second_input_secret)
        .await
        .expect("compaction inference input redacts every transcript secret");
    harness
        .assert_text_model_message_content_contains("[REDACTED]")
        .await
        .expect("compaction inference input carries the deterministic redaction marker");
    harness
        .assert_post_compaction_interactive_model_message_content_not_contains(input_secret)
        .await
        .expect("the next interactive request does not rehydrate the transcript secret");
    harness
        .assert_post_compaction_interactive_model_message_content_not_contains(second_input_secret)
        .await
        .expect("the next interactive request does not rehydrate any transcript secret");
    harness
        .assert_post_compaction_interactive_model_message_content_not_contains(output_secret)
        .await
        .expect("the next interactive request does not rehydrate model-output key material");
    harness
        .assert_compaction_redacted_once_since(before_recovery_milestones, 3)
        .await
        .expect("input and output redactions produce one typed aggregate milestone");
    harness
        .assert_summary_artifacts_lack(input_secret)
        .await
        .expect("the durable compaction summary does not persist the transcript secret");
    harness
        .assert_summary_artifacts_lack(second_input_secret)
        .await
        .expect("the durable compaction summary omits every transcript secret");
    harness
        .assert_summary_artifacts_lack(output_secret)
        .await
        .expect("the durable compaction summary omits model-output key material");
    harness
        .assert_summary_artifact_contains("[REDACTED]")
        .await
        .expect("the durable compaction summary contains only redaction markers");
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
async fn summary_exclusion_rejects_missing_durable_artifacts() {
    let harness = RebornIntegrationHarness::test_default()
        .script([RebornScriptedReply::text("ordinary reply")])
        .build()
        .await
        .expect("harness builds");
    harness
        .submit_turn("ordinary turn without compaction")
        .await
        .expect("turn establishes a durable thread without a summary");

    let error = harness
        .assert_summary_artifacts_lack("synthetic secret")
        .await
        .expect_err("an empty artifact set must not prove exclusion");

    assert_eq!(
        error.to_string(),
        "vacuous exclusion: zero durable summary artifacts persisted"
    );
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

#[tokio::test]
async fn transcript_write_failure_stops_without_another_model_or_tool_side_effect() {
    // This debug integration future exceeds libtest's default 2 MiB thread
    // stack; box it on the heap. CI sets no `RUST_MIN_STACK`, and assertions
    // remain unchanged in the helper below.
    Box::pin(transcript_write_failure_stops_without_another_model_or_tool_side_effect_body()).await;
}

async fn transcript_write_failure_stops_without_another_model_or_tool_side_effect_body() {
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
    harness
        .assert_transcript_failure_terminal(&state, UNPERSISTED_ASSISTANT_REPLY, 2)
        .await
        .expect("assistant transcript failure stays terminal, redacted, and single-shot");
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
    harness
        .assert_transcript_failure_terminal(&state, UNPERSISTED_TOOL_RESULT, 1)
        .await
        .expect("tool-result transcript failure stays terminal, redacted, and single-shot");
}
