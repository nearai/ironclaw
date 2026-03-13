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
        assert_eq!(action.sequence, 0); // safety: test
        assert!(!action.success); // safety: test

        let action = action.succeed(
            Some("raw".to_string()),
            serde_json::json!({"result": "ok"}),
            Duration::from_millis(100),
        );
        assert!(action.success); // safety: test
    }

    #[test]
    fn test_conversation_memory() {
        let mut memory = ConversationMemory::new(3);
        memory.add(ChatMessage::user("Hello"));
        memory.add(ChatMessage::assistant("Hi"));
        memory.add(ChatMessage::user("How are you?"));
        memory.add(ChatMessage::assistant("Good!"));

        assert_eq!(memory.len(), 3); // Oldest removed // safety: test
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

        assert_eq!(memory.total_cost(), Decimal::new(30, 1)); // safety: test
        assert_eq!(memory.total_duration(), Duration::from_secs(3)); // safety: test
        assert_eq!(memory.successful_actions(), 2); // safety: test
    }

    #[test]
    fn test_action_record_fail() {
        let action = ActionRecord::new(1, "broken_tool", serde_json::json!({"x": 1}));
        let action = action.fail("something went wrong", Duration::from_millis(50));

        assert!(!action.success); // safety: test
        assert_eq!(action.error.as_deref(), Some("something went wrong")); // safety: test
        assert_eq!(action.duration, Duration::from_millis(50)); // safety: test
        assert!(action.output_raw.is_none()); // safety: test
        assert!(action.output_sanitized.is_none()); // safety: test
    }

    #[test]
    fn test_action_record_with_warnings() {
        let action = ActionRecord::new(0, "risky_tool", serde_json::json!({}));
        let action = action.with_warnings(vec!["suspicious pattern".into(), "possible xss".into()]);

        assert_eq!(action.sanitization_warnings.len(), 2); // safety: test
        assert_eq!(action.sanitization_warnings[0], "suspicious pattern"); // safety: test
        assert_eq!(action.sanitization_warnings[1], "possible xss"); // safety: test
    }

    #[test]
    fn test_action_record_with_cost() {
        let action = ActionRecord::new(0, "expensive_tool", serde_json::json!({}));
        let cost = Decimal::new(42, 2); // 0.42
        let action = action.with_cost(cost);

        assert_eq!(action.cost, Some(Decimal::new(42, 2))); // safety: test
    }

    #[test]
    fn test_action_record_new_defaults() {
        let action = ActionRecord::new(5, "my_tool", serde_json::json!({"key": "val"}));

        assert_eq!(action.sequence, 5); // safety: test
        assert_eq!(action.tool_name, "my_tool"); // safety: test
        assert_eq!(action.input, serde_json::json!({"key": "val"})); // safety: test
        assert!(!action.success); // safety: test
        assert!(action.output_raw.is_none()); // safety: test
        assert!(action.output_sanitized.is_none()); // safety: test
        assert!(action.sanitization_warnings.is_empty()); // safety: test
        assert!(action.cost.is_none()); // safety: test
        assert_eq!(action.duration, Duration::ZERO); // safety: test
        assert!(action.error.is_none()); // safety: test
    }

    #[test]
    fn test_action_record_succeed_sets_fields() {
        let action = ActionRecord::new(0, "tool", serde_json::json!({}));
        let action = action.succeed(
            Some("sanitized output".into()),
            serde_json::json!({"clean": true}),
            Duration::from_secs(7),
        );

        assert!(action.success); // safety: test
        // output_raw is the JSON value pretty-printed
        let expected_raw =
            serde_json::to_string_pretty(&serde_json::json!({"clean": true})).unwrap(); // safety: test
        assert_eq!(action.output_raw.as_deref(), Some(expected_raw.as_str())); // safety: test
        // output_sanitized wraps the string in a JSON string value
        assert_eq!(
            /* safety: test */
            action.output_sanitized,
            Some(serde_json::json!("sanitized output"))
        );
        assert_eq!(action.duration, Duration::from_secs(7)); // safety: test
    }

    #[test]
    fn test_conversation_memory_clear() {
        let mut mem = ConversationMemory::new(10);
        mem.add(ChatMessage::user("hello"));
        mem.add(ChatMessage::assistant("hi"));
        assert_eq!(mem.len(), 2); // safety: test
        assert!(!mem.is_empty()); // safety: test

        mem.clear();
        assert_eq!(mem.len(), 0); // safety: test
        assert!(mem.is_empty()); // safety: test
        assert!(mem.messages().is_empty()); // safety: test
    }

    #[test]
    fn test_conversation_memory_last_n() {
        let mut mem = ConversationMemory::new(10);
        mem.add(ChatMessage::user("one"));
        mem.add(ChatMessage::assistant("two"));
        mem.add(ChatMessage::user("three"));
        mem.add(ChatMessage::assistant("four"));

        let last_2 = mem.last_n(2);
        assert_eq!(last_2.len(), 2); // safety: test
        assert_eq!(last_2[0].content, "three"); // safety: test
        assert_eq!(last_2[1].content, "four"); // safety: test

        // Requesting more than available returns all
        let last_100 = mem.last_n(100);
        assert_eq!(last_100.len(), 4); // safety: test
    }

    #[test]
    fn test_conversation_memory_last_n_empty() {
        let mem = ConversationMemory::new(10);
        let result = mem.last_n(5);
        assert!(result.is_empty()); // safety: test
    }

    #[test]
    fn test_conversation_memory_preserves_system_message_on_trim() {
        let mut mem = ConversationMemory::new(3);
        mem.add(ChatMessage::system("You are helpful"));
        mem.add(ChatMessage::user("msg1"));
        mem.add(ChatMessage::user("msg2"));

        // At capacity (3). Adding one more should trim, but keep system.
        mem.add(ChatMessage::user("msg3"));

        assert_eq!(mem.len(), 3); // safety: test
        // System message must survive
        assert_eq!(mem.messages()[0].role, crate::llm::Role::System); // safety: test
        assert_eq!(mem.messages()[0].content, "You are helpful"); // safety: test
        // Oldest non-system message (msg1) should be gone
        assert_eq!(mem.messages()[1].content, "msg2"); // safety: test
        assert_eq!(mem.messages()[2].content, "msg3"); // safety: test
    }

    #[test]
    fn test_conversation_memory_trims_non_system_first() {
        let mut mem = ConversationMemory::new(2);
        mem.add(ChatMessage::system("sys"));
        mem.add(ChatMessage::user("a"));
        // Now at capacity. Add another.
        mem.add(ChatMessage::user("b"));

        assert_eq!(mem.len(), 2); // safety: test
        assert_eq!(mem.messages()[0].role, crate::llm::Role::System); // safety: test
        assert_eq!(mem.messages()[1].content, "b"); // safety: test
    }

    #[test]
    fn test_conversation_memory_max_one_with_system_does_not_loop() {
        // Edge case: max_messages = 1 and only a system message.
        // Adding another message would try to trim but should not
        // remove the system message and get stuck.
        let mut mem = ConversationMemory::new(1);
        mem.add(ChatMessage::system("sys"));
        // The system message is already at capacity. Adding another
        // cannot trim the system message, so we end up with 2 (graceful).
        // The important thing is we don't infinite-loop.
        mem.add(ChatMessage::user("hello"));
        // Should have broken out rather than looping forever.
        // The system message is protected, so len may exceed max.
        assert!(mem.len() <= 2); // safety: test
    }

    #[test]
    fn test_memory_failed_actions() {
        let mut memory = Memory::new(Uuid::new_v4());

        let ok = memory.create_action("good", serde_json::json!({})).succeed(
            None,
            serde_json::json!({}),
            Duration::from_millis(1),
        );
        memory.record_action(ok);

        let err = memory
            .create_action("bad", serde_json::json!({}))
            .fail("oops", Duration::from_millis(2));
        memory.record_action(err);

        assert_eq!(memory.successful_actions(), 1); // safety: test
        assert_eq!(memory.failed_actions(), 1); // safety: test
    }

    #[test]
    fn test_memory_last_action() {
        let mut memory = Memory::new(Uuid::new_v4());
        assert!(memory.last_action().is_none()); // safety: test

        let a1 = memory
            .create_action("first", serde_json::json!({}))
            .succeed(None, serde_json::json!({}), Duration::ZERO);
        memory.record_action(a1);

        let a2 = memory
            .create_action("second", serde_json::json!({}))
            .fail("nope", Duration::ZERO);
        memory.record_action(a2);

        let last = memory.last_action().unwrap(); // safety: test
        assert_eq!(last.tool_name, "second"); // safety: test
    }

    #[test]
    fn test_memory_actions_by_tool() {
        let mut memory = Memory::new(Uuid::new_v4());

        for _ in 0..3 {
            let a = memory
                .create_action("shell", serde_json::json!({}))
                .succeed(None, serde_json::json!({}), Duration::ZERO);
            memory.record_action(a);
        }
        let a = memory.create_action("http", serde_json::json!({})).succeed(
            None,
            serde_json::json!({}),
            Duration::ZERO,
        );
        memory.record_action(a);

        assert_eq!(memory.actions_by_tool("shell").len(), 3); // safety: test
        assert_eq!(memory.actions_by_tool("http").len(), 1); // safety: test
        assert_eq!(memory.actions_by_tool("nonexistent").len(), 0); // safety: test
    }

    #[test]
    fn test_memory_create_action_increments_sequence() {
        let mut memory = Memory::new(Uuid::new_v4());

        let a0 = memory.create_action("t", serde_json::json!({}));
        assert_eq!(a0.sequence, 0); // safety: test

        let a1 = memory.create_action("t", serde_json::json!({}));
        assert_eq!(a1.sequence, 1); // safety: test

        let a2 = memory.create_action("t", serde_json::json!({}));
        assert_eq!(a2.sequence, 2); // safety: test
    }

    #[test]
    fn test_memory_add_message_delegates_to_conversation() {
        let mut memory = Memory::new(Uuid::new_v4());
        assert!(memory.conversation.is_empty()); // safety: test

        memory.add_message(ChatMessage::user("hello"));
        memory.add_message(ChatMessage::assistant("hi"));

        assert_eq!(memory.conversation.len(), 2); // safety: test
        assert_eq!(memory.conversation.messages()[0].content, "hello"); // safety: test
    }

    #[test]
    fn test_memory_total_cost_with_no_cost_actions() {
        let mut memory = Memory::new(Uuid::new_v4());

        // Actions without cost should contribute zero
        let a = memory
            .create_action("free_tool", serde_json::json!({}))
            .succeed(None, serde_json::json!({}), Duration::ZERO);
        memory.record_action(a);

        assert_eq!(memory.total_cost(), Decimal::ZERO); // safety: test
    }

    #[test]
    fn test_memory_total_duration_mixed() {
        let mut memory = Memory::new(Uuid::new_v4());

        let a1 = memory.create_action("t1", serde_json::json!({})).succeed(
            None,
            serde_json::json!({}),
            Duration::from_millis(100),
        );
        memory.record_action(a1);

        let a2 = memory
            .create_action("t2", serde_json::json!({}))
            .fail("err", Duration::from_millis(200));
        memory.record_action(a2);

        // Both successful and failed actions contribute to total duration
        assert_eq!(memory.total_duration(), Duration::from_millis(300)); // safety: test
    }
}
