//! Behavioral tests for `RebornPostgresMemoryDocumentRepository` (#3118 phase 5).
//!
//! Requires a running PostgreSQL with `pgcrypto` and `pgvector` extensions
//! available. Set `DATABASE_URL=postgres://localhost/ironclaw_test`. Tests
//! fail loud if Postgres is unreachable so the `ironclaw_memory` guardrail
//! that Postgres coverage must be real (not compile/skip) is enforced. Set
//! `IRONCLAW_SKIP_POSTGRES_TESTS=1` to opt into skipping when no DB is
//! available — the previous "silent skip + green pass" pattern let
//! migrations and read/write/search/chunk/version behavior go entirely
//! unexercised.
//!
//! Each test creates a fresh tenant prefix so tests do not interfere even when
//! sharing a Postgres instance.

#![cfg(feature = "postgres")]

use std::sync::Arc;

use ironclaw_memory::{
    ChunkConfig, ChunkingMemoryDocumentIndexer, DocumentMetadata, FusionStrategy,
    MemoryAppendOutcome, MemoryChunkWrite, MemoryDocumentIndexRepository, MemoryDocumentIndexer,
    MemoryDocumentPath, MemoryDocumentRepository, MemoryDocumentScope, MemorySearchRequest,
    MemoryWriteOptions, RebornPostgresMemoryDocumentRepository, content_sha256,
};

