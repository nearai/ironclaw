//! Wire-shape and breakpoint tests for the Anthropic OAuth transport.
//!
//! Split out of `anthropic_oauth.rs` (arch file-size budget): loopback
//! capture-server tests pinning the request JSON — including the #6984
//! `cache_control` breakpoint layout — plus direct branch tests for
//! `apply_cache_breakpoints`.

use super::*;
use crate::config::CacheRetention;
use crate::provider::ToolDefinition;

#[tokio::test]
async fn complete_preserves_missing_retry_after_on_headerless_502() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("loopback listener");
    let base_url = format!(
        "http://{}",
        listener.local_addr().expect("loopback address")
    );
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("accept request");
        let mut request = vec![0_u8; 4096];
        let _ = socket.read(&mut request).await.expect("read request");
        let body = r#"{"error":{"message":"upstream unavailable"}}"#;
        let response = format!(
            "HTTP/1.1 502 Bad Gateway\r\ncontent-type: application/json\r\n\
             content-length: {}\r\n\r\n{body}",
            body.len()
        );
        socket
            .write_all(response.as_bytes())
            .await
            .expect("write error response");
    });

    let mut config = RegistryProviderConfig::generic(
        crate::registry::ProviderProtocol::Anthropic,
        "anthropic_oauth",
        None,
        base_url,
        "claude-test",
    );
    config.oauth_token = Some(SecretString::from("test-token".to_string()));
    let provider = AnthropicOAuthProvider::new(&config).expect("provider");
    let error = provider
        .complete(CompletionRequest::new(vec![ChatMessage::user("hello")]))
        .await
        .expect_err("scripted provider error");
    server.await.expect("loopback server");

    assert!(matches!(
        error,
        LlmError::BadGateway {
            provider,
            status: 502,
            retry_after: None,
        } if provider == "anthropic_oauth"
    ));
}

/// One-shot loopback capture server: returns the base URL and a handle
/// resolving to the captured request body. Replies 400 — these tests
/// assert the request wire shape, not response handling.
async fn capture_one_request() -> (String, tokio::sync::oneshot::Receiver<String>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("loopback listener");
    let base_url = format!(
        "http://{}",
        listener.local_addr().expect("loopback address")
    );
    let (tx, rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("accept request");
        let mut request = Vec::new();
        let mut buffer = [0_u8; 4096];
        loop {
            let read = socket.read(&mut buffer).await.expect("read request");
            if read == 0 {
                return;
            }
            request.extend_from_slice(&buffer[..read]);
            let Some(header_end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") else {
                continue;
            };
            let headers =
                std::str::from_utf8(&request[..header_end]).expect("request headers are UTF-8");
            let content_length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then_some(value.trim())
                })
                .expect("content length header")
                .parse::<usize>()
                .expect("content length is numeric");
            let body_start = header_end + 4;
            if request.len() < body_start + content_length {
                continue;
            }
            tx.send(
                String::from_utf8(request[body_start..body_start + content_length].to_vec())
                    .expect("request body is UTF-8"),
            )
            .expect("test receives captured request");
            socket
                .write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n")
                .await
                .expect("write test response");
            return;
        }
    });
    (base_url, rx)
}

fn provider_with_retention(base_url: &str, retention: CacheRetention) -> AnthropicOAuthProvider {
    let mut config = RegistryProviderConfig::generic(
        crate::registry::ProviderProtocol::Anthropic,
        "anthropic_oauth",
        None,
        base_url,
        "claude-opus-4-6",
    );
    config.oauth_token = Some(SecretString::from("test-token".to_string()));
    config.cache_retention = retention;
    AnthropicOAuthProvider::new(&config).expect("provider")
}

async fn captured_json(rx: tokio::sync::oneshot::Receiver<String>) -> serde_json::Value {
    let body = tokio::time::timeout(std::time::Duration::from_secs(5), rx)
        .await
        .expect("request capture timed out")
        .expect("captured request body");
    serde_json::from_str(&body).expect("captured body is JSON")
}

