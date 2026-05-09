//! Resource reservation governor for IronClaw Reborn.
//!
//! `ironclaw_resources` enforces the host-level reservation protocol used by
//! runtime lanes before they spend money or consume scarce sandbox capacity:
//! reserve estimated resources, execute work, then reconcile actual usage or
//! release the unused hold.

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
#[cfg(any(feature = "libsql", feature = "postgres"))]
use std::future::Future;
use std::io::{ErrorKind, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
#[cfg(any(feature = "libsql", feature = "postgres"))]
use std::sync::{OnceLock, mpsc};

use fs2::FileExt;

use ironclaw_host_api::{
    AgentId, MissionId, ProjectId, ResourceEstimate, ResourceReservationId, ResourceScope,
    ResourceUsage, TenantId, ThreadId, UserId,
};
pub use ironclaw_host_api::{ReservationStatus, ResourceReceipt, ResourceReservation};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Durable account level that can carry resource limits and ledgers.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ResourceAccount {
    Tenant {
        tenant_id: TenantId,
    },
    User {
        tenant_id: TenantId,
        user_id: UserId,
    },
    Project {
        tenant_id: TenantId,
        user_id: UserId,
        project_id: ProjectId,
    },
    Agent {
        tenant_id: TenantId,
        user_id: UserId,
        project_id: Option<ProjectId>,
        agent_id: AgentId,
    },
    Mission {
        tenant_id: TenantId,
        user_id: UserId,
        project_id: Option<ProjectId>,
        mission_id: MissionId,
    },
    Thread {
        tenant_id: TenantId,
        user_id: UserId,
        project_id: Option<ProjectId>,
        mission_id: Option<MissionId>,
        thread_id: ThreadId,
    },
}

impl ResourceAccount {
    pub fn tenant(tenant_id: TenantId) -> Self {
        Self::Tenant { tenant_id }
    }

    pub fn user(tenant_id: TenantId, user_id: UserId) -> Self {
        Self::User { tenant_id, user_id }
    }

    pub fn project(tenant_id: TenantId, user_id: UserId, project_id: ProjectId) -> Self {
        Self::Project {
            tenant_id,
            user_id,
            project_id,
        }
    }

    pub fn agent(
        tenant_id: TenantId,
        user_id: UserId,
        project_id: Option<ProjectId>,
        agent_id: AgentId,
    ) -> Self {
        Self::Agent {
            tenant_id,
            user_id,
            project_id,
            agent_id,
        }
    }

    pub fn mission(
        tenant_id: TenantId,
        user_id: UserId,
        project_id: Option<ProjectId>,
        mission_id: MissionId,
    ) -> Self {
        Self::Mission {
            tenant_id,
            user_id,
            project_id,
            mission_id,
        }
    }

    pub fn thread(
        tenant_id: TenantId,
        user_id: UserId,
        project_id: Option<ProjectId>,
        mission_id: Option<MissionId>,
        thread_id: ThreadId,
    ) -> Self {
        Self::Thread {
            tenant_id,
            user_id,
            project_id,
            mission_id,
            thread_id,
        }
    }

    /// Returns every account whose limit applies to this scope, from broadest to
    /// narrowest.
    ///
    /// A reservation succeeds only if every account returned by this cascade
    /// remains within its limit. Deeper accounts do not override shallower
    /// accounts; tenant, user, project, agent, mission, and thread limits all
    /// apply when present.
    pub fn cascade(scope: &ResourceScope) -> Vec<Self> {
        let mut accounts = vec![
            Self::tenant(scope.tenant_id.clone()),
            Self::user(scope.tenant_id.clone(), scope.user_id.clone()),
        ];

        if let Some(project_id) = &scope.project_id {
            accounts.push(Self::project(
                scope.tenant_id.clone(),
                scope.user_id.clone(),
                project_id.clone(),
            ));
        }

        if let Some(agent_id) = &scope.agent_id {
            accounts.push(Self::agent(
                scope.tenant_id.clone(),
                scope.user_id.clone(),
                scope.project_id.clone(),
                agent_id.clone(),
            ));
        }

        if let Some(mission_id) = &scope.mission_id {
            accounts.push(Self::mission(
                scope.tenant_id.clone(),
                scope.user_id.clone(),
                scope.project_id.clone(),
                mission_id.clone(),
            ));
        }

        if let Some(thread_id) = &scope.thread_id {
            accounts.push(Self::thread(
                scope.tenant_id.clone(),
                scope.user_id.clone(),
                scope.project_id.clone(),
                scope.mission_id.clone(),
                thread_id.clone(),
            ));
        }

        accounts
    }
}

/// Optional maximums for each resource dimension.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceLimits {
    pub max_usd: Option<Decimal>,
    pub max_input_tokens: Option<u64>,
    pub max_output_tokens: Option<u64>,
    pub max_wall_clock_ms: Option<u64>,
    pub max_output_bytes: Option<u64>,
    pub max_network_egress_bytes: Option<u64>,
    pub max_process_count: Option<u32>,
    pub max_concurrency_slots: Option<u32>,
}

