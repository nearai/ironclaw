#![allow(dead_code)]

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use tokio::sync::oneshot;

#[derive(Clone, Debug)]
pub struct MockToolResponse {
    pub name: String,
    pub content: serde_json::Value,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordedMcpRequest {
    pub method: String,
    pub authorization: Option<String>,
    pub session_id: Option<String>,
    pub params: Option<serde_json::Value>,
}

pub struct MockMcpServer {
    pub base_url: String,
    state: Arc<MockState>,
    shutdown_tx: Option<oneshot::Sender<()>>,
    handle: Option<tokio::task::JoinHandle<()>>,
}

impl MockMcpServer {
    pub fn mcp_url(&self) -> String {
        format!("{}/mcp", self.base_url)
    }

    pub fn recorded_requests(&self) -> Vec<RecordedMcpRequest> {
        self.state.recorded_requests.lock().unwrap().clone()
    }

    pub fn force_http_status(&self, status: u16) {
        let status = StatusCode::from_u16(status).expect("valid forced MCP HTTP status");
        *self.state.force_status.lock().unwrap() = Some(status);
    }

    pub fn set_tool_call_error(&self, code: i64, message: impl Into<String>) {
        *self.state.force_tool_call_error.lock().unwrap() = Some((code, message.into()));
    }

    pub fn enable_sse_framing(&self) {
        *self.state.sse_framing.lock().unwrap() = true;
    }
}

impl Drop for MockMcpServer {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        if let Some(handle) = self.handle.take() {
            handle.abort();
        }
    }
}

struct MockState {
    base_url: String,
    tools: Vec<String>,
    tool_responses: HashMap<String, Vec<serde_json::Value>>,
    tool_response_idx: Mutex<HashMap<String, usize>>,
    recorded_requests: Mutex<Vec<RecordedMcpRequest>>,
    session_counter: Mutex<u64>,
    force_status: Mutex<Option<StatusCode>>,
    force_tool_call_error: Mutex<Option<(i64, String)>>,
    sse_framing: Mutex<bool>,
}

pub async fn start_mock_mcp_server(tool_responses: Vec<MockToolResponse>) -> MockMcpServer {
    let mut tools = Vec::new();
    let mut response_map: HashMap<String, Vec<serde_json::Value>> = HashMap::new();

    for response in tool_responses {
        if !tools.contains(&response.name) {
            tools.push(response.name.clone());
        }
        response_map
            .entry(response.name)
            .or_default()
            .push(response.content);
    }

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("mock MCP server should bind to loopback");
    let addr: SocketAddr = listener.local_addr().expect("mock MCP listener local addr");
    let base_url = format!("http://127.0.0.1:{}", addr.port());

    let state = Arc::new(MockState {
        base_url: base_url.clone(),
        tools,
        tool_responses: response_map,
        tool_response_idx: Mutex::new(HashMap::new()),
        recorded_requests: Mutex::new(Vec::new()),
        session_counter: Mutex::new(0),
        force_status: Mutex::new(None),
        force_tool_call_error: Mutex::new(None),
        sse_framing: Mutex::new(false),
    });

    let app = Router::new()
        .route(
            "/.well-known/oauth-protected-resource/mcp",
            get(handle_protected_resource),
        )
        .route(
            "/.well-known/oauth-authorization-server",
            get(handle_auth_server_metadata),
        )
        .route("/register", post(handle_register))
        .route("/authorize", get(handle_authorize))
        .route("/token", post(handle_token))
        .route("/mcp", post(handle_mcp))
        .with_state(Arc::clone(&state));

    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
            })
            .await
            .expect("mock MCP server should serve");
    });

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    MockMcpServer {
        base_url,
        state,
        shutdown_tx: Some(shutdown_tx),
        handle: Some(handle),
    }
}

async fn handle_protected_resource(State(state): State<Arc<MockState>>) -> impl IntoResponse {
    Json(serde_json::json!({
        "resource": format!("{}/mcp", state.base_url),
        "authorization_servers": [state.base_url],
        "scopes_supported": ["read", "write"]
    }))
}

async fn handle_auth_server_metadata(State(state): State<Arc<MockState>>) -> impl IntoResponse {
    Json(serde_json::json!({
        "issuer": state.base_url,
        "authorization_endpoint": format!("{}/authorize", state.base_url),
        "token_endpoint": format!("{}/token", state.base_url),
        "registration_endpoint": format!("{}/register", state.base_url),
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code"],
        "code_challenge_methods_supported": ["S256"],
        "scopes_supported": ["read", "write"]
    }))
}

async fn handle_register() -> impl IntoResponse {
    Json(serde_json::json!({
        "client_id": "mock-client-id",
        "client_name": "ironclaw-test",
        "redirect_uris": [],
        "grant_types": ["authorization_code"],
        "response_types": ["code"],
        "token_endpoint_auth_method": "none"
    }))
}

