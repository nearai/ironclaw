//! Legal-harness v1: chat-with-legal-documents skill.
//!
//! This module owns the persistence layer and the DOCX rendering pipeline
//! for the legal harness — the `/skills/legal/...` HTTP surface lives in
//! [`crate::channels::web::features::legal`] and calls into here.
//!
//! v1 split across three streams:
//!
//! - **Stream A — foundation**: project + document layer, schema migration,
//!   skill manifest. Owns the canonical migration that introduces the
//!   `legal_projects`, `legal_documents`, `legal_chats`, and
//!   `legal_chat_messages` tables.
//! - **Stream B — chat-with-docs**: chat creation, RAG, SSE streaming.
//! - **Stream C — DOCX export**: this stream. Renders an existing chat
//!   thread to a `.docx` byte stream so the user can hand the dialogue
//!   off to a colleague or attach it to a matter file.
//!
//! Every stream carries an identical copy of the migration so each PR is
//! independently testable; the canonical schema lives in the shared spec
//! and must not diverge between branches.
//!
//! The module is gated on the `libsql` feature: ironclaw uses libSQL as
//! its embedded data store for the legal harness per the shared spec, and
//! Stream C consumes that same backend directly. A postgres-only build
//! does not expose this module — adding postgres support is a v2 follow-up.
//!
//! # Layout
//!
//! - [`store`] — minimal libSQL reads needed for the export endpoint
//!   (chat header + messages with `document_refs`). Stream A and Stream B
//!   own the wider CRUD surface; Stream C only needs to *read* a chat to
//!   render it.
//! - [`docx`] — pure DOCX writer. Takes a [`ChatExport`] and returns
//!   `Vec<u8>` containing a valid OOXML zip. No I/O, no async — easy to
//!   unit-test in isolation.
//! - Top-level types ([`ChatExport`], [`ChatMessage`], [`ChatRole`]) are
//!   the shared shape that crosses the store/render boundary.

#![cfg(feature = "libsql")]

pub mod docx;
pub mod store;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Snapshot of a single chat thread, ready for DOCX rendering.
///
/// Built by [`store::LegalChatStore::load_chat_for_export`] in production
/// and constructed directly in tests so the DOCX writer can be exercised
/// without standing up a database.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatExport {
    /// Chat row id (TEXT primary key, ulid in production).
    pub id: String,
    /// Optional human-readable title; falls back to "Chat <id>" in the
    /// rendered DOCX when absent.
    pub title: Option<String>,
    /// Unix timestamp when the chat was created.
    pub created_at: DateTime<Utc>,
    /// Messages in chronological order (oldest first). Roles are
    /// constrained at the schema level to one of
    /// `user|assistant|system|tool` — see the canonical migration.
    pub messages: Vec<ChatMessage>,
}

/// One turn in a chat thread.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub id: String,
    pub role: ChatRole,
    pub content: String,
    /// Filenames of any documents this turn referenced. Resolved from
    /// `legal_chat_messages.document_refs` (a JSON array of
    /// `legal_documents.id` values) joined against `legal_documents`.
    /// Empty when the turn referenced no documents.
    pub document_refs: Vec<String>,
    pub created_at: DateTime<Utc>,
}

/// Allowed roles per the canonical schema's `CHECK` constraint. The
/// migration enforces these values in SQL — anything else surfaces as
/// [`LegalError::UnknownRole`] when the export reads a row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChatRole {
    User,
    Assistant,
    System,
    Tool,
}

impl ChatRole {
    /// Display label used as the heading text for each message in the
    /// rendered DOCX. Capitalized for visual scan-ability — the underlying
    /// row stores a lowercase token.
    pub fn label(self) -> &'static str {
        match self {
            ChatRole::User => "User",
            ChatRole::Assistant => "Assistant",
            ChatRole::System => "System",
            ChatRole::Tool => "Tool",
        }
    }

    pub(crate) fn from_db(s: &str) -> Result<Self, LegalError> {
        match s {
            "user" => Ok(ChatRole::User),
            "assistant" => Ok(ChatRole::Assistant),
            "system" => Ok(ChatRole::System),
            "tool" => Ok(ChatRole::Tool),
            other => Err(LegalError::UnknownRole(other.to_string())),
        }
    }
}

/// Errors surfaced by the legal-harness store + renderer.
///
/// The HTTP layer maps these to status codes:
/// `ChatNotFound → 404`, `ChatEmpty → 400`, everything else → 500.
#[derive(Debug, thiserror::Error)]
pub enum LegalError {
    /// No row in `legal_chats` matches the requested id.
    #[error("legal chat {0} not found")]
    ChatNotFound(String),
    /// The chat exists but has no messages — exporting it would produce a
    /// blank document, which is almost certainly a caller error.
    #[error("legal chat {0} has no messages")]
    ChatEmpty(String),
    /// A row's `role` column did not match the canonical CHECK
    /// constraint. The column is constrained at the schema level so this
    /// should be unreachable in practice; surface it cleanly anyway so
    /// migration-mismatch bugs do not panic the gateway.
    #[error("legal chat message has unknown role '{0}'")]
    UnknownRole(String),
    /// `document_refs` failed to parse as a JSON array of strings. The
    /// schema stores it as a TEXT column without enforcing the shape;
    /// readers must tolerate corruption rather than abort.
    #[error("legal chat message has malformed document_refs: {0}")]
    MalformedDocumentRefs(String),
    /// libSQL surfaced an error while reading the chat or messages.
    #[error("legal store database error: {0}")]
    Database(String),
    /// The DOCX writer failed to produce a valid OOXML payload.
    #[error("legal docx render error: {0}")]
    Render(String),
}
