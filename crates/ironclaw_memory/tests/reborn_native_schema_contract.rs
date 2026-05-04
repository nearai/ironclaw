//! Smoke tests for the Reborn-native memory schema (#3118 phase 3 PR 2).
//!
//! These tests prove that `run_migrations` materializes the
//! `reborn_memory_*` substrate cleanly on a fresh database and is idempotent.
//! Behavioral coverage of the repositories themselves lands in PRs 3 and 4.

#![cfg(any(feature = "libsql", feature = "postgres"))]

#[cfg(feature = "libsql")]
use ironclaw_memory::RebornLibSqlMemoryDocumentRepository;

#[cfg(feature = "libsql")]
async fn libsql_db() -> (std::sync::Arc<libsql::Database>, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("reborn_memory.db");
    let db = std::sync::Arc::new(
        libsql::Builder::new_local(db_path)
            .build()
            .await
            .expect("libsql build"),
    );
    (db, dir)
}

#[cfg(feature = "libsql")]
#[tokio::test]
async fn reborn_libsql_run_migrations_creates_native_substrate_idempotently() {
    let (db, _dir) = libsql_db().await;
    let repository = RebornLibSqlMemoryDocumentRepository::new(db.clone());

    // First run materializes the substrate from scratch.
    repository.run_migrations().await.expect("first migration");

    // Idempotent: re-running on an already-migrated DB is a no-op.
    repository.run_migrations().await.expect("re-run migration");

    // All four Reborn-native objects exist with the expected names.
    let conn = db.connect().expect("connect");
    let expected = [
        ("table", "reborn_memory_documents"),
        ("table", "reborn_memory_chunks"),
        ("table", "reborn_memory_chunks_fts"),
        ("table", "reborn_memory_document_versions"),
    ];
    for (kind, name) in expected {
        let mut rows = conn
            .query(
                "SELECT name FROM sqlite_master WHERE type = ?1 AND name = ?2",
                libsql::params![kind, name],
            )
            .await
            .expect("query schema");
        let row = rows
            .next()
            .await
            .expect("row")
            .unwrap_or_else(|| panic!("expected {kind} `{name}` to exist after migration"));
        let _: String = row.get(0).expect("name column");
    }

    // The legacy `memory_documents` table must NOT be created by the native
    // migration — Reborn memory is isolated from the legacy schema.
    let mut rows = conn
        .query(
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name = ?1",
            libsql::params!["memory_documents"],
        )
        .await
        .expect("query legacy");
    assert!(
        rows.next().await.expect("row").is_none(),
        "reborn-native migration must not create the legacy memory_documents table"
    );
}

#[cfg(feature = "postgres")]
#[allow(unused_imports)]
use ironclaw_memory::RebornPostgresMemoryDocumentRepository;
