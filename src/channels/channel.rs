//! Channel trait and message types.

use std::collections::HashMap;
use std::pin::Pin;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures::Stream;
use uuid::Uuid;

use crate::error::ChannelError;
use crate::llm::ImageAttachment;

/// A message received from an external channel.
#[derive(Debug, Clone)]
pub struct IncomingMessage {
    /// Unique message ID.
    pub id: Uuid,
    /// Channel this message came from.
    pub channel: String,
    /// User identifier within the channel.
    pub user_id: String,
    /// Optional display name.
    pub user_name: Option<String>,
    /// Message content.
    pub content: String,
    /// Thread/conversation ID for threaded conversations.
    pub thread_id: Option<String>,
    /// When the message was received.
    pub received_at: DateTime<Utc>,
    /// Channel-specific metadata.
    pub metadata: serde_json::Value,
    /// Images attached to this message.
    pub images: Vec<ImageAttachment>,
}

impl IncomingMessage {
    /// Create a new incoming message.
    pub fn new(
        channel: impl Into<String>,
        user_id: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            channel: channel.into(),
            user_id: user_id.into(),
            user_name: None,
            content: content.into(),
            thread_id: None,
            received_at: Utc::now(),
            metadata: serde_json::Value::Null,
            images: Vec::new(),
        }
    }

    /// Set the thread ID.
    pub fn with_thread(mut self, thread_id: impl Into<String>) -> Self {
        self.thread_id = Some(thread_id.into());
        self
    }

    /// Set metadata.
    pub fn with_metadata(mut self, metadata: serde_json::Value) -> Self {
        self.metadata = metadata;
        self
    }

    /// Set user name.
    pub fn with_user_name(mut self, name: impl Into<String>) -> Self {
        self.user_name = Some(name.into());
        self
    }

    /// Attach image attachments.
    pub fn with_images(mut self, images: Vec<ImageAttachment>) -> Self {
        self.images = images;
        self
    }
}

/// Stream of incoming messages.
pub type MessageStream = Pin<Box<dyn Stream<Item = IncomingMessage> + Send>>;

/// Response to send back to a channel.
#[derive(Debug, Clone)]
pub struct OutgoingResponse {
    /// The content to send.
    pub content: String,
    /// Optional thread ID to reply in.
    pub thread_id: Option<String>,
    /// Optional file paths to attach.
    pub attachments: Vec<String>,
    /// Channel-specific metadata for the response.
    pub metadata: serde_json::Value,
}

impl OutgoingResponse {
    /// Create a simple text response.
    pub fn text(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            thread_id: None,
            attachments: Vec::new(),
            metadata: serde_json::Value::Null,
        }
    }

    /// Set the thread ID for the response.
    pub fn in_thread(mut self, thread_id: impl Into<String>) -> Self {
        self.thread_id = Some(thread_id.into());
        self
    }

    /// Add attachments to the response.
    pub fn with_attachments(mut self, paths: Vec<String>) -> Self {
        self.attachments = paths;
        self
    }
}

/// Status update types for showing agent activity.
#[derive(Debug, Clone)]
pub enum StatusUpdate {
    /// Agent is thinking/processing.
    Thinking(String),
    /// Tool execution started.
    ToolStarted { name: String },
    /// Tool execution completed.
    ///
    /// Use [`StatusUpdate::tool_completed`] to construct this variant — it
    /// handles redaction of sensitive parameters and keeps the 9-line pattern
    /// in one place.
    ToolCompleted {
        name: String,
        success: bool,
        /// Error message when success is false.
        error: Option<String>,
        /// Tool input parameters (JSON string) for display on failure.
        /// Only populated when `success` is `false`. Values listed in the
        /// tool's `sensitive_params()` are replaced with `"[REDACTED]"`.
        parameters: Option<String>,
    },
    /// Brief preview of tool execution output.
    ToolResult { name: String, preview: String },
    /// Streaming text chunk.
    StreamChunk(String),
    /// General status message.
    Status(String),
    /// A sandbox job has started (shown as a clickable card in the UI).
    JobStarted {
        job_id: String,
        title: String,
        browse_url: String,
    },
    /// Tool requires user approval before execution.
    ApprovalNeeded {
        request_id: String,
        tool_name: String,
        description: String,
        parameters: serde_json::Value,
    },
    /// Extension needs user authentication (token or OAuth).
    AuthRequired {
        extension_name: String,
        instructions: Option<String>,
        auth_url: Option<String>,
        setup_url: Option<String>,
    },
    /// Extension authentication completed.
    AuthCompleted {
        extension_name: String,
        success: bool,
        message: String,
    },
    /// An image was generated or edited by a tool.
    ImageGenerated { data_url: String, path: String },
}

impl StatusUpdate {
    /// Build a `ToolCompleted` status with redacted parameters.
    ///
    /// On failure, serializes the tool's input parameters as pretty JSON after
    /// replacing any keys listed in the tool's `sensitive_params()` with
    /// `"[REDACTED]"`. On success, no parameters or error are included.
    ///
    /// Pass the resolved `Tool` reference (if available) so this method can
    /// query `sensitive_params()` directly — callers don't need to manage the
    /// borrow lifetime of the sensitive slice.
    pub fn tool_completed(
        name: String,
        result: &Result<String, crate::error::Error>,
        params: &serde_json::Value,
        tool: Option<&dyn crate::tools::Tool>,
    ) -> Self {
        let success = result.is_ok();
        let sensitive = tool.map(|t| t.sensitive_params()).unwrap_or(&[]);
        Self::ToolCompleted {
            name,
            success,
            error: result.as_ref().err().map(|e| e.to_string()),
            parameters: if !success {
                let safe = crate::tools::redact_params(params, sensitive);
                Some(serde_json::to_string_pretty(&safe).unwrap_or_else(|_| safe.to_string()))
            } else {
                None
            },
        }
    }
}