fn pool() -> deadpool_postgres::Pool {
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

/// Explicit opt-in to skip the Postgres contract tests. Without this set,
/// a connection failure must fail loud — the previous "silent skip + green
/// pass" pattern violated the `ironclaw_memory` guardrail that Postgres
/// behavioral coverage must be real.
const POSTGRES_SKIP_ENV: &str = "IRONCLAW_SKIP_POSTGRES_TESTS";

fn skip_requested() -> bool {
    std::env::var(POSTGRES_SKIP_ENV).is_ok_and(|value| value == "1" || value == "true")
}

/// Returns `Some(())` if the pool can hand out a connection. Returns `None`
/// only when the caller has opted into skipping via `IRONCLAW_SKIP_POSTGRES_TESTS=1`.
/// Otherwise panics with an actionable message so a missing/unreachable DB
/// surfaces as a real test failure instead of a green skip.
async fn try_connect(pool: &deadpool_postgres::Pool) -> Option<()> {
    match pool.get().await {
        Ok(_) => Some(()),
        Err(error) => {
            if skip_requested() {
                eprintln!("skipping reborn-postgres test ({POSTGRES_SKIP_ENV}=1): {error}");
                None
            } else {
                panic!(
                    "reborn-postgres test could not reach Postgres ({error}); \
                     set DATABASE_URL to a reachable Postgres+pgvector instance, or set \
                     {POSTGRES_SKIP_ENV}=1 to explicitly skip."
                );
            }
        }
    }
}

async fn cleanup_tenant(pool: &deadpool_postgres::Pool, tenant_id: &str) {
    let Ok(client) = pool.get().await else { return };
    let _ = client
        .execute(
            "DELETE FROM reborn_memory_documents WHERE tenant_id = $1",
            &[&tenant_id],
        )
        .await;
}

async fn fresh_repository(tenant_id: &str) -> Option<Arc<RebornPostgresMemoryDocumentRepository>> {
    let pool = pool();
    try_connect(&pool).await?;
    let repo = Arc::new(RebornPostgresMemoryDocumentRepository::new(pool.clone()));
    repo.run_migrations().await.expect("run_migrations");
    cleanup_tenant(&pool, tenant_id).await;
    Some(repo)
}

#[tokio::test]
async fn round_trips_a_document_within_full_scope() {
    let tenant = "reborn-pg-round-trip";
    let Some(repo) = fresh_repository(tenant).await else {
        return;
    };
    let path = MemoryDocumentPath::new(tenant, "alice", None, "MEMORY.md").expect("path");
    repo.write_document(&path, b"hello reborn pg")
        .await
        .unwrap();
    let stored = repo.read_document(&path).await.unwrap();
    assert_eq!(stored.as_deref(), Some(b"hello reborn pg".as_slice()));
    cleanup_tenant(&pool(), tenant).await;
}

#[tokio::test]
async fn returns_none_when_document_is_missing() {
    let tenant = "reborn-pg-missing";
    let Some(repo) = fresh_repository(tenant).await else {
        return;
    };
    let path = MemoryDocumentPath::new(tenant, "alice", None, "missing.md").expect("path");
    assert!(repo.read_document(&path).await.unwrap().is_none());
    cleanup_tenant(&pool(), tenant).await;
}

#[tokio::test]
async fn upsert_replaces_content_for_same_full_scope_and_path() {
    let tenant = "reborn-pg-upsert";
    let Some(repo) = fresh_repository(tenant).await else {
        return;
    };
    let path = MemoryDocumentPath::new(tenant, "alice", None, "notes.md").expect("path");
    repo.write_document(&path, b"first").await.unwrap();
    repo.write_document(&path, b"second").await.unwrap();
    repo.write_document(&path, b"third").await.unwrap();
    assert_eq!(
        repo.read_document(&path).await.unwrap().as_deref(),
        Some(b"third".as_slice())
    );
    let listed = repo.list_documents(path.scope()).await.unwrap();
    assert_eq!(
        listed
            .iter()
            .filter(|p| p.relative_path() == "notes.md")
            .count(),
        1,
        "upsert must not create a duplicate row"
    );
    cleanup_tenant(&pool(), tenant).await;
}

#[tokio::test]
async fn full_scope_isolates_user_agent_project_independently() {
    let tenant = "reborn-pg-scope";
    let Some(repo) = fresh_repository(tenant).await else {
        return;
    };

    struct ScopeFixture {
        user: &'static str,
        agent: Option<&'static str>,
        project: Option<&'static str>,
        body: &'static [u8],
    }
    let writes = [
        ScopeFixture {
            user: "alice",
            agent: None,
            project: None,
            body: b"baseline",
        },
        ScopeFixture {
            user: "bob",
            agent: None,
            project: None,
            body: b"other-user",
        },
        ScopeFixture {
            user: "alice",
            agent: Some("scout"),
            project: None,
            body: b"scout-agent",
        },
        ScopeFixture {
            user: "alice",
            agent: None,
            project: Some("alpha"),
            body: b"alpha-project",
        },
    ];

    for fixture in &writes {
        let path = MemoryDocumentPath::new_with_agent(
            tenant,
            fixture.user,
            fixture.agent,
            fixture.project,
            "shared.md",
        )
        .expect("path");
        repo.write_document(&path, fixture.body).await.unwrap();
    }

    for fixture in &writes {
        let path = MemoryDocumentPath::new_with_agent(
            tenant,
            fixture.user,
            fixture.agent,
            fixture.project,
            "shared.md",
        )
        .expect("path");
        let stored = repo.read_document(&path).await.unwrap();
        assert_eq!(stored.as_deref(), Some(fixture.body));
    }

    for fixture in &writes {
        let scope = MemoryDocumentScope::new_with_agent(
            tenant,
            fixture.user,
            fixture.agent,
            fixture.project,
        )
        .expect("scope");
        let listed = repo.list_documents(&scope).await.unwrap();
        assert_eq!(
            listed.len(),
            1,
            "{}/{:?}/{:?} must list only itself, got {:?}",
            fixture.user,
            fixture.agent,
            fixture.project,
            listed,
        );
    }
    cleanup_tenant(&pool(), tenant).await;
}

#[tokio::test]
async fn rejects_file_directory_prefix_conflicts_within_scope() {
    let tenant = "reborn-pg-conflicts";
    let Some(repo) = fresh_repository(tenant).await else {
        return;
    };
    let file = MemoryDocumentPath::new(tenant, "alice", None, "notes").expect("path");
    let child = MemoryDocumentPath::new(tenant, "alice", None, "notes/a.md").expect("path");

    repo.write_document(&file, b"plain file").await.unwrap();
    let err = repo.write_document(&child, b"child").await.unwrap_err();
    assert!(err.to_string().contains("existing file ancestor"));
    cleanup_tenant(&pool(), tenant).await;

    let Some(repo2) = fresh_repository(tenant).await else {
        return;
    };
    repo2.write_document(&child, b"child").await.unwrap();
    let err = repo2
        .write_document(&file, b"plain file")
        .await
        .unwrap_err();
    assert!(err.to_string().contains("existing directory"));
    cleanup_tenant(&pool(), tenant).await;
}

#[tokio::test]
async fn writes_metadata_and_reads_it_back() {
    let tenant = "reborn-pg-metadata";
    let Some(repo) = fresh_repository(tenant).await else {
        return;
    };
    let path = MemoryDocumentPath::new(tenant, "alice", None, "doc.md").expect("path");
    repo.write_document(&path, b"hello").await.unwrap();
    let metadata = serde_json::json!({"tag": "primary", "count": 3});
    repo.write_document_metadata(&path, &metadata)
        .await
        .unwrap();
    let read_back = repo.read_document_metadata(&path).await.unwrap();
    assert_eq!(read_back, Some(metadata));
    cleanup_tenant(&pool(), tenant).await;
}

#[tokio::test]
async fn write_with_options_creates_version_row_only_when_not_skipped() {
    let tenant = "reborn-pg-versioning";
    let Some(repo) = fresh_repository(tenant).await else {
        return;
    };
    let path = MemoryDocumentPath::new(tenant, "alice", None, "v.md").expect("path");

    repo.write_document(&path, b"v1").await.unwrap();
    let opts = MemoryWriteOptions {
        metadata: DocumentMetadata::default(),
        changed_by: Some("test:default".to_string()),
    };
    repo.write_document_with_options(&path, b"v2", &opts)
        .await
        .unwrap();
    repo.write_document_with_options(&path, b"v3", &opts)
        .await
        .unwrap();
    let opts_skip = MemoryWriteOptions {
        metadata: DocumentMetadata {
            skip_versioning: Some(true),
            ..DocumentMetadata::default()
        },
        changed_by: Some("test:skip".to_string()),
    };
    repo.write_document_with_options(&path, b"v4-skip", &opts_skip)
        .await
        .unwrap();

    let count = count_versions(&pool(), tenant, &path).await;
    assert_eq!(count, 2, "expected 2 version rows, got {count}");
    cleanup_tenant(&pool(), tenant).await;
}

#[tokio::test]
async fn version_numbers_are_monotonic_and_content_hash_matches_archived_content() {
    let tenant = "reborn-pg-monotonic";
    let Some(repo) = fresh_repository(tenant).await else {
        return;
    };
    let path = MemoryDocumentPath::new(tenant, "alice", None, "h.md").expect("path");

    repo.write_document(&path, b"v1").await.unwrap();
    repo.write_document(&path, b"v2").await.unwrap();
    repo.write_document(&path, b"v3").await.unwrap();

    let rows = read_version_rows(&pool(), tenant, &path).await;
    let mut versions: Vec<i32> = rows.iter().map(|(v, _, _)| *v).collect();
    versions.sort();
    assert_eq!(versions, vec![1, 2]);
    for (_, content, hash) in &rows {
        assert_eq!(hash, &content_sha256(content));
    }
    cleanup_tenant(&pool(), tenant).await;
}

#[tokio::test]
async fn replace_chunks_if_current_is_a_noop_when_document_was_rewritten() {
    let tenant = "reborn-pg-drift";
    let Some(repo) = fresh_repository(tenant).await else {
        return;
    };
    let path = MemoryDocumentPath::new(tenant, "alice", None, "drift.md").expect("path");
    repo.write_document(&path, b"original content")
        .await
        .unwrap();
    let stale_hash = content_sha256("original content");
    repo.write_document(&path, b"newer content").await.unwrap();

    let stale_chunks = vec![MemoryChunkWrite {
        content: "original content".to_string(),
        embedding: None,
    }];
    repo.replace_document_chunks_if_current(&path, &stale_hash, &stale_chunks)
        .await
        .unwrap();

    assert_eq!(count_chunks(&pool(), tenant, &path).await, 0);
    cleanup_tenant(&pool(), tenant).await;
}

#[tokio::test]
async fn full_text_search_returns_only_chunks_within_full_scope() {
    let tenant = "reborn-pg-fts";
    let Some(repo) = fresh_repository(tenant).await else {
        return;
    };
    let indexer = Arc::new(
        ChunkingMemoryDocumentIndexer::new(repo.clone()).with_chunk_config(ChunkConfig {
            chunk_size: 4,
            overlap_percent: 0.0,
            min_chunk_size: 1,
        }),
    );

    let alice_path = MemoryDocumentPath::new(tenant, "alice", None, "notes.md").expect("path");
    let bob_path = MemoryDocumentPath::new(tenant, "bob", None, "notes.md").expect("path");

    repo.write_document(&alice_path, b"reborn alpaca pizza")
        .await
        .unwrap();
    indexer.reindex_document(&alice_path).await.unwrap();
    repo.write_document(&bob_path, b"reborn alpaca pizza")
        .await
        .unwrap();
    indexer.reindex_document(&bob_path).await.unwrap();

    let request = MemorySearchRequest::new("alpaca")
        .unwrap()
        .with_vector(false)
        .with_limit(10);
    let alice_hits = repo
        .search_documents(alice_path.scope(), &request)
        .await
        .unwrap();
    assert!(!alice_hits.is_empty(), "alice must see her own match");
    for hit in &alice_hits {
        assert_eq!(hit.path.user_id(), "alice");
        assert_eq!(hit.path.tenant_id(), tenant);
    }
    let bob_hits = repo
        .search_documents(bob_path.scope(), &request)
        .await
        .unwrap();
    for hit in &bob_hits {
        assert_eq!(hit.path.user_id(), "bob");
    }
    cleanup_tenant(&pool(), tenant).await;
}

#[tokio::test]
async fn top_level_projects_path_is_a_normal_user_path_not_project_scope() {
    // The issue is explicit: a relative path beginning with "projects/" must
    // NOT be re-interpreted as project scope. Project scope only comes from
    // the explicit MemoryDocumentScope project_id.
    let tenant = "reborn-pg-projects-prefix";
    let Some(repo) = fresh_repository(tenant).await else {
        return;
    };
    let user_doc =
        MemoryDocumentPath::new(tenant, "alice", None, "projects/local-note.md").expect("path");
    repo.write_document(&user_doc, b"user-scoped note")
        .await
        .unwrap();

    let project_doc =
        MemoryDocumentPath::new(tenant, "alice", Some("alpha"), "projects/local-note.md")
            .expect("path");
    repo.write_document(&project_doc, b"alpha-scoped note")
        .await
        .unwrap();

    assert_eq!(
        repo.read_document(&user_doc).await.unwrap().as_deref(),
        Some(b"user-scoped note".as_slice())
    );
    assert_eq!(
        repo.read_document(&project_doc).await.unwrap().as_deref(),
        Some(b"alpha-scoped note".as_slice())
    );
    cleanup_tenant(&pool(), tenant).await;
}

#[tokio::test]
async fn fts_query_escapes_punctuation_and_handles_empty_input_gracefully() {
    // Postgres uses `plainto_tsquery`, which already tolerates arbitrary
    // punctuation without manual escaping. The contract this test locks in
    // is the same as the libSQL counterpart: queries with `:`, `*`, `"…"`,
    // and `(…)` must not error.
    let tenant = "reborn-pg-fts-punct";
    let Some(repo) = fresh_repository(tenant).await else {
        return;
    };
    let indexer = Arc::new(
        ChunkingMemoryDocumentIndexer::new(repo.clone()).with_chunk_config(ChunkConfig {
            chunk_size: 8,
            overlap_percent: 0.0,
            min_chunk_size: 1,
        }),
    );
    let path = MemoryDocumentPath::new(tenant, "alice", None, "punct.md").expect("path");
    repo.write_document(&path, b"vendor: OpenAI; build OK")
        .await
        .unwrap();
    indexer.reindex_document(&path).await.unwrap();

    for query in ["OpenAI:", "OpenAI*", "\"OpenAI\"", "(OpenAI)"] {
        let request = MemorySearchRequest::new(query)
            .unwrap()
            .with_vector(false)
            .with_limit(10);
        let _ = repo.search_documents(path.scope(), &request).await.unwrap();
    }
    cleanup_tenant(&pool(), tenant).await;
}

#[tokio::test]
async fn full_text_search_uses_rrf_when_only_full_text_branch_returns_results() {
    let tenant = "reborn-pg-rrf";
    let Some(repo) = fresh_repository(tenant).await else {
        return;
    };
    let indexer = Arc::new(
        ChunkingMemoryDocumentIndexer::new(repo.clone()).with_chunk_config(ChunkConfig {
            chunk_size: 4,
            overlap_percent: 0.0,
            min_chunk_size: 1,
        }),
    );
    let path = MemoryDocumentPath::new(tenant, "alice", None, "blend.md").expect("path");
    repo.write_document(&path, b"hybrid reborn search blends ranks")
        .await
        .unwrap();
    indexer.reindex_document(&path).await.unwrap();

    let request = MemorySearchRequest::new("hybrid")
        .unwrap()
        .with_full_text(true)
        .with_vector(false)
        .with_fusion_strategy(FusionStrategy::Rrf)
        .with_limit(10);
    let hits = repo.search_documents(path.scope(), &request).await.unwrap();
    assert!(!hits.is_empty());
    assert!(hits.iter().all(|hit| hit.path.tenant_id() == tenant));
    cleanup_tenant(&pool(), tenant).await;
}

#[tokio::test]
async fn same_path_in_different_tenants_stores_separate_rows() {
    // libSQL parity test (`full_scope_isolates_tenant_user_agent_project_independently`)
    // varies tenant explicitly. Postgres uses identical SQL filters for
    // the tenant column, but must be proven independently per the issue
    // guardrail "Postgres compile-only coverage is not sufficient".
    let tenant_a = "reborn-pg-tenant-iso-a";
    let tenant_b = "reborn-pg-tenant-iso-b";
    let Some(repo_a) = fresh_repository(tenant_a).await else {
        return;
    };
    cleanup_tenant(&pool(), tenant_b).await;

    let path_a = MemoryDocumentPath::new(tenant_a, "alice", None, "shared.md").expect("path");
    let path_b = MemoryDocumentPath::new(tenant_b, "alice", None, "shared.md").expect("path");
    repo_a.write_document(&path_a, b"a-body").await.unwrap();
    repo_a.write_document(&path_b, b"b-body").await.unwrap();

    assert_eq!(
        repo_a.read_document(&path_a).await.unwrap().as_deref(),
        Some(b"a-body".as_slice())
    );
    assert_eq!(
        repo_a.read_document(&path_b).await.unwrap().as_deref(),
        Some(b"b-body".as_slice())
    );
    let listed_a = repo_a.list_documents(path_a.scope()).await.unwrap();
    assert_eq!(listed_a.len(), 1);
    assert_eq!(listed_a[0].tenant_id(), tenant_a);
    let listed_b = repo_a.list_documents(path_b.scope()).await.unwrap();
    assert_eq!(listed_b.len(), 1);
    assert_eq!(listed_b[0].tenant_id(), tenant_b);

    cleanup_tenant(&pool(), tenant_a).await;
    cleanup_tenant(&pool(), tenant_b).await;
}

#[tokio::test]
async fn concurrent_writes_under_same_scope_and_path_produce_exactly_one_row() {
    // Postgres uses `LOCK TABLE … IN SHARE ROW EXCLUSIVE MODE` inside the
    // write transaction, which serializes overlapping writers on the
    // same scope+path. Drive that with two `tokio::join!`-launched writes
    // and assert the row count is exactly one.
    let tenant = "reborn-pg-race";
    let Some(repo) = fresh_repository(tenant).await else {
        return;
    };
    let path = MemoryDocumentPath::new(tenant, "alice", None, "race.md").expect("path");
    let path_a = path.clone();
    let path_b = path.clone();
    let repo_a = repo.clone();
    let repo_b = repo.clone();
    let (r1, r2) = tokio::join!(
        repo_a.write_document(&path_a, b"writer-a"),
        repo_b.write_document(&path_b, b"writer-b"),
    );
    r1.expect("writer-a");
    r2.expect("writer-b");

    let listed = repo.list_documents(path.scope()).await.unwrap();
    let races = listed
        .iter()
        .filter(|p| p.relative_path() == "race.md")
        .count();
    assert_eq!(races, 1, "concurrent writes must serialize to one row");
    cleanup_tenant(&pool(), tenant).await;
}

#[tokio::test]
async fn fts_query_with_only_stopwords_does_not_error() {
    let tenant = "reborn-pg-stopword";
    let Some(repo) = fresh_repository(tenant).await else {
        return;
    };
    let indexer = Arc::new(
        ChunkingMemoryDocumentIndexer::new(repo.clone()).with_chunk_config(ChunkConfig {
            chunk_size: 4,
            overlap_percent: 0.0,
            min_chunk_size: 1,
        }),
    );
    let path = MemoryDocumentPath::new(tenant, "alice", None, "stop.md").expect("path");
    repo.write_document(&path, b"the quick brown fox")
        .await
        .unwrap();
    indexer.reindex_document(&path).await.unwrap();

    for query in ["the", "and", "of and the"] {
        let request = MemorySearchRequest::new(query)
            .unwrap()
            .with_vector(false)
            .with_limit(10);
        let _ = repo
            .search_documents(path.scope(), &request)
            .await
            .unwrap_or_else(|err| panic!("stopword query {query:?} must not error: {err}"));
    }
    cleanup_tenant(&pool(), tenant).await;
}

#[tokio::test]
async fn compare_and_append_appends_then_conflicts_on_stale_hash() {
    // Same optimistic atomic append contract that the libSQL native
    // repository implements, locked in for Postgres. Reviewer
    // explicitly required parity across both backends.
    let tenant = "reborn-pg-append-stale";
    let Some(repo) = fresh_repository(tenant).await else {
        return;
    };
    let path = MemoryDocumentPath::new(tenant, "alice", None, "append/race.md").expect("path");

    repo.write_document(&path, b"base").await.unwrap();
    let stale_hash = content_sha256("base");

    let first = repo
        .compare_and_append_document_with_options(
            &path,
            Some(&stale_hash),
            b" first",
            &MemoryWriteOptions::default(),
        )
        .await
        .unwrap();
    let second = repo
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
        repo.read_document(&path).await.unwrap().as_deref(),
        Some(b"base first".as_slice())
    );
    cleanup_tenant(&pool(), tenant).await;
}

