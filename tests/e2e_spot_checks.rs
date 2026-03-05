//! E2E spot-check tests adapted from nearai/benchmarks SpotSuite tasks.jsonl.
//!
//! Each test replays an LLM trace through the real agent loop and validates
//! the result using declarative `expects` from the fixture JSON plus any
//! additional assertions that can't be expressed declaratively.

#[cfg(feature = "libsql")]
mod support;

#[cfg(feature = "libsql")]
mod spot_tests {
    use std::sync::Arc;
    use std::time::Duration;

    use ironclaw::tools::ToolRegistry;

    use crate::support::cleanup::CleanupGuard;
    use crate::support::test_rig::TestRigBuilder;
    use crate::support::trace_llm::LlmTrace;

    const FIXTURES: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/llm_traces/spot"
    );
    const TIMEOUT: Duration = Duration::from_secs(15);

    /// Build a ToolRegistry with both builtin and dev (file) tools.
    fn tools_with_file_support() -> Arc<ToolRegistry> {
        let registry = Arc::new(ToolRegistry::new());
        registry.register_builtin_tools();
        registry.register_dev_tools();
        registry
    }

    // -----------------------------------------------------------------------
    // Smoke tests -- no tools expected
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn spot_smoke_greeting() {
        let trace = LlmTrace::from_file(format!("{FIXTURES}/smoke_greeting.json")).unwrap();
        let rig = TestRigBuilder::new().with_trace(trace.clone()).build().await;

        rig.send_message("Hello! Introduce yourself briefly.").await;
        let responses = rig.wait_for_responses(1, TIMEOUT).await;

        rig.verify_trace_expects(&trace, &responses);
        rig.shutdown();
    }

    #[tokio::test]
    async fn spot_smoke_math() {
        let trace = LlmTrace::from_file(format!("{FIXTURES}/smoke_math.json")).unwrap();
        let rig = TestRigBuilder::new().with_trace(trace.clone()).build().await;

        rig.send_message("What is 47 * 23? Reply with just the number.")
            .await;
        let responses = rig.wait_for_responses(1, TIMEOUT).await;

        rig.verify_trace_expects(&trace, &responses);
        rig.shutdown();
    }

    // -----------------------------------------------------------------------
    // Tool tests -- verify correct tool selection
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn spot_tool_echo() {
        let trace = LlmTrace::from_file(format!("{FIXTURES}/tool_echo.json")).unwrap();
        let rig = TestRigBuilder::new().with_trace(trace.clone()).build().await;

        rig.send_message("Use the echo tool to repeat the message: 'Spot check passed'")
            .await;
        let responses = rig.wait_for_responses(1, TIMEOUT).await;

        rig.verify_trace_expects(&trace, &responses);
        rig.shutdown();
    }

    #[tokio::test]
    async fn spot_tool_json() {
        let trace = LlmTrace::from_file(format!("{FIXTURES}/tool_json.json")).unwrap();
        let rig = TestRigBuilder::new().with_trace(trace.clone()).build().await;

        rig.send_message("Parse this json for me: {\"key\": \"value\"}")
            .await;
        let responses = rig.wait_for_responses(1, TIMEOUT).await;

        rig.verify_trace_expects(&trace, &responses);
        rig.shutdown();
    }

    // -----------------------------------------------------------------------
    // Chain tests -- multi-tool sequences
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn spot_chain_write_read() {
        let _cleanup = CleanupGuard::new().file("/tmp/ironclaw_spot_test.txt");
        let _ = std::fs::remove_file("/tmp/ironclaw_spot_test.txt");

        let trace = LlmTrace::from_file(format!("{FIXTURES}/chain_write_read.json")).unwrap();
        let rig = TestRigBuilder::new()
            .with_trace(trace.clone())
            .with_tools(tools_with_file_support())
            .build()
            .await;

        rig.send_message(
            "Write the text 'ironclaw spot check' to /tmp/ironclaw_spot_test.txt \
             using the write_file tool, then read it back using read_file.",
        )
        .await;
        let responses = rig.wait_for_responses(1, TIMEOUT).await;

        rig.verify_trace_expects(&trace, &responses);

        // Extra: verify file on disk (can't express in expects).
        let content =
            std::fs::read_to_string("/tmp/ironclaw_spot_test.txt").expect("file should exist");
        assert_eq!(content, "ironclaw spot check");

        rig.shutdown();
    }

    // -----------------------------------------------------------------------
    // Robustness tests -- correct behavior under constraints
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn spot_robust_no_tool() {
        let trace = LlmTrace::from_file(format!("{FIXTURES}/robust_no_tool.json")).unwrap();
        let rig = TestRigBuilder::new().with_trace(trace.clone()).build().await;

        rig.send_message("What is the capital of France? Answer directly without using any tools.")
            .await;
        let responses = rig.wait_for_responses(1, TIMEOUT).await;

        rig.verify_trace_expects(&trace, &responses);
        rig.shutdown();
    }

    #[tokio::test]
    async fn spot_robust_correct_tool() {
        let trace = LlmTrace::from_file(format!("{FIXTURES}/robust_correct_tool.json")).unwrap();
        let rig = TestRigBuilder::new().with_trace(trace.clone()).build().await;

        rig.send_message("Please echo the word 'deterministic output'")
            .await;
        let responses = rig.wait_for_responses(1, TIMEOUT).await;

        rig.verify_trace_expects(&trace, &responses);
        rig.shutdown();
    }

    // -----------------------------------------------------------------------
    // Memory tests -- save and recall via file tools
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn spot_memory_save_recall() {
        let _cleanup = CleanupGuard::new().file("/tmp/bench-meeting.md");
        let _ = std::fs::remove_file("/tmp/bench-meeting.md");

        let trace = LlmTrace::from_file(format!("{FIXTURES}/memory_save_recall.json")).unwrap();
        let rig = TestRigBuilder::new()
            .with_trace(trace.clone())
            .with_tools(tools_with_file_support())
            .build()
            .await;

        rig.send_message(
            "Save these meeting notes to /tmp/bench-meeting.md:\n\
             Meeting: Project Phoenix sync\nAttendees: Alice, Bob, Carol\n\
             Decisions:\n- Launch date: April 15th\n- Budget: $50k approved\n\
             - Bob owns frontend, Carol owns backend\n\
             Then read it back and tell me who owns the frontend and what the launch date is.",
        )
        .await;
        let responses = rig.wait_for_responses(1, TIMEOUT).await;

        rig.verify_trace_expects(&trace, &responses);
        rig.shutdown();
    }
}
