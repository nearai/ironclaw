//! Scoped filesystem service for IronClaw Reborn.
//!
//! `ironclaw_filesystem` is the first service crate above
//! `ironclaw_host_api`. It resolves runtime-visible [`ScopedPath`] values
//! through a caller's [`MountView`], checks mount permissions, then performs the
//! operation against a trusted root filesystem namespace addressed by
//! [`VirtualPath`]. Backend implementations alone touch raw host paths.
//!
//! The local backend canonicalizes existing paths and their nearest existing
//! ancestors before opening files, and it re-roots new leaf paths on the checked
//! canonical parent. That narrows symlink escape opportunities but does not
//! provide a kernel-enforced race-free guarantee against a writable mount root
//! being modified between containment checks and opens. Production hardening for
//! hostile local directories should use fd-relative traversal such as `openat2`
//! with `RESOLVE_BENEATH`, `O_NOFOLLOW`, or a capability filesystem crate.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::{
    HostApiError, HostPath, MountPermissions, MountView, ScopedPath, VirtualPath,
};
use thiserror::Error;
use tokio::io::AsyncWriteExt;

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

/// Trusted catalog record for one virtual filesystem mount.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountDescriptor {
    pub virtual_root: VirtualPath,
    pub backend_id: BackendId,
    pub backend_kind: BackendKind,
    pub storage_class: StorageClass,
    pub content_kind: ContentKind,
    pub index_policy: IndexPolicy,
    pub capabilities: BackendCapabilities,
}

/// Catalog answer for the backend that owns a virtual path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathPlacement {
    pub path: VirtualPath,
    pub matched_root: VirtualPath,
    pub backend_id: BackendId,
    pub backend_kind: BackendKind,
    pub storage_class: StorageClass,
    pub content_kind: ContentKind,
    pub index_policy: IndexPolicy,
    pub capabilities: BackendCapabilities,
}

impl PathPlacement {
    fn from_descriptor(path: VirtualPath, descriptor: &MountDescriptor) -> Self {
        Self {
            path,
            matched_root: descriptor.virtual_root.clone(),
            backend_id: descriptor.backend_id.clone(),
            backend_kind: descriptor.backend_kind.clone(),
            storage_class: descriptor.storage_class,
            content_kind: descriptor.content_kind,
            index_policy: descriptor.index_policy,
            capabilities: descriptor.capabilities,
        }
    }
}

/// Trusted catalog over virtual filesystem mount placement.
///
/// The catalog explains where a [`VirtualPath`] is placed; it does not grant
/// runtime access. Untrusted callers must still go through [`ScopedFilesystem`]
/// and a scoped [`MountView`].
#[async_trait]
pub trait FilesystemCatalog: Send + Sync {
    async fn describe_path(&self, path: &VirtualPath) -> Result<PathPlacement, FilesystemError>;

    async fn mounts(&self) -> Result<Vec<MountDescriptor>, FilesystemError>;
}

/// Root filesystem that composes multiple backend roots behind one virtual namespace.
pub struct CompositeRootFilesystem {
    mounts: Vec<CompositeMount>,
}

struct CompositeMount {
    descriptor: MountDescriptor,
    backend: Arc<dyn RootFilesystem>,
}

impl CompositeRootFilesystem {
    pub fn new() -> Self {
        Self { mounts: Vec::new() }
    }

    pub fn mount<F>(
        &mut self,
        descriptor: MountDescriptor,
        backend: Arc<F>,
    ) -> Result<(), FilesystemError>
    where
        F: RootFilesystem + 'static,
    {
        let backend: Arc<dyn RootFilesystem> = backend;
        self.mount_dyn(descriptor, backend)
    }

    pub fn mount_dyn(
        &mut self,
        descriptor: MountDescriptor,
        backend: Arc<dyn RootFilesystem>,
    ) -> Result<(), FilesystemError> {
        if self
            .mounts
            .iter()
            .any(|mount| mount.descriptor.virtual_root.as_str() == descriptor.virtual_root.as_str())
        {
            return Err(FilesystemError::MountConflict {
                path: descriptor.virtual_root,
            });
        }
        self.mounts.push(CompositeMount {
            descriptor,
            backend,
        });
        Ok(())
    }

    fn matching_mount(&self, path: &VirtualPath) -> Result<&CompositeMount, FilesystemError> {
        self.mounts
            .iter()
            .filter(|mount| {
                path_prefix_matches(mount.descriptor.virtual_root.as_str(), path.as_str())
            })
            .max_by_key(|mount| mount.descriptor.virtual_root.as_str().len())
            .ok_or_else(|| FilesystemError::MountNotFound { path: path.clone() })
    }
}

impl Default for CompositeRootFilesystem {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl FilesystemCatalog for CompositeRootFilesystem {
    async fn describe_path(&self, path: &VirtualPath) -> Result<PathPlacement, FilesystemError> {
        let mount = self.matching_mount(path)?;
        Ok(PathPlacement::from_descriptor(
            path.clone(),
            &mount.descriptor,
        ))
    }

    async fn mounts(&self) -> Result<Vec<MountDescriptor>, FilesystemError> {
        let mut mounts: Vec<_> = self
            .mounts
            .iter()
            .map(|mount| mount.descriptor.clone())
            .collect();
        mounts.sort_by(|left, right| left.virtual_root.as_str().cmp(right.virtual_root.as_str()));
        Ok(mounts)
    }
}

#[async_trait]
impl RootFilesystem for CompositeRootFilesystem {
    async fn read_file(&self, path: &VirtualPath) -> Result<Vec<u8>, FilesystemError> {
        self.matching_mount(path)?.backend.read_file(path).await
    }