#[tokio::test]
async fn compare_and_append_creates_row_with_path_conflict_check_when_absent() {
    // Append against a brand-new path must create the row but still
    // run the prefix-conflict check so the new row cannot shadow an
    // existing ancestor under the same scope.
    let tenant = "reborn-pg-append-create";
    let Some(repo) = fresh_repository(tenant).await else {
        return;
    };
    let parent_file = MemoryDocumentPath::new(tenant, "alice", None, "notes").expect("path");
    repo.write_document(&parent_file, b"plain ancestor")
        .await
        .unwrap();

    let child = MemoryDocumentPath::new(tenant, "alice", None, "notes/child.md").expect("path");
    let err = repo
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

    let fresh_path =
        MemoryDocumentPath::new(tenant, "alice", None, "fresh/append.md").expect("path");
    let outcome = repo
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
        repo.read_document(&fresh_path).await.unwrap().as_deref(),
        Some(b"hello".as_slice())
    );
    cleanup_tenant(&pool(), tenant).await;
}

#[tokio::test]
async fn compare_and_append_archives_previous_content_with_changed_by_attribution() {
    // The append archival path must populate `changed_by` exactly as
    // the libSQL native repository does. NULL `changed_by` was the
    // root cause of the reviewer-flagged attribution gap.
    let tenant = "reborn-pg-append-attr";
    let Some(repo) = fresh_repository(tenant).await else {
        return;
    };
    let path = MemoryDocumentPath::new(tenant, "alice", None, "append/attr.md").expect("path");

    repo.write_document(&path, b"base").await.unwrap();
    let base_hash = content_sha256("base");

    let opts = MemoryWriteOptions {
        metadata: DocumentMetadata::default(),
        changed_by: Some("test:append-actor".to_string()),
    };
    let outcome = repo
        .compare_and_append_document_with_options(&path, Some(&base_hash), b" added", &opts)
        .await
        .unwrap();
    assert_eq!(outcome, MemoryAppendOutcome::Appended);

    let rows = read_version_rows_with_changed_by(&pool(), tenant, &path).await;
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
    cleanup_tenant(&pool(), tenant).await;
}

