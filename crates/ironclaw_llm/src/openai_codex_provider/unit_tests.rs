
use super::*;
use crate::codex_test_helpers::make_test_jwt;

#[test]
fn test_extract_account_id_success() {
    let jwt = make_test_jwt("acct_abc123");
    let result = extract_account_id(&jwt);
    assert!(result.is_ok());
    assert_eq!(result.unwrap(), "acct_abc123");
}

#[test]
fn test_extract_account_id_missing_claim() {
    use base64::Engine;
    let engine = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let header = engine.encode(b"{\"alg\":\"RS256\"}");
    let payload = engine.encode(b"{\"sub\":\"user123\"}");
    let sig = engine.encode(b"sig");
    let jwt = format!("{header}.{payload}.{sig}");

    let result = extract_account_id(&jwt);
    assert!(result.is_err());
}

#[test]
fn test_extract_account_id_invalid_jwt() {
    let result = extract_account_id("not-a-jwt");
    assert!(result.is_err());
}

#[test]
fn test_convert_user_message() {
    let msg = ChatMessage::user("Hello world");
    let items = convert_message(&msg, 0);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["role"], "user");
    assert_eq!(items[0]["content"][0]["type"], "input_text");
    assert_eq!(items[0]["content"][0]["text"], "Hello world");
}

#[test]
fn test_convert_system_message_excluded() {
    let msg = ChatMessage::system("You are helpful");
    let items = convert_message(&msg, 0);
    assert!(items.is_empty());
}

#[test]
fn test_convert_assistant_text_message() {
    let msg = ChatMessage::assistant("Sure, I can help");
    let items = convert_message(&msg, 3);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["type"], "message");
    assert_eq!(items[0]["role"], "assistant");
    assert_eq!(items[0]["id"], "msg_3");
    assert_eq!(items[0]["content"][0]["type"], "output_text");
}

#[test]
fn test_convert_assistant_with_tool_calls() {
    let tool_calls = vec![
        ToolCall {
            id: "call_1".to_string(),
            name: "search".to_string(),
            arguments: serde_json::json!({"query": "test"}),
            reasoning: None,
            signature: None,
            arguments_parse_error: None,
        },
        ToolCall {
            id: "call_2".to_string(),
            name: "read".to_string(),
            arguments: serde_json::json!({"path": "/tmp"}),
            reasoning: None,
            signature: None,
            arguments_parse_error: None,
        },
    ];
    let msg = ChatMessage::assistant_with_tool_calls(Some("Let me check".to_string()), tool_calls);
    let items = convert_message(&msg, 0);
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["type"], "function_call");
    assert_eq!(items[0]["call_id"], "call_1");
    assert_eq!(items[0]["name"], "search");
    assert_eq!(items[1]["type"], "function_call");
    assert_eq!(items[1]["call_id"], "call_2");
}

#[test]
fn test_convert_tool_result_message() {
    let msg = ChatMessage::tool_result("call_1", "search", "found 3 results");
    let items = convert_message(&msg, 0);
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["type"], "function_call_output");
    assert_eq!(items[0]["call_id"], "call_1");
    assert_eq!(items[0]["output"], "found 3 results");
}

#[test]
fn test_convert_tool_definition() {
    let tool = ToolDefinition {
        name: "my_tool".to_string(),
        description: "Does things".to_string(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "x": { "type": "string" }
            }
        }),
    };
    let json = convert_tool_definition(&tool);
    assert_eq!(json["type"], "function");
    assert_eq!(json["name"], "my_tool");
    assert_eq!(json["description"], "Does things");
}

