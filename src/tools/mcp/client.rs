//! MCP client for connecting to MCP servers.
//!
//! Supports both local (unauthenticated) and hosted (OAuth-authenticated) servers.
//! Uses the Streamable HTTP transport with session management.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::RwLock;

use crate::context::JobContext;
use crate::secrets::SecretsStore;
use crate::tools::mcp::auth::refresh_access_token;
use crate::tools::mcp::config::McpServerConfig;
use crate::tools::mcp::protocol::{
    CallToolResult, InitializeResult, ListToolsResult, McpRequest, McpResponse, McpTool,
};
use crate::tools::mcp::session::McpSessionManager;
use crate::tools::tool::{ApprovalRequirement, Tool, ToolError, ToolOutput};

/// MCP client for communicating with MCP servers.
///
/// Supports two modes:
/// - Simple: Just a URL, no auth or session management (for local/test servers)
/// - Authenticated: Full OAuth support with session management (for hosted servers)
pub struct McpClient {
    /// Server URL (for HTTP transport).
    server_url: String,

    /// Server name (for logging and session management).
    server_name: String,

    /// HTTP client.
    http_client: reqwest::Client,

    /// Request ID counter.
    next_id: AtomicU64,

    /// Cached tools.
    tools_cache: RwLock<Option<Vec<McpTool>>>,

    /// Session manager (shared across clients).
    session_manager: Option<Arc<McpSessionManager>>,

    /// Secrets store for retrieving access tokens.
    secrets: Option<Arc<dyn SecretsStore + Send + Sync>>,

    /// User ID for secrets lookup.
    user_id: String,

    /// Server configuration (for token secret name lookup).
    server_config: Option<McpServerConfig>,
}

impl McpClient {
    /// Create a new simple MCP client (no authentication).
    ///
    /// Use this for local development servers or servers that don't require auth.
    pub fn new(server_url: impl Into<String>) -> Self {
        let url = server_url.into();
        let name = extract_server_name(&url);

        Self {
            server_url: url,
            server_name: name,
            http_client: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .expect("Failed to create HTTP client"),
            next_id: AtomicU64::new(1),
            tools_cache: RwLock::new(None),
            session_manager: None,
            secrets: None,
            user_id: "default".to_string(),
            server_config: None,
        }
    }

    /// Create a new simple MCP client with a specific name.
    ///
    /// Use this when you have a configured server name but no authentication.
    pub fn new_with_name(server_name: impl Into<String>, server_url: impl Into<String>) -> Self {
        Self {
            server_url: server_url.into(),
            server_name: server_name.into(),
            http_client: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .expect("Failed to create HTTP client"),
            next_id: AtomicU64::new(1),
            tools_cache: RwLock::new(None),
            session_manager: None,
            secrets: None,
            user_id: "default".to_string(),
            server_config: None,
        }
    }

    /// Create a new authenticated MCP client.
    ///
    /// Use this for hosted MCP servers that require OAuth authentication.
    pub fn new_authenticated(
        config: McpServerConfig,
        session_manager: Arc<McpSessionManager>,
        secrets: Arc<dyn SecretsStore + Send + Sync>,
        user_id: impl Into<String>,
    ) -> Self {
        Self {
            server_url: config.url.clone(),
            server_name: config.name.clone(),
            http_client: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .expect("Failed to create HTTP client"),
            next_id: AtomicU64::new(1),
            tools_cache: RwLock::new(None),
            session_manager: Some(session_manager),
            secrets: Some(secrets),
            user_id: user_id.into(),
            server_config: Some(config),
        }
    }

    /// Get the server name.
    pub fn server_name(&self) -> &str {
        &self.server_name
    }

    /// Get the server URL.
    pub fn server_url(&self) -> &str {
        &self.server_url
    }

