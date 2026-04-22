//! Mock MCP server for E2E testing of the extension lifecycle.
//!
//! Provides a minimal HTTP server with:
//! - OAuth 2.1 discovery (`.well-known/oauth-protected-resource`, `.well-known/oauth-authorization-server`)
//! - Dynamic Client Registration (`/register`)
//! - Token exchange (`/token`)
//! - MCP JSON-RPC endpoint (`/mcp`) with `initialize`, `tools/list`, `tools/call`
//!
//! Tool call responses are pre-configured via `MockToolResponse`.

#![allow(dead_code)]

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;

/// A pre-configured response for a specific MCP tool call.
#[derive(Clone, Debug)]
pub struct MockToolResponse {
    /// Tool name (e.g., "notion-search").
    pub name: String,
    /// JSON response content for `tools/call`.
    pub content: serde_json::Value,
}

/// Full tool definition override — lets a test specify the exact
/// wire-shape of the tool advertised via `tools/list`. Needed for
/// tests that care about fields beyond name (e.g. annotations, which
/// drive the approval policy on `McpToolWrapper` and therefore
/// participate in the tool-surface conflict fingerprint).
#[derive(Clone, Debug)]
pub struct MockToolSpec {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
    pub annotations: Option<serde_json::Value>,
    /// JSON response content for `tools/call`.
    pub content: serde_json::Value,
}

/// A running mock MCP server.
pub struct MockMcpServer {
    /// Base URL including port (e.g., "http://127.0.0.1:12345").
    pub base_url: String,
    state: Arc<MockState>,
    /// Shutdown signal sender.
    shutdown_tx: Option<oneshot::Sender<()>>,
    /// Server task handle.
    handle: Option<tokio::task::JoinHandle<()>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordedMcpRequest {
    pub method: String,
    pub authorization: Option<String>,
    /// The inbound `Mcp-Session-Id` header, if the client echoed one back.
    pub session_id: Option<String>,
}

impl MockMcpServer {
    /// The MCP endpoint URL for use in registry entries.
    pub fn mcp_url(&self) -> String {
        format!("{}/mcp", self.base_url)
    }

    pub fn recorded_requests(&self) -> Vec<RecordedMcpRequest> {
        self.state.recorded_requests.lock().unwrap().clone()
    }

    pub fn clear_recorded_requests(&self) {
        self.state.recorded_requests.lock().unwrap().clear();
    }

    /// Shut down the server.
    pub async fn shutdown(mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        if let Some(h) = self.handle.take() {
            let _ = h.await;
        }
    }
}

impl Drop for MockMcpServer {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        if let Some(h) = self.handle.take() {
            h.abort();
        }
    }
}

/// Shared state for the mock server handlers.
struct MockState {
    /// Base URL (filled after bind).
    base_url: String,
    /// Tool definitions served by tools/list.
    tools: Vec<McpToolDef>,
    /// Pre-configured tool call responses keyed by tool name.
    /// Multiple calls to the same tool return responses in order.
    tool_responses: HashMap<String, Vec<serde_json::Value>>,
    /// Counter for tool_responses consumption (per tool name).
    tool_response_idx: std::sync::Mutex<HashMap<String, usize>>,
    /// Recorded MCP requests for auth/assertion tests.
    recorded_requests: std::sync::Mutex<Vec<RecordedMcpRequest>>,
    /// Monotonic counter for initialize responses; stamps a distinct
    /// `Mcp-Session-Id` per handshake so multi-user isolation tests can
    /// observe that each activation binds its own session.
    session_counter: std::sync::Mutex<u64>,
}

#[derive(Clone, Serialize)]
struct McpToolDef {
    name: String,
    description: String,
    #[serde(rename = "inputSchema")]
    input_schema: serde_json::Value,
    /// Optional — omitted from the JSON entirely when `None` so the
    /// wire matches a spec-minimum MCP server that emits no
    /// `annotations` field. Present when a test wants to exercise
    /// approval-hint behavior.
    #[serde(skip_serializing_if = "Option::is_none")]
    annotations: Option<serde_json::Value>,
}

