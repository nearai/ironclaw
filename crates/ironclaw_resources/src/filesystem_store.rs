//! Filesystem-backed [`ResourceGovernorStore`] under the `/resources` mount alias.
//!
//! Mirrors the migration shape used by the other consumer stores (see
//! `docs/plans/2026-05-16-scoped-filesystem-tenant-isolation.md`):
//! tenant/user identity is carried in the
//! [`MountView`](ironclaw_host_api::MountView) supplied to
//! [`ScopedFilesystem`](ironclaw_filesystem::ScopedFilesystem) at
//! construction time, so this crate never needs to encode tenant/user in
//! the path. The single snapshot file lives at `/resources/snapshot.json`
//! under the alias; per-tenant separation comes from the `MountView`
//! rewriting the alias to the tenant/user-scoped target.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, OnceLock, Weak, mpsc};

use ironclaw_filesystem::{
    CasExpectation, ContentType, Entry, FilesystemError, FilesystemOperation, RootFilesystem,
    ScopedFilesystem,
};
use ironclaw_host_api::{HostApiError, ResourceScope, ScopedPath};

use crate::{
    ResourceError, ResourceGovernorSnapshot, ResourceGovernorStore, snapshot_decode_error,
    storage_error,
};

/// Alias-relative path of the single resource-governor snapshot. Tenant
/// and user identity live in the caller-supplied
/// [`MountView`](ironclaw_host_api::MountView), so this path itself is
/// alias-relative.
const RESOURCES_PREFIX: &str = "/resources";
const SNAPSHOT_FILE_NAME: &str = "snapshot.json";

/// Filesystem-backed resource-governor snapshot store under the `/resources`
/// mount alias.
///
/// Construct with a [`ScopedFilesystem`] over any [`RootFilesystem`]. The
/// [`ScopedFilesystem`] resolves the `/resources` alias to a
/// tenant/user-scoped
/// [`VirtualPath`](ironclaw_host_api::VirtualPath) per its
/// [`MountView`](ironclaw_host_api::MountView) and enforces per-op ACL
/// before any backend dispatch — so tenant isolation is structural rather
/// than something this crate has to re-derive from `ResourceScope`.
///
/// The whole governor state (limits, reservations, usage by account) is
/// serialized as one snapshot at `/resources/snapshot.json`.
/// [`ResourceGovernorStore::update`] runs the caller's closure inside an
/// in-process per-path async lock so concurrent reservation/reconcile/
/// release transitions stay atomic against same-process writers; CAS-Version
/// preconditions surface cross-process races as
/// [`ResourceError::Storage`] errors. Byte-only backends
/// (`LocalFilesystem`) that don't support `CasExpectation::Version` fall
/// back to `CasExpectation::Any` under the same in-process lock — same
/// shape as the run-state and processes migrations.
#[derive(Clone)]
pub struct FilesystemResourceGovernorStore<F>
where
    F: RootFilesystem,
{
    filesystem: Arc<ScopedFilesystem<F>>,
    worker: AsyncStorageWorkerCell,
}

impl<F> FilesystemResourceGovernorStore<F>
where
    F: RootFilesystem + 'static,
{
    pub fn new(filesystem: Arc<ScopedFilesystem<F>>) -> Self {
        Self {
            filesystem,
            worker: new_storage_worker_cell(),
        }
    }
}

impl<F> ResourceGovernorStore for FilesystemResourceGovernorStore<F>
where
    F: RootFilesystem + 'static,
{
    fn update<T, U>(&self, update: U) -> Result<T, ResourceError>
    where
        T: Send + 'static,
        U: FnOnce(&mut ResourceGovernorSnapshot) -> Result<T, ResourceError> + Send + 'static,
    {
        let filesystem = Arc::clone(&self.filesystem);
        let worker_cell = self.worker.clone();
        run_async_on_storage_worker(&worker_cell, move || async move {
            let path = snapshot_path()?;
            let record_lock = filesystem_record_lock(&path);
            let _guard = record_lock.lock().await;
            // Resource quotas are process-global (operator-set caps applied
            // across all tenants). The snapshot record therefore lives under
            // the system scope rather than any tenant scope — tenant-scoped
            // resource accounting is a future capability that would change
            // the `ResourceGovernorStore` trait surface.
            let scope = ResourceScope::system();
            update_filesystem_snapshot(filesystem.as_ref(), &scope, &path, update).await
        })
    }
}

