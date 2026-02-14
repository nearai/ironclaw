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
use crate::tools::tool::{Tool, ToolError, ToolOutput};

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
            return Err(ToolError::ExternalService(format!(
                "MCP server returned status: {} - {}",
                status, body
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

    fn requires_approval(&self) -> bool {
        // Check the destructive_hint annotation from the MCP server
        self.tool.requires_approval()
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
}
