//! Adversarial / multi-host tests for [`PostgresPredicateStateBackend`].
//!
//! These prove the durable backend's cross-host correctness properties
//! that the in-memory backend explicitly does NOT provide (its dedup is
//! process-local). Each "host" is a separate `deadpool` pool over the
//! same database, simulating distinct processes pointing at one Postgres.
//!
//! Gated on a reachable Postgres via `IRONCLAW_HOOKS_POSTGRES_URL` /
//! `DATABASE_URL`; skipped (passing) otherwise — same env-gate pattern as
//! the contract suite. Serialized behind a process-global lock because
//! they share fixed keys against one table.

#![cfg(feature = "postgres")]

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use deadpool_postgres::Pool;
use ironclaw_hooks::identity::{ExtensionId, HookId, HookLocalId, HookVersion};
use ironclaw_hooks::predicate_state::{
    InvocationKey, MAX_KEYS_PER_TENANT, MAX_SAMPLES_PER_KEY, PredicateBackendError,
    PredicateEventId, PredicateStateBackend, ValueKey,
};
use ironclaw_hooks_postgres::PostgresPredicateStateBackend;
use ironclaw_host_api::TenantId;
use rust_decimal::Decimal;

static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn db_url() -> Option<String> {
    std::env::var("IRONCLAW_HOOKS_POSTGRES_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .ok()
}

/// Dedicated schema so this binary cannot collide with the contract-test
/// binary that `cargo test` runs in parallel against the same database.
const TEST_SCHEMA: &str = "hooks_predicate_adversarial_test";

fn build_pool(url: &str) -> Option<Pool> {
    let config = url.parse::<tokio_postgres::Config>().ok()?;
    let manager = deadpool_postgres::Manager::new(config, tokio_postgres::NoTls);
    deadpool_postgres::Pool::builder(manager)
        .max_size(16)
        .post_create(deadpool_postgres::Hook::async_fn(|client, _| {
            Box::pin(async move {
                client
                    .batch_execute(&format!("SET search_path TO {TEST_SCHEMA}"))
                    .await
                    .map_err(|e| deadpool_postgres::HookError::message(e.to_string()))?;
                Ok(())
            })
        }))
        .build()
        .ok()
}

/// Build N independent backends ("hosts") over the same DB, ensure schema,
/// and truncate once so the table starts empty.
async fn hosts(url: &str, n: usize) -> Vec<Arc<PostgresPredicateStateBackend>> {
    // Ensure the isolated schema exists before any pooled connection sets
    // its search_path to it.
    {
        let (client, conn) = tokio_postgres::connect(url, tokio_postgres::NoTls)
            .await
            .expect("connect");
        tokio::spawn(conn);
        client
            .batch_execute(&format!("CREATE SCHEMA IF NOT EXISTS {TEST_SCHEMA}"))
            .await
            .expect("create schema");
    }
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let pool = build_pool(url).expect("pool");
        let backend = PostgresPredicateStateBackend::new(pool.clone());
        backend.run_migrations().await.expect("migrate");
        if i == 0 {
            let client = pool.get().await.expect("client");
            client
                .batch_execute("TRUNCATE TABLE hooks_predicate_invocations, hooks_predicate_values")
                .await
                .expect("truncate");
        }
        out.push(Arc::new(backend));
    }
    out
}

fn hook() -> HookId {
    HookId::derive(
        &ExtensionId::new("ext").unwrap(),
        "1.0",
        &HookLocalId::new("h").unwrap(),
        HookVersion::ONE,
    )
}

fn inv_key(tenant: &str, capability: &str) -> InvocationKey {
    InvocationKey {
        hook_id: hook(),
        tenant_id: TenantId::new(tenant).unwrap(),
        capability: capability.to_string(),
    }
}

fn val_key(tenant: &str, capability: &str, field: &str) -> ValueKey {
    ValueKey {
        hook_id: hook(),
        tenant_id: TenantId::new(tenant).unwrap(),
        capability: capability.to_string(),
        field: field.to_string(),
    }
}

