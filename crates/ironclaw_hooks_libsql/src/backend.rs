//! Durable libSQL-backed [`PredicateStateBackend`] implementation.
//!
//! Mirrors the in-memory backend's invariants
//! (`ironclaw_hooks::predicate_state::InMemoryPredicateStateBackend`) against a
//! libSQL / SQLite database so predicate counter / value-sum state survives
//! process restart and is consistent across every host pointing at the same
//! database file. The schema lives in [`crate::schema`]; scope/key-hash
//! derivation in [`crate::hashing`].
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
//! When a key already holds [`MAX_SAMPLES_PER_KEY`] in-window rows and a
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

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ironclaw_hooks::predicate_state::{
    InvocationKey, MAX_KEYS_PER_TENANT, MAX_SAMPLES_PER_KEY, PredicateBackendError,
    PredicateEventId, PredicateStateBackend, ValueKey,
};
use libsql::{Connection, params};
use rust_decimal::Decimal;
use tokio::sync::Mutex;

use crate::hashing::{
    invocation_key_hash, tenant_scope_hash, to_epoch_millis, value_key_hash, window_cutoff_millis,
};
use crate::schema::{INVOCATIONS_TABLE, LIBSQL_PREDICATE_STATE_SCHEMA, VALUES_TABLE};

/// Connect-retry attempts for transient open failures
/// (`SQLITE_CANTOPEN` / `SQLITE_BUSY` during concurrent connection creation).
/// Mirrors `LibSqlBackend::connect` in `src/db/libsql/mod.rs`.
const CONNECT_ATTEMPTS: u32 = 3;

/// Durable libSQL-backed predicate-state backend.
///
/// # Concurrency contract
///
/// Mirrors the project's canonical libSQL write-serialisation model (see
/// `LibSqlBackend` / `LibSqlWorkspaceStore` in `src/db/libsql/`). Three
/// mechanisms, layered:
///
/// 1. **In-process write serialisation (`write_lock`).** Every mutating op
///    (`record_invocation`, `record_value`, `evict_older_than`,
///    `run_migrations`) acquires a per-backend `Arc<Mutex<()>>` and holds it
///    for the whole `BEGIN IMMEDIATE` … `COMMIT` transaction. This is the
///    primary admission control: it serialises all writers from THIS process
///    before they ever contend at the SQLite layer. Without it, an unbounded
///    fan-out (e.g. a heartbeat tick evaluating thousands of hooks at once)
///    races on the single SQLite write lock; in libSQL's replication-enabled
///    build a raw `BEGIN IMMEDIATE` statement can return `SQLITE_BUSY`
///    *immediately* instead of honouring `busy_timeout`, surfacing as
///    `Unavailable("database is locked")` — which **fail-closes** predicate
///    evaluation (a hook that should pass gets denied under load). Serialising
///    in-process keeps each process to one writer at a time, so the file-handle
///    footprint stays flat and the SQLite write lock is never contended from
///    within the process. This is exactly the `write_lock: Arc<Mutex<()>>`
///    pattern `LibSqlWorkspaceStore` uses for the same reason.
///
/// 2. **Cross-process serialisation (`PRAGMA busy_timeout = 5000`).** Two
///    separate processes (or two backend instances over different `Database`
///    handles) still reduce to two connections contending for the single
///    SQLite write lock; the busy timeout makes the loser wait up to 5 s
///    rather than fail. The in-process mutex does not span instances, so this
///    remains the cross-instance backstop.
///
/// 3. **Connect retry.** Connection creation retries with exponential backoff
///    on transient open failures (`SQLITE_CANTOPEN` during concurrent opens),
///    matching `LibSqlBackend::connect`.
///
/// None of this changes the transactional contract: each op still runs inside
/// one `BEGIN IMMEDIATE` … `COMMIT`, and a genuinely unavailable backend still
/// surfaces as [`PredicateBackendError::Unavailable`] /
/// [`PredicateBackendError::WindowOverflow`], which fail-close predicate
/// evaluation. The point is to keep *transient* load from being mistaken for a
/// real outage.
pub struct LibSqlPredicateStateBackend {
    db: Arc<libsql::Database>,
    /// Serialises all in-process writers across the whole `BEGIN IMMEDIATE`
    /// transaction (see the concurrency contract above). Mirrors the
    /// `write_lock` in `src/db/libsql/`.
    write_lock: Arc<Mutex<()>>,
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
            write_lock: Arc::new(Mutex::new(())),
            evictions: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// Open a fresh connection with the project-standard busy timeout. Retries
    /// connection creation with exponential backoff on transient open failures
    /// (`SQLITE_CANTOPEN` during concurrent opens) so a brief race doesn't
    /// fail-close predicate evaluation. Mirrors `LibSqlBackend::connect`.
    async fn connect(&self) -> Result<Connection, PredicateBackendError> {
        let mut last_err = None;
        for attempt in 0..CONNECT_ATTEMPTS {
            match self.db.connect() {
                Ok(conn) => {
                    conn.query("PRAGMA busy_timeout = 5000", ())
                        .await
                        .map_err(map_err)?;
                    return Ok(conn);
                }
                Err(e) => {
                    last_err = Some(e);
                    if attempt + 1 < CONNECT_ATTEMPTS {
                        tokio::time::sleep(Duration::from_millis(50 * 2u64.pow(attempt))).await;
                    }
                }
            }
        }
        Err(PredicateBackendError::Unavailable(format!(
            "failed to open libSQL connection after {CONNECT_ATTEMPTS} attempts: {}",
            last_err.map(|e| e.to_string()).unwrap_or_default()
        )))
    }

