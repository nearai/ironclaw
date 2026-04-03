//! Conversation surface — the UI layer, separate from execution.
//!
//! A conversation is a stream of entries visible to the user. Threads
//! (the execution units) run independently and produce entries that
//! appear in conversations. One conversation can have multiple active
//! threads; one thread can outlive its originating conversation.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::types::thread::ThreadId;

/// Strongly-typed conversation identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ConversationId(pub Uuid);

impl ConversationId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for ConversationId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for ConversationId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Strongly-typed entry identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EntryId(pub Uuid);

impl EntryId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for EntryId {
    fn default() -> Self {
        Self::new()
    }
}

/// Who sent a conversation entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EntrySender {
    /// The human user.
    User,
    /// The agent (from a specific thread).
    Agent { thread_id: ThreadId },
    /// System notification (thread started, completed, etc.).
    System,
}

/// A single entry in a conversation — a message visible to the user.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationEntry {
    pub id: EntryId,
    pub sender: EntrySender,
    pub content: String,
    /// Which thread produced this entry (if any).
    pub origin_thread_id: Option<ThreadId>,
    pub timestamp: DateTime<Utc>,
    /// Optional metadata (channel-specific formatting, attachments, etc.).
    pub metadata: serde_json::Value,
}

impl ConversationEntry {
    /// Create a user entry.
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            id: EntryId::new(),
            sender: EntrySender::User,
            content: content.into(),
            origin_thread_id: None,
            timestamp: Utc::now(),
            metadata: serde_json::Value::Null,
        }
    }

    /// Create an agent entry from a thread.
    pub fn agent(thread_id: ThreadId, content: impl Into<String>) -> Self {
        Self {
            id: EntryId::new(),
            sender: EntrySender::Agent { thread_id },
            content: content.into(),
            origin_thread_id: Some(thread_id),
            timestamp: Utc::now(),
            metadata: serde_json::Value::Null,
        }
    }

    /// Create a system notification entry.
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            id: EntryId::new(),
            sender: EntrySender::System,
            content: content.into(),
            origin_thread_id: None,
            timestamp: Utc::now(),
            metadata: serde_json::Value::Null,
        }
    }

    /// Create a system notification linked to a thread.
    pub fn system_for_thread(thread_id: ThreadId, content: impl Into<String>) -> Self {
        Self {
            id: EntryId::new(),
            sender: EntrySender::System,
            content: content.into(),
            origin_thread_id: Some(thread_id),
            timestamp: Utc::now(),
            metadata: serde_json::Value::Null,
        }
    }
}

/// A conversation surface — the UI-facing view of a chat.
///
/// Conversations are NOT execution boundaries. They are streams of entries
/// that may come from multiple concurrent threads. A user can start a new
/// thread while another is still running, and both produce entries in the
/// same conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversationSurface {
    pub id: ConversationId,
    /// Which channel this conversation is on (e.g. "telegram", "web", "cli").
    pub channel: String,
    /// The user who owns this conversation.
    pub user_id: String,
    /// All entries in chronological order.
    pub entries: Vec<ConversationEntry>,
    /// Currently active (non-terminal) thread IDs.
    pub active_threads: Vec<ThreadId>,
    /// Metadata (channel-specific state, external thread IDs, etc.).
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl ConversationSurface {
    pub fn new(channel: impl Into<String>, user_id: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            id: ConversationId::new(),
            channel: channel.into(),
            user_id: user_id.into(),
            entries: Vec::new(),
            active_threads: Vec::new(),
            metadata: serde_json::Value::Null,
            created_at: now,
            updated_at: now,
        }
    }

    /// Add an entry and update the timestamp.
    pub fn add_entry(&mut self, entry: ConversationEntry) {
        self.entries.push(entry);
        self.updated_at = Utc::now();
    }

    /// Register a thread as active in this conversation.
    pub fn track_thread(&mut self, thread_id: ThreadId) {
        if !self.active_threads.contains(&thread_id) {
            self.active_threads.push(thread_id);
        }
    }

    /// Remove a thread from the active list (it completed or failed).
    pub fn untrack_thread(&mut self, thread_id: ThreadId) {
        self.active_threads.retain(|id| *id != thread_id);
    }

    /// Get the most recent entry, if any.
    pub fn last_entry(&self) -> Option<&ConversationEntry> {
        self.entries.last()
    }

    /// Get all entries from a specific thread.
    pub fn entries_for_thread(&self, thread_id: ThreadId) -> Vec<&ConversationEntry> {
        self.entries
            .iter()
            .filter(|e| e.origin_thread_id == Some(thread_id))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversation_lifecycle() {
        let mut conv = ConversationSurface::new("telegram", "user_123");
        assert!(conv.entries.is_empty());
        assert!(conv.active_threads.is_empty());

        // User sends a message
        conv.add_entry(ConversationEntry::user("Hello!"));
        assert_eq!(conv.entries.len(), 1);

        // Thread starts
        let tid = ThreadId::new();
        conv.track_thread(tid);
        conv.add_entry(ConversationEntry::system_for_thread(tid, "Thread started"));
        assert_eq!(conv.active_threads.len(), 1);

        // Agent responds
        conv.add_entry(ConversationEntry::agent(tid, "Hi there!"));
        assert_eq!(conv.entries.len(), 3);

        // Thread completes
        conv.untrack_thread(tid);
        conv.add_entry(ConversationEntry::system_for_thread(
            tid,
            "Thread completed",
        ));
        assert!(conv.active_threads.is_empty());
        assert_eq!(conv.entries.len(), 4);
    }

    #[test]
    fn multiple_concurrent_threads() {
        let mut conv = ConversationSurface::new("web", "user_456");

        let t1 = ThreadId::new();
        let t2 = ThreadId::new();

        conv.track_thread(t1);
        conv.track_thread(t2);
        assert_eq!(conv.active_threads.len(), 2);

        conv.add_entry(ConversationEntry::agent(t1, "Research result A"));
        conv.add_entry(ConversationEntry::agent(t2, "Research result B"));
        conv.add_entry(ConversationEntry::agent(t1, "More from A"));

        let t1_entries = conv.entries_for_thread(t1);
        assert_eq!(t1_entries.len(), 2);

        let t2_entries = conv.entries_for_thread(t2);
        assert_eq!(t2_entries.len(), 1);

        conv.untrack_thread(t1);
        assert_eq!(conv.active_threads.len(), 1);
    }

    #[test]
    fn track_thread_is_idempotent() {
        let mut conv = ConversationSurface::new("cli", "user");
        let tid = ThreadId::new();
        conv.track_thread(tid);
        conv.track_thread(tid);
        assert_eq!(conv.active_threads.len(), 1);
    }
}
