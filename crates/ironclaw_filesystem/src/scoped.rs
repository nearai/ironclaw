use std::{sync::Arc, time::Instant};

use ironclaw_host_api::{
    HostApiError, MountPermissions, MountView, ResourceScope, ScopedPath, VirtualPath,
};
use ironclaw_observability::live_latency_started_at;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum PathClass {
    Workspace,
    Memory,
    Artifacts,
    Turns,
    Resources,
    Approvals,
    Authorization,
    Events,
    Processes,
    RunState,
    Secrets,
    Skills,
    System,
    Threads,
    Other,
}

impl PathClass {
    fn as_str(self) -> &'static str {
        match self {
            Self::Workspace => "workspace",
            Self::Memory => "memory",
            Self::Artifacts => "artifacts",
            Self::Turns => "turns",
            Self::Resources => "resources",
            Self::Approvals => "approvals",
            Self::Authorization => "authorization",
            Self::Events => "events",
            Self::Processes => "processes",
            Self::RunState => "run_state",
            Self::Secrets => "secrets",
            Self::Skills => "skills",
            Self::System => "system",
            Self::Threads => "threads",
            Self::Other => "other",
        }
    }
}

fn scoped_path_class(path: &ScopedPath) -> PathClass {
    match path.as_str().split('/').nth(1) {
        Some("workspace") => PathClass::Workspace,
        Some("memory") => PathClass::Memory,
        Some("artifacts") => PathClass::Artifacts,
        Some("turns") => PathClass::Turns,
        Some("resources") => PathClass::Resources,
        Some("approvals") => PathClass::Approvals,
        Some("authorization") => PathClass::Authorization,
        Some("events") => PathClass::Events,
        Some("processes") => PathClass::Processes,
        Some("run-state") => PathClass::RunState,
        Some("secrets") => PathClass::Secrets,
        Some("skills") => PathClass::Skills,
        Some("system") => PathClass::System,
        Some("threads") => PathClass::Threads,
        _ => PathClass::Other,
    }
}

fn scoped_path_detail(path: &ScopedPath) -> &'static str {
    let segments = path
        .as_str()
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    match segments.as_slice() {
        ["turns", "state.json"] => "turn_state_snapshot",
        ["resources", "snapshot.json"] => "resource_governor_snapshot",
        ["resources", "budget-gates.json"] => "budget_gate_snapshot",
        ["approvals", "capability-permissions", ..] => "approval_capability_permissions",
        ["approvals", "auto-approve", ..] => "approval_auto_approve",
        ["approvals", "persistent", ..] => "approval_persistent_policy",
        ["authorization", "leases", ..] => "authorization_leases",
        ["events", ..] => "events",
        ["processes", ..] => "processes",
        ["run-state", ..] => "run_state",
        ["secrets", ..] => "secrets",
        ["skills", ..] => "skill_bundles",
        ["system", "skills", ..] => "system_skill_bundles",
        ["threads", ..] => "threads",
        ["turns", ..]
        | ["resources", ..]
        | ["approvals", ..]
        | ["authorization", ..]
        | ["system", ..] => "unknown",
        _ => match segments.len() {
            0 => "root",
            1 => "top_level",
            2 => "one_level",
            _ => "nested",
        },
    }
}

fn filesystem_error_kind(error: &FilesystemError) -> &'static str {
    match error {
        FilesystemError::Contract(_) => "contract",
        FilesystemError::PermissionDenied { .. } => "permission_denied",
        FilesystemError::MountNotFound { .. } => "mount_not_found",
        FilesystemError::NotFound { .. } => "not_found",
        FilesystemError::PathOutsideMount { .. } => "path_outside_mount",
        FilesystemError::SymlinkEscape { .. } => "symlink_escape",
        FilesystemError::MountConflict { .. } => "mount_conflict",
        FilesystemError::Backend { .. } => "backend",
        FilesystemError::VersionMismatch { .. } => "version_mismatch",
        FilesystemError::Unsupported { .. } => "unsupported",
        FilesystemError::IndexConflict { .. } => "index_conflict",
        FilesystemError::DescriptorOverclaims { .. } => "descriptor_overclaims",
        FilesystemError::SerializeIndexed { .. } => "serialize_indexed",
        FilesystemError::DeserializeIndexed { .. } => "deserialize_indexed",
        FilesystemError::CorruptRecordVersion { .. } => "corrupt_record_version",
        FilesystemError::IndexSpecMissingAfterUpsert { .. } => "index_spec_missing_after_upsert",
        FilesystemError::BackendInfrastructure { .. } => "backend_infrastructure",
    }
}