/// Start a mock MCP server on a random port.
///
/// `tool_responses` configures what `tools/call` returns for each tool name.
/// Multiple responses for the same tool are returned in order.
pub async fn start_mock_mcp_server(tool_responses: Vec<MockToolResponse>) -> MockMcpServer {
    // Build tool definitions and response map.
    let mut tools = Vec::new();
    let mut response_map: HashMap<String, Vec<serde_json::Value>> = HashMap::new();
    let mut seen_tools = std::collections::HashSet::new();

    for tr in &tool_responses {
        if seen_tools.insert(tr.name.clone()) {
            tools.push(McpToolDef {
                name: tr.name.clone(),
                description: format!("Mock tool: {}", tr.name),
                input_schema: serde_json::json!({"type": "object", "properties": {}}),
                annotations: None,
            });
        }
        response_map
            .entry(tr.name.clone())
            .or_default()
            .push(tr.content.clone());
    }

    // Bind to a random port.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("failed to bind mock MCP server");
    let addr: SocketAddr = listener.local_addr().expect("no local addr");
    let base_url = format!("http://127.0.0.1:{}", addr.port());

    let state = Arc::new(MockState {
        base_url: base_url.clone(),
        tools,
        tool_responses: response_map,
        tool_response_idx: std::sync::Mutex::new(HashMap::new()),
        recorded_requests: std::sync::Mutex::new(Vec::new()),
        session_counter: std::sync::Mutex::new(0),
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
            .expect("mock MCP server failed");
    });

    // Wait briefly for the server to start accepting.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    MockMcpServer {
        base_url,
        state,
        shutdown_tx: Some(shutdown_tx),
        handle: Some(handle),
    }
}

/// Same as `start_mock_mcp_server` but every dimension of the
/// `tools/list` response is caller-controlled — description,
/// input schema, and annotations. Use this when a test needs to
/// exercise behavior that depends on specific fields the default
/// builder hard-codes (e.g. the tool-surface conflict check, which
/// hashes annotations to detect approval-policy divergence across
/// users of the same server name).
pub async fn start_mock_mcp_server_with_specs(specs: Vec<MockToolSpec>) -> MockMcpServer {
    let mut tools = Vec::new();
    let mut response_map: HashMap<String, Vec<serde_json::Value>> = HashMap::new();
    let mut seen_tools = std::collections::HashSet::new();

    for spec in &specs {
        if seen_tools.insert(spec.name.clone()) {
            tools.push(McpToolDef {
                name: spec.name.clone(),
                description: spec.description.clone(),
                input_schema: spec.input_schema.clone(),
                annotations: spec.annotations.clone(),
            });
        }
        response_map
            .entry(spec.name.clone())
            .or_default()
            .push(spec.content.clone());
    }

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("failed to bind mock MCP server");
    let addr: SocketAddr = listener.local_addr().expect("no local addr");
    let base_url = format!("http://127.0.0.1:{}", addr.port());

    let state = Arc::new(MockState {
        base_url: base_url.clone(),
        tools,
        tool_responses: response_map,
        tool_response_idx: std::sync::Mutex::new(HashMap::new()),
        recorded_requests: std::sync::Mutex::new(Vec::new()),
        session_counter: std::sync::Mutex::new(0),
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
            .expect("mock MCP server failed");
    });

    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    MockMcpServer {
        base_url,
        state,
        shutdown_tx: Some(shutdown_tx),
        handle: Some(handle),
    }
}

// ── OAuth discovery endpoints ───────────────────────────────────────────

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

// ── OAuth DCR ───────────────────────────────────────────────────────────

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

// ── OAuth authorize (auto-approve) ──────────────────────────────────────

