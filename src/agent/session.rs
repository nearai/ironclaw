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

use std::collections::{HashMap, HashSet, VecDeque};

use chrono::{DateTime, TimeDelta, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::llm::{ChatMessage, ToolCall, generate_tool_call_id};
use ironclaw_common::truncate_preview;

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
        self.active_thread = Some(thread_id);
        self.last_active_at = Utc::now();
        self.threads.entry(thread_id).or_insert(thread)
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
        match self.active_thread {
            None => self.create_thread(),
            Some(id) => {
                if self.threads.contains_key(&id) {
                    // Entry existence confirmed by contains_key above.
                    // get_mut borrows self.threads mutably, so we can't
                    // combine the check and access into if-let without
                    // conflicting with the self.create_thread() fallback.
                    self.threads.get_mut(&id).unwrap() // safety: contains_key guard above
                } else {
                    // Stale active_thread ID: create a new thread, which
                    // updates self.active_thread to the new thread's ID.
                    self.create_thread()
                }
            }
        }
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
/// Auth mode TTL — must stay in sync with
/// `crate::cli::oauth_defaults::OAUTH_FLOW_EXPIRY` (5 minutes / 300 s).
/// Defined separately to avoid a session→cli module dependency.
const AUTH_MODE_TTL_SECS: i64 = 300;
const AUTH_MODE_TTL: TimeDelta = TimeDelta::seconds(AUTH_MODE_TTL_SECS);

/// When `tool_auth` returns `awaiting_token`, the thread enters auth mode.
/// The next user message is intercepted before entering the normal pipeline
/// (no logging, no turn creation, no history) and routed directly to the
/// credential store.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingAuth {
    /// Extension name to authenticate.
    pub extension_name: String,
    /// When this auth mode was entered. Used for TTL expiry.
    #[serde(default = "Utc::now")]
    pub created_at: DateTime<Utc>,
}

impl PendingAuth {
    /// Returns `true` if this auth mode has exceeded the TTL.
    pub fn is_expired(&self) -> bool {
        Utc::now() - self.created_at > AUTH_MODE_TTL
    }
}

/// Pending tool approval request stored on a thread.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingApproval {
    /// Unique request ID.
    pub request_id: Uuid,
    /// Tool name requiring approval.
    pub tool_name: String,
    /// Tool parameters (original values, used for execution).
    pub parameters: serde_json::Value,
    /// Redacted tool parameters (sensitive values replaced with `[REDACTED]`).
    /// Used for display in approval UI, logs, and SSE broadcasts.
    #[serde(default)]
    pub display_parameters: serde_json::Value,
    /// Description of what the tool will do.
    pub description: String,
    /// Tool call ID from LLM (for proper context continuation).
    pub tool_call_id: String,
    /// Context messages at the time of the request (to resume from).
    pub context_messages: Vec<ChatMessage>,
    /// Remaining tool calls from the same assistant message that were not
    /// executed yet when approval was requested.
    #[serde(default)]
    pub deferred_tool_calls: Vec<ToolCall>,
    /// User timezone at the time the approval was requested, so it persists
    /// through the approval flow even if the approval message lacks timezone.
    #[serde(default)]
    pub user_timezone: Option<String>,
    /// Whether the "always" auto-approve option should be offered to the user.
    /// `false` when the tool returned `ApprovalRequirement::Always` (e.g.
    /// destructive shell commands), meaning every invocation must be confirmed.
    #[serde(default = "default_true")]
    pub allow_always: bool,
}

fn default_true() -> bool {
    true
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
    /// Messages queued while the thread was processing a turn.
    #[serde(default, skip_serializing_if = "VecDeque::is_empty")]
    pub pending_messages: VecDeque<String>,
}

