//! Message tool for sending messages to channels.
//!
//! Allows the agent to proactively message users on any connected channel.

use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::RwLock;

use crate::channels::{ChannelManager, OutgoingResponse};
use crate::context::JobContext;
use crate::tools::tool::{Tool, ToolError, ToolOutput, require_str};

fn is_path_safe(path: &str) -> bool {
    !path.contains("..")
}

/// Tool for sending messages to channels.
pub struct MessageTool {
    channel_manager: Arc<ChannelManager>,
    /// Default channel for current conversation (set per-turn).
    default_channel: Arc<RwLock<Option<String>>>,
    /// Default target (user_id or group_id) for current conversation (set per-turn).
    default_target: Arc<RwLock<Option<String>>>,
}

impl MessageTool {
    pub fn new(channel_manager: Arc<ChannelManager>) -> Self {
        Self {
            channel_manager,
            default_channel: Arc::new(RwLock::new(None)),
            default_target: Arc::new(RwLock::new(None)),
        }
    }

    /// Set the default channel and target for the current conversation turn.
    /// Call this before each agent turn with the incoming message's channel/target.
    pub async fn set_context(&self, channel: Option<String>, target: Option<String>) {
        *self.default_channel.write().await = channel;
        *self.default_target.write().await = target;
    }
}

#[async_trait]
impl Tool for MessageTool {
    fn name(&self) -> &str {
        "message"
    }

