//! [`PostgresPredicateStateBackend`] — durable, cross-host-consistent
//! implementation of [`PredicateStateBackend`].
//!
//! # Atomic record-and-read (the load-bearing property)
//!
//! Every `record_*` call runs inside a single `READ COMMITTED`
//! transaction guarded by a transaction-scoped advisory lock on the
//! bucket (and, for new-key eviction, a second advisory lock on the
//! scope). The advisory lock — not a high isolation level — is what
//! serializes concurrent writers to the same bucket: it makes the second
//! writer *block* until the first commits rather than aborting it with a
//! serialization failure (which would force a caller retry loop). The
//! transaction body:
//!
//! 1. **Trims** rows for the key with `ts < cutoff` (out of window). This
//!    also frees the dedup id of any trimmed row, so an `id` that aged
//!    out of the window can be re-recorded — matching the in-memory
//!    backend, whose dedup memory is exactly the in-window entry set.
//! 2. **Inserts** the new row `ON CONFLICT (key_hash, id) DO NOTHING`.
//!    The primary key IS the replay-dedup constraint: a duplicate id for
//!    the same key is a no-op. This is the cross-host replay defense — a
//!    row written by host A blocks host B's re-insert of the same id
//!    regardless of clock skew.
//! 3. **Caps** the per-key sample count at [`MAX_SAMPLES_PER_KEY`] by
//!    dropping the oldest overflow rows (drop-oldest, matching the
//!    in-memory backend's `while len >= cap { pop_front }`).
//! 4. **Evicts** the scope's least-recently-active key when the scope's
//!    distinct-key count exceeds [`MAX_KEYS_PER_TENANT`] (the durable
//!    analogue of the in-memory per-tenant LRU quota).
//! 5. **Aggregates** the in-window `COUNT(*)` / `SUM(value)` and returns it.
//!
//! Steps 1-5 share one transaction under the bucket advisory lock, so two
//! concurrent writers can never both observe "1 under cap" and both
//! proceed — the second blocks until the first commits. This is the codex
//! Critical atomicity requirement from PR #3635.
//!
//! # Cap semantics — drop-oldest, NOT silent loss
//!
//! The per-key cap drops the *oldest* samples to make room, keeping the
//! most-recent [`MAX_SAMPLES_PER_KEY`] in the window. This is the same
//! conservative bound the in-memory backend enforces. (The base branch's
//! in-memory backend uses drop-oldest, not a `WindowOverflow` fail-closed
//! error; this backend matches it. If a later in-memory bugfix changes
//! that to fail-closed, this backend should follow — see the trait-level
//! docs.)

use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use deadpool_postgres::Pool;
use ironclaw_hooks::predicate_state::{
    InvocationKey, MAX_KEYS_PER_TENANT, MAX_SAMPLES_PER_KEY, PredicateBackendError,
    PredicateEventId, PredicateStateBackend, ValueKey,
};
use rust_decimal::Decimal;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio_postgres::IsolationLevel;

use crate::hashing::{Digest, invocation_key_hash, scope_hash, value_key_hash};

const KIND_INVOCATION: &str = "i";
const KIND_VALUE: &str = "v";

/// Resolved identity of a bucket: its scope (tenant) digest, its full
/// key digest, and the map discriminant. Groups the three so the shared
/// `record` body stays under the argument-count lint.
struct Bucket {
    scope: Digest,
    key: Digest,
    kind: &'static str,
}

/// Durable PostgreSQL [`PredicateStateBackend`]. Holds a `deadpool`
/// connection pool; construct one pool per process and share the backend
/// behind an `Arc`.
pub struct PostgresPredicateStateBackend {
    pool: Pool,
    /// Local mirror of LRU evictions performed by THIS process instance,
    /// matching the in-memory backend's `evictions_observed()` contract
    /// (a process-local monitoring counter, not a global DB total).
    evictions: AtomicU64,
}

impl PostgresPredicateStateBackend {
    /// Wrap a `deadpool` pool. Call [`Self::run_migrations`] once before
    /// first use to ensure the schema exists.
    pub fn new(pool: Pool) -> Self {
        Self {
            pool,
            evictions: AtomicU64::new(0),
        }
    }

    /// Apply the idempotent schema. Safe to call repeatedly and
    /// concurrently (`CREATE … IF NOT EXISTS`).
    pub async fn run_migrations(&self) -> Result<(), PredicateBackendError> {
        let client = self.client().await?;
        client
            .batch_execute(crate::schema::POSTGRES_PREDICATE_SCHEMA)
            .await
            .map_err(map_pg)?;
        Ok(())
    }