/// Resource dimension that may deny a reservation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResourceDimension {
    Usd,
    InputTokens,
    OutputTokens,
    WallClockMs,
    OutputBytes,
    NetworkEgressBytes,
    ProcessCount,
    ConcurrencySlots,
}

impl std::fmt::Display for ResourceDimension {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Usd => "usd",
            Self::InputTokens => "input_tokens",
            Self::OutputTokens => "output_tokens",
            Self::WallClockMs => "wall_clock_ms",
            Self::OutputBytes => "output_bytes",
            Self::NetworkEgressBytes => "network_egress_bytes",
            Self::ProcessCount => "process_count",
            Self::ConcurrencySlots => "concurrency_slots",
        })
    }
}

/// Comparable amount for denial details.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResourceValue {
    Decimal(Decimal),
    Integer(u64),
}

/// Structured reservation denial.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceDenial {
    pub account: ResourceAccount,
    pub dimension: ResourceDimension,
    pub limit: ResourceValue,
    pub current_usage: ResourceValue,
    pub active_reserved: ResourceValue,
    pub requested: ResourceValue,
}

/// Resource governor errors.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ResourceError {
    #[error("resource limit exceeded for {dimension} at {account:?}", account = .0.account, dimension = .0.dimension)]
    LimitExceeded(Box<ResourceDenial>),
    #[error("resource reservation {id} already exists")]
    ReservationAlreadyExists { id: ResourceReservationId },
    #[error("invalid resource estimate for {dimension}: {reason}")]
    InvalidEstimate {
        dimension: ResourceDimension,
        reason: &'static str,
    },
    #[error("resource reservation {id} does not match requested scope or estimate")]
    ReservationMismatch { id: ResourceReservationId },
    #[error("unknown resource reservation {id}")]
    UnknownReservation { id: ResourceReservationId },
    #[error("resource reservation {id} is already {status:?}")]
    ReservationClosed {
        id: ResourceReservationId,
        status: ReservationStatus,
    },
    #[error("resource governor storage error: {reason}")]
    Storage { reason: String },
}

/// Aggregated resource usage/reservation tally.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceTally {
    pub usd: Decimal,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub wall_clock_ms: u64,
    pub output_bytes: u64,
    pub network_egress_bytes: u64,
    pub process_count: u32,
    pub concurrency_slots: u32,
}

impl ResourceTally {
    fn from_estimate(estimate: &ResourceEstimate) -> Self {
        Self {
            usd: estimate.usd.unwrap_or_default(),
            input_tokens: estimate.input_tokens.unwrap_or_default(),
            output_tokens: estimate.output_tokens.unwrap_or_default(),
            wall_clock_ms: estimate.wall_clock_ms.unwrap_or_default(),
            output_bytes: estimate.output_bytes.unwrap_or_default(),
            network_egress_bytes: estimate.network_egress_bytes.unwrap_or_default(),
            process_count: estimate.process_count.unwrap_or_default(),
            concurrency_slots: estimate.concurrency_slots.unwrap_or_default(),
        }
    }

    fn from_usage(usage: &ResourceUsage) -> Self {
        Self {
            usd: usage.usd,
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            wall_clock_ms: usage.wall_clock_ms,
            output_bytes: usage.output_bytes,
            network_egress_bytes: usage.network_egress_bytes,
            process_count: usage.process_count,
            concurrency_slots: 0,
        }
    }

    fn add_assign(&mut self, other: &Self) {
        self.usd = self.usd.checked_add(other.usd).unwrap_or(Decimal::MAX);
        self.input_tokens = self.input_tokens.saturating_add(other.input_tokens);
        self.output_tokens = self.output_tokens.saturating_add(other.output_tokens);
        self.wall_clock_ms = self.wall_clock_ms.saturating_add(other.wall_clock_ms);
        self.output_bytes = self.output_bytes.saturating_add(other.output_bytes);
        self.network_egress_bytes = self
            .network_egress_bytes
            .saturating_add(other.network_egress_bytes);
        self.process_count = self.process_count.saturating_add(other.process_count);
        self.concurrency_slots = self
            .concurrency_slots
            .saturating_add(other.concurrency_slots);
    }

    fn sub_assign(&mut self, other: &Self) {
        self.usd = self
            .usd
            .checked_sub(other.usd)
            .map(|value| value.max(Decimal::ZERO))
            .unwrap_or(Decimal::ZERO);
        self.input_tokens = self.input_tokens.saturating_sub(other.input_tokens);
        self.output_tokens = self.output_tokens.saturating_sub(other.output_tokens);
        self.wall_clock_ms = self.wall_clock_ms.saturating_sub(other.wall_clock_ms);
        self.output_bytes = self.output_bytes.saturating_sub(other.output_bytes);
        self.network_egress_bytes = self
            .network_egress_bytes
            .saturating_sub(other.network_egress_bytes);
        self.process_count = self.process_count.saturating_sub(other.process_count);
        self.concurrency_slots = self
            .concurrency_slots
            .saturating_sub(other.concurrency_slots);
    }
}

/// Synchronous resource governor contract.
pub trait ResourceGovernor: Send + Sync {
    /// Sets or replaces limits for a scoped resource account without mutating existing reservations.
    ///
    /// Persistent governors also expose `try_set_limit` so callers that need
    /// durable write confirmation can observe storage errors.
    fn set_limit(&self, account: ResourceAccount, limits: ResourceLimits);

