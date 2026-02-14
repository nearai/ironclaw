//! Session and thread model for turn-based agent interactions.
//!
//! A Session contains one or more Threads. Each Thread represents a
//! conversation/interaction sequence with the agent. Threads contain
//! Turns, which are request/response pairs.
//!
//! This model supports:
//! - Undo: Roll back to a previous turn
//! - Interrupt: Cancel the current turn mid-execution
//! - Compaction: Summarize old turns to save context
//! - Resume: Continue from a saved checkpoint

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::llm::ChatMessage;

/// A session containing one or more threads.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    /// Unique session ID.
    pub id: Uuid,
    /// User ID that owns this session.
    pub user_id: String,
    /// Active thread ID.
    pub active_thread: Option<Uuid>,
    /// All threads in this session.
    pub threads: HashMap<Uuid, Thread>,
    /// When the session was created.
    pub created_at: DateTime<Utc>,
    /// When the session was last active.
    pub last_active_at: DateTime<Utc>,
    /// Session metadata.
    pub metadata: serde_json::Value,
    /// Tools that have been auto-approved for this session ("always approve").
    #[serde(default)]
    pub auto_approved_tools: HashSet<String>,
}

impl Session {
    /// Create a new session.
    pub fn new(user_id: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            user_id: user_id.into(),
            active_thread: None,
            threads: HashMap::new(),
            created_at: now,
            last_active_at: now,
            metadata: serde_json::Value::Null,
            auto_approved_tools: HashSet::new(),
        }
    }

    /// Check if a tool has been auto-approved for this session.
    pub fn is_tool_auto_approved(&self, tool_name: &str) -> bool {
        self.auto_approved_tools.contains(tool_name)
    }

    /// Add a tool to the auto-approved set.
    pub fn auto_approve_tool(&mut self, tool_name: impl Into<String>) {
        self.auto_approved_tools.insert(tool_name.into());
    }

    /// Create a new thread in this session.
    pub fn create_thread(&mut self) -> &mut Thread {
        let thread = Thread::new(self.id);
        let thread_id = thread.id;
        self.threads.insert(thread_id, thread);
        self.active_thread = Some(thread_id);
        self.last_active_at = Utc::now();
        self.threads.get_mut(&thread_id).expect("just inserted")
    }

    /// Get the active thread.
    pub fn active_thread(&self) -> Option<&Thread> {
        self.active_thread.and_then(|id| self.threads.get(&id))
    }

    /// Get the active thread mutably.
    pub fn active_thread_mut(&mut self) -> Option<&mut Thread> {
        self.active_thread.and_then(|id| self.threads.get_mut(&id))
    }

    /// Get or create the active thread.
    pub fn get_or_create_thread(&mut self) -> &mut Thread {
        if self.active_thread.is_none() {
            self.create_thread();
        }
        self.active_thread_mut().expect("just created")
    }

    /// Switch to a different thread.
    pub fn switch_thread(&mut self, thread_id: Uuid) -> bool {
        if self.threads.contains_key(&thread_id) {
            self.active_thread = Some(thread_id);
            self.last_active_at = Utc::now();
            true
        } else {
            false
        }
    }
}

/// State of a thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThreadState {
    /// Thread is idle, waiting for input.
    Idle,
    /// Thread is processing a turn.
    Processing,
    /// Thread is waiting for user approval.
    AwaitingApproval,
    /// Thread has completed (no more turns expected).
    Completed,
    /// Thread was interrupted.
    Interrupted,
}

/// Pending auth token request.
///
/// When `tool_auth` returns `awaiting_token`, the thread enters auth mode.
/// The next user message is intercepted before entering the normal pipeline
/// (no logging, no turn creation, no history) and routed directly to the
/// credential store.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingAuth {
    /// Extension name to authenticate.
    pub extension_name: String,
}

/// Pending tool approval request stored on a thread.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingApproval {
    /// Unique request ID.
    pub request_id: Uuid,
    /// Tool name requiring approval.
    pub tool_name: String,
    /// Tool parameters.
    pub parameters: serde_json::Value,
    /// Description of what the tool will do.
    pub description: String,
    /// Tool call ID from LLM (for proper context continuation).
    pub tool_call_id: String,
    /// Context messages at the time of the request (to resume from).
    pub context_messages: Vec<ChatMessage>,
}

