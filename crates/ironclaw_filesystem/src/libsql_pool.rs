//! Bounded connection pool for [`LibSqlRootFilesystem`](crate::LibSqlRootFilesystem).
//!
//! The previous policy opened a brand-new libSQL connection
//! (`sqlite3_open_v2` + per-connection PRAGMA batch) for **every**
//! `RootFilesystem` operation. Under genuinely parallel CAS storms
//! (multi-threaded tokio runtime, one shared on-disk WAL database) that
//! unbounded open/PRAGMA/close churn intermittently fails inside the C
//! library with `SQLITE_MISUSE` ("bad parameter or other API misuse") or
//! spurious `disk I/O error` — issue #5466. The opposite extreme, one
//! shared connection for all callers, is also wrong: the CAS protocol
//! reads back rows-affected (`sqlite3_changes()`) after each `UPDATE ...
//! WHERE version = ?`, and that readback is per-connection state — two
//! tasks interleaving statements on one connection can observe each
//! other's counts, silently corrupting compare-and-swap into lost
//! updates. The resolution is the standard one for WAL SQLite: a small
//! bounded pool ([`deadpool`], the same pooling core the sibling
//! Postgres backend uses via `deadpool-postgres`) where each operation
//! checks out one connection for exclusive use and returns it on drop.
//!
//! ## Invariant: at most one checkout per call stack
//!
//! A `RootFilesystem` method must **drop its checked-out connection
//! before `.await`-ing any other `self` method that itself checks out a
//! connection** (see `put()`'s explicit `drop(conn)` before its
//! `current_version` readbacks, and `vector_nearest_query`'s before
//! `materialize_ranked`). Nested checkouts on one call stack can exhaust
//! the pool under load and stall every caller until the checkout wait
//! timeout fires. The concurrent-CAS storm regression test converts a
//! reintroduced nested checkout on the hot path into a deterministic
//! failure rather than a silent slowdown.

use std::sync::Arc;
use std::time::Duration;

use deadpool::managed::{Manager, Metrics, Pool, RecycleError, RecycleResult};

use crate::db::infrastructure_libsql_error;
use crate::{FilesystemError, FilesystemOperation};

/// Maximum simultaneously checked-out connections. Starting point, not a
/// profiled value: covers prod's default of 4 concurrent turn-runs plus
/// headroom for other stores sharing the process.
const LIBSQL_POOL_MAX_CONNECTIONS: usize = 8;

/// Deadline for a checkout waiting on a free connection. Prevents a
/// stalled holder from parking every other caller forever; comfortably
/// above the 5s per-connection `busy_timeout`.
const LIBSQL_POOL_CHECKOUT_TIMEOUT: Duration = Duration::from_secs(10);

pub(crate) const LIBSQL_CONNECT_ATTEMPTS: u32 = 3;
const LIBSQL_CONNECT_INITIAL_BACKOFF: Duration = Duration::from_millis(50);

/// Per-connection PRAGMAs applied to every libSQL connection.
///
/// libSQL/SQLite is a single-writer engine: there is no true "parallel
/// write" mode for a local file. Concurrent writers always serialise on
/// the database write lock. The throughput lever is therefore making each
/// write cheap and keeping readers from blocking the writer — which is
/// exactly what these PRAGMAs do, alongside `journal_mode=WAL` (set once at
/// migration time, see `run_migrations`).
///
/// - `busy_timeout=5000`: unchanged from the prior policy. A writer that
///   finds the write lock held waits up to 5s rather than failing fast.
/// - `synchronous=NORMAL`: in WAL mode this is crash-safe for the
///   *application* (committed transactions survive a process crash); only an
///   OS/power loss can roll back the most-recent commits, and the database
///   stays consistent regardless. It removes one fsync per commit, which is
///   the dominant cost of the serial-write path under load. This is the
///   standard high-throughput SQLite setting. Use `FULL` only if per-commit
///   power-loss durability is required (it is not for turn/loop state).
/// - `temp_store=MEMORY`: keep transient indexes/sorters off disk.
/// - `cache_size=-16000`: ~16 MiB page cache per connection (negative =
///   KiB), so hot pages (the turn-state and resource-snapshot rows that are
///   read-modify-written every turn) stay resident instead of being
///   re-read from the OS page cache on each op.
/// - `mmap_size`: memory-map up to 256 MiB for reads, cutting read syscall
///   overhead on the many read-before-write checks (`exact_entry`,
///   `has_child_entry`, snapshot loads) that surround each write.
/// - `wal_autocheckpoint=1000`: bound WAL growth (checkpoint every ~1000
///   pages / ~4 MiB) so the WAL does not grow without limit under a burst
///   of writes.
///
/// `journal_mode` is deliberately NOT set here: it is a persistent,
/// database-level property (stored in the file header) and changing it
/// inside or alongside ordinary work is wasteful and cannot run inside a
/// transaction. It is set exactly once in `run_migrations`.
const LIBSQL_CONNECTION_PRAGMAS: &str = "\
    PRAGMA busy_timeout = 5000;\
    PRAGMA synchronous = NORMAL;\
    PRAGMA temp_store = MEMORY;\
    PRAGMA cache_size = -16000;\
    PRAGMA mmap_size = 268435456;\
    PRAGMA wal_autocheckpoint = 1000;";

