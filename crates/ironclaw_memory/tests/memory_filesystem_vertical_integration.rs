#[cfg(feature = "libsql")]
mod libsql_phase5 {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;
    use ironclaw_filesystem::{FileType, FilesystemError, RootFilesystem};
    use ironclaw_host_api::VirtualPath;
    use ironclaw_memory::{
        ChunkConfig, ChunkingMemoryDocumentIndexer, EmbeddingError, EmbeddingProvider,
        LibSqlMemoryDocumentRepository, MemoryBackend, MemoryBackendCapabilities,
        MemoryBackendFilesystemAdapter, MemoryContext, MemoryDocumentPath,
        MemoryDocumentRepository, MemoryDocumentScope, MemorySearchRequest,
        RepositoryMemoryBackend, content_sha256,
    };

    #[tokio::test]
    async fn memory_virtual_filesystem_applies_scope_metadata_indexing_and_versioning() {
        let (db, _dir) = libsql_db().await;
        let repository = Arc::new(LibSqlMemoryDocumentRepository::new(db.clone()));
        repository.run_migrations().await.unwrap();
        let provider = Arc::new(DeterministicEmbeddingProvider::default());
        let indexer = Arc::new(
            ChunkingMemoryDocumentIndexer::new(repository.clone())
                .with_chunk_config(ChunkConfig {
                    chunk_size: 3,
                    overlap_percent: 0.0,
                    min_chunk_size: 1,
                })
                .with_embedding_provider(provider.clone()),
        );
        let backend = Arc::new(
            RepositoryMemoryBackend::new(repository.clone())
                .with_indexer(indexer)
                .with_embedding_provider(provider)
                .with_capabilities(MemoryBackendCapabilities {
                    file_documents: true,
                    metadata: true,
                    versioning: true,
                    full_text_search: true,
                    vector_search: true,
                    embeddings: true,
                    ..MemoryBackendCapabilities::default()
                }),
        );
        let filesystem = MemoryBackendFilesystemAdapter::new(backend);

        let archive_config_path = memory_path("archive/.config");
        filesystem
            .write_file(&archive_config_path, b"")
            .await
            .unwrap();
        let archive_config_document = document_path("archive/.config");
        repository
            .write_document_metadata(
                &archive_config_document,
                &serde_json::json!({"skip_indexing": true}),
            )
            .await
            .unwrap();

        let skipped_path = memory_path("archive/skipped.md");
        filesystem
            .write_file(&skipped_path, b"skip indexing sentinel")
            .await
            .unwrap();
        assert_eq!(
            filesystem.read_file(&skipped_path).await.unwrap(),
            b"skip indexing sentinel"
        );
        assert_eq!(chunk_count_for_path(&db, "archive/skipped.md").await, 0);

        let versioned_path = memory_path("notes/versioned.md");
        filesystem
            .write_file(&versioned_path, b"first searchable-token")
            .await
            .unwrap();
        filesystem
            .write_file(&versioned_path, b"second searchable-token")
            .await
            .unwrap();

        assert_eq!(
            filesystem.read_file(&versioned_path).await.unwrap(),
            b"second searchable-token"
        );
        let stat = filesystem.stat(&versioned_path).await.unwrap();
        assert_eq!(stat.file_type, FileType::File);
        assert_eq!(stat.len, "second searchable-token".len() as u64);
        let entries = filesystem
            .list_dir(&memory_path("notes"))
            .await
            .unwrap()
            .into_iter()
            .map(|entry| entry.name)
            .collect::<Vec<_>>();
        assert_eq!(entries, vec!["versioned.md".to_string()]);

        let conn = db.connect().unwrap();
        let mut document_rows = conn
            .query(
                "SELECT user_id, agent_id, path, content FROM memory_documents WHERE user_id = 'tenant:tenant-a:user:alice:project:project-1' AND agent_id = 'agent-a' AND path = 'notes/versioned.md'",
                (),
            )
            .await
            .unwrap();
        let document = document_rows.next().await.unwrap().expect("document row");
        assert_eq!(
            document.get::<String>(0).unwrap(),
            "tenant:tenant-a:user:alice:project:project-1"
        );
        assert_eq!(
            document.get::<Option<String>>(1).unwrap().as_deref(),
            Some("agent-a")
        );
        assert_eq!(document.get::<String>(2).unwrap(), "notes/versioned.md");
        assert_eq!(
            document.get::<String>(3).unwrap(),
            "second searchable-token"
        );

        let mut version_rows = conn
            .query(
                r#"
                SELECT v.content, v.content_hash, v.changed_by
                FROM memory_document_versions v
                JOIN memory_documents d ON d.id = v.document_id
                WHERE d.user_id = 'tenant:tenant-a:user:alice:project:project-1' AND d.agent_id = 'agent-a' AND d.path = 'notes/versioned.md'
                "#,
                (),
            )
            .await
            .unwrap();
        let version = version_rows.next().await.unwrap().expect("version row");
        assert_eq!(version.get::<String>(0).unwrap(), "first searchable-token");
        assert_eq!(
            version.get::<String>(1).unwrap(),
            content_sha256("first searchable-token")
        );
        assert_eq!(
            version.get::<Option<String>>(2).unwrap().as_deref(),
            Some("tenant:tenant-a:user:alice:project:project-1")
        );

        let indexed_chunks = chunks_for_path(&db, "notes/versioned.md").await;
        assert_eq!(indexed_chunks, vec!["second searchable-token".to_string()]);
    }

