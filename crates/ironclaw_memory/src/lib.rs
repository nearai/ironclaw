//! Provider-neutral memory contract types for IronClaw Reborn.
//!
//! This crate owns the host-facing IronClaw memory vocabulary: the
//! [`MemoryService`] trait and its operation shapes, the memory document
//! scope/path value types, prompt-write-safety vocabulary, and memory
//! significant-event/audit contracts. The native provider implementation, the
//! prompt-write-safety enforcement engine, and storage adapters live in the
//! `ironclaw_memory_native` provider crate, which depends on this crate and
//! re-exports these types for backward compatibility.

mod context;
mod events;
mod hash;
mod metadata;
mod path;
mod safety;
mod service;

pub use context::MemoryContext;
pub use events::{
    MemoryAuditContext, MemoryEventSinkError, MemorySignificantEvent, MemorySignificantEventKind,
    MemorySignificantEventSink, MemorySignificantEventSource, MemorySignificantEventStatus,
};
pub use hash::{content_bytes_sha256, content_sha256};
pub use metadata::{CONFIG_FILE_NAME, DocumentMetadata, HygieneMetadata};
pub use path::{
    MemoryDocumentPath, MemoryDocumentScope, validated_memory_relative_path,
    validated_memory_segment,
};
pub use safety::{
    DEFAULT_PROMPT_PROTECTED_PATHS, PromptProtectedPathClass, PromptProtectedPathRegistry,
    PromptSafetyAllowanceId, PromptSafetyPolicyVersion, PromptSafetyReason, PromptSafetyReasonCode,
    PromptSafetySeverity, PromptSafetySummary, PromptWriteOperation, PromptWriteSafetyDecision,
    PromptWriteSafetyError, PromptWriteSafetyEvent, PromptWriteSafetyEventKind,
    PromptWriteSafetyEventSink, PromptWriteSafetyPolicy, PromptWriteSafetyRequest,
    PromptWriteSource,
};
pub use service::{
    MEMORY_DISABLED_CONTEXT_ALIASES, MemoryContextProfileId, MemoryInteractionMessage,
    MemoryInteractionRole, MemoryInvocation, MemoryProfileSetStatus, MemoryService,
    MemoryServiceContextRequest, MemoryServiceContextSnippet, MemoryServiceError,
    MemoryServiceErrorKind, MemoryServiceProfileReadResponse, MemoryServiceProfileSetRequest,
    MemoryServiceProfileSetResponse, MemoryServiceReadRequest, MemoryServiceReadResponse,
    MemoryServiceRecordRequest, MemoryServiceRecordResponse, MemoryServiceSearchRequest,
    MemoryServiceSearchResponse, MemoryServiceSearchResult, MemoryServiceTreeRequest,
    MemoryServiceTreeResponse, MemoryServiceWriteRequest, MemoryServiceWriteResponse,
    MemoryWriteStatus, memory_context_disabled,
};
