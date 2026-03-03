//! E2E trace test: memory write flow.
//!
//! Validates that the agent can execute `memory_write` tool calls driven by
//! a TraceLlm trace, with a real workspace backed by libSQL.

#[cfg(feature = "libsql")]
mod support;

#[cfg(feature = "libsql")]
mod tests {
    use std::time::Duration;

    use crate::support::assertions::assert_all_tools_succeeded;
    use crate::support::test_rig::TestRigBuilder;
    use crate::support::trace_llm::LlmTrace;

    /// The agent writes a note to workspace memory via `memory_write`, then
    /// responds with a confirmation message.
    #[tokio::test]
    async fn test_memory_write_flow() {
        // 1. Load trace fixture.
        let trace = LlmTrace::from_file(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/llm_traces/memory_write_read.json"
        ))
        .expect("failed to load memory_write_read.json trace fixture");

        // 2. Build the rig with workspace enabled so memory tools are registered.
        let rig = TestRigBuilder::new()
            .with_trace(trace)
            .with_workspace(true)
            .build()
            .await;

        // 3. Send a message that triggers the memory_write tool call.
        rig.send_message("Please remember that Project Alpha launches on March 15th")
            .await;

        // 4. Wait for a response (up to 15 seconds).
        let responses = rig.wait_for_responses(1, Duration::from_secs(15)).await;

        // 5. Verify we got a non-empty response.
        assert!(
            !responses.is_empty(),
            "Expected at least one response from the agent"
        );
        assert!(
            !responses[0].content.is_empty(),
            "Expected a non-empty response"
        );

        // 6. Verify the memory_write tool was called.
        let started = rig.tool_calls_started();
        assert!(
            started.contains(&"memory_write".to_string()),
            "Expected memory_write in tool_calls_started, got: {:?}",
            started
        );

        // 7. Verify all tools completed successfully.
        let completed = rig.tool_calls_completed();
        assert_all_tools_succeeded(&completed);

        // 8. Shutdown.
        rig.shutdown();
    }
}