    #[tokio::test]
    async fn memory_search_fails_closed_before_side_effects_and_returns_only_scoped_results() {
        let spy_repository = Arc::new(SearchSpyRepository::default());
        let no_search_backend = RepositoryMemoryBackend::new(spy_repository.clone())
            .with_capabilities(MemoryBackendCapabilities {
                file_documents: true,
                full_text_search: false,
                vector_search: false,
                ..MemoryBackendCapabilities::default()
            });
        let context = MemoryContext::new(scope("project-1"));

        let err = no_search_backend
            .search(&context, MemorySearchRequest::new("needle").unwrap())
            .await
            .unwrap_err();

        assert!(
            err.to_string()
                .contains("memory backend does not support search")
        );
        assert_eq!(spy_repository.search_calls(), 0);

        let (db, _dir) = libsql_db().await;
        let repository = Arc::new(LibSqlMemoryDocumentRepository::new(db));
        repository.run_migrations().await.unwrap();
        let provider = Arc::new(DeterministicEmbeddingProvider::default());
        let indexer = Arc::new(
            ChunkingMemoryDocumentIndexer::new(repository.clone())
                .with_embedding_provider(provider.clone()),
        );
        let backend = Arc::new(
            RepositoryMemoryBackend::new(repository)
                .with_indexer(indexer)
                .with_embedding_provider(provider)
                .with_capabilities(MemoryBackendCapabilities {
                    file_documents: true,
                    full_text_search: true,
                    vector_search: true,
                    embeddings: true,
                    ..MemoryBackendCapabilities::default()
                }),
        );
        let filesystem = MemoryBackendFilesystemAdapter::new(backend.clone());

        for (path, content) in [
            (
                "/memory/tenants/tenant-a/users/alice/agents/agent-a/projects/project-1/notes/visible.md",
                "scope-token visible hybrid-vector",
            ),
            (
                "/memory/tenants/tenant-a/users/alice/agents/agent-a/projects/project-2/notes/hidden-project.md",
                "scope-token hidden project hybrid-vector",
            ),
            (
                "/memory/tenants/tenant-a/users/alice/agents/agent-b/projects/project-1/notes/hidden-agent.md",
                "scope-token hidden agent hybrid-vector",
            ),
            (
                "/memory/tenants/tenant-a/users/bob/agents/agent-a/projects/project-1/notes/hidden-user.md",
                "scope-token hidden user hybrid-vector",
            ),
        ] {
            filesystem
                .write_file(&VirtualPath::new(path).unwrap(), content.as_bytes())
                .await
                .unwrap();
        }

        let results = backend
            .search(
                &context,
                MemorySearchRequest::new("scope-token")
                    .unwrap()
                    .with_vector(false)
                    .with_limit(10),
            )
            .await
            .unwrap();

        let result_paths = results
            .iter()
            .map(|result| result.path.relative_path().to_string())
            .collect::<Vec<_>>();
        assert_eq!(result_paths, vec!["notes/visible.md".to_string()]);
        assert!(results[0].snippet.contains("scope-token"));
    }

