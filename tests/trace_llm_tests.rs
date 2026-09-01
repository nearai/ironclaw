mod support;

use crate::support::trace_llm::{LlmTrace, TraceLlm, TraceResponse, TraceStep, TraceToolCall};
use ironclaw_llm::{ChatMessage, LlmProvider, ToolCompletionRequest};

#[tokio::test]
async fn trace_llm_resolves_exact_result_binding_through_provider_call() {
    let trace = LlmTrace::single_turn(
        "trace-test",
        "create and read",
        vec![TraceStep {
            request_hint: None,
            response: TraceResponse::ToolCalls {
                tool_calls: vec![TraceToolCall {
                    id: "call_read".to_string(),
                    name: "google-docs__read_content".to_string(),
                    arguments: serde_json::json!({
                        "document_id": {
                            "$trace_result": {
                                "tool_call_id": "call_create",
                                "pointer": "/document/id"
                            }
                        }
                    }),
                }],
                input_tokens: 1,
                output_tokens: 1,
            },
            expected_tool_results: Vec::new(),
        }],
    );
    let provider = TraceLlm::from_trace(trace);
    let error = provider
        .complete_with_tools(ToolCompletionRequest::new(
            vec![ChatMessage::user("create and read")],
            Vec::new(),
        ))
        .await
        .expect_err("missing tool evidence should reject the replay step");
    assert!(error.to_string().contains("call_create"));
    assert_eq!(
        provider.calls(),
        0,
        "failed binding must not consume the step"
    );

    let request = ToolCompletionRequest::new(
        vec![
            ChatMessage::user("create and read"),
            ChatMessage::tool_result(
                "call_create",
                "google-docs__create_document",
                r#"{"document":{"id":"fresh-document"}}"#,
            ),
            ChatMessage::tool_result(
                "call_similar",
                "google-docs__create_document",
                r#"{"document":{"id":"wrong-document"}}"#,
            ),
        ],
        Vec::new(),
    );

    let response = provider
        .complete_with_tools(request)
        .await
        .expect("trace response should resolve");

    assert_eq!(
        response.tool_calls[0].arguments,
        serde_json::json!({"document_id": "fresh-document"})
    );
}

#[tokio::test]
async fn trace_llm_advertises_no_context_window_by_default() {
    let provider = TraceLlm::from_trace(LlmTrace::single_turn(
        "trace-window-default",
        "hello",
        Vec::new(),
    ));

    let metadata = provider.model_metadata().await.expect("metadata");

    assert_eq!(metadata.context_length, None);
    assert_eq!(metadata.id, "trace-window-default");
}

#[tokio::test]
async fn trace_llm_advertises_a_configured_context_window() {
    let provider = TraceLlm::from_trace(LlmTrace::single_turn(
        "trace-window-configured",
        "hello",
        Vec::new(),
    ))
    .with_advertised_context_window(40_000);

    let metadata = provider.model_metadata().await.expect("metadata");

    assert_eq!(metadata.context_length, Some(40_000));
    assert_eq!(metadata.id, "trace-window-configured");
}
