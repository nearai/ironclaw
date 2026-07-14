//! Tests for the shared [`cas_update`](super::cas_update) helper.
//!
//! Exercises the four behaviors Phase 2 store-migrators depend on:
//! high-contention correctness (no lost updates), bounded retries (persistent
//! mismatch terminates), the fail-closed capability gate, and the
//! create-if-absent first-write path. All tests use a controllable in-memory
//! backend; the only sleeps are the helper's own jittered backoff, which is
//! capped at 50ms so the storm test stays fast and deterministic.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use async_trait::async_trait;
use ironclaw_host_api::{
    MountAlias, MountGrant, MountPermissions, MountView, ResourceScope, ScopedPath, VirtualPath,
};
use serde::{Deserialize, Serialize};

use super::{CasApply, CasUpdateError, cas_update, cas_update_root};
use crate::{
    BackendCapabilities, BackendId, BackendKind, Capability, CasExpectation,
    CompositeRootFilesystem, ContentKind, ContentType, DirEntry, Entry, FileStat, FilesystemError,
    FilesystemOperation, InMemoryBackend, IndexPolicy, MountDescriptor, RecordKind, RecordVersion,
    RootFilesystem, ScopedFilesystem, StorageClass, VersionedEntry,
};

// ─── Fixtures ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Counter {
    value: u64,
}

#[derive(Debug)]
struct TestError(String);

impl std::fmt::Display for TestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

const COUNTER_PATH: &str = "/counters/state.json";

fn counter_path() -> ScopedPath {
    ScopedPath::new(COUNTER_PATH).unwrap()
}

fn root_counter_path() -> VirtualPath {
    VirtualPath::new("/engine/counters/state.json").unwrap()
}

fn decode_counter(bytes: &[u8]) -> Result<Counter, TestError> {
    serde_json::from_slice(bytes).map_err(|e| TestError(e.to_string()))
}

fn encode_counter(counter: &Counter) -> Result<Entry, TestError> {
    let body = serde_json::to_vec(counter).map_err(|e| TestError(e.to_string()))?;
    Ok(Entry::bytes(body).with_content_type(ContentType::json()))
}

/// Scope-agnostic single-tenant view over the `/counters` alias.
fn scoped<F: RootFilesystem>(root: Arc<F>) -> ScopedFilesystem<F> {
    ScopedFilesystem::with_fixed_view(
        root,
        MountView::new(vec![MountGrant::new(
            MountAlias::new("/counters").unwrap(),
            VirtualPath::new("/engine/counters").unwrap(),
            MountPermissions::read_write_list_delete(),
        )])
        .unwrap(),
    )
}

/// Increment-by-one `apply` closure shared by several tests.
async fn increment(current: Option<Counter>) -> Result<CasApply<Counter, u64>, TestError> {
    let next = current.map(|c| c.value).unwrap_or(0) + 1;
    Ok(CasApply::new(Counter { value: next }, next))
}

// ─── A non-CAS, byte-only backend ───────────────────────────────────────────

/// Byte-only backend that advertises [`BackendCapabilities::bytes_only`] (no
/// transaction tier) and **always** overwrites on `put` regardless of the CAS
/// expectation. If the capability gate ever let a write through, this backend
/// would silently clobber — the test asserts it never does.
struct NonCasBackend {
    inner: InMemoryBackend,
    get_called: AtomicBool,
    put_called: AtomicBool,
}

impl NonCasBackend {
    fn new() -> Self {
        Self {
            inner: InMemoryBackend::new(),
            get_called: AtomicBool::new(false),
            put_called: AtomicBool::new(false),
        }
    }
}

#[async_trait]
impl RootFilesystem for NonCasBackend {
    fn capabilities(&self) -> BackendCapabilities {
        // Known shape, no CAS tier → pre-flight gate must reject.
        BackendCapabilities::bytes_only()
    }

    async fn put(
        &self,
        path: &VirtualPath,
        entry: Entry,
        _cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError> {
        self.put_called.store(true, Ordering::SeqCst);
        // Blind overwrite: ignore the caller's CAS expectation entirely.
        self.inner.put(path, entry, CasExpectation::Any).await
    }

    async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        self.get_called.store(true, Ordering::SeqCst);
        self.inner.get(path).await
    }

    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        self.inner.list_dir(path).await
    }

    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        self.inner.stat(path).await
    }
}

/// CAS-capable backend that records every write precondition while delegating
/// storage to the real in-memory implementation.
struct RecordingBackend {
    inner: Arc<InMemoryBackend>,
    put_expectations: std::sync::Mutex<Vec<CasExpectation>>,
}

impl RecordingBackend {
    fn new() -> Self {
        Self {
            inner: Arc::new(InMemoryBackend::new()),
            put_expectations: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn expectations(&self) -> Vec<CasExpectation> {
        self.put_expectations.lock().unwrap().clone()
    }
}

#[async_trait]
impl RootFilesystem for RecordingBackend {
    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities::in_memory_full()
    }

    async fn put(
        &self,
        path: &VirtualPath,
        entry: Entry,
        cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError> {
        self.put_expectations.lock().unwrap().push(cas);
        self.inner.put(path, entry, cas).await
    }

    async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        self.inner.get(path).await
    }

    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        self.inner.list_dir(path).await
    }

    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        self.inner.stat(path).await
    }
}

/// Injects one winning write immediately before the caller's first versioned
/// put. The stale write must fail, forcing the helper to reread and reapply
/// against the winner's value and version.
struct OneRaceBackend {
    inner: Arc<InMemoryBackend>,
    raced: AtomicBool,
    get_calls: AtomicUsize,
    put_expectations: std::sync::Mutex<Vec<CasExpectation>>,
}

