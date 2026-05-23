//! Wire the durable [`LibSqlPredicateStateBackend`] through the shared
//! [`PredicateStateBackend`] contract suite (durable-backend PR 3/4), plus
//! libSQL-specific adversarial tests (concurrent writers, cross-host replay,
//! LRU + per-key cap under pressure).
//!
//! Requires `--features "libsql contract-tests"`. The whole file is gated on
//! `libsql` so default / postgres builds skip it.
#![cfg(feature = "libsql")]

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use ironclaw_hooks::identity::{ExtensionId, HookId, HookLocalId, HookVersion};
use ironclaw_hooks::predicate_state::libsql::LibSqlPredicateStateBackend;
use ironclaw_hooks::predicate_state::{
    InvocationKey, MAX_KEYS_PER_TENANT, MAX_SAMPLES_PER_KEY, PredicateEventId,
    PredicateStateBackend, ValueKey,
};
use ironclaw_host_api::TenantId;
use rust_decimal::Decimal;
use tempfile::TempDir;

/// Build a fresh, migrated libSQL-backed backend over a private temp-file
/// database. Returns the backend plus the `TempDir` guard (kept alive by the
/// caller for the duration of the test).
///
/// The contract macro's factory is a synchronous `Fn() -> B`, but building a
/// libSQL `Database` and running migrations is async. We bridge by running the
/// async setup on a dedicated current-thread runtime on a fresh OS thread,
/// which avoids the "cannot start a runtime from within a runtime" panic when
/// the surrounding `#[tokio::test]` is already on a tokio worker.
fn fresh_backend_blocking() -> (LibSqlPredicateStateBackend, TempDir) {
    std::thread::scope(|s| {
        s.spawn(|| {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("build current-thread runtime");
            rt.block_on(async {
                let dir = tempfile::tempdir().expect("tempdir");
                let path = dir.path().join("predicate_state.db");
                let db = Arc::new(
                    libsql::Builder::new_local(path.to_string_lossy().to_string())
                        .build()
                        .await
                        .expect("build libsql db"),
                );
                let backend = LibSqlPredicateStateBackend::new(db);
                backend.run_migrations().await.expect("migrate");
                (backend, dir)
            })
        })
        .join()
        .expect("setup thread joined")
    })
}

/// Factory for the contract macro: discards the `TempDir` guard by leaking it
/// into a process-lifetime store so the db file outlives the contract body.
/// Contract tests are short-lived; leaking a handful of temp dirs for the test
/// process is acceptable and keeps the `Fn() -> B` shape the macro requires.
fn contract_factory() -> LibSqlPredicateStateBackend {
    let (backend, dir) = fresh_backend_blocking();
    // Keep the temp dir alive for the rest of the process. `Box::leak` is
    // confined to the test binary.
    Box::leak(Box::new(dir));
    backend
}

ironclaw_hooks::predicate_backend_contract_test!(libsql_backend, crate::contract_factory);

// ---------------------------------------------------------------------------
// Adversarial tests
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

/// Build a backend over a shared temp-file db (file path returned so a second
/// connection can open the SAME database, simulating a second host).
async fn shared_db_backend() -> (Arc<libsql::Database>, TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("shared.db");
    let db = Arc::new(
        libsql::Builder::new_local(path.to_string_lossy().to_string())
            .build()
            .await
            .expect("build libsql db"),
    );
    let backend = LibSqlPredicateStateBackend::new(db.clone());
    backend.run_migrations().await.expect("migrate");
    (db, dir)
}

