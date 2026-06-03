use async_trait::async_trait;
use ironclaw_host_api::{
    NetworkMethod, RuntimeHttpEgress, RuntimeHttpEgressError, RuntimeHttpEgressRequest,
    RuntimeHttpEgressResponse,
};

#[derive(Default)]
pub(super) struct HostedMcpDiscoveryEgress {
    methods: std::sync::Mutex<Vec<String>>,
    credential_counts: std::sync::Mutex<Vec<usize>>,
}

impl HostedMcpDiscoveryEgress {
    pub(super) fn methods(&self) -> Vec<String> {
        self.methods
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    pub(super) fn credential_counts(&self) -> Vec<usize> {
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
                    "serverInfo": {"name": "notion-test", "version": "1.0.0"}
                }),
                vec![("Mcp-Session-Id".to_string(), "session-1".to_string())],
            ),
            "notifications/initialized" => {
                runtime_json_response(None, serde_json::json!({}), Vec::new())
            }
            "tools/list" => runtime_json_response(
                body["id"].as_u64(),
                serde_json::json!({
                    "tools": [
                        {
                            "name": "live-search",
                            "description": "Search live Notion content",
                            "inputSchema": {
                                "type": "object",
                                "properties": {"query": {"type": "string"}},
                                "required": ["query"]
                            }
                        }
                    ]
                }),
                Vec::new(),
            ),
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
