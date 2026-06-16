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
//! 1. **Trims** rows for the key with `occurred_at < cutoff` (out of
//!    window). This also frees the dedup `event_id` of any trimmed row, so
//!    an `event_id` that aged out of the window can be re-recorded —
//!    matching the in-memory backend, whose dedup memory is exactly the
//!    in-window entry set.
//! 2. **Dedup-checks** the incoming `event_id` against the in-window rows
//!    for the key. A match is a replay — a no-op that short-circuits before
//!    the cap check and returns the unchanged aggregate. This is the
//!    cross-host replay defense (a row written by host A blocks host B's
//!    re-insert of the same id regardless of clock skew) AND the property
//!    that lets a replay survive the cap boundary.
//! 3. **Caps fail-closed**: if the `event_id` is new (not a replay) and the
//!    in-window count is already at [`MAX_SAMPLES_PER_KEY`], the call
//!    returns [`PredicateBackendError::WindowOverflow`] WITHOUT inserting —
//!    matching the in-memory backend's `if !dedup && len >= cap { Err }`
//!    contract exactly. Silently evicting the oldest sample would weaken
//!    cap enforcement and break replay refusal (the evicted id would leave
//!    the dedup set while still logically in-window), so we fail closed.
//! 4. **Inserts** the new row `ON CONFLICT (key_hash, event_id) DO NOTHING`
//!    into the kind-specific table (`hooks_predicate_invocations` for
//!    counts, `hooks_predicate_values` for numeric sums — explicit typed
//!    tables, not a generic `kind` discriminator column).
//! 5. **Evicts** the scope's oldest-front key across both typed tables — the
//!    key whose oldest retained sample (`MIN(occurred_at)`) is oldest — when
//!    the scope's aggregate distinct-key count exceeds
//!    [`MAX_KEYS_PER_TENANT`] (the durable analogue of the in-memory
//!    per-tenant LRU quota, which ranks buckets by their oldest entry). Each
//!    victim's rows
//!    are deleted only after acquiring that victim key's per-key advisory
//!    lock with the NON-blocking `pg_try_advisory_xact_lock`, so eviction
//!    obeys the same per-bucket serialization as a recorder and can never
//!    deadlock against (or tear the aggregate of) a concurrent write to the
//!    victim bucket.
//! 6. **Aggregates** the in-window `COUNT(*)` (invocation table) / `SUM(value)`
//!    (value table) and returns it.
//!
//! Steps 1-6 share one transaction under the bucket advisory lock, so two
//! concurrent writers can never both observe "1 under cap" and both
//! proceed — the second blocks until the first commits. This is the codex
//! Critical atomicity requirement from PR #3635.
//!
//! # Cap semantics — fail-closed, NOT drop-oldest
//!
//! When a key's in-window sample count reaches [`MAX_SAMPLES_PER_KEY`] and
//! a NEW distinct id arrives, the backend returns
//! [`PredicateBackendError::WindowOverflow`] rather than evicting the
//! oldest sample to make room. This matches the in-memory backend and the
//! trait contract (PR #3635 followup / #3929): the evaluator maps the error
//! to a restrictive DENY/PauseApproval, so overflow surfaces as a refusal,
//! never a silent Allow. Replay of an already-recorded in-window id still
//! short-circuits to a no-op before the cap check (step 2), so replay
//! refusal survives the cap boundary.

use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use deadpool_postgres::Pool;
use ironclaw_hooks::predicate_state::{
    InvocationKey, MAX_KEYS_PER_TENANT, MAX_SAMPLES_PER_KEY, PredicateBackendError,
    PredicateEventId, PredicateStateBackend, ValueKey, window_cutoff,
};
use rust_decimal::Decimal;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio_postgres::IsolationLevel;

use crate::hashing::{Digest, invocation_key_hash, scope_hash, value_key_hash};
use crate::schema::{INVOCATIONS_TABLE, VALUES_TABLE};

