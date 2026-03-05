//! Memory management for job contexts.

use std::time::Duration;

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::llm::ChatMessage;

/// A record of an action taken during job execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionRecord {
    /// Unique action ID.
    pub id: Uuid,
    /// Sequence number within the job.
    pub sequence: u32,
    /// Tool that was used.
    pub tool_name: String,
    /// Input parameters.
    pub input: serde_json::Value,
    /// Raw output (before sanitization).
    pub output_raw: Option<String>,
    /// Sanitized output.
    pub output_sanitized: Option<serde_json::Value>,
    /// Any sanitization warnings.
    pub sanitization_warnings: Vec<String>,
    /// Cost of the action.
    pub cost: Option<Decimal>,
    /// Duration of the action.
    pub duration: Duration,
    /// Whether the action succeeded.
    pub success: bool,
    /// Error message if failed.
    pub error: Option<String>,
    /// When the action was executed.
    pub executed_at: DateTime<Utc>,
}

impl ActionRecord {
    /// Create a new action record.
    pub fn new(sequence: u32, tool_name: impl Into<String>, input: serde_json::Value) -> Self {
        Self {
            id: Uuid::new_v4(),
            sequence,
            tool_name: tool_name.into(),
            input,
            output_raw: None,
            output_sanitized: None,
            sanitization_warnings: Vec::new(),
            cost: None,
            duration: Duration::ZERO,
            success: false,
            error: None,
            executed_at: Utc::now(),
        }
    }

    /// Mark the action as successful.
    ///
    /// `output_sanitized` is the tool output after safety processing (string).
    /// `output_raw` is the original tool result (JSON value).
    pub fn succeed(
        mut self,
        output_sanitized: Option<String>,
        output_raw: serde_json::Value,
        duration: Duration,
    ) -> Self {
        self.success = true;
        self.output_raw = Some(serde_json::to_string_pretty(&output_raw).unwrap_or_default());
        self.output_sanitized = output_sanitized.map(serde_json::Value::String);
        self.duration = duration;
        self
    }

    /// Mark the action as failed.
    pub fn fail(mut self, error: impl Into<String>, duration: Duration) -> Self {
        self.success = false;
        self.error = Some(error.into());
        self.duration = duration;
        self
    }

    /// Add sanitization warnings.
    pub fn with_warnings(mut self, warnings: Vec<String>) -> Self {
        self.sanitization_warnings = warnings;
        self
    }

    /// Set the cost.
    pub fn with_cost(mut self, cost: Decimal) -> Self {
        self.cost = Some(cost);
        self
    }
}

/// Conversation history.
#[derive(Debug, Clone, Default)]
pub struct ConversationMemory {
    /// Messages in the conversation.
    messages: Vec<ChatMessage>,
    /// Maximum messages to keep.
    max_messages: usize,
}

impl ConversationMemory {
    /// Create a new conversation memory.
    pub fn new(max_messages: usize) -> Self {
        Self {
            messages: Vec::new(),
            max_messages,
        }
    }

    /// Add a message.
    pub fn add(&mut self, message: ChatMessage) {
        self.messages.push(message);

        // Trim old messages if needed (keeping system message if present)
        while self.messages.len() > self.max_messages {
            // Don't remove system messages
            if self.messages.first().map(|m| m.role) == Some(crate::llm::Role::System) {
                if self.messages.len() > 1 {
                    self.messages.remove(1);
                } else {
                    break;
                }
            } else {
                self.messages.remove(0);
            }
        }
    }

    /// Get all messages.
    pub fn messages(&self) -> &[ChatMessage] {
        &self.messages
    }

    /// Get the last N messages.
    pub fn last_n(&self, n: usize) -> &[ChatMessage] {
        let start = self.messages.len().saturating_sub(n);
        &self.messages[start..]
    }

    /// Clear the conversation.
    pub fn clear(&mut self) {
        self.messages.clear();
    }