async fn handle_authorize() -> impl IntoResponse {
    axum::response::Html("<html><body>Mock MCP OAuth authorize endpoint</body></html>")
}

async fn handle_token() -> impl IntoResponse {
    Json(serde_json::json!({
        "access_token": "mock-access-token",
        "token_type": "Bearer",
        "expires_in": 3600,
        "refresh_token": "mock-refresh-token"
    }))
}

#[derive(Deserialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    id: Option<serde_json::Value>,
    method: String,
    #[serde(default)]
    params: Option<serde_json::Value>,
}

async fn handle_mcp(
    State(state): State<Arc<MockState>>,
    headers: HeaderMap,
    Json(req): Json<JsonRpcRequest>,
) -> Response {
    let authorization = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    let session_id = headers
        .get("mcp-session-id")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);

    state
        .recorded_requests
        .lock()
        .unwrap()
        .push(RecordedMcpRequest {
            method: req.method.clone(),
            authorization: authorization.clone(),
            session_id,
            params: req.params.clone(),
        });

    if authorization
        .as_deref()
        .and_then(|value| value.split_once(' '))
        .map(|(scheme, token)| scheme.eq_ignore_ascii_case("Bearer") && !token.trim().is_empty())
        != Some(true)
    {
        let www_auth = format!(
            "Bearer resource_metadata=\"{}/.well-known/oauth-protected-resource/mcp\"",
            state.base_url
        );
        return (
            StatusCode::UNAUTHORIZED,
            [("www-authenticate", www_auth)],
            Json(serde_json::json!({
                "jsonrpc": "2.0",
                "id": req.id,
                "error": {"code": -32000, "message": "Unauthorized"}
            })),
        )
            .into_response();
    }

    if req.id.is_none() {
        return StatusCode::ACCEPTED.into_response();
    }

    let mut response_session_id = None;
    let response = match req.method.as_str() {
        "initialize" => {
            let session_id = {
                let mut counter = state.session_counter.lock().unwrap();
                *counter += 1;
                format!("mock-session-{}", *counter)
            };
            response_session_id = Some(session_id);
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": req.id,
                "result": {
                    "protocolVersion": "2024-11-05",
                    "serverInfo": {"name": "mock-mcp-server", "version": "1.0.0"},
                    "capabilities": {"tools": {}}
                }
            })
        }
        "tools/list" => {
            let tools: Vec<serde_json::Value> = state
                .tools
                .iter()
                .map(|name| {
                    serde_json::json!({
                        "name": name,
                        "description": format!("Mock tool: {name}"),
                        "inputSchema": {"type": "object", "properties": {}}
                    })
                })
                .collect();
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": req.id,
                "result": {"tools": tools}
            })
        }
        "tools/call" => tools_call_response(&state, &req),
        _ => serde_json::json!({
            "jsonrpc": "2.0",
            "id": req.id,
            "error": {"code": -32601, "message": format!("Method not found: {}", req.method)}
        }),
    };

    let status = if req.method == "tools/call" {
        state.force_status.lock().unwrap().unwrap_or(StatusCode::OK)
    } else {
        StatusCode::OK
    };

    if *state.sse_framing.lock().unwrap() {
        let json = serde_json::to_string(&response).expect("mock MCP JSON-RPC response serializes");
        let body = format!("event: ping\ndata:\n\nevent: message\ndata: {json}\n\n");
        if let Some(session_id) = response_session_id {
            return (
                status,
                [
                    ("content-type", "text/event-stream".to_owned()),
                    ("mcp-session-id", session_id),
                ],
                body,
            )
                .into_response();
        }
        return (status, [("content-type", "text/event-stream")], body).into_response();
    }

    if let Some(session_id) = response_session_id {
        (status, [("mcp-session-id", session_id)], Json(response)).into_response()
    } else {
        (status, Json(response)).into_response()
    }
}

fn tools_call_response(state: &MockState, req: &JsonRpcRequest) -> serde_json::Value {
    let tool_name = req
        .params
        .as_ref()
        .and_then(|params| params.get("name"))
        .and_then(|name| name.as_str())
        .unwrap_or("unknown");

    let content = {
        let mut idx_map = state.tool_response_idx.lock().unwrap();
        let idx = idx_map.entry(tool_name.to_owned()).or_insert(0);
        let result = state
            .tool_responses
            .get(tool_name)
            .and_then(|responses| responses.get(*idx))
            .cloned()
            .unwrap_or_else(|| serde_json::json!({"error": "no mock response configured"}));
        *idx += 1;
        result
    };

    let success_result = serde_json::json!({
        "content": [{
            "type": "text",
            "text": serde_json::to_string(&content)
                .expect("mock MCP tool content serializes")
        }]
    });

    if let Some((code, message)) = state.force_tool_call_error.lock().unwrap().clone() {
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": req.id,
            "error": {"code": code, "message": message},
            "result": success_result
        })
    } else {
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": req.id,
            "result": success_result
        })
    }
}
