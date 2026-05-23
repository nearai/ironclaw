//! Durable libSQL-backed [`PredicateStateBackend`] implementation.
//!
//! Mirrors the in-memory backend's invariants
//! (`ironclaw_hooks::predicate_state::InMemoryPredicateStateBackend`) against a
//! libSQL / SQLite database so predicate counter / value-sum state survives
//! process restart and is consistent across every host pointing at the same
//! database file. The schema lives in [`crate::schema`]; scope-hash derivation
//! in [`crate::hashing`].
//!
//! [`PredicateStateBackend`]: ironclaw_hooks::predicate_state::PredicateStateBackend
//!
//! # Clock basis (aligns with PR 2/4)
//!
//! The stored `occurred_at` is the **host-supplied `now: DateTime<Utc>`**
//! parameter (production passes [`chrono::Utc::now()`]), stored as epoch
//! milliseconds. We do NOT use SQLite `datetime('now')` for the row
//! timestamp: the [`PredicateStateBackend`] trait threads `now` in explicitly
//! so the contract harness can drive a deterministic clock, and window
//! trimming must be relative to that same clock to stay consistent with the
//! in-memory backend. Both durable backends store the host clock verbatim, so
//! they are byte-for-byte semantically identical. Epoch milliseconds (INTEGER)
//! is chosen over an ISO-8601 TEXT column so window arithmetic and ordering
//! are exact integer comparisons with sub-second resolution.
//!
//! # Atomicity
//!
//! Every `record_*` call runs inside a single `BEGIN IMMEDIATE` … `COMMIT`
//! transaction: `BEGIN IMMEDIATE` takes SQLite's write lock up front so
//! concurrent writers serialise (no read-modify-write race window — codex
//! Critical on PR #3635). The trim, dedup-insert, cap check, and the final
//! in-window count/sum read all happen under that one lock, so the write and
//! the returned value are atomic. libSQL's single-writer semantics mean "two
//! hosts" reduces to "two connections contending for the write lock";
//! `PRAGMA busy_timeout` lets the loser wait rather than fail.
//!
//! # Per-key cap: FAIL CLOSED (PR #3635 followup / #3929)
//!
//! When a scope already holds [`MAX_SAMPLES_PER_KEY`] in-window rows and a
//! NEW distinct `event_id` arrives, the call returns
//! [`PredicateBackendError::WindowOverflow`] rather than silently dropping the
//! oldest sample. Silent drop-oldest weakened cap enforcement (the count could
//! never exceed the cap, so an `InvocationCount { max >= cap }` predicate would
//! never fire) and broke replay refusal (the evicted id left the dedup set
//! while still logically in-window). A replay of an already-recorded id at the
//! cap dedups to a no-op BEFORE the overflow check, so replay refusal survives
//! the cap boundary. This matches the in-memory backend exactly.
//!
//! [`MAX_SAMPLES_PER_KEY`]: ironclaw_hooks::predicate_state::MAX_SAMPLES_PER_KEY
//! [`PredicateBackendError::WindowOverflow`]: ironclaw_hooks::predicate_state::PredicateBackendError::WindowOverflow
//!
//! # Running-sum consistency under eviction
//!
//! The value backend does not keep an external running sum; the in-window sum
//! is computed by summing the surviving rows inside the same transaction after
//! trimming, so it is always exactly the sum of the rows that survive —
//! consistency under eviction is structural.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ironclaw_hooks::predicate_state::{
    InvocationKey, MAX_HISTORY_KEYS, MAX_KEYS_PER_TENANT, MAX_SAMPLES_PER_KEY,
    PredicateBackendError, PredicateEventId, PredicateStateBackend, ValueKey,
};
use libsql::{Connection, params};
use rust_decimal::Decimal;

use crate::hashing::{
    invocation_scope_hash, to_epoch_millis, value_scope_hash, window_cutoff_millis,
};
use crate::schema::{INVOCATIONS_TABLE, LIBSQL_PREDICATE_STATE_SCHEMA, VALUES_TABLE};

/// Durable libSQL-backed predicate-state backend. Holds an
/// `Arc<libsql::Database>` and opens a fresh connection per operation — the
/// project's libSQL connection model (no pool; `PRAGMA busy_timeout` lets
/// concurrent writers wait on the single SQLite write lock).
pub struct LibSqlPredicateStateBackend {
    db: Arc<libsql::Database>,
    /// LRU evictions observed since construction. Persisted state can't be
    /// reconstructed across restart, so this counts evictions for THIS
    /// process instance (matching the in-memory backend's per-instance
    /// counter semantics).
    evictions: std::sync::atomic::AtomicU64,
}

