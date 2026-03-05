//! Unit tests for E2E test support modules.
//!
//! These tests live here (instead of inside `support/*.rs`) so they compile
//! and run exactly once, rather than being duplicated across every `e2e_*.rs`
//! test binary that declares `mod support;`.

mod support;

// ---------------------------------------------------------------------------
// assertions
// ---------------------------------------------------------------------------

mod assertions_tests {
    use crate::support::assertions::*;

    #[test]
    fn all_tools_succeeded_passes_when_all_true() {
        let completed = vec![("echo".to_string(), true), ("time".to_string(), true)];
        assert_all_tools_succeeded(&completed);
    }

    #[test]
    fn all_tools_succeeded_passes_on_empty() {
        assert_all_tools_succeeded(&[]);
    }

    #[test]
    #[should_panic(expected = "Expected all tools to succeed")]
    fn all_tools_succeeded_panics_on_failure() {
        let completed = vec![("echo".to_string(), true), ("shell".to_string(), false)];
        assert_all_tools_succeeded(&completed);
    }

    #[test]
    fn tool_succeeded_passes_when_present_and_true() {
        let completed = vec![("echo".to_string(), true), ("time".to_string(), false)];
        assert_tool_succeeded(&completed, "echo");
    }

    #[test]
    #[should_panic(expected = "Expected 'echo' to complete successfully")]
    fn tool_succeeded_panics_when_tool_missing() {
        let completed = vec![("time".to_string(), true)];
        assert_tool_succeeded(&completed, "echo");
    }

    #[test]
    #[should_panic(expected = "Expected 'shell' to complete successfully")]
    fn tool_succeeded_panics_when_tool_failed() {
        let completed = vec![("shell".to_string(), false)];
        assert_tool_succeeded(&completed, "shell");
    }
}

// ---------------------------------------------------------------------------
// cleanup
// ---------------------------------------------------------------------------

mod cleanup_tests {
    use crate::support::cleanup::CleanupGuard;

    #[test]
    fn cleanup_guard_removes_file() {
        let path = "/tmp/ironclaw_cleanup_guard_test.txt";
        std::fs::write(path, "test").unwrap();
        {
            let _guard = CleanupGuard::new().file(path);
            assert!(std::path::Path::new(path).exists());
        }
        assert!(!std::path::Path::new(path).exists());
    }

    #[test]
    fn cleanup_guard_removes_dir() {
        let dir = "/tmp/ironclaw_cleanup_guard_test_dir";
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(format!("{dir}/file.txt"), "test").unwrap();
        {
            let _guard = CleanupGuard::new().dir(dir);
            assert!(std::path::Path::new(dir).exists());
        }
        assert!(!std::path::Path::new(dir).exists());
    }
}

// ---------------------------------------------------------------------------
// test_channel
// ---------------------------------------------------------------------------

mod test_channel_tests {
    use std::sync::Arc;
    use std::time::Duration;

    use crate::support::test_channel::TestChannel;
    use ironclaw::channels::{Channel, IncomingMessage, OutgoingResponse, StatusUpdate};

    #[tokio::test]
    async fn send_and_receive_message() {
        let channel = TestChannel::new();
        let mut stream = channel.start().await.unwrap();

        channel.send_message("hello world").await;

        use futures::StreamExt;
        let msg = stream.next().await.expect("stream should yield a message");
        assert_eq!(msg.content, "hello world");
        assert_eq!(msg.channel, "test");
        assert_eq!(msg.user_id, "test-user");
    }

    #[tokio::test]
    async fn captures_responses() {
        let channel = TestChannel::new();
        let incoming = IncomingMessage::new("test", "test-user", "hi");

        channel
            .respond(&incoming, OutgoingResponse::text("reply 1"))
            .await
            .unwrap();
        channel
            .respond(&incoming, OutgoingResponse::text("reply 2"))
            .await
            .unwrap();

        let captured = channel.captured_responses();
        assert_eq!(captured.len(), 2);
        assert_eq!(captured[0].content, "reply 1");
        assert_eq!(captured[1].content, "reply 2");
    }