    async fn write_file(&self, path: &VirtualPath, bytes: &[u8]) -> Result<(), FilesystemError> {
        self.matching_mount(path)?
            .backend
            .write_file(path, bytes)
            .await
    }

    async fn append_file(&self, path: &VirtualPath, bytes: &[u8]) -> Result<(), FilesystemError> {
        self.matching_mount(path)?
            .backend
            .append_file(path, bytes)
            .await
    }

    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        self.matching_mount(path)?.backend.list_dir(path).await
    }

    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        self.matching_mount(path)?.backend.stat(path).await
    }

    async fn delete(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.matching_mount(path)?.backend.delete(path).await
    }

    async fn create_dir_all(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.matching_mount(path)?
            .backend
            .create_dir_all(path)
            .await
    }
}

/// Trusted root filesystem interface over canonical virtual paths.
#[async_trait]
pub trait RootFilesystem: Send + Sync {
    /// Reads a file by canonical virtual path without exposing backend host paths in errors.
    async fn read_file(&self, path: &VirtualPath) -> Result<Vec<u8>, FilesystemError>;

    /// Writes bytes to a canonical virtual path while preserving backend containment.
    async fn write_file(&self, path: &VirtualPath, bytes: &[u8]) -> Result<(), FilesystemError>;

    /// Appends bytes to a canonical virtual path. Backends that do not support append must fail closed before side effects.
    async fn append_file(&self, path: &VirtualPath, _bytes: &[u8]) -> Result<(), FilesystemError> {
        Err(FilesystemError::Backend {
            path: path.clone(),
            operation: FilesystemOperation::AppendFile,
            reason: "append_file is not supported by this backend".to_string(),
        })
    }

    /// Lists direct children of a canonical virtual directory; callers must handle pagination/backends in future implementations without bypassing scope.
    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError>;

    /// Returns metadata for a canonical virtual path without revealing raw host paths.
    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError>;

    /// Deletes an existing canonical virtual file or directory. Missing paths return [`FilesystemError::NotFound`]; backends that do not support delete must fail closed before side effects.
    async fn delete(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        Err(FilesystemError::Backend {
            path: path.clone(),
            operation: FilesystemOperation::Delete,
            reason: "delete is not supported by this backend".to_string(),
        })
    }

    /// Creates a canonical virtual directory and any missing parents. Backends that do not support directories must fail closed before side effects.
    async fn create_dir_all(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        Err(FilesystemError::Backend {
            path: path.clone(),
            operation: FilesystemOperation::CreateDirAll,
            reason: "create_dir_all is not supported by this backend".to_string(),
        })
    }
}

/// Invocation-scoped filesystem view over [`ScopedPath`] values.
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

    pub async fn read_file(&self, path: &ScopedPath) -> Result<Vec<u8>, FilesystemError> {
        let virtual_path = self.resolve_with_permission(path, FilesystemOperation::ReadFile)?;
        self.root.read_file(&virtual_path).await
    }

    pub async fn write_file(&self, path: &ScopedPath, bytes: &[u8]) -> Result<(), FilesystemError> {
        let virtual_path = self.resolve_with_permission(path, FilesystemOperation::WriteFile)?;
        self.root.write_file(&virtual_path, bytes).await
    }

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

    pub async fn create_dir_all(&self, path: &ScopedPath) -> Result<(), FilesystemError> {
        let virtual_path = self.resolve_with_permission(path, FilesystemOperation::CreateDirAll)?;
        self.root.create_dir_all(&virtual_path).await
    }

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
    }
}

/// Local filesystem backend mounted into the virtual namespace.
#[derive(Debug, Default)]
pub struct LocalFilesystem {
    mounts: Vec<LocalMount>,
}

#[derive(Debug, Clone)]
struct LocalMount {
    virtual_root: VirtualPath,
    host_root: PathBuf,
}

impl LocalFilesystem {
    pub fn new() -> Self {
        Self::default()
    }

    /// Mounts a host directory during trusted setup.
    ///
    /// This API is intentionally synchronous because it mutates in-memory mount
    /// configuration and is not part of the async runtime operation path. Async
    /// file operations after mount setup use `tokio::fs`.
    pub fn mount_local(
        &mut self,
        virtual_root: VirtualPath,
        host_root: HostPath,
    ) -> Result<(), FilesystemError> {
        if self
            .mounts
            .iter()
            .any(|mount| mount.virtual_root.as_str() == virtual_root.as_str())
        {
            return Err(FilesystemError::MountConflict { path: virtual_root });
        }

        let canonical_root = std::fs::canonicalize(host_root.as_path()).map_err(|error| {
            FilesystemError::Backend {
                path: virtual_root.clone(),
                operation: FilesystemOperation::MountLocal,
                reason: io_reason(error),
            }
        })?;

        if !canonical_root.is_dir() {
            return Err(FilesystemError::Backend {
                path: virtual_root,
                operation: FilesystemOperation::MountLocal,
                reason: "host root is not a directory".to_string(),
            });
        }

        self.mounts.push(LocalMount {
            virtual_root,
            host_root: canonical_root,
        });
        Ok(())
    }

    async fn resolve_existing(
        &self,
        path: &VirtualPath,
        operation: FilesystemOperation,
    ) -> Result<PathBuf, FilesystemError> {
        let (mount, joined) = self.resolve_joined(path)?;
        let canonical = tokio::fs::canonicalize(&joined)
            .await
            .map_err(|error| io_error(path.clone(), operation, error))?;
        ensure_contained(path, mount, &canonical, true)?;
        Ok(canonical)
    }

