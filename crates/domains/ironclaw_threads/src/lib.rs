//! Canonical session thread and transcript contracts for IronClaw Reborn.
//!
//! This crate owns the contract-first boundary for canonical Reborn threads and
//! transcript history. It provides an in-memory service for semantic tests and
//! a filesystem-backed durable service routed through `ironclaw_filesystem`.
//! Backend selection (libSQL, PostgreSQL, in-memory, local-disk) is made at
//! the `RootFilesystem` layer — the consumer-store level no longer carries
//! per-backend impls. See
//! `docs/internal/plans/2026-05-16-scoped-filesystem-tenant-isolation.md`.
#![warn(unreachable_pub)]

mod attachment_context;
mod capability_display_preview;
mod contract;
mod error;
mod filesystem_service;
mod identifiers;
mod in_memory;
mod prepared_context;
mod service;
mod stored_message;
mod structured_finalization;
mod subagent_result;
mod summary_artifacts;
mod title;
mod tool_result_records;
mod tool_result_reference;

pub use filesystem_service::FilesystemSessionThreadService;
// `title::derive_thread_title` is deliberately NOT re-exported here —
// it is an internal helper consumed only by the two backend impls in
// this crate, and keeping it off the public surface avoids committing
// to it via semver. Sibling modules import directly through
// `crate::title::derive_thread_title`.

pub use attachment_context::deproject_model_attachment_context;
pub use capability_display_preview::{
    CapabilityDisplayPreviewEnvelope, CapabilityDisplayPreviewEnvelopeInput,
    CapabilityDisplayPreviewStatus,
};
pub use contract::{
    AcceptInboundMessageRequest, AcceptSubagentResultRequest, AcceptedInboundMessage,
    AcceptedInboundMessageReplay, AcceptedSubagentResult, AppendAssistantDraftRequest,
    AppendCapabilityDisplayPreviewRequest, AppendFinalizedAssistantMessageRequest,
    AppendToolResultReferenceRequest, BoundedThreadMessageSnapshot, BoundedThreadMessages,
    BoundedThreadMessagesRequest, ContextImageAttachment, ContextMessage, ContextMessages,
    ContextWindow, ContextWindowTruncation, CreateSummaryArtifactRequest,
    DeleteToolResultRecordRequest, EnsureThreadRequest, FinalizedAssistantMessageByRunRequest,
    GOAL_STATEMENT_MAX_CHARS, GoalStatement, InboundMessageReplayMetadata,
    LatestThreadMessageRequest, ListThreadsForScopeRequest, ListThreadsForScopeResponse,
    LoadContextMessagesRequest, LoadContextWindowRequest, MessageContent, MessageKind,
    MessageStatus, PublishStructuredFinalizationMessageRequest, PutToolResultRecordRequest,
    ReadToolResultRecordRequest, RedactMessageRequest, ReplayAcceptedInboundMessageRequest,
    SessionThreadRecord, SummaryArtifact, SummaryContextMode, SummaryKind,
    SummaryModelContextPolicy, TOOL_RESULT_RECORD_READ_MAX_BYTES, ThreadGoal, ThreadHistory,
    ThreadHistoryRequest, ThreadMessageRange, ThreadMessageRangeRequest, ThreadMessageRecord,
    ThreadScope, ToolResultRecordChunk, ToolResultRecordRead, ToolResultRecordSelection,
    UpdateAssistantDraftRequest, UpdateThreadGoalRequest, UpdateToolResultRecordRequest,
    UpdateToolResultReferenceRequest, effective_tool_result_read_max_bytes,
};
pub use error::SessionThreadError;
pub use identifiers::{SummaryArtifactId, ThreadMessageId};
pub use in_memory::InMemorySessionThreadService;
pub use prepared_context::{
    AcceptedPreparedContext, PREPARED_CONTEXT_METADATA_MARKER_KEY,
    PREPARED_CONTEXT_RECORD_SCHEMA_VERSION, PREPARED_OUTPUT_SCHEMA_MAX_BYTES,
    PREPARED_OUTPUT_SCHEMA_MAX_DEPTH, PREPARED_SEED_PROVIDER_ID, PreparedContextRecord,
    PreparedContextRequest, ThreadServicePreparedContextSource, read_declarations_for_run_scope,
    record_is_prepared_context_hidden, validate_output_contract, validate_output_schema,
    validate_prepared_seed_content,
};
pub use structured_finalization::{
    PutStructuredFinalizationRequest, ReadStructuredFinalizationRequest,
    StructuredFinalizationAccounting, StructuredFinalizationRecord, StructuredFinalizationUsage,
};
pub use tool_result_records::{
    TOOL_RESULT_JSON_DEFAULT_LIMIT, TOOL_RESULT_JSON_MAX_LIMIT, ToolResultRecordReadError,
    model_result_preview_from_json_page, render_json_tool_result_page,
};
// The attachment vocabulary lives in `ironclaw_common` (next to `AttachmentKind`
// and `IncomingAttachment`); re-exposed here so transcript-contract consumers
// reach `AttachmentRef` through this crate without a direct `ironclaw_common`
// dependency.
pub use ironclaw_common::{AttachmentKind, AttachmentRef};
// The accept door's own request type (`PreparedContextRequest.messages`) is
// `Vec<AgentMessage>`; re-exposed so accept-door callers build seed messages
// in the door's own vocabulary rather than reaching past it into `ironclaw_llm`.
pub use ironclaw_llm::agent_message;
pub use service::SessionThreadService;
pub use subagent_result::FramedSubagentText;
pub use tool_result_reference::{
    ProviderToolCallReferenceEnvelope, ToolResultIntrinsicOutcome, ToolResultReferenceEnvelope,
    ToolResultSafeSummary,
};
