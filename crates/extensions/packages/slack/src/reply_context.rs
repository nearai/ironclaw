//! The Slack reply context: what ingress stores beside every normalized
//! message so the reply sink can address Slack's native Agent surface later.
//!
//! `NormalizedInboundMessage::reply_context` is opaque to the host (≤ 4 KiB,
//! stored server-side, handed back at reply time as
//! `ReplyReconcileRequest::reply_context`). Slack needs it because streaming
//! into a channel requires the recipient's user and team ids —
//! `chat.startStream` documents `recipient_user_id` and `recipient_team_id`
//! as "Required when streaming to channels" — and the conversation ref the
//! host keeps carries neither. Every field is copied from refs that were
//! already validated at ingress (≤ 512 bytes each), so the serialized
//! context stays far under the host bound by construction.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlackReplyContext {
    /// The workspace (`team_id`) the message came from, when the payload
    /// carried it. Streaming into a channel needs it as `recipient_team_id`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team_id: Option<String>,
    /// The Slack conversation id (`C…`, `G…`, `D…`).
    pub channel: String,
    /// The thread the reply belongs to. A top-level direct message roots the
    /// reply on its own `ts`: an Agent session is thread-based in DMs too
    /// (`agents.sessions.setStatus`: `thread_ts` is "Required for
    /// thread-based sessions in regular channels and DMs").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_ts: Option<String>,
    /// The Slack member id of the person the reply answers.
    pub user: String,
    /// Whether the conversation is a direct message with the bot.
    pub is_dm: bool,
}

impl SlackReplyContext {
    pub fn to_bytes(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(self)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reply_context_round_trips_and_stays_under_the_host_bound() {
        let context = SlackReplyContext {
            team_id: Some("T".repeat(512)),
            channel: "C".repeat(512),
            thread_ts: Some("1".repeat(512)),
            user: "U".repeat(512),
            is_dm: false,
        };
        let bytes = context.to_bytes().expect("serializes");
        assert!(
            bytes.len() <= ironclaw_extension_contracts::channel_adapter::MAX_REPLY_CONTEXT_BYTES,
            "worst-case validated refs must fit the 4 KiB reply-context bound"
        );
        assert_eq!(
            SlackReplyContext::from_bytes(&bytes).expect("parses"),
            context
        );
    }

    #[test]
    fn optional_fields_are_omitted_on_the_wire() {
        let context = SlackReplyContext {
            team_id: None,
            channel: "D123".to_string(),
            thread_ts: None,
            user: "U123".to_string(),
            is_dm: true,
        };
        let json: serde_json::Value =
            serde_json::from_slice(&context.to_bytes().expect("serializes")).expect("json");
        assert_eq!(
            json,
            serde_json::json!({ "channel": "D123", "user": "U123", "is_dm": true })
        );
    }
}