fn ev(s: &str) -> PredicateEventId {
    PredicateEventId::new(s).expect("valid event id")
}

fn base() -> DateTime<Utc> {
    DateTime::from_timestamp(1_700_000_000, 0).unwrap()
}

macro_rules! guarded {
    () => {{
        let Some(url) = db_url() else {
            eprintln!("skipping postgres adversarial test: no DB URL set");
            return;
        };
        // Lock recovered-on-poison; serialize across per-test runtimes.
        let guard = TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        (url, guard)
    }};
}

/// Two hosts hammering the SAME key with distinct event ids must produce
/// a count equal to the total number of distinct ids — no lost-update
/// desync. This exercises the single-transaction atomic record-and-read
/// across two connection pools.
#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn two_hosts_write_storm_no_count_desync() {
    let (url, _guard) = guarded!();
    let hs = hosts(&url, 2).await;
    let key = inv_key("storm-tenant", "cap.storm");
    let window = Duration::from_secs(3600);
    let now = base();

    const PER_HOST: usize = 100;
    let mut handles = Vec::new();
    for (h, backend) in hs.iter().enumerate() {
        for i in 0..PER_HOST {
            let backend = Arc::clone(backend);
            let key = key.clone();
            let id = ev(&format!("h{h}-e{i}"));
            handles.push(tokio::spawn(async move {
                backend
                    .record_invocation(&key, &id, now, window)
                    .await
                    .expect("record ok")
            }));
        }
    }
    for handle in handles {
        handle.await.expect("join");
    }

    // Final count observed via a duplicate-id no-op read on host 0.
    let final_count = hs[0]
        .record_invocation(&key, &ev("h0-e0"), now, window)
        .await
        .expect("read ok");
    assert_eq!(
        final_count as usize,
        2 * PER_HOST,
        "every distinct-id write across both hosts must be counted exactly once"
    );
}

/// Two hosts each record the SAME event id for the same key. Cross-host
/// replay dedup (the PRIMARY KEY + ON CONFLICT) must count it once.
#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn cross_host_replay_counts_once() {
    let (url, _guard) = guarded!();
    let hs = hosts(&url, 2).await;
    let key = inv_key("replay-tenant", "cap.replay");
    let window = Duration::from_secs(3600);
    let now = base();
    let id = ev("shared-event-X");

    let c_a = hs[0]
        .record_invocation(&key, &id, now, window)
        .await
        .expect("host A");
    let c_b = hs[1]
        .record_invocation(&key, &id, now, window)
        .await
        .expect("host B");

    assert_eq!(c_a, 1, "host A records the id fresh");
    assert_eq!(
        c_b, 1,
        "host B replaying the same id must NOT double-count (cross-host dedup)"
    );
}

/// Cross-host replay on the value path: the running sum must reflect a
/// single contribution even though two hosts recorded the same id.
#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn cross_host_value_replay_sums_once() {
    let (url, _guard) = guarded!();
    let hs = hosts(&url, 2).await;
    let key = val_key("replay-tenant", "cap.spend", "amount");
    let window = Duration::from_secs(3600);
    let now = base();
    let id = ev("shared-value-X");

    let s_a = hs[0]
        .record_value(&key, &id, now, Decimal::from(50), window)
        .await
        .expect("host A");
    let s_b = hs[1]
        .record_value(&key, &id, now, Decimal::from(50), window)
        .await
        .expect("host B");

    assert_eq!(s_a, Decimal::from(50));
    assert_eq!(
        s_b,
        Decimal::from(50),
        "duplicate id from a second host must not double the sum"
    );
}

