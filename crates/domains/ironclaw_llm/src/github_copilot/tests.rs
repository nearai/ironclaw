//! Tests for the dedicated GitHub Copilot Chat transport.
//!
//! Split from `github_copilot.rs` to keep the provider implementation within
//! the architecture file-size budget.

use super::*;

#[test]
fn context_overflow_413_maps_to_context_length_exceeded() {
    // A raw HTTP 413 (payload too large) must become ContextLengthExceeded
    // so the loop's context-shrink recovery fires.
    match context_length_error_for_status(413, "Request Entity Too Large") {
        Some(LlmError::ContextLengthExceeded { .. }) => {}
        other => panic!("expected ContextLengthExceeded, got {other:?}"),
    }
}

#[test]
fn context_overflow_400_body_maps_to_context_length_exceeded() {
    let body = r#"{"error":{"message":"This model's maximum context length is 128000 tokens. However, your messages resulted in 150000 tokens.","code":"context_length_exceeded"}}"#;
    match context_length_error_for_status(400, body) {
        Some(LlmError::ContextLengthExceeded { .. }) => {}
        other => panic!("expected ContextLengthExceeded, got {other:?}"),
    }
}

#[test]
fn unrelated_400_is_not_context_overflow() {
    // A plain bad-request (e.g. invalid tool schema) must NOT be classified
    // as context overflow — the caller falls through to RequestFailed.
    assert!(
        context_length_error_for_status(400, r#"{"error":{"message":"invalid tool schema"}}"#)
            .is_none()
    );
}

#[test]
fn unrelated_5xx_is_not_context_overflow() {
    assert!(context_length_error_for_status(500, "internal server error").is_none());
}

#[test]
fn test_convert_messages_basic() {
    let messages = vec![
        ChatMessage::system("You are helpful."),
        ChatMessage::user("Hello"),
        ChatMessage::assistant("Hi there!"),
    ];
    let converted = convert_messages(messages);
    assert_eq!(converted.len(), 3);
    assert_eq!(converted[0].role, "system");
    assert_eq!(converted[1].role, "user");
    assert_eq!(converted[2].role, "assistant");
}

#[test]
fn test_convert_messages_tool_calls() {
    let tool_calls = vec![ToolCall {
        id: "call_1".to_string(),
        name: "search".to_string(),
        arguments: serde_json::json!({"q": "test"}),
        reasoning: None,
        signature: None,
        arguments_parse_error: None,
    }];
    let messages = vec![
        ChatMessage::user("Search"),
        ChatMessage::assistant_with_tool_calls(Some("Searching...".to_string()), tool_calls),
        ChatMessage::tool_result("call_1", "search", "found it"),
    ];
    let converted = convert_messages(messages);
    assert_eq!(converted.len(), 3);
    assert!(converted[1].tool_calls.is_some());
    assert_eq!(converted[2].role, "tool");
    assert_eq!(converted[2].tool_call_id, Some("call_1".to_string()));
}

#[test]
fn copilot_request_serializes_native_response_schema() {
    let schema = crate::provider::JsonSchemaResponseFormat::strict(
        "suggestions",
        serde_json::json!({"type": "object", "properties": {"items": {"type": "array"}}}),
    );
    let request = OpenAiRequest {
        model: "gpt-4o".to_string(),
        messages: convert_messages(vec![ChatMessage::user("Return suggestions")]),
        max_tokens: None,
        temperature: None,
        stop: None,
        response_format: Some(openai_json_schema_response_format(
            crate::provider::CompletionResponseFormat::JsonSchema(schema),
        )),
        tools: None,
        tool_choice: None,
        prompt_cache_key: None,
    };
    let json = serde_json::to_value(request).expect("serialize Copilot request");
    assert_eq!(
        json["response_format"],
        serde_json::json!({
            "type": "json_schema",
            "json_schema": {
                "name": "suggestions",
                "strict": true,
                "schema": {"type": "object", "properties": {"items": {"type": "array"}}}
            }
        })
    );
}

#[test]
fn copilot_request_serializes_native_json_object_mode() {
    let request = OpenAiRequest {
        model: "gpt-4o".to_string(),
        messages: convert_messages(vec![ChatMessage::user("Return an object")]),
        max_tokens: None,
        temperature: None,
        stop: None,
        response_format: Some(openai_json_schema_response_format(
            crate::provider::CompletionResponseFormat::JsonObject,
        )),
        tools: None,
        tool_choice: None,
        prompt_cache_key: None,
    };
    let json = serde_json::to_value(request).expect("serialize Copilot request");
    assert_eq!(
        json["response_format"],
        serde_json::json!({"type": "json_object"})
    );
}

/// Wire-shape pin: `OpenAiRequest::prompt_cache_key` serializes to a
/// top-level `prompt_cache_key` field with the exact value when present.
#[test]
fn copilot_request_serializes_prompt_cache_key_when_present() {
    let request = OpenAiRequest {
        model: "gpt-4o".to_string(),
        messages: convert_messages(vec![ChatMessage::user("hello")]),
        max_tokens: None,
        temperature: None,
        stop: None,
        response_format: None,
        tools: None,
        tool_choice: None,
        prompt_cache_key: Some("thread-cache-key-abc".to_string()),
    };
    let json = serde_json::to_value(request).expect("serialize Copilot request");
    assert_eq!(json["prompt_cache_key"], "thread-cache-key-abc");
}

/// Absent metadata must not synthesize the field on the wire.
#[test]
fn copilot_request_omits_prompt_cache_key_when_absent() {
    let request = OpenAiRequest {
        model: "gpt-4o".to_string(),
        messages: convert_messages(vec![ChatMessage::user("hello")]),
        max_tokens: None,
        temperature: None,
        stop: None,
        response_format: None,
        tools: None,
        tool_choice: None,
        prompt_cache_key: None,
    };
    let json = serde_json::to_value(request).expect("serialize Copilot request");
    assert!(
        json.as_object()
            .expect("object")
            .get("prompt_cache_key")
            .is_none(),
        "no prompt_cache_key field may be emitted when None: {json}"
    );
}

/// Build a provider for testing the `prompt_cache_key` gate directly.
fn test_provider(unsupported_params: Vec<&str>) -> GithubCopilotProvider {
    let config = RegistryProviderConfig::generic(
        crate::registry::ProviderProtocol::GithubCopilot,
        "github_copilot",
        Some(SecretString::from("test-oauth-token".to_string())),
        "",
        "gpt-4o",
    )
    .with_unsupported_params(unsupported_params.into_iter().map(String::from).collect());
    GithubCopilotProvider::new(&config, 30).expect("build GithubCopilotProvider")
}

/// With `PROMPT_CACHE_KEY_METADATA` present in the request metadata and
/// no kill switch set, the resolved value matches exactly.
#[test]
fn prompt_cache_key_resolves_from_metadata_when_supported() {
    let provider = test_provider(vec![]);
    let mut metadata = std::collections::HashMap::new();
    metadata.insert(
        PROMPT_CACHE_KEY_METADATA.to_string(),
        "thread-cache-key-abc".to_string(),
    );
    assert_eq!(
        provider.prompt_cache_key(&metadata),
        Some("thread-cache-key-abc".to_string())
    );
}

/// With no metadata entry, resolution returns `None` (field omitted).
#[test]
fn prompt_cache_key_absent_without_metadata() {
    let provider = test_provider(vec![]);
    assert_eq!(
        provider.prompt_cache_key(&std::collections::HashMap::new()),
        None
    );
}

/// The kill switch: listing `"prompt_cache_key"` in `unsupported_params`
/// suppresses the field even though metadata carries a value.
#[test]
fn prompt_cache_key_suppressed_when_in_unsupported_params() {
    let provider = test_provider(vec!["prompt_cache_key"]);
    let mut metadata = std::collections::HashMap::new();
    metadata.insert(
        PROMPT_CACHE_KEY_METADATA.to_string(),
        "thread-cache-key-abc".to_string(),
    );
    assert_eq!(provider.prompt_cache_key(&metadata), None);
}

async fn capture_copilot_request() -> (String, tokio::sync::oneshot::Receiver<serde_json::Value>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind capture server");
    let address = listener.local_addr().expect("capture server address");
    let (tx, rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("accept request");
        let mut request = Vec::new();
        let mut buffer = [0_u8; 4096];
        loop {
            let count = socket.read(&mut buffer).await.expect("read request");
            assert!(count > 0, "request ended before headers");
            request.extend_from_slice(&buffer[..count]);
            if let Some(header_end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                let body_start = header_end + 4;
                let headers = String::from_utf8_lossy(&request[..header_end]);
                let content_length = headers
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().expect("content length"))
                    })
                    .expect("content-length header");
                while request.len() < body_start + content_length {
                    let count = socket.read(&mut buffer).await.expect("read body");
                    assert!(count > 0, "request ended before body");
                    request.extend_from_slice(&buffer[..count]);
                }
                let body =
                    serde_json::from_slice(&request[body_start..body_start + content_length])
                        .expect("request body is JSON");
                tx.send(body).expect("test receives captured request");
                let response = r#"{"choices":[{"message":{"content":"ok"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1}}"#;
                socket
                        .write_all(
                            format!(
                                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{response}",
                                response.len()
                            )
                            .as_bytes(),
                        )
                        .await
                        .expect("write response");
                return;
            }
        }
    });
    (format!("http://{address}"), rx)
}