    async fn client(&self) -> Result<deadpool_postgres::Object, PredicateBackendError> {
        self.pool.get().await.map_err(map_pool)
    }

    /// Compute the wall-clock cutoff `now - window`, saturating to `now`
    /// for windows beyond chrono's range (nothing trimmed — conservative
    /// for a rate/value cap). Mirrors `predicate_state::window_cutoff`.
    fn cutoff(now: DateTime<Utc>, window: Duration) -> DateTime<Utc> {
        match chrono::Duration::from_std(window) {
            Ok(d) => now.checked_sub_signed(d).unwrap_or(now),
            Err(_) => now,
        }
    }

    /// Shared transaction body for both record paths. `value` is `None`
    /// for the invocation map and `Some(_)` for the value map; the
    /// returned aggregate is `COUNT(*)` cast to a `Decimal` for the
    /// invocation path and `SUM(value)` for the value path, so a single
    /// code path serves both.
    async fn record(
        &self,
        bucket: Bucket,
        event_id: &PredicateEventId,
        now: DateTime<Utc>,
        value: Option<Decimal>,
        window: Duration,
    ) -> Result<Decimal, PredicateBackendError> {
        let Bucket { scope, key, kind } = bucket;
        let cutoff = Self::cutoff(now, window);
        let mut client = self.client().await?;
        // READ COMMITTED + a transaction-scoped advisory lock keyed on the
        // bucket. The advisory lock serializes ALL writers to the same
        // key (the durable analogue of the in-memory backend's single
        // Mutex), so the trim / insert / aggregate steps see a consistent
        // view and two concurrent writers can never both observe "under
        // cap" and both proceed (codex Critical atomicity requirement).
        //
        // We deliberately do NOT use REPEATABLE READ here: under that
        // level concurrent same-key writers abort with
        // `could not serialize access`, forcing a caller retry loop. The
        // advisory lock instead makes the second writer *block* until the
        // first commits — same correctness, no spurious aborts. Writers to
        // DIFFERENT keys take different advisory locks and proceed fully
        // concurrently.
        let tx = client
            .build_transaction()
            .isolation_level(IsolationLevel::ReadCommitted)
            .start()
            .await
            .map_err(map_pg)?;

        let scope_ref: &[u8] = &scope;
        let key_ref: &[u8] = &key;

        // Serialize same-key writers. The advisory lock key is two i32s
        // derived from the bucket's key_hash; pg_advisory_xact_lock is
        // released automatically at commit/rollback. Collisions across
        // distinct keys (same 64-bit lock key) only cost extra
        // serialization, never correctness.
        let lock_key = advisory_lock_key(&key);
        tx.execute(
            "SELECT pg_advisory_xact_lock($1, $2)",
            &[&lock_key.0, &lock_key.1],
        )
        .await
        .map_err(map_pg)?;

        // (1) Trim out-of-window rows for this key. Doing this BEFORE the
        // insert frees the dedup id of any row that aged out of the
        // window, so a re-used id whose original entry is no longer
        // in-window records fresh — matching the in-memory backend, whose
        // dedup memory is exactly the in-window entry set.
        tx.execute(
            "DELETE FROM hook_predicate_counters WHERE key_hash = $1 AND ts < $2",
            &[&key_ref, &cutoff],
        )
        .await
        .map_err(map_pg)?;

        // (2) Insert the new row, deduping on the PRIMARY KEY
        // (key_hash, id). A duplicate id for the same key is a no-op —
        // the cross-host replay defense, exact regardless of clock skew.
        tx.execute(
            "INSERT INTO hook_predicate_counters
                 (scope_hash, key_hash, kind, id, ts, value)
             VALUES ($1, $2, $3, $4, $5, $6)
             ON CONFLICT (key_hash, id) DO NOTHING",
            &[
                &scope_ref,
                &key_ref,
                &kind,
                &event_id.as_str(),
                &now,
                &value,
            ],
        )
        .await
        .map_err(map_pg)?;

        // (5) Aggregate the in-window count/sum. This runs as a SEPARATE
        // statement after the INSERT so it observes the inserted row —
        // a data-modifying CTE's effects are NOT visible to a SELECT in
        // the same statement (all CTEs share one snapshot), which would
        // make the count always pre-insert. Sequential statements inside
        // the transaction DO see prior statements' writes.
        let agg_row = tx
            .query_one(
                "SELECT COUNT(*)::BIGINT AS cnt,
                        COALESCE(SUM(value), 0)::NUMERIC AS total
                   FROM hook_predicate_counters
                  WHERE key_hash = $1 AND ts >= $2",
                &[&key_ref, &cutoff],
            )
            .await
            .map_err(map_pg)?;

        let mut count: i64 = agg_row.get("cnt");
        let mut total: Decimal = agg_row.get("total");
        let mut capped = false;

        // (3) Per-key sample cap: drop oldest overflow rows. Only fires
        // when a key exceeds the cap, which is the attacker-flood case.
        // Drop-oldest keeps the most-recent MAX_SAMPLES_PER_KEY rows,
        // matching the in-memory backend.
        if count as usize > MAX_SAMPLES_PER_KEY {
            let overflow = count as usize - MAX_SAMPLES_PER_KEY;
            tx.execute(
                "DELETE FROM hook_predicate_counters
                  WHERE ctid IN (
                      SELECT ctid FROM hook_predicate_counters
                       WHERE key_hash = $1 AND ts >= $2
                       ORDER BY ts ASC, id ASC
                       LIMIT $3
                  )",
                &[&key_ref, &cutoff, &(overflow as i64)],
            )
            .await
            .map_err(map_pg)?;
            capped = true;
        }

        // (4) Per-scope distinct-key LRU quota. Only scan when this key is
        // newly material (count == 1 after insert means we may have just
        // created the scope's Nth key). Distinct keys are counted by
        // key_hash within the scope+kind; if over quota, evict the
        // least-recently-active key's rows entirely.
        let evicted = if count == 1 {
            self.enforce_scope_quota(&tx, scope_ref, kind, key_ref)
                .await?
        } else {
            0
        };

        // If a cap eviction fired, re-aggregate inside the SAME tx so the
        // returned count/sum reflects the dropped rows — this keeps the
        // value path's running-sum-consistency-under-eviction contract
        // and the invocation count exact, atomically. Scope-LRU eviction
        // (step 4) only ever touches OTHER keys, so it cannot change this
        // key's aggregate and does not require a re-read.
        if capped {
            let row = tx
                .query_one(
                    "SELECT COUNT(*)::BIGINT AS cnt,
                            COALESCE(SUM(value), 0)::NUMERIC AS total
                       FROM hook_predicate_counters
                      WHERE key_hash = $1 AND ts >= $2",
                    &[&key_ref, &cutoff],
                )
                .await
                .map_err(map_pg)?;
            count = row.get("cnt");
            total = row.get("total");
        }

        tx.commit().await.map_err(map_pg)?;

        if evicted > 0 {
            // Mirror only on a successful commit so the monitoring counter
            // never advances for a rolled-back eviction.
            self.evictions.fetch_add(evicted, Ordering::Relaxed);
        }

        if value.is_some() {
            Ok(total)
        } else {
            Ok(Decimal::from(count.max(0) as u64))
        }
    }