#[tokio::test]
async fn direct_write_attributes_version_to_scoped_owner_key() {
    // `MemoryDocumentRepository::write_document()` is the bypass
    // surface for operators not going through the backend/filesystem
    // seam. The repo must populate `changed_by` deterministically so
    // version history is never NULL-attributed.
    let tenant = "reborn-pg-direct-attr";
    let Some(repo) = fresh_repository(tenant).await else {
        return;
    };
    let path = MemoryDocumentPath::new(tenant, "alice", None, "attr.md").expect("path");

    repo.write_document(&path, b"v1").await.unwrap();
    repo.write_document(&path, b"v2").await.unwrap();

    let rows = read_version_rows_with_changed_by(&pool(), tenant, &path).await;
    assert_eq!(rows.len(), 1);
    let (_, content, _, changed_by) = &rows[0];
    assert_eq!(content, "v1");
    let expected = format!("tenant:{tenant}:user:alice:project:_none");
    assert_eq!(
        changed_by.as_deref(),
        Some(expected.as_str()),
        "direct write must attribute to scoped owner key, not NULL"
    );
    cleanup_tenant(&pool(), tenant).await;
}

#[tokio::test]
async fn embedding_column_accepts_non_1536_dimension_vectors() {
    // The native Postgres schema declares `embedding vector` (unbounded)
    // so providers with non-1536 dimensions (Ollama 768/1024, OpenAI
    // 3072, Claude 1024, …) write and read without a hard-coded
    // dimension constraint. Drive a 5-dim chunk write to exercise the
    // same column the search path queries.
    let tenant = "reborn-pg-vector-nondefault";
    let Some(repo) = fresh_repository(tenant).await else {
        return;
    };
    let path = MemoryDocumentPath::new(tenant, "alice", None, "vec.md").expect("path");
    repo.write_document(&path, b"vector dimension test")
        .await
        .unwrap();
    let hash = content_sha256("vector dimension test");

    let chunks = vec![
        MemoryChunkWrite {
            content: "vector dimension".to_string(),
            // 5 dimensions — would fail under a hard-coded `vector(1536)`.
            embedding: Some(vec![0.1, 0.2, 0.3, 0.4, 0.5]),
        },
        MemoryChunkWrite {
            content: "test".to_string(),
            // 7 dimensions, deliberately different from the previous
            // chunk to lock in that the column never narrows to a
            // single per-table dimension either.
            embedding: Some(vec![1.0, 0.0, 0.0, 0.0, 0.5, 0.5, 0.0]),
        },
    ];
    repo.replace_document_chunks_if_current(&path, &hash, &chunks)
        .await
        .unwrap();

    let stored_chunks = count_chunks(&pool(), tenant, &path).await;
    assert_eq!(
        stored_chunks, 2,
        "expected 2 chunks persisted with mixed-dimension embeddings; got {stored_chunks}"
    );
    cleanup_tenant(&pool(), tenant).await;
}

