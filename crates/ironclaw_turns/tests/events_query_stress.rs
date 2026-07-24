//! Stress/perf harness for the durable turn-event read path (#6382 follow-up).
//!
//! Measures the indexed `query` read path against a faithful replica of the
//! legacy directory scan on a real **libSQL** file database (the production
//! durable backend for the default `ironclaw serve`), where a real B-tree index
//! exists — unlike the in-memory backend, whose `query` is itself a linear scan
//! and so hides the difference.
//!
//! Ignored by default (it seeds thousands of rows and is timing-sensitive). Run
//! explicitly:
//!
//! ```bash
//! cargo test -p ironclaw_turns --test events_query_stress -- --ignored --nocapture
//! # tune the population:
//! STRESS_THREADS=80 STRESS_EVENTS_PER_THREAD=200 \
//!   cargo test -p ironclaw_turns --test events_query_stress -- --ignored --nocapture
//! ```
//!
//! The scenario is the one the change targets: many threads' events interleaved
//! across the global cursor space, then reading a single thread's timeline. The
//! legacy scan must list + read every row after the cursor across *all* threads
//! to return one thread's slice; the indexed query reads only that slice.

use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::stream::{self, StreamExt, TryStreamExt};
use ironclaw_filesystem::{
    CasExpectation, ContentType, Entry, FileType, LibSqlRootFilesystem, PostgresRootFilesystem,
    RootFilesystem, ScopedFilesystem,
};
use ironclaw_host_api::{
    AgentId, MountAlias, MountGrant, MountPermissions, MountView, ProjectId, ResourceScope,
    ScopedPath, TenantId, ThreadId, UserId, VirtualPath,
};
use ironclaw_turns::{
    EventCursor, FilesystemTurnStateRowStore, TurnEventKind, TurnEventProjectionSource,
    TurnLifecycleEvent, TurnRunId, TurnScope, TurnStatus,
};

const EVENTS_DIR: &str = "/turns/rows/v1/events";
/// Matches `ROW_COLLECTION_READ_CONCURRENCY` in the production scan so the
/// baseline is a fair replica, not an artificially sequential one.
const SCAN_READ_CONCURRENCY: usize = 32;

fn env_usize(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn thread_scope(index: usize) -> TurnScope {
    TurnScope::new(
        TenantId::new("stress-tenant").unwrap(),
        Some(AgentId::new("stress-agent").unwrap()),
        Some(ProjectId::new("stress-project").unwrap()),
        ThreadId::new(format!("thread-{index:04}")).unwrap(),
    )
}

fn event(scope: &TurnScope, cursor: u64, run_id: TurnRunId) -> TurnLifecycleEvent {
    TurnLifecycleEvent {
        cursor: EventCursor(cursor),
        scope: scope.clone(),
        occurred_at: None,
        owner_user_id: Some(UserId::new("stress-owner").unwrap()),
        run_id,
        status: TurnStatus::Queued,
        kind: TurnEventKind::Submitted,
        blocked_gate: None,
        sanitized_reason: None,
        retryable: None,
        detail: None,
    }
}

fn events_row_path(cursor: u64) -> ScopedPath {
    ScopedPath::new(format!("{EVENTS_DIR}/{cursor:020}.json")).unwrap()
}

async fn build_libsql_scoped() -> (
    tempfile::TempDir,
    Arc<ScopedFilesystem<LibSqlRootFilesystem>>,
) {
    let dir = tempfile::tempdir().unwrap();
    let db = Arc::new(
        libsql::Builder::new_local(dir.path().join("turns-stress.db"))
            .build()
            .await
            .unwrap(),
    );
    let root = Arc::new(LibSqlRootFilesystem::new(db));
    root.run_migrations().await.unwrap();
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/turns").unwrap(),
        VirtualPath::new("/turns").unwrap(),
        MountPermissions::read_write_list_delete(),
    )])
    .unwrap();
    (
        dir,
        Arc::new(ScopedFilesystem::with_fixed_view(root, mounts)),
    )
}

async fn build_postgres_scoped() -> Option<Arc<ScopedFilesystem<PostgresRootFilesystem>>> {
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
    let root = Arc::new(PostgresRootFilesystem::new(pool));
    root.run_migrations().await.ok()?;
    let unique_root = format!("/turn-replay-test/{}", uuid::Uuid::new_v4().simple());
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/turns").ok()?,
        VirtualPath::new(unique_root).ok()?,
        MountPermissions::read_write_list_delete(),
    )])
    .ok()?;
    Some(Arc::new(ScopedFilesystem::with_fixed_view(root, mounts)))
}

