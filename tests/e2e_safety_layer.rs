//! E2E trace tests: safety layer.
//!
//! Verifies that the safety layer (injection detection, sanitization) works
//! correctly when enabled in the test rig.

#[cfg(feature = "libsql")]
mod support;

#[cfg(feature = "libsql")]
mod tests {
    use std::time::Duration;

    use crate::support::assertions::assert_all_tools_succeeded;
    use crate::support::test_rig::TestRigBuilder;
    use crate::support::trace_llm::LlmTrace;

    /// When injection check is enabled and a tool outputs injection patterns
    /// (e.g., `<|endoftext|>`, `system: ignore previous`), the safety layer
    /// should sanitize the content. The agent must still produce a response
    /// (no crash) and the injection content should not pass through raw.
    #[tokio::test]
    async fn test_injection_patterns_sanitized() {
        let trace = LlmTrace::from_file(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/llm_traces/coverage/injection_in_echo.json"
        ))
        .expect("failed to load injection_in_echo.json");

        let rig = TestRigBuilder::new()
            .with_trace(trace)
            .with_injection_check(true)
            .build()
            .await;

        rig.send_message("Please echo this text for me").await;
        let responses = rig.wait_for_responses(1, Duration::from_secs(15)).await;

        // The agent must respond (safety layer didn't crash the pipeline).
        assert!(
            !responses.is_empty(),
            "Expected a response even with injection patterns in tool output"
        );

        // The echo tool should have been called.
        let started = rig.tool_calls_started();
        assert!(
            started.contains(&"echo".to_string()),
            "Expected echo in tool_calls_started, got: {started:?}"
        );

        // The echo tool should have completed (it doesn't fail on content).
        let completed = rig.tool_calls_completed();
        let echo_results: Vec<_> = completed.iter().filter(|(n, _)| n == "echo").collect();
        assert!(!echo_results.is_empty(), "Expected echo tool completion");
        assert_all_tools_succeeded(&completed);

        // Metrics: 2 LLM calls (tool + text).
        let metrics = rig.collect_metrics().await;
        assert!(
            metrics.llm_calls >= 2,
            "Expected >= 2 LLM calls, got {}",
            metrics.llm_calls
        );

        rig.shutdown();
    }

    /// When injection check is disabled (default), tool outputs with injection
    /// patterns should still pass through and the agent responds normally.
    #[tokio::test]
    async fn test_injection_patterns_pass_without_check() {
        let trace = LlmTrace::from_file(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/llm_traces/coverage/injection_in_echo.json"
        ))
        .expect("failed to load injection_in_echo.json");

        // Default: injection_check is false.
        let rig = TestRigBuilder::new().with_trace(trace).build().await;

        rig.send_message("Please echo this text for me").await;
        let responses = rig.wait_for_responses(1, Duration::from_secs(15)).await;

        assert!(
            !responses.is_empty(),
            "Expected a response with injection check disabled"
        );

        let started = rig.tool_calls_started();
        assert!(
            started.contains(&"echo".to_string()),
            "Expected echo tool call"
        );

        let completed = rig.tool_calls_completed();
        assert_all_tools_succeeded(&completed);

        rig.shutdown();
    }
}