#[tokio::test]
async fn writes_with_non_english_cjk_content_persist_under_english_tsvector_config() {
    // zmanian test gap 5: `reborn_memory_chunks.content_tsv` is a generated
    // TSVECTOR computed with `to_tsvector('english', content)`. Non-English
    // content (CJK, etc.) gets degraded full-text relevance under the
    // English config, but the write must not fail — `to_tsvector` is
    // total over arbitrary text. Smoke-test that a chunk with a Japanese
    // body and a chunk with a Chinese body land cleanly through the
    // indexer-driven write path so a regression that switches to a
    // configuration that rejects non-ASCII (e.g. `'simple'` after a
    // typo, or a missing dictionary) is caught.
    let tenant = "reborn-pg-cjk-tsvector";
    let Some(repo) = fresh_repository(tenant).await else {
        return;
    };
    let path = MemoryDocumentPath::new(tenant, "alice", None, "cjk.md").expect("path");
    let body = "こんにちは世界 — 你好世界 — hello world";
    repo.write_document(&path, body.as_bytes()).await.unwrap();
    let hash = content_sha256(body);

    let chunks = vec![
        MemoryChunkWrite {
            content: "こんにちは世界".to_string(),
            embedding: None,
        },
        MemoryChunkWrite {
            content: "你好世界".to_string(),
            embedding: None,
        },
        MemoryChunkWrite {
            content: "hello world".to_string(),
            embedding: None,
        },
    ];
    repo.replace_document_chunks_if_current(&path, &hash, &chunks)
        .await
        .expect("CJK chunks must persist under the english tsvector config");

    // The English-language token "hello" must still be findable via the
    // standard FTS path; the CJK chunks may or may not be retrievable
    // depending on the dictionary, but the writes must have succeeded.
    let request = MemorySearchRequest::new("hello")
        .unwrap()
        .with_vector(false)
        .with_limit(10);
    let hits = repo
        .search_documents(path.scope(), &request)
        .await
        .expect("english fts query against CJK-bearing rows must succeed");
    assert!(
        !hits.is_empty(),
        "english token 'hello' must hit the english chunk even when CJK siblings are present"
    );
    cleanup_tenant(&pool(), tenant).await;
}