    /// Create the predicate-state tables and indexes if absent. Idempotent;
    /// wrapped in `BEGIN IMMEDIATE` so concurrent first-time migrations
    /// serialise. SQLite supports transactional DDL.
    pub async fn run_migrations(&self) -> Result<(), PredicateBackendError> {
        // Migrations are writers too: serialise with record_* / evict.
        let _write_guard = self.write_lock.lock().await;
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

    /// The one load-bearing record state machine shared by `record_invocation`
    /// and `record_value` (P2: the two paths used to duplicate this exact
    /// sequence, so every correctness fix had to be applied twice in the same
    /// order). The sequence is invariant across both tables:
    ///
    /// 1. acquire the in-process `write_lock`, open a connection, `BEGIN IMMEDIATE`;
    /// 2. trim out-of-window rows for the key;
    /// 3. dedup short-circuit on a replayed `event_id`;
    /// 4. fail-closed per-key sample cap;
    /// 5. per-tenant LRU quota eviction (only when inserting a brand-new key);
    /// 6. insert (`ON CONFLICT DO NOTHING` belt-and-suspenders);
    /// 7. read the in-window result under the same lock; commit / rollback.
    ///
    /// The two callers differ ONLY in which table they touch, how the row is
    /// inserted, and how the in-window result is read — all carried by `spec`
    /// ([`RecordSpec`]). The atomicity / fail-closed guarantees live here, once.
    async fn record<T>(&self, spec: RecordSpec<'_, T>) -> Result<T, PredicateBackendError> {
        let RecordSpec {
            table,
            scope,
            key_hash,
            event_id,
            now_ms,
            cutoff,
            overflow_key,
            insert,
            read_result,
        } = spec;

        // Serialise in-process writers before touching SQLite (see the
        // concurrency contract on the struct). Held for the whole transaction.
        let _write_guard = self.write_lock.lock().await;
        let conn = self.connect().await?;
        conn.execute("BEGIN IMMEDIATE", ()).await.map_err(map_err)?;

        let result = async {
            // 1. Trim entries outside the window for THIS key first, so the
            //    per-key cap and the final read both see only in-window rows.
            conn.execute(
                &format!("DELETE FROM {table} WHERE key_hash = ?1 AND occurred_at < ?2"),
                params![key_hash.clone(), cutoff],
            )
            .await
            .map_err(map_err)?;

            // 2. Dedup short-circuit: a replayed id is a no-op against the
            //    result and must NOT trip the overflow check (replay refusal
            //    survives the cap boundary). If the id is already present for
            //    this key, skip the insert + cap check entirely.
            let is_replay = event_id_exists(&conn, table, &key_hash, event_id).await?;

            let mut evicted = 0u64;
            if !is_replay {
                // 3. Per-key sample cap: FAIL CLOSED. If the key is already at
                //    the cap with in-window rows, a NEW distinct id cannot be
                //    recorded without dropping an existing in-window sample, so
                //    reject (#3929) rather than silently evicting the oldest.
                let in_window = key_in_window_count(&conn, table, &key_hash, cutoff).await?;
                if in_window as usize >= MAX_SAMPLES_PER_KEY {
                    return Err(PredicateBackendError::WindowOverflow {
                        key: overflow_key.clone(),
                        cap: MAX_SAMPLES_PER_KEY,
                    });
                }

                // 4. Per-tenant LRU quota: only relevant when inserting a NEW
                //    key (a fresh PRIMARY KEY). Mirror the in-memory backend's
                //    "evict before insert when at quota" semantics, ranking the
                //    tenant's keys by oldest-front (MIN(occurred_at)).
                if !key_exists(&conn, table, &key_hash).await? {
                    evicted += enforce_caps(&conn, table, &scope).await?;
                }

                // 5. Insert (table-specific shape via the spec). The dedup
                //    short-circuit already excludes replays, but keep
                //    ON CONFLICT DO NOTHING as a belt-and-suspenders guard
                //    against a concurrent insert of the same id.
                insert(&conn, &scope, &key_hash, event_id, now_ms).await?;
            }

            // 6. In-window result read under the same lock (count or sum).
            let value = read_result(&conn, &key_hash, cutoff).await?;
            Ok::<(T, u64), PredicateBackendError>((value, evicted))
        }
        .await;

        finish_txn(&conn, result, self).await
    }
}

/// Per-table inputs to the shared [`LibSqlPredicateStateBackend::record`] state
/// machine. Everything that differs between the invocation and value paths —
/// the table name, the precomputed hashes/timestamps, the human-readable
/// overflow key, the insert shape, and the in-window result read — is carried
/// here so the transaction template itself stays single-sourced.
struct RecordSpec<'a, T> {
    table: &'static str,
    scope: Vec<u8>,
    key_hash: Vec<u8>,
    event_id: &'a PredicateEventId,
    now_ms: i64,
    cutoff: i64,
    overflow_key: String,
    /// Table-specific insert. Receives the open connection plus the
    /// precomputed scope/key/event/timestamp; owns the column list and values.
    #[allow(clippy::type_complexity)]
    insert: Box<
        dyn for<'c> FnOnce(
                &'c Connection,
                &'c [u8],
                &'c [u8],
                &'c PredicateEventId,
                i64,
            ) -> Pin<
                Box<dyn Future<Output = Result<(), PredicateBackendError>> + Send + 'c>,
            > + Send
            + 'a,
    >,
    /// Table-specific in-window result read (count for invocations, decimal sum
    /// for values), returning the value the public method hands back.
    #[allow(clippy::type_complexity)]
    read_result: Box<
        dyn for<'c> FnOnce(
                &'c Connection,
                &'c [u8],
                i64,
            ) -> Pin<
                Box<dyn Future<Output = Result<T, PredicateBackendError>> + Send + 'c>,
            > + Send
            + 'a,
    >,
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
        let scope = tenant_scope_hash(key.tenant_id.as_str());
        let key_hash = invocation_key_hash(key);
        let cutoff = window_cutoff_millis(now, window);
        let now_ms = to_epoch_millis(now);
        let overflow_key = format!("{}/{}", key.tenant_id.as_str(), key.capability);

        self.record(RecordSpec {
            table: INVOCATIONS_TABLE,
            scope,
            key_hash,
            event_id,
            now_ms,
            cutoff,
            overflow_key,
            insert: Box::new(|conn, scope, key_hash, event_id, now_ms| {
                Box::pin(async move {
                    conn.execute(
                        &format!(
                            "INSERT INTO {INVOCATIONS_TABLE} (scope_hash, key_hash, event_id, occurred_at) \
                             VALUES (?1, ?2, ?3, ?4) ON CONFLICT (key_hash, event_id) DO NOTHING"
                        ),
                        params![scope.to_vec(), key_hash.to_vec(), event_id.as_str(), now_ms],
                    )
                    .await
                    .map(|_| ())
                    .map_err(map_err)
                })
            }),
            read_result: Box::new(|conn, key_hash, cutoff| {
                Box::pin(
                    async move { key_in_window_count(conn, INVOCATIONS_TABLE, key_hash, cutoff).await },
                )
            }),
        })
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
        let scope = tenant_scope_hash(key.tenant_id.as_str());
        let key_hash = value_key_hash(key);
        let cutoff = window_cutoff_millis(now, window);
        let now_ms = to_epoch_millis(now);
        let value_str = value.to_string();
        let overflow_key = format!(
            "{}/{}#{}",
            key.tenant_id.as_str(),
            key.capability,
            key.field
        );

        self.record(RecordSpec {
            table: VALUES_TABLE,
            scope,
            key_hash,
            event_id,
            now_ms,
            cutoff,
            overflow_key,
            insert: Box::new(move |conn, scope, key_hash, event_id, now_ms| {
                Box::pin(async move {
                    conn.execute(
                        &format!(
                            "INSERT INTO {VALUES_TABLE} (scope_hash, key_hash, event_id, occurred_at, value) \
                             VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT (key_hash, event_id) DO NOTHING"
                        ),
                        params![scope.to_vec(), key_hash.to_vec(), event_id.as_str(), now_ms, value_str],
                    )
                    .await
                    .map(|_| ())
                    .map_err(map_err)
                })
            }),
            read_result: Box::new(|conn, key_hash, cutoff| {
                Box::pin(async move {
                    // In-window sum computed from surviving rows — exact under
                    // eviction by construction (no external running sum to drift).
                    // rust_decimal preserved exactly via the TEXT value column;
                    // sum in Rust to avoid SQLite float accumulation.
                    sum_decimal(
                        conn,
                        &format!(
                            "SELECT value FROM {VALUES_TABLE} WHERE key_hash = ?1 AND occurred_at >= ?2"
                        ),
                        params![key_hash.to_vec(), cutoff],
                    )
                    .await
                })
            }),
        })
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
        // Reaper is a writer too: serialise with record_* / migrations.
        let _write_guard = self.write_lock.lock().await;
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