    /// Reserves estimated resources before costed/quota-limited work starts.
    ///
    /// A reservation succeeds only if every account in [`ResourceAccount::cascade`]
    /// would remain within its limits. Limits at deeper accounts do not override
    /// shallower limits; tenant, user, project, agent, mission, and thread limits
    /// all apply when present.
    fn reserve(
        &self,
        scope: ResourceScope,
        estimate: ResourceEstimate,
    ) -> Result<ResourceReservation, ResourceError>;

    /// Reserves estimated resources with a caller-supplied reservation id for obligation handoff.
    fn reserve_with_id(
        &self,
        scope: ResourceScope,
        estimate: ResourceEstimate,
        reservation_id: ResourceReservationId,
    ) -> Result<ResourceReservation, ResourceError>;

    /// Reconciles an active reservation with actual usage and releases reserved capacity exactly once.
    fn reconcile(
        &self,
        reservation_id: ResourceReservationId,
        actual: ResourceUsage,
    ) -> Result<ResourceReceipt, ResourceError>;

    /// Releases an active reservation without usage when work is cancelled or fails before reconciliation.
    fn release(
        &self,
        reservation_id: ResourceReservationId,
    ) -> Result<ResourceReceipt, ResourceError>;
}

/// Serializable governor snapshot stored by durable stores.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceGovernorSnapshot {
    state: ResourceState,
}

/// Transactional storage primitive for [`PersistentResourceGovernor`].
///
/// Implementations must serialize the whole closure with any other readers or
/// writers over the same account-wide ledger before writing the updated
/// snapshot back durably.
pub trait ResourceGovernorStore: Send + Sync + 'static {
    fn update<T, F>(&self, update: F) -> Result<T, ResourceError>
    where
        T: Send + 'static,
        F: FnOnce(&mut ResourceGovernorSnapshot) -> Result<T, ResourceError> + Send + 'static;
}

/// File-backed resource-governor store using a stable sidecar lock file around
/// each load/update/atomic-rename transaction.
#[derive(Debug, Clone)]
pub struct JsonFileResourceGovernorStore {
    path: PathBuf,
}

impl JsonFileResourceGovernorStore {
    pub fn new(path: impl AsRef<Path>) -> Self {
        Self {
            path: path.as_ref().to_path_buf(),
        }
    }
}

impl ResourceGovernorStore for JsonFileResourceGovernorStore {
    fn update<T, F>(&self, update: F) -> Result<T, ResourceError>
    where
        T: Send + 'static,
        F: FnOnce(&mut ResourceGovernorSnapshot) -> Result<T, ResourceError> + Send + 'static,
    {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(storage_error)?;
        }

        let lock_path = lock_path_for(&self.path);
        let lock_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path)
            .map_err(storage_error)?;
        lock_file.lock_exclusive().map_err(storage_error)?;

        let result = update_file_snapshot(&self.path, update);
        let unlock_result = lock_file.unlock().map_err(storage_error);
        match (result, unlock_result) {
            (Ok(value), Ok(())) => Ok(value),
            (Err(error), _) => Err(error),
            (Ok(_), Err(error)) => Err(error),
        }
    }
}

fn lock_path_for(path: &Path) -> PathBuf {
    let mut lock_path = path.as_os_str().to_owned();
    lock_path.push(".lock");
    PathBuf::from(lock_path)
}

fn temp_path_for(path: &Path) -> PathBuf {
    let mut temp_path = path.as_os_str().to_owned();
    temp_path.push(format!(".{}.tmp", ResourceReservationId::new()));
    PathBuf::from(temp_path)
}

fn update_file_snapshot<T, F>(path: &Path, update: F) -> Result<T, ResourceError>
where
    F: FnOnce(&mut ResourceGovernorSnapshot) -> Result<T, ResourceError>,
{
    let mut snapshot = read_file_snapshot(path)?;
    let value = update(&mut snapshot)?;
    write_file_snapshot_atomically(path, &snapshot)?;
    Ok(value)
}

fn read_file_snapshot(path: &Path) -> Result<ResourceGovernorSnapshot, ResourceError> {
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return Ok(ResourceGovernorSnapshot::default());
        }
        Err(error) => return Err(storage_error(error)),
    };
    let mut contents = String::new();
    file.read_to_string(&mut contents).map_err(storage_error)?;
    if contents.trim().is_empty() {
        Ok(ResourceGovernorSnapshot::default())
    } else {
        serde_json::from_str(&contents).map_err(storage_error)
    }
}

fn write_file_snapshot_atomically(
    path: &Path,
    snapshot: &ResourceGovernorSnapshot,
) -> Result<(), ResourceError> {
    let temp_path = temp_path_for(path);
    let encoded = serde_json::to_vec_pretty(snapshot).map_err(storage_error)?;
    let write_result = write_temp_snapshot(&temp_path, &encoded)
        .and_then(|()| std::fs::rename(&temp_path, path).map_err(storage_error))
        .and_then(|()| sync_parent_dir(path));
    if write_result.is_err() {
        let _ = std::fs::remove_file(&temp_path);
    }
    write_result
}

