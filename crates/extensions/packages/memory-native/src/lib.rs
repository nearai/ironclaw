//! Memory document filesystem adapters for IronClaw Reborn.
//!
//! This crate owns memory-specific path grammar and repository seams. The
//! generic filesystem crate owns only virtual path authority, scoped mounts,
//! backend cataloging, and backend routing.

mod backend;
mod chunking;
#[cfg(any(test, feature = "test-support"))]
pub mod contract_tests;
mod events;
mod filesystem;
mod indexer;
mod metadata;
mod path;
mod repo;
mod safety;
mod schema;
mod search;
mod service;
mod write_metadata;

pub use backend::{MemoryBackend, MemoryBackendCapabilities, RepositoryMemoryBackend};
pub use chunking::{ChunkConfig, MemoryChunkWrite, chunk_document};
pub use filesystem::{MemoryBackendFilesystemAdapter, MemoryDocumentFilesystem};
pub use indexer::{
    ChunkingMemoryDocumentIndexer, MemoryChunkReplaceOutcome, MemoryDocumentIndexRepository,
    MemoryDocumentIndexer,
};
pub use metadata::{MemoryBackendWriteOptions, MemoryWriteOptions};
pub use repo::{
    FilesystemMemoryDocumentRepository, InMemoryMemoryDocumentRepository, MemoryAppendOutcome,
    MemoryDocumentRepository, MemoryWriteOutcome,
};
pub use safety::DefaultPromptWriteSafetyPolicy;
pub use search::{FusionStrategy, MemorySearchRequest, MemorySearchResult};
pub use service::{
    MEMORY_GUIDANCE, MEMORY_GUIDANCE_ASSETS, MEMORY_GUIDANCE_DOC_REF, NativeMemoryService,
};
