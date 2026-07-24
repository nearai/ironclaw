//! Concurrent CAS-storm regression tests (#5466).
//!
//! Drives many genuinely parallel `cas_update` read-modify-write loops —
//! multi-threaded tokio runtime, `tokio::spawn` per writer and round — against
//! one shared snapshot path, per backend. The libSQL variant is the
//! regression pin for #5466: the previous connection-per-operation
//! policy intermittently failed inside the C library (`SQLITE_MISUSE`,
//! spurious `disk I/O error`) under exactly this load, and a
//! (rejected) single-shared-connection design instead corrupted the CAS
//! rows-affected readback into lost updates. Both defects are caught
//! here: any backend error fails a writer, and a lost update fails the
//! final-count assertion.
//!
//! The single-threaded-runtime sibling for the in-memory backend lives
//! in `src/cas/tests.rs` (`high_contention_storm_has_no_lost_updates`);
//! the in-memory variant here re-runs the invariant under real OS-thread
//! parallelism so the two database backends have an in-process control.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use ironclaw_filesystem::{
    CasApply, CasExpectation, ContentType, Entry, FilesystemError, InMemoryBackend, RootFilesystem,
    ScopedFilesystem, cas_update,
};
use ironclaw_host_api::{
    MountAlias, MountGrant, MountPermissions, MountView, ResourceScope, ScopedPath, VirtualPath,
};

const WRITERS: u64 = 16;
const ITERATIONS: u64 = 100;
const DELETE_STORM_ROUNDS: u64 = 25;

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
struct Counter {
    value: u64,
}

#[derive(Debug)]
struct TestError(String);

impl std::fmt::Display for TestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

fn decode(bytes: &[u8]) -> Result<Counter, TestError> {
    serde_json::from_slice(bytes).map_err(|e| TestError(e.to_string()))
}

fn encode(counter: &Counter) -> Result<Entry, TestError> {
    let body = serde_json::to_vec(counter).map_err(|e| TestError(e.to_string()))?;
    Ok(Entry::bytes(body).with_content_type(ContentType::json()))
}

async fn increment(current: Option<Counter>) -> Result<CasApply<Counter, u64>, TestError> {
    let next = current.map(|c| c.value).unwrap_or(0) + 1;
    Ok(CasApply::new(Counter { value: next }, next))
}

/// Single-tenant view mapping `/counters` onto `target` (a unique
/// [`VirtualPath`] prefix, so shared databases stay isolated per run).
fn scoped<F: RootFilesystem>(root: Arc<F>, target: &str) -> ScopedFilesystem<F> {
    ScopedFilesystem::with_fixed_view(
        root,
        MountView::new(vec![MountGrant::new(
            MountAlias::new("/counters").unwrap(),
            VirtualPath::new(target).unwrap(),
            MountPermissions::read_write_list_delete(),
        )])
        .unwrap(),
    )
}

/// 100 rounds of 16 spawned writers, each performing one CAS increment against
/// the same snapshot. Keeping rounds explicit bounds each operation's competing
/// writes below `cas_update`'s retry cap while preserving real contention.
/// Every `cas_update` must succeed (no backend errors — defect 1 of
/// #5466) and the final counter must equal `WRITERS * ITERATIONS`
/// (no lost updates — defect 2).
async fn run_storm<F: RootFilesystem + 'static>(fs: Arc<ScopedFilesystem<F>>) {
    let scope = ResourceScope::system();
    let path = ScopedPath::new("/counters/state.json").unwrap();

    for _ in 0..ITERATIONS {
        let mut handles = Vec::new();
        for _ in 0..WRITERS {
            let fs = Arc::clone(&fs);
            let scope = scope.clone();
            let path = path.clone();
            handles.push(tokio::spawn(async move {
                cas_update(fs.as_ref(), &scope, &path, decode, encode, increment)
                    .await
                    .expect("concurrent cas_update must not fail");
            }));
        }
        for handle in handles {
            handle.await.expect("writer task must not panic");
        }
    }

    let stored = fs
        .get(&scope, &path)
        .await
        .expect("final read must succeed")
        .expect("counter must exist after the storm");
    let counter = decode(&stored.entry.body).unwrap();
    assert_eq!(
        counter.value,
        WRITERS * ITERATIONS,
        "every concurrent increment must land exactly once (no lost updates)"
    );
}

