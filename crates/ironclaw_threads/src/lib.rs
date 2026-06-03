//! Canonical session thread and transcript contracts for IronClaw Reborn.
//!
//! This crate owns the contract-first boundary for canonical Reborn threads and
//! transcript history. It provides an in-memory service for semantic tests and
//! a filesystem-backed durable service routed through `ironclaw_filesystem`.
//! Backend selection (libSQL, PostgreSQL, in-memory, local-disk) is made at
//! the `RootFilesystem` layer — the consumer-store level no longer carries
//! per-backend impls. See
//! `docs/plans/2026-05-16-scoped-filesystem-tenant-isolation.md`.
#![warn(unreachable_pub)]

mod capability_display_preview;
mod contract;
mod error;
mod filesystem_service;
mod identifiers;
mod in_memory;
mod service;
mod summary_artifacts;
mod tool_result_reference;

pub use filesystem_service::FilesystemSessionThreadService;

pub use capability_display_preview::{
    CapabilityDisplayPreviewEnvelope, CapabilityDisplayPreviewEnvelopeInput,
    CapabilityDisplayPreviewStatus,
};
pub use contract::{
    AcceptInboundMessageRequest, AcceptedInboundMessage, AcceptedInboundMessageReplay,
    AppendAssistantDraftRequest, AppendCapabilityDisplayPreviewRequest,
    AppendToolResultReferenceRequest, ContextMessage, ContextMessages, ContextWindow,
    CreateSummaryArtifactRequest, EnsureThreadRequest, GOAL_STATEMENT_MAX_CHARS, GoalStatement,
    LatestThreadMessageRequest, ListThreadsForScopeRequest, ListThreadsForScopeResponse,
    LoadContextMessagesRequest, LoadContextWindowRequest, MessageContent, MessageKind,
    MessageStatus, RedactMessageRequest, ReplayAcceptedInboundMessageRequest, SessionThreadRecord,
    SummaryArtifact, SummaryKind, SummaryModelContextPolicy, ThreadGoal, ThreadHistory,
    ThreadHistoryRequest, ThreadMessageRange, ThreadMessageRangeRequest, ThreadMessageRecord,
    ThreadScope, UpdateAssistantDraftRequest, UpdateThreadGoalRequest,
    UpdateToolResultReferenceRequest,
};
pub use error::SessionThreadError;
pub use identifiers::{SummaryArtifactId, ThreadMessageId};
pub use in_memory::InMemorySessionThreadService;
pub use service::SessionThreadService;
pub use tool_result_reference::{
    ProviderToolCallReferenceEnvelope, ToolResultReferenceEnvelope, ToolResultSafeSummary,
};