fn wire_test_provider(base_url: String) -> GithubCopilotProvider {
    let client = Client::new();
    GithubCopilotProvider {
        token_manager: Arc::new(
            crate::github_copilot_auth::tests::token_manager_with_cached_token(
                client.clone(),
                "cached-session-token".to_string(),
            ),
        ),
        client,
        model: "gpt-4o".to_string(),
        base_url,
        active_model: std::sync::RwLock::new("gpt-4o".to_string()),
        extra_headers: Vec::new(),
        unsupported_params: HashSet::new(),
    }
}

#[tokio::test]
async fn complete_sends_prompt_cache_key_on_the_wire() {
    let (base_url, captured) = capture_copilot_request().await;
    let provider = wire_test_provider(base_url);
    let mut request = CompletionRequest::new(vec![ChatMessage::user("hello")]);
    request.metadata.insert(
        PROMPT_CACHE_KEY_METADATA.to_string(),
        "hashed-cache-key-abc".to_string(),
    );

    provider
        .complete(request)
        .await
        .expect("Copilot completion");

    let body = captured.await.expect("captured request");
    assert_eq!(body["prompt_cache_key"], "hashed-cache-key-abc");
}

#[tokio::test]
async fn complete_with_tools_sends_prompt_cache_key_on_the_wire() {
    let (base_url, captured) = capture_copilot_request().await;
    let provider = wire_test_provider(base_url);
    let mut request = ToolCompletionRequest::new(
        vec![ChatMessage::user("search")],
        vec![ToolDefinition {
            name: "search".to_string(),
            description: "Search".to_string(),
            parameters: serde_json::json!({"type": "object"}),
        }],
    );
    request.metadata.insert(
        PROMPT_CACHE_KEY_METADATA.to_string(),
        "hashed-cache-key-xyz".to_string(),
    );

    provider
        .complete_with_tools(request)
        .await
        .expect("Copilot tool completion");

    let body = captured.await.expect("captured request");
    assert_eq!(body["prompt_cache_key"], "hashed-cache-key-xyz");
}

