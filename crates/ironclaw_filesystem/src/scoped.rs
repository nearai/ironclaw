use std::sync::Arc;

use ironclaw_host_api::{
    HostApiError, MountPermissions, MountView, ResourceScope, ScopedPath, VirtualPath,
};

use crate::backend::{EventRecord, StorageTxn};
use crate::{
    CasExpectation, DirEntry, Entry, FileStat, FilesystemError, FilesystemOperation, Filter,
    IndexSpec, Page, RecordVersion, RootFilesystem, SeqNo, VersionedEntry, path_prefix_matches,
};

/// Resolver from a per-invocation [`ResourceScope`] to the [`MountView`] that
/// authorizes its filesystem operations.
///
/// Production composition supplies a tenant-rewriting resolver
/// (`invocation_mount_view`) so consumer aliases (`/secrets`,
/// `/authorization`, …) resolve to `/tenants/<tenant>/users/<user>/<alias>`
/// virtual paths. Single-tenant tests use the
/// [`ScopedFilesystem::with_fixed_view`] shortcut, whose resolver ignores
/// `scope` and returns a constant view.
pub type MountViewResolver =
    dyn Fn(&ResourceScope) -> Result<MountView, HostApiError> + Send + Sync;

/// Invocation-scoped filesystem view.
///
/// Every operation takes the caller's [`ResourceScope`]. The configured
/// [`MountViewResolver`] turns that scope into a per-call [`MountView`]; the
/// view's grants are then used for the per-op permission check and for
/// resolving the caller's [`ScopedPath`] to a [`VirtualPath`] before the
/// underlying [`RootFilesystem`] is touched.
///
/// Higher-level stores (SecretStore, ProcessStore, …) accept a
/// `Arc<ScopedFilesystem<F>>` and call the unified `put`/`get`/`query`/etc.
/// ops on it, plumbing the request scope through every call. The
/// [`ScopedFilesystem`] is the *single* per-process FS handle; tenant
/// isolation comes from the resolver, not from a per-tenant store cache.
#[derive(Clone)]
pub struct ScopedFilesystem<F: ?Sized> {
    root: Arc<F>,
    resolver: Arc<MountViewResolver>,
}

impl<F: ?Sized> std::fmt::Debug for ScopedFilesystem<F> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScopedFilesystem")
            .field("root", &"<RootFilesystem>")
            .field("resolver", &"<MountViewResolver>")
            .finish()
    }
}

