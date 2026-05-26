use async_trait::async_trait;
use std::sync::Arc;

use crate::context::JobContext;
use crate::pairing::PairingStore;
use crate::tools::tool::{ApprovalRequirement, Tool, ToolError, ToolOutput, require_str};

const CHANNEL: &str = "slack-relay";

pub struct PairingApproveTool {
    store: Arc<PairingStore>,
}

impl PairingApproveTool {
    pub fn new(store: Arc<PairingStore>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for PairingApproveTool {
    fn name(&self) -> &str {
        "pairing_approve"
    }

    fn description(&self) -> &str {
        "Approve a Slack pairing code to bind the user's Slack account to their IronClaw user. The user receives the code in Slack and provides it here."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "code": {
                    "type": "string",
                    "description": "The pairing code received in Slack (e.g. WZG8LQAB)"
                }
            },
            "required": ["code"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let code = require_str(&params, "code")?;
        let channel = CHANNEL;

        let user_id =
            crate::ownership::UserId::new(&ctx.user_id, crate::ownership::UserRole::Regular)
                .map_err(|e| ToolError::ExecutionFailed(format!("invalid user_id: {e}")))?;

        match self.store.approve(channel, code, &user_id).await {
            Ok(approval) => {
                let msg = format!(
                    "Pairing approved! Your {} account (external ID: {}) is now linked to your IronClaw user.",
                    approval.channel, approval.external_id
                );
                Ok(ToolOutput::text(&msg, start.elapsed()))
            }
            Err(e) => {
                let msg = format!(
                    "Pairing failed: {e}. Make sure the code is correct and hasn't expired."
                );
                Ok(ToolOutput::text(&msg, start.elapsed()))
            }
        }
    }

    fn requires_sanitization(&self) -> bool {
        false
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        ApprovalRequirement::Always
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::registry::is_protected_tool_name;

    #[test]
    fn tool_metadata() {
        let store = Arc::new(PairingStore::new_noop());
        let tool = PairingApproveTool::new(store);
        assert_eq!(tool.name(), "pairing_approve");
        assert!(tool.description().contains("Slack"));
        let schema = tool.parameters_schema();
        assert!(schema["properties"]["code"].is_object());
        assert!(schema["properties"]["channel"].is_null());
    }

    #[test]
    fn always_requires_approval() {
        let store = Arc::new(PairingStore::new_noop());
        let tool = PairingApproveTool::new(store);
        assert_eq!(
            tool.requires_approval(&serde_json::json!({})),
            ApprovalRequirement::Always
        );
    }

    #[test]
    fn is_protected_builtin() {
        assert!(is_protected_tool_name("pairing_approve"));
    }

    #[test]
    fn channel_is_slack_relay() {
        assert_eq!(CHANNEL, "slack-relay");
    }
}
