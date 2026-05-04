//! Legal harness — chat-with-documents subsystem.
//!
//! Stream B owns the chat layer of the legal harness v1: project chat
//! threads, message history, RAG-based assistant replies streamed back to
//! the client over SSE. Stream A owns the foundation (projects + document
//! upload + extraction); Stream C owns DOCX export. The same database
//! tables (`legal_projects`, `legal_documents`, `legal_chats`,
//! `legal_chat_messages`) are shared across all three streams per the
//! canonical migration.
//!
//! The HTTP surface lives under `channels/web/features/legal/`. This
//! module owns the storage trait + libSQL implementation only.

pub mod store;

pub use store::{
    LegalChat, LegalChatMessage, LegalDocumentText, LegalProjectMeta, LegalRole, LegalStore,
};

#[cfg(feature = "libsql")]
pub use store::LibSqlLegalStore;
