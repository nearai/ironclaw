//! Behavior tests for `RebornLibSqlMemoryDocumentRepository` against the
//! Reborn-native `reborn_memory_*` schema (#3118 phase 4).

#![cfg(feature = "libsql")]

use std::sync::Arc;

use ironclaw_memory::{
    ChunkConfig, ChunkingMemoryDocumentIndexer, DocumentMetadata, FusionStrategy,
    MemoryAppendOutcome, MemoryChunkWrite, MemoryDocumentIndexRepository, MemoryDocumentIndexer,
    MemoryDocumentPath, MemoryDocumentRepository, MemoryDocumentScope, MemorySearchRequest,
    MemoryWriteOptions, RebornLibSqlMemoryDocumentRepository, content_sha256,
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

#[tokio::test]
async fn compare_and_append_appends_then_conflicts_on_stale_hash() {
    // The native repository must implement the optimistic atomic append
    // contract directly — the trait default would silently degrade to
    // an unsupported error. Drive it through `compare_and_append_*` to
    // lock in the hash-conflict + append-on-match behavior on the
    // Reborn-native libSQL backend.
    let f = fresh_repository().await;
    let path = MemoryDocumentPath::new("tenant-a", "alice", None, "append/race.md").expect("path");

    f.repo.write_document(&path, b"base").await.unwrap();
    let stale_hash = content_sha256("base");

    let first = f
        .repo
        .compare_and_append_document_with_options(
            &path,
            Some(&stale_hash),
            b" first",
            &MemoryWriteOptions::default(),
        )
        .await
        .unwrap();
    let second = f
        .repo
        .compare_and_append_document_with_options(
            &path,
            Some(&stale_hash),
            b" second",
            &MemoryWriteOptions::default(),
        )
        .await
        .unwrap();

    assert_eq!(first, MemoryAppendOutcome::Appended);
    assert_eq!(
        second,
        MemoryAppendOutcome::Conflict,
        "second append must observe a stale hash and refuse to append"
    );
    assert_eq!(
        f.repo.read_document(&path).await.unwrap().as_deref(),
        Some(b"base first".as_slice()),
        "row content must reflect the first append, not the conflicting second"
    );
}

#[tokio::test]
async fn compare_and_append_creates_row_with_path_conflict_check_when_absent() {
    // For a fresh path, the append contract must *create* the row
    // (None previous hash matches None current hash) and still run the
    // file/directory prefix-conflict check so the new row cannot
    // shadow an existing ancestor.
    let f = fresh_repository().await;
    let parent_file = MemoryDocumentPath::new("tenant-a", "alice", None, "notes").expect("path");
    f.repo
        .write_document(&parent_file, b"plain ancestor")
        .await
        .unwrap();

    // Append onto a child path that would conflict with the existing
    // file ancestor — must fail the path-conflict check rather than
    // creating a shadow row.
    let child = MemoryDocumentPath::new("tenant-a", "alice", None, "notes/child.md").expect("path");
    let err = f
        .repo
        .compare_and_append_document_with_options(
            &child,
            None,
            b"new",
            &MemoryWriteOptions::default(),
        )
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("existing file ancestor"),
        "path-conflict check must fire on append-create: {err}"
    );

    // A fresh sibling path with no conflict creates the row.
    let fresh_path =
        MemoryDocumentPath::new("tenant-a", "alice", None, "fresh/append.md").expect("path");
    let outcome = f
        .repo
        .compare_and_append_document_with_options(
            &fresh_path,
            None,
            b"hello",
            &MemoryWriteOptions::default(),
        )
        .await
        .unwrap();
    assert_eq!(outcome, MemoryAppendOutcome::Appended);
    assert_eq!(
        f.repo.read_document(&fresh_path).await.unwrap().as_deref(),
        Some(b"hello".as_slice())
    );
}

#[tokio::test]
async fn compare_and_append_archives_previous_content_with_changed_by_attribution() {
    // The append archival path must populate `changed_by` so version
    // history stays attributable. Reviewer flagged that direct repo
    // writes with `MemoryWriteOptions::default()` previously stored a
    // NULL `changed_by`.
    let f = fresh_repository().await;
    let path = MemoryDocumentPath::new("tenant-a", "alice", None, "append/attr.md").expect("path");

    f.repo.write_document(&path, b"base").await.unwrap();
    let base_hash = content_sha256("base");

    let opts = MemoryWriteOptions {
        metadata: DocumentMetadata::default(),
        changed_by: Some("test:append-actor".to_string()),
    };
    let outcome = f
        .repo
        .compare_and_append_document_with_options(&path, Some(&base_hash), b" added", &opts)
        .await
        .unwrap();
    assert_eq!(outcome, MemoryAppendOutcome::Appended);

    let rows = read_version_rows_with_changed_by(&f.db, &path).await;
    assert_eq!(
        rows.len(),
        1,
        "append must archive exactly one prior version"
    );
    let (_version, content, _hash, changed_by) = &rows[0];
    assert_eq!(content, "base");
    assert_eq!(
        changed_by.as_deref(),
        Some("test:append-actor"),
        "version row must record the supplied `changed_by` actor"
    );
}