/// Seed `threads * events_per_thread` event rows as bare (unprojected) bodies —
/// the pre-upgrade on-disk shape — interleaved across the global cursor space so
/// each thread's events are scattered. Cursor `c` belongs to thread `c % threads`.
/// The store's backfill re-projects them via production logic on first read.
async fn seed<F>(scoped: &ScopedFilesystem<F>, threads: usize, events_per_thread: usize) -> usize
where
    F: RootFilesystem,
{
    let total = threads * events_per_thread;
    for cursor in 1..=total as u64 {
        let scope = thread_scope((cursor as usize - 1) % threads);
        let entry =
            Entry::bytes(serde_json::to_vec(&event(&scope, cursor, TurnRunId::new())).unwrap())
                .with_content_type(ContentType::json());
        scoped
            .put(
                &ResourceScope::system(),
                &events_row_path(cursor),
                entry,
                CasExpectation::Absent,
            )
            .await
            .unwrap();
    }
    total
}

/// Faithful replica of the legacy `read_row_collection_filtered(Events, key > after)`
/// scan: list the whole events collection, filter names by cursor, read every
/// matching body (buffered at the production concurrency), deserialize.
async fn legacy_scan_read(
    scoped: &ScopedFilesystem<LibSqlRootFilesystem>,
    after: Option<EventCursor>,
) -> usize {
    let after_key = after.map(|cursor| format!("{:020}", cursor.0));
    let dir = ScopedPath::new(EVENTS_DIR).unwrap();
    let entries = scoped
        .list_dir(&ResourceScope::system(), &dir)
        .await
        .unwrap();
    let paths: Vec<ScopedPath> = entries
        .into_iter()
        .filter(|entry| entry.file_type == FileType::File)
        .filter_map(|entry| entry.name.strip_suffix(".json").map(ToString::to_string))
        .filter(|key| {
            after_key
                .as_ref()
                .is_none_or(|after| key.as_str() > after.as_str())
        })
        .map(|key| ScopedPath::new(format!("{EVENTS_DIR}/{key}.json")).unwrap())
        .collect();
    let rows: Vec<TurnLifecycleEvent> = stream::iter(paths)
        .map(|path| async move {
            let versioned = scoped
                .get(&ResourceScope::system(), &path)
                .await?
                .expect("row present");
            Ok::<_, ironclaw_filesystem::FilesystemError>(
                serde_json::from_slice::<TurnLifecycleEvent>(&versioned.entry.body).unwrap(),
            )
        })
        .buffer_unordered(SCAN_READ_CONCURRENCY)
        .try_collect()
        .await
        .unwrap();
    rows.len()
}

async fn median_of<F, Fut>(iterations: usize, mut op: F) -> Duration
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let mut samples = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let started = Instant::now();
        op().await;
        samples.push(started.elapsed());
    }
    samples.sort();
    samples[samples.len() / 2]
}

#[tokio::test]
async fn durable_event_log_replay_pages_by_global_cursor_on_libsql() {
    let (_dir, scoped) = build_libsql_scoped().await;
    seed(&scoped, 2, 3).await;
    let store = FilesystemTurnStateRowStore::new(scoped);

    let first = store
        .read_turn_event_log_after(None, 2)
        .await
        .expect("first bounded replay page");
    assert_eq!(
        first
            .entries
            .iter()
            .map(|event| event.cursor)
            .collect::<Vec<_>>(),
        vec![EventCursor(1), EventCursor(2)]
    );
    assert!(first.truncated);
    assert_eq!(first.rebase_required, None);

    let second = store
        .read_turn_event_log_after(Some(first.next_cursor), 2)
        .await
        .expect("second bounded replay page");
    assert_eq!(
        second
            .entries
            .iter()
            .map(|event| event.cursor)
            .collect::<Vec<_>>(),
        vec![EventCursor(3), EventCursor(4)]
    );
    assert!(second.truncated);
}

