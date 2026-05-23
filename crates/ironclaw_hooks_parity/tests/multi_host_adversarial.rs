//! Cross-backend multi-host adversarial suite (durable-backend PR 4/4).
//!
//! These tests exercise the cross-host correctness properties that ONLY the
//! durable backends provide (the in-memory backend's dedup is process-local and
//! cannot defend against multi-host replay — closing the A3 deferral from
//! #3635). Each "host" is a distinct backend instance pointing at the SAME
//! database (a shared libSQL temp-file, or distinct `deadpool` pools over one
//! Postgres), simulating distinct processes.
//!
//! # Gating
//!
//! The whole binary is behind `--features integration` so default `cargo test`
//! stays fast.
//!
//! - **libSQL** legs run unconditionally under `integration` — the backend uses
//!   an embedded temp-file db with a shared `Database` handle across "hosts",
//!   so real concurrent multi-host behaviour is exercised with no server.
//! - **Postgres** legs additionally require `--features postgres` AND a
//!   reachable server via `IRONCLAW_HOOKS_POSTGRES_URL` / `DATABASE_URL`;
//!   skipped (passing) otherwise.
//!
//! Scenarios (all asserted identical-in-spirit across both durable backends):
//! 1. N concurrent writers across 2 hosts — no count desync, exactly-once.
//! 2. Cross-host replay — interleaved id submissions, exactly-once counting.
//! 3. LRU eviction race — concurrent inserts past the per-tenant quota.
//! 4. Per-key cap under attacker flood — fail-closed `WindowOverflow`, bounded.
//!    Also (4b) a cap-boundary race: fill to cap-1, race two fresh ids; exactly
//!    one wins the last slot, the other fails closed (no TOCTOU breach or
//!    double-reject).
//! 5. Clock-skew — two hosts pass different `DateTime<Utc>`; window follows the
//!    caller-supplied clock basis (both durable backends chose caller `now`).

#![cfg(feature = "integration")]

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use ironclaw_hooks::identity::{ExtensionId, HookId, HookLocalId, HookVersion};
use ironclaw_hooks::predicate_state::{
    InvocationKey, MAX_KEYS_PER_TENANT, MAX_SAMPLES_PER_KEY, PredicateBackendError,
    PredicateEventId, PredicateStateBackend, ValueKey,
};
use ironclaw_host_api::TenantId;
use rust_decimal::Decimal;

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn hook_id() -> HookId {
    HookId::derive(
        &ExtensionId::new("ext").expect("ext id"),
        "1.0",
        &HookLocalId::new("h").expect("hook local id"),
        HookVersion::ONE,
    )
}

fn tenant(name: &str) -> TenantId {
    TenantId::new(name).expect("tenant id")
}

fn ev(s: &str) -> PredicateEventId {
    PredicateEventId::new(s).expect("event id")
}

fn base() -> DateTime<Utc> {
    DateTime::from_timestamp(1_700_000_000, 0).expect("fixed timestamp")
}

fn at_secs(secs: i64) -> DateTime<Utc> {
    base() + chrono::Duration::seconds(secs)
}

fn at_millis(ms: i64) -> DateTime<Utc> {
    base() + chrono::Duration::milliseconds(ms)
}

fn inv_key(tenant_name: &str, capability: &str) -> InvocationKey {
    InvocationKey {
        hook_id: hook_id(),
        tenant_id: tenant(tenant_name),
        capability: capability.to_string(),
    }
}

fn val_key(tenant_name: &str, capability: &str, field: &str) -> ValueKey {
    ValueKey {
        hook_id: hook_id(),
        tenant_id: tenant(tenant_name),
        capability: capability.to_string(),
        field: field.to_string(),
    }
}

// ---------------------------------------------------------------------------
// libSQL "cluster": a set of backend instances over one shared temp-file db.
// ---------------------------------------------------------------------------

mod libsql_cluster {
    use super::*;
    use ironclaw_hooks_libsql::LibSqlPredicateStateBackend;
    use tempfile::TempDir;

    /// A shared libSQL database plus its temp-dir guard. Each "host" is a
    /// separate [`LibSqlPredicateStateBackend`] built over `db.clone()`.
    pub struct Cluster {
        pub db: Arc<libsql::Database>,
        _dir: TempDir,
    }

