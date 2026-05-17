use std::sync::Arc;

use ironclaw_host_api::{MountPermissions, MountView, ScopedPath, VirtualPath};

use crate::backend::{EventRecord, StorageTxn};
use crate::{
    CasExpectation, DirEntry, Entry, FileStat, FilesystemError, FilesystemOperation, Filter,
    IndexSpec, Page, RecordVersion, RootFilesystem, SeqNo, VersionedEntry,
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
        // Append on the event plane is a write — permission mirrors AppendFile.
        let virtual_path = self.resolve_with_permission(path, FilesystemOperation::AppendFile)?;
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
        FilesystemOperation::WriteFile => permissions.write,
        FilesystemOperation::AppendFile => permissions.write,
        FilesystemOperation::ListDir => permissions.list,
        // Stat is metadata-only: either read authority or list authority reveals
        // equivalent existence/type information without file contents.
        FilesystemOperation::Stat => permissions.read || permissions.list,
        FilesystemOperation::Delete => permissions.delete,
        FilesystemOperation::CreateDirAll => permissions.write,
        FilesystemOperation::MountLocal => false,
        // Query enumerates records, so requires both read (to see contents) and
        // list (to enumerate). Either alone is insufficient.
        FilesystemOperation::Query => permissions.read && permissions.list,
        // Index/transaction declarations mutate the mount's structural state.
        FilesystemOperation::EnsureIndex => permissions.write,
        FilesystemOperation::BeginTxn => permissions.write,
        // Tail mirrors read on the byte plane (append is covered by AppendFile).
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
        self.inner.put(path, entry, cas).await
    }

    async fn get(&mut self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        self.check(FilesystemOperation::ReadFile)?;
        self.inner.get(path).await
    }

    async fn delete(&mut self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.check(FilesystemOperation::Delete)?;
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