    #[tokio::test]
    async fn captures_status_events() {
        let channel = TestChannel::new();
        let metadata = serde_json::Value::Null;

        channel
            .send_status(
                StatusUpdate::ToolStarted {
                    name: "echo".to_string(),
                },
                &metadata,
            )
            .await
            .unwrap();
        channel
            .send_status(
                StatusUpdate::ToolCompleted {
                    name: "echo".to_string(),
                    success: true,
                },
                &metadata,
            )
            .await
            .unwrap();

        let events = channel.captured_status_events();
        assert_eq!(events.len(), 2);
        assert!(matches!(&events[0], StatusUpdate::ToolStarted { name } if name == "echo"));
        assert!(
            matches!(&events[1], StatusUpdate::ToolCompleted { name, success } if name == "echo" && *success)
        );
    }

    #[tokio::test]
    async fn tool_calls_started() {
        let channel = TestChannel::new();
        let metadata = serde_json::Value::Null;

        channel
            .send_status(
                StatusUpdate::ToolStarted {
                    name: "memory_search".to_string(),
                },
                &metadata,
            )
            .await
            .unwrap();
        channel
            .send_status(StatusUpdate::Thinking("hmm".to_string()), &metadata)
            .await
            .unwrap();
        channel
            .send_status(
                StatusUpdate::ToolStarted {
                    name: "echo".to_string(),
                },
                &metadata,
            )
            .await
            .unwrap();

        let started = channel.tool_calls_started();
        assert_eq!(started, vec!["memory_search", "echo"]);
    }

    #[tokio::test]
    async fn tool_results() {
        let channel = TestChannel::new();
        channel
            .send_status(
                StatusUpdate::ToolResult {
                    name: "echo".to_string(),
                    preview: "hello world".to_string(),
                },
                &serde_json::Value::Null,
            )
            .await
            .unwrap();
        channel
            .send_status(
                StatusUpdate::ToolResult {
                    name: "time".to_string(),
                    preview: "{\"iso\": \"2026-03-03\"}".to_string(),
                },
                &serde_json::Value::Null,
            )
            .await
            .unwrap();

        let results = channel.tool_results();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].0, "echo");
        assert_eq!(results[0].1, "hello world");
        assert_eq!(results[1].0, "time");
        assert!(results[1].1.contains("2026"));
    }

    #[tokio::test]
    async fn wait_for_responses() {
        let channel = TestChannel::new();
        let responses = Arc::clone(&channel.responses);

        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            responses
                .lock()
                .await
                .push(OutgoingResponse::text("delayed reply"));
        });

        let collected = channel.wait_for_responses(1, Duration::from_secs(2)).await;
        assert_eq!(collected.len(), 1);
        assert_eq!(collected[0].content, "delayed reply");
    }

    #[tokio::test]
    async fn tool_timings() {
        let channel = TestChannel::new();
        channel
            .send_status(
                StatusUpdate::ToolStarted {
                    name: "echo".to_string(),
                },
                &serde_json::Value::Null,
            )
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        channel
            .send_status(
                StatusUpdate::ToolCompleted {
                    name: "echo".to_string(),
                    success: true,
                },
                &serde_json::Value::Null,
            )
            .await
            .unwrap();

        let timings = channel.tool_timings();
        assert_eq!(timings.len(), 1);
        assert_eq!(timings[0].0, "echo");
        assert!(
            timings[0].1 >= 40,
            "Expected >= 40ms, got {}ms",
            timings[0].1
        );
    }
}

// ---------------------------------------------------------------------------
// trace_llm
// ---------------------------------------------------------------------------

mod trace_llm_tests {
    use crate::support::trace_llm::*;
    use ironclaw::llm::{
        ChatMessage, CompletionRequest, FinishReason, LlmProvider, ToolCompletionRequest,
    };

