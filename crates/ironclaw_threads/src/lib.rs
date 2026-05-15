//! Canonical session thread and transcript contracts for IronClaw Reborn.
//!
//! This crate owns the contract-first boundary for canonical Reborn threads and
//! transcript history. It provides an in-memory service for semantic tests and
//! feature-gated PostgreSQL/libSQL services for durable Reborn composition.
#![warn(unreachable_pub)]

mod contract;
#[cfg(any(feature = "libsql", feature = "postgres"))]
mod db;
mod error;
mod identifiers;
mod in_memory;
mod service;

#[cfg(feature = "libsql")]
pub use db::LibSqlSessionThreadService;
#[cfg(feature = "postgres")]
pub use db::PostgresSessionThreadService;

pub use contract::{
    AcceptInboundMessageRequest, AcceptedInboundMessage, AcceptedInboundMessageReplay,
    AppendAssistantDraftRequest, ContextMessage, ContextWindow, CreateSummaryArtifactRequest,
    EnsureThreadRequest, LoadContextWindowRequest, MessageContent, MessageKind, MessageStatus,
    RedactMessageRequest, ReplayAcceptedInboundMessageRequest, SessionThreadRecord,
    SummaryArtifact, ThreadHistory, ThreadHistoryRequest, ThreadMessageRecord, ThreadScope,
    UpdateAssistantDraftRequest,
};
pub use error::SessionThreadError;
pub use identifiers::ThreadMessageId;
pub use in_memory::InMemorySessionThreadService;
pub use service::SessionThreadService;