/// Snapshot path under the `/resources` mount alias. Tenant + user
/// identity live in the caller-supplied `MountView`, so the path itself
/// is alias-relative.
fn snapshot_path() -> Result<ScopedPath, ResourceError> {
    ScopedPath::new(format!("{RESOURCES_PREFIX}/{SNAPSHOT_FILE_NAME}")).map_err(invalid_path)
}

/// Read the current snapshot, apply the caller's `update`, and CAS the
/// resulting snapshot back atomically.
///
/// Concurrency: callers hold the per-path async record lock for the whole
/// read-modify-write window, so two in-process [`ResourceGovernorStore::update`]
/// calls against the same snapshot path serialize on the lock — the
/// `CasExpectation::Version` precondition only fires when a cross-process
/// writer races us between read and write. The closure is `FnOnce` per the
/// trait, so cross-process CAS contention surfaces as a
/// [`ResourceError::Storage`] error rather than being retried (the caller
/// must reissue the request); the libSQL/Postgres siblings this replaces
/// relied on `BEGIN IMMEDIATE` / `LOCK TABLE` for the same guarantee.
async fn update_filesystem_snapshot<F, T, U>(
    filesystem: &ScopedFilesystem<F>,
    scope: &ResourceScope,
    path: &ScopedPath,
    update: U,
) -> Result<T, ResourceError>
where
    F: RootFilesystem,
    U: FnOnce(&mut ResourceGovernorSnapshot) -> Result<T, ResourceError>,
{
    let (mut snapshot, expectation) = match filesystem.get(scope, path).await {
        Ok(Some(versioned)) => {
            let snapshot: ResourceGovernorSnapshot =
                serde_json::from_slice(&versioned.entry.body).map_err(snapshot_decode_error)?;
            (snapshot, CasExpectation::Version(versioned.version))
        }
        Ok(None) => (ResourceGovernorSnapshot::default(), CasExpectation::Absent),
        Err(error) => return Err(storage_error(error)),
    };
    let value = update(&mut snapshot)?;
    let encoded = serde_json::to_vec_pretty(&snapshot).map_err(storage_error)?;
    let entry = Entry::bytes(encoded).with_content_type(ContentType::json());
    match put_with_cas(filesystem, scope, path, entry, expectation).await {
        Ok(()) => Ok(value),
        Err(PutError::VersionMismatch) => Err(ResourceError::Storage {
            reason: format!(
                "cross-process CAS contention on resource governor snapshot {}",
                path.as_str()
            ),
        }),
        Err(PutError::Other(error)) => Err(error),
    }
}

/// Local error classification for the CAS-aware put helper. Mirrors the
/// shape used by the run-state and processes migrations.
enum PutError {
    VersionMismatch,
    Other(ResourceError),
}

/// Issue a `put` honoring the requested CAS expectation.
///
/// Falls back to `CasExpectation::Any` when the backend reports
/// `Unsupported` for the request — `LocalFilesystem` is byte-only and only
/// accepts `Any`. On the byte-only fallback path, `CasExpectation::Absent`
/// is emulated via a `get` precheck so callers still see
/// `PutError::VersionMismatch` when the record already exists. The
/// check-then-write race is closed by the in-process lock map; cross-
/// process callers on byte-only backends fall back to the documented
/// process-local limitation (`crates/ironclaw_resources/CLAUDE.md`).
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
                    .map_err(|error| PutError::Other(storage_error(error)))?;
                if existing.is_some() {
                    return Err(PutError::VersionMismatch);
                }
            }
            filesystem
                .put(scope, path, fallback_entry, CasExpectation::Any)
                .await
                .map(|_| ())
                .map_err(|error| PutError::Other(storage_error(error)))
        }
        Err(error) => Err(PutError::Other(storage_error(error))),
    }
}

fn invalid_path(error: HostApiError) -> ResourceError {
    ResourceError::Storage {
        reason: format!("invalid resource governor snapshot path: {error}"),
    }
}

