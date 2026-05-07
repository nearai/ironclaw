//! Canonical session thread and transcript contracts for IronClaw Reborn.
//!
//! This crate owns the contract-first boundary for canonical Reborn threads and
//! transcript history. It deliberately starts with an in-memory service so caller
//! tests can lock semantics before PostgreSQL/libSQL adapters are introduced.

mod contract;
mod error;
mod identifiers;
mod in_memory;
mod service;

pub use contract::{
    AcceptInboundMessageRequest, AcceptedInboundMessage, AppendAssistantDraftRequest,
    ContextMessage, ContextWindow, CreateSummaryArtifactRequest, EnsureThreadRequest,
    LoadContextWindowRequest, MessageContent, MessageKind, MessageStatus, RedactMessageRequest,
    SessionThreadRecord, SummaryArtifact, ThreadHistory, ThreadHistoryRequest, ThreadMessageRecord,
    ThreadScope, UpdateAssistantDraftRequest,
};
pub use error::SessionThreadError;
pub use identifiers::{SummaryArtifactId, ThreadMessageId};
pub use in_memory::InMemorySessionThreadService;
pub use service::SessionThreadService;