/// Pool of PRAGMA-initialized libSQL connections for one database.
pub(crate) type LibSqlPool = Pool<LibSqlConnectionManager>;

/// Checkout guard: derefs to the pooled [`libsql::Connection`] and
/// returns it to the pool on drop.
pub(crate) type PooledLibSqlConnection = deadpool::managed::Object<LibSqlConnectionManager>;

/// [`deadpool`] manager that opens PRAGMA-initialized connections via
/// [`connect_with_retry`].
pub(crate) struct LibSqlConnectionManager {
    db: Arc<libsql::Database>,
}

impl Manager for LibSqlConnectionManager {
    type Type = libsql::Connection;
    type Error = FilesystemError;

    async fn create(&self) -> Result<libsql::Connection, FilesystemError> {
        connect_with_retry(|| self.db.connect()).await
    }

    async fn recycle(
        &self,
        connection: &mut libsql::Connection,
        _metrics: &Metrics,
    ) -> RecycleResult<FilesystemError> {
        // A connection returned mid-transaction (e.g. a failed ROLLBACK in
        // `run_migrations`) must not be handed to an unrelated caller —
        // reject it here so the pool discards it and opens a fresh one.
        if connection.is_autocommit() {
            Ok(())
        } else {
            Err(RecycleError::message(
                "libSQL connection returned to pool inside an open transaction",
            ))
        }
    }
}

/// Build the bounded pool for `db`.
pub(crate) fn build_libsql_pool(db: Arc<libsql::Database>) -> LibSqlPool {
    match Pool::builder(LibSqlConnectionManager { db })
        .max_size(LIBSQL_POOL_MAX_CONNECTIONS)
        .wait_timeout(Some(LIBSQL_POOL_CHECKOUT_TIMEOUT))
        .runtime(deadpool::Runtime::Tokio1)
        .build()
    {
        Ok(pool) => pool,
        // `build()` fails only when a timeout is configured without a
        // runtime; the runtime is set two lines up.
        Err(error) => unreachable!("libSQL pool build cannot fail: {error}"),
    }
}

/// Open a connection with bounded retries and apply the per-connection
/// PRAGMAs. Concurrent writers wait on SQLite locks (`busy_timeout`);
/// transient file-open races get a short retry budget before surfacing
/// as infrastructure errors.
pub(crate) async fn connect_with_retry<F>(
    mut open: F,
) -> Result<libsql::Connection, FilesystemError>
where
    F: FnMut() -> Result<libsql::Connection, libsql::Error>,
{
    let mut last_error = None;
    for attempt in 0..LIBSQL_CONNECT_ATTEMPTS {
        match open() {
            Ok(conn) => {
                // Apply the per-connection PRAGMAs in a single round-trip.
                // `execute_batch` runs each statement and discards the rows
                // PRAGMAs like `busy_timeout` return, which is exactly what
                // we want — we only care about the side effect.
                conn.execute_batch(LIBSQL_CONNECTION_PRAGMAS)
                    .await
                    .map_err(|error| {
                        infrastructure_libsql_error(FilesystemOperation::Connect, error)
                    })?;
                return Ok(conn);
            }
            Err(error) => {
                last_error = Some(error);
                if attempt + 1 < LIBSQL_CONNECT_ATTEMPTS {
                    tokio::time::sleep(connect_backoff(attempt)).await;
                }
            }
        }
    }

    let reason = match last_error {
        Some(error) => {
            format!(
                "failed to create libSQL connection after {LIBSQL_CONNECT_ATTEMPTS} attempts: {error}"
            )
        }
        None => {
            format!("failed to create libSQL connection after {LIBSQL_CONNECT_ATTEMPTS} attempts")
        }
    };
    Err(crate::db::infrastructure_error(
        FilesystemOperation::Connect,
        reason,
    ))
}

fn connect_backoff(attempt: u32) -> Duration {
    LIBSQL_CONNECT_INITIAL_BACKOFF * 2u32.pow(attempt)
}