/// Which typed table a record targets. Replaces the old `kind CHAR(1)`
/// discriminator column: the table identity now carries the
/// invocation-vs-value distinction, and the value table has a NOT NULL
/// `value` column so the old `value: Option<Decimal>` "count smuggled
/// through a NUMERIC" double-duty is gone.
#[derive(Clone, Copy, PartialEq, Eq)]
enum RecordKind {
    /// `hooks_predicate_invocations`; aggregate is `COUNT(*)`.
    Invocation,
    /// `hooks_predicate_values`; aggregate is `SUM(value)`.
    Value,
}

impl RecordKind {
    /// The SQL table name this kind records into.
    fn table(self) -> &'static str {
        match self {
            RecordKind::Invocation => INVOCATIONS_TABLE,
            RecordKind::Value => VALUES_TABLE,
        }
    }
}

/// Resolved identity of a bucket: its scope (tenant) digest, its full key
/// digest, and which typed table it targets. Groups the three so the shared
/// `record` body stays under the argument-count lint.
struct Bucket {
    scope: Digest,
    key: Digest,
    kind: RecordKind,
    /// Human-readable label for the bucket, used only in the
    /// `WindowOverflow` error. Mirrors the in-memory backend's format:
    /// `{tenant}/{capability}` for invocations and
    /// `{tenant}/{capability}#{field}` for values.
    label: String,
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

    /// Compute the wall-clock cutoff `now - window`. Delegates to the
    /// **canonical** [`ironclaw_hooks::predicate_state::window_cutoff`] (the
    /// same function the libSQL backend uses) rather than reimplementing the
    /// `Duration → cutoff` math, so the overflow/boundary behaviour — saturate
    /// to `now` for windows beyond chrono's range, trim `occurred_at < cutoff`
    /// strictly — is byte-for-byte identical across backends and can't drift.
    fn cutoff(now: DateTime<Utc>, window: Duration) -> DateTime<Utc> {
        window_cutoff(now, window)
    }

    /// Shared transaction body for both record paths. The trim / dedup / cap
    /// / quota steps are identical across the two typed tables; only the
    /// INSERT column list and the final aggregate differ, expressed via
    /// `value` (`None` ⇒ invocation table, `COUNT(*)`; `Some(_)` ⇒ value
    /// table, `SUM(value)`). The `value` arg is NOT a smuggled count — for
    /// the invocation table it is genuinely absent (the table has no `value`
    /// column), and for the value table it is the real recorded numeric. The
    /// caller-facing `record_invocation`/`record_value` methods map the
    /// returned aggregate to their typed return.
    async fn record(
        &self,
        bucket: Bucket,
        event_id: &PredicateEventId,
        now: DateTime<Utc>,
        value: Option<Decimal>,
        window: Duration,
    ) -> Result<Decimal, PredicateBackendError> {
        let Bucket {
            scope,
            key,
            kind,
            label,
        } = bucket;
        debug_assert_eq!(
            value.is_some(),
            kind == RecordKind::Value,
            "value presence must match the value table"
        );
        let table = kind.table();
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
            &format!("DELETE FROM {table} WHERE key_hash = $1 AND occurred_at < $2"),
            &[&key_ref, &cutoff],
        )
        .await
        .map_err(map_pg)?;

        // (2) Replay-dedup check + pre-insert count, computed atomically in
        // one statement under the advisory lock. `cnt` is the in-window
        // sample count BEFORE this call's insert; `dup` is whether the
        // incoming id is already recorded in-window for this key. A replay
        // (`dup = true`) short-circuits to a no-op below — matching the
        // in-memory backend's `if !dedup_ids.contains(event_id)` guard.
        let pre_row = tx
            .query_one(
                &format!(
                    "SELECT COUNT(*)::BIGINT AS cnt,
                            BOOL_OR(event_id = $3) AS dup
                       FROM {table}
                      WHERE key_hash = $1 AND occurred_at >= $2"
                ),
                &[&key_ref, &cutoff, &event_id.as_str()],
            )
            .await
            .map_err(map_pg)?;
        let pre_count: i64 = pre_row.get("cnt");
        // BOOL_OR over an empty set is NULL; treat NULL as "no duplicate".
        let is_replay: bool = pre_row.get::<_, Option<bool>>("dup").unwrap_or(false);

