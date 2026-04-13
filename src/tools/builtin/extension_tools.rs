//! Agent-callable tools for managing extensions (MCP servers and WASM tools).
//!
//! These six tools let the LLM search, install, authenticate, activate, list,
//! and remove extensions entirely through conversation.

use std::sync::Arc;

use async_trait::async_trait;

use crate::context::JobContext;
use crate::extensions::{EnsureReadyIntent, EnsureReadyOutcome, ExtensionKind, ExtensionManager};
use crate::tools::permissions::{TOOL_RISK_DEFAULTS, effective_permission};
use crate::tools::registry::ToolRegistry;
use crate::tools::tool::{
    ApprovalRequirement, EngineCompatibility, Tool, ToolError, ToolOutput, require_str,
};

#[cfg(test)]
fn activation_error_requires_auth(err: &str) -> bool {
    let err_lower = err.to_ascii_lowercase();
    err_lower.contains("authentication required")
        || err_lower.contains("authentication")
        || err_lower.contains("unauthorized")
        || err_lower.contains("not authenticated")
        || err.contains("401")
}

fn output_from_ensure_ready(outcome: EnsureReadyOutcome) -> serde_json::Value {
    match outcome {
        EnsureReadyOutcome::Ready {
            name,
            kind,
            activation: Some(activation),
            ..
        } => serde_json::json!({
            "status": "ready",
            "name": name,
            "kind": kind,
            "tools_loaded": activation.tools_loaded,
            "message": activation.message,
        }),
        EnsureReadyOutcome::Ready {
            name,
            kind,
            phase,
            activation: None,
        } => serde_json::json!({
            "status": "ready",
            "name": name,
            "kind": kind,
            "phase": phase,
            "message": format!("Extension '{}' is ready.", name),
        }),
        EnsureReadyOutcome::NeedsAuth { auth, .. } => serde_json::to_value(&auth)
            .unwrap_or_else(|_| serde_json::json!({"error": "serialization failed"})),
        EnsureReadyOutcome::NeedsSetup {
            name,
            kind,
            instructions,
            setup_url,
            ..
        } => serde_json::json!({
            "status": "needs_setup",
            "name": name,
            "kind": kind,
            "instructions": instructions,
            "setup_url": setup_url,
        }),
    }
}

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
        "Search for available extensions to add new capabilities. Extensions include \
         channels (Telegram, Slack, Discord — connect messaging platforms so IronClaw can \
         receive and reply there), tools, and MCP servers. Use `tool_install` and \
         `tool_activate` to install and enable channels; use the `message` tool for proactive \
         outbound sends. Use discover:true to search online if the built-in registry has no \
         results."
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

    fn is_concurrent_safe(&self, _params: &serde_json::Value) -> bool {
        true // Read-only extension search
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
        "Install an extension (channel, tool, or MCP server). \
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
                    "enum": ["mcp_server", "wasm_tool", "wasm_channel"],
                    "description": "Extension type (auto-detected if omitted)"
                }
            },
            "required": ["name"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
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
                "wasm_channel" => Some(ExtensionKind::WasmChannel),
                _ => None,
            });

        let result = self
            .manager
            .install(name, url, kind_hint, &ctx.user_id)
            .await
            .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;

        let output = serde_json::to_value(&result)
            .unwrap_or_else(|_| serde_json::json!({"error": "serialization failed"}));

        Ok(ToolOutput::success(output, start.elapsed()))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
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
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let name = require_str(&params, "name")?;

        let result = self
            .manager
            .ensure_extension_ready(name, &ctx.user_id, EnsureReadyIntent::ExplicitAuth)
            .await
            .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;
        Ok(ToolOutput::success(
            output_from_ensure_ready(result),
            start.elapsed(),
        ))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        // In gateway mode, tool_auth only returns an auth URL for the frontend
        // to open — no browser is launched server-side, so no approval needed.
        if self.manager.should_use_gateway_mode() {
            ApprovalRequirement::Never
        } else {
            ApprovalRequirement::UnlessAutoApproved
        }
    }

    fn engine_compatibility(&self) -> EngineCompatibility {
        EngineCompatibility::V1Only
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
        "Activate an installed extension — starts channels, loads tools, or connects to MCP servers."
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
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let name = require_str(&params, "name")?;

        let result = self
            .manager
            .ensure_extension_ready(name, &ctx.user_id, EnsureReadyIntent::ExplicitActivate)
            .await
            .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;

        Ok(ToolOutput::success(
            output_from_ensure_ready(result),
            start.elapsed(),
        ))
    }
}