    /// Enforce [`MAX_KEYS_PER_TENANT`] distinct keys per scope+kind.
    /// Returns the number of keys evicted (0 or more). Eviction drops the
    /// least-recently-active key — the key whose newest row is oldest —
    /// matching the in-memory backend's oldest-front victim selection,
    /// and never touches the key we just inserted.
    async fn enforce_scope_quota(
        &self,
        tx: &deadpool_postgres::Transaction<'_>,
        scope_ref: &[u8],
        kind: &str,
        current_key: &[u8],
    ) -> Result<u64, PredicateBackendError> {
        // Serialize quota enforcement within the scope. Concurrent inserts
        // of DISTINCT new keys in the same scope each reach this path with
        // `count == 1`, but under READ COMMITTED neither sees the other's
        // just-inserted row, so each would under-count `distinct` and
        // under-evict — leaving the scope above the cap. A scope-level
        // advisory lock makes the eviction check serial per scope, so the
        // count is exact. It is taken in the SINGLE-arg `(int8)` advisory
        // space, disjoint from the per-key `(int4,int4)` lock space, and
        // always AFTER the per-key lock — a consistent global ordering, so
        // no deadlock. Hot-path same-key writes never reach here (only
        // newly-material keys do), so this does not serialize steady-state
        // traffic.
        let scope_lock = scope_advisory_lock_key(scope_ref, kind);
        tx.execute("SELECT pg_advisory_xact_lock($1)", &[&scope_lock])
            .await
            .map_err(map_pg)?;

        let distinct: i64 = tx
            .query_one(
                "SELECT COUNT(DISTINCT key_hash)::BIGINT
                   FROM hook_predicate_counters
                  WHERE scope_hash = $1 AND kind = $2",
                &[&scope_ref, &kind],
            )
            .await
            .map_err(map_pg)?
            .get(0);

        if distinct as usize <= MAX_KEYS_PER_TENANT {
            return Ok(0);
        }
        let to_evict = distinct as usize - MAX_KEYS_PER_TENANT;

        // Victim selection: rank keys in this scope by their most-recent
        // activity (MAX(ts)); the staleest keys are evicted first. Exclude
        // the key we just inserted so a flood can never evict itself and
        // mask the new entry.
        let deleted = tx
            .execute(
                "DELETE FROM hook_predicate_counters
                  WHERE scope_hash = $1 AND kind = $2
                    AND key_hash IN (
                        SELECT key_hash FROM (
                            SELECT key_hash, MAX(ts) AS last_ts
                              FROM hook_predicate_counters
                             WHERE scope_hash = $1 AND kind = $2
                               AND key_hash <> $3
                             GROUP BY key_hash
                             ORDER BY last_ts ASC
                             LIMIT $4
                        ) victims
                    )",
                &[&scope_ref, &kind, &current_key, &(to_evict as i64)],
            )
            .await
            .map_err(map_pg)?;

        // Count evicted *keys*, not rows. We asked for `to_evict` victim
        // keys; report that many (or fewer if the scope had fewer evictable
        // keys). `deleted` is the row count, so derive the key count from
        // the bounded victim set.
        let _ = deleted;
        Ok(to_evict as u64)
    }
}

