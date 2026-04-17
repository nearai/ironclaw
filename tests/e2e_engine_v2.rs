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
    use async_trait::async_trait;
    use std::sync::OnceLock;
    use std::time::Duration;

    use tokio::sync::Mutex;

    use crate::support::test_rig::TestRigBuilder;
    use crate::support::trace_llm::{LlmTrace, TraceResponse, TraceStep, TraceToolCall};
    use ironclaw::context::JobContext;
    use ironclaw::tools::{ApprovalRequirement, Tool, ToolError, ToolOutput};

    fn engine_v2_test_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

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

    struct ApprovalProbeTool;

    #[async_trait]
    impl Tool for ApprovalProbeTool {
        fn name(&self) -> &str {
            "approval_probe"
        }

        fn description(&self) -> &str {
            "Test tool that should be auto-approved in engine v2"
        }

        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "value": { "type": "string" }
                },
                "required": ["value"]
            })
        }

        async fn execute(
            &self,
            params: serde_json::Value,
            _ctx: &JobContext,
        ) -> Result<ToolOutput, ToolError> {
            Ok(ToolOutput::success(
                serde_json::json!({
                    "ok": true,
                    "echo": params.get("value").cloned().unwrap_or(serde_json::Value::Null),
                }),
                Duration::from_millis(1),
            ))
        }

        fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
            ApprovalRequirement::UnlessAutoApproved
        }
    }

    // -----------------------------------------------------------------------
    // Phase 1: Core scenarios — prove the v2 path works
    // -----------------------------------------------------------------------

    /// Smoke test: simple text response, no tools.
    /// Verifies that messages route through the engine v2 pipeline and a
    /// response arrives via the TestChannel.
    #[tokio::test]
    async fn v2_smoke_text_response() {
        let _guard = engine_v2_test_lock().lock().await;
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
        let _guard = engine_v2_test_lock().lock().await;
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
        let _guard = engine_v2_test_lock().lock().await;
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
        let _guard = engine_v2_test_lock().lock().await;
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
        let _guard = engine_v2_test_lock().lock().await;
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
        let _guard = engine_v2_test_lock().lock().await;
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

    /// Regression: engine v2 must honor the global auto-approve setting for
    /// `UnlessAutoApproved` tools, matching the legacy dispatcher.
    #[tokio::test]
    async fn v2_honors_global_auto_approve_for_unless_auto_approved_tools() {
        let _guard = engine_v2_test_lock().lock().await;
        let trace = LlmTrace::single_turn(
            "test-v2-auto-approve",
            "Run the approval probe tool",
            vec![
                TraceStep {
                    request_hint: None,
                    response: TraceResponse::ToolCalls {
                        tool_calls: vec![TraceToolCall {
                            id: "call_approval_probe_1".into(),
                            name: "approval_probe".into(),
                            arguments: serde_json::json!({ "value": "engine-v2" }),
                        }],
                        input_tokens: 10,
                        output_tokens: 5,
                    },
                    expected_tool_results: Vec::new(),
                },
                TraceStep {
                    request_hint: None,
                    response: TraceResponse::Text {
                        content: "approval probe completed".into(),
                        input_tokens: 10,
                        output_tokens: 5,
                    },
                    expected_tool_results: Vec::new(),
                },
            ],
        );
        let rig = TestRigBuilder::new()
            .with_engine_v2()
            .with_auto_approve_tools(true)
            .with_trace(trace)
            .with_extra_tools(vec![std::sync::Arc::new(ApprovalProbeTool)])
            .build()
            .await;

        rig.send_message("Run the approval probe tool").await;
        let responses = rig.wait_for_responses(1, Duration::from_secs(5)).await;

        assert_eq!(
            responses.len(),
            1,
            "expected a final response, got {responses:?}"
        );
        assert!(
            responses[0].content.contains("approval probe completed"),
            "unexpected response: {:?}",
            responses[0]
        );
        assert_v2_tool_used(&rig.tool_calls_started(), "approval_probe");
        assert!(
            !rig.captured_status_events().iter().any(|status| {
                matches!(
                    status,
                    ironclaw::channels::StatusUpdate::ApprovalNeeded { .. }
                )
            }),
            "engine v2 should not emit ApprovalNeeded when global auto-approve is enabled"
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
        let _guard = engine_v2_test_lock().lock().await;
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
        let _guard = engine_v2_test_lock().lock().await;
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

    /// Execution obligation: user says "run the echo tool", model first responds
    /// with a false capability refusal (text only), obligation nudge fires, then
    /// the model makes the tool call on the second attempt.
    #[tokio::test]
    async fn v2_execution_obligation_nudge_fires() {
        let _guard = engine_v2_test_lock().lock().await;
        let trace =
            LlmTrace::from_file(format!("{FIXTURES}/execution_obligation_nudge.json")).unwrap();
        let rig = TestRigBuilder::new()
            .with_engine_v2()
            .with_trace(trace.clone())
            .build()
            .await;

        // "run the echo tool" triggers user_signals_execution_intent → require_action_attempt
        rig.send_message("run the echo tool with 'obligation echo test'")
            .await;
        let responses = rig.wait_for_responses(1, TIMEOUT).await;

        rig.verify_trace_expects(&trace, &responses);
        assert_v2_tool_used(&rig.tool_calls_started(), "echo");

        // Verify the nudge message was injected (LLM was called at least twice:
        // once for the text refusal, once after the nudge)
        let llm_requests = rig.captured_llm_requests();
        assert!(
            llm_requests.len() >= 2,
            "expected at least 2 LLM calls (refusal + post-nudge), got {}",
            llm_requests.len()
        );

        rig.shutdown();
    }

    /// No obligation nudge on conversational messages that don't signal
    /// execution intent. The model responds with plain text and it's accepted.
    #[tokio::test]
    async fn v2_execution_obligation_no_nudge_on_conversational() {
        let _guard = engine_v2_test_lock().lock().await;
        let trace =
            LlmTrace::from_file(format!("{FIXTURES}/execution_obligation_no_nudge.json")).unwrap();
        let rig = TestRigBuilder::new()
            .with_engine_v2()
            .with_trace(trace.clone())
            .build()
            .await;

        // "What's the weather?" has no execution intent → no obligation
        rig.send_message("What's the weather like today?").await;
        let responses = rig.wait_for_responses(1, TIMEOUT).await;

        rig.verify_trace_expects(&trace, &responses);

        // Only 1 LLM call — no nudge injected
        let llm_requests = rig.captured_llm_requests();
        assert_eq!(
            llm_requests.len(),
            1,
            "expected exactly 1 LLM call (no nudge), got {}",
            llm_requests.len()
        );

        rig.shutdown();
    }

    /// Execution obligation exhaustion: model refuses to call tools on every
    /// attempt, hitting max_action_requirement_nudges. The final text response
    /// is accepted as completed (the feature terminates, no infinite loop).
    #[tokio::test]
    async fn v2_execution_obligation_exhaustion_terminates() {
        let _guard = engine_v2_test_lock().lock().await;
        let trace = LlmTrace::from_file(format!("{FIXTURES}/execution_obligation_exhaustion.json"))
            .unwrap();
        let rig = TestRigBuilder::new()
            .with_engine_v2()
            .with_trace(trace.clone())
            .build()
            .await;

        // "run the echo tool" triggers obligation, but model refuses every time
        rig.send_message("run the echo tool please").await;
        let responses = rig.wait_for_responses(1, TIMEOUT).await;

        // Should get a response (not hang or error)
        assert!(
            !responses.is_empty(),
            "should get a response even after nudge exhaustion"
        );

        // LLM called 3 times: initial refusal + 2 nudges (max_action_requirement_nudges=2)
        let llm_requests = rig.captured_llm_requests();
        assert_eq!(
            llm_requests.len(),
            3,
            "expected 3 LLM calls (1 refusal + 2 nudges), got {}",
            llm_requests.len()
        );

        // No tools were called
        assert!(
            rig.tool_calls_started().is_empty(),
            "no tools should have been called"
        );

        rig.shutdown();
    }

    /// Execution obligation on multi-turn: first message is conversational (no
    /// obligation), second message says "run the echo tool" and obligation fires.
    /// The test rig processes messages sequentially, so turn 2 spawns a new
    /// thread (the spawn path, where ThreadConfig.require_action_attempt is set
    /// by the router). The inject and resume paths are tested separately in
    /// engine_v2_gate_integration.rs (gate_resume_with_execution_obligation).
    #[tokio::test]
    async fn v2_execution_obligation_multi_turn() {
        let _guard = engine_v2_test_lock().lock().await;
        let trace =
            LlmTrace::from_file(format!("{FIXTURES}/execution_obligation_followup.json")).unwrap();
        let rig = TestRigBuilder::new()
            .with_engine_v2()
            .with_trace(trace.clone())
            .build()
            .await;

        let all_responses = rig.run_and_verify_trace(&trace, TIMEOUT).await;

        // Turn 1: conversational, no tools
        assert!(
            !all_responses[0].is_empty(),
            "turn 1 should produce a response"
        );

        // Turn 2: obligation should have fired — echo tool was used
        assert_v2_tool_used(&rig.tool_calls_started(), "echo");

        // The nudge should have been injected (at least 2 LLM calls for turn 2:
        // text refusal + post-nudge tool call). Turn 1 had 1 LLM call.
        let llm_requests = rig.captured_llm_requests();
        assert!(
            llm_requests.len() >= 3,
            "expected at least 3 LLM calls (1 for turn 1 + 2+ for turn 2 with nudge), got {}",
            llm_requests.len()
        );

        rig.shutdown();
    }
}
