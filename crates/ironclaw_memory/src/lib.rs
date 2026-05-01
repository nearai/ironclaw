//! Memory document filesystem adapters for IronClaw Reborn.
//!
//! This crate owns memory-specific path grammar and repository seams. The
//! generic filesystem crate owns only virtual path authority, scoped mounts,
//! backend cataloging, and backend routing.

mod backend;
mod chunking;
mod embedding;
mod filesystem;
mod indexer;
mod metadata;
mod path;
mod repo;
mod safety;
mod schema;
mod search;

pub use backend::{
    MemoryBackend, MemoryBackendCapabilities, MemoryContext, RepositoryMemoryBackend,
};
pub use chunking::{ChunkConfig, MemoryChunkWrite, chunk_document, content_sha256};
pub use embedding::{EmbeddingError, EmbeddingProvider};
pub use filesystem::{MemoryBackendFilesystemAdapter, MemoryDocumentFilesystem};
pub use indexer::{
    ChunkingMemoryDocumentIndexer, MemoryDocumentIndexRepository, MemoryDocumentIndexer,
};
pub use metadata::{CONFIG_FILE_NAME, DocumentMetadata, HygieneMetadata, MemoryWriteOptions};
pub use path::{MemoryDocumentPath, MemoryDocumentScope};
#[cfg(feature = "libsql")]
pub use repo::LibSqlMemoryDocumentRepository;
#[cfg(feature = "postgres")]
pub use repo::PostgresMemoryDocumentRepository;
pub use repo::{InMemoryMemoryDocumentRepository, MemoryAppendOutcome, MemoryDocumentRepository};
pub use safety::{
    DefaultPromptWriteSafetyPolicy, PromptProtectedPathClass, PromptProtectedPathRegistry,
    PromptSafetyAllowanceId, PromptSafetyPolicyVersion, PromptSafetyReason, PromptSafetyReasonCode,
    PromptSafetySeverity, PromptSafetySummary, PromptWriteOperation, PromptWriteSafetyDecision,
    PromptWriteSafetyError, PromptWriteSafetyEvent, PromptWriteSafetyEventKind,
    PromptWriteSafetyEventSink, PromptWriteSafetyPolicy, PromptWriteSafetyRequest,
    PromptWriteSource,
};
pub use search::{FusionStrategy, MemorySearchRequest, MemorySearchResult};
