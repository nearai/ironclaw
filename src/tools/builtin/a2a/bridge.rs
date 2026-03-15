//! A2A bridge tool — connects to a remote agent via the A2A protocol.

use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;
use tokio::sync::mpsc;

use crate::channels::IncomingMessage;
use crate::config::A2aConfig;
use crate::context::JobContext;
use crate::safety::LeakDetector;
use crate::secrets::SecretsStore;
use crate::tools::tool::{ApprovalRequirement, Tool, ToolError, ToolOutput, require_str};

use super::protocol::{
    EventKind, build_jsonrpc_request, classify_event, extract_text_from_result,
    has_message_content, parse_sse_events, result_has_text_parts, truncate_str,
};

/// Maximum SSE buffer size (10 MB) — same cap as MCP HTTP transport.
const MAX_SSE_BUFFER: usize = 10 * 1024 * 1024;

/// Maximum summary length for push notifications.
const MAX_SUMMARY_LEN: usize = 2000;

/// A2A bridge tool that delegates queries to a remote agent.
pub struct A2aBridgeTool {
    client: Client,
    config: A2aConfig,
    secrets_store: Arc<dyn SecretsStore + Send + Sync>,
    inject_tx: mpsc::Sender<IncomingMessage>,
    leak_detector: LeakDetector,
}

impl A2aBridgeTool {
    /// Create a new A2A bridge tool.
    ///
    /// The agent URL is validated for SSRF at construction time. Returns an error
    /// if the URL points to a private/local address.
    pub async fn new(
        config: A2aConfig,
        secrets_store: Arc<dyn SecretsStore + Send + Sync>,
        inject_tx: mpsc::Sender<IncomingMessage>,
    ) -> Result<Self, ToolError> {
        // Validate agent URL at construction time (defense in depth)
        validate_agent_url(&config.agent_url).await?;

        // H5: No-redirect policy to prevent SSRF via redirect
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| {
                ToolError::ExternalService(format!("failed to build HTTP client: {}", e))
            })?;

        Ok(Self {
            client,
            config,
            secrets_store,
            inject_tx,
            leak_detector: LeakDetector::new(),
        })
    }

    /// Build the full A2A endpoint URL.
    fn endpoint_url(&self) -> String {
        let base = self.config.agent_url.trim_end_matches('/');
        format!("{}/a2a/{}", base, self.config.assistant_id)
    }
}

#[async_trait]
impl Tool for A2aBridgeTool {
    fn name(&self) -> &str {
        &self.config.tool_name
    }

