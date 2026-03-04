//! E2E spot-check tests adapted from nearai/benchmarks SpotSuite tasks.jsonl.
//!
//! Each test replays an LLM trace through the real agent loop and validates
//! the result using the same assertion types as the benchmarks repo:
//! `response_contains`, `tools_used`, `tools_not_used`, `max_tool_calls`,
//! `response_matches`.

#[cfg(feature = "libsql")]
mod support;

#[cfg(feature = "libsql")]
mod spot_tests {
    use std::sync::Arc;
    use std::time::Duration;

    use ironclaw::tools::ToolRegistry;

    use crate::support::assertions::*;
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

    /// Spot: smoke-greeting
    /// Prompt: "Hello! Introduce yourself briefly."
    /// Assertions: response_matches: (?i)(hello|hi|hey|assistant|agent|help), max_tool_calls: 0
    #[tokio::test]
    async fn spot_smoke_greeting() {
        let trace = LlmTrace::from_file(format!("{FIXTURES}/smoke_greeting.json")).unwrap();
        let rig = TestRigBuilder::new().with_trace(trace).build().await;

        rig.send_message("Hello! Introduce yourself briefly.").await;
        let responses = rig.wait_for_responses(1, TIMEOUT).await;

        assert!(!responses.is_empty(), "no response");
        let text = &responses[0].content;
        assert_response_matches(text, "(?i)(hello|hi|hey|assistant|agent|help)");
        assert_max_tool_calls(&rig.tool_calls_started(), 0);

        rig.shutdown();
    }

    /// Spot: smoke-math
    /// Prompt: "What is 47 * 23? Reply with just the number."
    /// Assertions: response_contains: ["1081"], max_tool_calls: 0
    #[tokio::test]
    async fn spot_smoke_math() {
        let trace = LlmTrace::from_file(format!("{FIXTURES}/smoke_math.json")).unwrap();
        let rig = TestRigBuilder::new().with_trace(trace).build().await;

        rig.send_message("What is 47 * 23? Reply with just the number.")
            .await;
        let responses = rig.wait_for_responses(1, TIMEOUT).await;

        assert!(!responses.is_empty(), "no response");
        assert_response_contains(&responses[0].content, &["1081"]);
        assert_max_tool_calls(&rig.tool_calls_started(), 0);

        rig.shutdown();
    }

    // -----------------------------------------------------------------------
    // Tool tests -- verify correct tool selection
    // -----------------------------------------------------------------------

    /// Spot: tool-echo
    /// Prompt: "Use the echo tool to repeat: 'Spot check passed'"
    /// Assertions: tools_used: [echo], response_contains: ["Spot check passed"]
    #[tokio::test]
    async fn spot_tool_echo() {
        let trace = LlmTrace::from_file(format!("{FIXTURES}/tool_echo.json")).unwrap();
        let rig = TestRigBuilder::new().with_trace(trace).build().await;

        rig.send_message("Use the echo tool to repeat the message: 'Spot check passed'")
            .await;
        let responses = rig.wait_for_responses(1, TIMEOUT).await;

        assert!(!responses.is_empty(), "no response");
        let started = rig.tool_calls_started();
        assert_tools_used(&started, &["echo"]);
        assert_response_contains(&responses[0].content, &["Spot check passed"]);

        let completed = rig.tool_calls_completed();
        assert_all_tools_succeeded(&completed);

        rig.shutdown();
    }

    /// Spot: tool-json
    /// Prompt: "Parse this JSON for me: {\"key\": \"value\"}"
    /// Assertions: tools_used: [json], response_contains: ["key", "value"]
    #[tokio::test]
    async fn spot_tool_json() {
        let trace = LlmTrace::from_file(format!("{FIXTURES}/tool_json.json")).unwrap();
        let rig = TestRigBuilder::new().with_trace(trace).build().await;

        rig.send_message("Parse this json for me: {\"key\": \"value\"}")
            .await;
        let responses = rig.wait_for_responses(1, TIMEOUT).await;

        assert!(!responses.is_empty(), "no response");
        let started = rig.tool_calls_started();
        assert_tools_used(&started, &["json"]);
        assert_response_contains(&responses[0].content, &["key", "value"]);

        let completed = rig.tool_calls_completed();
        assert_all_tools_succeeded(&completed);

        rig.shutdown();
    }

    // -----------------------------------------------------------------------
    // Chain tests -- multi-tool sequences
    // -----------------------------------------------------------------------