impl<F> ScopedFilesystem<F>
where
    F: RootFilesystem + ?Sized,
{
    /// Construct a scope-aware filesystem. `resolver` is invoked on every op
    /// to produce the [`MountView`] that authorizes that op.
    pub fn new<R>(root: Arc<F>, resolver: R) -> Self
    where
        R: Fn(&ResourceScope) -> Result<MountView, HostApiError> + Send + Sync + 'static,
    {
        Self {
            root,
            resolver: Arc::new(resolver),
        }
    }

    /// Construct a single-tenant filesystem whose resolver ignores `scope`
    /// and always returns `view`. Intended for tests, single-tenant CLI
    /// fixtures, and bootstrap paths that own the `RootFilesystem` directly.
    /// Production multi-tenant composition uses [`Self::new`] with
    /// `invocation_mount_view`.
    pub fn with_fixed_view(root: Arc<F>, view: MountView) -> Self {
        Self::new(root, move |_scope| Ok(view.clone()))
    }

    /// Resolve `path` for `scope` to the backend-facing [`VirtualPath`]
    /// without performing any FS op. Useful to consumer crates that need
    /// the canonical virtual path for an audit/log message.
    pub fn resolve(
        &self,
        scope: &ResourceScope,
        path: &ScopedPath,
    ) -> Result<VirtualPath, FilesystemError> {
        let view = self.mount_view(scope)?;
        view.resolve(path).map_err(FilesystemError::from)
    }

    /// Return the per-scope [`MountView`] used to authorize ops at this
    /// scope. Each call resolves the view fresh; callers that need to inspect
    /// it repeatedly should cache the returned value.
    pub fn mount_view(&self, scope: &ResourceScope) -> Result<MountView, FilesystemError> {
        (self.resolver)(scope).map_err(FilesystemError::from)
    }

    // ─── Unified entry plane ──────────────────────────────────────────────

    /// Write an [`Entry`] at `path` with a CAS precondition.
    pub async fn put(
        &self,
        scope: &ResourceScope,
        path: &ScopedPath,
        entry: Entry,
        cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError> {
        let virtual_path =
            self.resolve_with_permission(scope, path, FilesystemOperation::WriteFile)?;
        self.root.put(&virtual_path, entry, cas).await
    }

    /// Read the entry at `path`, returning `None` if absent.
    pub async fn get(
        &self,
        scope: &ResourceScope,
        path: &ScopedPath,
    ) -> Result<Option<VersionedEntry>, FilesystemError> {
        let virtual_path =
            self.resolve_with_permission(scope, path, FilesystemOperation::ReadFile)?;
        self.root.get(&virtual_path).await
    }

    /// Filtered query over `prefix`.
    pub async fn query(
        &self,
        scope: &ResourceScope,
        prefix: &ScopedPath,
        filter: &Filter,
        page: Page,
    ) -> Result<Vec<VersionedEntry>, FilesystemError> {
        let virtual_path =
            self.resolve_with_permission(scope, prefix, FilesystemOperation::Query)?;
        self.root.query(&virtual_path, filter, page).await
    }

    /// Declare an index on the mount under `prefix`.
    pub async fn ensure_index(
        &self,
        scope: &ResourceScope,
        prefix: &ScopedPath,
        spec: &IndexSpec,
    ) -> Result<(), FilesystemError> {
        let virtual_path =
            self.resolve_with_permission(scope, prefix, FilesystemOperation::EnsureIndex)?;
        self.root.ensure_index(&virtual_path, spec).await
    }

    /// Begin a multi-key transaction (capability-gated).
    ///
    /// PR #3659 review fix: returns a permission-checking wrapper around the
    /// underlying [`StorageTxn`] so the per-operation ACL is preserved across
    /// the transaction boundary. Without this wrapper, a caller granted only
    /// `write` could still `get` / `delete` through the raw txn handle once
    /// any backend implements transactions.
    pub async fn begin(
        &self,
        scope: &ResourceScope,
        prefix: &ScopedPath,
    ) -> Result<Box<dyn StorageTxn>, FilesystemError> {
        let view = self.mount_view(scope)?;
        let virtual_path =
            resolve_with_permission_view(&view, prefix, FilesystemOperation::BeginTxn)?;
        let inner = self.root.begin(&virtual_path).await?;
        let permissions = view.resolve_with_grant(prefix)?.1.permissions.clone();
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
        scope: &ResourceScope,
        path: &ScopedPath,
        payload: Vec<u8>,
    ) -> Result<SeqNo, FilesystemError> {
        let virtual_path =
            self.resolve_with_permission(scope, path, FilesystemOperation::Append)?;
        self.root.append(&virtual_path, payload).await
    }

    /// Read events at `path` starting just after `from`.
    pub async fn tail(
        &self,
        scope: &ResourceScope,
        path: &ScopedPath,
        from: SeqNo,
    ) -> Result<Vec<EventRecord>, FilesystemError> {
        let virtual_path = self.resolve_with_permission(scope, path, FilesystemOperation::Tail)?;
        self.root.tail(&virtual_path, from).await
    }

    /// Read at most `max_records` events at `path` starting just after `from`.
    pub async fn tail_bounded(
        &self,
        scope: &ResourceScope,
        path: &ScopedPath,
        from: SeqNo,
        max_records: usize,
    ) -> Result<Vec<EventRecord>, FilesystemError> {
        let virtual_path = self.resolve_with_permission(scope, path, FilesystemOperation::Tail)?;
        self.root
            .tail_bounded(&virtual_path, from, max_records)
            .await
    }

    /// Return the highest seq present at `path` with `seq > from`, or `None`
    /// when the gap is empty. SQL-backed mounts serve this with an O(1)
    /// `MAX(seq)` query; see [`RootFilesystem::head_seq`].
    pub async fn head_seq(
        &self,
        scope: &ResourceScope,
        path: &ScopedPath,
        from: SeqNo,
    ) -> Result<Option<SeqNo>, FilesystemError> {
        let virtual_path =
            self.resolve_with_permission(scope, path, FilesystemOperation::HeadSeq)?;
        self.root.head_seq(&virtual_path, from).await
    }

    // ─── Legacy bytes-plane methods (DEPRECATED — transitional) ───────────

    /// **DEPRECATED — use [`read_bytes`](Self::read_bytes) or
    /// [`get`](Self::get) instead.**
    pub async fn read_file(
        &self,
        scope: &ResourceScope,
        path: &ScopedPath,
    ) -> Result<Vec<u8>, FilesystemError> {
        let virtual_path =
            self.resolve_with_permission(scope, path, FilesystemOperation::ReadFile)?;
        self.root.read_file(&virtual_path).await
    }

    /// **DEPRECATED — use [`write_bytes`](Self::write_bytes) or
    /// [`put`](Self::put) instead.**
    pub async fn write_file(
        &self,
        scope: &ResourceScope,
        path: &ScopedPath,
        bytes: &[u8],
    ) -> Result<(), FilesystemError> {
        let virtual_path =
            self.resolve_with_permission(scope, path, FilesystemOperation::WriteFile)?;
        self.root.write_file(&virtual_path, bytes).await
    }

    /// Write bytes using an already-authorized mount view instead of the
    /// filesystem's configured resolver.
    ///
    /// This is for host adapters that parse a scoped path against the exact
    /// invocation-visible mounts and need the write to use that same authority.
    pub async fn write_bytes_with_mount_view(
        &self,
        view: &MountView,
        path: &ScopedPath,
        bytes: &[u8],
    ) -> Result<(), FilesystemError> {
        let virtual_path =
            resolve_with_permission_view(view, path, FilesystemOperation::WriteFile)?;
        self.root
            .put(
                &virtual_path,
                Entry::bytes(bytes.to_vec()),
                CasExpectation::Any,
            )
            .await
            .map(|_| ())
    }

    /// **DEPRECATED — no direct replacement on the unified surface.** Use
    /// `append`/`tail` for log-shaped mounts or `get`+`put` for
    /// read-modify-write.
    pub async fn append_file(
        &self,
        scope: &ResourceScope,
        path: &ScopedPath,
        bytes: &[u8],
    ) -> Result<(), FilesystemError> {
        let virtual_path =
            self.resolve_with_permission(scope, path, FilesystemOperation::AppendFile)?;
        self.root.append_file(&virtual_path, bytes).await
    }

    pub async fn list_dir(
        &self,
        scope: &ResourceScope,
        path: &ScopedPath,
    ) -> Result<Vec<DirEntry>, FilesystemError> {
        let virtual_path =
            self.resolve_with_permission(scope, path, FilesystemOperation::ListDir)?;
        self.root.list_dir(&virtual_path).await
    }

    pub async fn list_dir_bounded(
        &self,
        scope: &ResourceScope,
        path: &ScopedPath,
        max_entries: usize,
    ) -> Result<Vec<DirEntry>, FilesystemError> {
        let virtual_path =
            self.resolve_with_permission(scope, path, FilesystemOperation::ListDir)?;
        self.root.list_dir_bounded(&virtual_path, max_entries).await
    }

    pub async fn stat(
        &self,
        scope: &ResourceScope,
        path: &ScopedPath,
    ) -> Result<FileStat, FilesystemError> {
        let virtual_path = self.resolve_with_permission(scope, path, FilesystemOperation::Stat)?;
        self.root.stat(&virtual_path).await
    }

    pub async fn delete(
        &self,
        scope: &ResourceScope,
        path: &ScopedPath,
    ) -> Result<(), FilesystemError> {
        let virtual_path =
            self.resolve_with_permission(scope, path, FilesystemOperation::Delete)?;
        self.root.delete(&virtual_path).await
    }

    /// **DEPRECATED — the unified entry plane infers directories from path
    /// prefixes.** New consumer code must not call this.
    pub async fn create_dir_all(
        &self,
        scope: &ResourceScope,
        path: &ScopedPath,
    ) -> Result<(), FilesystemError> {
        let virtual_path =
            self.resolve_with_permission(scope, path, FilesystemOperation::CreateDirAll)?;
        self.root.create_dir_all(&virtual_path).await
    }

    // ─── Convenience helpers for byte-only callers ────────────────────────

    /// Read the body bytes at `path`. Convenience wrapper over [`get`] that
    /// errors if the path has no entry.
    pub async fn read_bytes(
        &self,
        scope: &ResourceScope,
        path: &ScopedPath,
    ) -> Result<Vec<u8>, FilesystemError> {
        match self.get(scope, path).await? {
            Some(versioned) => Ok(versioned.entry.body),
            None => {
                let virtual_path =
                    self.resolve_with_permission(scope, path, FilesystemOperation::ReadFile)?;
                Err(FilesystemError::NotFound {
                    path: virtual_path,
                    operation: FilesystemOperation::ReadFile,
                })
            }
        }
    }

    /// Read the body bytes at `path` only when the backend can enforce the
    /// supplied size bound before materializing oversized content.
    pub async fn read_bytes_bounded(
        &self,
        scope: &ResourceScope,
        path: &ScopedPath,
        max_bytes: usize,
    ) -> Result<Option<Vec<u8>>, FilesystemError> {
        let virtual_path =
            self.resolve_with_permission(scope, path, FilesystemOperation::ReadFile)?;
        self.root.read_file_bounded(&virtual_path, max_bytes).await
    }

    /// Write `body` as an opaque-file entry at `path` (no CAS precondition).
    /// Convenience wrapper over [`put`].
    pub async fn write_bytes(
        &self,
        scope: &ResourceScope,
        path: &ScopedPath,
        body: Vec<u8>,
    ) -> Result<(), FilesystemError> {
        self.put(scope, path, Entry::bytes(body), CasExpectation::Any)
            .await
            .map(|_| ())
    }

    // ─── Internals ────────────────────────────────────────────────────────

    fn resolve_with_permission(
        &self,
        scope: &ResourceScope,
        path: &ScopedPath,
        operation: FilesystemOperation,
    ) -> Result<VirtualPath, FilesystemError> {
        let view = self.mount_view(scope)?;
        resolve_with_permission_view(&view, path, operation)
    }
}

fn resolve_with_permission_view(
    view: &MountView,
    path: &ScopedPath,
    operation: FilesystemOperation,
) -> Result<VirtualPath, FilesystemError> {
    let (virtual_path, grant) = view.resolve_with_grant(path)?;
    if !operation_allowed(&grant.permissions, operation) {
        return Err(FilesystemError::PermissionDenied {
            path: path.clone(),
            operation,
        });
    }
    Ok(virtual_path)
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
        FilesystemOperation::Stat => permissions.read || permissions.list,
        FilesystemOperation::Delete => permissions.delete,
        FilesystemOperation::MountLocal | FilesystemOperation::Connect => false,
        FilesystemOperation::Query => permissions.read && permissions.list,
        FilesystemOperation::Tail | FilesystemOperation::HeadSeq => permissions.read,
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
            Err(FilesystemError::Backend {
                path: self.mount_prefix.clone(),
                operation,
                reason: "scoped transaction lacks the required permission".to_string(),
            })
        }
    }

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
        self.inner.commit().await
    }

    async fn rollback(self: Box<Self>) {
        self.inner.rollback().await
    }
}

#[cfg(test)]
mod tests;
