//! E2E persistence + versioning coverage for the reborn memory substrate.
//!
//! Targets PR #3180 invariants 8 and 9: `MemoryAppendOutcome` semantics, version
//! hash chain, durability across DB reopen, migration idempotency, and the
//! "no orphan chunks after replace" invariant. Vertical: `RepositoryMemoryBackend`
//! → `LibSqlMemoryDocumentRepository`, with `ChunkingMemoryDocumentIndexer`
//! attached so we exercise the chunk-replace path too.

#[cfg(feature = "libsql")]
mod libsql_e2e {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use ironclaw_memory::{
        ChunkConfig, ChunkingMemoryDocumentIndexer, EmbeddingError, EmbeddingProvider,
        LibSqlMemoryDocumentRepository, MemoryAppendOutcome, MemoryBackend,
        MemoryBackendCapabilities, MemoryContext, MemoryDocumentPath, MemoryDocumentRepository,
        MemoryDocumentScope, RepositoryMemoryBackend, content_sha256,
    };

    const OWNER_KEY: &str = "tenant:tenant-a:user:alice:project:project-1";

    #[tokio::test]
    async fn version_chain_records_each_replace_with_correct_hash_under_libsql() {
        let (db, _dir) = libsql_db().await;
        let repository = Arc::new(LibSqlMemoryDocumentRepository::new(db.clone()));
        repository.run_migrations().await.unwrap();
        let backend = Arc::new(RepositoryMemoryBackend::new(repository.clone()));
        let context = MemoryContext::new(scope());
        let path = doc_path("notes/versioned.md");

        for body in ["v1-content", "v2-content", "v3-content"] {
            backend
                .write_document(&context, &path, body.as_bytes())
                .await
                .unwrap();
        }

        // Final document content reflects the last replace.
        let stored = repository.read_document(&path).await.unwrap().unwrap();
        assert_eq!(stored, b"v3-content");

        // Version table records the prior contents, hashed via content_sha256.
        let versions = read_versions(&db, "notes/versioned.md").await;
        assert_eq!(
            versions
                .iter()
                .map(|(content, hash, _)| (content.as_str(), hash.as_str()))
                .collect::<Vec<_>>(),
            vec![
                ("v1-content", content_sha256("v1-content").as_str()),
                ("v2-content", content_sha256("v2-content").as_str()),
            ],
        );
        // changed_by is the scoped owner key for every version row.
        for (_, _, changed_by) in &versions {
            assert_eq!(changed_by.as_deref(), Some(OWNER_KEY));
        }
    }

    #[tokio::test]
    async fn compare_and_append_returns_appended_then_conflict_then_appended_with_fresh_hash() {
        let (db, _dir) = libsql_db().await;
        let repository = Arc::new(LibSqlMemoryDocumentRepository::new(db.clone()));
        repository.run_migrations().await.unwrap();
        let backend = Arc::new(RepositoryMemoryBackend::new(repository.clone()));
        let context = MemoryContext::new(scope());
        let path = doc_path("notes/append.md");

        // Initial state.
        backend
            .write_document(&context, &path, b"base")
            .await
            .unwrap();
        let h_base = content_sha256("base");

        // Append against the correct current hash → Appended.
        let outcome = backend
            .compare_and_append_document(&context, &path, Some(&h_base), b"-x")
            .await
            .unwrap();
        assert_eq!(outcome, MemoryAppendOutcome::Appended);
        assert_eq!(
            repository.read_document(&path).await.unwrap().unwrap(),
            b"base-x"
        );

        // Replay with the now-stale hash → Conflict; document is unchanged.
        let outcome = backend
            .compare_and_append_document(&context, &path, Some(&h_base), b"-y")
            .await
            .unwrap();
        assert_eq!(outcome, MemoryAppendOutcome::Conflict);
        assert_eq!(
            repository.read_document(&path).await.unwrap().unwrap(),
            b"base-x"
        );

        // Recompute the fresh hash and append again → Appended.
        let h_after_x = content_sha256("base-x");
        let outcome = backend
            .compare_and_append_document(&context, &path, Some(&h_after_x), b"-z")
            .await
            .unwrap();
        assert_eq!(outcome, MemoryAppendOutcome::Appended);
        assert_eq!(
            repository.read_document(&path).await.unwrap().unwrap(),
            b"base-x-z"
        );
    }

