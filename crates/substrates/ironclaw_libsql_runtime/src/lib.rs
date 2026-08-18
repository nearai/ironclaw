//! Shared libSQL connection runtime.
//!
//! SQLite WAL permits concurrent readers but still admits only one writer.
//! This runtime makes that constraint explicit: adapters share an eight-slot
//! reader pool and a one-slot writer pool for one `libsql::Database`.

use std::{
    fmt,
    ops::Deref,
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use deadpool::managed::{
    BuildError, Manager, Metrics, Object, Pool, PoolError, RecycleError, RecycleResult,
};
use thiserror::Error;

pub const LIBSQL_READ_POOL_MAX_CONNECTIONS: usize = 8;

const LIBSQL_WRITER_POOL_MAX_CONNECTIONS: usize = 1;
/// Read slots on a journal lane (`LibSqlRuntime::split_journal_lane`).
///
/// A journal lane serves one latency-sensitive writer plus its replay reads,
/// not the data plane's fan-out, so it keeps a deliberately small reader pool.
const LIBSQL_JOURNAL_LANE_READ_POOL_MAX_CONNECTIONS: usize = 2;
const LIBSQL_POOL_CHECKOUT_TIMEOUT: Duration = Duration::from_secs(10);
const LIBSQL_CONNECT_ATTEMPTS: u32 = 3;
const LIBSQL_CONNECT_INITIAL_BACKOFF: Duration = Duration::from_millis(50);
const LIBSQL_WRITE_CONNECTION_PRAGMAS: &str = "\
    PRAGMA busy_timeout = 5000;\
    PRAGMA synchronous = NORMAL;\
    PRAGMA temp_store = MEMORY;\
    PRAGMA cache_size = -16000;\
    PRAGMA mmap_size = 268435456;\
    PRAGMA wal_autocheckpoint = 1000;";
const LIBSQL_READ_CONNECTION_PRAGMAS: &str = "\
    PRAGMA busy_timeout = 5000;\
    PRAGMA synchronous = NORMAL;\
    PRAGMA temp_store = MEMORY;\
    PRAGMA cache_size = -16000;\
    PRAGMA mmap_size = 268435456;\
    PRAGMA wal_autocheckpoint = 1000;\
    PRAGMA query_only = ON;";

type LibSqlPool = Pool<LibSqlConnectionManager>;

/// Identifies the admission lane without exposing a database target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

/// Stable classification for pool-checkout failures. Adapters use this typed
/// reason to distinguish retryable writer admission pressure from broken
/// runtime infrastructure without parsing error text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LibSqlCheckoutFailureReason {
    Timeout,
    Closed,
    RuntimeUnavailable,
    PostCreateHook,
}

impl fmt::Display for LibSqlCheckoutFailureReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Timeout => formatter.write_str("timeout"),
            Self::Closed => formatter.write_str("closed"),
            Self::RuntimeUnavailable => formatter.write_str("runtime unavailable"),
            Self::PostCreateHook => formatter.write_str("post-create hook"),
        }
    }
}

/// Redacted runtime failures safe to map at a storage-adapter boundary.
#[derive(Debug, Error)]
pub enum LibSqlRuntimeError {
    #[error("libSQL database construction failed")]
    DatabaseBuild {
        #[source]
        source: libsql::Error,
    },
    #[error("libSQL connection failed during {operation}")]
    Connection {
        operation: &'static str,
        #[source]
        source: libsql::Error,
    },
    #[error("libSQL {lane} connection checkout failed ({reason})")]
    Checkout {
        lane: LibSqlLane,
        reason: LibSqlCheckoutFailureReason,
    },
    #[error("libSQL {lane} pool construction failed")]
    PoolBuild {
        lane: LibSqlLane,
        #[source]
        source: BuildError,
    },
    #[error("libSQL writer acquisition is not reentrant")]
    ReentrantWriter,
    #[error("libSQL journal lane was already split from this runtime")]
    JournalLaneAlreadySplit,
}

/// Checkout from a connection pool configured with `PRAGMA query_only = ON`.
///
/// The lease deliberately exposes only the row-returning query API. The
/// connection itself stays private, and libSQL rejects write SQL even if it is
/// submitted through `query`.
pub struct LibSqlReadConnectionLease(Object<LibSqlConnectionManager>);

