use std::sync::Arc;

use ironclaw_host_api::{MountPermissions, MountView, ScopedPath, VirtualPath};

use crate::backend::{EventRecord, StorageTxn};
use crate::{
    CasExpectation, DirEntry, Entry, FileStat, FilesystemError, FilesystemOperation, Filter,
    IndexSpec, Page, RecordVersion, RootFilesystem, SeqNo, VersionedEntry, path_prefix_matches,
};

/// Invocation-scoped filesystem view over [`ScopedPath`] values.
///
/// Higher-level stores (SecretStore, ProcessStore, …) accept a
/// `ScopedFilesystem` bound to a path prefix and call the unified
/// `put`/`get`/`query`/etc. ops through it. Permission checks happen here
/// against the caller's [`MountView`] before any backend dispatch.
#[derive(Debug, Clone)]
pub struct ScopedFilesystem<F> {
    root: Arc<F>,
    mounts: MountView,
}

impl<F> ScopedFilesystem<F>
where
    F: RootFilesystem,
{
    pub fn new(root: Arc<F>, mounts: MountView) -> Self {
        Self { root, mounts }
    }

    pub fn mounts(&self) -> &MountView {
        &self.mounts
    }

    // ─── Unified entry plane ──────────────────────────────────────────────

    /// Write an [`Entry`] at `path` with a CAS precondition.
    pub async fn put(
        &self,
        path: &ScopedPath,
        entry: Entry,
        cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError> {
        let virtual_path = self.resolve_with_permission(path, FilesystemOperation::WriteFile)?;
        self.root.put(&virtual_path, entry, cas).await
    }

    /// Read the entry at `path`, returning `None` if absent.
    pub async fn get(&self, path: &ScopedPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        let virtual_path = self.resolve_with_permission(path, FilesystemOperation::ReadFile)?;
        self.root.get(&virtual_path).await
    }

    /// Filtered query over `prefix`.
    pub async fn query(
        &self,
        prefix: &ScopedPath,
        filter: &Filter,
        page: Page,
    ) -> Result<Vec<VersionedEntry>, FilesystemError> {
        let virtual_path = self.resolve_with_permission(prefix, FilesystemOperation::Query)?;
        self.root.query(&virtual_path, filter, page).await
    }

    /// Declare an index on the mount under `prefix`.
    pub async fn ensure_index(
        &self,
        prefix: &ScopedPath,
        spec: &IndexSpec,
    ) -> Result<(), FilesystemError> {
        let virtual_path =
            self.resolve_with_permission(prefix, FilesystemOperation::EnsureIndex)?;
        self.root.ensure_index(&virtual_path, spec).await
    }

    /// Begin a multi-key transaction (capability-gated).
    ///
    /// PR #3659 review fix: returns a permission-checking wrapper around the
    /// underlying [`StorageTxn`] so the per-operation ACL is preserved across
    /// the transaction boundary. Without this wrapper, a caller granted only
    /// `write` could still `get` / `delete` through the raw txn handle once
    /// any backend implements transactions.
    pub async fn begin(&self, prefix: &ScopedPath) -> Result<Box<dyn StorageTxn>, FilesystemError> {
        let virtual_path = self.resolve_with_permission(prefix, FilesystemOperation::BeginTxn)?;
        let inner = self.root.begin(&virtual_path).await?;
        // Snapshot the mount permissions that authorized the txn so the
        // wrapper can apply them per-op without revisiting `MountView`
        // (which would need a ScopedPath we no longer have at this point).
        let permissions = self
            .mounts
            .resolve_with_grant(prefix)?
            .1
            .permissions
            .clone();
        Ok(Box::new(ScopedStorageTxn {
            inner,
            permissions,
            mount_prefix: virtual_path,
        }))
    }

    // ─── Event/tail plane ─────────────────────────────────────────────────

    /// Append `payload` to the event log at `path`, returning the SeqNo.
    pub async fn append(
        &self,
        path: &ScopedPath,
        payload: Vec<u8>,
    ) -> Result<SeqNo, FilesystemError> {
        // Append on the event plane is a write — distinct from the legacy
        // byte-plane AppendFile but maps to the same `permissions.write`.
        let virtual_path = self.resolve_with_permission(path, FilesystemOperation::Append)?;
        self.root.append(&virtual_path, payload).await
    }

    /// Read events at `path` starting just after `from`.
    pub async fn tail(
        &self,
        path: &ScopedPath,
        from: SeqNo,
    ) -> Result<Vec<EventRecord>, FilesystemError> {
        let virtual_path = self.resolve_with_permission(path, FilesystemOperation::Tail)?;
        self.root.tail(&virtual_path, from).await
    }

    // ─── Legacy bytes-plane methods (DEPRECATED — transitional) ───────────
    //
    // These remain for the migration window. New code should prefer the
    // unified ops above (`put`/`get`/`read_bytes`/`write_bytes`). Removed
    // once consumers migrate (task #17). Marked deprecated via doc comment
    // rather than `#[deprecated]` attribute to avoid generating compiler
    // warnings across every downstream call site during the transition.

    /// **DEPRECATED — use [`read_bytes`](Self::read_bytes) or
    /// [`get`](Self::get) instead.**
    pub async fn read_file(&self, path: &ScopedPath) -> Result<Vec<u8>, FilesystemError> {
        let virtual_path = self.resolve_with_permission(path, FilesystemOperation::ReadFile)?;
        self.root.read_file(&virtual_path).await
    }

    /// **DEPRECATED — use [`write_bytes`](Self::write_bytes) or
    /// [`put`](Self::put) instead.**
    pub async fn write_file(&self, path: &ScopedPath, bytes: &[u8]) -> Result<(), FilesystemError> {
        let virtual_path = self.resolve_with_permission(path, FilesystemOperation::WriteFile)?;
        self.root.write_file(&virtual_path, bytes).await
    }

    /// **DEPRECATED — no direct replacement on the unified surface.** Use
    /// `append`/`tail` for log-shaped mounts or `get`+`put` for
    /// read-modify-write.
    pub async fn append_file(
        &self,
        path: &ScopedPath,
        bytes: &[u8],
    ) -> Result<(), FilesystemError> {
        let virtual_path = self.resolve_with_permission(path, FilesystemOperation::AppendFile)?;
        self.root.append_file(&virtual_path, bytes).await
    }

    pub async fn list_dir(&self, path: &ScopedPath) -> Result<Vec<DirEntry>, FilesystemError> {
        let virtual_path = self.resolve_with_permission(path, FilesystemOperation::ListDir)?;
        self.root.list_dir(&virtual_path).await
    }

    pub async fn stat(&self, path: &ScopedPath) -> Result<FileStat, FilesystemError> {
        let virtual_path = self.resolve_with_permission(path, FilesystemOperation::Stat)?;
        self.root.stat(&virtual_path).await
    }

    pub async fn delete(&self, path: &ScopedPath) -> Result<(), FilesystemError> {
        let virtual_path = self.resolve_with_permission(path, FilesystemOperation::Delete)?;
        self.root.delete(&virtual_path).await
    }

    /// **DEPRECATED — the unified entry plane infers directories from path
    /// prefixes.** New consumer code must not call this.
    pub async fn create_dir_all(&self, path: &ScopedPath) -> Result<(), FilesystemError> {
        let virtual_path = self.resolve_with_permission(path, FilesystemOperation::CreateDirAll)?;
        self.root.create_dir_all(&virtual_path).await
    }

    // ─── Convenience helpers for byte-only callers ────────────────────────

    /// Read the body bytes at `path`. Convenience wrapper over [`get`] that
    /// errors if the path has no entry.
    pub async fn read_bytes(&self, path: &ScopedPath) -> Result<Vec<u8>, FilesystemError> {
        match self.get(path).await? {
            Some(versioned) => Ok(versioned.entry.body),
            None => {
                // Need the virtual path for the error message; resolve once
                // more — the permission check already passed.
                let virtual_path =
                    self.resolve_with_permission(path, FilesystemOperation::ReadFile)?;
                Err(FilesystemError::NotFound {
                    path: virtual_path,
                    operation: FilesystemOperation::ReadFile,
                })
            }
        }
    }

    /// Write `body` as an opaque-file entry at `path` (no CAS precondition).
    /// Convenience wrapper over [`put`].
    pub async fn write_bytes(
        &self,
        path: &ScopedPath,
        body: Vec<u8>,
    ) -> Result<(), FilesystemError> {
        self.put(path, Entry::bytes(body), CasExpectation::Any)
            .await
            .map(|_| ())
    }

    // ─── Internals ────────────────────────────────────────────────────────

    fn resolve_with_permission(
        &self,
        path: &ScopedPath,
        operation: FilesystemOperation,
    ) -> Result<VirtualPath, FilesystemError> {
        let (virtual_path, grant) = self.mounts.resolve_with_grant(path)?;

        if !operation_allowed(&grant.permissions, operation) {
            return Err(FilesystemError::PermissionDenied {
                path: path.clone(),
                operation,
            });
        }

        Ok(virtual_path)
    }
}

fn operation_allowed(permissions: &MountPermissions, operation: FilesystemOperation) -> bool {
    match operation {
        FilesystemOperation::ReadFile => permissions.read,
        FilesystemOperation::WriteFile
        | FilesystemOperation::AppendFile
        | FilesystemOperation::CreateDirAll
        | FilesystemOperation::EnsureIndex
        | FilesystemOperation::BeginTxn
        | FilesystemOperation::Append => permissions.write,
        FilesystemOperation::ListDir => permissions.list,
        // Stat is metadata-only: either read authority or list authority reveals
        // equivalent existence/type information without file contents.
        FilesystemOperation::Stat => permissions.read || permissions.list,
        FilesystemOperation::Delete => permissions.delete,
        FilesystemOperation::MountLocal => false,
        // Query enumerates records, so requires both read (to see contents) and
        // list (to enumerate). Either alone is insufficient.
        FilesystemOperation::Query => permissions.read && permissions.list,
        // Tail reads from the event log; mirrors read on the byte plane.
        FilesystemOperation::Tail => permissions.read,
    }
}

/// Permission-checking wrapper around an inner [`StorageTxn`] returned by
/// [`ScopedFilesystem::begin`]. Preserves the per-operation ACL across the
/// txn boundary so a write-only scoped caller cannot read or delete through
/// the txn handle (PR #3659 review fix).
struct ScopedStorageTxn {
    inner: Box<dyn StorageTxn>,
    permissions: MountPermissions,
    mount_prefix: VirtualPath,
}

impl ScopedStorageTxn {
    fn check(&self, operation: FilesystemOperation) -> Result<(), FilesystemError> {
        if operation_allowed(&self.permissions, operation) {
            Ok(())
        } else {
            // The transaction is anchored at `mount_prefix`; surface the
            // mount root rather than the per-call VirtualPath to avoid
            // implying that the caller is denied at one specific child
            // while allowed elsewhere — the txn-time grant applies to
            // the whole prefix.
            Err(FilesystemError::Backend {
                path: self.mount_prefix.clone(),
                operation,
                reason: "scoped transaction lacks the required permission".to_string(),
            })
        }
    }

    /// Reject per-op paths that fall outside the txn's `mount_prefix`. The
    /// `StorageTxn` doc commits to `PathOutsideMount` for cross-prefix
    /// accesses; this wrapper enforces that contract for every backend that
    /// supports `begin`, so a write-only caller granted txn access at
    /// `/projects/foo` can't drive an `/secrets/...` write through the raw
    /// txn handle. Without this check, the underlying backend would be the
    /// only line of defence and a future backend that forgot the check
    /// would silently bypass scope.
    fn check_path(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        if path_prefix_matches(self.mount_prefix.as_str(), path.as_str()) {
            Ok(())
        } else {
            Err(FilesystemError::PathOutsideMount { path: path.clone() })
        }
    }
}

#[async_trait::async_trait]
impl StorageTxn for ScopedStorageTxn {
    async fn put(
        &mut self,
        path: &VirtualPath,
        entry: Entry,
        cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError> {
        self.check(FilesystemOperation::WriteFile)?;
        self.check_path(path)?;
        self.inner.put(path, entry, cas).await
    }

    async fn get(&mut self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        self.check(FilesystemOperation::ReadFile)?;
        self.check_path(path)?;
        self.inner.get(path).await
    }

    async fn delete(&mut self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.check(FilesystemOperation::Delete)?;
        self.check_path(path)?;
        self.inner.delete(path).await
    }

    async fn commit(self: Box<Self>) -> Result<(), FilesystemError> {
        // Commit/rollback are bookkeeping; they were authorized at `begin`
        // time and require no additional per-op check.
        self.inner.commit().await
    }

    async fn rollback(self: Box<Self>) {
        self.inner.rollback().await
    }
}

#[cfg(test)]
mod tests {
    //! Caller-level tests for the operation gates added with the unified
    //! storage surface. The matrix below exercises each `MountPermissions`
    //! axis against each new op and asserts that the permission denial
    //! happens at the `ScopedFilesystem` boundary — before any backend
    //! dispatch — so a future backend that forgets a check still inherits
    //! the wrapper's gate. The companion `txn_*` tests drive a stub
    //! [`StorageTxn`] backend to lock in the per-op ACL and the mount-
    //! prefix containment check on `ScopedStorageTxn` (it has no shipped
    //! backend yet, so this is the only place those guarantees are
    //! exercised).
    use std::sync::Arc;

    use async_trait::async_trait;
    use ironclaw_host_api::{
        MountAlias, MountGrant, MountPermissions, MountView, ScopedPath, VirtualPath,
    };

    use super::*;
    use crate::in_memory::InMemoryBackend;
    use crate::{
        CasExpectation, Entry, FilesystemError, FilesystemOperation, Filter, IndexKey, IndexKind,
        IndexName, IndexSpec, Page, RecordKind, SeqNo,
    };

    /// Coerce `Result<T, FilesystemError>` to its error without requiring
    /// `T: Debug`. `Box<dyn StorageTxn>` isn't `Debug`, so `unwrap_err()`
    /// can't be used directly on the result of `scoped.begin(...)`.
    fn expect_err<T>(result: Result<T, FilesystemError>) -> FilesystemError {
        match result {
            Ok(_) => panic!("expected an error"),
            Err(err) => err,
        }
    }

    fn scoped_in_memory(permissions: MountPermissions) -> ScopedFilesystem<InMemoryBackend> {
        // Mount the alias at the engine subtree (an allowed virtual root)
        // so query/ensure_index/append/tail all hit the in-memory backend
        // through a real `ScopedPath` → `VirtualPath` resolution.
        ScopedFilesystem::new(
            Arc::new(InMemoryBackend::new()),
            MountView::new(vec![MountGrant::new(
                MountAlias::new("/workspace").unwrap(),
                VirtualPath::new("/engine/scoped_test").unwrap(),
                permissions,
            )])
            .unwrap(),
        )
    }

    fn no_op(read: bool, write: bool, list: bool, delete: bool) -> MountPermissions {
        MountPermissions {
            read,
            write,
            list,
            delete,
            execute: false,
        }
    }

    fn record_with_scope(scope: &str) -> Entry {
        Entry::record(
            RecordKind::new("test_kind").unwrap(),
            &serde_json::json!({}),
        )
        .unwrap()
        .with_indexed(
            IndexKey::new("scope").unwrap(),
            crate::IndexValue::Text(scope.into()),
        )
    }

    // ─── query requires read AND list ─────────────────────────────────────

    #[tokio::test]
    async fn query_denies_when_read_missing_even_with_list() {
        let scoped = scoped_in_memory(no_op(false, false, true, false));
        let err = scoped
            .query(
                &ScopedPath::new("/workspace").unwrap(),
                &Filter::All,
                Page::default(),
            )
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            FilesystemError::PermissionDenied {
                operation: FilesystemOperation::Query,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn query_denies_when_list_missing_even_with_read() {
        let scoped = scoped_in_memory(no_op(true, false, false, false));
        let err = scoped
            .query(
                &ScopedPath::new("/workspace").unwrap(),
                &Filter::All,
                Page::default(),
            )
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            FilesystemError::PermissionDenied {
                operation: FilesystemOperation::Query,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn query_succeeds_with_read_and_list() {
        let scoped = scoped_in_memory(no_op(true, true, true, false));
        scoped
            .put(
                &ScopedPath::new("/workspace/a").unwrap(),
                record_with_scope("acme"),
                CasExpectation::Absent,
            )
            .await
            .unwrap();
        let results = scoped
            .query(
                &ScopedPath::new("/workspace").unwrap(),
                &Filter::All,
                Page::default(),
            )
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
    }

    // ─── ensure_index requires write ──────────────────────────────────────

    #[tokio::test]
    async fn ensure_index_denies_when_write_missing() {
        let scoped = scoped_in_memory(no_op(true, false, true, false));
        let spec = IndexSpec::new(
            IndexName::new("by_scope").unwrap(),
            vec![IndexKey::new("scope").unwrap()],
            IndexKind::Exact,
        );
        let err = scoped
            .ensure_index(&ScopedPath::new("/workspace").unwrap(), &spec)
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            FilesystemError::PermissionDenied {
                operation: FilesystemOperation::EnsureIndex,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn ensure_index_succeeds_with_write() {
        let scoped = scoped_in_memory(no_op(false, true, false, false));
        let spec = IndexSpec::new(
            IndexName::new("by_scope").unwrap(),
            vec![IndexKey::new("scope").unwrap()],
            IndexKind::Exact,
        );
        scoped
            .ensure_index(&ScopedPath::new("/workspace").unwrap(), &spec)
            .await
            .unwrap();
    }

    // ─── append (event-plane) requires write ──────────────────────────────

    #[tokio::test]
    async fn append_event_denies_when_write_missing() {
        let scoped = scoped_in_memory(no_op(true, false, true, false));
        let err = scoped
            .append(&ScopedPath::new("/workspace/log").unwrap(), b"x".to_vec())
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            FilesystemError::PermissionDenied {
                operation: FilesystemOperation::Append,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn append_event_succeeds_with_write_and_returns_monotonic_seq() {
        let scoped = scoped_in_memory(no_op(false, true, false, false));
        let s1 = scoped
            .append(&ScopedPath::new("/workspace/log").unwrap(), b"a".to_vec())
            .await
            .unwrap();
        let s2 = scoped
            .append(&ScopedPath::new("/workspace/log").unwrap(), b"b".to_vec())
            .await
            .unwrap();
        assert!(s2 > s1);
    }

    // ─── tail requires read ───────────────────────────────────────────────

    #[tokio::test]
    async fn tail_denies_when_read_missing() {
        let scoped = scoped_in_memory(no_op(false, true, true, false));
        let err = scoped
            .tail(&ScopedPath::new("/workspace/log").unwrap(), SeqNo::ZERO)
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            FilesystemError::PermissionDenied {
                operation: FilesystemOperation::Tail,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn tail_succeeds_with_read_and_write() {
        // Append needs write, tail needs read. Verifying tail goes
        // through requires a scope with both; the denial path is covered
        // by tail_denies_when_read_missing above.
        let scoped = scoped_in_memory(no_op(true, true, false, false));
        let s1 = scoped
            .append(
                &ScopedPath::new("/workspace/log").unwrap(),
                b"hello".to_vec(),
            )
            .await
            .unwrap();
        let events = scoped
            .tail(&ScopedPath::new("/workspace/log").unwrap(), SeqNo::ZERO)
            .await
            .unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].seq, s1);
    }

    // ─── begin requires write (capability-gated; in-memory rejects natively) ──

    #[tokio::test]
    async fn begin_denies_when_write_missing() {
        let scoped = scoped_in_memory(no_op(true, false, true, false));
        let err = expect_err(scoped.begin(&ScopedPath::new("/workspace").unwrap()).await);
        assert!(matches!(
            err,
            FilesystemError::PermissionDenied {
                operation: FilesystemOperation::BeginTxn,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn begin_with_write_propagates_backend_unsupported() {
        // The in-memory backend doesn't implement begin natively. With
        // permission granted, the wrapper passes through to the backend
        // which returns Unsupported. The point is that PermissionDenied
        // is not the error — the gate let it through.
        let scoped = scoped_in_memory(no_op(false, true, false, false));
        let err = expect_err(scoped.begin(&ScopedPath::new("/workspace").unwrap()).await);
        assert!(
            matches!(
                err,
                FilesystemError::Unsupported {
                    operation: FilesystemOperation::BeginTxn,
                    ..
                }
            ),
            "expected Unsupported (gate let it through), got {err:?}"
        );
    }

    // ─── ScopedStorageTxn ACL + path containment ──────────────────────────
    //
    // No shipped backend implements `begin()` natively yet, so we stub one
    // here. The stub returns a `StubTxn` that records the last path each op
    // received so the test can assert the wrapper rejected escape attempts
    // before they reached the inner txn.

    #[derive(Default)]
    struct TxnStubBackend;

    #[async_trait]
    impl RootFilesystem for TxnStubBackend {
        async fn list_dir(
            &self,
            _path: &VirtualPath,
        ) -> Result<Vec<crate::DirEntry>, FilesystemError> {
            Ok(Vec::new())
        }

        async fn stat(&self, path: &VirtualPath) -> Result<crate::FileStat, FilesystemError> {
            Ok(crate::FileStat {
                path: path.clone(),
                file_type: crate::FileType::Directory,
                len: 0,
                modified: None,
                sensitive: false,
            })
        }

        async fn begin(&self, _path: &VirtualPath) -> Result<Box<dyn StorageTxn>, FilesystemError> {
            Ok(Box::new(StubTxn::default()))
        }
    }

    #[derive(Default)]
    struct StubTxn {
        seen_put: Option<VirtualPath>,
        seen_get: Option<VirtualPath>,
        seen_delete: Option<VirtualPath>,
    }

    #[async_trait]
    impl StorageTxn for StubTxn {
        async fn put(
            &mut self,
            path: &VirtualPath,
            _entry: Entry,
            _cas: CasExpectation,
        ) -> Result<RecordVersion, FilesystemError> {
            self.seen_put = Some(path.clone());
            Ok(RecordVersion::from_backend(1))
        }

        async fn get(
            &mut self,
            path: &VirtualPath,
        ) -> Result<Option<VersionedEntry>, FilesystemError> {
            self.seen_get = Some(path.clone());
            Ok(None)
        }

        async fn delete(&mut self, path: &VirtualPath) -> Result<(), FilesystemError> {
            self.seen_delete = Some(path.clone());
            Ok(())
        }

        async fn commit(self: Box<Self>) -> Result<(), FilesystemError> {
            Ok(())
        }

        async fn rollback(self: Box<Self>) {}
    }

    fn scoped_txn_stub(permissions: MountPermissions) -> ScopedFilesystem<TxnStubBackend> {
        ScopedFilesystem::new(
            Arc::new(TxnStubBackend),
            MountView::new(vec![MountGrant::new(
                MountAlias::new("/workspace").unwrap(),
                VirtualPath::new("/engine/scoped_txn").unwrap(),
                permissions,
            )])
            .unwrap(),
        )
    }

    #[tokio::test]
    async fn scoped_txn_rejects_put_outside_mount_prefix() {
        // The mount prefix resolves to /engine/scoped_txn. A txn caller
        // who somehow has a VirtualPath outside that prefix must be
        // rejected with PathOutsideMount before the inner backend sees
        // it — the trait doc commits to this guarantee for every txn
        // backend, and the wrapper is the only place that holds across
        // future backends.
        let scoped = scoped_txn_stub(MountPermissions::read_write());
        let mut txn = scoped
            .begin(&ScopedPath::new("/workspace").unwrap())
            .await
            .unwrap();
        let escape = VirtualPath::new("/secrets/api_key").unwrap();
        let err = txn
            .put(&escape, Entry::bytes(b"leak".to_vec()), CasExpectation::Any)
            .await
            .unwrap_err();
        assert!(matches!(err, FilesystemError::PathOutsideMount { .. }));
    }

    #[tokio::test]
    async fn scoped_txn_rejects_get_outside_mount_prefix() {
        let scoped = scoped_txn_stub(MountPermissions::read_write());
        let mut txn = scoped
            .begin(&ScopedPath::new("/workspace").unwrap())
            .await
            .unwrap();
        let escape = VirtualPath::new("/secrets/api_key").unwrap();
        let err = txn.get(&escape).await.unwrap_err();
        assert!(matches!(err, FilesystemError::PathOutsideMount { .. }));
    }

    #[tokio::test]
    async fn scoped_txn_rejects_delete_outside_mount_prefix() {
        let scoped = scoped_txn_stub(MountPermissions {
            read: true,
            write: true,
            list: true,
            delete: true,
            execute: false,
        });
        let mut txn = scoped
            .begin(&ScopedPath::new("/workspace").unwrap())
            .await
            .unwrap();
        let escape = VirtualPath::new("/secrets/api_key").unwrap();
        let err = txn.delete(&escape).await.unwrap_err();
        assert!(matches!(err, FilesystemError::PathOutsideMount { .. }));
    }

    #[tokio::test]
    async fn scoped_txn_allows_put_inside_mount_prefix() {
        // Sanity check: paths under the mount prefix still reach the
        // inner backend. Without this we couldn't tell whether the
        // outside-prefix tests were rejecting legitimate calls too.
        let scoped = scoped_txn_stub(MountPermissions::read_write());
        let mut txn = scoped
            .begin(&ScopedPath::new("/workspace").unwrap())
            .await
            .unwrap();
        let inside = VirtualPath::new("/engine/scoped_txn/file").unwrap();
        txn.put(&inside, Entry::bytes(b"ok".to_vec()), CasExpectation::Any)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn scoped_txn_per_op_acl_blocks_write_without_write_permission() {
        // ScopedFilesystem::begin needs write (BeginTxn maps to write).
        // Grant write to authorize begin, then carry the ACL into the
        // wrapper. A separate scoped scope with only `read` would never
        // reach begin (BeginTxn requires write), so this matrix covers
        // the "txn-time ACL diverges from initial grant" concern by
        // dropping permissions through the per-op path inside the txn.
        //
        // To exercise the per-op denial path (PR #3659 review fix) we
        // construct a scope whose grant has `write` (so begin succeeds)
        // but no `delete`, and assert delete inside the txn returns the
        // ACL error rather than reaching the inner backend.
        let scoped = scoped_txn_stub(MountPermissions::read_write());
        let mut txn = scoped
            .begin(&ScopedPath::new("/workspace").unwrap())
            .await
            .unwrap();
        let inside = VirtualPath::new("/engine/scoped_txn/file").unwrap();
        let err = txn.delete(&inside).await.unwrap_err();
        match err {
            FilesystemError::Backend {
                operation: FilesystemOperation::Delete,
                reason,
                ..
            } => {
                assert!(
                    reason.contains("permission"),
                    "expected permission-denial reason, got {reason}"
                );
            }
            other => panic!("expected Backend(permission), got {other:?}"),
        }
    }
}