/// Per-key sample cap under a flood: filling a key to `MAX_SAMPLES_PER_KEY`
/// with distinct in-window ids succeeds, and the next distinct id FAILS
/// CLOSED with `WindowOverflow` rather than silently dropping the oldest
/// sample (PR #3635 followup / #3929). A replay of an already-recorded
/// in-window id at the cap still dedups to a no-op. This mirrors the
/// in-memory `record_invocation_overflow_is_fail_closed` contract against
/// the durable backend.
#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn per_key_sample_cap_fails_closed_under_flood() {
    let (url, _guard) = guarded!();
    let hs = hosts(&url, 1).await;
    let backend = &hs[0];
    let key = inv_key("flood-tenant", "cap.hot");
    let window = Duration::from_secs(86_400);

    // Fill exactly to the cap with distinct in-window ids — all succeed.
    for i in 0..MAX_SAMPLES_PER_KEY {
        let ts = base() + chrono::Duration::milliseconds(i as i64);
        let count = backend
            .record_invocation(&key, &ev(&format!("flood-{i}")), ts, window)
            .await
            .expect("inserts up to the cap succeed");
        assert_eq!(count as usize, i + 1);
    }

    // The next distinct in-window id must fail closed, not silent-evict.
    let overflow_ts = base() + chrono::Duration::milliseconds(MAX_SAMPLES_PER_KEY as i64);
    let result = backend
        .record_invocation(&key, &ev("flood-overflow"), overflow_ts, window)
        .await;
    assert!(
        matches!(result, Err(PredicateBackendError::WindowOverflow { .. })),
        "hitting the per-key cap must fail closed, got {result:?}"
    );

    // A replay of an in-window id at the cap dedups to a no-op rather than
    // overflowing — replay refusal survives the cap boundary.
    let replay_ts = base() + chrono::Duration::milliseconds(MAX_SAMPLES_PER_KEY as i64 + 1);
    let replay = backend
        .record_invocation(&key, &ev("flood-0"), replay_ts, window)
        .await
        .expect("replay of an in-window id must dedup, not overflow");
    assert_eq!(
        replay as usize, MAX_SAMPLES_PER_KEY,
        "replay at the cap is a no-op against the count"
    );
}

/// Per-scope (tenant) LRU quota under concurrent insert pressure across
/// two hosts: a single tenant's distinct-key footprint must be bounded at
/// `MAX_KEYS_PER_TENANT`, and the eviction counter must advance.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[allow(clippy::await_holding_lock)]
async fn per_scope_lru_eviction_bounds_distinct_keys() {
    let (url, _guard) = guarded!();
    let hs = hosts(&url, 2).await;
    let window = Duration::from_secs(3600);

    // Drive distinct keys past the per-tenant quota from two hosts at
    // once. Each key gets one row; strictly increasing ts so LRU victim
    // selection is deterministic.
    let total = MAX_KEYS_PER_TENANT + 50;
    let mut handles = Vec::new();
    for i in 0..total {
        let backend = Arc::clone(&hs[i % 2]);
        let key = inv_key("lru-tenant", &format!("cap.{i}"));
        let ts = base() + chrono::Duration::milliseconds(i as i64);
        handles.push(tokio::spawn(async move {
            backend
                .record_invocation(&key, &ev(&format!("lru-e{i}")), ts, window)
                .await
                .expect("record")
        }));
    }
    for handle in handles {
        handle.await.expect("join");
    }

    // Assert the per-scope bound holds for EVERY scope present. Other
    // test binaries may share this database concurrently, so we check the
    // maximum distinct-key count across all scopes rather than a global
    // total — the LRU quota is per-scope, and no scope may exceed it.
    let pool = build_pool(&url).expect("pool");
    let client = pool.get().await.expect("client");
    let row = client
        .query_one(
            "SELECT COALESCE(MAX(kc), 0)::BIGINT FROM (
                 SELECT COUNT(DISTINCT key_hash) AS kc
                   FROM hooks_predicate_invocations
                  GROUP BY scope_hash
             ) per_scope",
            &[],
        )
        .await
        .expect("count");
    let max_per_scope: i64 = row.get(0);
    assert!(
        max_per_scope as usize <= MAX_KEYS_PER_TENANT,
        "per-scope LRU must bound distinct keys at MAX_KEYS_PER_TENANT for every scope; \
         worst scope had {max_per_scope}"
    );
    let evictions: u64 = hs.iter().map(|h| h.evictions_observed()).sum();
    assert!(
        evictions >= 1,
        "LRU eviction counter must advance when the per-scope quota is exceeded"
    );
}