    async fn resolve_for_write(
        &self,
        path: &VirtualPath,
        operation: FilesystemOperation,
    ) -> Result<PathBuf, FilesystemError> {
        let (mount, joined) = self.resolve_joined(path)?;

        if tokio::fs::try_exists(&joined)
            .await
            .map_err(|error| io_error(path.clone(), operation, error))?
        {
            let canonical = tokio::fs::canonicalize(&joined)
                .await
                .map_err(|error| io_error(path.clone(), operation, error))?;
            ensure_contained(path, mount, &canonical, true)?;
            return Ok(canonical);
        }

        let parent = joined
            .parent()
            .ok_or_else(|| FilesystemError::PathOutsideMount { path: path.clone() })?;
        ensure_existing_ancestor_contained(path, mount, parent, operation).await?;
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|error| io_error(path.clone(), FilesystemOperation::CreateDirAll, error))?;
        let canonical_parent = tokio::fs::canonicalize(parent)
            .await
            .map_err(|error| io_error(path.clone(), FilesystemOperation::CreateDirAll, error))?;
        // `joined` is constructed from validated virtual path segments under the
        // backend root. If its canonical parent leaves the backend root, an
        // existing symlink in the parent chain caused the escape.
        ensure_contained(path, mount, &canonical_parent, true)?;
        // Re-root the final path on the canonicalized, containment-checked
        // parent rather than returning `joined` (which still contains the
        // un-canonicalized ancestor components). This narrows the TOCTOU
        // window between the containment check and the eventual write — a
        // later swap of an ancestor symlink does not change the path we hand
        // back. Robust defense (openat / O_NOFOLLOW / cap-std) is tracked as a
        // follow-up; see PR #2996 review.
        let file_name = joined
            .file_name()
            .ok_or_else(|| FilesystemError::PathOutsideMount { path: path.clone() })?;
        Ok(canonical_parent.join(file_name))
    }

    async fn resolve_for_create_dir_all(
        &self,
        path: &VirtualPath,
    ) -> Result<PathBuf, FilesystemError> {
        let (mount, joined) = self.resolve_joined(path)?;
        ensure_existing_ancestor_contained(path, mount, &joined, FilesystemOperation::CreateDirAll)
            .await?;
        tokio::fs::create_dir_all(&joined)
            .await
            .map_err(|error| io_error(path.clone(), FilesystemOperation::CreateDirAll, error))?;
        let canonical = tokio::fs::canonicalize(&joined)
            .await
            .map_err(|error| io_error(path.clone(), FilesystemOperation::CreateDirAll, error))?;
        ensure_contained(path, mount, &canonical, true)?;
        Ok(canonical)
    }

    fn resolve_joined(
        &self,
        path: &VirtualPath,
    ) -> Result<(&LocalMount, PathBuf), FilesystemError> {
        let mount = self
            .mounts
            .iter()
            .filter(|mount| path_prefix_matches(mount.virtual_root.as_str(), path.as_str()))
            .max_by_key(|mount| mount.virtual_root.as_str().len())
            .ok_or_else(|| FilesystemError::MountNotFound { path: path.clone() })?;

        let tail = path
            .as_str()
            .strip_prefix(mount.virtual_root.as_str())
            .unwrap_or_default()
            .trim_start_matches('/');

        let mut joined = mount.host_root.clone();
        if !tail.is_empty() {
            for segment in tail.split('/') {
                joined.push(segment);
            }
        }
        Ok((mount, joined))
    }
}

#[async_trait]
impl RootFilesystem for LocalFilesystem {
    async fn read_file(&self, path: &VirtualPath) -> Result<Vec<u8>, FilesystemError> {
        let resolved = self
            .resolve_existing(path, FilesystemOperation::ReadFile)
            .await?;
        tokio::fs::read(resolved)
            .await
            .map_err(|error| io_error(path.clone(), FilesystemOperation::ReadFile, error))
    }

    async fn write_file(&self, path: &VirtualPath, bytes: &[u8]) -> Result<(), FilesystemError> {
        let resolved = self
            .resolve_for_write(path, FilesystemOperation::WriteFile)
            .await?;
        tokio::fs::write(resolved, bytes)
            .await
            .map_err(|error| io_error(path.clone(), FilesystemOperation::WriteFile, error))
    }

    async fn append_file(&self, path: &VirtualPath, bytes: &[u8]) -> Result<(), FilesystemError> {
        let resolved = self
            .resolve_for_write(path, FilesystemOperation::AppendFile)
            .await?;
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .write(true)
            .open(resolved)
            .await
            .map_err(|error| io_error(path.clone(), FilesystemOperation::AppendFile, error))?;
        file.write_all(bytes)
            .await
            .map_err(|error| io_error(path.clone(), FilesystemOperation::AppendFile, error))?;
        file.flush()
            .await
            .map_err(|error| io_error(path.clone(), FilesystemOperation::AppendFile, error))
    }

    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        let resolved = self
            .resolve_existing(path, FilesystemOperation::ListDir)
            .await?;
        let mut read_dir = tokio::fs::read_dir(resolved)
            .await
            .map_err(|error| io_error(path.clone(), FilesystemOperation::ListDir, error))?;
        let mut entries = Vec::new();
        while let Some(entry) = read_dir
            .next_entry()
            .await
            .map_err(|error| io_error(path.clone(), FilesystemOperation::ListDir, error))?
        {
            let name = entry.file_name().to_string_lossy().to_string();
            let entry_path =
                VirtualPath::new(format!("{}/{}", path.as_str().trim_end_matches('/'), name))?;
            let metadata = entry
                .metadata()
                .await
                .map_err(|error| io_error(entry_path.clone(), FilesystemOperation::Stat, error))?;
            entries.push(DirEntry {
                name,
                path: entry_path,
                file_type: file_type_from_metadata(&metadata),
            });
        }
        entries.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(entries)
    }

    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        let resolved = self
            .resolve_existing(path, FilesystemOperation::Stat)
            .await?;
        let metadata = tokio::fs::metadata(resolved)
            .await
            .map_err(|error| io_error(path.clone(), FilesystemOperation::Stat, error))?;
        Ok(FileStat {
            path: path.clone(),
            file_type: file_type_from_metadata(&metadata),
            len: metadata.len(),
        })
    }

    async fn delete(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        let resolved = self
            .resolve_existing(path, FilesystemOperation::Delete)
            .await?;
        let metadata = tokio::fs::metadata(&resolved)
            .await
            .map_err(|error| io_error(path.clone(), FilesystemOperation::Delete, error))?;
        let result = if metadata.is_dir() {
            tokio::fs::remove_dir_all(resolved).await
        } else {
            tokio::fs::remove_file(resolved).await
        };
        result.map_err(|error| io_error(path.clone(), FilesystemOperation::Delete, error))
    }

    async fn create_dir_all(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.resolve_for_create_dir_all(path).await.map(|_| ())
    }
}

