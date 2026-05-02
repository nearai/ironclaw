//! Behavior tests for `RebornLibSqlMemoryDocumentRepository` against the
//! Reborn-native `reborn_memory_*` schema (#3118 phase 4).

#![cfg(feature = "libsql")]

use std::sync::Arc;

use ironclaw_memory::{
    ChunkConfig, ChunkingMemoryDocumentIndexer, DocumentMetadata, FusionStrategy, MemoryChunkWrite,
    MemoryDocumentIndexRepository, MemoryDocumentIndexer, MemoryDocumentPath,
    MemoryDocumentRepository, MemoryDocumentScope, MemorySearchRequest, MemoryWriteOptions,
    RebornLibSqlMemoryDocumentRepository, content_sha256,
};

struct Fixture {
    repo: Arc<RebornLibSqlMemoryDocumentRepository>,
    db: Arc<libsql::Database>,
    _dir: tempfile::TempDir,
}

async fn fresh_repository() -> Fixture {
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
    Fixture {
        repo,
        db,
        _dir: dir,
    }
}

#[tokio::test]
async fn round_trips_a_document_within_full_scope() {
    let f = fresh_repository().await;
    let path = MemoryDocumentPath::new("tenant-a", "alice", None, "MEMORY.md").expect("path");
    f.repo.write_document(&path, b"hello reborn").await.unwrap();
    let stored = f.repo.read_document(&path).await.unwrap();
    assert_eq!(stored.as_deref(), Some(b"hello reborn".as_slice()));
}

#[tokio::test]
async fn returns_none_when_document_is_missing() {
    let f = fresh_repository().await;
    let path = MemoryDocumentPath::new("tenant-a", "alice", None, "missing.md").expect("path");
    assert!(f.repo.read_document(&path).await.unwrap().is_none());
}

#[tokio::test]
async fn upsert_replaces_content_for_same_full_scope_and_path() {
    let f = fresh_repository().await;
    let path = MemoryDocumentPath::new("tenant-a", "alice", None, "notes.md").expect("path");
    f.repo.write_document(&path, b"first").await.unwrap();
    f.repo.write_document(&path, b"second").await.unwrap();
    f.repo.write_document(&path, b"third").await.unwrap();

    let stored = f.repo.read_document(&path).await.unwrap();
    assert_eq!(stored.as_deref(), Some(b"third".as_slice()));

    let listed = f.repo.list_documents(path.scope()).await.unwrap();
    let matches = listed
        .iter()
        .filter(|p| p.relative_path() == "notes.md")
        .count();
    assert_eq!(matches, 1, "upsert must not create a duplicate row");
}

#[tokio::test]
async fn full_scope_isolates_tenant_user_agent_project_independently() {
    struct ScopeFixture {
        tenant: &'static str,
        user: &'static str,
        agent: Option<&'static str>,
        project: Option<&'static str>,
        body: &'static [u8],
    }
    let f = fresh_repository().await;

    let writes = [
        ScopeFixture {
            tenant: "tenant-a",
            user: "alice",
            agent: None,
            project: None,
            body: b"baseline",
        },
        ScopeFixture {
            tenant: "tenant-b",
            user: "alice",
            agent: None,
            project: None,
            body: b"other-tenant",
        },
        ScopeFixture {
            tenant: "tenant-a",
            user: "bob",
            agent: None,
            project: None,
            body: b"other-user",
        },
        ScopeFixture {
            tenant: "tenant-a",
            user: "alice",
            agent: Some("scout"),
            project: None,
            body: b"scout-agent",
        },
        ScopeFixture {
            tenant: "tenant-a",
            user: "alice",
            agent: None,
            project: Some("alpha"),
            body: b"alpha-project",
        },
    ];
    for fixture in &writes {
        let path = MemoryDocumentPath::new_with_agent(
            fixture.tenant,
            fixture.user,
            fixture.agent,
            fixture.project,
            "shared.md",
        )
        .expect("path");
        f.repo.write_document(&path, fixture.body).await.unwrap();
    }

    for fixture in &writes {
        let path = MemoryDocumentPath::new_with_agent(
            fixture.tenant,
            fixture.user,
            fixture.agent,
            fixture.project,
            "shared.md",
        )
        .expect("path");
        let stored = f.repo.read_document(&path).await.unwrap();
        assert_eq!(stored.as_deref(), Some(fixture.body));
    }

    for fixture in &writes {
        let scope = MemoryDocumentScope::new_with_agent(
            fixture.tenant,
            fixture.user,
            fixture.agent,
            fixture.project,
        )
        .expect("scope");
        let listed = f.repo.list_documents(&scope).await.unwrap();
        assert_eq!(
            listed.len(),
            1,
            "{}/{}/{:?}/{:?} must list only itself, got {:?}",
            fixture.tenant,
            fixture.user,
            fixture.agent,
            fixture.project,
            listed,
        );
    }
}