/// Wire-level pin for issue #6984: under `Short` retention the OAuth
/// transport emits the three explicit cache breakpoints — system prompt
/// block, last tool definition, and the last content block of the last
/// message (here a tool_result, the common agent-loop tail).
#[tokio::test]
async fn oauth_short_retention_places_explicit_cache_breakpoints() {
    let (base_url, captured) = capture_one_request().await;
    let provider = provider_with_retention(&base_url, CacheRetention::Short);

    let request = ToolCompletionRequest::new(
        vec![
            ChatMessage::system("You are helpful."),
            ChatMessage::user("Run the tool."),
            ChatMessage::assistant_with_tool_calls(
                None,
                vec![ToolCall {
                    id: "call_1".to_string(),
                    name: "alpha".to_string(),
                    arguments: serde_json::json!({}),
                    ..ToolCall::default()
                }],
            ),
            ChatMessage::tool_result("call_1", "alpha", "tool says hi"),
        ],
        vec![
            ToolDefinition {
                name: "alpha".to_string(),
                description: "First tool".to_string(),
                parameters: serde_json::json!({"type": "object", "properties": {}}),
            },
            ToolDefinition {
                name: "beta".to_string(),
                description: "Second tool".to_string(),
                parameters: serde_json::json!({"type": "object", "properties": {}}),
            },
        ],
    );
    let _ = provider.complete_with_tools(request).await;
    let body = captured_json(captured).await;

    let system = body["system"]
        .as_array()
        .expect("system serialized as blocks when caching is on");
    assert_eq!(
        system.last().expect("system block")["cache_control"]["type"],
        "ephemeral"
    );
    assert!(system.last().unwrap()["cache_control"].get("ttl").is_none());

    let tools = body["tools"].as_array().expect("tools array");
    assert_eq!(tools.len(), 2);
    assert!(tools[0].get("cache_control").is_none());
    assert_eq!(tools[1]["name"], "beta");
    assert_eq!(tools[1]["cache_control"]["type"], "ephemeral");

    let messages = body["messages"].as_array().expect("messages");
    let last_content = messages.last().expect("last message")["content"]
        .as_array()
        .expect("last message content blocks");
    let last_block = last_content.last().expect("content block");
    assert_eq!(last_block["type"], "tool_result");
    assert_eq!(last_block["cache_control"]["type"], "ephemeral");
    // Only the final block carries a marker.
    for message in &messages[..messages.len() - 1] {
        if let Some(blocks) = message["content"].as_array() {
            for block in blocks {
                assert!(block.get("cache_control").is_none(), "{body}");
            }
        }
    }
}

/// Wire-level pin: `Long` retention stamps a 1h TTL on every breakpoint
/// (uniform TTLs satisfy Anthropic's longer-before-shorter ordering rule).
#[tokio::test]
async fn oauth_long_retention_uses_1h_ttl_markers() {
    let (base_url, captured) = capture_one_request().await;
    let provider = provider_with_retention(&base_url, CacheRetention::Long);

    let _ = provider
        .complete(CompletionRequest::new(vec![
            ChatMessage::system("You are helpful."),
            ChatMessage::user("Question"),
        ]))
        .await;
    let body = captured_json(captured).await;

    let system = body["system"].as_array().expect("system blocks");
    assert_eq!(system.last().unwrap()["cache_control"]["ttl"], "1h");

    let messages = body["messages"].as_array().expect("messages");
    let last_content = messages.last().unwrap()["content"]
        .as_array()
        .expect("last message content blocks");
    assert_eq!(last_content.last().unwrap()["cache_control"]["ttl"], "1h");
}

/// Wire-level pin: a model without prompt-cache support (claude-2 era)
/// downgrades to no caching at construction, keeping the legacy shape.
#[tokio::test]
async fn oauth_unsupported_model_downgrades_to_no_caching() {
    let (base_url, captured) = capture_one_request().await;
    let mut config = RegistryProviderConfig::generic(
        crate::registry::ProviderProtocol::Anthropic,
        "anthropic_oauth",
        None,
        &base_url,
        "claude-2.1",
    );
    config.oauth_token = Some(SecretString::from("test-token".to_string()));
    config.cache_retention = CacheRetention::Short;
    let provider = AnthropicOAuthProvider::new(&config).expect("provider");

    let _ = provider
        .complete(CompletionRequest::new(vec![
            ChatMessage::system("You are helpful."),
            ChatMessage::user("Question"),
        ]))
        .await;
    let body = captured_json(captured).await;

    assert!(body["system"].is_string(), "{body}");
    assert!(
        !serde_json::to_string(&body)
            .unwrap()
            .contains("cache_control")
    );
}

fn request_with_messages(messages: Vec<AnthropicMessage>) -> AnthropicRequest {
    AnthropicRequest {
        model: "claude-opus-4-6".to_string(),
        messages,
        system: None,
        max_tokens: 128,
        temperature: None,
        thinking: None,
        tools: None,
        tool_choice: None,
    }
}

