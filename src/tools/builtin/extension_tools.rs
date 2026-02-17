//! Agent-callable tools for managing extensions (MCP servers and WASM tools).
//!
//! These six tools let the LLM search, install, authenticate, activate, list,
//! and remove extensions entirely through conversation.

use std::sync::Arc;

use async_trait::async_trait;

use crate::context::JobContext;
use crate::extensions::{ExtensionKind, ExtensionManager};
use crate::tools::tool::{Tool, ToolError, ToolOutput, require_str};

// ── tool_search ──────────────────────────────────────────────────────────

pub struct ToolSearchTool {
    manager: Arc<ExtensionManager>,
}

impl ToolSearchTool {
    pub fn new(manager: Arc<ExtensionManager>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl Tool for ToolSearchTool {
    fn name(&self) -> &str {
        "tool_search"
    }

    fn description(&self) -> &str {
        "Search for available extensions (MCP servers, WASM tools) to add. \
         Use discover:true to search online if the built-in registry has no results."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search query (name, keyword, or description fragment)"
                },
                "discover": {
                    "type": "boolean",
                    "description": "If true, also search online (slower, 5-15s). Try without first.",
                    "default": false
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let query = params.get("query").and_then(|v| v.as_str()).unwrap_or("");
        let discover = params
            .get("discover")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let results = self
            .manager
            .search(query, discover)
            .await
            .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;

        let output = serde_json::json!({
            "results": results,
            "count": results.len(),
            "searched_online": discover,
        });

        Ok(ToolOutput::success(output, start.elapsed()))
    }
}

// ── tool_install ─────────────────────────────────────────────────────────

pub struct ToolInstallTool {
    manager: Arc<ExtensionManager>,
}

impl ToolInstallTool {
    pub fn new(manager: Arc<ExtensionManager>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl Tool for ToolInstallTool {
    fn name(&self) -> &str {
        "tool_install"
    }

    fn description(&self) -> &str {
        "Install an extension (MCP server or WASM tool). \
         Use the name from tool_search results, or provide an explicit URL."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Extension name (from search results or custom)"
                },
                "url": {
                    "type": "string",
                    "description": "Explicit URL (for extensions not in the registry)"
                },
                "kind": {
                    "type": "string",
                    "enum": ["mcp_server", "wasm_tool"],
                    "description": "Extension type (auto-detected if omitted)"
                }
            },
            "required": ["name"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let name = require_str(&params, "name")?;

        let url = params.get("url").and_then(|v| v.as_str());

        let kind_hint = params
            .get("kind")
            .and_then(|v| v.as_str())
            .and_then(|k| match k {
                "mcp_server" => Some(ExtensionKind::McpServer),
                "wasm_tool" => Some(ExtensionKind::WasmTool),
                _ => None,
            });

        let result = self
            .manager
            .install(name, url, kind_hint)
            .await
            .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;

        let output = serde_json::to_value(&result)
            .unwrap_or_else(|_| serde_json::json!({"error": "serialization failed"}));

        Ok(ToolOutput::success(output, start.elapsed()))
    }

    fn requires_approval(&self) -> bool {
        true
    }
}

// ── tool_auth ────────────────────────────────────────────────────────────

pub struct ToolAuthTool {
    manager: Arc<ExtensionManager>,
}

impl ToolAuthTool {
    pub fn new(manager: Arc<ExtensionManager>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl Tool for ToolAuthTool {
    fn name(&self) -> &str {
        "tool_auth"
    }

    fn description(&self) -> &str {
        "Initiate authentication for an extension. For OAuth, returns a URL. \
         For manual auth, returns instructions. The user provides their token \
         through a secure channel, never through this tool."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Extension name to authenticate"
                }
            },
            "required": ["name"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let name = require_str(&params, "name")?;

        let result = self
            .manager
            .auth(name, None)
            .await
            .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;

        // Auto-activate after successful auth so tools are available immediately
        if result.status == "authenticated" {
            match self.manager.activate(name).await {
                Ok(activate_result) => {
                    let output = serde_json::json!({
                        "status": "authenticated_and_activated",
                        "name": name,
                        "tools_loaded": activate_result.tools_loaded,
                        "message": activate_result.message,
                    });
                    return Ok(ToolOutput::success(output, start.elapsed()));
                }
                Err(e) => {
                    tracing::warn!(
                        "Extension '{}' authenticated but activation failed: {}",
                        name,
                        e
                    );
                    let output = serde_json::json!({
                        "status": "authenticated",
                        "name": name,
                        "activation_error": e.to_string(),
                        "message": format!(
                            "Authenticated but activation failed: {}. Try tool_activate.",
                            e
                        ),
                    });
                    return Ok(ToolOutput::success(output, start.elapsed()));
                }
            }
        }

        let output = serde_json::to_value(&result)
            .unwrap_or_else(|_| serde_json::json!({"error": "serialization failed"}));

        Ok(ToolOutput::success(output, start.elapsed()))
    }

    fn requires_approval(&self) -> bool {
        true
    }
}

// ── tool_activate ────────────────────────────────────────────────────────

pub struct ToolActivateTool {
    manager: Arc<ExtensionManager>,
}

impl ToolActivateTool {
    pub fn new(manager: Arc<ExtensionManager>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl Tool for ToolActivateTool {
    fn name(&self) -> &str {
        "tool_activate"
    }

    fn description(&self) -> &str {
        "Activate an installed extension, connecting to MCP servers or loading WASM tools into the runtime."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Extension name to activate"
                }
            },
            "required": ["name"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let name = require_str(&params, "name")?;

        match self.manager.activate(name).await {
            Ok(result) => {
                let output = serde_json::to_value(&result)
                    .unwrap_or_else(|_| serde_json::json!({"error": "serialization failed"}));
                Ok(ToolOutput::success(output, start.elapsed()))
            }
            Err(activate_err) => {
                let err_str = activate_err.to_string();
                let needs_auth = err_str.contains("authentication")
                    || err_str.contains("401")
                    || err_str.contains("Unauthorized")
                    || err_str.contains("not authenticated");

                if !needs_auth {
                    return Err(ToolError::ExecutionFailed(err_str));
                }

                // Activation failed due to missing auth; initiate auth flow
                // so the agent loop can show the auth card.
                match self.manager.auth(name, None).await {
                    Ok(auth_result) if auth_result.status == "authenticated" => {
                        // Auth succeeded (e.g. env var was set); retry activation.
                        let result = self
                            .manager
                            .activate(name)
                            .await
                            .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;
                        let output = serde_json::to_value(&result).unwrap_or_else(
                            |_| serde_json::json!({"error": "serialization failed"}),
                        );
                        Ok(ToolOutput::success(output, start.elapsed()))
                    }
                    Ok(auth_result) => {
                        // Auth needs user input (awaiting_token). Return the auth
                        // result so detect_auth_awaiting picks it up.
                        let output = serde_json::to_value(&auth_result).unwrap_or_else(
                            |_| serde_json::json!({"error": "serialization failed"}),
                        );
                        Ok(ToolOutput::success(output, start.elapsed()))
                    }
                    Err(auth_err) => Err(ToolError::ExecutionFailed(format!(
                        "Activation failed ({}), and authentication also failed: {}",
                        err_str, auth_err
                    ))),
                }
            }
        }
    }
}

// ── tool_list ────────────────────────────────────────────────────────────

pub struct ToolListTool {
    manager: Arc<ExtensionManager>,
}

impl ToolListTool {
    pub fn new(manager: Arc<ExtensionManager>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl Tool for ToolListTool {
    fn name(&self) -> &str {
        "tool_list"
    }

    fn description(&self) -> &str {
        "List all installed extensions with their authentication and activation status."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "kind": {
                    "type": "string",
                    "enum": ["mcp_server", "wasm_tool", "wasm_channel"],
                    "description": "Filter by extension type (omit to list all)"
                }
            }
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let kind_filter = params
            .get("kind")
            .and_then(|v| v.as_str())
            .and_then(|k| match k {
                "mcp_server" => Some(ExtensionKind::McpServer),
                "wasm_tool" => Some(ExtensionKind::WasmTool),
                "wasm_channel" => Some(ExtensionKind::WasmChannel),
                _ => None,
            });

        let extensions = self
            .manager
            .list(kind_filter)
            .await
            .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;

        let output = serde_json::json!({
            "extensions": extensions,
            "count": extensions.len(),
        });

        Ok(ToolOutput::success(output, start.elapsed()))
    }
}

// ── tool_remove ──────────────────────────────────────────────────────────

pub struct ToolRemoveTool {
    manager: Arc<ExtensionManager>,
}

impl ToolRemoveTool {
    pub fn new(manager: Arc<ExtensionManager>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl Tool for ToolRemoveTool {
    fn name(&self) -> &str {
        "tool_remove"
    }

    fn description(&self) -> &str {
        "Remove an installed extension (MCP server or WASM tool). \
         Unregisters tools and deletes configuration."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Extension name to remove"
                }
            },
            "required": ["name"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let name = require_str(&params, "name")?;

        let message = self
            .manager
            .remove(name)
            .await
            .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;

        let output = serde_json::json!({
            "name": name,
            "message": message,
        });

        Ok(ToolOutput::success(output, start.elapsed()))
    }

    fn requires_approval(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tool_search_schema() {
        let tool = ToolSearchTool {
            manager: test_manager_stub(),
        };
        assert_eq!(tool.name(), "tool_search");
        let schema = tool.parameters_schema();
        assert!(schema.get("properties").is_some());
        assert!(schema["properties"].get("query").is_some());
    }

    #[test]
    fn test_tool_install_schema() {
        let tool = ToolInstallTool {
            manager: test_manager_stub(),
        };
        assert_eq!(tool.name(), "tool_install");
        assert!(tool.requires_approval());
        let schema = tool.parameters_schema();
        assert!(schema["properties"].get("name").is_some());
        assert!(schema["properties"].get("url").is_some());
    }

    #[test]
    fn test_tool_auth_schema() {
        let tool = ToolAuthTool {
            manager: test_manager_stub(),
        };
        assert_eq!(tool.name(), "tool_auth");
        assert!(tool.requires_approval());
        let schema = tool.parameters_schema();
        assert!(schema["properties"].get("name").is_some());
        // token param must NOT be in schema (security: tokens never go through LLM)
        assert!(
            schema["properties"].get("token").is_none(),
            "tool_auth must not have a token parameter"
        );
    }

    #[test]
    fn test_tool_activate_schema() {
        let tool = ToolActivateTool {
            manager: test_manager_stub(),
        };
        assert_eq!(tool.name(), "tool_activate");
        assert!(!tool.requires_approval());
    }

    #[test]
    fn test_tool_list_schema() {
        let tool = ToolListTool {
            manager: test_manager_stub(),
        };
        assert_eq!(tool.name(), "tool_list");
        assert!(!tool.requires_approval());
        let schema = tool.parameters_schema();
        assert!(schema["properties"].get("kind").is_some());
    }

    #[test]
    fn test_tool_remove_schema() {
        let tool = ToolRemoveTool {
            manager: test_manager_stub(),
        };
        assert_eq!(tool.name(), "tool_remove");
        assert!(tool.requires_approval());
    }

    /// Create a stub manager for schema tests (these don't call execute).
    fn test_manager_stub() -> Arc<ExtensionManager> {
        use crate::secrets::{InMemorySecretsStore, SecretsCrypto};
        use crate::tools::ToolRegistry;
        use crate::tools::mcp::session::McpSessionManager;

        let master_key =
            secrecy::SecretString::from("0123456789abcdef0123456789abcdef".to_string());
        let crypto = Arc::new(SecretsCrypto::new(master_key).unwrap());

        Arc::new(ExtensionManager::new(
            Arc::new(McpSessionManager::new()),
            Some(Arc::new(InMemorySecretsStore::new(crypto))),
            Arc::new(ToolRegistry::new()),
            None,
            std::path::PathBuf::from("/tmp/ironclaw-test-tools"),
            std::path::PathBuf::from("/tmp/ironclaw-test-channels"),
            None,
            "test".to_string(),
            None,
        ))
    }
}
