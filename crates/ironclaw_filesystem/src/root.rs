use async_trait::async_trait;
use ironclaw_host_api::VirtualPath;

use crate::backend::{EventRecord, StorageTxn};
use crate::{
    BackendCapabilities, CasExpectation, DirEntry, Entry, FileStat, FilesystemError,
    FilesystemOperation, Filter, IndexSpec, Page, RecordVersion, SeqNo, VersionedEntry,
};

/// Unified filesystem interface over canonical virtual paths.
///
/// Both individual storage backends (local files, Postgres, libSQL, HSM,
/// in-memory) and the composite dispatcher
/// ([`CompositeRootFilesystem`](crate::CompositeRootFilesystem)) implement this
/// trait. There is intentionally only one trait — the dispatcher *is* a
/// backend that routes by longest-prefix mount.
///
/// The trait surface is divided into:
/// - **Capabilities/identity** — every backend declares what it can do.
/// - **Unified entry plane** — [`put`](Self::put) / [`get`](Self::get) /
///   [`delete`](Self::delete) / [`list_dir`](Self::list_dir) /
///   [`query`](Self::query) / [`ensure_index`](Self::ensure_index) /
///   [`stat`](Self::stat). Bytes files and structured records both flow
///   through these methods as different inhabitants of [`Entry`].
/// - **Atomicity** — [`begin`](Self::begin) for backends that natively
///   support multi-key transactions. Stores must always work with CAS
///   (`put` + `CasExpectation::Version`) as the floor.
/// - **Event plane** — [`append`](Self::append) / [`tail`](Self::tail) for
///   log-shaped mounts.
/// - **Legacy bytes plane** — [`read_file`](Self::read_file) /
///   [`write_file`](Self::write_file) / [`append_file`](Self::append_file) /
///   [`list_dir_bytes`](Self::list_dir_bytes) / [`create_dir_all`](Self::create_dir_all).
///   Kept during migration; default impls route legacy reads/writes through
///   `put`/`get`. Removed after task 17 (`src/db/` dissolution) lands.
#[async_trait]
pub trait RootFilesystem: Send + Sync {
    // ─── Capabilities / identity ──────────────────────────────────────────

