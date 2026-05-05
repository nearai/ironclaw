//! Behavior contract for `RepositoryMemoryBackend` layered over the
//! Reborn-native repositories (#3118 phase 6).
//!
//! These tests port pure-behavior coverage from the legacy
//! `db_memory_repository_contract.rs` and `memory_filesystem_contract.rs`
//! to the native `reborn_memory_*` substrate. The semantics under test
//! live above the bare repository — they belong to the
//! `RepositoryMemoryBackend` composition (`.config` inheritance, schema
//! validation, indexer best-effort, capability fail-closed,
//! embedding-dimension guard).
//!
//! libSQL tests run in-process against a temp DB. Postgres tests follow
//! the standard `DATABASE_URL=postgres://localhost/ironclaw_test` pattern
//! and skip cleanly when no DB is reachable.

#![cfg(any(feature = "libsql", feature = "postgres"))]

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_filesystem::FilesystemError;
use ironclaw_memory::{
    EmbeddingError, EmbeddingProvider, MemoryBackend, MemoryBackendCapabilities, MemoryContext,
    MemoryDocumentIndexer, MemoryDocumentPath, MemoryDocumentScope, MemorySearchRequest,
    RepositoryMemoryBackend,
};

#[cfg(feature = "libsql")]
use ironclaw_memory::{ChunkConfig, ChunkingMemoryDocumentIndexer, FusionStrategy};

#[cfg(feature = "libsql")]
use ironclaw_memory::RebornLibSqlMemoryDocumentRepository;
#[cfg(feature = "postgres")]
use ironclaw_memory::RebornPostgresMemoryDocumentRepository;

// --- shared test stubs ----------------------------------------------------

/// Deterministic embedding provider for backend tests. Produces a
/// one-hot vector at position 0/1/2 depending on coarse content
/// category — three "buckets" matching the legacy workspace contract:
/// hybrid/semantic-only content vs. unrelated content vs. everything
/// else. Generic over `DIM` so the same semantics work against
/// libSQL (DIM=3, blob-shaped embeddings) and Postgres (DIM=1536,
/// fixed-width pgvector column).
struct RecordingEmbeddingProvider<const DIM: usize>;

#[async_trait]
impl<const DIM: usize> EmbeddingProvider for RecordingEmbeddingProvider<DIM> {
    fn dimension(&self) -> usize {
        DIM
    }

    fn model_name(&self) -> &str {
        "deterministic-test-embedding"
    }

    async fn embed(&self, text: &str) -> Result<Vec<f32>, EmbeddingError> {
        let mut v = vec![0.0_f32; DIM];
        let slot = if text.contains("hybrid-vector")
            || text == "literal"
            || text.contains("semantic-only")
        {
            0
        } else if text.contains("unrelated") {
            1
        } else {
            2
        };
        v[slot] = 1.0;
        Ok(v)
    }
}

/// EmbeddingProvider that declares one `dimension()` but returns vectors of
/// a different actual dimension. Used to drive the write/index-time
/// dimension validation path that mirrors the search-side check at
/// `backend.rs:513-527`. zmanian test gap 2.
struct LyingEmbeddingProvider {
    declared: usize,
    actual: usize,
}

#[async_trait]
impl EmbeddingProvider for LyingEmbeddingProvider {
    fn dimension(&self) -> usize {
        self.declared
    }

    fn model_name(&self) -> &str {
        "lying-test-embedding"
    }

    async fn embed(&self, _text: &str) -> Result<Vec<f32>, EmbeddingError> {
        Ok(vec![0.5_f32; self.actual])
    }
}

struct FailingIndexer;

#[async_trait]
impl MemoryDocumentIndexer for FailingIndexer {
    async fn reindex_document(&self, _path: &MemoryDocumentPath) -> Result<(), FilesystemError> {
        Err(FilesystemError::Backend {
            path: ironclaw_host_api::VirtualPath::new("/memory").unwrap(),
            operation: ironclaw_filesystem::FilesystemOperation::WriteFile,
            reason: "synthetic indexer failure".into(),
        })
    }
}

// --- libSQL ---------------------------------------------------------------

#[cfg(feature = "libsql")]
struct LibsqlFixture {
    repo: Arc<RebornLibSqlMemoryDocumentRepository>,
    db: Arc<libsql::Database>,
    _dir: tempfile::TempDir,
}

#[cfg(feature = "libsql")]
async fn libsql_repo() -> LibsqlFixture {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("reborn_memory.db");
    let db = Arc::new(
        libsql::Builder::new_local(db_path)
            .build()
            .await
            .expect("libsql build"),
    );
    let repo = Arc::new(RebornLibSqlMemoryDocumentRepository::new(db.clone()));
    repo.run_migrations().await.expect("run_migrations");
    LibsqlFixture {
        repo,
        db,
        _dir: dir,
    }
}

