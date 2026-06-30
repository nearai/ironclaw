//! Shared CAS-snapshot infrastructure for filesystem-backed governor
//! stores in this crate.
//!
//! Both [`FilesystemResourceGovernorStore`](crate::FilesystemResourceGovernorStore)
//! and [`FilesystemBudgetGateStore`](crate::FilesystemBudgetGateStore)
//! share the same shape: a single JSON snapshot per scope, read-modify-
//! write through `ScopedFilesystem` with a `CasExpectation::Version`
//! precondition, an in-process per-path async lock that serializes
//! same-process writers, and a dedicated current-thread tokio runtime
//! that bridges the sync trait surface to the async filesystem API.
//!
//! Before this module existed, each store carried ~350 lines of its
//! own copy of this infrastructure. The two were drifting (different
//! snapshot encoding, different worker thread names, different
//! retention logic) and a third store would have meant a third copy.
//! Now the two stores parameterize this module over their snapshot
//! shape + error type and stay tiny shims (review feedback #3899:
//! collapse the duplicate filesystem-store infrastructure).
//!
//! The trait surface is intentionally private — only the two filesystem
//! stores in this crate consume it. Downstream crates use the
//! per-store public APIs.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, OnceLock, Weak, mpsc};

use ironclaw_filesystem::{
    CasExpectation, ContentType, Entry, FilesystemError, FilesystemOperation, RootFilesystem,
    ScopedFilesystem,
};
use ironclaw_host_api::{ResourceScope, ScopedPath};
use serde::{Serialize, de::DeserializeOwned};

/// Snapshot encoding boundary shared by all filesystem-backed stores
/// in this crate. The store provides:
///
/// - A `Snapshot` type (the in-memory representation of the JSON).
/// - The fresh-snapshot default used when the underlying file does
///   not yet exist.
/// - The error type produced for storage failures and the
///   constructor [`StorageError::storage`] used to surface them.
///
/// Decoding uses `serde_json::from_slice` and encoding uses
/// `serde_json::to_vec_pretty`; per-store custom shaping (schema-
/// version checks, retention pruning) belongs in the caller's
/// `update` closure.
pub(crate) trait StorageError: Send + 'static {
    /// Construct an error from a single sanitized message. Used for
    /// CAS contention, decode failures, worker-stopped paths, etc.
    fn storage(reason: String) -> Self;

    /// Helper for the common "wrap a Display error" path. Default
    /// implementation calls [`Self::storage`] with `error.to_string()`.
    fn storage_from(error: impl std::fmt::Display) -> Self
    where
        Self: Sized,
    {
        Self::storage(error.to_string())
    }
}

/// Per-store handle: a `ScopedFilesystem`, a fixed snapshot path
/// (validated lazily inside `update` to keep `new` infallible), the
/// scope used to resolve the alias, and a lazily-spawned async-runtime
/// worker. Constructed once per store instance; multiple stores can
/// share the same `ScopedFilesystem` but each has its own worker cell
/// so cell-spawn errors stay scoped per store.
pub(crate) struct CasSnapshotStore<F>
where
    F: RootFilesystem,
{
    filesystem: Arc<ScopedFilesystem<F>>,
    path_str: &'static str,
    scope: ResourceScope,
    worker: AsyncStorageWorkerCell,
    worker_thread_name: &'static str,
}

impl<F> Clone for CasSnapshotStore<F>
where
    F: RootFilesystem,
{
    fn clone(&self) -> Self {
        Self {
            filesystem: Arc::clone(&self.filesystem),
            path_str: self.path_str,
            scope: self.scope.clone(),
            worker: Arc::clone(&self.worker),
            worker_thread_name: self.worker_thread_name,
        }
    }
}