    fn scope(project_id: &str) -> MemoryDocumentScope {
        MemoryDocumentScope::new_with_agent("tenant-a", "alice", Some("agent-a"), Some(project_id))
            .unwrap()
    }

    fn document_path(relative_path: &str) -> MemoryDocumentPath {
        MemoryDocumentPath::new_with_agent(
            "tenant-a",
            "alice",
            Some("agent-a"),
            Some("project-1"),
            relative_path,
        )
        .unwrap()
    }

    fn memory_path(relative_path: &str) -> VirtualPath {
        VirtualPath::new(format!(
            "/memory/tenants/tenant-a/users/alice/agents/agent-a/projects/project-1/{relative_path}"
        ))
        .unwrap()
    }

    async fn chunks_for_path(db: &Arc<libsql::Database>, relative_path: &str) -> Vec<String> {
        let conn = db.connect().unwrap();
        let mut rows = conn
            .query(
                r#"
                SELECT c.content
                FROM memory_chunks c
                JOIN memory_documents d ON d.id = c.document_id
                WHERE d.user_id = 'tenant:tenant-a:user:alice:project:project-1'
                  AND d.agent_id = 'agent-a'
                  AND d.path = ?1
                ORDER BY c.chunk_index
                "#,
                libsql::params![relative_path],
            )
            .await
            .unwrap();
        let mut chunks = Vec::new();
        while let Some(row) = rows.next().await.unwrap() {
            chunks.push(row.get::<String>(0).unwrap());
        }
        chunks
    }

    async fn chunk_count_for_path(db: &Arc<libsql::Database>, relative_path: &str) -> i64 {
        chunks_for_path(db, relative_path).await.len() as i64
    }

    async fn libsql_db() -> (Arc<libsql::Database>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("memory.db");
        let db = Arc::new(libsql::Builder::new_local(db_path).build().await.unwrap());
        (db, dir)
    }

    #[derive(Default)]
    struct DeterministicEmbeddingProvider {
        calls: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl EmbeddingProvider for DeterministicEmbeddingProvider {
        fn dimension(&self) -> usize {
            3
        }

        fn model_name(&self) -> &str {
            "deterministic-test-embedding"
        }

        async fn embed(&self, text: &str) -> Result<Vec<f32>, EmbeddingError> {
            self.calls.lock().unwrap().push(text.to_string());
            if text.contains("hybrid-vector") || text.contains("scope-token") {
                Ok(vec![1.0, 0.0, 0.0])
            } else if text.contains("unrelated") {
                Ok(vec![0.0, 1.0, 0.0])
            } else {
                Ok(vec![0.0, 0.0, 1.0])
            }
        }
    }

    #[derive(Default)]
    struct SearchSpyRepository {
        search_calls: Mutex<usize>,
    }

    impl SearchSpyRepository {
        fn search_calls(&self) -> usize {
            *self.search_calls.lock().unwrap()
        }
    }

    #[async_trait]
    impl MemoryDocumentRepository for SearchSpyRepository {
        async fn read_document(
            &self,
            _path: &MemoryDocumentPath,
        ) -> Result<Option<Vec<u8>>, FilesystemError> {
            Ok(None)
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
        ) -> Result<Vec<ironclaw_memory::MemorySearchResult>, FilesystemError> {
            *self.search_calls.lock().unwrap() += 1;
            Ok(Vec::new())
        }
    }
}