    /// Get the next request ID.
    fn next_request_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::SeqCst)
    }

    /// Get the access token for this server (if authenticated).
    ///
    /// Returns the stored token regardless of whether OAuth was pre-configured
    /// or obtained via Dynamic Client Registration.
    async fn get_access_token(&self) -> Result<Option<String>, ToolError> {
        let Some(ref secrets) = self.secrets else {
            return Ok(None);
        };

        let Some(ref config) = self.server_config else {
            return Ok(None);
        };

        // Try to get stored token (from either pre-configured OAuth or DCR)
        match secrets
            .get_decrypted(&self.user_id, &config.token_secret_name())
            .await
        {
            Ok(token) => Ok(Some(token.expose().to_string())),
            Err(crate::secrets::SecretError::NotFound(_)) => Ok(None),
            Err(e) => Err(ToolError::ExternalService(format!(
                "Failed to get access token: {}",
                e
            ))),
        }
    }

    /// Send a request to the MCP server with auth and session headers.
    /// Automatically attempts token refresh on 401 errors.
    async fn send_request(&self, request: McpRequest) -> Result<McpResponse, ToolError> {
        // Try up to 2 times: first attempt, then retry after token refresh
        for attempt in 0..2 {
            // Request both JSON and SSE as per MCP spec
            let mut req_builder = self
                .http_client
                .post(&self.server_url)
                .header("Accept", "application/json, text/event-stream")
                .header("Content-Type", "application/json")
                .json(&request);

            // Add Authorization header if we have a token
            if let Some(token) = self.get_access_token().await? {
                req_builder = req_builder.header("Authorization", format!("Bearer {}", token));
            }

            // Add Mcp-Session-Id header if we have a session
            if let Some(ref session_manager) = self.session_manager
                && let Some(session_id) = session_manager.get_session_id(&self.server_name).await
            {
                req_builder = req_builder.header("Mcp-Session-Id", session_id);
            }

            let response = req_builder.send().await.map_err(|e| {
                let mut chain = format!("MCP request failed: {}", e);
                let mut source = std::error::Error::source(&e);
                while let Some(cause) = source {
                    chain.push_str(&format!(" -> {}", cause));
                    source = cause.source();
                }
                ToolError::ExternalService(chain)
            })?;

            // Check for 401 Unauthorized - try to refresh token on first attempt
            if response.status() == reqwest::StatusCode::UNAUTHORIZED {
                if attempt == 0 {
                    // Try to refresh the token
                    if let Some(ref secrets) = self.secrets
                        && let Some(ref config) = self.server_config
                    {
                        tracing::debug!(
                            "MCP token expired, attempting refresh for '{}'",
                            self.server_name
                        );
                        match refresh_access_token(config, secrets, &self.user_id).await {
                            Ok(_) => {
                                tracing::info!("MCP token refreshed for '{}'", self.server_name);
                                // Continue to next iteration to retry with new token
                                continue;
                            }
                            Err(e) => {
                                tracing::debug!(
                                    "Token refresh failed for '{}': {}",
                                    self.server_name,
                                    e
                                );
                                // Fall through to return auth error
                            }
                        }
                    }
                }
                return Err(ToolError::ExternalService(format!(
                    "MCP server '{}' requires authentication. Run: ironclaw mcp auth {}",
                    self.server_name, self.server_name
                )));
            }

            // Success path - return the parsed response
            return self.parse_response(response).await;
        }

        // Should not reach here, but just in case
        Err(ToolError::ExternalService(
            "MCP request failed after retry".to_string(),
        ))
    }

    /// Parse the HTTP response into an MCP response.
    async fn parse_response(&self, response: reqwest::Response) -> Result<McpResponse, ToolError> {
        // Extract session ID from response header
        if let Some(ref session_manager) = self.session_manager
            && let Some(session_id) = response
                .headers()
                .get("Mcp-Session-Id")
                .and_then(|v| v.to_str().ok())
        {
            session_manager
                .update_session_id(&self.server_name, Some(session_id.to_string()))
                .await;
        }

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            let preview = sanitize_error_body(&body);
            return Err(ToolError::ExternalService(format!(
                "MCP server returned status: {status} - {preview}",
            )));
        }

        // Check content type to handle SSE vs JSON responses
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();

        if content_type.contains("text/event-stream") {
            // SSE response - read chunks until we get a complete JSON message
            use futures::StreamExt;

            let mut stream = response.bytes_stream();
            let mut buffer = String::new();

            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|e| {
                    ToolError::ExternalService(format!("Failed to read SSE chunk: {}", e))
                })?;

                buffer.push_str(&String::from_utf8_lossy(&chunk));

                // Look for complete SSE data lines
                for line in buffer.lines() {
                    if let Some(json_str) = line.strip_prefix("data: ") {
                        // Try to parse - if valid JSON, we're done
                        if let Ok(response) = serde_json::from_str::<McpResponse>(json_str) {
                            return Ok(response);
                        }
                    }
                }
            }

            Err(ToolError::ExternalService(format!(
                "No valid data in SSE response: {}",
                buffer
            )))
        } else {
            // JSON response
            response.json().await.map_err(|e| {
                ToolError::ExternalService(format!("Failed to parse MCP response: {}", e))
            })
        }
    }

    /// Initialize the connection to the MCP server.
    ///
    /// This should be called once per session to establish capabilities.
    pub async fn initialize(&self) -> Result<InitializeResult, ToolError> {
        // Check if already initialized
        if let Some(ref session_manager) = self.session_manager
            && session_manager.is_initialized(&self.server_name).await
        {
            // Return cached/default capabilities
            return Ok(InitializeResult::default());
        }

        // Ensure we have a session
        if let Some(ref session_manager) = self.session_manager {
            session_manager
                .get_or_create(&self.server_name, &self.server_url)
                .await;
        }

        let request = McpRequest::initialize(self.next_request_id());
        let response = self.send_request(request).await?;

        if let Some(error) = response.error {
            return Err(ToolError::ExternalService(format!(
                "MCP initialization error: {} (code {})",
                error.message, error.code
            )));
        }

        let result: InitializeResult = response
            .result
            .ok_or_else(|| {
                ToolError::ExternalService("No result in initialize response".to_string())
            })
            .and_then(|r| {
                serde_json::from_value(r).map_err(|e| {
                    ToolError::ExternalService(format!("Invalid initialize result: {}", e))
                })
            })?;

        // Mark session as initialized
        if let Some(ref session_manager) = self.session_manager {
            session_manager.mark_initialized(&self.server_name).await;
        }

        // Send initialized notification
        let notification = McpRequest::initialized_notification();
        // Fire and forget - notifications don't have responses
        let _ = self.send_request(notification).await;

        Ok(result)
    }

    /// List available tools from the MCP server.
    pub async fn list_tools(&self) -> Result<Vec<McpTool>, ToolError> {
        // Check cache first
        if let Some(tools) = self.tools_cache.read().await.as_ref() {
            return Ok(tools.clone());
        }

        // Ensure initialized for authenticated sessions
        if self.session_manager.is_some() {
            self.initialize().await?;
        }

        let request = McpRequest::list_tools(self.next_request_id());
        let response = self.send_request(request).await?;

        if let Some(error) = response.error {
            return Err(ToolError::ExternalService(format!(
                "MCP error: {} (code {})",
                error.message, error.code
            )));
        }

        let result: ListToolsResult = response
            .result
            .ok_or_else(|| ToolError::ExternalService("No result in MCP response".to_string()))
            .and_then(|r| {
                serde_json::from_value(r)
                    .map_err(|e| ToolError::ExternalService(format!("Invalid tools list: {}", e)))
            })?;

        // Cache the tools
        *self.tools_cache.write().await = Some(result.tools.clone());

        Ok(result.tools)
    }

    /// Call a tool on the MCP server.
    pub async fn call_tool(
        &self,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<CallToolResult, ToolError> {
        // Ensure initialized for authenticated sessions
        if self.session_manager.is_some() {
            self.initialize().await?;
        }

        let request = McpRequest::call_tool(self.next_request_id(), name, arguments);
        let response = self.send_request(request).await?;

        if let Some(error) = response.error {
            return Err(ToolError::ExecutionFailed(format!(
                "MCP tool error: {} (code {})",
                error.message, error.code
            )));
        }

        response
            .result
            .ok_or_else(|| ToolError::ExternalService("No result in MCP response".to_string()))
            .and_then(|r| {
                serde_json::from_value(r)
                    .map_err(|e| ToolError::ExternalService(format!("Invalid tool result: {}", e)))
            })
    }

    /// Clear the tools cache.
    pub async fn clear_cache(&self) {
        *self.tools_cache.write().await = None;
    }

    /// Create Tool implementations for all MCP tools.
    pub async fn create_tools(&self) -> Result<Vec<Arc<dyn Tool>>, ToolError> {
        let mcp_tools = self.list_tools().await?;
        let client = Arc::new(self.clone());

        Ok(mcp_tools
            .into_iter()
            .map(|t| {
                let prefixed_name = format!("{}_{}", self.server_name, t.name);
                Arc::new(McpToolWrapper {
                    tool: t,
                    prefixed_name,
                    client: client.clone(),
                }) as Arc<dyn Tool>
            })
            .collect())
    }

    /// Test the connection to the MCP server.
    pub async fn test_connection(&self) -> Result<(), ToolError> {
        self.initialize().await?;
        self.list_tools().await?;
        Ok(())
    }
}

