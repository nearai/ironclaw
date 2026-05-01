//! Memory indexer trait and chunking-based indexer implementation.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_filesystem::{FilesystemError, FilesystemOperation};

use crate::chunking::{ChunkConfig, MemoryChunkWrite, chunk_document, content_sha256};
use crate::embedding::{
    EmbeddingProvider, embedding_filesystem_error, validate_embedding_dimension,
};
use crate::metadata::resolve_document_metadata;
use crate::path::{MemoryDocumentPath, memory_error, valid_memory_path};
use crate::repo::MemoryDocumentRepository;

/// Hook invoked after successful memory document writes so derived state can be refreshed.
#[async_trait]
pub trait MemoryDocumentIndexer: Send + Sync {
    async fn reindex_document(&self, path: &MemoryDocumentPath) -> Result<(), FilesystemError>;
}

/// Repository operations used by the memory indexer to keep chunk/search rows in sync.
#[async_trait]
pub trait MemoryDocumentIndexRepository: Send + Sync {
    async fn replace_document_chunks_if_current(
        &self,
        path: &MemoryDocumentPath,
        expected_content_hash: &str,
        chunks: &[MemoryChunkWrite],
    ) -> Result<(), FilesystemError>;

    async fn delete_document_chunks(
        &self,
        path: &MemoryDocumentPath,
    ) -> Result<(), FilesystemError>;
}

/// Memory document indexer that chunks documents and updates DB-backed chunk rows.
pub struct ChunkingMemoryDocumentIndexer<R> {
    repository: Arc<R>,
    chunk_config: ChunkConfig,
    embedding_provider: Option<Arc<dyn EmbeddingProvider>>,
}

impl<R> ChunkingMemoryDocumentIndexer<R>
where
    R: MemoryDocumentRepository + MemoryDocumentIndexRepository + 'static,
{
    pub fn new(repository: Arc<R>) -> Self {
        Self {
            repository,
            chunk_config: ChunkConfig::default(),
            embedding_provider: None,
        }
    }

    pub fn with_chunk_config(mut self, chunk_config: ChunkConfig) -> Self {
        self.chunk_config = chunk_config;
        self
    }

    pub fn with_embedding_provider<P>(mut self, provider: Arc<P>) -> Self
    where
        P: EmbeddingProvider + 'static,
    {
        self.embedding_provider = Some(provider);
        self
    }
}

#[async_trait]
impl<R> MemoryDocumentIndexer for ChunkingMemoryDocumentIndexer<R>
where
    R: MemoryDocumentRepository + MemoryDocumentIndexRepository + 'static,
{
    async fn reindex_document(&self, path: &MemoryDocumentPath) -> Result<(), FilesystemError> {
        let Some(bytes) = self.repository.read_document(path).await? else {
            return Ok(());
        };
        let content = std::str::from_utf8(&bytes).map_err(|_| {
            memory_error(
                path.virtual_path().unwrap_or_else(|_| valid_memory_path()),
                FilesystemOperation::WriteFile,
                "memory document content must be UTF-8",
            )
        })?;
        let metadata = resolve_document_metadata(self.repository.as_ref(), path).await?;
        if metadata.skip_indexing == Some(true) {
            return self.repository.delete_document_chunks(path).await;
        }
        let content_hash_at_read = content_sha256(content);
        let chunk_texts = chunk_document(content, self.chunk_config.clone());
        let chunks =
            build_chunk_writes(path, chunk_texts, self.embedding_provider.as_deref()).await?;
        if chunks.is_empty() {
            self.repository.delete_document_chunks(path).await
        } else {
            self.repository
                .replace_document_chunks_if_current(path, &content_hash_at_read, &chunks)
                .await
        }
    }
}

async fn build_chunk_writes(
    path: &MemoryDocumentPath,
    chunk_texts: Vec<String>,
    embedding_provider: Option<&dyn EmbeddingProvider>,
) -> Result<Vec<MemoryChunkWrite>, FilesystemError> {
    let Some(provider) = embedding_provider else {
        return Ok(chunk_texts
            .into_iter()
            .map(|content| MemoryChunkWrite {
                content,
                embedding: None,
            })
            .collect());
    };
    let embeddings = provider.embed_batch(&chunk_texts).await.map_err(|error| {
        embedding_filesystem_error(
            path.virtual_path().unwrap_or_else(|_| valid_memory_path()),
            FilesystemOperation::WriteFile,
            error,
        )
    })?;
    if embeddings.len() != chunk_texts.len() {
        return Err(memory_error(
            path.virtual_path().unwrap_or_else(|_| valid_memory_path()),
            FilesystemOperation::WriteFile,
            format!(
                "embedding provider returned {} embeddings for {} chunks",
                embeddings.len(),
                chunk_texts.len()
            ),
        ));
    }
    let expected_dimension = provider.dimension();
    chunk_texts
        .into_iter()
        .zip(embeddings)
        .map(|(content, embedding)| {
            validate_embedding_dimension(expected_dimension, embedding.len()).map_err(|error| {
                embedding_filesystem_error(
                    path.virtual_path().unwrap_or_else(|_| valid_memory_path()),
                    FilesystemOperation::WriteFile,
                    error,
                )
            })?;
            Ok(MemoryChunkWrite {
                content,
                embedding: Some(embedding),
            })
        })
        .collect()
}