fn path_prefix_matches(prefix: &str, path: &str) -> bool {
    path == prefix || path.starts_with(&format!("{prefix}/"))
}

async fn ensure_existing_ancestor_contained(
    virtual_path: &VirtualPath,
    mount: &LocalMount,
    candidate: &Path,
    operation: FilesystemOperation,
) -> Result<(), FilesystemError> {
    let mut ancestor = candidate.to_path_buf();
    while !tokio::fs::try_exists(&ancestor)
        .await
        .map_err(|error| io_error(virtual_path.clone(), operation, error))?
    {
        ancestor = ancestor
            .parent()
            .ok_or_else(|| FilesystemError::PathOutsideMount {
                path: virtual_path.clone(),
            })?
            .to_path_buf();
    }
    let canonical = tokio::fs::canonicalize(&ancestor)
        .await
        .map_err(|error| io_error(virtual_path.clone(), operation, error))?;
    ensure_contained(virtual_path, mount, &canonical, true)
}

fn ensure_contained(
    virtual_path: &VirtualPath,
    mount: &LocalMount,
    candidate: &Path,
    existing_target: bool,
) -> Result<(), FilesystemError> {
    if candidate.starts_with(&mount.host_root) {
        Ok(())
    } else if existing_target {
        Err(FilesystemError::SymlinkEscape {
            path: virtual_path.clone(),
        })
    } else {
        Err(FilesystemError::PathOutsideMount {
            path: virtual_path.clone(),
        })
    }
}

fn file_type_from_metadata(metadata: &std::fs::Metadata) -> FileType {
    let file_type = metadata.file_type();
    if file_type.is_file() {
        FileType::File
    } else if file_type.is_dir() {
        FileType::Directory
    } else if file_type.is_symlink() {
        FileType::Symlink
    } else {
        FileType::Other
    }
}

fn io_error(
    path: VirtualPath,
    operation: FilesystemOperation,
    error: std::io::Error,
) -> FilesystemError {
    tracing::debug!(
        virtual_path = path.as_str(),
        %operation,
        error = %error,
        "local filesystem backend error"
    );
    if error.kind() == std::io::ErrorKind::NotFound {
        FilesystemError::NotFound { path, operation }
    } else {
        FilesystemError::Backend {
            path,
            operation,
            reason: error.kind().to_string(),
        }
    }
}

fn io_reason(error: std::io::Error) -> String {
    error.kind().to_string()
}

#[cfg(feature = "postgres")]
/// PostgreSQL-backed [`RootFilesystem`] storing file contents by virtual path.
pub struct PostgresRootFilesystem {
    pool: deadpool_postgres::Pool,
}

#[cfg(feature = "postgres")]
impl PostgresRootFilesystem {
    pub fn new(pool: deadpool_postgres::Pool) -> Self {
        Self { pool }
    }

    pub async fn run_migrations(&self) -> Result<(), FilesystemError> {
        let client = self.client().await?;
        client
            .batch_execute(POSTGRES_ROOT_FILESYSTEM_SCHEMA)
            .await
            .map_err(|error| {
                db_error(
                    valid_engine_path(),
                    FilesystemOperation::CreateDirAll,
                    error,
                )
            })
    }

    async fn client(&self) -> Result<deadpool_postgres::Object, FilesystemError> {
        self.pool
            .get()
            .await
            .map_err(|error| FilesystemError::Backend {
                path: valid_engine_path(),
                operation: FilesystemOperation::Stat,
                reason: error.to_string(),
            })
    }
}

#[cfg(feature = "postgres")]
#[async_trait]
impl RootFilesystem for PostgresRootFilesystem {
    async fn read_file(&self, path: &VirtualPath) -> Result<Vec<u8>, FilesystemError> {
        let client = self.client().await?;
        let row = client
            .query_opt(
                "SELECT contents, is_dir FROM root_filesystem_entries WHERE path = $1",
                &[&path.as_str()],
            )
            .await
            .map_err(|error| db_error(path.clone(), FilesystemOperation::ReadFile, error))?;
        let Some(row) = row else {
            return Err(not_found(path.clone(), FilesystemOperation::ReadFile));
        };
        let is_dir: bool = row.get("is_dir");
        if is_dir {
            return Err(FilesystemError::Backend {
                path: path.clone(),
                operation: FilesystemOperation::ReadFile,
                reason: "is a directory".to_string(),
            });
        }
        Ok(row.get("contents"))
    }