/// Count the in-window rows for `key_hash` (`occurred_at >= cutoff`). Used both
/// for the fail-closed cap check and the final returned count.
async fn key_in_window_count(
    conn: &Connection,
    table: &str,
    key_hash: &[u8],
    cutoff: i64,
) -> Result<u32, PredicateBackendError> {
    scalar_u32(
        conn,
        &format!("SELECT count(*) FROM {table} WHERE key_hash = ?1 AND occurred_at >= ?2"),
        params![key_hash.to_vec(), cutoff],
    )
    .await
}

/// Whether `event_id` is already recorded for `key_hash` (the dedup
/// short-circuit).
async fn event_id_exists(
    conn: &Connection,
    table: &str,
    key_hash: &[u8],
    event_id: &PredicateEventId,
) -> Result<bool, PredicateBackendError> {
    let count = scalar_u32(
        conn,
        &format!("SELECT count(*) FROM {table} WHERE key_hash = ?1 AND event_id = ?2"),
        params![key_hash.to_vec(), event_id.as_str()],
    )
    .await?;
    Ok(count > 0)
}

/// Whether any row exists for `key_hash` — i.e. whether this is an existing
/// bucket (the quota only runs when inserting a brand-new key).
async fn key_exists(
    conn: &Connection,
    table: &str,
    key_hash: &[u8],
) -> Result<bool, PredicateBackendError> {
    let count = scalar_u32(
        conn,
        &format!("SELECT count(*) FROM {table} WHERE key_hash = ?1"),
        params![key_hash.to_vec()],
    )
    .await?;
    Ok(count > 0)
}