fn trace_fs_latency<T>(
    operation: &'static str,
    path: &ScopedPath,
    started_at: Option<Instant>,
    result: &Result<T, FilesystemError>,
    bytes: Option<usize>,
) {
    let path_class = scoped_path_class(path);
    let path_detail = scoped_path_detail(path);
    match result {
        Ok(_) => ironclaw_observability::live_latency_trace_ok!(
            "filesystem",
            operation,
            started_at,
            path_class = path_class.as_str(),
            path_detail,
            bytes = bytes.unwrap_or(0),
            "filesystem operation completed",
        ),
        Err(error) => ironclaw_observability::live_latency_trace_error!(
            "filesystem",
            operation,
            started_at,
            filesystem_error_kind(error),
            path_class = path_class.as_str(),
            path_detail,
            bytes = bytes.unwrap_or(0),
            "filesystem operation failed",
        ),
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

    /// Capabilities advertised by the underlying [`RootFilesystem`].
    ///
    /// Exposed so capability-gated helpers (such as
    /// [`cas_update`](crate::cas_update)) can fail closed before a
    /// read-modify-write loop when the backend cannot honor compare-and-swap.
    ///
    /// Note on the composite router: a
    /// [`CompositeRootFilesystem`](crate::CompositeRootFilesystem) returns
    /// [`BackendCapabilities::default`] here because it routes per-path and
    /// cannot answer capabilities without a concrete path. Callers that gate on
    /// this value must therefore treat the *default/empty* shape as "unknown,
    /// defer to op-time" rather than "no CAS", and still map an op-time
    /// `Unsupported(WriteFile)` to their capability-missing error.
    pub fn capabilities(&self) -> crate::BackendCapabilities {
        self.root.capabilities()
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
        let started_at = live_latency_started_at();
        let bytes = entry.body.len();
        let virtual_path =
            self.resolve_with_permission(scope, path, FilesystemOperation::WriteFile)?;
        let result = self.root.put(&virtual_path, entry, cas).await;
        trace_fs_latency("put", path, started_at, &result, Some(bytes));
        result
    }

    /// Read the entry at `path`, returning `None` if absent.
    pub async fn get(
        &self,
        scope: &ResourceScope,
        path: &ScopedPath,
    ) -> Result<Option<VersionedEntry>, FilesystemError> {
        let started_at = live_latency_started_at();
        let virtual_path =
            self.resolve_with_permission(scope, path, FilesystemOperation::ReadFile)?;
        let result = self.root.get(&virtual_path).await;
        trace_fs_latency("get", path, started_at, &result, None);
        result
    }

    /// Filtered query over `prefix`.
    pub async fn query(
        &self,
        scope: &ResourceScope,
        prefix: &ScopedPath,
        filter: &Filter,
        page: Page,
    ) -> Result<Vec<VersionedEntry>, FilesystemError> {
        let started_at = live_latency_started_at();
        let virtual_path =
            self.resolve_with_permission(scope, prefix, FilesystemOperation::Query)?;
        let result = self.root.query(&virtual_path, filter, page).await;
        trace_fs_latency("query", prefix, started_at, &result, None);
        result
    }

    /// Declare an index on the mount under `prefix`.
    pub async fn ensure_index(
        &self,
        scope: &ResourceScope,
        prefix: &ScopedPath,
        spec: &IndexSpec,
    ) -> Result<(), FilesystemError> {
        let started_at = live_latency_started_at();
        let virtual_path =
            self.resolve_with_permission(scope, prefix, FilesystemOperation::EnsureIndex)?;
        let result = self.root.ensure_index(&virtual_path, spec).await;
        trace_fs_latency("ensure_index", prefix, started_at, &result, None);
        result
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
        let started_at = live_latency_started_at();
        let view = self.mount_view(scope)?;
        let virtual_path =
            resolve_with_permission_view(&view, prefix, FilesystemOperation::BeginTxn)?;
        let result = self.root.begin(&virtual_path).await;
        trace_fs_latency("begin", prefix, started_at, &result, None);
        let inner = result?;
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
        let started_at = live_latency_started_at();
        let bytes = payload.len();
        let virtual_path =
            self.resolve_with_permission(scope, path, FilesystemOperation::Append)?;
        let result = self.root.append(&virtual_path, payload).await;
        trace_fs_latency("append", path, started_at, &result, Some(bytes));
        result
    }

    /// Append multiple `payloads` to the event log at `path` in one backend
    /// round-trip, returning the assigned SeqNos in payload order. The mount /
    /// permission is resolved once for the shared path; the multi-row write
    /// itself happens in the backend's [`RootFilesystem::append_batch`].
    pub async fn append_batch(
        &self,
        scope: &ResourceScope,
        path: &ScopedPath,
        payloads: Vec<Vec<u8>>,
    ) -> Result<Vec<SeqNo>, FilesystemError> {
        let started_at = live_latency_started_at();
        let bytes = payloads.iter().map(Vec::len).sum();
        let virtual_path =
            self.resolve_with_permission(scope, path, FilesystemOperation::Append)?;
        let result = self.root.append_batch(&virtual_path, payloads).await;
        trace_fs_latency("append_batch", path, started_at, &result, Some(bytes));
        result
    }

    /// Read events at `path` starting just after `from`.
    pub async fn tail(
        &self,
        scope: &ResourceScope,
        path: &ScopedPath,
        from: SeqNo,
    ) -> Result<Vec<EventRecord>, FilesystemError> {
        let started_at = live_latency_started_at();
        let virtual_path = self.resolve_with_permission(scope, path, FilesystemOperation::Tail)?;
        let result = self.root.tail(&virtual_path, from).await;
        trace_fs_latency("tail", path, started_at, &result, None);
        result
    }

    /// Read at most `max_records` events at `path` starting just after `from`.
    pub async fn tail_bounded(
        &self,
        scope: &ResourceScope,
        path: &ScopedPath,
        from: SeqNo,
        max_records: usize,
    ) -> Result<Vec<EventRecord>, FilesystemError> {
        let started_at = live_latency_started_at();
        let virtual_path = self.resolve_with_permission(scope, path, FilesystemOperation::Tail)?;
        let result = self
            .root
            .tail_bounded(&virtual_path, from, max_records)
            .await;
        trace_fs_latency("tail_bounded", path, started_at, &result, None);
        result
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
        let started_at = live_latency_started_at();
        let virtual_path =
            self.resolve_with_permission(scope, path, FilesystemOperation::HeadSeq)?;
        let result = self.root.head_seq(&virtual_path, from).await;
        trace_fs_latency("head_seq", path, started_at, &result, None);
        result
    }

    /// Reserve a path-local monotonic sequence number.
    pub async fn reserve_sequence(
        &self,
        scope: &ResourceScope,
        path: &ScopedPath,
    ) -> Result<SeqNo, FilesystemError> {
        let started_at = live_latency_started_at();
        let virtual_path =
            self.resolve_with_permission(scope, path, FilesystemOperation::ReserveSeq)?;
        let result = self.root.reserve_sequence(&virtual_path).await;
        trace_fs_latency("reserve_sequence", path, started_at, &result, None);
        result
    }

    // ─── Legacy bytes-plane methods (DEPRECATED — transitional) ───────────

    /// **DEPRECATED — use [`read_bytes`](Self::read_bytes) or
    /// [`get`](Self::get) instead.**
    pub async fn read_file(
        &self,
        scope: &ResourceScope,
        path: &ScopedPath,
    ) -> Result<Vec<u8>, FilesystemError> {
        let started_at = live_latency_started_at();
        let virtual_path =
            self.resolve_with_permission(scope, path, FilesystemOperation::ReadFile)?;
        let result = self.root.read_file(&virtual_path).await;
        trace_fs_latency("read_file", path, started_at, &result, None);
        result
    }

    /// **DEPRECATED — use [`write_bytes`](Self::write_bytes) or
    /// [`put`](Self::put) instead.**
    pub async fn write_file(
        &self,
        scope: &ResourceScope,
        path: &ScopedPath,
        bytes: &[u8],
    ) -> Result<(), FilesystemError> {
        let started_at = live_latency_started_at();
        let virtual_path =
            self.resolve_with_permission(scope, path, FilesystemOperation::WriteFile)?;
        let result = self.root.write_file(&virtual_path, bytes).await;
        trace_fs_latency("write_file", path, started_at, &result, Some(bytes.len()));
        result
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
        let started_at = live_latency_started_at();
        let virtual_path =
            resolve_with_permission_view(view, path, FilesystemOperation::WriteFile)?;
        let result = self
            .root
            .put(
                &virtual_path,
                Entry::bytes(bytes.to_vec()),
                CasExpectation::Any,
            )
            .await
            .map(|_| ());
        trace_fs_latency(
            "write_bytes_with_mount_view",
            path,
            started_at,
            &result,
            Some(bytes.len()),
        );
        result
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
        let started_at = live_latency_started_at();
        let virtual_path =
            self.resolve_with_permission(scope, path, FilesystemOperation::AppendFile)?;
        let result = self.root.append_file(&virtual_path, bytes).await;
        trace_fs_latency("append_file", path, started_at, &result, Some(bytes.len()));
        result
    }

    pub async fn list_dir(
        &self,
        scope: &ResourceScope,
        path: &ScopedPath,
    ) -> Result<Vec<DirEntry>, FilesystemError> {
        let started_at = live_latency_started_at();
        let virtual_path =
            self.resolve_with_permission(scope, path, FilesystemOperation::ListDir)?;
        let result = self.root.list_dir(&virtual_path).await;
        trace_fs_latency("list_dir", path, started_at, &result, None);
        result
    }

    pub async fn list_dir_bounded(
        &self,
        scope: &ResourceScope,
        path: &ScopedPath,
        max_entries: usize,
    ) -> Result<Vec<DirEntry>, FilesystemError> {
        let started_at = live_latency_started_at();
        let virtual_path =
            self.resolve_with_permission(scope, path, FilesystemOperation::ListDir)?;
        let result = self.root.list_dir_bounded(&virtual_path, max_entries).await;
        trace_fs_latency("list_dir_bounded", path, started_at, &result, None);
        result
    }

    pub async fn stat(
        &self,
        scope: &ResourceScope,
        path: &ScopedPath,
    ) -> Result<FileStat, FilesystemError> {
        let started_at = live_latency_started_at();
        let virtual_path = self.resolve_with_permission(scope, path, FilesystemOperation::Stat)?;
        let result = self.root.stat(&virtual_path).await;
        trace_fs_latency("stat", path, started_at, &result, None);
        result
    }

    pub async fn delete(
        &self,
        scope: &ResourceScope,
        path: &ScopedPath,
    ) -> Result<(), FilesystemError> {
        let started_at = live_latency_started_at();
        let virtual_path =
            self.resolve_with_permission(scope, path, FilesystemOperation::Delete)?;
        let result = self.root.delete(&virtual_path).await;
        trace_fs_latency("delete", path, started_at, &result, None);
        result
    }

    /// Delete the single entry at `path` only when its version equals
    /// `expected_version`. See [`RootFilesystem::delete_if_version`].
    pub async fn delete_if_version(
        &self,
        scope: &ResourceScope,
        path: &ScopedPath,
        expected_version: RecordVersion,
    ) -> Result<(), FilesystemError> {
        let started_at = live_latency_started_at();
        let virtual_path =
            self.resolve_with_permission(scope, path, FilesystemOperation::Delete)?;
        let result = self
            .root
            .delete_if_version(&virtual_path, expected_version)
            .await;
        trace_fs_latency("delete_if_version", path, started_at, &result, None);
        result
    }

    /// **DEPRECATED — the unified entry plane infers directories from path
    /// prefixes.** New consumer code must not call this.
    pub async fn create_dir_all(
        &self,
        scope: &ResourceScope,
        path: &ScopedPath,
    ) -> Result<(), FilesystemError> {
        let started_at = live_latency_started_at();
        let virtual_path =
            self.resolve_with_permission(scope, path, FilesystemOperation::CreateDirAll)?;
        let result = self.root.create_dir_all(&virtual_path).await;
        trace_fs_latency("create_dir_all", path, started_at, &result, None);
        result
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
        | FilesystemOperation::Append
        | FilesystemOperation::ReserveSeq => permissions.write,
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

    async fn reserve_sequence(&mut self, path: &VirtualPath) -> Result<SeqNo, FilesystemError> {
        self.check(FilesystemOperation::ReserveSeq)?;
        self.check_path(path)?;
        self.inner.reserve_sequence(path).await
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