impl LibSqlReadConnectionLease {
    pub async fn query(
        &self,
        sql: &str,
        params: impl libsql::params::IntoParams,
    ) -> libsql::Result<libsql::Rows> {
        self.0.query(sql, params).await
    }
}

/// Exclusive checkout from the single-slot writer pool.
pub struct LibSqlWriteConnectionLease {
    connection: Object<LibSqlConnectionManager>,
    _holder: LibSqlWriterHolderGuard,
}

struct LibSqlWriterHolderGuard {
    writer_holder: Arc<Mutex<Option<tokio::task::Id>>>,
    holder_task_id: Option<tokio::task::Id>,
}

impl LibSqlWriteConnectionLease {
    /// Permanently remove this connection from the pool.
    ///
    /// Cancellation cleanup uses this when the connection may still own an
    /// open transaction. Physically dropping it releases SQLite's writer lock
    /// immediately; a later checkout creates a clean replacement.
    pub fn discard(self) {
        let Self {
            connection,
            _holder,
        } = self;
        drop(Object::take(connection));
    }
}

impl Deref for LibSqlWriteConnectionLease {
    type Target = libsql::Connection;

    fn deref(&self) -> &Self::Target {
        &self.connection
    }
}

impl Drop for LibSqlWriterHolderGuard {
    fn drop(&mut self) {
        let Some(holder_task_id) = self.holder_task_id else {
            return;
        };
        let mut holder = recover_writer_holder_lock(&self.writer_holder);
        if holder.as_ref() == Some(&holder_task_id) {
            *holder = None;
        }
    }
}

/// One process-local connection runtime for one libSQL database.
pub struct LibSqlRuntime {
    db: Arc<libsql::Database>,
    read_pool: LibSqlPool,
    write_pool: LibSqlPool,
    writer_holder: Arc<Mutex<Option<tokio::task::Id>>>,
    journal_lane_split: Arc<AtomicBool>,
    /// Exact connection target used when this runtime opened its own database.
    /// Caller-supplied handles intentionally carry no target provenance.
    opened_target: Option<Arc<str>>,
}

impl fmt::Debug for LibSqlRuntime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LibSqlRuntime")
            .field("read_pool", &self.read_pool.status())
            .field("write_pool", &self.write_pool.status())
            .field(
                "writer_holder_present",
                &recover_writer_holder_lock(&self.writer_holder).is_some(),
            )
            .field("opened_target_present", &self.opened_target.is_some())
            .finish()
    }
}

impl LibSqlRuntime {
    pub fn new(db: Arc<libsql::Database>) -> Result<Self, LibSqlRuntimeError> {
        Self::from_database(db, None)
    }

    /// Open one libSQL target and construct its shared admission pools.
    ///
    /// The retained target is private provenance. Production composition can
    /// verify that the runtime and its durability claim came from the same
    /// construction input instead of trusting a second caller-supplied string.
    pub async fn open(
        path_or_url: impl Into<String>,
        auth_token: Option<String>,
    ) -> Result<Self, LibSqlRuntimeError> {
        let path_or_url = path_or_url.into();
        let database = if is_remote_libsql_target(&path_or_url) {
            libsql::Builder::new_remote(path_or_url.clone(), auth_token.unwrap_or_default())
                .build()
                .await
        } else {
            libsql::Builder::new_local(&path_or_url).build().await
        }
        .map_err(|source| LibSqlRuntimeError::DatabaseBuild { source })?;
        Self::from_database(Arc::new(database), Some(Arc::from(path_or_url)))
    }

    /// Whether this runtime itself opened `path_or_url`.
    ///
    /// Runtimes built from caller-supplied database handles return false
    /// because the handle carries no verifiable target identity.
    pub fn target_matches(&self, path_or_url: &str) -> bool {
        self.opened_target
            .as_deref()
            .is_some_and(|opened| opened == path_or_url)
    }

    fn from_database(
        db: Arc<libsql::Database>,
        opened_target: Option<Arc<str>>,
    ) -> Result<Self, LibSqlRuntimeError> {
        Self::with_read_pool_size(
            db,
            opened_target,
            LIBSQL_READ_POOL_MAX_CONNECTIONS,
            Arc::new(AtomicBool::new(false)),
        )
    }