/// Trait for message channels.
///
/// Channels receive messages from external sources and convert them to
/// a unified format. They also handle sending responses back.
#[async_trait]
pub trait Channel: Send + Sync {
    /// Get the channel name (e.g., "cli", "slack", "telegram", "http").
    fn name(&self) -> &str;

    /// Start listening for messages.
    ///
    /// Returns a stream of incoming messages. The channel should handle
    /// reconnection and error recovery internally.
    async fn start(&self) -> Result<MessageStream, ChannelError>;

    /// Send a response back to the user.
    ///
    /// The response is sent in the context of the original message
    /// (same channel, same thread if applicable).
    async fn respond(
        &self,
        msg: &IncomingMessage,
        response: OutgoingResponse,
    ) -> Result<(), ChannelError>;

    /// Send a status update (thinking, tool execution, etc.).
    ///
    /// The metadata contains channel-specific routing info (e.g., Telegram chat_id)
    /// needed to deliver the status to the correct destination.
    ///
    /// Default implementation does nothing (for channels that don't support status).
    async fn send_status(
        &self,
        _status: StatusUpdate,
        _metadata: &serde_json::Value,
    ) -> Result<(), ChannelError> {
        Ok(())
    }

    /// Send a proactive message without a prior incoming message.
    ///
    /// Used for alerts, heartbeat notifications, and other agent-initiated communication.
    /// The user_id helps target a specific user within the channel.
    ///
    /// Default implementation does nothing (for channels that don't support broadcast).
    async fn broadcast(
        &self,
        _user_id: &str,
        _response: OutgoingResponse,
    ) -> Result<(), ChannelError> {
        Ok(())
    }

    /// Check if the channel is healthy.
    async fn health_check(&self) -> Result<(), ChannelError>;

    /// Get conversation context from message metadata for system prompt.
    ///
    /// Returns key-value pairs like "sender", "sender_uuid", "group" that
    /// help the LLM understand who it's talking to.
    ///
    /// Default implementation returns empty map.
    fn conversation_context(&self, _metadata: &serde_json::Value) -> HashMap<String, String> {
        HashMap::new()
    }

    /// Gracefully shut down the channel.
    async fn shutdown(&self) -> Result<(), ChannelError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Stub tool that marks `"value"` as sensitive.
    struct SecretTool;

    #[async_trait]
    impl crate::tools::Tool for SecretTool {
        fn name(&self) -> &str {
            "secret_save"
        }
        fn description(&self) -> &str {
            "stub"
        }
        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object", "properties": {}})
        }
        async fn execute(
            &self,
            _params: serde_json::Value,
            _ctx: &crate::context::JobContext,
        ) -> Result<crate::tools::ToolOutput, crate::tools::ToolError> {
            unreachable!()
        }
        fn sensitive_params(&self) -> &[&str] {
            &["value"]
        }
    }

    #[test]
    fn tool_completed_redacts_sensitive_params_on_failure() {
        let params = serde_json::json!({"name": "api_key", "value": "sk-secret-123"});
        let err: Result<String, crate::error::Error> =
            Err(crate::error::ToolError::ExecutionFailed {
                name: "secret_save".into(),
                reason: "db error".into(),
            }
            .into());
        let tool = SecretTool;

        let status = StatusUpdate::tool_completed(
            "secret_save".into(),
            &err,
            &params,
            Some(&tool as &dyn crate::tools::Tool),
        );

        if let StatusUpdate::ToolCompleted {
            success,
            error,
            parameters,
            ..
        } = &status
        {
            assert!(!success);
            let err_msg = error.as_deref().expect("should have error");
            assert!(err_msg.contains("db error"), "error: {}", err_msg);
            let param_str = parameters
                .as_ref()
                .expect("should have parameters on failure");
            assert!(
                param_str.contains("[REDACTED]"),
                "sensitive value should be redacted: {}",
                param_str
            );
            assert!(
                !param_str.contains("sk-secret-123"),
                "raw secret should not appear: {}",
                param_str
            );
            assert!(
                param_str.contains("api_key"),
                "non-sensitive params should be preserved: {}",
                param_str
            );
        } else {
            panic!("expected ToolCompleted variant");
        }
    }

    #[test]
    fn tool_completed_no_params_on_success() {
        let params = serde_json::json!({"name": "key", "value": "secret"});
        let ok: Result<String, crate::error::Error> = Ok("done".into());

        let status = StatusUpdate::tool_completed("secret_save".into(), &ok, &params, None);

        if let StatusUpdate::ToolCompleted {
            success,
            error,
            parameters,
            ..
        } = &status
        {
            assert!(success);
            assert!(error.is_none());
            assert!(parameters.is_none(), "no params should be sent on success");
        } else {
            panic!("expected ToolCompleted variant");
        }
    }

    #[test]
    fn tool_completed_no_tool_passes_params_unredacted() {
        let params = serde_json::json!({"cmd": "ls -la"});
        let err: Result<String, crate::error::Error> =
            Err(crate::error::ToolError::ExecutionFailed {
                name: "shell".into(),
                reason: "timeout".into(),
            }
            .into());

        let status = StatusUpdate::tool_completed("shell".into(), &err, &params, None);

        if let StatusUpdate::ToolCompleted { parameters, .. } = &status {
            let param_str = parameters.as_ref().expect("should have parameters");
            assert!(
                param_str.contains("ls -la"),
                "non-sensitive params should pass through: {}",
                param_str
            );
        } else {
            panic!("expected ToolCompleted variant");
        }
    }
}
