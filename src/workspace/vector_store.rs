//! Vector store abstraction for workspace semantic search.
//!
//! Separates vector search from the main `Database` trait so that
//! third-party vector backends (LanceDB, Qdrant, Pinecone, etc.) can
//! be added by implementing a 4-method trait instead of wrapping the
//! entire ~80-method `Database` trait.
//!
//! When no external vector store is configured, the built-in database
//! vector support (pgvector / libsql_vector_idx) is used via the
//! `Database::hybrid_search` method directly.

use async_trait::async_trait;
use uuid::Uuid;

use crate::error::WorkspaceError;
use crate::workspace::search::RankedResult;

/// External vector store for semantic search.
///
/// Implementations hold chunk embeddings and perform vector similarity
/// queries. Document/chunk metadata and FTS stay in the main database;
/// only embeddings live here.
///
/// # Adding a new backend
///
/// 1. Implement this trait for your backend (4 methods).
/// 2. Feature-gate the module (`#[cfg(feature = "mybackend")]`).
/// 3. Pass `Arc<dyn VectorStore>` to `Workspace::with_vector_store()`.
///
/// That's it — no Database wrapper, no delegation boilerplate.
#[async_trait]
pub trait VectorStore: Send + Sync {
    /// Store an embedding for a chunk.
    async fn store_embedding(
        &self,
        chunk_id: Uuid,
        document_id: Uuid,
        user_id: &str,
        agent_id: Option<Uuid>,
        content: &str,
        embedding: &[f32],
    ) -> Result<(), WorkspaceError>;

    /// Update an existing chunk's embedding (delete + re-insert is fine).
    async fn update_embedding(
        &self,
        chunk_id: Uuid,
        document_id: Uuid,
        user_id: &str,
        agent_id: Option<Uuid>,
        content: &str,
        embedding: &[f32],
    ) -> Result<(), WorkspaceError>;

    /// Delete all embeddings for a document.
    async fn delete_embeddings(&self, document_id: Uuid) -> Result<(), WorkspaceError>;

    /// Vector similarity search, filtered by user and optional agent.
    ///
    /// Returns results ranked by similarity (rank 1 = most similar).
    async fn vector_search(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        embedding: &[f32],
        limit: usize,
    ) -> Result<Vec<RankedResult>, WorkspaceError>;
}