impl<F> CasSnapshotStore<F>
where
    F: RootFilesystem + 'static,
{
    pub(crate) fn new(
        filesystem: Arc<ScopedFilesystem<F>>,
        path_str: &'static str,
        default_scope: ResourceScope,
        worker_thread_name: &'static str,
    ) -> Self {
        Self {
            filesystem,
            path_str,
            scope: default_scope,
            worker: Arc::new(OnceLock::new()),
            worker_thread_name,
        }
    }

    /// Run a read-modify-write transaction against the underlying
    /// snapshot using the store's default scope.
    ///
    /// Concurrency: same-process writers serialize on a per-path async
    /// lock for the duration of the closure + write. Cross-process
    /// contention surfaces as `E::storage(...)` from the CAS-mismatch
    /// branch. Byte-only backends (`LocalFilesystem`) that don't
    /// support `CasExpectation::Version` fall back to
    /// `CasExpectation::Any` under the same in-process lock.
    pub(crate) fn update<S, T, E, U>(&self, update: U) -> Result<T, E>
    where
        S: Snapshot,
        T: Send + 'static,
        E: StorageError,
        U: FnOnce(&mut S) -> Result<T, E> + Send + 'static,
    {
        self.update_with_scope::<S, T, E, U>(self.scope.clone(), update)
    }

    /// Run a read-modify-write transaction against the underlying
    /// snapshot using a caller-supplied [`ResourceScope`].
    ///
    /// The `ScopedFilesystem` rewrites the snapshot path under the
    /// supplied scope's tenant/user mount view, so two distinct scopes
    /// hit separate snapshot files. The same per-path in-process async
    /// lock + cross-process CAS semantics apply.
    pub(crate) fn update_with_scope<S, T, E, U>(
        &self,
        scope: ResourceScope,
        update: U,
    ) -> Result<T, E>
    where
        S: Snapshot,
        T: Send + 'static,
        E: StorageError,
        U: FnOnce(&mut S) -> Result<T, E> + Send + 'static,
    {
        let filesystem = Arc::clone(&self.filesystem);
        let path_str = self.path_str;
        let worker_cell = Arc::clone(&self.worker);
        let worker_name = self.worker_thread_name;
        run_on_worker(&worker_cell, worker_name, move || async move {
            let path = ScopedPath::new(path_str.to_string()).map_err(|error| {
                E::storage(format!("invalid snapshot path {path_str}: {error}"))
            })?;
            let record_lock = filesystem_record_lock(&path);
            let _guard = record_lock.lock().await;
            update_snapshot::<F, S, T, E, U>(&filesystem, &scope, &path, update).await
        })
    }
}

/// Snapshots the CAS store wraps. Each store provides its own concrete
/// snapshot type (e.g. `ResourceGovernorSnapshot`, `BudgetGateSnapshot`)
/// implementing this trait so the shared `update_snapshot` helper can
/// decode, build a default-on-absent, and re-encode without knowing
/// per-store schema details.
pub(crate) trait Snapshot: DeserializeOwned + Serialize + Send + 'static {
    /// Construct the snapshot used when the underlying file does not
    /// yet exist (first write).
    fn fresh() -> Self;
}

async fn update_snapshot<F, S, T, E, U>(
    filesystem: &ScopedFilesystem<F>,
    scope: &ResourceScope,
    path: &ScopedPath,
    update: U,
) -> Result<T, E>
where
    F: RootFilesystem,
    S: Snapshot,
    E: StorageError,
    U: FnOnce(&mut S) -> Result<T, E>,
{
    let (mut snapshot, expectation) = match filesystem.get(scope, path).await {
        Ok(Some(versioned)) => {
            let snapshot: S = serde_json::from_slice(&versioned.entry.body)
                .map_err(|error| E::storage(format!("decode snapshot: {error}")))?;
            (snapshot, CasExpectation::Version(versioned.version))
        }
        Ok(None) => (S::fresh(), CasExpectation::Absent),
        Err(error) => return Err(E::storage_from(error)),
    };
    let value = update(&mut snapshot)?;
    let encoded = serde_json::to_vec_pretty(&snapshot).map_err(E::storage_from)?;
    let entry = Entry::bytes(encoded).with_content_type(ContentType::json());
    match put_with_cas(filesystem, scope, path, entry, expectation).await {
        Ok(()) => Ok(value),
        Err(PutError::VersionMismatch) => Err(E::storage(format!(
            "cross-process CAS contention on snapshot {}",
            path.as_str()
        ))),
        Err(PutError::Other(err)) => Err(err),
    }
}