/// Branch coverage for `apply_cache_breakpoints`: a tool_use-tailed
/// assistant message (no system, no tools) gets its last block marked.
#[test]
fn apply_breakpoints_marks_tool_use_tail_without_system_or_tools() {
    let mut request = request_with_messages(vec![AnthropicMessage {
        role: "assistant".to_string(),
        content: AnthropicContent::Blocks(vec![AnthropicContentBlock::ToolUse {
            id: "call_1".to_string(),
            name: "alpha".to_string(),
            input: serde_json::json!({}),
            cache_control: None,
        }]),
    }]);
    apply_cache_breakpoints(&mut request, CacheRetention::Short);

    assert!(request.system.is_none());
    let AnthropicContent::Blocks(blocks) = &request.messages[0].content else {
        panic!("blocks expected");
    };
    let AnthropicContentBlock::ToolUse { cache_control, .. } = &blocks[0] else {
        panic!("tool_use expected");
    };
    assert!(cache_control.is_some());
}

/// Branch coverage: an image-tailed message gets its image block marked.
#[test]
fn apply_breakpoints_marks_image_tail() {
    let mut request = request_with_messages(vec![AnthropicMessage {
        role: "user".to_string(),
        content: AnthropicContent::Blocks(vec![AnthropicContentBlock::Image {
            source: AnthropicImageSource {
                source_type: "base64",
                media_type: "image/png".to_string(),
                data: "aGk=".to_string(),
            },
            cache_control: None,
        }]),
    }]);
    apply_cache_breakpoints(&mut request, CacheRetention::Short);

    let AnthropicContent::Blocks(blocks) = &request.messages[0].content else {
        panic!("blocks expected");
    };
    let AnthropicContentBlock::Image { cache_control, .. } = &blocks[0] else {
        panic!("image expected");
    };
    assert!(cache_control.is_some());
}

/// Branch coverage: an empty trailing text message stays in string form —
/// the API rejects cache_control on empty text blocks — and an empty
/// message list is a no-op.
#[test]
fn apply_breakpoints_skips_empty_text_tail_and_empty_transcript() {
    let mut request = request_with_messages(vec![AnthropicMessage {
        role: "user".to_string(),
        content: AnthropicContent::Text(String::new()),
    }]);
    apply_cache_breakpoints(&mut request, CacheRetention::Short);
    assert!(matches!(
        &request.messages[0].content,
        AnthropicContent::Text(text) if text.is_empty()
    ));

    let mut empty = request_with_messages(Vec::new());
    apply_cache_breakpoints(&mut empty, CacheRetention::Long);
    assert!(empty.messages.is_empty());
}

/// Branch coverage: a text block at the tail of a multimodal message gets the
/// marker (the `Text` arm of `set_cache_control`), and an empty block list is
/// a no-op.
#[test]
fn apply_breakpoints_marks_text_block_tail_and_skips_empty_blocks() {
    let mut request = request_with_messages(vec![AnthropicMessage {
        role: "user".to_string(),
        content: AnthropicContent::Blocks(vec![
            AnthropicContentBlock::Image {
                source: AnthropicImageSource {
                    source_type: "base64",
                    media_type: "image/png".to_string(),
                    data: "aGk=".to_string(),
                },
                cache_control: None,
            },
            AnthropicContentBlock::Text {
                text: "caption".to_string(),
                cache_control: None,
            },
        ]),
    }]);
    apply_cache_breakpoints(&mut request, CacheRetention::Short);

    let AnthropicContent::Blocks(blocks) = &request.messages[0].content else {
        panic!("blocks expected");
    };
    let AnthropicContentBlock::Image { cache_control, .. } = &blocks[0] else {
        panic!("image expected");
    };
    assert!(cache_control.is_none(), "only the tail block is marked");
    let AnthropicContentBlock::Text { cache_control, .. } = &blocks[1] else {
        panic!("text expected");
    };
    assert!(cache_control.is_some());

    let mut empty_blocks = request_with_messages(vec![AnthropicMessage {
        role: "user".to_string(),
        content: AnthropicContent::Blocks(Vec::new()),
    }]);
    empty_blocks.tools = Some(Vec::new());
    apply_cache_breakpoints(&mut empty_blocks, CacheRetention::Short);
    assert!(matches!(
        &empty_blocks.messages[0].content,
        AnthropicContent::Blocks(blocks) if blocks.is_empty()
    ));
    assert!(matches!(&empty_blocks.tools, Some(tools) if tools.is_empty()));
}