/// Caller-level regression test: drives `convert_tool_definition` end to
/// end with a GitHub-Copilot-shaped MCP tool definition and asserts that
/// the resulting Responses API JSON would no longer trip the 400. This
/// is the test that would have caught the original failure mode. The
/// helper-level tests for the underlying flatten live next to the
/// helper itself in `rig_adapter.rs`.
#[test]
fn test_convert_tool_definition_handles_top_level_oneof_dispatcher() {
    let tool = ToolDefinition {
        name: "github".to_string(),
        description: "GitHub MCP umbrella tool".to_string(),
        parameters: serde_json::json!({
            "type": "object",
            "oneOf": [
                {
                    "properties": {
                        "action": { "const": "create_issue" },
                        "title":  { "type": "string" },
                        "body":   { "type": "string" }
                    },
                    "required": ["action", "title"]
                },
                {
                    "properties": {
                        "action": { "const": "list_issues" },
                        "repo":   { "type": "string" }
                    },
                    "required": ["action", "repo"]
                }
            ]
        }),
    };
    let json = convert_tool_definition(&tool);

    let params = &json["parameters"];
    assert_eq!(params["type"], "object", "top-level type must be object");
    assert!(
        params.get("oneOf").is_none(),
        "top-level oneOf must not survive into the request body"
    );
    assert!(
        params.get("anyOf").is_none() && params.get("allOf").is_none(),
        "no other top-level union keywords either"
    );
    assert_eq!(params["additionalProperties"], true);

    let description = json["description"].as_str().unwrap();
    assert!(
        description.starts_with("GitHub MCP umbrella tool"),
        "original description must come first"
    );
    assert!(
        description.contains("Upstream JSON schema"),
        "advisory hint must be appended"
    );
    assert!(
        description.contains("create_issue") && description.contains("list_issues"),
        "variant info must be retained in the hint so the LLM can choose"
    );
}

#[test]
fn test_parse_sse_text_response() {
    let sse_body = r#"data: {"type":"response.output_item.added","item":{"type":"message","role":"assistant","id":"msg_1"}}

data: {"type":"response.output_text.delta","delta":"Hello "}

data: {"type":"response.output_text.delta","delta":"world!"}

data: {"type":"response.completed","response":{"status":"completed","usage":{"input_tokens":10,"output_tokens":5}}}

"#;
    let result = parse_sse_response(sse_body);
    assert!(result.is_ok());
    let parsed = result.unwrap();
    assert_eq!(parsed.text_content, "Hello world!");
    assert_eq!(parsed.input_tokens, 10);
    assert_eq!(parsed.output_tokens, 5);
    assert_eq!(parsed.finish_reason, FinishReason::Stop);
    assert!(parsed.tool_calls.is_empty());
}

#[test]
fn test_parse_sse_reasoning_summary_response() {
    let sse_body = r#"data: {"type":"response.reasoning_summary_text.delta","delta":"Thinking Steps\n"}

data: {"type":"response.reasoning_summary_text.delta","delta":"[] Inspect context."}

data: {"type":"response.output_text.delta","delta":"Done."}

data: {"type":"response.completed","response":{"status":"completed","usage":{"input_tokens":10,"output_tokens":5}}}

"#;
    let parsed = parse_sse_response(sse_body).unwrap();
    assert_eq!(parsed.text_content, "Done.");
    assert_eq!(
        parsed.reasoning.as_deref(),
        Some("Thinking Steps\n[] Inspect context.")
    );
}

#[test]
fn test_parse_sse_tool_call_response() {
    let sse_body = r#"data: {"type":"response.output_item.added","item":{"type":"function_call","id":"fc_1","call_id":"call_abc","name":"search"}}

data: {"type":"response.function_call_arguments.delta","item_id":"fc_1","delta":"{\"query\":"}

data: {"type":"response.function_call_arguments.delta","item_id":"fc_1","delta":"\"test\"}"}

data: {"type":"response.output_item.done","item":{"type":"function_call","id":"fc_1","call_id":"call_abc","name":"search","arguments":"{\"query\":\"test\"}"}}

data: {"type":"response.completed","response":{"status":"completed","usage":{"input_tokens":15,"output_tokens":8}}}

"#;
    let result = parse_sse_response(sse_body);
    assert!(result.is_ok());
    let parsed = result.unwrap();
    assert!(parsed.text_content.is_empty());
    assert_eq!(parsed.tool_calls.len(), 1);
    assert_eq!(parsed.tool_calls[0].id, "call_abc");
    assert_eq!(parsed.tool_calls[0].name, "search");
    assert_eq!(
        parsed.tool_calls[0].arguments,
        serde_json::json!({"query": "test"})
    );
    assert_eq!(parsed.finish_reason, FinishReason::ToolUse);
}