    /// Capabilities advertised by this backend. Mount-time validation in
    /// [`CompositeRootFilesystem::mount`](crate::CompositeRootFilesystem::mount)
    /// uses this to refuse backends that cannot serve the indexes a consumer
    /// has declared.
    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities::default()
    }

    // ─── Unified entry plane ──────────────────────────────────────────────

    /// Write an [`Entry`] at `path` with a compare-and-swap precondition.
    /// Returns the new [`RecordVersion`].
    ///
    /// Default impl is `Unsupported` — backends that want to participate in
    /// the unified surface must implement `put` natively. Byte-only backends
    /// can do this with a thin delegation to their own native `write_file`,
    /// gated on `kind = None`, empty `indexed`, and `CasExpectation::Any`;
    /// see `LocalFilesystem::put` for the canonical pattern. We deliberately
    /// do **not** route the default `put` through `self.write_file`, because
    /// the default `write_file` routes through `self.put` — a backend that
    /// overrode neither would recurse to a stack overflow.
    async fn put(
        &self,
        path: &VirtualPath,
        _entry: Entry,
        _cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError> {
        unsupported(path, FilesystemOperation::WriteFile)
    }

    /// Read the entry at `path`, returning `None` if no entry is present.
    ///
    /// Default impl is `Unsupported`. Same recursion concern as `put`:
    /// `read_file`'s default delegates here, so we must not delegate the
    /// other direction in the trait default. Byte-only backends implement
    /// `get` by wrapping their native `read_file` result in
    /// `Some(VersionedEntry { entry: Entry::bytes(body), version: 0 })`
    /// directly. See `LocalFilesystem::get` for the canonical pattern.
    async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        unsupported(path, FilesystemOperation::ReadFile)
    }

    /// Lists direct children of a canonical virtual directory.
    ///
    /// Lightweight: returns path + type, no payload, no pagination. Use
    /// [`query`](Self::query) when you need pagination, filtering, or the
    /// materialized entry bodies.
    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError>;

    /// Filtered query over `prefix`. Returns the materialized entries
    /// matching `filter`. Backends without `query` capability return
    /// [`FilesystemError::Unsupported`].
    async fn query(
        &self,
        path: &VirtualPath,
        _filter: &Filter,
        _page: Page,
    ) -> Result<Vec<VersionedEntry>, FilesystemError> {
        unsupported(path, FilesystemOperation::Query)
    }

    /// Declare an index on a mount prefix. Idempotent: re-declaring the same
    /// spec is a no-op; declaring a conflicting spec returns
    /// [`FilesystemError::IndexConflict`].
    async fn ensure_index(
        &self,
        path: &VirtualPath,
        _spec: &IndexSpec,
    ) -> Result<(), FilesystemError> {
        unsupported(path, FilesystemOperation::EnsureIndex)
    }

    /// Returns metadata for a canonical virtual path without revealing raw host paths.
    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError>;

    /// Deletes an existing canonical virtual file or directory. Missing paths
    /// return [`FilesystemError::NotFound`]; backends that do not support
    /// delete must fail closed before side effects.
    async fn delete(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        Err(FilesystemError::Backend {
            path: path.clone(),
            operation: FilesystemOperation::Delete,
            reason: "delete is not supported by this backend".to_string(),
        })
    }

    // ─── Atomicity ────────────────────────────────────────────────────────

    /// Begin a multi-key transaction scoped to `prefix`. Backends with only
    /// CAS support return [`FilesystemError::Unsupported`]; consumers must
    /// always have a CAS-only path.
    async fn begin(&self, path: &VirtualPath) -> Result<Box<dyn StorageTxn>, FilesystemError> {
        unsupported(path, FilesystemOperation::BeginTxn)
    }

    // ─── Event plane (append/tail) ────────────────────────────────────────

    /// Append `payload` to the event log at `path`, returning the assigned
    /// monotonic [`SeqNo`]. Distinct from [`append_file`](Self::append_file),
    /// which is the legacy byte-append on a regular file.
    async fn append(
        &self,
        path: &VirtualPath,
        _payload: Vec<u8>,
    ) -> Result<SeqNo, FilesystemError> {
        unsupported(path, FilesystemOperation::Append)
    }

    /// Read events at `path` starting at `from` (exclusive). Returns at most
    /// one page of records; consumers loop with the latest seq to drain the
    /// log. Streaming support will replace this Vec return shape in a later
    /// pass; the unstable signature is intentional.
    async fn tail(
        &self,
        path: &VirtualPath,
        _from: SeqNo,
    ) -> Result<Vec<EventRecord>, FilesystemError> {
        unsupported(path, FilesystemOperation::Tail)
    }

    // ─── Legacy bytes plane (DEPRECATED — removed after consumer migration) ─
    //
    // The methods below predate the unified [`put`]/[`get`] surface and exist
    // only so existing call sites (engine v2 sandbox tools, the host_runtime
    // coding tools, in-tree test scaffolds) keep compiling during the
    // consumer-migration window. New code MUST use the unified ops:
    //   - `read_file(path)`     → `get(path)?.entry.body`
    //   - `write_file(path, b)` → `put(path, Entry::bytes(b), CasExpectation::Any)`
    //   - `append_file(path, b)`→ no replacement on the unified surface; use
    //                              `append`/`tail` for log-shaped mounts, or
    //                              `get`+`put` for read-modify-write
    //   - `create_dir_all(path)`→ no longer needed; the entry plane infers
    //                              directories from path prefixes
    //
    // These methods will be removed in the consumer-migration cleanup pass
    // (task #17 in the rework plan). Do not extend them; do not call them
    // from new consumer code.

    /// **DEPRECATED — use [`get`](Self::get) instead.**
    ///
    /// Reads a file by canonical virtual path without exposing backend host
    /// paths in errors. Default impl routes through `get` and extracts the
    /// body; backends that have a faster native byte read may override.
    /// Removed once consumer migration completes (rework task #17). New
    /// consumer code must call `get` directly.
    async fn read_file(&self, path: &VirtualPath) -> Result<Vec<u8>, FilesystemError> {
        match self.get(path).await? {
            Some(entry) => Ok(entry.entry.body),
            None => Err(FilesystemError::NotFound {
                path: path.clone(),
                operation: FilesystemOperation::ReadFile,
            }),
        }
    }

    /// **DEPRECATED — use [`put`](Self::put) instead.**
    ///
    /// Writes bytes to a canonical virtual path while preserving backend
    /// containment. Default impl routes through `put` with
    /// `CasExpectation::Any`. Removed once consumer migration completes
    /// (rework task #17). New consumer code must call `put` with
    /// `Entry::bytes(...)` and an explicit `CasExpectation`.
    async fn write_file(&self, path: &VirtualPath, bytes: &[u8]) -> Result<(), FilesystemError> {
        self.put(path, Entry::bytes(bytes.to_vec()), CasExpectation::Any)
            .await
            .map(|_| ())
    }

    /// **DEPRECATED — no direct replacement on the unified surface.**
    ///
    /// Distinct from [`append`](Self::append), which is the event-plane
    /// sequence operation. Use `append`/`tail` for log-shaped mounts or a
    /// `get` + `put` read-modify-write loop for arbitrary bytes. Removed
    /// once consumer migration completes (rework task #17).
    async fn append_file(&self, path: &VirtualPath, _bytes: &[u8]) -> Result<(), FilesystemError> {
        Err(FilesystemError::Unsupported {
            path: path.clone(),
            operation: FilesystemOperation::AppendFile,
        })
    }

    /// **DEPRECATED — the entry plane infers directories from path prefixes.**
    ///
    /// Creates a canonical virtual directory and any missing parents.
    /// Backends that do not support directories must fail closed before side
    /// effects. New consumer code must not call this — `put` against a leaf
    /// path implicitly establishes the directory hierarchy.
    async fn create_dir_all(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        Err(FilesystemError::Unsupported {
            path: path.clone(),
            operation: FilesystemOperation::CreateDirAll,
        })
    }
}

fn unsupported<T>(
    path: &VirtualPath,
    operation: FilesystemOperation,
) -> Result<T, FilesystemError> {
    Err(FilesystemError::Unsupported {
        path: path.clone(),
        operation,
    })
}