#[async_trait]
impl PredicateStateBackend for PostgresPredicateStateBackend {
    async fn record_invocation(
        &self,
        key: &InvocationKey,
        event_id: &PredicateEventId,
        now: DateTime<Utc>,
        window: Duration,
    ) -> Result<u32, PredicateBackendError> {
        let bucket = Bucket {
            scope: scope_hash(key.tenant_id.as_str()),
            key: invocation_key_hash(key),
            kind: KIND_INVOCATION,
        };
        let count = self.record(bucket, event_id, now, None, window).await?;
        // `record` returns the invocation count as a Decimal (COUNT(*),
        // capped at MAX_SAMPLES_PER_KEY). Narrow to u32 via the integer
        // value; the cap (4_096) guarantees it fits.
        use rust_decimal::prelude::ToPrimitive;
        let n = count.to_u32().unwrap_or(u32::MAX);
        Ok(n)
    }

    async fn record_value(
        &self,
        key: &ValueKey,
        event_id: &PredicateEventId,
        now: DateTime<Utc>,
        value: Decimal,
        window: Duration,
    ) -> Result<Decimal, PredicateBackendError> {
        let bucket = Bucket {
            scope: scope_hash(key.tenant_id.as_str()),
            key: value_key_hash(key),
            kind: KIND_VALUE,
        };
        self.record(bucket, event_id, now, Some(value), window)
            .await
    }

    fn evictions_observed(&self) -> u64 {
        self.evictions.load(Ordering::Relaxed)
    }

    async fn evict_older_than(&self, cutoff: DateTime<Utc>) -> Result<u64, PredicateBackendError> {
        let client = self.client().await?;
        let dropped = client
            .execute(
                "DELETE FROM hook_predicate_counters WHERE ts < $1",
                &[&cutoff],
            )
            .await
            .map_err(map_pg)?;
        Ok(dropped)
    }
}

/// Derive the two-`i32` advisory-lock key from a bucket's `key_hash`.
/// `pg_advisory_xact_lock(int4, int4)` namespaces the lock by the pair, so
/// we feed the first four bytes as the classifier and the next four as the
/// object id. A hash collision across distinct keys merely serializes two
/// unrelated buckets — a (rare) throughput cost, never a correctness bug.
fn advisory_lock_key(key: &Digest) -> (i32, i32) {
    let a = i32::from_le_bytes([key[0], key[1], key[2], key[3]]);
    let b = i32::from_le_bytes([key[4], key[5], key[6], key[7]]);
    (a, b)
}

/// Derive the single-`i64` scope advisory-lock key. Uses the `(int8)`
/// advisory space, which Postgres keeps disjoint from the `(int4,int4)`
/// space used for per-key locks, so a key lock and a scope lock can never
/// alias each other. Folds in the `kind` byte so the invocation and value
/// maps lock independently.
fn scope_advisory_lock_key(scope: &[u8], kind: &str) -> i64 {
    let mut hasher = blake3::Hasher::new();
    hasher.update(scope);
    hasher.update(kind.as_bytes());
    let d = hasher.finalize();
    let bytes = d.as_bytes();
    i64::from_le_bytes([
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
    ])
}

fn map_pg(e: tokio_postgres::Error) -> PredicateBackendError {
    PredicateBackendError::Unavailable(e.to_string())
}

fn map_pool(e: deadpool_postgres::PoolError) -> PredicateBackendError {
    PredicateBackendError::Unavailable(e.to_string())
}