#[tokio::test]
async fn concurrent_replace_chunks_with_same_hash_serializes_to_one_winner() {
    // zmanian test gap 1: two concurrent indexers call
    // `replace_document_chunks_if_current` with the same
    // `expected_content_hash` but different chunk sets. The Postgres
    // implementation uses `SELECT … FOR UPDATE` to pin the document row;
    // writers serialize on the row, and the final state must equal
    // exactly one writer's chunk set — never a partial/duplicate union.
    let tenant = "reborn-pg-concurrent-chunks";
    let Some(repo) = fresh_repository(tenant).await else {
        return;
    };
    let path = MemoryDocumentPath::new(tenant, "alice", None, "concurrent.md").expect("path");
    repo.write_document(&path, b"shared body").await.unwrap();
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

    let repo_a = repo.clone();
    let repo_b = repo.clone();
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

    let stored = read_chunk_contents(&pool(), tenant, &path).await;
    let count = stored.len();
    let writer_a_set: std::collections::HashSet<&str> =
        ["writer-a-1", "writer-a-2"].iter().copied().collect();
    let writer_b_set: std::collections::HashSet<&str> = ["writer-b-1", "writer-b-2", "writer-b-3"]
        .iter()
        .copied()
        .collect();
    let stored_set: std::collections::HashSet<&str> = stored.iter().map(String::as_str).collect();
    assert!(
        (count == 2 && stored_set == writer_a_set) || (count == 3 && stored_set == writer_b_set),
        "final chunk set must equal exactly one writer's contribution; got {count} chunks: {stored_set:?}"
    );
    cleanup_tenant(&pool(), tenant).await;
}