    fn text_step(content: &str, input_tokens: u32, output_tokens: u32) -> TraceStep {
        TraceStep {
            request_hint: None,
            response: TraceResponse::Text {
                content: content.to_string(),
                input_tokens,
                output_tokens,
            },
            expected_tool_results: Vec::new(),
        }
    }

    fn tool_calls_step(calls: Vec<TraceToolCall>, input: u32, output: u32) -> TraceStep {
        TraceStep {
            request_hint: None,
            response: TraceResponse::ToolCalls {
                tool_calls: calls,
                input_tokens: input,
                output_tokens: output,
            },
            expected_tool_results: Vec::new(),
        }
    }

    fn simple_tool_call(name: &str) -> TraceToolCall {
        TraceToolCall {
            id: format!("call_{name}"),
            name: name.to_string(),
            arguments: serde_json::json!({"key": "value"}),
        }
    }

    fn make_request(user_msg: &str) -> ToolCompletionRequest {
        ToolCompletionRequest::new(vec![ChatMessage::user(user_msg)], vec![])
    }

    fn make_completion_request(user_msg: &str) -> CompletionRequest {
        CompletionRequest::new(vec![ChatMessage::user(user_msg)])
    }

    #[tokio::test]
    async fn replays_text_response() {
        let trace =
            LlmTrace::single_turn("test-model", "hi", vec![text_step("Hello world", 100, 20)]);
        let llm = TraceLlm::from_trace(trace);

        let resp = llm.complete_with_tools(make_request("hi")).await.unwrap();

        assert_eq!(resp.content.as_deref(), Some("Hello world"));
        assert!(resp.tool_calls.is_empty());
        assert_eq!(resp.input_tokens, 100);
        assert_eq!(resp.output_tokens, 20);
        assert_eq!(resp.finish_reason, FinishReason::Stop);
        assert_eq!(llm.calls(), 1);
    }

    #[tokio::test]
    async fn replays_tool_calls() {
        let trace = LlmTrace::single_turn(
            "test-model",
            "search memory",
            vec![tool_calls_step(
                vec![simple_tool_call("memory_search")],
                80,
                15,
            )],
        );
        let llm = TraceLlm::from_trace(trace);

        let resp = llm
            .complete_with_tools(make_request("search memory"))
            .await
            .unwrap();

        assert!(resp.content.is_none());
        assert_eq!(resp.tool_calls.len(), 1);
        assert_eq!(resp.tool_calls[0].name, "memory_search");
        assert_eq!(resp.tool_calls[0].id, "call_memory_search");
        assert_eq!(
            resp.tool_calls[0].arguments,
            serde_json::json!({"key": "value"})
        );
        assert_eq!(resp.input_tokens, 80);
        assert_eq!(resp.output_tokens, 15);
        assert_eq!(resp.finish_reason, FinishReason::ToolUse);
    }

    #[tokio::test]
    async fn advances_through_steps() {
        let trace = LlmTrace::single_turn(
            "test-model",
            "do something",
            vec![
                tool_calls_step(vec![simple_tool_call("echo")], 50, 10),
                text_step("Done!", 60, 5),
            ],
        );
        let llm = TraceLlm::from_trace(trace);

        let resp1 = llm
            .complete_with_tools(make_request("do something"))
            .await
            .unwrap();
        assert_eq!(resp1.tool_calls.len(), 1);
        assert_eq!(resp1.tool_calls[0].name, "echo");
        assert_eq!(llm.calls(), 1);

        let resp2 = llm
            .complete_with_tools(make_request("continue"))
            .await
            .unwrap();
        assert_eq!(resp2.content.as_deref(), Some("Done!"));
        assert!(resp2.tool_calls.is_empty());
        assert_eq!(llm.calls(), 2);
    }

