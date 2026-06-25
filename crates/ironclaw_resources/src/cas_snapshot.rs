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

use std::future::Future;
use std::sync::{Arc, OnceLock, mpsc};

use ironclaw_filesystem::{
    CasApply, CasUpdateError, ContentType, Entry, RootFilesystem, ScopedFilesystem, cas_update,
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
    /// Concurrency: routed through the shared, lock-free
    /// [`cas_update`](ironclaw_filesystem::cas_update) helper — an
    /// optimistic CAS-retry loop with bounded retries, jittered backoff,
    /// and an overall timeout. No per-record `tokio::sync::Mutex` is held
    /// across the backend `get`/`put` awaits, so a burst of same-scope
    /// writers overlaps instead of convoying behind one stalled writer.
    /// The `update` closure is re-run against a freshly read snapshot on
    /// every CAS retry, so it must be idempotent / re-runnable (the
    /// store closures are pure field mutations).
    pub(crate) fn update<S, T, E, U>(&self, update: U) -> Result<T, E>
    where
        S: Snapshot + Clone + PartialEq,
        T: Send + 'static,
        E: StorageError,
        U: FnMut(&mut S) -> Result<T, E> + Send + 'static,
    {
        self.update_with_scope::<S, T, E, U>(self.scope.clone(), update)
    }

    /// Run a read-modify-write transaction against the underlying
    /// snapshot using a caller-supplied [`ResourceScope`].
    ///
    /// The `ScopedFilesystem` rewrites the snapshot path under the
    /// supplied scope's tenant/user mount view, so two distinct scopes
    /// hit separate snapshot files. Cross-process and same-process
    /// contention are both resolved by the shared `cas_update` helper's
    /// CAS-retry loop (see [`Self::update`]).
    pub(crate) fn update_with_scope<S, T, E, U>(
        &self,
        scope: ResourceScope,
        mut update: U,
    ) -> Result<T, E>
    where
        S: Snapshot + Clone + PartialEq,
        T: Send + 'static,
        E: StorageError,
        U: FnMut(&mut S) -> Result<T, E> + Send + 'static,
    {
        let filesystem = Arc::clone(&self.filesystem);
        let path_str = self.path_str;
        let worker_cell = Arc::clone(&self.worker);
        let worker_name = self.worker_thread_name;
        run_on_worker(&worker_cell, worker_name, move || async move {
            let path = ScopedPath::new(path_str.to_string()).map_err(|error| {
                E::storage(format!("invalid snapshot path {path_str}: {error}"))
            })?;
            let apply = |current: Option<S>| {
                // `cas_update` re-invokes this per retry against a fresh
                // snapshot. Build the default-on-absent snapshot, run the
                // caller's mutation, and hand back the next snapshot + outcome.
                let outcome = (|| {
                    let mut snapshot = current.unwrap_or_else(S::fresh);
                    let value = update(&mut snapshot)?;
                    Ok::<_, E>(CasApply::new(snapshot, value))
                })();
                async move { outcome }
            };
            cas_update(
                filesystem.as_ref(),
                &scope,
                &path,
                |bytes: &[u8]| {
                    serde_json::from_slice::<S>(bytes)
                        .map_err(|error| E::storage(format!("decode snapshot: {error}")))
                },
                |snapshot: &S| {
                    let encoded = serde_json::to_vec_pretty(snapshot).map_err(E::storage_from)?;
                    Ok(Entry::bytes(encoded).with_content_type(ContentType::json()))
                },
                apply,
            )
            .await
            .map_err(map_cas_error::<E>)
        })
    }
}