fn write_temp_snapshot(temp_path: &Path, encoded: &[u8]) -> Result<(), ResourceError> {
    let mut temp_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(temp_path)
        .map_err(storage_error)?;
    temp_file.write_all(encoded).map_err(storage_error)?;
    temp_file.write_all(b"\n").map_err(storage_error)?;
    temp_file.sync_all().map_err(storage_error)
}

fn sync_parent_dir(path: &Path) -> Result<(), ResourceError> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    File::open(parent)
        .and_then(|dir| dir.sync_all())
        .map_err(storage_error)
}

fn storage_error(error: impl std::fmt::Display) -> ResourceError {
    ResourceError::Storage {
        reason: error.to_string(),
    }
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
const SNAPSHOT_ID: &str = "default";
#[cfg(any(feature = "libsql", feature = "postgres"))]
const RESOURCE_GOVERNOR_SCHEMA: &str = "\
CREATE TABLE IF NOT EXISTS ironclaw_resource_governor_snapshots (\
    snapshot_id TEXT PRIMARY KEY,\
    state_json TEXT NOT NULL,\
    updated_at_ms BIGINT NOT NULL DEFAULT 0\
);";

#[cfg(any(feature = "libsql", feature = "postgres"))]
type AsyncStorageJob = Box<dyn FnOnce(&tokio::runtime::Runtime) + Send + 'static>;

#[cfg(any(feature = "libsql", feature = "postgres"))]
#[derive(Debug, Clone)]
struct AsyncStorageWorker {
    sender: mpsc::Sender<AsyncStorageJob>,
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
impl AsyncStorageWorker {
    fn spawn(name: &'static str) -> Result<Self, ResourceError> {
        let (sender, receiver) = mpsc::channel::<AsyncStorageJob>();
        let (ready_sender, ready_receiver) = mpsc::channel();
        std::thread::Builder::new()
            .name(name.to_string())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        let _ = ready_sender.send(Err(storage_error(error)));
                        return;
                    }
                };
                let _ = ready_sender.send(Ok(()));
                while let Ok(job) = receiver.recv() {
                    job(&runtime);
                }
            })
            .map_err(storage_error)?;
        ready_receiver
            .recv()
            .map_err(|_| storage_error("resource governor storage worker failed to start"))??;
        Ok(Self { sender })
    }

    fn run<T, Fut, F>(&self, build: F) -> Result<T, ResourceError>
    where
        T: Send + 'static,
        Fut: Future<Output = Result<T, ResourceError>> + Send + 'static,
        F: FnOnce() -> Fut + Send + 'static,
    {
        let (result_sender, result_receiver) = mpsc::channel();
        self.sender
            .send(Box::new(move |runtime| {
                let result = runtime.block_on(build());
                let _ = result_sender.send(result);
            }))
            .map_err(|_| storage_error("resource governor storage worker stopped"))?;
        result_receiver
            .recv()
            .map_err(|_| storage_error("resource governor storage worker stopped"))?
    }
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
fn run_async_on_shared_storage_worker<T, Fut, F>(build: F) -> Result<T, ResourceError>
where
    T: Send + 'static,
    Fut: Future<Output = Result<T, ResourceError>> + Send + 'static,
    F: FnOnce() -> Fut + Send + 'static,
{
    static WORKER: OnceLock<Result<AsyncStorageWorker, String>> = OnceLock::new();
    let worker = WORKER.get_or_init(|| {
        AsyncStorageWorker::spawn("resource-governor-storage").map_err(|error| error.to_string())
    });
    match worker {
        Ok(worker) => worker.run(build),
        Err(error) => Err(storage_error(error)),
    }
}

#[cfg(feature = "libsql")]
#[derive(Debug, Clone)]
pub struct LibSqlResourceGovernorStore {
    db: std::sync::Arc<libsql::Database>,
}

#[cfg(feature = "libsql")]
impl LibSqlResourceGovernorStore {
    pub fn new(db: std::sync::Arc<libsql::Database>) -> Self {
        Self { db }
    }

    pub async fn run_migrations(&self) -> Result<(), ResourceError> {
        let conn = self.connect().await?;
        conn.execute_batch("BEGIN IMMEDIATE")
            .await
            .map_err(storage_error)?;
        let result = conn.execute_batch(RESOURCE_GOVERNOR_SCHEMA).await;
        match result {
            Ok(_) => conn
                .execute_batch("COMMIT")
                .await
                .map(|_| ())
                .map_err(storage_error),
            Err(error) => {
                let _ = conn.execute_batch("ROLLBACK").await;
                Err(storage_error(error))
            }
        }
    }

    async fn connect(&self) -> Result<libsql::Connection, ResourceError> {
        let conn = self.db.connect().map_err(storage_error)?;
        conn.query("PRAGMA busy_timeout = 5000", ())
            .await
            .map_err(storage_error)?;
        Ok(conn)
    }
}

