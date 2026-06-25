//! Tests for the shared [`cas_update`](super::cas_update) helper.
//!
//! Exercises the four behaviors Phase 2 store-migrators depend on:
//! high-contention correctness (no lost updates), bounded retries (persistent
//! mismatch terminates), the fail-closed capability gate, and the
//! create-if-absent first-write path. All tests use a controllable in-memory
//! backend; the only sleeps are the helper's own jittered backoff, which is
//! capped at 50ms so the storm test stays fast and deterministic.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use ironclaw_host_api::{
    MountAlias, MountGrant, MountPermissions, MountView, ResourceScope, ScopedPath, VirtualPath,
};
use serde::{Deserialize, Serialize};

use super::{CasApply, CasUpdateError, cas_update};
use crate::{
    BackendCapabilities, CasExpectation, ContentType, DirEntry, Entry, FileStat, FilesystemError,
    InMemoryBackend, RecordVersion, RootFilesystem, ScopedFilesystem, VersionedEntry,
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
}

impl NonCasBackend {
    fn new() -> Self {
        Self {
            inner: InMemoryBackend::new(),
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
        // Blind overwrite: ignore the caller's CAS expectation entirely.
        self.inner.put(path, entry, CasExpectation::Any).await
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

// ─── Tests ──────────────────────────────────────────────────────────────────

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
