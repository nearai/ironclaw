//! MCP protocol types.

use serde::{Deserialize, Deserializer, Serialize};

/// Flexibly deserialize a JSON-RPC id that may be a number, string, or null.
fn deserialize_flexible_id<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
where
    D: Deserializer<'de>,
{
    let value: Option<serde_json::Value> = Option::deserialize(deserializer)?;
    match value {
        Some(serde_json::Value::Number(n)) => Ok(n.as_u64()),
        Some(serde_json::Value::String(s)) => Ok(s.parse::<u64>().ok()),
        _ => Ok(None),
    }
}

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
    /// MCP protocol uses camelCase `inputSchema`.
    #[serde(
        default = "default_input_schema",
        rename = "inputSchema",
        alias = "input_schema"
    )]
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
///
/// The MCP specification uses camelCase for annotation field names
/// (e.g. `destructiveHint`, `readOnlyHint`). The `rename_all` attribute
/// ensures correct deserialization from spec-compliant servers.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
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
    /// Request ID (None for notifications per JSON-RPC spec).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<u64>,
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
            id: Some(id),
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
    /// Per JSON-RPC spec, notifications MUST NOT have an id field.
    pub fn initialized_notification() -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id: None,
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
    /// Request ID (may be missing for notifications or non-standard for errors).
    #[serde(deserialize_with = "deserialize_flexible_id")]
    pub id: Option<u64>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mcp_tool_deserialize_camel_case_input_schema() {
        // MCP protocol uses camelCase "inputSchema"
        let json = serde_json::json!({
            "name": "list_issues",
            "description": "List GitHub issues",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "owner": { "type": "string" },
                    "repo": { "type": "string" }
                },
                "required": ["owner", "repo"]
            }
        });

        let tool: McpTool = serde_json::from_value(json).expect("deserialize McpTool");
        assert_eq!(tool.name, "list_issues");
        assert_eq!(tool.description, "List GitHub issues");

        // The schema must have the properties, not the empty default
        let props = tool.input_schema.get("properties").expect("has properties");
        assert!(props.get("owner").is_some());
        assert!(props.get("repo").is_some());
    }

    #[test]
    fn test_mcp_tool_deserialize_snake_case_alias() {
        // Also accept snake_case "input_schema" for flexibility
        let json = serde_json::json!({
            "name": "search",
            "description": "Search",
            "input_schema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string" }
                }
            }
        });

        let tool: McpTool = serde_json::from_value(json).expect("deserialize McpTool");
        let props = tool.input_schema.get("properties").expect("has properties");
        assert!(props.get("query").is_some());
    }

    #[test]
    fn test_mcp_tool_missing_schema_gets_default() {
        let json = serde_json::json!({
            "name": "ping",
            "description": "Ping"
        });

        let tool: McpTool = serde_json::from_value(json).expect("deserialize McpTool");
        assert_eq!(tool.input_schema["type"], "object");
        assert!(tool.input_schema["properties"].is_object());
    }

    #[test]
    fn test_initialize_request() {
        let req = McpRequest::initialize(42);
        assert_eq!(req.jsonrpc, "2.0");
        assert_eq!(req.id, Some(42));
        assert_eq!(req.method, "initialize");

        let params = req.params.expect("initialize must have params");
        assert_eq!(params["protocolVersion"], PROTOCOL_VERSION);
        assert!(params["capabilities"].is_object());
        assert!(params["capabilities"]["roots"].is_object());
        assert!(params["capabilities"]["sampling"].is_object());
        assert_eq!(params["clientInfo"]["name"], "ironclaw");
        assert!(params["clientInfo"]["version"].is_string());
    }

    #[test]
    fn test_initialized_notification() {
        let req = McpRequest::initialized_notification();
        assert_eq!(req.jsonrpc, "2.0");
        assert_eq!(req.method, "notifications/initialized");
        assert!(req.params.is_none());
    }

    #[test]
    fn test_call_tool_request() {
        let args = serde_json::json!({"query": "rust async"});
        let req = McpRequest::call_tool(7, "search", args.clone());
        assert_eq!(req.id, Some(7));
        assert_eq!(req.method, "tools/call");

        let params = req.params.expect("call_tool must have params");
        assert_eq!(params["name"], "search");
        assert_eq!(params["arguments"], args);
    }

    #[test]
    fn test_mcp_response_deserialize_success() {
        let json = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": { "tools": [] }
        });
        let resp: McpResponse = serde_json::from_value(json).expect("deserialize");
        assert_eq!(resp.id, Some(1));
        assert!(resp.result.is_some());
        assert!(resp.error.is_none());
    }

    #[test]
    fn test_mcp_response_deserialize_error() {
        let json = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "error": {
                "code": -32601,
                "message": "Method not found"
            }
        });
        let resp: McpResponse = serde_json::from_value(json).expect("deserialize");
        assert!(resp.result.is_none());
        let err = resp.error.expect("should have error");
        assert_eq!(err.code, -32601);
        assert_eq!(err.message, "Method not found");
        assert!(err.data.is_none());
    }

    #[test]
    fn test_mcp_error_roundtrip() {
        let err = McpError {
            code: -32600,
            message: "Invalid Request".to_string(),
            data: Some(serde_json::json!({"detail": "missing field"})),
        };
        let serialized = serde_json::to_string(&err).expect("serialize");
        let deserialized: McpError = serde_json::from_str(&serialized).expect("deserialize");
        assert_eq!(deserialized.code, err.code);
        assert_eq!(deserialized.message, err.message);
        assert_eq!(deserialized.data, err.data);
    }

    #[test]
    fn test_initialize_result_full() {
        let json = serde_json::json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {
                "tools": { "listChanged": true },
                "resources": { "subscribe": true, "listChanged": false },
                "prompts": { "listChanged": true },
                "logging": {}
            },
            "serverInfo": {
                "name": "test-server",
                "version": "1.2.3"
            },
            "instructions": "Use this server for testing."
        });
        let result: InitializeResult = serde_json::from_value(json).expect("deserialize");
        assert_eq!(result.protocol_version.as_deref(), Some("2024-11-05"));

        let tools_cap = result.capabilities.tools.expect("has tools capability");
        assert!(tools_cap.list_changed);

        let res_cap = result
            .capabilities
            .resources
            .expect("has resources capability");
        assert!(res_cap.subscribe);
        assert!(!res_cap.list_changed);

        let prompts_cap = result.capabilities.prompts.expect("has prompts capability");
        assert!(prompts_cap.list_changed);

        assert!(result.capabilities.logging.is_some());

        let info = result.server_info.expect("has server info");
        assert_eq!(info.name, "test-server");
        assert_eq!(info.version.as_deref(), Some("1.2.3"));
        assert_eq!(
            result.instructions.as_deref(),
            Some("Use this server for testing.")
        );
    }

    #[test]
    fn test_content_block_as_text() {
        let text_block = ContentBlock::Text {
            text: "hello".to_string(),
        };
        assert_eq!(text_block.as_text(), Some("hello"));

        let image_block = ContentBlock::Image {
            data: "base64data".to_string(),
            mime_type: "image/png".to_string(),
        };
        assert!(image_block.as_text().is_none());

        let resource_block = ContentBlock::Resource {
            uri: "file:///tmp/a.txt".to_string(),
            mime_type: Some("text/plain".to_string()),
            text: Some("content".to_string()),
        };
        assert!(resource_block.as_text().is_none());
    }

    #[test]
    fn test_content_block_serde_tagged_union() {
        let text_block = ContentBlock::Text {
            text: "hi".to_string(),
        };
        let json = serde_json::to_value(&text_block).expect("serialize");
        assert_eq!(json["type"], "text");
        assert_eq!(json["text"], "hi");

        let image_block = ContentBlock::Image {
            data: "abc".to_string(),
            mime_type: "image/jpeg".to_string(),
        };
        let json = serde_json::to_value(&image_block).expect("serialize");
        assert_eq!(json["type"], "image");
        assert_eq!(json["data"], "abc");
        assert_eq!(json["mime_type"], "image/jpeg");

        let resource_block = ContentBlock::Resource {
            uri: "file:///x".to_string(),
            mime_type: None,
            text: None,
        };
        let json = serde_json::to_value(&resource_block).expect("serialize");
        assert_eq!(json["type"], "resource");
        assert_eq!(json["uri"], "file:///x");
    }

    #[test]
    fn test_call_tool_result_is_error() {
        let success: CallToolResult = serde_json::from_value(serde_json::json!({
            "content": [{"type": "text", "text": "done"}],
            "is_error": false
        }))
        .expect("deserialize");
        assert!(!success.is_error);
        assert_eq!(success.content.len(), 1);

        let failure: CallToolResult = serde_json::from_value(serde_json::json!({
            "content": [{"type": "text", "text": "boom"}],
            "is_error": true
        }))
        .expect("deserialize");
        assert!(failure.is_error);
    }

    #[test]
    fn test_call_tool_result_is_error_defaults_false() {
        let result: CallToolResult = serde_json::from_value(serde_json::json!({
            "content": []
        }))
        .expect("deserialize");
        assert!(!result.is_error);
    }

    #[test]
    fn test_requires_approval_with_destructive_hint() {
        let tool = McpTool {
            name: "delete_all".to_string(),
            description: "Deletes everything".to_string(),
            input_schema: default_input_schema(),
            annotations: Some(McpToolAnnotations {
                destructive_hint: true,
                ..Default::default()
            }),
        };
        assert!(tool.requires_approval());
    }

    #[test]
    fn test_requires_approval_without_destructive_hint() {
        let tool = McpTool {
            name: "read_file".to_string(),
            description: "Reads a file".to_string(),
            input_schema: default_input_schema(),
            annotations: Some(McpToolAnnotations {
                destructive_hint: false,
                read_only_hint: true,
                ..Default::default()
            }),
        };
        assert!(!tool.requires_approval());
    }

    #[test]
    fn test_requires_approval_no_annotations() {
        let tool = McpTool {
            name: "ping".to_string(),
            description: "Ping".to_string(),
            input_schema: default_input_schema(),
            annotations: None,
        };
        assert!(!tool.requires_approval());
    }

    #[test]
    fn test_annotations_deserialize_camel_case_from_mcp_spec() {
        // MCP spec uses camelCase for annotation fields (destructiveHint, readOnlyHint, etc.).
        // Without #[serde(rename_all = "camelCase")] on McpToolAnnotations, serde silently
        // defaults all fields to false and destructive tools bypass approval.
        let json = serde_json::json!({
            "name": "pods_delete",
            "description": "Delete a pod",
            "inputSchema": {"type": "object"},
            "annotations": {
                "destructiveHint": true,
                "readOnlyHint": false,
                "sideEffectsHint": true
            }
        });
        let tool: McpTool = serde_json::from_value(json).expect("deserialize");
        assert!(
            tool.requires_approval(),
            "tools with destructiveHint: true must require approval"
        );
        let ann = tool.annotations.unwrap();
        assert!(ann.destructive_hint);
        assert!(!ann.read_only_hint);
        assert!(ann.side_effects_hint);
    }

    #[test]
    fn test_mcp_tool_annotations_defaults() {
        let annotations = McpToolAnnotations::default();
        assert!(!annotations.destructive_hint);
        assert!(!annotations.side_effects_hint);
        assert!(!annotations.read_only_hint);
        assert!(annotations.execution_time_hint.is_none());
    }

    #[test]
    fn test_execution_time_hint_serde() {
        // Fast
        let json = serde_json::json!("fast");
        let hint: ExecutionTimeHint = serde_json::from_value(json).expect("deserialize fast");
        assert_eq!(hint, ExecutionTimeHint::Fast);
        let serialized = serde_json::to_value(hint).expect("serialize fast");
        assert_eq!(serialized, "fast");

        // Medium
        let json = serde_json::json!("medium");
        let hint: ExecutionTimeHint = serde_json::from_value(json).expect("deserialize medium");
        assert_eq!(hint, ExecutionTimeHint::Medium);
        let serialized = serde_json::to_value(hint).expect("serialize medium");
        assert_eq!(serialized, "medium");

        // Slow
        let json = serde_json::json!("slow");
        let hint: ExecutionTimeHint = serde_json::from_value(json).expect("deserialize slow");
        assert_eq!(hint, ExecutionTimeHint::Slow);
        let serialized = serde_json::to_value(hint).expect("serialize slow");
        assert_eq!(serialized, "slow");
    }

    #[test]
    fn test_notification_serializes_without_id_field() {
        // JSON-RPC 2.0 spec: notifications MUST NOT have an "id" field.
        let notif = McpRequest::initialized_notification();
        let json = serde_json::to_value(&notif).expect("serialize notification");
        assert!(
            json.get("id").is_none(),
            "notifications must not contain an 'id' field per JSON-RPC 2.0 spec"
        );
        assert_eq!(json.get("method").unwrap(), "notifications/initialized");
    }

    #[test]
    fn test_response_with_string_id() {
        // Some MCP servers return id as a string instead of a number.
        let json = serde_json::json!({
            "jsonrpc": "2.0",
            "id": "42",
            "result": {}
        });
        let resp: McpResponse = serde_json::from_value(json).expect("deserialize string id");
        assert_eq!(resp.id, Some(42));
    }

    #[test]
    fn test_response_with_null_id() {
        // JSON-RPC error responses may have a null id.
        let json = serde_json::json!({
            "jsonrpc": "2.0",
            "id": null,
            "error": { "code": -32700, "message": "Parse error" }
        });
        let resp: McpResponse = serde_json::from_value(json).expect("deserialize null id");
        assert_eq!(resp.id, None);
    }

    #[test]
    fn test_response_with_non_numeric_string_id() {
        // Some servers send non-numeric string ids — these should parse as None.
        let json = serde_json::json!({
            "jsonrpc": "2.0",
            "id": "not-a-number",
            "result": {}
        });
        let resp: McpResponse =
            serde_json::from_value(json).expect("deserialize non-numeric string id");
        assert_eq!(resp.id, None);
    }

    #[test]
    fn test_mcp_tool_roundtrip_preserves_schema() {
        // Simulate what list_tools returns from a real MCP server
        let server_response = serde_json::json!({
            "tools": [{
                "name": "github-copilot_list_issues",
                "description": "List issues for a repository",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "owner": { "type": "string", "description": "Repository owner" },
                        "repo": { "type": "string", "description": "Repository name" },
                        "state": { "type": "string", "enum": ["open", "closed", "all"] }
                    },
                    "required": ["owner", "repo"]
                }
            }]
        });

        let result: ListToolsResult =
            serde_json::from_value(server_response).expect("deserialize ListToolsResult");
        assert_eq!(result.tools.len(), 1);

        let tool = &result.tools[0];
        assert_eq!(tool.name, "github-copilot_list_issues");

        let required = tool.input_schema.get("required").expect("has required");
        assert!(required.as_array().expect("is array").len() == 2);
    }
}