/// Branch coverage: a system value already in block form is restored
/// untouched — never dropped by the take-and-rebuild — pinning the
/// restore-on-non-Text arm.
#[test]
fn apply_breakpoints_preserves_prebuilt_system_blocks() {
    let mut request = request_with_messages(vec![AnthropicMessage {
        role: "user".to_string(),
        content: AnthropicContent::Text("question".to_string()),
    }]);
    request.system = Some(AnthropicSystem::Blocks(vec![AnthropicSystemBlock {
        block_type: "text",
        text: "prebuilt".to_string(),
        cache_control: None,
    }]));
    apply_cache_breakpoints(&mut request, CacheRetention::Short);

    let Some(AnthropicSystem::Blocks(blocks)) = &request.system else {
        panic!("prebuilt system blocks must survive apply_cache_breakpoints");
    };
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].text, "prebuilt");
    assert!(blocks[0].cache_control.is_none(), "restored untouched");
}

/// Wire-level pin: retention `None` keeps the legacy wire shape — system
/// as a plain string, and no cache_control anywhere.
#[tokio::test]
async fn oauth_no_retention_keeps_legacy_wire_shape() {
    let (base_url, captured) = capture_one_request().await;
    let provider = provider_with_retention(&base_url, CacheRetention::None);

    let _ = provider
        .complete(CompletionRequest::new(vec![
            ChatMessage::system("You are helpful."),
            ChatMessage::user("Question"),
        ]))
        .await;
    let body = captured_json(captured).await;

    assert!(
        body["system"].is_string(),
        "system stays a plain string when caching is off: {body}"
    );
    assert!(
        !serde_json::to_string(&body)
            .unwrap()
            .contains("cache_control"),
        "no cache_control may be emitted when caching is off: {body}"
    );
}

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
    let body = r#"{"type":"error","error":{"type":"invalid_request_error","message":"prompt is too long: 234872 tokens > 200000 maximum"}}"#;
    match context_length_error_for_status(400, body) {
        Some(LlmError::ContextLengthExceeded { used, limit }) => {
            assert_eq!(used, 234872);
            assert_eq!(limit, 200000);
        }
        other => panic!("expected ContextLengthExceeded, got {other:?}"),
    }
}

#[test]
fn unrelated_400_is_not_context_overflow() {
    // A plain bad-request (e.g. invalid request shape) must NOT be
    // classified as context overflow — the caller falls through to
    // RequestFailed.
    assert!(
        context_length_error_for_status(400, r#"{"error":{"message":"invalid request body"}}"#)
            .is_none()
    );
}

#[test]
fn unrelated_5xx_is_not_context_overflow() {
    assert!(context_length_error_for_status(503, "service unavailable").is_none());
}

#[test]
fn test_convert_messages_extracts_system() {
    let messages = vec![
        ChatMessage::system("You are helpful."),
        ChatMessage::user("Hello"),
    ];
    let (system, msgs) = convert_messages(messages);
    assert_eq!(system, Some("You are helpful.".to_string()));
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].role, "user");
}

#[test]
fn test_convert_messages_multiple_systems() {
    let messages = vec![
        ChatMessage::system("System 1"),
        ChatMessage::system("System 2"),
        ChatMessage::user("Hello"),
    ];
    let (system, msgs) = convert_messages(messages);
    assert_eq!(system, Some("System 1\n\nSystem 2".to_string()));
    assert_eq!(msgs.len(), 1);
}

#[test]
fn test_convert_messages_user_image_becomes_base64_image_block() {
    let messages = vec![ChatMessage::user_with_parts(
        "what is this?",
        vec![ContentPart::ImageUrl {
            image_url: crate::provider::ImageUrl {
                url: "data:image/png;base64,AQIDBA==".to_string(),
                detail: None,
            },
        }],
    )];
    let (_system, msgs) = convert_messages(messages);
    assert_eq!(msgs.len(), 1);
    // Text rides as the first block, the image as a base64 `image` block.
    let value = serde_json::to_value(&msgs[0]).expect("serialize");
    let blocks = value["content"].as_array().expect("content blocks");
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0]["type"], "text");
    assert_eq!(blocks[0]["text"], "what is this?");
    assert_eq!(blocks[1]["type"], "image");
    assert_eq!(blocks[1]["source"]["type"], "base64");
    assert_eq!(blocks[1]["source"]["media_type"], "image/png");
    assert_eq!(blocks[1]["source"]["data"], "AQIDBA==");
}

