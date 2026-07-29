//! Shared libSQL connection runtime.
//!
//! SQLite WAL permits concurrent readers but still admits only one writer.
//! This runtime makes that constraint explicit: adapters share an eight-slot
//! reader pool and a one-slot writer pool for one `libsql::Database`.

use std::{
    fmt,
    ops::Deref,
    sync::Arc,
    time::{Duration, Instant},
};

use deadpool::managed::{Manager, Metrics, Object, Pool, PoolError, RecycleError, RecycleResult};
use thiserror::Error;

pub const LIBSQL_READ_POOL_MAX_CONNECTIONS: usize = 8;

const LIBSQL_WRITER_POOL_MAX_CONNECTIONS: usize = 1;
const LIBSQL_POOL_CHECKOUT_TIMEOUT: Duration = Duration::from_secs(10);
const LIBSQL_CONNECT_ATTEMPTS: u32 = 3;
const LIBSQL_CONNECT_INITIAL_BACKOFF: Duration = Duration::from_millis(50);
const LIBSQL_CONNECTION_PRAGMAS: &str = "\
    PRAGMA busy_timeout = 5000;\
    PRAGMA synchronous = NORMAL;\
    PRAGMA temp_store = MEMORY;\
    PRAGMA cache_size = -16000;\
    PRAGMA mmap_size = 268435456;\
    PRAGMA wal_autocheckpoint = 1000;";

type LibSqlPool = Pool<LibSqlConnectionManager>;

/// Identifies the admission lane without exposing a database target.
#[derive(Debug, Clone, Copy)]
pub enum LibSqlLane {
    Read,
    Write,
}

impl fmt::Display for LibSqlLane {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read => formatter.write_str("read"),
            Self::Write => formatter.write_str("write"),
        }
    }
}

/// Redacted runtime failures safe to map at a storage-adapter boundary.
#[derive(Debug, Error)]
pub enum LibSqlRuntimeError {
    #[error("libSQL connection failed during {operation}")]
    Connection {
        operation: &'static str,
        #[source]
        source: libsql::Error,
    },
    #[error("libSQL {lane} connection checkout failed ({reason})")]
    Checkout {
        lane: LibSqlLane,
        reason: &'static str,
    },
}

/// Exclusive checkout from either the reader or writer pool.
pub struct LibSqlConnectionLease(Object<LibSqlConnectionManager>);

impl Deref for LibSqlConnectionLease {
    type Target = libsql::Connection;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// One process-local connection runtime for one libSQL database.
pub struct LibSqlRuntime {
    read_pool: LibSqlPool,
    write_pool: LibSqlPool,
}

impl fmt::Debug for LibSqlRuntime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LibSqlRuntime")
            .field("read_pool", &self.read_pool.status())
            .field("write_pool", &self.write_pool.status())
            .finish()
    }
}

impl LibSqlRuntime {
    pub fn new(db: Arc<libsql::Database>) -> Self {
        Self {
            read_pool: build_pool(Arc::clone(&db), LIBSQL_READ_POOL_MAX_CONNECTIONS),
            write_pool: build_pool(db, LIBSQL_WRITER_POOL_MAX_CONNECTIONS),
        }
    }

    pub async fn read(&self) -> Result<LibSqlConnectionLease, LibSqlRuntimeError> {
        checkout(&self.read_pool, LibSqlLane::Read).await
    }

    pub async fn write(&self) -> Result<LibSqlConnectionLease, LibSqlRuntimeError> {
        checkout(&self.write_pool, LibSqlLane::Write).await
    }
}

struct LibSqlConnectionManager {
    db: Arc<libsql::Database>,
}

impl Manager for LibSqlConnectionManager {
    type Type = libsql::Connection;
    type Error = LibSqlRuntimeError;

    async fn create(&self) -> Result<Self::Type, Self::Error> {
        connect_with_retry(|| self.db.connect()).await
    }

    async fn recycle(
        &self,
        connection: &mut Self::Type,
        _metrics: &Metrics,
    ) -> RecycleResult<Self::Error> {
        if connection.is_autocommit() {
            Ok(())
        } else {
            Err(RecycleError::message(
                "libSQL connection returned to pool inside an open transaction",
            ))
        }
    }
}

fn build_pool(db: Arc<libsql::Database>, max_size: usize) -> LibSqlPool {
    build_pool_with_config(db, max_size, LIBSQL_POOL_CHECKOUT_TIMEOUT)
}

fn build_pool_with_config(
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
        // The runtime is always configured above, which is the builder's only
        // possible failure when a timeout is present.
        Err(error) => unreachable!("libSQL pool build cannot fail: {error}"),
    }
}

async fn checkout(
    pool: &LibSqlPool,
    lane: LibSqlLane,
) -> Result<LibSqlConnectionLease, LibSqlRuntimeError> {
    let queued = pool.status().waiting;
    let started = Instant::now();
    let result = pool.get().await;
    let wait_ms = started.elapsed().as_millis();
    tracing::debug!(
        lane = %lane,
        wait_ms,
        queued,
        "libSQL connection checkout completed"
    );
    result
        .map(LibSqlConnectionLease)
        .map_err(|error| map_pool_error(error, lane))
}

fn map_pool_error(error: PoolError<LibSqlRuntimeError>, lane: LibSqlLane) -> LibSqlRuntimeError {
    match error {
        PoolError::Backend(error) => error,
        PoolError::Timeout(_) => LibSqlRuntimeError::Checkout {
            lane,
            reason: "timeout",
        },
        PoolError::Closed => LibSqlRuntimeError::Checkout {
            lane,
            reason: "closed",
        },
        PoolError::NoRuntimeSpecified => LibSqlRuntimeError::Checkout {
            lane,
            reason: "runtime unavailable",
        },
        PoolError::PostCreateHook(_) => LibSqlRuntimeError::Checkout {
            lane,
            reason: "post-create hook",
        },
    }
}

