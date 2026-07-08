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
//! connection** (see `vector_nearest_query`'s explicit `drop(conn)`
//! before `materialize_ranked`). Where the same connection can serve both
//! statements — e.g. `put()`'s post-write version readback via
//! `current_version_with_conn` — reuse the held checkout instead of
//! dropping and re-checking-out; that removes the invariant for that call
//! site entirely rather than relying on a manual `drop()`. Nested
//! checkouts on one call stack can exhaust the pool under load and stall
//! every caller until the checkout wait timeout fires. The concurrent-CAS
//! storm regression test converts a reintroduced nested checkout on the
//! hot path into a deterministic failure rather than a silent slowdown.

use std::sync::Arc;
use std::time::Duration;

use deadpool::managed::{Manager, Metrics, Pool, RecycleError, RecycleResult};

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

/// Build the bounded pool for `db`, using the production sizing/timeout
/// constants.
pub(crate) fn build_libsql_pool(db: Arc<libsql::Database>) -> LibSqlPool {
    build_libsql_pool_with_config(
        db,
        LIBSQL_POOL_MAX_CONNECTIONS,
        LIBSQL_POOL_CHECKOUT_TIMEOUT,
    )
}

/// Test/config seam behind [`build_libsql_pool`]: builds a pool with an
/// explicit `max_size`/`wait_timeout` instead of the production constants,
/// so tests can construct a deliberately tiny, fast-timing-out pool (e.g.
/// size 1) to exercise checkout-exhaustion behaviour without waiting out
/// the real 10s production timeout.
pub(crate) fn build_libsql_pool_with_config(
    db: Arc<libsql::Database>,
    max_size: usize,
    wait_timeout: Duration,
) -> LibSqlPool {
    match Pool::builder(LibSqlConnectionManager { db })
        .max_size(max_size)
        .wait_timeout(Some(wait_timeout))
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
pub(crate) async fn connect_with_retry<F>(open: F) -> Result<libsql::Connection, FilesystemError>
where
    F: FnMut() -> Result<libsql::Connection, libsql::Error>,
{
    connect_with_retry_and_pragmas(open, |_| LIBSQL_CONNECTION_PRAGMAS).await
}

/// Like [`connect_with_retry`], but lets the caller vary the PRAGMA batch
/// applied on each attempt. Retries cover both a failed connection *open*
/// and a failed PRAGMA *application* — a connection that opens but whose
/// setup batch fails (e.g. a transient lock on the file) is discarded and
/// re-opened rather than surfaced as a permanent error.
async fn connect_with_retry_and_pragmas<F, P>(
    mut open: F,
    mut pragmas_for_attempt: P,
) -> Result<libsql::Connection, FilesystemError>
where
    F: FnMut() -> Result<libsql::Connection, libsql::Error>,
    P: FnMut(u32) -> &'static str,
{
    let mut last_error = None;
    for attempt in 0..LIBSQL_CONNECT_ATTEMPTS {
        match open() {
            Ok(conn) => {
                // Apply the per-connection PRAGMAs in a single round-trip.
                // `execute_batch` runs each statement and discards the rows
                // PRAGMAs like `busy_timeout` return, which is exactly what
                // we want — we only care about the side effect.
                match conn.execute_batch(pragmas_for_attempt(attempt)).await {
                    Ok(_) => return Ok(conn),
                    Err(error) => {
                        last_error = Some(error);
                        if attempt + 1 < LIBSQL_CONNECT_ATTEMPTS {
                            tokio::time::sleep(connect_backoff(attempt)).await;
                        }
                    }
                }
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
                "failed to create or initialize libSQL connection after {LIBSQL_CONNECT_ATTEMPTS} attempts: {error}"
            )
        }
        None => {
            format!(
                "failed to create or initialize libSQL connection after {LIBSQL_CONNECT_ATTEMPTS} attempts"
            )
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

#[cfg(test)]
mod tests {
    //! Pool-internal regression tests. Cross-backend contract tests live
    //! in `tests/`; these cover the deadpool `Manager` seam
    //! (`recycle`/`create`) directly, which isn't reachable from that
    //! integration surface.

    use super::*;

    /// A connection returned to the pool while a transaction is still open
    /// (e.g. a caller that `BEGIN`s and then drops its checkout without
    /// `COMMIT`/`ROLLBACK`) must never be handed to the next caller —
    /// two unrelated operations interleaving statements on one open
    /// transaction would silently corrupt state. `recycle` rejects it, so
    /// deadpool discards the poisoned connection and opens a fresh one on
    /// the next checkout (see `libsql_pool`'s one-checkout-per-call-stack
    /// invariant note above).
    #[tokio::test]
    async fn recycle_rejects_connection_returned_inside_transaction() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("recycle-reject-test.db");
        let db = Arc::new(libsql::Builder::new_local(db_path).build().await.unwrap());
        let pool = build_libsql_pool(db);

        {
            let conn = pool.get().await.unwrap();
            conn.execute("BEGIN", ()).await.unwrap();
            assert!(
                !conn.is_autocommit(),
                "connection must be mid-transaction before it's returned to the pool"
            );
            // `conn` drops here, returning to the pool with an open
            // transaction and no COMMIT/ROLLBACK.
        }

        // The only idle connection in the pool is the one just returned
        // with an open transaction. The next checkout must observe a
        // fresh, autocommit-clean connection rather than the poisoned one.
        let next = pool.get().await.unwrap();
        assert!(
            next.is_autocommit(),
            "pool must discard a connection returned inside an open transaction, not reuse it"
        );
    }

    /// When every open attempt fails, `connect_with_retry` must exhaust its
    /// fixed retry budget (not retry forever) and surface a `Connect`
    /// infrastructure error whose reason includes the final attempt's
    /// cause, so a permanently broken database file/path fails fast and
    /// diagnosably instead of hanging or masking the underlying error.
    #[tokio::test]
    async fn connect_with_retry_returns_connect_error_after_exhausting_open_failures() {
        let mut attempts = 0;

        let result = connect_with_retry(|| {
            attempts += 1;
            Err(libsql::Error::ConnectionFailed(format!(
                "synthetic permanent failure {attempts}"
            )))
        })
        .await;

        assert_eq!(
            attempts, LIBSQL_CONNECT_ATTEMPTS,
            "must stop after exhausting the fixed retry budget, not retry forever"
        );
        match result {
            Err(FilesystemError::BackendInfrastructure { operation, reason }) => {
                assert_eq!(operation, FilesystemOperation::Connect);
                assert!(
                    reason.contains("synthetic permanent failure"),
                    "reason must include the final open attempt's cause: {reason}"
                );
            }
            other => panic!("expected FilesystemError::BackendInfrastructure, got {other:?}"),
        }
    }

    /// A connection that opens successfully but whose PRAGMA batch fails
    /// (e.g. a transient lock on the file mid-setup) must be retried with
    /// a fresh `open()` call rather than surfaced immediately — `create`
    /// only has one chance to hand deadpool a usable connection per
    /// checkout.
    #[tokio::test]
    async fn connect_retries_transient_pragma_failures_before_succeeding() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("connect-retry-pragma-test.db");
        let db = libsql::Builder::new_local(db_path).build().await.unwrap();
        let mut opens = 0;
        let mut initializers = 0;

        let conn = connect_with_retry_and_pragmas(
            || {
                opens += 1;
                db.connect()
            },
            |_| {
                initializers += 1;
                if initializers == 1 {
                    "THIS IS NOT SQL"
                } else {
                    LIBSQL_CONNECTION_PRAGMAS
                }
            },
        )
        .await
        .unwrap();

        assert_eq!(opens, 2);
        assert_eq!(initializers, 2);
        let mut rows = conn.query("PRAGMA busy_timeout", ()).await.unwrap();
        let row = rows.next().await.unwrap().unwrap();
        let timeout: i64 = row.get(0).unwrap();
        assert_eq!(timeout, 5000);
    }

    /// A connection that opens successfully but whose PRAGMA batch fails on
    /// *every* attempt (not just a transient one, unlike the test above)
    /// must exhaust the fixed retry budget and surface a `Connect`
    /// infrastructure error carrying the PRAGMA failure as its cause —
    /// mirroring the permanent-open-failure test, but for the PRAGMA half
    /// of `connect_with_retry_and_pragmas`'s two failure branches.
    #[tokio::test]
    async fn connect_with_retry_returns_connect_error_after_exhausting_pragma_failures() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("connect-retry-pragma-permanent-test.db");
        let db = libsql::Builder::new_local(db_path).build().await.unwrap();
        let mut opens = 0;
        let mut initializers = 0;

        let result = connect_with_retry_and_pragmas(
            || {
                opens += 1;
                db.connect()
            },
            |_| {
                initializers += 1;
                "THIS IS NOT SQL"
            },
        )
        .await;

        assert_eq!(
            opens, LIBSQL_CONNECT_ATTEMPTS,
            "must stop after exhausting the fixed retry budget, not retry forever"
        );
        assert_eq!(
            initializers, LIBSQL_CONNECT_ATTEMPTS,
            "every open succeeded, so every attempt must have reached the PRAGMA batch"
        );
        match result {
            Err(FilesystemError::BackendInfrastructure { operation, reason }) => {
                assert_eq!(operation, FilesystemOperation::Connect);
                assert!(
                    reason.contains("THIS IS NOT SQL") || reason.to_lowercase().contains("syntax"),
                    "reason must include the final PRAGMA attempt's cause: {reason}"
                );
            }
            other => panic!("expected FilesystemError::BackendInfrastructure, got {other:?}"),
        }
    }
}
