mod support;

use std::collections::HashMap;

use secrecy::SecretString;

use ironclaw::llm::{
    CacheRetention, ChatMessage, LlmConfig, NearAiConfig, ProviderProtocol, RegistryProviderConfig,
    SessionConfig, ToolCompletionRequest, ToolDefinition, create_llm_provider,
    create_session_manager,
};

use crate::support::mock_openai_server::{
    MockOpenAiResponse, MockOpenAiRule, MockOpenAiServerBuilder,
};

fn dummy_nearai_config() -> NearAiConfig {
    NearAiConfig {
        model: "unused".to_string(),
        cheap_model: None,
        base_url: "http://127.0.0.1:1".to_string(),
        api_key: None,
        fallback_model: None,
        max_retries: 0,
        circuit_breaker_threshold: None,
        circuit_breaker_recovery_secs: 30,
        response_cache_enabled: false,
        response_cache_ttl_secs: 3600,
        response_cache_max_entries: 100,
        failover_cooldown_secs: 300,
        failover_cooldown_threshold: 3,
        smart_routing_cascade: false,
    }
}

#[tokio::test]
async fn gemini_provider_replays_thought_signature_in_second_turn_tool_history() {
    let first_turn_raw = serde_json::json!({
        "id": "chatcmpl-mock-1",
        "object": "chat.completion",
        "created": 0,
        "model": "mock-model",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": serde_json::Value::Null,
                "tool_calls": [{
                    "id": "call_search_1",
                    "type": "function",
                    "function": {
                        "name": "memory_search",
                        "arguments": "{\"query\":\"telegram\"}",
                        "thoughtSignature": "sig_from_gemini"
                    }
                }]
            },
            "finish_reason": "tool_calls"
        }],
        "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15},
        "candidates": [{
            "content": {
                "parts": [{
                    "functionCall": {
                        "name": "memory_search",
                        "thoughtSignature": "sig_from_gemini"
                    }
                }]
            }
        }]
    });

    let mock = MockOpenAiServerBuilder::new()
        .with_rule(MockOpenAiRule::on_user_contains(
            "search",
            MockOpenAiResponse::Raw(first_turn_raw),
        ))
        .with_default_response(MockOpenAiResponse::Text("done".to_string()))
        .start()
        .await;

    let llm_config = LlmConfig {
        backend: "openai_compatible".to_string(),
        session: SessionConfig::default(),
        nearai: dummy_nearai_config(),
        provider: Some(RegistryProviderConfig {
            protocol: ProviderProtocol::OpenAiCompletions,
            provider_id: "gemini".to_string(),
            api_key: Some(SecretString::from("dummy".to_string())),
            base_url: mock.openai_base_url(),
            model: "mock-model".to_string(),
            extra_headers: Vec::new(),
            oauth_token: None,
            is_codex_chatgpt: false,
            refresh_token: None,
            auth_path: None,
            cache_retention: CacheRetention::None,
            unsupported_params: Vec::new(),
        }),
        bedrock: None,
        gemini_oauth: None,
        openai_codex: None,
        request_timeout_secs: 10,
        cheap_model: None,
        smart_routing_cascade: false,
        max_retries: 0,
        circuit_breaker_threshold: None,
        circuit_breaker_recovery_secs: 30,
        response_cache_enabled: false,
        response_cache_ttl_secs: 3600,
        response_cache_max_entries: 100,
    };

    let session = create_session_manager(SessionConfig::default()).await;
    let llm = create_llm_provider(&llm_config, session)
        .await
        .expect("provider should initialize");

    let tools = vec![ToolDefinition {
        name: "memory_search".to_string(),
        description: "search".to_string(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {"query": {"type": "string"}},
            "required": ["query"]
        }),
    }];

    let first_req = ToolCompletionRequest::new(vec![ChatMessage::user("search memory")], tools);
    let first_resp = llm
        .complete_with_tools(first_req)
        .await
        .expect("first turn should succeed");
    assert_eq!(first_resp.tool_calls.len(), 1);
    assert_eq!(
        first_resp.tool_calls[0].thought_signature.as_deref(),
        Some("sig_from_gemini")
    );

    let second_messages = vec![
        ChatMessage::user("search memory"),
        ChatMessage::assistant_with_tool_calls(None, first_resp.tool_calls.clone()),
        ChatMessage::tool_result("call_search_1", "memory_search", "ok"),
        ChatMessage::user("continue"),
    ];
    let mut second_req = ToolCompletionRequest::new(second_messages, vec![]);
    second_req.metadata = HashMap::new();

    let _ = llm
        .complete_with_tools(second_req)
        .await
        .expect("second turn should succeed");

    let requests = mock.requests().await;
    assert!(requests.len() >= 2, "expected two outbound requests");
    let second_payload = serde_json::to_string(&requests[1]).expect("serialize second payload");
    assert!(
        second_payload.contains("thoughtSignature") && second_payload.contains("sig_from_gemini"),
        "expected replayed Gemini thought signature in second turn payload: {second_payload}"
    );

    mock.shutdown().await;
}