        if is_replay {
            // Replay refusal: the id is already in-window for this key, so
            // this is a no-op against the count/sum. Short-circuit BEFORE
            // the cap check so a replay at the cap dedups rather than
            // overflowing — matching the in-memory contract. Aggregate and
            // return the unchanged state.
            let agg = self.aggregate(&tx, kind, key_ref, &cutoff).await?;
            tx.commit().await.map_err(map_pg)?;
            return Ok(agg);
        }

        // (3) Per-key sample cap — FAIL CLOSED. The id is new (not a
        // replay). If the in-window count is already at the cap, refuse to
        // insert and return `WindowOverflow` (PR #3635 followup / #3929).
        // Silently dropping the oldest sample to make room would weaken cap
        // enforcement and break replay refusal — so we fail closed,
        // matching the in-memory backend's `if !dedup && len >= cap { Err }`.
        if pre_count as usize >= MAX_SAMPLES_PER_KEY {
            // Roll back so the trim above (which freed aged-out dedup ids)
            // is not committed independently of a rejected record; the
            // caller observes a clean no-write overflow.
            drop(tx);
            return Err(PredicateBackendError::WindowOverflow {
                key: label,
                cap: MAX_SAMPLES_PER_KEY,
            });
        }

        // (4) Insert the new row, deduping on the PRIMARY KEY
        // (key_hash, event_id) as belt-and-suspenders against a concurrent
        // racer that inserted the same id between our dedup check and here
        // (the advisory lock serializes same-key writers, so this conflict
        // is not expected, but ON CONFLICT keeps it a no-op if it occurs).
        // The column list differs per typed table: the invocation table has
        // no `value` column; the value table's `value` is NOT NULL.
        match value {
            None => {
                tx.execute(
                    &format!(
                        "INSERT INTO {table} (scope_hash, key_hash, event_id, occurred_at)
                         VALUES ($1, $2, $3, $4)
                         ON CONFLICT (key_hash, event_id) DO NOTHING"
                    ),
                    &[&scope_ref, &key_ref, &event_id.as_str(), &now],
                )
                .await
                .map_err(map_pg)?;
            }
            Some(v) => {
                tx.execute(
                    &format!(
                        "INSERT INTO {table} (scope_hash, key_hash, event_id, occurred_at, value)
                         VALUES ($1, $2, $3, $4, $5)
                         ON CONFLICT (key_hash, event_id) DO NOTHING"
                    ),
                    &[&scope_ref, &key_ref, &event_id.as_str(), &now, &v],
                )
                .await
                .map_err(map_pg)?;
            }
        }

        // (6) Aggregate the in-window count/sum. This runs as a SEPARATE
        // statement after the INSERT so it observes the inserted row —
        // a data-modifying CTE's effects are NOT visible to a SELECT in
        // the same statement (all CTEs share one snapshot), which would
        // make the count always pre-insert. Sequential statements inside
        // the transaction DO see prior statements' writes.
        //
        // The current key's in-window count is needed independently of the
        // returned aggregate (the value table's aggregate is a SUM, not a
        // count), so read it explicitly to detect when this record created a
        // brand-new bucket and should run the aggregate tenant quota pass.
        let in_window_count: i64 = tx
            .query_one(
                &format!(
                    "SELECT COUNT(*)::BIGINT FROM {table}
                      WHERE key_hash = $1 AND occurred_at >= $2"
                ),
                &[&key_ref, &cutoff],
            )
            .await
            .map_err(map_pg)?
            .get(0);

        // (5) Per-scope aggregate distinct-key LRU quota. Only scan when this
        // key is newly material (count == 1 after insert means we may have
        // just created the scope's Nth key). Distinct keys are counted by
        // key_hash within the scope across both typed tables; if over quota,
        // evict the least-recently-active key's rows entirely. Scope-LRU
        // eviction only ever touches OTHER keys, so it cannot change this
        // key's aggregate and does not require a re-read.
        let evicted = if in_window_count == 1 {
            self.enforce_scope_quota(&tx, scope_ref, key_ref).await?
        } else {
            0
        };

