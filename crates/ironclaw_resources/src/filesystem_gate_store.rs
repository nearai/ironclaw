//! Filesystem-backed [`BudgetGateStore`] under the `/resources` mount alias.
//!
//! Mirrors [`FilesystemResourceGovernorStore`](crate::FilesystemResourceGovernorStore):
//! one JSON snapshot at `/resources/budget-gates.json`, tenant/user identity
//! supplied by the caller's
//! [`MountView`](ironclaw_host_api::MountView), CAS-protected put with an
//! in-process per-path lock so concurrent writers serialize. Cross-process
//! contention surfaces as [`BudgetGateError::Storage`].
//!
//! Without persistent gate storage, every process restart loses every pending
//! budget approval gate — users have to re-request approval. That's the
//! follow-up from #3841 this module addresses.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, OnceLock, Weak, mpsc};

use chrono::{DateTime, Utc};
use ironclaw_filesystem::{
    CasExpectation, ContentType, Entry, FilesystemError, FilesystemOperation, RootFilesystem,
    ScopedFilesystem,
};
use ironclaw_host_api::{HostApiError, ResourceScope, ScopedPath};
use serde::{Deserialize, Serialize};

use crate::gate::{
    BudgetApprovalGate, BudgetGateError, BudgetGateId, BudgetGateOutcome, BudgetGateStatus,
    BudgetGateStore,
};

const RESOURCES_PREFIX: &str = "/resources";
const GATES_FILE_NAME: &str = "budget-gates.json";

/// Filesystem-backed budget gate store.
///
/// Construct with any
/// [`ScopedFilesystem`](ironclaw_filesystem::ScopedFilesystem). All gates
/// persist into one tenant/user-scoped snapshot file; per-tenant separation
/// is structural via the caller's
/// [`MountView`](ironclaw_host_api::MountView).
#[derive(Clone)]
pub struct FilesystemBudgetGateStore<F>
where
    F: RootFilesystem,
{
    filesystem: Arc<ScopedFilesystem<F>>,
    worker: AsyncStorageWorkerCell,
}

impl<F> std::fmt::Debug for FilesystemBudgetGateStore<F>
where
    F: RootFilesystem,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FilesystemBudgetGateStore").finish()
    }
}

