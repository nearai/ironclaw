//! OpenClaw conversation history import.

use std::sync::Arc;

use serde_json::json;
use uuid::Uuid;

use crate::db::Database;
use crate::import::{ImportError, ImportOptions};

use super::reader::OpenClawConversation;

/// Import a conversation and its messages into IronClaw.
///
/// Returns (conversation_id, message_count) on success.
pub async fn import_conversation(
    db: &Arc<dyn Database>,
    conv: OpenClawConversation,
    opts: &ImportOptions,
) -> Result<(Uuid, usize), ImportError> {
    // Create conversation with metadata that includes original OpenClaw ID for deduplication
    let metadata = json!({
        "openclaw_conversation_id": conv.id,
        "openclaw_channel": conv.channel,
    });

    let conv_id = db
        .create_conversation_with_metadata(&conv.channel, &opts.user_id, &metadata)
        .await
        .map_err(|e| ImportError::Database(e.to_string()))?;

    // Add messages
    let mut message_count = 0;
    for msg in &conv.messages {
        let role = match msg.role.to_lowercase().as_str() {
            "user" | "human" => "user",
            "assistant" | "ai" => "assistant",
            _ => &msg.role,
        };

        db.add_conversation_message(conv_id, role, &msg.content)
            .await
            .map_err(|e| ImportError::Database(e.to_string()))?;

        message_count += 1;
    }

    Ok((conv_id, message_count))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import::openclaw::reader::OpenClawMessage;

    #[test]
    fn test_conversation_import_structure() {
        // Verify that OpenClawConversation can be created with test data
        let conv = OpenClawConversation {
            id: "conv-123".to_string(),
            channel: "telegram".to_string(),
            created_at: None,
            messages: vec![
                OpenClawMessage {
                    role: "user".to_string(),
                    content: "Hello".to_string(),
                    created_at: None,
                },
                OpenClawMessage {
                    role: "assistant".to_string(),
                    content: "Hi there".to_string(),
                    created_at: None,
                },
            ],
        };

        assert_eq!(conv.id, "conv-123");
        assert_eq!(conv.messages.len(), 2);
        assert_eq!(conv.channel, "telegram");
    }
}
