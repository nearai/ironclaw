//! Core type definitions for the engine.
//!
//! All data structures live here. No async, no I/O — just types and
//! validation logic.

pub mod capability;
pub mod conversation;
pub mod error;
pub mod event;
pub mod memory;
pub mod message;
pub mod mission;
pub mod project;
pub mod provenance;
pub mod step;
pub mod thread;

/// Default user_id for backwards-compatible deserialization of records
/// created before multi-tenant isolation was added.
pub(crate) fn default_user_id() -> String {
    "legacy".to_string()
}