impl OneRaceBackend {
    fn new(inner: Arc<InMemoryBackend>) -> Self {
        Self {
            inner,
            raced: AtomicBool::new(false),
            get_calls: AtomicUsize::new(0),
            put_expectations: std::sync::Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl RootFilesystem for OneRaceBackend {
    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities::in_memory_full()
    }

    async fn put(
        &self,
        path: &VirtualPath,
        entry: Entry,
        cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError> {
        self.put_expectations.lock().unwrap().push(cas);
        if matches!(cas, CasExpectation::Version(_)) && !self.raced.swap(true, Ordering::SeqCst) {
            let current = self.inner.get(path).await?.expect("seeded race value");
            self.inner
                .put(
                    path,
                    encode_counter(&Counter { value: 10 }).unwrap(),
                    CasExpectation::Version(current.version),
                )
                .await?;
        }
        self.inner.put(path, entry, cas).await
    }

    async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        self.get_calls.fetch_add(1, Ordering::SeqCst);
        self.inner.get(path).await
    }

    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        self.inner.list_dir(path).await
    }

    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        self.inner.stat(path).await
    }
}

fn counter_mount_descriptor() -> MountDescriptor {
    MountDescriptor {
        virtual_root: VirtualPath::new("/engine/counters").unwrap(),
        backend_id: BackendId::new("root-cas-test").unwrap(),
        backend_kind: BackendKind::DatabaseFilesystem,
        storage_class: StorageClass::StructuredRecords,
        content_kind: ContentKind::SystemState,
        index_policy: IndexPolicy::NotIndexed,
        capabilities: BackendCapabilities::empty()
            .with(Capability::Read)
            .with(Capability::Write)
            .with_txn(crate::TxnCapability::Cas),
    }
}

// ─── A backend that always reports VersionMismatch on put ───────────────────

/// CAS-capable backend whose `put` always fails with `VersionMismatch`,
/// simulating a snapshot under permanent contention. Reads succeed so the
/// helper can keep re-reading and retrying until it exhausts its budget.
struct AlwaysMismatchBackend {
    inner: InMemoryBackend,
    put_attempts: AtomicUsize,
}

impl AlwaysMismatchBackend {
    fn new() -> Self {
        Self {
            inner: InMemoryBackend::new(),
            put_attempts: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl RootFilesystem for AlwaysMismatchBackend {
    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities::in_memory_full()
    }

    async fn put(
        &self,
        path: &VirtualPath,
        _entry: Entry,
        _cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError> {
        self.put_attempts.fetch_add(1, Ordering::SeqCst);
        Err(FilesystemError::VersionMismatch {
            path: path.clone(),
            expected: Some(RecordVersion::from_backend(1)),
            found: Some(RecordVersion::from_backend(2)),
        })
    }

    async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        // Always present at version 1, so the helper takes the put path (and
        // races it into a VersionMismatch) on every attempt without needing a
        // seed write — the seed `put` would itself be rejected by this backend.
        Ok(Some(VersionedEntry {
            path: path.clone(),
            entry: encode_counter(&Counter { value: 0 }).unwrap(),
            version: RecordVersion::from_backend(1),
        }))
    }

    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        self.inner.list_dir(path).await
    }

    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        self.inner.stat(path).await
    }
}

// ─── A backend whose `get` hangs forever ─────────────────────────────────────

/// Backend that suspends on every `get` call, simulating a wedged backend
/// operation. Used to exercise the [`super::FILESYSTEM_APPLY_TIMEOUT`]
/// deadline. The test drives it under paused Tokio time so the 15-second
/// deadline fires instantly without real wall-clock delay.
struct HangingBackend;

#[async_trait]
impl RootFilesystem for HangingBackend {
    fn capabilities(&self) -> BackendCapabilities {
        // Advertise full CAS capability so the pre-flight gate passes and the
        // helper enters the loop — where `get` then hangs forever.
        BackendCapabilities::in_memory_full()
    }

    async fn get(&self, _path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        // Suspend indefinitely. With `start_paused = true` Tokio auto-advances
        // its clock once every task is blocked on a non-timer future, which
        // causes the `tokio::time::timeout(FILESYSTEM_APPLY_TIMEOUT, …)`
        // wrapper in `cas_update` to fire.
        std::future::pending().await
    }

