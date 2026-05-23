//! Durable libSQL-backed [`PredicateStateBackend`] (durable-backend PR 3/4).
//!
//! Mirrors the in-memory backend's invariants
//! ([`super::InMemoryPredicateStateBackend`]) against a libSQL / SQLite
//! database so predicate counter / value-sum state survives process restart
//! and is consistent across every host pointing at the same database file.
//!
//! # Schema
//!
//! Two tables, one per predicate kind:
//!
//! ```sql
//! CREATE TABLE hooks_predicate_invocations (
//!     scope_hash   BLOB    NOT NULL,  -- blake3(hook_id ‖ tenant_id ‖ capability)
//!     event_id     TEXT    NOT NULL,  -- PredicateEventId, host-assigned, ≤64 char hex
//!     occurred_at  INTEGER NOT NULL,  -- epoch milliseconds (canonical host clock)
//!     tenant_id    TEXT    NOT NULL,  -- retained for the per-tenant LRU + reaper
//!     PRIMARY KEY (scope_hash, event_id)
//! );
//! CREATE TABLE hooks_predicate_values (
//!     scope_hash   BLOB    NOT NULL,  -- blake3(hook_id ‖ tenant_id ‖ capability ‖ field)
//!     event_id     TEXT    NOT NULL,
//!     occurred_at  INTEGER NOT NULL,
//!     value        TEXT    NOT NULL,  -- rust_decimal, exact string (NUMERIC convention)
//!     tenant_id    TEXT    NOT NULL,
//!     PRIMARY KEY (scope_hash, event_id)
//! );
//! ```
//!
//! ## id column type (Codex #3635 finding)
//!
//! `event_id` is **TEXT**, not a `uuid`-typed column: a synthesized
//! [`super::PredicateEventId`] is a 64-char blake3 hex digest which does not
//! fit a 36-char `uuid`. SQLite has no native `uuid` type regardless, but the
//! TEXT choice is called out here so the libSQL and Postgres schemas stay
//! semantically aligned (the Postgres sibling, PR 2/4, must also avoid a
//! `uuid` column for the same reason).
//!
//! ## Replay-dedup
//!
//! The `PRIMARY KEY (scope_hash, event_id)` is the durable equivalent of the
//! in-memory `dedup_ids` set, scoped to the counter key (NOT global). Records
//! use `INSERT … ON CONFLICT (scope_hash, event_id) DO NOTHING` so a replayed
//! `event_id` against the same key is a no-op, while the same `event_id`
//! against a different key (or the other table) still records — matching the
//! [`super::PredicateStateBackend`] replay-refusal contract. Because the
//! invocation and value tables are separate, the same `event_id` in both does
//! not collide (the `event_id_dedup_isolated_across_maps` contract).
//!
//! # Clock basis (aligns with PR 2/4)
//!
//! The stored `occurred_at` is the **host-supplied `now: DateTime<Utc>`**
//! parameter (production passes [`chrono::Utc::now()`]), stored as epoch
//! milliseconds. We do NOT use SQLite `datetime('now')` for the row
//! timestamp: the [`PredicateStateBackend`] trait threads `now` in
//! explicitly so the contract harness can drive a deterministic clock, and
//! window trimming must be relative to that same clock to stay consistent
//! with the in-memory backend. "DB-clock canonical time" therefore means the
//! single canonical clock is the host's `Utc::now()` captured once at the
//! dispatch site and passed through `now` — both durable backends store that
//! value verbatim, so they are byte-for-byte semantically identical. Epoch
//! milliseconds (INTEGER) is chosen over an ISO-8601 TEXT column so window
//! arithmetic and ordering are exact integer comparisons with sub-second
//! resolution (the `at_millis` tests exercise this).
//!
//! # Atomicity
//!
//! Every `record_*` call runs inside a single `BEGIN IMMEDIATE` …`COMMIT`
//! transaction: `BEGIN IMMEDIATE` takes SQLite's write lock up front so
//! concurrent writers serialise (no read-modify-write race window — codex
//! Critical on PR #3635). The trim, dedup-insert, per-key cap eviction, and
//! the final in-window count/sum read all happen under that one lock, so the
//! write and the returned value are atomic. libSQL's single-writer semantics
//! mean "two hosts" reduces to "two connections contending for the write
//! lock"; `PRAGMA busy_timeout` lets the loser wait rather than fail.
//!
//! # Running-sum consistency under eviction
//!
//! The value backend does not keep an external running sum; the in-window sum
//! is computed by `SELECT total(value) WHERE occurred_at >= cutoff` inside the
//! same transaction *after* trimming and cap-eviction, so it is always exactly
//! the sum of the rows that survive — consistency under eviction is structural.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use libsql::{Connection, params};
use rust_decimal::Decimal;

use super::{
    InvocationKey, MAX_HISTORY_KEYS, MAX_KEYS_PER_TENANT, MAX_SAMPLES_PER_KEY,
    PredicateBackendError, PredicateEventId, PredicateStateBackend, ValueKey,
};