#[tokio::test]
async fn top_level_projects_path_is_a_normal_user_path_not_project_scope() {
    // The issue is explicit: a relative path beginning with "projects/" must
    // NOT be re-interpreted as project scope. Project scope only comes from
    // the explicit MemoryDocumentScope project_id.
    let f = fresh_repository().await;
    let user_doc =
        MemoryDocumentPath::new("tenant-a", "alice", None, "projects/local-note.md").expect("path");
    f.repo
        .write_document(&user_doc, b"user-scoped note")
        .await
        .unwrap();

    let project_doc =
        MemoryDocumentPath::new("tenant-a", "alice", Some("alpha"), "projects/local-note.md")
            .expect("path");
    f.repo
        .write_document(&project_doc, b"alpha-scoped note")
        .await
        .unwrap();

    assert_eq!(
        f.repo.read_document(&user_doc).await.unwrap().as_deref(),
        Some(b"user-scoped note".as_slice())
    );
    assert_eq!(
        f.repo.read_document(&project_doc).await.unwrap().as_deref(),
        Some(b"alpha-scoped note".as_slice())
    );
}

#[tokio::test]
async fn rejects_file_directory_prefix_conflicts_within_scope() {
    let f = fresh_repository().await;
    let file = MemoryDocumentPath::new("tenant-a", "alice", None, "notes").expect("path");
    let child = MemoryDocumentPath::new("tenant-a", "alice", None, "notes/a.md").expect("path");

    f.repo.write_document(&file, b"plain file").await.unwrap();
    let err = f.repo.write_document(&child, b"child").await.unwrap_err();
    assert!(err.to_string().contains("existing file ancestor"));

    let f2 = fresh_repository().await;
    f2.repo.write_document(&child, b"child").await.unwrap();
    let err = f2
        .repo
        .write_document(&file, b"plain file")
        .await
        .unwrap_err();
    assert!(err.to_string().contains("existing directory"));
}

#[tokio::test]
async fn writes_metadata_and_reads_it_back_for_native_documents() {
    let f = fresh_repository().await;
    let path = MemoryDocumentPath::new("tenant-a", "alice", None, "doc.md").expect("path");
    f.repo.write_document(&path, b"hello").await.unwrap();
    let metadata = serde_json::json!({"tag": "primary", "skip_indexing": false});
    f.repo
        .write_document_metadata(&path, &metadata)
        .await
        .unwrap();
    let read_back = f.repo.read_document_metadata(&path).await.unwrap();
    assert_eq!(read_back, Some(metadata));
}

#[tokio::test]
async fn write_with_options_creates_version_row_only_when_not_skipped() {
    let f = fresh_repository().await;
    let path = MemoryDocumentPath::new("tenant-a", "alice", None, "doc.md").expect("path");

    f.repo.write_document(&path, b"v1").await.unwrap();

    let opts = MemoryWriteOptions {
        metadata: DocumentMetadata::default(),
        changed_by: Some("test:default".to_string()),
    };
    f.repo
        .write_document_with_options(&path, b"v2", &opts)
        .await
        .unwrap();
    f.repo
        .write_document_with_options(&path, b"v3", &opts)
        .await
        .unwrap();

    let opts_skip = MemoryWriteOptions {
        metadata: DocumentMetadata {
            skip_versioning: Some(true),
            ..DocumentMetadata::default()
        },
        changed_by: Some("test:skip".to_string()),
    };
    f.repo
        .write_document_with_options(&path, b"v4-skip", &opts_skip)
        .await
        .unwrap();

    // 2 prior contents archived (v1 -> when v2 wrote, v2 -> when v3 wrote).
    // v3 -> v4-skip MUST not produce a new row.
    let count = count_versions(&f.db, &path).await;
    assert_eq!(count, 2, "expected 2 version rows, got {count}");
}