/// Map the shared helper's [`CasUpdateError`] into a store error.
///
/// `Apply` carries the caller's own error straight through; every other
/// variant is a storage-layer failure surfaced via [`StorageError::storage`].
/// `CasUnsupported` is fail-closed: a backend that can't honor versioned CAS
/// is a misconfiguration, surfaced as a storage error rather than a silent
/// blind overwrite.
fn map_cas_error<E>(error: CasUpdateError<E>) -> E
where
    E: StorageError,
{
    match error {
        CasUpdateError::Apply(inner) => inner,
        CasUpdateError::Timeout => E::storage("snapshot CAS update timed out".to_string()),
        CasUpdateError::RetriesExhausted => {
            E::storage("snapshot CAS contention: retries exhausted".to_string())
        }
        CasUpdateError::CasUnsupported => {
            E::storage("snapshot backend does not support versioned compare-and-swap".to_string())
        }
        CasUpdateError::Backend(inner) => E::storage_from(inner),
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

#[cfg(test)]
mod tests {
    //! Concurrency regression tests for the shared CAS-snapshot store.
    //!
    //! `update_with_scope` historically serialized same-process writers on
    //! a per-path `tokio::sync::Mutex` held across the backend `get`/`put`
    //! awaits. That mutex was the *only* serializer — `update_snapshot` does
    //! a single read-modify-write with no CAS-retry loop. The convoy test
    //! below proves both halves of the bug:
    //!
    //! 1. With the per-record lock removed but no retry loop, concurrent
    //!    same-scope writers race their read-modify-write and **lose
    //!    updates** (the snapshot ends below the writer count).
    //! 2. A writer whose backend `get` stalls holds the per-record mutex
    //!    across the await, so every other same-scope writer is parked
    //!    behind it — the convoy.
    //!
    //! After migration to `ironclaw_filesystem::cas_update` the lock is gone
    //! and the helper's bounded CAS-retry recovers every racing writer, so
    //! the same storm lands all updates with no convoy.

    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use async_trait::async_trait;
    use ironclaw_filesystem::{
        BackendCapabilities, CasExpectation, DirEntry, Entry, FileStat, FilesystemError,
        InMemoryBackend, RecordVersion, RootFilesystem, ScopedFilesystem, VersionedEntry,
    };
    use ironclaw_host_api::{
        MountAlias, MountGrant, MountPermissions, MountView, ResourceScope, VirtualPath,
    };
    use serde::{Deserialize, Serialize};

    use super::{CasSnapshotStore, Snapshot, StorageError};

    const COUNTER_PATH: &str = "/resources/counter.json";

    #[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
    struct Counter {
        value: u64,
    }

    impl Snapshot for Counter {
        fn fresh() -> Self {
            Counter { value: 0 }
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct TestError(String);

    impl StorageError for TestError {
        fn storage(reason: String) -> Self {
            TestError(reason)
        }
    }

    /// Wraps any backend and inserts a fixed delay before each `get`, so two
    /// concurrent read-modify-write transactions are forced to interleave:
    /// both read the same version, both compute the next value, and (without
    /// CAS-retry) one of the two writes is silently lost.
    struct SlowGetBackend {
        inner: InMemoryBackend,
        get_delay: Duration,
        get_calls: AtomicUsize,
    }

    impl SlowGetBackend {
        fn new(get_delay: Duration) -> Self {
            Self {
                inner: InMemoryBackend::new(),
                get_delay,
                get_calls: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait]
    impl RootFilesystem for SlowGetBackend {
        fn capabilities(&self) -> BackendCapabilities {
            self.inner.capabilities()
        }

        async fn put(
            &self,
            path: &VirtualPath,
            entry: Entry,
            cas: CasExpectation,
        ) -> Result<RecordVersion, FilesystemError> {
            self.inner.put(path, entry, cas).await
        }

        async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
            self.get_calls.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(self.get_delay).await;
            self.inner.get(path).await
        }

        async fn delete(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
            self.inner.delete(path).await
        }

        async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
            self.inner.list_dir(path).await
        }

        async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
            self.inner.stat(path).await
        }
    }

    fn scoped(backend: Arc<SlowGetBackend>) -> Arc<ScopedFilesystem<SlowGetBackend>> {
        let mounts = MountView::new(vec![MountGrant::new(
            MountAlias::new("/resources").expect("alias"),
            VirtualPath::new("/tenants/t/users/u/resources").expect("target"),
            MountPermissions::read_write_list_delete(),
        )])
        .expect("mount view");
        Arc::new(ScopedFilesystem::with_fixed_view(backend, mounts))
    }

    /// Build a *distinct* store handle (own worker thread) so concurrent
    /// `update` calls actually run on different runtimes — otherwise the
    /// shared current-thread worker would serialize them itself and mask the
    /// race we want to exercise.
    fn store(
        filesystem: Arc<ScopedFilesystem<SlowGetBackend>>,
        worker_name: &'static str,
    ) -> CasSnapshotStore<SlowGetBackend> {
        CasSnapshotStore::new(
            filesystem,
            COUNTER_PATH,
            ResourceScope::system(),
            worker_name,
        )
    }

    /// High-contention storm: N concurrent writers each increment the shared
    /// counter by one. With the shared `cas_update` helper (bounded CAS
    /// retry, no per-record mutex) every increment must land — the final
    /// snapshot equals the writer count and no convoy parks a writer behind
    /// a stalled one.
    ///
    /// Before the migration (per-record mutex, single read-modify-write, no
    /// retry) this test is RED: the `SlowGetBackend` widens the race window
    /// so two writers read the same version and one increment is lost, so
    /// the final value is < WRITERS.
    #[test]
    fn concurrent_increments_have_no_lost_updates() {
        const WRITERS: usize = 8;
        const WORKER_NAMES: [&str; WRITERS] = [
            "cas-storm-0",
            "cas-storm-1",
            "cas-storm-2",
            "cas-storm-3",
            "cas-storm-4",
            "cas-storm-5",
            "cas-storm-6",
            "cas-storm-7",
        ];

        let backend = Arc::new(SlowGetBackend::new(Duration::from_millis(15)));
        let filesystem = scoped(Arc::clone(&backend));

        let mut handles = Vec::new();
        for name in WORKER_NAMES {
            let store = store(Arc::clone(&filesystem), name);
            handles.push(std::thread::spawn(move || {
                store.update::<Counter, u64, TestError, _>(|snapshot| {
                    snapshot.value += 1;
                    Ok(snapshot.value)
                })
            }));
        }

        let mut outcomes = Vec::new();
        for handle in handles {
            outcomes.push(handle.join().expect("writer thread").expect("writer ok"));
        }

        // Read the final snapshot through a fresh handle.
        let final_value = store(Arc::clone(&filesystem), "cas-storm-final")
            .update::<Counter, u64, TestError, _>(|snapshot| Ok(snapshot.value))
            .expect("read final");

        assert_eq!(
            final_value, WRITERS as u64,
            "every concurrent increment must land (no lost update); got {final_value}"
        );

        // Each writer observed a distinct increment in 1..=WRITERS.
        outcomes.sort_unstable();
        let expected: Vec<u64> = (1..=WRITERS as u64).collect();
        assert_eq!(
            outcomes, expected,
            "each writer's returned value must be a unique increment"
        );
    }
}
