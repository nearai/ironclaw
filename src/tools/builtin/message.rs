//! Message tool for sending messages to channels.
//!
//! Allows the agent to proactively message users on any connected channel.

use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use async_trait::async_trait;

use crate::bootstrap::ironclaw_base_dir;
use crate::channels::{ChannelManager, OutgoingResponse};
use crate::context::JobContext;
use crate::extensions::ExtensionManager;
use crate::tools::tool::{
    ApprovalRequirement, Tool, ToolError, ToolOutput, ToolRateLimitConfig, require_str,
};

/// Tool for sending messages to channels.
pub struct MessageTool {
    channel_manager: Arc<ChannelManager>,
    extension_manager: Option<Arc<ExtensionManager>>,
    /// Default channel for current conversation (set per-turn).
    /// Uses std::sync::RwLock because requires_approval() is sync and called from async context.
    default_channel: Arc<RwLock<Option<String>>>,
    /// Default target (user_id or group_id) for current conversation (set per-turn).
    default_target: Arc<RwLock<Option<String>>>,
    /// Base directory for attachment path validation (sandbox).
    pub(crate) base_dir: PathBuf,
}

impl MessageTool {
    pub fn new(channel_manager: Arc<ChannelManager>) -> Self {
        let base_dir = ironclaw_base_dir();

        Self {
            channel_manager,
            extension_manager: None,
            default_channel: Arc::new(RwLock::new(None)),
            default_target: Arc::new(RwLock::new(None)),
            base_dir,
        }
    }

    pub fn with_extension_manager(mut self, extension_manager: Arc<ExtensionManager>) -> Self {
        self.extension_manager = Some(extension_manager);
        self
    }

    /// Set the base directory for attachment validation.
    /// This is primarily used for testing or future configuration.
    pub fn with_base_dir(mut self, dir: PathBuf) -> Self {
        self.base_dir = dir;
        self
    }