#[cfg(feature = "libsql")]
impl ResourceGovernorStore for LibSqlResourceGovernorStore {
    fn update<T, F>(&self, update: F) -> Result<T, ResourceError>
    where
        T: Send + 'static,
        F: FnOnce(&mut ResourceGovernorSnapshot) -> Result<T, ResourceError> + Send + 'static,
    {
        let store = self.clone();
        run_async_on_shared_storage_worker(move || async move {
            let conn = store.connect().await?;
            conn.execute_batch("BEGIN IMMEDIATE")
                .await
                .map_err(storage_error)?;
            let result = update_libsql_snapshot(&conn, update).await;
            match result {
                Ok(value) => {
                    conn.execute_batch("COMMIT").await.map_err(storage_error)?;
                    Ok(value)
                }
                Err(error) => {
                    let _ = conn.execute_batch("ROLLBACK").await;
                    Err(error)
                }
            }
        })
    }
}

#[cfg(feature = "libsql")]
async fn update_libsql_snapshot<T, F>(
    conn: &libsql::Connection,
    update: F,
) -> Result<T, ResourceError>
where
    F: FnOnce(&mut ResourceGovernorSnapshot) -> Result<T, ResourceError>,
{
    let mut rows = conn
        .query(
            "SELECT state_json FROM ironclaw_resource_governor_snapshots WHERE snapshot_id = ?1",
            libsql::params![SNAPSHOT_ID],
        )
        .await
        .map_err(storage_error)?;
    let mut snapshot = if let Some(row) = rows.next().await.map_err(storage_error)? {
        let state_json: String = row.get(0).map_err(storage_error)?;
        serde_json::from_str(&state_json).map_err(storage_error)?
    } else {
        ResourceGovernorSnapshot::default()
    };

    let value = update(&mut snapshot)?;
    let encoded = serde_json::to_string(&snapshot).map_err(storage_error)?;
    conn.execute(
        "INSERT INTO ironclaw_resource_governor_snapshots (snapshot_id, state_json, updated_at_ms) \
         VALUES (?1, ?2, strftime('%s','now') * 1000) \
         ON CONFLICT(snapshot_id) DO UPDATE SET \
         state_json = excluded.state_json, updated_at_ms = excluded.updated_at_ms",
        libsql::params![SNAPSHOT_ID, encoded],
    )
    .await
    .map_err(storage_error)?;
    Ok(value)
}

#[cfg(feature = "postgres")]
#[derive(Debug, Clone)]
pub struct PostgresResourceGovernorStore {
    pool: deadpool_postgres::Pool,
}

#[cfg(feature = "postgres")]
impl PostgresResourceGovernorStore {
    pub fn new(pool: deadpool_postgres::Pool) -> Self {
        Self { pool }
    }

    pub async fn run_migrations(&self) -> Result<(), ResourceError> {
        let mut client = self.pool.get().await.map_err(storage_error)?;
        let transaction = client.transaction().await.map_err(storage_error)?;
        let result = transaction.batch_execute(RESOURCE_GOVERNOR_SCHEMA).await;
        match result {
            Ok(()) => transaction.commit().await.map_err(storage_error),
            Err(error) => {
                let _ = transaction.rollback().await;
                Err(storage_error(error))
            }
        }
    }
}

#[cfg(feature = "postgres")]
impl ResourceGovernorStore for PostgresResourceGovernorStore {
    fn update<T, F>(&self, update: F) -> Result<T, ResourceError>
    where
        T: Send + 'static,
        F: FnOnce(&mut ResourceGovernorSnapshot) -> Result<T, ResourceError> + Send + 'static,
    {
        let store = self.clone();
        run_async_on_shared_storage_worker(move || async move {
            let mut client = store.pool.get().await.map_err(storage_error)?;
            let transaction = client.transaction().await.map_err(storage_error)?;
            transaction
                .batch_execute("LOCK TABLE ironclaw_resource_governor_snapshots IN EXCLUSIVE MODE")
                .await
                .map_err(storage_error)?;
            let result = update_postgres_snapshot(&transaction, update).await;
            match result {
                Ok(value) => {
                    transaction.commit().await.map_err(storage_error)?;
                    Ok(value)
                }
                Err(error) => {
                    let _ = transaction.rollback().await;
                    Err(error)
                }
            }
        })
    }
}

#[cfg(feature = "postgres")]
async fn update_postgres_snapshot<T, F>(
    transaction: &tokio_postgres::Transaction<'_>,
    update: F,
) -> Result<T, ResourceError>
where
    F: FnOnce(&mut ResourceGovernorSnapshot) -> Result<T, ResourceError>,
{
    let row = transaction
        .query_opt(
            "SELECT state_json FROM ironclaw_resource_governor_snapshots WHERE snapshot_id = $1",
            &[&SNAPSHOT_ID],
        )
        .await
        .map_err(storage_error)?;
    let mut snapshot = if let Some(row) = row {
        let state_json: String = row.get(0);
        serde_json::from_str(&state_json).map_err(storage_error)?
    } else {
        ResourceGovernorSnapshot::default()
    };

    let value = update(&mut snapshot)?;
    let encoded = serde_json::to_string(&snapshot).map_err(storage_error)?;
    transaction
        .execute(
            "INSERT INTO ironclaw_resource_governor_snapshots (snapshot_id, state_json, updated_at_ms) \
             VALUES ($1, $2, (EXTRACT(EPOCH FROM NOW()) * 1000)::BIGINT) \
             ON CONFLICT(snapshot_id) DO UPDATE SET \
             state_json = excluded.state_json, updated_at_ms = excluded.updated_at_ms",
            &[&SNAPSHOT_ID, &encoded],
        )
        .await
        .map_err(storage_error)?;
    Ok(value)
}

