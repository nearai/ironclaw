use std::time::SystemTime;

use ironclaw_host_api::{HostApiError, ScopedPath, VirtualPath};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::index::IndexName;
use crate::record::RecordVersion;

/// Filesystem operation used for permission checks and audit/error reporting.
///
/// The legacy byte-plane variants (`ReadFile`, `WriteFile`, …) describe the
/// *intent* of an operation against the underlying [`MountPermissions`]
/// surface and are reused by the unified `put`/`get` ops as their permission
/// witness — `put` is a write, `get` is a read. The newer variants
/// (`Query`, `EnsureIndex`, `BeginTxn`, `Tail`) describe operations that have
/// no analogue in the legacy enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilesystemOperation {
    MountLocal,
    ReadFile,
    WriteFile,
    AppendFile,
    ListDir,
    Stat,
    Delete,
    CreateDirAll,
    Query,
    EnsureIndex,
    BeginTxn,
    Tail,
}

impl std::fmt::Display for FilesystemOperation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::MountLocal => "mount_local",
            Self::ReadFile => "read_file",
            Self::WriteFile => "write_file",
            Self::AppendFile => "append_file",
            Self::ListDir => "list_dir",
            Self::Stat => "stat",
            Self::Delete => "delete",
            Self::CreateDirAll => "create_dir_all",
            Self::Query => "query",
            Self::EnsureIndex => "ensure_index",
            Self::BeginTxn => "begin_txn",
            Self::Tail => "tail",
        })
    }
}

/// Filesystem service failures.
///
/// Display output intentionally uses scoped/virtual paths rather than raw host
/// paths. Backend implementations may log lower-level errors separately, but
/// user-facing errors should preserve host path confidentiality.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum FilesystemError {
    #[error(transparent)]
    Contract(#[from] HostApiError),
    #[error("permission denied for {operation} on scoped path {path}")]
    PermissionDenied {
        path: ScopedPath,
        operation: FilesystemOperation,
    },
    #[error("no backend mount found for virtual path {path}")]
    MountNotFound { path: VirtualPath },
    #[error("virtual path not found for {operation} at {path}")]
    NotFound {
        path: VirtualPath,
        operation: FilesystemOperation,
    },
    #[error("virtual path escaped backend mount {path}")]
    PathOutsideMount { path: VirtualPath },
    #[error("symlink escapes backend mount at virtual path {path}")]
    SymlinkEscape { path: VirtualPath },
    #[error("backend mount conflict at virtual path {path}")]
    MountConflict { path: VirtualPath },
    #[error("filesystem backend error during {operation} at {path}: {reason}")]
    Backend {
        path: VirtualPath,
        operation: FilesystemOperation,
        reason: String,
    },
    /// Compare-and-swap precondition failed: the existing record's version did
    /// not match the caller's expectation. Stores typically retry by reading
    /// the current version and re-applying the transformation.
    #[error("version mismatch at {path}: expected {expected:?}, found {found:?}")]
    VersionMismatch {
        path: VirtualPath,
        expected: Option<RecordVersion>,
        found: Option<RecordVersion>,
    },
    /// Mounted backend does not implement the requested operation. Capability
    /// checks at mount time should catch most cases; this remains for
    /// runtime-conditional capabilities (e.g. a Postgres mount built against a
    /// server without `pgvector` rejecting `IndexKind::Vector`).
    #[error("operation {operation} is not supported by the mount at {path}")]
    Unsupported {
        path: VirtualPath,
        operation: FilesystemOperation,
    },
    /// Declaring an index conflicted with an existing definition (e.g. the
    /// same name already exists with a different `keys` ordering or `kind`).
    #[error("index conflict for {name} at {path}: {reason:?}")]
    IndexConflict {
        path: VirtualPath,
        name: IndexName,
        reason: IndexConflictReason,
    },
    /// Mount descriptor advertised capabilities the backend doesn't provide
    /// on the new capability axes (records / query / index / events) or the
    /// transaction tier. Surfaced at mount time so consumers see the
    /// shortfall before any op-time `Unsupported` arrives.
    #[error(
        "mount descriptor at {path} claims capabilities the backend does not provide: \
         missing={missing:?}, txn_shortfall={txn_shortfall}"
    )]
    DescriptorOverclaims {
        path: VirtualPath,
        missing: Vec<Capability>,
        txn_shortfall: bool,
    },
    /// JSON serialization of an [`Entry::indexed`](crate::Entry::indexed)
    /// projection failed. Indicates a non-serializable value managed to slip
    /// into the indexed map.
    #[error("failed to serialize indexed projection for {operation} at {path}")]
    SerializeIndexed {
        path: VirtualPath,
        operation: FilesystemOperation,
    },
    /// JSON deserialization of an indexed projection round-tripping out of
    /// storage failed. Indicates either schema drift or a backend that
    /// produced a malformed payload.
    #[error("failed to deserialize indexed projection for {operation} at {path}")]
    DeserializeIndexed {
        path: VirtualPath,
        operation: FilesystemOperation,
    },
    /// A persisted [`RecordVersion`](crate::RecordVersion) read back as a
    /// value outside `u64`'s range (e.g. negative on a libSQL `INTEGER`
    /// column). Symptom of schema corruption or unsafe direct SQL writes —
    /// surface as a backend error rather than silently masking to `0`.
    #[error("corrupt record version {raw} at {path}")]
    CorruptRecordVersion { path: VirtualPath, raw: i64 },
    /// Sentinel "this can't happen" surface: an `INSERT ... ON CONFLICT DO
    /// NOTHING` followed by a `SELECT` returned no row. The only way this
    /// fires is if a concurrent `DELETE` from outside the index-spec code
    /// path raced with the ensure_index call.
    #[error("index spec disappeared after upsert at {path}: {name}")]
    IndexSpecMissingAfterUpsert { path: VirtualPath, name: IndexName },
}

