//! Engine v2 acceptance tests.
//!
//! These tests replay LLM traces through the engine v2 pipeline (via
//! `TestRigBuilder::with_engine_v2()`) to prove tool dispatch, conversation
//! continuity, error handling, and status events work correctly.
//!
//! The v2 engine routes through `src/bridge/router.rs` → `ironclaw_engine`
//! instead of the v1 agentic loop in `src/agent/dispatcher.rs`.

#[cfg(feature = "libsql")]
mod support;

#[cfg(feature = "libsql")]
mod engine_v2_tests {
    use std::time::Duration;

    use crate::support::test_rig::TestRigBuilder;
    use crate::support::trace_llm::LlmTrace;

    /// Check that a tool name appears in the started list.
    /// Engine v2 formats tool names as `"name(param_summary)"`, so we match
    /// by prefix rather than exact equality.
    fn assert_v2_tool_used(started: &[String], tool: &str) {
        assert!(
            started
                .iter()
                .any(|s| s == tool || s.starts_with(&format!("{tool}("))),
            "v2 tools_used: \"{tool}\" not called, got: {started:?}"
        );
    }

    const FIXTURES: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/llm_traces/engine_v2"
    );
    const TIMEOUT: Duration = Duration::from_secs(15);

    // -----------------------------------------------------------------------
    // Phase 1: Core scenarios — prove the v2 path works
    // -----------------------------------------------------------------------

    /// Smoke test: simple text response, no tools.
    /// Verifies that messages route through the engine v2 pipeline and a
    /// response arrives via the TestChannel.
    #[tokio::test]
    async fn v2_smoke_text_response() {
        let trace = LlmTrace::from_file(format!("{FIXTURES}/smoke_text.json")).unwrap();
        let rig = TestRigBuilder::new()
            .with_engine_v2()
            .with_trace(trace.clone())
            .build()
            .await;

        rig.send_message("Hello! Introduce yourself briefly.").await;
        let responses = rig.wait_for_responses(1, TIMEOUT).await;

        rig.verify_trace_expects(&trace, &responses);
        assert!(
            !responses.is_empty(),
            "v2 engine should produce at least one response"
        );
        rig.shutdown();
    }

    /// Single tool call: echo tool → tool result → text response.
    /// Verifies that EffectBridgeAdapter dispatches tool calls and results
    /// flow back through the engine thread.
    #[tokio::test]
    async fn v2_single_tool_call() {
        let trace = LlmTrace::from_file(format!("{FIXTURES}/single_tool_echo.json")).unwrap();
        let rig = TestRigBuilder::new()
            .with_engine_v2()
            .with_trace(trace.clone())
            .build()
            .await;

        rig.send_message("Use the echo tool to repeat: 'V2 echo test'")
            .await;
        let responses = rig.wait_for_responses(1, TIMEOUT).await;

        rig.verify_trace_expects(&trace, &responses);
        assert_v2_tool_used(&rig.tool_calls_started(), "echo");
        rig.shutdown();
    }

    /// Multi-tool chain: echo + time → sequential calls → text.
    /// Verifies that multiple tool invocations work in a single engine thread.
    #[tokio::test]
    async fn v2_multi_tool_chain() {
        let trace = LlmTrace::from_file(format!("{FIXTURES}/multi_tool_chain.json")).unwrap();
        let rig = TestRigBuilder::new()
            .with_engine_v2()
            .with_trace(trace.clone())
            .build()
            .await;

        rig.send_message("Use the echo tool to say 'chain step 1', then check the time.")
            .await;
        let responses = rig.wait_for_responses(1, TIMEOUT).await;

        rig.verify_trace_expects(&trace, &responses);
        let tools = rig.tool_calls_started();
        assert_v2_tool_used(&tools, "echo");
        assert_v2_tool_used(&tools, "time");
        rig.shutdown();
    }

    /// Tool error recovery: tool returns error → LLM acknowledges gracefully.
    /// Verifies that error propagation through the engine thread works.
    #[tokio::test]
    async fn v2_tool_error_recovery() {
        let trace = LlmTrace::from_file(format!("{FIXTURES}/tool_error_recovery.json")).unwrap();
        let rig = TestRigBuilder::new()
            .with_engine_v2()
            .with_trace(trace.clone())
            .build()
            .await;

        rig.send_message("Parse this json for me: not valid json {")
            .await;
        let responses = rig.wait_for_responses(1, TIMEOUT).await;

        rig.verify_trace_expects(&trace, &responses);
        rig.shutdown();
    }

    /// Multi-turn conversation: second turn references context from first.
    /// Verifies that ConversationManager preserves context across turns.
    #[tokio::test]
    async fn v2_multi_turn_conversation() {
        let trace = LlmTrace::from_file(format!("{FIXTURES}/multi_turn.json")).unwrap();
        let rig = TestRigBuilder::new()
            .with_engine_v2()
            .with_trace(trace.clone())
            .build()
            .await;

        rig.run_and_verify_trace(&trace, Duration::from_secs(30))
            .await;
        rig.shutdown();
    }

    /// Status events: verify that tool calls produce ToolStarted/ToolCompleted events.
    #[tokio::test]
    async fn v2_status_events() {
        let trace = LlmTrace::from_file(format!("{FIXTURES}/single_tool_echo.json")).unwrap();
        let rig = TestRigBuilder::new()
            .with_engine_v2()
            .with_trace(trace)
            .build()
            .await;

        rig.send_message("Use the echo tool to repeat: 'V2 echo test'")
            .await;
        let _ = rig.wait_for_responses(1, TIMEOUT).await;

        let started = rig.tool_calls_started();
        let completed = rig.tool_calls_completed();
        assert!(!started.is_empty(), "should have ToolStarted status events");
        assert!(
            !completed.is_empty(),
            "should have ToolCompleted status events"
        );
        rig.shutdown();
    }

    // -----------------------------------------------------------------------
    // Phase 2: Replay existing v1 recorded traces through v2
    // -----------------------------------------------------------------------

    /// V1 parity: replay the telegram_check recorded trace through engine v2.
    /// Uses manual assertions because the v1 fixture's `expects` uses exact
    /// tool names, but v2 formats them as `"name(param_summary)"`.
    #[tokio::test]
    async fn v2_recorded_telegram_check() {
        let path = format!(
            "{}/tests/fixtures/llm_traces/recorded/telegram_check.json",
            env!("CARGO_MANIFEST_DIR")
        );
        let trace = LlmTrace::from_file(&path).unwrap();
        let rig = TestRigBuilder::new()
            .with_engine_v2()
            .with_trace(trace)
            .build()
            .await;

        rig.send_message("check telegram connection").await;
        let responses = rig.wait_for_responses(1, Duration::from_secs(30)).await;

        assert!(!responses.is_empty(), "should get a response");
        // The telegram_check trace exercises tool_list — verify it was called.
        assert_v2_tool_used(&rig.tool_calls_started(), "tool_list");
        // Response should mention Telegram connectivity.
        let combined: String = responses.iter().map(|r| r.content.clone()).collect();
        assert!(
            combined.to_lowercase().contains("telegram"),
            "response should mention Telegram, got: {combined}"
        );
        rig.shutdown();
    }

    /// V1 parity: replay the weather_sf recorded trace through engine v2.
    /// Exercises the HTTP tool with a large response.
    #[tokio::test]
    async fn v2_recorded_weather_sf() {
        let path = format!(
            "{}/tests/fixtures/llm_traces/recorded/weather_sf.json",
            env!("CARGO_MANIFEST_DIR")
        );
        let trace = LlmTrace::from_file(&path).unwrap();
        let rig = TestRigBuilder::new()
            .with_engine_v2()
            .with_trace(trace)
            .build()
            .await;

        rig.send_message("check weather in SF today").await;
        let responses = rig.wait_for_responses(1, Duration::from_secs(30)).await;

        assert!(!responses.is_empty(), "should get a response");
        assert_v2_tool_used(&rig.tool_calls_started(), "http");
        rig.shutdown();
    }
}
