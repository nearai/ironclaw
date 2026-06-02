//! Live E2E tests for provider reasoning summary preservation.
//!
//! Verifies that reasoning summaries from the configured LLM provider
//! (OpenAI Codex, Anthropic, etc.) flow through the full stack and surface
//! as `StatusUpdate::Thinking` events in the channel.
//!
//! # Running
//!
//! ```bash
//! # Live — uses ~/.ironclaw/.env, real API call:
//! IRONCLAW_LIVE_TEST=1 cargo test --features libsql --test e2e_live_reasoning -- --ignored --nocapture
//!
//! # Replay — deterministic, no API keys (after a trace has been recorded):
//! cargo test --features libsql --test e2e_live_reasoning -- --ignored
//! ```

#[cfg(feature = "libsql")]
mod support;

#[cfg(feature = "libsql")]
mod live_tests {
    use std::time::Duration;

    use ironclaw::channels::StatusUpdate;

    use crate::support::live_harness::{LiveTestHarnessBuilder, TestMode};

    /// Ask a question that requires multi-step reasoning.
    /// Asserts that at least one `StatusUpdate::Thinking` event was emitted —
    /// confirming that the provider's reasoning summary reached the channel.
    #[tokio::test]
    #[ignore] // Live tier: requires LLM API keys + network access
    async fn provider_reasoning_surfaces_as_thinking_status() {
        let harness = LiveTestHarnessBuilder::new("provider_reasoning_surfaces_as_thinking_status")
            .with_engine_v2(true)
            .with_auto_approve_tools(false)
            .build()
            .await;

        // Live-only: replay fixtures don't carry StatusUpdate events,
        // so we can't assert on Thinking in replay mode.
        if harness.mode() != TestMode::Live {
            eprintln!(
                "[ReasoningE2E] Live-only test — skipping outside IRONCLAW_LIVE_TEST=1. \
                 StatusUpdate::Thinking events are not recorded in trace fixtures."
            );
            return;
        }

        let rig = harness.rig();

        // A problem that reliably triggers extended reasoning on o-series / Claude.
        rig.send_message(
            "Think carefully: if I have a 3x3 grid and I place tokens alternately, \
             starting with X in the top-left and going row by row, what is the \
             final state of the grid after all 9 cells are filled? Show the grid.",
        )
        .await;

        let responses = rig.wait_for_responses(1, Duration::from_secs(60)).await;

        assert!(
            !responses.is_empty(),
            "Expected at least one response from the model"
        );

        // Check that at least one Thinking status update was emitted.
        let status_events = rig.captured_status_events();
        let thinking_events: Vec<&str> = status_events
            .iter()
            .filter_map(|s| {
                if let StatusUpdate::Thinking(msg) = s {
                    Some(msg.as_str())
                } else {
                    None
                }
            })
            .collect();

        assert!(
            !thinking_events.is_empty(),
            "Expected at least one Thinking status update from the provider's reasoning summary, \
             but got none.\n\
             Status events received: {:?}\n\
             Final response: {:?}",
            status_events
                .iter()
                .map(|s| format!("{s:?}"))
                .collect::<Vec<_>>(),
            responses.iter().map(|r| &r.content).collect::<Vec<_>>(),
        );

        eprintln!(
            "[ReasoningE2E] ✓ {} thinking event(s) emitted. First: {:?}",
            thinking_events.len(),
            thinking_events.first().map(|s| &s[..s.len().min(120)])
        );
    }

    /// Verify reasoning does NOT surface for a model that doesn't support it.
    ///
    /// This test requires manually overriding LLM_MODEL to a non-reasoning model
    /// (e.g. gpt-4o) via env. Skipped by default — run with:
    ///
    /// ```bash
    /// IRONCLAW_LIVE_TEST=1 LLM_MODEL=gpt-4o \
    ///   cargo test --features libsql --test e2e_live_reasoning \
    ///   -- non_reasoning_model_emits_no_thinking --ignored --nocapture
    /// ```
    #[tokio::test]
    #[ignore] // Live tier: requires LLM_MODEL set to a non-reasoning model
    async fn non_reasoning_model_emits_no_thinking() {
        let model = std::env::var("LLM_MODEL").unwrap_or_default();
        if model.is_empty() {
            eprintln!(
                "[ReasoningE2E] Set LLM_MODEL=gpt-4o (or similar) to run this test. Skipping."
            );
            return;
        }

        let harness = LiveTestHarnessBuilder::new("non_reasoning_model_emits_no_thinking")
            .with_engine_v2(true)
            .build()
            .await;

        if harness.mode() != TestMode::Live {
            return;
        }

        let rig = harness.rig();
        rig.send_message("What is 2 + 2?").await;
        let _ = rig.wait_for_responses(1, Duration::from_secs(30)).await;

        let thinking_events: Vec<_> = rig
            .captured_status_events()
            .into_iter()
            .filter(|s| matches!(s, StatusUpdate::Thinking(_)))
            .collect();

        assert!(
            thinking_events.is_empty(),
            "Non-reasoning model {model:?} should not emit Thinking events, \
             but got: {thinking_events:?}"
        );

        eprintln!("[ReasoningE2E] ✓ No Thinking events for non-reasoning model {model:?}");
    }
}
