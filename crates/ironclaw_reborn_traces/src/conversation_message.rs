use chrono::{DateTime, Utc};
use uuid::Uuid;

/// A single message in a conversation, retained as a Trace Commons compatibility
/// DTO for historical IronClaw trace payloads.
#[derive(Debug, Clone)]
pub struct ConversationMessage {
    pub id: Uuid,
    pub role: String,
    pub content: String,
    pub created_at: DateTime<Utc>,
}
