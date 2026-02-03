//! MCP client for connecting to MCP servers.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::RwLock;

use crate::context::JobContext;
use crate::tools::mcp::protocol::{
    CallToolResult, ListToolsResult, McpRequest, McpResponse, McpTool,
};
use crate::tools::tool::{Tool, ToolError, ToolOutput};

/// MCP client for communicating with MCP servers.
pub struct McpClient {
    /// Server URL (for HTTP transport).
    server_url: String,
    /// HTTP client.
    http_client: reqwest::Client,
    /// Request ID counter.
    next_id: AtomicU64,
    /// Cached tools.
    tools_cache: RwLock<Option<Vec<McpTool>>>,
}

impl McpClient {
    /// Create a new MCP client.
    pub fn new(server_url: impl Into<String>) -> Self {
        Self {
            server_url: server_url.into(),
            http_client: reqwest::Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .expect("Failed to create HTTP client"),
            next_id: AtomicU64::new(1),
            tools_cache: RwLock::new(None),
        }
    }

    /// Get the next request ID.
    fn next_request_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::SeqCst)
    }

    /// Send a request to the MCP server.
    async fn send_request(&self, request: McpRequest) -> Result<McpResponse, ToolError> {
        let response = self
            .http_client
            .post(&self.server_url)
            .json(&request)
            .send()
            .await
            .map_err(|e| ToolError::ExternalService(format!("MCP request failed: {}", e)))?;

        if !response.status().is_success() {
            return Err(ToolError::ExternalService(format!(
                "MCP server returned status: {}",
                response.status()
            )));
        }

        response
            .json()
            .await
            .map_err(|e| ToolError::ExternalService(format!("Failed to parse MCP response: {}", e)))
    }

    /// List available tools from the MCP server.
    pub async fn list_tools(&self) -> Result<Vec<McpTool>, ToolError> {
        // Check cache first
        if let Some(tools) = self.tools_cache.read().await.as_ref() {
            return Ok(tools.clone());
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
                Arc::new(McpToolWrapper {
                    tool: t,
                    client: client.clone(),
                }) as Arc<dyn Tool>
            })
            .collect())
    }
}

impl Clone for McpClient {
    fn clone(&self) -> Self {
        Self {
            server_url: self.server_url.clone(),
            http_client: self.http_client.clone(),
            next_id: AtomicU64::new(self.next_id.load(Ordering::SeqCst)),
            tools_cache: RwLock::new(None),
        }
    }
}

/// Wrapper that implements Tool for an MCP tool.
struct McpToolWrapper {
    tool: McpTool,
    client: Arc<McpClient>,
}

#[async_trait]
impl Tool for McpToolWrapper {
    fn name(&self) -> &str {
        &self.tool.name
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
}
