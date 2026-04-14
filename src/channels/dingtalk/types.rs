//! DingTalk API types and data structures.

use serde::{Deserialize, Serialize};

// ─── DingTalk Stream Protocol Types ─────────────────────────────────────────

/// A message frame received over the Stream WebSocket.
///
/// Per the DingTalk Stream protocol, `topic` and `messageId` live inside
/// the `headers` map — NOT as top-level fields. Helper methods extract them.
#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct StreamFrame {
    /// "SYSTEM" for control, "CALLBACK" for messages, "EVENT" for events
    #[serde(rename = "type")]
    pub frame_type: Option<String>,
    /// JSON-encoded payload
    pub data: Option<String>,
    /// Protocol headers containing topic, messageId, contentType, time, etc.
    #[serde(default)]
    pub headers: Option<serde_json::Value>,
}

impl StreamFrame {
    pub fn topic(&self) -> Option<&str> {
        self.headers
            .as_ref()
            .and_then(|h| h.get("topic"))
            .and_then(|v| v.as_str())
    }

    pub fn message_id(&self) -> Option<&str> {
        self.headers
            .as_ref()
            .and_then(|h| h.get("messageId"))
            .and_then(|v| v.as_str())
    }
}

/// DingTalk bot callback message payload (inside StreamFrame.data).
#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct BotCallbackPayload {
    pub conversation_id: Option<String>,
    pub conversation_type: Option<String>,
    pub text: Option<TextContent>,
    #[serde(default)]
    pub rich_text: Option<serde_json::Value>,
    pub sender_id: Option<String>,
    pub sender_nick: Option<String>,
    pub sender_staff_id: Option<String>,
    pub msg_id: Option<String>,
    pub msgtype: Option<String>,
    pub robot_code: Option<String>,
    #[serde(default)]
    pub is_in_at_list: Option<bool>,
    /// Session webhook URL (for direct replies within ~1 hour)
    pub session_webhook: Option<String>,
    /// Session webhook expiry timestamp (ms)
    pub session_webhook_expired_time: Option<u64>,
    /// General content blob for non-text message types (audio, video, file, picture, etc.)
    /// DingTalk encodes this as a nested JSON string or object depending on msgtype.
    #[serde(default)]
    pub content: Option<serde_json::Value>,
    /// Whether this message is a reply/quote to another message.
    #[serde(default, rename = "isReplyMsg")]
    pub is_reply_msg: Option<bool>,
    /// The quoted/replied-to message payload (present when `is_reply_msg` is true).
    #[serde(default, rename = "repliedMsg")]
    pub replied_msg: Option<serde_json::Value>,
}

#[derive(Deserialize, Debug)]
pub struct TextContent {
    pub content: Option<String>,
}

// ─── Metadata for Response Routing ──────────────────────────────────────────

/// Stored per incoming message for routing replies back.
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DingTalkMetadata {
    pub conversation_id: String,
    pub conversation_type: String,
    pub sender_staff_id: String,
    pub sender_nick: String,
    pub msg_id: String,
    pub robot_code: Option<String>,
    /// Session webhook URL for direct reply (faster, no auth needed).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_webhook: Option<String>,
    /// Session webhook expiry timestamp in milliseconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_webhook_expired_time: Option<u64>,
}

// ─── AI Card State ─────────────────────────────────────────────────────────

/// Phase of an AI streaming card lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardPhase {
    /// Card created, no content sent yet.
    Processing,
    /// Streaming content in progress.
    Inputing,
}

/// State of an active AI streaming card for a single message.
#[derive(Debug, Clone)]
pub struct CardState {
    /// DingTalk card instance ID returned by createAndDeliver.
    pub instance_id: String,
    /// Accumulated content buffer for the card.
    pub content_buffer: String,
    /// Accumulated thinking/reasoning buffer (for `all` mode).
    pub thinking_buffer: String,
    /// Last time real user-visible content was streamed to the card.
    ///
    /// The initial empty activation stream is tracked separately and must not
    /// consume the first real-content flush budget.
    pub last_content_update: Option<std::time::Instant>,
    /// Current phase of the card.
    pub phase: CardPhase,
    /// When true, stop attempting card delivery and fall back to markdown.
    pub fallback_required: bool,
}

// ─── DingTalk API Types ─────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct MarkdownMsgParam {
    pub title: String,
    pub text: String,
}