async fn connect_with_retry<F>(mut open: F) -> Result<libsql::Connection, LibSqlRuntimeError>
where
    F: FnMut() -> Result<libsql::Connection, libsql::Error>,
{
    connect_with_retry_and_pragmas(&mut open, |_| LIBSQL_CONNECTION_PRAGMAS).await
}

async fn connect_with_retry_and_pragmas<F, P>(
    mut open: F,
    mut pragmas_for_attempt: P,
) -> Result<libsql::Connection, LibSqlRuntimeError>
where
    F: FnMut() -> Result<libsql::Connection, libsql::Error>,
    P: FnMut(u32) -> &'static str,
{
    let mut last_error = None;
    for attempt in 0..LIBSQL_CONNECT_ATTEMPTS {
        match open() {
            Ok(connection) => match connection.execute_batch(pragmas_for_attempt(attempt)).await {
                Ok(_) => return Ok(connection),
                Err(error) => last_error = Some(error),
            },
            Err(error) => last_error = Some(error),
        }
        if attempt + 1 < LIBSQL_CONNECT_ATTEMPTS {
            tokio::time::sleep(LIBSQL_CONNECT_INITIAL_BACKOFF * 2u32.pow(attempt)).await;
        }
    }

    match last_error {
        Some(source) => Err(LibSqlRuntimeError::Connection {
            operation: "open or initialize",
            source,
        }),
        None => Err(LibSqlRuntimeError::Checkout {
            lane: LibSqlLane::Read,
            reason: "connection attempts exhausted",
        }),
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Duration};

    use super::*;

    /// Break caught: permitting two concurrent writer leases would recreate
    /// SQLite writer-lock contention inside one IronClaw process.
    #[tokio::test]
    async fn one_writer_waits_while_a_reader_remains_available() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("runtime.db");
        let database = Arc::new(
            libsql::Builder::new_local(path)
                .build()
                .await
                .expect("database"),
        );
        let runtime = Arc::new(LibSqlRuntime::new(database));

        let first_writer = runtime.write().await.expect("first writer");
        let waiting_runtime = Arc::clone(&runtime);
        let mut second_writer =
            tokio::spawn(async move { waiting_runtime.write().await.expect("second writer") });

        assert!(
            tokio::time::timeout(Duration::from_millis(25), &mut second_writer)
                .await
                .is_err(),
            "a second writer must wait for the sole writer lane"
        );
        tokio::time::timeout(Duration::from_millis(250), runtime.read())
            .await
            .expect("reader must not queue behind writer")
            .expect("reader checkout");

        drop(first_writer);
        tokio::time::timeout(Duration::from_millis(250), second_writer)
            .await
            .expect("second writer admitted")
            .expect("writer task");
    }

    #[tokio::test]
    async fn recycle_rejects_connection_returned_inside_transaction() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("recycle.db");
        let database = Arc::new(
            libsql::Builder::new_local(path)
                .build()
                .await
                .expect("database"),
        );
        let pool = build_pool(database, 1);

        {
            let connection = pool.get().await.expect("first checkout");
            connection
                .execute("BEGIN", ())
                .await
                .expect("begin transaction");
            assert!(!connection.is_autocommit());
        }

        let next = pool.get().await.expect("replacement checkout");
        assert!(
            next.is_autocommit(),
            "a connection returned mid-transaction must be discarded"
        );
    }

    #[tokio::test]
    async fn checkout_timeout_is_redacted_and_typed() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("timeout.db");
        let database = Arc::new(
            libsql::Builder::new_local(path)
                .build()
                .await
                .expect("database"),
        );
        let pool = build_pool_with_config(database, 1, Duration::from_millis(25));
        let _held = pool.get().await.expect("held checkout");

        let error = match checkout(&pool, LibSqlLane::Write).await {
            Ok(_) => panic!("checkout must time out"),
            Err(error) => error,
        };
        assert!(matches!(
            error,
            LibSqlRuntimeError::Checkout {
                lane: LibSqlLane::Write,
                reason: "timeout",
            }
        ));
    }

    #[tokio::test]
    async fn connection_retry_stops_after_the_fixed_budget() {
        let mut attempts = 0;
        let result = connect_with_retry(|| {
            attempts += 1;
            Err(libsql::Error::ConnectionFailed(format!(
                "synthetic permanent failure {attempts}"
            )))
        })
        .await;

        assert_eq!(attempts, LIBSQL_CONNECT_ATTEMPTS);
        assert!(matches!(
            result,
            Err(LibSqlRuntimeError::Connection {
                operation: "open or initialize",
                ..
            })
        ));
    }

    #[tokio::test]
    async fn connection_retry_reopens_after_transient_initialization_failure() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("pragma-retry.db");
        let database = libsql::Builder::new_local(path)
            .build()
            .await
            .expect("database");
        let mut opens = 0;
        let mut initializers = 0;

        let connection = connect_with_retry_and_pragmas(
            || {
                opens += 1;
                database.connect()
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
        .expect("second initialization succeeds");

        assert_eq!(opens, 2);
        assert_eq!(initializers, 2);
        let mut rows = connection
            .query("PRAGMA busy_timeout", ())
            .await
            .expect("busy timeout query");
        let timeout: i64 = rows
            .next()
            .await
            .expect("row read")
            .expect("row")
            .get(0)
            .expect("timeout");
        assert_eq!(timeout, 5000);
    }
}