impl Clone for McpClient {
    fn clone(&self) -> Self {
        Self {
            server_url: self.server_url.clone(),
            server_name: self.server_name.clone(),
            http_client: self.http_client.clone(),
            next_id: AtomicU64::new(self.next_id.load(Ordering::SeqCst)),
            tools_cache: RwLock::new(None),
            session_manager: self.session_manager.clone(),
            secrets: self.secrets.clone(),
            user_id: self.user_id.clone(),
            server_config: self.server_config.clone(),
        }
    }
}

/// Extract a server name from a URL for logging/display purposes.
fn extract_server_name(url: &str) -> String {
    reqwest::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|h| h.to_string()))
        .unwrap_or_else(|| "unknown".to_string())
        .replace('.', "_")
}

/// Wrapper that implements Tool for an MCP tool.
struct McpToolWrapper {
    tool: McpTool,
    /// Prefixed name (server_name_tool_name) for unique identification.
    prefixed_name: String,
    client: Arc<McpClient>,
}

#[async_trait]
impl Tool for McpToolWrapper {
    fn name(&self) -> &str {
        &self.prefixed_name
    }

    fn description(&self) -> &str {
        &self.tool.description
    }

    fn parameters_schema(&self) -> serde_json::Value {
        self.tool.input_schema.clone()
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        // Use the original tool name (without prefix) for the actual call
        let result = self.client.call_tool(&self.tool.name, params).await?;

        // Convert content blocks to a single result
        let content: String = result
            .content
            .iter()
            .filter_map(|block| block.as_text())
            .collect::<Vec<_>>()
            .join("\n");

        if result.is_error {
            return Err(ToolError::ExecutionFailed(content));
        }

        Ok(ToolOutput::text(content, start.elapsed()))
    }