enum PutError<E> {
    VersionMismatch,
    Other(E),
}

async fn put_with_cas<F, E>(
    filesystem: &ScopedFilesystem<F>,
    scope: &ResourceScope,
    path: &ScopedPath,
    entry: Entry,
    cas: CasExpectation,
) -> Result<(), PutError<E>>
where
    F: RootFilesystem,
    E: StorageError,
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
                    .map_err(|err| PutError::Other(E::storage_from(err)))?;
                if existing.is_some() {
                    return Err(PutError::VersionMismatch);
                }
            }
            filesystem
                .put(scope, path, fallback_entry, CasExpectation::Any)
                .await
                .map(|_| ())
                .map_err(|err| PutError::Other(E::storage_from(err)))
        }
        Err(err) => Err(PutError::Other(E::storage_from(err))),
    }
}

// ---------------------------------------------------------------------------
// Async-runtime adapter — the sync `update` API needs to call into the
// async `ScopedFilesystem`. We run a dedicated current-thread tokio
// runtime in a worker thread per store and post closures to it via a
// `mpsc::Sender<AsyncStorageJob>`.
//
// The worker struct is intentionally non-generic over the error type:
// closures box the future and erase the result, and the receiving
// side just reads `Result<T, E>` over the result channel. This lets a
// single `AsyncStorageWorker` serve `Result<_, ResourceError>` and
// `Result<_, BudgetGateError>` callers without duplicate workers per
// error type.
// ---------------------------------------------------------------------------

type AsyncStorageJob = Box<dyn FnOnce(&tokio::runtime::Runtime) + Send + 'static>;

struct AsyncStorageWorker {
    sender: mpsc::Sender<AsyncStorageJob>,
}

impl AsyncStorageWorker {
    fn spawn(name: &'static str) -> Result<Self, String> {
        let (sender, receiver) = mpsc::channel::<AsyncStorageJob>();
        let (ready_sender, ready_receiver) = mpsc::channel::<Result<(), String>>();
        std::thread::Builder::new()
            .name(name.to_string())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        let _ = ready_sender.send(Err(error.to_string()));
                        return;
                    }
                };
                let _ = ready_sender.send(Ok(()));
                while let Ok(job) = receiver.recv() {
                    job(&runtime);
                }
            })
            .map_err(|error| error.to_string())?;
        ready_receiver
            .recv()
            .map_err(|_| "cas snapshot storage worker failed to start".to_string())??;
        Ok(Self { sender })
    }

    fn run<T, E, Fut, F>(&self, build: F) -> Result<T, E>
    where
        T: Send + 'static,
        E: StorageError,
        Fut: Future<Output = Result<T, E>> + Send + 'static,
        F: FnOnce() -> Fut + Send + 'static,
    {
        let (result_sender, result_receiver) = mpsc::channel();
        self.sender
            .send(Box::new(move |runtime| {
                let result = runtime.block_on(build());
                let _ = result_sender.send(result);
            }))
            .map_err(|_| E::storage("cas snapshot storage worker stopped".to_string()))?;
        result_receiver
            .recv()
            .map_err(|_| E::storage("cas snapshot storage worker stopped".to_string()))?
    }
}

type AsyncStorageWorkerCell = Arc<OnceLock<Result<AsyncStorageWorker, String>>>;

fn run_on_worker<T, E, Fut, F>(
    worker_cell: &AsyncStorageWorkerCell,
    worker_thread_name: &'static str,
    build: F,
) -> Result<T, E>
where
    T: Send + 'static,
    E: StorageError,
    Fut: Future<Output = Result<T, E>> + Send + 'static,
    F: FnOnce() -> Fut + Send + 'static,
{
    let worker = worker_cell.get_or_init(|| AsyncStorageWorker::spawn(worker_thread_name));
    match worker {
        Ok(worker) => worker.run(build),
        Err(error) => Err(E::storage(error.clone())),
    }
}

// ---------------------------------------------------------------------------
// Per-path async serialization. Same shape as the run-state / processes
// / outbound / authorization migrations: an `OnceLock`-initialized map
// of weak Mutex handles, lazily pruned on each acquisition so the map
// size stays bounded under tenant churn.
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