const INVOCATIONS_TABLE: &str = "hooks_predicate_invocations";
const VALUES_TABLE: &str = "hooks_predicate_values";

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

/// The libSQL migration body. Mirrors the schema documented at the module
/// level. Lives outside the `BEGIN IMMEDIATE` wrapper so the caller owns the
/// single rollback path (the established pattern in
/// `ironclaw_filesystem::libsql`).
async fn run_migrations_inner(conn: &Connection) -> Result<(), PredicateBackendError> {
    conn.execute_batch(LIBSQL_PREDICATE_STATE_SCHEMA)
        .await
        .map(|_| ())
        .map_err(map_err)
}

const LIBSQL_PREDICATE_STATE_SCHEMA: &str = "\
CREATE TABLE IF NOT EXISTS hooks_predicate_invocations (
    scope_hash  BLOB    NOT NULL,
    event_id    TEXT    NOT NULL,
    occurred_at INTEGER NOT NULL,
    tenant_id   TEXT    NOT NULL,
    PRIMARY KEY (scope_hash, event_id)
);
CREATE INDEX IF NOT EXISTS idx_hooks_predicate_invocations_scope_ts
    ON hooks_predicate_invocations (scope_hash, occurred_at);
CREATE INDEX IF NOT EXISTS idx_hooks_predicate_invocations_ts
    ON hooks_predicate_invocations (occurred_at);
CREATE INDEX IF NOT EXISTS idx_hooks_predicate_invocations_tenant
    ON hooks_predicate_invocations (tenant_id);

CREATE TABLE IF NOT EXISTS hooks_predicate_values (
    scope_hash  BLOB    NOT NULL,
    event_id    TEXT    NOT NULL,
    occurred_at INTEGER NOT NULL,
    value       TEXT    NOT NULL,
    tenant_id   TEXT    NOT NULL,
    PRIMARY KEY (scope_hash, event_id)
);
CREATE INDEX IF NOT EXISTS idx_hooks_predicate_values_scope_ts
    ON hooks_predicate_values (scope_hash, occurred_at);
CREATE INDEX IF NOT EXISTS idx_hooks_predicate_values_ts
    ON hooks_predicate_values (occurred_at);
CREATE INDEX IF NOT EXISTS idx_hooks_predicate_values_tenant
    ON hooks_predicate_values (tenant_id);
";

/// blake3 of the invocation scope components. Length-prefixed so distinct
/// component splits can't alias (`("a","bc")` vs `("ab","c")`).
fn invocation_scope_hash(key: &InvocationKey) -> Vec<u8> {
    let mut hasher = blake3::Hasher::new();
    hash_component(&mut hasher, key.hook_id.as_bytes());
    hash_component(&mut hasher, key.tenant_id.as_str().as_bytes());
    hash_component(&mut hasher, key.capability.as_bytes());
    hasher.finalize().as_bytes().to_vec()
}

/// blake3 of the value scope components (invocation scope + numeric field).
fn value_scope_hash(key: &ValueKey) -> Vec<u8> {
    let mut hasher = blake3::Hasher::new();
    hash_component(&mut hasher, key.hook_id.as_bytes());
    hash_component(&mut hasher, key.tenant_id.as_str().as_bytes());
    hash_component(&mut hasher, key.capability.as_bytes());
    hash_component(&mut hasher, key.field.as_bytes());
    hasher.finalize().as_bytes().to_vec()
}