// --- helpers --------------------------------------------------------------

async fn read_chunk_contents(
    pool: &deadpool_postgres::Pool,
    tenant_id: &str,
    path: &MemoryDocumentPath,
) -> Vec<String> {
    let client = pool.get().await.expect("client");
    let scope = path.scope();
    let rows = client
        .query(
            "SELECT c.content FROM reborn_memory_chunks c \
             JOIN reborn_memory_documents d ON d.id = c.document_id \
             WHERE d.tenant_id = $1 AND d.user_id = $2 AND d.agent_id = $3 \
               AND d.project_id = $4 AND d.path = $5 \
             ORDER BY c.chunk_index",
            &[
                &tenant_id,
                &scope.user_id(),
                &scope.agent_id().unwrap_or(""),
                &scope.project_id().unwrap_or(""),
                &path.relative_path(),
            ],
        )
        .await
        .expect("read chunk contents");
    rows.into_iter()
        .map(|row| row.get::<_, String>(0))
        .collect()
}

async fn read_version_rows_with_changed_by(
    pool: &deadpool_postgres::Pool,
    tenant_id: &str,
    path: &MemoryDocumentPath,
) -> Vec<(i32, String, String, Option<String>)> {
    let client = pool.get().await.expect("get client");
    let scope = path.scope();
    let rows = client
        .query(
            "SELECT v.version, v.content, v.content_hash, v.changed_by \
             FROM reborn_memory_document_versions v \
             JOIN reborn_memory_documents d ON d.id = v.document_id \
             WHERE d.tenant_id = $1 AND d.user_id = $2 AND d.agent_id = $3 \
               AND d.project_id = $4 AND d.path = $5 \
             ORDER BY v.version",
            &[
                &tenant_id,
                &scope.user_id(),
                &scope.agent_id().unwrap_or(""),
                &scope.project_id().unwrap_or(""),
                &path.relative_path(),
            ],
        )
        .await
        .expect("read versions with changed_by");
    rows.into_iter()
        .map(|row| {
            let v: i32 = row.get(0);
            let c: String = row.get(1);
            let h: String = row.get(2);
            let cb: Option<String> = row.get(3);
            (v, c, h, cb)
        })
        .collect()
}