    async fn put(
        &self,
        _path: &VirtualPath,
        _entry: Entry,
        _cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError> {
        unimplemented!("HangingBackend::put is unreachable in this test")
    }

    async fn list_dir(&self, _path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        unimplemented!("HangingBackend::list_dir is unreachable in this test")
    }

    async fn stat(&self, _path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        unimplemented!("HangingBackend::stat is unreachable in this test")
    }
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn root_absent_record_creation_uses_absent_expectation() {
    let backend = RecordingBackend::new();

    let outcome = cas_update_root(
        &backend,
        &root_counter_path(),
        decode_counter,
        encode_counter,
        increment,
    )
    .await
    .unwrap();

    assert_eq!(outcome, 1);
    assert_eq!(backend.expectations(), vec![CasExpectation::Absent]);
}

#[tokio::test]
async fn root_version_mismatch_rereads_and_reapplies_to_fresh_snapshot() {
    let inner = Arc::new(InMemoryBackend::new());
    let path = root_counter_path();
    let initial_version = inner
        .put(
            &path,
            encode_counter(&Counter { value: 1 }).unwrap(),
            CasExpectation::Absent,
        )
        .await
        .unwrap();
    let backend = OneRaceBackend::new(Arc::clone(&inner));
    let apply_calls = AtomicUsize::new(0);

    let outcome = cas_update_root(
        &backend,
        &path,
        decode_counter,
        encode_counter,
        |current: Option<Counter>| {
            apply_calls.fetch_add(1, Ordering::SeqCst);
            increment(current)
        },
    )
    .await
    .unwrap();

    assert_eq!(outcome, 11, "retry must apply to the concurrent winner");
    assert_eq!(backend.get_calls.load(Ordering::SeqCst), 2);
    assert_eq!(apply_calls.load(Ordering::SeqCst), 2);
    assert_eq!(
        backend.put_expectations.lock().unwrap().as_slice(),
        [
            CasExpectation::Version(initial_version),
            CasExpectation::Version(RecordVersion::from_backend(2)),
        ]
    );
    let stored = inner.get(&path).await.unwrap().unwrap();
    assert_eq!(decode_counter(&stored.entry.body).unwrap().value, 11);
}

#[tokio::test]
async fn root_equality_no_op_suppresses_write() {
    let backend = RecordingBackend::new();
    let path = root_counter_path();
    let version_before = backend
        .inner
        .put(
            &path,
            encode_counter(&Counter { value: 5 }).unwrap(),
            CasExpectation::Absent,
        )
        .await
        .unwrap();

    cas_update_root(
        &backend,
        &path,
        decode_counter,
        encode_counter,
        |current: Option<Counter>| async move {
            let snapshot = current.unwrap();
            Ok::<_, TestError>(CasApply::new(snapshot, ()))
        },
    )
    .await
    .unwrap();

    assert!(backend.expectations().is_empty());
    assert_eq!(
        backend.inner.get(&path).await.unwrap().unwrap().version,
        version_before
    );
}

#[tokio::test]
async fn root_explicit_no_op_on_absent_record_suppresses_write() {
    let backend = RecordingBackend::new();

    let outcome = cas_update_root(
        &backend,
        &root_counter_path(),
        decode_counter,
        encode_counter,
        |current: Option<Counter>| async move {
            assert!(current.is_none());
            Ok::<_, TestError>(CasApply::no_op(Counter { value: 0 }, 42))
        },
    )
    .await
    .unwrap();

    assert_eq!(outcome, 42);
    assert!(backend.expectations().is_empty());
    assert!(
        backend
            .inner
            .get(&root_counter_path())
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn root_known_non_cas_backend_rejects_before_get_apply_or_put() {
    let backend = NonCasBackend::new();
    let apply_called = AtomicBool::new(false);

    let result = cas_update_root(
        &backend,
        &root_counter_path(),
        decode_counter,
        encode_counter,
        |current: Option<Counter>| {
            apply_called.store(true, Ordering::SeqCst);
            increment(current)
        },
    )
    .await;

    assert!(matches!(result, Err(CasUpdateError::CasUnsupported)));
    assert!(!backend.get_called.load(Ordering::SeqCst));
    assert!(!apply_called.load(Ordering::SeqCst));
    assert!(!backend.put_called.load(Ordering::SeqCst));
}

#[tokio::test]
async fn root_unknown_capabilities_defer_to_op_time_unsupported() {
    let backend = UnsupportedWriteBackend::new();
    let apply_called = AtomicBool::new(false);

    let result = cas_update_root(
        &backend,
        &root_counter_path(),
        decode_counter,
        encode_counter,
        |current: Option<Counter>| {
            apply_called.store(true, Ordering::SeqCst);
            increment(current)
        },
    )
    .await;

    assert!(matches!(result, Err(CasUpdateError::CasUnsupported)));
    assert!(apply_called.load(Ordering::SeqCst));
    assert!(backend.put_called.load(Ordering::SeqCst));
}

#[tokio::test]
async fn root_decode_error_is_preserved_without_put() {
    let path = root_counter_path();
    let decode_backend = RecordingBackend::new();
    decode_backend
        .inner
        .put(
            &path,
            Entry::bytes(b"not-json".to_vec()),
            CasExpectation::Absent,
        )
        .await
        .unwrap();
    let decode_result: Result<u64, CasUpdateError<TestError>> = cas_update_root(
        &decode_backend,
        &path,
        decode_counter,
        encode_counter,
        increment,
    )
    .await;
    match decode_result {
        Err(CasUpdateError::Apply(error)) => assert!(!error.0.is_empty()),
        other => panic!("decode error must remain Apply, got {other:?}"),
    }
    assert!(decode_backend.expectations().is_empty());
}

#[tokio::test]
async fn root_apply_error_is_preserved_without_put() {
    let path = root_counter_path();
    let apply_backend = RecordingBackend::new();
    let apply_result: Result<u64, CasUpdateError<TestError>> = cas_update_root(
        &apply_backend,
        &path,
        decode_counter,
        encode_counter,
        |_current: Option<Counter>| async move {
            Err::<CasApply<Counter, u64>, _>(TestError("apply".to_string()))
        },
    )
    .await;
    match apply_result {
        Err(CasUpdateError::Apply(error)) => assert_eq!(error.0, "apply"),
        other => panic!("apply error must remain Apply, got {other:?}"),
    }
    assert!(apply_backend.expectations().is_empty());
}

#[tokio::test]
async fn root_encode_error_is_preserved_without_put() {
    let path = root_counter_path();
    let encode_backend = RecordingBackend::new();
    let encode_result: Result<u64, CasUpdateError<TestError>> = cas_update_root(
        &encode_backend,
        &path,
        decode_counter,
        |_snapshot: &Counter| -> Result<Entry, TestError> { Err(TestError("encode".to_string())) },
        increment,
    )
    .await;
    match encode_result {
        Err(CasUpdateError::Apply(error)) => assert_eq!(error.0, "encode"),
        other => panic!("encode error must remain Apply, got {other:?}"),
    }
    assert!(encode_backend.expectations().is_empty());
}

#[tokio::test]
async fn root_accepts_dyn_root_filesystem() {
    let filesystem: Arc<dyn RootFilesystem> = Arc::new(InMemoryBackend::new());

    let outcome = cas_update_root(
        filesystem.as_ref(),
        &root_counter_path(),
        decode_counter,
        encode_counter,
        increment,
    )
    .await
    .unwrap();

    assert_eq!(outcome, 1);
}

#[tokio::test]
async fn root_composite_with_unknown_aggregate_capabilities_routes_to_cas_mount() {
    let backend = Arc::new(InMemoryBackend::new());
    let mut composite = CompositeRootFilesystem::new();
    composite
        .mount(counter_mount_descriptor(), Arc::clone(&backend))
        .unwrap();

    let outcome = cas_update_root(
        &composite,
        &root_counter_path(),
        decode_counter,
        encode_counter,
        increment,
    )
    .await
    .unwrap();

    assert_eq!(outcome, 1);
    let stored = backend.get(&root_counter_path()).await.unwrap().unwrap();
    assert_eq!(decode_counter(&stored.entry.body).unwrap().value, 1);
}

#[tokio::test]
async fn create_if_absent_first_write_succeeds() {
    let fs = Arc::new(scoped(Arc::new(InMemoryBackend::new())));
    let scope = ResourceScope::system();

    let outcome = cas_update(
        fs.as_ref(),
        &scope,
        &counter_path(),
        decode_counter,
        encode_counter,
        increment,
    )
    .await
    .unwrap();

    assert_eq!(outcome, 1, "first write returns the new value");

    // The record now exists at version 1 with the expected body.
    let stored = fs
        .get(&scope, &counter_path())
        .await
        .unwrap()
        .expect("counter persisted");
    let counter = decode_counter(&stored.entry.body).unwrap();
    assert_eq!(counter.value, 1);
}

#[tokio::test]
async fn no_op_apply_skips_write() {
    let fs = Arc::new(scoped(Arc::new(InMemoryBackend::new())));
    let scope = ResourceScope::system();

    // Seed a value of 5.
    cas_update(
        fs.as_ref(),
        &scope,
        &counter_path(),
        decode_counter,
        encode_counter,
        |current: Option<Counter>| async move {
            let _ = current;
            Ok::<_, TestError>(CasApply::new(Counter { value: 5 }, ()))
        },
    )
    .await
    .unwrap();

    let version_before = fs
        .get(&scope, &counter_path())
        .await
        .unwrap()
        .unwrap()
        .version;

    // An apply that returns the unchanged snapshot must not bump the version.
    cas_update(
        fs.as_ref(),
        &scope,
        &counter_path(),
        decode_counter,
        encode_counter,
        |current: Option<Counter>| async move {
            let snapshot = current.unwrap();
            Ok::<_, TestError>(CasApply::new(snapshot, ()))
        },
    )
    .await
    .unwrap();

    let version_after = fs
        .get(&scope, &counter_path())
        .await
        .unwrap()
        .unwrap()
        .version;
    assert_eq!(
        version_before, version_after,
        "no-op apply must not issue a write"
    );
}

#[tokio::test]
async fn high_contention_storm_has_no_lost_updates() {
    const WRITERS: u64 = 50;

    let fs = Arc::new(scoped(Arc::new(InMemoryBackend::new())));
    let scope = ResourceScope::system();

    let mut handles = Vec::new();
    for _ in 0..WRITERS {
        let fs = fs.clone();
        let scope = scope.clone();
        handles.push(tokio::spawn(async move {
            cas_update(
                fs.as_ref(),
                &scope,
                &counter_path(),
                decode_counter,
                encode_counter,
                increment,
            )
            .await
        }));
    }

    let mut observed = Vec::new();
    for handle in handles {
        observed.push(handle.await.unwrap().expect("writer succeeded"));
    }

    // Final value must equal the number of writers — every increment landed.
    let final_counter = decode_counter(
        &fs.get(&scope, &counter_path())
            .await
            .unwrap()
            .unwrap()
            .entry
            .body,
    )
    .unwrap();
    assert_eq!(
        final_counter.value, WRITERS,
        "every concurrent increment must be observed (no lost update)"
    );

    // Each writer observed a distinct increment value in 1..=WRITERS.
    observed.sort_unstable();
    let expected: Vec<u64> = (1..=WRITERS).collect();
    assert_eq!(
        observed, expected,
        "each writer's returned outcome must be a unique increment"
    );
}

#[tokio::test]
async fn persistent_version_mismatch_exhausts_retries() {
    let backend = Arc::new(AlwaysMismatchBackend::new());
    let fs = Arc::new(scoped(backend.clone()));
    let scope = ResourceScope::system();

    // `get` always returns a synthetic existing record, so every attempt takes
    // the put path and races into a VersionMismatch — no seed write needed.
    let result = cas_update(
        fs.as_ref(),
        &scope,
        &counter_path(),
        decode_counter,
        encode_counter,
        increment,
    )
    .await;

    assert!(
        matches!(result, Err(CasUpdateError::RetriesExhausted)),
        "persistent VersionMismatch must terminate with RetriesExhausted, got {result:?}"
    );
    assert_eq!(
        backend.put_attempts.load(Ordering::SeqCst),
        super::FILESYSTEM_CAS_RETRIES,
        "the loop must attempt exactly the retry cap before giving up"
    );
}

#[tokio::test]
async fn non_cas_backend_is_rejected_not_overwritten() {
    let backend = Arc::new(NonCasBackend::new());
    let fs = Arc::new(scoped(backend.clone()));
    let scope = ResourceScope::system();

    let result = cas_update(
        fs.as_ref(),
        &scope,
        &counter_path(),
        decode_counter,
        encode_counter,
        increment,
    )
    .await;

    assert!(
        matches!(result, Err(CasUpdateError::CasUnsupported)),
        "a non-CAS backend must be rejected by the capability gate, got {result:?}"
    );

    // Critically: nothing was written. The pre-flight gate refused before the
    // blind-overwrite `put` could run.
    let stored = fs.get(&scope, &counter_path()).await.unwrap();
    assert!(
        stored.is_none(),
        "the capability gate must reject before any write (no blind overwrite)"
    );
}

#[tokio::test]
async fn no_op_constructor_skips_write_on_absent_record() {
    // `CasApply::no_op` must skip the write even when `current` is `None`.
    // The `PartialEq` fast path cannot fire here because there is no `existing`
    // to compare against, so the explicit `write: false` flag is the only
    // correct signal.
    let fs = Arc::new(scoped(Arc::new(InMemoryBackend::new())));
    let scope = ResourceScope::system();

    let outcome = cas_update(
        fs.as_ref(),
        &scope,
        &counter_path(),
        decode_counter,
        encode_counter,
        |current: Option<Counter>| async move {
            assert!(current.is_none(), "record must be absent");
            Ok::<_, TestError>(CasApply::no_op(Counter { value: 0 }, 42u64))
        },
    )
    .await
    .unwrap();

    assert_eq!(outcome, 42u64, "no_op must return the supplied outcome");

    // Critically: the file must not have been created.
    let stored = fs.get(&scope, &counter_path()).await.unwrap();
    assert!(
        stored.is_none(),
        "CasApply::no_op on an absent record must not write the record"
    );
}

#[tokio::test]
async fn apply_error_is_carried_through_unwrapped() {
    let fs = Arc::new(scoped(Arc::new(InMemoryBackend::new())));
    let scope = ResourceScope::system();

    let result: Result<u64, CasUpdateError<TestError>> = cas_update(
        fs.as_ref(),
        &scope,
        &counter_path(),
        decode_counter,
        encode_counter,
        |_current: Option<Counter>| async move {
            Err::<CasApply<Counter, u64>, _>(TestError("boom".to_string()))
        },
    )
    .await;

    match result {
        Err(CasUpdateError::Apply(TestError(reason))) => assert_eq!(reason, "boom"),
        other => panic!("expected Apply error carried through, got {other:?}"),
    }
}

#[tokio::test(start_paused = true)]
async fn timeout_fires_when_backend_get_hangs() {
    // `HangingBackend::get` suspends forever via `std::future::pending()`.
    // With paused Tokio time the runtime auto-advances its clock the moment
    // every task is blocked, so `tokio::time::timeout(FILESYSTEM_APPLY_TIMEOUT,
    // …)` fires without any real wall-clock delay.
    let fs = Arc::new(scoped(Arc::new(HangingBackend)));
    let scope = ResourceScope::system();

    let result: Result<u64, CasUpdateError<TestError>> = cas_update(
        fs.as_ref(),
        &scope,
        &counter_path(),
        decode_counter,
        encode_counter,
        increment,
    )
    .await;

    assert!(
        matches!(result, Err(CasUpdateError::Timeout)),
        "a wedged backend `get` must trigger CasUpdateError::Timeout after \
         FILESYSTEM_APPLY_TIMEOUT elapses, got {result:?}"
    );
}

// ─── A backend with default capabilities whose put returns Unsupported ────────

/// Backend with default/unknown capabilities (so `capabilities_known()` returns
/// `false` and the pre-flight gate defers to op-time) whose `get` returns
/// `Ok(None)` and whose `put` returns
/// `FilesystemError::Unsupported { operation: WriteFile }`. This simulates the
/// composite-router fallback path: a backend that cannot honor CAS writes is
/// not caught up front, but is caught fail-closed when the write is attempted.
///
/// `put_called` is set to `true` the moment `put` is entered, proving that
/// the loop reached the write path via the op-time gate rather than aborting
/// early at the pre-flight capability gate.
struct UnsupportedWriteBackend {
    put_called: AtomicBool,
}

impl UnsupportedWriteBackend {
    fn new() -> Self {
        Self {
            put_called: AtomicBool::new(false),
        }
    }
}

#[async_trait]
impl RootFilesystem for UnsupportedWriteBackend {
    fn capabilities(&self) -> BackendCapabilities {
        // Default/empty shape — identical to `BackendCapabilities::empty()`.
        // `capabilities_known()` compares against `BackendCapabilities::default()`
        // and returns `false` here, so the pre-flight gate is bypassed and the
        // loop reaches `put` before the error surfaces.
        BackendCapabilities::default()
    }

    async fn get(&self, _path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        // No existing record — the loop will attempt a first write.
        Ok(None)
    }

    async fn put(
        &self,
        path: &VirtualPath,
        _entry: Entry,
        _cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError> {
        // Record that `put` was reached before returning the error, so the
        // test can assert the loop took the op-time path rather than the
        // pre-flight capability gate.
        self.put_called.store(true, Ordering::SeqCst);
        Err(FilesystemError::Unsupported {
            path: path.clone(),
            operation: FilesystemOperation::WriteFile,
        })
    }

    async fn list_dir(&self, _path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        unimplemented!("UnsupportedWriteBackend::list_dir is unreachable in this test")
    }

    async fn stat(&self, _path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        unimplemented!("UnsupportedWriteBackend::stat is unreachable in this test")
    }
}

#[tokio::test]
async fn unsupported_write_file_maps_to_cas_unsupported() {
    // Regression for the op-time fail-closed path (distinct from the pre-flight
    // path exercised by `non_cas_backend_is_rejected_not_overwritten`).
    //
    // The backend advertises `BackendCapabilities::default()` — the empty/unknown
    // shape — so `capabilities_known()` returns `false` and the pre-flight gate
    // does NOT fire. The helper enters the loop, `get` returns `Ok(None)`,
    // `increment` returns a real change (`Counter { value: 1 }`), and `cas_update`
    // attempts the write. The backend's `put` then returns
    // `FilesystemError::Unsupported { operation: WriteFile }`, which the
    // `Unsupported { operation: FilesystemOperation::WriteFile, .. } =>
    // return Err(CasUpdateError::CasUnsupported)` arm of `cas_update_loop` maps
    // to `CasUpdateError::CasUnsupported`.
    //
    // This is the composite-router fallback path: a backend without CAS support
    // is not always detectable at pre-flight but must still fail closed.
    let backend = Arc::new(UnsupportedWriteBackend::new());
    let fs = Arc::new(scoped(backend.clone()));
    let scope = ResourceScope::system();

    let result = cas_update(
        fs.as_ref(),
        &scope,
        &counter_path(),
        decode_counter,
        encode_counter,
        increment,
    )
    .await;

    assert!(
        matches!(result, Err(CasUpdateError::CasUnsupported)),
        "an op-time Unsupported(WriteFile) from a default-capability backend must \
         map to CasUpdateError::CasUnsupported (fail-closed op-time path), got {result:?}"
    );
    // Prove the error came from the op-time path: `put` must have been reached.
    // If a future change made `BackendCapabilities::default()` trip the
    // pre-flight gate instead, `put` would never fire and the assertion above
    // would pass for the wrong reason — this guard catches that regression.
    assert!(
        backend.put_called.load(Ordering::SeqCst),
        "put must have been invoked: the loop must reach the write path (op-time gate) \
         before CasUpdateError::CasUnsupported is returned"
    );
}

// ─── A CAS-capable backend whose `get` returns a generic error ────────────────

/// CAS-capable backend whose `get` always returns a [`FilesystemError::Backend`]
/// error — not a not-found / `Ok(None)`, not a `VersionMismatch`. Used to
/// exercise the `get`-error arm of `cas_update_loop` — the
/// `Err(error) => return Err(CasUpdateError::Backend(error))` branch of the
/// `match filesystem.get(...)` — when the read itself fails. The pre-flight
/// gate passes because full capabilities are declared; `get` then fails before any
/// decode or apply runs.
struct GetErrorBackend;

#[async_trait]
impl RootFilesystem for GetErrorBackend {
    fn capabilities(&self) -> BackendCapabilities {
        // Full CAS-capable shape — pre-flight gate passes and the loop enters.
        BackendCapabilities::in_memory_full()
    }

    async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        // A plain infrastructure error on the read path. Not a not-found
        // (`Ok(None)`) and not a `VersionMismatch` — a backend that is simply
        // broken or temporarily unavailable. The loop's get-error arm must
        // forward it unchanged as `CasUpdateError::Backend`.
        Err(FilesystemError::Backend {
            path: path.clone(),
            operation: FilesystemOperation::ReadFile,
            reason: "simulated backend read failure".to_string(),
        })
    }

    async fn put(
        &self,
        _path: &VirtualPath,
        _entry: Entry,
        _cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError> {
        unimplemented!("GetErrorBackend::put is unreachable in this test")
    }

    async fn list_dir(&self, _path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        unimplemented!("GetErrorBackend::list_dir is unreachable in this test")
    }

    async fn stat(&self, _path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        unimplemented!("GetErrorBackend::stat is unreachable in this test")
    }
}

// ─── A backend whose `get` returns a record with an unparseable body ──────────

/// CAS-capable backend whose `get` always returns a [`VersionedEntry`] whose
/// body is not valid JSON — simulating a corrupted or schema-incompatible
/// snapshot. Used to exercise the
/// `decode(&versioned.entry.body).map_err(CasUpdateError::Apply)?` step of the
/// `Ok(Some(versioned))` arm in `cas_update_loop`: the record is present so decode
/// runs, `serde_json` fails, and the error is wrapped as `CasUpdateError::Apply`.
struct MalformedBodyBackend;

#[async_trait]
impl RootFilesystem for MalformedBodyBackend {
    fn capabilities(&self) -> BackendCapabilities {
        // Full CAS-capable shape — pre-flight gate passes and the loop enters.
        BackendCapabilities::in_memory_full()
    }

    async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        // Return a present record whose body is not valid JSON. `decode_counter`
        // calls `serde_json::from_slice` on it, which fails and produces a
        // `TestError`; the loop wraps it as `CasUpdateError::Apply`.
        Ok(Some(VersionedEntry {
            path: path.clone(),
            entry: Entry::bytes(b"not-valid-json".to_vec()),
            version: RecordVersion::from_backend(1),
        }))
    }

    async fn put(
        &self,
        _path: &VirtualPath,
        _entry: Entry,
        _cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError> {
        unimplemented!("MalformedBodyBackend::put is unreachable in this test")
    }

    async fn list_dir(&self, _path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        unimplemented!("MalformedBodyBackend::list_dir is unreachable in this test")
    }

    async fn stat(&self, _path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        unimplemented!("MalformedBodyBackend::stat is unreachable in this test")
    }
}

// ─── A CAS-capable backend that tracks whether `put` was invoked ──────────────

/// CAS-capable backend whose `get` returns `Ok(None)` (absent record) and whose
/// `put` records that it was invoked via `put_called`. Used to assert that a
/// failing `encode` closure prevents the write from ever being attempted: the
/// loop reaches the encode step, the `?` operator short-circuits before
/// `filesystem.put(...)` is reached, and `put_called` remains `false`.
struct PutTrackingBackend {
    put_called: AtomicBool,
}

impl PutTrackingBackend {
    fn new() -> Self {
        Self {
            put_called: AtomicBool::new(false),
        }
    }
}

#[async_trait]
impl RootFilesystem for PutTrackingBackend {
    fn capabilities(&self) -> BackendCapabilities {
        // Full CAS-capable shape — pre-flight gate passes and the loop enters.
        BackendCapabilities::in_memory_full()
    }

    async fn get(&self, _path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        // No existing record. `apply` (`increment`) receives `None`, returns a
        // real change (Counter { value: 1 }), so the loop reaches the encode step.
        Ok(None)
    }

    async fn put(
        &self,
        _path: &VirtualPath,
        _entry: Entry,
        _cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError> {
        // Record that `put` was reached. If `encode` fails with `?` before this
        // point, the loop never arrives here and this flag stays `false`.
        self.put_called.store(true, Ordering::SeqCst);
        Ok(RecordVersion::from_backend(1))
    }

    async fn list_dir(&self, _path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        unimplemented!("PutTrackingBackend::list_dir is unreachable in this test")
    }

    async fn stat(&self, _path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        unimplemented!("PutTrackingBackend::stat is unreachable in this test")
    }
}

// ─── A CAS-capable backend whose `put` returns a generic non-CAS error ────────

/// CAS-capable backend whose `get` returns `Ok(None)` and whose `put` returns
/// a plain [`FilesystemError::Backend`] — neither `VersionMismatch` nor
/// `Unsupported { operation: WriteFile }`. Used to exercise the catch-all
/// `Err(error) => Err(CasUpdateError::Backend(error))` arm in `cas_update_loop`
/// — the fallback after the `VersionMismatch` and `Unsupported { operation:
/// WriteFile, .. }` arms on the `match filesystem.put(...)`. The pre-flight gate
/// passes because full capabilities are declared; `get` returns absent so the
/// loop attempts a first write; `put` then returns the generic error the loop
/// must forward as-is.
struct GenericPutErrorBackend;

#[async_trait]
impl RootFilesystem for GenericPutErrorBackend {
    fn capabilities(&self) -> BackendCapabilities {
        // Full CAS-capable shape — pre-flight gate passes and the loop enters.
        BackendCapabilities::in_memory_full()
    }

    async fn get(&self, _path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        // No existing record — loop takes the first-write path and reaches `put`.
        Ok(None)
    }

    async fn put(
        &self,
        path: &VirtualPath,
        _entry: Entry,
        _cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError> {
        // A plain backend failure that is neither `VersionMismatch` (which
        // would trigger a retry) nor `Unsupported { WriteFile }` (which maps
        // to `CasUnsupported`). The loop's catch-all arm must forward it as
        // `CasUpdateError::Backend`.
        Err(FilesystemError::Backend {
            path: path.clone(),
            operation: FilesystemOperation::WriteFile,
            reason: "simulated generic backend failure".to_string(),
        })
    }

    async fn list_dir(&self, _path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        unimplemented!("GenericPutErrorBackend::list_dir is unreachable in this test")
    }

    async fn stat(&self, _path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        unimplemented!("GenericPutErrorBackend::stat is unreachable in this test")
    }
}

#[tokio::test]
async fn backend_put_error_maps_to_backend() {
    // Regression for the catch-all `Err(error) => Err(CasUpdateError::Backend(error))`
    // arm in `cas_update_loop` — the fallback on the `match filesystem.put(...)`
    // after the `VersionMismatch` (retry) and `Unsupported { operation: WriteFile,
    // .. }` (CasUnsupported) arms.
    //
    // The backend advertises full CAS capability so the pre-flight gate passes
    // and the helper enters the loop. `get` returns `Ok(None)` so the loop
    // takes the first-write (`CasExpectation::Absent`) path and calls `put`.
    // `put` returns `FilesystemError::Backend` — a generic infrastructure
    // failure that is neither `VersionMismatch` (retry) nor
    // `Unsupported { WriteFile }` (maps to CasUnsupported). The helper must
    // forward it unchanged as `CasUpdateError::Backend(_)`.
    let fs = Arc::new(scoped(Arc::new(GenericPutErrorBackend)));
    let scope = ResourceScope::system();

    let result: Result<u64, CasUpdateError<TestError>> = cas_update(
        fs.as_ref(),
        &scope,
        &counter_path(),
        decode_counter,
        encode_counter,
        increment,
    )
    .await;

    assert!(
        matches!(result, Err(CasUpdateError::Backend(_))),
        "a non-VersionMismatch, non-Unsupported(WriteFile) put error must surface as \
         CasUpdateError::Backend, got {result:?}"
    );
}

#[tokio::test]
async fn backend_get_error_maps_to_backend() {
    // Regression for the `get`-error arm in `cas_update_loop`:
    // `Err(error) => return Err(CasUpdateError::Backend(error))` on the
    // `match filesystem.get(...)`.
    //
    // The backend advertises full CAS capability so the pre-flight gate passes
    // and the helper enters the loop. `get` then returns a plain infrastructure
    // error (not `Ok(None)` and not a `VersionMismatch`). The helper must
    // forward it unchanged as `CasUpdateError::Backend(_)`.
    let fs = Arc::new(scoped(Arc::new(GetErrorBackend)));
    let scope = ResourceScope::system();

    let result: Result<u64, CasUpdateError<TestError>> = cas_update(
        fs.as_ref(),
        &scope,
        &counter_path(),
        decode_counter,
        encode_counter,
        increment,
    )
    .await;

    assert!(
        matches!(result, Err(CasUpdateError::Backend(_))),
        "a backend error from get must surface as CasUpdateError::Backend, got {result:?}"
    );
}

#[tokio::test]
async fn malformed_snapshot_decode_maps_to_apply() {
    // Regression for the decode-error step in `cas_update_loop`'s
    // `Ok(Some(versioned))` arm: `decode(&versioned.entry.body)
    // .map_err(CasUpdateError::Apply)?`.
    //
    // The backend advertises full CAS capability so the pre-flight gate passes
    // and the helper enters the loop. `get` returns a present record whose body
    // is not valid JSON. `decode_counter` calls `serde_json::from_slice` on the
    // garbled bytes, which fails and produces a `TestError`; the loop wraps it
    // as `CasUpdateError::Apply`.
    let fs = Arc::new(scoped(Arc::new(MalformedBodyBackend)));
    let scope = ResourceScope::system();

    let result: Result<u64, CasUpdateError<TestError>> = cas_update(
        fs.as_ref(),
        &scope,
        &counter_path(),
        decode_counter,
        encode_counter,
        increment,
    )
    .await;

    match result {
        Err(CasUpdateError::Apply(_)) => {}
        other => panic!(
            "a decode failure on a present snapshot must surface as CasUpdateError::Apply, \
             got {other:?}"
        ),
    }
}

#[tokio::test]
async fn encode_failure_maps_to_apply_without_write() {
    // Regression for the encode-error step in `cas_update_loop`, after the
    // equality fast-path: `encode(&snapshot).map_err(CasUpdateError::Apply)?`.
    //
    // The backend advertises full CAS capability so the pre-flight gate passes
    // and the helper enters the loop. `get` returns `Ok(None)` so `apply`
    // (`increment`) receives `None` and produces a real change (Counter { value: 1 }),
    // which the loop attempts to encode. The encode closure always returns an error;
    // the `?` operator short-circuits before `filesystem.put(...)` is ever called,
    // and the error is wrapped as `CasUpdateError::Apply`.
    let backend = Arc::new(PutTrackingBackend::new());
    let fs = Arc::new(scoped(backend.clone()));
    let scope = ResourceScope::system();

    let result: Result<u64, CasUpdateError<TestError>> = cas_update(
        fs.as_ref(),
        &scope,
        &counter_path(),
        decode_counter,
        |_snapshot: &Counter| -> Result<Entry, TestError> {
            Err(TestError("encode always fails".to_string()))
        },
        increment,
    )
    .await;

    assert!(
        matches!(result, Err(CasUpdateError::Apply(_))),
        "an encode error must surface as CasUpdateError::Apply, got {result:?}"
    );
    assert!(
        !backend.put_called.load(Ordering::SeqCst),
        "put must not be called when encode fails before the write"
    );
}

// ─── A byte-only backend that rejects record-shaped entries at put time ───────

/// Backend with default/unknown capabilities (pre-flight gate deferred to op
/// time) whose `get` returns `Ok(None)` (absent → first write) and whose `put`
/// gates on `entry.kind.is_some()`: a record-shaped entry is rejected with
/// `FilesystemError::Unsupported { operation: WriteFile }`, while a byte-only
/// entry (kind = None) would be accepted. This models `LocalFilesystem`'s check
/// at local.rs:189-208: `if entry.kind.is_some() || !entry.indexed.is_empty() {
/// return Unsupported { WriteFile } }`.
///
/// `put_called` records that `put` was reached, proving the rejection came via
/// the op-time path (not the pre-flight capability gate).
struct KindGatedByteOnlyBackend {
    put_called: AtomicBool,
}

impl KindGatedByteOnlyBackend {
    fn new() -> Self {
        Self {
            put_called: AtomicBool::new(false),
        }
    }
}

#[async_trait]
impl RootFilesystem for KindGatedByteOnlyBackend {
    fn capabilities(&self) -> BackendCapabilities {
        // Default/unknown shape — `capabilities_known()` returns `false`, so the
        // pre-flight gate is bypassed and the loop enters before the error
        // surfaces at op time.
        BackendCapabilities::default()
    }

    async fn get(&self, _path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        // Absent record → loop takes the first-write (`CasExpectation::Absent`)
        // path and calls encode then put.
        Ok(None)
    }

    async fn put(
        &self,
        path: &VirtualPath,
        entry: Entry,
        cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError> {
        self.put_called.store(true, Ordering::SeqCst);
        assert!(
            matches!(cas, CasExpectation::Absent),
            "first writes must use CasExpectation::Absent"
        );
        if entry.kind.is_some() {
            // Record-shaped entries are rejected — mirrors `LocalFilesystem`'s
            // `if entry.kind.is_some() || !entry.indexed.is_empty()` guard.
            Err(FilesystemError::Unsupported {
                path: path.clone(),
                operation: FilesystemOperation::WriteFile,
            })
        } else {
            // Byte-only entries (kind = None) would be accepted.
            // This branch is not exercised in this test (the encode closure
            // always produces a record-shaped entry).
            Ok(RecordVersion::from_backend(1))
        }
    }

    async fn list_dir(&self, _path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        unimplemented!("KindGatedByteOnlyBackend::list_dir is unreachable in this test")
    }

    async fn stat(&self, _path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        unimplemented!("KindGatedByteOnlyBackend::stat is unreachable in this test")
    }
}

#[tokio::test]
async fn record_shaped_first_write_fails_closed_on_byte_only_backend() {
    // Regression for the byte-only first-write fail-closed gap.
    //
    // Store encoders now set `entry.kind = Some(RecordKind)` (record-shaped).
    // `LocalFilesystem` rejects record-shaped entries with `Unsupported { WriteFile }`
    // BEFORE the CAS check (local.rs:189-208: `if entry.kind.is_some() ||
    // !entry.indexed.is_empty() { return Unsupported { WriteFile } }`), even for
    // `CasExpectation::Absent`. This means `cas_update` with a record-shaped encode
    // closure against a byte-only backend must fail-closed as
    // `CasUpdateError::CasUnsupported` on the first (absent) write — closing the
    // gap where byte-only entries previously slipped through via Absent create.
    //
    // This test differs from `unsupported_write_file_maps_to_cas_unsupported`
    // (which returns Unsupported unconditionally regardless of entry shape) by
    // making the backend's `put` branch specifically on `entry.kind.is_some()` —
    // documenting that it is the record-shaped nature of the entry that triggers
    // the rejection, not a blanket refusal.
    let backend = Arc::new(KindGatedByteOnlyBackend::new());
    let fs = Arc::new(scoped(backend.clone()));
    let scope = ResourceScope::system();

    let result: Result<u64, CasUpdateError<TestError>> = cas_update(
        fs.as_ref(),
        &scope,
        &counter_path(),
        decode_counter,
        |snapshot: &Counter| -> Result<Entry, TestError> {
            // Produce a record-shaped entry: `entry.kind = Some(RecordKind)`.
            // This is what post-fix store encoders emit.
            let kind = RecordKind::new("test_record").map_err(|e| TestError(e.to_string()))?;
            let data = serde_json::to_value(snapshot).map_err(|e| TestError(e.to_string()))?;
            Entry::record(kind, &data).map_err(|e| TestError(e.to_string()))
        },
        increment,
    )
    .await;

    assert!(
        matches!(result, Err(CasUpdateError::CasUnsupported)),
        "a record-shaped entry against a byte-only backend must fail-closed as \
         CasUpdateError::CasUnsupported on the first (absent) write, got {result:?}"
    );
    // Prove the rejection came from the op-time path: `put` must have been reached.
    // If a future change made `BackendCapabilities::default()` trip the pre-flight
    // gate, `put` would never be entered and `CasUnsupported` would surface for the
    // wrong reason — this guard catches that regression.
    assert!(
        backend.put_called.load(Ordering::SeqCst),
        "put must have been reached (op-time path): the kind-gated rejection must \
         come from the backend's put, not from the pre-flight capability gate"
    );
}