impl<F> FilesystemBudgetGateStore<F>
where
    F: RootFilesystem + 'static,
{
    pub fn new(filesystem: Arc<ScopedFilesystem<F>>) -> Self {
        Self {
            filesystem,
            worker: new_storage_worker_cell(),
        }
    }

    fn with_snapshot<T, U>(&self, update: U) -> Result<T, BudgetGateError>
    where
        T: Send + 'static,
        U: FnOnce(&mut BudgetGateSnapshot) -> Result<T, BudgetGateError> + Send + 'static,
    {
        let filesystem = Arc::clone(&self.filesystem);
        let worker_cell = self.worker.clone();
        run_async_on_storage_worker(&worker_cell, move || async move {
            let path = snapshot_path()?;
            let record_lock = filesystem_record_lock(&path);
            let _guard = record_lock.lock().await;
            // System scope: budget approval gates are process-global like
            // the governor snapshot — tenant separation is provided by the
            // ScopedFilesystem MountView.
            let scope = ResourceScope::system();
            update_filesystem_snapshot(filesystem.as_ref(), &scope, &path, update).await
        })
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BudgetGateSnapshot {
    /// Schema version. Bump when the on-disk shape changes; today there is
    /// only v1.
    schema_version: u32,
    /// All gates, keyed by id. Terminal-state gates persist so audit /
    /// `get(id)` lookups can still hydrate them after a restart.
    gates: HashMap<BudgetGateId, BudgetApprovalGate>,
}

impl BudgetGateSnapshot {
    const CURRENT_SCHEMA: u32 = 1;

    fn ensure_current(&mut self) -> Result<(), BudgetGateError> {
        if self.schema_version == 0 {
            // Default value (never persisted) — coerce to current schema.
            self.schema_version = Self::CURRENT_SCHEMA;
            return Ok(());
        }
        if self.schema_version != Self::CURRENT_SCHEMA {
            return Err(BudgetGateError::Storage {
                reason: format!(
                    "budget gate snapshot schema {} is not supported (expected {})",
                    self.schema_version,
                    Self::CURRENT_SCHEMA
                ),
            });
        }
        Ok(())
    }
}

impl<F> BudgetGateStore for FilesystemBudgetGateStore<F>
where
    F: RootFilesystem + 'static,
{
    fn open(&self, gate: BudgetApprovalGate) -> Result<(), BudgetGateError> {
        self.with_snapshot(move |snapshot| {
            snapshot.ensure_current()?;
            snapshot.gates.insert(gate.id, gate);
            Ok(())
        })
    }

    fn resolve(
        &self,
        id: BudgetGateId,
        outcome: BudgetGateOutcome,
        at: DateTime<Utc>,
    ) -> Result<BudgetApprovalGate, BudgetGateError> {
        self.with_snapshot(move |snapshot| {
            snapshot.ensure_current()?;
            let gate = snapshot
                .gates
                .get_mut(&id)
                .ok_or(BudgetGateError::Unknown { id })?;
            if gate.status.is_terminal() {
                return Err(BudgetGateError::AlreadyResolved { id });
            }
            gate.status = match outcome {
                BudgetGateOutcome::Approve {
                    increased_limit,
                    by,
                } => BudgetGateStatus::Approved {
                    increased_limit,
                    by,
                    at,
                },
                BudgetGateOutcome::Cancel { by } => BudgetGateStatus::Cancelled { by, at },
            };
            Ok(gate.clone())
        })
    }

    fn expire_pending_older_than(
        &self,
        cutoff: DateTime<Utc>,
    ) -> Result<Vec<BudgetApprovalGate>, BudgetGateError> {
        self.with_snapshot(move |snapshot| {
            snapshot.ensure_current()?;
            let mut expired = Vec::new();
            for gate in snapshot.gates.values_mut() {
                if matches!(gate.status, BudgetGateStatus::Pending) && gate.expires_at <= cutoff {
                    gate.status = BudgetGateStatus::Expired { at: cutoff };
                    expired.push(gate.clone());
                }
            }
            Ok(expired)
        })
    }

    fn get(&self, id: BudgetGateId) -> Result<Option<BudgetApprovalGate>, BudgetGateError> {
        self.with_snapshot(move |snapshot| {
            snapshot.ensure_current()?;
            Ok(snapshot.gates.get(&id).cloned())
        })
    }

    fn list_pending(&self) -> Result<Vec<BudgetApprovalGate>, BudgetGateError> {
        self.with_snapshot(move |snapshot| {
            snapshot.ensure_current()?;
            Ok(snapshot
                .gates
                .values()
                .filter(|gate| matches!(gate.status, BudgetGateStatus::Pending))
                .cloned()
                .collect())
        })
    }
}

fn snapshot_path() -> Result<ScopedPath, BudgetGateError> {
    ScopedPath::new(format!("{RESOURCES_PREFIX}/{GATES_FILE_NAME}")).map_err(invalid_path)
}

async fn update_filesystem_snapshot<F, T, U>(
    filesystem: &ScopedFilesystem<F>,
    scope: &ResourceScope,
    path: &ScopedPath,
    update: U,
) -> Result<T, BudgetGateError>
where
    F: RootFilesystem,
    U: FnOnce(&mut BudgetGateSnapshot) -> Result<T, BudgetGateError>,
{
    let (mut snapshot, expectation) = match filesystem.get(scope, path).await {
        Ok(Some(versioned)) => {
            let decoded: BudgetGateSnapshot = serde_json::from_slice(&versioned.entry.body)
                .map_err(|error| BudgetGateError::Storage {
                    reason: format!("decode budget gate snapshot: {error}"),
                })?;
            (decoded, CasExpectation::Version(versioned.version))
        }
        Ok(None) => (
            BudgetGateSnapshot {
                schema_version: BudgetGateSnapshot::CURRENT_SCHEMA,
                gates: HashMap::new(),
            },
            CasExpectation::Absent,
        ),
        Err(error) => return Err(storage_error(error)),
    };
    let value = update(&mut snapshot)?;
    snapshot.schema_version = BudgetGateSnapshot::CURRENT_SCHEMA;
    let encoded = serde_json::to_vec_pretty(&snapshot).map_err(storage_error)?;
    let entry = Entry::bytes(encoded).with_content_type(ContentType::json());
    match put_with_cas(filesystem, scope, path, entry, expectation).await {
        Ok(()) => Ok(value),
        Err(PutError::VersionMismatch) => Err(BudgetGateError::Storage {
            reason: format!(
                "cross-process CAS contention on budget gate snapshot {}",
                path.as_str()
            ),
        }),
        Err(PutError::Other(err)) => Err(err),
    }
}

enum PutError {
    VersionMismatch,
    Other(BudgetGateError),
}

async fn put_with_cas<F>(
    filesystem: &ScopedFilesystem<F>,
    scope: &ResourceScope,
    path: &ScopedPath,
    entry: Entry,
    cas: CasExpectation,
) -> Result<(), PutError>
where
    F: RootFilesystem,
{
    let fallback_entry = entry.clone();
    let cas_for_fallback = cas;
    match filesystem.put(scope, path, entry, cas).await {
        Ok(_) => Ok(()),
        Err(FilesystemError::VersionMismatch { .. }) => Err(PutError::VersionMismatch),
        Err(FilesystemError::Unsupported {
            operation: FilesystemOperation::WriteFile,
            ..
        }) => {
            if matches!(cas_for_fallback, CasExpectation::Absent) {
                let existing = filesystem
                    .get(scope, path)
                    .await
                    .map_err(|err| PutError::Other(storage_error(err)))?;
                if existing.is_some() {
                    return Err(PutError::VersionMismatch);
                }
            }
            filesystem
                .put(scope, path, fallback_entry, CasExpectation::Any)
                .await
                .map(|_| ())
                .map_err(|err| PutError::Other(storage_error(err)))
        }
        Err(err) => Err(PutError::Other(storage_error(err))),
    }
}

fn storage_error<E: std::fmt::Display>(err: E) -> BudgetGateError {
    BudgetGateError::Storage {
        reason: err.to_string(),
    }
}

fn invalid_path(error: HostApiError) -> BudgetGateError {
    BudgetGateError::Storage {
        reason: format!("invalid budget gate snapshot path: {error}"),
    }
}

// ---------------------------------------------------------------------------
// Async-runtime adapter — mirrors filesystem_store.rs. Sync trait, async
// filesystem, current-thread tokio runtime in a dedicated worker thread.
// Kept parallel rather than shared because the two stores have different
// error types; we will refactor once a third filesystem-backed store needs
// it.
// ---------------------------------------------------------------------------

type AsyncStorageJob = Box<dyn FnOnce(&tokio::runtime::Runtime) + Send + 'static>;

#[derive(Debug, Clone)]
struct AsyncStorageWorker {
    sender: mpsc::Sender<AsyncStorageJob>,
}

impl AsyncStorageWorker {
    fn spawn(name: &'static str) -> Result<Self, BudgetGateError> {
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
            .map_err(|_| storage_error("budget gate storage worker failed to start"))??;
        Ok(Self { sender })
    }

    fn run<T, Fut, F>(&self, build: F) -> Result<T, BudgetGateError>
    where
        T: Send + 'static,
        Fut: Future<Output = Result<T, BudgetGateError>> + Send + 'static,
        F: FnOnce() -> Fut + Send + 'static,
    {
        let (result_sender, result_receiver) = mpsc::channel();
        self.sender
            .send(Box::new(move |runtime| {
                let result = runtime.block_on(build());
                let _ = result_sender.send(result);
            }))
            .map_err(|_| storage_error("budget gate storage worker stopped"))?;
        result_receiver
            .recv()
            .map_err(|_| storage_error("budget gate storage worker stopped"))?
    }
}

type AsyncStorageWorkerCell = Arc<OnceLock<Result<AsyncStorageWorker, String>>>;

fn new_storage_worker_cell() -> AsyncStorageWorkerCell {
    Arc::new(OnceLock::new())
}

fn run_async_on_storage_worker<T, Fut, F>(
    worker_cell: &AsyncStorageWorkerCell,
    build: F,
) -> Result<T, BudgetGateError>
where
    T: Send + 'static,
    Fut: Future<Output = Result<T, BudgetGateError>> + Send + 'static,
    F: FnOnce() -> Fut + Send + 'static,
{
    let worker = worker_cell.get_or_init(|| {
        AsyncStorageWorker::spawn("budget-gate-filesystem").map_err(|error| error.to_string())
    });
    match worker {
        Ok(worker) => worker.run(build),
        Err(error) => Err(storage_error(error)),
    }
}

type FilesystemRecordLock = Arc<tokio::sync::Mutex<()>>;

static FILESYSTEM_RECORD_LOCKS: OnceLock<
    std::sync::Mutex<HashMap<String, Weak<tokio::sync::Mutex<()>>>>,
> = OnceLock::new();

fn filesystem_record_lock(path: &ScopedPath) -> FilesystemRecordLock {
    let locks = FILESYSTEM_RECORD_LOCKS.get_or_init(|| std::sync::Mutex::new(HashMap::new()));
    let mut guard = locks
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    guard.retain(|_, weak| weak.strong_count() > 0);

    let key = path.as_str();
    if let Some(existing) = guard.get(key).and_then(Weak::upgrade) {
        return existing;
    }

    let fresh: FilesystemRecordLock = Arc::new(tokio::sync::Mutex::new(()));
    guard.insert(key.to_string(), Arc::downgrade(&fresh));
    fresh
}

#[cfg(test)]
mod tests {
    use ironclaw_filesystem::InMemoryBackend;
    use ironclaw_host_api::{MountAlias, MountGrant, MountPermissions, MountView, VirtualPath};
    use rust_decimal::Decimal;

    use super::*;
    use crate::gate::{BudgetApprovalGate, BudgetGateId, BudgetGateStatus};
    use crate::{ResourceAccount, ResourceApprovalNeeded, ResourceDimension, ResourceValue};
    use ironclaw_host_api::{TenantId, UserId};

    fn scoped_resources_fs(
        backend: Arc<InMemoryBackend>,
        tenant: &str,
        user: &str,
    ) -> Arc<ScopedFilesystem<InMemoryBackend>> {
        let tenant_user_prefix = format!("/tenants/{tenant}/users/{user}");
        let mounts = MountView::new(vec![MountGrant::new(
            MountAlias::new("/resources").expect("alias"),
            VirtualPath::new(format!("{tenant_user_prefix}/resources")).expect("target"),
            MountPermissions::read_write_list_delete(),
        )])
        .expect("mount view");
        Arc::new(ScopedFilesystem::with_fixed_view(backend, mounts))
    }

    fn sample_needed() -> ResourceApprovalNeeded {
        ResourceApprovalNeeded {
            account: ResourceAccount::tenant(TenantId::new("t").unwrap()),
            dimension: ResourceDimension::Usd,
            limit: ResourceValue::Decimal(Decimal::from(10)),
            current_usage: ResourceValue::Decimal(Decimal::from(0)),
            active_reserved: ResourceValue::Decimal(Decimal::from(0)),
            requested: ResourceValue::Decimal(Decimal::from(9)),
            utilization: 0.91,
            period_end: None,
        }
    }

    fn sample_gate() -> BudgetApprovalGate {
        BudgetApprovalGate {
            id: BudgetGateId::new(),
            needed: sample_needed(),
            opened_at: Utc::now(),
            expires_at: Utc::now() + chrono::Duration::hours(24),
            status: BudgetGateStatus::Pending,
        }
    }

    #[test]
    fn open_and_get_round_trips_through_filesystem() {
        let backend = Arc::new(InMemoryBackend::new());
        let scoped = scoped_resources_fs(Arc::clone(&backend), "tenant-fs", "alice");
        let store = FilesystemBudgetGateStore::new(scoped);
        let gate = sample_gate();
        let id = gate.id;
        store.open(gate.clone()).unwrap();
        let reloaded = store.get(id).unwrap().unwrap();
        assert_eq!(reloaded.id, id);
        assert!(matches!(reloaded.status, BudgetGateStatus::Pending));
    }

    #[test]
    fn pending_gate_survives_restart_via_fresh_handle() {
        // Regression for #3841 follow-up: pending gates must NOT be lost
        // on process restart. A fresh `FilesystemBudgetGateStore` over the
        // same backend filesystem must rehydrate the prior snapshot.
        let backend = Arc::new(InMemoryBackend::new());
        let gate = sample_gate();
        let id = gate.id;
        {
            let scoped = scoped_resources_fs(Arc::clone(&backend), "tenant-fs", "alice");
            let store = FilesystemBudgetGateStore::new(scoped);
            store.open(gate).unwrap();
        }
        // Restart: fresh store, same backing filesystem.
        let scoped = scoped_resources_fs(Arc::clone(&backend), "tenant-fs", "alice");
        let store = FilesystemBudgetGateStore::new(scoped);
        let reloaded = store.get(id).unwrap().unwrap();
        assert_eq!(reloaded.id, id);
        assert!(matches!(reloaded.status, BudgetGateStatus::Pending));
        let pending = store.list_pending().unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, id);
    }

    #[test]
    fn resolve_updates_gate_status_after_reload() {
        let backend = Arc::new(InMemoryBackend::new());
        let scoped = scoped_resources_fs(Arc::clone(&backend), "tenant-fs", "alice");
        let store = FilesystemBudgetGateStore::new(scoped);
        let gate = sample_gate();
        let id = gate.id;
        store.open(gate).unwrap();
        let resolved = store
            .resolve(
                id,
                BudgetGateOutcome::Cancel {
                    by: UserId::new("alice").unwrap(),
                },
                Utc::now(),
            )
            .unwrap();
        assert!(matches!(
            resolved.status,
            BudgetGateStatus::Cancelled { .. }
        ));

        // Fresh handle still sees the terminal status.
        let scoped2 = scoped_resources_fs(Arc::clone(&backend), "tenant-fs", "alice");
        let store2 = FilesystemBudgetGateStore::new(scoped2);
        let reloaded = store2.get(id).unwrap().unwrap();
        assert!(matches!(
            reloaded.status,
            BudgetGateStatus::Cancelled { .. }
        ));
        assert!(store2.list_pending().unwrap().is_empty());
    }

    #[test]
    fn expire_pending_older_than_persists_terminal_state() {
        let backend = Arc::new(InMemoryBackend::new());
        let scoped = scoped_resources_fs(Arc::clone(&backend), "tenant-fs", "alice");
        let store = FilesystemBudgetGateStore::new(scoped);
        let mut gate = sample_gate();
        gate.expires_at = Utc::now() - chrono::Duration::hours(1);
        let id = gate.id;
        store.open(gate).unwrap();
        let expired = store.expire_pending_older_than(Utc::now()).unwrap();
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].id, id);

        // Fresh handle: the expiry persisted.
        let scoped2 = scoped_resources_fs(Arc::clone(&backend), "tenant-fs", "alice");
        let store2 = FilesystemBudgetGateStore::new(scoped2);
        let reloaded = store2.get(id).unwrap().unwrap();
        assert!(matches!(reloaded.status, BudgetGateStatus::Expired { .. }));
    }
}