    /// Set the default channel and target for the current conversation turn.
    /// Call this before each agent turn with the incoming message's channel/target.
    pub async fn set_context(&self, channel: Option<String>, target: Option<String>) {
        *self
            .default_channel
            .write()
            .unwrap_or_else(|e| e.into_inner()) = channel;
        *self
            .default_target
            .write()
            .unwrap_or_else(|e| e.into_inner()) = target;
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
         Supports file attachments: first download the file with the http tool using save_to \
         (e.g., http GET https://picsum.photos/800/600 save_to=/tmp/photo.jpg), then pass \
         the file path in the attachments array. Images are sent as photos on Telegram. \
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
        ctx: &JobContext,
    ) -> Result<ToolOutput, ToolError> {
        let start = std::time::Instant::now();

        let content = require_str(&params, "content")?;

        let explicit_channel = params
            .get("channel")
            .and_then(|v| v.as_str())
            .map(|value| value.to_string());
        let default_channel = self
            .default_channel
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let metadata_channel = ctx
            .metadata
            .get("notify_channel")
            .and_then(|v| v.as_str())
            .map(|value| value.to_string());

        // Get channel: use param → conversation default → job metadata → None (broadcast all)
        let channel: Option<String> = explicit_channel
            .clone()
            .or_else(|| default_channel.clone())
            .or_else(|| metadata_channel.clone());

        let can_use_default_target = match (explicit_channel.as_deref(), default_channel.as_deref())
        {
            (None, _) => true,
            (Some(explicit), Some(current)) if explicit == current => true,
            _ => false,
        };
        let can_use_metadata_target =
            match (explicit_channel.as_deref(), metadata_channel.as_deref()) {
                (None, _) => true,
                (Some(explicit), Some(current)) if explicit == current => true,
                _ => false,
            };

        // Get target: use param → conversation default → job metadata → owner scope
        // fallback when a specific channel is known.
        let target = if let Some(t) = params.get("target").and_then(|v| v.as_str()) {
            Some(t.to_string())
        } else if can_use_default_target
            && let Some(t) = self
                .default_target
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .clone()
        {
            Some(t)
        } else if can_use_metadata_target
            && let Some(t) = ctx.metadata.get("notify_user").and_then(|v| v.as_str())
        {
            Some(t.to_string())
        } else if channel.is_some() {
            if let Some(channel_name) = channel.as_deref() {
                if let Some(extension_manager) = self.extension_manager.as_ref()
                    && let Some(target) = extension_manager
                        .notification_target_for_channel(channel_name)
                        .await
                {
                    Some(target)
                } else {
                    Some(ctx.user_id.clone())
                }
            } else {
                Some(ctx.user_id.clone())
            }
        } else {
            None
        };

        let Some(target) = target else {
            return Err(ToolError::ExecutionFailed(
                "No target specified and no channel-scoped routing target could be resolved. Provide target parameter."
                    .to_string(),
            ));
        };

        let attachments: Vec<String> = match params.get("attachments") {
            Some(v) => serde_json::from_value(v.clone()).map_err(|e| {
                ToolError::ExecutionFailed(format!("Invalid attachments format: {}", e))
            })?,
            None => Vec::new(),
        };

        let attachment_count = attachments.len();

        // Validate all attachment paths against the sandbox and verify existence.
        // Allow paths under the base_dir (~/.ironclaw) or /tmp/.
        for path in &attachments {
            let tmp_dir = PathBuf::from("/tmp");
            let resolved =
                crate::tools::builtin::path_utils::validate_path(path, Some(&self.base_dir))
                    .or_else(|_| {
                        crate::tools::builtin::path_utils::validate_path(path, Some(&tmp_dir))
                    })
                    .map_err(|e| {
                        ToolError::ExecutionFailed(format!(
                            "Attachment path must be within {} or /tmp/: {}",
                            self.base_dir.display(),
                            e
                        ))
                    })?;
            if !resolved.exists() {
                return Err(ToolError::ExecutionFailed(format!(
                    "Attachment file not found: {}",
                    path
                )));
            }
        }

        let mut response = OutgoingResponse::text(content);
        if !attachments.is_empty() {
            response = response.with_attachments(attachments);
        }

        if let Some(ref channel) = channel {
            // Send to a specific channel
            match self
                .channel_manager
                .broadcast(channel, &target, response)
                .await
            {
                Ok(()) => {
                    tracing::info!(
                        message_sent = true,
                        channel = %channel,
                        target = %target,
                        attachments = attachment_count,
                        "Message sent via message tool"
                    );
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
        } else {
            // No channel specified — broadcast to all channels (routine with notify.channel = None)
            let results = self.channel_manager.broadcast_all(&target, response).await;
            let mut succeeded = Vec::new();
            let mut failed: Vec<&str> = Vec::new();
            for (ch, result) in &results {
                match result {
                    Ok(()) => succeeded.push(ch.as_str()),
                    Err(e) => {
                        tracing::warn!(
                            channel = %ch,
                            target = %target,
                            "broadcast_all: channel failed: {}", e
                        );
                        failed.push(ch.as_str());
                    }
                }
            }
            if succeeded.is_empty() {
                let err_msg = if failed.is_empty() {
                    "No channels connected.".to_string()
                } else {
                    format!("All channels failed: {}", failed.join(", "))
                };
                Err(ToolError::ExecutionFailed(err_msg))
            } else {
                tracing::info!(
                    message_sent = true,
                    channels = ?succeeded,
                    target = %target,
                    attachments = attachment_count,
                    "Message broadcast via message tool"
                );
                let msg = format!(
                    "Broadcast message to {} (target: {})",
                    succeeded.join(", "),
                    target
                );
                Ok(ToolOutput::text(msg, start.elapsed()))
            }
        }
    }

    fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
        // Message tool only delivers to channels the user has configured
        // (TUI, Telegram, Slack, web gateway, etc.) via ChannelManager::broadcast.
        ApprovalRequirement::Never
    }

    fn rate_limit_config(&self) -> Option<ToolRateLimitConfig> {
        Some(ToolRateLimitConfig::new(10, 100))
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
    async fn message_tool_with_attachments_outside_sandbox() {
        let tool = MessageTool::new(Arc::new(ChannelManager::new()));

        // Set context
        tool.set_context(Some("signal".to_string()), Some("+1234567890".to_string()))
            .await;

        // Execute with attachments outside both sandbox (~/.ironclaw) and /tmp/
        let ctx = crate::context::JobContext::new("test", "test description");
        let result = tool
            .execute(
                serde_json::json!({
                    "content": "hello",
                    "attachments": ["/etc/passwd", "/var/log/syslog"]
                }),
                &ctx,
            )
            .await;

        // Should fail due to sandbox rejection (paths outside allowed directories)
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("sandbox") || err.contains("escapes") || err.contains("must be within"),
        );
    }

    #[tokio::test]
    async fn message_tool_with_attachments_inside_sandbox_no_channel() {
        use std::fs;

        let tool = MessageTool::new(Arc::new(ChannelManager::new()));
        tool.set_context(Some("signal".to_string()), Some("+1234567890".to_string()))
            .await;

        // Create temp files inside the sandbox
        let sandbox_dir = &tool.base_dir;
        let temp_dir = tempfile::tempdir_in(sandbox_dir).unwrap();
        let file1 = temp_dir.path().join("file1.txt");
        let file2 = temp_dir.path().join("file2.png");
        fs::write(&file1, "test").unwrap();
        fs::write(&file2, "test").unwrap();

        let ctx = crate::context::JobContext::new("test", "test description");
        let result = tool
            .execute(
                serde_json::json!({
                    "content": "hello",
                    "attachments": [file1.to_string_lossy(), file2.to_string_lossy()]
                }),
                &ctx,
            )
            .await;

        // Path validation passes, but channel broadcast fails (no real channel)
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("channel") || err.contains("Channel"));
    }

    #[tokio::test]
    async fn message_tool_with_attachments_in_tmp_no_channel() {
        use std::fs;

        let tool = MessageTool::new(Arc::new(ChannelManager::new()));
        tool.set_context(Some("telegram".to_string()), Some("12345".to_string()))
            .await;

        // Create temp files under /tmp (allowed as secondary attachment dir)
        let temp_dir = tempfile::tempdir_in("/tmp").unwrap();
        let file1 = temp_dir.path().join("photo.jpg");
        let file2 = temp_dir.path().join("doc.pdf");
        fs::write(&file1, "fake image data").unwrap();
        fs::write(&file2, "fake pdf data").unwrap();

        let ctx = crate::context::JobContext::new("test", "test description");
        let result = tool
            .execute(
                serde_json::json!({
                    "content": "here are the files",
                    "attachments": [file1.to_string_lossy(), file2.to_string_lossy()]
                }),
                &ctx,
            )
            .await;

        // Path validation passes for /tmp paths, fails at channel send (no real channel)
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("channel") || err.contains("Channel"),
            "expected channel error (path validation should pass), got: {}",
            err
        );
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
        use crate::tools::builtin::path_utils::is_path_safe_basic;
        assert!(!is_path_safe_basic("../etc/passwd"));
        assert!(!is_path_safe_basic("foo/../bar"));
        assert!(!is_path_safe_basic("foo/bar/../../secret"));
    }