    async fn write_file(&self, path: &VirtualPath, bytes: &[u8]) -> Result<(), FilesystemError> {
        let client = self.client().await?;
        client
            .execute(
                r#"
                INSERT INTO root_filesystem_entries (path, contents, is_dir)
                VALUES ($1, $2, FALSE)
                ON CONFLICT (path) DO UPDATE SET
                    contents = EXCLUDED.contents,
                    is_dir = FALSE,
                    updated_at = NOW()
                "#,
                &[&path.as_str(), &bytes],
            )
            .await
            .map_err(|error| db_error(path.clone(), FilesystemOperation::WriteFile, error))?;
        Ok(())
    }

    async fn append_file(&self, path: &VirtualPath, bytes: &[u8]) -> Result<(), FilesystemError> {
        let client = self.client().await?;
        if matches!(
            self.exact_entry_with_client(&client, path).await?,
            Some((_, FileType::Directory))
        ) {
            return Err(FilesystemError::Backend {
                path: path.clone(),
                operation: FilesystemOperation::AppendFile,
                reason: "cannot append to a directory".to_string(),
            });
        }
        // TODO(reborn): append rewrites the whole DB row. Do not use this path
        // for high-volume JSONL/event streams; route those through typed event
        // stores or append-capable artifact backends instead.
        client
            .execute(
                r#"
                INSERT INTO root_filesystem_entries (path, contents, is_dir)
                VALUES ($1, $2, FALSE)
                ON CONFLICT (path) DO UPDATE SET
                    contents = root_filesystem_entries.contents || EXCLUDED.contents,
                    is_dir = FALSE,
                    updated_at = NOW()
                "#,
                &[&path.as_str(), &bytes],
            )
            .await
            .map_err(|error| db_error(path.clone(), FilesystemOperation::AppendFile, error))?;
        Ok(())
    }

    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        let client = self.client().await?;
        let exact_entry = self.exact_entry_with_client(&client, path).await?;
        if matches!(exact_entry, Some((_, FileType::File))) {
            return Err(FilesystemError::Backend {
                path: path.clone(),
                operation: FilesystemOperation::ListDir,
                reason: "not a directory".to_string(),
            });
        }
        let rows = self
            .child_entries_with_client(&client, path, FilesystemOperation::ListDir)
            .await?;
        let children = direct_children(path, rows);
        if matches!(exact_entry, Some((_, FileType::Directory))) && is_not_found(&children) {
            return Ok(Vec::new());
        }
        children
    }

    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        let client = self.client().await?;
        if let Some((len, file_type)) = self.exact_entry_with_client(&client, path).await? {
            return Ok(FileStat {
                path: path.clone(),
                file_type,
                len,
            });
        }
        if self.has_child_entry_with_client(&client, path).await? {
            return Ok(FileStat {
                path: path.clone(),
                file_type: FileType::Directory,
                len: 0,
            });
        }
        Err(not_found(path.clone(), FilesystemOperation::Stat))
    }

    async fn delete(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        let client = self.client().await?;
        let child_pattern = child_path_like_pattern(path);
        let deleted = client
            .execute(
                "DELETE FROM root_filesystem_entries WHERE path = $1 OR path LIKE $2 ESCAPE '!'",
                &[&path.as_str(), &child_pattern],
            )
            .await
            .map_err(|error| db_error(path.clone(), FilesystemOperation::Delete, error))?;
        if deleted == 0 {
            return Err(not_found(path.clone(), FilesystemOperation::Delete));
        }
        Ok(())
    }

    async fn create_dir_all(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        let mut client = self.client().await?;
        let transaction = client
            .transaction()
            .await
            .map_err(|error| db_error(path.clone(), FilesystemOperation::CreateDirAll, error))?;
        for prefix in virtual_path_prefixes(path)? {
            let row = transaction
                .query_opt(
                    "SELECT is_dir FROM root_filesystem_entries WHERE path = $1",
                    &[&prefix.as_str()],
                )
                .await
                .map_err(|error| {
                    db_error(prefix.clone(), FilesystemOperation::CreateDirAll, error)
                })?;
            if row.is_some_and(|row| !row.get::<_, bool>("is_dir")) {
                return Err(FilesystemError::Backend {
                    path: prefix,
                    operation: FilesystemOperation::CreateDirAll,
                    reason: "file exists where directory is required".to_string(),
                });
            }
            transaction
                .execute(
                    r#"
                    INSERT INTO root_filesystem_entries (path, contents, is_dir)
                    VALUES ($1, ''::bytea, TRUE)
                    ON CONFLICT (path) DO NOTHING
                    "#,
                    &[&prefix.as_str()],
                )
                .await
                .map_err(|error| {
                    db_error(path.clone(), FilesystemOperation::CreateDirAll, error)
                })?;
        }
        transaction
            .commit()
            .await
            .map_err(|error| db_error(path.clone(), FilesystemOperation::CreateDirAll, error))?;
        Ok(())
    }
}

#[cfg(feature = "postgres")]
impl PostgresRootFilesystem {
    async fn exact_entry_with_client(
        &self,
        client: &tokio_postgres::Client,
        path: &VirtualPath,
    ) -> Result<Option<(u64, FileType)>, FilesystemError> {
        let row = client
            .query_opt(
                "SELECT OCTET_LENGTH(contents)::bigint AS len, is_dir FROM root_filesystem_entries WHERE path = $1",
                &[&path.as_str()],
            )
            .await
            .map_err(|error| db_error(path.clone(), FilesystemOperation::Stat, error))?;
        Ok(row.map(|row| {
            let len: i64 = row.get("len");
            let is_dir: bool = row.get("is_dir");
            (
                if is_dir { 0 } else { len.max(0) as u64 },
                if is_dir {
                    FileType::Directory
                } else {
                    FileType::File
                },
            )
        }))
    }

