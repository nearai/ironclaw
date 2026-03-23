//! Effect bridge adapter — wraps `ToolRegistry` + `SafetyLayer` as `ironclaw_engine::EffectExecutor`.

use std::sync::Arc;
use std::time::Duration;

use ironclaw_engine::{
    ActionDef, ActionResult, CapabilityLease, EffectExecutor, EngineError, ThreadExecutionContext,
};

use crate::context::JobContext;
use crate::safety::SafetyLayer;
use crate::tools::ToolRegistry;

/// Wraps the existing tool pipeline to implement the engine's `EffectExecutor`.
pub struct EffectBridgeAdapter {
    tools: Arc<ToolRegistry>,
    safety: Arc<SafetyLayer>,
}

impl EffectBridgeAdapter {
    pub fn new(tools: Arc<ToolRegistry>, safety: Arc<SafetyLayer>) -> Self {
        Self { tools, safety }
    }
}

#[async_trait::async_trait]
impl EffectExecutor for EffectBridgeAdapter {
    async fn execute_action(
        &self,
        action_name: &str,
        parameters: serde_json::Value,
        _lease: &CapabilityLease,
        context: &ThreadExecutionContext,
    ) -> Result<ActionResult, EngineError> {
        // Build a minimal JobContext for tool execution
        let job_ctx = JobContext::with_user(
            &context.user_id,
            "engine_v2",
            format!("Thread {}", context.thread_id),
        );

        // Convert Python identifier (underscores) back to tool name (hyphens).
        // Python can't have hyphens in function names, so the system prompt
        // lists tools with underscores. We need to try both forms.
        let hyphenated = action_name.replace('_', "-");
        let lookup_name = if self.tools.get(action_name).await.is_some() {
            action_name
        } else {
            &hyphenated
        };

        // Execute through the existing tool pipeline
        let result = crate::tools::execute::execute_tool_with_safety(
            &self.tools,
            &self.safety,
            lookup_name,
            &parameters,
            &job_ctx,
        )
        .await;

        match result {
            Ok(output) => {
                // Tool output is a String. If it's valid JSON, parse it so the
                // Python code gets a dict/list instead of a string that needs
                // manual parsing. This prevents double-serialization.
                let output_value = serde_json::from_str::<serde_json::Value>(&output)
                    .unwrap_or(serde_json::Value::String(output));

                Ok(ActionResult {
                    call_id: String::new(), // Caller fills this in
                    action_name: action_name.to_string(),
                    output: output_value,
                    is_error: false,
                    duration: Duration::from_millis(1), // TODO: measure actual duration
                })
            }
            Err(e) => Ok(ActionResult {
                call_id: String::new(),
                action_name: action_name.to_string(),
                output: serde_json::json!({"error": e.to_string()}),
                is_error: true,
                duration: Duration::ZERO,
            }),
        }
    }

    async fn available_actions(
        &self,
        _leases: &[CapabilityLease],
    ) -> Result<Vec<ActionDef>, EngineError> {
        let tool_defs = self.tools.tool_definitions().await;
        Ok(tool_defs
            .into_iter()
            .map(|td| ActionDef {
                // Convert hyphens to underscores for valid Python identifiers
                name: td.name.replace('-', "_"),
                description: td.description,
                parameters_schema: td.parameters,
                effects: vec![], // Effect classification happens at the engine level
                requires_approval: false,
            })
            .collect())
    }
}