fn hash_component(hasher: &mut blake3::Hasher, bytes: &[u8]) {
    hasher.update(&(bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
}

/// Epoch milliseconds for the canonical host clock. `timestamp_millis`
/// returns `i64`; the column is INTEGER so this is exact.
fn to_epoch_millis(now: DateTime<Utc>) -> i64 {
    now.timestamp_millis()
}

/// Window cutoff in epoch milliseconds. `window` is a non-negative
/// [`Duration`]; we saturate on overflow so a pathological multi-million-year
/// window trims nothing (conservative for a rate/value cap), matching the
/// in-memory backend's `window_cutoff` saturation behavior. The trim
/// comparison is `occurred_at < cutoff` (strictly older), so an entry exactly
/// at the cutoff is retained — the `< cutoff, not <=` contract.
fn window_cutoff_millis(now: DateTime<Utc>, window: Duration) -> i64 {
    let now_ms = now.timestamp_millis();
    let window_ms = i64::try_from(window.as_millis()).unwrap_or(i64::MAX);
    now_ms.saturating_sub(window_ms)
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

            // 2. LRU + per-tenant quota: only relevant when inserting a NEW
            //    scope (a fresh PRIMARY KEY). Mirror the in-memory backend's
            //    "evict before insert when at cap" semantics.
            let mut evicted = 0u64;
            let scope_exists = scope_exists(&conn, INVOCATIONS_TABLE, &scope).await?;
            if !scope_exists {
                evicted += enforce_caps_invocations(&conn, &tenant).await?;
            }

            // 3. Replay-dedup insert: no-op if (scope_hash, event_id) exists.
            conn.execute(
                &format!(
                    "INSERT INTO {INVOCATIONS_TABLE} (scope_hash, event_id, occurred_at, tenant_id) \
                     VALUES (?1, ?2, ?3, ?4) ON CONFLICT (scope_hash, event_id) DO NOTHING"
                ),
                params![scope.clone(), event_id.as_str(), now_ms, tenant.clone()],
            )
            .await
            .map_err(map_err)?;

            // 4. Per-key sample cap: keep only the most-recent
            //    MAX_SAMPLES_PER_KEY rows for this scope, dropping oldest.
            evicted += enforce_sample_cap_invocations(&conn, &scope).await?;

            // 5. In-window count read under the same lock.
            let count = scalar_u32(
                &conn,
                &format!(
                    "SELECT count(*) FROM {INVOCATIONS_TABLE} WHERE scope_hash = ?1 AND occurred_at >= ?2"
                ),
                params![scope.clone(), cutoff],
            )
            .await?;
            Ok::<(u32, u64), PredicateBackendError>((count, evicted))
        }
        .await;

        finish_txn(
            &conn,
            result,
            |this, evicted| this.bump_evictions(evicted),
            self,
        )
        .await
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

        let conn = self.connect().await?;
        conn.execute("BEGIN IMMEDIATE", ()).await.map_err(map_err)?;

        let result = async {
            conn.execute(
                &format!("DELETE FROM {VALUES_TABLE} WHERE scope_hash = ?1 AND occurred_at < ?2"),
                params![scope.clone(), cutoff],
            )
            .await
            .map_err(map_err)?;

            let mut evicted = 0u64;
            let scope_exists = scope_exists(&conn, VALUES_TABLE, &scope).await?;
            if !scope_exists {
                evicted += enforce_caps_values(&conn, &tenant).await?;
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

            evicted += enforce_sample_cap_values(&conn, &scope).await?;

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

        finish_txn(
            &conn,
            result,
            |this, evicted| this.bump_evictions(evicted),
            self,
        )
        .await
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
    apply_evictions: impl Fn(&LibSqlPredicateStateBackend, u64),
    backend: &LibSqlPredicateStateBackend,
) -> Result<T, PredicateBackendError> {
    match result {
        Ok((value, evicted)) => {
            conn.execute("COMMIT", ()).await.map_err(map_err)?;
            apply_evictions(backend, evicted);
            Ok(value)
        }
        Err(err) => {
            let _ = conn.execute("ROLLBACK", ()).await;
            Err(err)
        }
    }
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
/// [`MAX_KEYS_PER_TENANT`] distinct-scope caps before inserting a NEW
/// invocation scope. Returns the number of scopes evicted (each eviction may
/// remove several rows, but we count scopes to match the in-memory backend's
/// `evictions` semantics — one increment per evicted bucket).
async fn enforce_caps_invocations(
    conn: &Connection,
    tenant: &str,
) -> Result<u64, PredicateBackendError> {
    enforce_caps(conn, INVOCATIONS_TABLE, tenant).await
}

async fn enforce_caps_values(
    conn: &Connection,
    tenant: &str,
) -> Result<u64, PredicateBackendError> {
    enforce_caps(conn, VALUES_TABLE, tenant).await
}

/// Shared LRU + per-tenant-quota enforcement. A "scope" is a distinct
/// `scope_hash`; its "front timestamp" is its `min(occurred_at)`. The victim
/// is the scope with the smallest front timestamp — the durable equivalent of
/// the in-memory `min_by_key(front ts)` LRU.
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

/// Per-key sample cap: retain only the most-recent [`MAX_SAMPLES_PER_KEY`]
/// rows for `scope`, dropping the oldest. Returns 0 (per-key cap eviction is a
/// within-bucket trim, not an LRU bucket eviction — the in-memory backend does
/// not bump `evictions` for it either).
async fn enforce_sample_cap_invocations(
    conn: &Connection,
    scope: &[u8],
) -> Result<u64, PredicateBackendError> {
    enforce_sample_cap(conn, INVOCATIONS_TABLE, scope).await
}

async fn enforce_sample_cap_values(
    conn: &Connection,
    scope: &[u8],
) -> Result<u64, PredicateBackendError> {
    enforce_sample_cap(conn, VALUES_TABLE, scope).await
}

async fn enforce_sample_cap(
    conn: &Connection,
    table: &str,
    scope: &[u8],
) -> Result<u64, PredicateBackendError> {
    // Delete all but the newest MAX_SAMPLES_PER_KEY rows. Order by
    // (occurred_at, event_id) so ties on timestamp are broken deterministically
    // and the keep-set is stable.
    conn.execute(
        &format!(
            "DELETE FROM {table} WHERE scope_hash = ?1 AND event_id NOT IN ( \
               SELECT event_id FROM {table} WHERE scope_hash = ?1 \
               ORDER BY occurred_at DESC, event_id DESC LIMIT ?2 \
             )"
        ),
        params![scope.to_vec(), MAX_SAMPLES_PER_KEY as i64],
    )
    .await
    .map_err(map_err)?;
    Ok(0)
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
