//! Thread messages — the engine's own message type.
//!
//! Simpler than the main crate's `ChatMessage`. Bridge adapters handle
//! conversion between `ThreadMessage` and `ChatMessage`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::types::provenance::Provenance;
use crate::types::step::ActionCall;

/// Role of a message participant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageRole {
    System,
    User,
    Assistant,
    /// Result from a capability action (replaces "Tool" role).
    ActionResult,
}

/// A multimodal content part attached to a thread message.
///
/// This mirrors the small subset of chat-completion content parts that the
/// engine needs without depending on the host crate's LLM provider types.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum MessageContentPart {
    /// Text content part.
    #[serde(rename = "text")]
    Text { text: String },
    /// Image URL content part. Data URLs are supported for inline images.
    #[serde(rename = "image_url")]
    ImageUrl { image_url: MessageImageUrl },
}

/// Image URL reference for multimodal content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageImageUrl {
    /// URL or data: URI, e.g. `data:image/png;base64,...`.
    pub url: String,
    /// Detail level hint: `auto`, `low`, or `high`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// A message in a thread's conversation history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreadMessage {
    pub role: MessageRole,
    pub content: String,
    /// Multimodal parts for the current in-memory LLM request.
    ///
    /// These are deliberately transient: image attachments can be large data
    /// URLs, while the durable record already includes the attachment's saved
    /// project path in `content`.
    #[serde(skip)]
    pub content_parts: Vec<MessageContentPart>,
    pub provenance: Provenance,
    /// For ActionResult messages: the call ID this is responding to.
    pub action_call_id: Option<String>,
    /// For ActionResult messages: the action name.
    pub action_name: Option<String>,
    /// For Assistant messages: actions the LLM wants to execute.
    pub action_calls: Option<Vec<ActionCall>>,
    pub timestamp: DateTime<Utc>,
}

impl ThreadMessage {
    /// Create a system message.
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::System,
            content: content.into(),
            content_parts: Vec::new(),
            provenance: Provenance::System,
            action_call_id: None,
            action_name: None,
            action_calls: None,
            timestamp: Utc::now(),
        }
    }

    /// Create a user message.
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::User,
            content: content.into(),
            content_parts: Vec::new(),
            provenance: Provenance::User,
            action_call_id: None,
            action_name: None,
            action_calls: None,
            timestamp: Utc::now(),
        }
    }

    /// Create a user message with transient multimodal content parts.
    pub fn user_with_content_parts(
        content: impl Into<String>,
        content_parts: Vec<MessageContentPart>,
    ) -> Self {
        Self {
            role: MessageRole::User,
            content: content.into(),
            content_parts,
            provenance: Provenance::User,
            action_call_id: None,
            action_name: None,
            action_calls: None,
            timestamp: Utc::now(),
        }
    }

    /// Create an assistant text message.
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: MessageRole::Assistant,
            content: content.into(),
            content_parts: Vec::new(),
            provenance: Provenance::LlmGenerated,
            action_call_id: None,
            action_name: None,
            action_calls: None,
            timestamp: Utc::now(),
        }
    }

    /// Create an assistant message with action calls.
    pub fn assistant_with_actions(content: Option<String>, calls: Vec<ActionCall>) -> Self {
        Self {
            role: MessageRole::Assistant,
            content: content.unwrap_or_default(),
            content_parts: Vec::new(),
            provenance: Provenance::LlmGenerated,
            action_call_id: None,
            action_name: None,
            action_calls: Some(calls),
            timestamp: Utc::now(),
        }
    }

    /// Create an action result message.
    pub fn action_result(
        call_id: impl Into<String>,
        action_name: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        let name: String = action_name.into();
        Self {
            role: MessageRole::ActionResult,
            content: content.into(),
            content_parts: Vec::new(),
            provenance: Provenance::ToolOutput {
                action_name: name.clone(),
            },
            action_call_id: Some(call_id.into()),
            action_name: Some(name),
            action_calls: None,
            timestamp: Utc::now(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_with_content_parts_sets_transient_parts() {
        let message = ThreadMessage::user_with_content_parts(
            "look",
            vec![MessageContentPart::ImageUrl {
                image_url: MessageImageUrl {
                    url: "data:image/png;base64,abc".to_string(),
                    detail: Some("auto".to_string()),
                },
            }],
        );

        assert_eq!(message.role, MessageRole::User);
        assert_eq!(message.content, "look");
        assert_eq!(message.content_parts.len(), 1);
    }

    #[test]
    fn content_parts_are_not_serialized() {
        let message = ThreadMessage::user_with_content_parts(
            "look",
            vec![MessageContentPart::ImageUrl {
                image_url: MessageImageUrl {
                    url: "data:image/png;base64,abc".to_string(),
                    detail: Some("auto".to_string()),
                },
            }],
        );

        let value = serde_json::to_value(&message).expect("serialize message");
        assert!(value.get("content_parts").is_none());
    }
}