// ── tool_list ────────────────────────────────────────────────────────────

pub struct ToolListTool {
    manager: Arc<ExtensionManager>,
    registry: Option<Arc<ToolRegistry>>,
    settings_store: Option<Arc<dyn crate::db::SettingsStore + Send + Sync>>,
}

impl ToolListTool {
    pub fn new(manager: Arc<ExtensionManager>) -> Self {
        Self {
            manager,
            registry: None,
            settings_store: None,
        }
    }

    /// Attach a tool registry so `kind="builtin"` listings are available.
    pub fn with_registry(mut self, registry: Arc<ToolRegistry>) -> Self {
        self.registry = Some(registry);
        self
    }

    /// Attach a settings store so permission states can be read per user.
    pub fn with_settings_store(
        mut self,
        store: Arc<dyn crate::db::SettingsStore + Send + Sync>,
    ) -> Self {
        self.settings_store = Some(store);
        self
    }
}

#[async_trait]
impl Tool for ToolListTool {
    fn name(&self) -> &str {
        "tool_list"
    }

    fn description(&self) -> &str {
        "List extensions and built-in tools with their authentication, activation, and permission \
         status. Set include_available:true to also show registry entries not yet installed. \
         Use kind=\"builtin\" to list only built-in Rust tools."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "kind": {
                    "type": "string",
                    "enum": ["mcp_server", "wasm_tool", "wasm_channel", "builtin"],
                    "description": "Filter by extension type (omit to list all, including builtins)"
                },
                "include_available": {
                    "type": "boolean",
                    "description": "If true, also include registry entries that are not yet installed",
                    "default": false
                }
            }
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let kind_str = params.get("kind").and_then(|v| v.as_str());
        let want_builtin = kind_str.is_none() || kind_str == Some("builtin");
        let want_extensions = kind_str.is_none() || kind_str != Some("builtin");

        let include_available = params
            .get("include_available")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // Load per-user permission overrides (best-effort; empty map on any failure).
        let perm_overrides: std::collections::HashMap<
            String,
            crate::tools::permissions::PermissionState,
        > = if let Some(ref store) = self.settings_store {
            match store.get_all_settings(&ctx.user_id).await {
                Ok(map) => {
                    let settings = crate::settings::Settings::from_db_map(&map);
                    settings.tool_permissions
                }
                Err(e) => {
                    tracing::warn!("Failed to load tool permissions: {}", e);
                    std::collections::HashMap::new()
                }
            }
        } else {
            std::collections::HashMap::new()
        };

        let mut output = serde_json::json!({});

        // Built-in tools section — restrict to tools registered via register_sync()
        // so that dynamically-installed WASM/MCP tools are not duplicated here.
        if want_builtin && let Some(ref registry) = self.registry {
            let builtin_names = registry.builtin_tool_names().await;
            let tools = registry.all().await;
            let empty_params = serde_json::json!({});
            let builtin_list: Vec<serde_json::Value> = tools
                .iter()
                .filter(|tool| builtin_names.contains(tool.name()))
                .map(|tool| {
                    let name = tool.name().to_string();
                    let perm_state = effective_permission(&name, &perm_overrides);
                    let default_state = TOOL_RISK_DEFAULTS
                        .get(name.as_str())
                        .copied()
                        .unwrap_or(crate::tools::permissions::PermissionState::AskEachTime);
                    let locked = matches!(
                        tool.requires_approval(&empty_params),
                        ApprovalRequirement::Always
                    );
                    serde_json::json!({
                        "name": name,
                        "description": tool.description(),
                        "permission_state": perm_state,
                        "default_state": default_state,
                        "locked": locked,
                    })
                })
                .collect();
            let count = builtin_list.len();
            output["builtins"] = serde_json::json!(builtin_list);
            output["builtin_count"] = serde_json::json!(count);
        }

