//! Integration tests for the OpenAI-compatible API endpoints.
//!
//! Uses a mock LLM provider so no real API key is needed.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use rust_decimal::Decimal;

use ironclaw::channels::web::server::{GatewayState, start_server};
use ironclaw::channels::web::sse::SseManager;
use ironclaw::channels::web::ws::WsConnectionTracker;
use ironclaw::error::LlmError;
use ironclaw::llm::{
    CompletionRequest, CompletionResponse, FinishReason, LlmProvider, ToolCompletionRequest,
    ToolCompletionResponse,
};

const AUTH_TOKEN: &str = "test-openai-token";

// ---------------------------------------------------------------------------
// Mock LLM provider
// ---------------------------------------------------------------------------

struct MockLlmProvider;

#[async_trait]
impl LlmProvider for MockLlmProvider {
    fn model_name(&self) -> &str {
        "mock-model-v1"
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        (Decimal::ZERO, Decimal::ZERO)
    }

    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        // Echo the last user message back
        let user_msg = req
            .messages
            .iter()
            .rev()
            .find(|m| m.role == ironclaw::llm::Role::User)
            .map(|m| m.content.clone())
            .unwrap_or_else(|| "no user message".to_string());

        Ok(CompletionResponse {
            content: format!("Mock response to: {}", user_msg),
            input_tokens: 10,
            output_tokens: 5,
            finish_reason: FinishReason::Stop,
            response_id: None,
        })
    }

    async fn complete_with_tools(
        &self,
        req: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        // If tools are provided, return a tool call
        if let Some(tool) = req.tools.first() {
            Ok(ToolCompletionResponse {
                content: None,
                tool_calls: vec![ironclaw::llm::ToolCall {
                    id: "call_mock_001".to_string(),
                    name: tool.name.clone(),
                    arguments: serde_json::json!({"test": true}),
                }],
                input_tokens: 15,
                output_tokens: 8,
                finish_reason: FinishReason::ToolUse,
                response_id: None,
            })
        } else {
            Ok(ToolCompletionResponse {
                content: Some("No tools available".to_string()),
                tool_calls: vec![],
                input_tokens: 10,
                output_tokens: 4,
                finish_reason: FinishReason::Stop,
                response_id: None,
            })
        }
    }

    async fn list_models(&self) -> Result<Vec<String>, LlmError> {
        Ok(vec![
            "mock-model-v1".to_string(),
            "mock-model-v2".to_string(),
        ])
    }
}

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

