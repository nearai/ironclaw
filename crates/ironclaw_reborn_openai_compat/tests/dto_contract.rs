use ironclaw_reborn_openai_compat::{
    OpenAiChatCompletionChunk, OpenAiChatCompletionId, OpenAiChatCompletionRequest,
    OpenAiChatCompletionResponse, OpenAiChatFinishReason, OpenAiChatMessageRole,
    OpenAiCompatErrorCode, OpenAiCompatErrorKind, OpenAiResponseErrorObject, OpenAiResponseId,
    OpenAiResponseObject, OpenAiResponseOutputItem, OpenAiResponseOutputItemStatus,
    OpenAiResponseStatus, OpenAiResponseUsage, OpenAiResponsesCreateRequest, OpenAiResponsesInput,
    OpenAiResponsesInputItem, OpenAiResponsesMessageRole,
};
use serde_json::json;

#[test]
fn chat_completion_request_round_trips_explicit_compat_fields() {
    let request: OpenAiChatCompletionRequest = serde_json::from_value(json!({
        "model": "gpt-reborn",
        "messages": [
            {"role": "developer", "content": "follow product policy"},
            {"role": "user", "content": "hello"}
        ],
        "stream": true,
        "tools": [{
            "type": "function",
            "function": {
                "name": "lookup_order",
                "description": "Look up an order",
                "parameters": {"type": "object"},
                "strict": true
            }
        }],
        "tool_choice": {"type": "function", "function": {"name": "lookup_order"}},
        "future_openai_option": "ignored until explicitly supported"
    }))
    .expect("chat request");

    assert_eq!(request.model, "gpt-reborn");
    assert_eq!(request.messages[0].role, OpenAiChatMessageRole::Developer);
    assert_eq!(request.stream, Some(true));
    assert_eq!(
        request.tools.as_ref().expect("tools")[0].function.name,
        "lookup_order"
    );

    let serialized = serde_json::to_value(&request).expect("serialize request");
    assert!(serialized.get("future_openai_option").is_none());
}

#[test]
fn responses_create_request_accepts_text_or_item_input() {
    let text: OpenAiResponsesCreateRequest = serde_json::from_value(json!({
        "model": "gpt-reborn",
        "input": "hello",
        "stream": false,
        "previous_response_id": "resp_previous"
    }))
    .expect("text input");
    assert!(matches!(text.input, OpenAiResponsesInput::Text(_)));
    assert_eq!(
        text.previous_response_id
            .as_ref()
            .expect("previous response")
            .as_str(),
        "resp_previous"
    );

    let items: OpenAiResponsesCreateRequest = serde_json::from_value(json!({
        "model": "gpt-reborn",
        "input": [{
            "type": "message",
            "role": "user",
            "content": [{"type": "input_text", "text": "hello"}]
        }],
        "tools": [{"type": "web_search_preview"}],
        "tool_choice": "auto"
    }))
    .expect("item input");
    match items.input {
        OpenAiResponsesInput::Items(items) => {
            assert!(matches!(
                items[0],
                OpenAiResponsesInputItem::Message {
                    role: OpenAiResponsesMessageRole::User,
                    ..
                }
            ));
        }
        OpenAiResponsesInput::Text(_) => panic!("expected item input"),
    }
    assert_eq!(items.tools.as_ref().expect("tools").len(), 1);

    let invalid_previous_response = serde_json::from_value::<OpenAiResponsesCreateRequest>(json!({
        "model": "gpt-reborn",
        "input": "hello",
        "previous_response_id": "chatcmpl-not-a-response"
    }))
    .expect_err("previous_response_id must be a typed response ref");
    assert!(
        invalid_previous_response
            .to_string()
            .contains("expected OpenAI-compatible prefix")
    );
}

#[test]
fn responses_items_are_tagged_and_tolerate_future_request_fields() {
    let function_call: OpenAiResponsesInputItem = serde_json::from_value(json!({
        "type": "function_call",
        "call_id": "call_1",
        "name": "lookup_order",
        "arguments": "{\"id\":\"123\"}"
    }))
    .expect("function call input item");
    assert!(matches!(
        function_call,
        OpenAiResponsesInputItem::FunctionCall { .. }
    ));

    let message = serde_json::from_value::<OpenAiResponsesInputItem>(json!({
        "type": "message",
        "role": "user",
        "content": "hello",
        "future_openai_item_field": {"ignored": true}
    }))
    .expect("future request fields must not break deserialization");
    assert!(matches!(
        message,
        OpenAiResponsesInputItem::Message {
            role: OpenAiResponsesMessageRole::User,
            ..
        }
    ));
}

#[test]
fn request_dtos_reject_missing_required_fields() {
    serde_json::from_value::<OpenAiChatCompletionRequest>(json!({
        "messages": [{"role": "user", "content": "hi"}]
    }))
    .expect_err("missing chat model must reject");
    serde_json::from_value::<OpenAiChatCompletionRequest>(json!({
        "model": "gpt-reborn"
    }))
    .expect_err("missing chat messages must reject");
    serde_json::from_value::<OpenAiResponsesCreateRequest>(json!({
        "input": "hello"
    }))
    .expect_err("missing responses model must reject");
    serde_json::from_value::<OpenAiResponsesCreateRequest>(json!({
        "model": "gpt-reborn"
    }))
    .expect_err("missing responses input must reject");
}