/// Reason a [`FilesystemError::IndexConflict`] was raised.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexConflictReason {
    /// The existing spec has different `keys` or a different `kind` from the
    /// one declared in this call.
    SpecMismatch,
    /// The declaration has an empty `keys` list. Indexes must declare at
    /// least one key.
    EmptyKeys,
}

/// Coarse file type returned by [`FileStat`] and [`DirEntry`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileType {
    File,
    Directory,
    Symlink,
    Other,
}

/// Directory entry returned by [`RootFilesystem::list_dir`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntry {
    pub name: String,
    pub path: VirtualPath,
    pub file_type: FileType,
}

/// File metadata returned by [`RootFilesystem::stat`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileStat {
    pub path: VirtualPath,
    pub file_type: FileType,
    pub len: u64,
    pub modified: Option<SystemTime>,
    pub sensitive: bool,
}

/// Stable identifier for a mounted filesystem backend.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BackendId(String);

impl BackendId {
    pub fn new(value: impl Into<String>) -> Result<Self, HostApiError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(HostApiError::InvalidId {
                kind: "filesystem backend",
                value,
                reason: "backend id must not be empty".to_string(),
            });
        }
        if value.contains('/')
            || value.contains('\\')
            || value.contains('\0')
            || value.chars().any(char::is_control)
        {
            return Err(HostApiError::InvalidId {
                kind: "filesystem backend",
                value,
                reason: "backend id must be a simple non-path identifier".to_string(),
            });
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Coarse class of backend implementation behind a virtual mount.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendKind {
    LocalFilesystem,
    DatabaseFilesystem,
    MemoryDocuments,
    ObjectStore,
    Custom(String),
}

/// Storage shape represented by a mount.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageClass {
    /// File-like contents addressed by virtual paths.
    FileContent,
    /// Structured records that may expose file-shaped projections.
    StructuredRecords,
    /// Derived data such as chunks, indexes, or embeddings.
    DerivedProjection,
}

/// Semantic kind of content exposed at a mount.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentKind {
    GenericFile,
    ProjectFile,
    Artifact,
    MemoryDocument,
    SystemState,
    ExtensionPackage,
    StructuredRecord,
}

/// Indexing/embedding policy associated with file-shaped content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexPolicy {
    NotIndexed,
    FullText,
    Vector,
    FullTextAndVector,
    BackendDefined,
}

/// Index kinds a backend can materialize when it serves the record plane.
///
/// Individual capability a backend can advertise.
///
/// Each variant is a single bit in the [`BackendCapabilities`] bitmask.
/// Transaction semantics are richer than a single bit (ordered: None → Cas →
/// MultiKey) and live in [`TxnCapability`] alongside the bitmask.
///
/// `read`/`write`/`append`/`list`/`stat`/`delete` predate the unified surface
/// and are kept here so existing catalog metadata still round-trips; the
/// mount-time validator in
/// [`CompositeRootFilesystem`](crate::CompositeRootFilesystem) only enforces
/// the **new** capability axes (Records, Query, Index*, Events) until each
/// backend opts into an accurate `capabilities()` override.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    // Legacy bytes plane.
    Read,
    Write,
    Append,
    List,
    Stat,
    Delete,
    // Record plane.
    Records,
    Query,
    // Index kinds.
    IndexExact,
    IndexPrefix,
    IndexFts,
    IndexVector,
    // Event plane (`append`/`tail`).
    Events,
}

impl Capability {
    const fn bit(self) -> u32 {
        1 << (self as u32)
    }

