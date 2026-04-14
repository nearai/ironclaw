//! Regression test for web SSE ordering.
//!
//! The assistant response must be emitted before the terminal `Done` status
//! so the browser can render the message before the turn closes.

#[cfg(feature = "libsql")]
mod support;

#[cfg(feature = "libsql")]
mod response_order_tests {
    use std::sync::Arc;
    use std::time::Duration;

    use async_trait::async_trait;

    use crate::support::test_channel::CapturedEvent;
    use crate::support::test_rig::TestRigBuilder;
    use crate::support::trace_llm::{LlmTrace, TraceResponse, TraceStep, TraceToolCall, TraceTurn};
    use ironclaw::channels::StatusUpdate;
    use ironclaw::context::JobContext;
    use ironclaw::tools::{ApprovalRequirement, Tool, ToolError, ToolOutput};

    const TIMEOUT: Duration = Duration::from_secs(15);

    /// Tool that always requires explicit approval. Used to drive the v1
    /// `SubmissionResult::NeedApproval` path so we can verify the agent does
    /// **not** emit a terminal `Done` while the turn is paused awaiting input.
    struct AlwaysApproveTool;

    #[async_trait]
    impl Tool for AlwaysApproveTool {
        fn name(&self) -> &str {
            "always_approve_probe"
        }

        fn description(&self) -> &str {
            "Test tool that always requires explicit approval"
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
            _params: serde_json::Value,
            _ctx: &JobContext,
        ) -> Result<ToolOutput, ToolError> {
            // Should never run during this test — the turn pauses on approval.
            Ok(ToolOutput::success(
                serde_json::json!({"ok": true}),
                Duration::from_millis(1),
            ))
        }

        fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
            ApprovalRequirement::Always
        }
    }

    fn single_response_trace() -> LlmTrace {
        LlmTrace::new(
            "trace-order-test",
            vec![TraceTurn {
                user_input: "Say hello".to_string(),
                steps: vec![TraceStep {
                    request_hint: None,
                    response: TraceResponse::Text {
                        content: "Hello there".to_string(),
                        input_tokens: 1,
                        output_tokens: 1,
                    },
                    expected_tool_results: Vec::new(),
                }],
                expects: Default::default(),
            }],
        )
    }

    #[tokio::test]
    async fn response_arrives_before_done_status() {
        let rig = TestRigBuilder::new()
            .with_trace(single_response_trace())
            .build()
            .await;
        rig.clear().await;

        rig.send_message("Say hello").await;
        let responses = rig.wait_for_responses(1, TIMEOUT).await;
        assert_eq!(responses.len(), 1);
        assert_eq!(responses[0].content, "Hello there");

        let events = rig.captured_events();
        let response_index = events
            .iter()
            .position(|event| matches!(event, CapturedEvent::Response(_)))
            .expect("response event not captured");
        let done_index = events
            .iter()
            .position(|event| matches!(event, CapturedEvent::Status(StatusUpdate::Status(message)) if message == "Done"))
            .expect("Done status not captured");

        assert!(
            response_index < done_index,
            "response must be emitted before Done"
        );

        rig.shutdown();
    }

    /// When the LLM returns an empty Text response, the dispatcher's fallback
    /// substitutes "I'm not sure how to respond to that." (see
    /// `src/llm/reasoning.rs`). The agent must still send the substituted
    /// response **before** the terminal `Done` status — same ordering invariant
    /// as the happy path, just with the fallback text.
    #[tokio::test]
    async fn done_emitted_after_empty_response_fallback() {
        let empty_response_trace = LlmTrace::new(
            "trace-empty-response",
            vec![TraceTurn {
                user_input: "Do nothing".to_string(),
                steps: vec![TraceStep {
                    request_hint: None,
                    response: TraceResponse::Text {
                        content: String::new(),
                        input_tokens: 1,
                        output_tokens: 0,
                    },
                    expected_tool_results: Vec::new(),
                }],
                expects: Default::default(),
            }],
        );

        let rig = TestRigBuilder::new()
            .with_trace(empty_response_trace)
            .build()
            .await;
        rig.clear().await;

        rig.send_message("Do nothing").await;

        // The dispatcher substitutes an empty LLM response with a fallback
        // message, so the Response event is still captured.
        let responses = rig.wait_for_responses(1, TIMEOUT).await;
        assert_eq!(responses.len(), 1);
        assert_eq!(
            responses[0].content, "I'm not sure how to respond to that.",
            "empty LLM response should be replaced with the dispatcher fallback"
        );

        assert!(
            rig.wait_for_done(TIMEOUT).await,
            "Done status must be emitted after the fallback response"
        );

        // Verify the fallback Response is emitted before Done.
        let events = rig.captured_events();
        let response_index = events
            .iter()
            .position(|event| matches!(event, CapturedEvent::Response(_)))
            .expect("fallback response event not captured");
        let done_index = events
            .iter()
            .position(|event| matches!(event, CapturedEvent::Status(StatusUpdate::Status(message)) if message == "Done"))
            .expect("Done status not captured");
        assert!(
            response_index < done_index,
            "fallback response must be emitted before Done"
        );

        rig.shutdown();
    }

    /// When a tool requires user approval, the agent must emit `ApprovalNeeded`
    /// but **not** a terminal `Done` — the thread is paused, not complete.
    /// Sending `Done` while awaiting approval would trip the web UI's
    /// missing-response safety net (see #2079) and trigger a spurious history
    /// reload underneath the live approval prompt.
    #[tokio::test]
    async fn no_done_emitted_while_awaiting_approval() {
        let approval_trace = LlmTrace::new(
            "trace-approval-pending",
            vec![TraceTurn {
                user_input: "Run the probe".to_string(),
                steps: vec![TraceStep {
                    request_hint: None,
                    response: TraceResponse::ToolCalls {
                        tool_calls: vec![TraceToolCall {
                            id: "call_1".to_string(),
                            name: "always_approve_probe".to_string(),
                            arguments: serde_json::json!({"value": "go"}),
                        }],
                        input_tokens: 1,
                        output_tokens: 1,
                    },
                    expected_tool_results: Vec::new(),
                }],
                expects: Default::default(),
            }],
        );

        let rig = TestRigBuilder::new()
            .with_trace(approval_trace)
            .with_extra_tools(vec![Arc::new(AlwaysApproveTool)])
            // Disable session-level auto-approve so the approval is actually requested.
            .with_auto_approve_tools(false)
            .build()
            .await;
        rig.clear().await;

        rig.send_message("Run the probe").await;

        // Wait for the ApprovalNeeded status — that proves the turn reached
        // the pause point. We poll the captured status events directly because
        // ApprovalNeeded is not exposed via a dedicated waiter.
        let deadline = tokio::time::Instant::now() + TIMEOUT;
        let approval_seen = loop {
            let seen = rig
                .captured_status_events()
                .iter()
                .any(|s| matches!(s, StatusUpdate::ApprovalNeeded { .. }));
            if seen {
                break true;
            }
            if tokio::time::Instant::now() >= deadline {
                break false;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        };
        assert!(
            approval_seen,
            "ApprovalNeeded status must be emitted for the always-approve tool"
        );

        // Give the agent a small window after the approval to (incorrectly)
        // emit a trailing Done. The bug we are guarding against happens
        // immediately after handle_message returns, so 200ms is generous.
        tokio::time::sleep(Duration::from_millis(200)).await;

        let events = rig.captured_events();

        // No assistant Response should have been delivered — the tool never
        // executed, so the LLM never produced a final text response.
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, CapturedEvent::Response(_))),
            "no Response event should be emitted while awaiting approval, got: {events:?}"
        );

        // The critical assertion: no terminal Done while the turn is paused.
        let done_count = events
            .iter()
            .filter(|e| matches!(e, CapturedEvent::Status(StatusUpdate::Status(message)) if message == "Done"))
            .count();
        assert_eq!(
            done_count, 0,
            "Done must not be emitted while awaiting approval"
        );

        rig.shutdown();
    }
}