    impl Cluster {
        pub async fn new() -> Self {
            let dir = tempfile::tempdir().expect("tempdir");
            let path = dir.path().join("cluster.db");
            let db = Arc::new(
                libsql::Builder::new_local(path.to_string_lossy().to_string())
                    .build()
                    .await
                    .expect("build libsql db"),
            );
            // Migrate once via a throwaway host.
            LibSqlPredicateStateBackend::new(db.clone())
                .run_migrations()
                .await
                .expect("migrate");
            Self { db, _dir: dir }
        }

        pub fn host(&self) -> Arc<dyn PredicateStateBackend> {
            Arc::new(LibSqlPredicateStateBackend::new(self.db.clone()))
        }

        /// Count distinct scopes recorded for `tenant` in the invocation table —
        /// used to assert the per-tenant LRU quota held under flood.
        pub async fn distinct_invocation_scopes(&self, tenant: &str) -> usize {
            let conn = self.db.connect().expect("connect");
            let mut rows = conn
                .query(
                    "SELECT count(DISTINCT scope_hash) FROM hooks_predicate_invocations \
                     WHERE tenant_id = ?1",
                    libsql::params![tenant],
                )
                .await
                .expect("query");
            let row = rows.next().await.expect("row").expect("some");
            let v: i64 = row.get(0).expect("i64");
            v.max(0) as usize
        }
    }
}

// ---------------------------------------------------------------------------
// Scenario implementations — generic over a "cluster" via closures that hand
// back fresh host handles + an optional distinct-scope counter. We run them
// against the libSQL cluster always, and (under `postgres`) the Postgres one.
// ---------------------------------------------------------------------------

/// 1. N concurrent writers across 2 hosts hammer one key with DISTINCT ids;
///    every write must be counted exactly once (no lost-update desync).
async fn scenario_concurrent_writers_no_desync(
    host_a: Arc<dyn PredicateStateBackend>,
    host_b: Arc<dyn PredicateStateBackend>,
    observer: Arc<dyn PredicateStateBackend>,
) {
    let key = inv_key("alpha", "cap.concurrent");
    let now = at_secs(0);
    let window = Duration::from_secs(60);
    const N: usize = 48;

    let mut handles = Vec::with_capacity(N);
    for i in 0..N {
        let backend = if i % 2 == 0 {
            Arc::clone(&host_a)
        } else {
            Arc::clone(&host_b)
        };
        let key = key.clone();
        handles.push(tokio::spawn(async move {
            backend
                .record_invocation(&key, &ev(&format!("evt-{i}")), now, window)
                .await
                .expect("record ok")
        }));
    }

    // Collect EVERY writer's returned count. The evaluator gates on this
    // returned value, so a backend that durably inserts all N rows but returns
    // stale counts mid-race (e.g. reads its own pre-commit snapshot) would be a
    // real bug even though the final row count is correct. For N distinct-id
    // writers, the N returned counts must be exactly the set {1, 2, ..., N}:
    // each writer observes a strictly different in-window count and no two see
    // the same value (no lost-update where two writers both return the same k).
    let mut counts: Vec<u32> = Vec::with_capacity(N);
    for h in handles {
        counts.push(h.await.expect("joined"));
    }
    counts.sort_unstable();
    let expected: Vec<u32> = (1..=N as u32).collect();
    assert_eq!(
        counts, expected,
        "the N concurrent distinct-id writers must each return a distinct \
         in-window count forming exactly 1..=N (no duplicate/stale counts \
         mid-race), got {counts:?}"
    );

    // Observer re-reads via a duplicate id (no-op insert) — sees the full count.
    let final_count = observer
        .record_invocation(&key, &ev("evt-0"), now, window)
        .await
        .expect("read ok");
    assert_eq!(
        final_count as usize, N,
        "two hosts writing N distinct ids concurrently must be counted exactly once each"
    );
}

