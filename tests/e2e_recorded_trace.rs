//! E2E test for traces produced by `RecordingLlm`.
//!
//! Validates that recorded traces (with `user_input` steps, `memory_snapshot`,
//! and `expected_tool_results`) can be loaded and replayed through the agent loop.

#[cfg(feature = "libsql")]
mod support;

#[cfg(feature = "libsql")]
mod recorded_trace_tests {
    use std::time::Duration;

    use crate::support::assertions::*;
    use crate::support::test_rig::TestRigBuilder;
    use crate::support::trace_llm::LlmTrace;

    const FIXTURES: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/llm_traces/recorded"
    );
    const TIMEOUT: Duration = Duration::from_secs(15);

    /// Recorded trace: telegram connection check.
    ///
    /// This trace was adapted from a live recording session. It tests that:
    /// 1. `user_input` steps are skipped by TraceLlm (not consumed as LLM calls)
    /// 2. `memory_snapshot` is deserialized (not yet restored into workspace)
    /// 3. `expected_tool_results` is deserialized and available for future verification
    /// 4. The agent loop replays tool_calls → text correctly
    #[tokio::test]
    async fn recorded_telegram_check() {
        let trace = LlmTrace::from_file(format!("{FIXTURES}/telegram_check.json")).unwrap();

        // Verify the extended format fields loaded correctly.
        assert_eq!(trace.memory_snapshot.len(), 1);
        assert_eq!(trace.memory_snapshot[0].path, "IDENTITY.md");

        // The trace has 3 total steps: 1 user_input + 2 playable (tool_calls + text).
        assert_eq!(trace.steps.len(), 3);
        assert_eq!(trace.playable_steps().len(), 2);

        // Verify expected_tool_results on the final step.
        let playable = trace.playable_steps();
        let last_playable = playable.last().unwrap();
        assert_eq!(last_playable.expected_tool_results.len(), 1);
        assert_eq!(last_playable.expected_tool_results[0].name, "echo");

        let rig = TestRigBuilder::new().with_trace(trace).build().await;

        rig.send_message("is telegram connected?").await;
        let responses = rig.wait_for_responses(1, TIMEOUT).await;

        assert!(!responses.is_empty(), "no response");
        assert_response_contains(&responses[0].content, &["Telegram", "connected"]);

        let started = rig.tool_calls_started();
        assert_tools_used(&started, &["echo"]);

        let completed = rig.tool_calls_completed();
        assert_all_tools_succeeded(&completed);

        // Verify the echo tool result came through.
        let results = rig.tool_results();
        let echo_result = results.iter().find(|(n, _)| n == "echo");
        assert!(
            echo_result.is_some(),
            "Expected echo tool result, got: {results:?}"
        );

        rig.shutdown();
    }

    /// Verify that a recorded trace with only user_input steps and no playable
    /// steps still deserializes correctly.
    #[test]
    fn recorded_trace_all_user_input() {
        let json = r#"{
            "model_name": "recorded-all-user-input",
            "memory_snapshot": [],
            "steps": [
                { "response": { "type": "user_input", "content": "hello" } },
                { "response": { "type": "user_input", "content": "world" } }
            ]
        }"#;
        let trace: LlmTrace = serde_json::from_str(json).unwrap();
        assert_eq!(trace.steps.len(), 2);
        assert_eq!(trace.playable_steps().len(), 0);
    }

    /// Verify backward compatibility: a trace without the new fields
    /// still loads correctly (memory_snapshot, http_exchanges default to empty).
    #[test]
    fn recorded_trace_backward_compat() {
        let json = r#"{
            "model_name": "old-format",
            "steps": [
                {
                    "response": {
                        "type": "text",
                        "content": "hello",
                        "input_tokens": 10,
                        "output_tokens": 5
                    }
                }
            ]
        }"#;
        let trace: LlmTrace = serde_json::from_str(json).unwrap();
        assert!(trace.memory_snapshot.is_empty());
        assert!(trace.http_exchanges.is_empty());
        assert_eq!(trace.playable_steps().len(), 1);
    }
}
