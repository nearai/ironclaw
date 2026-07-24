//! Fault-injecting [`RootFilesystem`] decorator for tests (`test-support`).
//!
//! # Why this exists
//!
//! A store's fault-handling used to be tested by hand-writing a fake that
//! implements the *whole store trait* (`SecretStorePort`, `ProcessStorePort`, …) and
//! returns an error on the Nth call. That fake never runs a single line of the
//! production `Filesystem*Store` — its encryption, path building, CAS retry,
//! and `FilesystemError -> DomainError` mapping are all bypassed. The test
//! proves the fake returns what it was told to; it proves nothing about
//! production.
//!
//! [`FaultInjecting`] moves the fault *below* the store: it wraps the real
//! backend ([`InMemoryBackend`](crate::InMemoryBackend), typically), and the
//! consumer builds its genuine `Filesystem*Store` over it. Every op now runs
//! the real store code end-to-end, and the injected [`FilesystemError`] flows
//! through the store's actual error-mapping path — which the store fake could
//! only assume.
//!
//! It also *records* every gated op, so it subsumes the "recording"/"counting"
//! store fakes: assert on [`FaultInjecting::recorded`] instead of a bespoke
//! `Mutex<Vec<Handle>>` inside a fake.
//!
//! # Example
//!
//! ```ignore
//! use ironclaw_filesystem::{FaultInjecting, Fault, InMemoryBackend, FilesystemOperation};
//! use std::sync::Arc;
//!
//! // Fail the 2nd entry-plane write to any path; record all ops.
//! let backend = Arc::new(
//!     FaultInjecting::new(InMemoryBackend::new())
//!         .with_fault(Fault::on(FilesystemOperation::WriteFile).nth(2).backend("boom")),
//! );
//! let store = MyStore::over(backend.clone()); // real production store
//! // ... drive `store`; the real store surfaces its mapped domain error ...
//! assert_eq!(backend.count(FilesystemOperation::WriteFile), 2);
//! ```

use std::sync::Mutex;

use async_trait::async_trait;
use ironclaw_host_api::VirtualPath;

use crate::backend::{EventRecord, StorageTxn};
use crate::{
    BackendCapabilities, CasExpectation, DirEntry, Entry, FileStat, FilesystemError,
    FilesystemOperation, Filter, IndexSpec, Page, RecordVersion, RootFilesystem, SeqNo,
    VersionedEntry,
};

/// Which [`FilesystemError`] a matched [`Fault`] returns. Constructed with the
/// intercepted path + operation at fire time, so the error carries the real
/// call context (mirroring what a genuine backend failure would report).
#[derive(Debug, Clone)]
pub enum FaultKind {
    /// [`FilesystemError::Backend`] — a generic backend I/O failure carrying
    /// `reason`. The common case: "the write/read hit the storage layer and it
    /// failed."
    Backend(String),
    /// [`FilesystemError::BackendBusy`] — a retryable contention outcome that
    /// did not commit.
    BackendBusy,
    /// [`FilesystemError::NotFound`] for the op's path.
    NotFound,
    /// [`FilesystemError::Unsupported`] for the op's path.
    Unsupported,
}

impl FaultKind {
    fn into_error(self, path: &VirtualPath, operation: FilesystemOperation) -> FilesystemError {
        let path = path.clone();
        match self {
            FaultKind::Backend(reason) => FilesystemError::Backend {
                path,
                operation,
                reason,
            },
            FaultKind::BackendBusy => FilesystemError::BackendBusy { path, operation },
            FaultKind::NotFound => FilesystemError::NotFound { path, operation },
            FaultKind::Unsupported => FilesystemError::Unsupported { path, operation },
        }
    }
}

/// How many matching occurrences a [`Fault`] fires on.
#[derive(Debug, Clone, Copy)]
enum Trigger {
    /// Every matching op fails.
    Always,
    /// Only the `n`th matching op fails (1-indexed); the rest pass through.
    Nth(usize),
}

/// Which paths a [`Fault`] applies to.
#[derive(Debug, Clone)]
enum PathMatch {
    /// Any path.
    Any,
    /// Paths whose string representation contains this substring. Deliberately
    /// a plain `contains` (not a glob) — tenant-rewriting mounts turn `/secrets`
    /// into `/tenants/<t>/users/<u>/secrets/...`, so a substring like `secrets`
    /// is the robust, mount-shape-independent target.
    Contains(String),
}

/// A single fault rule: fail `operation`s matching `path` on `trigger`,
/// returning `kind`. Build with [`Fault::on`] and the fluent setters.
#[derive(Debug, Clone)]
pub struct Fault {
    operation: FilesystemOperation,
    path: PathMatch,
    trigger: Trigger,
    kind: FaultKind,
}