/// A conversation thread within a session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Thread {
    /// Unique thread ID.
    pub id: Uuid,
    /// Parent session ID.
    pub session_id: Uuid,
    /// Current state.
    pub state: ThreadState,
    /// Turns in this thread.
    pub turns: Vec<Turn>,
    /// When the thread was created.
    pub created_at: DateTime<Utc>,
    /// When the thread was last updated.
    pub updated_at: DateTime<Utc>,
    /// Thread metadata (e.g., title, tags).
    pub metadata: serde_json::Value,
    /// Pending approval request (when state is AwaitingApproval).
    #[serde(default)]
    pub pending_approval: Option<PendingApproval>,
    /// Pending auth token request (thread is in auth mode).
    #[serde(default)]
    pub pending_auth: Option<PendingAuth>,
    /// Last NEAR AI response ID for response chaining. Persisted to DB
    /// metadata so we can resume chaining across restarts.
    #[serde(default)]
    pub last_response_id: Option<String>,
}

impl Thread {
    /// Create a new thread.
    pub fn new(session_id: Uuid) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            session_id,
            state: ThreadState::Idle,
            turns: Vec::new(),
            created_at: now,
            updated_at: now,
            metadata: serde_json::Value::Null,
            pending_approval: None,
            pending_auth: None,
            last_response_id: None,
        }
    }

    /// Create a thread with a specific ID (for DB hydration).
    pub fn with_id(id: Uuid, session_id: Uuid) -> Self {
        let now = Utc::now();
        Self {
            id,
            session_id,
            state: ThreadState::Idle,
            turns: Vec::new(),
            created_at: now,
            updated_at: now,
            metadata: serde_json::Value::Null,
            pending_approval: None,
            pending_auth: None,
            last_response_id: None,
        }
    }

    /// Get the current turn number (1-indexed for display).
    pub fn turn_number(&self) -> usize {
        self.turns.len() + 1
    }

    /// Get the last turn.
    pub fn last_turn(&self) -> Option<&Turn> {
        self.turns.last()
    }

    /// Get the last turn mutably.
    pub fn last_turn_mut(&mut self) -> Option<&mut Turn> {
        self.turns.last_mut()
    }

    /// Start a new turn with user input.
    pub fn start_turn(&mut self, user_input: impl Into<String>) -> &mut Turn {
        let turn_number = self.turns.len();
        let turn = Turn::new(turn_number, user_input);
        self.turns.push(turn);
        self.state = ThreadState::Processing;
        self.updated_at = Utc::now();
        self.turns.last_mut().expect("just pushed")
    }

    /// Complete the current turn with a response.
    pub fn complete_turn(&mut self, response: impl Into<String>) {
        if let Some(turn) = self.turns.last_mut() {
            turn.complete(response);
        }
        self.state = ThreadState::Idle;
        self.updated_at = Utc::now();
    }

    /// Fail the current turn with an error.
    pub fn fail_turn(&mut self, error: impl Into<String>) {
        if let Some(turn) = self.turns.last_mut() {
            turn.fail(error);
        }
        self.state = ThreadState::Idle;
        self.updated_at = Utc::now();
    }

    /// Mark the thread as awaiting approval with pending request details.
    pub fn await_approval(&mut self, pending: PendingApproval) {
        self.state = ThreadState::AwaitingApproval;
        self.pending_approval = Some(pending);
        self.updated_at = Utc::now();
    }

    /// Take the pending approval (clearing it from the thread).
    pub fn take_pending_approval(&mut self) -> Option<PendingApproval> {
        self.pending_approval.take()
    }

    /// Clear pending approval and return to idle state.
    pub fn clear_pending_approval(&mut self) {
        self.pending_approval = None;
        self.state = ThreadState::Idle;
        self.updated_at = Utc::now();
    }

    /// Enter auth mode: next user message will be routed directly to
    /// the credential store, bypassing the normal pipeline entirely.
    pub fn enter_auth_mode(&mut self, extension_name: String) {
        self.pending_auth = Some(PendingAuth { extension_name });
        self.updated_at = Utc::now();
    }

    /// Take the pending auth (clearing auth mode).
    pub fn take_pending_auth(&mut self) -> Option<PendingAuth> {
        self.pending_auth.take()
    }

    /// Interrupt the current turn.
    pub fn interrupt(&mut self) {
        if let Some(turn) = self.turns.last_mut() {
            turn.interrupt();
        }
        self.state = ThreadState::Interrupted;
        self.updated_at = Utc::now();
    }

    /// Resume after interruption.
    pub fn resume(&mut self) {
        if self.state == ThreadState::Interrupted {
            self.state = ThreadState::Idle;
            self.updated_at = Utc::now();
        }
    }

    /// Get all messages for context building.
    pub fn messages(&self) -> Vec<ChatMessage> {
        let mut messages = Vec::new();
        for turn in &self.turns {
            messages.push(ChatMessage::user(&turn.user_input));
            if let Some(ref response) = turn.response {
                messages.push(ChatMessage::assistant(response));
            }
        }
        messages
    }

    /// Truncate turns to a specific count (keeping most recent).
    pub fn truncate_turns(&mut self, keep: usize) {
        if self.turns.len() > keep {
            let drain_count = self.turns.len() - keep;
            self.turns.drain(0..drain_count);
            // Re-number remaining turns
            for (i, turn) in self.turns.iter_mut().enumerate() {
                turn.turn_number = i;
            }
        }
    }

    /// Restore thread state from a checkpoint's messages.
    ///
    /// Clears existing turns and rebuilds from message pairs.
    /// Messages should alternate: user, assistant, user, assistant...
    pub fn restore_from_messages(&mut self, messages: Vec<ChatMessage>) {
        self.turns.clear();
        self.state = ThreadState::Idle;

        // Messages alternate: user, assistant, user, assistant...
        let mut iter = messages.into_iter().peekable();
        let mut turn_number = 0;

        while let Some(msg) = iter.next() {
            if msg.role == crate::llm::Role::User {
                let mut turn = Turn::new(turn_number, &msg.content);

                // Check if next is assistant response
                if let Some(next) = iter.peek()
                    && next.role == crate::llm::Role::Assistant
                {
                    let response = iter.next().expect("peeked");
                    turn.complete(&response.content);
                }

                self.turns.push(turn);
                turn_number += 1;
            }
        }

        self.updated_at = Utc::now();
    }
}