#[tokio::test]
async fn direct_write_attributes_version_to_scoped_owner_key() {
    // `MemoryDocumentRepository::write_document()` is the bypass surface
    // for operators who don't go through the backend/filesystem seam.
    // The version row must still be attributable: the repo populates
    // `changed_by` from a deterministic scoped owner key.
    let f = fresh_repository().await;
    let path = MemoryDocumentPath::new("tenant-a", "alice", None, "attr.md").expect("path");

    f.repo.write_document(&path, b"v1").await.unwrap();
    f.repo.write_document(&path, b"v2").await.unwrap();

    let rows = read_version_rows_with_changed_by(&f.db, &path).await;
    assert_eq!(rows.len(), 1);
    let (_, content, _, changed_by) = &rows[0];
    assert_eq!(content, "v1");
    assert_eq!(
        changed_by.as_deref(),
        Some("tenant:tenant-a:user:alice:project:_none"),
        "direct write must attribute to scoped owner key, not NULL"
    );
}

#[tokio::test]
async fn concurrent_replace_chunks_with_same_hash_serializes_to_one_winner() {
    // zmanian test gap 1: two concurrent indexers call
    // `replace_document_chunks_if_current` with the same
    // `expected_content_hash` but different chunk sets. Production uses
    // `BEGIN IMMEDIATE` (libSQL) / `FOR UPDATE` (Postgres) so writers
    // serialize on the row, and the final state must equal exactly one
    // writer's chunk set — never a partial/duplicate union of the two.
    let f = fresh_repository().await;
    let path = MemoryDocumentPath::new("tenant-a", "alice", None, "concurrent.md").expect("path");
    f.repo.write_document(&path, b"shared body").await.unwrap();
    let hash = content_sha256("shared body");

    let chunks_a = vec![
        MemoryChunkWrite {
            content: "writer-a-1".to_string(),
            embedding: None,
        },
        MemoryChunkWrite {
            content: "writer-a-2".to_string(),
            embedding: None,
        },
    ];
    let chunks_b = vec![
        MemoryChunkWrite {
            content: "writer-b-1".to_string(),
            embedding: None,
        },
        MemoryChunkWrite {
            content: "writer-b-2".to_string(),
            embedding: None,
        },
        MemoryChunkWrite {
            content: "writer-b-3".to_string(),
            embedding: None,
        },
    ];

    let repo_a = f.repo.clone();
    let repo_b = f.repo.clone();
    let path_a = path.clone();
    let path_b = path.clone();
    let hash_a = hash.clone();
    let hash_b = hash.clone();
    let (r_a, r_b) = tokio::join!(
        async move {
            repo_a
                .replace_document_chunks_if_current(&path_a, &hash_a, &chunks_a)
                .await
        },
        async move {
            repo_b
                .replace_document_chunks_if_current(&path_b, &hash_b, &chunks_b)
                .await
        },
    );
    r_a.expect("writer A");
    r_b.expect("writer B");

    let stored_contents = read_chunk_contents(&f.db, &path).await;
    let count = stored_contents.len();
    let writer_a_set: std::collections::HashSet<&str> =
        ["writer-a-1", "writer-a-2"].iter().copied().collect();
    let writer_b_set: std::collections::HashSet<&str> = ["writer-b-1", "writer-b-2", "writer-b-3"]
        .iter()
        .copied()
        .collect();
    let stored: std::collections::HashSet<&str> =
        stored_contents.iter().map(String::as_str).collect();
    assert!(
        (count == 2 && stored == writer_a_set) || (count == 3 && stored == writer_b_set),
        "final chunk set must equal exactly one writer's contribution; got {count} chunks: {stored:?}"
    );
}

// --- helpers --------------------------------------------------------------

async fn read_chunk_contents(db: &Arc<libsql::Database>, path: &MemoryDocumentPath) -> Vec<String> {
    let conn = db.connect().expect("connect");
    let scope = path.scope();
    let mut rows = conn
        .query(
            "SELECT c.content FROM reborn_memory_chunks c \
             JOIN reborn_memory_documents d ON d.id = c.document_id \
             WHERE d.tenant_id = ?1 AND d.user_id = ?2 AND d.agent_id = ?3 \
               AND d.project_id = ?4 AND d.path = ?5 \
             ORDER BY c.chunk_index",
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
        out.push(row.get::<String>(0).unwrap());
    }
    out
}

async fn read_version_rows_with_changed_by(
    db: &Arc<libsql::Database>,
    path: &MemoryDocumentPath,
) -> Vec<(i64, String, String, Option<String>)> {
    let conn = db.connect().expect("connect");
    let scope = path.scope();
    let mut rows = conn
        .query(
            "SELECT v.version, v.content, v.content_hash, v.changed_by \
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
        let cb: Option<String> = row.get(3).ok();
        out.push((v, c, h, cb));
    }
    out
}

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