/// Maximum number of messages that can be queued while a thread is processing.
/// 10 merged messages can produce a large combined input for the LLM, but this
/// is acceptable for the personal assistant use case where a single user sends
/// rapid follow-ups. The drain loop processes them as one newline-delimited turn.
pub const MAX_PENDING_MESSAGES: usize = 10;

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
            pending_messages: VecDeque::new(),
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
            pending_messages: VecDeque::new(),
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

    /// Queue a message for processing after the current turn completes.
    /// Returns `false` if the queue is at capacity ([`MAX_PENDING_MESSAGES`]).
    pub fn queue_message(&mut self, content: String) -> bool {
        if self.pending_messages.len() >= MAX_PENDING_MESSAGES {
            return false;
        }
        self.pending_messages.push_back(content);
        self.updated_at = Utc::now();
        true
    }

    /// Take the next pending message from the queue.
    pub fn take_pending_message(&mut self) -> Option<String> {
        self.pending_messages.pop_front()
    }

    /// Drain all pending messages from the queue.
    /// Multiple messages are joined with newlines so the LLM receives
    /// full context from rapid consecutive inputs (#259).
    pub fn drain_pending_messages(&mut self) -> Option<String> {
        if self.pending_messages.is_empty() {
            return None;
        }
        let parts: Vec<String> = self.pending_messages.drain(..).collect();
        self.updated_at = Utc::now();
        Some(parts.join("\n"))
    }

    /// Re-queue previously drained content at the front of the queue.
    /// Used to preserve user input when the drain loop fails to process
    /// merged messages (soft error, hard error, interrupt).
    ///
    /// This intentionally bypasses [`MAX_PENDING_MESSAGES`] — the content
    /// was already counted against the cap before draining. The overshoot
    /// is bounded to 1 entry (the re-queued merged string) plus any new
    /// messages that arrived during the failed attempt.
    pub fn requeue_drained(&mut self, content: String) {
        self.pending_messages.push_front(content);
        self.updated_at = Utc::now();
    }

    /// Start a new turn with user input.
    pub fn start_turn(&mut self, user_input: impl Into<String>) -> &mut Turn {
        let turn_number = self.turns.len();
        let turn = Turn::new(turn_number, user_input);
        self.turns.push(turn);
        self.state = ThreadState::Processing;
        self.updated_at = Utc::now();
        // turn_number was len() before push, so it's a valid index after push
        &mut self.turns[turn_number]
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
        self.pending_auth = Some(PendingAuth {
            extension_name,
            created_at: Utc::now(),
        });
        self.updated_at = Utc::now();
    }

    /// Take the pending auth (clearing auth mode).
    pub fn take_pending_auth(&mut self) -> Option<PendingAuth> {
        self.pending_auth.take()
    }

    /// Interrupt the current turn and discard any queued messages.
    pub fn interrupt(&mut self) {
        if let Some(turn) = self.turns.last_mut() {
            turn.interrupt();
        }
        self.pending_messages.clear();
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

    /// Get all messages for context building, including tool call history.
    ///
    /// Emits the full LLM-compatible message sequence per turn:
    /// `user → [assistant_with_tool_calls → tool_result*] → assistant`
    ///
    /// This ensures the LLM sees prior tool executions and won't re-attempt
    /// completed actions in subsequent turns.
    pub fn messages(&self) -> Vec<ChatMessage> {
        let mut messages = Vec::new();
        // We use the enumeration index (`turn_idx`) rather than `turn.turn_number`
        // intentionally: after `truncate_turns()`, the remaining turns are
        // re-numbered starting from 0, so the enumeration index and turn_number
        // are equivalent. Using the index avoids coupling to the field and keeps
        // tool-call ID generation deterministic for the current message window.
        for (turn_idx, turn) in self.turns.iter().enumerate() {
            if turn.image_content_parts.is_empty() {
                messages.push(ChatMessage::user(&turn.user_input));
            } else {
                messages.push(ChatMessage::user_with_parts(
                    &turn.user_input,
                    turn.image_content_parts.clone(),
                ));
            }

            if !turn.tool_calls.is_empty() {
                // Assign synthetic call IDs for this turn's tool calls, so that
                // declarations and results can be consistently correlated.
                let tool_calls_with_ids: Vec<(String, &_)> = turn
                    .tool_calls
                    .iter()
                    .enumerate()
                    .map(|(tc_idx, tc)| {
                        // Use provider-compatible tool call IDs derived from turn/tool indices.
                        (generate_tool_call_id(turn_idx, tc_idx), tc)
                    })
                    .collect();

                // Build ToolCall objects using the synthetic call IDs.
                let tool_calls: Vec<ToolCall> = tool_calls_with_ids
                    .iter()
                    .map(|(call_id, tc)| ToolCall {
                        id: call_id.clone(),
                        name: tc.name.clone(),
                        arguments: tc.parameters.clone(),
                    })
                    .collect();

                // Assistant message declaring the tool calls (no text content)
                messages.push(ChatMessage::assistant_with_tool_calls(None, tool_calls));

                // Individual tool result messages, truncated to limit context size.
                for (call_id, tc) in tool_calls_with_ids {
                    let content = if let Some(ref err) = tc.error {
                        // .error already contains the full error text;
                        // pass through without wrapping to avoid double-prefix.
                        truncate_preview(err, 1000)
                    } else if let Some(ref res) = tc.result {
                        let raw = match res {
                            serde_json::Value::String(s) => s.clone(),
                            other => other.to_string(),
                        };
                        truncate_preview(&raw, 1000)
                    } else {
                        "OK".to_string()
                    };
                    messages.push(ChatMessage::tool_result(call_id, &tc.name, content));
                }
            }
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
    /// Clears existing turns and rebuilds from the message sequence.
    /// Handles the full message pattern including tool messages:
    /// `user → [assistant_with_tool_calls → tool_result*] → assistant`
    ///
    /// Also supports the legacy pattern (user/assistant pairs only) for
    /// backward compatibility with old checkpoint data.
    pub fn restore_from_messages(&mut self, messages: Vec<ChatMessage>) {
        self.turns.clear();
        self.state = ThreadState::Idle;

        let mut iter = messages.into_iter().peekable();
        let mut turn_number = 0;

        while let Some(msg) = iter.next() {
            if msg.role == crate::llm::Role::User {
                let mut turn = Turn::new(turn_number, &msg.content);

                // Consume tool call sequences (assistant_with_tool_calls + tool_results).
                // A single turn may contain multiple rounds of tool calls, so we
                // track the cumulative base index into turn.tool_calls.
                while let Some(next) = iter.peek() {
                    if next.role == crate::llm::Role::Assistant && next.tool_calls.is_some() {
                        let call_base_idx = turn.tool_calls.len();

                        if let Some(assistant_msg) = iter.next()
                            && let Some(ref tcs) = assistant_msg.tool_calls
                        {
                            for tc in tcs {
                                turn.record_tool_call(&tc.name, tc.arguments.clone());
                            }
                        }

                        // Consume the corresponding tool_result messages,
                        // indexing relative to this batch's base offset.
                        let mut pos = 0;
                        while let Some(tr) = iter.peek() {
                            if tr.role != crate::llm::Role::Tool {
                                break;
                            }
                            if let Some(tool_msg) = iter.next() {
                                let idx = call_base_idx + pos;
                                if idx < turn.tool_calls.len() {
                                    // Store as result — the error/success distinction
                                    // is for the live turn only; restored context just
                                    // needs the content the LLM originally saw.
                                    turn.tool_calls[idx].result =
                                        Some(serde_json::Value::String(tool_msg.content.clone()));
                                }
                            }
                            pos += 1;
                        }
                    } else {
                        break;
                    }
                }

                // Check if next is the final assistant response for this turn
                let is_final_assistant = iter.peek().is_some_and(|n| {
                    n.role == crate::llm::Role::Assistant && n.tool_calls.is_none()
                });
                if is_final_assistant && let Some(response) = iter.next() {
                    turn.complete(&response.content);
                }

                self.turns.push(turn);
                turn_number += 1;
            } else {
                // Skip non-user messages that aren't anchored to a turn
                continue;
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
    /// Transient image content parts for multimodal LLM input.
    /// Not serialized — images are only needed for the current LLM call.
    /// The text description in `user_input` persists for compaction/context.
    #[serde(skip)]
    pub image_content_parts: Vec<crate::llm::ContentPart>,
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
            image_content_parts: Vec::new(),
        }
    }

    /// Complete this turn.
    pub fn complete(&mut self, response: impl Into<String>) {
        self.response = Some(response.into());
        self.state = TurnState::Completed;
        self.completed_at = Some(Utc::now());
        // Free image data — only needed for the initial LLM call, not subsequent turns
        self.image_content_parts.clear();
    }

    /// Fail this turn.
    pub fn fail(&mut self, error: impl Into<String>) {
        self.error = Some(error.into());
        self.state = TurnState::Failed;
        self.completed_at = Some(Utc::now());
        self.image_content_parts.clear();
    }

    /// Interrupt this turn.
    pub fn interrupt(&mut self) {
        self.state = TurnState::Interrupted;
        self.completed_at = Some(Utc::now());
        self.image_content_parts.clear();
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
        let before = Utc::now();
        let mut thread = Thread::new(Uuid::new_v4());
        assert!(thread.pending_auth.is_none());

        thread.enter_auth_mode("telegram".to_string());
        assert!(thread.pending_auth.is_some());
        let pending = thread.pending_auth.as_ref().unwrap();
        assert_eq!(pending.extension_name, "telegram");
        assert!(pending.created_at >= before);
        assert!(!pending.is_expired());
    }

    #[test]
    fn test_take_pending_auth() {
        let mut thread = Thread::new(Uuid::new_v4());
        thread.enter_auth_mode("notion".to_string());

        let pending = thread.take_pending_auth();
        assert!(pending.is_some());
        let pending = pending.unwrap();
        assert_eq!(pending.extension_name, "notion");
        assert!(!pending.is_expired());
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
        assert!(json.contains("created_at"));

        let restored: Thread = serde_json::from_str(&json).expect("should deserialize");
        assert!(restored.pending_auth.is_some());
        let pending = restored.pending_auth.unwrap();
        assert_eq!(pending.extension_name, "openai");
        assert!(!pending.is_expired());
    }

    #[test]
    fn test_pending_auth_expiry() {
        let mut pending = PendingAuth {
            extension_name: "test".to_string(),
            created_at: Utc::now(),
        };
        assert!(!pending.is_expired());
        // Backdate beyond the TTL
        pending.created_at = Utc::now() - AUTH_MODE_TTL - TimeDelta::seconds(1);
        assert!(pending.is_expired());
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

        let json = serde_json::to_string(&thread).unwrap();
        let restored: Thread = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.id, thread.id);
        assert_eq!(restored.session_id, thread.session_id);
        assert_eq!(restored.turns.len(), 1);
        assert_eq!(restored.turns[0].user_input, "hello");
        assert_eq!(restored.turns[0].response, Some("world".to_string()));
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
            display_parameters: serde_json::json!({"command": "rm -rf /"}),
            description: "dangerous command".to_string(),
            tool_call_id: "call_123".to_string(),
            context_messages: vec![ChatMessage::user("do it")],
            deferred_tool_calls: vec![],
            user_timezone: None,
            allow_always: false,
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
            display_parameters: serde_json::json!({}),
            description: "test".to_string(),
            tool_call_id: "call_456".to_string(),
            context_messages: vec![],
            deferred_tool_calls: vec![],
            user_timezone: None,
            allow_always: true,
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

    // Regression tests for #568: tool call history must survive hydration.

    #[test]
    fn test_messages_includes_tool_calls() {
        let mut thread = Thread::new(Uuid::new_v4());

        thread.start_turn("Search for X");
        {
            let turn = thread.turns.last_mut().unwrap();
            turn.record_tool_call("memory_search", serde_json::json!({"query": "X"}));
            turn.record_tool_result(serde_json::json!("Found X in doc.md"));
        }
        thread.complete_turn("I found X in doc.md.");

        let messages = thread.messages();
        // user + assistant_with_tool_calls + tool_result + assistant = 4
        assert_eq!(messages.len(), 4);

        assert_eq!(messages[0].role, crate::llm::Role::User);
        assert_eq!(messages[0].content, "Search for X");

        assert_eq!(messages[1].role, crate::llm::Role::Assistant);
        assert!(messages[1].tool_calls.is_some());
        let tcs = messages[1].tool_calls.as_ref().unwrap();
        assert_eq!(tcs.len(), 1);
        assert_eq!(tcs[0].name, "memory_search");

        assert_eq!(messages[2].role, crate::llm::Role::Tool);
        assert!(messages[2].content.contains("Found X"));

        assert_eq!(messages[3].role, crate::llm::Role::Assistant);
        assert_eq!(messages[3].content, "I found X in doc.md.");
    }

    #[test]
    fn test_messages_multiple_tool_calls_per_turn() {
        let mut thread = Thread::new(Uuid::new_v4());

        thread.start_turn("Do two things");
        {
            let turn = thread.turns.last_mut().unwrap();
            turn.record_tool_call("echo", serde_json::json!({"msg": "a"}));
            turn.record_tool_result(serde_json::json!("a"));
            turn.record_tool_call("time", serde_json::json!({}));
            turn.record_tool_error("timeout");
        }
        thread.complete_turn("Done.");

        let messages = thread.messages();
        // user + assistant_with_calls(2) + tool_result + tool_result + assistant = 5
        assert_eq!(messages.len(), 5);

        let tcs = messages[1].tool_calls.as_ref().unwrap();
        assert_eq!(tcs.len(), 2);

        // First tool: success
        assert_eq!(messages[2].content, "a");
        // Second tool: error (passed through directly, no wrapping)
        assert!(messages[3].content.contains("timeout"));
    }

    #[test]
    fn test_restore_from_messages_with_tool_calls() {
        let mut thread = Thread::new(Uuid::new_v4());

        // Build a message sequence with tool calls
        let tc = ToolCall {
            id: "call_0".to_string(),
            name: "search".to_string(),
            arguments: serde_json::json!({"q": "test"}),
        };
        let messages = vec![
            ChatMessage::user("Find test"),
            ChatMessage::assistant_with_tool_calls(None, vec![tc]),
            ChatMessage::tool_result("call_0", "search", "result: found"),
            ChatMessage::assistant("Found it."),
        ];

        thread.restore_from_messages(messages);

        assert_eq!(thread.turns.len(), 1);
        let turn = &thread.turns[0];
        assert_eq!(turn.user_input, "Find test");
        assert_eq!(turn.tool_calls.len(), 1);
        assert_eq!(turn.tool_calls[0].name, "search");
        assert_eq!(
            turn.tool_calls[0].result,
            Some(serde_json::Value::String("result: found".to_string()))
        );
        assert_eq!(turn.response, Some("Found it.".to_string()));
    }

    #[test]
    fn test_restore_from_messages_with_tool_error() {
        let mut thread = Thread::new(Uuid::new_v4());

        let tc = ToolCall {
            id: "call_0".to_string(),
            name: "http".to_string(),
            arguments: serde_json::json!({}),
        };
        let messages = vec![
            ChatMessage::user("Fetch URL"),
            ChatMessage::assistant_with_tool_calls(None, vec![tc]),
            ChatMessage::tool_result("call_0", "http", "Error: timeout"),
            ChatMessage::assistant("The request timed out."),
        ];

        thread.restore_from_messages(messages);

        // restore_from_messages stores all tool content as result (not error),
        // because it can't reliably distinguish errors from results that happen
        // to start with "Error: ". The content is preserved for LLM context.
        let turn = &thread.turns[0];
        assert_eq!(
            turn.tool_calls[0].result,
            Some(serde_json::Value::String("Error: timeout".to_string()))
        );
    }

    #[test]
    fn test_messages_round_trip_with_tools() {
        // Build a thread with tool calls, get messages(), restore, get messages() again
        // The two message sequences should be equivalent.
        let mut thread = Thread::new(Uuid::new_v4());

        thread.start_turn("Do search");
        {
            let turn = thread.turns.last_mut().unwrap();
            turn.record_tool_call("search", serde_json::json!({"q": "test"}));
            turn.record_tool_result(serde_json::json!("found"));
        }
        thread.complete_turn("Here are results.");

        let messages_original = thread.messages();

        // Restore into a new thread
        let mut thread2 = Thread::new(Uuid::new_v4());
        thread2.restore_from_messages(messages_original.clone());

        let messages_restored = thread2.messages();

        // Same number of messages
        assert_eq!(messages_original.len(), messages_restored.len());

        // Same roles
        for (orig, rest) in messages_original.iter().zip(messages_restored.iter()) {
            assert_eq!(orig.role, rest.role);
        }

        // Same final response
        assert_eq!(
            messages_original.last().unwrap().content,
            messages_restored.last().unwrap().content
        );
    }

    #[test]
    fn test_restore_multi_stage_tool_calls() {
        let mut thread = Thread::new(Uuid::new_v4());

        let tc1 = ToolCall {
            id: "call_a".to_string(),
            name: "search".to_string(),
            arguments: serde_json::json!({"q": "data"}),
        };
        let tc2 = ToolCall {
            id: "call_b".to_string(),
            name: "write".to_string(),
            arguments: serde_json::json!({"path": "out.txt"}),
        };
        let messages = vec![
            ChatMessage::user("Find and save"),
            ChatMessage::assistant_with_tool_calls(None, vec![tc1]),
            ChatMessage::tool_result("call_a", "search", "found data"),
            ChatMessage::assistant_with_tool_calls(None, vec![tc2]),
            ChatMessage::tool_result("call_b", "write", "written"),
            ChatMessage::assistant("Done, saved to out.txt"),
        ];

        thread.restore_from_messages(messages);

        assert_eq!(thread.turns.len(), 1);
        let turn = &thread.turns[0];
        assert_eq!(turn.tool_calls.len(), 2);
        assert_eq!(turn.tool_calls[0].name, "search");
        assert_eq!(turn.tool_calls[1].name, "write");
        assert_eq!(
            turn.tool_calls[0].result,
            Some(serde_json::Value::String("found data".to_string()))
        );
        assert_eq!(
            turn.tool_calls[1].result,
            Some(serde_json::Value::String("written".to_string()))
        );
        assert_eq!(turn.response, Some("Done, saved to out.txt".to_string()));
    }

    #[test]
    fn test_messages_truncates_large_tool_results() {
        let mut thread = Thread::new(Uuid::new_v4());

        thread.start_turn("Read big file");
        {
            let turn = thread.turns.last_mut().unwrap();
            turn.record_tool_call("read_file", serde_json::json!({"path": "big.txt"}));
            let big_result = "x".repeat(2000);
            turn.record_tool_result(serde_json::json!(big_result));
        }
        thread.complete_turn("Here's the file content.");

        let messages = thread.messages();
        let tool_result_content = &messages[2].content;
        assert!(
            tool_result_content.len() <= 1010,
            "Tool result should be truncated, got {} chars",
            tool_result_content.len()
        );
        assert!(tool_result_content.ends_with("..."));
    }

    #[test]
    fn test_thread_message_queue() {
        let mut thread = Thread::new(Uuid::new_v4());

        // Queue is initially empty
        assert!(thread.pending_messages.is_empty());
        assert!(thread.take_pending_message().is_none());

        // Queue messages and verify FIFO ordering
        assert!(thread.queue_message("first".to_string()));
        assert!(thread.queue_message("second".to_string()));
        assert!(thread.queue_message("third".to_string()));
        assert_eq!(thread.pending_messages.len(), 3);

        assert_eq!(thread.take_pending_message(), Some("first".to_string()));
        assert_eq!(thread.take_pending_message(), Some("second".to_string()));
        assert_eq!(thread.take_pending_message(), Some("third".to_string()));
        assert!(thread.take_pending_message().is_none());

        // Fill to capacity — all 10 should succeed
        for i in 0..MAX_PENDING_MESSAGES {
            assert!(thread.queue_message(format!("msg-{}", i)));
        }
        assert_eq!(thread.pending_messages.len(), MAX_PENDING_MESSAGES);

        // 11th message rejected by queue_message itself
        assert!(!thread.queue_message("overflow".to_string()));
        assert_eq!(thread.pending_messages.len(), MAX_PENDING_MESSAGES);

        // Drain and verify order
        for i in 0..MAX_PENDING_MESSAGES {
            assert_eq!(thread.take_pending_message(), Some(format!("msg-{}", i)));
        }
        assert!(thread.take_pending_message().is_none());
    }

    #[test]
    fn test_thread_message_queue_serialization() {
        let mut thread = Thread::new(Uuid::new_v4());

        // Empty queue should not appear in serialization (skip_serializing_if)
        let json = serde_json::to_string(&thread).unwrap();
        assert!(!json.contains("pending_messages"));

        // Non-empty queue should serialize and deserialize
        thread.queue_message("queued msg".to_string());
        let json = serde_json::to_string(&thread).unwrap();
        assert!(json.contains("pending_messages"));
        assert!(json.contains("queued msg"));

        let restored: Thread = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.pending_messages.len(), 1);
        assert_eq!(restored.pending_messages[0], "queued msg");
    }

    #[test]
    fn test_thread_message_queue_default_on_old_data() {
        // Deserialization of old data without pending_messages should default to empty
        let thread = Thread::new(Uuid::new_v4());
        let json = serde_json::to_string(&thread).unwrap();

        // The field is absent (skip_serializing_if), simulating old data
        assert!(!json.contains("pending_messages"));
        let restored: Thread = serde_json::from_str(&json).unwrap();
        assert!(restored.pending_messages.is_empty());
    }

    #[test]
    fn test_interrupt_clears_pending_messages() {
        let mut thread = Thread::new(Uuid::new_v4());

        // Start a turn so there's something to interrupt
        thread.start_turn("initial input");

        // Queue several messages while "processing"
        thread.queue_message("queued-1".to_string());
        thread.queue_message("queued-2".to_string());
        thread.queue_message("queued-3".to_string());
        assert_eq!(thread.pending_messages.len(), 3);

        // Interrupt should clear the queue
        thread.interrupt();
        assert!(thread.pending_messages.is_empty());
        assert_eq!(thread.state, ThreadState::Interrupted);
    }

    #[test]
    fn test_thread_state_idle_after_full_drain() {
        let mut thread = Thread::new(Uuid::new_v4());

        // Simulate a full drain cycle: start turn, queue messages, complete turn,
        // then drain all queued messages as a single merged turn (#259).
        thread.start_turn("turn 1");
        assert_eq!(thread.state, ThreadState::Processing);

        thread.queue_message("queued-a".to_string());
        thread.queue_message("queued-b".to_string());

        // Complete the turn (simulates process_user_input finishing)
        thread.complete_turn("response 1");
        assert_eq!(thread.state, ThreadState::Idle);

        // Drain: merge all queued messages and process as a single turn
        let merged = thread.drain_pending_messages().unwrap();
        assert_eq!(merged, "queued-a\nqueued-b");
        thread.start_turn(&merged);
        thread.complete_turn("response for merged");

        // Queue is fully drained, thread is idle
        assert!(thread.drain_pending_messages().is_none());
        assert!(thread.pending_messages.is_empty());
        assert_eq!(thread.state, ThreadState::Idle);
    }

    #[test]
    fn test_drain_pending_messages_merges_with_newlines() {
        let mut thread = Thread::new(Uuid::new_v4());

        // Empty queue returns None
        assert!(thread.drain_pending_messages().is_none());

        // Single message returned as-is (no trailing newline)
        thread.queue_message("only one".to_string());
        assert_eq!(
            thread.drain_pending_messages(),
            Some("only one".to_string()),
        );
        assert!(thread.pending_messages.is_empty());

        // Multiple messages joined with newlines
        thread.queue_message("hey".to_string());
        thread.queue_message("can you check the server".to_string());
        thread.queue_message("it started 10 min ago".to_string());
        assert_eq!(
            thread.drain_pending_messages(),
            Some("hey\ncan you check the server\nit started 10 min ago".to_string()),
        );
        assert!(thread.pending_messages.is_empty());

        // Queue is empty after drain
        assert!(thread.drain_pending_messages().is_none());
    }

    #[test]
    fn test_requeue_drained_preserves_content_at_front() {
        let mut thread = Thread::new(Uuid::new_v4());

        // Re-queue into empty queue
        thread.requeue_drained("failed batch".to_string());
        assert_eq!(thread.pending_messages.len(), 1);
        assert_eq!(thread.pending_messages[0], "failed batch");

        // New messages go behind the re-queued content
        thread.queue_message("new msg".to_string());
        assert_eq!(thread.pending_messages.len(), 2);

        // Drain should return re-queued content first (front of queue)
        let merged = thread.drain_pending_messages().unwrap();
        assert_eq!(merged, "failed batch\nnew msg");
    }
}