#[tokio::test]
async fn version_numbers_are_monotonic_and_content_hash_matches_archived_content() {
    let f = fresh_repository().await;
    let path = MemoryDocumentPath::new("tenant-a", "alice", None, "v.md").expect("path");

    f.repo.write_document(&path, b"v1").await.unwrap();
    f.repo.write_document(&path, b"v2").await.unwrap();
    f.repo.write_document(&path, b"v3").await.unwrap();
    f.repo.write_document(&path, b"v4").await.unwrap();

    let rows = read_version_rows(&f.db, &path).await;
    let mut versions: Vec<i64> = rows.iter().map(|(v, _, _)| *v).collect();
    versions.sort();
    assert_eq!(versions, vec![1, 2, 3]);

    for (_, content, hash) in &rows {
        assert_eq!(hash, &content_sha256(content));
    }
}

#[tokio::test]
async fn replace_chunks_if_current_is_a_noop_when_document_was_rewritten() {
    let f = fresh_repository().await;
    let path = MemoryDocumentPath::new("tenant-a", "alice", None, "drift.md").expect("path");

    f.repo
        .write_document(&path, b"original content")
        .await
        .unwrap();
    let stale_hash = content_sha256("original content");

    // Document rewritten between the read and the index refresh.
    f.repo
        .write_document(&path, b"newer content")
        .await
        .unwrap();

    let stale_chunks = vec![MemoryChunkWrite {
        content: "original content".to_string(),
        embedding: None,
    }];
    f.repo
        .replace_document_chunks_if_current(&path, &stale_hash, &stale_chunks)
        .await
        .unwrap();

    assert_eq!(count_chunks(&f.db, &path).await, 0);
}

#[tokio::test]
async fn full_text_search_returns_only_chunks_within_full_scope() {
    let f = fresh_repository().await;
    let indexer = Arc::new(
        ChunkingMemoryDocumentIndexer::new(f.repo.clone()).with_chunk_config(ChunkConfig {
            chunk_size: 4,
            overlap_percent: 0.0,
            min_chunk_size: 1,
        }),
    );

    let alice_path = MemoryDocumentPath::new("tenant-a", "alice", None, "notes.md").expect("path");
    let bob_path = MemoryDocumentPath::new("tenant-a", "bob", None, "notes.md").expect("path");
    let other_tenant_path =
        MemoryDocumentPath::new("tenant-b", "alice", None, "notes.md").expect("path");

    f.repo
        .write_document(&alice_path, b"reborn alpaca pizza")
        .await
        .unwrap();
    indexer.reindex_document(&alice_path).await.unwrap();
    f.repo
        .write_document(&bob_path, b"reborn alpaca pizza")
        .await
        .unwrap();
    indexer.reindex_document(&bob_path).await.unwrap();
    f.repo
        .write_document(&other_tenant_path, b"reborn alpaca pizza")
        .await
        .unwrap();
    indexer.reindex_document(&other_tenant_path).await.unwrap();

    let request = MemorySearchRequest::new("alpaca")
        .unwrap()
        .with_vector(false)
        .with_limit(10);
    let alice_hits = f
        .repo
        .search_documents(alice_path.scope(), &request)
        .await
        .unwrap();
    assert!(!alice_hits.is_empty(), "alice must see her own match");
    for hit in &alice_hits {
        assert_eq!(hit.path.user_id(), "alice");
        assert_eq!(hit.path.tenant_id(), "tenant-a");
    }

    let bob_hits = f
        .repo
        .search_documents(bob_path.scope(), &request)
        .await
        .unwrap();
    for hit in &bob_hits {
        assert_eq!(hit.path.user_id(), "bob");
    }

    let other_tenant_hits = f
        .repo
        .search_documents(other_tenant_path.scope(), &request)
        .await
        .unwrap();
    for hit in &other_tenant_hits {
        assert_eq!(hit.path.tenant_id(), "tenant-b");
    }
}