impl Fault {
    /// Start a rule targeting `operation`. Defaults: any path, fires always,
    /// returns a `Backend` error. Refine with [`Self::nth`], [`Self::path`],
    /// and a `kind` setter.
    pub fn on(operation: FilesystemOperation) -> Self {
        Self {
            operation,
            path: PathMatch::Any,
            trigger: Trigger::Always,
            kind: FaultKind::Backend("injected fault".to_string()),
        }
    }

    /// Fire only on the `n`th matching occurrence (1-indexed).
    pub fn nth(mut self, n: usize) -> Self {
        self.trigger = Trigger::Nth(n);
        self
    }

    /// Restrict to paths whose string contains `needle`.
    pub fn path(mut self, needle: impl Into<String>) -> Self {
        self.path = PathMatch::Contains(needle.into());
        self
    }

    /// Return a [`FilesystemError::Backend`] with `reason`.
    pub fn backend(mut self, reason: impl Into<String>) -> Self {
        self.kind = FaultKind::Backend(reason.into());
        self
    }

    /// Return the given [`FaultKind`].
    pub fn returning(mut self, kind: FaultKind) -> Self {
        self.kind = kind;
        self
    }

    fn matches(&self, operation: FilesystemOperation, path: &VirtualPath) -> bool {
        if self.operation != operation {
            return false;
        }
        match &self.path {
            PathMatch::Any => true,
            PathMatch::Contains(needle) => path.as_str().contains(needle.as_str()),
        }
    }
}

/// A recorded gated operation (op + path), in call order. Exposed by
/// [`FaultInjecting::recorded`] so tests assert on the real backend traffic the
/// store produced instead of a bespoke recording fake.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedOp {
    pub operation: FilesystemOperation,
    pub path: VirtualPath,
}

struct FaultState {
    rules: Vec<(Fault, usize)>,
    recorded: Vec<RecordedOp>,
}

/// Wraps an inner [`RootFilesystem`], injecting configured [`Fault`]s and
/// recording every gated op. See the [module docs](self).
pub struct FaultInjecting<F> {
    inner: F,
    state: Mutex<FaultState>,
}

impl<F> std::fmt::Debug for FaultInjecting<F> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let recorded = self
            .state
            .lock()
            .map(|s| s.recorded.len())
            .unwrap_or_default();
        f.debug_struct("FaultInjecting")
            .field("recorded_ops", &recorded)
            .finish_non_exhaustive()
    }
}

impl<F> FaultInjecting<F> {
    /// Wrap `inner` with no faults configured (a pure recorder until faults are
    /// added).
    pub fn new(inner: F) -> Self {
        Self {
            inner,
            state: Mutex::new(FaultState {
                rules: Vec::new(),
                recorded: Vec::new(),
            }),
        }
    }

    /// Add a [`Fault`], consuming and returning `self` for builder-style setup.
    pub fn with_fault(self, fault: Fault) -> Self {
        self.add_fault(fault);
        self
    }

    /// Add a [`Fault`] after construction (e.g. through a shared `Arc` handle,
    /// mid-test). Interior-mutable so a wrapped backend can be re-armed without
    /// unwrapping the `Arc`.
    pub fn add_fault(&self, fault: Fault) {
        self.lock().rules.push((fault, 0));
    }

    /// All gated ops recorded so far, in call order.
    pub fn recorded(&self) -> Vec<RecordedOp> {
        self.lock().recorded.clone()
    }

    /// Paths of recorded ops matching `operation`, in call order.
    pub fn recorded_paths(&self, operation: FilesystemOperation) -> Vec<VirtualPath> {
        self.lock()
            .recorded
            .iter()
            .filter(|op| op.operation == operation)
            .map(|op| op.path.clone())
            .collect()
    }

    /// Count of recorded ops matching `operation`.
    pub fn count(&self, operation: FilesystemOperation) -> usize {
        self.lock()
            .recorded
            .iter()
            .filter(|op| op.operation == operation)
            .count()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, FaultState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Record the op, then return the injected error if a rule fires. Called at
    /// the top of every gated [`RootFilesystem`] method.
    fn gate(
        &self,
        operation: FilesystemOperation,
        path: &VirtualPath,
    ) -> Result<(), FilesystemError> {
        let mut state = self.lock();
        state.recorded.push(RecordedOp {
            operation,
            path: path.clone(),
        });
        for (fault, seen) in state.rules.iter_mut() {
            if !fault.matches(operation, path) {
                continue;
            }
            *seen += 1;
            let fire = match fault.trigger {
                Trigger::Always => true,
                Trigger::Nth(n) => *seen == n,
            };
            if fire {
                return Err(fault.kind.clone().into_error(path, operation));
            }
        }
        Ok(())
    }
}

#[async_trait]
impl<F> RootFilesystem for FaultInjecting<F>
where
    F: RootFilesystem,
{
    fn capabilities(&self) -> BackendCapabilities {
        self.inner.capabilities()
    }

    async fn put(
        &self,
        path: &VirtualPath,
        entry: Entry,
        cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError> {
        self.gate(FilesystemOperation::WriteFile, path)?;
        self.inner.put(path, entry, cas).await
    }

    async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        self.gate(FilesystemOperation::ReadFile, path)?;
        self.inner.get(path).await
    }

    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        self.gate(FilesystemOperation::ListDir, path)?;
        self.inner.list_dir(path).await
    }

