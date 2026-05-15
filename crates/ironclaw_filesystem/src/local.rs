use std::path::{Path, PathBuf};

use async_trait::async_trait;
use ironclaw_host_api::{HostPath, VirtualPath};
use ironclaw_safety::sensitive_paths::is_sensitive_path;
use tokio::io::AsyncWriteExt;

use crate::{
    CasExpectation, DirEntry, Entry, FileStat, FileType, FilesystemError, FilesystemOperation,
    RecordVersion, RootFilesystem, VersionedEntry, path_prefix_matches,
};

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
    /// Native `put` for the byte-only local filesystem. Opaque-file entries
    /// (`kind = None`, empty `indexed`) with `CasExpectation::Any` delegate
    /// to `write_file`. Record-shaped entries, populated indexed
    /// projections, and `CasExpectation::Absent` / `Version(_)` are
    /// `Unsupported` because the local filesystem has no native metadata or
    /// version tracking (sidecar metadata is a future addition; see the
    /// reborn storage rework plan). We implement `put` here rather than
    /// relying on a trait default so that the put/write_file pair is
    /// non-recursive even when downstream consumers route through `put`.
    async fn put(
        &self,
        path: &VirtualPath,
        entry: Entry,
        cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError> {
        if entry.kind.is_some() || !entry.indexed.is_empty() {
            return Err(FilesystemError::Unsupported {
                path: path.clone(),
                operation: FilesystemOperation::WriteFile,
            });
        }
        if !matches!(cas, CasExpectation::Any) {
            return Err(FilesystemError::Unsupported {
                path: path.clone(),
                operation: FilesystemOperation::WriteFile,
            });
        }
        self.write_file(path, &entry.body).await?;
        Ok(RecordVersion::from_backend(0))
    }

    /// Native `get` mirroring `put`: read the bytes and wrap as an opaque
    /// `Entry`. Version is always `0` because the local filesystem doesn't
    /// track per-path versions. Non-existent paths return `Ok(None)`;
    /// directories or symlinks return their respective `read_file` errors.
    async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        match self.read_file(path).await {
            Ok(body) => Ok(Some(VersionedEntry {
                entry: Entry::bytes(body),
                version: RecordVersion::from_backend(0),
            })),
            Err(FilesystemError::NotFound { .. }) => Ok(None),
            Err(error) => Err(error),
        }
    }

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
        let metadata = tokio::fs::metadata(&resolved)
            .await
            .map_err(|error| io_error(path.clone(), FilesystemOperation::Stat, error))?;
        Ok(FileStat {
            path: path.clone(),
            file_type: file_type_from_metadata(&metadata),
            len: metadata.len(),
            modified: metadata.modified().ok(),
            sensitive: is_sensitive_path(&resolved),
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
