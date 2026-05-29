//! Unit tests for E2E test support modules.
//!
//! These tests live here (instead of inside `support/*.rs`) so they compile
//! and run exactly once, rather than being duplicated across every `e2e_*.rs`
//! test binary that declares `mod support;`.

#[path = "support/reborn/mod.rs"]
mod reborn_support;
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

    #[test]
    fn tool_order_passes_for_correct_order() {
        let started: Vec<String> = vec!["write_file", "echo", "read_file"]
            .into_iter()
            .map(String::from)
            .collect();
        assert_tool_order(&started, &["write_file", "read_file"]);
    }

    #[test]
    fn tool_order_passes_for_consecutive() {
        let started: Vec<String> = vec!["write_file", "read_file"]
            .into_iter()
            .map(String::from)
            .collect();
        assert_tool_order(&started, &["write_file", "read_file"]);
    }

    #[test]
    #[should_panic(expected = "assert_tool_order")]
    fn tool_order_panics_for_wrong_order() {
        let started: Vec<String> = vec!["read_file", "write_file"]
            .into_iter()
            .map(String::from)
            .collect();
        assert_tool_order(&started, &["write_file", "read_file"]);
    }

    #[test]
    #[should_panic(expected = "assert_tool_order")]
    fn tool_order_panics_for_missing_tool() {
        let started: Vec<String> = vec!["echo".to_string()];
        assert_tool_order(&started, &["echo", "write_file"]);
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

    #[test]
    fn cleanup_guard_file_does_not_remove_dir() {
        let dir = "/tmp/ironclaw_cleanup_guard_file_not_dir";
        std::fs::create_dir_all(dir).unwrap();
        {
            // Registering a directory path as .file() should not remove it
            // (remove_file fails on directories).
            let _guard = CleanupGuard::new().file(dir);
        }
        assert!(
            std::path::Path::new(dir).exists(),
            "dir should still exist when registered as file"
        );
        // Clean up manually.
        let _ = std::fs::remove_dir_all(dir);
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
                    detail: None,
                    call_id: None,
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
                    error: None,
                    parameters: None,
                    call_id: None,
                    duration_ms: None,
                },
                &metadata,
            )
            .await
            .unwrap();

        let events = channel.captured_status_events();
        assert_eq!(events.len(), 2);
        assert!(matches!(&events[0], StatusUpdate::ToolStarted { name, .. } if name == "echo"));
        assert!(
            matches!(&events[1], StatusUpdate::ToolCompleted { name, success, .. } if name == "echo" && *success)
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
                    detail: None,
                    call_id: None,
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
                    detail: None,
                    call_id: None,
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
                    call_id: None,
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
                    call_id: None,
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
                    detail: None,
                    call_id: None,
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
                    error: None,
                    parameters: None,
                    call_id: None,
                    duration_ms: None,
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
// reborn support
// ---------------------------------------------------------------------------

mod reborn_support_tests {
    use std::{
        net::{IpAddr, Ipv4Addr},
        sync::{Arc, Mutex},
        time::Duration,
    };

    use async_trait::async_trait;
    use ironclaw_filesystem::ScopedFilesystem;
    use ironclaw_host_api::{
        AgentId, CapabilityId, InvocationId, MountAlias, MountGrant, MountPermissions, MountView,
        NetworkMethod, NetworkPolicy, NetworkTargetPattern, ProjectId, ResourceScope, TenantId,
        ThreadId, UserId, VirtualPath,
    };
    use ironclaw_loop_support::{
        HostManagedModelErrorKind, HostManagedModelGateway, HostManagedModelMessage,
        HostManagedModelMessageRole, HostManagedModelRequest, HostManagedModelResponse,
        HostManagedToolResultContent,
    };
    use ironclaw_network::{
        NetworkHttpEgress, NetworkHttpError, NetworkHttpRequest, NetworkHttpResponse,
        NetworkHttpTransport, NetworkResolver, NetworkTransportRequest, NetworkUsage,
        PolicyNetworkHttpEgress,
    };
    use ironclaw_product_adapters::{
        AuthRequirement, DeliveryStatus, ExternalConversationRef, FakeProjectionStream,
        FakeProtocolHttpEgress, FinalReplyView, OutboundDeliverySink, ProductAdapter,
        ProductAdapterError, ProductInboundAck, ProductOutboundEnvelope, ProductOutboundPayload,
        ProductOutboundTarget, ProductProjectionItem, ProductProjectionState, ProductRenderOutcome,
        ProductWorkflow, ProjectionCursor, ProjectionStream, ProtocolAuthEvidence,
    };
    use ironclaw_product_workflow::{
        ActionDispatchKind, ActionFingerprintKey, ConversationBindingService,
        DefaultInboundTurnService, DefaultProductWorkflow, IdempotencyDecision, IdempotencyLedger,
        InboundTurnService, ProductActionId, ProductConversationRouteKind, ProductWorkflowError,
        ResolveBindingRequest, SourceBindingKey,
    };
    use ironclaw_threads::{
        AcceptInboundMessageRequest, AppendAssistantDraftRequest, EnsureThreadRequest,
        MessageContent, ProviderToolCallReferenceEnvelope, SessionThreadService, ThreadScope,
        ToolResultSafeSummary,
    };
    use ironclaw_turns::{
        CancelRunRequest, CancelRunResponse, GetRunStateRequest, LoopMessageRef,
        ReplyTargetBindingRef, ResumeTurnRequest, ResumeTurnResponse, RunProfileId,
        RunProfileVersion, SubmitTurnRequest, SubmitTurnResponse, ThreadBusy, TurnCoordinator,
        TurnError, TurnId, TurnRunId, TurnRunState, TurnScope, TurnStatus,
        events::EventCursor,
        run_profile::{
            CapabilityBatchInvocation, CapabilityInputRef, CapabilityInvocation, CapabilityOutcome,
            LoopCapabilityPort, ModelProfileId, ParentLoopOutput, VisibleCapabilityRequest,
        },
    };
    use tokio::sync::Barrier;

    use crate::reborn_support::delivery::RecordingOutboundDeliverySink;
    use crate::reborn_support::filesystem::local_filesystem;
    use crate::reborn_support::harness::RecordingTestCapabilityPort;
    use crate::reborn_support::model_replay::{
        RebornModelReplayStep, RebornScriptedProviderToolCall, RebornTraceReplayError,
        RebornTraceReplayModelGateway, capability_call_from_trace_with_surface,
    };
    use crate::reborn_support::network::RecordingNetworkHttpTransport;
    use crate::reborn_support::product_workflow::{
        FilesystemIdempotencyLedger, RebornProductWorkflowHarness,
        RebornProductWorkflowHarnessError, resource_scope,
    };
    use crate::reborn_support::session_thread::{RebornThreadHarness, RebornThreadHarnessError};
    use crate::reborn_support::test_adapter::{RebornTestIngress, RebornTestProductAdapter};
    use crate::support::trace_llm::{
        ExpectedToolResult, LlmTrace, TraceExpects, TraceResponse, TraceStep, TraceToolCall,
        TraceTurn,
    };

    #[tokio::test]
    async fn trace_replay_records_requests_and_returns_reply() {
        let gateway = RebornTraceReplayModelGateway::with_responses([
            HostManagedModelResponse::assistant_reply("hello"),
        ]);
        let response = gateway
            .stream_model(model_request(Vec::new()))
            .await
            .expect("trace response");

        assert_eq!(response.safe_text_deltas, vec!["hello"]);
        assert_eq!(gateway.requests().len(), 1);
        gateway.assert_exhausted();
    }

    #[tokio::test]
    async fn trace_replay_replays_multiple_steps_in_order() {
        let gateway = RebornTraceReplayModelGateway::from_trace(LlmTrace::new(
            "trace",
            vec![
                TraceTurn {
                    user_input: "first".to_string(),
                    steps: vec![
                        TraceStep {
                            request_hint: None,
                            response: TraceResponse::Text {
                                content: "first reply".to_string(),
                                input_tokens: 1,
                                output_tokens: 1,
                            },
                            expected_tool_results: Vec::new(),
                        },
                        TraceStep {
                            request_hint: None,
                            response: TraceResponse::ToolCalls {
                                tool_calls: vec![TraceToolCall {
                                    id: "call-ordered".to_string(),
                                    name: "test.echo".to_string(),
                                    arguments: serde_json::json!({"message": "second"}),
                                }],
                                input_tokens: 1,
                                output_tokens: 1,
                            },
                            expected_tool_results: Vec::new(),
                        },
                    ],
                    expects: TraceExpects::default(),
                },
                TraceTurn {
                    user_input: "second".to_string(),
                    steps: vec![TraceStep {
                        request_hint: None,
                        response: TraceResponse::Text {
                            content: "third reply".to_string(),
                            input_tokens: 1,
                            output_tokens: 1,
                        },
                        expected_tool_results: Vec::new(),
                    }],
                    expects: TraceExpects::default(),
                },
            ],
        ))
        .expect("trace gateway");

        let first = gateway
            .stream_model(model_request(Vec::new()))
            .await
            .expect("first response");
        assert_eq!(first.safe_text_deltas, vec!["first reply"]);

        let second = gateway
            .stream_model(model_request(Vec::new()))
            .await
            .expect("second response");
        let ParentLoopOutput::CapabilityCalls(calls) = second.output else {
            panic!("expected capability calls");
        };
        assert_eq!(calls[0].capability_id.as_str(), "test.echo");
        assert_eq!(
            calls[0]
                .provider_replay
                .as_ref()
                .expect("provider replay")
                .provider_call_id,
            "call-ordered"
        );

        let third = gateway
            .stream_model(model_request(Vec::new()))
            .await
            .expect("third response");
        assert_eq!(third.safe_text_deltas, vec!["third reply"]);
        gateway.assert_exhausted();
    }

    #[tokio::test]
    async fn trace_replay_returns_capability_call() {
        let gateway = RebornTraceReplayModelGateway::from_trace(LlmTrace::single_turn(
            "trace",
            "run tool",
            vec![TraceStep {
                request_hint: None,
                response: TraceResponse::ToolCalls {
                    tool_calls: vec![TraceToolCall {
                        id: "call-1".to_string(),
                        name: "test.echo".to_string(),
                        arguments: serde_json::json!({"message": "hi"}),
                    }],
                    input_tokens: 1,
                    output_tokens: 1,
                },
                expected_tool_results: Vec::new(),
            }],
        ))
        .expect("trace gateway");

        let response = gateway
            .stream_model(model_request(Vec::new()))
            .await
            .expect("capability response");
        let ParentLoopOutput::CapabilityCalls(calls) = response.output else {
            panic!("expected capability calls");
        };
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].capability_id.as_str(), "test.echo");
        assert_eq!(
            calls[0]
                .provider_replay
                .as_ref()
                .expect("provider replay")
                .provider_call_id,
            "call-1"
        );
    }

    #[tokio::test]
    async fn trace_replay_rejects_scripted_capability_not_advertised_by_surface() {
        let missing_capability =
            CapabilityId::new("test.missing").expect("valid missing capability id");
        let gateway = RebornTraceReplayModelGateway::with_scripted_steps([
            RebornModelReplayStep::ProviderToolCalls {
                calls: vec![RebornScriptedProviderToolCall::new(
                    missing_capability,
                    "call-missing",
                    serde_json::json!({"message": "hi"}),
                )],
                expected_tool_results: Vec::new(),
            },
        ]);

        let error = gateway
            .stream_model_with_capabilities(
                model_request(Vec::new()),
                Arc::new(RecordingTestCapabilityPort::echo()),
            )
            .await
            .expect_err("unadvertised scripted capability should fail");

        assert_eq!(error.kind, HostManagedModelErrorKind::InvalidOutput);
        assert!(
            error
                .safe_summary
                .contains("scripted capability test.missing was not advertised to the model"),
            "unexpected error summary: {}",
            error.safe_summary
        );
        assert_eq!(gateway.remaining_responses(), 0);
    }

    #[tokio::test]
    async fn trace_replay_fails_when_exhausted() {
        let gateway = RebornTraceReplayModelGateway::with_responses(Vec::new());
        let error = gateway
            .stream_model(model_request(Vec::new()))
            .await
            .expect_err("empty trace should fail");
        assert!(
            error.safe_summary.contains("no matching step"),
            "unexpected error summary: {}",
            error.safe_summary
        );
    }

    #[tokio::test]
    async fn trace_replay_requires_expected_tool_result_before_next_response() {
        let trace_step = TraceStep {
            request_hint: None,
            response: TraceResponse::Text {
                content: "after tool".to_string(),
                input_tokens: 1,
                output_tokens: 1,
            },
            expected_tool_results: vec![ExpectedToolResult {
                tool_call_id: "call-1".to_string(),
                name: "test.echo".to_string(),
                content: "tool output".to_string(),
            }],
        };
        let missing = RebornTraceReplayModelGateway::from_trace(LlmTrace::single_turn(
            "trace",
            "run tool",
            vec![trace_step.clone()],
        ))
        .expect("missing gateway");
        assert!(
            missing
                .stream_model(model_request(Vec::new()))
                .await
                .is_err(),
            "missing expected tool result must fail"
        );
        assert!(
            missing.requests().is_empty(),
            "failed validations should not record successful model requests"
        );
        assert_eq!(missing.remaining_responses(), 1);

        let substring = RebornTraceReplayModelGateway::from_trace(LlmTrace::single_turn(
            "trace",
            "run tool",
            vec![trace_step.clone()],
        ))
        .expect("substring gateway");
        assert!(
            substring
                .stream_model(model_request(vec![tool_result_message(
                    "call-1",
                    "test.echo",
                    "tool output with suffix",
                )]))
                .await
                .is_err(),
            "tool result validation must require exact content equality"
        );
        assert!(substring.requests().is_empty());

        let matched = RebornTraceReplayModelGateway::from_trace(LlmTrace::single_turn(
            "trace",
            "run tool",
            vec![trace_step],
        ))
        .expect("matched gateway");
        matched
            .stream_model(model_request(vec![tool_result_message(
                "call-1",
                "test.echo",
                "tool output",
            )]))
            .await
            .expect("matching tool result");
        assert_eq!(matched.requests().len(), 1);
    }

    #[tokio::test]
    async fn scripted_response_step_validates_expected_tool_results() {
        let gateway =
            RebornTraceReplayModelGateway::with_scripted_steps([RebornModelReplayStep::Response {
                response: HostManagedModelResponse::assistant_reply("after tool"),
                expected_tool_results: vec![ExpectedToolResult {
                    tool_call_id: "call-scripted".to_string(),
                    name: "builtin.write_file".to_string(),
                    content: "result:ref-123".to_string(),
                }],
            }]);

        assert!(
            gateway
                .stream_model(model_request(Vec::new()))
                .await
                .is_err(),
            "scripted response step must reject missing expected tool result"
        );
        assert!(gateway.requests().is_empty());
        assert_eq!(gateway.remaining_responses(), 1);

        gateway
            .stream_model(model_request(vec![tool_result_message(
                "call-scripted",
                "builtin.write_file",
                "result:ref-123",
            )]))
            .await
            .expect("matching scripted tool result");
        assert_eq!(gateway.requests().len(), 1);
        gateway.assert_exhausted();
    }

    #[tokio::test]
    async fn scripted_provider_tool_call_flow_validates_follow_up_tool_result() {
        let gateway = RebornTraceReplayModelGateway::with_scripted_steps([
            RebornModelReplayStep::ProviderToolCalls {
                calls: vec![RebornScriptedProviderToolCall::new(
                    CapabilityId::new("test.echo").expect("valid capability id"),
                    "call-scripted",
                    serde_json::json!({"message": "hi"}),
                )],
                expected_tool_results: Vec::new(),
            },
            RebornModelReplayStep::Response {
                response: HostManagedModelResponse::assistant_reply("after tool"),
                expected_tool_results: vec![ExpectedToolResult {
                    tool_call_id: "call-scripted".to_string(),
                    name: "test_echo".to_string(),
                    content: "echo: hi".to_string(),
                }],
            },
        ]);

        gateway
            .stream_model_with_capabilities(
                model_request(Vec::new()),
                Arc::new(RecordingTestCapabilityPort::echo()),
            )
            .await
            .expect("scripted provider tool call");
        assert_eq!(gateway.remaining_responses(), 1);

        assert!(
            gateway
                .stream_model(model_request(vec![tool_result_message_with_capability_id(
                    "call-scripted",
                    "test_echo",
                    "test.echo",
                    "echo: hi with suffix",
                )]))
                .await
                .is_err(),
            "scripted follow-up must reject mismatched tool result content"
        );
        assert_eq!(gateway.remaining_responses(), 1);

        gateway
            .stream_model(model_request(vec![tool_result_message_with_capability_id(
                "call-scripted",
                "test_echo",
                "test.echo",
                "echo: hi",
            )]))
            .await
            .expect("matching scripted follow-up tool result");
        assert_eq!(gateway.requests().len(), 2);
        gateway.assert_exhausted();
    }

    #[test]
    fn trace_replay_rejects_user_input_response() {
        let error = RebornTraceReplayModelGateway::from_trace(LlmTrace::single_turn(
            "trace",
            "user marker",
            vec![TraceStep {
                request_hint: None,
                response: TraceResponse::UserInput {
                    content: "hello".to_string(),
                },
                expected_tool_results: Vec::new(),
            }],
        ))
        .expect_err("user input response is not replayable");

        assert!(matches!(error, RebornTraceReplayError::UnsupportedResponse));
    }

    #[test]
    fn capability_call_from_trace_rejects_invalid_inputs() {
        let invalid_surface = capability_call_from_trace_with_surface(
            TraceToolCall {
                id: "call-1".to_string(),
                name: "test.echo".to_string(),
                arguments: serde_json::json!({}),
            },
            "invalid surface!",
        )
        .expect_err("invalid surface version");
        assert!(matches!(
            invalid_surface,
            RebornTraceReplayError::InvalidSurfaceVersion(_)
        ));

        let invalid_capability = capability_call_from_trace_with_surface(
            TraceToolCall {
                id: "call-1".to_string(),
                name: "Bad Name".to_string(),
                arguments: serde_json::json!({}),
            },
            "trace_replay_v1",
        )
        .expect_err("invalid capability id");
        assert!(matches!(
            invalid_capability,
            RebornTraceReplayError::InvalidCapabilityId { .. }
        ));

        let invalid_input_ref = capability_call_from_trace_with_surface(
            TraceToolCall {
                id: "bad\nid".to_string(),
                name: "test.echo".to_string(),
                arguments: serde_json::json!({}),
            },
            "trace_replay_v1",
        )
        .expect_err("invalid input ref");
        assert!(matches!(
            invalid_input_ref,
            RebornTraceReplayError::InvalidInputRef { .. }
        ));
    }

    #[tokio::test]
    async fn trace_replay_preserves_provider_tool_call_metadata() {
        let gateway = RebornTraceReplayModelGateway::from_trace(LlmTrace::single_turn(
            "trace",
            "run tool",
            vec![TraceStep {
                request_hint: None,
                response: TraceResponse::ToolCalls {
                    tool_calls: vec![TraceToolCall {
                        id: "provider-call-9".to_string(),
                        name: "test.search".to_string(),
                        arguments: serde_json::json!({"q": "near"}),
                    }],
                    input_tokens: 1,
                    output_tokens: 1,
                },
                expected_tool_results: Vec::new(),
            }],
        ))
        .expect("trace gateway");

        let response = gateway
            .stream_model(model_request(Vec::new()))
            .await
            .expect("capability response");
        let ParentLoopOutput::CapabilityCalls(calls) = response.output else {
            panic!("expected capability calls");
        };
        let replay = calls[0].provider_replay.as_ref().expect("provider replay");
        assert_eq!(replay.provider_call_id, "provider-call-9");
        assert_eq!(replay.provider_tool_name, "test.search");
        assert_eq!(replay.arguments, serde_json::json!({"q": "near"}));
    }

    #[tokio::test]
    async fn recording_network_transport_records_request_and_returns_scripted_response() {
        let transport = RecordingNetworkHttpTransport::new();
        transport.push_response(NetworkHttpResponse {
            status: 200,
            headers: vec![],
            body: b"ok".to_vec(),
            usage: NetworkUsage::default(),
        });

        let response = transport
            .execute(NetworkTransportRequest {
                method: NetworkMethod::Post,
                url: "https://api.example.test/v1".to_string(),
                headers: vec![("x-safe".to_string(), "yes".to_string())],
                body: b"hello".to_vec(),
                resolved_ips: vec![IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34))],
                response_body_limit: Some(1024),
                timeout_ms: Some(50),
            })
            .await
            .expect("scripted response");

        assert_eq!(response.status, 200);
        let requests = transport.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].body_len, 5);
        assert_eq!(
            requests[0].headers,
            vec![("x-safe".to_string(), "yes".to_string())]
        );

        let failing = RecordingNetworkHttpTransport::new();
        failing.push_error(NetworkHttpError::Transport {
            reason: "scripted".to_string(),
            request_bytes: 0,
            response_bytes: 0,
        });
        let error = failing
            .execute(NetworkTransportRequest {
                method: NetworkMethod::Get,
                url: "https://api.example.test/v1".to_string(),
                headers: vec![],
                body: Vec::new(),
                resolved_ips: vec![IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34))],
                response_body_limit: None,
                timeout_ms: None,
            })
            .await
            .expect_err("scripted error");
        assert!(matches!(error, NetworkHttpError::Transport { .. }));
    }

    #[tokio::test]
    async fn network_transport_mixed_results() {
        let transport = RecordingNetworkHttpTransport::new();
        transport.push_response(NetworkHttpResponse {
            status: 200,
            headers: vec![],
            body: b"first".to_vec(),
            usage: NetworkUsage::default(),
        });
        transport.push_error(NetworkHttpError::Transport {
            reason: "scripted failure".to_string(),
            request_bytes: 0,
            response_bytes: 0,
        });
        transport.push_response(NetworkHttpResponse {
            status: 202,
            headers: vec![],
            body: b"third".to_vec(),
            usage: NetworkUsage::default(),
        });

        assert_eq!(
            execute_recorded_get(&transport)
                .await
                .expect("first")
                .status,
            200
        );
        assert!(execute_recorded_get(&transport).await.is_err());
        assert_eq!(
            execute_recorded_get(&transport)
                .await
                .expect("third")
                .status,
            202
        );
        assert_eq!(transport.requests().len(), 3);
    }

    #[tokio::test]
    async fn recording_network_transport_sanitizes_sensitive_request_data() {
        let transport = RecordingNetworkHttpTransport::new();
        let _ = transport
            .execute(NetworkTransportRequest {
                method: NetworkMethod::Get,
                url: "https://api.example.test/v1?token=secret&ok=1".to_string(),
                headers: vec![
                    ("authorization".to_string(), "Bearer secret".to_string()),
                    ("x-api-key".to_string(), "secret-key".to_string()),
                    ("x-token-type".to_string(), "Bearer".to_string()),
                    ("x-secret-hash-algorithm".to_string(), "sha256".to_string()),
                ],
                body: b"secret body".to_vec(),
                resolved_ips: vec![IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34))],
                response_body_limit: None,
                timeout_ms: None,
            })
            .await;

        let request = transport.requests().pop().expect("recorded request");
        assert_eq!(request.url, "https://api.example.test/v1?<redacted>");
        assert_eq!(
            request.headers,
            vec![
                ("authorization".to_string(), "<redacted>".to_string()),
                ("x-api-key".to_string(), "<redacted>".to_string()),
                ("x-token-type".to_string(), "Bearer".to_string()),
                ("x-secret-hash-algorithm".to_string(), "sha256".to_string()),
            ]
        );
        assert_eq!(request.body_len, 11);
        assert_ne!(request.body_sha256, "secret body");
    }

    #[tokio::test]
    async fn recording_network_transport_errors_on_unexpected_request() {
        let transport = RecordingNetworkHttpTransport::new();

        let error = execute_recorded_get(&transport)
            .await
            .expect_err("unexpected request should fail");

        assert!(
            matches!(&error, NetworkHttpError::Transport { reason, .. } if reason.contains("unexpected HTTP request")),
            "unexpected error: {error:?}"
        );
        assert_eq!(transport.requests().len(), 1);
    }

    #[tokio::test]
    async fn policy_network_egress_blocks_private_ip_before_transport() {
        let transport = RecordingNetworkHttpTransport::new();
        let _default_policy_egress = transport.policy_egress();
        let egress = PolicyNetworkHttpEgress::new_with_resolver(
            transport.clone(),
            StaticResolver(vec![IpAddr::V4(Ipv4Addr::new(10, 0, 0, 7))]),
        );

        let error = egress
            .execute(NetworkHttpRequest {
                scope: sample_scope(),
                method: NetworkMethod::Post,
                url: "https://api.example.test/v1".to_string(),
                headers: vec![],
                body: b"hello".to_vec(),
                policy: policy("api.example.test", Some(443), true, Some(1024)),
                response_body_limit: Some(1024),
                timeout_ms: None,
            })
            .await
            .expect_err("private IP should be denied before transport");

        assert!(matches!(error, NetworkHttpError::PolicyDenied { .. }));
        assert!(transport.requests().is_empty());
    }

    #[tokio::test]
    async fn recording_delivery_sink_records_statuses() {
        let sink = RecordingOutboundDeliverySink::new();
        let target = ReplyTargetBindingRef::new("reply-target").expect("reply target");
        sink.record(DeliveryStatus::Delivered {
            attempt_id: uuid::Uuid::new_v4(),
            target: target.clone(),
            run_id: None,
        })
        .await;

        let statuses = sink.statuses();
        assert_eq!(statuses.len(), 1);
        assert!(
            matches!(&statuses[0], DeliveryStatus::Delivered { target: actual, .. } if actual == &target)
        );
    }

    #[tokio::test]
    async fn filesystem_thread_harness_round_trips_history() {
        let harness = RebornThreadHarness::filesystem_temp(thread_scope("round-trip"))
            .expect("thread harness");
        let thread_id = write_thread_history(&harness).await;
        let history = harness.history(thread_id).await.expect("history");
        assert_eq!(history.len(), 2);
        harness
            .assert_final_reply(ThreadId::new("thread-round-trip").unwrap(), "assistant")
            .await
            .expect("final reply");
    }

    #[tokio::test]
    async fn filesystem_thread_harness_reopens_temp_backend_history() {
        let harness =
            RebornThreadHarness::filesystem_temp(thread_scope("reopen")).expect("thread harness");
        let thread_id = write_thread_history(&harness).await;
        let reopened = harness.reopened().expect("reopened harness");
        let history = reopened.history(thread_id).await.expect("reopened history");
        assert_eq!(history.len(), 2);
        reopened
            .assert_final_reply(ThreadId::new("thread-reopen").unwrap(), "assistant")
            .await
            .expect("reopened final reply");
    }

    #[tokio::test]
    async fn filesystem_thread_harness_checks_latest_final_reply() {
        let harness =
            RebornThreadHarness::filesystem_temp(thread_scope("latest")).expect("thread harness");
        let thread_id = ThreadId::new("thread-latest").unwrap();
        harness
            .service
            .ensure_thread(EnsureThreadRequest {
                scope: harness.scope.clone(),
                thread_id: Some(thread_id.clone()),
                created_by_actor_id: "alice".to_string(),
                title: Some("Latest reply".to_string()),
                metadata_json: None,
            })
            .await
            .expect("ensure thread");

        for text in ["old expected reply", "new different reply"] {
            let draft = harness
                .service
                .append_assistant_draft(AppendAssistantDraftRequest {
                    scope: harness.scope.clone(),
                    thread_id: thread_id.clone(),
                    turn_run_id: format!("run-{text}"),
                    content: MessageContent::text("draft"),
                })
                .await
                .expect("append draft");
            harness
                .service
                .finalize_assistant_message(
                    &harness.scope,
                    &thread_id,
                    draft.message_id,
                    MessageContent::text(text),
                )
                .await
                .expect("finalize draft");
        }

        assert!(
            harness
                .assert_final_reply(thread_id, "old expected")
                .await
                .is_err(),
            "only the latest finalized assistant reply should be accepted"
        );
    }

    #[tokio::test]
    async fn recording_capability_batch_stops_after_first_suspension() {
        let port = RecordingTestCapabilityPort::approval_then_echo();
        let surface = port
            .visible_capabilities(VisibleCapabilityRequest)
            .await
            .expect("visible capabilities");
        let capability_id = surface.descriptors[0].capability_id.clone();

        let outcome = port
            .invoke_capability_batch(CapabilityBatchInvocation {
                invocations: vec![
                    CapabilityInvocation {
                        surface_version: surface.version.clone(),
                        capability_id: capability_id.clone(),
                        input_ref: CapabilityInputRef::new("input:first").expect("first input"),
                    },
                    CapabilityInvocation {
                        surface_version: surface.version,
                        capability_id,
                        input_ref: CapabilityInputRef::new("input:second").expect("second input"),
                    },
                ],
                stop_on_first_suspension: true,
            })
            .await
            .expect("batch outcome");

        assert!(outcome.stopped_on_suspension);
        assert!(
            matches!(
                outcome.outcomes.as_slice(),
                [CapabilityOutcome::ApprovalRequired { .. }]
            ),
            "batch should return only the first suspension"
        );
        assert_eq!(
            port.invocation_count(),
            1,
            "second invocation must not run after suspension"
        );
    }

    #[tokio::test]
    async fn thread_harness_assert_final_reply_missing() {
        let harness =
            RebornThreadHarness::filesystem_temp(thread_scope("missing")).expect("thread harness");
        let thread_id = ThreadId::new("thread-missing").unwrap();
        harness
            .service
            .ensure_thread(EnsureThreadRequest {
                scope: harness.scope.clone(),
                thread_id: Some(thread_id.clone()),
                created_by_actor_id: "alice".to_string(),
                title: Some("Missing reply".to_string()),
                metadata_json: None,
            })
            .await
            .expect("ensure thread");

        let error = harness
            .assert_final_reply(thread_id, "assistant")
            .await
            .expect_err("missing final reply");
        assert!(matches!(
            error,
            RebornThreadHarnessError::MissingFinalReply(_)
        ));
    }

    #[tokio::test]
    async fn filesystem_binding_service_reopens_and_isolates_tenants() {
        let harness = RebornProductWorkflowHarness::filesystem_temp(product_scope("tenant-a"))
            .expect("product workflow harness");
        let request = binding_request("event-1", "alice", "room-1");

        let service = harness.binding_service().expect("binding service");
        let binding = service
            .resolve_binding(request.clone())
            .await
            .expect("binding");
        assert_eq!(binding.tenant_id.as_str(), "tenant-a");
        assert_eq!(binding.agent_id.as_ref().unwrap().as_str(), "agent-product");
        assert_eq!(
            binding.project_id.as_ref().unwrap().as_str(),
            "project-product"
        );

        let reopened = harness.reopened().expect("reopened product harness");
        let reopened_binding = reopened
            .binding_service()
            .expect("reopened binding service")
            .resolve_binding(request.clone())
            .await
            .expect("reopened binding");
        assert_eq!(reopened_binding, binding);

        let mut other_scope = product_scope("tenant-a");
        other_scope.agent_id = Some(AgentId::new("agent-other").unwrap());
        other_scope.project_id = Some(ProjectId::new("project-other").unwrap());
        let other_agent = harness
            .with_scope(other_scope)
            .expect("other-agent harness");
        let other_agent_binding = other_agent
            .binding_service()
            .expect("other-agent binding service")
            .resolve_binding(request.clone())
            .await
            .expect("other-agent binding");
        assert_eq!(
            other_agent_binding.agent_id.as_ref().unwrap().as_str(),
            "agent-other"
        );
        assert_eq!(
            other_agent_binding.project_id.as_ref().unwrap().as_str(),
            "project-other"
        );
        assert_ne!(other_agent_binding.agent_id, binding.agent_id);
        assert_ne!(other_agent_binding.project_id, binding.project_id);

        let tenant_b = harness
            .with_scope(product_scope("tenant-b"))
            .expect("tenant-b harness");
        let tenant_b_binding = tenant_b
            .binding_service()
            .expect("tenant-b binding service")
            .resolve_binding(request)
            .await
            .expect("tenant-b binding");
        assert_eq!(tenant_b_binding.tenant_id.as_str(), "tenant-b");
        assert_ne!(tenant_b_binding.user_id, binding.user_id);
        assert_ne!(tenant_b_binding.thread_id, binding.thread_id);
    }

    #[test]
    fn binding_service_missing_agent_scope() {
        let mut scope = product_scope("tenant-no-agent");
        scope.agent_id = None;
        let harness =
            RebornProductWorkflowHarness::filesystem_temp(scope).expect("product workflow harness");
        let Err(error) = harness.binding_service() else {
            panic!("missing agent id should fail");
        };
        assert!(matches!(
            error,
            RebornProductWorkflowHarnessError::MissingAgentScope
        ));
    }

    #[tokio::test]
    async fn product_workflow_malformed_json() {
        let harness =
            RebornProductWorkflowHarness::filesystem_temp(product_scope("tenant-corrupt"))
                .expect("product workflow harness");
        let request = binding_request("event-corrupt", "alice", "room-1");
        harness
            .corrupt_binding_record_for_test(&request, b"not valid json".to_vec())
            .await
            .expect("corrupt binding record");

        let error = harness
            .binding_service()
            .expect("binding service")
            .resolve_binding(request)
            .await
            .expect_err("corrupt binding JSON should fail");
        assert!(
            matches!(error, ProductWorkflowError::Transient { .. }),
            "unexpected error: {error:?}"
        );
    }

    #[tokio::test]
    async fn filesystem_idempotency_ledger_replays_releases_and_recovers() {
        let harness = RebornProductWorkflowHarness::filesystem_temp(product_scope("tenant-ledger"))
            .expect("product workflow harness");
        let ledger = harness.idempotency_ledger();
        let fingerprint = fingerprint_for("event-1", "alice", "room-1");
        let received_at = chrono::Utc::now();

        let IdempotencyDecision::New(action) = ledger
            .begin_or_replay(fingerprint.clone(), received_at)
            .await
            .expect("first begin")
        else {
            panic!("first begin should reserve a new action");
        };
        assert!(
            ledger
                .begin_or_replay(fingerprint.clone(), received_at)
                .await
                .is_err(),
            "fresh in-flight duplicate must fail closed"
        );

        ledger.release(action.clone()).await.expect("release");
        let IdempotencyDecision::New(mut action_after_release) = ledger
            .begin_or_replay(fingerprint.clone(), received_at)
            .await
            .expect("begin after release")
        else {
            panic!("released reservation should allow a new action");
        };
        action_after_release.mark_dispatched(ActionDispatchKind::NoOp);
        action_after_release.settle(ProductInboundAck::NoOp);
        ledger
            .settle(action_after_release.clone())
            .await
            .expect("settle action");

        let reopened = harness.reopened().expect("reopened product harness");
        let IdempotencyDecision::Replay(replayed) = reopened
            .idempotency_ledger()
            .begin_or_replay(fingerprint.clone(), received_at)
            .await
            .expect("replay settled action")
        else {
            panic!("settled action should replay forever");
        };
        assert_eq!(replayed.action_id, action_after_release.action_id);

        let expiring = harness.idempotency_ledger_with_ttl(Duration::from_millis(0));
        let expiring_fingerprint = fingerprint_for("event-ttl", "alice", "room-1");
        let IdempotencyDecision::New(first_expiring) = expiring
            .begin_or_replay(expiring_fingerprint.clone(), received_at)
            .await
            .expect("first expiring begin")
        else {
            panic!("first expiring begin should be new");
        };
        let IdempotencyDecision::New(second_expiring) = expiring
            .begin_or_replay(expiring_fingerprint, received_at)
            .await
            .expect("expired reservation should be reclaimed")
        else {
            panic!("expired reservation should produce a new action");
        };
        assert_ne!(first_expiring.action_id, second_expiring.action_id);
    }

    #[tokio::test]
    async fn idempotency_settle_errors() {
        let harness =
            RebornProductWorkflowHarness::filesystem_temp(product_scope("tenant-settle-errors"))
                .expect("product workflow harness");
        let ledger = harness.idempotency_ledger();
        let received_at = chrono::Utc::now();

        let missing = ironclaw_product_workflow::ProductInboundAction::begin(
            fingerprint_for("event-missing", "alice", "room-1"),
            received_at,
        );
        assert!(ledger.settle(missing).await.is_err());

        let fingerprint = fingerprint_for("event-stale", "alice", "room-1");
        let IdempotencyDecision::New(action) = ledger
            .begin_or_replay(fingerprint, received_at)
            .await
            .expect("reserve action")
        else {
            panic!("first begin should reserve a new action");
        };
        let mut stale = action.clone();
        stale.action_id = ProductActionId::new();
        assert!(ledger.settle(stale).await.is_err());
    }

    #[tokio::test]
    async fn idempotency_release_removes_dispatched_nonterminal_reservation() {
        let harness =
            RebornProductWorkflowHarness::filesystem_temp(product_scope("tenant-release"))
                .expect("product workflow harness");
        let ledger = harness.idempotency_ledger();
        let fingerprint = fingerprint_for("event-release", "alice", "room-1");
        let received_at = chrono::Utc::now();
        let IdempotencyDecision::New(mut action) = ledger
            .begin_or_replay(fingerprint.clone(), received_at)
            .await
            .expect("reserve action")
        else {
            panic!("first begin should reserve a new action");
        };
        action.mark_dispatched(ActionDispatchKind::NoOp);
        ledger.release(action).await.expect("release dispatched");

        let IdempotencyDecision::New(_) = ledger
            .begin_or_replay(fingerprint, received_at)
            .await
            .expect("released dispatched reservation should retry")
        else {
            panic!("released dispatched reservation should allow a new action");
        };
    }

    #[tokio::test]
    async fn idempotency_release_ignores_settled_and_stale_actions() {
        let harness =
            RebornProductWorkflowHarness::filesystem_temp(product_scope("tenant-release-guard"))
                .expect("product workflow harness");
        let ledger = harness.idempotency_ledger();
        let received_at = chrono::Utc::now();

        let settled_fingerprint = fingerprint_for("event-release-settled", "alice", "room-1");
        let IdempotencyDecision::New(mut settled_action) = ledger
            .begin_or_replay(settled_fingerprint.clone(), received_at)
            .await
            .expect("reserve settled action")
        else {
            panic!("first begin should reserve a new action");
        };
        settled_action.mark_dispatched(ActionDispatchKind::NoOp);
        settled_action.settle(ProductInboundAck::NoOp);
        ledger
            .settle(settled_action.clone())
            .await
            .expect("settle action");
        ledger
            .release(settled_action.clone())
            .await
            .expect("release settled action");
        let IdempotencyDecision::Replay(replayed) = ledger
            .begin_or_replay(settled_fingerprint, received_at)
            .await
            .expect("settled action should replay")
        else {
            panic!("settled release should preserve replay");
        };
        assert_eq!(replayed.action_id, settled_action.action_id);

        let stale_fingerprint = fingerprint_for("event-release-stale", "alice", "room-1");
        let IdempotencyDecision::New(active_action) = ledger
            .begin_or_replay(stale_fingerprint.clone(), received_at)
            .await
            .expect("reserve active action")
        else {
            panic!("first begin should reserve a new action");
        };
        let mut stale_action = active_action.clone();
        stale_action.action_id = ProductActionId::new();
        ledger
            .release(stale_action)
            .await
            .expect("release stale action");
        assert!(
            ledger
                .begin_or_replay(stale_fingerprint, received_at)
                .await
                .is_err(),
            "stale release must leave active reservation in flight"
        );
    }

    #[tokio::test]
    async fn idempotency_ledger_invalid_lease_ttl() {
        let harness =
            RebornProductWorkflowHarness::filesystem_temp(product_scope("tenant-invalid-ttl"))
                .expect("product workflow harness");
        let ledger = harness.idempotency_ledger_with_ttl(Duration::MAX);
        let error = ledger
            .begin_or_replay(
                fingerprint_for("event-invalid-ttl", "alice", "room-1"),
                chrono::Utc::now(),
            )
            .await
            .expect_err("invalid ttl should fail");
        assert!(
            matches!(error, ProductWorkflowError::Transient { .. }),
            "unexpected error: {error:?}"
        );
    }

    #[tokio::test]
    async fn filesystem_idempotency_ledger_serializes_concurrent_begin() {
        let harness = RebornProductWorkflowHarness::filesystem_temp(product_scope("tenant-race"))
            .expect("product workflow harness");
        let fingerprint = fingerprint_for("event-race", "alice", "room-1");
        let received_at = chrono::Utc::now();
        let start = Arc::new(Barrier::new(2));

        let first_ledger = harness.idempotency_ledger();
        let first_start = Arc::clone(&start);
        let first_fingerprint = fingerprint.clone();
        let first = async move {
            first_start.wait().await;
            first_ledger
                .begin_or_replay(first_fingerprint, received_at)
                .await
        };

        let second_ledger = harness.idempotency_ledger();
        let second_start = Arc::clone(&start);
        let second = async move {
            second_start.wait().await;
            second_ledger
                .begin_or_replay(fingerprint, received_at)
                .await
        };

        let (first_result, second_result) = tokio::join!(first, second);
        let mut new_count = 0;
        let mut rejected_count = 0;
        for result in [first_result, second_result] {
            match result {
                Ok(IdempotencyDecision::New(_)) => new_count += 1,
                Err(ProductWorkflowError::Transient { .. }) => rejected_count += 1,
                other => panic!("unexpected concurrent idempotency result: {other:?}"),
            }
        }

        assert_eq!(new_count, 1);
        assert_eq!(rejected_count, 1);
    }

    #[tokio::test]
    async fn standalone_filesystem_idempotency_ledgers_share_serialization_lock() {
        let root = tempfile::tempdir().expect("tempdir");
        let backend = Arc::new(local_filesystem(root.path()).expect("local filesystem"));
        let scope = product_scope("tenant-standalone-race");
        let mounts = MountView::new(vec![MountGrant::new(
            MountAlias::new("/workflow").expect("valid workflow alias"),
            VirtualPath::new(format!(
                "/engine/tenants/{}/users/{}/standalone-workflow",
                scope.tenant_id, scope.user_id
            ))
            .expect("valid workflow target"),
            MountPermissions::read_write_list_delete(),
        )])
        .expect("mount view");
        let filesystem = Arc::new(ScopedFilesystem::with_fixed_view(backend, mounts));
        let fingerprint = fingerprint_for("event-standalone-race", "alice", "room-1");
        let received_at = chrono::Utc::now();
        let start = Arc::new(Barrier::new(2));

        let first_ledger = FilesystemIdempotencyLedger::new(
            Arc::clone(&filesystem),
            scope.clone(),
            Duration::from_secs(60),
        );
        let first_start = Arc::clone(&start);
        let first_fingerprint = fingerprint.clone();
        let first = async move {
            first_start.wait().await;
            first_ledger
                .begin_or_replay(first_fingerprint, received_at)
                .await
        };

        let second_ledger = FilesystemIdempotencyLedger::new(
            Arc::clone(&filesystem),
            scope,
            Duration::from_secs(60),
        );
        let second_start = Arc::clone(&start);
        let second = async move {
            second_start.wait().await;
            second_ledger
                .begin_or_replay(fingerprint, received_at)
                .await
        };

        let (first_result, second_result) = tokio::join!(first, second);
        let mut new_count = 0;
        let mut rejected_count = 0;
        for result in [first_result, second_result] {
            match result {
                Ok(IdempotencyDecision::New(_)) => new_count += 1,
                Err(ProductWorkflowError::Transient { .. }) => rejected_count += 1,
                other => panic!("unexpected concurrent standalone idempotency result: {other:?}"),
            }
        }

        assert_eq!(new_count, 1);
        assert_eq!(rejected_count, 1);
    }

    #[tokio::test]
    async fn shared_backend_product_workflow_harnesses_share_serialization_lock() {
        let root = Arc::new(tempfile::tempdir().expect("tempdir"));
        let backend = Arc::new(local_filesystem(root.path()).expect("local filesystem"));
        let scope = product_scope("tenant-shared-backend-race");
        let first_harness = RebornProductWorkflowHarness::filesystem_shared_backend(
            scope.clone(),
            Arc::clone(&backend),
            Arc::clone(&root),
        )
        .expect("first product workflow harness");
        let second_harness = RebornProductWorkflowHarness::filesystem_shared_backend(
            scope,
            Arc::clone(&backend),
            Arc::clone(&root),
        )
        .expect("second product workflow harness");
        let fingerprint = fingerprint_for("event-shared-backend-race", "alice", "room-1");
        let received_at = chrono::Utc::now();
        let start = Arc::new(Barrier::new(2));

        let first_ledger = first_harness.idempotency_ledger();
        let first_start = Arc::clone(&start);
        let first_fingerprint = fingerprint.clone();
        let first = async move {
            first_start.wait().await;
            first_ledger
                .begin_or_replay(first_fingerprint, received_at)
                .await
        };

        let second_ledger = second_harness.idempotency_ledger();
        let second_start = Arc::clone(&start);
        let second = async move {
            second_start.wait().await;
            second_ledger
                .begin_or_replay(fingerprint, received_at)
                .await
        };

        let (first_result, second_result) = tokio::join!(first, second);
        let mut new_count = 0;
        let mut rejected_count = 0;
        for result in [first_result, second_result] {
            match result {
                Ok(IdempotencyDecision::New(_)) => new_count += 1,
                Err(ProductWorkflowError::Transient { .. }) => rejected_count += 1,
                other => panic!("unexpected concurrent shared-backend result: {other:?}"),
            }
        }

        assert_eq!(new_count, 1);
        assert_eq!(rejected_count, 1);
    }

    #[tokio::test]
    async fn product_workflow_uses_filesystem_binding_and_idempotency_services() {
        let product_harness =
            RebornProductWorkflowHarness::filesystem_temp(product_scope("tenant-workflow"))
                .expect("product workflow harness");
        let thread_harness =
            RebornThreadHarness::filesystem_temp(thread_scope("workflow")).expect("thread harness");
        let coordinator = DryRunCapturingTurnCoordinator::default();
        let binding_service: Arc<dyn ConversationBindingService> =
            Arc::new(product_harness.binding_service().expect("binding service"));
        let inbound: Arc<dyn InboundTurnService> = Arc::new(DefaultInboundTurnService::new(
            Arc::clone(&binding_service),
            thread_harness.service_instance().expect("thread service"),
            coordinator.clone(),
        ));
        let ledger: Arc<dyn IdempotencyLedger> = Arc::new(product_harness.idempotency_ledger());
        let workflow = DefaultProductWorkflow::new(inbound, ledger, binding_service);
        let envelope = test_envelope("event-workflow", "alice", "room-workflow", "hi");

        let first = workflow
            .accept_inbound(envelope.clone())
            .await
            .expect("first accept");
        assert!(matches!(first, ProductInboundAck::Accepted { .. }));
        let duplicate = workflow.accept_inbound(envelope).await.expect("duplicate");
        assert!(matches!(duplicate, ProductInboundAck::Duplicate { .. }));
        assert_eq!(
            coordinator.submission_count(),
            1,
            "duplicate event should replay the settled workflow action without submitting again"
        );
    }

    #[tokio::test]
    async fn product_workflow_retries_after_filesystem_deferred_busy_release() {
        let product_harness =
            RebornProductWorkflowHarness::filesystem_temp(product_scope("tenant-workflow-busy"))
                .expect("product workflow harness");
        let thread_harness = RebornThreadHarness::filesystem_temp(thread_scope("workflow-busy"))
            .expect("thread harness");
        let coordinator = DryRunCapturingTurnCoordinator::default();
        coordinator.set_busy(TurnRunId::new());
        let binding_service: Arc<dyn ConversationBindingService> =
            Arc::new(product_harness.binding_service().expect("binding service"));
        let inbound: Arc<dyn InboundTurnService> = Arc::new(DefaultInboundTurnService::new(
            Arc::clone(&binding_service),
            thread_harness.service_instance().expect("thread service"),
            coordinator.clone(),
        ));
        let ledger: Arc<dyn IdempotencyLedger> = Arc::new(product_harness.idempotency_ledger());
        let workflow = DefaultProductWorkflow::new(inbound, ledger, binding_service);
        let envelope = test_envelope("event-workflow-busy", "alice", "room-workflow-busy", "hi");

        let first = workflow
            .accept_inbound(envelope.clone())
            .await
            .expect("busy accept");
        assert!(matches!(first, ProductInboundAck::DeferredBusy { .. }));
        assert_eq!(coordinator.submission_count(), 1);

        coordinator.set_accepting();
        let second = workflow
            .accept_inbound(envelope)
            .await
            .expect("retry submit");
        assert!(matches!(second, ProductInboundAck::Accepted { .. }));
        assert_eq!(
            coordinator.submission_count(),
            2,
            "DeferredBusy must release the idempotency reservation for immediate retry"
        );
    }

    #[test]
    fn test_adapter_parses_payload_without_minting_trusted_context() {
        let adapter = RebornTestProductAdapter::new("reborn-test", "install-1").expect("adapter");
        let evidence = ProtocolAuthEvidence::test_verified(AuthRequirement::BearerToken, "alice");
        let raw = RebornTestProductAdapter::text_payload("event-1", "alice", "thread-1", "hi")
            .expect("payload");
        let parsed = adapter
            .parse_inbound(&raw, &evidence)
            .expect("parsed inbound");
        assert_eq!(parsed.external_event_id.as_str(), "event-1");

        let failed =
            ProtocolAuthEvidence::failed(ironclaw_product_adapters::ProtocolAuthFailure::Missing);
        assert!(adapter.parse_inbound(&raw, &failed).is_err());
    }

    #[test]
    fn test_adapter_rejects_actor_spoofing_against_verified_subject() {
        let adapter = RebornTestProductAdapter::new("reborn-test", "install-1").expect("adapter");
        let evidence = ProtocolAuthEvidence::test_verified(AuthRequirement::BearerToken, "alice");
        let raw = RebornTestProductAdapter::text_payload("event-1", "bob", "thread-1", "hi")
            .expect("payload");
        let error = adapter
            .parse_inbound(&raw, &evidence)
            .expect_err("actor mismatch should reject");
        assert!(matches!(error, ProductAdapterError::Authentication(_)));
    }

    #[test]
    fn test_adapter_rejects_subscription_actor_spoofing_against_verified_subject() {
        let adapter = RebornTestProductAdapter::new("reborn-test", "install-1").expect("adapter");
        let evidence = ProtocolAuthEvidence::test_verified(AuthRequirement::BearerToken, "alice");
        let raw = RebornTestProductAdapter::subscription_payload(
            "event-sub-1",
            "bob",
            "thread-1",
            None,
            None,
        )
        .expect("subscription payload");
        let error = adapter
            .parse_inbound(&raw, &evidence)
            .expect_err("subscription actor mismatch should reject");
        assert!(matches!(error, ProductAdapterError::Authentication(_)));
    }

    #[tokio::test]
    async fn projection_subscription_resolution_and_stream_are_scope_isolated() {
        let scope_a = resource_scope(
            TenantId::new("tenant-projection-a").unwrap(),
            UserId::new("host-user").unwrap(),
            AgentId::new("agent-projection").unwrap(),
            Some(ProjectId::new("project-projection").unwrap()),
        );
        let scope_b = resource_scope(
            TenantId::new("tenant-projection-b").unwrap(),
            UserId::new("host-user").unwrap(),
            AgentId::new("agent-projection").unwrap(),
            Some(ProjectId::new("project-projection").unwrap()),
        );
        let product_a = RebornProductWorkflowHarness::filesystem_temp(scope_a)
            .expect("tenant A product harness");
        let product_b = product_a
            .with_scope(scope_b)
            .expect("tenant B product harness");
        let ingress = RebornTestIngress::new(
            RebornTestProductAdapter::new("reborn-test", "install-1").expect("adapter"),
        );

        let binding_a = product_a
            .binding_service()
            .expect("tenant A binding service")
            .resolve_binding(binding_request(
                "event-projection-a",
                "alice",
                "room-projection",
            ))
            .await
            .expect("tenant A binding");
        let binding_b = product_b
            .binding_service()
            .expect("tenant B binding service")
            .resolve_binding(binding_request(
                "event-projection-b",
                "alice",
                "room-projection",
            ))
            .await
            .expect("tenant B binding");
        assert_ne!(binding_a.tenant_id, binding_b.tenant_id);
        assert_ne!(binding_a.thread_id, binding_b.thread_id);

        let workflow_a = projection_workflow(&product_a, "projection-a");
        let workflow_b = projection_workflow(&product_b, "projection-b");
        let envelope_a = ingress
            .verified_subscription_envelope(
                "event-sub-a",
                "alice",
                "room-projection",
                Some(binding_a.thread_id.as_str()),
                Some(ProjectionCursor::new("cursor:tenant-a:0").expect("cursor")),
            )
            .expect("tenant A subscription envelope");
        let envelope_b = ingress
            .verified_subscription_envelope(
                "event-sub-b",
                "alice",
                "room-projection",
                Some(binding_b.thread_id.as_str()),
                Some(ProjectionCursor::new("cursor:tenant-b:0").expect("cursor")),
            )
            .expect("tenant B subscription envelope");

        let subscription_a = workflow_a
            .resolve_projection_subscription(envelope_a)
            .await
            .expect("tenant A subscription");
        let subscription_b = workflow_b
            .resolve_projection_subscription(envelope_b)
            .await
            .expect("tenant B subscription");
        assert_eq!(subscription_a.scope.tenant_id, binding_a.tenant_id);
        assert_eq!(subscription_b.scope.tenant_id, binding_b.tenant_id);
        assert_eq!(subscription_a.scope.thread_id, binding_a.thread_id);
        assert_eq!(subscription_b.scope.thread_id, binding_b.thread_id);
        assert_ne!(subscription_a.scope, subscription_b.scope);
        assert_ne!(subscription_a.actor.user_id, subscription_b.actor.user_id);

        let stream = FakeProjectionStream::new();
        let target_a = ReplyTargetBindingRef::new("reply:projection-a").unwrap();
        let target_b = ReplyTargetBindingRef::new("reply:projection-b").unwrap();
        stream.push_for_request(
            subscription_a.clone(),
            projection_update_envelope(
                &ingress,
                target_a,
                binding_a.thread_id.as_str(),
                "tenant A event",
            ),
        );
        stream.push_for_request(
            subscription_b.clone(),
            projection_update_envelope(
                &ingress,
                target_b,
                binding_b.thread_id.as_str(),
                "tenant B event",
            ),
        );

        let drained_a = stream
            .drain(subscription_a.clone())
            .await
            .expect("drain tenant A");
        assert_eq!(drained_a.len(), 1);
        assert_projection_contains_only(&drained_a, "tenant A event", "tenant B event");

        let empty_a_replay = stream
            .drain(subscription_a)
            .await
            .expect("tenant A replay after drain");
        assert!(empty_a_replay.is_empty());

        let drained_b = stream.drain(subscription_b).await.expect("drain tenant B");
        assert_eq!(drained_b.len(), 1);
        assert_projection_contains_only(&drained_b, "tenant B event", "tenant A event");
    }

    #[tokio::test]
    async fn projection_subscription_rejects_cross_thread_hint() {
        let scope_a = resource_scope(
            TenantId::new("tenant-projection-hint-a").unwrap(),
            UserId::new("host-user").unwrap(),
            AgentId::new("agent-projection").unwrap(),
            Some(ProjectId::new("project-projection").unwrap()),
        );
        let scope_b = resource_scope(
            TenantId::new("tenant-projection-hint-b").unwrap(),
            UserId::new("host-user").unwrap(),
            AgentId::new("agent-projection").unwrap(),
            Some(ProjectId::new("project-projection").unwrap()),
        );
        let product_a = RebornProductWorkflowHarness::filesystem_temp(scope_a)
            .expect("tenant A product harness");
        let product_b = product_a
            .with_scope(scope_b)
            .expect("tenant B product harness");
        let binding_a = product_a
            .binding_service()
            .expect("tenant A binding service")
            .resolve_binding(binding_request(
                "event-projection-hint-a",
                "alice",
                "room-projection-hint",
            ))
            .await
            .expect("tenant A binding");
        let binding_b = product_b
            .binding_service()
            .expect("tenant B binding service")
            .resolve_binding(binding_request(
                "event-projection-hint-b",
                "alice",
                "room-projection-hint",
            ))
            .await
            .expect("tenant B binding");
        let workflow_a = projection_workflow(&product_a, "projection-hint-a");
        let ingress = RebornTestIngress::new(
            RebornTestProductAdapter::new("reborn-test", "install-1").expect("adapter"),
        );
        let cross_hint = ingress
            .verified_subscription_envelope(
                "event-sub-cross-hint",
                "alice",
                "room-projection-hint",
                Some(binding_b.thread_id.as_str()),
                None,
            )
            .expect("cross hint envelope");

        let error = workflow_a
            .resolve_projection_subscription(cross_hint)
            .await
            .expect_err("tenant A subscription must reject tenant B thread hint");
        assert!(matches!(
            error,
            ProductAdapterError::WorkflowRejected { .. }
        ));
        assert_ne!(binding_a.thread_id, binding_b.thread_id);
    }

    #[test]
    fn test_adapter_rejects_malformed_inbound_payload() {
        let adapter = RebornTestProductAdapter::new("reborn-test", "install-1").expect("adapter");
        let evidence = ProtocolAuthEvidence::test_verified(AuthRequirement::BearerToken, "alice");
        let error = adapter
            .parse_inbound(b"not valid json", &evidence)
            .expect_err("malformed payload");
        assert!(matches!(
            error,
            ProductAdapterError::MalformedInboundPayload { .. }
        ));
    }

    #[test]
    fn test_adapter_rejects_semantically_invalid_inbound_payload() {
        let adapter = RebornTestProductAdapter::new("reborn-test", "install-1").expect("adapter");
        let evidence = ProtocolAuthEvidence::test_verified(AuthRequirement::BearerToken, "alice");
        let invalid_payloads = [
            serde_json::json!({
                "event_id": "",
                "user_id": "alice",
                "thread_id": "thread-1",
                "text": "hi",
            }),
            serde_json::json!({
                "event_id": "event-1",
                "user_id": "bad\u{0000}user",
                "thread_id": "thread-1",
                "text": "hi",
            }),
            serde_json::json!({
                "event_id": "event-1",
                "user_id": "alice",
                "thread_id": "",
                "text": "hi",
            }),
            serde_json::json!({
                "event_id": "event-1",
                "user_id": "alice",
                "thread_id": "thread-1",
                "text": "bad\u{0000}text",
            }),
        ];

        for payload in invalid_payloads {
            let raw = serde_json::to_vec(&payload).expect("valid json payload");
            assert!(
                adapter.parse_inbound(&raw, &evidence).is_err(),
                "payload should be rejected: {payload}"
            );
        }
    }

    #[test]
    fn test_ingress_stamps_trusted_context_outside_adapter() {
        let adapter = RebornTestProductAdapter::new("reborn-test", "install-1").expect("adapter");
        let ingress = RebornTestIngress::new(adapter);
        assert_eq!(ingress.adapter().adapter_id().as_str(), "reborn-test");
        let envelope = ingress
            .verified_text_envelope("event-1", "alice", "thread-1", "hi")
            .expect("trusted envelope");
        assert_eq!(envelope.external_event_id().as_str(), "event-1");
        assert_eq!(envelope.adapter_id().as_str(), "reborn-test");

        let raw = RebornTestProductAdapter::text_payload("event-2", "alice", "thread-1", "hi")
            .expect("payload");
        assert!(ingress.failed_auth_payload(&raw).is_err());
    }

    #[tokio::test]
    async fn test_adapter_render_outbound_records_delivery() {
        let adapter = RebornTestProductAdapter::new("reborn-test", "install-1").expect("adapter");
        let target_ref = ReplyTargetBindingRef::new("reply-target").expect("reply target");
        let envelope = ProductOutboundEnvelope::new(
            adapter.adapter_id().clone(),
            adapter.installation_id().clone(),
            ProductOutboundTarget::new(
                target_ref.clone(),
                ExternalConversationRef::new(None, "room-1", None, None).expect("conversation ref"),
                None,
            ),
            ProjectionCursor::new("cursor:1").expect("projection cursor"),
            ProductOutboundPayload::FinalReply(FinalReplyView {
                turn_run_id: TurnRunId::new(),
                text: "hello".to_string(),
                generated_at: chrono::Utc::now(),
            }),
        );
        let attempt_id = envelope.delivery_attempt_id;
        let sink = RecordingOutboundDeliverySink::new();
        let egress = FakeProtocolHttpEgress::new(["api.example.test".to_string()]);

        let outcome = adapter
            .render_outbound(envelope, &egress, &sink)
            .await
            .expect("render outbound");

        assert_eq!(outcome, ProductRenderOutcome::DeliveryRecorded);
        let statuses = sink.statuses();
        assert_eq!(statuses.len(), 1);
        assert!(matches!(
            &statuses[0],
            DeliveryStatus::Delivered {
                attempt_id: actual_attempt,
                target,
                run_id: None,
            } if *actual_attempt == attempt_id && target == &target_ref
        ));
    }

    fn model_request(messages: Vec<HostManagedModelMessage>) -> HostManagedModelRequest {
        HostManagedModelRequest {
            model_profile_id: ModelProfileId::new("test_model").expect("model profile"),
            messages,
            surface_version: None,
            resolved_model_route: None,
            run_id: TurnRunId::new(),
            turn_id: TurnId::new(),
        }
    }

    async fn execute_recorded_get(
        transport: &RecordingNetworkHttpTransport,
    ) -> Result<NetworkHttpResponse, NetworkHttpError> {
        transport
            .execute(NetworkTransportRequest {
                method: NetworkMethod::Get,
                url: "https://api.example.test/v1".to_string(),
                headers: vec![],
                body: Vec::new(),
                resolved_ips: vec![IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34))],
                response_body_limit: None,
                timeout_ms: None,
            })
            .await
    }

    fn tool_result_message(
        provider_call_id: &str,
        provider_tool_name: &str,
        content: &str,
    ) -> HostManagedModelMessage {
        tool_result_message_with_capability_id(
            provider_call_id,
            provider_tool_name,
            provider_tool_name,
            content,
        )
    }

    fn tool_result_message_with_capability_id(
        provider_call_id: &str,
        provider_tool_name: &str,
        capability_id: &str,
        content: &str,
    ) -> HostManagedModelMessage {
        HostManagedModelMessage {
            role: HostManagedModelMessageRole::ToolResult,
            content: content.to_string(),
            content_ref: LoopMessageRef::new("msg:tool.result.1").expect("message ref"),
            tool_result_provider_call: Some(ProviderToolCallReferenceEnvelope {
                provider_id: "trace_replay".to_string(),
                provider_model_id: "trace_replay".to_string(),
                provider_turn_id: "trace-turn".to_string(),
                provider_call_id: provider_call_id.to_string(),
                provider_tool_name: provider_tool_name.to_string(),
                capability_id: CapabilityId::new(capability_id).expect("capability id"),
                arguments: serde_json::json!({}),
                response_reasoning: None,
                reasoning: None,
                signature: None,
            }),
            tool_result_content: Some(HostManagedToolResultContent::Resolved {
                safe_summary: ToolResultSafeSummary::new("tool completed").expect("safe summary"),
            }),
        }
    }

    fn thread_scope(label: &str) -> ThreadScope {
        ThreadScope {
            tenant_id: TenantId::new("tenant-reborn-support").unwrap(),
            agent_id: AgentId::new("agent-reborn-support").unwrap(),
            project_id: None,
            owner_user_id: Some(UserId::new("user-reborn-support").unwrap()),
            mission_id: None,
        }
        .with_thread_label(label)
    }

    async fn write_thread_history(harness: &RebornThreadHarness) -> ThreadId {
        let suffix = harness
            .scope
            .agent_id
            .as_str()
            .strip_prefix("agent-")
            .unwrap_or("support");
        let thread_id = ThreadId::new(format!("thread-{suffix}")).unwrap();
        harness
            .service
            .ensure_thread(EnsureThreadRequest {
                scope: harness.scope.clone(),
                thread_id: Some(thread_id.clone()),
                created_by_actor_id: "alice".to_string(),
                title: Some("Reborn support".to_string()),
                metadata_json: None,
            })
            .await
            .expect("ensure thread");
        harness
            .service
            .accept_inbound_message(AcceptInboundMessageRequest {
                scope: harness.scope.clone(),
                thread_id: thread_id.clone(),
                actor_id: "alice".to_string(),
                source_binding_id: Some("source".to_string()),
                reply_target_binding_id: Some("reply".to_string()),
                external_event_id: Some(format!("event-{suffix}")),
                content: MessageContent::text("hello"),
            })
            .await
            .expect("accept inbound");
        let draft = harness
            .service
            .append_assistant_draft(AppendAssistantDraftRequest {
                scope: harness.scope.clone(),
                thread_id: thread_id.clone(),
                turn_run_id: "run-1".to_string(),
                content: MessageContent::text("draft"),
            })
            .await
            .expect("append draft");
        harness
            .service
            .finalize_assistant_message(
                &harness.scope,
                &thread_id,
                draft.message_id,
                MessageContent::text("assistant reply"),
            )
            .await
            .expect("finalize draft");
        thread_id
    }

    trait ThreadScopeLabelExt {
        fn with_thread_label(self, label: &str) -> Self;
    }

    impl ThreadScopeLabelExt for ThreadScope {
        fn with_thread_label(mut self, label: &str) -> Self {
            self.agent_id = AgentId::new(format!("agent-{label}")).unwrap();
            self
        }
    }

    #[derive(Clone)]
    struct StaticResolver(Vec<IpAddr>);

    impl NetworkResolver for StaticResolver {
        fn resolve_ips(&self, _host: &str, _port: u16) -> Result<Vec<IpAddr>, NetworkHttpError> {
            Ok(self.0.clone())
        }
    }

    fn policy(
        host_pattern: &str,
        port: Option<u16>,
        deny_private_ip_ranges: bool,
        max_egress_bytes: Option<u64>,
    ) -> NetworkPolicy {
        NetworkPolicy {
            allowed_targets: vec![NetworkTargetPattern {
                scheme: None,
                host_pattern: host_pattern.to_string(),
                port,
            }],
            deny_private_ip_ranges,
            max_egress_bytes,
        }
    }

    fn sample_scope() -> ResourceScope {
        ResourceScope {
            tenant_id: TenantId::new("tenant1").unwrap(),
            user_id: UserId::new("user1").unwrap(),
            agent_id: None,
            project_id: None,
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        }
    }

    fn product_scope(tenant: &str) -> ResourceScope {
        resource_scope(
            TenantId::new(tenant).unwrap(),
            UserId::new("host-user").unwrap(),
            AgentId::new("agent-product").unwrap(),
            Some(ProjectId::new("project-product").unwrap()),
        )
    }

    fn projection_workflow(
        product_harness: &RebornProductWorkflowHarness,
        thread_label: &str,
    ) -> DefaultProductWorkflow {
        let binding_service: Arc<dyn ConversationBindingService> =
            Arc::new(product_harness.binding_service().expect("binding service"));
        let thread_harness = RebornThreadHarness::filesystem_temp(thread_scope(thread_label))
            .expect("thread harness");
        let inbound: Arc<dyn InboundTurnService> = Arc::new(DefaultInboundTurnService::new(
            Arc::clone(&binding_service),
            thread_harness.service_instance().expect("thread service"),
            DryRunCapturingTurnCoordinator::default(),
        ));
        let ledger: Arc<dyn IdempotencyLedger> = Arc::new(product_harness.idempotency_ledger());
        DefaultProductWorkflow::new(inbound, ledger, binding_service)
    }

    fn projection_update_envelope(
        ingress: &RebornTestIngress,
        target_ref: ReplyTargetBindingRef,
        thread_id: &str,
        body: &str,
    ) -> ProductOutboundEnvelope {
        ProductOutboundEnvelope::new(
            ingress.adapter().adapter_id().clone(),
            ingress.adapter().installation_id().clone(),
            ProductOutboundTarget::new(
                target_ref,
                ExternalConversationRef::new(None, "room-projection", None, None)
                    .expect("conversation ref"),
                None,
            ),
            ProjectionCursor::new(format!("cursor:{thread_id}:{body}")).expect("cursor"),
            ProductOutboundPayload::ProjectionUpdate {
                state: ProductProjectionState::new(
                    thread_id,
                    vec![ProductProjectionItem::Text {
                        id: format!("item:{thread_id}"),
                        body: body.to_string(),
                    }],
                )
                .expect("projection state"),
            },
        )
    }

    fn assert_projection_contains_only(
        envelopes: &[ProductOutboundEnvelope],
        expected: &str,
        forbidden: &str,
    ) {
        let rendered = serde_json::to_string(envelopes).expect("serialize projection envelopes");
        assert!(
            rendered.contains(expected),
            "projection stream should include expected event {expected:?}: {rendered}"
        );
        assert!(
            !rendered.contains(forbidden),
            "projection stream must not leak forbidden event {forbidden:?}: {rendered}"
        );
    }

    fn test_envelope(
        event_id: &str,
        user_id: &str,
        thread_id: &str,
        text: &str,
    ) -> ironclaw_product_adapters::ProductInboundEnvelope {
        let adapter = RebornTestProductAdapter::new("reborn-test", "install-1").expect("adapter");
        RebornTestIngress::new(adapter)
            .verified_text_envelope(event_id, user_id, thread_id, text)
            .expect("trusted envelope")
    }

    fn binding_request(event_id: &str, user_id: &str, thread_id: &str) -> ResolveBindingRequest {
        let envelope = test_envelope(event_id, user_id, thread_id, "hi");
        ResolveBindingRequest {
            adapter_id: envelope.adapter_id().clone(),
            installation_id: envelope.installation_id().clone(),
            external_actor_ref: envelope.external_actor_ref().clone(),
            external_conversation_ref: envelope.external_conversation_ref().clone(),
            external_event_id: envelope.external_event_id().clone(),
            route_kind: ProductConversationRouteKind::Direct,
            auth_claim: envelope.auth_claim().clone(),
        }
    }

    fn fingerprint_for(event_id: &str, user_id: &str, thread_id: &str) -> ActionFingerprintKey {
        let envelope = test_envelope(event_id, user_id, thread_id, "hi");
        ActionFingerprintKey::new(
            envelope.adapter_id().clone(),
            envelope.installation_id().clone(),
            envelope.external_actor_ref().clone(),
            SourceBindingKey::new(envelope.source_binding_key()).expect("source binding key"),
            envelope.external_event_id().clone(),
        )
    }

    /// Documented dry-run substitution: this test exercises the real product
    /// workflow, binding service, idempotency ledger, and thread service while
    /// capturing the final turn submission instead of starting the Reborn
    /// runtime loop or routing product traffic.
    #[derive(Clone)]
    struct DryRunCapturingTurnCoordinator {
        submissions: Arc<Mutex<Vec<SubmitTurnRequest>>>,
        outcome: Arc<Mutex<DryRunTurnOutcome>>,
    }

    #[derive(Clone, Copy)]
    enum DryRunTurnOutcome {
        Accepted,
        Busy { active_run_id: TurnRunId },
    }

    impl Default for DryRunCapturingTurnCoordinator {
        fn default() -> Self {
            Self {
                submissions: Arc::new(Mutex::new(Vec::new())),
                outcome: Arc::new(Mutex::new(DryRunTurnOutcome::Accepted)),
            }
        }
    }

    impl DryRunCapturingTurnCoordinator {
        fn set_accepting(&self) {
            *self
                .outcome
                .lock()
                .expect("capturing coordinator outcome lock poisoned") =
                DryRunTurnOutcome::Accepted;
        }

        fn set_busy(&self, active_run_id: TurnRunId) {
            *self
                .outcome
                .lock()
                .expect("capturing coordinator outcome lock poisoned") =
                DryRunTurnOutcome::Busy { active_run_id };
        }

        fn submission_count(&self) -> usize {
            self.submissions
                .lock()
                .expect("capturing coordinator submissions lock poisoned")
                .len()
        }
    }

    #[async_trait]
    impl TurnCoordinator for DryRunCapturingTurnCoordinator {
        async fn prepare_turn(&self, _scope: TurnScope) -> Result<TurnRunId, TurnError> {
            Ok(TurnRunId::new())
        }

        async fn submit_turn(
            &self,
            request: SubmitTurnRequest,
        ) -> Result<SubmitTurnResponse, TurnError> {
            self.submissions
                .lock()
                .expect("capturing coordinator submissions lock poisoned")
                .push(request.clone());
            match *self
                .outcome
                .lock()
                .expect("capturing coordinator outcome lock poisoned")
            {
                DryRunTurnOutcome::Accepted => Ok(SubmitTurnResponse::Accepted {
                    turn_id: TurnId::new(),
                    run_id: TurnRunId::new(),
                    status: TurnStatus::Queued,
                    resolved_run_profile_id: RunProfileId::default_profile(),
                    resolved_run_profile_version: RunProfileVersion::new(1),
                    event_cursor: EventCursor::default(),
                    accepted_message_ref: request.accepted_message_ref,
                    reply_target_binding_ref: request.reply_target_binding_ref,
                }),
                DryRunTurnOutcome::Busy { active_run_id } => {
                    Err(TurnError::ThreadBusy(ThreadBusy {
                        active_run_id,
                        status: TurnStatus::Running,
                        event_cursor: EventCursor::default(),
                    }))
                }
            }
        }

        async fn resume_turn(
            &self,
            _request: ResumeTurnRequest,
        ) -> Result<ResumeTurnResponse, TurnError> {
            panic!("resume_turn is not used by reborn support tests")
        }

        async fn cancel_run(
            &self,
            _request: CancelRunRequest,
        ) -> Result<CancelRunResponse, TurnError> {
            panic!("cancel_run is not used by reborn support tests")
        }

        async fn get_run_state(
            &self,
            _request: GetRunStateRequest,
        ) -> Result<TurnRunState, TurnError> {
            panic!("get_run_state is not used by reborn support tests")
        }
    }
}