    fn description(&self) -> &str {
        &self.config.tool_description
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Natural language query for the remote agent"
                },
                "context": {
                    "type": "object",
                    "description": "Optional structured context passed alongside the query"
                },
                "thread_id": {
                    "type": "string",
                    "description": "Thread ID for multi-turn conversations. Reuse to continue a previous session."
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let query = require_str(&params, "query")?;
        let context = params.get("context");
        let thread_id = params.get("thread_id").and_then(|v| v.as_str());

        // C2: Scan outgoing content for secret leaks
        let query_bytes = query.as_bytes();
        self.leak_detector
            .scan_http_request(&self.endpoint_url(), &[], Some(query_bytes))
            .map_err(|e| {
                ToolError::NotAuthorized(format!("leak detection blocked request: {}", e))
            })?;

        if let Some(ctx_val) = context {
            let ctx_str = serde_json::to_string(ctx_val).unwrap_or_default();
            self.leak_detector
                .scan_http_request(&self.endpoint_url(), &[], Some(ctx_str.as_bytes()))
                .map_err(|e| {
                    ToolError::NotAuthorized(format!("leak detection blocked context: {}", e))
                })?;
        }

        // Try to get API key from secrets store (optional — agent may not require auth)
        let api_key = self
            .secrets_store
            .get_decrypted(&ctx.user_id, &self.config.api_key_secret)
            .await
            .ok();

        // Build JSON-RPC request
        let body = build_jsonrpc_request(query, context, thread_id);
        let url = self.endpoint_url();

        // Send POST — accept both SSE and JSON so the server can pick.
        // LangGraph requires application/json to be present in the Accept
        // header; pure text/event-stream is rejected.
        let mut request = self
            .client
            .post(&url)
            .header("Accept", "text/event-stream, application/json")
            .header("Content-Type", "application/json");

        if let Some(ref key) = api_key {
            request = request.bearer_auth(key.expose());
        }

        // M2: Use configured request_timeout for the initial connection
        let response =
            tokio::time::timeout(self.config.request_timeout, request.json(&body).send())
                .await
                .map_err(|_| ToolError::Timeout(self.config.request_timeout))?
                .map_err(|e| {
                    if e.is_timeout() {
                        ToolError::Timeout(self.config.request_timeout)
                    } else {
                        ToolError::ExternalService(format!("A2A request failed: {}", e))
                    }
                })?;

        let status = response.status();
        if !status.is_success() {
            let error_body = response.text().await.unwrap_or_default();
            return Err(ToolError::ExternalService(format!(
                "A2A agent returned HTTP {}: {}",
                status, error_body
            )));
        }

        // Check content-type: if the server returned JSON instead of SSE,
        // parse the full body directly (LangGraph returns application/json).
        let is_json_response = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .map(|ct| ct.contains("application/json"))
            .unwrap_or(false);

        if is_json_response {
            let body = response.text().await.map_err(|e| {
                ToolError::ExternalService(format!("failed to read JSON response: {}", e))
            })?;
            let parsed: serde_json::Value = serde_json::from_str(&body).map_err(|e| {
                ToolError::ExternalService(format!("invalid JSON from A2A agent: {}", e))
            })?;

            // Check for JSON-RPC error
            if let Some(err) = parsed.get("error") {
                let msg = err
                    .get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("unknown error");
                return Err(ToolError::ExternalService(format!(
                    "A2A agent error: {}",
                    msg
                )));
            }

            let result = parsed
                .get("result")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            let summary = extract_text_from_result(&result, MAX_SUMMARY_LEN);
            let result_json = serde_json::json!({
                "status": "completed",
                "result": summary,
            });
            return Ok(ToolOutput::success(result_json, start.elapsed()));
        }

        // SSE path: read first event to determine sync vs async
        let mut stream = response.bytes_stream();
        let mut buffer = String::new();

        let first_event = tokio::time::timeout(self.config.request_timeout, async {
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|e| {
                    ToolError::ExternalService(format!("failed to read SSE chunk: {}", e))
                })?;
                buffer.push_str(&String::from_utf8_lossy(&chunk));

                if buffer.len() > MAX_SSE_BUFFER {
                    return Err(ToolError::ExternalService(
                        "SSE buffer exceeded 10 MB limit".to_string(),
                    ));
                }

                // Try to parse complete SSE events from buffer
                let events = parse_sse_events(&mut buffer);
                if let Some(event) = events.into_iter().next() {
                    return Ok(event);
                }
            }
            Err(ToolError::ExternalService(
                "SSE stream ended without any events".to_string(),
            ))
        })
        .await
        .map_err(|_| ToolError::Timeout(self.config.request_timeout))??;

        // Handle first event
        match classify_event(&first_event) {
            EventKind::Error(msg) => Err(ToolError::ExternalService(format!(
                "A2A agent error: {}",
                msg
            ))),
            EventKind::Final(result) => {
                let summary = extract_text_from_result(&result, MAX_SUMMARY_LEN);
                let result_json = serde_json::json!({
                    "status": "completed",
                    "result": summary,
                });
                Ok(ToolOutput::success(result_json, start.elapsed()))
            }
            EventKind::InProgress {
                task_id,
                context_id,
            } => {
                // Spawn background consumer with cancellation via inject_tx closure
                let inject_tx = self.inject_tx.clone();
                let task_timeout = self.config.task_timeout;
                let query_owned = query.to_string();
                let task_id_for_spawn = task_id.clone();
                let message_prefix = self.config.message_prefix.clone();

                tokio::spawn(async move {
                    spawn_stream_consumer(
                        stream,
                        buffer,
                        inject_tx,
                        task_timeout,
                        query_owned,
                        task_id_for_spawn,
                        message_prefix,
                    )
                    .await;
                });

                let short_id = &task_id[..8.min(task_id.len())];
                let mut result_json = serde_json::json!({
                    "status": "submitted",
                    "task_id": task_id,
                    "message": format!(
                        "Query submitted (task: {}). Results will be pushed when ready.",
                        short_id
                    ),
                });

                // H3: Include context ID for multi-turn support
                if let Some(cid) = context_id {
                    result_json["context_id"] = serde_json::Value::String(cid);
                }

                Ok(ToolOutput::success(result_json, start.elapsed()))
            }
        }
    }

    fn estimated_duration(&self, _params: &serde_json::Value) -> Option<Duration> {
        Some(Duration::from_secs(10))
    }

    fn requires_sanitization(&self) -> bool {
        true // External data always needs sanitization
    }

    // M3: Always require approval — sends user content to an external service
    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::Always
    }

    fn execution_timeout(&self) -> Duration {
        // Controls the initial request phase (reading first SSE event).
        // The background consumer has its own timeout via task_timeout.
        Duration::from_secs(600)
    }

    fn rate_limit_config(&self) -> Option<crate::tools::tool::ToolRateLimitConfig> {
        Some(crate::tools::tool::ToolRateLimitConfig::new(10, 100))
    }
}

