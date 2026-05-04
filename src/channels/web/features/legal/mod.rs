//! Legal harness skill — projects, documents, and (in companion PRs)
//! chats with DOCX export.
//!
//! This module owns the **foundation** layer: project + document CRUD,
//! multipart upload with inline PDF/DOCX text extraction, and
//! content-addressed blob storage on the local filesystem. Streams B
//! (chat-with-docs) and C (DOCX export) are landing in companion PRs and
//! consume the same migration introduced here
//! (`migrations/V26__legal_harness.sql` + the libSQL incremental
//! migration with the matching version).
//!
//! See `skills/legal/SKILL.md` for the agent-facing description and
//! `legal-harness-spec.md` for the cross-stream coordinator.

pub mod blobs;
pub mod extract;
pub mod models;

// Storage and HTTP handlers depend on libSQL-specific types (the libsql
// crate is feature-gated). Postgres-only builds compile the data
// model/blob/extract paths but the network surface stays disabled until
// the Postgres query layer lands.
#[cfg(feature = "libsql")]
pub mod handlers;
#[cfg(feature = "libsql")]
pub mod store;
