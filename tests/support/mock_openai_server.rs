#![allow(dead_code)]

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::sync::{Mutex, oneshot};

#[derive(Clone)]
pub struct MockOpenAiRule {
    contains: String,
    response: MockOpenAiResponse,
}

impl MockOpenAiRule {
    pub fn on_user_contains(contains: impl Into<String>, response: MockOpenAiResponse) -> Self {
        Self {
            contains: contains.into(),
            response,
        }
    }
}

#[derive(Clone)]
pub enum MockOpenAiResponse {
    Text(String),
    ToolCalls(Vec<MockToolCall>),
    Raw(Value),
}

#[derive(Clone)]
pub struct MockToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

impl MockToolCall {
    pub fn new(id: impl Into<String>, name: impl Into<String>, arguments: Value) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            arguments,
        }
    }
}

#[derive(Default)]
pub struct MockOpenAiServerBuilder {
    models: Vec<String>,
    rules: Vec<MockOpenAiRule>,
    default_response: Option<MockOpenAiResponse>,
}

impl MockOpenAiServerBuilder {
    pub fn new() -> Self {
        Self {
            models: vec!["mock-model".to_string()],
            ..Self::default()
        }
    }

    pub fn with_models(mut self, models: Vec<String>) -> Self {
        self.models = models;
        self
    }

    pub fn with_rule(mut self, rule: MockOpenAiRule) -> Self {
        self.rules.push(rule);
        self
    }

    pub fn with_default_response(mut self, response: MockOpenAiResponse) -> Self {
        self.default_response = Some(response);
        self
    }

    pub async fn start(self) -> MockOpenAiServer {
        let state = Arc::new(MockOpenAiState {
            models: self.models,
            rules: self.rules,
            default_response: self
                .default_response
                .unwrap_or_else(|| MockOpenAiResponse::Text("OK".to_string())),
            requests: Mutex::new(Vec::new()),
            response_counter: AtomicU64::new(1),
        });

        let app = Router::new()
            .route("/v1/models", get(models_handler))
            .route("/v1/chat/completions", post(chat_completions_handler))
            .with_state(Arc::clone(&state));

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("failed to bind mock openai server");
        let addr = listener.local_addr().expect("failed to read bound addr");

        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        let handle = tokio::spawn(async move {
            let _ = axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = shutdown_rx.await;
                })
                .await;
        });

        MockOpenAiServer {
            addr,
            state,
            shutdown_tx: Some(shutdown_tx),
            server_task: Some(handle),
        }
    }
}

pub struct MockOpenAiServer {
    addr: SocketAddr,
    state: Arc<MockOpenAiState>,
    shutdown_tx: Option<oneshot::Sender<()>>,
    server_task: Option<tokio::task::JoinHandle<()>>,
}

impl MockOpenAiServer {
    pub fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    pub fn openai_base_url(&self) -> String {
        format!("{}/v1", self.base_url())
    }

    pub async fn requests(&self) -> Vec<Value> {
        self.state.requests.lock().await.clone()
    }

    pub async fn shutdown(mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        if let Some(handle) = self.server_task.take() {
            let _ = handle.await;
        }
    }
}

impl Drop for MockOpenAiServer {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        if let Some(handle) = self.server_task.take() {
            handle.abort();
        }
    }
}

struct MockOpenAiState {
    models: Vec<String>,
    rules: Vec<MockOpenAiRule>,
    default_response: MockOpenAiResponse,
    requests: Mutex<Vec<Value>>,
    response_counter: AtomicU64,
}

async fn models_handler(State(state): State<Arc<MockOpenAiState>>) -> Json<Value> {
    Json(json!({
        "object": "list",
        "data": state
            .models
            .iter()
            .map(|id| json!({"id": id, "object": "model"}))
            .collect::<Vec<_>>()
    }))
}

async fn chat_completions_handler(
    State(state): State<Arc<MockOpenAiState>>,
    Json(body): Json<Value>,
) -> Result<Json<Value>, (StatusCode, String)> {
    state.requests.lock().await.push(body.clone());

    let model = body
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or("mock-model");
    let last_role = body
        .pointer("/messages")
        .and_then(|m| m.as_array())
        .and_then(|arr| arr.last())
        .and_then(|v| v.get("role"))
        .and_then(|r| r.as_str())
        .unwrap_or_default();

    fn extract_text_content(msg: &Value) -> Option<String> {
        let content = msg.get("content")?;
        if let Some(s) = content.as_str() {
            return Some(s.to_string());
        }
        if let Some(parts) = content.as_array() {
            let mut out = String::new();
            for part in parts {
                if part.get("type").and_then(|v| v.as_str()) == Some("text")
                    && let Some(text) = part.get("text").and_then(|v| v.as_str())
                {
                    if !out.is_empty() {
                        out.push(' ');
                    }
                    out.push_str(text);
                }
            }
            if !out.is_empty() {
                return Some(out);
            }
        }
        None
    }

    let latest_user = body
        .pointer("/messages")
        .and_then(|m| m.as_array())
        .and_then(|arr| {
            arr.iter().rev().find_map(|msg| {
                if msg.get("role").and_then(|r| r.as_str()) == Some("user") {
                    extract_text_content(msg)
                } else {
                    None
                }
            })
        })
        .unwrap_or_default();

    let selected = if last_role == "user" {
        let latest_user_lower = latest_user.to_ascii_lowercase();
        state
            .rules
            .iter()
            .find(|r| latest_user_lower.contains(&r.contains.to_ascii_lowercase()))
            .map(|r| r.response.clone())
            .unwrap_or_else(|| state.default_response.clone())
    } else {
        state.default_response.clone()
    };

    let n = state.response_counter.fetch_add(1, Ordering::Relaxed);
    let response = match selected {
        MockOpenAiResponse::Text(content) => json!({
            "id": format!("chatcmpl-mock-{n}"),
            "object": "chat.completion",
            "created": 0,
            "model": model,
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": content},
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}
        }),
        MockOpenAiResponse::ToolCalls(tool_calls) => {
            let calls = tool_calls
                .iter()
                .map(|tc| {
                    json!({
                        "id": tc.id,
                        "type": "function",
                        "function": {
                            "name": tc.name,
                            "arguments": tc.arguments.to_string()
                        }
                    })
                })
                .collect::<Vec<_>>();
            json!({
                "id": format!("chatcmpl-mock-{n}"),
                "object": "chat.completion",
                "created": 0,
                "model": model,
                "choices": [{
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": serde_json::Value::Null,
                        "tool_calls": calls
                    },
                    "finish_reason": "tool_calls"
                }],
                "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}
            })
        }
        MockOpenAiResponse::Raw(v) => v,
    };

    Ok(Json(response))
}