    fn with_read_pool_size(
        db: Arc<libsql::Database>,
        opened_target: Option<Arc<str>>,
        read_pool_max_connections: usize,
        journal_lane_split: Arc<AtomicBool>,
    ) -> Result<Self, LibSqlRuntimeError> {
        Ok(Self {
            read_pool: build_pool(Arc::clone(&db), read_pool_max_connections, LibSqlLane::Read)?,
            write_pool: build_pool(
                Arc::clone(&db),
                LIBSQL_WRITER_POOL_MAX_CONNECTIONS,
                LibSqlLane::Write,
            )?,
            db,
            writer_holder: Arc::new(Mutex::new(None)),
            journal_lane_split,
            opened_target,
        })
    }

    /// A second admission runtime over the same database, for one
    /// latency-sensitive journal writer.
    ///
    /// The data-plane runtime admits exactly one writer at a time, and callers
    /// queue for that slot FIFO with a multi-second checkout timeout. Bulk
    /// per-turn traffic (events, messages) therefore parks a journal write —
    /// the resource-governor delta journal, the process journal — behind an
    /// arbitrarily deep queue of unrelated writes, which is how a heartbeat
    /// times out on a database that is not actually overloaded
    /// (nearai/ironclaw#7714). Postgres solves this with a dedicated pool
    /// (#7471); this is the libSQL equivalent.
    ///
    /// Why a second connection helps even though SQLite has one writer per
    /// database file: the two lanes contend for SQLite's write lock, not for a
    /// pool slot. `run_migrations` puts the database in WAL journaling, so a
    /// write lock is held only for the duration of one short transaction and
    /// `busy_timeout` retries across it, whereas a pool slot is held for as
    /// long as its holder keeps the lease and grants nothing to a starved
    /// waiter. The queue the journal waits in goes from "every writer in the
    /// process" to "one transaction".
    ///
    /// This deliberately admits exactly one extra process-wide writer. Raising
    /// [`LIBSQL_WRITER_POOL_MAX_CONNECTIONS`] instead would let unbounded bulk
    /// writers fight over the write lock, which is the contention the
    /// single-slot pool exists to prevent. For the same reason all journals
    /// share this one lane rather than taking a lane each.
    ///
    /// Constraint: the reentrancy guard is lane-local, and so is the writer
    /// slot. A data-plane transaction that holds SQLite's write lock (the
    /// filesystem's `BEGIN IMMEDIATE` batches) therefore blocks a journal-lane
    /// write for up to the connection's `busy_timeout` before it surfaces as
    /// `BackendBusy` — the lane bounds queueing, it does not bypass the file
    /// lock. That is why the journals' own retry windows must exceed a single
    /// backend attempt (see `DEFAULT_BUSY_RETRY_POLICY` in the resource
    /// governor). Callers must not hold a data-plane write lease across a
    /// journal-lane write: the guard cannot see the other lane's holder, so the
    /// self-deadlock that `ReentrantWriter` catches within a lane goes
    /// undetected across lanes and stalls for the full `busy_timeout` instead.
    pub fn split_journal_lane(&self) -> Result<Self, LibSqlRuntimeError> {
        self.journal_lane_split
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| LibSqlRuntimeError::JournalLaneAlreadySplit)?;
        let lane = Self::with_read_pool_size(
            Arc::clone(&self.db),
            self.opened_target.clone(),
            LIBSQL_JOURNAL_LANE_READ_POOL_MAX_CONNECTIONS,
            Arc::clone(&self.journal_lane_split),
        );
        if lane.is_err() {
            self.journal_lane_split.store(false, Ordering::Release);
        }
        lane
    }

    pub async fn read(&self) -> Result<LibSqlReadConnectionLease, LibSqlRuntimeError> {
        checkout(&self.read_pool, LibSqlLane::Read)
            .await
            .map(LibSqlReadConnectionLease)
    }

    pub async fn write(&self) -> Result<LibSqlWriteConnectionLease, LibSqlRuntimeError> {
        let holder_task_id = tokio::task::try_id();
        if holder_task_id.is_some()
            && *recover_writer_holder_lock(&self.writer_holder) == holder_task_id
        {
            return Err(LibSqlRuntimeError::ReentrantWriter);
        }
        let connection = checkout(&self.write_pool, LibSqlLane::Write).await?;
        if let Some(holder_task_id) = holder_task_id {
            *recover_writer_holder_lock(&self.writer_holder) = Some(holder_task_id);
        }
        Ok(LibSqlWriteConnectionLease {
            connection,
            _holder: LibSqlWriterHolderGuard {
                writer_holder: Arc::clone(&self.writer_holder),
                holder_task_id,
            },
        })
    }
}