async fn start_test_server() -> (SocketAddr, Arc<GatewayState>) {
    let state = Arc::new(GatewayState {
        msg_tx: tokio::sync::RwLock::new(None),
        sse: SseManager::new(),
        workspace: None,
        session_manager: None,
        log_broadcaster: None,
        extension_manager: None,
        tool_registry: None,
        store: None,
        job_manager: None,
        prompt_queue: None,
        user_id: "test-user".to_string(),
        shutdown_tx: tokio::sync::RwLock::new(None),
        ws_tracker: Some(Arc::new(WsConnectionTracker::new())),
        llm_provider: Some(Arc::new(MockLlmProvider)),
    });

    let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let bound_addr = start_server(addr, state.clone(), AUTH_TOKEN.to_string())
        .await
        .expect("Failed to start test server");

    (bound_addr, state)
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_chat_completions_basic() {
    let (addr, _state) = start_test_server().await;
    let url = format!("http://{}/v1/chat/completions", addr);

    let resp = client()
        .post(&url)
        .bearer_auth(AUTH_TOKEN)
        .json(&serde_json::json!({
            "model": "mock-model-v1",
            "messages": [
                {"role": "user", "content": "Hello world"}
            ]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);

    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["object"], "chat.completion");
    assert_eq!(body["model"], "mock-model-v1");
    assert_eq!(body["choices"][0]["finish_reason"], "stop");

    let content = body["choices"][0]["message"]["content"]
        .as_str()
        .unwrap();
    assert!(
        content.contains("Hello world"),
        "Expected echo, got: {}",
        content
    );

    // Check usage
    assert_eq!(body["usage"]["prompt_tokens"], 10);
    assert_eq!(body["usage"]["completion_tokens"], 5);
    assert_eq!(body["usage"]["total_tokens"], 15);
}

#[tokio::test]
async fn test_chat_completions_with_system_message() {
    let (addr, _state) = start_test_server().await;
    let url = format!("http://{}/v1/chat/completions", addr);

    let resp = client()
        .post(&url)
        .bearer_auth(AUTH_TOKEN)
        .json(&serde_json::json!({
            "model": "mock-model-v1",
            "messages": [
                {"role": "system", "content": "You are helpful."},
                {"role": "user", "content": "What is 2+2?"}
            ],
            "temperature": 0.5,
            "max_tokens": 100
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    let content = body["choices"][0]["message"]["content"]
        .as_str()
        .unwrap();
    assert!(content.contains("2+2"));
}

#[tokio::test]
async fn test_chat_completions_with_tools() {
    let (addr, _state) = start_test_server().await;
    let url = format!("http://{}/v1/chat/completions", addr);

    let resp = client()
        .post(&url)
        .bearer_auth(AUTH_TOKEN)
        .json(&serde_json::json!({
            "model": "mock-model-v1",
            "messages": [
                {"role": "user", "content": "What's the weather?"}
            ],
            "tools": [{
                "type": "function",
                "function": {
                    "name": "get_weather",
                    "description": "Get the weather",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "location": {"type": "string"}
                        }
                    }
                }
            }]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();

    assert_eq!(body["choices"][0]["finish_reason"], "tool_calls");

    let tool_calls = &body["choices"][0]["message"]["tool_calls"];
    assert!(tool_calls.is_array());
    assert_eq!(tool_calls[0]["id"], "call_mock_001");
    assert_eq!(tool_calls[0]["type"], "function");
    assert_eq!(tool_calls[0]["function"]["name"], "get_weather");
}

#[tokio::test]
async fn test_chat_completions_streaming() {
    let (addr, _state) = start_test_server().await;
    let url = format!("http://{}/v1/chat/completions", addr);

    let resp = client()
        .post(&url)
        .bearer_auth(AUTH_TOKEN)
        .json(&serde_json::json!({
            "model": "mock-model-v1",
            "messages": [
                {"role": "user", "content": "Stream test"}
            ],
            "stream": true
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);

    // Check simulated streaming header
    assert_eq!(
        resp.headers().get("x-ironclaw-streaming").and_then(|v| v.to_str().ok()),
        Some("simulated"),
        "Expected x-ironclaw-streaming: simulated header"
    );

    let text = resp.text().await.unwrap();

    // Should contain SSE data lines
    assert!(text.contains("data:"), "Expected SSE data lines, got: {}", text);
    // Should end with [DONE]
    assert!(text.contains("[DONE]"), "Expected [DONE] sentinel, got: {}", text);
    // Should contain the role chunk
    assert!(text.contains("\"role\":\"assistant\""), "Expected role chunk, got: {}", text);

    // Collect all content from the chunks
    let mut full_content = String::new();
    for line in text.lines() {
        if let Some(data) = line.strip_prefix("data:") {
            let data = data.trim();
            if data == "[DONE]" {
                continue;
            }
            if let Ok(chunk) = serde_json::from_str::<serde_json::Value>(data) {
                if let Some(content) = chunk["choices"][0]["delta"]["content"].as_str() {
                    full_content.push_str(content);
                }
            }
        }
    }
    assert!(
        full_content.contains("Stream test"),
        "Expected reassembled content to contain 'Stream test', got: '{}'",
        full_content
    );
}

#[tokio::test]
async fn test_chat_completions_empty_messages() {
    let (addr, _state) = start_test_server().await;
    let url = format!("http://{}/v1/chat/completions", addr);

    let resp = client()
        .post(&url)
        .bearer_auth(AUTH_TOKEN)
        .json(&serde_json::json!({
            "model": "mock-model-v1",
            "messages": []
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 400);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body["error"]["message"]
        .as_str()
        .unwrap()
        .contains("empty"));
}

#[tokio::test]
async fn test_chat_completions_model_mismatch() {
    let (addr, _state) = start_test_server().await;
    let url = format!("http://{}/v1/chat/completions", addr);

    let resp = client()
        .post(&url)
        .bearer_auth(AUTH_TOKEN)
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "Hi"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 404);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["error"]["code"], "model_not_found");
    assert!(body["error"]["message"]
        .as_str()
        .unwrap()
        .contains("mock-model-v1"));
}

#[tokio::test]
async fn test_chat_completions_no_auth() {
    let (addr, _state) = start_test_server().await;
    let url = format!("http://{}/v1/chat/completions", addr);

    let resp = client()
        .post(&url)
        // No auth header
        .json(&serde_json::json!({
            "model": "mock-model-v1",
            "messages": [{"role": "user", "content": "Hi"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn test_models_endpoint() {
    let (addr, _state) = start_test_server().await;
    let url = format!("http://{}/v1/models", addr);

    let resp = client()
        .get(&url)
        .bearer_auth(AUTH_TOKEN)
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();

    assert_eq!(body["object"], "list");
    let data = body["data"].as_array().unwrap();
    assert_eq!(data.len(), 2);
    assert_eq!(data[0]["id"], "mock-model-v1");
    assert_eq!(data[1]["id"], "mock-model-v2");
    assert_eq!(data[0]["object"], "model");
}

#[tokio::test]
async fn test_models_no_auth() {
    let (addr, _state) = start_test_server().await;
    let url = format!("http://{}/v1/models", addr);

    let resp = client().get(&url).send().await.unwrap();
    assert_eq!(resp.status(), 401);
}

#[tokio::test]
async fn test_no_llm_provider_returns_503() {
    // Create state WITHOUT llm_provider
    let state = Arc::new(GatewayState {
        msg_tx: tokio::sync::RwLock::new(None),
        sse: SseManager::new(),
        workspace: None,
        session_manager: None,
        log_broadcaster: None,
        extension_manager: None,
        tool_registry: None,
        store: None,
        job_manager: None,
        prompt_queue: None,
        user_id: "test-user".to_string(),
        shutdown_tx: tokio::sync::RwLock::new(None),
        ws_tracker: Some(Arc::new(WsConnectionTracker::new())),
        llm_provider: None, // No LLM!
    });

    let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let bound_addr = start_server(addr, state, AUTH_TOKEN.to_string())
        .await
        .unwrap();

    let url = format!("http://{}/v1/chat/completions", bound_addr);
    let resp = client()
        .post(&url)
        .bearer_auth(AUTH_TOKEN)
        .json(&serde_json::json!({
            "model": "mock-model-v1",
            "messages": [{"role": "user", "content": "Hi"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 503);
}

#[tokio::test]
async fn test_chat_completions_body_too_large() {
    let (addr, _state) = start_test_server().await;
    let url = format!("http://{}/v1/chat/completions", addr);

    // Build a payload over 2 MB (axum's default body limit)
    let big_content = "x".repeat(3 * 1024 * 1024);
    let resp = client()
        .post(&url)
        .bearer_auth(AUTH_TOKEN)
        .json(&serde_json::json!({
            "model": "mock-model-v1",
            "messages": [{"role": "user", "content": big_content}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 413);
}