    /// Iterator over every capability. Useful for serialization and
    /// validation reporting.
    pub fn all() -> &'static [Capability] {
        &[
            Capability::Read,
            Capability::Write,
            Capability::Append,
            Capability::List,
            Capability::Stat,
            Capability::Delete,
            Capability::Records,
            Capability::Query,
            Capability::IndexExact,
            Capability::IndexPrefix,
            Capability::IndexFts,
            Capability::IndexVector,
            Capability::Events,
        ]
    }
}

/// Transaction semantics offered by a backend.
///
/// Stores must work with `Cas` as the floor; richer backends opt into
/// `MultiKey` for stronger guarantees, but consumers never *depend* on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TxnCapability {
    #[default]
    None,
    /// Compare-and-swap on individual records (see
    /// [`CasExpectation`](crate::CasExpectation)).
    Cas,
    /// Backend implements [`StorageTxn`](crate::StorageTxn) for atomic
    /// multi-key updates within a single mount.
    MultiKey,
}

/// Capabilities advertised by a mounted backend for diagnostics and routing.
///
/// Stored as a compact `u32` bitmask over [`Capability`] plus an ordered
/// [`TxnCapability`]. Build with `BackendCapabilities::empty().with(...)` —
/// or use one of the `sql_typical` / `in_memory_full` / `bytes_only`
/// convenience constructors. Mount-time validation in
/// [`CompositeRootFilesystem`](crate::CompositeRootFilesystem) refuses
/// backends whose capabilities don't satisfy the descriptor's claims on the
/// new axes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BackendCapabilities {
    flags: u32,
    txn: TxnCapability,
}

impl BackendCapabilities {
    /// No capabilities advertised — the safe default.
    pub const fn empty() -> Self {
        Self {
            flags: 0,
            txn: TxnCapability::None,
        }
    }

    /// Add a [`Capability`] flag.
    pub const fn with(mut self, cap: Capability) -> Self {
        self.flags |= cap.bit();
        self
    }

    /// Remove a [`Capability`] flag.
    pub const fn without(mut self, cap: Capability) -> Self {
        self.flags &= !cap.bit();
        self
    }

    /// Set the transaction capability tier.
    pub const fn with_txn(mut self, txn: TxnCapability) -> Self {
        self.txn = txn;
        self
    }

    /// Does this backend advertise `cap`?
    pub const fn has(&self, cap: Capability) -> bool {
        self.flags & cap.bit() != 0
    }

    /// Transaction tier.
    pub const fn txn(&self) -> TxnCapability {
        self.txn
    }

    /// All capabilities currently set, in [`Capability::all`] order.
    pub fn iter(&self) -> impl Iterator<Item = Capability> + '_ {
        Capability::all().iter().copied().filter(|c| self.has(*c))
    }

    /// Convenience: read + write + list + stat + delete + records + query
    /// + IndexExact + IndexPrefix + CAS — the typical SQL backend shape.
    pub const fn sql_typical() -> Self {
        Self::empty()
            .with(Capability::Read)
            .with(Capability::Write)
            .with(Capability::Append)
            .with(Capability::List)
            .with(Capability::Stat)
            .with(Capability::Delete)
            .with(Capability::Records)
            .with(Capability::Query)
            .with(Capability::IndexExact)
            .with(Capability::IndexPrefix)
            .with_txn(TxnCapability::Cas)
    }

    /// Convenience: every capability the in-memory reference backend
    /// implements. Includes Events on top of `sql_typical`.
    pub const fn in_memory_full() -> Self {
        Self::sql_typical().with(Capability::Events)
    }

    /// Convenience: read + write + append + list + stat + delete only.
    /// Matches a byte-only backend that hasn't yet opted into records.
    pub const fn bytes_only() -> Self {
        Self::empty()
            .with(Capability::Read)
            .with(Capability::Write)
            .with(Capability::Append)
            .with(Capability::List)
            .with(Capability::Stat)
            .with(Capability::Delete)
    }
}

/// Serialize/deserialize as `{ caps: [...], txn: "..." }` so the on-the-wire
/// shape stays readable rather than leaking the bitmask integer. Decoder
/// silently ignores unknown capability strings — a backend that advertises a
/// future variant against an older reader degrades to "missing" rather than
/// failing parse.
impl Serialize for BackendCapabilities {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Wire<'a> {
            caps: Vec<Capability>,
            txn: &'a TxnCapability,
        }
        Wire {
            caps: self.iter().collect(),
            txn: &self.txn,
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for BackendCapabilities {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Wire {
            #[serde(default)]
            caps: Vec<Capability>,
            #[serde(default)]
            txn: TxnCapability,
        }
        let wire = Wire::deserialize(deserializer)?;
        let mut out = BackendCapabilities::empty().with_txn(wire.txn);
        for cap in wire.caps {
            out = out.with(cap);
        }
        Ok(out)
    }
}