    async fn query(
        &self,
        path: &VirtualPath,
        filter: &Filter,
        page: Page,
    ) -> Result<Vec<VersionedEntry>, FilesystemError> {
        self.gate(FilesystemOperation::Query, path)?;
        self.inner.query(path, filter, page).await
    }

    async fn ensure_index(
        &self,
        path: &VirtualPath,
        spec: &IndexSpec,
    ) -> Result<(), FilesystemError> {
        self.gate(FilesystemOperation::EnsureIndex, path)?;
        self.inner.ensure_index(path, spec).await
    }

    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        self.gate(FilesystemOperation::Stat, path)?;
        self.inner.stat(path).await
    }

    async fn delete(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.gate(FilesystemOperation::Delete, path)?;
        self.inner.delete(path).await
    }

    async fn delete_if_version(
        &self,
        path: &VirtualPath,
        expected_version: RecordVersion,
    ) -> Result<(), FilesystemError> {
        self.gate(FilesystemOperation::Delete, path)?;
        self.inner.delete_if_version(path, expected_version).await
    }

    async fn begin(&self, path: &VirtualPath) -> Result<Box<dyn StorageTxn>, FilesystemError> {
        self.gate(FilesystemOperation::BeginTxn, path)?;
        self.inner.begin(path).await
    }

    async fn append(&self, path: &VirtualPath, payload: Vec<u8>) -> Result<SeqNo, FilesystemError> {
        self.gate(FilesystemOperation::Append, path)?;
        self.inner.append(path, payload).await
    }

    async fn append_batch(
        &self,
        path: &VirtualPath,
        payloads: Vec<Vec<u8>>,
    ) -> Result<Vec<SeqNo>, FilesystemError> {
        self.gate(FilesystemOperation::Append, path)?;
        self.inner.append_batch(path, payloads).await
    }

    async fn tail(
        &self,
        path: &VirtualPath,
        from: SeqNo,
    ) -> Result<Vec<EventRecord>, FilesystemError> {
        self.gate(FilesystemOperation::Tail, path)?;
        self.inner.tail(path, from).await
    }

    async fn head_seq(
        &self,
        path: &VirtualPath,
        from: SeqNo,
    ) -> Result<Option<SeqNo>, FilesystemError> {
        self.gate(FilesystemOperation::HeadSeq, path)?;
        self.inner.head_seq(path, from).await
    }

    async fn reserve_sequence(&self, path: &VirtualPath) -> Result<SeqNo, FilesystemError> {
        self.gate(FilesystemOperation::ReserveSeq, path)?;
        self.inner.reserve_sequence(path).await
    }

    // ─── Legacy bytes plane ───────────────────────────────────────────────
    //
    // These have *non-delegating* trait defaults that return `Unsupported`
    // unconditionally (unlike `read_file`/`write_file`/`*_bounded`, whose
    // defaults route through the forwarded `get`/`put`/`list_dir`/`tail`
    // primitives and therefore already reach the inner backend). If the
    // decorator did not override them, a call would hit the trait default and
    // report `Unsupported` even when the wrapped backend supports the op —
    // masking real behavior. Forward them with the same fault gating the other
    // ops use.

    async fn append_file(&self, path: &VirtualPath, bytes: &[u8]) -> Result<(), FilesystemError> {
        self.gate(FilesystemOperation::AppendFile, path)?;
        self.inner.append_file(path, bytes).await
    }

    async fn create_dir_all(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.gate(FilesystemOperation::CreateDirAll, path)?;
        self.inner.create_dir_all(path).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DiskFilesystem, InMemoryBackend};
    use ironclaw_host_api::HostPath;

    fn path(p: &str) -> VirtualPath {
        VirtualPath::new(p).unwrap()
    }

    #[tokio::test]
    async fn nth_fault_fires_once_then_passes_through() {
        let fs = FaultInjecting::new(InMemoryBackend::new()).with_fault(
            Fault::on(FilesystemOperation::WriteFile)
                .nth(2)
                .backend("boom"),
        );

        // First write succeeds, reaching the real backend.
        fs.put(
            &path("/projects/1"),
            Entry::bytes(b"one".to_vec()),
            CasExpectation::Any,
        )
        .await
        .expect("first put succeeds");

        // Second write is faulted with the configured Backend error + context.
        let err = fs
            .put(
                &path("/projects/2"),
                Entry::bytes(b"two".to_vec()),
                CasExpectation::Any,
            )
            .await
            .expect_err("second put is faulted");
        assert!(matches!(
            err,
            FilesystemError::Backend { operation: FilesystemOperation::WriteFile, ref reason, .. }
                if reason == "boom"
        ));

        // Third write passes through again — Nth is a one-shot.
        fs.put(
            &path("/projects/3"),
            Entry::bytes(b"three".to_vec()),
            CasExpectation::Any,
        )
        .await
        .expect("third put succeeds");

        // The faulted write did not reach the backend: /projects/2 is absent.
        assert!(fs.get(&path("/projects/2")).await.unwrap().is_none());
        assert!(fs.get(&path("/projects/1")).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn records_gated_ops_in_call_order() {
        let fs = FaultInjecting::new(InMemoryBackend::new());
        fs.put(
            &path("/projects/x"),
            Entry::bytes(b"v".to_vec()),
            CasExpectation::Any,
        )
        .await
        .unwrap();
        let _ = fs.get(&path("/projects/x")).await.unwrap();
        fs.delete(&path("/projects/x")).await.unwrap();

        assert_eq!(fs.count(FilesystemOperation::WriteFile), 1);
        assert_eq!(fs.count(FilesystemOperation::ReadFile), 1);
        assert_eq!(
            fs.recorded_paths(FilesystemOperation::Delete),
            vec![path("/projects/x")]
        );
        assert_eq!(fs.recorded().len(), 3);
    }

    #[tokio::test]
    async fn path_filter_scopes_the_fault() {
        let fs = FaultInjecting::new(InMemoryBackend::new()).with_fault(
            Fault::on(FilesystemOperation::WriteFile)
                .path("secrets")
                .backend("no secrets"),
        );

        // A non-matching path is untouched.
        fs.put(
            &path("/projects/a"),
            Entry::bytes(b"a".to_vec()),
            CasExpectation::Any,
        )
        .await
        .expect("non-secrets write passes");

        // A matching path is faulted (Always trigger — every match fails).
        let err = fs
            .put(
                &path("/tenants/t/users/u/secrets/k"),
                Entry::bytes(b"s".to_vec()),
                CasExpectation::Any,
            )
            .await
            .expect_err("secrets write is faulted");
        assert!(matches!(err, FilesystemError::Backend { .. }));
    }

    /// The legacy bytes-plane methods `append_file` / `create_dir_all` have
    /// non-delegating trait defaults (they return `Unsupported` outright rather
    /// than routing through a forwarded primitive). The decorator must override
    /// them to reach the inner backend, or a supporting backend's op is masked
    /// as `Unsupported`. Uses a real `DiskFilesystem` — which supports both —
    /// so this pins genuine forwarding, gating, and recording end-to-end.
    #[tokio::test]
    async fn forwards_legacy_bytes_plane_to_supporting_inner() {
        let storage = tempfile::tempdir().unwrap();
        let mut disk = DiskFilesystem::new();
        disk.mount_local(
            path("/projects"),
            HostPath::from_path_buf(storage.path().to_path_buf()),
        )
        .unwrap();
        let fs = FaultInjecting::new(disk);

        // create_dir_all forwards to the inner backend and succeeds — not the
        // trait default's `Unsupported`.
        fs.create_dir_all(&path("/projects/nested"))
            .await
            .expect("create_dir_all forwards to a supporting inner backend");

        // append_file forwards too; two appends concatenate on the real file.
        fs.append_file(&path("/projects/nested/log"), b"one")
            .await
            .expect("first append_file forwards");
        fs.append_file(&path("/projects/nested/log"), b"two")
            .await
            .expect("second append_file forwards");
        assert_eq!(
            fs.read_file(&path("/projects/nested/log")).await.unwrap(),
            b"onetwo".to_vec()
        );

        // Both ops were gated and recorded, like every other forwarded op.
        assert_eq!(fs.count(FilesystemOperation::CreateDirAll), 1);
        assert_eq!(fs.count(FilesystemOperation::AppendFile), 2);

        // The gate applies to these ops too: an injected fault fires.
        let faulted = FaultInjecting::new(DiskFilesystem::new())
            .with_fault(Fault::on(FilesystemOperation::CreateDirAll).backend("no dirs"));
        let err = faulted
            .create_dir_all(&path("/projects/x"))
            .await
            .expect_err("create_dir_all is faulted");
        assert!(matches!(
            err,
            FilesystemError::Backend {
                operation: FilesystemOperation::CreateDirAll,
                ..
            }
        ));
    }
}
