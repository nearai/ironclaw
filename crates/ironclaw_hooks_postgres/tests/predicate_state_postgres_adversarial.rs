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
    InvocationKey, MAX_KEYS_PER_TENANT, MAX_SAMPLES_PER_KEY, PredicateEventId,
    PredicateStateBackend, ValueKey,
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
                .batch_execute("TRUNCATE TABLE hook_predicate_counters")
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

/// Per-key sample cap under a flood: the in-window count must never
/// exceed `MAX_SAMPLES_PER_KEY`, with drop-oldest keeping the most-recent
/// samples. Uses a small, deterministic overflow.
#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn per_key_sample_cap_bounds_count_under_flood() {
    let (url, _guard) = guarded!();
    let hs = hosts(&url, 1).await;
    let backend = &hs[0];
    let key = inv_key("flood-tenant", "cap.hot");
    let window = Duration::from_secs(86_400);

    // Insert cap + overflow distinct ids at strictly increasing ts so
    // drop-oldest is well-defined.
    let overflow = 40usize;
    let total = MAX_SAMPLES_PER_KEY + overflow;
    let mut last = 0u32;
    for i in 0..total {
        let ts = base() + chrono::Duration::milliseconds(i as i64);
        last = backend
            .record_invocation(&key, &ev(&format!("flood-{i}")), ts, window)
            .await
            .expect("record");
    }
    assert_eq!(
        last as usize, MAX_SAMPLES_PER_KEY,
        "count must be pinned at the per-key cap under sustained flood"
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
                   FROM hook_predicate_counters
                  WHERE kind = 'i'
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
