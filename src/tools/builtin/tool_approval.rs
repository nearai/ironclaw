//! Tool for managing persistent tool approval allowlists.
//!
//! Stores per-user "always allow" tool decisions in settings so approvals can
//! be audited and reversed.

use std::collections::BTreeSet;
use std::sync::Arc;

use async_trait::async_trait;

use crate::context::JobContext;
use crate::db::Database;
use crate::tools::tool::{ApprovalRequirement, Tool, ToolDomain, ToolError, ToolOutput};

const TOOL_APPROVALS_KEY: &str = "auto_approved_tools";

fn parse_tool_set(value: Option<serde_json::Value>) -> BTreeSet<String> {
    value
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|v| v.as_str().map(|s| s.to_string()))
        .collect()
}

fn to_json_array(tools: &BTreeSet<String>) -> serde_json::Value {
    serde_json::Value::Array(
        tools
            .iter()
            .cloned()
            .map(serde_json::Value::String)
            .collect(),
    )
}

/// Manage persistent "always allow" approvals for tools.
pub struct ToolApprovalTool {
    store: Arc<dyn Database>,
}

impl ToolApprovalTool {
    pub fn new(store: Arc<dyn Database>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for ToolApprovalTool {
    fn name(&self) -> &str {
        "tool_approval"
    }

    fn description(&self) -> &str {
        "Manage persistent tool approval allowlist for this user. \
         Use action=list|allow|revoke|clear. This is auditable and can reverse accidental 'always allow'."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["list", "allow", "revoke", "clear"],
                    "description": "Action to perform on persistent tool approvals."
                },
                "tool": {
                    "type": "string",
                    "description": "Tool name for allow/revoke actions."
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();
        let user_id = &ctx.user_id;

        let action = params
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParameters("Missing action".to_string()))?;

        let current = self
            .store
            .get_setting(user_id, TOOL_APPROVALS_KEY)
            .await
            .map_err(|e| ToolError::ExecutionFailed(format!("Failed to load approvals: {}", e)))?;
        let mut tools = parse_tool_set(current);

        let mut changed = false;
        let message = match action {
            "list" => "Listed persistent approvals".to_string(),
            "allow" => {
                let tool = params
                    .get("tool")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| ToolError::InvalidParameters("Missing tool".to_string()))?;
                changed = tools.insert(tool.to_string());
                if changed {
                    format!("Allowed '{}' persistently", tool)
                } else {
                    format!("'{}' was already persistently allowed", tool)
                }
            }
            "revoke" => {
                let tool = params
                    .get("tool")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| ToolError::InvalidParameters("Missing tool".to_string()))?;
                changed = tools.remove(tool);
                if changed {
                    format!("Revoked persistent allow for '{}'", tool)
                } else {
                    format!("'{}' was not persistently allowed", tool)
                }
            }
            "clear" => {
                changed = !tools.is_empty();
                tools.clear();
                "Cleared all persistent approvals".to_string()
            }
            other => {
                return Err(ToolError::InvalidParameters(format!(
                    "Unsupported action '{}'",
                    other
                )));
            }
        };

        if changed {
            self.store
                .set_setting(user_id, TOOL_APPROVALS_KEY, &to_json_array(&tools))
                .await
                .map_err(|e| {
                    ToolError::ExecutionFailed(format!("Failed to persist approvals: {}", e))
                })?;
        }

        Ok(ToolOutput::success(
            serde_json::json!({
                "action": action,
                "changed": changed,
                "approvals": tools.into_iter().collect::<Vec<_>>(),
                "message": message,
            }),
            start.elapsed(),
        ))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::Always
    }

    fn domain(&self) -> ToolDomain {
        ToolDomain::Orchestrator
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_serialize_are_stable() {
        let parsed = parse_tool_set(Some(serde_json::json!(["shell", "open_file", "shell"])));
        assert_eq!(
            parsed.into_iter().collect::<Vec<_>>(),
            vec!["open_file".to_string(), "shell".to_string()]
        );
    }

    #[test]
    fn to_json_array_roundtrip() {
        let mut set = BTreeSet::new();
        set.insert("read_file".to_string());
        set.insert("open_file".to_string());
        let json = to_json_array(&set);
        assert_eq!(json, serde_json::json!(["open_file", "read_file"]));
        let roundtrip = parse_tool_set(Some(json));
        assert_eq!(roundtrip, set);
    }
}