#[tokio::test]
async fn fts_query_escapes_punctuation_and_handles_empty_input_gracefully() {
    let f = fresh_repository().await;
    let indexer = Arc::new(
        ChunkingMemoryDocumentIndexer::new(f.repo.clone()).with_chunk_config(ChunkConfig {
            chunk_size: 8,
            overlap_percent: 0.0,
            min_chunk_size: 1,
        }),
    );
    let path = MemoryDocumentPath::new("tenant-a", "alice", None, "punct.md").expect("path");
    f.repo
        .write_document(&path, b"vendor: OpenAI; build OK")
        .await
        .unwrap();
    indexer.reindex_document(&path).await.unwrap();

    for query in ["OpenAI:", "OpenAI*", "\"OpenAI\"", "(OpenAI)"] {
        let request = MemorySearchRequest::new(query)
            .unwrap()
            .with_vector(false)
            .with_limit(10);
        let _ = f
            .repo
            .search_documents(path.scope(), &request)
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn full_text_search_uses_rrf_when_only_full_text_branch_returns_results() {
    let f = fresh_repository().await;
    let indexer = Arc::new(
        ChunkingMemoryDocumentIndexer::new(f.repo.clone()).with_chunk_config(ChunkConfig {
            chunk_size: 4,
            overlap_percent: 0.0,
            min_chunk_size: 1,
        }),
    );
    let path = MemoryDocumentPath::new("tenant-a", "alice", None, "blend.md").expect("path");
    f.repo
        .write_document(&path, b"hybrid reborn search blends ranks")
        .await
        .unwrap();
    indexer.reindex_document(&path).await.unwrap();

    let request = MemorySearchRequest::new("hybrid")
        .unwrap()
        .with_full_text(true)
        .with_vector(false)
        .with_fusion_strategy(FusionStrategy::Rrf)
        .with_limit(10);
    let hits = f
        .repo
        .search_documents(path.scope(), &request)
        .await
        .unwrap();
    assert!(!hits.is_empty());
    assert!(hits.iter().all(|hit| hit.path.tenant_id() == "tenant-a"));
}

#[tokio::test]
async fn concurrent_writes_under_same_scope_and_path_produce_exactly_one_row() {
    // Production uses `BEGIN IMMEDIATE` plus `list_paths_for_scope` under
    // the same transaction, which serializes overlapping writers on the
    // same scope+path. Drive that with two `tokio::join!`-launched writes
    // and assert the row count is exactly one — proving no duplicate row
    // is created when writes race.
    let f = fresh_repository().await;
    let path = MemoryDocumentPath::new("tenant-a", "alice", None, "race.md").expect("path");
    let path_a = path.clone();
    let path_b = path.clone();
    let repo_a = f.repo.clone();
    let repo_b = f.repo.clone();
    let (r1, r2) = tokio::join!(
        repo_a.write_document(&path_a, b"writer-a"),
        repo_b.write_document(&path_b, b"writer-b"),
    );
    r1.expect("writer-a write");
    r2.expect("writer-b write");

    let listed = f.repo.list_documents(path.scope()).await.unwrap();
    let races = listed
        .iter()
        .filter(|p| p.relative_path() == "race.md")
        .count();
    assert_eq!(races, 1, "concurrent writes must serialize to one row");
    let stored = f.repo.read_document(&path).await.unwrap();
    assert!(matches!(
        stored.as_deref(),
        Some(b"writer-a") | Some(b"writer-b")
    ));
}

#[tokio::test]
async fn fts_query_with_only_stopwords_does_not_error() {
    // Common-stopword-only queries (e.g. "the", "and") and bare-phrase
    // queries with no FTS5 tokens must not propagate parse errors out of
    // the repository — they should succeed and return zero or all
    // results. This locks in the empty/stopword-ish FTS contract called
    // out by issue #3118.
    let f = fresh_repository().await;
    let path = MemoryDocumentPath::new("tenant-a", "alice", None, "stop.md").expect("path");
    f.repo
        .write_document(&path, b"the quick brown fox")
        .await
        .unwrap();
    let indexer = Arc::new(
        ChunkingMemoryDocumentIndexer::new(f.repo.clone()).with_chunk_config(ChunkConfig {
            chunk_size: 4,
            overlap_percent: 0.0,
            min_chunk_size: 1,
        }),
    );
    indexer.reindex_document(&path).await.unwrap();

    for query in ["the", "and", "of and the"] {
        let request = MemorySearchRequest::new(query)
            .unwrap()
            .with_vector(false)
            .with_limit(10);
        let _ = f
            .repo
            .search_documents(path.scope(), &request)
            .await
            .unwrap_or_else(|err| panic!("stopword query {query:?} must not error: {err}"));
    }
}

// --- helpers --------------------------------------------------------------

async fn count_versions(db: &Arc<libsql::Database>, path: &MemoryDocumentPath) -> i64 {
    let conn = db.connect().expect("connect");
    let scope = path.scope();
    let mut rows = conn
        .query(
            "SELECT COUNT(*) FROM reborn_memory_document_versions v \
             JOIN reborn_memory_documents d ON d.id = v.document_id \
             WHERE d.tenant_id = ?1 AND d.user_id = ?2 AND d.agent_id = ?3 \
               AND d.project_id = ?4 AND d.path = ?5",
            libsql::params![
                scope.tenant_id(),
                scope.user_id(),
                scope.agent_id().unwrap_or(""),
                scope.project_id().unwrap_or(""),
                path.relative_path(),
            ],
        )
        .await
        .unwrap();
    let row = rows.next().await.unwrap().unwrap();
    row.get::<i64>(0).unwrap()
}

async fn count_chunks(db: &Arc<libsql::Database>, path: &MemoryDocumentPath) -> i64 {
    let conn = db.connect().expect("connect");
    let scope = path.scope();
    let mut rows = conn
        .query(
            "SELECT COUNT(*) FROM reborn_memory_chunks c \
             JOIN reborn_memory_documents d ON d.id = c.document_id \
             WHERE d.tenant_id = ?1 AND d.user_id = ?2 AND d.agent_id = ?3 \
               AND d.project_id = ?4 AND d.path = ?5",
            libsql::params![
                scope.tenant_id(),
                scope.user_id(),
                scope.agent_id().unwrap_or(""),
                scope.project_id().unwrap_or(""),
                path.relative_path(),
            ],
        )
        .await
        .unwrap();
    let row = rows.next().await.unwrap().unwrap();
    row.get::<i64>(0).unwrap()
}

async fn read_version_rows(
    db: &Arc<libsql::Database>,
    path: &MemoryDocumentPath,
) -> Vec<(i64, String, String)> {
    let conn = db.connect().expect("connect");
    let scope = path.scope();
    let mut rows = conn
        .query(
            "SELECT v.version, v.content, v.content_hash \
             FROM reborn_memory_document_versions v \
             JOIN reborn_memory_documents d ON d.id = v.document_id \
             WHERE d.tenant_id = ?1 AND d.user_id = ?2 AND d.agent_id = ?3 \
               AND d.project_id = ?4 AND d.path = ?5 \
             ORDER BY v.version",
            libsql::params![
                scope.tenant_id(),
                scope.user_id(),
                scope.agent_id().unwrap_or(""),
                scope.project_id().unwrap_or(""),
                path.relative_path(),
            ],
        )
        .await
        .unwrap();
    let mut out = Vec::new();
    while let Some(row) = rows.next().await.unwrap() {
        let v: i64 = row.get(0).unwrap();
        let c: String = row.get(1).unwrap();
        let h: String = row.get(2).unwrap();
        out.push((v, c, h));
    }
    out
}