    async fn child_entries_with_client(
        &self,
        client: &tokio_postgres::Client,
        parent: &VirtualPath,
        operation: FilesystemOperation,
    ) -> Result<Vec<(VirtualPath, u64, FileType)>, FilesystemError> {
        let pattern = child_path_like_pattern(parent);
        let rows = client
            .query(
                "SELECT path, OCTET_LENGTH(contents)::bigint AS len, is_dir FROM root_filesystem_entries WHERE path LIKE $1 ESCAPE '!' ORDER BY path",
                &[&pattern],
            )
            .await
            .map_err(|error| db_error(parent.clone(), operation, error))?;
        rows.into_iter()
            .map(|row| {
                let path: String = row.get("path");
                let len: i64 = row.get("len");
                let is_dir: bool = row.get("is_dir");
                Ok((
                    VirtualPath::new(path)?,
                    if is_dir { 0 } else { len.max(0) as u64 },
                    if is_dir {
                        FileType::Directory
                    } else {
                        FileType::File
                    },
                ))
            })
            .collect()
    }

    async fn has_child_entry_with_client(
        &self,
        client: &tokio_postgres::Client,
        parent: &VirtualPath,
    ) -> Result<bool, FilesystemError> {
        let pattern = child_path_like_pattern(parent);
        let row = client
            .query_opt(
                "SELECT 1 FROM root_filesystem_entries WHERE path LIKE $1 ESCAPE '!' LIMIT 1",
                &[&pattern],
            )
            .await
            .map_err(|error| db_error(parent.clone(), FilesystemOperation::Stat, error))?;
        Ok(row.is_some())
    }
}

#[cfg(feature = "postgres")]
const POSTGRES_ROOT_FILESYSTEM_SCHEMA: &str = concat!(
    include_str!("../../../migrations/V26__root_filesystem_entries.sql"),
    "\n",
    include_str!("../../../migrations/V27__root_filesystem_entries_directories.sql"),
);

#[cfg(feature = "libsql")]
/// libSQL-backed [`RootFilesystem`] storing file contents by virtual path.
pub struct LibSqlRootFilesystem {
    db: Arc<libsql::Database>,
}

#[cfg(feature = "libsql")]
impl LibSqlRootFilesystem {
    pub fn new(db: Arc<libsql::Database>) -> Self {
        Self { db }
    }

    pub async fn run_migrations(&self) -> Result<(), FilesystemError> {
        let conn = self.connect().await?;
        conn.execute_batch(LIBSQL_ROOT_FILESYSTEM_SCHEMA)
            .await
            .map_err(|error| {
                libsql_db_error(
                    valid_engine_path(),
                    FilesystemOperation::CreateDirAll,
                    error,
                )
            })?;
        ensure_libsql_root_is_dir_column(&conn).await?;
        Ok(())
    }

    async fn connect(&self) -> Result<libsql::Connection, FilesystemError> {
        let conn = self
            .db
            .connect()
            .map_err(|error| FilesystemError::Backend {
                path: valid_engine_path(),
                operation: FilesystemOperation::Stat,
                reason: error.to_string(),
            })?;
        conn.query("PRAGMA busy_timeout = 5000", ())
            .await
            .map_err(|error| {
                libsql_db_error(valid_engine_path(), FilesystemOperation::Stat, error)
            })?;
        Ok(conn)
    }
}

