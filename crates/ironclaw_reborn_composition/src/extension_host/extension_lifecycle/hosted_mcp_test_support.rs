use async_trait::async_trait;
use ironclaw_host_api::{
    NetworkMethod, RuntimeHttpEgress, RuntimeHttpEgressError, RuntimeHttpEgressRequest,
    RuntimeHttpEgressResponse,
};

/// Scripted hosted-MCP discovery egress: answers the `initialize` →
/// `notifications/initialized` → `tools/list` handshake with a single
/// discoverable tool. v3 hosted-MCP packages publish NO model-visible tools
/// until live discovery runs, so tests that need an active hosted-MCP tool
/// (auth gates, dispatch, capability visibility) script discovery through
/// this seam instead of relying on retired v2 placeholder tools.
pub(crate) struct HostedMcpDiscoveryEgress {
    tool_name: String,
    read_only: bool,
    methods: std::sync::Mutex<Vec<String>>,
    credential_counts: std::sync::Mutex<Vec<usize>>,
}

impl Default for HostedMcpDiscoveryEgress {
    fn default() -> Self {
        Self::with_tool_name("live-search")
    }
}

impl HostedMcpDiscoveryEgress {
    /// Script discovery to return one tool with the given MCP tool name; the
    /// published capability id becomes `{extension_id}.{tool_name}`.
    pub(crate) fn with_tool_name(tool_name: &str) -> Self {
        Self {
            tool_name: tool_name.to_string(),
            read_only: false,
            methods: std::sync::Mutex::new(Vec::new()),
            credential_counts: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// Annotate the scripted tool `readOnlyHint: true` so the discovered
    /// capability does not inherit the provider's package-level
    /// `external_write` effect (unannotated tools stay conservative).
    pub(crate) fn read_only(mut self) -> Self {
        self.read_only = true;
        self
    }

    pub(crate) fn methods(&self) -> Vec<String> {
        self.methods
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn credential_counts(&self) -> Vec<usize> {
        self.credential_counts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

#[async_trait]
impl RuntimeHttpEgress for HostedMcpDiscoveryEgress {
    async fn execute(
        &self,
        request: RuntimeHttpEgressRequest,
    ) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
        if request.method != NetworkMethod::Post {
            return Err(RuntimeHttpEgressError::Request {
                reason: "unexpected_method".to_string(),
                request_bytes: request.body.len() as u64,
                response_bytes: 0,
            });
        }
        let body: serde_json::Value =
            serde_json::from_slice(&request.body).map_err(|_| RuntimeHttpEgressError::Request {
                reason: "invalid_json_rpc_body".to_string(),
                request_bytes: request.body.len() as u64,
                response_bytes: 0,
            })?;
        let method = body
            .get("method")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| RuntimeHttpEgressError::Request {
                reason: "missing_json_rpc_method".to_string(),
                request_bytes: request.body.len() as u64,
                response_bytes: 0,
            })?;
        self.methods
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(method.to_string());
        self.credential_counts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(request.credential_injections.len());
        match method {
            "initialize" => runtime_json_response(
                body["id"].as_u64(),
                serde_json::json!({
                    "protocolVersion": "2025-06-18",
                    "capabilities": {"tools": {}},
                    "serverInfo": {"name": "hosted-mcp-test", "version": "1.0.0"}
                }),
                vec![("Mcp-Session-Id".to_string(), "session-1".to_string())],
            ),
            "notifications/initialized" => {
                runtime_json_response(None, serde_json::json!({}), Vec::new())
            }
            "tools/list" => {
                let mut tool = serde_json::json!({
                    "name": self.tool_name,
                    "description": format!("Scripted hosted MCP tool {}", self.tool_name),
                    "inputSchema": {
                        "type": "object",
                        "properties": {"query": {"type": "string"}},
                        "required": ["query"]
                    }
                });
                if self.read_only {
                    tool["annotations"] = serde_json::json!({"readOnlyHint": true});
                }
                runtime_json_response(
                    body["id"].as_u64(),
                    serde_json::json!({ "tools": [tool] }),
                    Vec::new(),
                )
            }
            _ => Err(RuntimeHttpEgressError::Request {
                reason: "unexpected_method".to_string(),
                request_bytes: request.body.len() as u64,
                response_bytes: 0,
            }),
        }
    }
}

fn runtime_json_response(
    id: Option<u64>,
    result: serde_json::Value,
    extra_headers: Vec<(String, String)>,
) -> Result<RuntimeHttpEgressResponse, RuntimeHttpEgressError> {
    let mut headers = vec![("content-type".to_string(), "application/json".to_string())];
    headers.extend(extra_headers);
    let body = serde_json::to_vec(&serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": result,
    }))
    .map_err(|_| RuntimeHttpEgressError::Request {
        reason: "serialize_json_rpc_response".to_string(),
        request_bytes: 0,
        response_bytes: 0,
    })?;
    Ok(RuntimeHttpEgressResponse {
        status: 200,
        headers,
        response_bytes: body.len() as u64,
        body,
        saved_body: None,
        request_bytes: 0,
        redaction_applied: false,
    })
}