#[test]
fn test_convert_messages_defaults_missing_image_detail_to_auto() {
    let messages = vec![ChatMessage::user_with_parts(
        "describe this",
        vec![ContentPart::ImageUrl {
            image_url: crate::ImageUrl {
                url: "data:image/jpeg;base64,Zm9v".to_string(),
                detail: None,
            },
        }],
    )];

    let converted = convert_messages(messages);
    let content = serde_json::to_value(&converted[0].content).expect("serialize content");
    assert_eq!(
        content[1]["image_url"]["url"],
        "data:image/jpeg;base64,Zm9v"
    );
    assert_eq!(content[1]["image_url"]["detail"], "auto");
}

#[test]
fn test_convert_messages_preserves_explicit_image_detail() {
    for expected in ["low", "high"] {
        let messages = vec![ChatMessage::user_with_parts(
            "describe this",
            vec![ContentPart::ImageUrl {
                image_url: crate::ImageUrl {
                    url: format!("https://example.com/{expected}.png"),
                    detail: Some(expected.to_string()),
                },
            }],
        )];

        let converted = convert_messages(messages);
        let content = serde_json::to_value(&converted[0].content).expect("serialize content");
        assert_eq!(content[1]["image_url"]["detail"], expected);
    }
}

#[test]
fn copilot_flattens_top_level_oneof_at_the_provider_boundary() {
    let tool = convert_tool_definition(ToolDefinition {
        name: "evm-rpc.invoke".to_string(),
        description: "Invoke an EVM RPC operation.".to_string(),
        parameters: serde_json::json!({
            "oneOf": [
                {
                    "type": "object",
                    "properties": {
                        "action": {"const": "get_balance"},
                        "address": {"type": "string"}
                    },
                    "required": ["action", "address"]
                },
                {
                    "type": "object",
                    "properties": {
                        "action": {"const": "get_block"},
                        "block": {"type": "string"}
                    },
                    "required": ["action", "block"]
                }
            ]
        }),
    });

    assert_eq!(tool.function.parameters["type"], "object");
    assert!(tool.function.parameters.get("oneOf").is_none());
    assert!(
        tool.function.parameters["properties"]
            .get("action")
            .is_some()
    );
    assert!(
        tool.function.parameters["properties"]
            .get("address")
            .is_some()
    );
    assert!(
        tool.function.parameters["properties"]
            .get("block")
            .is_some()
    );
    assert!(tool.function.description.contains("Upstream JSON schema"));
}

#[test]
fn test_extract_choice_text_only() {
    let choice = OpenAiChoice {
        message: OpenAiResponseMessage {
            content: Some("Hello!".to_string()),
            tool_calls: None,
        },
        finish_reason: Some("stop".to_string()),
    };
    let (content, tool_calls) = extract_choice_content(&choice);
    assert_eq!(content, Some("Hello!".to_string()));
    assert!(tool_calls.is_empty());
}

#[test]
fn test_extract_choice_with_tool_calls() {
    let choice = OpenAiChoice {
        message: OpenAiResponseMessage {
            content: Some("Let me search.".to_string()),
            tool_calls: Some(vec![OpenAiResponseToolCall {
                id: "call_1".to_string(),
                function: OpenAiResponseFunction {
                    name: "search".to_string(),
                    arguments: r#"{"q":"test"}"#.to_string(),
                },
            }]),
        },
        finish_reason: Some("tool_calls".to_string()),
    };
    let (content, tool_calls) = extract_choice_content(&choice);
    assert_eq!(content, Some("Let me search.".to_string()));
    assert_eq!(tool_calls.len(), 1);
    assert_eq!(tool_calls[0].name, "search");
    assert_eq!(tool_calls[0].arguments["q"], "test");
}