// ---------------------------------------------------------------------------
// Async-runtime adapter: `ResourceGovernorStore::update` is synchronous (the
// trait contract is shared with the in-memory store) but `ScopedFilesystem`
// is async, so callers cross over via a dedicated current-thread tokio
// runtime thread. Same shape as the deleted libSQL/Postgres stores; the
// worker is lazily spawned the first time the store is used.
// ---------------------------------------------------------------------------

type AsyncStorageJob = Box<dyn FnOnce(&tokio::runtime::Runtime) + Send + 'static>;

#[derive(Debug, Clone)]
struct AsyncStorageWorker {
    sender: mpsc::Sender<AsyncStorageJob>,
}

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

type AsyncStorageWorkerCell = Arc<OnceLock<Result<AsyncStorageWorker, String>>>;

fn new_storage_worker_cell() -> AsyncStorageWorkerCell {
    Arc::new(OnceLock::new())
}

fn run_async_on_storage_worker<T, Fut, F>(
    worker_cell: &AsyncStorageWorkerCell,
    build: F,
) -> Result<T, ResourceError>
where
    T: Send + 'static,
    Fut: Future<Output = Result<T, ResourceError>> + Send + 'static,
    F: FnOnce() -> Fut + Send + 'static,
{
    let worker = worker_cell.get_or_init(|| {
        AsyncStorageWorker::spawn("resource-governor-filesystem").map_err(|error| error.to_string())
    });
    match worker {
        Ok(worker) => worker.run(build),
        Err(error) => Err(storage_error(error)),
    }
}

// ---------------------------------------------------------------------------
// Per-path async serialization for the filesystem-backed resource-governor
// store. The shape mirrors the run-state / processes / outbound /
// authorization migrations: an `OnceLock`-initialized map of weak Mutex
// handles, lazily pruned on each acquisition so the map size stays bounded
// under tenant churn (audit finding F1 in the original ScopedFilesystem
// design).
// ---------------------------------------------------------------------------

type FilesystemRecordLock = Arc<tokio::sync::Mutex<()>>;

static FILESYSTEM_RECORD_LOCKS: OnceLock<
    std::sync::Mutex<HashMap<String, Weak<tokio::sync::Mutex<()>>>>,
> = OnceLock::new();

