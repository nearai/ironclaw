//! Channel trait and message types.

use std::pin::Pin;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures::Stream;
use uuid::Uuid;

use crate::error::ChannelError;

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
    /// Channel-specific metadata for the response.
    pub metadata: serde_json::Value,
}

impl OutgoingResponse {
    /// Create a simple text response.
    pub fn text(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            thread_id: None,
            metadata: serde_json::Value::Null,
        }
    }

    /// Set the thread ID for the response.
    pub fn in_thread(mut self, thread_id: impl Into<String>) -> Self {
        self.thread_id = Some(thread_id.into());
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
    ToolCompleted { name: String, success: bool },
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

    /// Gracefully shut down the channel.
    async fn shutdown(&self) -> Result<(), ChannelError> {
        Ok(())
    }
}
