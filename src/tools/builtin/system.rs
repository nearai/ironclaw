//! System introspection tools.
//!
//! These tools replace hardcoded system commands (`/tools`, `/version`) with
//! proper `Tool` implementations that go through the standard dispatch
//! pipeline with audit trail. They work in both v1 and v2 engines.
//!
//! Future tools (`system_skills_list`, `system_model_get/set`) are planned
//! as part of #2049's Phase 4 follow-up.

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
        let mut tools: Vec<serde_json::Value> = defs
            .into_iter()
            .map(|td| {
                json!({
                    "name": td.name,
                    "description": td.description
                })
            })
            .collect();

        // Merge v2 capability actions (not `Tool` impls, absent from
        // `tool_definitions()`). No-op on v1. Assumes v1/v2 names don't overlap.
        if let Some(cap_registry) = self.registry.capability_registry().await {
            for cap in cap_registry.list() {
                for action in &cap.actions {
                    tools.push(json!({
                        "name": action.name,
                        "description": action.description,
                    }));
                }
            }
        }

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

    fn sample_capability() -> ironclaw_engine::types::capability::Capability {
        ironclaw_engine::types::capability::Capability {
            name: "missions".into(),
            description: "Mission lifecycle".into(),
            actions: vec![ironclaw_engine::types::capability::ActionDef {
                name: "mission_create".into(),
                description: "Create a new mission".into(),
                parameters_schema: serde_json::json!({"type": "object"}),
                effects: vec![],
                requires_approval: false,
            }],
            knowledge: vec![],
            policies: vec![],
        }
    }

    #[tokio::test]
    async fn system_tools_list_includes_capability_actions() {
        let registry = Arc::new(ToolRegistry::new());
        let mut caps = ironclaw_engine::CapabilityRegistry::new();
        caps.register(sample_capability());
        registry.set_capability_registry(Arc::new(caps)).await;

        let tool = SystemToolsListTool::new(Arc::clone(&registry));
        let ctx = JobContext::default();
        let result = tool
            .execute(serde_json::json!({}), &ctx)
            .await
            .expect("system_tools_list should succeed");

        let names: Vec<&str> = result.result["tools"]
            .as_array()
            .expect("tools array")
            .iter()
            .filter_map(|t| t["name"].as_str())
            .collect();
        assert!(
            names.contains(&"mission_create"),
            "system_tools_list must include capability actions when a capability \
             registry is wired, got: {names:?}"
        );
    }
}
