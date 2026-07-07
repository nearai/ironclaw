//! Concurrent CAS-storm regression tests (#5466).
//!
//! Drives many genuinely parallel `cas_update` read-modify-write loops —
//! multi-threaded tokio runtime, `tokio::spawn` per writer — against one
//! shared snapshot path, per backend. The libSQL variant is the
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

use ironclaw_filesystem::{
    CasApply, ContentType, Entry, InMemoryBackend, RootFilesystem, ScopedFilesystem, cas_update,
};
use ironclaw_host_api::{
    MountAlias, MountGrant, MountPermissions, MountView, ResourceScope, ScopedPath, VirtualPath,
};

const WRITERS: u64 = 16;
const ITERATIONS: u64 = 100;

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

/// 16 spawned writers x 100 CAS increments each against one snapshot.
/// Every `cas_update` must succeed (no backend errors — defect 1 of
/// #5466) and the final counter must equal `WRITERS * ITERATIONS`
/// (no lost updates — defect 2).
async fn run_storm<F: RootFilesystem + 'static>(fs: Arc<ScopedFilesystem<F>>) {
    let scope = ResourceScope::system();
    let path = ScopedPath::new("/counters/state.json").unwrap();

    let mut handles = Vec::new();
    for _ in 0..WRITERS {
        let fs = Arc::clone(&fs);
        let scope = scope.clone();
        let path = path.clone();
        handles.push(tokio::spawn(async move {
            for _ in 0..ITERATIONS {
                cas_update(fs.as_ref(), &scope, &path, decode, encode, increment)
                    .await
                    .expect("concurrent cas_update must not fail");
            }
        }));
    }
    for handle in handles {
        handle.await.expect("writer task must not panic");
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

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn in_memory_concurrent_cas_storm_has_no_lost_updates() {
    let fs = Arc::new(scoped(Arc::new(InMemoryBackend::new()), "/engine/counters"));
    run_storm(fs).await;
}

#[cfg(feature = "libsql")]
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

#[cfg(feature = "postgres")]
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
async fn postgres_concurrent_cas_storm_has_no_errors_or_lost_updates() {
    // Mirrors `db_root_filesystem_contract.rs`'s skip-when-unreachable
    // pattern so environments without Postgres pass vacuously.
    if std::env::var("IRONCLAW_SKIP_POSTGRES_TESTS").is_ok() {
        return;
    }
    // silent-ok: no Postgres URL configured means the environment has no
    // Postgres to test against; skip vacuously rather than fail CI.
    let Ok(url) = std::env::var("IRONCLAW_FILESYSTEM_POSTGRES_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
    else {
        return;
    };
    // silent-ok: an unparsable URL means the environment's Postgres config
    // isn't usable here; skip rather than fail on a config format issue
    // this test doesn't own.
    let Ok(config) = url.parse::<tokio_postgres::Config>() else {
        return;
    };
    let manager = deadpool_postgres::Manager::new(config, tokio_postgres::NoTls);
    // silent-ok: pool construction failure means the environment can't
    // stand up a Postgres pool right now; skip rather than fail this
    // storm test on infrastructure it doesn't own.
    let Ok(pool) = deadpool_postgres::Pool::builder(manager)
        .max_size(4)
        .build()
    else {
        return;
    };
    let root = Arc::new(ironclaw_filesystem::PostgresRootFilesystem::new(pool));
    // silent-ok: an unreachable/misconfigured Postgres fails migrations;
    // skip rather than fail this storm test on connectivity it doesn't own.
    if root.run_migrations().await.is_err() {
        return;
    }
    // Unique prefix per run: CAS storms against a shared database must
    // not contend with a previous run's leftover snapshot.
    let target = format!("/engine/cas_storm_{}", uuid::Uuid::new_v4().simple());
    let fs = Arc::new(scoped(root, &target));
    run_storm(fs).await;
}