// ---------------------------------------------------------------------------
// trace_llm
// ---------------------------------------------------------------------------

mod trace_llm_tests {
    use crate::support::trace_llm::*;
    use ironclaw_llm::{
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

    /// Hint matching must be case-insensitive: a hint of "write" should match
    /// a user message starting with "Write". Regression test for the bug where
    /// case-sensitive `contains` left hinted steps permanently stuck at the
    /// queue head while unhinted steps were consumed out of order.
    #[tokio::test]
    async fn hint_matching_is_case_insensitive() {
        let trace = LlmTrace::single_turn(
            "test-model",
            "Write a file",
            vec![
                TraceStep {
                    request_hint: Some(RequestHint {
                        last_user_message_contains: Some("write".to_string()),
                        min_message_count: None,
                    }),
                    response: TraceResponse::Text {
                        content: "hinted step".to_string(),
                        input_tokens: 10,
                        output_tokens: 5,
                    },
                    expected_tool_results: Vec::new(),
                },
                TraceStep {
                    request_hint: None,
                    response: TraceResponse::Text {
                        content: "unhinted step".to_string(),
                        input_tokens: 10,
                        output_tokens: 5,
                    },
                    expected_tool_results: Vec::new(),
                },
            ],
        );
        let llm = TraceLlm::from_trace(trace);

        // The hinted step should match "Write" (capital W) against hint "write".
        let resp = llm
            .complete_with_tools(make_request("Write a file"))
            .await
            .unwrap();
        assert_eq!(
            resp.content.as_deref(),
            Some("hinted step"),
            "hinted step should be selected (case-insensitive match)"
        );
        assert_eq!(llm.hint_mismatches(), 0);
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
    async fn complete_skips_tool_calls_step() {
        // complete() is called in force_text mode where tools aren't available.
        // When the trace has a ToolCalls step followed by a Text step, complete()
        // should skip the ToolCalls and return the Text response.
        let trace = LlmTrace::single_turn(
            "test-model",
            "hi",
            vec![
                tool_calls_step(vec![simple_tool_call("echo")], 10, 5),
                text_step("skipped past tools", 20, 8),
            ],
        );
        let llm = TraceLlm::from_trace(trace);

        let resp = llm
            .complete(make_completion_request("hi"))
            .await
            .expect("complete() should skip ToolCalls and return the Text step");

        assert_eq!(resp.content, "skipped past tools");
        assert_eq!(resp.input_tokens, 20);
        assert_eq!(resp.output_tokens, 8);
        assert_eq!(resp.finish_reason, FinishReason::Stop);
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