/// 2. Cross-host replay: interleaved id submissions from two hosts; a replayed
///    id is a no-op against the count (durable PK dedups across hosts).
async fn scenario_cross_host_replay_exactly_once(
    host_a: Arc<dyn PredicateStateBackend>,
    host_b: Arc<dyn PredicateStateBackend>,
) {
    let key = inv_key("alpha", "cap.replay");
    let window = Duration::from_secs(60);

    let c1 = host_a
        .record_invocation(&key, &ev("shared-evt"), at_secs(0), window)
        .await
        .expect("ok");
    assert_eq!(c1, 1);
    // Host B replays the same id — must not increment.
    let c2 = host_b
        .record_invocation(&key, &ev("shared-evt"), at_secs(1), window)
        .await
        .expect("ok");
    assert_eq!(c2, 1, "cross-host replay must not double-count");
    // Distinct id on B advances.
    let c3 = host_b
        .record_invocation(&key, &ev("fresh-evt"), at_secs(2), window)
        .await
        .expect("ok");
    assert_eq!(c3, 2);

    // Value path too.
    let vkey = val_key("alpha", "cap.spend", "amount");
    let s1 = host_a
        .record_value(
            &vkey,
            &ev("v-shared"),
            at_secs(0),
            Decimal::from(50),
            window,
        )
        .await
        .expect("ok");
    assert_eq!(s1, Decimal::from(50));
    let s2 = host_b
        .record_value(
            &vkey,
            &ev("v-shared"),
            at_secs(1),
            Decimal::from(50),
            window,
        )
        .await
        .expect("ok");
    assert_eq!(
        s2,
        Decimal::from(50),
        "cross-host value replay must not double-count the sum"
    );
}

/// 4. Per-key cap under attacker flood from concurrent hosts — fail-closed
///    `WindowOverflow` at the cap, bounded (never exceeds the cap). We fill to
///    the cap serially (deterministic), then have BOTH hosts race a fresh
///    in-window id past the cap; every such attempt must fail closed.
async fn scenario_per_key_cap_fails_closed_under_flood(
    host_a: Arc<dyn PredicateStateBackend>,
    host_b: Arc<dyn PredicateStateBackend>,
) {
    let key = inv_key("alpha", "cap.hot");
    let window = Duration::from_secs(3600);

    for i in 0..MAX_SAMPLES_PER_KEY {
        host_a
            .record_invocation(&key, &ev(&format!("e-{i}")), at_millis(i as i64), window)
            .await
            .expect("inserts up to the cap succeed");
    }

    // Both hosts flood fresh in-window ids past the cap concurrently.
    let mut handles = Vec::new();
    for h in [Arc::clone(&host_a), Arc::clone(&host_b)] {
        for j in 0..8 {
            let h = Arc::clone(&h);
            let key = key.clone();
            handles.push(tokio::spawn(async move {
                h.record_invocation(
                    &key,
                    &ev(&format!("flood-{j}-{:p}", Arc::as_ptr(&h))),
                    at_millis(MAX_SAMPLES_PER_KEY as i64 + j),
                    window,
                )
                .await
            }));
        }
    }
    for handle in handles {
        let res = handle.await.expect("joined");
        assert!(
            matches!(res, Err(PredicateBackendError::WindowOverflow { .. })),
            "flood past the per-key cap must fail closed, got {res:?}"
        );
    }

    // A replay of an in-window id still dedups to a no-op at the cap.
    let replay = host_b
        .record_invocation(
            &key,
            &ev("e-0"),
            at_millis(MAX_SAMPLES_PER_KEY as i64 + 100),
            window,
        )
        .await
        .expect("replay of an in-window id must dedup, not overflow");
    assert_eq!(
        replay as usize, MAX_SAMPLES_PER_KEY,
        "count stays bounded at the cap under flood"
    );
}