        // Final returned aggregate (COUNT for invocations, SUM for values).
        let agg = self.aggregate(&tx, kind, key_ref, &cutoff).await?;

        tx.commit().await.map_err(map_pg)?;

        if evicted > 0 {
            // Mirror only on a successful commit so the monitoring counter
            // never advances for a rolled-back eviction.
            self.evictions.fetch_add(evicted, Ordering::Relaxed);
        }

        Ok(agg)
    }

    /// In-window aggregate for a key: `COUNT(*)` (as a `Decimal`) for the
    /// invocation table, `SUM(value)` for the value table. Centralizes the
    /// per-table aggregate SQL so the invocation table — which has no `value`
    /// column — is never asked to `SUM(value)`.
    async fn aggregate(
        &self,
        tx: &deadpool_postgres::Transaction<'_>,
        kind: RecordKind,
        key_ref: &[u8],
        cutoff: &DateTime<Utc>,
    ) -> Result<Decimal, PredicateBackendError> {
        let table = kind.table();
        match kind {
            RecordKind::Invocation => {
                let count: i64 = tx
                    .query_one(
                        &format!(
                            "SELECT COUNT(*)::BIGINT FROM {table}
                              WHERE key_hash = $1 AND occurred_at >= $2"
                        ),
                        &[&key_ref, &cutoff],
                    )
                    .await
                    .map_err(map_pg)?
                    .get(0);
                Ok(Decimal::from(count.max(0) as u64))
            }
            RecordKind::Value => {
                let total: Decimal = tx
                    .query_one(
                        &format!(
                            "SELECT COALESCE(SUM(value), 0)::NUMERIC FROM {table}
                              WHERE key_hash = $1 AND occurred_at >= $2"
                        ),
                        &[&key_ref, &cutoff],
                    )
                    .await
                    .map_err(map_pg)?
                    .get(0);
                Ok(total)
            }
        }
    }

    /// Enforce [`MAX_KEYS_PER_TENANT`] distinct keys per scope across both
    /// typed predicate tables.
    /// Returns the number of keys evicted (0 or more). Eviction drops the
    /// key whose OLDEST retained sample is oldest (`MIN(ts)` per key) —
    /// the "oldest-front" victim selection, matching the in-memory backend
    /// (which ranks buckets by their front/oldest entry) and the libSQL
    /// backend. It never touches the key we just inserted.
    ///
    /// # Reaper requirement — quota counts un-reaped expired rows
    ///
    /// The unioned distinct-key count below counts EVERY stored row for the
    /// scope across both tables, including expired rows from OTHER keys: the
    /// per-key window trim in `record_*` only deletes the current key's
    /// out-of-window rows, never sibling keys. So a tenant with many idle
    /// short-window keys can read at `MAX_KEYS_PER_TENANT` and trip LRU
    /// eviction (advancing `evictions_observed`) even though its *active* key
    /// count is lower. The evicted keys are already expired, so gate
    /// correctness is unaffected, but operators MUST schedule a periodic
    /// [`PredicateStateBackend::evict_older_than`] reaper to keep the quota
    /// aligned with the active key count. See
    /// `ironclaw_hooks/docs/successors/03-persistent-counter.md` (Reaper
    /// requirement). This is NOT fixed by a behavior change here: counting only
    /// in-window rows would require a per-key window the quota does not have.
    ///
    /// [`PredicateStateBackend::evict_older_than`]: ironclaw_hooks::predicate_state::PredicateStateBackend::evict_older_than
    ///
    /// # Lock discipline for victim eviction (deadlock + race fix)
    ///
    /// Deleting a victim key's rows is itself a write to that bucket, so it
    /// MUST participate in the per-key advisory-lock serialization just like
    /// a `record` call would — otherwise this pass could delete rows for
    /// key B while another transaction is concurrently recording B under
    /// B's own per-key lock, producing a torn aggregate (the recorder's
    /// COUNT/SUM straddling a delete it never serialized against) and a
    /// deadlock:
    ///
    /// - Txn V (recording victim key B): holds B's per-key lock (it trimmed
    ///   B's out-of-window rows), then *blocks* waiting for the scope lock
    ///   inside its own `enforce_scope_quota`.
    /// - Txn L (this LRU pass): holds the scope lock, then *blocks* waiting
    ///   on B's row locks to DELETE them.
    /// - Cycle: V waits on the scope lock held by L; L waits on B's row
    ///   locks held by V → deadlock.
    ///
    /// The fix: before deleting a victim's rows, `pg_try_advisory_xact_lock`
    /// that victim's per-key lock. The *try* variant returns immediately
    /// (never blocks), so the cycle above can never form — L observes B's
    /// lock held by V and skips B instead of waiting. Skipped (in-flight)
    /// victims are passed over for the next-staleest candidate. We over-fetch
    /// candidates so skips don't leave us under quota.
    async fn enforce_scope_quota(
        &self,
        tx: &deadpool_postgres::Transaction<'_>,
        scope_ref: &[u8],
        current_key: &[u8],
    ) -> Result<u64, PredicateBackendError> {
        // Serialize quota enforcement within the scope. Concurrent inserts
        // of DISTINCT new keys in the same scope each reach this path with
        // `count == 1`, but under READ COMMITTED neither sees the other's
        // just-inserted row, so each would under-count `distinct` and
        // under-evict — leaving the scope above the cap. A scope-level
        // advisory lock makes the eviction check serial per scope, so the
        // count is exact. It is taken in the SINGLE-arg `(int8)` advisory
        // space, disjoint from the per-key `(int4,int4)` lock space. Hot-path
        // same-key writes never reach here (only newly-material keys do), so
        // this does not serialize steady-state traffic.
        let scope_lock = scope_advisory_lock_key(scope_ref);
        tx.execute("SELECT pg_advisory_xact_lock($1)", &[&scope_lock])
            .await
            .map_err(map_pg)?;

        let distinct: i64 = tx
            .query_one(
                &format!(
                    "SELECT COUNT(*)::BIGINT FROM (
                         SELECT key_hash FROM {INVOCATIONS_TABLE} WHERE scope_hash = $1
                         UNION
                         SELECT key_hash FROM {VALUES_TABLE} WHERE scope_hash = $1
                     ) tenant_keys"
                ),
                &[&scope_ref],
            )
            .await
            .map_err(map_pg)?
            .get(0);

        if distinct as usize <= MAX_KEYS_PER_TENANT {
            return Ok(0);
        }
        let to_evict = distinct as usize - MAX_KEYS_PER_TENANT;

        // Victim candidates: rank keys in this scope by their OLDEST retained
        // sample (MIN(ts)) and evict the key whose oldest sample is oldest —
        // "oldest-front" selection, matching the in-memory and libSQL
        // backends. The in-memory backend ranks buckets by their front
        // (oldest) entry's timestamp (`entries.front()` + `min_by_key`), so
        // the durable analogue is MIN(ts) per key, NOT MAX(ts). Using MAX(ts)
        // here would diverge: a key with one ancient sample and one fresh
        // sample would be ranked by the fresh sample and spared, while the
        // in-memory backend ranks it by the ancient sample and evicts it. The
        // single-sample-per-key parity matrix masks this (MIN == MAX), but
        // multi-sample keys would evict different keys across backends.
        //
        // Exclude the key we just inserted so a flood can never evict itself
        // and mask the new entry. We over-fetch beyond `to_evict` so that if
        // some candidates are in-flight under their own per-key lock (try-lock
        // fails, see below) we can fall through to the next-oldest key and
        // still meet the quota. Bound the over-fetch so a pathological scope
        // can't pull an unbounded candidate set into memory.
        const CANDIDATE_OVERFETCH: i64 = 64;
        let candidate_limit = (to_evict as i64).saturating_add(CANDIDATE_OVERFETCH);
        let candidate_rows = tx
            .query(
                &format!(
                    "SELECT table_name, key_hash FROM (
                         SELECT 'invocation'::TEXT AS table_name,
                                key_hash,
                                MIN(occurred_at) AS oldest_ts
                           FROM {INVOCATIONS_TABLE}
                          WHERE scope_hash = $1
                            AND key_hash <> $2
                          GROUP BY key_hash
                         UNION ALL
                         SELECT 'value'::TEXT AS table_name,
                                key_hash,
                                MIN(occurred_at) AS oldest_ts
                           FROM {VALUES_TABLE}
                          WHERE scope_hash = $1
                            AND key_hash <> $2
                          GROUP BY key_hash
                     ) victims
                     ORDER BY oldest_ts ASC, table_name ASC, key_hash ASC
                     LIMIT $3"
                ),
                &[&scope_ref, &current_key, &candidate_limit],
            )
            .await
            .map_err(map_pg)?;

        // Evict victims one at a time, each under its own per-key advisory
        // lock taken with the NON-blocking `pg_try_advisory_xact_lock`. A
        // victim whose lock is already held (a concurrent `record` is
        // mid-flight against that bucket) is skipped — never waited on — so
        // this pass cannot deadlock against a recorder, and it never deletes
        // rows out from under a transaction that did not serialize against
        // us. Skipped victims simply stay in the scope until the next
        // newly-material insert reruns the quota check.
        let mut evicted = 0u64;
        for row in &candidate_rows {
            if evicted as usize >= to_evict {
                break;
            }
            let victim_table_name: String = row.get(0);
            let victim_key: Vec<u8> = row.get(1);
            let victim_table = match victim_table_name.as_str() {
                "invocation" => INVOCATIONS_TABLE,
                "value" => VALUES_TABLE,
                _ => continue,
            };
            let lock_key = advisory_lock_key_from_bytes(&victim_key);
            let got_lock: bool = tx
                .query_one(
                    "SELECT pg_try_advisory_xact_lock($1, $2)",
                    &[&lock_key.0, &lock_key.1],
                )
                .await
                .map_err(map_pg)?
                .get(0);
            if !got_lock {
                // In-flight under its own per-key lock; skip and try the
                // next-staleest candidate rather than block (deadlock-free).
                continue;
            }
            tx.execute(
                &format!("DELETE FROM {victim_table} WHERE scope_hash = $1 AND key_hash = $2"),
                &[&scope_ref, &victim_key],
            )
            .await
            .map_err(map_pg)?;
            evicted += 1;
        }

        Ok(evicted)
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
            kind: RecordKind::Invocation,
            label: format!("{}/{}", key.tenant_id.as_str(), key.capability),
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
            kind: RecordKind::Value,
            label: format!(
                "{}/{}#{}",
                key.tenant_id.as_str(),
                key.capability,
                key.field
            ),
        };
        self.record(bucket, event_id, now, Some(value), window)
            .await
    }

    fn evictions_observed(&self) -> u64 {
        self.evictions.load(Ordering::Relaxed)
    }

    async fn evict_older_than(&self, cutoff: DateTime<Utc>) -> Result<u64, PredicateBackendError> {
        let client = self.client().await?;
        // Reap both typed tables; return the total rows deleted.
        let inv = client
            .execute(
                &format!("DELETE FROM {INVOCATIONS_TABLE} WHERE occurred_at < $1"),
                &[&cutoff],
            )
            .await
            .map_err(map_pg)?;
        let val = client
            .execute(
                &format!("DELETE FROM {VALUES_TABLE} WHERE occurred_at < $1"),
                &[&cutoff],
            )
            .await
            .map_err(map_pg)?;
        Ok(inv + val)
    }
}

