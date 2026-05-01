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
#[cfg(feature = "libsql")]
pub use repo::RebornLibSqlMemoryDocumentRepository;
#[cfg(feature = "postgres")]
pub use repo::RebornPostgresMemoryDocumentRepository;
pub use repo::{InMemoryMemoryDocumentRepository, MemoryDocumentRepository};
pub use search::{FusionStrategy, MemorySearchRequest, MemorySearchResult};
