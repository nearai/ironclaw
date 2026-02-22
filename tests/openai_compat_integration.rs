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

#[derive(Default)]
struct MockLlmState {
    completion_models: tokio::sync::Mutex<Vec<Option<String>>>,
    tool_completion_models: tokio::sync::Mutex<Vec<Option<String>>>,
}

struct MockLlmProvider {
    state: Arc<MockLlmState>,
}

impl MockLlmProvider {
    fn new(state: Arc<MockLlmState>) -> Self {
        Self { state }
    }
}

#[async_trait]
impl LlmProvider for MockLlmProvider {
    fn model_name(&self) -> &str {
        "mock-model-v1"
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        (Decimal::ZERO, Decimal::ZERO)
    }

    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        self.state
            .completion_models
            .lock()
            .await
            .push(req.model.clone());

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
            cache_read_input_tokens: 0,
            cache_creation_input_tokens: 0,
        })
    }

    async fn complete_with_tools(
        &self,
        req: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        self.state
            .tool_completion_models
            .lock()
            .await
            .push(req.model.clone());

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
                cache_read_input_tokens: 0,
                cache_creation_input_tokens: 0,
            })
        } else {
            Ok(ToolCompletionResponse {
                content: Some("No tools available".to_string()),
                tool_calls: vec![],
                input_tokens: 10,
                output_tokens: 4,
                finish_reason: FinishReason::Stop,
                cache_read_input_tokens: 0,
                cache_creation_input_tokens: 0,
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

struct FixedModelProvider {
    model: &'static str,
}

impl FixedModelProvider {
    fn new(model: &'static str) -> Self {
        Self { model }
    }
}

#[async_trait]
impl LlmProvider for FixedModelProvider {
    fn model_name(&self) -> &str {
        self.model
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        (Decimal::ZERO, Decimal::ZERO)
    }

    async fn complete(&self, _req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        Ok(CompletionResponse {
            content: "fixed response".to_string(),
            input_tokens: 10,
            output_tokens: 5,
            finish_reason: FinishReason::Stop,
            cache_read_input_tokens: 0,
            cache_creation_input_tokens: 0,
        })
    }

    async fn complete_with_tools(
        &self,
        _req: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        Ok(ToolCompletionResponse {
            content: Some("fixed response".to_string()),
            tool_calls: vec![],
            input_tokens: 10,
            output_tokens: 5,
            finish_reason: FinishReason::Stop,
            cache_read_input_tokens: 0,
            cache_creation_input_tokens: 0,
        })
    }

    fn effective_model_name(&self, _requested_model: Option<&str>) -> String {
        self.model.to_string()
    }
}

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

async fn start_test_server() -> (SocketAddr, Arc<GatewayState>, Arc<MockLlmState>) {
    let mock_state = Arc::new(MockLlmState::default());

    let llm_provider: Arc<dyn LlmProvider> = Arc::new(MockLlmProvider::new(mock_state.clone()));
    let (bound_addr, state) = start_test_server_with_provider(llm_provider).await;

    (bound_addr, state, mock_state)
}

async fn start_test_server_with_provider(
    llm_provider: Arc<dyn LlmProvider>,
) -> (SocketAddr, Arc<GatewayState>) {
    let state = Arc::new(GatewayState {
        msg_tx: tokio::sync::RwLock::new(None),
        sse: SseManager::new(),
        workspace: None,
        session_manager: None,
        log_broadcaster: None,
        log_level_handle: None,
        extension_manager: None,
        tool_registry: None,
        store: None,
        job_manager: None,
        prompt_queue: None,
        user_id: "test-user".to_string(),
        shutdown_tx: tokio::sync::RwLock::new(None),
        ws_tracker: Some(Arc::new(WsConnectionTracker::new())),
        llm_provider: Some(llm_provider),
        skill_registry: None,
        skill_catalog: None,
        chat_rate_limiter: ironclaw::channels::web::server::RateLimiter::new(30, 60),
        registry_entries: Vec::new(),
        cost_guard: None,
        startup_time: std::time::Instant::now(),
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
    let (addr, _state, mock_state) = start_test_server().await;
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

    let content = body["choices"][0]["message"]["content"].as_str().unwrap();
    assert!(
        content.contains("Hello world"),
        "Expected echo, got: {}",
        content
    );

    // Check usage
    assert_eq!(body["usage"]["prompt_tokens"], 10);
    assert_eq!(body["usage"]["completion_tokens"], 5);
    assert_eq!(body["usage"]["total_tokens"], 15);

    let models = mock_state.completion_models.lock().await;
    assert_eq!(*models, vec![Some("mock-model-v1".to_string())]);
}

#[tokio::test]
async fn test_chat_completions_with_system_message() {
    let (addr, _state, _mock_state) = start_test_server().await;
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
    let content = body["choices"][0]["message"]["content"].as_str().unwrap();
    assert!(content.contains("2+2"));
}

#[tokio::test]
async fn test_chat_completions_with_tools() {
    let (addr, _state, mock_state) = start_test_server().await;
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

    let models = mock_state.tool_completion_models.lock().await;
    assert_eq!(*models, vec![Some("mock-model-v1".to_string())]);
}

#[tokio::test]
async fn test_chat_completions_streaming() {
    let (addr, _state, mock_state) = start_test_server().await;
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
        resp.headers()
            .get("x-ironclaw-streaming")
            .and_then(|v| v.to_str().ok()),
        Some("simulated"),
        "Expected x-ironclaw-streaming: simulated header"
    );

    let text = resp.text().await.unwrap();

    // Should contain SSE data lines
    assert!(
        text.contains("data:"),
        "Expected SSE data lines, got: {}",
        text
    );
    // Should end with [DONE]
    assert!(
        text.contains("[DONE]"),
        "Expected [DONE] sentinel, got: {}",
        text
    );
    // Should contain the role chunk
    assert!(
        text.contains("\"role\":\"assistant\""),
        "Expected role chunk, got: {}",
        text
    );

    // Collect all content from the chunks
    let mut full_content = String::new();
    for line in text.lines() {
        if let Some(data) = line.strip_prefix("data:") {
            let data = data.trim();
            if data == "[DONE]" {
                continue;
            }
            if let Ok(chunk) = serde_json::from_str::<serde_json::Value>(data)
                && let Some(content) = chunk["choices"][0]["delta"]["content"].as_str()
            {
                full_content.push_str(content);
            }
        }
    }
    assert!(
        full_content.contains("Stream test"),
        "Expected reassembled content to contain 'Stream test', got: '{}'",
        full_content
    );

    let models = mock_state.completion_models.lock().await;
    assert_eq!(*models, vec![Some("mock-model-v1".to_string())]);
}

#[tokio::test]
async fn test_chat_completions_empty_messages() {
    let (addr, _state, _mock_state) = start_test_server().await;
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
    assert!(body["error"]["message"].as_str().unwrap().contains("empty"));
}

#[tokio::test]
async fn test_chat_completions_model_override() {
    let (addr, _state, mock_state) = start_test_server().await;
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

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["model"], "gpt-4");

    let models = mock_state.completion_models.lock().await;
    assert_eq!(*models, vec![Some("gpt-4".to_string())]);
}

#[tokio::test]
async fn test_chat_completions_uses_effective_model_when_override_ignored() {
    let provider: Arc<dyn LlmProvider> = Arc::new(FixedModelProvider::new("configured-model"));
    let (addr, _state) = start_test_server_with_provider(provider).await;
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

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(body["model"], "configured-model");
}

#[tokio::test]
async fn test_chat_completions_streaming_uses_effective_model_when_override_ignored() {
    let provider: Arc<dyn LlmProvider> = Arc::new(FixedModelProvider::new("configured-model"));
    let (addr, _state) = start_test_server_with_provider(provider).await;
    let url = format!("http://{}/v1/chat/completions", addr);

    let resp = client()
        .post(&url)
        .bearer_auth(AUTH_TOKEN)
        .json(&serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "Hi"}],
            "stream": true
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 200);
    let text = resp.text().await.unwrap();
    assert!(
        text.contains("\"model\":\"configured-model\""),
        "Expected streaming chunks to report configured model, got: {}",
        text
    );
}

#[tokio::test]
async fn test_chat_completions_model_too_long() {
    let (addr, _state, mock_state) = start_test_server().await;
    let url = format!("http://{}/v1/chat/completions", addr);

    let resp = client()
        .post(&url)
        .bearer_auth(AUTH_TOKEN)
        .json(&serde_json::json!({
            "model": "m".repeat(300),
            "messages": [{"role": "user", "content": "Hi"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 400);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("model"),
        "Expected model validation error, got: {}",
        body
    );

    // Validation should fail before provider invocation.
    let models = mock_state.completion_models.lock().await;
    assert!(
        models.is_empty(),
        "provider should not be called: {:?}",
        *models
    );
}

#[tokio::test]
async fn test_chat_completions_model_with_control_chars() {
    let (addr, _state, mock_state) = start_test_server().await;
    let url = format!("http://{}/v1/chat/completions", addr);

    let resp = client()
        .post(&url)
        .bearer_auth(AUTH_TOKEN)
        .json(&serde_json::json!({
            "model": "gpt-4\noops",
            "messages": [{"role": "user", "content": "Hi"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 400);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("control"),
        "Expected model validation error, got: {}",
        body
    );

    // Validation should fail before provider invocation.
    let models = mock_state.completion_models.lock().await;
    assert!(
        models.is_empty(),
        "provider should not be called: {:?}",
        *models
    );
}

#[tokio::test]
async fn test_chat_completions_model_with_surrounding_whitespace() {
    let (addr, _state, mock_state) = start_test_server().await;
    let url = format!("http://{}/v1/chat/completions", addr);

    let resp = client()
        .post(&url)
        .bearer_auth(AUTH_TOKEN)
        .json(&serde_json::json!({
            "model": " gpt-4 ",
            "messages": [{"role": "user", "content": "Hi"}]
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 400);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("leading or trailing whitespace"),
        "Expected model validation error, got: {}",
        body
    );

    let models = mock_state.completion_models.lock().await;
    assert!(
        models.is_empty(),
        "provider should not be called: {:?}",
        *models
    );
}

#[tokio::test]
async fn test_chat_completions_no_auth() {
    let (addr, _state, _mock_state) = start_test_server().await;
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
    let (addr, _state, _mock_state) = start_test_server().await;
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
    let (addr, _state, _mock_state) = start_test_server().await;
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
        log_level_handle: None,
        extension_manager: None,
        tool_registry: None,
        store: None,
        job_manager: None,
        prompt_queue: None,
        user_id: "test-user".to_string(),
        shutdown_tx: tokio::sync::RwLock::new(None),
        ws_tracker: Some(Arc::new(WsConnectionTracker::new())),
        llm_provider: None, // No LLM!
        skill_registry: None,
        skill_catalog: None,
        chat_rate_limiter: ironclaw::channels::web::server::RateLimiter::new(30, 60),
        registry_entries: Vec::new(),
        cost_guard: None,
        startup_time: std::time::Instant::now(),
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
    let (addr, _state, _mock_state) = start_test_server().await;
    let url = format!("http://{}/v1/chat/completions", addr);

    // Build a payload over 1 MB (the gateway's DefaultBodyLimit)
    let big_content = "x".repeat(2 * 1024 * 1024);
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