// ── SSRF validation ─────────────────────────────────────────────────

/// Validate an A2A agent URL for SSRF protection.
///
/// Unlike `HttpTool::validate_url()`, this allows both HTTP and HTTPS schemes
/// (the operator chooses the protocol), but still blocks localhost and private IPs
/// to prevent SSRF.
async fn validate_agent_url(url: &str) -> Result<(), ToolError> {
    let parsed = reqwest::Url::parse(url)
        .map_err(|e| ToolError::InvalidParameters(format!("invalid agent URL: {}", e)))?;

    let scheme = parsed.scheme();
    if scheme != "http" && scheme != "https" {
        return Err(ToolError::InvalidParameters(format!(
            "A2A agent URL must use http or https scheme, got '{}'",
            scheme
        )));
    }

    let host = parsed
        .host_str()
        .ok_or_else(|| ToolError::InvalidParameters("agent URL missing host".to_string()))?;

    let host_lower = host.to_lowercase();
    if host_lower == "localhost" || host_lower.ends_with(".localhost") {
        return Err(ToolError::NotAuthorized(
            "A2A agent URL must not point to localhost".to_string(),
        ));
    }

    // Block literal private/local IPs
    if let Ok(ip) = host.parse::<IpAddr>()
        && is_disallowed_ip(&ip)
    {
        return Err(ToolError::NotAuthorized(
            "A2A agent URL must not point to a private or local IP".to_string(),
        ));
    }

    // DNS resolution check — prevent rebinding to private IPs
    let port = parsed.port_or_known_default().unwrap_or(443);
    if let Ok(addrs) = tokio::net::lookup_host((host, port)).await {
        for addr in addrs {
            if is_disallowed_ip(&addr.ip()) {
                return Err(ToolError::NotAuthorized(format!(
                    "A2A agent hostname '{}' resolves to disallowed IP {}",
                    host,
                    addr.ip()
                )));
            }
        }
    }

    Ok(())
}

/// Check if an IP address is private, loopback, link-local, or otherwise
/// disallowed for outbound requests.
fn is_disallowed_ip(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_multicast()
                || v4.is_unspecified()
                || *v4 == std::net::Ipv4Addr::new(169, 254, 169, 254) // AWS metadata
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_unique_local()
                || v6.is_unicast_link_local()
                || v6.is_multicast()
                || v6.is_unspecified()
        }
    }
}

// ── Background SSE consumer ─────────────────────────────────────────