#[test]
fn test_parse_sse_error_response() {
    let sse_body = r#"data: {"type":"error","code":"rate_limit_exceeded","message":"Too many requests"}

"#;
    let result = parse_sse_response(sse_body);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("rate_limit_exceeded"));
}

#[test]
fn test_parse_sse_failed_response() {
    let sse_body = r#"data: {"type":"response.failed","response":{"status":"failed","status_details":{"error":{"message":"Model overloaded"}}}}

"#;
    let result = parse_sse_response(sse_body);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("Model overloaded"));
}

#[test]
fn test_parse_sse_incomplete_status() {
    let sse_body = r#"data: {"type":"response.output_text.delta","delta":"partial"}

data: {"type":"response.completed","response":{"status":"incomplete","usage":{"input_tokens":5,"output_tokens":2}}}

"#;
    let result = parse_sse_response(sse_body);
    assert!(result.is_ok());
    let parsed = result.unwrap();
    assert_eq!(parsed.text_content, "partial");
    assert_eq!(parsed.finish_reason, FinishReason::Length);
}

/// `[DONE]` stops parsing (events after it are ignored), but on its own
/// it is NOT a completion signal — the Responses API always emits
/// `response.completed` before the SSE terminator. A `[DONE]` that arrives
/// without a preceding `response.completed` is a truncated stream and must
/// surface as a retryable error rather than a successful `Stop`.
#[test]
fn test_parse_sse_done_marker_without_completed_is_truncated() {
    let sse_body = r#"data: {"type":"response.output_text.delta","delta":"hello"}

data: [DONE]

data: {"type":"response.output_text.delta","delta":" ignored"}

"#;
    let result = parse_sse_response(sse_body);
    assert!(
        result.is_err(),
        "[DONE] without response.completed is truncated"
    );
    assert!(matches!(
        result.unwrap_err(),
        LlmError::InvalidResponse { .. }
    ));
}

/// `[DONE]` after a proper `response.completed` is the normal terminator:
/// parsing stops, later events are ignored, and the completed response is
/// returned successfully.
#[test]
fn test_parse_sse_completed_then_done_marker() {
    let sse_body = r#"data: {"type":"response.output_text.delta","delta":"hello"}

data: {"type":"response.completed","response":{"status":"completed","usage":{"input_tokens":1,"output_tokens":1}}}

data: [DONE]

data: {"type":"response.output_text.delta","delta":" ignored"}

"#;
    let result = parse_sse_response(sse_body);
    assert!(result.is_ok());
    let parsed = result.unwrap();
    assert_eq!(parsed.text_content, "hello");
}

/// Regression: a stream that ends WITHOUT a terminal `response.completed`
/// event (mid-stream disconnect) must NOT be reported as a successful
/// `Stop`. Partial content that ends abruptly is a truncated stream and
/// must surface as a retryable `InvalidResponse` so the loop retries.
#[test]
fn test_parse_sse_truncated_stream_with_partial_text_is_error() {
    let sse_body = r#"data: {"type":"response.output_item.added","item":{"type":"message","role":"assistant","id":"msg_1"}}

data: {"type":"response.output_text.delta","delta":"partial answer that got cut"}

"#;
    let result = parse_sse_response(sse_body);
    assert!(
        result.is_err(),
        "truncated stream must not be reported as success"
    );
    match result.unwrap_err() {
        LlmError::InvalidResponse { reason, .. } => {
            assert!(
                reason.contains("response.completed"),
                "reason should explain the missing terminal event: {reason}"
            );
        }
        other => panic!("expected InvalidResponse, got {other:?}"),
    }
}

/// Regression: a stream with no events at all (no content, no terminal
/// completion) must surface as `EmptyResponse` so the caller can retry.
#[test]
fn test_parse_sse_truncated_stream_empty_is_error() {
    let sse_body = ":keepalive\n\n";
    let result = parse_sse_response(sse_body);
    assert!(
        result.is_err(),
        "empty truncated stream must not be reported as success"
    );
    assert!(matches!(
        result.unwrap_err(),
        LlmError::EmptyResponse { .. }
    ));
}

