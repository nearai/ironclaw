//! E2E trace test: validates that the agent can execute `write_file` and
//! `read_file` tool calls driven by a TraceLlm trace.

#[cfg(feature = "libsql")]
mod support;

#[cfg(feature = "libsql")]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use ironclaw::tools::ToolRegistry;

    use crate::support::assertions::assert_all_tools_succeeded;
    use crate::support::test_rig::TestRigBuilder;
    use crate::support::trace_llm::LlmTrace;

    const TEST_DIR: &str = "/tmp/ironclaw_e2e_test";
    const TEST_FILE: &str = "/tmp/ironclaw_e2e_test/hello.txt";
    const EXPECTED_CONTENT: &str = "Hello, E2E test!";

    /// Clean the temp directory if it exists, then recreate it.
    fn setup_test_dir() {
        let _ = std::fs::remove_dir_all(TEST_DIR);
        std::fs::create_dir_all(TEST_DIR).expect("failed to create test directory");
    }

    /// Remove the temp directory.
    fn cleanup_test_dir() {
        let _ = std::fs::remove_dir_all(TEST_DIR);
    }

    /// Build a `ToolRegistry` that includes both built-in and dev (file) tools.
    fn tools_with_file_support() -> Arc<ToolRegistry> {
        let registry = Arc::new(ToolRegistry::new());
        registry.register_builtin_tools();
        registry.register_dev_tools();
        registry
    }

    #[tokio::test]
    async fn test_file_write_and_read_flow() {
        // 1. Prepare a clean temp directory.
        setup_test_dir();

        // 2. Load the LLM trace fixture.
        let fixture_path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/llm_traces/file_write_read.json"
        );
        let trace = LlmTrace::from_file(fixture_path).expect("failed to load trace fixture");

        // 3. Build the rig with file tools registered.
        let rig = TestRigBuilder::new()
            .with_trace(trace)
            .with_tools(tools_with_file_support())
            .build()
            .await;

        // 4. Send a message that triggers the write_file tool call.
        rig.send_message("Please write a greeting to a file and read it back.")
            .await;

        // 5. Wait for the final text response (15s timeout).
        let responses = rig.wait_for_responses(1, Duration::from_secs(15)).await;

        // 6. Assert we got a non-empty response.
        assert!(
            !responses.is_empty(),
            "Expected at least one response from the agent"
        );
        let final_text = &responses[0].content;
        assert!(
            !final_text.is_empty(),
            "Expected a non-empty response, got empty string"
        );

        // 7. Assert the file exists on disk with the expected content.
        let file_content =
            std::fs::read_to_string(TEST_FILE).expect("hello.txt should exist after write_file");
        assert_eq!(
            file_content, EXPECTED_CONTENT,
            "File content mismatch: expected {:?}, got {:?}",
            EXPECTED_CONTENT, file_content
        );

        // 8. Assert both tool calls were observed.
        let started = rig.tool_calls_started();
        assert!(
            started.contains(&"write_file".to_string()),
            "Expected write_file in tool_calls_started, got: {:?}",
            started
        );
        assert!(
            started.contains(&"read_file".to_string()),
            "Expected read_file in tool_calls_started, got: {:?}",
            started
        );

        // 8b. Assert all tools completed successfully.
        let completed = rig.tool_calls_completed();
        assert_all_tools_succeeded(&completed);

        // 9. Clean up.
        cleanup_test_dir();

        // 10. Shutdown the agent.
        rig.shutdown();
    }
}