#[cfg(feature = "libsql")]
#[async_trait]
impl RootFilesystem for LibSqlRootFilesystem {
    async fn read_file(&self, path: &VirtualPath) -> Result<Vec<u8>, FilesystemError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT contents, is_dir FROM root_filesystem_entries WHERE path = ?1",
                libsql::params![path.as_str()],
            )
            .await
            .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::ReadFile, error))?;
        let Some(row) = rows
            .next()
            .await
            .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::ReadFile, error))?
        else {
            return Err(not_found(path.clone(), FilesystemOperation::ReadFile));
        };
        let is_dir: i64 = row
            .get(1)
            .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::ReadFile, error))?;
        if is_dir != 0 {
            return Err(FilesystemError::Backend {
                path: path.clone(),
                operation: FilesystemOperation::ReadFile,
                reason: "is a directory".to_string(),
            });
        }
        row.get(0)
            .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::ReadFile, error))
    }

    async fn write_file(&self, path: &VirtualPath, bytes: &[u8]) -> Result<(), FilesystemError> {
        let conn = self.connect().await?;
        conn.execute(
            r#"
            INSERT INTO root_filesystem_entries (path, contents, is_dir, updated_at)
            VALUES (?1, ?2, 0, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
            ON CONFLICT (path) DO UPDATE SET
                contents = excluded.contents,
                is_dir = 0,
                updated_at = excluded.updated_at
            "#,
            libsql::params![path.as_str(), libsql::Value::Blob(bytes.to_vec())],
        )
        .await
        .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::WriteFile, error))?;
        Ok(())
    }

    async fn append_file(&self, path: &VirtualPath, bytes: &[u8]) -> Result<(), FilesystemError> {
        if matches!(
            self.exact_entry(path).await?,
            Some((_, FileType::Directory))
        ) {
            return Err(FilesystemError::Backend {
                path: path.clone(),
                operation: FilesystemOperation::AppendFile,
                reason: "cannot append to a directory".to_string(),
            });
        }
        let conn = self.connect().await?;
        // TODO(reborn): append rewrites the whole DB row. Do not use this path
        // for high-volume JSONL/event streams; route those through typed event
        // stores or append-capable artifact backends instead.
        conn.execute(
            r#"
            INSERT INTO root_filesystem_entries (path, contents, is_dir, updated_at)
            VALUES (?1, ?2, 0, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
            ON CONFLICT (path) DO UPDATE SET
                contents = CAST(root_filesystem_entries.contents || excluded.contents AS BLOB),
                is_dir = 0,
                updated_at = excluded.updated_at
            "#,
            libsql::params![path.as_str(), libsql::Value::Blob(bytes.to_vec())],
        )
        .await
        .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::AppendFile, error))?;
        Ok(())
    }

    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        let exact_entry = self.exact_entry(path).await?;
        if matches!(exact_entry, Some((_, FileType::File))) {
            return Err(FilesystemError::Backend {
                path: path.clone(),
                operation: FilesystemOperation::ListDir,
                reason: "not a directory".to_string(),
            });
        }
        let rows = self
            .child_entries(path, FilesystemOperation::ListDir)
            .await?;
        let children = direct_children(path, rows);
        if matches!(exact_entry, Some((_, FileType::Directory))) && is_not_found(&children) {
            return Ok(Vec::new());
        }
        children
    }

    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        if let Some((len, file_type)) = self.exact_entry(path).await? {
            return Ok(FileStat {
                path: path.clone(),
                file_type,
                len,
            });
        }
        if self.has_child_entry(path).await? {
            return Ok(FileStat {
                path: path.clone(),
                file_type: FileType::Directory,
                len: 0,
            });
        }
        Err(not_found(path.clone(), FilesystemOperation::Stat))
    }

    async fn delete(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        let conn = self.connect().await?;
        let deleted = conn
            .execute(
                "DELETE FROM root_filesystem_entries WHERE path = ?1 OR path LIKE ?2 ESCAPE '!'",
                libsql::params![path.as_str(), child_path_like_pattern(path)],
            )
            .await
            .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::Delete, error))?;
        if deleted == 0 {
            return Err(not_found(path.clone(), FilesystemOperation::Delete));
        }
        Ok(())
    }

    async fn create_dir_all(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        let conn = self.connect().await?;
        let transaction = conn.transaction().await.map_err(|error| {
            libsql_db_error(path.clone(), FilesystemOperation::CreateDirAll, error)
        })?;
        for prefix in virtual_path_prefixes(path)? {
            let mut rows = transaction
                .query(
                    "SELECT is_dir FROM root_filesystem_entries WHERE path = ?1",
                    libsql::params![prefix.as_str()],
                )
                .await
                .map_err(|error| {
                    libsql_db_error(prefix.clone(), FilesystemOperation::CreateDirAll, error)
                })?;
            if let Some(row) = rows.next().await.map_err(|error| {
                libsql_db_error(prefix.clone(), FilesystemOperation::CreateDirAll, error)
            })? {
                let is_dir: i64 = row.get(0).map_err(|error| {
                    libsql_db_error(prefix.clone(), FilesystemOperation::CreateDirAll, error)
                })?;
                if is_dir == 0 {
                    return Err(FilesystemError::Backend {
                        path: prefix,
                        operation: FilesystemOperation::CreateDirAll,
                        reason: "file exists where directory is required".to_string(),
                    });
                }
            }
            transaction
                .execute(
                    r#"
                    INSERT INTO root_filesystem_entries (path, contents, is_dir, updated_at)
                    VALUES (?1, X'', 1, strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
                    ON CONFLICT (path) DO NOTHING
                    "#,
                    libsql::params![prefix.as_str()],
                )
                .await
                .map_err(|error| {
                    libsql_db_error(path.clone(), FilesystemOperation::CreateDirAll, error)
                })?;
        }
        transaction.commit().await.map_err(|error| {
            libsql_db_error(path.clone(), FilesystemOperation::CreateDirAll, error)
        })?;
        Ok(())
    }
}

#[cfg(feature = "libsql")]
async fn ensure_libsql_root_is_dir_column(
    conn: &libsql::Connection,
) -> Result<(), FilesystemError> {
    let mut rows = conn
        .query(
            "SELECT 1 FROM pragma_table_info('root_filesystem_entries') WHERE name = 'is_dir'",
            (),
        )
        .await
        .map_err(|error| {
            libsql_db_error(
                valid_engine_path(),
                FilesystemOperation::CreateDirAll,
                error,
            )
        })?;
    if rows
        .next()
        .await
        .map_err(|error| {
            libsql_db_error(
                valid_engine_path(),
                FilesystemOperation::CreateDirAll,
                error,
            )
        })?
        .is_some()
    {
        return Ok(());
    }
    conn.execute(
        "ALTER TABLE root_filesystem_entries ADD COLUMN is_dir INTEGER NOT NULL DEFAULT 0 CHECK (is_dir IN (0, 1))",
        (),
    )
    .await
    .map_err(|error| {
        libsql_db_error(
            valid_engine_path(),
            FilesystemOperation::CreateDirAll,
            error,
        )
    })?;
    Ok(())
}

