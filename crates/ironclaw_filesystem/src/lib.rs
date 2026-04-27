//! Scoped filesystem service for IronClaw Reborn.
//!
//! `ironclaw_filesystem` is the first service crate above
//! `ironclaw_host_api`. It resolves runtime-visible [`ScopedPath`] values
//! through a caller's [`MountView`], checks mount permissions, then performs the
//! operation against a trusted root filesystem namespace addressed by
//! [`VirtualPath`]. Backend implementations alone touch raw host paths.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::{
    HostApiError, HostPath, MountGrant, MountPermissions, MountView, ScopedPath, VirtualPath,
};
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
    #[error("permission denied for {operation} on scoped path {path:?}")]
    PermissionDenied {
        path: ScopedPath,
        operation: FilesystemOperation,
    },
    #[error("no backend mount found for virtual path {path:?}")]
    MountNotFound { path: VirtualPath },
    #[error("virtual path escaped backend mount {path:?}")]
    PathOutsideMount { path: VirtualPath },
    #[error("symlink escapes backend mount at virtual path {path:?}")]
    SymlinkEscape { path: VirtualPath },
    #[error("backend mount conflict at virtual path {path:?}")]
    MountConflict { path: VirtualPath },
    #[error("filesystem backend error during {operation} at {path:?}: {reason}")]
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
                virtual_prefix_matches(mount.descriptor.virtual_root.as_str(), path.as_str())
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

    /// Deletes a canonical virtual file or directory. Backends that do not support delete must fail closed before side effects.
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
        let grant =
            matching_mount(&self.mounts, path).ok_or_else(|| match self.mounts.resolve(path) {
                Ok(_) => FilesystemError::from(HostApiError::InvalidMount {
                    value: path.as_str().to_string(),
                    reason: "scoped path matched no mount grant".to_string(),
                }),
                Err(error) => FilesystemError::from(error),
            })?;

        if !operation_allowed(&grant.permissions, operation) {
            return Err(FilesystemError::PermissionDenied {
                path: path.clone(),
                operation,
            });
        }

        self.mounts.resolve(path).map_err(FilesystemError::from)
    }
}

fn matching_mount<'a>(view: &'a MountView, path: &ScopedPath) -> Option<&'a MountGrant> {
    let raw = path.as_str();
    view.mounts
        .iter()
        .filter(|mount| alias_matches(mount.alias.as_str(), raw))
        .max_by_key(|mount| mount.alias.as_str().len())
}

fn alias_matches(alias: &str, path: &str) -> bool {
    path == alias || path.starts_with(&format!("{alias}/"))
}