#[tokio::test]
async fn durable_event_log_replay_pages_by_global_cursor_on_postgres() {
    let Some(scoped) = build_postgres_scoped().await else {
        return;
    };
    seed(&scoped, 2, 3).await;
    let store = FilesystemTurnStateRowStore::new(scoped);

    let first = store
        .read_turn_event_log_after(None, 2)
        .await
        .expect("first bounded Postgres replay page");
    assert_eq!(
        first
            .entries
            .iter()
            .map(|event| event.cursor)
            .collect::<Vec<_>>(),
        vec![EventCursor(1), EventCursor(2)]
    );
    assert!(first.truncated);
    let second = store
        .read_turn_event_log_after(Some(first.next_cursor), 2)
        .await
        .expect("second bounded Postgres replay page");
    assert_eq!(
        second
            .entries
            .iter()
            .map(|event| event.cursor)
            .collect::<Vec<_>>(),
        vec![EventCursor(3), EventCursor(4)]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "perf stress harness; run explicitly with --ignored --nocapture"]
async fn events_query_vs_scan_perf() {
    let threads = env_usize("STRESS_THREADS", 40);
    let events_per_thread = env_usize("STRESS_EVENTS_PER_THREAD", 100);
    let iterations = env_usize("STRESS_ITERATIONS", 20);

    let (_dir, scoped) = build_libsql_scoped().await;

    let seed_started = Instant::now();
    let total = seed(&scoped, threads, events_per_thread).await;
    let seed_elapsed = seed_started.elapsed();

    let store = FilesystemTurnStateRowStore::new(Arc::clone(&scoped));
    let read_scope = thread_scope(0);

    // First read triggers ensure_index + the one-time backfill of all seeded
    // rows. Time it separately — it is the migration cost, paid once.
    let backfill_started = Instant::now();
    let first = store
        .read_turn_events_after(&read_scope, None, None, 100)
        .await
        .unwrap();
    let backfill_elapsed = backfill_started.elapsed();
    assert!(
        !first.entries.is_empty(),
        "backfill must make the seeded rows queryable"
    );

    // Steady-state: read one thread's timeline from origin.
    let query_from_origin = median_of(iterations, || async {
        let page = store
            .read_turn_events_after(&read_scope, None, None, 100)
            .await
            .unwrap();
        std::hint::black_box(page.entries.len());
    })
    .await;

    // Steady-state: a caught-up client (cursor near the newest of its thread).
    let newest = first.entries.iter().map(|e| e.cursor).max().unwrap();
    let query_caught_up = median_of(iterations, || async {
        let page = store
            .read_turn_events_after(&read_scope, None, Some(newest), 100)
            .await
            .unwrap();
        std::hint::black_box(page.entries.len());
    })
    .await;

    // Baseline: the legacy directory scan over the same data.
    let scan_from_origin = median_of(iterations, || async {
        std::hint::black_box(legacy_scan_read(&scoped, None).await);
    })
    .await;
    let scan_caught_up = median_of(iterations, || async {
        std::hint::black_box(legacy_scan_read(&scoped, Some(newest)).await);
    })
    .await;

    let speedup = |scan: Duration, query: Duration| {
        scan.as_secs_f64() / query.as_secs_f64().max(f64::MIN_POSITIVE)
    };

    println!("\n=== turn-event durable read: query vs legacy scan (libSQL) ===");
    println!(
        "population: {threads} threads x {events_per_thread} events = {total} rows; \
         one thread = {} events; {iterations} iterations (median)",
        first.entries.len()
    );
    println!("seed time:        {seed_elapsed:?}");
    println!("backfill (1x):    {backfill_elapsed:?}  ({total} rows re-projected)");
    println!("-- read one thread's timeline --");
    println!(
        "from origin:   scan {scan_from_origin:?}  |  query {query_from_origin:?}  |  {:.1}x faster",
        speedup(scan_from_origin, query_from_origin)
    );
    println!(
        "caught up:     scan {scan_caught_up:?}  |  query {query_caught_up:?}  |  {:.1}x faster",
        speedup(scan_caught_up, query_caught_up)
    );
    println!("================================================================\n");

    // Guard against a regression that would silently defeat the index: at this
    // population the indexed query must not be slower than the full scan.
    assert!(
        query_from_origin <= scan_from_origin,
        "indexed query ({query_from_origin:?}) must not be slower than the full scan \
         ({scan_from_origin:?}) at {total} rows"
    );
}