/// In a real flow, this would show a consent screen. For testing, we just
/// need the endpoint to exist. The test will bypass OAuth by injecting
/// tokens directly.
async fn handle_authorize() -> impl IntoResponse {
    // Return a simple HTML page; in practice the test injects tokens directly.
    axum::response::Html(
        "<html><body>Mock OAuth: authorize endpoint. Tests bypass this.</body></html>",
    )
}

// ── OAuth token exchange ────────────────────────────────────────────────

async fn handle_token() -> impl IntoResponse {
    Json(serde_json::json!({
        "access_token": "mock-access-token",
        "token_type": "Bearer",
        "expires_in": 3600,
        "refresh_token": "mock-refresh-token"
    }))
}

// ── MCP JSON-RPC endpoint ───────────────────────────────────────────────

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
) -> impl IntoResponse {
    // Check for auth header.
    let auth = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let inbound_session_id = headers
        .get("mcp-session-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    state
        .recorded_requests
        .lock()
        .unwrap()
        .push(RecordedMcpRequest {
            method: req.method.clone(),
            authorization: if auth.is_empty() {
                None
            } else {
                Some(auth.to_string())
            },
            session_id: inbound_session_id,
        });

    if !auth.starts_with("Bearer ")
        || auth
            .split_once(' ')
            .map(|(_, v)| v.trim())
            .unwrap_or("")
            .is_empty()
    {
        // Return 401 with WWW-Authenticate header per MCP OAuth spec.
        let www_auth = format!(
            "Bearer resource_metadata=\"{}/.well-known/oauth-protected-resource/mcp\"",
            state.base_url
        );
        return (
            StatusCode::UNAUTHORIZED,
            [("www-authenticate", www_auth.as_str())],
            Json(serde_json::json!({
                "jsonrpc": "2.0",
                "id": req.id,
                "error": {"code": -32000, "message": "Unauthorized"}
            })),
        )
            .into_response();
    }

    // Handle notifications (no id) silently.
    if req.id.is_none() {
        return StatusCode::OK.into_response();
    }

    let mut response_session_id: Option<String> = None;
    let response = match req.method.as_str() {
        "initialize" => {
            // Mint a fresh session per handshake — that's how real MCP
            // servers behave, and it's what lets the isolation test assert
            // that user-A and user-B never share a session ID.
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
                    "serverInfo": {
                        "name": "mock-mcp-server",
                        "version": "1.0.0"
                    },
                    "capabilities": {
                        "tools": {}
                    }
                }
            })
        }
        "tools/list" => {
            let tools: Vec<serde_json::Value> = state
                .tools
                .iter()
                .map(|t| serde_json::to_value(t).unwrap())
                .collect();
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": req.id,
                "result": {
                    "tools": tools
                }
            })
        }
        "tools/call" => {
            let tool_name = req
                .params
                .as_ref()
                .and_then(|p| p.get("name"))
                .and_then(|n| n.as_str())
                .unwrap_or("unknown");

            let content = {
                let mut idx_map = state.tool_response_idx.lock().unwrap();
                let idx = idx_map.entry(tool_name.to_string()).or_insert(0);
                let responses = state.tool_responses.get(tool_name);
                let result = responses
                    .and_then(|r| r.get(*idx))
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({"error": "no mock response configured"}));
                *idx += 1;
                result
            };

            serde_json::json!({
                "jsonrpc": "2.0",
                "id": req.id,
                "result": {
                    "content": [
                        {
                            "type": "text",
                            "text": serde_json::to_string(&content).unwrap_or_default()
                        }
                    ]
                }
            })
        }
        _ => serde_json::json!({
            "jsonrpc": "2.0",
            "id": req.id,
            "error": {"code": -32601, "message": format!("Method not found: {}", req.method)}
        }),
    };

    if let Some(session_id) = response_session_id {
        (
            StatusCode::OK,
            [("mcp-session-id", session_id.as_str())],
            Json(response),
        )
            .into_response()
    } else {
        Json(response).into_response()
    }
}