fn operation_allowed(permissions: &MountPermissions, operation: FilesystemOperation) -> bool {
    match operation {
        FilesystemOperation::ReadFile => permissions.read,
        FilesystemOperation::WriteFile => permissions.write,
        FilesystemOperation::AppendFile => permissions.write,
        FilesystemOperation::ListDir => permissions.list,
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

    fn resolve_existing(
        &self,
        path: &VirtualPath,
        operation: FilesystemOperation,
    ) -> Result<PathBuf, FilesystemError> {
        let (mount, joined) = self.resolve_joined(path)?;
        let canonical =
            std::fs::canonicalize(&joined).map_err(|error| FilesystemError::Backend {
                path: path.clone(),
                operation,
                reason: io_reason(error),
            })?;
        ensure_contained(path, mount, &canonical, true)?;
        Ok(canonical)
    }

    fn resolve_for_write(
        &self,
        path: &VirtualPath,
        operation: FilesystemOperation,
    ) -> Result<PathBuf, FilesystemError> {
        let (mount, joined) = self.resolve_joined(path)?;

        if joined.exists() {
            let canonical =
                std::fs::canonicalize(&joined).map_err(|error| FilesystemError::Backend {
                    path: path.clone(),
                    operation,
                    reason: io_reason(error),
                })?;
            ensure_contained(path, mount, &canonical, true)?;
            return Ok(canonical);
        }

        let parent = joined
            .parent()
            .ok_or_else(|| FilesystemError::PathOutsideMount { path: path.clone() })?;
        ensure_existing_ancestor_contained(path, mount, parent, operation)?;
        std::fs::create_dir_all(parent).map_err(|error| FilesystemError::Backend {
            path: path.clone(),
            operation: FilesystemOperation::CreateDirAll,
            reason: io_reason(error),
        })?;
        let canonical_parent =
            std::fs::canonicalize(parent).map_err(|error| FilesystemError::Backend {
                path: path.clone(),
                operation: FilesystemOperation::CreateDirAll,
                reason: io_reason(error),
            })?;
        // `joined` is constructed from validated virtual path segments under the
        // backend root. If its canonical parent leaves the backend root, an
        // existing symlink in the parent chain caused the escape.
        ensure_contained(path, mount, &canonical_parent, true)?;
        Ok(joined)
    }

    fn resolve_for_create_dir_all(&self, path: &VirtualPath) -> Result<PathBuf, FilesystemError> {
        let (mount, joined) = self.resolve_joined(path)?;
        ensure_existing_ancestor_contained(
            path,
            mount,
            &joined,
            FilesystemOperation::CreateDirAll,
        )?;
        std::fs::create_dir_all(&joined).map_err(|error| FilesystemError::Backend {
            path: path.clone(),
            operation: FilesystemOperation::CreateDirAll,
            reason: io_reason(error),
        })?;
        let canonical =
            std::fs::canonicalize(&joined).map_err(|error| FilesystemError::Backend {
                path: path.clone(),
                operation: FilesystemOperation::CreateDirAll,
                reason: io_reason(error),
            })?;
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
            .filter(|mount| virtual_prefix_matches(mount.virtual_root.as_str(), path.as_str()))
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
        let resolved = self.resolve_existing(path, FilesystemOperation::ReadFile)?;
        std::fs::read(resolved).map_err(|error| FilesystemError::Backend {
            path: path.clone(),
            operation: FilesystemOperation::ReadFile,
            reason: io_reason(error),
        })
    }

    async fn write_file(&self, path: &VirtualPath, bytes: &[u8]) -> Result<(), FilesystemError> {
        let resolved = self.resolve_for_write(path, FilesystemOperation::WriteFile)?;
        std::fs::write(resolved, bytes).map_err(|error| FilesystemError::Backend {
            path: path.clone(),
            operation: FilesystemOperation::WriteFile,
            reason: io_reason(error),
        })
    }

    async fn append_file(&self, path: &VirtualPath, bytes: &[u8]) -> Result<(), FilesystemError> {
        let resolved = self.resolve_for_write(path, FilesystemOperation::AppendFile)?;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(resolved)
            .map_err(|error| FilesystemError::Backend {
                path: path.clone(),
                operation: FilesystemOperation::AppendFile,
                reason: io_reason(error),
            })?;
        file.write_all(bytes)
            .map_err(|error| FilesystemError::Backend {
                path: path.clone(),
                operation: FilesystemOperation::AppendFile,
                reason: io_reason(error),
            })
    }

    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        let resolved = self.resolve_existing(path, FilesystemOperation::ListDir)?;
        let mut entries = Vec::new();
        for entry in std::fs::read_dir(resolved).map_err(|error| FilesystemError::Backend {
            path: path.clone(),
            operation: FilesystemOperation::ListDir,
            reason: io_reason(error),
        })? {
            let entry = entry.map_err(|error| FilesystemError::Backend {
                path: path.clone(),
                operation: FilesystemOperation::ListDir,
                reason: io_reason(error),
            })?;
            let name = entry.file_name().to_string_lossy().to_string();
            let entry_path =
                VirtualPath::new(format!("{}/{}", path.as_str().trim_end_matches('/'), name))?;
            let metadata = entry.metadata().map_err(|error| FilesystemError::Backend {
                path: entry_path.clone(),
                operation: FilesystemOperation::Stat,
                reason: io_reason(error),
            })?;
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
        let resolved = self.resolve_existing(path, FilesystemOperation::Stat)?;
        let metadata = std::fs::metadata(resolved).map_err(|error| FilesystemError::Backend {
            path: path.clone(),
            operation: FilesystemOperation::Stat,
            reason: io_reason(error),
        })?;
        Ok(FileStat {
            path: path.clone(),
            file_type: file_type_from_metadata(&metadata),
            len: metadata.len(),
        })
    }

    async fn delete(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        let resolved = self.resolve_existing(path, FilesystemOperation::Delete)?;
        let metadata = std::fs::metadata(&resolved).map_err(|error| FilesystemError::Backend {
            path: path.clone(),
            operation: FilesystemOperation::Delete,
            reason: io_reason(error),
        })?;
        if metadata.is_dir() {
            std::fs::remove_dir_all(resolved)
        } else {
            std::fs::remove_file(resolved)
        }
        .map_err(|error| FilesystemError::Backend {
            path: path.clone(),
            operation: FilesystemOperation::Delete,
            reason: io_reason(error),
        })
    }

    async fn create_dir_all(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.resolve_for_create_dir_all(path).map(|_| ())
    }
}

fn virtual_prefix_matches(prefix: &str, path: &str) -> bool {
    path == prefix || path.starts_with(&format!("{prefix}/"))
}

fn ensure_existing_ancestor_contained(
    virtual_path: &VirtualPath,
    mount: &LocalMount,
    candidate: &Path,
    operation: FilesystemOperation,
) -> Result<(), FilesystemError> {
    let mut ancestor = candidate;
    while !ancestor.exists() {
        ancestor = ancestor
            .parent()
            .ok_or_else(|| FilesystemError::PathOutsideMount {
                path: virtual_path.clone(),
            })?;
    }
    let canonical = std::fs::canonicalize(ancestor).map_err(|error| FilesystemError::Backend {
        path: virtual_path.clone(),
        operation,
        reason: io_reason(error),
    })?;
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
            self.exact_entry(path).await?,
            Some((_, FileType::Directory))
        ) {
            return Err(FilesystemError::Backend {
                path: path.clone(),
                operation: FilesystemOperation::AppendFile,
                reason: "cannot append to a directory".to_string(),
            });
        }
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
        let exact_entry = self.exact_entry(path).await?;
        if matches!(exact_entry, Some((_, FileType::File))) {
            return Err(FilesystemError::Backend {
                path: path.clone(),
                operation: FilesystemOperation::ListDir,
                reason: "not a directory".to_string(),
            });
        }
        let rows = self.all_paths().await?;
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
        let rows = self.all_paths().await?;
        if rows
            .iter()
            .any(|(child_path, _, _)| virtual_prefix_matches(path.as_str(), child_path.as_str()))
        {
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
        client
            .execute(
                "DELETE FROM root_filesystem_entries WHERE path = $1 OR path LIKE $2",
                &[
                    &path.as_str(),
                    &format!("{}/%", path.as_str().trim_end_matches('/')),
                ],
            )
            .await
            .map_err(|error| db_error(path.clone(), FilesystemOperation::Delete, error))?;
        Ok(())
    }

    async fn create_dir_all(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        let client = self.client().await?;
        for prefix in virtual_path_prefixes(path)? {
            if matches!(self.exact_entry(&prefix).await?, Some((_, FileType::File))) {
                return Err(FilesystemError::Backend {
                    path: prefix,
                    operation: FilesystemOperation::CreateDirAll,
                    reason: "file exists where directory is required".to_string(),
                });
            }
            client
                .execute(
                    r#"
                    INSERT INTO root_filesystem_entries (path, contents, is_dir)
                    VALUES ($1, '\\x'::bytea, TRUE)
                    ON CONFLICT (path) DO NOTHING
                    "#,
                    &[&prefix.as_str()],
                )
                .await
                .map_err(|error| {
                    db_error(path.clone(), FilesystemOperation::CreateDirAll, error)
                })?;
        }
        Ok(())
    }
}

