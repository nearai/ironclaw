#![cfg(any(feature = "libsql", feature = "postgres"))]

use async_trait::async_trait;
use ironclaw_filesystem::{FilesystemError, RootFilesystem};
use ironclaw_host_api::VirtualPath;
use ironclaw_memory::{
    ChunkConfig, ChunkingMemoryDocumentIndexer, DocumentMetadata, EmbeddingError,
    EmbeddingProvider, FusionStrategy, MemoryBackend, MemoryBackendCapabilities,
    MemoryBackendFilesystemAdapter, MemoryContext, MemoryDocumentFilesystem, MemoryDocumentPath,
    MemoryDocumentRepository, MemoryDocumentScope, MemorySearchRequest, RepositoryMemoryBackend,
};

#[cfg(feature = "libsql")]
use ironclaw_memory::LibSqlMemoryDocumentRepository;
#[cfg(feature = "postgres")]
use ironclaw_memory::PostgresMemoryDocumentRepository;

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_memory_repository_persists_documents_across_instances() {
    let (db, _dir) = libsql_db().await;
    let repository = LibSqlMemoryDocumentRepository::new(db.clone());
    repository.run_migrations().await.unwrap();

    let path = MemoryDocumentPath::new("tenant-a", "alice", None, "MEMORY.md").unwrap();
    repository
        .write_document(&path, b"remember this")
        .await
        .unwrap();

    let reopened = LibSqlMemoryDocumentRepository::new(db);
    assert_eq!(
        reopened.read_document(&path).await.unwrap().unwrap(),
        b"remember this"
    );
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_memory_repository_maps_agent_scope_to_agent_id_column() {
    let (db, _dir) = libsql_db().await;
    let repository = LibSqlMemoryDocumentRepository::new(db.clone());
    repository.run_migrations().await.unwrap();

    let path = MemoryDocumentPath::new_with_agent(
        "tenant-a",
        "alice",
        Some("agent-1"),
        Some("project-1"),
        "notes/a.md",
    )
    .unwrap();
    repository
        .write_document(&path, b"agent db backed note")
        .await
        .unwrap();

    let conn = db.connect().unwrap();
    let mut rows = conn
        .query(
            "SELECT user_id, agent_id, path, content FROM memory_documents",
            (),
        )
        .await
        .unwrap();
    let row = rows.next().await.unwrap().expect("memory document row");
    let user_id: String = row.get(0).unwrap();
    let agent_id: Option<String> = row.get(1).unwrap();
    let db_path: String = row.get(2).unwrap();
    let content: String = row.get(3).unwrap();

    assert_eq!(user_id, "tenant:tenant-a:user:alice:project:project-1");
    assert_eq!(agent_id.as_deref(), Some("agent-1"));
    assert_eq!(db_path, "notes/a.md");
    assert_eq!(content, "agent db backed note");
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_memory_repository_isolates_tenant_user_agent_and_project_scopes() {
    let (db, _dir) = libsql_db().await;
    let repository = LibSqlMemoryDocumentRepository::new(db);
    repository.run_migrations().await.unwrap();

    for (path, content) in [
        (
            MemoryDocumentPath::new_with_agent(
                "tenant-a",
                "alice",
                Some("agent-1"),
                Some("project-1"),
                "MEMORY.md",
            )
            .unwrap(),
            b"tenant-a alice agent-1 project-1".as_slice(),
        ),
        (
            MemoryDocumentPath::new_with_agent(
                "tenant-a",
                "alice",
                Some("agent-2"),
                Some("project-1"),
                "MEMORY.md",
            )
            .unwrap(),
            b"tenant-a alice agent-2 project-1".as_slice(),
        ),
        (
            MemoryDocumentPath::new_with_agent(
                "tenant-a",
                "alice",
                None,
                Some("project-1"),
                "MEMORY.md",
            )
            .unwrap(),
            b"tenant-a alice no-agent project-1".as_slice(),
        ),
    ] {
        repository.write_document(&path, content).await.unwrap();
    }

    let visible = repository
        .list_documents(
            &MemoryDocumentScope::new_with_agent(
                "tenant-a",
                "alice",
                Some("agent-1"),
                Some("project-1"),
            )
            .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].tenant_id(), "tenant-a");
    assert_eq!(visible[0].user_id(), "alice");
    assert_eq!(visible[0].agent_id(), Some("agent-1"));
    assert_eq!(visible[0].project_id(), Some("project-1"));
    assert_eq!(visible[0].relative_path(), "MEMORY.md");
    assert_eq!(
        repository
            .read_document(&visible[0])
            .await
            .unwrap()
            .unwrap(),
        b"tenant-a alice agent-1 project-1"
    );
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_memory_repository_isolates_tenant_user_and_project_scopes() {
    let (db, _dir) = libsql_db().await;
    let repository = LibSqlMemoryDocumentRepository::new(db);
    repository.run_migrations().await.unwrap();

    for (path, content) in [
        (
            MemoryDocumentPath::new("tenant-a", "alice", Some("project-1"), "MEMORY.md").unwrap(),
            b"tenant-a alice project-1".as_slice(),
        ),
        (
            MemoryDocumentPath::new("tenant-a", "bob", Some("project-1"), "MEMORY.md").unwrap(),
            b"tenant-a bob project-1".as_slice(),
        ),
        (
            MemoryDocumentPath::new("tenant-b", "alice", Some("project-1"), "MEMORY.md").unwrap(),
            b"tenant-b alice project-1".as_slice(),
        ),
        (
            MemoryDocumentPath::new("tenant-a", "alice", Some("project-2"), "MEMORY.md").unwrap(),
            b"tenant-a alice project-2".as_slice(),
        ),
    ] {
        repository.write_document(&path, content).await.unwrap();
    }

    let visible = repository
        .list_documents(&MemoryDocumentScope::new("tenant-a", "alice", Some("project-1")).unwrap())
        .await
        .unwrap();

    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].tenant_id(), "tenant-a");
    assert_eq!(visible[0].user_id(), "alice");
    assert_eq!(visible[0].project_id(), Some("project-1"));
    assert_eq!(visible[0].relative_path(), "MEMORY.md");
    assert_eq!(
        repository
            .read_document(&visible[0])
            .await
            .unwrap()
            .unwrap(),
        b"tenant-a alice project-1"
    );
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_memory_repository_lists_none_project_documents_under_top_level_projects_directory()
{
    let (db, _dir) = libsql_db().await;
    let repository = LibSqlMemoryDocumentRepository::new(db);
    repository.run_migrations().await.unwrap();

    let path =
        MemoryDocumentPath::new("tenant-a", "alice", None, "projects/local-note.md").unwrap();
    repository
        .write_document(&path, b"top-level projects note")
        .await
        .unwrap();

    let visible = repository
        .list_documents(&MemoryDocumentScope::new("tenant-a", "alice", None).unwrap())
        .await
        .unwrap();

    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].project_id(), None);
    assert_eq!(visible[0].relative_path(), "projects/local-note.md");
    assert_eq!(
        repository
            .read_document(&visible[0])
            .await
            .unwrap()
            .unwrap(),
        b"top-level projects note"
    );
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_memory_repository_rejects_file_directory_prefix_conflicts() {
    let (db, _dir) = libsql_db().await;
    let repository = LibSqlMemoryDocumentRepository::new(db);
    repository.run_migrations().await.unwrap();

    let file = MemoryDocumentPath::new("tenant-a", "alice", None, "notes").unwrap();
    let child = MemoryDocumentPath::new("tenant-a", "alice", None, "notes/a.md").unwrap();

    repository
        .write_document(&file, b"plain file")
        .await
        .unwrap();
    let err = repository
        .write_document(&child, b"child")
        .await
        .unwrap_err();
    assert!(err.to_string().contains("existing file ancestor"));

    let (db, _dir) = libsql_db().await;
    let repository = LibSqlMemoryDocumentRepository::new(db);
    repository.run_migrations().await.unwrap();
    repository.write_document(&child, b"child").await.unwrap();
    let err = repository
        .write_document(&file, b"plain file")
        .await
        .unwrap_err();
    assert!(err.to_string().contains("existing directory"));
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_memory_repository_upserts_duplicate_document_paths() {
    let (db, _dir) = libsql_db().await;
    let repository = LibSqlMemoryDocumentRepository::new(db.clone());
    repository.run_migrations().await.unwrap();

    let path = MemoryDocumentPath::new("tenant-a", "alice", None, "notes/a.md").unwrap();
    repository.write_document(&path, b"first").await.unwrap();
    repository.write_document(&path, b"second").await.unwrap();

    let conn = db.connect().unwrap();
    let mut rows = conn
        .query("SELECT path, content FROM memory_documents", ())
        .await
        .unwrap();
    let row = rows
        .next()
        .await
        .unwrap()
        .expect("single memory document row");
    assert_eq!(row.get::<String>(0).unwrap(), "notes/a.md");
    assert_eq!(row.get::<String>(1).unwrap(), "second");
    assert!(rows.next().await.unwrap().is_none());
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_memory_document_filesystem_reads_and_writes_through_db_repository() {
    let (db, _dir) = libsql_db().await;
    let repository = std::sync::Arc::new(LibSqlMemoryDocumentRepository::new(db));
    repository.run_migrations().await.unwrap();
    let filesystem = MemoryDocumentFilesystem::new(repository);
    let path = VirtualPath::new(
        "/memory/tenants/tenant-a/users/alice/agents/_none/projects/project-1/notes/a.md",
    )
    .unwrap();

    filesystem
        .write_file(&path, b"filesystem db note")
        .await
        .unwrap();

    assert_eq!(
        filesystem.read_file(&path).await.unwrap(),
        b"filesystem db note"
    );
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_memory_document_filesystem_reuses_current_chunking_and_fts_schema() {
    let (db, _dir) = libsql_db().await;
    let repository = std::sync::Arc::new(LibSqlMemoryDocumentRepository::new(db.clone()));
    repository.run_migrations().await.unwrap();
    let indexer = std::sync::Arc::new(
        ChunkingMemoryDocumentIndexer::new(repository.clone()).with_chunk_config(ChunkConfig {
            chunk_size: 4,
            overlap_percent: 0.0,
            min_chunk_size: 1,
        }),
    );
    let backend =
        std::sync::Arc::new(RepositoryMemoryBackend::new(repository).with_indexer(indexer));
    let filesystem = MemoryBackendFilesystemAdapter::new(backend);
    let path = VirtualPath::new(
        "/memory/tenants/tenant-a/users/alice/agents/_none/projects/_none/notes.md",
    )
    .unwrap();

    filesystem
        .write_file(&path, b"alpha beta gamma delta epsilon zeta")
        .await
        .unwrap();

    let conn = db.connect().unwrap();
    let mut chunk_rows = conn
        .query(
            "SELECT c.chunk_index, c.content FROM memory_chunks c JOIN memory_documents d ON d.id = c.document_id ORDER BY c.chunk_index",
            (),
        )
        .await
        .unwrap();
    let mut chunks = Vec::new();
    while let Some(row) = chunk_rows.next().await.unwrap() {
        chunks.push((row.get::<i64>(0).unwrap(), row.get::<String>(1).unwrap()));
    }
    assert_eq!(
        chunks,
        vec![
            (0, "alpha beta gamma delta".to_string()),
            (1, "epsilon zeta".to_string()),
        ]
    );

    let mut fts_rows = conn
        .query(
            r#"
            SELECT c.content
            FROM memory_chunks_fts fts
            JOIN memory_chunks c ON c._rowid = fts.rowid
            WHERE memory_chunks_fts MATCH 'epsilon'
            "#,
            (),
        )
        .await
        .unwrap();
    let row = fts_rows.next().await.unwrap().expect("fts result");
    assert_eq!(row.get::<String>(0).unwrap(), "epsilon zeta");
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_memory_document_filesystem_versions_previous_content_and_replaces_chunks() {
    let (db, _dir) = libsql_db().await;
    let repository = std::sync::Arc::new(LibSqlMemoryDocumentRepository::new(db.clone()));
    repository.run_migrations().await.unwrap();
    let indexer = std::sync::Arc::new(ChunkingMemoryDocumentIndexer::new(repository.clone()));
    let backend =
        std::sync::Arc::new(RepositoryMemoryBackend::new(repository).with_indexer(indexer));
    let filesystem = MemoryBackendFilesystemAdapter::new(backend);
    let path = VirtualPath::new(
        "/memory/tenants/tenant-a/users/alice/agents/_none/projects/_none/notes.md",
    )
    .unwrap();

    filesystem
        .write_file(&path, b"first content")
        .await
        .unwrap();
    filesystem
        .write_file(&path, b"second content")
        .await
        .unwrap();

    let conn = db.connect().unwrap();
    let mut version_rows = conn
        .query(
            "SELECT content, content_hash, changed_by FROM memory_document_versions",
            (),
        )
        .await
        .unwrap();
    let version = version_rows.next().await.unwrap().expect("version row");
    assert_eq!(version.get::<String>(0).unwrap(), "first content");
    assert_eq!(
        version.get::<String>(1).unwrap(),
        "sha256:2cd4837c7726f70047c8fdafb52801dbfef2cb4f7bc4cfb2e0441980f9d4a3b8"
    );
    assert_eq!(
        version.get::<Option<String>>(2).unwrap().as_deref(),
        Some("tenant:tenant-a:user:alice:project:_none")
    );

    let mut chunk_rows = conn
        .query("SELECT content FROM memory_chunks ORDER BY chunk_index", ())
        .await
        .unwrap();
    let only_chunk = chunk_rows.next().await.unwrap().expect("chunk row");
    assert_eq!(only_chunk.get::<String>(0).unwrap(), "second content");
    assert!(chunk_rows.next().await.unwrap().is_none());
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_repository_backend_inherits_config_metadata_for_skip_indexing() {
    let (db, _dir) = libsql_db().await;
    let repository = std::sync::Arc::new(LibSqlMemoryDocumentRepository::new(db.clone()));
    repository.run_migrations().await.unwrap();
    let config_path = MemoryDocumentPath::new("tenant-a", "alice", None, "folder/.config").unwrap();
    repository.write_document(&config_path, b"").await.unwrap();
    repository
        .write_document_metadata(&config_path, &serde_json::json!({"skip_indexing": true}))
        .await
        .unwrap();
    let indexer = std::sync::Arc::new(ChunkingMemoryDocumentIndexer::new(repository.clone()));
    let backend =
        std::sync::Arc::new(RepositoryMemoryBackend::new(repository).with_indexer(indexer));
    let filesystem = MemoryBackendFilesystemAdapter::new(backend);
    let path = VirtualPath::new(
        "/memory/tenants/tenant-a/users/alice/agents/_none/projects/_none/folder/a.md",
    )
    .unwrap();

    filesystem
        .write_file(&path, b"alpha beta gamma")
        .await
        .unwrap();

    let conn = db.connect().unwrap();
    let mut rows = conn
        .query("SELECT COUNT(*) FROM memory_chunks", ())
        .await
        .unwrap();
    let row = rows.next().await.unwrap().unwrap();
    assert_eq!(row.get::<i64>(0).unwrap(), 0);
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_repository_backend_honors_skip_versioning_from_config() {
    let (db, _dir) = libsql_db().await;
    let repository = std::sync::Arc::new(LibSqlMemoryDocumentRepository::new(db.clone()));
    repository.run_migrations().await.unwrap();
    let config_path = MemoryDocumentPath::new("tenant-a", "alice", None, "logs/.config").unwrap();
    repository.write_document(&config_path, b"").await.unwrap();
    repository
        .write_document_metadata(&config_path, &serde_json::json!({"skip_versioning": true}))
        .await
        .unwrap();
    let backend = std::sync::Arc::new(RepositoryMemoryBackend::new(repository));
    let filesystem = MemoryBackendFilesystemAdapter::new(backend);
    let path = VirtualPath::new(
        "/memory/tenants/tenant-a/users/alice/agents/_none/projects/_none/logs/a.md",
    )
    .unwrap();

    filesystem.write_file(&path, b"first").await.unwrap();
    filesystem.write_file(&path, b"second").await.unwrap();

    let conn = db.connect().unwrap();
    let mut rows = conn
        .query("SELECT COUNT(*) FROM memory_document_versions", ())
        .await
        .unwrap();
    let row = rows.next().await.unwrap().unwrap();
    assert_eq!(row.get::<i64>(0).unwrap(), 0);
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_repository_backend_validates_schema_from_config_before_write() {
    let (db, _dir) = libsql_db().await;
    let repository = std::sync::Arc::new(LibSqlMemoryDocumentRepository::new(db.clone()));
    repository.run_migrations().await.unwrap();
    let config_path =
        MemoryDocumentPath::new("tenant-a", "alice", None, "settings/.config").unwrap();
    repository.write_document(&config_path, b"").await.unwrap();
    repository
        .write_document_metadata(
            &config_path,
            &serde_json::json!({
                "schema": {
                    "type": "object",
                    "properties": {"provider": {"type": "string"}},
                    "required": ["provider"]
                }
            }),
        )
        .await
        .unwrap();
    let backend = std::sync::Arc::new(RepositoryMemoryBackend::new(repository));
    let filesystem = MemoryBackendFilesystemAdapter::new(backend);
    let path = VirtualPath::new(
        "/memory/tenants/tenant-a/users/alice/agents/_none/projects/_none/settings/llm.json",
    )
    .unwrap();

    let err = filesystem
        .write_file(&path, br#"{"missing":"provider"}"#)
        .await
        .unwrap_err();

    assert!(err.to_string().contains("schema validation failed"));
    let conn = db.connect().unwrap();
    let mut rows = conn
        .query(
            "SELECT COUNT(*) FROM memory_documents WHERE path = 'settings/llm.json'",
            (),
        )
        .await
        .unwrap();
    let row = rows.next().await.unwrap().unwrap();
    assert_eq!(row.get::<i64>(0).unwrap(), 0);
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_chunking_indexer_stores_provider_embeddings_with_chunks() {
    let (db, _dir) = libsql_db().await;
    let repository = std::sync::Arc::new(LibSqlMemoryDocumentRepository::new(db.clone()));
    repository.run_migrations().await.unwrap();
    let provider = std::sync::Arc::new(DeterministicEmbeddingProvider::default());
    let indexer = std::sync::Arc::new(
        ChunkingMemoryDocumentIndexer::new(repository.clone())
            .with_chunk_config(ChunkConfig {
                chunk_size: 2,
                overlap_percent: 0.0,
                min_chunk_size: 1,
            })
            .with_embedding_provider(provider.clone()),
    );
    let backend =
        std::sync::Arc::new(RepositoryMemoryBackend::new(repository).with_indexer(indexer));
    let filesystem = MemoryBackendFilesystemAdapter::new(backend);
    let path = VirtualPath::new(
        "/memory/tenants/tenant-a/users/alice/agents/_none/projects/_none/embeddings.md",
    )
    .unwrap();

    filesystem
        .write_file(&path, b"hybrid-vector words unrelated words")
        .await
        .unwrap();

    let conn = db.connect().unwrap();
    let mut rows = conn
        .query(
            "SELECT content, embedding FROM memory_chunks ORDER BY chunk_index",
            (),
        )
        .await
        .unwrap();
    let mut stored = Vec::new();
    while let Some(row) = rows.next().await.unwrap() {
        let content = row.get::<String>(0).unwrap();
        let embedding = decode_test_embedding(&row.get::<Vec<u8>>(1).unwrap());
        stored.push((content, embedding));
    }

    assert_eq!(
        stored,
        vec![
            ("hybrid-vector words".to_string(), vec![1.0, 0.0, 0.0]),
            ("unrelated words".to_string(), vec![0.0, 1.0, 0.0]),
        ]
    );
    assert_eq!(
        provider.calls(),
        vec![
            "hybrid-vector words".to_string(),
            "unrelated words".to_string(),
        ]
    );
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_repository_backend_hybrid_search_fuses_full_text_and_vector_results() {
    let (db, _dir) = libsql_db().await;
    let repository = std::sync::Arc::new(LibSqlMemoryDocumentRepository::new(db));
    repository.run_migrations().await.unwrap();
    let provider = std::sync::Arc::new(DeterministicEmbeddingProvider::default());
    let indexer = std::sync::Arc::new(
        ChunkingMemoryDocumentIndexer::new(repository.clone())
            .with_embedding_provider(provider.clone()),
    );
    let backend = RepositoryMemoryBackend::new(repository)
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
        let path = MemoryDocumentPath::new("tenant-a", "alice", None, relative_path).unwrap();
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

    let paths = results
        .iter()
        .map(|result| result.path.relative_path().to_string())
        .collect::<Vec<_>>();
    assert_eq!(paths[0], "notes/hybrid.md");
    assert!(results[0].is_hybrid());
    assert!(paths.contains(&"notes/fts-only.md".to_string()));
    assert!(paths.contains(&"notes/vector-only.md".to_string()));
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_repository_backend_fails_closed_for_unsupported_vector_search() {
    let (db, _dir) = libsql_db().await;
    let repository = std::sync::Arc::new(LibSqlMemoryDocumentRepository::new(db));
    repository.run_migrations().await.unwrap();
    let backend =
        RepositoryMemoryBackend::new(repository).with_capabilities(MemoryBackendCapabilities {
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
async fn libsql_repository_backend_searches_indexed_chunks() {
    let (db, _dir) = libsql_db().await;
    let repository = std::sync::Arc::new(LibSqlMemoryDocumentRepository::new(db));
    repository.run_migrations().await.unwrap();
    let indexer = std::sync::Arc::new(ChunkingMemoryDocumentIndexer::new(repository.clone()));
    let backend = RepositoryMemoryBackend::new(repository)
        .with_indexer(indexer)
        .with_capabilities(MemoryBackendCapabilities {
            file_documents: true,
            full_text_search: true,
            ..MemoryBackendCapabilities::default()
        });
    let context = MemoryContext::new(MemoryDocumentScope::new("tenant-a", "alice", None).unwrap());
    let path = MemoryDocumentPath::new("tenant-a", "alice", None, "notes/search.md").unwrap();

    backend
        .write_document(&context, &path, b"alpha beta searchable-token")
        .await
        .unwrap();

    let results = backend
        .search(
            &context,
            MemorySearchRequest::new("searchable-token")
                .unwrap()
                .with_limit(3),
        )
        .await
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].path.relative_path(), "notes/search.md");
    assert!(results[0].snippet.contains("searchable-token"));
}

#[test]
fn document_metadata_merges_like_current_workspace_metadata() {
    let base = serde_json::json!({"skip_indexing": true, "schema": {"type": "object"}});
    let overlay = serde_json::json!({"skip_indexing": false, "skip_versioning": true});
    let merged = DocumentMetadata::from_value(&DocumentMetadata::merge(&base, &overlay));

    assert_eq!(merged.skip_indexing, Some(false));
    assert_eq!(merged.skip_versioning, Some(true));
    assert_eq!(merged.schema, Some(serde_json::json!({"type": "object"})));
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_memory_repository_stores_text_in_existing_memory_documents_shape() {
    let (db, _dir) = libsql_db().await;
    let repository = LibSqlMemoryDocumentRepository::new(db.clone());
    repository.run_migrations().await.unwrap();

    let path =
        MemoryDocumentPath::new("tenant-a", "alice", Some("project-1"), "notes/a.md").unwrap();
    repository
        .write_document(&path, b"db backed note")
        .await
        .unwrap();

    let conn = db.connect().unwrap();
    let mut rows = conn
        .query(
            "SELECT user_id, agent_id, path, content FROM memory_documents",
            (),
        )
        .await
        .unwrap();
    let row = rows.next().await.unwrap().expect("memory document row");
    let user_id: String = row.get(0).unwrap();
    let agent_id: Option<String> = row.get(1).unwrap();
    let db_path: String = row.get(2).unwrap();
    let content: String = row.get(3).unwrap();

    assert_eq!(user_id, "tenant:tenant-a:user:alice:project:project-1");
    assert_eq!(agent_id, None);
    assert_eq!(db_path, "notes/a.md");
    assert_eq!(content, "db backed note");
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn libsql_memory_repository_rejects_non_utf8_documents() {
    let (db, _dir) = libsql_db().await;
    let repository = LibSqlMemoryDocumentRepository::new(db);
    repository.run_migrations().await.unwrap();

    let path = MemoryDocumentPath::new("tenant-a", "alice", None, "binary.bin").unwrap();
    let err = repository
        .write_document(&path, &[0xff, 0xfe, 0xfd])
        .await
        .unwrap_err();

    assert!(matches!(err, FilesystemError::Backend { .. }));
    assert!(
        err.to_string()
            .contains("memory document content must be UTF-8")
    );
}

#[cfg(feature = "postgres")]
#[test]
fn postgres_memory_repository_implements_memory_repository_contract() {
    fn assert_repository<T: MemoryDocumentRepository>() {}
    assert_repository::<PostgresMemoryDocumentRepository>();
}

#[cfg(feature = "libsql")]
async fn libsql_db() -> (std::sync::Arc<libsql::Database>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("memory.db");
    let db = std::sync::Arc::new(libsql::Builder::new_local(db_path).build().await.unwrap());
    (db, dir)
}

#[derive(Default)]
struct DeterministicEmbeddingProvider {
    calls: std::sync::Mutex<Vec<String>>,
}

impl DeterministicEmbeddingProvider {
    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
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
        if text == "literal" || text.contains("hybrid-vector") || text.contains("semantic-only") {
            Ok(vec![1.0, 0.0, 0.0])
        } else if text.contains("unrelated") {
            Ok(vec![0.0, 1.0, 0.0])
        } else {
            Ok(vec![0.0, 0.0, 1.0])
        }
    }
}

fn decode_test_embedding(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect()
}

#[allow(dead_code)]
struct _TraitObjectCheck;

#[async_trait]
impl MemoryDocumentRepository for _TraitObjectCheck {
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
}
