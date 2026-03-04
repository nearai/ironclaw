//! E2E trace test: tool error path.
//!
//! Validates that the agent handles tool errors gracefully (no crash)
//! when a tool call is made with missing required parameters.

#[cfg(feature = "libsql")]
mod support;

#[cfg(feature = "libsql")]
mod tests {
    use std::time::Duration;

    use crate::support::test_rig::TestRigBuilder;
    use crate::support::trace_llm::LlmTrace;

    /// When the LLM calls `read_file` with missing required parameters, the tool
    /// returns an error.  The agent must handle this gracefully -- it should not
    /// panic -- and eventually produce a text response.
    #[tokio::test]
    async fn test_tool_error_handled_gracefully() {
        // 1. Load trace fixture.
        let trace = LlmTrace::from_file(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/llm_traces/error_path.json"
        ))
        .expect("failed to load error_path.json trace fixture");

        // 2. Build the test rig with the trace.
        let rig = TestRigBuilder::new().with_trace(trace).build().await;

        // 3. Send a message that will trigger the read_file tool call.
        rig.send_message("Read a file for me").await;

        // 4. Wait for a response (up to 15 seconds).
        let responses = rig.wait_for_responses(1, Duration::from_secs(15)).await;

        // 5. The agent must have responded (it didn't crash).
        assert!(
            !responses.is_empty(),
            "Expected at least one response from the agent after a tool error"
        );

        // 6. The tool call should have been attempted.
        let started = rig.tool_calls_started();
        assert!(
            started.contains(&"read_file".to_string()),
            "Expected read_file in tool_calls_started, got: {:?}",
            started
        );

        // 7. Shutdown the rig.
        rig.shutdown();
    }
}