    #[test]
    fn path_traversal_accepts_normal_paths() {
        use crate::tools::builtin::path_utils::is_path_safe_basic;
        assert!(is_path_safe_basic("/tmp/file.txt"));
        assert!(is_path_safe_basic("documents/report.pdf"));
        assert!(is_path_safe_basic("my-file.png"));
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

        let tool = MessageTool::new(Arc::new(ChannelManager::new()));
        tool.set_context(Some("signal".to_string()), Some("+1234567890".to_string()))
            .await;

        // Create a temp file within the sandbox directory
        let sandbox_dir = &tool.base_dir;
        let temp_dir = tempfile::tempdir_in(sandbox_dir).unwrap();
        let temp_path = temp_dir.path().join("test.txt");
        fs::write(&temp_path, "test content").unwrap();
        let temp_path_str = temp_path.to_string_lossy().to_string();

        let ctx = crate::context::JobContext::new("test", "test description");
        let result = tool
            .execute(
                serde_json::json!({
                    "content": "here's the file",
                    "attachments": [temp_path_str]
                }),
                &ctx,
            )
            .await;

        // Should succeed in path validation (file is in sandbox)
        // but fail on channel broadcast (no actual channel)
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("not found") || err.contains("Failed") || err.contains("broadcast"),
            "Expected channel error, got: {}",
            err
        );
    }

    #[tokio::test]
    async fn message_tool_passes_multiple_attachments_to_broadcast() {
        use std::fs;

        let tool = MessageTool::new(Arc::new(ChannelManager::new()));
        tool.set_context(Some("signal".to_string()), Some("+1234567890".to_string()))
            .await;

        // Create temp files within the sandbox directory
        let sandbox_dir = &tool.base_dir;
        let temp_dir = tempfile::tempdir_in(sandbox_dir).unwrap();
        let temp_path1 = temp_dir.path().join("test1.txt");
        let temp_path2 = temp_dir.path().join("test2.txt");
        fs::write(&temp_path1, "test content 1").unwrap();
        fs::write(&temp_path2, "test content 2").unwrap();
        let path1 = temp_path1.to_string_lossy().to_string();
        let path2 = temp_path2.to_string_lossy().to_string();

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

        // Should succeed in path validation (files are in sandbox)
        // but fail on channel broadcast (no actual channel)
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("not found") || err.contains("Failed") || err.contains("broadcast"),
            "Expected channel error, got: {}",
            err
        );
    }

    #[test]
    fn requires_approval_always_never() {
        // Message tool only sends to user-owned channels, so never needs approval.
        let tool = MessageTool::new(Arc::new(ChannelManager::new()));
        assert_eq!(
            tool.requires_approval(&serde_json::json!({"content": "hello"})),
            ApprovalRequirement::Never,
        );
        assert_eq!(
            tool.requires_approval(&serde_json::json!({"content": "hi", "channel": "telegram"})),
            ApprovalRequirement::Never,
        );
    }

    #[tokio::test]
    async fn message_tool_falls_back_to_job_metadata() {
        // Regression: when no conversation context is set (e.g. routine full-job),
        // the message tool should fall back to notify_channel/notify_user from
        // JobContext metadata instead of returning "No target specified".
        let tool = MessageTool::new(Arc::new(ChannelManager::new()));

        let mut ctx = crate::context::JobContext::new("routine-job", "price alert");
        ctx.metadata = serde_json::json!({
            "notify_channel": "telegram",
            "notify_user": "123456789",
        });

        // No set_context called — simulates a routine full-job worker
        let result = tool
            .execute(serde_json::json!({"content": "NEAR price is $5"}), &ctx)
            .await;

        // Should fail at channel broadcast (no real channel), NOT at
        // "No target specified and no active conversation"
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            !err.contains("No target specified"),
            "Should not get 'No target specified' when metadata has notify_user, got: {}",
            err
        );
        assert!(
            !err.contains("No channel specified"),
            "Should not get 'No channel specified' when metadata has notify_channel, got: {}",
            err
        );
    }

    #[tokio::test]
    async fn message_tool_falls_back_to_ctx_user_when_channel_known() {
        // Regression for owner-scoped notifications: a channel can be known
        // even when the concrete delivery target is omitted, so the message
        // tool should pass ctx.user_id through to the channel layer.
        let tool = MessageTool::new(Arc::new(ChannelManager::new()));

        let mut ctx =
            crate::context::JobContext::with_user("owner-scope", "routine-job", "price alert");
        ctx.metadata = serde_json::json!({
            "notify_channel": "telegram",
        });

        let result = tool
            .execute(serde_json::json!({"content": "NEAR price is $5"}), &ctx)
            .await;

        assert!(result.is_err()); // safety: test-only assertion
        let err = result.unwrap_err().to_string();
        let mentions_missing_target = err.contains("No target specified");
        assert!(!mentions_missing_target); // safety: test-only assertion
        let mentions_missing_channel = err.contains("No channel specified");
        assert!(!mentions_missing_channel); // safety: test-only assertion
    }

    #[tokio::test]
    async fn message_tool_no_metadata_still_errors() {
        // When neither conversation context nor metadata is set, should still
        // return a clear error (target resolution fails).
        let tool = MessageTool::new(Arc::new(ChannelManager::new()));
        let ctx = crate::context::JobContext::new("orphan-job", "no notify config");

        let result = tool
            .execute(serde_json::json!({"content": "hello"}), &ctx)
            .await;

        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("No target specified"),
            "Expected 'No target specified' error, got: {}",
            err
        );
    }

    #[tokio::test]
    async fn message_tool_broadcasts_all_when_no_channel() {
        // Regression: when notify.channel is None but notify_user is set,
        // the message tool should attempt broadcast_all instead of erroring
        // with "No channel specified".
        let tool = MessageTool::new(Arc::new(ChannelManager::new()));

        let mut ctx = crate::context::JobContext::new("routine-job", "price alert");
        ctx.metadata = serde_json::json!({
            "notify_user": "123456789",
        });

        let result = tool
            .execute(serde_json::json!({"content": "NEAR price is $5"}), &ctx)
            .await;

        // Should fail because no channels are registered (empty ChannelManager),
        // NOT because "No channel specified".
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            !err.contains("No channel specified"),
            "Should not get 'No channel specified' when broadcasting, got: {}",
            err
        );
        assert!(
            err.contains("No channels connected") || err.contains("All channels failed"),
            "Expected channel delivery error, got: {}",
            err
        );
    }
}