        // Extension (MCP / WASM) section.
        if want_extensions {
            let kind_filter = kind_str.and_then(|k| match k {
                "mcp_server" => Some(ExtensionKind::McpServer),
                "wasm_tool" => Some(ExtensionKind::WasmTool),
                "wasm_channel" => Some(ExtensionKind::WasmChannel),
                _ => None,
            });

            let extensions = self
                .manager
                .list(kind_filter, include_available, &ctx.user_id)
                .await
                .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;

            let count = extensions.len();
            output["extensions"] = serde_json::json!(extensions);
            output["count"] = serde_json::json!(count);
        }

        Ok(ToolOutput::success(output, start.elapsed()))
    }

    fn is_concurrent_safe(&self, _params: &serde_json::Value) -> bool {
        true // Read-only extension listing
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
        "Permanently remove an installed extension (channel, tool, or MCP server) from disk. \
         This action cannot be undone — the WASM binary and configuration files will be deleted."
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
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let name = require_str(&params, "name")?;

        let message = self
            .manager
            .remove(name, &ctx.user_id)
            .await
            .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;

        let output = serde_json::json!({
            "name": name,
            "message": message,
        });

        Ok(ToolOutput::success(output, start.elapsed()))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::Always
    }

    fn engine_compatibility(&self) -> EngineCompatibility {
        EngineCompatibility::V1Only
    }
}

// ── tool_upgrade ─────────────────────────────────────────────────────

pub struct ToolUpgradeTool {
    manager: Arc<ExtensionManager>,
}

impl ToolUpgradeTool {
    pub fn new(manager: Arc<ExtensionManager>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl Tool for ToolUpgradeTool {
    fn name(&self) -> &str {
        "tool_upgrade"
    }

    fn description(&self) -> &str {
        "Upgrade installed WASM extensions (channels and tools) to match the current \
         host WIT version. If name is omitted, checks and upgrades all installed WASM \
         extensions. Authentication and secrets are preserved."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Extension name to upgrade (omit to upgrade all)"
                }
            }
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let name = params.get("name").and_then(|v| v.as_str());

        let result = self
            .manager
            .upgrade(name, &ctx.user_id)
            .await
            .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;

        let output = serde_json::to_value(&result)
            .unwrap_or_else(|_| serde_json::json!({"error": "serialization failed"}));

        Ok(ToolOutput::success(output, start.elapsed()))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::UnlessAutoApproved
    }
}

// ── extension_info ────────────────────────────────────────────────────

pub struct ExtensionInfoTool {
    manager: Arc<ExtensionManager>,
}

impl ExtensionInfoTool {
    pub fn new(manager: Arc<ExtensionManager>) -> Self {
        Self { manager }
    }
}

#[async_trait]
impl Tool for ExtensionInfoTool {
    fn name(&self) -> &str {
        "extension_info"
    }

    fn description(&self) -> &str {
        "Show detailed information about an installed extension, including version \
         and WIT version compatibility."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Extension name to get info about"
                }
            },
            "required": ["name"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let name = require_str(&params, "name")?;

        let info = self
            .manager
            .extension_info(name, &ctx.user_id)
            .await
            .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;

        Ok(ToolOutput::success(info, start.elapsed()))
    }

    fn is_concurrent_safe(&self, _params: &serde_json::Value) -> bool {
        true // Read-only extension detail
    }
}

// ── tool_permission_set ───────────────────────────────────────────────────

pub struct ToolPermissionSetTool {
    registry: Arc<ToolRegistry>,
    settings_store: Option<Arc<dyn crate::db::SettingsStore + Send + Sync>>,
}