/// State of a turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TurnState {
    /// Turn is being processed.
    Processing,
    /// Turn completed successfully.
    Completed,
    /// Turn failed with an error.
    Failed,
    /// Turn was interrupted.
    Interrupted,
}

/// A single turn (request/response pair) in a thread.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Turn {
    /// Turn number (0-indexed).
    pub turn_number: usize,
    /// User input that started this turn.
    pub user_input: String,
    /// Agent response (if completed).
    pub response: Option<String>,
    /// Tool calls made during this turn.
    pub tool_calls: Vec<TurnToolCall>,
    /// Turn state.
    pub state: TurnState,
    /// When the turn started.
    pub started_at: DateTime<Utc>,
    /// When the turn completed.
    pub completed_at: Option<DateTime<Utc>>,
    /// Error message (if failed).
    pub error: Option<String>,
}

impl Turn {
    /// Create a new turn.
    pub fn new(turn_number: usize, user_input: impl Into<String>) -> Self {
        Self {
            turn_number,
            user_input: user_input.into(),
            response: None,
            tool_calls: Vec::new(),
            state: TurnState::Processing,
            started_at: Utc::now(),
            completed_at: None,
            error: None,
        }
    }

    /// Complete this turn.
    pub fn complete(&mut self, response: impl Into<String>) {
        self.response = Some(response.into());
        self.state = TurnState::Completed;
        self.completed_at = Some(Utc::now());
    }

    /// Fail this turn.
    pub fn fail(&mut self, error: impl Into<String>) {
        self.error = Some(error.into());
        self.state = TurnState::Failed;
        self.completed_at = Some(Utc::now());
    }

    /// Interrupt this turn.
    pub fn interrupt(&mut self) {
        self.state = TurnState::Interrupted;
        self.completed_at = Some(Utc::now());
    }

    /// Record a tool call.
    pub fn record_tool_call(&mut self, name: impl Into<String>, params: serde_json::Value) {
        self.tool_calls.push(TurnToolCall {
            name: name.into(),
            parameters: params,
            result: None,
            error: None,
        });
    }

    /// Record tool call result.
    pub fn record_tool_result(&mut self, result: serde_json::Value) {
        if let Some(call) = self.tool_calls.last_mut() {
            call.result = Some(result);
        }
    }