    fn description(&self) -> &str {
        "Send a message to a channel. If channel/target omitted, uses the current conversation's \
         channel and sender/group. Use to proactively message users on any connected channel. \
         - Signal: target accepts E.164 (+1234567890) or group ID \
         - Telegram: target accepts username or chat ID \
         - Slack: target accepts channel (#general) or user ID"
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "content": {
                    "type": "string",
                    "description": "Message text to send"
                },
                "channel": {
                    "type": "string",
                    "description": "Target channel (defaults to current channel if omitted)"
                },
                "target": {
                    "type": "string",
                    "description": "Recipient: E.164 phone, group ID, chat ID (defaults to current sender/group if omitted)"
                },
                "attachments": {
                    "type": "array",
                    "items": {"type": "string"},
                    "description": "Optional file paths to attach to the message"
                }
            },
            "required": ["content"]
        })
    }

    async fn execute(
        &self,
        params: serde_json::Value,
        _ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let content = require_str(&params, "content")?;

        // Get channel: use param or fall back to default
        let channel = if let Some(c) = params.get("channel").and_then(|v| v.as_str()) {
            c.to_string()
        } else {
            self.default_channel.read().await.clone().ok_or_else(|| {
                ToolError::ExecutionFailed(
                    "No channel specified and no active conversation. Provide channel parameter."
                        .to_string(),
                )
            })?
        };

        // Get target: use param or fall back to default
        let target = if let Some(t) = params.get("target").and_then(|v| v.as_str()) {
            t.to_string()
        } else {
            self.default_target.read().await.clone().ok_or_else(|| {
                ToolError::ExecutionFailed(
                    "No target specified and no active conversation. Provide target parameter."
                        .to_string(),
                )
            })?
        };

        let attachments: Vec<String> = params
            .get("attachments")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        for path in &attachments {
            if !is_path_safe(path) {
                return Err(ToolError::ExecutionFailed(format!(
                    "Attachment path contains forbidden sequence: {}",
                    path
                )));
            }
        }

        let mut response = OutgoingResponse::text(content);
        if !attachments.is_empty() {
            response = response.with_attachments(attachments);
        }

        match self
            .channel_manager
            .broadcast(&channel, &target, response)
            .await
        {
            Ok(()) => {
                let msg = format!("Sent message to {}:{}", channel, target);
                Ok(ToolOutput::text(msg, start.elapsed()))
            }
            Err(e) => {
                let available = self.channel_manager.channel_names().await.join(", ");
                let err_msg = if available.is_empty() {
                    format!(
                        "Failed to send to {}:{}: {}. No channels connected.",
                        channel, target, e
                    )
                } else {
                    format!(
                        "Failed to send to {}:{}. Available channels: {}. Error: {}",
                        channel, target, available, e
                    )
                };
                Err(ToolError::ExecutionFailed(err_msg))
            }
        }
    }

    fn requires_sanitization(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_tool_name() {
        let tool = MessageTool::new(Arc::new(ChannelManager::new()));
        assert_eq!(tool.name(), "message");
    }

    #[test]
    fn message_tool_description() {
        let tool = MessageTool::new(Arc::new(ChannelManager::new()));
        assert!(!tool.description().is_empty());
    }

    #[test]
    fn message_tool_schema_has_required_fields() {
        let tool = MessageTool::new(Arc::new(ChannelManager::new()));
        let schema = tool.parameters_schema();

        let params = schema.get("properties").unwrap();
        assert!(params.get("content").is_some());
        assert!(params.get("channel").is_some());
        assert!(params.get("target").is_some());

        // Only content is required - channel and target can be inferred from conversation context
        let required = schema.get("required").unwrap().as_array().unwrap();
        assert!(required.iter().any(|v| v == "content"));
        assert!(!required.iter().any(|v| v == "channel"));
        assert!(!required.iter().any(|v| v == "target"));
    }

    #[test]
    fn message_tool_schema_has_optional_attachments() {
        let tool = MessageTool::new(Arc::new(ChannelManager::new()));
        let schema = tool.parameters_schema();

        let params = schema.get("properties").unwrap();
        assert!(params.get("attachments").is_some());
    }

    #[tokio::test]
    async fn message_tool_set_context_updates_defaults() {
        let tool = MessageTool::new(Arc::new(ChannelManager::new()));

        // Initially no defaults set
        let ctx = crate::context::JobContext::new("test", "test description");
        let result = tool
            .execute(serde_json::json!({"content": "hello"}), &ctx)
            .await;
        assert!(result.is_err()); // Should fail without defaults

        // Set context
        tool.set_context(Some("signal".to_string()), Some("+1234567890".to_string()))
            .await;

        // Now execute should use the defaults (though it will fail because channel doesn't exist)
        let result = tool
            .execute(serde_json::json!({"content": "hello"}), &ctx)
            .await;
        // Will fail because channel doesn't exist, but should attempt to use the defaults
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("signal") || err.contains("No channels connected"));
    }

    #[tokio::test]
    async fn message_tool_explicit_params_override_defaults() {
        let tool = MessageTool::new(Arc::new(ChannelManager::new()));

        // Set defaults
        tool.set_context(Some("signal".to_string()), Some("+1234567890".to_string()))
            .await;

        // Execute with explicit params - should fail but check that it uses explicit params
        let ctx = crate::context::JobContext::new("test", "test description");
        let result = tool
            .execute(
                serde_json::json!({
                    "content": "hello",
                    "channel": "telegram",
                    "target": "@username"
                }),
                &ctx,
            )
            .await;

        // Will fail because channel doesn't exist
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        // Should reference telegram, not signal
        assert!(err.contains("telegram") || err.contains("No channels connected"));
    }

    #[tokio::test]
    async fn message_tool_with_attachments() {
        let tool = MessageTool::new(Arc::new(ChannelManager::new()));

        // Set context
        tool.set_context(Some("signal".to_string()), Some("+1234567890".to_string()))
            .await;

        // Execute with attachments
        let ctx = crate::context::JobContext::new("test", "test description");
        let result = tool
            .execute(
                serde_json::json!({
                    "content": "hello",
                    "attachments": ["/tmp/file1.txt", "/tmp/file2.png"]
                }),
                &ctx,
            )
            .await;

        // Will fail because channel doesn't exist, but that's expected
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn message_tool_requires_content() {
        let tool = MessageTool::new(Arc::new(ChannelManager::new()));

        let ctx = crate::context::JobContext::new("test", "test description");
        let result = tool
            .execute(
                serde_json::json!({
                    "channel": "signal",
                    "target": "+1234567890"
                }),
                &ctx,
            )
            .await;

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("content") || err.contains("required"));
    }

    #[test]
    fn message_tool_does_not_require_sanitization() {
        let tool = MessageTool::new(Arc::new(ChannelManager::new()));
        assert!(!tool.requires_sanitization());
    }

    #[test]
    fn path_traversal_rejects_double_dot() {
        assert!(!is_path_safe("../etc/passwd"));
        assert!(!is_path_safe("foo/../bar"));
        assert!(!is_path_safe("foo/bar/../../secret"));
    }

    #[test]
    fn path_traversal_accepts_normal_paths() {
        assert!(is_path_safe("/tmp/file.txt"));
        assert!(is_path_safe("documents/report.pdf"));
        assert!(is_path_safe("my-file.png"));
    }

    #[tokio::test]
    async fn message_tool_rejects_path_traversal_attachments() {
        let tool = MessageTool::new(Arc::new(ChannelManager::new()));
        tool.set_context(Some("signal".to_string()), Some("+1234567890".to_string()))
            .await;

        let ctx = crate::context::JobContext::new("test", "test description");
        let result = tool
            .execute(
                serde_json::json!({
                    "content": "here's the file",
                    "attachments": ["../../../etc/passwd"]
                }),
                &ctx,
            )
            .await;

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("forbidden") || err.contains(".."));
    }

    #[tokio::test]
    async fn message_tool_passes_attachment_to_broadcast() {
        use std::fs;
        use tempfile::NamedTempFile;

        let tool = MessageTool::new(Arc::new(ChannelManager::new()));
        tool.set_context(Some("signal".to_string()), Some("+1234567890".to_string()))
            .await;

        let temp_file = NamedTempFile::new().unwrap();
        let temp_path = temp_file.path().to_string_lossy().to_string();
        fs::write(&temp_path, "test content").unwrap();

        let ctx = crate::context::JobContext::new("test", "test description");
        let result = tool
            .execute(
                serde_json::json!({
                    "content": "here's the file",
                    "attachments": [temp_path]
                }),
                &ctx,
            )
            .await;

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("not found") || err.contains("Failed"),
            "Expected channel not found error, got: {}",
            err
        );
    }

    #[tokio::test]
    async fn message_tool_passes_multiple_attachments_to_broadcast() {
        use std::fs;
        use tempfile::NamedTempFile;

        let tool = MessageTool::new(Arc::new(ChannelManager::new()));
        tool.set_context(Some("signal".to_string()), Some("+1234567890".to_string()))
            .await;

        let temp_file1 = NamedTempFile::new().unwrap();
        let temp_file2 = NamedTempFile::new().unwrap();
        let path1 = temp_file1.path().to_string_lossy().to_string();
        let path2 = temp_file2.path().to_string_lossy().to_string();
        fs::write(&path1, "test content 1").unwrap();
        fs::write(&path2, "test content 2").unwrap();

        let ctx = crate::context::JobContext::new("test", "test description");
        let result = tool
            .execute(
                serde_json::json!({
                    "content": "files attached",
                    "attachments": [path1, path2]
                }),
                &ctx,
            )
            .await;

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("not found") || err.contains("Failed"),
            "Expected channel not found error, got: {}",
            err
        );
    }
}
