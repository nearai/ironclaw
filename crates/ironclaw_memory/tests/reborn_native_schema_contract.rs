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
use ironclaw_memory::RebornPostgresMemoryDocumentRepository;

/// Explicit opt-in to skip the Postgres schema smoke test. Without this set,
/// a connection failure must fail loud — a silent skip would let the
/// Postgres migration ship as compile-only coverage, violating the
/// `ironclaw_memory` guardrail that Postgres coverage must be real.
#[cfg(feature = "postgres")]
const POSTGRES_SKIP_ENV: &str = "IRONCLAW_SKIP_POSTGRES_TESTS";

#[cfg(feature = "postgres")]
fn postgres_skip_requested() -> bool {
    std::env::var(POSTGRES_SKIP_ENV).is_ok_and(|value| value == "1" || value == "true")
}

#[cfg(feature = "postgres")]
async fn postgres_pool_or_skip() -> Option<deadpool_postgres::Pool> {
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://localhost/ironclaw_test".to_string());
    let config: tokio_postgres::Config = database_url
        .parse()
        .expect("DATABASE_URL must be a valid Postgres URL");
    let mgr = deadpool_postgres::Manager::new(config, tokio_postgres::NoTls);
    let pool = deadpool_postgres::Pool::builder(mgr)
        .max_size(2)
        .build()
        .expect("build deadpool");
    match pool.get().await {
        Ok(_) => Some(pool),
        Err(error) => {
            if postgres_skip_requested() {
                eprintln!(
                    "skipping reborn-postgres schema smoke test ({POSTGRES_SKIP_ENV}=1): {error}"
                );
                None
            } else {
                panic!(
                    "reborn-postgres schema smoke test could not reach Postgres ({error}); \
                     set DATABASE_URL to a reachable Postgres+pgvector instance, or set \
                     {POSTGRES_SKIP_ENV}=1 to explicitly skip."
                );
            }
        }
    }
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn reborn_postgres_run_migrations_creates_native_substrate_idempotently() {
    let Some(pool) = postgres_pool_or_skip().await else {
        return;
    };
    let repository = RebornPostgresMemoryDocumentRepository::new(pool.clone());

    // First run materializes the substrate from scratch.
    repository.run_migrations().await.expect("first migration");

    // Idempotent: re-running on an already-migrated DB is a no-op (every
    // CREATE in the schema uses `IF NOT EXISTS`).
    repository.run_migrations().await.expect("re-run migration");

    let client = pool.get().await.expect("client");

    // Both required extensions are installed. pgcrypto provides
    // `gen_random_uuid()` for the document/version primary keys; pgvector
    // provides the `VECTOR` type and the `<=>` cosine distance operator
    // used by the search path.
    for extension in ["pgcrypto", "vector"] {
        let row = client
            .query_one(
                "SELECT 1 FROM pg_extension WHERE extname = $1",
                &[&extension],
            )
            .await
            .unwrap_or_else(|_| panic!("expected `{extension}` extension to be installed"));
        let _: i32 = row.get(0);
    }

    // The three Reborn-native tables exist with the expected names.
    for table in [
        "reborn_memory_documents",
        "reborn_memory_chunks",
        "reborn_memory_document_versions",
    ] {
        let row = client
            .query_one(
                "SELECT 1 FROM information_schema.tables \
                 WHERE table_schema = 'public' AND table_name = $1",
                &[&table],
            )
            .await
            .unwrap_or_else(|_| panic!("expected table `{table}` to exist after migration"));
        let _: i32 = row.get(0);
    }

    // The `reborn_memory_chunks.content_tsv` generated column exists with
    // type `tsvector`. Generated TSVECTOR DDL must not be left compile-only.
    let tsv_row = client
        .query_one(
            "SELECT data_type, is_generated FROM information_schema.columns \
             WHERE table_schema = 'public' AND table_name = 'reborn_memory_chunks' \
               AND column_name = 'content_tsv'",
            &[],
        )
        .await
        .expect("content_tsv column must exist");
    let data_type: String = tsv_row.get("data_type");
    let is_generated: String = tsv_row.get("is_generated");
    assert_eq!(data_type, "tsvector");
    assert_eq!(is_generated, "ALWAYS");

    // `reborn_memory_chunks.embedding` is the *unbounded* `vector` type so
    // any provider dimension (Ollama 768/1024-dim, OpenAI 1536/3072-dim,
    // …) is accepted. pgvector renders unbounded vectors as `udt_name =
    // 'vector'` with a NULL `character_maximum_length` and no dimension
    // suffix on the formatted type. Match either of the two equivalent
    // ways pgvector reports an unbounded column.
    let embedding_row = client
        .query_one(
            "SELECT format_type(a.atttypid, a.atttypmod) AS formatted \
             FROM pg_attribute a \
             JOIN pg_class c ON c.oid = a.attrelid \
             JOIN pg_namespace n ON n.oid = c.relnamespace \
             WHERE n.nspname = 'public' \
               AND c.relname = 'reborn_memory_chunks' \
               AND a.attname = 'embedding'",
            &[],
        )
        .await
        .expect("reborn_memory_chunks.embedding column must exist");
    let embedding_type: String = embedding_row.get("formatted");
    assert_eq!(
        embedding_type, "vector",
        "embedding column must be unbounded `vector`, not a fixed-dimension \
         `vector(N)` — provider dimension flexibility is part of the contract"
    );

    // No HNSW index — unbounded vectors require linear scan but accept any
    // dimension. Verify the index from the previous fixed-dim shape is not
    // present so a future re-introduction without dimension constraint
    // changes is caught.
    let hnsw_present = client
        .query_opt(
            "SELECT 1 FROM pg_indexes \
             WHERE schemaname = 'public' \
               AND indexname = 'idx_reborn_memory_chunks_embedding'",
            &[],
        )
        .await
        .expect("hnsw index lookup");
    assert!(
        hnsw_present.is_none(),
        "no HNSW index should exist on unbounded vector embedding"
    );

    // The legacy `memory_documents` table must NOT be created by the native
    // migration — Reborn memory is isolated from the legacy schema.
    let legacy = client
        .query_opt(
            "SELECT 1 FROM information_schema.tables \
             WHERE table_schema = 'public' AND table_name = 'memory_documents'",
            &[],
        )
        .await
        .expect("legacy lookup");
    assert!(
        legacy.is_none(),
        "reborn-native migration must not create the legacy memory_documents table"
    );
}

#[cfg(feature = "postgres")]
#[tokio::test]
async fn reborn_postgres_run_migrations_serializes_concurrent_callers_from_two_pools() {
    // zmanian H1 + test gap 6: two `RebornPostgresMemoryDocumentRepository`
    // instances on independent pools (simulating two processes) call
    // `run_migrations` concurrently against a *cold* schema. The session-
    // level `pg_advisory_lock(REBORN_MIGRATION_LOCK_ID)` must serialize
    // them so both succeed and the schema lands exactly once. Any breakage
    // of the advisory-lock contract (or a regression that races
    // `CREATE EXTENSION` / DDL across connections) trips the assertion.
    //
    // Uses a per-test Postgres schema so dropping the tables to drive the
    // cold path does not disturb sibling tests sharing the database.
    let Some(_) = postgres_pool_or_skip().await else {
        return;
    };
    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://localhost/ironclaw_test".to_string());

    // Use a unique schema name so parallel runs of this test against the
    // same database also do not interfere.
    let schema = format!(
        "reborn_pg_migrate_race_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    );

    let pool_for_schema = |schema: &str| {
        let mut config: tokio_postgres::Config = database_url.parse().expect("valid DATABASE_URL");
        config.options(format!("-c search_path={schema},public"));
        let mgr = deadpool_postgres::Manager::new(config, tokio_postgres::NoTls);
        deadpool_postgres::Pool::builder(mgr)
            .max_size(2)
            .build()
            .expect("build deadpool")
    };

    // Provision the isolated schema using a temporary unscoped pool.
    {
        let mut bootstrap_config: tokio_postgres::Config =
            database_url.parse().expect("valid DATABASE_URL");
        bootstrap_config.application_name("reborn-migration-race-bootstrap");
        let mgr = deadpool_postgres::Manager::new(bootstrap_config, tokio_postgres::NoTls);
        let bootstrap_pool = deadpool_postgres::Pool::builder(mgr)
            .max_size(1)
            .build()
            .expect("bootstrap pool");
        let client = bootstrap_pool.get().await.expect("bootstrap client");
        client
            .batch_execute(&format!("CREATE SCHEMA IF NOT EXISTS {schema}"))
            .await
            .expect("create schema");
    }

    let pool_a = pool_for_schema(&schema);
    let pool_b = pool_for_schema(&schema);

    let repo_a = RebornPostgresMemoryDocumentRepository::new(pool_a.clone());
    let repo_b = RebornPostgresMemoryDocumentRepository::new(pool_b.clone());
    let (a, b) = tokio::join!(repo_a.run_migrations(), repo_b.run_migrations());
    a.expect("repo A migration must succeed under contention");
    b.expect("repo B migration must succeed under contention");

    // Schema landed exactly once — the unique constraint name is the
    // simplest fingerprint that is created by the schema and is
    // duplicate-detectable if the DDL ran twice without the IF NOT EXISTS
    // guarantees.
    let client = pool_a.get().await.expect("verify client");
    let count: i64 = client
        .query_one(
            "SELECT COUNT(*) FROM information_schema.table_constraints \
             WHERE constraint_schema = $1 \
               AND constraint_name = 'reborn_memory_documents_unique_scope_path'",
            &[&schema],
        )
        .await
        .expect("count unique constraint")
        .get(0);
    assert_eq!(
        count, 1,
        "concurrent migrations must produce exactly one constraint definition"
    );

    // Tear down the per-test schema so the database does not accumulate
    // schemas across runs.
    let _ = client
        .batch_execute(&format!("DROP SCHEMA {schema} CASCADE"))
        .await;
}