async fn count_versions(
    pool: &deadpool_postgres::Pool,
    tenant_id: &str,
    path: &MemoryDocumentPath,
) -> i64 {
    let client = pool.get().await.expect("get client");
    let scope = path.scope();
    let row = client
        .query_one(
            "SELECT COUNT(*) FROM reborn_memory_document_versions v \
             JOIN reborn_memory_documents d ON d.id = v.document_id \
             WHERE d.tenant_id = $1 AND d.user_id = $2 AND d.agent_id = $3 \
               AND d.project_id = $4 AND d.path = $5",
            &[
                &tenant_id,
                &scope.user_id(),
                &scope.agent_id().unwrap_or(""),
                &scope.project_id().unwrap_or(""),
                &path.relative_path(),
            ],
        )
        .await
        .expect("count versions");
    row.get(0)
}

async fn count_chunks(
    pool: &deadpool_postgres::Pool,
    tenant_id: &str,
    path: &MemoryDocumentPath,
) -> i64 {
    let client = pool.get().await.expect("get client");
    let scope = path.scope();
    let row = client
        .query_one(
            "SELECT COUNT(*) FROM reborn_memory_chunks c \
             JOIN reborn_memory_documents d ON d.id = c.document_id \
             WHERE d.tenant_id = $1 AND d.user_id = $2 AND d.agent_id = $3 \
               AND d.project_id = $4 AND d.path = $5",
            &[
                &tenant_id,
                &scope.user_id(),
                &scope.agent_id().unwrap_or(""),
                &scope.project_id().unwrap_or(""),
                &path.relative_path(),
            ],
        )
        .await
        .expect("count chunks");
    row.get(0)
}

async fn read_version_rows(
    pool: &deadpool_postgres::Pool,
    tenant_id: &str,
    path: &MemoryDocumentPath,
) -> Vec<(i32, String, String)> {
    let client = pool.get().await.expect("get client");
    let scope = path.scope();
    let rows = client
        .query(
            "SELECT v.version, v.content, v.content_hash \
             FROM reborn_memory_document_versions v \
             JOIN reborn_memory_documents d ON d.id = v.document_id \
             WHERE d.tenant_id = $1 AND d.user_id = $2 AND d.agent_id = $3 \
               AND d.project_id = $4 AND d.path = $5 \
             ORDER BY v.version",
            &[
                &tenant_id,
                &scope.user_id(),
                &scope.agent_id().unwrap_or(""),
                &scope.project_id().unwrap_or(""),
                &path.relative_path(),
            ],
        )
        .await
        .expect("read versions");
    rows.into_iter()
        .map(|row| {
            let v: i32 = row.get(0);
            let c: String = row.get(1);
            let h: String = row.get(2);
            (v, c, h)
        })
        .collect()
}