#[cfg(feature = "libsql")]
async fn count_rows_libsql(db: &Arc<libsql::Database>, table: &str) -> i64 {
    let conn = db.connect().expect("connect");
    let mut rows = conn
        .query(&format!("SELECT COUNT(*) FROM {table}"), ())
        .await
        .expect("count query");
    let row = rows
        .next()
        .await
        .expect("next row")
        .expect("count row exists");
    row.get(0).expect("count value")
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_backend_skip_indexing_from_config_writes_zero_chunks_to_db() {
    use ironclaw_memory::MemoryDocumentRepository;
    let fixture = libsql_repo().await;
    let repo = fixture.repo.clone();
    let config_path =
        MemoryDocumentPath::new("tenant-a", "alice", None, "folder/.config").expect("path");
    repo.write_document(&config_path, b"").await.unwrap();
    repo.write_document_metadata(&config_path, &serde_json::json!({"skip_indexing": true}))
        .await
        .unwrap();

    // Use the real ChunkingMemoryDocumentIndexer which reads the resolved
    // metadata and short-circuits on `skip_indexing`. This proves the
    // signal actually reaches the chunk-writing layer — the previous
    // version of this test only counted indexer invocations, which a
    // skip-aware indexer respects but does not observably zero out.
    let indexer = Arc::new(ChunkingMemoryDocumentIndexer::new(repo.clone()));
    let backend = RepositoryMemoryBackend::new(repo.clone()).with_indexer(indexer);
    let context = MemoryContext::new(MemoryDocumentScope::new("tenant-a", "alice", None).unwrap());
    let path = MemoryDocumentPath::new("tenant-a", "alice", None, "folder/note.md").expect("path");

    backend
        .write_document(&context, &path, b"alpha beta gamma")
        .await
        .unwrap();

    let chunk_count = count_rows_libsql(&fixture.db, "reborn_memory_chunks").await;
    assert_eq!(
        chunk_count, 0,
        "skip_indexing inherited from .config must produce zero chunks"
    );
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_backend_document_metadata_overrides_inherited_config() {
    use ironclaw_memory::MemoryDocumentRepository;
    let fixture = libsql_repo().await;
    let repo = fixture.repo.clone();
    // Parent .config says skip_versioning=true; document-level metadata
    // overrides it back to false. The next overwrite must produce a
    // version row, proving doc-level metadata wins over inherited config.
    let cfg = MemoryDocumentPath::new("tenant-a", "alice", None, "logs/.config").expect("path");
    repo.write_document(&cfg, b"").await.unwrap();
    repo.write_document_metadata(&cfg, &serde_json::json!({"skip_versioning": true}))
        .await
        .unwrap();

    let path = MemoryDocumentPath::new("tenant-a", "alice", None, "logs/entry.md").expect("path");
    repo.write_document(&path, b"v1").await.unwrap();
    repo.write_document_metadata(&path, &serde_json::json!({"skip_versioning": false}))
        .await
        .unwrap();

    let backend = RepositoryMemoryBackend::new(repo.clone());
    let context = MemoryContext::new(MemoryDocumentScope::new("tenant-a", "alice", None).unwrap());
    backend
        .write_document(&context, &path, b"v2")
        .await
        .unwrap();

    let count = count_rows_libsql(&fixture.db, "reborn_memory_document_versions").await;
    assert_eq!(
        count, 1,
        "document-level metadata must override inherited .config skip_versioning"
    );
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_backend_closer_config_overrides_parent_config() {
    use ironclaw_memory::MemoryDocumentRepository;
    let fixture = libsql_repo().await;
    let repo = fixture.repo.clone();
    let parent_cfg = MemoryDocumentPath::new("tenant-a", "alice", None, ".config").expect("path");
    repo.write_document(&parent_cfg, b"").await.unwrap();
    repo.write_document_metadata(&parent_cfg, &serde_json::json!({"skip_versioning": true}))
        .await
        .unwrap();

    let child_cfg =
        MemoryDocumentPath::new("tenant-a", "alice", None, "logs/.config").expect("path");
    repo.write_document(&child_cfg, b"").await.unwrap();
    repo.write_document_metadata(&child_cfg, &serde_json::json!({"skip_versioning": false}))
        .await
        .unwrap();

    let backend = RepositoryMemoryBackend::new(repo.clone());
    let context = MemoryContext::new(MemoryDocumentScope::new("tenant-a", "alice", None).unwrap());
    let path = MemoryDocumentPath::new("tenant-a", "alice", None, "logs/entry.md").expect("path");

    backend
        .write_document(&context, &path, b"v1")
        .await
        .unwrap();
    backend
        .write_document(&context, &path, b"v2")
        .await
        .unwrap();

    // The closer `logs/.config` says skip_versioning=false, so a version
    // row must exist for the v1 -> v2 overwrite even though the root
    // `.config` would have suppressed it.
    let count = count_rows_libsql(&fixture.db, "reborn_memory_document_versions").await;
    assert_eq!(count, 1, "closer .config must override parent .config");
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_backend_honors_skip_versioning_from_config() {
    use ironclaw_memory::MemoryDocumentRepository;
    let fixture = libsql_repo().await;
    let repo = fixture.repo.clone();
    let cfg_path =
        MemoryDocumentPath::new("tenant-a", "alice", None, "logs/.config").expect("path");
    repo.write_document(&cfg_path, b"").await.unwrap();
    repo.write_document_metadata(&cfg_path, &serde_json::json!({"skip_versioning": true}))
        .await
        .unwrap();

    let backend = RepositoryMemoryBackend::new(repo.clone());
    let context = MemoryContext::new(MemoryDocumentScope::new("tenant-a", "alice", None).unwrap());
    let path = MemoryDocumentPath::new("tenant-a", "alice", None, "logs/entry.md").expect("path");

    backend
        .write_document(&context, &path, b"v1")
        .await
        .unwrap();
    backend
        .write_document(&context, &path, b"v2")
        .await
        .unwrap();

    let count = count_rows_libsql(&fixture.db, "reborn_memory_document_versions").await;
    assert_eq!(count, 0);
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_backend_validates_schema_from_config_before_write() {
    use ironclaw_memory::MemoryDocumentRepository;
    let fixture = libsql_repo().await;
    let repo = fixture.repo.clone();
    let cfg_path =
        MemoryDocumentPath::new("tenant-a", "alice", None, "settings/.config").expect("path");
    repo.write_document(&cfg_path, b"").await.unwrap();
    repo.write_document_metadata(
        &cfg_path,
        &serde_json::json!({
            "schema": {
                "type": "object",
                "properties": {"provider": {"type": "string"}},
                "required": ["provider"],
            }
        }),
    )
    .await
    .unwrap();

    let backend = RepositoryMemoryBackend::new(repo.clone());
    let context = MemoryContext::new(MemoryDocumentScope::new("tenant-a", "alice", None).unwrap());
    let path =
        MemoryDocumentPath::new("tenant-a", "alice", None, "settings/llm.json").expect("path");

    let err = backend
        .write_document(&context, &path, br#"{"missing":"provider"}"#)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("schema validation failed"));

    // Document must NOT be persisted on schema rejection.
    let stored = repo.read_document(&path).await.unwrap();
    assert!(stored.is_none(), "schema-rejected write must not persist");
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_backend_reports_write_success_when_indexer_fails_after_persist() {
    use ironclaw_memory::MemoryDocumentRepository;
    let fixture = libsql_repo().await;
    let repo = fixture.repo.clone();
    let backend = RepositoryMemoryBackend::new(repo.clone()).with_indexer(Arc::new(FailingIndexer));
    let context = MemoryContext::new(MemoryDocumentScope::new("tenant-a", "alice", None).unwrap());
    let path = MemoryDocumentPath::new("tenant-a", "alice", None, "doc.md").expect("path");

    backend
        .write_document(&context, &path, b"persisted")
        .await
        .expect("write must succeed even if the indexer fails after persist");

    let stored = repo.read_document(&path).await.unwrap();
    assert_eq!(stored.as_deref(), Some(b"persisted".as_slice()));
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_backend_search_fails_closed_for_unsupported_vector_search() {
    let fixture = libsql_repo().await;
    let repo = fixture.repo.clone();
    let backend = RepositoryMemoryBackend::new(repo).with_capabilities(MemoryBackendCapabilities {
        file_documents: true,
        full_text_search: true,
        vector_search: false,
        ..MemoryBackendCapabilities::default()
    });
    let context = MemoryContext::new(MemoryDocumentScope::new("tenant-a", "alice", None).unwrap());

    let err = backend
        .search(
            &context,
            MemorySearchRequest::new("literal")
                .unwrap()
                .with_full_text(false)
                .with_query_embedding(vec![1.0, 0.0, 0.0]),
        )
        .await
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("memory backend does not support vector search")
    );
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_backend_search_fails_closed_on_query_embedding_dimension_mismatch() {
    let fixture = libsql_repo().await;
    let repo = fixture.repo.clone();
    let provider = Arc::new(RecordingEmbeddingProvider::<3>);
    let backend = RepositoryMemoryBackend::new(repo)
        .with_embedding_provider(provider)
        .with_capabilities(MemoryBackendCapabilities {
            file_documents: true,
            full_text_search: false,
            vector_search: true,
            embeddings: true,
            ..MemoryBackendCapabilities::default()
        });
    let context = MemoryContext::new(MemoryDocumentScope::new("tenant-a", "alice", None).unwrap());

    let err = backend
        .search(
            &context,
            MemorySearchRequest::new("literal")
                .unwrap()
                .with_full_text(false)
                .with_query_embedding(vec![1.0, 0.0]),
        )
        .await
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("query embedding dimension 2 does not match"),
        "expected dimension-mismatch error, got: {err}"
    );
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_backend_index_fails_closed_when_provider_returns_wrong_dimension() {
    // zmanian test gap 2: the search-side dimension mismatch check is
    // covered above. The mirror check at write/index time (provider
    // declares dim N, returns dim M) was previously only exercised in
    // unit tests on the embedding helper. Drive it through the backend
    // write seam so a regression that drops the validation in
    // `compute_chunk_embeddings` is caught.
    let fixture = libsql_repo().await;
    let repo = fixture.repo.clone();
    let provider = Arc::new(LyingEmbeddingProvider {
        declared: 4,
        actual: 2,
    });
    let indexer = Arc::new(
        ChunkingMemoryDocumentIndexer::new(repo.clone()).with_embedding_provider(provider.clone()),
    );
    let backend = RepositoryMemoryBackend::new(repo.clone())
        .with_indexer(indexer)
        .with_embedding_provider(provider)
        .with_capabilities(MemoryBackendCapabilities {
            file_documents: true,
            full_text_search: true,
            vector_search: true,
            embeddings: true,
            ..MemoryBackendCapabilities::default()
        });
    let context = MemoryContext::new(MemoryDocumentScope::new("tenant-a", "alice", None).unwrap());
    let path = MemoryDocumentPath::new("tenant-a", "alice", None, "lie.md").expect("path");

    // Backend write succeeds because indexer failures are best-effort —
    // the durable row lands on disk. But the chunk index must NOT contain
    // any rows because the indexer fails closed on the dim mismatch.
    backend
        .write_document(&context, &path, b"body content")
        .await
        .expect("durable write must succeed even if indexer fails closed");
    let chunk_count = count_rows_libsql(&fixture.db, "reborn_memory_chunks").await;
    assert_eq!(
        chunk_count, 0,
        "indexer must refuse to persist mis-dimensioned embeddings; got {chunk_count} chunks"
    );

    // The dimension validation also surfaces when the indexer is invoked
    // directly so callers that bypass the write best-effort policy see
    // the error explicitly.
    use ironclaw_memory::MemoryDocumentIndexer;
    let standalone_indexer = ChunkingMemoryDocumentIndexer::new(repo).with_embedding_provider(
        Arc::new(LyingEmbeddingProvider {
            declared: 4,
            actual: 2,
        }),
    );
    let err = standalone_indexer
        .reindex_document(&path)
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("dimension"),
        "expected dimension-mismatch error from indexer, got: {err}"
    );
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_backend_hybrid_search_fuses_full_text_and_vector_results() {
    let fixture = libsql_repo().await;
    let repo = fixture.repo.clone();
    let provider = Arc::new(RecordingEmbeddingProvider::<3>);
    let indexer = Arc::new(
        ChunkingMemoryDocumentIndexer::new(repo.clone()).with_embedding_provider(provider.clone()),
    );
    let backend = RepositoryMemoryBackend::new(repo)
        .with_indexer(indexer)
        .with_embedding_provider(provider)
        .with_capabilities(MemoryBackendCapabilities {
            file_documents: true,
            full_text_search: true,
            vector_search: true,
            embeddings: true,
            ..MemoryBackendCapabilities::default()
        });
    let context = MemoryContext::new(MemoryDocumentScope::new("tenant-a", "alice", None).unwrap());
    for (relative_path, content) in [
        ("notes/hybrid.md", b"literal hybrid-vector".as_slice()),
        ("notes/fts-only.md", b"literal unrelated".as_slice()),
        ("notes/vector-only.md", b"semantic-only".as_slice()),
    ] {
        let path = MemoryDocumentPath::new("tenant-a", "alice", None, relative_path).expect("path");
        backend
            .write_document(&context, &path, content)
            .await
            .unwrap();
    }

    let results = backend
        .search(
            &context,
            MemorySearchRequest::new("literal")
                .unwrap()
                .with_limit(3)
                .with_fusion_strategy(FusionStrategy::Rrf),
        )
        .await
        .unwrap();
    assert!(
        results
            .iter()
            .any(|r| r.path.relative_path() == "notes/hybrid.md"),
        "rrf fusion must surface the hybrid match"
    );
    let hybrid = results
        .iter()
        .find(|r| r.path.relative_path() == "notes/hybrid.md")
        .unwrap();
    assert!(
        hybrid.is_hybrid(),
        "hybrid match must report is_hybrid=true"
    );
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_backend_weighted_score_fusion_orders_results_by_weights() {
    let fixture = libsql_repo().await;
    let repo = fixture.repo.clone();
    let provider = Arc::new(RecordingEmbeddingProvider::<3>);
    let indexer = Arc::new(
        ChunkingMemoryDocumentIndexer::new(repo.clone()).with_embedding_provider(provider.clone()),
    );
    let backend = RepositoryMemoryBackend::new(repo)
        .with_indexer(indexer)
        .with_embedding_provider(provider)
        .with_capabilities(MemoryBackendCapabilities {
            file_documents: true,
            full_text_search: true,
            vector_search: true,
            embeddings: true,
            ..MemoryBackendCapabilities::default()
        });
    let context = MemoryContext::new(MemoryDocumentScope::new("tenant-a", "alice", None).unwrap());

    for (relative_path, content) in [
        ("hybrid.md", b"literal hybrid-vector".as_slice()),
        ("fts.md", b"literal unrelated".as_slice()),
    ] {
        let path = MemoryDocumentPath::new("tenant-a", "alice", None, relative_path).expect("path");
        backend
            .write_document(&context, &path, content)
            .await
            .unwrap();
    }

    // Heavy full-text weight + minimal vector weight: the FTS-only doc
    // (`fts.md`, ranked higher in FTS, absent from vector) must lead the
    // hybrid doc (`hybrid.md`, also FTS but additionally pulled by
    // vector). With WeightedScore = w_ft / rank + w_vec / rank, swinging
    // the weights swings the order — proving the strategy actually
    // consumes the weights instead of behaving like RRF.
    let fts_heavy = backend
        .search(
            &context,
            MemorySearchRequest::new("literal")
                .unwrap()
                .with_limit(3)
                .with_fusion_strategy(FusionStrategy::WeightedScore)
                .with_full_text_weight(10.0)
                .with_vector_weight(0.001),
        )
        .await
        .unwrap();
    assert!(!fts_heavy.is_empty(), "weighted-score must produce results");

    let vec_heavy = backend
        .search(
            &context,
            MemorySearchRequest::new("literal")
                .unwrap()
                .with_limit(3)
                .with_fusion_strategy(FusionStrategy::WeightedScore)
                .with_full_text_weight(0.001)
                .with_vector_weight(10.0),
        )
        .await
        .unwrap();

    let fts_top = fts_heavy[0].path.relative_path().to_string();
    let vec_top = vec_heavy[0].path.relative_path().to_string();
    assert_ne!(
        fts_top, vec_top,
        "weighted-score must reorder when weights flip; got fts_top={fts_top}, vec_top={vec_top}"
    );
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_backend_search_honors_pre_fusion_limit_per_branch() {
    // The API clamps `pre_fusion_limit` up to at least `limit`, so the
    // observable cap is `pre_fusion_limit` only when it >= limit. Use
    // limit=2, pre_fusion_limit=2 (effective per-branch cap = 2): with 6
    // matching docs, the FTS branch must contribute at most 2 candidates,
    // and fusion must therefore output at most 2 paths.
    let fixture = libsql_repo().await;
    let repo = fixture.repo.clone();
    let indexer = Arc::new(
        ChunkingMemoryDocumentIndexer::new(repo.clone()).with_chunk_config(ChunkConfig {
            chunk_size: 4,
            overlap_percent: 0.0,
            min_chunk_size: 1,
        }),
    );
    let backend = RepositoryMemoryBackend::new(repo)
        .with_indexer(indexer)
        .with_capabilities(MemoryBackendCapabilities {
            file_documents: true,
            full_text_search: true,
            ..MemoryBackendCapabilities::default()
        });
    let context = MemoryContext::new(MemoryDocumentScope::new("tenant-a", "alice", None).unwrap());

    for index in 0..6 {
        let path = MemoryDocumentPath::new(
            "tenant-a",
            "alice",
            None,
            format!("doc-{index}.md").as_str(),
        )
        .expect("path");
        backend
            .write_document(&context, &path, b"shared-keyword body content")
            .await
            .unwrap();
    }

    // Establish the precondition first: with no per-branch cap, the FTS
    // branch must surface every matching document. Without this, the
    // `<= 2` assertion below would be vacuously satisfied by an empty or
    // 1-result response from a regression that broke FTS or indexing.
    let unbounded = backend
        .search(
            &context,
            MemorySearchRequest::new("shared-keyword")
                .unwrap()
                .with_limit(10)
                .with_pre_fusion_limit(10)
                .with_vector(false),
        )
        .await
        .unwrap();
    assert!(
        unbounded.len() >= 6,
        "expected at least 6 unbounded matches before testing the per-branch cap, got {}",
        unbounded.len()
    );

    let request = MemorySearchRequest::new("shared-keyword")
        .unwrap()
        .with_limit(2)
        .with_pre_fusion_limit(2)
        .with_vector(false);
    // Sanity-check: API clamps pre_fusion_limit >= limit (here both 2).
    assert_eq!(request.pre_fusion_limit(), 2);

    let results = backend.search(&context, request).await.unwrap();
    assert_eq!(
        results.len(),
        2,
        "pre_fusion_limit=2 must truncate to exactly 2 fused candidates with 6 matches available"
    );
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_backend_search_with_many_candidates_caps_pre_fusion_per_branch() {
    // zmanian test gap 7: drive the per-branch SQL `LIMIT` against >50
    // matching candidates so a regression that drops the per-branch cap
    // and lets every match flow into fusion would surface as either a
    // non-deterministic ranking shift or a result-count blowup. The
    // contract is: `pre_fusion_limit` caps each branch's candidate set
    // before fusion, and the final result set respects `limit`.
    let fixture = libsql_repo().await;
    let repo = fixture.repo.clone();
    let indexer = Arc::new(
        ChunkingMemoryDocumentIndexer::new(repo.clone()).with_chunk_config(ChunkConfig {
            chunk_size: 4,
            overlap_percent: 0.0,
            min_chunk_size: 1,
        }),
    );
    let backend = RepositoryMemoryBackend::new(repo)
        .with_indexer(indexer)
        .with_capabilities(MemoryBackendCapabilities {
            file_documents: true,
            full_text_search: true,
            ..MemoryBackendCapabilities::default()
        });
    let context = MemoryContext::new(MemoryDocumentScope::new("tenant-a", "alice", None).unwrap());

    // 60 docs all containing the same searchable token — well above the
    // 50-candidate threshold zmanian called out.
    for index in 0..60 {
        let path = MemoryDocumentPath::new("tenant-a", "alice", None, format!("doc-{index}.md"))
            .expect("path");
        backend
            .write_document(&context, &path, b"shared-needle body content")
            .await
            .unwrap();
    }

    // Establish that the unbounded candidate set is at least 50 — a
    // regression that broke FTS or indexing would shrink this and make
    // the `limit` assertion below vacuous.
    let unbounded = backend
        .search(
            &context,
            MemorySearchRequest::new("shared-needle")
                .unwrap()
                .with_limit(60)
                .with_pre_fusion_limit(60)
                .with_vector(false),
        )
        .await
        .unwrap();
    assert!(
        unbounded.len() >= 50,
        "expected ≥50 unbounded matches before exercising the clamp; got {}",
        unbounded.len()
    );

    // pre_fusion_limit=20 with limit=10: the per-branch SQL must cap to
    // 20 candidates, fusion outputs at most 10. The result set must
    // contain unique paths and match the limit exactly.
    let request = MemorySearchRequest::new("shared-needle")
        .unwrap()
        .with_limit(10)
        .with_pre_fusion_limit(20)
        .with_vector(false);
    assert_eq!(request.limit(), 10);
    assert_eq!(request.pre_fusion_limit(), 20);

    let results = backend.search(&context, request).await.unwrap();
    assert_eq!(
        results.len(),
        10,
        "limit=10 with 60 candidates must yield exactly 10 results"
    );
    let unique: std::collections::HashSet<&str> =
        results.iter().map(|r| r.path.relative_path()).collect();
    assert_eq!(
        unique.len(),
        10,
        "result paths must be unique; got duplicates in {unique:?}"
    );
}

#[cfg(feature = "libsql")]
#[test]
fn pre_fusion_limit_is_clamped_up_to_limit() {
    // Lock in the contract that `with_pre_fusion_limit(N)` is at least
    // `limit` regardless of caller order. Without this clamp, the per-
    // branch SQL `LIMIT` could be smaller than the post-fusion `limit`,
    // silently shrinking the candidate set.
    let request = MemorySearchRequest::new("x")
        .unwrap()
        .with_limit(5)
        .with_pre_fusion_limit(2);
    assert_eq!(
        request.pre_fusion_limit(),
        5,
        "pre_fusion_limit must clamp up to limit"
    );

    let request = MemorySearchRequest::new("x")
        .unwrap()
        .with_limit(5)
        .with_pre_fusion_limit(20);
    assert_eq!(
        request.pre_fusion_limit(),
        20,
        "pre_fusion_limit above limit must be preserved"
    );

    // Reverse-order regression: a small `pre_fusion_limit` set while `limit`
    // is small, followed by a *larger* `with_limit`, must re-clamp
    // `pre_fusion_limit` up. Before this fix `with_limit` did not touch
    // `pre_fusion_limit`, so the per-branch SQL `LIMIT` could be smaller than
    // the requested final limit.
    let request = MemorySearchRequest::new("x")
        .unwrap()
        .with_limit(2)
        .with_pre_fusion_limit(2)
        .with_limit(5);
    assert_eq!(
        request.pre_fusion_limit(),
        5,
        "pre_fusion_limit must clamp up when a later with_limit raises the floor"
    );

    // Raising limit must never narrow a pre_fusion_limit that is already above
    // the new limit.
    let request = MemorySearchRequest::new("x")
        .unwrap()
        .with_limit(2)
        .with_pre_fusion_limit(20)
        .with_limit(5);
    assert_eq!(
        request.pre_fusion_limit(),
        20,
        "pre_fusion_limit above limit must be preserved across with_limit"
    );
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_backend_search_honors_limit() {
    let fixture = libsql_repo().await;
    let repo = fixture.repo.clone();
    let indexer = Arc::new(
        ChunkingMemoryDocumentIndexer::new(repo.clone()).with_chunk_config(ChunkConfig {
            chunk_size: 4,
            overlap_percent: 0.0,
            min_chunk_size: 1,
        }),
    );
    let backend = RepositoryMemoryBackend::new(repo)
        .with_indexer(indexer)
        .with_capabilities(MemoryBackendCapabilities {
            file_documents: true,
            full_text_search: true,
            ..MemoryBackendCapabilities::default()
        });
    let context = MemoryContext::new(MemoryDocumentScope::new("tenant-a", "alice", None).unwrap());

    for index in 0..5 {
        let path = MemoryDocumentPath::new(
            "tenant-a",
            "alice",
            None,
            format!("doc-{index}.md").as_str(),
        )
        .expect("path");
        backend
            .write_document(&context, &path, b"shared-keyword body content")
            .await
            .unwrap();
    }

    // Establish the precondition: an unbounded search surfaces every
    // matching doc. Without this baseline a regression that broke FTS or
    // indexing entirely would still satisfy `results.len() <= 2`.
    let unbounded = backend
        .search(
            &context,
            MemorySearchRequest::new("shared-keyword")
                .unwrap()
                .with_limit(10),
        )
        .await
        .unwrap();
    assert!(
        unbounded.len() >= 5,
        "expected at least 5 unbounded matches before testing limit truncation, got {}",
        unbounded.len()
    );

    let results = backend
        .search(
            &context,
            MemorySearchRequest::new("shared-keyword")
                .unwrap()
                .with_limit(2),
        )
        .await
        .unwrap();
    assert_eq!(
        results.len(),
        2,
        "limit must truncate to exactly 2 results with 5 matches available"
    );
}

// --- Postgres -------------------------------------------------------------

#[cfg(feature = "postgres")]
fn pg_pool() -> deadpool_postgres::Pool {
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://localhost/ironclaw_test".to_string());
    let config: tokio_postgres::Config = database_url
        .parse()
        .expect("DATABASE_URL must be a valid Postgres URL");
    let mgr = deadpool_postgres::Manager::new(config, tokio_postgres::NoTls);
    deadpool_postgres::Pool::builder(mgr)
        .max_size(4)
        .build()
        .expect("build deadpool")
}

/// Explicit opt-in to skip the Postgres backend tests. Without this set,
/// a connection failure must fail loud — the previous "silent skip + green
/// pass" pattern violated the `ironclaw_memory` guardrail that Postgres
/// behavioral coverage must be real.
#[cfg(feature = "postgres")]
const POSTGRES_SKIP_ENV: &str = "IRONCLAW_SKIP_POSTGRES_TESTS";

#[cfg(feature = "postgres")]
fn pg_skip_requested() -> bool {
    std::env::var(POSTGRES_SKIP_ENV).is_ok_and(|value| value == "1" || value == "true")
}

#[cfg(feature = "postgres")]
async fn pg_require_connection(pool: &deadpool_postgres::Pool) -> Option<()> {
    match pool.get().await {
        Ok(_) => Some(()),
        Err(error) => {
            if pg_skip_requested() {
                eprintln!("skipping reborn-postgres backend test ({POSTGRES_SKIP_ENV}=1): {error}");
                None
            } else {
                panic!(
                    "reborn-postgres backend test could not reach Postgres ({error}); \
                     set DATABASE_URL to a reachable Postgres+pgvector instance, or set \
                     {POSTGRES_SKIP_ENV}=1 to explicitly skip."
                );
            }
        }
    }
}

#[cfg(feature = "postgres")]
async fn pg_cleanup_tenant(pool: &deadpool_postgres::Pool, tenant_id: &str) {
    let Ok(client) = pool.get().await else { return };
    let _ = client
        .execute(
            "DELETE FROM reborn_memory_documents WHERE tenant_id = $1",
            &[&tenant_id],
        )
        .await;
}

#[cfg(feature = "postgres")]
async fn pg_repo(tenant_id: &str) -> Option<Arc<RebornPostgresMemoryDocumentRepository>> {
    let pool = pg_pool();
    pg_require_connection(&pool).await?;
    let repo = Arc::new(RebornPostgresMemoryDocumentRepository::new(pool.clone()));
    repo.run_migrations().await.expect("run_migrations");
    pg_cleanup_tenant(&pool, tenant_id).await;
    Some(repo)
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_backend_validates_schema_from_config_before_write() {
    use ironclaw_memory::MemoryDocumentRepository;
    let tenant = "reborn-pg-be-schema";
    let Some(repo) = pg_repo(tenant).await else {
        return;
    };
    let cfg_path =
        MemoryDocumentPath::new(tenant, "alice", None, "settings/.config").expect("path");
    repo.write_document(&cfg_path, b"").await.unwrap();
    repo.write_document_metadata(
        &cfg_path,
        &serde_json::json!({
            "schema": {
                "type": "object",
                "properties": {"provider": {"type": "string"}},
                "required": ["provider"],
            }
        }),
    )
    .await
    .unwrap();

    let backend = RepositoryMemoryBackend::new(repo.clone());
    let context = MemoryContext::new(MemoryDocumentScope::new(tenant, "alice", None).unwrap());
    let path = MemoryDocumentPath::new(tenant, "alice", None, "settings/llm.json").expect("path");

    let err = backend
        .write_document(&context, &path, br#"{"missing":"provider"}"#)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("schema validation failed"));
    let stored = repo.read_document(&path).await.unwrap();
    assert!(stored.is_none(), "schema-rejected write must not persist");
    pg_cleanup_tenant(&pg_pool(), tenant).await;
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_backend_honors_skip_versioning_from_config() {
    use ironclaw_memory::MemoryDocumentRepository;
    let tenant = "reborn-pg-be-skipver";
    let Some(repo) = pg_repo(tenant).await else {
        return;
    };
    let cfg_path = MemoryDocumentPath::new(tenant, "alice", None, "logs/.config").expect("path");
    repo.write_document(&cfg_path, b"").await.unwrap();
    repo.write_document_metadata(&cfg_path, &serde_json::json!({"skip_versioning": true}))
        .await
        .unwrap();

    let backend = RepositoryMemoryBackend::new(repo.clone());
    let context = MemoryContext::new(MemoryDocumentScope::new(tenant, "alice", None).unwrap());
    let path = MemoryDocumentPath::new(tenant, "alice", None, "logs/entry.md").expect("path");

    backend
        .write_document(&context, &path, b"v1")
        .await
        .unwrap();
    backend
        .write_document(&context, &path, b"v2")
        .await
        .unwrap();

    let pool = pg_pool();
    let client = pool.get().await.expect("get client");
    let row = client
        .query_one(
            "SELECT COUNT(*) FROM reborn_memory_document_versions v \
             JOIN reborn_memory_documents d ON d.id = v.document_id \
             WHERE d.tenant_id = $1",
            &[&tenant],
        )
        .await
        .unwrap();
    let count: i64 = row.get(0);
    assert_eq!(count, 0);
    pg_cleanup_tenant(&pool, tenant).await;
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_backend_reports_write_success_when_indexer_fails_after_persist() {
    use ironclaw_memory::MemoryDocumentRepository;
    let tenant = "reborn-pg-be-indexerfail";
    let Some(repo) = pg_repo(tenant).await else {
        return;
    };
    let backend = RepositoryMemoryBackend::new(repo.clone()).with_indexer(Arc::new(FailingIndexer));
    let context = MemoryContext::new(MemoryDocumentScope::new(tenant, "alice", None).unwrap());
    let path = MemoryDocumentPath::new(tenant, "alice", None, "doc.md").expect("path");

    backend
        .write_document(&context, &path, b"persisted")
        .await
        .expect("write must succeed even if the indexer fails after persist");

    let stored = repo.read_document(&path).await.unwrap();
    assert_eq!(stored.as_deref(), Some(b"persisted".as_slice()));
    pg_cleanup_tenant(&pg_pool(), tenant).await;
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_backend_search_fails_closed_on_query_embedding_dimension_mismatch() {
    let tenant = "reborn-pg-be-dimmm";
    let Some(repo) = pg_repo(tenant).await else {
        return;
    };
    let provider = Arc::new(RecordingEmbeddingProvider::<1536>);
    let backend = RepositoryMemoryBackend::new(repo)
        .with_embedding_provider(provider)
        .with_capabilities(MemoryBackendCapabilities {
            file_documents: true,
            full_text_search: false,
            vector_search: true,
            embeddings: true,
            ..MemoryBackendCapabilities::default()
        });
    let context = MemoryContext::new(MemoryDocumentScope::new(tenant, "alice", None).unwrap());

    let err = backend
        .search(
            &context,
            MemorySearchRequest::new("literal")
                .unwrap()
                .with_full_text(false)
                .with_query_embedding(vec![1.0, 0.0]),
        )
        .await
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("query embedding dimension 2 does not match"),
        "expected dimension-mismatch error, got: {err}"
    );
    pg_cleanup_tenant(&pg_pool(), tenant).await;
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_backend_index_fails_closed_when_provider_returns_wrong_dimension() {
    // zmanian test gap 2 (Postgres mirror): provider declares dim N,
    // returns dim M; the indexer must fail closed before any chunks are
    // written. The durable document row still lands because backend
    // writes treat indexer failures as best-effort.
    let tenant = "reborn-pg-be-write-dimmm";
    let Some(repo) = pg_repo(tenant).await else {
        return;
    };
    let provider = Arc::new(LyingEmbeddingProvider {
        declared: 4,
        actual: 2,
    });
    let indexer = Arc::new(
        ChunkingMemoryDocumentIndexer::new(repo.clone()).with_embedding_provider(provider.clone()),
    );
    let backend = RepositoryMemoryBackend::new(repo.clone())
        .with_indexer(indexer)
        .with_embedding_provider(provider)
        .with_capabilities(MemoryBackendCapabilities {
            file_documents: true,
            full_text_search: true,
            vector_search: true,
            embeddings: true,
            ..MemoryBackendCapabilities::default()
        });
    let context = MemoryContext::new(MemoryDocumentScope::new(tenant, "alice", None).unwrap());
    let path = MemoryDocumentPath::new(tenant, "alice", None, "lie.md").expect("path");

    backend
        .write_document(&context, &path, b"body content")
        .await
        .expect("durable write must succeed even if indexer fails closed");

    // Verify zero chunks were inserted via a direct SQL count.
    let client = pg_pool().get().await.expect("client");
    let count: i64 = client
        .query_one(
            "SELECT COUNT(*) FROM reborn_memory_chunks c \
             JOIN reborn_memory_documents d ON d.id = c.document_id \
             WHERE d.tenant_id = $1",
            &[&tenant],
        )
        .await
        .expect("count chunks")
        .get(0);
    assert_eq!(
        count, 0,
        "indexer must refuse to persist mis-dimensioned embeddings; got {count} chunks"
    );

    // Direct indexer invocation surfaces the dimension error explicitly.
    use ironclaw_memory::MemoryDocumentIndexer;
    let standalone_indexer = ChunkingMemoryDocumentIndexer::new(repo).with_embedding_provider(
        Arc::new(LyingEmbeddingProvider {
            declared: 4,
            actual: 2,
        }),
    );
    let err = standalone_indexer
        .reindex_document(&path)
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("dimension"),
        "expected dimension-mismatch error from indexer, got: {err}"
    );
    pg_cleanup_tenant(&pg_pool(), tenant).await;
}

// Postgres counterparts for the search/fusion/limit contract that previously
// only ran against libSQL. `RebornPostgresMemoryDocumentRepository` has its
// own FTS (`tsvector`) and pgvector query implementations, so a Postgres-only
// regression in ranking, fusion inputs, vector results, or limit handling
// would not be caught by the libSQL versions above.

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_backend_hybrid_search_fuses_full_text_and_vector_results() {
    let tenant = "reborn-pg-be-hybrid";
    let Some(repo) = pg_repo(tenant).await else {
        return;
    };
    let provider = Arc::new(RecordingEmbeddingProvider::<1536>);
    let indexer = Arc::new(
        ChunkingMemoryDocumentIndexer::new(repo.clone()).with_embedding_provider(provider.clone()),
    );
    let backend = RepositoryMemoryBackend::new(repo.clone())
        .with_indexer(indexer)
        .with_embedding_provider(provider)
        .with_capabilities(MemoryBackendCapabilities {
            file_documents: true,
            full_text_search: true,
            vector_search: true,
            embeddings: true,
            ..MemoryBackendCapabilities::default()
        });
    let context = MemoryContext::new(MemoryDocumentScope::new(tenant, "alice", None).unwrap());
    for (relative_path, content) in [
        ("notes/hybrid.md", b"literal hybrid-vector".as_slice()),
        ("notes/fts-only.md", b"literal unrelated".as_slice()),
        ("notes/vector-only.md", b"semantic-only".as_slice()),
    ] {
        let path = MemoryDocumentPath::new(tenant, "alice", None, relative_path).expect("path");
        backend
            .write_document(&context, &path, content)
            .await
            .unwrap();
    }

    let results = backend
        .search(
            &context,
            MemorySearchRequest::new("literal")
                .unwrap()
                .with_limit(3)
                .with_fusion_strategy(FusionStrategy::Rrf),
        )
        .await
        .unwrap();
    assert!(
        results
            .iter()
            .any(|r| r.path.relative_path() == "notes/hybrid.md"),
        "rrf fusion must surface the hybrid match"
    );
    let hybrid = results
        .iter()
        .find(|r| r.path.relative_path() == "notes/hybrid.md")
        .unwrap();
    assert!(
        hybrid.is_hybrid(),
        "hybrid match must report is_hybrid=true"
    );
    pg_cleanup_tenant(&pg_pool(), tenant).await;
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_backend_weighted_score_fusion_orders_results_by_weights() {
    let tenant = "reborn-pg-be-weighted";
    let Some(repo) = pg_repo(tenant).await else {
        return;
    };
    let provider = Arc::new(RecordingEmbeddingProvider::<1536>);
    let indexer = Arc::new(
        ChunkingMemoryDocumentIndexer::new(repo.clone()).with_embedding_provider(provider.clone()),
    );
    let backend = RepositoryMemoryBackend::new(repo.clone())
        .with_indexer(indexer)
        .with_embedding_provider(provider)
        .with_capabilities(MemoryBackendCapabilities {
            file_documents: true,
            full_text_search: true,
            vector_search: true,
            embeddings: true,
            ..MemoryBackendCapabilities::default()
        });
    let context = MemoryContext::new(MemoryDocumentScope::new(tenant, "alice", None).unwrap());

    // Asymmetric FTS relevance: `fts.md` has many more "literal" matches so
    // Postgres `ts_rank_cd` ranks it strictly above `hybrid.md` on the FTS
    // branch — without this, the two docs tie under `plainto_tsquery` and
    // the order across the FTS-heavy and vector-heavy searches becomes
    // non-deterministic. `hybrid.md` is the only doc whose embedding matches
    // the query bucket, so the vector branch deterministically prefers it.
    for (relative_path, content) in [
        ("hybrid.md", b"literal hybrid-vector".as_slice()),
        (
            "fts.md",
            b"literal literal literal literal unrelated".as_slice(),
        ),
    ] {
        let path = MemoryDocumentPath::new(tenant, "alice", None, relative_path).expect("path");
        backend
            .write_document(&context, &path, content)
            .await
            .unwrap();
    }

    let fts_heavy = backend
        .search(
            &context,
            MemorySearchRequest::new("literal")
                .unwrap()
                .with_limit(3)
                .with_fusion_strategy(FusionStrategy::WeightedScore)
                .with_full_text_weight(10.0)
                .with_vector_weight(0.001),
        )
        .await
        .unwrap();
    assert!(!fts_heavy.is_empty(), "weighted-score must produce results");

    let vec_heavy = backend
        .search(
            &context,
            MemorySearchRequest::new("literal")
                .unwrap()
                .with_limit(3)
                .with_fusion_strategy(FusionStrategy::WeightedScore)
                .with_full_text_weight(0.001)
                .with_vector_weight(10.0),
        )
        .await
        .unwrap();

    let fts_top = fts_heavy[0].path.relative_path().to_string();
    let vec_top = vec_heavy[0].path.relative_path().to_string();
    assert_eq!(
        fts_top, "fts.md",
        "fts-heavy weights must surface the FTS leader; got {fts_top}"
    );
    assert_eq!(
        vec_top, "hybrid.md",
        "vector-heavy weights must surface the vector leader; got {vec_top}"
    );
    pg_cleanup_tenant(&pg_pool(), tenant).await;
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_backend_search_honors_pre_fusion_limit_per_branch() {
    let tenant = "reborn-pg-be-prefusion";
    let Some(repo) = pg_repo(tenant).await else {
        return;
    };
    let indexer = Arc::new(
        ChunkingMemoryDocumentIndexer::new(repo.clone()).with_chunk_config(ChunkConfig {
            chunk_size: 4,
            overlap_percent: 0.0,
            min_chunk_size: 1,
        }),
    );
    let backend = RepositoryMemoryBackend::new(repo.clone())
        .with_indexer(indexer)
        .with_capabilities(MemoryBackendCapabilities {
            file_documents: true,
            full_text_search: true,
            ..MemoryBackendCapabilities::default()
        });
    let context = MemoryContext::new(MemoryDocumentScope::new(tenant, "alice", None).unwrap());

    for index in 0..6 {
        let path =
            MemoryDocumentPath::new(tenant, "alice", None, format!("doc-{index}.md").as_str())
                .expect("path");
        backend
            .write_document(&context, &path, b"shared-keyword body content")
            .await
            .unwrap();
    }

    let unbounded = backend
        .search(
            &context,
            MemorySearchRequest::new("shared-keyword")
                .unwrap()
                .with_limit(10)
                .with_pre_fusion_limit(10)
                .with_vector(false),
        )
        .await
        .unwrap();
    assert!(
        unbounded.len() >= 6,
        "expected at least 6 unbounded matches before testing the per-branch cap, got {}",
        unbounded.len()
    );

    let request = MemorySearchRequest::new("shared-keyword")
        .unwrap()
        .with_limit(2)
        .with_pre_fusion_limit(2)
        .with_vector(false);
    assert_eq!(request.pre_fusion_limit(), 2);

    let results = backend.search(&context, request).await.unwrap();
    assert_eq!(
        results.len(),
        2,
        "pre_fusion_limit=2 must truncate to exactly 2 fused candidates with 6 matches available"
    );
    pg_cleanup_tenant(&pg_pool(), tenant).await;
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn postgres_backend_search_honors_limit() {
    let tenant = "reborn-pg-be-limit";
    let Some(repo) = pg_repo(tenant).await else {
        return;
    };
    let indexer = Arc::new(
        ChunkingMemoryDocumentIndexer::new(repo.clone()).with_chunk_config(ChunkConfig {
            chunk_size: 4,
            overlap_percent: 0.0,
            min_chunk_size: 1,
        }),
    );
    let backend = RepositoryMemoryBackend::new(repo.clone())
        .with_indexer(indexer)
        .with_capabilities(MemoryBackendCapabilities {
            file_documents: true,
            full_text_search: true,
            ..MemoryBackendCapabilities::default()
        });
    let context = MemoryContext::new(MemoryDocumentScope::new(tenant, "alice", None).unwrap());

    for index in 0..5 {
        let path =
            MemoryDocumentPath::new(tenant, "alice", None, format!("doc-{index}.md").as_str())
                .expect("path");
        backend
            .write_document(&context, &path, b"shared-keyword body content")
            .await
            .unwrap();
    }

    let unbounded = backend
        .search(
            &context,
            MemorySearchRequest::new("shared-keyword")
                .unwrap()
                .with_limit(10),
        )
        .await
        .unwrap();
    assert!(
        unbounded.len() >= 5,
        "expected at least 5 unbounded matches before testing limit truncation, got {}",
        unbounded.len()
    );

    let results = backend
        .search(
            &context,
            MemorySearchRequest::new("shared-keyword")
                .unwrap()
                .with_limit(2),
        )
        .await
        .unwrap();
    assert_eq!(
        results.len(),
        2,
        "limit must truncate to exactly 2 results with 5 matches available"
    );
    pg_cleanup_tenant(&pg_pool(), tenant).await;
}
