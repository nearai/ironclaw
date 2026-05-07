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
///
/// Chunk clearing is intentionally folded into
/// [`replace_document_chunks_if_current`] (called with an empty `chunks`
/// slice) — there is no separate unguarded `delete_document_chunks` method.
/// A hash-guarded clear is the only safe shape: an unconditional delete
/// races with concurrent writes that have already produced fresh chunks
/// for newer content, silently leaving the latest write unsearchable.
#[async_trait]
pub trait MemoryDocumentIndexRepository: Send + Sync {
    async fn replace_document_chunks_if_current(
        &self,
        path: &MemoryDocumentPath,
        expected_content_hash: &str,
        chunks: &[MemoryChunkWrite],
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
        // Compute the read-time hash up front so every chunk-write outcome
        // (skip_indexing, empty chunks, real chunks) goes through the same
        // hash-guarded `replace_document_chunks_if_current` path. An
        // unconditional delete here would race with a concurrent writer that
        // has already produced fresh chunks for newer content: a stale
        // reindex (older bytes, older `skip_indexing` flag) could remove the
        // newer write's chunk rows. The hash guard turns the stale clear
        // into a no-op once the document has moved on.
        let content_hash_at_read = content_sha256(content);
        let metadata = resolve_document_metadata(self.repository.as_ref(), path).await?;
        if metadata.skip_indexing == Some(true) {
            return self
                .repository
                .replace_document_chunks_if_current(path, &content_hash_at_read, &[])
                .await;
        }
        let chunk_texts = chunk_document(content, self.chunk_config.clone());
        if chunk_texts.is_empty() {
            return self
                .repository
                .replace_document_chunks_if_current(path, &content_hash_at_read, &[])
                .await;
        }
        let chunks = match build_chunk_writes(
            path,
            chunk_texts.clone(),
            self.embedding_provider.as_deref(),
        )
        .await
        {
            Ok(chunks) => chunks,
            Err(error) => {
                // **Embedding-generation outage degrades to text-only
                // indexing, not loss of search.** Previous behavior
                // hash-cleared the chunk rows on provider failure;
                // backend writes intentionally swallow indexer errors
                // after persistence, so the durable document landed
                // correctly but full-text searchability for the new
                // content was wiped. We now persist text-only chunks
                // (embedding = NULL) using the same hash guard so FTS
                // stays current while vector search degrades. The
                // embedding error is still returned so callers/log
                // surfaces can report the provider outage (zmanian
                // #3180 MED `indexer.rs:117`).
                let text_only: Vec<MemoryChunkWrite> = chunk_texts
                    .into_iter()
                    .map(|content| MemoryChunkWrite {
                        content,
                        embedding: None,
                    })
                    .collect();
                self.repository
                    .replace_document_chunks_if_current(path, &content_hash_at_read, &text_only)
                    .await?;
                return Err(error);
            }
        };
        // **Re-resolve metadata immediately before the final replace.**
        // A concurrent metadata write that flips `skip_indexing=true`
        // (and runs the matching parent-config chunk-clear) between our
        // initial resolve at the top of this function and the replace
        // below would otherwise have its clear undone — the content
        // hash the reindex is guarded on did not change, so
        // `replace_document_chunks_if_current` would happily insert
        // chunks again, leaving the document marked skip-indexed yet
        // searchable until the next reindex.
        //
        // Placing the re-resolve here (after the potentially long
        // embedding-build call) narrows the race window to a small
        // interval. A fully race-free fix needs an atomic
        // metadata-version-checked replace at the repository layer; that
        // is intentionally deferred until the repo trait is widened.
        // For now, the narrowed window plus the existing hash guard
        // makes the race vanishingly improbable in practice (zmanian
        // #3180 MED `indexer.rs:122`).
        let metadata_at_commit = resolve_document_metadata(self.repository.as_ref(), path).await?;
        if metadata_at_commit.skip_indexing == Some(true) {
            return self
                .repository
                .replace_document_chunks_if_current(path, &content_hash_at_read, &[])
                .await;
        }
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
    use crate::embedding::EmbeddingError;
    use crate::path::MemoryDocumentScope;
    use crate::search::{MemorySearchRequest, MemorySearchResult};

    #[derive(Debug, PartialEq, Eq)]
    enum IndexerCall {
        Replace { chunks: usize, hash: String },
    }

    struct RejectingEmbeddingProvider;

    #[async_trait]
    impl EmbeddingProvider for RejectingEmbeddingProvider {
        fn dimension(&self) -> usize {
            3
        }

        fn model_name(&self) -> &str {
            "rejecting"
        }

        async fn embed(&self, _text: &str) -> Result<Vec<f32>, EmbeddingError> {
            Err(EmbeddingError::ProviderUnavailable {
                reason: "synthetic failure".to_string(),
            })
        }
    }

    struct RecordingRepo {
        content: Vec<u8>,
        // {"skip_indexing": true} when set, otherwise empty
        metadata: Option<serde_json::Value>,
        calls: Mutex<Vec<IndexerCall>>,
    }

    impl RecordingRepo {
        fn new(content: impl Into<Vec<u8>>) -> Self {
            Self {
                content: content.into(),
                metadata: None,
                calls: Mutex::new(Vec::new()),
            }
        }

        fn with_skip_indexing(mut self) -> Self {
            self.metadata = Some(serde_json::json!({"skip_indexing": true}));
            self
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

        async fn read_document_metadata(
            &self,
            _path: &MemoryDocumentPath,
        ) -> Result<Option<serde_json::Value>, FilesystemError> {
            Ok(self.metadata.clone())
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
    }

    fn doc_path() -> MemoryDocumentPath {
        MemoryDocumentPath::new("tenant", "user", Some("project"), "note.md").unwrap()
    }

    // Regression for PR #3183 review: the indexer used to call an
    // unconditional `delete_document_chunks` for both `skip_indexing == true`
    // and the empty-chunks case. A stale reindex (older bytes, older
    // skip_indexing flag) could then race with a concurrent writer that had
    // already produced fresh chunks for newer content and silently clobber
    // them. The fix routes both paths through the hash-checked
    // `replace_document_chunks_if_current` helper, and the unguarded
    // `delete_document_chunks` was removed from the trait entirely so no
    // future caller can re-introduce the race. The recording repo here only
    // implements the hash-checked path; if either branch ever started
    // calling something else, this test would not compile.
    #[tokio::test]
    async fn skip_indexing_routes_through_hash_checked_replace() {
        let content = "alpha beta gamma delta epsilon";
        let repo = Arc::new(RecordingRepo::new(content).with_skip_indexing());
        let indexer = ChunkingMemoryDocumentIndexer::new(repo.clone());
        indexer.reindex_document(&doc_path()).await.unwrap();
        let calls = repo.calls.lock().unwrap();
        assert_eq!(calls.len(), 1, "expected exactly one indexer call");
        let IndexerCall::Replace { chunks, hash } = &calls[0];
        assert_eq!(*chunks, 0, "skip_indexing must clear with empty chunk set");
        assert_eq!(*hash, content_sha256(content));
    }

    #[tokio::test]
    async fn empty_chunks_route_through_hash_checked_replace() {
        let repo = Arc::new(RecordingRepo::new("   \n\t  "));
        let indexer = ChunkingMemoryDocumentIndexer::new(repo.clone());
        indexer.reindex_document(&doc_path()).await.unwrap();
        let calls = repo.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        let IndexerCall::Replace { chunks, hash } = &calls[0];
        assert_eq!(*chunks, 0, "expected empty chunk set");
        assert_eq!(*hash, content_sha256("   \n\t  "));
    }

    #[tokio::test]
    async fn non_empty_chunks_route_through_hash_checked_replace() {
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

    #[tokio::test]
    async fn empty_chunks_do_not_call_embedding_provider_before_clearing() {
        let content = "   \n\t  ";
        let repo = Arc::new(RecordingRepo::new(content));
        let indexer = ChunkingMemoryDocumentIndexer::new(repo.clone())
            .with_embedding_provider(Arc::new(RejectingEmbeddingProvider));
        indexer.reindex_document(&doc_path()).await.unwrap();
        let calls = repo.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert!(matches!(
            &calls[0],
            IndexerCall::Replace { chunks, hash }
                if *chunks == 0 && hash == &content_sha256(content)
        ));
    }

    /// Repo that reports `skip_indexing=false` on the first metadata
    /// read (the indexer's initial resolve) and `skip_indexing=true` on
    /// every subsequent read. Models a concurrent metadata write that
    /// arrives after the indexer's initial decision but before its
    /// final commit.
    struct FlipsSkipIndexingMidwayRepo {
        content: Vec<u8>,
        reads: Mutex<usize>,
        calls: Mutex<Vec<IndexerCall>>,
    }

    impl FlipsSkipIndexingMidwayRepo {
        fn new(content: impl Into<Vec<u8>>) -> Self {
            Self {
                content: content.into(),
                reads: Mutex::new(0),
                calls: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl MemoryDocumentRepository for FlipsSkipIndexingMidwayRepo {
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

        async fn read_document_metadata(
            &self,
            _path: &MemoryDocumentPath,
        ) -> Result<Option<serde_json::Value>, FilesystemError> {
            let mut reads = self.reads.lock().unwrap();
            *reads += 1;
            // First read (initial resolve): no skip. Subsequent reads
            // (the re-resolve right before commit): skip_indexing=true.
            if *reads <= 1 {
                Ok(Some(serde_json::json!({})))
            } else {
                Ok(Some(serde_json::json!({"skip_indexing": true})))
            }
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
    impl MemoryDocumentIndexRepository for FlipsSkipIndexingMidwayRepo {
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
    }

    #[tokio::test]
    async fn concurrent_skip_indexing_metadata_write_wins_against_in_flight_reindex() {
        // zmanian #3180 MED `indexer.rs:122`: a concurrent metadata
        // write that flips `skip_indexing=true` between the indexer's
        // initial resolve and its final replace must NOT have its
        // chunk-clear undone by the in-flight reindex (the content
        // hash is unchanged, so the legacy guard alone wouldn't reject
        // the replace). The fix re-resolves metadata immediately
        // before the final replace and short-circuits to an empty
        // chunk set when the resolved skip_indexing is now true.
        let content = "alpha beta gamma delta epsilon";
        let repo = Arc::new(FlipsSkipIndexingMidwayRepo::new(content));
        let indexer = ChunkingMemoryDocumentIndexer::new(repo.clone());
        indexer.reindex_document(&doc_path()).await.unwrap();
        let calls = repo.calls.lock().unwrap();
        assert_eq!(
            calls.len(),
            1,
            "indexer must commit exactly one replace call per reindex"
        );
        let IndexerCall::Replace { chunks, hash } = &calls[0];
        assert_eq!(
            *chunks, 0,
            "concurrent skip_indexing=true write must short-circuit the reindex to an empty chunk set"
        );
        assert_eq!(*hash, content_sha256(content));
    }

    #[tokio::test]
    async fn embedding_failure_persists_text_only_chunks_with_hash_guard() {
        // zmanian #3180 MED `indexer.rs:117`: an embedding-generation
        // outage must not wipe full-text searchability. Previous
        // behavior cleared chunks (count = 0) and returned the error;
        // backend writes intentionally swallow indexer errors after
        // persistence, so a successfully overwritten document
        // disappeared from FTS too. New contract: text-only chunks
        // (embedding = None) are persisted via the hash-guarded
        // replace, FTS stays current, vector search degrades, and the
        // embedding error is still returned for observability.
        let content = "alpha beta gamma delta epsilon";
        let repo = Arc::new(RecordingRepo::new(content));
        let indexer = ChunkingMemoryDocumentIndexer::new(repo.clone())
            .with_embedding_provider(Arc::new(RejectingEmbeddingProvider));
        let err = indexer.reindex_document(&doc_path()).await.unwrap_err();
        assert!(
            err.to_string().contains("synthetic failure"),
            "embedding error must still surface to callers for observability; got: {err}"
        );
        let calls = repo.calls.lock().unwrap();
        assert_eq!(calls.len(), 1, "expected exactly one indexer call");
        let IndexerCall::Replace { chunks, hash } = &calls[0];
        assert!(
            *chunks > 0,
            "embedding failure must NOT wipe chunks; expected text-only chunks, got {chunks}"
        );
        assert_eq!(
            *hash,
            content_sha256(content),
            "the hash guard must reflect the read-time content so a concurrent writer's fresh chunks aren't clobbered"
        );
    }
}
