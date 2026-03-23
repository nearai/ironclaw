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

/// A message in a thread's conversation history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreadMessage {
    pub role: MessageRole,
    pub content: String,
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
            provenance: Provenance::LlmGenerated,
            action_call_id: None,
            action_name: None,
            action_calls: None,
            timestamp: Utc::now(),
        }
    }

    /// Create an assistant message with action calls.
    pub fn assistant_with_actions(
        content: Option<String>,
        calls: Vec<ActionCall>,
    ) -> Self {
        Self {
            role: MessageRole::Assistant,
            content: content.unwrap_or_default(),
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
