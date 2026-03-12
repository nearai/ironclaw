//! On-demand tool discovery (like CLI `--help`).
//!
//! Two levels of detail:
//! - Default: name, description, parameter names (compact ~150 bytes)
//! - `include_schema: true`: adds the full typed JSON Schema
//!
//! Keeps the tools array compact (WASM tools use permissive schemas)
//! while allowing precise discovery when needed.

use std::sync::Arc;

use async_trait::async_trait;

use crate::context::JobContext;
use crate::tools::registry::ToolRegistry;
use crate::tools::tool::{Tool, ToolError, ToolOutput, require_str};

pub struct ToolInfoTool {
    registry: Arc<ToolRegistry>,
}

impl ToolInfoTool {
    pub fn new(registry: Arc<ToolRegistry>) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl Tool for ToolInfoTool {
    fn name(&self) -> &str {
        "tool_info"
    }

    fn description(&self) -> &str {
        "Get info about any tool: description and parameter names. \
         Set include_schema to true for the full typed parameter schema."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Name of the tool to get info about"
                },
                "include_schema": {
                    "type": "boolean",
                    "description": "If true, include the full typed JSON Schema for parameters (larger response). Default: false.",
                    "default": false
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
        let include_schema = params
            .get("include_schema")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let tool = self.registry.get(name).await.ok_or_else(|| {
            ToolError::InvalidParameters(format!("No tool named '{name}' is registered"))
        })?;

        let schema = tool.discovery_schema();

        // Extract just param names from the schema's "properties" keys
        let param_names: Vec<&str> = schema
            .get("properties")
            .and_then(|p| p.as_object())
            .map(|props| props.keys().map(|k| k.as_str()).collect())
            .unwrap_or_default();

        let mut info = serde_json::json!({
            "name": tool.name(),
            "description": tool.description(),
            "parameters": param_names,
        });

        if include_schema {
            info["schema"] = schema;
        }

        Ok(ToolOutput::success(info, start.elapsed()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::builtin::EchoTool;

    #[tokio::test]
    async fn test_tool_info_default_returns_param_names() {
        let registry = Arc::new(ToolRegistry::new());
        registry.register(Arc::new(EchoTool)).await;

        let tool = ToolInfoTool::new(Arc::clone(&registry));
        let ctx = JobContext::default();
        let result = tool
            .execute(serde_json::json!({"name": "echo"}), &ctx)
            .await
            .unwrap();

        let info = &result.result;
        assert_eq!(info["name"], "echo");
        assert!(!info["description"].as_str().unwrap().is_empty());
        // Default: parameters is an array of names, not the full schema
        assert!(info["parameters"].is_array());
        assert!(
            info["parameters"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v.as_str() == Some("message")),
            "echo tool should have 'message' parameter: {:?}",
            info["parameters"]
        );
        // No schema field by default
        assert!(info.get("schema").is_none());
    }

    #[tokio::test]
    async fn test_tool_info_with_schema() {
        let registry = Arc::new(ToolRegistry::new());
        registry.register(Arc::new(EchoTool)).await;

        let tool = ToolInfoTool::new(Arc::clone(&registry));
        let ctx = JobContext::default();
        let result = tool
            .execute(
                serde_json::json!({"name": "echo", "include_schema": true}),
                &ctx,
            )
            .await
            .unwrap();

        let info = &result.result;
        assert_eq!(info["name"], "echo");
        // With include_schema: true, schema field should be present
        assert!(info["schema"].is_object());
        assert!(info["schema"]["properties"].is_object());
    }

    #[tokio::test]
    async fn test_tool_info_unknown_tool() {
        let registry = Arc::new(ToolRegistry::new());
        let tool = ToolInfoTool::new(registry);
        let ctx = JobContext::default();
        let result = tool
            .execute(serde_json::json!({"name": "nonexistent"}), &ctx)
            .await;
        assert!(result.is_err());
    }
}