/// Derive the two-`i32` advisory-lock key from a bucket's `key_hash`.
/// `pg_advisory_xact_lock(int4, int4)` namespaces the lock by the pair, so
/// we feed the first four bytes as the classifier and the next four as the
/// object id. A hash collision across distinct keys merely serializes two
/// unrelated buckets — a (rare) throughput cost, never a correctness bug.
fn advisory_lock_key(key: &Digest) -> (i32, i32) {
    advisory_lock_key_from_bytes(key)
}

/// Same derivation as [`advisory_lock_key`] but over a raw byte slice — used
/// by the scope-LRU eviction path, which reads candidate victims' `key_hash`
/// back from the `BYTEA` column as bytes (not a typed [`Digest`]). It MUST
/// produce the identical `(i32, i32)` lock key the recording path uses for
/// the same bucket, otherwise the victim try-lock would guard a different
/// lock than the recorder holds and the serialization would be defeated.
/// `key_hash` is always a 32-byte blake3 digest, so the first 8 bytes are
/// present; a shorter slice (never expected) is zero-padded so the function
/// is total rather than panicking on an out-of-range index.
fn advisory_lock_key_from_bytes(key: &[u8]) -> (i32, i32) {
    let mut buf = [0u8; 8];
    let n = key.len().min(8);
    buf[..n].copy_from_slice(&key[..n]);
    let a = i32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
    let b = i32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]);
    (a, b)
}

