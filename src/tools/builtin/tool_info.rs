//! On-demand tool discovery (like CLI `--help`).
//!
//! Three levels of detail:
//! - Default: name, description, parameter names (compact ~150 bytes)
//! - `detail: "summary"`: adds curated rules, notes, and examples
//! - `detail: "schema"` / `include_schema: true`: adds the full typed JSON Schema
//!
//! Keeps the tools array compact (WASM tools use permissive schemas)
//! while allowing precise discovery when needed.

use std::sync::Weak;

use async_trait::async_trait;

use crate::context::JobContext;
use crate::tools::registry::ToolRegistry;
use crate::tools::tool::{Tool, ToolDiscoverySummary, ToolError, ToolOutput, require_str};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ToolInfoDetail {
    Names,
    Summary,
    Schema,
}

impl ToolInfoDetail {
    fn parse(params: &serde_json::Value) -> Result<Self, ToolError> {
        if params
            .get("include_schema")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            return Ok(Self::Schema);
        }

        match params.get("detail").and_then(|v| v.as_str()) {
            None | Some("names") => Ok(Self::Names),
            Some("summary") => Ok(Self::Summary),
            Some("schema") => Ok(Self::Schema),
            Some(other) => Err(ToolError::InvalidParameters(format!(
                "invalid detail '{other}' (expected 'names', 'summary', or 'schema')"
            ))),
        }
    }
}

fn schema_param_names(schema: &serde_json::Value) -> Vec<String> {
    schema
        .get("properties")
        .and_then(|p| p.as_object())
        .map(|props| props.keys().cloned().collect())
        .unwrap_or_default()
}

fn fallback_summary(schema: &serde_json::Value) -> ToolDiscoverySummary {
    ToolDiscoverySummary {
        always_required: schema
            .get("required")
            .and_then(|v| v.as_array())
            .map(|required| {
                required
                    .iter()
                    .filter_map(|value| value.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default(),
        ..ToolDiscoverySummary::default()
    }
}

pub struct ToolInfoTool {
    registry: Weak<ToolRegistry>,
}

impl ToolInfoTool {
    pub fn new(registry: Weak<ToolRegistry>) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl Tool for ToolInfoTool {
    fn name(&self) -> &str {
        "tool_info"
    }

    fn description(&self) -> &str {
        "Get info about any tool: description, parameter names, curated summary guidance, or full discovery schema."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Name of the tool to get info about"
                },
                "detail": {
                    "type": "string",
                    "enum": ["names", "summary", "schema"],
                    "description": "Response detail level. 'names' returns parameter names only. 'summary' adds curated rules/examples. 'schema' returns the full discovery schema.",
                    "default": "names"
                },
                "include_schema": {
                    "type": "boolean",
                    "description": "Deprecated compatibility alias for detail='schema'. If true, include the full discovery schema.",
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
        let detail = ToolInfoDetail::parse(&params)?;

        let registry = self.registry.upgrade().ok_or_else(|| {
            ToolError::ExecutionFailed(
                "tool registry is no longer available for tool_info".to_string(),
            )
        })?;

        let tool = registry.get(name).await.ok_or_else(|| {
            ToolError::InvalidParameters(format!("No tool named '{name}' is registered"))
        })?;

        let schema = tool.discovery_schema();
        let param_names = schema_param_names(&schema);

        let mut info = serde_json::json!({
            "name": tool.name(),
            "description": tool.description(),
            "parameters": param_names,
        });

        match detail {
            ToolInfoDetail::Names => {}
            ToolInfoDetail::Summary => {
                let summary = tool
                    .discovery_summary()
                    .unwrap_or_else(|| fallback_summary(&schema));
                info["summary"] =
                    serde_json::to_value(summary).expect("ToolDiscoverySummary should serialize");
            }
            ToolInfoDetail::Schema => {
                info["schema"] = schema;
            }
        }

        Ok(ToolOutput::success(info, start.elapsed()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::builtin::EchoTool;
    use std::sync::Arc;

    #[tokio::test]
    async fn test_tool_info_default_returns_param_names() {
        let registry = Arc::new(ToolRegistry::new());
        registry.register(Arc::new(EchoTool)).await;

        let tool = ToolInfoTool::new(Arc::downgrade(&registry));
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
    async fn test_tool_info_with_summary() {
        let registry = Arc::new(ToolRegistry::new());
        registry.register(Arc::new(EchoTool)).await;

        let tool = ToolInfoTool::new(Arc::downgrade(&registry));
        let ctx = JobContext::default();
        let result = tool
            .execute(
                serde_json::json!({"name": "echo", "detail": "summary"}),
                &ctx,
            )
            .await
            .unwrap();

        let info = &result.result;
        assert_eq!(info["name"], "echo");
        assert!(info["summary"].is_object());
        assert_eq!(
            info["summary"]["always_required"],
            serde_json::json!(["message"])
        );
    }

    #[tokio::test]
    async fn test_tool_info_with_schema() {
        let registry = Arc::new(ToolRegistry::new());
        registry.register(Arc::new(EchoTool)).await;

        let tool = ToolInfoTool::new(Arc::downgrade(&registry));
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
    async fn test_tool_info_invalid_detail() {
        let registry = Arc::new(ToolRegistry::new());
        registry.register(Arc::new(EchoTool)).await;

        let tool = ToolInfoTool::new(Arc::downgrade(&registry));
        let ctx = JobContext::default();
        let result = tool
            .execute(
                serde_json::json!({"name": "echo", "detail": "verbose"}),
                &ctx,
            )
            .await;
        assert!(matches!(result, Err(ToolError::InvalidParameters(_))));
    }

    #[tokio::test]
    async fn test_tool_info_unknown_tool() {
        let registry = Arc::new(ToolRegistry::new());
        let tool = ToolInfoTool::new(Arc::downgrade(&registry));
        let ctx = JobContext::default();
        let result = tool
            .execute(serde_json::json!({"name": "nonexistent"}), &ctx)
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_tool_info_registry_dropped() {
        let registry = Arc::new(ToolRegistry::new());
        let tool = ToolInfoTool::new(Arc::downgrade(&registry));
        drop(registry);

        let ctx = JobContext::default();
        let result = tool
            .execute(serde_json::json!({"name": "echo"}), &ctx)
            .await;
        assert!(matches!(result, Err(ToolError::ExecutionFailed(_))));
    }
}
