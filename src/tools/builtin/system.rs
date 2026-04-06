//! System introspection and configuration tools.
//!
//! These tools replace hardcoded system commands (`/tools`, `/skills`, `/model`,
//! `/version`) with proper `Tool` implementations that go through the standard
//! dispatch pipeline with audit trail. They work in both v1 and v2 engines.

use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use serde_json::json;

use crate::context::JobContext;
use crate::tools::registry::ToolRegistry;
use crate::tools::tool::{Tool, ToolError, ToolOutput};

// ==================== system_tools_list ====================

/// Lists all registered tools with their names and descriptions.
pub struct SystemToolsListTool {
    registry: Arc<ToolRegistry>,
}

impl SystemToolsListTool {
    pub fn new(registry: Arc<ToolRegistry>) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl Tool for SystemToolsListTool {
    fn name(&self) -> &str {
        "system_tools_list"
    }

    fn description(&self) -> &str {
        "List all registered tools with names and descriptions"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
    }

    async fn execute(
        &self,
        _params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();
        let defs = self.registry.tool_definitions().await;
        let tools: Vec<serde_json::Value> = defs
            .into_iter()
            .map(|td| {
                json!({
                    "name": td.name,
                    "description": td.description
                })
            })
            .collect();
        Ok(ToolOutput::success(
            json!({ "tools": tools, "count": tools.len() }),
            start.elapsed(),
        ))
    }
}

// ==================== system_version ====================

/// Returns the agent version information.
pub struct SystemVersionTool;

#[async_trait]
impl Tool for SystemVersionTool {
    fn name(&self) -> &str {
        "system_version"
    }

    fn description(&self) -> &str {
        "Get the agent version and build information"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
    }

    async fn execute(
        &self,
        _params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = Instant::now();
        Ok(ToolOutput::success(
            json!({
                "version": env!("CARGO_PKG_VERSION"),
                "name": env!("CARGO_PKG_NAME"),
            }),
            start.elapsed(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_tool_name() {
        let tool = SystemVersionTool;
        assert_eq!(tool.name(), "system_version");
    }

    #[test]
    fn tools_list_tool_name() {
        let registry = Arc::new(ToolRegistry::new());
        let tool = SystemToolsListTool::new(registry);
        assert_eq!(tool.name(), "system_tools_list");
    }
}
