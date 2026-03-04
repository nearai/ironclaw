//! Focused regression tests for OpenAI compatibility edge cases.

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
    CompletionRequest, CompletionResponse, FinishReason, LlmProvider, Role, ToolCall,
    ToolCompletionRequest, ToolCompletionResponse,
};

const AUTH_TOKEN: &str = "test-openai-regression-token";

#[derive(Default)]
struct CaptureState {
    complete_requests: tokio::sync::Mutex<Vec<CompletionRequest>>,
    tool_requests: tokio::sync::Mutex<Vec<ToolCompletionRequest>>,
}

struct CaptureProvider {
    state: Arc<CaptureState>,
}

impl CaptureProvider {
    fn new(state: Arc<CaptureState>) -> Self {
        Self { state }
    }
}

#[async_trait]
impl LlmProvider for CaptureProvider {
    fn model_name(&self) -> &str {
        "mock-model"
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        (Decimal::ZERO, Decimal::ZERO)
    }

    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        self.state.complete_requests.lock().await.push(req.clone());
        let echoed = req
            .messages
            .iter()
            .rev()
            .find(|m| m.role == Role::User)
            .map(|m| m.content.clone())
            .unwrap_or_default();
        Ok(CompletionResponse {
            content: format!("echo: {}", echoed),
            input_tokens: 1,
            output_tokens: 1,
            finish_reason: FinishReason::Stop,
        })
    }

    async fn complete_with_tools(
        &self,
        req: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        self.state.tool_requests.lock().await.push(req.clone());
        let first_tool = req.tools.first().ok_or_else(|| LlmError::InvalidResponse {
            provider: "mock".to_string(),
            reason: "no tools provided".to_string(),
        })?;
        Ok(ToolCompletionResponse {
            content: None,
            tool_calls: vec![ToolCall {
                id: "call_1".to_string(),
                name: first_tool.name.clone(),
                arguments: serde_json::json!({"ok": true}),
            }],
            input_tokens: 1,
            output_tokens: 1,
            finish_reason: FinishReason::ToolUse,
        })
    }
}

async fn start_test_server() -> (SocketAddr, Arc<CaptureState>) {
    let capture_state = Arc::new(CaptureState::default());
    let llm_provider: Arc<dyn LlmProvider> = Arc::new(CaptureProvider::new(capture_state.clone()));

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
        restart_requested: std::sync::atomic::AtomicBool::new(false),
    });

    let addr: SocketAddr = "127.0.0.1:0".parse().expect("valid addr");
    let bound_addr = start_server(addr, state, AUTH_TOKEN.to_string())
        .await
        .expect("start server");
    (bound_addr, capture_state)
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("client")
}

#[tokio::test]
async fn developer_role_is_accepted() {
    let (addr, capture_state) = start_test_server().await;
    let url = format!("http://{}/v1/chat/completions", addr);

    let resp = client()
        .post(url)
        .bearer_auth(AUTH_TOKEN)
        .json(&serde_json::json!({
            "model": "mock-model",
            "messages": [
                {"role":"developer","content":"Follow policy"},
                {"role":"user","content":"Hello"}
            ]
        }))
        .send()
        .await
        .expect("request");

    assert_eq!(resp.status(), 200);
    let requests = capture_state.complete_requests.lock().await;
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].messages[0].role, Role::System);
    assert_eq!(requests[0].messages[0].content, "Follow policy");
}

#[tokio::test]
async fn array_content_is_accepted() {
    let (addr, capture_state) = start_test_server().await;
    let url = format!("http://{}/v1/chat/completions", addr);

    let resp = client()
        .post(url)
        .bearer_auth(AUTH_TOKEN)
        .json(&serde_json::json!({
            "model": "mock-model",
            "messages": [
                {"role":"user","content":[
                    {"type":"text","text":"Hello "},
                    {"type":"text","text":"array"}
                ]}
            ]
        }))
        .send()
        .await
        .expect("request");

    assert_eq!(resp.status(), 200);
    let requests = capture_state.complete_requests.lock().await;
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].messages[0].content, "Hello array");
}

#[tokio::test]
async fn named_tool_choice_filters_to_named_tool() {
    let (addr, capture_state) = start_test_server().await;
    let url = format!("http://{}/v1/chat/completions", addr);

    let resp = client()
        .post(url)
        .bearer_auth(AUTH_TOKEN)
        .json(&serde_json::json!({
            "model": "mock-model",
            "messages": [{"role":"user","content":"pick tool"}],
            "tools": [
                {"type":"function","function":{"name":"tool_a","parameters":{"type":"object","properties":{}}}},
                {"type":"function","function":{"name":"tool_b","parameters":{"type":"object","properties":{}}}}
            ],
            "tool_choice": {"type":"function","function":{"name":"tool_b"}}
        }))
        .send()
        .await
        .expect("request");

    assert_eq!(resp.status(), 200);
    let body: serde_json::Value = resp.json().await.expect("json");
    assert_eq!(
        body["choices"][0]["message"]["tool_calls"][0]["function"]["name"],
        "tool_b"
    );

    let tool_requests = capture_state.tool_requests.lock().await;
    assert_eq!(tool_requests.len(), 1);
    assert_eq!(tool_requests[0].tools.len(), 1);
    assert_eq!(tool_requests[0].tools[0].name, "tool_b");
    assert_eq!(tool_requests[0].tool_choice.as_deref(), Some("required"));
}

#[tokio::test]
async fn bad_tool_args_return_openai_error_json() {
    let (addr, _capture_state) = start_test_server().await;
    let url = format!("http://{}/v1/chat/completions", addr);

    let resp = client()
        .post(url)
        .bearer_auth(AUTH_TOKEN)
        .json(&serde_json::json!({
            "model": "mock-model",
            "messages": [
                {
                    "role":"assistant",
                    "content":null,
                    "tool_calls":[
                        {
                            "id":"call_1",
                            "type":"function",
                            "function":{"name":"bad_tool","arguments":"{not-json"}
                        }
                    ]
                }
            ]
        }))
        .send()
        .await
        .expect("request");

    assert_eq!(resp.status(), 400);
    let body: serde_json::Value = resp.json().await.expect("json");
    assert_eq!(body["error"]["type"], "invalid_request_error");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("function.arguments")
    );
}

#[tokio::test]
async fn malformed_json_body_returns_openai_error_json() {
    let (addr, _capture_state) = start_test_server().await;
    let url = format!("http://{}/v1/chat/completions", addr);

    let resp = client()
        .post(url)
        .bearer_auth(AUTH_TOKEN)
        .header("content-type", "application/json")
        .body(r#"{"model":"mock-model","messages":["#)
        .send()
        .await
        .expect("request");

    assert_eq!(resp.status(), 400);
    let body: serde_json::Value = resp.json().await.expect("json");
    assert_eq!(body["error"]["type"], "invalid_request_error");
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap_or("")
            .contains("Invalid JSON body")
    );
}