/// A stream that produced tool calls but ended WITHOUT a terminal
/// `response.completed` is still truncated and must surface as a retryable
/// error, not a successful `ToolUse`.
#[test]
fn test_parse_sse_truncated_stream_with_tool_calls_is_error() {
    let sse_body = r#"data: {"type":"response.output_item.added","item":{"type":"function_call","id":"fc_1","call_id":"call_abc","name":"search"}}

data: {"type":"response.output_item.done","item":{"type":"function_call","id":"fc_1","call_id":"call_abc","name":"search","arguments":"{\"query\":\"test\"}"}}

"#;
    let result = parse_sse_response(sse_body);
    assert!(
        result.is_err(),
        "truncated stream with tool calls must not be reported as success"
    );
    assert!(matches!(
        result.unwrap_err(),
        LlmError::InvalidResponse { .. }
    ));
}

/// An SSE `error` event whose message is a context-overflow error must map
/// to `ContextLengthExceeded` (not generic `RequestFailed`) so the loop's
/// context-shrink recovery fires.
#[test]
fn test_parse_sse_error_context_length_maps_to_context_exceeded() {
    let sse_body = r#"data: {"type":"error","code":"context_length_exceeded","message":"This model's maximum context length is 128000 tokens. However, your messages resulted in 150000 tokens."}

"#;
    let result = parse_sse_response(sse_body);
    match result {
        Err(LlmError::ContextLengthExceeded { used, limit }) => {
            assert_eq!(used, 150000);
            assert_eq!(limit, 128000);
        }
        other => panic!("expected ContextLengthExceeded, got {other:?}"),
    }
}

/// A `response.failed` event whose error message is a context-overflow
/// error must also map to `ContextLengthExceeded`.
#[test]
fn test_parse_sse_failed_context_length_maps_to_context_exceeded() {
    let sse_body = r#"data: {"type":"response.failed","response":{"status":"failed","status_details":{"error":{"message":"prompt is too long: 200000 tokens > 128000 maximum"}}}}

"#;
    let result = parse_sse_response(sse_body);
    match result {
        Err(LlmError::ContextLengthExceeded { used, limit }) => {
            assert_eq!(used, 200000);
            assert_eq!(limit, 128000);
        }
        other => panic!("expected ContextLengthExceeded, got {other:?}"),
    }
}

#[tokio::test]
async fn test_provider_new() {
    let jwt = make_test_jwt("acct_test");
    let provider = OpenAiCodexProvider::new(
        "gpt-5.3-codex",
        "https://chatgpt.com/backend-api/codex",
        &jwt,
        300,
    );
    assert!(provider.is_ok());
    let provider = provider.unwrap();
    assert_eq!(provider.model_name(), "gpt-5.3-codex");
    assert_eq!(provider.cost_per_token(), (Decimal::ZERO, Decimal::ZERO));
    assert_eq!(provider.calculate_cost(1000, 500), Decimal::ZERO);
}

#[tokio::test]
async fn test_update_token() {
    let jwt1 = make_test_jwt("acct_old");
    let provider = OpenAiCodexProvider::new(
        "gpt-5.3-codex",
        "https://chatgpt.com/backend-api/codex",
        &jwt1,
        300,
    )
    .unwrap();

    let jwt2 = make_test_jwt("acct_new");
    let result = provider.update_token(&jwt2).await;
    assert!(result.is_ok());

    // Verify account_id was updated
    let auth = provider.auth.read().await;
    assert_eq!(auth.account_id, "acct_new");
}

#[test]
fn test_build_request_body_structure() {
    let jwt = make_test_jwt("acct_test");
    let provider = OpenAiCodexProvider::new(
        "gpt-5.3-codex",
        "https://chatgpt.com/backend-api/codex",
        &jwt,
        300,
    )
    .unwrap();

    let messages = vec![
        ChatMessage::system("You are helpful"),
        ChatMessage::user("Hello"),
    ];

    let body = provider.build_request_body(&messages, None);

    assert_eq!(body["model"], "gpt-5.3-codex");
    assert_eq!(body["store"], false);
    assert_eq!(body["stream"], true);
    assert_eq!(body["instructions"], "You are helpful");
    // input should only contain the user message, not system
    let input = body["input"].as_array().unwrap();
    assert_eq!(input.len(), 1);
    assert_eq!(input[0]["role"], "user");
    // No tools
    assert!(body.get("tools").is_none());
}