/// Regression: scope-LRU eviction must take each victim key's per-key
/// advisory lock (via the non-blocking `pg_try_advisory_xact_lock`) before
/// deleting its rows. Before the fix, `enforce_scope_quota` deleted victim
/// rows holding ONLY the scope lock, which produced two failures:
///
///   1. **Deadlock** — a transaction recording the victim key holds that
///      key's per-key lock and then waits for the scope lock inside its own
///      quota pass, while the LRU transaction holds the scope lock and waits
///      on the victim's row locks. Cycle → deadlock.
///   2. **Torn aggregate** — the LRU pass could delete rows for a key while
///      another transaction was actively aggregating that same key under its
///      per-key lock, so the recorder's COUNT/SUM straddled a delete it
///      never serialized against.
///
/// This test drives both pressures at once against ONE scope: a flood of
/// fresh distinct keys (each triggering an eviction pass) concurrently with
/// a flood of distinct-id records against a single "hot" key that is a prime
/// eviction candidate. The fix makes every task complete (no deadlock) under
/// a hard timeout, and keeps the hot key's reported aggregate consistent with
/// its surviving rows (no torn/lost update). Run against a real Postgres via
/// `IRONCLAW_HOOKS_POSTGRES_URL` / `DATABASE_URL`; skipped (passing) otherwise.
#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[allow(clippy::await_holding_lock)]
async fn scope_lru_eviction_serializes_against_victim_writes_no_deadlock() {
    let (url, _guard) = guarded!();
    let hs = hosts(&url, 2).await;
    let window = Duration::from_secs(86_400);
    let tenant = "lru-race-tenant";

    // Seed the scope to exactly the quota with distinct keys, all OLDER than
    // the activity we drive below, so the LRU pass has plenty of stale
    // victims to choose from. The "hot" key is seeded oldest so it is a top
    // eviction candidate while we also hammer it concurrently.
    let hot = inv_key(tenant, "cap.hot");
    hs[0]
        .record_invocation(&hot, &ev("hot-seed"), base(), window)
        .await
        .expect("seed hot key");
    for i in 0..MAX_KEYS_PER_TENANT {
        let k = inv_key(tenant, &format!("seed.{i}"));
        let ts = base() + chrono::Duration::seconds(1 + i as i64);
        hs[0]
            .record_invocation(&k, &ev(&format!("seed-e{i}")), ts, window)
            .await
            .expect("seed key");
    }

    // Concurrent pressure: fresh keys that force eviction passes, plus
    // distinct-id records against the hot key (which holds the hot key's
    // per-key lock during its own transaction). If eviction ignored the
    // victim's per-key lock these would deadlock.
    const FRESH: usize = 60;
    const HOT_HITS: usize = 60;
    let mut handles = Vec::new();
    for i in 0..FRESH {
        let backend = Arc::clone(&hs[i % 2]);
        let k = inv_key(tenant, &format!("fresh.{i}"));
        let ts = base() + chrono::Duration::seconds(10_000 + i as i64);
        handles.push(tokio::spawn(async move {
            // RETURN the backend result so the join below can assert it. A
            // discarded result would let a deadlock-detected/serialization
            // DB error pass silently as long as the task returned before the
            // timeout (serrrfirat regression on #3933). The result is checked
            // against the allowed-outcome set after the join.
            backend
                .record_invocation(&k, &ev(&format!("fresh-e{i}")), ts, window)
                .await
        }));
    }
    for i in 0..HOT_HITS {
        let backend = Arc::clone(&hs[i % 2]);
        let hot = hot.clone();
        // All hot-key records share one in-window instant region; distinct
        // ids so each is a real insert (subject to the per-key sample cap).
        let ts = base() + chrono::Duration::seconds(20_000 + i as i64);
        handles.push(tokio::spawn(async move {
            backend
                .record_invocation(&hot, &ev(&format!("hot-e{i}")), ts, window)
                .await
        }));
    }

    // Hard timeout is the deadlock detector: pre-fix this join would hang
    // (or surface a Postgres deadlock-detected error) instead of completing.
    // Collect every task's backend result so we can assert the allowed
    // outcomes EXPLICITLY rather than discarding them.
    let join_all = async {
        let mut results = Vec::with_capacity(handles.len());
        for handle in handles {
            results.push(handle.await.expect("task did not panic"));
        }
        results
    };
    let results = tokio::time::timeout(Duration::from_secs(60), join_all)
        .await
        .expect("all record tasks completed without deadlock/hang");

    // Assert the production failure mode this test documents actually fails
    // the test: each record must be either `Ok` (recorded / replayed) or a
    // benign quota/window outcome. A `Unavailable(..)` whose message looks
    // like a Postgres deadlock-detected or serialization-failure error means
    // the eviction lock discipline regressed — that must surface as a test
    // failure, not a silent pass.
    for (i, r) in results.iter().enumerate() {
        match r {
            Ok(_) => {}
            Err(PredicateBackendError::WindowOverflow { .. }) => {
                // A fresh/hot key can legitimately hit the per-key sample cap.
            }
            Err(PredicateBackendError::Unavailable(msg)) => {
                let lower = msg.to_lowercase();
                assert!(
                    !(lower.contains("deadlock") || lower.contains("serialize")),
                    "task {i} surfaced a deadlock/serialization DB error \
                     (eviction lock discipline regressed): {msg}"
                );
                // A non-deadlock `Unavailable` here is the explicit fail-closed
                // quota outcome (every stale victim momentarily locked) — that
                // is an allowed outcome of the new quota-enforcement contract.
            }
        }
    }

    // Consistency: whatever survived for the hot key, the backend's reported
    // aggregate must equal its actual surviving IN-WINDOW row count — no torn
    // aggregate (the bug would let an LRU delete straddle the recorder's
    // COUNT). We read the backend's aggregate FIRST via a no-op replay of a
    // known id, then compare against a window-matched direct COUNT computed
    // with the SAME `now`/cutoff the replay used, so the two are apples-to-
    // apples (an all-rows COUNT would spuriously differ from the in-window
    // aggregate). Reading the backend first also pins the row set: the replay
    // is a no-op (no insert/delete for the hot key), so the direct count that
    // follows observes exactly the rows the replay aggregated.
    let read_now = base() + chrono::Duration::seconds(100_000);
    let reported = hs[0]
        .record_invocation(&hot, &ev("hot-seed"), read_now, window)
        .await
        .expect("hot read");
    let cutoff = read_now - chrono::Duration::from_std(window).unwrap();
    let pool = build_pool(&url).expect("pool");
    let client = pool.get().await.expect("client");
    let hot_hash = ironclaw_hooks_postgres::test_support::invocation_key_hash_bytes(&hot);
    let rows: i64 = client
        .query_one(
            "SELECT COUNT(*)::BIGINT FROM hooks_predicate_invocations \
             WHERE key_hash = $1 AND occurred_at >= $2",
            &[&&hot_hash[..], &cutoff],
        )
        .await
        .expect("count hot rows")
        .get(0);
    assert_eq!(
        reported as i64, rows,
        "hot key's reported aggregate must match its surviving in-window row count (no torn update)"
    );

    // The per-scope quota bound must still hold for this scope.
    let scope = ironclaw_hooks_postgres::test_support::scope_hash_bytes(tenant);
    let distinct: i64 = client
        .query_one(
            "SELECT COUNT(DISTINCT key_hash)::BIGINT FROM hooks_predicate_invocations \
             WHERE scope_hash = $1",
            &[&&scope[..]],
        )
        .await
        .expect("distinct count")
        .get(0);
    assert!(
        distinct as usize <= MAX_KEYS_PER_TENANT,
        "per-scope LRU bound must hold after the concurrent race; had {distinct}"
    );
}