fn is_remote_libsql_target(path_or_url: &str) -> bool {
    let Some((scheme, _)) = path_or_url.split_once("://") else {
        return false;
    };
    scheme.eq_ignore_ascii_case("libsql")
        || scheme.eq_ignore_ascii_case("https")
        || scheme.eq_ignore_ascii_case("http")
}

fn recover_writer_holder_lock(
    holder: &Mutex<Option<tokio::task::Id>>,
) -> MutexGuard<'_, Option<tokio::task::Id>> {
    match holder.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

struct LibSqlConnectionManager {
    db: Arc<libsql::Database>,
    lane: LibSqlLane,
}

impl Manager for LibSqlConnectionManager {
    type Type = libsql::Connection;
    type Error = LibSqlRuntimeError;

    async fn create(&self) -> Result<Self::Type, Self::Error> {
        connect_with_retry(|| self.db.connect(), connection_pragmas(self.lane)).await
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

fn connection_pragmas(lane: LibSqlLane) -> &'static str {
    match lane {
        LibSqlLane::Read => LIBSQL_READ_CONNECTION_PRAGMAS,
        LibSqlLane::Write => LIBSQL_WRITE_CONNECTION_PRAGMAS,
    }
}

fn build_pool(
    db: Arc<libsql::Database>,
    max_size: usize,
    lane: LibSqlLane,
) -> Result<LibSqlPool, LibSqlRuntimeError> {
    build_pool_with_config(db, max_size, lane, LIBSQL_POOL_CHECKOUT_TIMEOUT)
}

fn build_pool_with_config(
    db: Arc<libsql::Database>,
    max_size: usize,
    lane: LibSqlLane,
    wait_timeout: Duration,
) -> Result<LibSqlPool, LibSqlRuntimeError> {
    Pool::builder(LibSqlConnectionManager { db, lane })
        .max_size(max_size)
        .wait_timeout(Some(wait_timeout))
        .runtime(deadpool::Runtime::Tokio1)
        .build()
        .map_err(|source| LibSqlRuntimeError::PoolBuild { lane, source })
}

async fn checkout(
    pool: &LibSqlPool,
    lane: LibSqlLane,
) -> Result<Object<LibSqlConnectionManager>, LibSqlRuntimeError> {
    let queued = pool.status().waiting;
    let started = Instant::now();
    let result = pool.get().await;
    let wait_ms = started.elapsed().as_millis();
    match result {
        Ok(connection) => {
            tracing::trace!(
                lane = %lane,
                wait_ms,
                queued,
                "libSQL connection checkout completed"
            );
            Ok(connection)
        }
        Err(error) => {
            let error = map_pool_error(error, lane);
            tracing::debug!(
                lane = %lane,
                wait_ms,
                queued,
                reason = %error,
                "libSQL connection checkout failed"
            );
            Err(error)
        }
    }
}

fn map_pool_error(error: PoolError<LibSqlRuntimeError>, lane: LibSqlLane) -> LibSqlRuntimeError {
    match error {
        PoolError::Backend(error) => error,
        PoolError::Timeout(_) => LibSqlRuntimeError::Checkout {
            lane,
            reason: LibSqlCheckoutFailureReason::Timeout,
        },
        PoolError::Closed => LibSqlRuntimeError::Checkout {
            lane,
            reason: LibSqlCheckoutFailureReason::Closed,
        },
        PoolError::NoRuntimeSpecified => LibSqlRuntimeError::Checkout {
            lane,
            reason: LibSqlCheckoutFailureReason::RuntimeUnavailable,
        },
        PoolError::PostCreateHook(_) => LibSqlRuntimeError::Checkout {
            lane,
            reason: LibSqlCheckoutFailureReason::PostCreateHook,
        },
    }
}

async fn connect_with_retry<F>(
    mut open: F,
    pragmas: &'static str,
) -> Result<libsql::Connection, LibSqlRuntimeError>
where
    F: FnMut() -> Result<libsql::Connection, libsql::Error>,
{
    connect_with_retry_and_pragmas(&mut open, |_| pragmas).await
}

async fn connect_with_retry_and_pragmas<F, P>(
    mut open: F,
    mut pragmas_for_attempt: P,
) -> Result<libsql::Connection, LibSqlRuntimeError>
where
    F: FnMut() -> Result<libsql::Connection, libsql::Error>,
    P: FnMut(u32) -> &'static str,
{
    let mut attempt = 0;
    loop {
        let error = match open() {
            Ok(connection) => match connection.execute_batch(pragmas_for_attempt(attempt)).await {
                Ok(_) => return Ok(connection),
                Err(error) => error,
            },
            Err(error) => error,
        };
        attempt += 1;
        if attempt >= LIBSQL_CONNECT_ATTEMPTS {
            return Err(LibSqlRuntimeError::Connection {
                operation: "open or initialize",
                source: error,
            });
        }
        tokio::time::sleep(LIBSQL_CONNECT_INITIAL_BACKOFF * 2u32.pow(attempt - 1)).await;
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
        let runtime = Arc::new(LibSqlRuntime::new(database).expect("runtime"));

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

    /// The starvation fix for nearai/ironclaw#7714: a journal lane must be
    /// admitted while the data-plane writer slot is occupied, and its write
    /// must land in the same database the data plane reads. If the lane ever
    /// shared the data-plane pool again, this checkout would block until the
    /// pool timeout instead — exactly the ~10s-per-attempt stall from the
    /// bench log.
    #[tokio::test]
    async fn journal_lane_writes_while_the_data_plane_writer_is_held() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("journal-lane.db");
        let database = Arc::new(
            libsql::Builder::new_local(path)
                .build()
                .await
                .expect("database"),
        );
        let data_plane = LibSqlRuntime::new(Arc::clone(&database)).expect("data-plane runtime");
        let journal_lane = data_plane.split_journal_lane().expect("journal lane");

        let setup = data_plane.write().await.expect("setup writer");
        setup
            .execute_batch("PRAGMA journal_mode = WAL")
            .await
            .expect("wal journaling");
        setup
            .execute("CREATE TABLE journal (value TEXT NOT NULL)", ())
            .await
            .expect("create journal table");
        drop(setup);

        // Bulk traffic occupies the sole data-plane writer slot for the whole
        // journal write, the way a per-turn write burst does in production.
        let data_plane_writer = data_plane.write().await.expect("data-plane writer");

        let journal_writer = tokio::time::timeout(Duration::from_millis(500), journal_lane.write())
            .await
            .expect("journal lane must not queue behind the data-plane writer")
            .expect("journal writer");
        journal_writer
            .execute("INSERT INTO journal (value) VALUES ('heartbeat')", ())
            .await
            .expect("journal write must commit while the data plane holds its slot");
        drop(journal_writer);
        drop(data_plane_writer);

        let reader = data_plane.read().await.expect("reader");
        let mut rows = reader
            .query("SELECT value FROM journal", ())
            .await
            .expect("read journal rows");
        let value: String = rows
            .next()
            .await
            .expect("row lookup")
            .expect("journal row")
            .get(0)
            .expect("journal value");
        assert_eq!(
            value, "heartbeat",
            "the journal lane must address the same database the data plane reads"
        );
    }

    #[tokio::test]
    async fn only_one_journal_lane_can_be_split_from_a_runtime() {
        let database = Arc::new(
            libsql::Builder::new_local(":memory:")
                .build()
                .await
                .expect("database"),
        );
        let data_plane = LibSqlRuntime::new(database).expect("data-plane runtime");

        let journal_lane = data_plane.split_journal_lane().expect("journal lane");
        assert!(matches!(
            journal_lane.split_journal_lane(),
            Err(LibSqlRuntimeError::JournalLaneAlreadySplit)
        ));
        assert!(matches!(
            data_plane.split_journal_lane(),
            Err(LibSqlRuntimeError::JournalLaneAlreadySplit)
        ));
    }

    #[tokio::test]
    async fn reader_lane_rejects_write_sql() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("reader-is-read-only.db");
        let database = Arc::new(
            libsql::Builder::new_local(path)
                .build()
                .await
                .expect("database"),
        );
        let runtime = LibSqlRuntime::new(database).expect("runtime");
        let writer = runtime.write().await.expect("writer");
        writer
            .execute("CREATE TABLE guarded_writes (value TEXT NOT NULL)", ())
            .await
            .expect("create guarded table");
        drop(writer);

        let reader = runtime.read().await.expect("reader");
        let mut pragma_rows = reader
            .query("PRAGMA query_only", ())
            .await
            .expect("query reader enforcement pragma");
        let query_only: i64 = pragma_rows
            .next()
            .await
            .expect("read query_only")
            .expect("query_only row")
            .get(0)
            .expect("query_only value");
        assert_eq!(query_only, 1, "reader connection must enable query_only");
        let result = reader
            .query(
                "INSERT INTO guarded_writes (value) VALUES ('bypass') RETURNING value",
                (),
            )
            .await;
        let rejected = match result {
            Ok(mut rows) => rows.next().await.is_err(),
            Err(_) => true,
        };

        assert!(
            rejected,
            "the reader lane must reject SQL that attempts a write"
        );
    }