    #[tokio::test]
    async fn errors_when_exhausted() {
        let trace =
            LlmTrace::single_turn("test-model", "first", vec![text_step("only once", 10, 5)]);
        let llm = TraceLlm::from_trace(trace);

        let resp1 = llm.complete_with_tools(make_request("first")).await;
        assert!(resp1.is_ok());

        let resp2 = llm.complete_with_tools(make_request("second")).await;
        assert!(resp2.is_err());
        let err = resp2.unwrap_err();
        let err_msg = err.to_string();
        assert!(
            err_msg.contains("exhausted"),
            "Expected 'exhausted' in error: {err_msg}"
        );
    }

    #[tokio::test]
    async fn validates_request_hints() {
        let trace = LlmTrace::single_turn(
            "test-model",
            "say hello please",
            vec![TraceStep {
                request_hint: Some(RequestHint {
                    last_user_message_contains: Some("hello".to_string()),
                    min_message_count: Some(1),
                }),
                response: TraceResponse::Text {
                    content: "matched".to_string(),
                    input_tokens: 10,
                    output_tokens: 5,
                },
                expected_tool_results: Vec::new(),
            }],
        );
        let llm = TraceLlm::from_trace(trace);

        let resp = llm
            .complete_with_tools(make_request("say hello please"))
            .await
            .unwrap();

        assert_eq!(resp.content.as_deref(), Some("matched"));
        assert_eq!(llm.hint_mismatches(), 0);
    }

    #[tokio::test]
    async fn hint_mismatch_warns_but_continues() {
        let trace = LlmTrace::single_turn(
            "test-model",
            "apple",
            vec![TraceStep {
                request_hint: Some(RequestHint {
                    last_user_message_contains: Some("banana".to_string()),
                    min_message_count: Some(5),
                }),
                response: TraceResponse::Text {
                    content: "still works".to_string(),
                    input_tokens: 10,
                    output_tokens: 5,
                },
                expected_tool_results: Vec::new(),
            }],
        );
        let llm = TraceLlm::from_trace(trace);

        let resp = llm
            .complete_with_tools(make_request("apple"))
            .await
            .unwrap();

        assert_eq!(resp.content.as_deref(), Some("still works"));
        assert_eq!(llm.hint_mismatches(), 2);
    }

    #[tokio::test]
    async fn from_json_file() {
        let fixture_path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/llm_traces/simple_text.json"
        );
        let llm = TraceLlm::from_file(fixture_path).unwrap();

        assert_eq!(llm.model_name(), "test-model");

        let resp = llm
            .complete_with_tools(make_request("anything"))
            .await
            .unwrap();

