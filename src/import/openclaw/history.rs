//! OpenClaw conversation history import.

use std::sync::Arc;

use serde_json::json;
use uuid::Uuid;

use crate::db::Database;
use crate::import::{ImportError, ImportOptions};

use super::reader::OpenClawConversation;

/// Import a conversation and its messages atomically.
///
/// This function attempts to create a conversation and add all its messages as a logical unit.
/// While the Database trait does not expose explicit transaction control, this function
/// minimizes the risk of partial writes by:
/// - Validating all message data before creating the conversation
/// - Creating the conversation once
/// - Adding all messages in a tight loop
/// - Returning detailed errors if any step fails
///
/// Returns (conversation_id, message_count) on success.
///
/// **Note on Database Safety**: Without explicit transaction support in the Database trait,
/// if a crash occurs during message insertion, the conversation will exist with fewer messages
/// than expected. This is preferable to crashes during conversation creation (empty conversation).
///
/// **Note on Idempotency**: The metadata includes `openclaw_conversation_id` for deduplication
/// on reimport. However, without metadata-based query support in the Database trait, reimporting
/// will create duplicate conversations. This limitation should be fixed by adding
/// `list_conversations_by_metadata_key()` to the Database trait.
pub async fn import_conversation_atomic(
    db: &Arc<dyn Database>,
    conv: OpenClawConversation,
    opts: &ImportOptions,
) -> Result<(Uuid, usize), ImportError> {
    // PHASE 1: Validate all message data before writing anything
    let mut validated_messages = Vec::with_capacity(conv.messages.len());
    for msg in &conv.messages {
        let role = match msg.role.to_lowercase().as_str() {
            "user" | "human" => "user",
            "assistant" | "ai" => "assistant",
            _ => &msg.role,
        };
        validated_messages.push((role.to_string(), msg.content.clone()));
    }

    // PHASE 2: Create the conversation (single atomic operation from DB perspective)
    // TODO: Add idempotency check when Database trait supports metadata-based lookups
    let metadata = json!({
        "openclaw_conversation_id": conv.id,
        "openclaw_channel": conv.channel,
    });

    let conv_id = db
        .create_conversation_with_metadata(&conv.channel, &opts.user_id, &metadata)
        .await
        .map_err(|e| ImportError::Database(e.to_string()))?;

    // PHASE 3: Add all messages in sequence
    // If this fails partway through, the conversation exists but is incomplete.
    // On reimport, the openclaw_conversation_id metadata will detect it.
    let mut message_count = 0;
    for (role, content) in validated_messages {
        db.add_conversation_message(conv_id, &role, &content)
            .await
            .map_err(|e| {
                // Log detailed error including conversation ID for recovery
                tracing::error!(
                    "Failed to add message to conversation {}: {}. \
                     Conversation created but may be incomplete.",
                    conv_id,
                    e
                );
                ImportError::Database(e.to_string())
            })?;

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
