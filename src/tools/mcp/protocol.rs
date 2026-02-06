//! MCP protocol types.

use serde::{Deserialize, Serialize};

/// MCP protocol version.
pub const PROTOCOL_VERSION: &str = "2024-11-05";

/// An MCP tool definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpTool {
    /// Tool name.
    pub name: String,
    /// Tool description.
    #[serde(default)]
    pub description: String,
    /// JSON Schema for input parameters.
    /// Defaults to empty object schema if not provided.
    #[serde(default = "default_input_schema")]
    pub input_schema: serde_json::Value,
    /// Optional annotations from the MCP server.
    #[serde(default)]
    pub annotations: Option<McpToolAnnotations>,
}

/// Default input schema (empty object).
fn default_input_schema() -> serde_json::Value {
    serde_json::json!({"type": "object", "properties": {}})
}

/// Annotations for an MCP tool that provide hints about its behavior.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct McpToolAnnotations {
    /// Hint that this tool performs destructive operations that cannot be undone.
    /// Tools with this hint set to true should require user approval before execution.
    #[serde(default)]
    pub destructive_hint: bool,

    /// Hint that this tool may have side effects beyond its return value.
    #[serde(default)]
    pub side_effects_hint: bool,

    /// Hint that this tool performs read-only operations.
    #[serde(default)]
    pub read_only_hint: bool,

    /// Hint about the expected execution time category.
    #[serde(default)]
    pub execution_time_hint: Option<ExecutionTimeHint>,
}

/// Hint about how long a tool typically takes to execute.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionTimeHint {
    /// Typically completes in under 1 second.
    Fast,
    /// Typically completes in 1-10 seconds.
    Medium,
    /// Typically completes in more than 10 seconds.
    Slow,
}

impl McpTool {
    /// Check if this tool requires user approval based on its annotations.
    pub fn requires_approval(&self) -> bool {
        self.annotations
            .as_ref()
            .map(|a| a.destructive_hint)
            .unwrap_or(false)
    }
}

/// Request to an MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpRequest {
    /// JSON-RPC version.
    pub jsonrpc: String,
    /// Request ID.
    pub id: u64,
    /// Method name.
    pub method: String,
    /// Request parameters.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
}

impl McpRequest {
    /// Create a new MCP request.
    pub fn new(id: u64, method: impl Into<String>, params: Option<serde_json::Value>) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            method: method.into(),
            params,
        }
    }

    /// Create an initialize request.
    pub fn initialize(id: u64) -> Self {
        Self::new(
            id,
            "initialize",
            Some(serde_json::json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {
                    "roots": { "listChanged": false },
                    "sampling": {}
                },
                "clientInfo": {
                    "name": "ironclaw",
                    "version": env!("CARGO_PKG_VERSION")
                }
            })),
        )
    }

    /// Create an initialized notification (sent after initialize).
    pub fn initialized_notification() -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id: 0, // Notifications don't have IDs, but we need one for the struct
            method: "notifications/initialized".to_string(),
            params: None,
        }
    }

    /// Create a tools/list request.
    pub fn list_tools(id: u64) -> Self {
        Self::new(id, "tools/list", None)
    }

    /// Create a tools/call request.
    pub fn call_tool(id: u64, name: &str, arguments: serde_json::Value) -> Self {
        Self::new(
            id,
            "tools/call",
            Some(serde_json::json!({
                "name": name,
                "arguments": arguments
            })),
        )
    }
}

/// Response from an MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpResponse {
    /// JSON-RPC version.
    pub jsonrpc: String,
    /// Request ID.
    pub id: u64,
    /// Result (on success).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    /// Error (on failure).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<McpError>,
}

/// MCP error.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpError {
    /// Error code.
    pub code: i32,
    /// Error message.
    pub message: String,
    /// Additional data.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

/// Result of the initialize handshake.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InitializeResult {
    /// Protocol version supported by the server.
    #[serde(rename = "protocolVersion")]
    pub protocol_version: Option<String>,

    /// Server capabilities.
    #[serde(default)]
    pub capabilities: ServerCapabilities,

    /// Server information.
    #[serde(rename = "serverInfo")]
    pub server_info: Option<ServerInfo>,

    /// Instructions for using this server.
    pub instructions: Option<String>,
}

/// Server capabilities advertised during initialization.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ServerCapabilities {
    /// Tool capabilities.
    #[serde(default)]
    pub tools: Option<ToolsCapability>,

    /// Resource capabilities.
    #[serde(default)]
    pub resources: Option<ResourcesCapability>,

    /// Prompt capabilities.
    #[serde(default)]
    pub prompts: Option<PromptsCapability>,

    /// Logging capabilities.
    #[serde(default)]
    pub logging: Option<serde_json::Value>,
}

/// Tool-related capabilities.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolsCapability {
    /// Whether the tool list can change.
    #[serde(rename = "listChanged", default)]
    pub list_changed: bool,
}

/// Resource-related capabilities.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResourcesCapability {
    /// Whether subscriptions are supported.
    #[serde(default)]
    pub subscribe: bool,

    /// Whether the resource list can change.
    #[serde(rename = "listChanged", default)]
    pub list_changed: bool,
}

/// Prompt-related capabilities.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PromptsCapability {
    /// Whether the prompt list can change.
    #[serde(rename = "listChanged", default)]
    pub list_changed: bool,
}

/// Server information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerInfo {
    /// Server name.
    pub name: String,

    /// Server version.
    pub version: Option<String>,
}

/// Result of listing tools.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListToolsResult {
    pub tools: Vec<McpTool>,
}

/// Result of calling a tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallToolResult {
    pub content: Vec<ContentBlock>,
    #[serde(default)]
    pub is_error: bool,
}

/// Content block in a tool result.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image")]
    Image { data: String, mime_type: String },
    #[serde(rename = "resource")]
    Resource {
        uri: String,
        mime_type: Option<String>,
        text: Option<String>,
    },
}

impl ContentBlock {
    /// Get text content if this is a text block.
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text { text } => Some(text),
            _ => None,
        }
    }
}