/// 4b. Cap-boundary race: fill to exactly `MAX_SAMPLES_PER_KEY - 1` (one slot
///     left), then race TWO fresh DISTINCT ids from two hosts at the same
///     instant. Exactly one must win the last slot (returning the cap as its
///     count) and the other must fail closed with `WindowOverflow`. This is the
///     race the bulk-flood scenario can't isolate: at the very boundary a
///     backend with a check-then-insert TOCTOU could let BOTH writers in
///     (count = cap + 1, cap breached) or reject BOTH (fail-closed when a slot
///     was actually free). The atomic record-and-cap path must admit exactly
///     one.
async fn scenario_cap_boundary_race_admits_exactly_one(
    host_a: Arc<dyn PredicateStateBackend>,
    host_b: Arc<dyn PredicateStateBackend>,
    observer: Arc<dyn PredicateStateBackend>,
) {
    let key = inv_key("alpha", "cap.boundary-race");
    let window = Duration::from_secs(3600);

    // Fill to MAX_SAMPLES_PER_KEY - 1 distinct ids: one free slot remains.
    for i in 0..(MAX_SAMPLES_PER_KEY - 1) {
        host_a
            .record_invocation(&key, &ev(&format!("fill-{i}")), at_millis(i as i64), window)
            .await
            .expect("fill below the cap succeeds");
    }

    // Race two FRESH distinct ids from the two hosts for the single free slot,
    // both at the same in-window instant.
    let now = at_millis(MAX_SAMPLES_PER_KEY as i64);
    let ha = {
        let key = key.clone();
        let host_a = Arc::clone(&host_a);
        tokio::spawn(async move {
            host_a
                .record_invocation(&key, &ev("race-a"), now, window)
                .await
        })
    };
    let hb = {
        let key = key.clone();
        let host_b = Arc::clone(&host_b);
        tokio::spawn(async move {
            host_b
                .record_invocation(&key, &ev("race-b"), now, window)
                .await
        })
    };
    let ra = ha.await.expect("joined a");
    let rb = hb.await.expect("joined b");

    // Exactly one Ok (at the cap) and exactly one WindowOverflow.
    let results = [&ra, &rb];
    let oks: Vec<u32> = results
        .iter()
        .filter_map(|r| r.as_ref().ok().copied())
        .collect();
    let overflows = results
        .iter()
        .filter(|r| matches!(r, Err(PredicateBackendError::WindowOverflow { .. })))
        .count();
    assert_eq!(
        oks.len(),
        1,
        "exactly one writer must win the last slot at the cap boundary; got ra={ra:?}, rb={rb:?}"
    );
    assert_eq!(
        overflows, 1,
        "the losing writer must fail closed with WindowOverflow; got ra={ra:?}, rb={rb:?}"
    );
    assert_eq!(
        oks[0] as usize, MAX_SAMPLES_PER_KEY,
        "the winning writer's count must be exactly the cap (no breach, no slot left empty)"
    );

    // Observer confirms the bucket is exactly at the cap and stays fail-closed:
    // any further fresh distinct id overflows.
    let overflow_again = observer
        .record_invocation(
            &key,
            &ev("post-race-fresh"),
            at_millis(MAX_SAMPLES_PER_KEY as i64 + 1),
            window,
        )
        .await;
    assert!(
        matches!(
            overflow_again,
            Err(PredicateBackendError::WindowOverflow { .. })
        ),
        "after the boundary race the key is at the cap; a further fresh id must fail closed, \
         got {overflow_again:?}"
    );
}

/// 5. Clock-skew: two hosts pass DIFFERENT `now` values for the same key. The
///    window follows the caller-supplied clock basis (both durable backends use
///    caller `now`, NOT a server clock). Host A records at t=0; host B records a
///    distinct id at t=0 but with a window that, measured from B's later `now`,
///    would have trimmed A's entry — confirming each call trims against the
///    `now` IT was given, and a far-future `now` from one host trims earlier
///    entries deterministically regardless of which host observes.
async fn scenario_clock_skew_follows_caller_clock(
    host_a: Arc<dyn PredicateStateBackend>,
    host_b: Arc<dyn PredicateStateBackend>,
) {
    let key = inv_key("alpha", "cap.skew");
    let window = Duration::from_secs(60);

    // Host A (clock at t=0) records.
    let c1 = host_a
        .record_invocation(&key, &ev("a-0"), at_secs(0), window)
        .await
        .expect("ok");
    assert_eq!(c1, 1);

    // Host B's clock is skewed far ahead (t=10_000s). Its window cutoff is
    // measured from ITS `now`, so A's t=0 entry is outside the 60s window and
    // gets trimmed — the resulting count is 1 (just B's new entry), proving the
    // window basis is the caller-supplied `now`, not the earliest entry or a
    // server clock.
    let c2 = host_b
        .record_invocation(&key, &ev("b-skew"), at_secs(10_000), window)
        .await
        .expect("ok");
    assert_eq!(
        c2, 1,
        "skewed-ahead host trims the earlier entry via its own caller-supplied now"
    );
}

