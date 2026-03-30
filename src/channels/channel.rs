//! Channel trait and message types.

use std::collections::HashMap;
use std::pin::Pin;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures::Stream;
use uuid::Uuid;

use crate::error::ChannelError;

/// Kind of attachment carried on an incoming message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttachmentKind {
    /// Audio content (voice notes, audio files).
    Audio,
    /// Image content (photos, screenshots).
    Image,
    /// Document content (PDFs, files).
    Document,
}

impl AttachmentKind {
    /// Infer attachment kind from MIME type.
    pub fn from_mime_type(mime: &str) -> Self {
        let base = mime.split(';').next().unwrap_or(mime).trim();
        if base.starts_with("audio/") {
            Self::Audio
        } else if base.starts_with("image/") {
            Self::Image
        } else {
            Self::Document
        }
    }
}

/// A file or media attachment on an incoming message.
#[derive(Debug, Clone)]
pub struct IncomingAttachment {
    /// Unique identifier within the channel (e.g., Telegram file_id).
    pub id: String,
    /// What kind of content this is.
    pub kind: AttachmentKind,
    /// MIME type (e.g., "image/jpeg", "audio/ogg", "application/pdf").
    pub mime_type: String,
    /// Original filename, if known.
    pub filename: Option<String>,
    /// File size in bytes, if known.
    pub size_bytes: Option<u64>,
    /// URL to download the file from the channel's API.
    pub source_url: Option<String>,
    /// Opaque key for host-side storage (e.g., after download/caching).
    pub storage_key: Option<String>,
    /// Extracted text content (e.g., OCR result, PDF text, audio transcript).
    pub extracted_text: Option<String>,
    /// Raw file bytes (for small files downloaded by the channel).
    pub data: Vec<u8>,
    /// Duration in seconds (for audio/video).
    pub duration_secs: Option<u32>,
}

/// Where an incoming message originated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MessageSource {
    /// Normal user-submitted message.
    #[default]
    User,
    /// Generated internally (e.g. job monitor) — bypasses user-input pipeline.
    Internal,
    /// Routine agent-review — enters agentic loop but skips submission parser,
    /// hooks, and event triggers.
    RoutineReview,
}

/// A message received from an external channel.
#[derive(Debug, Clone)]
pub struct IncomingMessage {
    /// Unique message ID.
    pub id: Uuid,
    /// Channel this message came from.
    pub channel: String,
    /// Storage/persistence scope for this interaction.
    ///
    /// For owner-capable channels this is the stable instance owner ID when the
    /// configured owner is speaking; otherwise it can be a guest/sender-scoped
    /// identifier to preserve isolation.
    pub user_id: String,
    /// Stable instance owner scope for this IronClaw deployment.
    pub owner_id: String,
    /// Channel-specific sender/actor identifier.
    pub sender_id: String,
    /// Optional display name.
    pub user_name: Option<String>,
    /// Message content.
    pub content: String,
    /// Thread/conversation ID for threaded conversations.
    pub thread_id: Option<String>,
    /// Stable channel/chat/thread scope for this conversation.
    pub conversation_scope_id: Option<String>,
    /// When the message was received.
    pub received_at: DateTime<Utc>,
    /// Channel-specific metadata.
    pub metadata: serde_json::Value,
    /// IANA timezone string from the client (e.g. "America/New_York").
    pub timezone: Option<String>,
    /// File or media attachments on this message.
    pub attachments: Vec<IncomingAttachment>,
    /// Where this message originated. Controls which pipeline stages run.
    /// Not settable via metadata, so external channels cannot spoof it.
    pub(crate) source: MessageSource,
}

impl IncomingMessage {
    /// Create a new incoming message.
    pub fn new(
        channel: impl Into<String>,
        user_id: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        let user_id = user_id.into();
        Self {
            id: Uuid::new_v4(),
            channel: channel.into(),
            owner_id: user_id.clone(),
            sender_id: user_id.clone(),
            user_id,
            user_name: None,
            content: content.into(),
            thread_id: None,
            conversation_scope_id: None,
            received_at: Utc::now(),
            metadata: serde_json::Value::Null,
            timezone: None,
            attachments: Vec::new(),
            source: MessageSource::User,
        }
    }

    /// Set the thread ID.
    pub fn with_thread(mut self, thread_id: impl Into<String>) -> Self {
        let thread_id = thread_id.into();
        self.conversation_scope_id = Some(thread_id.clone());
        self.thread_id = Some(thread_id);
        self
    }

    /// Set the stable owner scope for this message.
    pub fn with_owner_id(mut self, owner_id: impl Into<String>) -> Self {
        self.owner_id = owner_id.into();
        self
    }

    /// Set the channel-specific sender/actor identifier.
    pub fn with_sender_id(mut self, sender_id: impl Into<String>) -> Self {
        self.sender_id = sender_id.into();
        self
    }