/// Durable resource governor backed by a transactional [`ResourceGovernorStore`].
#[derive(Debug)]
pub struct PersistentResourceGovernor<S>
where
    S: ResourceGovernorStore,
{
    store: S,
}

impl<S> PersistentResourceGovernor<S>
where
    S: ResourceGovernorStore,
{
    pub fn new(store: S) -> Self {
        Self { store }
    }

    pub fn try_set_limit(
        &self,
        account: ResourceAccount,
        limits: ResourceLimits,
    ) -> Result<(), ResourceError> {
        self.store.update(move |snapshot| {
            set_limit_in_state(&mut snapshot.state, account, limits);
            Ok(())
        })
    }

    pub fn reserved_for(&self, account: &ResourceAccount) -> Result<ResourceTally, ResourceError> {
        let account = account.clone();
        self.store.update(move |snapshot| {
            Ok(snapshot
                .state
                .reserved_by_account
                .get(&account)
                .cloned()
                .unwrap_or_default())
        })
    }

    pub fn usage_for(&self, account: &ResourceAccount) -> Result<ResourceTally, ResourceError> {
        let account = account.clone();
        self.store.update(move |snapshot| {
            Ok(snapshot
                .state
                .usage_by_account
                .get(&account)
                .cloned()
                .unwrap_or_default())
        })
    }
}

impl<S> ResourceGovernor for PersistentResourceGovernor<S>
where
    S: ResourceGovernorStore,
{
    fn set_limit(&self, account: ResourceAccount, limits: ResourceLimits) {
        let _ = self.try_set_limit(account, limits);
    }

    fn reserve(
        &self,
        scope: ResourceScope,
        estimate: ResourceEstimate,
    ) -> Result<ResourceReservation, ResourceError> {
        self.reserve_with_id(scope, estimate, ResourceReservationId::new())
    }

    fn reserve_with_id(
        &self,
        scope: ResourceScope,
        estimate: ResourceEstimate,
        reservation_id: ResourceReservationId,
    ) -> Result<ResourceReservation, ResourceError> {
        self.store.update(move |snapshot| {
            reserve_in_state(&mut snapshot.state, scope, estimate, reservation_id)
        })
    }

    fn reconcile(
        &self,
        reservation_id: ResourceReservationId,
        actual: ResourceUsage,
    ) -> Result<ResourceReceipt, ResourceError> {
        self.store
            .update(move |snapshot| reconcile_in_state(&mut snapshot.state, reservation_id, actual))
    }

    fn release(
        &self,
        reservation_id: ResourceReservationId,
    ) -> Result<ResourceReceipt, ResourceError> {
        self.store
            .update(move |snapshot| release_in_state(&mut snapshot.state, reservation_id))
    }
}