#[test]
fn test_convert_messages_text_only_user_stays_a_string() {
    let messages = vec![ChatMessage::user("just text")];
    let (_system, msgs) = convert_messages(messages);
    let value = serde_json::to_value(&msgs[0]).expect("serialize");
    // No inline images → compact string content, not a blocks array.
    assert_eq!(value["content"], "just text");
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
        ChatMessage::user("Search for test"),
        ChatMessage::assistant_with_tool_calls(Some("Let me search.".to_string()), tool_calls),
        ChatMessage::tool_result("call_1", "search", "found it"),
    ];
    let (system, msgs) = convert_messages(messages);
    assert!(system.is_none());
    assert_eq!(msgs.len(), 3);
    assert_eq!(msgs[0].role, "user");
    assert_eq!(msgs[1].role, "assistant");
    // Tool result should be a user message
    assert_eq!(msgs[2].role, "user");
}

#[test]
fn test_extract_response_text_only() {
    let response = AnthropicResponse {
        content: vec![AnthropicResponseBlock::Text {
            text: "Hello!".to_string(),
        }],
        stop_reason: Some("end_turn".to_string()),
        usage: AnthropicUsage {
            input_tokens: 10,
            output_tokens: 5,
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: 0,
        },
    };
    let extracted = extract_response_content(&response);
    assert_eq!(extracted.content, Some("Hello!".to_string()));
    assert!(extracted.tool_calls.is_empty());
}

#[test]
fn test_extract_response_with_tool_use() {
    let response = AnthropicResponse {
        content: vec![
            AnthropicResponseBlock::Text {
                text: "Let me search.".to_string(),
            },
            AnthropicResponseBlock::ToolUse {
                id: "call_1".to_string(),
                name: "search".to_string(),
                input: serde_json::json!({"q": "test"}),
            },
        ],
        stop_reason: Some("tool_use".to_string()),
        usage: AnthropicUsage {
            input_tokens: 20,
            output_tokens: 15,
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: 0,
        },
    };
    let extracted = extract_response_content(&response);
    assert_eq!(extracted.content, Some("Let me search.".to_string()));
    assert_eq!(extracted.tool_calls.len(), 1);
    assert_eq!(extracted.tool_calls[0].name, "search");
}

#[test]
fn test_extract_response_preserves_thinking_as_reasoning() {
    let response = AnthropicResponse {
        content: vec![
            AnthropicResponseBlock::Thinking {
                thinking: Some("Raw thinking".to_string()),
                summary: Some("Summarized thinking".to_string()),
                _signature: Some("sig".to_string()),
            },
            AnthropicResponseBlock::Text {
                text: "Done.".to_string(),
            },
        ],
        stop_reason: Some("end_turn".to_string()),
        usage: AnthropicUsage {
            input_tokens: 20,
            output_tokens: 15,
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: 0,
        },
    };
    let extracted = extract_response_content(&response);
    assert_eq!(extracted.content, Some("Done.".to_string()));
    assert_eq!(extracted.reasoning, Some("Summarized thinking".to_string()));
}

#[test]
fn test_extract_response_uses_thinking_when_summary_absent() {
    let response = AnthropicResponse {
        content: vec![
            AnthropicResponseBlock::Thinking {
                thinking: Some("Raw thinking fallback".to_string()),
                summary: None,
                _signature: Some("sig".to_string()),
            },
            AnthropicResponseBlock::Text {
                text: "Done.".to_string(),
            },
        ],
        stop_reason: Some("end_turn".to_string()),
        usage: AnthropicUsage {
            input_tokens: 20,
            output_tokens: 15,
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: 0,
        },
    };

    let extracted = extract_response_content(&response);

    assert_eq!(extracted.content, Some("Done.".to_string()));
    assert_eq!(
        extracted.reasoning,
        Some("Raw thinking fallback".to_string())
    );
}

/// Regression test for #1136: token field must be mutable via RwLock
/// so that a refreshed token persists across subsequent requests.
#[test]
fn test_token_update_persists() {
    let original = SecretString::from("old_token".to_string());
    let token = std::sync::RwLock::new(original);

    // Read the original
    assert_eq!(token.read().unwrap().expose_secret(), "old_token");

    // Simulate a successful refresh
    let refreshed = SecretString::from("new_token".to_string());
    *token.write().unwrap() = refreshed;

    // Subsequent reads see the updated token
    assert_eq!(token.read().unwrap().expose_secret(), "new_token");
}