    /// Record tool call error.
    pub fn record_tool_error(&mut self, error: impl Into<String>) {
        if let Some(call) = self.tool_calls.last_mut() {
            call.error = Some(error.into());
        }
    }
}

/// Record of a tool call made during a turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnToolCall {
    /// Tool name.
    pub name: String,
    /// Parameters passed to the tool.
    pub parameters: serde_json::Value,
    /// Result from the tool (if successful).
    pub result: Option<serde_json::Value>,
    /// Error from the tool (if failed).
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_creation() {
        let mut session = Session::new("user-123");
        assert!(session.active_thread.is_none());

        session.create_thread();
        assert!(session.active_thread.is_some());
    }

    #[test]
    fn test_thread_turns() {
        let mut thread = Thread::new(Uuid::new_v4());

        thread.start_turn("Hello");
        assert_eq!(thread.state, ThreadState::Processing);
        assert_eq!(thread.turns.len(), 1);

        thread.complete_turn("Hi there!");
        assert_eq!(thread.state, ThreadState::Idle);
        assert_eq!(thread.turns[0].response, Some("Hi there!".to_string()));
    }

    #[test]
    fn test_thread_messages() {
        let mut thread = Thread::new(Uuid::new_v4());

        thread.start_turn("First message");
        thread.complete_turn("First response");
        thread.start_turn("Second message");
        thread.complete_turn("Second response");

        let messages = thread.messages();
        assert_eq!(messages.len(), 4);
    }

    #[test]
    fn test_turn_tool_calls() {
        let mut turn = Turn::new(0, "Test input");
        turn.record_tool_call("echo", serde_json::json!({"message": "test"}));
        turn.record_tool_result(serde_json::json!("test"));

        assert_eq!(turn.tool_calls.len(), 1);
        assert!(turn.tool_calls[0].result.is_some());
    }

    #[test]
    fn test_restore_from_messages() {
        let mut thread = Thread::new(Uuid::new_v4());

        // First add some turns
        thread.start_turn("Original message");
        thread.complete_turn("Original response");

        // Now restore from different messages
        let messages = vec![
            ChatMessage::user("Hello"),
            ChatMessage::assistant("Hi there!"),
            ChatMessage::user("How are you?"),
            ChatMessage::assistant("I'm good!"),
        ];

        thread.restore_from_messages(messages);

        assert_eq!(thread.turns.len(), 2);
        assert_eq!(thread.turns[0].user_input, "Hello");
        assert_eq!(thread.turns[0].response, Some("Hi there!".to_string()));
        assert_eq!(thread.turns[1].user_input, "How are you?");
        assert_eq!(thread.turns[1].response, Some("I'm good!".to_string()));
        assert_eq!(thread.state, ThreadState::Idle);
    }

    #[test]
    fn test_restore_from_messages_incomplete_turn() {
        let mut thread = Thread::new(Uuid::new_v4());

        // Messages with incomplete last turn (no assistant response)
        let messages = vec![
            ChatMessage::user("Hello"),
            ChatMessage::assistant("Hi there!"),
            ChatMessage::user("How are you?"),
        ];

        thread.restore_from_messages(messages);

        assert_eq!(thread.turns.len(), 2);
        assert_eq!(thread.turns[1].user_input, "How are you?");
        assert!(thread.turns[1].response.is_none());
    }

    #[test]
    fn test_enter_auth_mode() {
        let mut thread = Thread::new(Uuid::new_v4());
        assert!(thread.pending_auth.is_none());

        thread.enter_auth_mode("telegram".to_string());
        assert!(thread.pending_auth.is_some());
        assert_eq!(
            thread.pending_auth.as_ref().unwrap().extension_name,
            "telegram"
        );
    }

    #[test]
    fn test_take_pending_auth() {
        let mut thread = Thread::new(Uuid::new_v4());
        thread.enter_auth_mode("notion".to_string());

        let pending = thread.take_pending_auth();
        assert!(pending.is_some());
        assert_eq!(pending.unwrap().extension_name, "notion");

        // Should be cleared after take
        assert!(thread.pending_auth.is_none());
        assert!(thread.take_pending_auth().is_none());
    }

    #[test]
    fn test_pending_auth_serialization() {
        let mut thread = Thread::new(Uuid::new_v4());
        thread.enter_auth_mode("openai".to_string());

        let json = serde_json::to_string(&thread).expect("should serialize");
        assert!(json.contains("pending_auth"));
        assert!(json.contains("openai"));

        let restored: Thread = serde_json::from_str(&json).expect("should deserialize");
        assert!(restored.pending_auth.is_some());
        assert_eq!(restored.pending_auth.unwrap().extension_name, "openai");
    }

    #[test]
    fn test_pending_auth_default_none() {
        // Deserialization of old data without pending_auth should default to None
        let mut thread = Thread::new(Uuid::new_v4());
        thread.pending_auth = None;
        let json = serde_json::to_string(&thread).expect("serialize");

        // Remove the pending_auth field to simulate old data
        let json = json.replace(",\"pending_auth\":null", "");
        let restored: Thread = serde_json::from_str(&json).expect("should deserialize");
        assert!(restored.pending_auth.is_none());
    }

    #[test]
    fn test_thread_with_id() {
        let specific_id = Uuid::new_v4();
        let session_id = Uuid::new_v4();
        let thread = Thread::with_id(specific_id, session_id);

        assert_eq!(thread.id, specific_id);
        assert_eq!(thread.session_id, session_id);
        assert_eq!(thread.state, ThreadState::Idle);
        assert!(thread.turns.is_empty());
    }

    #[test]
    fn test_thread_with_id_restore_messages() {
        let thread_id = Uuid::new_v4();
        let session_id = Uuid::new_v4();
        let mut thread = Thread::with_id(thread_id, session_id);

        let messages = vec![
            ChatMessage::user("Hello from DB"),
            ChatMessage::assistant("Restored response"),
        ];
        thread.restore_from_messages(messages);

        assert_eq!(thread.id, thread_id);
        assert_eq!(thread.turns.len(), 1);
        assert_eq!(thread.turns[0].user_input, "Hello from DB");
        assert_eq!(
            thread.turns[0].response,
            Some("Restored response".to_string())
        );
    }

    #[test]
    fn test_restore_from_messages_empty() {
        let mut thread = Thread::new(Uuid::new_v4());

        // Add a turn first, then restore with empty vec
        thread.start_turn("hello");
        thread.complete_turn("hi");
        assert_eq!(thread.turns.len(), 1);

        thread.restore_from_messages(Vec::new());

        // Should clear all turns and stay idle
        assert!(thread.turns.is_empty());
        assert_eq!(thread.state, ThreadState::Idle);
    }

    #[test]
    fn test_restore_from_messages_only_assistant_messages() {
        let mut thread = Thread::new(Uuid::new_v4());

        // Only assistant messages (no user messages to anchor turns)
        let messages = vec![
            ChatMessage::assistant("I'm here"),
            ChatMessage::assistant("Still here"),
        ];

        thread.restore_from_messages(messages);

        // Assistant-only messages have no user turn to attach to, so
        // they should be skipped entirely.
        assert!(thread.turns.is_empty());
    }

    #[test]
    fn test_restore_from_messages_multiple_user_messages_in_a_row() {
        let mut thread = Thread::new(Uuid::new_v4());

        // Two user messages with no assistant response between them
        let messages = vec![
            ChatMessage::user("first"),
            ChatMessage::user("second"),
            ChatMessage::assistant("reply to second"),
        ];

        thread.restore_from_messages(messages);

        // First user message becomes a turn with no response,
        // second user message pairs with the assistant response.
        assert_eq!(thread.turns.len(), 2);
        assert_eq!(thread.turns[0].user_input, "first");
        assert!(thread.turns[0].response.is_none());
        assert_eq!(thread.turns[1].user_input, "second");
        assert_eq!(
            thread.turns[1].response,
            Some("reply to second".to_string())
        );
    }

    #[test]
    fn test_thread_switch() {
        let mut session = Session::new("user-1");

        let t1_id = session.create_thread().id;
        let t2_id = session.create_thread().id;

        // After creating two threads, active should be the last one
        assert_eq!(session.active_thread, Some(t2_id));

        // Switch back to the first
        assert!(session.switch_thread(t1_id));
        assert_eq!(session.active_thread, Some(t1_id));

        // Switching to a nonexistent thread should fail
        let fake_id = Uuid::new_v4();
        assert!(!session.switch_thread(fake_id));
        // Active thread should remain unchanged
        assert_eq!(session.active_thread, Some(t1_id));
    }

    #[test]
    fn test_get_or_create_thread_idempotent() {
        let mut session = Session::new("user-1");

        let tid1 = session.get_or_create_thread().id;
        let tid2 = session.get_or_create_thread().id;

        // Should return the same thread (not create a new one each time)
        assert_eq!(tid1, tid2);
        assert_eq!(session.threads.len(), 1);
    }

    #[test]
    fn test_truncate_turns() {
        let mut thread = Thread::new(Uuid::new_v4());

        for i in 0..5 {
            thread.start_turn(format!("msg-{}", i));
            thread.complete_turn(format!("resp-{}", i));
        }
        assert_eq!(thread.turns.len(), 5);

        thread.truncate_turns(3);
        assert_eq!(thread.turns.len(), 3);

        // Should keep the most recent turns
        assert_eq!(thread.turns[0].user_input, "msg-2");
        assert_eq!(thread.turns[1].user_input, "msg-3");
        assert_eq!(thread.turns[2].user_input, "msg-4");

        // Turn numbers should be re-indexed
        assert_eq!(thread.turns[0].turn_number, 0);
        assert_eq!(thread.turns[1].turn_number, 1);
        assert_eq!(thread.turns[2].turn_number, 2);
    }

    #[test]
    fn test_truncate_turns_noop_when_fewer() {
        let mut thread = Thread::new(Uuid::new_v4());

        thread.start_turn("only one");
        thread.complete_turn("response");

        thread.truncate_turns(10);
        assert_eq!(thread.turns.len(), 1);
        assert_eq!(thread.turns[0].user_input, "only one");
    }

    #[test]
    fn test_thread_interrupt_and_resume() {
        let mut thread = Thread::new(Uuid::new_v4());

        thread.start_turn("do something");
        assert_eq!(thread.state, ThreadState::Processing);

        thread.interrupt();
        assert_eq!(thread.state, ThreadState::Interrupted);

        let last_turn = thread.last_turn().unwrap();
        assert_eq!(last_turn.state, TurnState::Interrupted);
        assert!(last_turn.completed_at.is_some());

        thread.resume();
        assert_eq!(thread.state, ThreadState::Idle);
    }

    #[test]
    fn test_resume_only_from_interrupted() {
        let mut thread = Thread::new(Uuid::new_v4());

        // Idle thread: resume should be a no-op
        assert_eq!(thread.state, ThreadState::Idle);
        thread.resume();
        assert_eq!(thread.state, ThreadState::Idle);

        // Processing thread: resume should not change state
        thread.start_turn("work");
        assert_eq!(thread.state, ThreadState::Processing);
        thread.resume();
        assert_eq!(thread.state, ThreadState::Processing);
    }

    #[test]
    fn test_turn_fail() {
        let mut thread = Thread::new(Uuid::new_v4());

        thread.start_turn("risky operation");
        thread.fail_turn("connection timed out");

        assert_eq!(thread.state, ThreadState::Idle);

        let turn = thread.last_turn().unwrap();
        assert_eq!(turn.state, TurnState::Failed);
        assert_eq!(turn.error, Some("connection timed out".to_string()));
        assert!(turn.response.is_none());
        assert!(turn.completed_at.is_some());
    }

    #[test]
    fn test_messages_with_incomplete_last_turn() {
        let mut thread = Thread::new(Uuid::new_v4());

        thread.start_turn("first");
        thread.complete_turn("first reply");
        thread.start_turn("second (in progress)");

        let messages = thread.messages();
        // Should have 3 messages: user, assistant, user (no assistant for in-progress)
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0].content, "first");
        assert_eq!(messages[1].content, "first reply");
        assert_eq!(messages[2].content, "second (in progress)");
    }

    #[test]
    fn test_thread_serialization_round_trip() {
        let mut thread = Thread::new(Uuid::new_v4());

        thread.start_turn("hello");
        thread.complete_turn("world");
        thread.last_response_id = Some("resp_abc123".to_string());

        let json = serde_json::to_string(&thread).unwrap();
        let restored: Thread = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.id, thread.id);
        assert_eq!(restored.session_id, thread.session_id);
        assert_eq!(restored.turns.len(), 1);
        assert_eq!(restored.turns[0].user_input, "hello");
        assert_eq!(restored.turns[0].response, Some("world".to_string()));
        assert_eq!(restored.last_response_id, Some("resp_abc123".to_string()));
    }

    #[test]
    fn test_session_serialization_round_trip() {
        let mut session = Session::new("user-ser");
        session.create_thread();
        session.auto_approve_tool("echo");

        let json = serde_json::to_string(&session).unwrap();
        let restored: Session = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.user_id, "user-ser");
        assert_eq!(restored.threads.len(), 1);
        assert!(restored.is_tool_auto_approved("echo"));
        assert!(!restored.is_tool_auto_approved("shell"));
    }

    #[test]
    fn test_auto_approved_tools() {
        let mut session = Session::new("user-1");

        assert!(!session.is_tool_auto_approved("shell"));
        session.auto_approve_tool("shell");
        assert!(session.is_tool_auto_approved("shell"));

        // Idempotent
        session.auto_approve_tool("shell");
        assert_eq!(session.auto_approved_tools.len(), 1);
    }

    #[test]
    fn test_turn_tool_call_error() {
        let mut turn = Turn::new(0, "test");
        turn.record_tool_call("http", serde_json::json!({"url": "example.com"}));
        turn.record_tool_error("timeout");

        assert_eq!(turn.tool_calls.len(), 1);
        assert_eq!(turn.tool_calls[0].error, Some("timeout".to_string()));
        assert!(turn.tool_calls[0].result.is_none());
    }

    #[test]
    fn test_turn_number_increments() {
        let mut thread = Thread::new(Uuid::new_v4());

        // Before any turns, turn_number() is 1 (1-indexed for display)
        assert_eq!(thread.turn_number(), 1);

        thread.start_turn("first");
        thread.complete_turn("done");
        assert_eq!(thread.turn_number(), 2);

        thread.start_turn("second");
        assert_eq!(thread.turn_number(), 3);
    }

    #[test]
    fn test_complete_turn_on_empty_thread() {
        let mut thread = Thread::new(Uuid::new_v4());

        // Completing a turn when there are no turns should be a safe no-op
        thread.complete_turn("phantom response");
        assert_eq!(thread.state, ThreadState::Idle);
        assert!(thread.turns.is_empty());
    }

    #[test]
    fn test_fail_turn_on_empty_thread() {
        let mut thread = Thread::new(Uuid::new_v4());

        // Failing a turn when there are no turns should be a safe no-op
        thread.fail_turn("phantom error");
        assert_eq!(thread.state, ThreadState::Idle);
        assert!(thread.turns.is_empty());
    }

    #[test]
    fn test_pending_approval_flow() {
        let mut thread = Thread::new(Uuid::new_v4());

        let approval = PendingApproval {
            request_id: Uuid::new_v4(),
            tool_name: "shell".to_string(),
            parameters: serde_json::json!({"command": "rm -rf /"}),
            description: "dangerous command".to_string(),
            tool_call_id: "call_123".to_string(),
            context_messages: vec![ChatMessage::user("do it")],
        };

        thread.await_approval(approval);
        assert_eq!(thread.state, ThreadState::AwaitingApproval);
        assert!(thread.pending_approval.is_some());

        let taken = thread.take_pending_approval();
        assert!(taken.is_some());
        assert_eq!(taken.unwrap().tool_name, "shell");
        assert!(thread.pending_approval.is_none());
    }

    #[test]
    fn test_clear_pending_approval() {
        let mut thread = Thread::new(Uuid::new_v4());

        let approval = PendingApproval {
            request_id: Uuid::new_v4(),
            tool_name: "http".to_string(),
            parameters: serde_json::json!({}),
            description: "test".to_string(),
            tool_call_id: "call_456".to_string(),
            context_messages: vec![],
        };

        thread.await_approval(approval);
        thread.clear_pending_approval();

        assert_eq!(thread.state, ThreadState::Idle);
        assert!(thread.pending_approval.is_none());
    }

    #[test]
    fn test_active_thread_accessors() {
        let mut session = Session::new("user-1");

        assert!(session.active_thread().is_none());
        assert!(session.active_thread_mut().is_none());

        let tid = session.create_thread().id;

        assert!(session.active_thread().is_some());
        assert_eq!(session.active_thread().unwrap().id, tid);

        // Mutably modify through accessor
        session.active_thread_mut().unwrap().start_turn("test");
        assert_eq!(
            session.active_thread().unwrap().state,
            ThreadState::Processing
        );
    }
}