        assert_eq!(resp.content.as_deref(), Some("Hello from fixture file!"));
        assert_eq!(resp.input_tokens, 50);
        assert_eq!(resp.output_tokens, 10);
    }

    #[tokio::test]
    async fn complete_text_step() {
        let trace = LlmTrace::single_turn("test-model", "hi", vec![text_step("plain text", 30, 8)]);
        let llm = TraceLlm::from_trace(trace);

        let resp = llm.complete(make_completion_request("hi")).await.unwrap();

        assert_eq!(resp.content, "plain text");
        assert_eq!(resp.input_tokens, 30);
        assert_eq!(resp.output_tokens, 8);
        assert_eq!(resp.finish_reason, FinishReason::Stop);
    }

    #[tokio::test]
    async fn complete_errors_on_tool_calls_step() {
        let trace = LlmTrace::single_turn(
            "test-model",
            "hi",
            vec![tool_calls_step(vec![simple_tool_call("echo")], 10, 5)],
        );
        let llm = TraceLlm::from_trace(trace);

        let result = llm.complete(make_completion_request("hi")).await;

        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("tool_calls"),
            "Expected 'tool_calls' in error: {err_msg}"
        );
    }

    #[tokio::test]
    async fn captured_requests() {
        let trace = LlmTrace::single_turn(
            "test-model",
            "test",
            vec![text_step("resp1", 10, 5), text_step("resp2", 10, 5)],
        );
        let llm = TraceLlm::from_trace(trace);

        llm.complete_with_tools(make_request("first message"))
            .await
            .unwrap();
        llm.complete_with_tools(make_request("second message"))
            .await
            .unwrap();

        let captured = llm.captured_requests();
        assert_eq!(captured.len(), 2);
        assert_eq!(captured[0].len(), 1);
        assert_eq!(captured[0][0].content, "first message");
        assert_eq!(captured[1][0].content, "second message");
    }

    #[test]
    fn deserialize_flat_steps_as_single_turn() {
        let json = r#"{"model_name": "m", "steps": [
            {"response": {"type": "text", "content": "hi", "input_tokens": 1, "output_tokens": 1}}
        ]}"#;
        let trace: LlmTrace = serde_json::from_str(json).unwrap();
        assert_eq!(trace.turns.len(), 1);
        assert_eq!(trace.turns[0].user_input, "(test input)");
        assert_eq!(trace.turns[0].steps.len(), 1);
    }

    #[test]
    fn deserialize_turns_format() {
        let json = r#"{"model_name": "m", "turns": [
            {"user_input": "hello", "steps": [
                {"response": {"type": "text", "content": "hi", "input_tokens": 1, "output_tokens": 1}}
            ]},
            {"user_input": "bye", "steps": [
                {"response": {"type": "text", "content": "bye", "input_tokens": 1, "output_tokens": 1}}
            ]}
        ]}"#;
        let trace: LlmTrace = serde_json::from_str(json).unwrap();
        assert_eq!(trace.turns.len(), 2);
        assert_eq!(trace.turns[0].user_input, "hello");
        assert_eq!(trace.turns[1].user_input, "bye");
    }

    #[tokio::test]
    async fn multi_turn() {
        let trace = LlmTrace::new(
            "turns-model",
            vec![
                TraceTurn {
                    user_input: "first".to_string(),
                    steps: vec![text_step("turn 1 response", 10, 5)],
                    expects: TraceExpects::default(),
                },
                TraceTurn {
                    user_input: "second".to_string(),
                    steps: vec![text_step("turn 2 response", 20, 10)],
                    expects: TraceExpects::default(),
                },
            ],
        );
        let llm = TraceLlm::from_trace(trace);

        let resp1 = llm
            .complete_with_tools(make_request("first"))
            .await
            .unwrap();
        assert_eq!(resp1.content.as_deref(), Some("turn 1 response"));

        let resp2 = llm
            .complete_with_tools(make_request("second"))
            .await
            .unwrap();
        assert_eq!(resp2.content.as_deref(), Some("turn 2 response"));

        assert_eq!(llm.calls(), 2);
    }
}

// ---------------------------------------------------------------------------
// test_rig
// ---------------------------------------------------------------------------

#[cfg(feature = "libsql")]
mod test_rig_tests {
    use std::time::Duration;

    use crate::support::test_rig::TestRigBuilder;
    use crate::support::trace_llm::{LlmTrace, TraceResponse, TraceStep};

    #[tokio::test]
    async fn rig_builds_and_runs() {
        let trace = LlmTrace::single_turn(
            "test-model",
            "Hello test rig",
            vec![TraceStep {
                request_hint: None,
                response: TraceResponse::Text {
                    content: "I am the test rig response.".to_string(),
                    input_tokens: 50,
                    output_tokens: 15,
                },
                expected_tool_results: Vec::new(),
            }],
        );

        let rig = TestRigBuilder::new().with_trace(trace).build().await;

        rig.send_message("Hello test rig").await;

        let responses = rig.wait_for_responses(1, Duration::from_secs(10)).await;

        assert!(
            !responses.is_empty(),
            "Expected at least one response from the agent"
        );
        let found = responses
            .iter()
            .any(|r| r.content.contains("I am the test rig response."));
        assert!(
            found,
            "Expected a response containing the trace text, got: {:?}",
            responses.iter().map(|r| &r.content).collect::<Vec<_>>()
        );

        rig.shutdown();
    }
}
