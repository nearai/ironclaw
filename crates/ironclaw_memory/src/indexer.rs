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
        // Route empty chunk sets through the same hash-checked replacement path
        // used for non-empty sets. An unconditional delete would otherwise race
        // with a concurrent writer that has already produced fresh chunk rows
        // for newer content: indexer A reads a whitespace document and computes
        // an empty chunk list, writer B replaces chunks with real content, A
        // resumes and clobbers B's rows. The hash guard makes A's delete a
        // no-op once the document has moved on.
        self.repository
            .replace_document_chunks_if_current(path, &content_hash_at_read, &chunks)
            .await
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

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::path::MemoryDocumentScope;
    use crate::search::{MemorySearchRequest, MemorySearchResult};

    #[derive(Debug, PartialEq, Eq)]
    enum IndexerCall {
        Replace { chunks: usize, hash: String },
        Delete,
    }

    struct RecordingRepo {
        content: Vec<u8>,
        calls: Mutex<Vec<IndexerCall>>,
    }

    impl RecordingRepo {
        fn new(content: impl Into<Vec<u8>>) -> Self {
            Self {
                content: content.into(),
                calls: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl MemoryDocumentRepository for RecordingRepo {
        async fn read_document(
            &self,
            _path: &MemoryDocumentPath,
        ) -> Result<Option<Vec<u8>>, FilesystemError> {
            Ok(Some(self.content.clone()))
        }

        async fn write_document(
            &self,
            _path: &MemoryDocumentPath,
            _bytes: &[u8],
        ) -> Result<(), FilesystemError> {
            Ok(())
        }

        async fn list_documents(
            &self,
            _scope: &MemoryDocumentScope,
        ) -> Result<Vec<MemoryDocumentPath>, FilesystemError> {
            Ok(Vec::new())
        }

        async fn search_documents(
            &self,
            _scope: &MemoryDocumentScope,
            _request: &MemorySearchRequest,
        ) -> Result<Vec<MemorySearchResult>, FilesystemError> {
            Ok(Vec::new())
        }
    }

    #[async_trait]
    impl MemoryDocumentIndexRepository for RecordingRepo {
        async fn replace_document_chunks_if_current(
            &self,
            _path: &MemoryDocumentPath,
            expected_content_hash: &str,
            chunks: &[MemoryChunkWrite],
        ) -> Result<(), FilesystemError> {
            self.calls.lock().unwrap().push(IndexerCall::Replace {
                chunks: chunks.len(),
                hash: expected_content_hash.to_string(),
            });
            Ok(())
        }

        async fn delete_document_chunks(
            &self,
            _path: &MemoryDocumentPath,
        ) -> Result<(), FilesystemError> {
            self.calls.lock().unwrap().push(IndexerCall::Delete);
            Ok(())
        }
    }

    fn doc_path() -> MemoryDocumentPath {
        MemoryDocumentPath::new("tenant", "user", Some("project"), "note.md").unwrap()
    }

    // Regression for PR #3180 review: empty/whitespace documents previously
    // routed through the unconditional `delete_document_chunks`, which races
    // with a concurrent writer that has already inserted fresh chunks for
    // newer content. Empty chunk sets must go through the same hash-checked
    // replacement path as non-empty sets so the delete becomes a no-op once
    // the document content has moved on.
    #[tokio::test]
    async fn empty_chunks_route_through_hash_checked_replace_not_unconditional_delete() {
        let repo = Arc::new(RecordingRepo::new("   \n\t  "));
        let indexer = ChunkingMemoryDocumentIndexer::new(repo.clone());
        indexer.reindex_document(&doc_path()).await.unwrap();
        let calls = repo.calls.lock().unwrap();
        assert_eq!(calls.len(), 1, "expected exactly one indexer call");
        match &calls[0] {
            IndexerCall::Replace { chunks, hash } => {
                assert_eq!(*chunks, 0, "expected empty chunk set");
                assert_eq!(*hash, content_sha256("   \n\t  "));
            }
            IndexerCall::Delete => {
                panic!("empty chunks must not call unconditional delete_document_chunks")
            }
        }
    }

    #[tokio::test]
    async fn non_empty_chunks_still_route_through_hash_checked_replace() {
        let content = "alpha beta gamma delta epsilon";
        let repo = Arc::new(RecordingRepo::new(content));
        let indexer = ChunkingMemoryDocumentIndexer::new(repo.clone());
        indexer.reindex_document(&doc_path()).await.unwrap();
        let calls = repo.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert!(matches!(
            &calls[0],
            IndexerCall::Replace { chunks, hash }
                if *chunks > 0 && hash == &content_sha256(content)
        ));
    }
}