/// Enforce the per-tenant [`MAX_KEYS_PER_TENANT`] distinct-key quota before
/// inserting a NEW key. A tenant is a distinct `scope_hash`; its keys are the
/// distinct `key_hash` values under that scope, each key's "front timestamp"
/// being its `min(occurred_at)`. The victim is the tenant's key with the
/// smallest front timestamp — the durable equivalent of the in-memory
/// `min_by_key(front ts)` LRU. Returns the number of keys evicted (one
/// increment per evicted bucket, matching the in-memory backend's `evictions`
/// semantics).
///
/// # No global cap (parity with the Postgres backend)
///
/// Unlike the in-memory backend, the durable backends do NOT enforce the
/// global [`MAX_HISTORY_KEYS`] cap. That cap is an in-memory-only memory-
/// footprint bound (threat-model finding **D5**, see the constant's rustdoc);
/// a disk-backed store is not memory-bound, so durable backends reap instead
/// via time-based [`PredicateStateBackend::evict_older_than`] plus this
/// per-tenant quota. Enforcing a global cap here made libSQL diverge from
/// Postgres (which has none) once total scopes exceed `MAX_HISTORY_KEYS` while
/// staying under the per-tenant quota — a cross-backend inconsistency the
/// parity matrix had to exclude. Dropping it restores parity.
///
/// [`MAX_HISTORY_KEYS`]: ironclaw_hooks::predicate_state::MAX_HISTORY_KEYS
/// [`MAX_KEYS_PER_TENANT`]: ironclaw_hooks::predicate_state::MAX_KEYS_PER_TENANT
/// [`PredicateStateBackend::evict_older_than`]: ironclaw_hooks::predicate_state::PredicateStateBackend::evict_older_than
async fn enforce_caps(
    conn: &Connection,
    table: &str,
    scope: &[u8],
) -> Result<u64, PredicateBackendError> {
    let mut evicted = 0u64;

    // Per-tenant quota (matches in-memory: a tenant at its cap evicts ITS OWN
    // oldest key so it can't push out other tenants). No global cap — see
    // the fn-level doc; durable backends reap by time, not a global key count.
    // The tenant grain is `scope_hash`; its keys are the distinct `key_hash`
    // values under it.
    let tenant_keys = scalar_u32(
        conn,
        &format!("SELECT count(DISTINCT key_hash) FROM {table} WHERE scope_hash = ?1"),
        params![scope.to_vec()],
    )
    .await?;
    if tenant_keys as usize >= MAX_KEYS_PER_TENANT && evict_oldest_key(conn, table, scope).await? {
        evicted += 1;
    }
    Ok(evicted)
}

/// Delete every row of the tenant's key whose `min(occurred_at)` is smallest
/// (oldest-front victim selection). Returns true if a key was evicted. Always
/// tenant-scoped: durable backends only enforce the per-tenant quota (no global
/// cap — see [`enforce_caps`]).
async fn evict_oldest_key(
    conn: &Connection,
    table: &str,
    scope: &[u8],
) -> Result<bool, PredicateBackendError> {
    let select_victim = format!(
        "SELECT key_hash FROM {table} WHERE scope_hash = ?1 \
         GROUP BY key_hash ORDER BY min(occurred_at) ASC, key_hash ASC LIMIT 1"
    );
    let mut rows = conn
        .query(&select_victim, params![scope.to_vec()])
        .await
        .map_err(map_err)?;
    let Some(row) = rows.next().await.map_err(map_err)? else {
        return Ok(false);
    };
    let victim: Vec<u8> = row.get_value(0).map_err(map_err).and_then(blob_value)?;
    drop(rows);
    conn.execute(
        &format!("DELETE FROM {table} WHERE key_hash = ?1"),
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