#[cfg(feature = "postgres")]
impl PostgresRootFilesystem {
    async fn exact_entry(
        &self,
        path: &VirtualPath,
    ) -> Result<Option<(u64, FileType)>, FilesystemError> {
        let client = self.client().await?;
        let row = client
            .query_opt(
                "SELECT OCTET_LENGTH(contents) AS len, is_dir FROM root_filesystem_entries WHERE path = $1",
                &[&path.as_str()],
            )
            .await
            .map_err(|error| db_error(path.clone(), FilesystemOperation::Stat, error))?;
        Ok(row.map(|row| {
            let len: i32 = row.get("len");
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

    async fn all_paths(&self) -> Result<Vec<(VirtualPath, u64, FileType)>, FilesystemError> {
        let client = self.client().await?;
        let rows = client
            .query(
                "SELECT path, OCTET_LENGTH(contents) AS len, is_dir FROM root_filesystem_entries ORDER BY path",
                &[],
            )
            .await
            .map_err(|error| {
                db_error(
                    VirtualPath::new("/engine").unwrap_or_else(|_| unreachable!("literal virtual path is valid")),
                    FilesystemOperation::ListDir,
                    error,
                )
            })?;
        rows.into_iter()
            .map(|row| {
                let path: String = row.get("path");
                let len: i32 = row.get("len");
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
}

#[cfg(feature = "postgres")]
const POSTGRES_ROOT_FILESYSTEM_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS root_filesystem_entries (
    path TEXT PRIMARY KEY CHECK (path LIKE '/%'),
    contents BYTEA NOT NULL DEFAULT '\\x',
    is_dir BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
ALTER TABLE root_filesystem_entries
    ADD COLUMN IF NOT EXISTS is_dir BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE root_filesystem_entries
    ALTER COLUMN contents SET DEFAULT '\\x';
CREATE INDEX IF NOT EXISTS idx_root_filesystem_entries_path
    ON root_filesystem_entries(path);
"#;

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
        let mut contents = match self.read_file(path).await {
            Ok(contents) => contents,
            Err(FilesystemError::Backend { reason, .. }) if reason == "not found" => Vec::new(),
            Err(error) => return Err(error),
        };
        contents.extend_from_slice(bytes);
        self.write_file(path, &contents).await
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
        let rows = self.all_paths().await?;
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
        let rows = self.all_paths().await?;
        if rows
            .iter()
            .any(|(child_path, _, _)| virtual_prefix_matches(path.as_str(), child_path.as_str()))
        {
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
        conn.execute(
            "DELETE FROM root_filesystem_entries WHERE path = ?1 OR path LIKE ?2",
            libsql::params![
                path.as_str(),
                format!("{}/%", path.as_str().trim_end_matches('/'))
            ],
        )
        .await
        .map_err(|error| libsql_db_error(path.clone(), FilesystemOperation::Delete, error))?;
        Ok(())
    }

    async fn create_dir_all(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        let conn = self.connect().await?;
        for prefix in virtual_path_prefixes(path)? {
            if matches!(self.exact_entry(&prefix).await?, Some((_, FileType::File))) {
                return Err(FilesystemError::Backend {
                    path: prefix,
                    operation: FilesystemOperation::CreateDirAll,
                    reason: "file exists where directory is required".to_string(),
                });
            }
            conn.execute(
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
        Ok(row.map(|row| {
            let len = row.get::<i64>(0).unwrap_or(0).max(0) as u64;
            let is_dir = row.get::<i64>(1).unwrap_or(0) != 0;
            (
                if is_dir { 0 } else { len },
                if is_dir {
                    FileType::Directory
                } else {
                    FileType::File
                },
            )
        }))
    }

    async fn all_paths(&self) -> Result<Vec<(VirtualPath, u64, FileType)>, FilesystemError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT path, length(contents), is_dir FROM root_filesystem_entries ORDER BY path",
                (),
            )
            .await
            .map_err(|error| {
                libsql_db_error(valid_engine_path(), FilesystemOperation::ListDir, error)
            })?;
        let mut paths = Vec::new();
        while let Some(row) = rows.next().await.map_err(|error| {
            libsql_db_error(valid_engine_path(), FilesystemOperation::ListDir, error)
        })? {
            let path: String = row.get(0).map_err(|error| {
                libsql_db_error(valid_engine_path(), FilesystemOperation::ListDir, error)
            })?;
            let len = row.get::<i64>(1).unwrap_or(0).max(0) as u64;
            let is_dir = row.get::<i64>(2).unwrap_or(0) != 0;
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
CREATE INDEX IF NOT EXISTS idx_root_filesystem_entries_path
    ON root_filesystem_entries(path);
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
fn not_found(path: VirtualPath, operation: FilesystemOperation) -> FilesystemError {
    FilesystemError::Backend {
        path,
        operation,
        reason: "not found".to_string(),
    }
}

#[cfg(any(feature = "postgres", feature = "libsql"))]
fn is_not_found<T>(result: &Result<T, FilesystemError>) -> bool {
    matches!(
        result,
        Err(FilesystemError::Backend { reason, .. }) if reason == "not found"
    )
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
