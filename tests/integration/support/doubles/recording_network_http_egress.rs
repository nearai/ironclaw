/// Test double substituting the production `NetworkHttpEgress` impl:
/// `PolicyNetworkHttpEgress` (`crates/ironclaw_network/src/egress.rs`) over
/// `ReqwestNetworkTransport` (`crates/ironclaw_network/src/transport.rs`).
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use ironclaw_network::{
    NetworkHttpEgress, NetworkHttpError, NetworkHttpRequest, NetworkHttpResponse, NetworkUsage,
};

#[derive(Debug, Clone)]
enum ResponseScript {
    Static,
    McpDiscovery { tools: Vec<serde_json::Value> },
}

#[derive(Debug, Clone)]
pub(crate) struct RecordingNetworkHttpEgress {
    default_body: Vec<u8>,
    response_script: ResponseScript,
    response_bodies: Arc<Mutex<VecDeque<Vec<u8>>>>,
    /// W4-AUTHGATE-WIRE: FIFO of scripted non-default statuses, consumed ahead
    /// of the hardcoded `200` default. Lets a test drive the runtime-401 path
    /// for capabilities whose real HTTP call flows through this **network**
    /// lane (`GithubIssueTools`, via `try_with_host_http_egress`) rather than
    /// the runtime egress matcher. Empty by default — pre-existing callers
    /// keep the old hardcoded-200 behavior byte-identical.
    status_queue: Arc<Mutex<VecDeque<u16>>>,
    requests: Arc<Mutex<Vec<NetworkHttpRequest>>>,
}

impl RecordingNetworkHttpEgress {
    pub(crate) fn with_body(body: Vec<u8>) -> Self {
        Self {
            default_body: body,
            response_script: ResponseScript::Static,
            response_bodies: Arc::new(Mutex::new(VecDeque::new())),
            status_queue: Arc::new(Mutex::new(VecDeque::new())),
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Script the hosted-MCP JSON-RPC handshake at the real network-egress
    /// boundary. Unlike [`with_body`](Self::with_body), this inspects each
    /// request and returns the response appropriate to `initialize`,
    /// `notifications/initialized`, `tools/list`, or `tools/call`.
    ///
    /// The production host-runtime egress adapter still runs above this double;
    /// only the final network operation is replaced. Existing profiles continue
    /// to use the static response mode unchanged.
    pub(crate) fn with_mcp_discovery_tools(tools: Vec<serde_json::Value>) -> Self {
        Self {
            default_body: Vec::new(),
            response_script: ResponseScript::McpDiscovery { tools },
            response_bodies: Arc::new(Mutex::new(VecDeque::new())),
            status_queue: Arc::new(Mutex::new(VecDeque::new())),
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub(crate) fn requests(&self) -> Vec<NetworkHttpRequest> {
        self.requests.lock().unwrap().clone()
    }

    /// Enqueue one FIFO scripted status, consumed by the next `execute` call
    /// ahead of the hardcoded `200` default.
    pub(crate) fn push_status(&self, status: u16) {
        self.status_queue.lock().unwrap().push_back(status);
    }
}

#[async_trait::async_trait]
impl NetworkHttpEgress for RecordingNetworkHttpEgress {
    async fn execute(
        &self,
        request: NetworkHttpRequest,
    ) -> Result<NetworkHttpResponse, NetworkHttpError> {
        let request_bytes = request.body.len() as u64;
        let scripted_response = match &self.response_script {
            ResponseScript::Static => None,
            ResponseScript::McpDiscovery { tools } => {
                Some(mcp_response_for_request(&request, tools, request_bytes)?)
            }
        };
        self.requests.lock().unwrap().push(request);
        let (status, headers, body) = match scripted_response {
            Some(response) => response,
            None => {
                let body = self
                    .response_bodies
                    .lock()
                    .unwrap()
                    .pop_front()
                    .unwrap_or_else(|| self.default_body.clone());
                let status = self.status_queue.lock().unwrap().pop_front().unwrap_or(200);
                (
                    status,
                    vec![("content-type".to_string(), "application/json".to_string())],
                    body,
                )
            }
        };
        Ok(NetworkHttpResponse {
            status,
            headers,
            body: body.clone(),
            usage: NetworkUsage {
                request_bytes,
                response_bytes: body.len() as u64,
                resolved_ip: None,
            },
        })
    }
}

type ScriptedNetworkResponse = (u16, Vec<(String, String)>, Vec<u8>);

fn mcp_response_for_request(
    request: &NetworkHttpRequest,
    tools: &[serde_json::Value],
    request_bytes: u64,
) -> Result<ScriptedNetworkResponse, NetworkHttpError> {
    if request.method != ironclaw_host_api::NetworkMethod::Post {
        return Err(script_error("unexpected MCP HTTP method", request_bytes));
    }
    let body: serde_json::Value = serde_json::from_slice(&request.body)
        .map_err(|_| script_error("invalid MCP JSON-RPC request", request_bytes))?;
    let method = body
        .get("method")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| script_error("missing MCP JSON-RPC method", request_bytes))?;

    match method {
        "initialize" => json_rpc_response(
            body.get("id").cloned(),
            serde_json::json!({
                "protocolVersion": "2025-06-18",
                "capabilities": {"tools": {"listChanged": false}},
                "serverInfo": {"name": "scripted-registered-mcp", "version": "1.0.0"}
            }),
            vec![(
                "Mcp-Session-Id".to_string(),
                "registered-session-1".to_string(),
            )],
            request_bytes,
        ),
        "notifications/initialized" => Ok((202, Vec::new(), Vec::new())),
        "tools/list" => json_rpc_response(
            body.get("id").cloned(),
            serde_json::json!({"tools": tools}),
            Vec::new(),
            request_bytes,
        ),
        "tools/call" => json_rpc_response(
            body.get("id").cloned(),
            serde_json::json!({
                "content": [{"type": "text", "text": "scripted registered MCP result"}],
                "isError": false
            }),
            Vec::new(),
            request_bytes,
        ),
        _ => Err(script_error(
            "unexpected MCP JSON-RPC method",
            request_bytes,
        )),
    }
}

fn json_rpc_response(
    id: Option<serde_json::Value>,
    result: serde_json::Value,
    mut extra_headers: Vec<(String, String)>,
    request_bytes: u64,
) -> Result<ScriptedNetworkResponse, NetworkHttpError> {
    let body = serde_json::to_vec(&serde_json::json!({
        "jsonrpc": "2.0",
        "id": id.unwrap_or(serde_json::Value::Null),
        "result": result,
    }))
    .map_err(|_| script_error("serialize MCP JSON-RPC response", request_bytes))?;
    let mut headers = vec![("content-type".to_string(), "application/json".to_string())];
    headers.append(&mut extra_headers);
    Ok((200, headers, body))
}

fn script_error(reason: &'static str, request_bytes: u64) -> NetworkHttpError {
    NetworkHttpError::Transport {
        reason: reason.to_string(),
        request_bytes,
        response_bytes: 0,
    }
}