#[test]
fn test_build_request_body_with_tools() {
    let jwt = make_test_jwt("acct_test");
    let provider = OpenAiCodexProvider::new(
        "gpt-5.3-codex",
        "https://chatgpt.com/backend-api/codex",
        &jwt,
        300,
    )
    .unwrap();

    let messages = vec![ChatMessage::user("Search for X")];
    let tools = vec![ToolDefinition {
        name: "search".to_string(),
        description: "Search for things".to_string(),
        parameters: serde_json::json!({"type": "object"}),
    }];

    let body = provider.build_request_body(&messages, Some(&tools));

    assert!(body.get("tools").is_some());
    let tools_arr = body["tools"].as_array().unwrap();
    assert_eq!(tools_arr.len(), 1);
    assert_eq!(tools_arr[0]["type"], "function");
    assert_eq!(body["tool_choice"], "auto");
    assert_eq!(body["parallel_tool_calls"], true);
}

#[test]
fn test_parse_sse_multiple_tool_calls() {
    let sse_body = r#"data: {"type":"response.output_item.added","item":{"type":"function_call","id":"fc_1","call_id":"call_1","name":"read_file"}}

data: {"type":"response.function_call_arguments.done","item_id":"fc_1","arguments":"{\"path\":\"/tmp/a\"}"}

data: {"type":"response.output_item.done","item":{"type":"function_call","id":"fc_1","call_id":"call_1","name":"read_file","arguments":"{\"path\":\"/tmp/a\"}"}}

data: {"type":"response.output_item.added","item":{"type":"function_call","id":"fc_2","call_id":"call_2","name":"read_file"}}

data: {"type":"response.function_call_arguments.done","item_id":"fc_2","arguments":"{\"path\":\"/tmp/b\"}"}

data: {"type":"response.output_item.done","item":{"type":"function_call","id":"fc_2","call_id":"call_2","name":"read_file","arguments":"{\"path\":\"/tmp/b\"}"}}

data: {"type":"response.completed","response":{"status":"completed","usage":{"input_tokens":20,"output_tokens":12}}}

"#;
    let result = parse_sse_response(sse_body);
    assert!(result.is_ok());
    let parsed = result.unwrap();
    assert_eq!(parsed.tool_calls.len(), 2);
    assert_eq!(parsed.tool_calls[0].id, "call_1");
    assert_eq!(parsed.tool_calls[0].name, "read_file");
    assert_eq!(parsed.tool_calls[1].id, "call_2");
    assert_eq!(parsed.tool_calls[1].name, "read_file");
    assert_eq!(parsed.finish_reason, FinishReason::ToolUse);
}

/// Regression test: tool names with dots (e.g. MCP tools) must be sanitized
/// to match OpenAI's `^[a-zA-Z0-9_-]+$` pattern.
#[test]
fn test_sanitize_tool_name_replaces_dots() {
    assert_eq!(super::sanitize_tool_name("memory_search"), "memory_search");
    assert_eq!(
        super::sanitize_tool_name("mcp.server.tool"),
        "mcp_server_tool"
    );
    assert_eq!(super::sanitize_tool_name("tool@v2"), "tool_v2");
    assert_eq!(super::sanitize_tool_name("my-tool"), "my-tool");
}

/// Regression test: convert_tool_definition sanitizes the name.
#[test]
fn test_convert_tool_definition_sanitizes_name() {
    let tool = ToolDefinition {
        name: "mcp.server.search".to_string(),
        description: "Search".to_string(),
        parameters: serde_json::json!({"type": "object", "properties": {}}),
    };
    let json = super::convert_tool_definition(&tool);
    assert_eq!(json["name"], "mcp_server_search");
}

