//! Memory document filesystem adapters for IronClaw Reborn.
//!
//! This crate owns memory-specific path grammar and repository seams. The
//! generic filesystem crate owns only virtual path authority, scoped mounts,
//! backend cataloging, and backend routing.

mod backend;
mod chunking;
mod embedding;
mod events;
mod filesystem;
mod indexer;
mod metadata;
mod path;
mod repo;
mod safety;
mod schema;
mod search;
mod services;

pub use backend::{
    MemoryBackend, MemoryBackendCapabilities, MemoryContext, RepositoryMemoryBackend,
};
pub use chunking::{ChunkConfig, MemoryChunkWrite, chunk_document, content_sha256};
pub use embedding::{EmbeddingError, EmbeddingProvider};
pub use events::{
    MemoryAuditContext, MemoryEventSinkError, MemorySignificantEvent, MemorySignificantEventKind,
    MemorySignificantEventSink, MemorySignificantEventSource, MemorySignificantEventStatus,
};
pub use filesystem::{MemoryBackendFilesystemAdapter, MemoryDocumentFilesystem};
pub use indexer::{
    ChunkingMemoryDocumentIndexer, MemoryChunkReplaceOutcome, MemoryDocumentIndexRepository,
    MemoryDocumentIndexer,
};
pub use metadata::{CONFIG_FILE_NAME, DocumentMetadata, HygieneMetadata, MemoryWriteOptions};
pub use path::{MemoryDocumentPath, MemoryDocumentScope};
pub use repo::{
    FilesystemMemoryDocumentRepository, InMemoryMemoryDocumentRepository, MemoryAppendOutcome,
    MemoryDocumentRepository,
};
pub use safety::{
    DEFAULT_PROMPT_PROTECTED_PATHS, DefaultPromptWriteSafetyPolicy, PromptProtectedPathClass,
    PromptProtectedPathRegistry, PromptSafetyAllowanceId, PromptSafetyPolicyVersion,
    PromptSafetyReason, PromptSafetyReasonCode, PromptSafetySeverity, PromptSafetySummary,
    PromptWriteOperation, PromptWriteSafetyDecision, PromptWriteSafetyError,
    PromptWriteSafetyEvent, PromptWriteSafetyEventKind, PromptWriteSafetyEventSink,
    PromptWriteSafetyPolicy, PromptWriteSafetyRequest, PromptWriteSource,
};
pub use search::{FusionStrategy, MemorySearchRequest, MemorySearchResult};
pub use services::{
    MemoryAppendDocumentRequest, MemoryBootstrapClearOutcome, MemoryBootstrapClearRequest,
    MemoryDocumentEntry, MemoryDocumentRecord, MemoryDocumentService, MemoryLayerService,
    MemoryLayerWriteMode, MemoryLayerWriteOutcome, MemoryLayerWriteRequest,
    MemoryListDocumentsRequest, MemoryPatchDocumentOutcome, MemoryPatchDocumentRequest,
    MemoryProductSearchHit, MemoryProductSearchRequest, MemoryProfileService,
    MemoryProfileSyncOutcome, MemoryProfileSyncRequest, MemoryPromptWriteSafetyDecision,
    MemoryPromptWriteSafetyPolicy, MemoryPromptWriteSafetyRequest, MemoryReadDocumentRequest,
    MemorySearchGroupContext, MemorySearchService, MemorySeedService, MemoryServiceError,
    MemoryServiceErrorCode, MemoryStatus, MemoryStatusRequest, MemoryTreeRequest,
    MemoryVersionListRequest, MemoryVersionReadRequest, MemoryVersionRecord, MemoryVersionService,
    MemoryVersionSummary, MemoryWriteActor, MemoryWriteAuthority, MemoryWriteDocumentOutcome,
    MemoryWriteDocumentRequest, MemoryWritePurpose, MemoryWriteSurface,
};
