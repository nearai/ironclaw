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
use crate::tools::tool::{
    Tool, ToolDiscoverySummary, ToolError, ToolOutput, require_str, resolve_with_aliases,
};

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
    let mut names = std::collections::BTreeSet::new();

    if let Some(props) = schema.get("properties").and_then(|p| p.as_object()) {
        names.extend(props.keys().cloned());
    }

    for key in ["allOf", "oneOf", "anyOf"] {
        if let Some(variants) = schema.get(key).and_then(|v| v.as_array()) {
            for variant in variants {
                if let Some(props) = variant.get("properties").and_then(|p| p.as_object()) {
                    names.extend(props.keys().cloned());
                }
            }
        }
    }

    names.into_iter().collect()
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

        // Search the v1 tool map first, then fall through to v2 capability
        // actions. On v1 the capability registry is `None`, so the fallback
        // is a no-op. Assumes v1 tool and v2 capability names don't overlap.
        let (action_name, description, schema, summary) = if let Some(tool) =
            registry.get(name).await
        {
            if !tool
                .engine_compatibility()
                .is_visible_in(registry.engine_version())
            {
                return Err(ToolError::InvalidParameters(format!(
                    "Tool '{name}' is not available in the current engine version"
                )));
            }
            let schema = tool.discovery_schema();
            let summary = tool
                .discovery_summary()
                .unwrap_or_else(|| fallback_summary(&schema));
            (
                tool.name().to_string(),
                tool.description().to_string(),
                schema,
                summary,
            )
        } else if let Some(cap_registry) = registry.capability_registry().await
            && let Some(action) =
                resolve_with_aliases(name, |n| cap_registry.find_action(n).map(|(_, a)| a.clone()))
        {
            let schema = action.parameters_schema.clone();
            let summary = fallback_summary(&schema);
            (action.name, action.description, schema, summary)
        } else {
            return Err(ToolError::InvalidParameters(format!(
                "No tool named '{name}' is registered"
            )));
        };

        let param_names = schema_param_names(&schema);
        let mut info = serde_json::json!({
            "name": action_name,
            "description": description,
            "parameters": param_names,
        });

        match detail {
            ToolInfoDetail::Names => {}
            ToolInfoDetail::Summary => {
                info["summary"] = serde_json::to_value(summary).map_err(|err| {
                    ToolError::ExecutionFailed(format!(
                        "failed to serialize discovery summary: {err}"
                    ))
                })?;
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
    use crate::tools::EngineVersion;
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

    #[tokio::test]
    async fn test_tool_info_rejects_v1_only_in_v2_registry() {
        use crate::tools::tool::{EngineCompatibility, EngineVersion};

        struct V1OnlyStub;

        #[async_trait]
        impl Tool for V1OnlyStub {
            fn name(&self) -> &str {
                "v1_stub"
            }
            fn description(&self) -> &str {
                "test"
            }
            fn parameters_schema(&self) -> serde_json::Value {
                serde_json::json!({"type": "object"})
            }
            async fn execute(
                &self,
                _params: serde_json::Value,
                _ctx: &JobContext,
            ) -> Result<ToolOutput, ToolError> {
                unreachable!()
            }
            fn engine_compatibility(&self) -> EngineCompatibility {
                EngineCompatibility::V1Only
            }
        }

        let registry = Arc::new(ToolRegistry::new().with_engine_version(EngineVersion::V2));
        registry.register(Arc::new(V1OnlyStub)).await;

        let tool = ToolInfoTool::new(Arc::downgrade(&registry));
        let ctx = JobContext::default();
        let result = tool
            .execute(serde_json::json!({"name": "v1_stub"}), &ctx)
            .await;
        assert!(
            matches!(result, Err(ToolError::InvalidParameters(ref msg)) if msg.contains("not available")),
            "tool_info should reject V1Only tools in V2 registry"
        );
    }

    // Engine v2 capability-registry fallback: capability actions live in
    // `CapabilityRegistry`, not `ToolRegistry`. These tests lock in that
    // `tool_info` consults both sources.

    fn sample_capability() -> ironclaw_engine::types::capability::Capability {
        ironclaw_engine::types::capability::Capability {
            name: "missions".into(),
            description: "Mission lifecycle".into(),
            actions: vec![ironclaw_engine::types::capability::ActionDef {
                name: "mission_create".into(),
                description: "Create a new mission".into(),
                parameters_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "name": {"type": "string"},
                        "goal": {"type": "string"},
                        "cadence": {"type": "string"}
                    },
                    "required": ["name", "goal", "cadence"]
                }),
                effects: vec![],
                requires_approval: false,
            }],
            knowledge: vec![],
            policies: vec![],
        }
    }

    #[tokio::test]
    async fn test_tool_info_surfaces_capability_action_on_v2_registry() {
        let registry = Arc::new(ToolRegistry::new().with_engine_version(EngineVersion::V2));

        let mut caps = ironclaw_engine::CapabilityRegistry::new();
        caps.register(sample_capability());
        registry.set_capability_registry(Arc::new(caps)).await;

        let tool = ToolInfoTool::new(Arc::downgrade(&registry));
        let ctx = JobContext::default();

        let result = tool
            .execute(
                serde_json::json!({"name": "mission_create", "detail": "schema"}),
                &ctx,
            )
            .await
            .expect("mission_create should resolve via capability registry on v2");

        let info = &result.result;
        assert_eq!(info["name"], "mission_create");
        assert_eq!(info["description"], "Create a new mission");
        let params = info["parameters"].as_array().expect("parameters array");
        let param_strs: Vec<&str> = params.iter().filter_map(|v| v.as_str()).collect();
        assert!(
            param_strs.contains(&"name")
                && param_strs.contains(&"goal")
                && param_strs.contains(&"cadence"),
            "expected name/goal/cadence in params: {param_strs:?}"
        );
        assert_eq!(info["schema"]["required"], serde_json::json!(["name", "goal", "cadence"]));
    }

    #[tokio::test]
    async fn test_tool_info_capability_action_summary_path() {
        // The summary detail for capability actions goes through
        // `fallback_summary(&schema)` — the only summary source for
        // capabilities, since `ActionDef` has no `discovery_summary()`.
        let registry = Arc::new(ToolRegistry::new().with_engine_version(EngineVersion::V2));
        let mut caps = ironclaw_engine::CapabilityRegistry::new();
        caps.register(sample_capability());
        registry.set_capability_registry(Arc::new(caps)).await;

        let tool = ToolInfoTool::new(Arc::downgrade(&registry));
        let ctx = JobContext::default();
        let result = tool
            .execute(
                serde_json::json!({"name": "mission_create", "detail": "summary"}),
                &ctx,
            )
            .await
            .expect("summary detail should resolve for capability action");

        assert_eq!(
            result.result["summary"]["always_required"],
            serde_json::json!(["name", "goal", "cadence"]),
            "fallback_summary should pull required fields from the schema"
        );
    }

    #[tokio::test]
    async fn test_tool_info_resolves_hyphenated_capability_alias() {
        // LLMs and providers sometimes normalize underscores to hyphens.
        // tool_info must resolve `mission-create` the same as `mission_create`
        // — mirroring `ToolRegistry::get`'s alias logic.
        let registry = Arc::new(ToolRegistry::new().with_engine_version(EngineVersion::V2));
        let mut caps = ironclaw_engine::CapabilityRegistry::new();
        caps.register(sample_capability());
        registry.set_capability_registry(Arc::new(caps)).await;

        let tool = ToolInfoTool::new(Arc::downgrade(&registry));
        let ctx = JobContext::default();
        let result = tool
            .execute(serde_json::json!({"name": "mission-create"}), &ctx)
            .await
            .expect("hyphenated form should resolve to underscored capability action");

        assert_eq!(result.result["name"], "mission_create");
    }

    #[tokio::test]
    async fn test_tool_info_v2_without_capability_registry_still_rejects_missions() {
        // Unwired v2 registry (bootstrap hasn't run yet) must reject
        // cleanly — no panic, no misleading schema.
        let registry = Arc::new(ToolRegistry::new().with_engine_version(EngineVersion::V2));

        let tool = ToolInfoTool::new(Arc::downgrade(&registry));
        let ctx = JobContext::default();

        let result = tool
            .execute(serde_json::json!({"name": "mission_create"}), &ctx)
            .await;
        assert!(
            matches!(result, Err(ToolError::InvalidParameters(ref msg)) if msg.contains("No tool named")),
            "unwired v2 registry must reject cleanly, got: {result:?}"
        );
    }
}