/// In-memory governor used by early Reborn contract tests.
#[derive(Debug, Default)]
pub struct InMemoryResourceGovernor {
    state: Mutex<ResourceState>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ResourceState {
    limits: HashMap<ResourceAccount, ResourceLimits>,
    reserved_by_account: HashMap<ResourceAccount, ResourceTally>,
    usage_by_account: HashMap<ResourceAccount, ResourceTally>,
    reservations: HashMap<ResourceReservationId, ReservationRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ReservationRecord {
    reservation: ResourceReservation,
    accounts: Vec<ResourceAccount>,
    tally: ResourceTally,
    status: ReservationStatus,
    actual: Option<ResourceUsage>,
}

#[derive(Serialize, Deserialize)]
struct ResourceStateSerde {
    limits: Vec<(ResourceAccount, ResourceLimits)>,
    reserved_by_account: Vec<(ResourceAccount, ResourceTally)>,
    usage_by_account: Vec<(ResourceAccount, ResourceTally)>,
    reservations: Vec<(ResourceReservationId, ReservationRecord)>,
}

impl Serialize for ResourceState {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        ResourceStateSerde {
            limits: self
                .limits
                .iter()
                .map(|(account, limits)| (account.clone(), limits.clone()))
                .collect(),
            reserved_by_account: self
                .reserved_by_account
                .iter()
                .map(|(account, tally)| (account.clone(), tally.clone()))
                .collect(),
            usage_by_account: self
                .usage_by_account
                .iter()
                .map(|(account, tally)| (account.clone(), tally.clone()))
                .collect(),
            reservations: self
                .reservations
                .iter()
                .map(|(id, record)| (*id, record.clone()))
                .collect(),
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ResourceState {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = ResourceStateSerde::deserialize(deserializer)?;
        Ok(Self {
            limits: value.limits.into_iter().collect(),
            reserved_by_account: value.reserved_by_account.into_iter().collect(),
            usage_by_account: value.usage_by_account.into_iter().collect(),
            reservations: value.reservations.into_iter().collect(),
        })
    }
}

impl InMemoryResourceGovernor {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn reserved_for(&self, account: &ResourceAccount) -> ResourceTally {
        self.lock_state()
            .reserved_by_account
            .get(account)
            .cloned()
            .unwrap_or_default()
    }

    pub fn usage_for(&self, account: &ResourceAccount) -> ResourceTally {
        self.lock_state()
            .usage_by_account
            .get(account)
            .cloned()
            .unwrap_or_default()
    }

    fn lock_state(&self) -> MutexGuard<'_, ResourceState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl ResourceGovernor for InMemoryResourceGovernor {
    fn set_limit(&self, account: ResourceAccount, limits: ResourceLimits) {
        set_limit_in_state(&mut self.lock_state(), account, limits);
    }

    fn reserve(
        &self,
        scope: ResourceScope,
        estimate: ResourceEstimate,
    ) -> Result<ResourceReservation, ResourceError> {
        self.reserve_with_id(scope, estimate, ResourceReservationId::new())
    }

    fn reserve_with_id(
        &self,
        scope: ResourceScope,
        estimate: ResourceEstimate,
        reservation_id: ResourceReservationId,
    ) -> Result<ResourceReservation, ResourceError> {
        reserve_in_state(&mut self.lock_state(), scope, estimate, reservation_id)
    }

    fn reconcile(
        &self,
        reservation_id: ResourceReservationId,
        actual: ResourceUsage,
    ) -> Result<ResourceReceipt, ResourceError> {
        reconcile_in_state(&mut self.lock_state(), reservation_id, actual)
    }

    fn release(
        &self,
        reservation_id: ResourceReservationId,
    ) -> Result<ResourceReceipt, ResourceError> {
        release_in_state(&mut self.lock_state(), reservation_id)
    }
}

fn set_limit_in_state(state: &mut ResourceState, account: ResourceAccount, limits: ResourceLimits) {
    state.limits.insert(account, limits);
}

fn reserve_in_state(
    state: &mut ResourceState,
    scope: ResourceScope,
    estimate: ResourceEstimate,
    reservation_id: ResourceReservationId,
) -> Result<ResourceReservation, ResourceError> {
    validate_estimate(&estimate)?;

    if state.reservations.contains_key(&reservation_id) {
        return Err(ResourceError::ReservationAlreadyExists { id: reservation_id });
    }
    let accounts = ResourceAccount::cascade(&scope);
    let requested = ResourceTally::from_estimate(&estimate);

    for account in &accounts {
        if let Some(limits) = state.limits.get(account) {
            let usage = state
                .usage_by_account
                .get(account)
                .cloned()
                .unwrap_or_default();
            let reserved = state
                .reserved_by_account
                .get(account)
                .cloned()
                .unwrap_or_default();
            if let Some(denial) = check_limits(account, limits, &usage, &reserved, &requested) {
                return Err(ResourceError::LimitExceeded(Box::new(denial)));
            }
        }
    }

    let reservation = ResourceReservation {
        id: reservation_id,
        scope,
        estimate,
    };

    for account in &accounts {
        state
            .reserved_by_account
            .entry(account.clone())
            .or_default()
            .add_assign(&requested);
    }

    state.reservations.insert(
        reservation.id,
        ReservationRecord {
            reservation: reservation.clone(),
            accounts,
            tally: requested,
            status: ReservationStatus::Active,
            actual: None,
        },
    );

    Ok(reservation)
}

fn reconcile_in_state(
    state: &mut ResourceState,
    reservation_id: ResourceReservationId,
    actual: ResourceUsage,
) -> Result<ResourceReceipt, ResourceError> {
    let mut record = state
        .reservations
        .remove(&reservation_id)
        .ok_or(ResourceError::UnknownReservation { id: reservation_id })?;

    if record.status != ReservationStatus::Active {
        let status = record.status;
        state.reservations.insert(reservation_id, record);
        return Err(ResourceError::ReservationClosed {
            id: reservation_id,
            status,
        });
    }

    if let Err(error) = validate_usage(&actual) {
        state.reservations.insert(reservation_id, record);
        return Err(error);
    }

    for account in &record.accounts {
        state
            .reserved_by_account
            .entry(account.clone())
            .or_default()
            .sub_assign(&record.tally);
        state
            .usage_by_account
            .entry(account.clone())
            .or_default()
            .add_assign(&ResourceTally::from_usage(&actual));
    }

    record.status = ReservationStatus::Reconciled;
    record.actual = Some(actual.clone());
    let receipt = ResourceReceipt {
        id: reservation_id,
        scope: record.reservation.scope.clone(),
        status: ReservationStatus::Reconciled,
        estimate: record.reservation.estimate.clone(),
        actual: Some(actual),
    };
    state.reservations.insert(reservation_id, record);
    Ok(receipt)
}

fn release_in_state(
    state: &mut ResourceState,
    reservation_id: ResourceReservationId,
) -> Result<ResourceReceipt, ResourceError> {
    let mut record = state
        .reservations
        .remove(&reservation_id)
        .ok_or(ResourceError::UnknownReservation { id: reservation_id })?;

    if record.status != ReservationStatus::Active {
        let status = record.status;
        state.reservations.insert(reservation_id, record);
        return Err(ResourceError::ReservationClosed {
            id: reservation_id,
            status,
        });
    }

    for account in &record.accounts {
        state
            .reserved_by_account
            .entry(account.clone())
            .or_default()
            .sub_assign(&record.tally);
    }

    record.status = ReservationStatus::Released;
    let receipt = ResourceReceipt {
        id: reservation_id,
        scope: record.reservation.scope.clone(),
        status: ReservationStatus::Released,
        estimate: record.reservation.estimate.clone(),
        actual: None,
    };
    state.reservations.insert(reservation_id, record);
    Ok(receipt)
}

fn validate_estimate(estimate: &ResourceEstimate) -> Result<(), ResourceError> {
    if let Some(usd) = estimate.usd
        && usd < Decimal::ZERO
    {
        return Err(ResourceError::InvalidEstimate {
            dimension: ResourceDimension::Usd,
            reason: "must be non-negative",
        });
    }

    Ok(())
}

fn validate_usage(usage: &ResourceUsage) -> Result<(), ResourceError> {
    if usage.usd < Decimal::ZERO {
        return Err(ResourceError::InvalidEstimate {
            dimension: ResourceDimension::Usd,
            reason: "must be non-negative",
        });
    }

    Ok(())
}

/// Returns the first denied dimension in canonical resource order.
///
/// This intentionally reports one denial rather than aggregating all failed
/// dimensions so callers have a deterministic, compact failure reason.
fn check_limits(
    account: &ResourceAccount,
    limits: &ResourceLimits,
    usage: &ResourceTally,
    reserved: &ResourceTally,
    requested: &ResourceTally,
) -> Option<ResourceDenial> {
    check_decimal(
        account,
        ResourceDimension::Usd,
        limits.max_usd,
        usage.usd,
        reserved.usd,
        requested.usd,
    )
    .or_else(|| {
        check_integer(
            account,
            ResourceDimension::InputTokens,
            limits.max_input_tokens,
            usage.input_tokens,
            reserved.input_tokens,
            requested.input_tokens,
        )
    })
    .or_else(|| {
        check_integer(
            account,
            ResourceDimension::OutputTokens,
            limits.max_output_tokens,
            usage.output_tokens,
            reserved.output_tokens,
            requested.output_tokens,
        )
    })
    .or_else(|| {
        check_integer(
            account,
            ResourceDimension::WallClockMs,
            limits.max_wall_clock_ms,
            usage.wall_clock_ms,
            reserved.wall_clock_ms,
            requested.wall_clock_ms,
        )
    })
    .or_else(|| {
        check_integer(
            account,
            ResourceDimension::OutputBytes,
            limits.max_output_bytes,
            usage.output_bytes,
            reserved.output_bytes,
            requested.output_bytes,
        )
    })
    .or_else(|| {
        check_integer(
            account,
            ResourceDimension::NetworkEgressBytes,
            limits.max_network_egress_bytes,
            usage.network_egress_bytes,
            reserved.network_egress_bytes,
            requested.network_egress_bytes,
        )
    })
    .or_else(|| {
        check_integer(
            account,
            ResourceDimension::ProcessCount,
            limits.max_process_count.map(u64::from),
            u64::from(usage.process_count),
            u64::from(reserved.process_count),
            u64::from(requested.process_count),
        )
    })
    .or_else(|| {
        check_integer(
            account,
            ResourceDimension::ConcurrencySlots,
            limits.max_concurrency_slots.map(u64::from),
            u64::from(usage.concurrency_slots),
            u64::from(reserved.concurrency_slots),
            u64::from(requested.concurrency_slots),
        )
    })
}

fn check_decimal(
    account: &ResourceAccount,
    dimension: ResourceDimension,
    limit: Option<Decimal>,
    usage: Decimal,
    reserved: Decimal,
    requested: Decimal,
) -> Option<ResourceDenial> {
    let limit = limit?;
    let exceeds = match usage
        .checked_add(reserved)
        .and_then(|subtotal| subtotal.checked_add(requested))
    {
        Some(total) => total > limit,
        None => true,
    };
    if exceeds {
        Some(ResourceDenial {
            account: account.clone(),
            dimension,
            limit: ResourceValue::Decimal(limit),
            current_usage: ResourceValue::Decimal(usage),
            active_reserved: ResourceValue::Decimal(reserved),
            requested: ResourceValue::Decimal(requested),
        })
    } else {
        None
    }
}

fn check_integer(
    account: &ResourceAccount,
    dimension: ResourceDimension,
    limit: Option<u64>,
    usage: u64,
    reserved: u64,
    requested: u64,
) -> Option<ResourceDenial> {
    let limit = limit?;
    if usage.saturating_add(reserved).saturating_add(requested) > limit {
        Some(ResourceDenial {
            account: account.clone(),
            dimension,
            limit: ResourceValue::Integer(limit),
            current_usage: ResourceValue::Integer(usage),
            active_reserved: ResourceValue::Integer(reserved),
            requested: ResourceValue::Integer(requested),
        })
    } else {
        None
    }
}