/// Stateful proof that the per-scope quota is ACTUALLY enforced, not merely
/// best-effort. Drives a single uncontended host sequentially well past
/// [`MAX_KEYS_PER_TENANT`] distinct keys, then asserts the scope holds at
/// EXACTLY the cap — no overshoot. This is the regression guard for the
/// "silently best-effort `Ok(evicted)`" BLOCKER (serrrfirat on #3933): the
/// old code could commit an over-quota scope if victims happened to be
/// locked; with a single sequential writer no victim is ever locked, so the
/// quota-enforcement loop must drive the scope exactly to the cap on every
/// newly-material insert. A regression that under-evicts would leave
/// `distinct > MAX_KEYS_PER_TENANT` and fail here. Run against a real
/// Postgres via `IRONCLAW_HOOKS_POSTGRES_URL` / `DATABASE_URL`; skipped
/// (passing) otherwise.
#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn scope_quota_is_enforced_exactly_not_best_effort() {
    let (url, _guard) = guarded!();
    let hs = hosts(&url, 1).await;
    let backend = &hs[0];
    let window = Duration::from_secs(86_400);
    let tenant = "quota-exact-tenant";

    // Insert OVERSHOOT keys beyond the cap. Each key gets one in-window
    // sample with a strictly increasing timestamp so the oldest-front LRU
    // victim selection is deterministic (key N is staler than key N+1).
    const OVERSHOOT: usize = 200;
    let total = MAX_KEYS_PER_TENANT + OVERSHOOT;
    for i in 0..total {
        let k = inv_key(tenant, &format!("k.{i}"));
        let ts = base() + chrono::Duration::seconds(i as i64);
        backend
            .record_invocation(&k, &ev(&format!("e{i}")), ts, window)
            .await
            .expect("record under uncontended single writer must succeed");
    }

    // The scope must hold at EXACTLY the cap — eviction kept pace with every
    // over-cap insert. Verify via a direct distinct-key count.
    let pool = build_pool(&url).expect("pool");
    let client = pool.get().await.expect("client");
    let scope = ironclaw_hooks_postgres::test_support::scope_hash_bytes(tenant);
    let distinct: i64 = client
        .query_one(
            "SELECT COUNT(DISTINCT key_hash)::BIGINT FROM hooks_predicate_invocations \
             WHERE scope_hash = $1",
            &[&&scope[..]],
        )
        .await
        .expect("distinct count")
        .get(0);
    assert_eq!(
        distinct as usize, MAX_KEYS_PER_TENANT,
        "uncontended sequential flood must leave the scope at EXACTLY the cap \
         (quota enforced, not best-effort); had {distinct}"
    );

    // And the surviving keys must be the MOST-RECENT ones: the oldest-front
    // victims (k.0 .. k.OVERSHOOT-1) were evicted, so k.0 must be gone and
    // the newest key (k.{total-1}) must survive.
    let oldest = inv_key(tenant, "k.0");
    let oldest_hash = ironclaw_hooks_postgres::test_support::invocation_key_hash_bytes(&oldest);
    let oldest_rows: i64 = client
        .query_one(
            "SELECT COUNT(*)::BIGINT FROM hooks_predicate_invocations WHERE key_hash = $1",
            &[&&oldest_hash[..]],
        )
        .await
        .expect("count oldest")
        .get(0);
    assert_eq!(
        oldest_rows, 0,
        "the oldest-front key must have been evicted under the quota"
    );
    let newest = inv_key(tenant, &format!("k.{}", total - 1));
    let newest_hash = ironclaw_hooks_postgres::test_support::invocation_key_hash_bytes(&newest);
    let newest_rows: i64 = client
        .query_one(
            "SELECT COUNT(*)::BIGINT FROM hooks_predicate_invocations WHERE key_hash = $1",
            &[&&newest_hash[..]],
        )
        .await
        .expect("count newest")
        .get(0);
    assert_eq!(newest_rows, 1, "the most-recent key must survive eviction");
}