    #[tokio::test]
    async fn nested_writer_acquisition_fails_without_waiting_for_pool_timeout() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("nested-writer.db");
        let database = Arc::new(
            libsql::Builder::new_local(path)
                .build()
                .await
                .expect("database"),
        );
        let runtime = Arc::new(LibSqlRuntime::new(database).expect("runtime"));
        let nested = tokio::spawn(async move {
            let _held_writer = runtime.write().await.expect("first writer");
            tokio::time::timeout(Duration::from_millis(25), runtime.write())
                .await
                .expect("nested writer acquisition must fail before the pool timeout")
        })
        .await
        .expect("nested writer test task");

        assert!(
            nested.is_err(),
            "nested writer acquisition must return a typed runtime error"
        );
        assert!(matches!(nested, Err(LibSqlRuntimeError::ReentrantWriter)));
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
        let pool = build_pool(database, 1, LibSqlLane::Write).expect("pool");

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
        let pool =
            build_pool_with_config(database, 1, LibSqlLane::Write, Duration::from_millis(25))
                .expect("pool");
        let _held = pool.get().await.expect("held checkout");

        let error = match checkout(&pool, LibSqlLane::Write).await {
            Ok(_) => panic!("checkout must time out"),
            Err(error) => error,
        };
        assert!(matches!(
            error,
            LibSqlRuntimeError::Checkout {
                lane: LibSqlLane::Write,
                reason: LibSqlCheckoutFailureReason::Timeout,
            }
        ));
    }

    #[tokio::test]
    async fn dropped_immediate_transaction_rolls_back_before_writer_reuse() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("transaction-drop.db");
        let database = Arc::new(
            libsql::Builder::new_local(path)
                .build()
                .await
                .expect("database"),
        );
        let runtime = LibSqlRuntime::new(database).expect("runtime");
        let connection = runtime.write().await.expect("writer");
        connection
            .execute("CREATE TABLE cancellation_safety (value TEXT NOT NULL)", ())
            .await
            .expect("create table");

        {
            let transaction = connection
                .transaction_with_behavior(libsql::TransactionBehavior::Immediate)
                .await
                .expect("begin immediate transaction");
            transaction
                .execute(
                    "INSERT INTO cancellation_safety (value) VALUES ('uncommitted')",
                    (),
                )
                .await
                .expect("insert uncommitted row");
        }

        assert!(
            connection.is_autocommit(),
            "dropping the transaction must release the writer lock"
        );
        let mut rows = connection
            .query("SELECT COUNT(*) FROM cancellation_safety", ())
            .await
            .expect("query rolled-back table");
        let count: i64 = rows
            .next()
            .await
            .expect("read count")
            .expect("count row")
            .get(0)
            .expect("count");
        assert_eq!(count, 0, "dropped transaction must roll back its write");
    }

    #[tokio::test]
    async fn connection_retry_stops_after_the_fixed_budget() {
        let mut attempts = 0;
        let result = connect_with_retry(
            || {
                attempts += 1;
                Err(libsql::Error::ConnectionFailed(format!(
                    "synthetic permanent failure {attempts}"
                )))
            },
            LIBSQL_WRITE_CONNECTION_PRAGMAS,
        )
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
                    LIBSQL_WRITE_CONNECTION_PRAGMAS
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