    /// Spot: chain-write-read
    /// Prompt: Write text to file, read it back.
    /// Assertions: tools_used: [write_file, read_file], response_contains: ["ironclaw spot check"]
    #[tokio::test]
    async fn spot_chain_write_read() {
        let _cleanup = CleanupGuard::new().file("/tmp/ironclaw_spot_test.txt");
        // Clean up from any previous run.
        let _ = std::fs::remove_file("/tmp/ironclaw_spot_test.txt");

        let trace = LlmTrace::from_file(format!("{FIXTURES}/chain_write_read.json")).unwrap();
        let rig = TestRigBuilder::new()
            .with_trace(trace)
            .with_tools(tools_with_file_support())
            .build()
            .await;

        rig.send_message(
            "Write the text 'ironclaw spot check' to /tmp/ironclaw_spot_test.txt \
             using the write_file tool, then read it back using read_file.",
        )
        .await;
        let responses = rig.wait_for_responses(1, TIMEOUT).await;

        assert!(!responses.is_empty(), "no response");
        let started = rig.tool_calls_started();
        assert_tools_used(&started, &["write_file", "read_file"]);
        assert_response_contains(&responses[0].content, &["ironclaw spot check"]);

        let completed = rig.tool_calls_completed();
        assert_all_tools_succeeded(&completed);

        let results = rig.tool_results();
        let read_result = results.iter().find(|(n, _)| n == "read_file");
        assert!(
            read_result.is_some() && read_result.unwrap().1.contains("ironclaw spot check"),
            "Expected read_file result to contain 'ironclaw spot check'"
        );

        // Verify file on disk.
        let content =
            std::fs::read_to_string("/tmp/ironclaw_spot_test.txt").expect("file should exist");
        assert_eq!(content, "ironclaw spot check");

        rig.shutdown();
    }

    // -----------------------------------------------------------------------
    // Robustness tests -- correct behavior under constraints
    // -----------------------------------------------------------------------

    /// Spot: robust-no-tool
    /// Prompt: "What is the capital of France? Answer directly without using any tools."
    /// Assertions: response_contains: ["Paris"], max_tool_calls: 0
    #[tokio::test]
    async fn spot_robust_no_tool() {
        let trace = LlmTrace::from_file(format!("{FIXTURES}/robust_no_tool.json")).unwrap();
        let rig = TestRigBuilder::new().with_trace(trace).build().await;

        rig.send_message("What is the capital of France? Answer directly without using any tools.")
            .await;
        let responses = rig.wait_for_responses(1, TIMEOUT).await;

        assert!(!responses.is_empty(), "no response");
        assert_response_contains(&responses[0].content, &["Paris"]);
        assert_max_tool_calls(&rig.tool_calls_started(), 0);

        rig.shutdown();
    }

    /// Spot: robust-correct-tool
    /// Prompt: "Please echo the word 'deterministic output'"
    /// Assertions: tools_used: [echo], tools_not_used: [shell, time]
    #[tokio::test]
    async fn spot_robust_correct_tool() {
        let trace = LlmTrace::from_file(format!("{FIXTURES}/robust_correct_tool.json")).unwrap();
        let rig = TestRigBuilder::new().with_trace(trace).build().await;

        rig.send_message("Please echo the word 'deterministic output'")
            .await;
        let responses = rig.wait_for_responses(1, TIMEOUT).await;

        assert!(!responses.is_empty(), "no response");
        let started = rig.tool_calls_started();
        assert_tools_used(&started, &["echo"]);
        assert_tools_not_used(&started, &["shell", "time"]);

        let completed = rig.tool_calls_completed();
        assert_all_tools_succeeded(&completed);

        rig.shutdown();
    }

    // -----------------------------------------------------------------------
    // Memory tests -- save and recall via file tools
    // -----------------------------------------------------------------------

    /// Spot: memory-save-meeting (adapted)
    /// Prompt: Save meeting notes, read back, answer questions.
    /// Assertions: tools_used: [write_file, read_file], response_contains: ["Bob", "frontend", "April 15"]
    #[tokio::test]
    async fn spot_memory_save_recall() {
        let _cleanup = CleanupGuard::new().file("/tmp/bench-meeting.md");
        let _ = std::fs::remove_file("/tmp/bench-meeting.md");

        let trace = LlmTrace::from_file(format!("{FIXTURES}/memory_save_recall.json")).unwrap();
        let rig = TestRigBuilder::new()
            .with_trace(trace)
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

        assert!(!responses.is_empty(), "no response");
        let started = rig.tool_calls_started();
        assert_tools_used(&started, &["write_file", "read_file"]);
        assert_response_contains(&responses[0].content, &["Bob", "frontend", "April 15"]);

        let completed = rig.tool_calls_completed();
        assert_all_tools_succeeded(&completed);

        rig.shutdown();
    }
}