impl ToolPermissionSetTool {
    pub fn new(
        registry: Arc<ToolRegistry>,
        settings_store: Option<Arc<dyn crate::db::SettingsStore + Send + Sync>>,
    ) -> Self {
        Self {
            registry,
            settings_store,
        }
    }
}

#[async_trait]
impl Tool for ToolPermissionSetTool {
    fn name(&self) -> &str {
        "tool_permission_set"
    }

    fn description(&self) -> &str {
        "Get or set the permission state for a tool. Use to view current permissions or propose \
         a change (requires user approval). States: always_allow (no prompt), ask_each_time \
         (approval required), disabled (tool hidden from LLM)."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "tool_name": {
                    "type": "string",
                    "description": "Name of the tool to configure"
                },
                "state": {
                    "type": "string",
                    "enum": ["always_allow", "ask_each_time", "disabled"],
                    "description": "New permission state. Omit to just read the current state."
                }
            },
            "required": ["tool_name"]
        })
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::Always
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let tool_name = require_str(&params, "tool_name")?;

        // Verify that the target tool exists in the registry.
        let target_tool =
            self.registry.get(tool_name).await.ok_or_else(|| {
                ToolError::InvalidParameters(format!("Unknown tool: '{tool_name}'"))
            })?;

        // Load current settings for the user.
        let settings = if let Some(ref store) = self.settings_store {
            match store.get_all_settings(&ctx.user_id).await {
                Ok(map) => crate::settings::Settings::from_db_map(&map),
                Err(e) => {
                    return Err(ToolError::ExecutionFailed(format!(
                        "Failed to load settings: {e}"
                    )));
                }
            }
        } else {
            crate::settings::Settings::default()
        };

        let prev_state =
            crate::tools::permissions::effective_permission(tool_name, &settings.tool_permissions);

        // Read-only mode when no state param; reject non-string state values.
        let state_str = match params.get("state") {
            None => {
                let default_state = TOOL_RISK_DEFAULTS
                    .get(tool_name)
                    .copied()
                    .unwrap_or(crate::tools::permissions::PermissionState::AskEachTime);
                let output = serde_json::json!({
                    "tool_name": tool_name,
                    "current_state": prev_state,
                    "default_state": default_state,
                    "locked": matches!(
                        target_tool.requires_approval(&serde_json::json!({})),
                        ApprovalRequirement::Always
                    ),
                });
                return Ok(ToolOutput::success(output, start.elapsed()));
            }
            Some(v) => v.as_str().ok_or_else(|| {
                ToolError::InvalidParameters(
                    "'state' must be a string: always_allow, ask_each_time, or disabled"
                        .to_string(),
                )
            })?,
        };

        // Check that the target tool doesn't always require approval (locked tools
        // cannot have their permission lowered — they will always prompt).
        let empty_params = serde_json::json!({});
        if matches!(
            target_tool.requires_approval(&empty_params),
            ApprovalRequirement::Always
        ) {
            return Err(ToolError::InvalidParameters(format!(
                "'{tool_name}' always requires approval and its permission cannot be changed"
            )));
        }

        // Parse the requested new state.
        let new_state = match state_str {
            "always_allow" => crate::tools::permissions::PermissionState::AlwaysAllow,
            "ask_each_time" => crate::tools::permissions::PermissionState::AskEachTime,
            "disabled" => crate::tools::permissions::PermissionState::Disabled,
            other => {
                return Err(ToolError::InvalidParameters(format!(
                    "Invalid state '{other}'; expected always_allow, ask_each_time, or disabled"
                )));
            }
        };

        // Persist the updated permission.
        if let Some(ref store) = self.settings_store {
            let new_state_json = serde_json::to_value(new_state)
                .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;
            store
                .set_setting(
                    &ctx.user_id,
                    &format!("tool_permissions.{tool_name}"),
                    &new_state_json,
                )
                .await
                .map_err(|e| {
                    ToolError::ExecutionFailed(format!("Failed to save permission: {e}"))
                })?;
        } else {
            return Err(ToolError::ExecutionFailed(
                "No settings store configured — permission changes cannot be persisted".to_string(),
            ));
        }

        let output = serde_json::json!({
            "tool_name": tool_name,
            "prev_state": prev_state,
            "new_state": new_state,
        });
        Ok(ToolOutput::success(output, start.elapsed()))
    }

    fn engine_compatibility(&self) -> EngineCompatibility {
        EngineCompatibility::V1Only
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
    fn test_tool_search_description_clarifies_channel_setup_vs_sending() {
        let tool = ToolSearchTool {
            manager: test_manager_stub(),
        };

        let description = tool.description();
        assert!(description.contains("Use `tool_install` and `tool_activate`"));
        assert!(description.contains("use the `message` tool for proactive outbound sends"));
    }

    #[test]
    fn test_tool_install_schema() {
        use crate::tools::tool::ApprovalRequirement;
        let tool = ToolInstallTool {
            manager: test_manager_stub(),
        };
        assert_eq!(tool.name(), "tool_install");
        assert_eq!(
            tool.requires_approval(&serde_json::json!({})),
            ApprovalRequirement::UnlessAutoApproved
        );
        let schema = tool.parameters_schema();
        assert!(schema["properties"].get("name").is_some());
        assert!(schema["properties"].get("url").is_some());
    }

    #[test]
    fn test_tool_auth_schema() {
        use crate::tools::tool::ApprovalRequirement;
        let tool = ToolAuthTool {
            manager: test_manager_stub(),
        };
        assert_eq!(tool.name(), "tool_auth");
        assert_eq!(
            tool.requires_approval(&serde_json::json!({})),
            ApprovalRequirement::UnlessAutoApproved
        );
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
        use crate::tools::tool::ApprovalRequirement;
        let tool = ToolActivateTool {
            manager: test_manager_stub(),
        };
        assert_eq!(tool.name(), "tool_activate");
        assert_eq!(
            tool.requires_approval(&serde_json::json!({})),
            ApprovalRequirement::Never
        );
    }

    #[test]
    fn activation_error_requires_auth_detects_auth_required_variants() {
        assert!(activation_error_requires_auth("Authentication required"));
        assert!(activation_error_requires_auth("not authenticated"));
        assert!(activation_error_requires_auth("401 unauthorized"));
        assert!(activation_error_requires_auth("Unauthorized"));
        assert!(!activation_error_requires_auth(
            "Activation failed: crashed"
        ));
    }

    #[test]
    fn test_tool_list_schema() {
        use crate::tools::tool::ApprovalRequirement;
        let tool = ToolListTool::new(test_manager_stub());
        assert_eq!(tool.name(), "tool_list");
        assert_eq!(
            tool.requires_approval(&serde_json::json!({})),
            ApprovalRequirement::Never
        );
        let schema = tool.parameters_schema();
        assert!(schema["properties"].get("kind").is_some());
        // Verify the new "builtin" kind is included in the enum.
        let enum_vals = schema["properties"]["kind"]["enum"]
            .as_array()
            .expect("kind must have an enum array");
        let kind_names: Vec<&str> = enum_vals.iter().filter_map(|v| v.as_str()).collect();
        assert!(
            kind_names.contains(&"builtin"),
            "tool_list kind enum must include 'builtin'"
        );
    }

    #[test]
    fn test_tool_remove_schema() {
        use crate::tools::tool::ApprovalRequirement;
        let tool = ToolRemoveTool {
            manager: test_manager_stub(),
        };
        assert_eq!(tool.name(), "tool_remove");
        assert_eq!(
            tool.requires_approval(&serde_json::json!({})),
            ApprovalRequirement::Always
        );
    }

    #[test]
    fn tool_remove_always_requires_approval_regardless_of_params() {
        use crate::tools::tool::ApprovalRequirement;
        let tool = ToolRemoveTool {
            manager: test_manager_stub(),
        };

        let test_cases = vec![
            ("no params", serde_json::json!({})),
            ("empty name", serde_json::json!({"name": ""})),
            ("slack", serde_json::json!({"name": "slack"})),
            ("github-cli", serde_json::json!({"name": "github-cli"})),
            (
                "with extra fields",
                serde_json::json!({"name": "tool", "extra": "field"}),
            ),
        ];

        for (case_name, params) in test_cases {
            assert_eq!(
                tool.requires_approval(&params),
                ApprovalRequirement::Always,
                "tool_remove must always require approval for case: {}",
                case_name
            );
        }
    }

    #[tokio::test]
    async fn tool_auth_no_approval_in_gateway_mode() {
        let manager = test_manager_stub();
        manager
            .enable_gateway_mode("http://localhost:3000".to_string())
            .await;
        let tool = ToolAuthTool {
            manager: manager.clone(),
        };
        assert_eq!(
            tool.requires_approval(&serde_json::json!({})),
            ApprovalRequirement::Never,
            "tool_auth should not require approval in gateway mode"
        );
    }

    #[test]
    fn test_tool_upgrade_schema() {
        use crate::tools::tool::ApprovalRequirement;
        let tool = ToolUpgradeTool {
            manager: test_manager_stub(),
        };
        assert_eq!(tool.name(), "tool_upgrade");
        assert_eq!(
            tool.requires_approval(&serde_json::json!({})),
            ApprovalRequirement::UnlessAutoApproved
        );
        let schema = tool.parameters_schema();
        // name is optional (omit to upgrade all)
        assert!(schema["properties"].get("name").is_some());
        assert!(
            schema.get("required").is_none(),
            "tool_upgrade should have no required params"
        );
    }

    #[test]
    fn test_extension_info_schema() {
        let tool = ExtensionInfoTool {
            manager: test_manager_stub(),
        };
        assert_eq!(tool.name(), "extension_info");
        let schema = tool.parameters_schema();
        assert!(schema["properties"].get("name").is_some());
        let required = schema["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v.as_str() == Some("name")));
    }

    /// Create a stub manager for schema tests (these don't call execute).
    fn test_manager_stub() -> Arc<ExtensionManager> {
        use crate::secrets::{InMemorySecretsStore, SecretsCrypto};
        use crate::testing::credentials::TEST_CRYPTO_KEY;
        use crate::tools::ToolRegistry;
        use crate::tools::mcp::session::McpSessionManager;

        let master_key = secrecy::SecretString::from(TEST_CRYPTO_KEY.to_string());
        let crypto = Arc::new(SecretsCrypto::new(master_key).unwrap());

        Arc::new(ExtensionManager::new(
            Arc::new(McpSessionManager::new()),
            Arc::new(crate::tools::mcp::process::McpProcessManager::new()),
            Arc::new(InMemorySecretsStore::new(crypto)),
            Arc::new(ToolRegistry::new()),
            None,
            None,
            std::env::temp_dir().join("ironclaw-test-tools"),
            std::env::temp_dir().join("ironclaw-test-channels"),
            None,
            "test".to_string(),
            None,
            Vec::new(),
        ))
    }

    // ── tool_permission_set tests ─────────────────────────────────────────

    /// A simple tool used in tests that requires approval Always (locked).
    struct LockedTool;

    #[async_trait]
    impl Tool for LockedTool {
        fn name(&self) -> &str {
            "locked_tool"
        }
        fn description(&self) -> &str {
            "A test tool that always requires approval"
        }
        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({ "type": "object", "properties": {} })
        }
        async fn execute(
            &self,
            _params: serde_json::Value,
            _ctx: &JobContext,
        ) -> Result<ToolOutput, ToolError> {
            unreachable!()
        }
        fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
            ApprovalRequirement::Always
        }
    }

    /// A simple tool used in tests that does not lock its permission.
    struct NormalTool;

    #[async_trait]
    impl Tool for NormalTool {
        fn name(&self) -> &str {
            "normal_tool"
        }
        fn description(&self) -> &str {
            "A normal test tool"
        }
        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({ "type": "object", "properties": {} })
        }
        async fn execute(
            &self,
            _params: serde_json::Value,
            _ctx: &JobContext,
        ) -> Result<ToolOutput, ToolError> {
            unreachable!()
        }
    }

    #[tokio::test]
    async fn test_tool_permission_set_unknown_tool_returns_error() {
        use crate::context::JobContext;
        use crate::tools::ToolRegistry;

        let registry = Arc::new(ToolRegistry::new());
        // Do not register any tool — asking for "unknown_xyz" should fail.
        let tool = ToolPermissionSetTool::new(Arc::clone(&registry), None);
        let ctx = JobContext::default();
        let result = tool
            .execute(serde_json::json!({"tool_name": "unknown_xyz"}), &ctx) // safety: Tool::execute, not DB
            .await;
        assert!(result.is_err(), "expected error for unknown tool");
        let err = result.unwrap_err();
        assert!(
            matches!(err, ToolError::InvalidParameters(_)),
            "expected InvalidParameters, got {err:?}"
        );
    }

    #[tokio::test]
    async fn test_tool_permission_set_locked_tool_rejected() {
        use crate::context::JobContext;
        use crate::tools::ToolRegistry;

        let registry = Arc::new(ToolRegistry::new());
        registry.register(Arc::new(LockedTool)).await;

        let tool = ToolPermissionSetTool::new(Arc::clone(&registry), None);
        let ctx = JobContext::default();

        // Trying to change the permission state of a locked tool must fail.
        let result = tool
            .execute(
                serde_json::json!({"tool_name": "locked_tool", "state": "always_allow"}),
                &ctx,
            )
            .await;
        assert!(
            result.is_err(),
            "locked tool permission change must be rejected"
        );
        let err = result.unwrap_err();
        assert!(
            matches!(err, ToolError::InvalidParameters(_)),
            "expected InvalidParameters for locked tool, got {err:?}"
        );
    }

    #[test]
    fn test_tool_permission_set_always_requires_approval() {
        use crate::tools::ToolRegistry;

        let registry = Arc::new(ToolRegistry::new());
        let tool = ToolPermissionSetTool::new(Arc::clone(&registry), None);
        assert_eq!(
            tool.requires_approval(&serde_json::json!({})),
            ApprovalRequirement::Always,
            "tool_permission_set must always require approval"
        );
    }

    #[tokio::test]
    async fn test_tool_list_includes_builtin_kind() {
        use crate::context::JobContext;
        use crate::tools::ToolRegistry;

        let registry = Arc::new(ToolRegistry::new());
        // Use register_sync (built-in path) so the tool appears in builtin_tool_names().
        registry.register_sync(Arc::new(NormalTool));

        let manager = test_manager_stub();
        let list_tool = ToolListTool::new(manager).with_registry(Arc::clone(&registry));

        let ctx = JobContext::default();
        let result = list_tool
            .execute(serde_json::json!({"kind": "builtin"}), &ctx) // safety: Tool::execute, not DB
            .await
            .expect("tool_list kind=builtin should succeed");

        let builtins = result.result["builtins"]
            .as_array()
            .expect("result should have builtins array");
        assert!(
            !builtins.is_empty(),
            "builtins list should not be empty when registry has tools"
        );
        let names: Vec<&str> = builtins
            .iter()
            .filter_map(|entry| entry["name"].as_str())
            .collect();
        assert!(
            names.contains(&"normal_tool"),
            "registered normal_tool should appear in builtins listing"
        );
        // Each entry must have required fields.
        for entry in builtins {
            assert!(entry.get("name").is_some(), "missing name field");
            assert!(
                entry.get("description").is_some(),
                "missing description field"
            );
            assert!(
                entry.get("permission_state").is_some(),
                "missing permission_state"
            );
            assert!(
                entry.get("default_state").is_some(),
                "missing default_state"
            );
            assert!(entry.get("locked").is_some(), "missing locked field");
        }
        // Extensions should not be present for kind=builtin.
        assert!(
            result.result.get("extensions").is_none(),
            "kind=builtin should not return extensions"
        );
    }
}