/// Background SSE stream consumer that reads remaining events and pushes
/// the final result back to the agent loop via `inject_tx`.
///
/// Implements H6: checks `inject_tx.is_closed()` each iteration so the
/// task terminates promptly when the session ends.
async fn spawn_stream_consumer(
    stream: impl futures::Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Unpin + Send,
    mut buffer: String,
    inject_tx: mpsc::Sender<IncomingMessage>,
    task_timeout: Duration,
    query: String,
    task_id: String,
    message_prefix: String,
) {
    let short_id = &task_id[..8.min(task_id.len())];

    let result = tokio::time::timeout(task_timeout, async {
        let mut pinned_stream = std::pin::pin!(stream);
        let mut last_content_event: Option<serde_json::Value> = None;

        while let Some(chunk) = pinned_stream.next().await {
            // H6: Stop if the channel is closed (session ended)
            if inject_tx.is_closed() {
                tracing::debug!(task_id = %short_id, "A2A: inject channel closed, stopping consumer");
                return Err("session ended".to_string());
            }

            let chunk = match chunk {
                Ok(c) => c,
                Err(e) => return Err(format!("SSE stream error: {}", e)),
            };

            buffer.push_str(&String::from_utf8_lossy(&chunk));

            if buffer.len() > MAX_SSE_BUFFER {
                return Err("SSE buffer exceeded 10 MB limit".to_string());
            }

            // Process all complete events in the buffer
            for event in parse_sse_events(&mut buffer) {
                match classify_event(&event) {
                    EventKind::Error(msg) => {
                        return Err(format!("A2A agent error: {}", msg));
                    }
                    EventKind::Final(result) => {
                        // The final event often has only status metadata (no text).
                        // Prefer last_content_event if the final result lacks text parts.
                        if !result_has_text_parts(&result)
                            && let Some(prev) = last_content_event
                        {
                            return Ok(prev);
                        }
                        return Ok(result);
                    }
                    EventKind::InProgress { .. } => {
                        // Track events that carry message text
                        if has_message_content(&event.raw) {
                            last_content_event = Some(
                                event
                                    .raw
                                    .get("result")
                                    .cloned()
                                    .unwrap_or(event.raw.clone()),
                            );
                        }
                    }
                }
            }
        }

        // Stream ended — try remaining buffer
        for event in parse_sse_events(&mut buffer) {
            if let EventKind::Final(result) = classify_event(&event) {
                return Ok(result);
            }
        }

        // Return last content event if we have one
        if let Some(last) = last_content_event {
            return Ok(last);
        }

        Err("SSE stream ended without final result".to_string())
    })
    .await;

    let query_preview = truncate_str(&query, 60);
    let msg = match result {
        Ok(Ok(result)) => {
            let summary = extract_text_from_result(&result, MAX_SUMMARY_LEN);
            IncomingMessage::new(
                "a2a_bridge",
                "system",
                format!(
                    "{} Query completed — \"{}\"\n\n{}",
                    message_prefix, query_preview, summary
                ),
            )
        }
        Ok(Err(e)) => IncomingMessage::new(
            "a2a_bridge",
            "system",
            format!(
                "{} Query failed (task: {}) — {}",
                message_prefix, short_id, e
            ),
        ),
        Err(_) => IncomingMessage::new(
            "a2a_bridge",
            "system",
            format!(
                "{} Query timed out (task: {}) — waited {}s",
                message_prefix,
                short_id,
                task_timeout.as_secs()
            ),
        ),
    };

    if inject_tx.send(msg).await.is_err() {
        tracing::debug!(
            task_id = %short_id,
            "A2A inject channel closed, result dropped"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn validate_url_rejects_localhost() {
        assert!(validate_agent_url("http://localhost:5085").await.is_err());
        assert!(
            validate_agent_url("https://app.localhost:5085/a2a")
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn validate_url_rejects_private_ips() {
        assert!(validate_agent_url("http://192.168.1.1:5085").await.is_err());
        assert!(validate_agent_url("http://10.0.0.1:5085").await.is_err());
        assert!(validate_agent_url("http://172.16.0.1:5085").await.is_err());
        assert!(validate_agent_url("http://127.0.0.1:5085").await.is_err());
    }

    #[tokio::test]
    async fn validate_url_rejects_aws_metadata() {
        assert!(
            validate_agent_url("http://169.254.169.254/latest/meta-data")
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn validate_url_rejects_bad_scheme() {
        assert!(validate_agent_url("ftp://example.com/a2a").await.is_err());
        assert!(validate_agent_url("file:///etc/passwd").await.is_err());
    }

    #[tokio::test]
    async fn validate_url_accepts_public_https() {
        assert!(
            validate_agent_url("https://api.example.com:5085")
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn validate_url_accepts_public_http() {
        // HTTP is allowed (operator's choice), unlike HttpTool which requires HTTPS
        assert!(
            validate_agent_url("http://api.example.com:5085")
                .await
                .is_ok()
        );
    }

    #[test]
    fn is_disallowed_ip_checks() {
        assert!(is_disallowed_ip(&"127.0.0.1".parse().unwrap()));
        assert!(is_disallowed_ip(&"10.0.0.1".parse().unwrap()));
        assert!(is_disallowed_ip(&"192.168.0.1".parse().unwrap()));
        assert!(is_disallowed_ip(&"172.16.0.1".parse().unwrap()));
        assert!(is_disallowed_ip(&"169.254.169.254".parse().unwrap()));
        assert!(is_disallowed_ip(&"::1".parse().unwrap()));
        assert!(!is_disallowed_ip(&"8.8.8.8".parse().unwrap()));
    }

    #[tokio::test]
    async fn schema_has_required_query() {
        let config = A2aConfig {
            enabled: true,
            agent_url: "https://example.com".to_string(),
            assistant_id: "test".to_string(),
            tool_name: "a2a_query".to_string(),
            tool_description: "test tool".to_string(),
            message_prefix: "[a2a]".to_string(),
            request_timeout: Duration::from_secs(60),
            task_timeout: Duration::from_secs(1200),
            api_key_secret: "key".to_string(),
        };
        let (tx, _rx) = mpsc::channel(1);
        let store: Arc<dyn SecretsStore + Send + Sync> =
            Arc::new(crate::testing::credentials::test_secrets_store());
        let tool = A2aBridgeTool::new(config, store, tx).await.unwrap();
        let schema = tool.parameters_schema();
        let required = schema["required"].as_array().unwrap();
        assert!(required.contains(&serde_json::json!("query")));
    }

    #[tokio::test]
    async fn tool_name_from_config() {
        let config = A2aConfig {
            enabled: true,
            agent_url: "https://example.com".to_string(),
            assistant_id: "test".to_string(),
            tool_name: "my_custom_tool".to_string(),
            tool_description: "custom description".to_string(),
            message_prefix: "[custom]".to_string(),
            request_timeout: Duration::from_secs(60),
            task_timeout: Duration::from_secs(1200),
            api_key_secret: "".to_string(),
        };
        let (tx, _rx) = mpsc::channel(1);
        let store: Arc<dyn SecretsStore + Send + Sync> =
            Arc::new(crate::testing::credentials::test_secrets_store());
        let tool = A2aBridgeTool::new(config, store, tx).await.unwrap();
        assert_eq!(tool.name(), "my_custom_tool");
        assert_eq!(tool.description(), "custom description");
    }

    #[tokio::test]
    async fn requires_always_approval() {
        let config = A2aConfig {
            enabled: true,
            agent_url: "https://example.com".to_string(),
            assistant_id: "test".to_string(),
            tool_name: "a2a_query".to_string(),
            tool_description: "test".to_string(),
            message_prefix: "[a2a]".to_string(),
            request_timeout: Duration::from_secs(60),
            task_timeout: Duration::from_secs(1200),
            api_key_secret: "".to_string(),
        };
        let (tx, _rx) = mpsc::channel(1);
        let store: Arc<dyn SecretsStore + Send + Sync> =
            Arc::new(crate::testing::credentials::test_secrets_store());
        let tool = A2aBridgeTool::new(config, store, tx).await.unwrap();
        assert_eq!(
            tool.requires_approval(&serde_json::json!({})),
            ApprovalRequirement::Always
        );
    }
}