/// Regression test: function_call items sanitize tool names.
#[test]
fn test_convert_message_sanitizes_tool_call_name() {
    let tool_calls = vec![ToolCall {
        id: "call_1".to_string(),
        name: "mcp.server.search".to_string(),
        arguments: serde_json::json!({"q": "test"}),
        reasoning: None,
        signature: None,
        arguments_parse_error: None,
    }];
    let msg = ChatMessage::assistant_with_tool_calls(None, tool_calls);
    let items = super::convert_message(&msg, 0);
    assert_eq!(items[0]["name"], "mcp_server_search");
}

/// Regression: sanitized tool names in API responses must be reverse-mapped
/// back to original names so the tool registry can look them up.
#[test]
fn test_sanitized_name_reverse_mapping() {
    use std::collections::HashMap;

    let tools = [
        ToolDefinition {
            name: "mcp.server.search".to_string(),
            description: "Search".to_string(),
            parameters: serde_json::json!({"type": "object", "properties": {}}),
        },
        ToolDefinition {
            name: "memory_search".to_string(),
            description: "Memory".to_string(),
            parameters: serde_json::json!({"type": "object", "properties": {}}),
        },
    ];

    // Build name map (same logic as complete_with_tools)
    let name_map: HashMap<String, String> = tools
        .iter()
        .filter_map(|t| {
            let sanitized = super::sanitize_tool_name(&t.name);
            if sanitized != t.name {
                Some((sanitized, t.name.clone()))
            } else {
                None
            }
        })
        .collect();

    // Only the MCP tool should appear (its name changed)
    assert_eq!(name_map.len(), 1);
    assert_eq!(
        name_map.get("mcp_server_search"),
        Some(&"mcp.server.search".to_string())
    );

    // Simulate a tool call coming back with the sanitized name
    let mut tc = ToolCall {
        id: "call_1".to_string(),
        name: "mcp_server_search".to_string(),
        arguments: serde_json::json!({}),
        reasoning: None,
        signature: None,
        arguments_parse_error: None,
    };
    if let Some(original) = name_map.get(&tc.name) {
        tc.name = original.clone();
    }
    assert_eq!(tc.name, "mcp.server.search");
}

/// Regression test for #1969: orphaned tool results must be sanitized
/// before building the request body, otherwise the Responses API returns
/// HTTP 400 because function_call_output references a non-existent call_id.
#[test]
fn test_build_request_sanitizes_orphaned_tool_results() {
    use crate::provider::sanitize_tool_messages;

    // An orphaned tool result: no preceding assistant message with a
    // matching tool_call for "call_orphan".
    let mut messages = vec![
        ChatMessage::system("You are helpful"),
        ChatMessage::user("hello"),
        ChatMessage::assistant("I'll use a tool"),
        ChatMessage::tool_result("call_orphan", "search", "found 3 results"),
    ];

    // Before sanitization the message is Role::Tool with a tool_call_id.
    assert_eq!(messages[3].role, Role::Tool);
    assert_eq!(messages[3].tool_call_id, Some("call_orphan".to_string()));

    sanitize_tool_messages(&mut messages);

    // After sanitization it must be rewritten to a user message.
    assert_eq!(messages[3].role, Role::User);
    assert!(messages[3].content.contains("[Tool `search` returned:"));
    assert!(messages[3].content.contains("found 3 results"));
    assert!(messages[3].tool_call_id.is_none());
    assert!(messages[3].name.is_none());

    // Verify the rewritten message converts to a user input item (not
    // a function_call_output that would cause HTTP 400).
    let jwt = make_test_jwt("acct_test");
    let provider = OpenAiCodexProvider::new(
        "gpt-5.3-codex",
        "https://chatgpt.com/backend-api/codex",
        &jwt,
        300,
    )
    .unwrap();

    let body = provider.build_request_body(&messages, None);
    let input = body["input"].as_array().unwrap();

    // Should have 3 non-system items: user, assistant, rewritten-user
    assert_eq!(input.len(), 3);
    // The last item must be a user message, not a function_call_output
    assert_eq!(input[2]["role"], "user");
    assert!(
        input[2]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("[Tool `search` returned:")
    );
}
