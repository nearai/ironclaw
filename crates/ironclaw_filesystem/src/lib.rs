//! Scoped filesystem service for IronClaw Reborn.
//!
//! `ironclaw_filesystem` is the first service crate above
//! `ironclaw_host_api`. It resolves runtime-visible [`ScopedPath`] values
//! through a caller's [`MountView`], checks mount permissions, then performs the
//! operation against a trusted root filesystem namespace addressed by
//! [`VirtualPath`]. Backend implementations alone touch raw host paths.

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

/// Trusted root filesystem interface over canonical virtual paths.
#[async_trait]
pub trait RootFilesystem: Send + Sync {
    async fn read_file(&self, path: &VirtualPath) -> Result<Vec<u8>, FilesystemError>;
    async fn write_file(&self, path: &VirtualPath, bytes: &[u8]) -> Result<(), FilesystemError>;
    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError>;
    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError>;
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

    pub async fn list_dir(&self, path: &ScopedPath) -> Result<Vec<DirEntry>, FilesystemError> {
        let virtual_path = self.resolve_with_permission(path, FilesystemOperation::ListDir)?;
        self.root.list_dir(&virtual_path).await
    }

    pub async fn stat(&self, path: &ScopedPath) -> Result<FileStat, FilesystemError> {
        let virtual_path = self.resolve_with_permission(path, FilesystemOperation::Stat)?;
        self.root.stat(&virtual_path).await
    }

    fn resolve_with_permission(
        &self,
        path: &ScopedPath,
        operation: FilesystemOperation,
    ) -> Result<VirtualPath, FilesystemError> {
        let grant = matching_mount(&self.mounts, path).ok_or_else(|| {
            FilesystemError::from(
                self.mounts
                    .resolve(path)
                    .expect_err("missing matching mount must fail mount view resolution"),
            )
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

    fn resolve_for_write(&self, path: &VirtualPath) -> Result<PathBuf, FilesystemError> {
        let (mount, joined) = self.resolve_joined(path)?;

        if joined.exists() {
            let canonical =
                std::fs::canonicalize(&joined).map_err(|error| FilesystemError::Backend {
                    path: path.clone(),
                    operation: FilesystemOperation::WriteFile,
                    reason: io_reason(error),
                })?;
            ensure_contained(path, mount, &canonical, true)?;
            return Ok(canonical);
        }

        let parent = joined
            .parent()
            .ok_or_else(|| FilesystemError::PathOutsideMount { path: path.clone() })?;
        ensure_existing_parent_chain_contained(path, mount, parent)?;
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
        let resolved = self.resolve_for_write(path)?;
        std::fs::write(resolved, bytes).map_err(|error| FilesystemError::Backend {
            path: path.clone(),
            operation: FilesystemOperation::WriteFile,
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
}

fn virtual_prefix_matches(prefix: &str, path: &str) -> bool {
    path == prefix || path.starts_with(&format!("{prefix}/"))
}

fn ensure_existing_parent_chain_contained(
    virtual_path: &VirtualPath,
    mount: &LocalMount,
    parent: &Path,
) -> Result<(), FilesystemError> {
    let mut current = parent;
    while !current.exists() {
        current = current
            .parent()
            .ok_or_else(|| FilesystemError::PathOutsideMount {
                path: virtual_path.clone(),
            })?;
    }
    let canonical = std::fs::canonicalize(current).map_err(|error| FilesystemError::Backend {
        path: virtual_path.clone(),
        operation: FilesystemOperation::CreateDirAll,
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