/// Derive the single-`i64` scope advisory-lock key. Uses the `(int8)`
/// advisory space, which Postgres keeps disjoint from the `(int4,int4)`
/// space used for per-key locks, so a key lock and a scope lock can never
/// alias each other. The lock is scope-wide across both predicate tables
/// because [`MAX_KEYS_PER_TENANT`] is an aggregate per-tenant quota.
fn scope_advisory_lock_key(scope: &[u8]) -> i64 {
    let mut hasher = blake3::Hasher::new();
    hasher.update(scope);
    hasher.update(b"predicate-scope");
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The scope-LRU eviction path try-locks each victim key using
    /// `advisory_lock_key_from_bytes` over the `key_hash` bytes it read back
    /// from the DB, while the recording path locks via `advisory_lock_key`
    /// over the typed `Digest`. If these two derivations ever diverged, the
    /// eviction would guard a DIFFERENT advisory lock than a concurrent
    /// recorder holds, defeating the per-bucket serialization the fix
    /// depends on. This pins them equal for the full 32-byte digest — a
    /// provable-by-inspection guard for the lock-acquisition invariant that
    /// does not need a live Postgres.
    #[test]
    fn eviction_and_record_derive_identical_per_key_lock() {
        let digest: Digest = {
            let mut d = [0u8; 32];
            for (i, b) in d.iter_mut().enumerate() {
                *b = (i as u8).wrapping_mul(7).wrapping_add(3);
            }
            d
        };
        let from_digest = advisory_lock_key(&digest);
        let from_bytes = advisory_lock_key_from_bytes(&digest[..]);
        assert_eq!(
            from_digest, from_bytes,
            "victim try-lock key must equal the recorder's lock key for the same bucket"
        );
    }

    /// Distinct buckets must (almost always) map to distinct per-key lock
    /// keys; a single differing leading byte must change the derived lock.
    #[test]
    fn distinct_digests_yield_distinct_lock_keys() {
        let mut a = [0u8; 32];
        let mut b = [0u8; 32];
        a[0] = 1;
        b[0] = 2;
        assert_ne!(advisory_lock_key(&a), advisory_lock_key(&b));
    }

    /// `advisory_lock_key_from_bytes` is total: a short slice (never
    /// expected from a 32-byte `key_hash`, but defensive) zero-pads rather
    /// than panicking on an out-of-range index.
    #[test]
    fn short_slice_zero_pads_without_panic() {
        assert_eq!(advisory_lock_key_from_bytes(&[]), (0, 0));
        assert_eq!(
            advisory_lock_key_from_bytes(&[0xFF]),
            (i32::from_le_bytes([0xFF, 0, 0, 0]), 0)
        );
    }
}
