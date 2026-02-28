use async_trait::async_trait;

use crate::context::JobContext;
use crate::tools::tool::{
    ApprovalRequirement, Tool, ToolError, ToolOutput, ToolRateLimitConfig, require_str,
};

/// Tool for executing local platform hooks securely (Zero-Trust Prompted)
pub struct PlatformHooksTool {
}

impl PlatformHooksTool {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl Tool for PlatformHooksTool {
    fn name(&self) -> &str {
        "platform_hooks"
    }

    fn description(&self) -> &str {
        "Execute local system scripts or Apple Shortcuts safely. Used to mimic OpenClaw \
         platform integrations outside the WASM sandbox. Requires explicit user approval."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "hook_type": {
                    "type": "string",
                    "enum": ["applescript", "shell", "shortcut"],
                    "description": "The type of hook to execute"
                },
                "script_content": {
                    "type": "string",
                    "description": "The content of the script or the name of the shortcut"
                }
            },
            "required": ["hook_type", "script_content"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let hook_type = require_str(&params, "hook_type")?;
        let script_content = require_str(&params, "script_content")?;

        let start = std::time::Instant::now();

        // Placeholder for executing platform hooks (e.g. osascript, sh)
        
        tracing::info!(
            hook_type = %hook_type,
            "Platform Hook triggered"
        );

        let output = format!(
            "Successfully executed platform hook of type {}. \
            (Note: execution placeholder triggered)",
            hook_type
        );

        Ok(ToolOutput::text(output, start.elapsed()))
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        // Must ALWAYS require approval for running native host scripts
        ApprovalRequirement::Always
    }

    fn rate_limit_config(&self) -> Option<ToolRateLimitConfig> {
        Some(ToolRateLimitConfig::new(5, 50))
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}