    /// Set the conversation scope for this message.
    pub fn with_conversation_scope(mut self, scope_id: impl Into<String>) -> Self {
        self.conversation_scope_id = Some(scope_id.into());
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

    /// Set the client timezone.
    pub fn with_timezone(mut self, tz: impl Into<String>) -> Self {
        self.timezone = Some(tz.into());
        self
    }

    /// Set attachments.
    pub fn with_attachments(mut self, attachments: Vec<IncomingAttachment>) -> Self {
        self.attachments = attachments;
        self
    }

    /// Mark this message as internal (bypasses user-input pipeline).
    pub(crate) fn into_internal(mut self) -> Self {
        self.source = MessageSource::Internal;
        self
    }

    /// Mark this message as a routine review (enters agentic loop but skips
    /// submission parser, hooks, and event triggers).
    pub(crate) fn into_routine_review(mut self) -> Self {
        self.source = MessageSource::RoutineReview;
        self
    }

    /// Whether this message was generated internally (backward-compat accessor).
    /// Used by job_monitor tests and channel tests; production code matches on `source` directly.
    #[allow(dead_code)]
    pub(crate) fn is_internal(&self) -> bool {
        self.source == MessageSource::Internal
    }

    /// Effective conversation scope, falling back to thread_id for legacy callers.
    pub fn conversation_scope(&self) -> Option<&str> {
        self.conversation_scope_id
            .as_deref()
            .or(self.thread_id.as_deref())
    }

    /// Best-effort routing target for proactive replies on the current channel.
    pub fn routing_target(&self) -> Option<String> {
        routing_target_from_metadata(&self.metadata).or_else(|| {
            if self.sender_id.is_empty() {
                None
            } else {
                Some(self.sender_id.clone())
            }
        })
    }
}

/// Extract a channel-specific proactive routing target from message metadata.
pub fn routing_target_from_metadata(metadata: &serde_json::Value) -> Option<String> {
    metadata
        .get("signal_target")
        .and_then(|value| match value {
            serde_json::Value::String(s) => Some(s.clone()),
            serde_json::Value::Number(n) => Some(n.to_string()),
            _ => None,
        })
        .or_else(|| {
            metadata.get("chat_id").and_then(|value| match value {
                serde_json::Value::String(s) => Some(s.clone()),
                serde_json::Value::Number(n) => Some(n.to_string()),
                _ => None,
            })
        })
        .or_else(|| {
            metadata.get("target").and_then(|value| match value {
                serde_json::Value::String(s) => Some(s.clone()),
                serde_json::Value::Number(n) => Some(n.to_string()),
                _ => None,
            })
        })
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

/// A single tool decision within a reasoning update.
#[derive(Debug, Clone)]
pub struct ToolDecision {
    /// Tool name.
    pub tool_name: String,
    /// Agent's reasoning for choosing this tool.
    pub rationale: String,
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
        /// When `true`, the UI should offer an "always" option that auto-approves
        /// future calls to this tool for the rest of the session.  When `false`
        /// (i.e. `ApprovalRequirement::Always`), the tool must be approved every
        /// time and the "always" button should be hidden.
        allow_always: bool,
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
    /// An image was generated by a tool.
    ImageGenerated {
        /// Base64 data URL of the generated image.
        data_url: String,
        /// Optional workspace path where the image was saved.
        path: Option<String>,
    },
    /// Suggested follow-up messages for the user.
    Suggestions { suggestions: Vec<String> },
    /// Agent reasoning update (why it chose specific tools).
    ReasoningUpdate {
        /// Human-readable summary of the agent's decision.
        narrative: String,
        /// Per-tool decisions.
        decisions: Vec<ToolDecision>,
    },
    /// Per-turn token usage and cost summary (shown as subtle metadata).
    TurnCost {
        input_tokens: u64,
        output_tokens: u64,
        cost_usd: String,
    },
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

/// Trait for channels that support hot-secret-swapping during SIGHUP reload.
///
/// This allows channels to update authentication credentials without restarting,
/// enabling zero-downtime configuration reloads. Channels that don't support
/// secret updates can simply not implement this trait.
#[async_trait]
pub trait ChannelSecretUpdater: Send + Sync {
    /// Update the secret for this channel.
    ///
    /// Called during SIGHUP configuration reload. Implementation should:
    /// - Apply the new secret atomically
    /// - Not fail the entire reload if secret update fails
    /// - Log appropriate errors/info messages
    ///
    /// The secret is optional (may be None if secret is no longer configured).
    async fn update_secret(&self, new_secret: Option<secrecy::SecretString>);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::credentials::TEST_REDACT_SECRET_123;

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
        let params = serde_json::json!({"name": "api_key", "value": TEST_REDACT_SECRET_123});
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
                !param_str.contains(TEST_REDACT_SECRET_123),
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

    #[test]
    fn message_source_default_is_user() {
        let msg = IncomingMessage::new("test", "user1", "hello");
        assert_eq!(msg.source, MessageSource::User);
        assert!(!msg.is_internal());
    }

    #[test]
    fn into_internal_sets_source() {
        let msg = IncomingMessage::new("test", "user1", "hello").into_internal();
        assert_eq!(msg.source, MessageSource::Internal);
        assert!(msg.is_internal());
    }

    #[test]
    fn into_routine_review_sets_source() {
        let msg = IncomingMessage::new("test", "user1", "hello").into_routine_review();
        assert_eq!(msg.source, MessageSource::RoutineReview);
        assert!(!msg.is_internal());
    }

    #[test]
    fn test_incoming_message_with_timezone() {
        let msg = IncomingMessage::new("test", "user1", "hello").with_timezone("America/New_York");
        assert_eq!(msg.timezone.as_deref(), Some("America/New_York"));
    }
}