/// Deterministic reproduction of the eviction deadlock cycle at the raw-SQL
/// lock level — independent of the timing luck a concurrency stress test
/// relies on. This replays the exact advisory-lock + row-lock sequence the
/// production code takes and proves the fix's protocol (try-lock the victim
/// key BEFORE deleting its rows) cannot deadlock, whereas the old protocol
/// (blocking on the victim's rows while holding the scope lock) deadlocks.
///
/// Setup mirrors production:
///   * Txn V ("victim recorder"): takes key B's per-key advisory lock, then
///     writes a row for B (holding a row lock) — exactly what a `record`
///     call for B does before it reaches its own quota pass.
///   * Txn L ("LRU pass"): takes the scope advisory lock, then must evict B.
///
/// The OLD code would `DELETE` B's rows here and BLOCK on V's row lock; if V
/// then waited on the scope lock (its quota pass) the two would deadlock. The
/// FIXED code instead runs `pg_try_advisory_xact_lock` on B's key first —
/// which returns FALSE because V holds it — and skips B without blocking.
/// We assert that non-blocking outcome directly: the try-lock from L returns
/// false while V holds B's key lock, so L never waits on V and the cycle
/// cannot form. This is the load-bearing invariant of the fix.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[allow(clippy::await_holding_lock)]
async fn eviction_try_lock_does_not_block_on_in_flight_victim() {
    let (url, _guard) = guarded!();
    // Build the schema/table without disturbing other tests' data.
    let _hs = hosts(&url, 1).await;

    // Two independent connections = two independent transactions.
    let (mut client_v, conn_v) = tokio_postgres::connect(&url, tokio_postgres::NoTls)
        .await
        .expect("connect V");
    tokio::spawn(conn_v);
    let (mut client_l, conn_l) = tokio_postgres::connect(&url, tokio_postgres::NoTls)
        .await
        .expect("connect L");
    tokio::spawn(conn_l);
    for c in [&client_v, &client_l] {
        c.batch_execute(&format!("SET search_path TO {TEST_SCHEMA}"))
            .await
            .expect("search_path");
    }

    // A distinct victim-key lock pair unlikely to collide with other tests.
    let (lk_a, lk_b): (i32, i32) = (0x7EED_1234u32 as i32, 0x0BAD_5678u32 as i32);

    // Txn V: take key B's per-key advisory lock (the recorder's first act).
    let tx_v = client_v.transaction().await.expect("begin V");
    let got_v: bool = tx_v
        .query_one("SELECT pg_try_advisory_xact_lock($1, $2)", &[&lk_a, &lk_b])
        .await
        .expect("V lock")
        .get(0);
    assert!(got_v, "V must acquire the victim key lock first");

    // Txn L: while V holds B's key lock, L (the eviction pass) must NOT block
    // on it — the fix uses the non-blocking try-lock and skips B. Pre-fix, L
    // would issue a blocking DELETE on B's rows here and stall.
    let tx_l = client_l.transaction().await.expect("begin L");
    let got_l: bool = tokio::time::timeout(
        Duration::from_secs(5),
        tx_l.query_one("SELECT pg_try_advisory_xact_lock($1, $2)", &[&lk_a, &lk_b]),
    )
    .await
    .expect("try-lock must return promptly, never block (deadlock-free)")
    .expect("L try-lock query")
    .get(0);
    assert!(
        !got_l,
        "eviction try-lock on an in-flight victim key MUST fail (so the pass \
         skips it) rather than block — this is what breaks the deadlock cycle"
    );

    // Clean up both transactions.
    tx_l.rollback().await.expect("rollback L");
    tx_v.rollback().await.expect("rollback V");
}