    #[tokio::test]
    async fn documents_chunks_versions_durable_across_repository_reopen() {
        // Build a libSQL DB on a real temp file, write through the full
        // stack (chunking + indexing + version-on-overwrite), then drop
        // EVERY handle that holds an Arc to the database — repository,
        // backend, indexer, provider, AND the `libsql::Database` itself.
        // Re-open by constructing a fresh `libsql::Database` from the
        // same path. This is the only shape that exercises true on-disk
        // durability: re-wrapping the same `Arc<libsql::Database>` would
        // pass even for a bug that only surfaces after the OS-level file
        // handle is closed and reopened.
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("memory.db");

        {
            let db = Arc::new(libsql::Builder::new_local(&db_path).build().await.unwrap());
            let provider = Arc::new(DeterministicEmbeddingProvider::default());
            let repository = Arc::new(LibSqlMemoryDocumentRepository::new(db.clone()));
            repository.run_migrations().await.unwrap();
            let indexer = Arc::new(
                ChunkingMemoryDocumentIndexer::new(repository.clone())
                    .with_chunk_config(ChunkConfig {
                        chunk_size: 6,
                        overlap_percent: 0.0,
                        min_chunk_size: 1,
                    })
                    .with_embedding_provider(provider.clone()),
            );
            let backend = Arc::new(
                RepositoryMemoryBackend::new(repository.clone())
                    .with_indexer(indexer)
                    .with_embedding_provider(provider.clone())
                    .with_capabilities(full_capabilities()),
            );
            let context = MemoryContext::new(scope());
            backend
                .write_document(
                    &context,
                    &doc_path("notes/durable.md"),
                    b"durable content body",
                )
                .await
                .unwrap();
            // Explicit drops to make the close-then-reopen intent visible
            // at the call site (otherwise the values just go out of scope
            // at the block's `}`).
            drop(backend);
            drop(repository);
            drop(provider);
            drop(db);
        }

        // Re-open: a brand new `libsql::Database` against the same path,
        // not a re-wrapped Arc. Migrations are idempotent so the second
        // run is a no-op against the existing schema.
        let db_reopened = Arc::new(libsql::Builder::new_local(&db_path).build().await.unwrap());
        let repository_reopened =
            Arc::new(LibSqlMemoryDocumentRepository::new(db_reopened.clone()));
        repository_reopened.run_migrations().await.unwrap();
        let stored = repository_reopened
            .read_document(&doc_path("notes/durable.md"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored, b"durable content body");

        // Document and chunk rows survived the close-then-reopen.
        assert_eq!(count_documents(&db_reopened, "notes/durable.md").await, 1);
        assert!(count_chunks(&db_reopened, "notes/durable.md").await >= 1);
        // A first write has no prior content, so no version row exists.
        // Asserting `== 0` here was tautological against `read_versions`
        // for an unwritten path (zmanian's review): the strong test is
        // to perform a SECOND write through a fresh backend on the
        // reopened handle and assert exactly one prior-content version
        // row appears. That actually exercises version-durability across
        // the drop.
        assert_eq!(count_versions(&db_reopened, "notes/durable.md").await, 0);

        let provider_reopened = Arc::new(DeterministicEmbeddingProvider::default());
        let indexer_reopened = Arc::new(
            ChunkingMemoryDocumentIndexer::new(repository_reopened.clone())
                .with_chunk_config(ChunkConfig {
                    chunk_size: 6,
                    overlap_percent: 0.0,
                    min_chunk_size: 1,
                })
                .with_embedding_provider(provider_reopened.clone()),
        );
        let backend_reopened = Arc::new(
            RepositoryMemoryBackend::new(repository_reopened.clone())
                .with_indexer(indexer_reopened)
                .with_embedding_provider(provider_reopened)
                .with_capabilities(full_capabilities()),
        );
        let context = MemoryContext::new(scope());
        backend_reopened
            .write_document(
                &context,
                &doc_path("notes/durable.md"),
                b"durable content body v2",
            )
            .await
            .unwrap();
        assert_eq!(count_versions(&db_reopened, "notes/durable.md").await, 1);
        assert_eq!(
            repository_reopened
                .read_document(&doc_path("notes/durable.md"))
                .await
                .unwrap()
                .unwrap(),
            b"durable content body v2"
        );
    }

    #[tokio::test]
    async fn run_migrations_is_idempotent_and_preserves_data() {
        let (db, _dir) = libsql_db().await;
        let repository = Arc::new(LibSqlMemoryDocumentRepository::new(db.clone()));
        repository.run_migrations().await.unwrap();

        let backend = Arc::new(RepositoryMemoryBackend::new(repository.clone()));
        let context = MemoryContext::new(scope());
        backend
            .write_document(&context, &doc_path("notes/seed.md"), b"seed-content")
            .await
            .unwrap();

        // Re-running migrations is idempotent: no panic, no schema regression.
        repository.run_migrations().await.unwrap();

        // Data preserved.
        assert_eq!(
            repository
                .read_document(&doc_path("notes/seed.md"))
                .await
                .unwrap()
                .unwrap(),
            b"seed-content"
        );

        // The four canonical tables exist exactly once.
        let tables = list_user_tables(&db).await;
        for canonical in [
            "memory_documents",
            "memory_chunks",
            "memory_chunks_fts",
            "memory_document_versions",
        ] {
            assert_eq!(
                tables.iter().filter(|t| t.as_str() == canonical).count(),
                1,
                "expected exactly one {canonical} table, found {:?}",
                tables,
            );
        }
    }

    #[tokio::test]
    async fn replace_overwrites_chunks_atomically_with_no_orphans() {
        let (db, _dir) = libsql_db().await;
        let repository = Arc::new(LibSqlMemoryDocumentRepository::new(db.clone()));
        repository.run_migrations().await.unwrap();
        let provider = Arc::new(DeterministicEmbeddingProvider::default());
        let indexer = Arc::new(
            ChunkingMemoryDocumentIndexer::new(repository.clone())
                .with_chunk_config(ChunkConfig {
                    chunk_size: 4,
                    overlap_percent: 0.0,
                    min_chunk_size: 1,
                })
                .with_embedding_provider(provider.clone()),
        );
        let backend = Arc::new(
            RepositoryMemoryBackend::new(repository.clone())
                .with_indexer(indexer)
                .with_embedding_provider(provider)
                .with_capabilities(full_capabilities()),
        );
        let context = MemoryContext::new(scope());
        let path = doc_path("notes/chunked.md");

        // First write produces multiple chunks (long content / chunk_size=4 words).
        backend
            .write_document(&context, &path, b"alpha bravo charlie delta echo foxtrot")
            .await
            .unwrap();
        let initial_chunk_count = count_chunks(&db, "notes/chunked.md").await;
        assert!(
            initial_chunk_count >= 2,
            "expected multiple chunks for long content, got {initial_chunk_count}"
        );

        // Rewrite with shorter content. Chunk row count must drop to match the new
        // content; no chunk row may still reference the old text.
        backend
            .write_document(&context, &path, b"single")
            .await
            .unwrap();

        let final_chunks = chunks_for_path(&db, "notes/chunked.md").await;
        assert!(
            !final_chunks.is_empty(),
            "expected at least one chunk after rewrite"
        );
        for chunk in &final_chunks {
            // No leftover chunk content from the first write.
            for stale_token in ["alpha", "bravo", "charlie", "delta", "echo", "foxtrot"] {
                assert!(
                    !chunk.contains(stale_token),
                    "orphan chunk references prior content {stale_token}: {chunk:?}",
                );
            }
        }
        // memory_documents row count unchanged (replace, not insert).
        assert_eq!(count_documents(&db, "notes/chunked.md").await, 1);
    }

    #[tokio::test]
    async fn compare_and_append_emits_distinct_version_row_per_logical_change() {
        let (db, _dir) = libsql_db().await;
        let repository = Arc::new(LibSqlMemoryDocumentRepository::new(db.clone()));
        repository.run_migrations().await.unwrap();
        let backend = Arc::new(RepositoryMemoryBackend::new(repository.clone()));
        let context = MemoryContext::new(scope());
        let path = doc_path("notes/append-versioned.md");

        backend
            .write_document(&context, &path, b"hello ")
            .await
            .unwrap();
        let h0 = content_sha256("hello ");
        let outcome = backend
            .compare_and_append_document(&context, &path, Some(&h0), b"world")
            .await
            .unwrap();
        assert_eq!(outcome, MemoryAppendOutcome::Appended);

        assert_eq!(
            repository.read_document(&path).await.unwrap().unwrap(),
            b"hello world"
        );
        // Version row count after one initial write + one successful
        // append: EXACTLY one prior-content version row (capturing the
        // initial `"hello "`). Asserting `!versions.is_empty()` would
        // still pass if `compare_and_append_document` regressed to
        // emitting duplicate version rows for a single logical change —
        // exactly the row-cardinality regression this test is here to
        // catch.
        let versions = read_versions(&db, "notes/append-versioned.md").await;
        assert_eq!(
            versions.len(),
            1,
            "compare_and_append must emit exactly one version row per logical change \
             (initial write has no prior content; append produces one prior-content row); \
             got {} rows: {:?}",
            versions.len(),
            versions,
        );
        let (prior_content, prior_hash, _) = &versions[0];
        assert_eq!(prior_content, "hello ");
        assert_eq!(prior_hash, &content_sha256("hello "));
    }

    // ----- helpers -----

    async fn libsql_db() -> (Arc<libsql::Database>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("memory.db");
        let db = Arc::new(libsql::Builder::new_local(db_path).build().await.unwrap());
        (db, dir)
    }

    fn scope() -> MemoryDocumentScope {
        MemoryDocumentScope::new("tenant-a", "alice", Some("project-1")).unwrap()
    }

    fn doc_path(relative: &str) -> MemoryDocumentPath {
        MemoryDocumentPath::new("tenant-a", "alice", Some("project-1"), relative).unwrap()
    }

    fn full_capabilities() -> MemoryBackendCapabilities {
        MemoryBackendCapabilities {
            file_documents: true,
            metadata: true,
            versioning: true,
            full_text_search: true,
            vector_search: true,
            embeddings: true,
            ..MemoryBackendCapabilities::default()
        }
    }

    async fn read_versions(
        db: &Arc<libsql::Database>,
        relative_path: &str,
    ) -> Vec<(String, String, Option<String>)> {
        let conn = db.connect().unwrap();
        let mut rows = conn
            .query(
                r#"
                SELECT v.content, v.content_hash, v.changed_by
                FROM memory_document_versions v
                JOIN memory_documents d ON d.id = v.document_id
                WHERE d.user_id = ?1 AND d.path = ?2
                ORDER BY v.version
                "#,
                libsql::params![OWNER_KEY, relative_path],
            )
            .await
            .unwrap();
        let mut versions = Vec::new();
        while let Some(row) = rows.next().await.unwrap() {
            versions.push((
                row.get::<String>(0).unwrap(),
                row.get::<String>(1).unwrap(),
                row.get::<Option<String>>(2).unwrap(),
            ));
        }
        versions
    }

    async fn chunks_for_path(db: &Arc<libsql::Database>, relative_path: &str) -> Vec<String> {
        let conn = db.connect().unwrap();
        let mut rows = conn
            .query(
                r#"
                SELECT c.content
                FROM memory_chunks c
                JOIN memory_documents d ON d.id = c.document_id
                WHERE d.user_id = ?1 AND d.path = ?2
                ORDER BY c.chunk_index
                "#,
                libsql::params![OWNER_KEY, relative_path],
            )
            .await
            .unwrap();
        let mut chunks = Vec::new();
        while let Some(row) = rows.next().await.unwrap() {
            chunks.push(row.get::<String>(0).unwrap());
        }
        chunks
    }

    async fn count_chunks(db: &Arc<libsql::Database>, relative_path: &str) -> i64 {
        chunks_for_path(db, relative_path).await.len() as i64
    }

    async fn count_versions(db: &Arc<libsql::Database>, relative_path: &str) -> i64 {
        read_versions(db, relative_path).await.len() as i64
    }

    async fn count_documents(db: &Arc<libsql::Database>, relative_path: &str) -> i64 {
        let conn = db.connect().unwrap();
        let mut rows = conn
            .query(
                "SELECT COUNT(*) FROM memory_documents WHERE user_id = ?1 AND path = ?2",
                libsql::params![OWNER_KEY, relative_path],
            )
            .await
            .unwrap();
        let row = rows.next().await.unwrap().unwrap();
        row.get::<i64>(0).unwrap()
    }

    async fn list_user_tables(db: &Arc<libsql::Database>) -> Vec<String> {
        let conn = db.connect().unwrap();
        let mut rows = conn
            .query(
                "SELECT name FROM sqlite_master WHERE type IN ('table', 'view') ORDER BY name",
                (),
            )
            .await
            .unwrap();
        let mut names = Vec::new();
        while let Some(row) = rows.next().await.unwrap() {
            names.push(row.get::<String>(0).unwrap());
        }
        names
    }

    #[derive(Default)]
    struct DeterministicEmbeddingProvider {
        calls: Mutex<usize>,
    }

    #[async_trait]
    impl EmbeddingProvider for DeterministicEmbeddingProvider {
        fn dimension(&self) -> usize {
            3
        }

        fn model_name(&self) -> &str {
            "deterministic-test-embedding"
        }

        async fn embed(&self, _text: &str) -> Result<Vec<f32>, EmbeddingError> {
            *self.calls.lock().unwrap() += 1;
            Ok(vec![1.0, 0.0, 0.0])
        }
    }
}