/// Two simulated hosts (two `LibSqlPredicateStateBackend` instances over the
/// same db file) hammer the same invocation key concurrently with DISTINCT
/// event ids. `BEGIN IMMEDIATE` must serialise the read-modify-write so every
/// write is counted exactly once — the final count equals the number of
/// distinct ids (no lost-update desync).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_hosts_concurrent_writes_no_desync() {
    let (db, _dir) = shared_db_backend().await;
    let host_a = Arc::new(LibSqlPredicateStateBackend::new(db.clone()));
    let host_b = Arc::new(LibSqlPredicateStateBackend::new(db.clone()));
    let key = inv_key("alpha", "cap.concurrent");
    let now = at_secs(0);
    let window = Duration::from_secs(60);

    const N: usize = 24;
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
                .record_invocation(&key, &ev(&format!("event-{i}")), now, window)
                .await
                .expect("record ok")
        }));
    }
    for h in handles {
        h.await.expect("task joined");
    }
    // Re-read via a duplicate id (no-op insert) on a THIRD instance.
    let observer = LibSqlPredicateStateBackend::new(db.clone());
    let final_count = observer
        .record_invocation(&key, &ev("event-0"), now, window)
        .await
        .expect("read ok");
    assert_eq!(
        final_count as usize, N,
        "two hosts writing distinct ids concurrently must each be counted exactly once"
    );
}

/// Cross-host replay dedup: host A records an event, host B replays the SAME
/// event id against the SAME key. The durable PRIMARY KEY must dedup across
/// hosts (the property the in-memory backend explicitly cannot provide).
#[tokio::test]
async fn cross_host_replay_is_deduped() {
    let (db, _dir) = shared_db_backend().await;
    let host_a = LibSqlPredicateStateBackend::new(db.clone());
    let host_b = LibSqlPredicateStateBackend::new(db.clone());
    let key = inv_key("alpha", "cap.replay");
    let window = Duration::from_secs(60);

    let c1 = host_a
        .record_invocation(&key, &ev("shared-evt"), at_secs(0), window)
        .await
        .expect("ok");
    assert_eq!(c1, 1);

    // Host B replays the same event id — must be a no-op against the count.
    let c2 = host_b
        .record_invocation(&key, &ev("shared-evt"), at_secs(1), window)
        .await
        .expect("ok");
    assert_eq!(
        c2, 1,
        "cross-host replay of a recorded event id must not increment"
    );

    // A distinct id on host B does advance.
    let c3 = host_b
        .record_invocation(&key, &ev("fresh-evt"), at_secs(2), window)
        .await
        .expect("ok");
    assert_eq!(c3, 2);
}

/// Cross-host replay dedup for the value path.
#[tokio::test]
async fn cross_host_replay_is_deduped_for_values() {
    let (db, _dir) = shared_db_backend().await;
    let host_a = LibSqlPredicateStateBackend::new(db.clone());
    let host_b = LibSqlPredicateStateBackend::new(db.clone());
    let key = val_key("alpha", "cap.spend", "amount");
    let window = Duration::from_secs(60);

    let s1 = host_a
        .record_value(
            &key,
            &ev("shared-evt"),
            at_secs(0),
            Decimal::from(50),
            window,
        )
        .await
        .expect("ok");
    assert_eq!(s1, Decimal::from(50));

    let s2 = host_b
        .record_value(
            &key,
            &ev("shared-evt"),
            at_secs(1),
            Decimal::from(50),
            window,
        )
        .await
        .expect("ok");
    assert_eq!(
        s2,
        Decimal::from(50),
        "cross-host replay must not double-count the value sum"
    );
}

/// Per-key sample cap holds under sustained pressure: inserting well past
/// `MAX_SAMPLES_PER_KEY` distinct events keeps the bucket pinned at the cap
/// (oldest dropped), and the reported count never exceeds the cap.
#[tokio::test]
async fn per_key_sample_cap_holds_under_pressure() {
    let (backend, _dir) = fresh_backend_blocking();
    let key = inv_key("alpha", "cap.hot");
    let window = Duration::from_secs(3600);
    let overflow = MAX_SAMPLES_PER_KEY + 50;

    let mut last = 0u32;
    for i in 0..overflow {
        last = backend
            .record_invocation(&key, &ev(&format!("evt-{i}")), at_millis(i as i64), window)
            .await
            .expect("ok");
    }
    assert_eq!(
        last as usize, MAX_SAMPLES_PER_KEY,
        "drop-oldest must pin the in-window count at the per-key cap under sustained pressure"
    );
}