    fn requires_sanitization(&self) -> bool {
        true // MCP tools are external, always sanitize
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        // Delegate to the MCP protocol type's own requires_approval() bool method
        if self.tool.requires_approval() {
            ApprovalRequirement::UnlessAutoApproved
        } else {
            ApprovalRequirement::Never
        }
    }
}

/// Sanitize an HTTP error response body for safe display.
///
/// Detects full HTML error pages (containing `<html` or `<!DOCTYPE`) and
/// strips all tags, collapsing whitespace.  Non-HTML bodies are left
/// intact.  In both cases the result is truncated to 200 *characters*
/// (char-boundary safe) so that large payloads don't bloat error messages.
///
/// See #263 — raw HTML error pages were propagating through the error
/// chain into the web UI, causing a white screen.
fn sanitize_error_body(body: &str) -> String {
    const MAX_CHARS: usize = 200;

    // Only strip tags when the body looks like a full HTML document.
    // Plain text that happens to contain `<` / `>` (e.g. log lines,
    // comparison expressions) is left untouched.
    let lower = body.to_ascii_lowercase();
    let is_html_document = lower.contains("<html") || lower.contains("<!doctype");

    let text = if is_html_document {
        let stripped = body
            .chars()
            .fold((String::new(), false), |(mut out, in_tag), c| {
                if c == '<' {
                    (out, true)
                } else if c == '>' {
                    (out, false)
                } else if !in_tag {
                    out.push(c);
                    (out, false)
                } else {
                    (out, true)
                }
            })
            .0;
        stripped.split_whitespace().collect::<Vec<_>>().join(" ")
    } else {
        body.to_string()
    };

    // Truncate at a char boundary (safe for multi-byte UTF-8).
    if text.chars().count() > MAX_CHARS {
        let byte_offset = text
            .char_indices()
            .nth(MAX_CHARS)
            .map(|(i, _)| i)
            .unwrap_or(text.len());
        format!("{}... ({} bytes total)", &text[..byte_offset], body.len())
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mcp_request_list_tools() {
        let req = McpRequest::list_tools(1);
        assert_eq!(req.method, "tools/list");
        assert_eq!(req.id, 1);
    }

    #[test]
    fn test_mcp_request_call_tool() {
        let req = McpRequest::call_tool(2, "test", serde_json::json!({"key": "value"}));
        assert_eq!(req.method, "tools/call");
        assert!(req.params.is_some());
    }

    #[test]
    fn test_extract_server_name() {
        assert_eq!(
            extract_server_name("https://mcp.notion.com/v1"),
            "mcp_notion_com"
        );
        assert_eq!(extract_server_name("http://localhost:8080"), "localhost");
        assert_eq!(extract_server_name("invalid"), "unknown");
    }

    #[test]
    fn test_simple_client_creation() {
        let client = McpClient::new("http://localhost:8080");
        assert_eq!(client.server_url(), "http://localhost:8080");
        assert!(client.session_manager.is_none());
        assert!(client.secrets.is_none());
    }

    #[test]
    fn test_extract_server_name_with_port() {
        assert_eq!(
            extract_server_name("http://example.com:3000"),
            "example_com"
        );
    }

    #[test]
    fn test_extract_server_name_with_path() {
        assert_eq!(
            extract_server_name("http://api.server.io/v2/mcp"),
            "api_server_io"
        );
    }

    #[test]
    fn test_extract_server_name_with_query_params() {
        assert_eq!(
            extract_server_name("http://mcp.example.com/endpoint?token=abc&v=1"),
            "mcp_example_com"
        );
    }

    #[test]
    fn test_extract_server_name_https() {
        assert_eq!(
            extract_server_name("https://secure.mcp.dev"),
            "secure_mcp_dev"
        );
    }

    #[test]
    fn test_extract_server_name_ip_address() {
        assert_eq!(
            extract_server_name("http://192.168.1.100:9090/mcp"),
            "192_168_1_100"
        );
    }

    #[test]
    fn test_new_defaults() {
        let client = McpClient::new("http://localhost:9999");
        assert_eq!(client.server_url(), "http://localhost:9999");
        assert_eq!(client.server_name(), "localhost");
        assert!(client.session_manager.is_none());
        assert!(client.secrets.is_none());
        assert_eq!(client.user_id, "default");
    }

    #[test]
    fn test_new_with_name_uses_custom_name() {
        let client = McpClient::new_with_name("my-server", "http://localhost:8080");
        assert_eq!(client.server_name(), "my-server");
        assert_eq!(client.server_url(), "http://localhost:8080");
        assert_eq!(client.user_id, "default");
        assert!(client.session_manager.is_none());
        assert!(client.secrets.is_none());
    }

    #[test]
    fn test_server_name_accessor() {
        let client = McpClient::new("https://tools.example.org/mcp");
        assert_eq!(client.server_name(), "tools_example_org");
    }

    #[test]
    fn test_server_url_accessor() {
        let url = "https://tools.example.org/mcp?v=2";
        let client = McpClient::new(url);
        assert_eq!(client.server_url(), url);
    }

    #[test]
    fn test_clone_preserves_fields() {
        let client = McpClient::new_with_name("cloned-server", "http://localhost:5555");
        // Bump the request ID a few times
        client.next_request_id();
        client.next_request_id();

        let cloned = client.clone();
        assert_eq!(cloned.server_url(), "http://localhost:5555");
        assert_eq!(cloned.server_name(), "cloned-server");
        assert_eq!(cloned.user_id, "default");
        // The atomic counter value is copied
        assert_eq!(cloned.next_id.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn test_clone_resets_tools_cache() {
        let client = McpClient::new("http://localhost:5555");
        // The clone implementation resets tools_cache to None
        let cloned = client.clone();
        let cache = cloned.tools_cache.read().await;
        assert!(cache.is_none());
    }

    #[test]
    fn test_next_request_id_monotonically_increasing() {
        let client = McpClient::new("http://localhost:1234");
        let id1 = client.next_request_id();
        let id2 = client.next_request_id();
        let id3 = client.next_request_id();
        assert_eq!(id1, 1);
        assert_eq!(id2, 2);
        assert_eq!(id3, 3);
    }

    #[test]
    fn test_mcp_tool_requires_approval_destructive() {
        use crate::tools::mcp::protocol::{McpTool, McpToolAnnotations};

        let tool = McpTool {
            name: "delete_all".to_string(),
            description: "Deletes everything".to_string(),
            input_schema: serde_json::json!({"type": "object"}),
            annotations: Some(McpToolAnnotations {
                destructive_hint: true,
                side_effects_hint: false,
                read_only_hint: false,
                execution_time_hint: None,
            }),
        };
        assert!(tool.requires_approval());
    }

    #[test]
    fn test_mcp_tool_no_approval_when_not_destructive() {
        use crate::tools::mcp::protocol::{McpTool, McpToolAnnotations};

        let tool = McpTool {
            name: "read_data".to_string(),
            description: "Reads data".to_string(),
            input_schema: serde_json::json!({"type": "object"}),
            annotations: Some(McpToolAnnotations {
                destructive_hint: false,
                side_effects_hint: true,
                read_only_hint: false,
                execution_time_hint: None,
            }),
        };
        assert!(!tool.requires_approval());
    }

    #[test]
    fn test_mcp_tool_no_approval_when_no_annotations() {
        use crate::tools::mcp::protocol::McpTool;

        let tool = McpTool {
            name: "simple_tool".to_string(),
            description: "A simple tool".to_string(),
            input_schema: serde_json::json!({"type": "object"}),
            annotations: None,
        };
        assert!(!tool.requires_approval());
    }

    // Regression tests for #263: HTML error bodies must not propagate raw
    // markup through the error chain into the web UI.

    #[test]
    fn test_sanitize_error_body_strips_html_tags() {
        let html = r#"<!DOCTYPE html><html><body><h1>422 Error</h1><p>Invalid token</p></body></html>"#;
        let result = sanitize_error_body(html);
        assert!(!result.contains('<'), "HTML tags must be stripped");
        assert!(!result.contains('>'), "HTML tags must be stripped");
        assert!(result.contains("422 Error"));
        assert!(result.contains("Invalid token"));
    }

    #[test]
    fn test_sanitize_error_body_truncates_large_html_page() {
        let html = format!(
            "<html><body><p>{}</p></body></html>",
            "error detail ".repeat(50)
        );
        let result = sanitize_error_body(&html);
        assert!(result.contains("..."));
        assert!(result.contains("bytes total)"));
        assert!(!result.contains('<'));
    }

    #[test]
    fn test_sanitize_error_body_passes_short_plain_text() {
        assert_eq!(sanitize_error_body("Not Found"), "Not Found");
    }

    #[test]
    fn test_sanitize_error_body_truncates_long_plain_text() {
        let long = "x".repeat(300);
        let result = sanitize_error_body(&long);
        assert!(result.contains("..."));
        assert!(result.contains("300 bytes total)"));
    }

    #[test]
    fn test_sanitize_error_body_multibyte_no_panic() {
        // 300 CJK characters = 900 bytes; truncation must land on a
        // char boundary, not in the middle of a multi-byte sequence.
        let cjk = "错误".repeat(150);
        let result = sanitize_error_body(&cjk);
        assert!(result.contains("..."));
        // Must be valid UTF-8 (would have panicked otherwise).
        assert!(result.is_char_boundary(result.len()));
    }

    #[test]
    fn test_sanitize_error_body_strips_uppercase_html() {
        let html = "<HTML><BODY><H1>500 Internal Server Error</H1></BODY></HTML>";
        let result = sanitize_error_body(html);
        assert!(!result.contains('<'), "uppercase HTML tags must be stripped");
        assert!(result.contains("500 Internal Server Error"));
    }

    #[test]
    fn test_sanitize_error_body_preserves_angle_brackets_in_non_html() {
        // Text with < and > that is NOT an HTML document should be
        // left untouched (e.g. log lines, comparison expressions).
        let text = "value < 10 and value > 0";
        assert_eq!(sanitize_error_body(text), text);
    }
}