    /// Get message count.
    pub fn len(&self) -> usize {
        self.messages.len()
    }

    /// Check if empty.
    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }
}

/// Combined memory for a job.
#[derive(Debug, Clone)]
pub struct Memory {
    /// Job ID.
    pub job_id: Uuid,
    /// Conversation history.
    pub conversation: ConversationMemory,
    /// Action history.
    pub actions: Vec<ActionRecord>,
    /// Next action sequence number.
    next_sequence: u32,
}

impl Memory {
    /// Create a new memory instance.
    pub fn new(job_id: Uuid) -> Self {
        Self {
            job_id,
            conversation: ConversationMemory::new(100),
            actions: Vec::new(),
            next_sequence: 0,
        }
    }

    /// Add a conversation message.
    pub fn add_message(&mut self, message: ChatMessage) {
        self.conversation.add(message);
    }

    /// Create a new action record.
    pub fn create_action(
        &mut self,
        tool_name: impl Into<String>,
        input: serde_json::Value,
    ) -> ActionRecord {
        let seq = self.next_sequence;
        self.next_sequence += 1;
        ActionRecord::new(seq, tool_name, input)
    }

    /// Record a completed action.
    pub fn record_action(&mut self, action: ActionRecord) {
        self.actions.push(action);
    }

    /// Get total cost of all actions.
    pub fn total_cost(&self) -> Decimal {
        self.actions
            .iter()
            .filter_map(|a| a.cost)
            .fold(Decimal::ZERO, |acc, c| acc + c)
    }

    /// Get total duration of all actions.
    pub fn total_duration(&self) -> Duration {
        self.actions
            .iter()
            .map(|a| a.duration)
            .fold(Duration::ZERO, |acc, d| acc + d)
    }

    /// Get successful action count.
    pub fn successful_actions(&self) -> usize {
        self.actions.iter().filter(|a| a.success).count()
    }

    /// Get failed action count.
    pub fn failed_actions(&self) -> usize {
        self.actions.iter().filter(|a| !a.success).count()
    }

    /// Get the last action.
    pub fn last_action(&self) -> Option<&ActionRecord> {
        self.actions.last()
    }

    /// Get actions by tool name.
    pub fn actions_by_tool(&self, tool_name: &str) -> Vec<&ActionRecord> {
        self.actions
            .iter()
            .filter(|a| a.tool_name == tool_name)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_action_record() {
        let action = ActionRecord::new(0, "test", serde_json::json!({"key": "value"}));
        assert_eq!(action.sequence, 0);
        assert!(!action.success);

        let action = action.succeed(
            Some("raw".to_string()),
            serde_json::json!({"result": "ok"}),
            Duration::from_millis(100),
        );
        assert!(action.success);
    }

    #[test]
    fn test_conversation_memory() {
        let mut memory = ConversationMemory::new(3);
        memory.add(ChatMessage::user("Hello"));
        memory.add(ChatMessage::assistant("Hi"));
        memory.add(ChatMessage::user("How are you?"));
        memory.add(ChatMessage::assistant("Good!"));

        assert_eq!(memory.len(), 3); // Oldest removed
    }

    #[test]
    fn test_memory_totals() {
        let mut memory = Memory::new(Uuid::new_v4());

        let action1 = memory
            .create_action("tool1", serde_json::json!({}))
            .succeed(None, serde_json::json!({}), Duration::from_secs(1))
            .with_cost(Decimal::new(10, 1));
        memory.record_action(action1);

        let action2 = memory
            .create_action("tool2", serde_json::json!({}))
            .succeed(None, serde_json::json!({}), Duration::from_secs(2))
            .with_cost(Decimal::new(20, 1));
        memory.record_action(action2);

        assert_eq!(memory.total_cost(), Decimal::new(30, 1));
        assert_eq!(memory.total_duration(), Duration::from_secs(3));
        assert_eq!(memory.successful_actions(), 2);
    }
}