impl LibSqlPredicateStateBackend {
    /// Construct over a shared libSQL database handle. Call
    /// [`Self::run_migrations`] once before first use.
    pub fn new(db: Arc<libsql::Database>) -> Self {
        Self {
            db,
            evictions: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// Open a fresh connection with the project-standard busy timeout so a
    /// concurrent writer holding the write lock makes us wait rather than
    /// fail immediately.
    async fn connect(&self) -> Result<Connection, PredicateBackendError> {
        let conn = self.db.connect().map_err(map_err)?;
        conn.query("PRAGMA busy_timeout = 5000", ())
            .await
            .map_err(map_err)?;
        Ok(conn)
    }

    /// Create the predicate-state tables and indexes if absent. Idempotent;
    /// wrapped in `BEGIN IMMEDIATE` so concurrent first-time migrations
    /// serialise. SQLite supports transactional DDL.
    pub async fn run_migrations(&self) -> Result<(), PredicateBackendError> {
        let conn = self.connect().await?;
        conn.execute("BEGIN IMMEDIATE", ()).await.map_err(map_err)?;
        let result = run_migrations_inner(&conn).await;
        match result {
            Ok(()) => conn
                .execute("COMMIT", ())
                .await
                .map(|_| ())
                .map_err(map_err),
            Err(err) => {
                let _ = conn.execute("ROLLBACK", ()).await;
                Err(err)
            }
        }
    }

    fn bump_evictions(&self, n: u64) {
        if n > 0 {
            self.evictions
                .fetch_add(n, std::sync::atomic::Ordering::Relaxed);
        }
    }
}

/// The libSQL migration body. Mirrors the schema documented in
/// [`crate::schema`]. Lives outside the `BEGIN IMMEDIATE` wrapper so the caller
/// owns the single rollback path (the established pattern in
/// `ironclaw_filesystem::libsql`).
async fn run_migrations_inner(conn: &Connection) -> Result<(), PredicateBackendError> {
    conn.execute_batch(LIBSQL_PREDICATE_STATE_SCHEMA)
        .await
        .map(|_| ())
        .map_err(map_err)
}

fn map_err(error: libsql::Error) -> PredicateBackendError {
    PredicateBackendError::Unavailable(error.to_string())
}

#[async_trait]
impl PredicateStateBackend for LibSqlPredicateStateBackend {
    async fn record_invocation(
        &self,
        key: &InvocationKey,
        event_id: &PredicateEventId,
        now: DateTime<Utc>,
        window: Duration,
    ) -> Result<u32, PredicateBackendError> {
        let scope = invocation_scope_hash(key);
        let cutoff = window_cutoff_millis(now, window);
        let now_ms = to_epoch_millis(now);
        let tenant = key.tenant_id.as_str().to_string();
        let overflow_key = format!("{}/{}", key.tenant_id.as_str(), key.capability);

        let conn = self.connect().await?;
        conn.execute("BEGIN IMMEDIATE", ()).await.map_err(map_err)?;

        let result = async {
            // 1. Trim entries outside the window for THIS scope first, so the
            //    per-key cap and the final count both see only in-window rows.
            conn.execute(
                &format!(
                    "DELETE FROM {INVOCATIONS_TABLE} WHERE scope_hash = ?1 AND occurred_at < ?2"
                ),
                params![scope.clone(), cutoff],
            )
            .await
            .map_err(map_err)?;

            // 2. Dedup short-circuit: a replayed id is a no-op against the
            //    count and must NOT trip the overflow check (replay refusal
            //    survives the cap boundary). If the id is already present for
            //    this scope, skip the insert + cap check entirely.
            let is_replay = event_id_exists(&conn, INVOCATIONS_TABLE, &scope, event_id).await?;

            let mut evicted = 0u64;
            if !is_replay {
                // 3. Per-key sample cap: FAIL CLOSED. If the scope is already at
                //    the cap with in-window rows, a NEW distinct id cannot be
                //    recorded without dropping an existing in-window sample, so
                //    reject (#3929) rather than silently evicting the oldest.
                let in_window = scope_in_window_count(&conn, INVOCATIONS_TABLE, &scope, cutoff).await?;
                if in_window as usize >= MAX_SAMPLES_PER_KEY {
                    return Err(PredicateBackendError::WindowOverflow {
                        key: overflow_key.clone(),
                        cap: MAX_SAMPLES_PER_KEY,
                    });
                }

                // 4. LRU + per-tenant quota: only relevant when inserting a NEW
                //    scope (a fresh PRIMARY KEY). Mirror the in-memory backend's
                //    "evict before insert when at cap" semantics.
                let scope_exists = scope_exists(&conn, INVOCATIONS_TABLE, &scope).await?;
                if !scope_exists {
                    evicted += enforce_caps(&conn, INVOCATIONS_TABLE, &tenant).await?;
                }

                // 5. Insert. The dedup short-circuit above already excludes
                //    replays, but keep ON CONFLICT DO NOTHING as a belt-and-
                //    suspenders guard against a concurrent insert of the same id.
                conn.execute(
                    &format!(
                        "INSERT INTO {INVOCATIONS_TABLE} (scope_hash, event_id, occurred_at, tenant_id) \
                         VALUES (?1, ?2, ?3, ?4) ON CONFLICT (scope_hash, event_id) DO NOTHING"
                    ),
                    params![scope.clone(), event_id.as_str(), now_ms, tenant.clone()],
                )
                .await
                .map_err(map_err)?;
            }

            // 6. In-window count read under the same lock.
            let count = scope_in_window_count(&conn, INVOCATIONS_TABLE, &scope, cutoff).await?;
            Ok::<(u32, u64), PredicateBackendError>((count, evicted))
        }
        .await;

        finish_txn(&conn, result, self).await
    }

    async fn record_value(
        &self,
        key: &ValueKey,
        event_id: &PredicateEventId,
        now: DateTime<Utc>,
        value: Decimal,
        window: Duration,
    ) -> Result<Decimal, PredicateBackendError> {
        let scope = value_scope_hash(key);
        let cutoff = window_cutoff_millis(now, window);
        let now_ms = to_epoch_millis(now);
        let tenant = key.tenant_id.as_str().to_string();
        let value_str = value.to_string();
        let overflow_key = format!(
            "{}/{}#{}",
            key.tenant_id.as_str(),
            key.capability,
            key.field
        );

        let conn = self.connect().await?;
        conn.execute("BEGIN IMMEDIATE", ()).await.map_err(map_err)?;

        let result = async {
            conn.execute(
                &format!("DELETE FROM {VALUES_TABLE} WHERE scope_hash = ?1 AND occurred_at < ?2"),
                params![scope.clone(), cutoff],
            )
            .await
            .map_err(map_err)?;

            // Dedup short-circuit (see record_invocation).
            let is_replay = event_id_exists(&conn, VALUES_TABLE, &scope, event_id).await?;

            let mut evicted = 0u64;
            if !is_replay {
                // Per-key sample cap: FAIL CLOSED. Unlike InvocationCount,
                // NumericSum has no per-sample count threshold to bound the
                // window at validation time, so the per-key cap is the only
                // sample bound. The old drop-oldest silently UNDERCOUNTED the
                // sum (in-window values discarded) — a fail-OPEN weakening of
                // the value cap. We now reject (#3929).
                let in_window = scope_in_window_count(&conn, VALUES_TABLE, &scope, cutoff).await?;
                if in_window as usize >= MAX_SAMPLES_PER_KEY {
                    return Err(PredicateBackendError::WindowOverflow {
                        key: overflow_key.clone(),
                        cap: MAX_SAMPLES_PER_KEY,
                    });
                }

                let scope_exists = scope_exists(&conn, VALUES_TABLE, &scope).await?;
                if !scope_exists {
                    evicted += enforce_caps(&conn, VALUES_TABLE, &tenant).await?;
                }

                conn.execute(
                    &format!(
                        "INSERT INTO {VALUES_TABLE} (scope_hash, event_id, occurred_at, value, tenant_id) \
                         VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT (scope_hash, event_id) DO NOTHING"
                    ),
                    params![scope.clone(), event_id.as_str(), now_ms, value_str, tenant.clone()],
                )
                .await
                .map_err(map_err)?;
            }

            // In-window sum computed from surviving rows — exact under
            // eviction by construction (no external running sum to drift).
            // rust_decimal preserved exactly via the TEXT value column; sum in
            // Rust to avoid SQLite float accumulation.
            let sum = sum_decimal(
                &conn,
                &format!(
                    "SELECT value FROM {VALUES_TABLE} WHERE scope_hash = ?1 AND occurred_at >= ?2"
                ),
                params![scope.clone(), cutoff],
            )
            .await?;
            Ok::<(Decimal, u64), PredicateBackendError>((sum, evicted))
        }
        .await;

        finish_txn(&conn, result, self).await
    }

    fn evictions_observed(&self) -> u64 {
        self.evictions.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Reaper: drop every row whose `occurred_at` is strictly older than
    /// `cutoff`, across both tables. Returns the total rows deleted. Runs in
    /// its own `BEGIN IMMEDIATE` transaction.
    async fn evict_older_than(&self, cutoff: DateTime<Utc>) -> Result<u64, PredicateBackendError> {
        let cutoff_ms = to_epoch_millis(cutoff);
        let conn = self.connect().await?;
        conn.execute("BEGIN IMMEDIATE", ()).await.map_err(map_err)?;
        let result = async {
            let a = conn
                .execute(
                    &format!("DELETE FROM {INVOCATIONS_TABLE} WHERE occurred_at < ?1"),
                    params![cutoff_ms],
                )
                .await
                .map_err(map_err)?;
            let b = conn
                .execute(
                    &format!("DELETE FROM {VALUES_TABLE} WHERE occurred_at < ?1"),
                    params![cutoff_ms],
                )
                .await
                .map_err(map_err)?;
            Ok::<u64, PredicateBackendError>(a + b)
        }
        .await;
        match result {
            Ok(dropped) => {
                conn.execute("COMMIT", ()).await.map_err(map_err)?;
                Ok(dropped)
            }
            Err(err) => {
                let _ = conn.execute("ROLLBACK", ()).await;
                Err(err)
            }
        }
    }
}

/// Commit on success / rollback on error, applying the eviction-counter
/// side effect only after a successful commit (so a rolled-back transaction
/// never advances the operator-visible eviction metric).
async fn finish_txn<T>(
    conn: &Connection,
    result: Result<(T, u64), PredicateBackendError>,
    backend: &LibSqlPredicateStateBackend,
) -> Result<T, PredicateBackendError> {
    match result {
        Ok((value, evicted)) => {
            conn.execute("COMMIT", ()).await.map_err(map_err)?;
            backend.bump_evictions(evicted);
            Ok(value)
        }
        Err(err) => {
            let _ = conn.execute("ROLLBACK", ()).await;
            Err(err)
        }
    }
}

/// Count the in-window rows for `scope` (`occurred_at >= cutoff`). Used both
/// for the fail-closed cap check and the final returned count.
async fn scope_in_window_count(
    conn: &Connection,
    table: &str,
    scope: &[u8],
    cutoff: i64,
) -> Result<u32, PredicateBackendError> {
    scalar_u32(
        conn,
        &format!("SELECT count(*) FROM {table} WHERE scope_hash = ?1 AND occurred_at >= ?2"),
        params![scope.to_vec(), cutoff],
    )
    .await
}

/// Whether `event_id` is already recorded for `scope` (the dedup short-circuit).
async fn event_id_exists(
    conn: &Connection,
    table: &str,
    scope: &[u8],
    event_id: &PredicateEventId,
) -> Result<bool, PredicateBackendError> {
    let count = scalar_u32(
        conn,
        &format!("SELECT count(*) FROM {table} WHERE scope_hash = ?1 AND event_id = ?2"),
        params![scope.to_vec(), event_id.as_str()],
    )
    .await?;
    Ok(count > 0)
}

async fn scope_exists(
    conn: &Connection,
    table: &str,
    scope: &[u8],
) -> Result<bool, PredicateBackendError> {
    let count = scalar_u32(
        conn,
        &format!("SELECT count(*) FROM {table} WHERE scope_hash = ?1"),
        params![scope.to_vec()],
    )
    .await?;
    Ok(count > 0)
}

/// Enforce the global [`MAX_HISTORY_KEYS`] and per-tenant
/// [`MAX_KEYS_PER_TENANT`] distinct-scope caps before inserting a NEW scope.
/// A "scope" is a distinct `scope_hash`; its "front timestamp" is its
/// `min(occurred_at)`. The victim is the scope with the smallest front
/// timestamp — the durable equivalent of the in-memory `min_by_key(front ts)`
/// LRU. Returns the number of scopes evicted (one increment per evicted bucket,
/// matching the in-memory backend's `evictions` semantics).
///
/// [`MAX_HISTORY_KEYS`]: ironclaw_hooks::predicate_state::MAX_HISTORY_KEYS
/// [`MAX_KEYS_PER_TENANT`]: ironclaw_hooks::predicate_state::MAX_KEYS_PER_TENANT
async fn enforce_caps(
    conn: &Connection,
    table: &str,
    tenant: &str,
) -> Result<u64, PredicateBackendError> {
    let mut evicted = 0u64;

    // Per-tenant quota first (matches in-memory: a tenant at its cap evicts
    // ITS OWN oldest scope so it can't push out other tenants).
    let tenant_scopes = scalar_u32(
        conn,
        &format!("SELECT count(DISTINCT scope_hash) FROM {table} WHERE tenant_id = ?1"),
        params![tenant],
    )
    .await?;
    if tenant_scopes as usize >= MAX_KEYS_PER_TENANT {
        if evict_oldest_scope(conn, table, Some(tenant)).await? {
            evicted += 1;
        }
    } else {
        // Global cap: evict the oldest scope across ALL tenants.
        let total_scopes = scalar_u32(
            conn,
            &format!("SELECT count(DISTINCT scope_hash) FROM {table}"),
            params![],
        )
        .await?;
        if total_scopes as usize >= MAX_HISTORY_KEYS
            && evict_oldest_scope(conn, table, None).await?
        {
            evicted += 1;
        }
    }
    Ok(evicted)
}

/// Delete every row of the scope whose `min(occurred_at)` is smallest
/// (optionally restricted to `tenant`). Returns true if a scope was evicted.
async fn evict_oldest_scope(
    conn: &Connection,
    table: &str,
    tenant: Option<&str>,
) -> Result<bool, PredicateBackendError> {
    let select_victim = match tenant {
        Some(_) => format!(
            "SELECT scope_hash FROM {table} WHERE tenant_id = ?1 \
             GROUP BY scope_hash ORDER BY min(occurred_at) ASC, scope_hash ASC LIMIT 1"
        ),
        None => format!(
            "SELECT scope_hash FROM {table} \
             GROUP BY scope_hash ORDER BY min(occurred_at) ASC, scope_hash ASC LIMIT 1"
        ),
    };
    let mut rows = match tenant {
        Some(t) => conn.query(&select_victim, params![t]).await,
        None => conn.query(&select_victim, params![]).await,
    }
    .map_err(map_err)?;
    let Some(row) = rows.next().await.map_err(map_err)? else {
        return Ok(false);
    };
    let victim: Vec<u8> = row.get_value(0).map_err(map_err).and_then(blob_value)?;
    drop(rows);
    conn.execute(
        &format!("DELETE FROM {table} WHERE scope_hash = ?1"),
        params![victim],
    )
    .await
    .map_err(map_err)?;
    Ok(true)
}

async fn scalar_u32(
    conn: &Connection,
    sql: &str,
    params: impl libsql::params::IntoParams,
) -> Result<u32, PredicateBackendError> {
    let mut rows = conn.query(sql, params).await.map_err(map_err)?;
    let row = rows.next().await.map_err(map_err)?;
    let Some(row) = row else { return Ok(0) };
    let v: i64 = row.get(0).map_err(map_err)?;
    Ok(v.max(0) as u32)
}

/// Sum a column of rust_decimal-as-TEXT values in Rust. SQLite `total()` would
/// accumulate in f64 and lose `rust_decimal` precision, so we read the rows
/// and add with `Decimal`.
async fn sum_decimal(
    conn: &Connection,
    sql: &str,
    params: impl libsql::params::IntoParams,
) -> Result<Decimal, PredicateBackendError> {
    use std::str::FromStr;
    let mut rows = conn.query(sql, params).await.map_err(map_err)?;
    let mut sum = Decimal::ZERO;
    while let Some(row) = rows.next().await.map_err(map_err)? {
        let s: String = row.get(0).map_err(map_err)?;
        let d = Decimal::from_str(&s)
            .map_err(|e| PredicateBackendError::Unavailable(format!("decimal decode: {e}")))?;
        sum += d;
    }
    Ok(sum)
}

fn blob_value(v: libsql::Value) -> Result<Vec<u8>, PredicateBackendError> {
    match v {
        libsql::Value::Blob(b) => Ok(b),
        other => Err(PredicateBackendError::Unavailable(format!(
            "expected BLOB scope_hash, got {other:?}"
        ))),
    }
}
