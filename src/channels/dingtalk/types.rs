//! DingTalk API types and data structures.

use serde::{Deserialize, Serialize};

// ─── DingTalk Stream Protocol Types ─────────────────────────────────────────

/// Response from the Stream gateway registration endpoint.
#[derive(Deserialize, Debug)]
pub struct StreamEndpointResponse {
    pub endpoint: Option<String>,
    pub ticket: Option<String>,
}

/// A message frame received over the Stream WebSocket.
#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct StreamFrame {
    /// "SYSTEM" for control, "CALLBACK" for messages
    #[serde(rename = "type")]
    pub frame_type: Option<String>,
    /// Subscription type: "/v1.0/im/bot/messages/get" etc.
    pub topic: Option<String>,
    /// Unique message ID for acknowledgement
    pub message_id: Option<String>,
    /// JSON-encoded payload
    pub data: Option<String>,
    /// Header fields
    pub headers: Option<serde_json::Value>,
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
}

// ─── DingTalk API Types ─────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct MarkdownMsgParam {
    pub title: String,
    pub text: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccessTokenResponse {
    pub access_token: Option<String>,
    pub expires_in: Option<u64>,
}