// ---------------------------------------------------------------------------
// libSQL drivers (always run under `integration`)
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn libsql_concurrent_writers_no_desync() {
    let cluster = libsql_cluster::Cluster::new().await;
    scenario_concurrent_writers_no_desync(cluster.host(), cluster.host(), cluster.host()).await;
}

#[tokio::test]
async fn libsql_cross_host_replay_exactly_once() {
    let cluster = libsql_cluster::Cluster::new().await;
    scenario_cross_host_replay_exactly_once(cluster.host(), cluster.host()).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn libsql_lru_eviction_race_holds_quota() {
    let cluster = libsql_cluster::Cluster::new().await;
    let backend = cluster.host();
    let window = Duration::from_secs(3600);

    // Quiet tenant beta records one scope.
    let beta = inv_key("beta", "beta.cap");
    backend
        .record_invocation(&beta, &ev("beta-evt"), at_secs(0), window)
        .await
        .expect("ok");

    // Noisy tenant alpha floods past its quota concurrently. Bound concurrency
    // (libSQL opens a connection per op; unbounded fan-out exhausts FDs).
    let flood = MAX_KEYS_PER_TENANT + 16;
    let sem = Arc::new(tokio::sync::Semaphore::new(16));
    let mut handles = Vec::with_capacity(flood);
    for i in 0..flood {
        let backend = Arc::clone(&backend);
        let sem = Arc::clone(&sem);
        handles.push(tokio::spawn(async move {
            let _permit = sem.acquire_owned().await.expect("permit");
            let key = inv_key("alpha", &format!("alpha.cap.{i}"));
            backend
                .record_invocation(
                    &key,
                    &ev(&format!("a-{i}")),
                    at_millis(i as i64 + 1),
                    window,
                )
                .await
                .expect("ok");
        }));
    }
    for h in handles {
        h.await.expect("joined");
    }

    // Quiet tenant beta survives.
    let beta_count = backend
        .record_invocation(&beta, &ev("beta-evt"), at_secs(0), window)
        .await
        .expect("ok");
    assert_eq!(beta_count, 1, "quiet tenant scope must survive the flood");

    // Alpha held at quota; evictions advanced deterministically.
    let alpha_scopes = cluster.distinct_invocation_scopes("alpha").await;
    assert!(
        alpha_scopes <= MAX_KEYS_PER_TENANT,
        "noisy tenant capped at its quota; got {alpha_scopes}"
    );
    assert!(
        backend.evictions_observed() >= 1,
        "eviction counter must advance under the flood"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn libsql_per_key_cap_fails_closed_under_flood() {
    let cluster = libsql_cluster::Cluster::new().await;
    scenario_per_key_cap_fails_closed_under_flood(cluster.host(), cluster.host()).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn libsql_cap_boundary_race_admits_exactly_one() {
    let cluster = libsql_cluster::Cluster::new().await;
    scenario_cap_boundary_race_admits_exactly_one(cluster.host(), cluster.host(), cluster.host())
        .await;
}

#[tokio::test]
async fn libsql_clock_skew_follows_caller_clock() {
    let cluster = libsql_cluster::Cluster::new().await;
    scenario_clock_skew_follows_caller_clock(cluster.host(), cluster.host()).await;
}

// ---------------------------------------------------------------------------
// Postgres drivers (compiled under `postgres`, run with a DB URL)
// ---------------------------------------------------------------------------

#[cfg(feature = "postgres")]
mod postgres_cluster {
    use super::*;
    use deadpool_postgres::Pool;
    use ironclaw_hooks_postgres::PostgresPredicateStateBackend;

    /// Process-global serialization: the Postgres legs share fixed keys against
    /// one table, so they must not interleave with each other.
    static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    const SCHEMA: &str = "hooks_parity_multihost";

    fn db_url() -> Option<String> {
        std::env::var("IRONCLAW_HOOKS_POSTGRES_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .ok()
    }

    fn build_pool(url: &str) -> Option<Pool> {
        let config = url.parse::<tokio_postgres::Config>().ok()?;
        let manager = deadpool_postgres::Manager::new(config, tokio_postgres::NoTls);
        deadpool_postgres::Pool::builder(manager)
            .max_size(16)
            .post_create(deadpool_postgres::Hook::async_fn(|client, _| {
                Box::pin(async move {
                    client
                        .batch_execute(&format!("SET search_path TO {SCHEMA}"))
                        .await
                        .map_err(|e| deadpool_postgres::HookError::message(e.to_string()))?;
                    Ok(())
                })
            }))
            .build()
            .ok()
    }

    /// Ensure schema + migrated table, truncate, and return a fresh pool. `None`
    /// if no DB URL.
    async fn prepare() -> Option<String> {
        let url = db_url()?;
        let (client, conn) = tokio_postgres::connect(&url, tokio_postgres::NoTls)
            .await
            .ok()?;
        tokio::spawn(conn);
        client
            .batch_execute(&format!("CREATE SCHEMA IF NOT EXISTS {SCHEMA}"))
            .await
            .ok()?;
        let pool = build_pool(&url)?;
        let backend = PostgresPredicateStateBackend::new(pool.clone());
        backend.run_migrations().await.ok()?;
        let c = pool.get().await.ok()?;
        c.batch_execute("TRUNCATE TABLE hook_predicate_counters")
            .await
            .ok()?;
        Some(url)
    }

    fn host(url: &str) -> Arc<dyn PredicateStateBackend> {
        Arc::new(PostgresPredicateStateBackend::new(
            build_pool(url).expect("pool"),
        ))
    }

    /// `IRONCLAW_REQUIRE_POSTGRES=1` turns a missing/unreachable Postgres into a
    /// HARD failure (CI sets this) instead of a silent skip-pass; local runs
    /// without it still skip cleanly.
    fn require_postgres() -> bool {
        std::env::var("IRONCLAW_REQUIRE_POSTGRES")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false)
    }

    macro_rules! pg_skip_or {
        ($url:ident) => {
            match prepare().await {
                Some(u) => u,
                None => {
                    if require_postgres() {
                        panic!(
                            "IRONCLAW_REQUIRE_POSTGRES=1 but Postgres multi-host parity \
                             could not run (IRONCLAW_HOOKS_POSTGRES_URL / DATABASE_URL \
                             unset or unreachable). Refusing to skip-pass under the CI \
                             hard-gate."
                        );
                    }
                    eprintln!(
                        "skipping postgres multi-host parity: \
                         IRONCLAW_HOOKS_POSTGRES_URL / DATABASE_URL not set"
                    );
                    return;
                }
            }
        };
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[allow(clippy::await_holding_lock)]
    async fn postgres_concurrent_writers_no_desync() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let url = pg_skip_or!(url);
        scenario_concurrent_writers_no_desync(host(&url), host(&url), host(&url)).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[allow(clippy::await_holding_lock)]
    async fn postgres_cross_host_replay_exactly_once() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let url = pg_skip_or!(url);
        scenario_cross_host_replay_exactly_once(host(&url), host(&url)).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[allow(clippy::await_holding_lock)]
    async fn postgres_per_key_cap_fails_closed_under_flood() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let url = pg_skip_or!(url);
        scenario_per_key_cap_fails_closed_under_flood(host(&url), host(&url)).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[allow(clippy::await_holding_lock)]
    async fn postgres_cap_boundary_race_admits_exactly_one() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let url = pg_skip_or!(url);
        scenario_cap_boundary_race_admits_exactly_one(host(&url), host(&url), host(&url)).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[allow(clippy::await_holding_lock)]
    async fn postgres_clock_skew_follows_caller_clock() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let url = pg_skip_or!(url);
        scenario_clock_skew_follows_caller_clock(host(&url), host(&url)).await;
    }
}