#[cfg(feature = "libsql")]
impl LibSqlRootFilesystem {
    async fn exact_entry(
        &self,
        path: &VirtualPath,
    ) -> Result<Option<(u64, FileType)>, FilesystemError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT length(contents), is_dir FROM root_filesystem_entries WHERE path = ?1",
                libsql::params![path.as_str()],
            )
            .await
            .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::Stat, error))?;
        let row = rows
            .next()
            .await
            .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::Stat, error))?;
        let Some(row) = row else { return Ok(None) };
        let len_raw: i64 = row
            .get(0)
            .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::Stat, error))?;
        let is_dir_raw: i64 = row
            .get(1)
            .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::Stat, error))?;
        let len = len_raw.max(0) as u64;
        let is_dir = is_dir_raw != 0;
        Ok(Some((
            if is_dir { 0 } else { len },
            if is_dir {
                FileType::Directory
            } else {
                FileType::File
            },
        )))
    }

    async fn child_entries(
        &self,
        parent: &VirtualPath,
        operation: FilesystemOperation,
    ) -> Result<Vec<(VirtualPath, u64, FileType)>, FilesystemError> {
        let conn = self.connect().await?;
        let pattern = child_path_like_pattern(parent);
        let mut rows = conn
            .query(
                "SELECT path, length(contents), is_dir FROM root_filesystem_entries WHERE path LIKE ?1 ESCAPE '!' ORDER BY path",
                libsql::params![pattern],
            )
            .await
            .map_err(|error| libsql_db_error(parent.clone(), operation, error))?;
        let mut paths = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|error| libsql_db_error(parent.clone(), operation, error))?
        {
            let path: String = row
                .get(0)
                .map_err(|error| libsql_db_error(parent.clone(), operation, error))?;
            let len_raw: i64 = row
                .get(1)
                .map_err(|error| libsql_db_error(parent.clone(), operation, error))?;
            let is_dir_raw: i64 = row
                .get(2)
                .map_err(|error| libsql_db_error(parent.clone(), operation, error))?;
            let len = len_raw.max(0) as u64;
            let is_dir = is_dir_raw != 0;
            paths.push((
                VirtualPath::new(path)?,
                if is_dir { 0 } else { len },
                if is_dir {
                    FileType::Directory
                } else {
                    FileType::File
                },
            ));
        }
        Ok(paths)
    }

    async fn has_child_entry(&self, parent: &VirtualPath) -> Result<bool, FilesystemError> {
        let conn = self.connect().await?;
        let pattern = child_path_like_pattern(parent);
        let mut rows = conn
            .query(
                "SELECT 1 FROM root_filesystem_entries WHERE path LIKE ?1 ESCAPE '!' LIMIT 1",
                libsql::params![pattern],
            )
            .await
            .map_err(|error| libsql_db_error(parent.clone(), FilesystemOperation::Stat, error))?;
        Ok(rows
            .next()
            .await
            .map_err(|error| libsql_db_error(parent.clone(), FilesystemOperation::Stat, error))?
            .is_some())
    }
}

#[cfg(feature = "libsql")]
const LIBSQL_ROOT_FILESYSTEM_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS root_filesystem_entries (
    path TEXT PRIMARY KEY,
    contents BLOB NOT NULL DEFAULT X'',
    is_dir INTEGER NOT NULL DEFAULT 0 CHECK (is_dir IN (0, 1)),
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
-- The PRIMARY KEY on `path` already provides a unique index for equality
-- lookups, so no separate index is created.
"#;

#[cfg(any(feature = "postgres", feature = "libsql"))]
fn virtual_path_prefixes(path: &VirtualPath) -> Result<Vec<VirtualPath>, HostApiError> {
    let mut prefixes = Vec::new();
    let mut current = String::new();
    for segment in path.as_str().trim_matches('/').split('/') {
        if segment.is_empty() {
            continue;
        }
        current.push('/');
        current.push_str(segment);
        prefixes.push(VirtualPath::new(current.clone())?);
    }
    Ok(prefixes)
}

#[cfg(any(feature = "postgres", feature = "libsql"))]
fn direct_children(
    parent: &VirtualPath,
    rows: Vec<(VirtualPath, u64, FileType)>,
) -> Result<Vec<DirEntry>, FilesystemError> {
    let mut entries = std::collections::BTreeMap::<String, DirEntry>::new();
    let prefix = format!("{}/", parent.as_str().trim_end_matches('/'));
    for (path, _len, row_file_type) in rows {
        let Some(tail) = path.as_str().strip_prefix(&prefix) else {
            continue;
        };
        if tail.is_empty() {
            continue;
        }
        let (name, file_type) = if let Some((directory, _rest)) = tail.split_once('/') {
            (directory.to_string(), FileType::Directory)
        } else {
            (tail.to_string(), row_file_type)
        };
        let entry_path = VirtualPath::new(format!(
            "{}/{}",
            parent.as_str().trim_end_matches('/'),
            name
        ))?;
        entries.entry(name.clone()).or_insert(DirEntry {
            name,
            path: entry_path,
            file_type,
        });
    }
    if entries.is_empty() {
        return Err(not_found(parent.clone(), FilesystemOperation::ListDir));
    }
    Ok(entries.into_values().collect())
}

#[cfg(any(feature = "postgres", feature = "libsql"))]
fn child_path_like_pattern(path: &VirtualPath) -> String {
    let mut pattern = String::new();
    for character in path.as_str().trim_end_matches('/').chars() {
        match character {
            '!' | '%' | '_' => {
                pattern.push('!');
                pattern.push(character);
            }
            _ => pattern.push(character),
        }
    }
    pattern.push_str("/%");
    pattern
}

#[cfg(any(feature = "postgres", feature = "libsql"))]
fn not_found(path: VirtualPath, operation: FilesystemOperation) -> FilesystemError {
    FilesystemError::NotFound { path, operation }
}

#[cfg(any(feature = "postgres", feature = "libsql"))]
fn is_not_found<T>(result: &Result<T, FilesystemError>) -> bool {
    matches!(result, Err(FilesystemError::NotFound { .. }))
}

#[cfg(feature = "postgres")]
fn db_error(
    path: VirtualPath,
    operation: FilesystemOperation,
    error: tokio_postgres::Error,
) -> FilesystemError {
    FilesystemError::Backend {
        path,
        operation,
        reason: error.to_string(),
    }
}

#[cfg(feature = "libsql")]
fn libsql_db_error(
    path: VirtualPath,
    operation: FilesystemOperation,
    error: libsql::Error,
) -> FilesystemError {
    FilesystemError::Backend {
        path,
        operation,
        reason: error.to_string(),
    }
}

#[cfg(any(feature = "postgres", feature = "libsql"))]
fn valid_engine_path() -> VirtualPath {
    VirtualPath::new("/engine").unwrap_or_else(|_| unreachable!("literal virtual path is valid"))
}