/// `WRITERS`-way contention on `delete_if_version` itself, repeated over
/// `DELETE_STORM_ROUNDS` recreate/delete cycles. Round-A review finding
/// (PR #5749): the earlier CAS storm only ever exercised `put`/`cas_update`
/// concurrency — `delete_if_version` (the operation this PR adds) had no
/// concurrency coverage at all. Each round recreates the shared path at a
/// known version, then `WRITERS` tasks race to `delete_if_version` it at
/// that exact version: CAS-delete atomicity guarantees exactly one winner
/// per round, and every loser must observe a well-formed `NotFound` (the
/// row it raced for is already gone) — never a backend/infrastructure
/// error, a panic, or more than one reported success, which would indicate
/// the pool exhausted/deadlocked or the delete lost its atomicity under
/// `WRITERS`-way contention.
///
/// Round-B review finding: this does NOT exercise (and cannot be used as
/// a regression pin for) the separate delete-then-recreate diagnosis race
/// the `BEGIN IMMEDIATE`/`FOR UPDATE` atomicity fix (commit 1792aebb2)
/// targets — every racer here shares one pre-fetched version and nothing
/// recreates the path mid-round, so ordinary single-row locking would
/// make this pass even against the pre-fix two-statement implementation.
/// That specific regression is pinned deterministically (no concurrency
/// needed) by `postgres.rs`'s `delete_if_version_statements_are_single_round_trip_and_single_key`
/// and `libsql.rs`'s `delete_if_version_diagnosis_reuses_the_delete_connection_under_a_size_one_pool`.
/// This storm test instead proves a different, still-necessary property:
/// `delete_if_version` behaves correctly and the pool doesn't
/// exhaust/deadlock under real `WRITERS`-way parallel contention.
async fn run_delete_storm<F: RootFilesystem + 'static>(fs: Arc<ScopedFilesystem<F>>) {
    let scope = ResourceScope::system();
    let path = ScopedPath::new("/counters/delete-storm.json").unwrap();

    for round in 0..DELETE_STORM_ROUNDS {
        let version = fs
            .put(&scope, &path, Entry::bytes(vec![1]), CasExpectation::Any)
            .await
            .expect("round setup put must succeed");
        let round_wins = Arc::new(AtomicU64::new(0));

        let mut handles = Vec::new();
        for _ in 0..WRITERS {
            let fs = Arc::clone(&fs);
            let scope = scope.clone();
            let path = path.clone();
            let round_wins = Arc::clone(&round_wins);
            handles.push(tokio::spawn(async move {
                match fs.delete_if_version(&scope, &path, version).await {
                    Ok(()) => {
                        round_wins.fetch_add(1, Ordering::SeqCst);
                    }
                    Err(FilesystemError::NotFound { .. }) => {}
                    Err(other) => panic!(
                        "concurrent delete_if_version must only ever return Ok or NotFound \
                         for racers on the same known version, got: {other:?}"
                    ),
                }
            }));
        }
        for handle in handles {
            handle.await.expect("delete-storm racer must not panic");
        }

        // Asserted per round (not just summed across all rounds): a 0-win
        // round and a 2-win round elsewhere could otherwise cancel out in
        // a total-only check and mask a real lost/duplicated delete.
        assert_eq!(
            round_wins.load(Ordering::SeqCst),
            1,
            "round {round}: exactly one delete_if_version racer must win \
             (no lost or duplicated deletes)"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn in_memory_concurrent_cas_storm_has_no_lost_updates() {
    let fs = Arc::new(scoped(Arc::new(InMemoryBackend::new()), "/engine/counters"));
    run_storm(fs).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn in_memory_concurrent_delete_if_version_storm_has_exactly_one_winner_per_round() {
    let fs = Arc::new(scoped(Arc::new(InMemoryBackend::new()), "/engine/counters"));
    run_delete_storm(fs).await;
}

#[tokio::test]
async fn cas_delete_returns_successful_mutation_outcome_after_verification() {
    let fs = scoped(Arc::new(InMemoryBackend::new()), "/engine/counters");
    let scope = ResourceScope::system();
    let path = ScopedPath::new("/counters/delete-outcome.json").unwrap();
    fs.put(
        &scope,
        &path,
        encode(&Counter { value: 1 }).unwrap(),
        CasExpectation::Absent,
    )
    .await
    .expect("seed counter");

    let outcome = cas_update(
        &fs,
        &scope,
        &path,
        decode,
        encode,
        |current: Option<Counter>| async move {
            Ok(match current {
                Some(snapshot) => CasApply::delete(snapshot, "delete-applied"),
                None => CasApply::no_op(Counter { value: 0 }, "verification-only"),
            })
        },
    )
    .await
    .expect("delete and post-delete verification");

    assert_eq!(
        outcome, "delete-applied",
        "post-delete verification must not replace the successful mutation outcome"
    );
    assert!(
        fs.get(&scope, &path)
            .await
            .expect("read deleted path")
            .is_none()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn libsql_concurrent_cas_storm_has_no_errors_or_lost_updates() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("cas-storm.db");
    let db = Arc::new(libsql::Builder::new_local(db_path).build().await.unwrap());
    let root = Arc::new(ironclaw_filesystem::LibSqlRootFilesystem::new(db));
    root.run_migrations().await.unwrap();
    let fs = Arc::new(scoped(root, "/engine/counters"));
    run_storm(fs).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn libsql_concurrent_delete_if_version_storm_has_exactly_one_winner_per_round() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("delete-cas-storm.db");
    let db = Arc::new(libsql::Builder::new_local(db_path).build().await.unwrap());
    let root = Arc::new(ironclaw_filesystem::LibSqlRootFilesystem::new(db));
    root.run_migrations().await.unwrap();
    let fs = Arc::new(scoped(root, "/engine/counters"));
    run_delete_storm(fs).await;
}

/// Connects to a locally-configured Postgres for the storm tests, or
/// returns `None` if the environment has none reachable/usable. Mirrors
/// `db_root_filesystem_contract.rs`'s skip-when-unreachable pattern so
/// environments without Postgres pass vacuously rather than failing CI on
/// infrastructure this test doesn't own.
async fn connect_postgres_for_storm() -> Option<Arc<ironclaw_filesystem::PostgresRootFilesystem>> {
    if std::env::var("IRONCLAW_SKIP_POSTGRES_TESTS").is_ok() {
        return None;
    }
    let url = std::env::var("IRONCLAW_FILESYSTEM_POSTGRES_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .ok()?;
    let config = url.parse::<tokio_postgres::Config>().ok()?;
    let manager = deadpool_postgres::Manager::new(config, tokio_postgres::NoTls);
    let pool = deadpool_postgres::Pool::builder(manager)
        .max_size(4)
        .build()
        .ok()?;
    let root = Arc::new(ironclaw_filesystem::PostgresRootFilesystem::new(pool));
    if root.run_migrations().await.is_err() {
        return None;
    }
    Some(root)
}
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn postgres_concurrent_cas_storm_has_no_errors_or_lost_updates() {
    let Some(root) = connect_postgres_for_storm().await else {
        return;
    };
    // Unique prefix per run: CAS storms against a shared database must
    // not contend with a previous run's leftover snapshot.
    let target = format!("/engine/cas_storm_{}", uuid::Uuid::new_v4().simple());
    let fs = Arc::new(scoped(root, &target));
    run_storm(fs).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn postgres_concurrent_delete_if_version_storm_has_exactly_one_winner_per_round() {
    let Some(root) = connect_postgres_for_storm().await else {
        return;
    };
    let target = format!("/engine/delete_cas_storm_{}", uuid::Uuid::new_v4().simple());
    let fs = Arc::new(scoped(root, &target));
    run_delete_storm(fs).await;
}
