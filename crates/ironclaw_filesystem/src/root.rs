use async_trait::async_trait;
use ironclaw_host_api::VirtualPath;

use crate::backend::{EventRecord, StorageTxn};
use crate::{
    BackendCapabilities, CasExpectation, DirEntry, Entry, FileStat, FilesystemError,
    FilesystemOperation, Filter, IndexSpec, Page, RecordVersion, SeqNo, VersionedEntry,
};

/// One write leg of a [`put_batch`](RootFilesystem::put_batch) call: an
/// [`Entry`] to write at `path` under the given [`CasExpectation`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchPut {
    pub path: VirtualPath,
    pub entry: Entry,
    pub cas: CasExpectation,
}

/// Universal upper bound on the number of legs in a single
/// [`put_batch`](RootFilesystem::put_batch). The default impl rejects larger
/// batches before opening a transaction; backends with native multi-key
/// `put_batch` overrides reuse this same cap rather than defining their own.
pub const MAX_BATCH_PUTS: usize = 64;

/// Universal upper bound on the number of paths in a single
/// [`get_batch`](RootFilesystem::get_batch). Deliberately **not**
/// [`MAX_BATCH_PUTS`]: a read batch has no transaction legs to bound, so the
/// motivation is different — SQL `IN`-clause / `= ANY` payload size and the
/// per-statement bound-parameter ceiling, not multi-key txn cost. A read batch
/// is therefore allowed to be considerably larger than a write batch.
pub const MAX_BATCH_GETS: usize = 256;

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

    /// Lists at most `max_entries` direct children of a canonical virtual
    /// directory.
    ///
    /// Backends that can stop directory enumeration early should override this.
    /// The default preserves compatibility by delegating to [`Self::list_dir`]
    /// and truncating the result after materialization.
    async fn list_dir_bounded(
        &self,
        path: &VirtualPath,
        max_entries: usize,
    ) -> Result<Vec<DirEntry>, FilesystemError> {
        let mut entries = self.list_dir(path).await?;
        entries.truncate(max_entries);
        Ok(entries)
    }

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

    /// Read an opaque file only when its body is at most `max_bytes`.
    ///
    /// Returns `Ok(None)` when the file exists but exceeds the caller's limit.
    /// Streaming backends should enforce this before materializing the full body.
    async fn read_file_bounded(
        &self,
        path: &VirtualPath,
        max_bytes: usize,
    ) -> Result<Option<Vec<u8>>, FilesystemError> {
        let stat = self.stat(path).await?;
        if stat.len > max_bytes as u64 {
            return Ok(None);
        }
        let Some(entry) = self.get(path).await? else {
            return Err(FilesystemError::NotFound {
                path: path.clone(),
                operation: FilesystemOperation::ReadFile,
            });
        };
        if entry.entry.body.len() > max_bytes {
            return Ok(None);
        }
        Ok(Some(entry.entry.body))
    }

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

    /// Write several [`Entry`] values in one call.
    ///
    /// Availability contract:
    /// - `puts.len() == 0` is a programmer error and returns
    ///   [`FilesystemError::BackendInfrastructure`] (no path is in scope for an
    ///   empty batch).
    /// - `puts.len() == 1` is **always** available: it routes through
    ///   [`put`](Self::put) and needs no transaction, so every backend (even
    ///   CAS-only ones) serves it.
    /// - `puts.len() > 1` is **all-or-nothing atomic only on
    ///   [`TxnCapability::MultiKey`](crate::TxnCapability::MultiKey) backends**
    ///   (Postgres today; libSQL after PR-3; the in-memory reference after
    ///   PR-4). The default impl below opens a [`begin`](Self::begin)
    ///   transaction over the longest common directory prefix; on a CAS-only
    ///   backend `begin` returns [`FilesystemError::Unsupported`] and that
    ///   propagates unchanged — callers that require atomic batching MUST gate
    ///   on [`Capability::BatchPut`](crate::Capability::BatchPut) and fall back
    ///   to per-key CAS when it is absent.
    ///
    /// Returns one [`RecordVersion`] per put, in input order. On any failure in
    /// a multi-key batch the transaction is rolled back and nothing is written.
    async fn put_batch(&self, puts: Vec<BatchPut>) -> Result<Vec<RecordVersion>, FilesystemError> {
        match puts.len() {
            0 => Err(FilesystemError::BackendInfrastructure {
                operation: FilesystemOperation::PutBatch,
                reason: "empty put_batch".to_string(),
            }),
            1 => {
                let mut puts = puts;
                let BatchPut { path, entry, cas } = puts.swap_remove(0);
                Ok(vec![self.put(&path, entry, cas).await?])
            }
            _ => {
                if puts.len() > MAX_BATCH_PUTS {
                    return Err(FilesystemError::BackendInfrastructure {
                        operation: FilesystemOperation::PutBatch,
                        reason: "batch exceeds MAX_BATCH_PUTS".to_string(),
                    });
                }
                let prefix =
                    crate::common_dir_prefix(puts.iter().map(|p| &p.path)).ok_or_else(|| {
                        FilesystemError::BackendInfrastructure {
                            operation: FilesystemOperation::PutBatch,
                            reason: "put_batch entries share no common directory prefix"
                                .to_string(),
                        }
                    })?;
                let mut txn = self.begin(&prefix).await?;
                let mut versions = Vec::with_capacity(puts.len());
                for BatchPut { path, entry, cas } in puts {
                    match txn.put(&path, entry, cas).await {
                        Ok(version) => versions.push(version),
                        Err(error) => {
                            txn.rollback().await;
                            return Err(error);
                        }
                    }
                }
                txn.commit().await?;
                Ok(versions)
            }
        }
    }

    /// Read several entries in one call.
    ///
    /// Returns one [`Option<VersionedEntry>`] per input path, **in input
    /// order** (`None` = the path holds no entry). This is the read analogue
    /// of [`put_batch`](Self::put_batch): a pure-latency collapse of N serial
    /// [`get`](Self::get) round-trips into one. There is **no** atomicity and
    /// **no** CAS — a read batch observes whatever each path holds when the
    /// backend reads it; it carries no all-or-nothing promise.
    ///
    /// Availability contract:
    /// - `paths.len() == 0` is a valid no-op and returns `Ok(vec![])` — unlike
    ///   the empty `put_batch`, an empty read has nothing to fail on.
    /// - `paths.len() > MAX_BATCH_GETS` returns
    ///   [`FilesystemError::BackendInfrastructure`].
    ///
    /// The default impl reads sequentially through [`get`](Self::get), so
    /// `get_batch` is **always available** on every backend. Native backends
    /// override it with a single `WHERE path IN (…)` / `= ANY` query for the
    /// latency win — they are merely faster, never required — so this primitive
    /// intentionally needs no [`Capability`](crate::Capability) bit.
    async fn get_batch(
        &self,
        paths: Vec<VirtualPath>,
    ) -> Result<Vec<Option<VersionedEntry>>, FilesystemError> {
        if paths.is_empty() {
            return Ok(Vec::new());
        }
        if paths.len() > MAX_BATCH_GETS {
            return Err(FilesystemError::BackendInfrastructure {
                operation: FilesystemOperation::ReadFile,
                reason: "batch exceeds MAX_BATCH_GETS".to_string(),
            });
        }
        let mut out = Vec::with_capacity(paths.len());
        for path in &paths {
            out.push(self.get(path).await?);
        }
        Ok(out)
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

    /// Append multiple `payloads` to the event log at `path` in one backend
    /// round-trip, returning the assigned monotonic [`SeqNo`]s in payload
    /// order. All payloads target the **same** `path`.
    ///
    /// The return type promises atomic all-or-nothing semantics: on success all
    /// seqs are assigned; on error none are. A per-item loop cannot honor this
    /// contract — if the loop commits some payloads and then fails, earlier
    /// writes are already durable but the caller only sees an `Err` with no
    /// record of which seqs committed. Retrying the whole batch then duplicates
    /// the committed prefix, a silent correctness bug.
    ///
    /// Therefore the default is [`Unsupported`](FilesystemError::Unsupported).
    /// Backends that can write a batch atomically (Postgres/libSQL multi-row
    /// INSERT, in-memory) **must** override this method. Ordering is preserved:
    /// the assigned seqs must be monotonic in payload order.
    ///
    /// [`CompositeRootFilesystem`](crate::CompositeRootFilesystem) overrides
    /// this to forward to the resolved mount's `append_batch`, so callers
    /// going through the composite dispatcher will reach the backend override.
    async fn append_batch(
        &self,
        path: &VirtualPath,
        payloads: Vec<Vec<u8>>,
    ) -> Result<Vec<SeqNo>, FilesystemError> {
        let _ = payloads;
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

    /// Read at most `max_records` events at `path` starting at `from`
    /// (exclusive).
    ///
    /// Backends with native paging should override this so consumers do not
    /// materialize the full tail before applying their replay limit.
    async fn tail_bounded(
        &self,
        path: &VirtualPath,
        from: SeqNo,
        max_records: usize,
    ) -> Result<Vec<EventRecord>, FilesystemError> {
        let mut records = self.tail(path, from).await?;
        records.truncate(max_records);
        Ok(records)
    }

    /// Return the highest seq present at `path` with `seq > from`, or `None`
    /// when no such record exists. This is the head/replay-boundary probe used
    /// by durable event logs at subscription start.
    ///
    /// The default impl is a correctness-preserving fallback that routes
    /// through [`tail`](Self::tail) and takes the max observed seq, which
    /// materializes the gap into memory. Backends with a native max-seq query
    /// (Postgres, libSQL) MUST override this with an O(1) `MAX(seq)` lookup so
    /// a new subscription (`from = 0`) does not load the whole stream just to
    /// find its head.
    async fn head_seq(
        &self,
        path: &VirtualPath,
        from: SeqNo,
    ) -> Result<Option<SeqNo>, FilesystemError> {
        let records = self.tail(path, from).await?;
        Ok(records.into_iter().map(|record| record.seq).max())
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

#[cfg(test)]
mod tests {
    use super::*;

    struct DefaultBoundedBackend;

    #[async_trait::async_trait]
    impl RootFilesystem for DefaultBoundedBackend {
        async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
            Ok(["a", "b", "c"]
                .into_iter()
                .map(|name| DirEntry {
                    name: name.to_string(),
                    path: VirtualPath::new(format!("{}/{}", path.as_str(), name)).unwrap(),
                    file_type: crate::FileType::File,
                })
                .collect())
        }

        async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
            Err(FilesystemError::NotFound {
                path: path.clone(),
                operation: FilesystemOperation::Stat,
            })
        }

        async fn tail(
            &self,
            _path: &VirtualPath,
            _from: SeqNo,
        ) -> Result<Vec<EventRecord>, FilesystemError> {
            Ok((1..=3)
                .map(|seq| EventRecord {
                    seq: SeqNo::from_backend(seq),
                    payload: vec![seq as u8],
                })
                .collect())
        }
    }

    #[tokio::test]
    async fn list_dir_bounded_default_truncates_materialized_entries() {
        let backend = DefaultBoundedBackend;
        let path = VirtualPath::new("/projects").unwrap();

        let none = backend.list_dir_bounded(&path, 0).await.unwrap();
        let all = backend.list_dir_bounded(&path, 10).await.unwrap();

        assert!(none.is_empty());
        assert_eq!(all.len(), 3);
        assert_eq!(all[2].name, "c");
    }

    #[tokio::test]
    async fn tail_bounded_default_truncates_materialized_records() {
        let backend = DefaultBoundedBackend;
        let path = VirtualPath::new("/events").unwrap();

        let none = backend.tail_bounded(&path, SeqNo::ZERO, 0).await.unwrap();
        let first_two = backend.tail_bounded(&path, SeqNo::ZERO, 2).await.unwrap();

        assert!(none.is_empty());
        assert_eq!(first_two.len(), 2);
        assert_eq!(first_two[1].seq, SeqNo::from_backend(2));
    }
}