#[test]
fn response_dtos_serialize_openai_shapes() {
    let chat = OpenAiChatCompletionResponse {
        id: OpenAiChatCompletionId::new("chatcmpl-test").expect("chat id"),
        object: "chat.completion".to_string(),
        created: 1_777_777_777,
        model: "gpt-reborn".to_string(),
        choices: vec![ironclaw_reborn_openai_compat::OpenAiChatChoice {
            index: 0,
            message: ironclaw_reborn_openai_compat::OpenAiChatMessage {
                role: OpenAiChatMessageRole::Assistant,
                content: Some(json!("hello")),
                name: None,
                tool_call_id: None,
                tool_calls: None,
            },
            finish_reason: Some(OpenAiChatFinishReason::Stop),
        }],
        usage: None,
    };
    let chat_json = serde_json::to_value(chat).expect("chat response json");
    assert_eq!(chat_json["object"], "chat.completion");
    assert_eq!(chat_json["choices"][0]["finish_reason"], "stop");

    let chunk = OpenAiChatCompletionChunk {
        id: OpenAiChatCompletionId::new("chatcmpl-test").expect("chat id"),
        object: "chat.completion.chunk".to_string(),
        created: 1_777_777_777,
        model: "gpt-reborn".to_string(),
        choices: vec![ironclaw_reborn_openai_compat::OpenAiChatStreamChoice {
            index: 0,
            delta: ironclaw_reborn_openai_compat::OpenAiChatDelta {
                role: Some(OpenAiChatMessageRole::Assistant),
                content: Some("he".to_string()),
                tool_calls: Some(vec![
                    ironclaw_reborn_openai_compat::OpenAiChatToolCallDelta {
                        index: 0,
                        id: Some("call_1".to_string()),
                        kind: Some(ironclaw_reborn_openai_compat::OpenAiChatToolKind::Function),
                        function: Some(
                            ironclaw_reborn_openai_compat::OpenAiChatToolCallFunctionDelta {
                                name: Some("lookup_order".to_string()),
                                arguments: Some("{".to_string()),
                            },
                        ),
                    },
                ]),
            },
            finish_reason: None,
        }],
        usage: None,
    };
    let chunk_json = serde_json::to_value(chunk).expect("chunk json");
    assert_eq!(chunk_json["object"], "chat.completion.chunk");
    assert_eq!(chunk_json["choices"][0]["delta"]["content"], "he");
    assert_eq!(
        chunk_json["choices"][0]["delta"]["tool_calls"][0]["index"],
        0
    );
    assert_eq!(
        chunk_json["choices"][0]["delta"]["tool_calls"][0]["function"]["arguments"],
        "{"
    );

    let response = OpenAiResponseObject {
        id: OpenAiResponseId::new("resp_test").expect("response id"),
        object: "response".to_string(),
        created_at: 1_777_777_777,
        status: OpenAiResponseStatus::Completed,
        model: "gpt-reborn".to_string(),
        output: vec![
            OpenAiResponseOutputItem::Message {
                id: "msg_1".to_string(),
                status: Some(OpenAiResponseOutputItemStatus::Completed),
                role: OpenAiResponsesMessageRole::Assistant,
                content: json!([{"type": "output_text", "text": "hello"}]),
            },
            OpenAiResponseOutputItem::FunctionCall {
                id: "fc_1".to_string(),
                status: Some(OpenAiResponseOutputItemStatus::Completed),
                call_id: "call_1".to_string(),
                name: "lookup_order".to_string(),
                arguments: "{\"id\":\"123\"}".to_string(),
            },
            OpenAiResponseOutputItem::FunctionCallOutput {
                id: "fco_1".to_string(),
                status: Some(OpenAiResponseOutputItemStatus::Completed),
                call_id: "call_1".to_string(),
                output: json!({"ok": true}),
            },
        ],
        error: None,
        incomplete_details: None,
        usage: Some(OpenAiResponseUsage {
            input_tokens: 3,
            output_tokens: 5,
            total_tokens: 8,
        }),
    };
    let response_json = serde_json::to_value(response).expect("response json");
    assert_eq!(response_json["object"], "response");
    assert_eq!(response_json["status"], "completed");
    assert_eq!(response_json["output"][0]["type"], "message");
    assert_eq!(response_json["output"][1]["type"], "function_call");
    assert_eq!(response_json["output"][2]["type"], "function_call_output");
    assert_eq!(response_json["usage"]["input_tokens"], 3);
    assert!(response_json["usage"].get("prompt_tokens").is_none());

    let queued = OpenAiResponseObject {
        id: OpenAiResponseId::new("resp_queued").expect("response id"),
        object: "response".to_string(),
        created_at: 1_777_777_778,
        status: OpenAiResponseStatus::Queued,
        model: "gpt-reborn".to_string(),
        output: vec![],
        error: None,
        incomplete_details: None,
        usage: None,
    };
    let queued_json = serde_json::to_value(queued).expect("queued response json");
    assert_eq!(queued_json["output"], json!([]));
}

#[test]
fn response_error_object_uses_sanitized_vocabulary() {
    let error = OpenAiResponseErrorObject::from_kind(OpenAiCompatErrorKind::ServiceUnavailable);
    assert_eq!(error.code(), OpenAiCompatErrorCode::ServiceUnavailable);
    assert_eq!(error.message(), "The service is temporarily unavailable.");

    let serialized = serde_json::to_value(&error).expect("serialize response error");
    assert_eq!(serialized["code"], "service_unavailable");
    assert_eq!(
        serialized["message"],
        "The service is temporarily unavailable."
    );

    let injected = serde_json::from_value::<OpenAiResponseErrorObject>(json!({
        "code": "service_unavailable",
        "message": "provider stack /Users/alice secret-token"
    }))
    .expect_err("arbitrary response error messages must reject");
    assert!(
        injected
            .to_string()
            .contains("response error message must match sanitized error code")
    );
}
