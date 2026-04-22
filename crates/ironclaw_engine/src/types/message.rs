//! Thread messages — the engine's own message type.
//!
//! Simpler than the main crate's `ChatMessage`. Bridge adapters handle
//! conversion between `ThreadMessage` and `ChatMessage`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::types::provenance::Provenance;
use crate::types::step::{ActionCall, AssistantContent};

/// Strongly-typed message identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MessageId(pub Uuid);

impl MessageId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for MessageId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for MessageId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// Role of a message participant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageRole {
    System,
    User,
    Assistant,
    /// Result from a capability action (replaces "Tool" role).
    ActionResult,
}

/// A message in a thread's conversation history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreadMessage {
    /// Stable identifier — `MessageAdded` events reference messages by this
    /// id so consumers can resolve typed content without re-deriving it from
    /// a truncated preview string.
    #[serde(default)]
    pub id: MessageId,
    pub role: MessageRole,
    pub content: String,
    /// Optional typed assistant-content semantics. This supplements the legacy
    /// `content` string so persisted rows and existing callers remain
    /// backwards-compatible while engine-v2 adapters gain stronger semantics.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assistant_content: Option<AssistantContent>,
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
            id: MessageId::new(),
            role: MessageRole::System,
            content: content.into(),
            assistant_content: None,
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
            id: MessageId::new(),
            role: MessageRole::User,
            content: content.into(),
            assistant_content: None,
            provenance: Provenance::User,
            action_call_id: None,
            action_name: None,
            action_calls: None,
            timestamp: Utc::now(),
        }
    }

    /// Create an assistant text message.
    pub fn assistant(content: impl Into<String>) -> Self {
        Self::assistant_with_typed_content(AssistantContent::Final(content.into()))
    }

    /// Create an assistant message with explicit typed content.
    pub fn assistant_with_typed_content(content: AssistantContent) -> Self {
        let raw_content = content.text().to_string();
        Self {
            id: MessageId::new(),
            role: MessageRole::Assistant,
            content: raw_content,
            assistant_content: Some(content),
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
            id: MessageId::new(),
            role: MessageRole::Assistant,
            content: content.unwrap_or_default(),
            assistant_content: None,
            provenance: Provenance::LlmGenerated,
            action_call_id: None,
            action_name: None,
            action_calls: Some(calls),
            timestamp: Utc::now(),
        }
    }

    /// Create an assistant message with action calls and typed content.
    pub fn assistant_with_action_content(
        content: Option<AssistantContent>,
        calls: Vec<ActionCall>,
    ) -> Self {
        let raw_content = content
            .as_ref()
            .map(|content| content.text().to_string())
            .unwrap_or_default();
        Self {
            id: MessageId::new(),
            role: MessageRole::Assistant,
            content: raw_content,
            assistant_content: content,
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
            id: MessageId::new(),
            role: MessageRole::ActionResult,
            content: content.into(),
            assistant_content: None,
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