/// Value-path running-sum consistency under cap eviction: with a constant
/// value per insert, the post-cap sum equals `cap * value` exactly (the sum is
/// recomputed from surviving rows, so it can never drift).
#[tokio::test]
async fn value_sum_consistent_under_cap_eviction() {
    let (backend, _dir) = fresh_backend_blocking();
    let key = val_key("alpha", "cap.spend", "amount");
    let window = Duration::from_secs(3600);
    let overflow = MAX_SAMPLES_PER_KEY + 32;
    let value = Decimal::from(3);

    let mut last = Decimal::ZERO;
    for i in 0..overflow {
        last = backend
            .record_value(
                &key,
                &ev(&format!("v-{i}")),
                at_millis(i as i64),
                value,
                window,
            )
            .await
            .expect("ok");
    }
    assert_eq!(
        last,
        Decimal::from(MAX_SAMPLES_PER_KEY as u64) * value,
        "post-cap sum must equal cap * value (recomputed from surviving rows)"
    );
}

/// Per-tenant LRU quota under concurrent pressure: a single noisy tenant
/// inserting more than `MAX_KEYS_PER_TENANT` distinct scopes must be held at
/// its quota (its own oldest scopes evicted) and must NOT evict a quiet
/// tenant's scope. Drives the public API concurrently to exercise the
/// serialised eviction path.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn per_tenant_quota_isolates_under_concurrent_pressure() {
    let (db, _dir) = shared_db_backend().await;
    let backend = Arc::new(LibSqlPredicateStateBackend::new(db.clone()));
    let window = Duration::from_secs(60);

    // Quiet tenant β records one scope first.
    let beta_key = inv_key("beta", "beta.cap");
    backend
        .record_invocation(&beta_key, &ev("beta-evt"), at_secs(0), window)
        .await
        .expect("ok");

    // Noisy tenant α floods past its quota. libSQL opens a fresh connection
    // per operation against a single file; an unbounded fan-out of thousands
    // of simultaneous opens exhausts OS file handles (SQLite CANTOPEN), so we
    // bound concurrency with a semaphore. The contention that matters — many
    // writers serialising on the single `BEGIN IMMEDIATE` write lock and the
    // eviction path racing — is fully exercised with a bounded pool.
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
                    &ev(&format!("alpha-e{i}")),
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

    // β's scope must survive (α can only evict its own scopes).
    let beta_count = backend
        .record_invocation(&beta_key, &ev("beta-evt"), at_secs(0), window)
        .await
        .expect("ok");
    assert_eq!(
        beta_count, 1,
        "quiet tenant β's scope must survive noisy tenant α's flood (replay no-op confirms it persists)"
    );

    // α must be held at its per-tenant quota.
    let alpha_scopes = count_distinct_scopes(&db, "hooks_predicate_invocations", "alpha").await;
    assert!(
        alpha_scopes <= MAX_KEYS_PER_TENANT,
        "noisy tenant α must be capped at its per-tenant quota; got {alpha_scopes}"
    );
}

async fn count_distinct_scopes(db: &Arc<libsql::Database>, table: &str, tenant: &str) -> usize {
    let conn = db.connect().expect("connect");
    let mut rows = conn
        .query(
            &format!("SELECT count(DISTINCT scope_hash) FROM {table} WHERE tenant_id = ?1"),
            libsql::params![tenant],
        )
        .await
        .expect("query");
    let row = rows.next().await.expect("row").expect("some");
    let v: i64 = row.get(0).expect("i64");
    v.max(0) as usize
}

/// Durable survival across "restart": a backend instance writes, is dropped,
/// and a NEW instance over the SAME db file reads the persisted count.
#[tokio::test]
async fn state_survives_restart() {
    let (db, _dir) = shared_db_backend().await;
    let key = inv_key("alpha", "cap.persist");
    let window = Duration::from_secs(3600);
    {
        let backend = LibSqlPredicateStateBackend::new(db.clone());
        for i in 0..3 {
            backend
                .record_invocation(&key, &ev(&format!("e{i}")), at_secs(i), window)
                .await
                .expect("ok");
        }
    }
    // New instance, same db file — no migration re-run needed.
    let restarted = LibSqlPredicateStateBackend::new(db.clone());
    let count = restarted
        .record_invocation(&key, &ev("e0"), at_secs(3), window)
        .await
        .expect("ok");
    assert_eq!(
        count, 3,
        "persisted count must survive a backend instance restart"
    );
}
