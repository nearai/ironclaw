use std::time::SystemTime;

use ironclaw_host_api::{HostApiError, ScopedPath, VirtualPath};
use thiserror::Error;

/// Filesystem operation used for permission checks and audit/error reporting.
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
        })
    }
}

/// Filesystem service failures.
///
/// Display output intentionally uses scoped/virtual paths rather than raw host
/// paths. Backend implementations may log lower-level errors separately, but
/// user-facing errors should preserve host path confidentiality.
#[derive(Debug, Error)]
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

/// Capabilities advertised by a mounted backend for diagnostics and routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BackendCapabilities {
    pub read: bool,
    pub write: bool,
    pub append: bool,
    pub list: bool,
    pub stat: bool,
    pub delete: bool,
    pub indexed: bool,
    pub embedded: bool,
}