fn filesystem_record_lock(path: &ScopedPath) -> FilesystemRecordLock {
    let locks = FILESYSTEM_RECORD_LOCKS.get_or_init(|| std::sync::Mutex::new(HashMap::new()));
    let mut guard = locks
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    // Drop entries whose owning Arc has been released.
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
    use std::sync::Arc;

    use ironclaw_filesystem::InMemoryBackend;
    use ironclaw_host_api::{
        InvocationId, MountAlias, MountGrant, MountPermissions, MountView, ProjectId,
        ResourceEstimate, ResourceScope, TenantId, UserId, VirtualPath,
    };
    use rust_decimal_macros::dec;

    use super::*;
    use crate::{PersistentResourceGovernor, ResourceAccount, ResourceGovernor, ResourceLimits};

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

    fn sample_scope(tenant: &str, user: &str, project: Option<&str>) -> ResourceScope {
        ResourceScope {
            tenant_id: TenantId::new(tenant).unwrap(),
            user_id: UserId::new(user).unwrap(),
            agent_id: None,
            project_id: project.map(|value| ProjectId::new(value).unwrap()),
            mission_id: None,
            thread_id: None,
            invocation_id: InvocationId::new(),
        }
    }

    #[test]
    fn snapshot_persists_and_reloads_through_scoped_filesystem() {
        let backend = Arc::new(InMemoryBackend::new());
        let scoped = scoped_resources_fs(Arc::clone(&backend), "tenant-a", "alice");

        let store = FilesystemResourceGovernorStore::new(Arc::clone(&scoped));
        let scope = sample_scope("tenant-a", "alice", Some("p1"));
        let account = ResourceAccount::tenant(scope.tenant_id.clone());

        let governor = PersistentResourceGovernor::new(store);
        governor
            .try_set_limit(
                account.clone(),
                ResourceLimits {
                    max_usd: Some(dec!(1.00)),
                    max_concurrency_slots: Some(1),
                    ..ResourceLimits::default()
                },
            )
            .unwrap();
        let reservation = governor
            .reserve(
                scope.clone(),
                ResourceEstimate {
                    concurrency_slots: Some(1),
                    ..ResourceEstimate::default()
                },
            )
            .unwrap();

        // Reload from the same on-disk snapshot.
        let reloaded = PersistentResourceGovernor::new(FilesystemResourceGovernorStore::new(
            Arc::clone(&scoped),
        ));
        assert_eq!(
            reloaded.reserved_for(&account).unwrap().concurrency_slots,
            1
        );

        // Concurrency-slot budget is exhausted; a second reservation must be
        // denied even though it goes through a fresh store handle.
        let denied = reloaded
            .reserve(
                scope,
                ResourceEstimate {
                    concurrency_slots: Some(1),
                    ..ResourceEstimate::default()
                },
            )
            .unwrap_err();
        assert!(matches!(denied, ResourceError::LimitExceeded { .. }));

        reloaded.release(reservation.id).unwrap();
    }

    /// Cross-tenant isolation regression — two `ScopedFilesystem`s over the
    /// same `RootFilesystem` with disjoint `MountView` targets must produce
    /// fully disjoint snapshots. Writing on tenant A must not be visible
    /// from tenant B, even when both scopes carry the same `user_id` and
    /// `project_id`.
    ///
    /// Fails closed if the `ScopedFilesystem` wrapping ever regresses to a
    /// raw `&F: RootFilesystem` — mirrors the regression test landed for
    /// run-state, processes, secrets, outbound, and authorization.
    #[test]
    fn isolates_two_tenants_with_same_user_project_ids() {
        let backend = Arc::new(InMemoryBackend::new());
        let scoped_a = scoped_resources_fs(Arc::clone(&backend), "tenant-a", "alice");
        let scoped_b = scoped_resources_fs(Arc::clone(&backend), "tenant-b", "alice");

        let governor_a =
            PersistentResourceGovernor::new(FilesystemResourceGovernorStore::new(scoped_a));
        let governor_b =
            PersistentResourceGovernor::new(FilesystemResourceGovernorStore::new(scoped_b));

        let scope_a = sample_scope("tenant-a", "alice", Some("p1"));
        let scope_b = sample_scope("tenant-b", "alice", Some("p1"));
        let account_a = ResourceAccount::tenant(scope_a.tenant_id.clone());
        let account_b = ResourceAccount::tenant(scope_b.tenant_id.clone());

        governor_a
            .try_set_limit(
                account_a.clone(),
                ResourceLimits {
                    max_concurrency_slots: Some(1),
                    ..ResourceLimits::default()
                },
            )
            .unwrap();
        governor_a
            .reserve(
                scope_a,
                ResourceEstimate {
                    concurrency_slots: Some(1),
                    ..ResourceEstimate::default()
                },
            )
            .unwrap();
        // Tenant A has consumed its single slot.
        assert_eq!(
            governor_a
                .reserved_for(&account_a)
                .unwrap()
                .concurrency_slots,
            1
        );

        // Tenant B sees zero — no limit even set, no reservation visible.
        assert_eq!(
            governor_b
                .reserved_for(&account_b)
                .unwrap()
                .concurrency_slots,
            0
        );
        // Tenant B can reserve in its own scope without hitting tenant A's limit.
        governor_b
            .try_set_limit(
                account_b.clone(),
                ResourceLimits {
                    max_concurrency_slots: Some(1),
                    ..ResourceLimits::default()
                },
            )
            .unwrap();
        let reservation = governor_b
            .reserve(
                scope_b,
                ResourceEstimate {
                    concurrency_slots: Some(1),
                    ..ResourceEstimate::default()
                },
            )
            .unwrap();
        assert_eq!(
            governor_b
                .reserved_for(&account_b)
                .unwrap()
                .concurrency_slots,
            1
        );
        // Tenant A's reservation count is unaffected.
        assert_eq!(
            governor_a
                .reserved_for(&account_a)
                .unwrap()
                .concurrency_slots,
            1
        );

        governor_b.release(reservation.id).unwrap();
    }
}
