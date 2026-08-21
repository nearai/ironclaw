use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

use crate::{
    AtomicSubtreeEntry, CasExpectation, DirEntry, Entry, FileStat, FileType, FilesystemError,
    FilesystemOperation, RecordVersion, RootFilesystem, VersionedEntry,
    local_capability::{
        CapabilityFileType, CapabilityWriteError, DiskDirectoryCapability, run_capability_access,
        run_capability_blocking,
    },
    path_prefix_matches,
    root::validate_atomic_subtree_entries,
};
use async_trait::async_trait;
use ironclaw_host_api::path::{HostPath, VirtualPath};
use ironclaw_safety::sensitive_paths::is_sensitive_path;

static LOCAL_WRITE_TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// The on-disk `RootFilesystem` backend, mounted into the virtual namespace.
///
/// The name states the **storage medium** — disk, a peer of `InMemoryBackend`,
/// `LibSqlRootFilesystem`, and `PostgresRootFilesystem` — not a deployment mode.
/// Renamed from `LocalFilesystem` because `Local` read like a deployment tier
/// while this is simply the disk backend a `DeploymentConfig` may select
/// (arch-simplification §4.4 Bucket 2).
#[derive(Debug, Default)]
pub struct DiskFilesystem {
    mounts: Vec<LocalMount>,
}

#[derive(Debug, Clone)]
struct LocalMount {
    virtual_root: VirtualPath,
    host_root: PathBuf,
    capability: DiskDirectoryCapability,
    /// When `true`, this mount is shared by many callers who are each only
    /// ever granted a single leaf subtree of it (one [`MountGrant`] target
    /// per caller, narrowed by the composition-layer `MountView` resolver —
    /// e.g. the sandboxed-profile `/workspace` mount, where every user's
    /// `MountView` target is `/workspace/<digest>`). Containment for such a
    /// mount rejects the bare shared root. The first path segment after
    /// `virtual_root` is server-selected by the caller's [`MountGrant`], and
    /// every later component is traversed without following symlinks from the
    /// retained shared-root capability.
    leaf_scoped: bool,
}

impl DiskFilesystem {
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
        self.mount_local_impl(virtual_root, host_root, false)
    }

    /// Creates and admits a local mount without following path components,
    /// retaining the opened directory capability for all later writes.
    pub async fn mount_local_create(
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
        let host_path = host_root.as_path().to_path_buf();
        let capability =
            run_capability_blocking(move || DiskDirectoryCapability::admit_or_create(&host_path))
                .await
                .map_err(|error| {
                    local_capability_error(
                        virtual_root.clone(),
                        FilesystemOperation::MountLocal,
                        error,
                    )
                })?;
        self.mounts.push(LocalMount {
            virtual_root,
            host_root: host_root.as_path().to_path_buf(),
            capability,
            leaf_scoped: false,
        });
        Ok(())
    }

    /// Mounts an already-admitted directory capability without reopening the
    /// ambient host pathname between admission and mount installation.
    pub fn mount_local_capability(
        &mut self,
        virtual_root: VirtualPath,
        host_root: HostPath,
        capability: DiskDirectoryCapability,
    ) -> Result<(), FilesystemError> {
        if self
            .mounts
            .iter()
            .any(|mount| mount.virtual_root.as_str() == virtual_root.as_str())
        {
            return Err(FilesystemError::MountConflict { path: virtual_root });
        }
        let host_path = host_root.as_path();
        let matches = capability
            .matches_existing_path(host_path)
            .map_err(|error| {
                local_capability_error(virtual_root.clone(), FilesystemOperation::MountLocal, error)
            })?;
        if !matches {
            return Err(FilesystemError::Backend {
                path: virtual_root,
                operation: FilesystemOperation::MountLocal,
                reason: "retained directory capability does not match the supplied host root"
                    .to_string(),
            });
        }
        self.mounts.push(LocalMount {
            virtual_root,
            host_root: host_root.as_path().to_path_buf(),
            capability,
            leaf_scoped: false,
        });
        Ok(())
    }

    /// Mounts a host directory shared across many callers, each of whom is
    /// only ever granted (via their own `MountView`) a single leaf subtree
    /// of it — e.g. the `HostedSingleTenantVolumeSandboxed` profile's
    /// `/workspace` mount, whose shared parent holds every user's leaf
    /// sandbox-workspace directory. Containment for paths resolved through
    /// this mount is pinned per-request to `host_root/<leaf>`, where `<leaf>`
    /// is the first path segment after `virtual_root` — closing a symlink
    /// planted inside one caller's leaf from resolving into a sibling leaf,
    /// which a plain [`mount_local`](Self::mount_local) mount (containment at
    /// the shared root itself would expose. See [`LocalMount::leaf_scoped`].
    pub fn mount_local_per_leaf(
        &mut self,
        virtual_root: VirtualPath,
        host_root: HostPath,
    ) -> Result<(), FilesystemError> {
        self.mount_local_impl(virtual_root, host_root, true)
    }

    fn mount_local_impl(
        &mut self,
        virtual_root: VirtualPath,
        host_root: HostPath,
        leaf_scoped: bool,
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
        let capability =
            DiskDirectoryCapability::from_existing(&canonical_root).map_err(|error| {
                local_capability_error(virtual_root.clone(), FilesystemOperation::MountLocal, error)
            })?;

        self.mounts.push(LocalMount {
            virtual_root,
            host_root: canonical_root,
            capability,
            leaf_scoped,
        });
        Ok(())
    }

    /// Resolves `path` to its host-side join point plus the containment
    /// invariant callers must enforce against it.
    ///
    /// Reads and writes both traverse from the retained capability. The joined
    /// path is retained only for lexical sensitivity classification; it is
    /// never reopened for filesystem I/O.
    fn resolve_joined(&self, path: &VirtualPath) -> Result<ResolvedMountPath, FilesystemError> {
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
        let mut relative = PathBuf::new();
        if tail.is_empty() {
            // A leaf-scoped mount has no safe containment root for the bare
            // mount path itself — that would be "every caller's leaf", the
            // shared-parent boundary this mount kind exists to eliminate.
            // The composition-layer `MountView` always supplies a leaf, but
            // that invariant is enforced one layer up, so fail closed here.
            if mount.leaf_scoped {
                return Err(FilesystemError::PathOutsideMount { path: path.clone() });
            }
        } else {
            for segment in tail.split('/') {
                joined.push(segment);
                relative.push(segment);
            }
        }
        Ok(ResolvedMountPath {
            joined,
            relative,
            capability: mount.capability.clone(),
        })
    }
}

/// Capability-relative resolution plus the lexical host path used only for
/// sensitivity classification.
struct ResolvedMountPath {
    /// Lexical host path used only for sensitivity classification, never I/O.
    joined: PathBuf,
    /// Capability-relative path, never containing parent or root components.
    relative: PathBuf,
    /// Retained mount-root handle used by mutating operations.
    capability: DiskDirectoryCapability,
}

#[async_trait]
impl RootFilesystem for DiskFilesystem {
    /// Native `put` for the byte-only local filesystem. Opaque-file entries
    /// (`kind = None`, empty `indexed`) support `CasExpectation::Any` and
    /// `CasExpectation::Absent`; record-shaped entries, populated indexed
    /// projections, and `Version(_)` are `Unsupported` because the local
    /// filesystem has no native metadata or version tracking (sidecar
    /// metadata is a future addition; see the reborn storage rework plan).
    /// We implement `put` here rather than relying on a trait default so that
    /// the put/write_file pair is non-recursive even when downstream consumers
    /// route through `put`.
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
        if matches!(cas, CasExpectation::Version(_)) {
            return Err(FilesystemError::Unsupported {
                path: path.clone(),
                operation: FilesystemOperation::WriteFile,
            });
        }
        self.write_file_with_cas(path, &entry.body, cas).await?;
        Ok(RecordVersion::from_backend(0))
    }

    /// Native `get` mirroring `put`: read the bytes and wrap as an opaque
    /// `Entry`. Version is always `0` because the local filesystem doesn't
    /// track per-path versions. Non-existent paths return `Ok(None)`;
    /// directories or symlinks return their respective `read_file` errors.
    async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        match self.read_file(path).await {
            Ok(body) => Ok(Some(VersionedEntry {
                path: path.clone(),
                entry: Entry::bytes(body),
                version: RecordVersion::from_backend(0),
            })),
            Err(FilesystemError::NotFound { .. }) => Ok(None),
            Err(error) => Err(error),
        }
    }

    async fn create_subtree_atomic(
        &self,
        prefix: &VirtualPath,
        entries: Vec<AtomicSubtreeEntry>,
    ) -> Result<Vec<RecordVersion>, FilesystemError> {
        validate_local_atomic_subtree(prefix, &entries)?;
        let resolved = self.resolve_joined(prefix)?;
        let relative_prefix = resolved.relative;
        let capability = resolved.capability;
        let prefix_with_separator = format!("{}/", prefix.as_str().trim_end_matches('/'));
        let entry_count = entries.len();
        let mut relative_entries = Vec::with_capacity(entry_count);
        for item in entries {
            let relative = item
                .path
                .as_str()
                .strip_prefix(&prefix_with_separator)
                .ok_or_else(|| FilesystemError::PathOutsideMount {
                    path: item.path.clone(),
                })?;
            relative_entries.push((PathBuf::from(relative), item.entry.body));
        }
        let counter = LOCAL_WRITE_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        match tokio::task::spawn_blocking(move || {
            capability.create_subtree_atomic(&relative_prefix, relative_entries, counter)
        })
        .await
        .map_err(|error| {
            local_capability_error(
                prefix.clone(),
                FilesystemOperation::CreateSubtreeAtomic,
                std::io::Error::other(error),
            )
        })? {
            Ok(()) => Ok(vec![RecordVersion::from_backend(0); entry_count]),
            Err(CapabilityWriteError::Io(error)) => Err(local_capability_error(
                prefix.clone(),
                FilesystemOperation::CreateSubtreeAtomic,
                error,
            )),
            Err(CapabilityWriteError::SymlinkEscape) => Err(FilesystemError::SymlinkEscape {
                path: prefix.clone(),
            }),
            Err(CapabilityWriteError::VersionMismatch) => Err(FilesystemError::VersionMismatch {
                path: prefix.clone(),
                expected: None,
                found: Some(RecordVersion::from_backend(0)),
            }),
            Err(CapabilityWriteError::UnsupportedVersion) => Err(FilesystemError::Unsupported {
                path: prefix.clone(),
                operation: FilesystemOperation::CreateSubtreeAtomic,
            }),
        }
    }

    async fn read_file(&self, path: &VirtualPath) -> Result<Vec<u8>, FilesystemError> {
        let resolved = self.resolve_joined(path)?;
        let relative = resolved.relative;
        let capability = resolved.capability;
        run_capability_access(move || capability.read_file(&relative, None))
            .await
            .map_err(|error| capability_access_error(path, FilesystemOperation::ReadFile, error))?
            .ok_or_else(|| FilesystemError::Backend {
                path: path.clone(),
                operation: FilesystemOperation::ReadFile,
                reason: "unbounded file read returned no bytes".to_string(),
            })
    }

    async fn read_file_bounded(
        &self,
        path: &VirtualPath,
        max_bytes: usize,
    ) -> Result<Option<Vec<u8>>, FilesystemError> {
        let resolved = self.resolve_joined(path)?;
        let relative = resolved.relative;
        let capability = resolved.capability;
        run_capability_access(move || capability.read_file(&relative, Some(max_bytes)))
            .await
            .map_err(|error| capability_access_error(path, FilesystemOperation::ReadFile, error))
    }

    async fn write_file(&self, path: &VirtualPath, bytes: &[u8]) -> Result<(), FilesystemError> {
        self.write_file_with_cas(path, bytes, CasExpectation::Any)
            .await
    }

    async fn append_file(&self, path: &VirtualPath, bytes: &[u8]) -> Result<(), FilesystemError> {
        let resolved = self.resolve_joined(path)?;
        let relative = resolved.relative;
        let capability = resolved.capability;
        let bytes = bytes.to_vec();
        match tokio::task::spawn_blocking(move || capability.append(&relative, &bytes))
            .await
            .map_err(|error| {
                local_capability_error(
                    path.clone(),
                    FilesystemOperation::AppendFile,
                    std::io::Error::other(error),
                )
            })? {
            Ok(()) => Ok(()),
            Err(CapabilityWriteError::Io(error)) => Err(local_capability_error(
                path.clone(),
                FilesystemOperation::AppendFile,
                error,
            )),
            Err(CapabilityWriteError::SymlinkEscape) => {
                Err(FilesystemError::SymlinkEscape { path: path.clone() })
            }
            Err(CapabilityWriteError::VersionMismatch) => Err(FilesystemError::VersionMismatch {
                path: path.clone(),
                expected: None,
                found: Some(RecordVersion::from_backend(0)),
            }),
            Err(CapabilityWriteError::UnsupportedVersion) => Err(FilesystemError::Unsupported {
                path: path.clone(),
                operation: FilesystemOperation::AppendFile,
            }),
        }
    }

    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        self.list_dir_bounded(path, usize::MAX).await
    }

    async fn list_dir_bounded(
        &self,
        path: &VirtualPath,
        max_entries: usize,
    ) -> Result<Vec<DirEntry>, FilesystemError> {
        let resolved = self.resolve_joined(path)?;
        let relative = resolved.relative;
        let capability = resolved.capability;
        let raw_entries =
            run_capability_access(move || capability.list_dir_bounded(&relative, max_entries))
                .await
                .map_err(|error| {
                    capability_access_error(path, FilesystemOperation::ListDir, error)
                })?;
        let mut entries = Vec::new();
        for entry in raw_entries {
            let name = entry.name.to_string_lossy().to_string();
            let entry_path =
                VirtualPath::new(format!("{}/{}", path.as_str().trim_end_matches('/'), name))?;
            entries.push(DirEntry {
                name,
                path: entry_path,
                file_type: file_type_from_capability(entry.file_type),
            });
        }
        entries.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(entries)
    }

    async fn list_dir_page(
        &self,
        path: &VirtualPath,
        after: Option<&str>,
        max_entries: usize,
    ) -> Result<Vec<DirEntry>, FilesystemError> {
        if max_entries == 0 {
            return Ok(Vec::new());
        }
        let resolved = self.resolve_joined(path)?;
        let relative = resolved.relative;
        let capability = resolved.capability;
        let after = after.map(str::to_owned);
        let raw_entries = run_capability_access(move || {
            capability.list_dir_page(&relative, after.as_deref(), max_entries)
        })
        .await
        .map_err(|error| capability_access_error(path, FilesystemOperation::ListDir, error))?;
        let mut page = Vec::with_capacity(raw_entries.len());
        for entry in raw_entries {
            let name = entry.name.to_string_lossy().to_string();
            let entry_path =
                VirtualPath::new(format!("{}/{}", path.as_str().trim_end_matches('/'), name))?;
            page.push(DirEntry {
                name,
                path: entry_path,
                file_type: file_type_from_capability(entry.file_type),
            });
        }
        Ok(page)
    }

    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        let resolved = self.resolve_joined(path)?;
        let sensitive = is_sensitive_path(&resolved.joined);
        let relative = resolved.relative;
        let capability = resolved.capability;
        let metadata = run_capability_access(move || capability.metadata(&relative))
            .await
            .map_err(|error| capability_access_error(path, FilesystemOperation::Stat, error))?;
        Ok(FileStat {
            path: path.clone(),
            file_type: file_type_from_capability(metadata.file_type),
            len: metadata.len,
            modified: metadata.modified,
            sensitive,
        })
    }

    async fn delete(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        let resolved = self.resolve_joined(path)?;
        let relative = resolved.relative;
        let capability = resolved.capability;
        run_capability_access(move || capability.remove(&relative))
            .await
            .map_err(|error| capability_access_error(path, FilesystemOperation::Delete, error))
    }

    async fn create_dir_all(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        let resolved = self.resolve_joined(path)?;
        let relative = resolved.relative;
        let capability = resolved.capability;
        run_capability_blocking(move || capability.create_dir_all(&relative))
            .await
            .map_err(|error| {
                local_capability_error(path.clone(), FilesystemOperation::CreateDirAll, error)
            })
    }
}

impl DiskFilesystem {
    async fn write_file_with_cas(
        &self,
        path: &VirtualPath,
        bytes: &[u8],
        cas: CasExpectation,
    ) -> Result<(), FilesystemError> {
        let resolved = self.resolve_joined(path)?;
        let relative = resolved.relative;
        let capability = resolved.capability;
        let bytes = bytes.to_vec();
        let counter = LOCAL_WRITE_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        match tokio::task::spawn_blocking(move || {
            capability.atomic_write(&relative, &bytes, cas, counter)
        })
        .await
        .map_err(|error| {
            local_capability_error(
                path.clone(),
                FilesystemOperation::WriteFile,
                std::io::Error::other(error),
            )
        })? {
            Ok(()) => Ok(()),
            Err(CapabilityWriteError::Io(error)) => Err(local_capability_error(
                path.clone(),
                FilesystemOperation::WriteFile,
                error,
            )),
            Err(CapabilityWriteError::SymlinkEscape) => {
                Err(FilesystemError::SymlinkEscape { path: path.clone() })
            }
            Err(CapabilityWriteError::VersionMismatch) => Err(FilesystemError::VersionMismatch {
                path: path.clone(),
                expected: None,
                found: Some(RecordVersion::from_backend(0)),
            }),
            Err(CapabilityWriteError::UnsupportedVersion) => Err(FilesystemError::Unsupported {
                path: path.clone(),
                operation: FilesystemOperation::WriteFile,
            }),
        }
    }
}

fn local_capability_error(
    path: VirtualPath,
    operation: FilesystemOperation,
    source: std::io::Error,
) -> FilesystemError {
    FilesystemError::LocalCapability {
        path,
        operation,
        source,
    }
}

fn capability_access_error(
    path: &VirtualPath,
    operation: FilesystemOperation,
    error: CapabilityWriteError,
) -> FilesystemError {
    match error {
        CapabilityWriteError::Io(error) => io_error(path.clone(), operation, error),
        CapabilityWriteError::SymlinkEscape => {
            FilesystemError::SymlinkEscape { path: path.clone() }
        }
        CapabilityWriteError::VersionMismatch | CapabilityWriteError::UnsupportedVersion => {
            FilesystemError::Unsupported {
                path: path.clone(),
                operation,
            }
        }
    }
}

fn file_type_from_capability(file_type: CapabilityFileType) -> FileType {
    match file_type {
        CapabilityFileType::File => FileType::File,
        CapabilityFileType::Directory => FileType::Directory,
        CapabilityFileType::Other => FileType::Other,
    }
}

fn validate_local_atomic_subtree(
    prefix: &VirtualPath,
    entries: &[AtomicSubtreeEntry],
) -> Result<(), FilesystemError> {
    validate_atomic_subtree_entries(prefix, entries)?;
    for item in entries {
        if item.entry.kind.is_some() || !item.entry.indexed.is_empty() {
            return Err(FilesystemError::Unsupported {
                path: item.path.clone(),
                operation: FilesystemOperation::CreateSubtreeAtomic,
            });
        }
    }
    Ok(())
}

fn io_error(
    path: VirtualPath,
    operation: FilesystemOperation,
    error: std::io::Error,
) -> FilesystemError {
    if error.kind() == std::io::ErrorKind::NotFound {
        return FilesystemError::NotFound { path, operation };
    }

    tracing::debug!(
        virtual_path = path.as_str(),
        %operation,
        error = %error,
        "local filesystem backend error"
    );
    FilesystemError::Backend {
        path,
        operation,
        reason: error.kind().to_string(),
    }
}

fn io_reason(error: std::io::Error) -> String {
    error.kind().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[cfg(unix)]
    #[tokio::test]
    async fn capability_mount_does_not_reopen_a_replaced_ambient_path() {
        #[cfg(target_os = "macos")]
        let temp = tempfile::Builder::new()
            .tempdir_in("/private/tmp")
            .expect("temporary root");
        #[cfg(not(target_os = "macos"))]
        let temp = tempdir().expect("temporary root");
        let admitted_path = temp.path().join("prompts");
        let moved_path = temp.path().join("original-prompts");
        let outside = temp.path().join("outside");
        std::fs::create_dir(&admitted_path).expect("prompts root");
        std::fs::create_dir(&outside).expect("outside root");
        let admitted =
            DiskDirectoryCapability::admit_or_create(&admitted_path).expect("admit prompts root");

        std::fs::rename(&admitted_path, &moved_path).expect("move admitted root");
        std::os::unix::fs::symlink(&outside, &admitted_path).expect("replace ambient path");

        let mut filesystem = DiskFilesystem::new();
        filesystem
            .mount_local_capability(
                VirtualPath::new("/system/prompts").unwrap(),
                HostPath::from_path_buf(moved_path.clone()),
                admitted,
            )
            .expect("mount retained capability");
        filesystem
            .write_file(
                &VirtualPath::new("/system/prompts/default.md").unwrap(),
                b"trusted prompt",
            )
            .await
            .expect("write through admitted mount");

        assert_eq!(
            std::fs::read(moved_path.join("default.md")).expect("original tree write"),
            b"trusted prompt"
        );
        assert!(!outside.join("default.md").exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn capability_mount_reads_from_retained_root_after_ambient_replacement() {
        #[cfg(target_os = "macos")]
        let temp = tempfile::Builder::new()
            .tempdir_in("/private/tmp")
            .expect("temporary root");
        #[cfg(not(target_os = "macos"))]
        let temp = tempdir().expect("temporary root");
        let admitted_path = temp.path().join("workspace");
        let moved_path = temp.path().join("original-workspace");
        std::fs::create_dir(&admitted_path).expect("workspace root");
        std::fs::write(admitted_path.join("state.txt"), b"trusted").expect("trusted file");
        let admitted =
            DiskDirectoryCapability::admit_or_create(&admitted_path).expect("admit workspace");

        let mut filesystem = DiskFilesystem::new();
        filesystem
            .mount_local_capability(
                VirtualPath::new("/projects/workspace").expect("virtual root"),
                HostPath::from_path_buf(admitted_path.clone()),
                admitted,
            )
            .expect("mount retained capability");

        std::fs::rename(&admitted_path, &moved_path).expect("move admitted root");
        std::fs::create_dir(&admitted_path).expect("replacement root");
        std::fs::write(admitted_path.join("state.txt"), b"replacement").expect("replacement file");

        let bytes = filesystem
            .read_file(&VirtualPath::new("/projects/workspace/state.txt").expect("virtual file"))
            .await
            .expect("read through retained capability");
        assert_eq!(bytes, b"trusted");
    }

    #[cfg(unix)]
    #[test]
    fn capability_mount_rejects_a_mismatched_host_root() {
        #[cfg(target_os = "macos")]
        let temp = tempfile::Builder::new()
            .tempdir_in("/private/tmp")
            .expect("temporary root");
        #[cfg(not(target_os = "macos"))]
        let temp = tempdir().expect("temporary root");
        let admitted_path = temp.path().join("admitted");
        let different_path = temp.path().join("different");
        std::fs::create_dir(&admitted_path).expect("admitted root");
        std::fs::create_dir(&different_path).expect("different root");
        let admitted =
            DiskDirectoryCapability::admit_or_create(&admitted_path).expect("admit root");
        let mut filesystem = DiskFilesystem::new();

        let error = filesystem
            .mount_local_capability(
                VirtualPath::new("/projects/workspace").expect("virtual root"),
                HostPath::from_path_buf(different_path),
                admitted,
            )
            .expect_err("path and retained capability must name the same directory");

        assert!(error.to_string().contains("does not match"), "{error}");
    }

    #[tokio::test]
    #[tracing_test::traced_test]
    async fn missing_local_paths_do_not_log_backend_error() {
        let storage = tempdir().unwrap();
        let mut root = DiskFilesystem::new();
        root.mount_local(
            VirtualPath::new("/projects").unwrap(),
            HostPath::from_path_buf(storage.path().to_path_buf()),
        )
        .unwrap();

        let read_error = root
            .read_file(&VirtualPath::new("/projects/missing.txt").unwrap())
            .await
            .unwrap_err();
        let stat_error = root
            .stat(&VirtualPath::new("/projects/also-missing.txt").unwrap())
            .await
            .unwrap_err();

        assert!(matches!(read_error, FilesystemError::NotFound { .. }));
        assert!(matches!(stat_error, FilesystemError::NotFound { .. }));
        assert!(!logs_contain("local filesystem backend error"));
    }

    #[test]
    #[tracing_test::traced_test]
    fn non_not_found_io_error_logs_backend_error() {
        let error = io_error(
            VirtualPath::new("/projects/secret.txt").unwrap(),
            FilesystemOperation::ReadFile,
            std::io::Error::from(std::io::ErrorKind::PermissionDenied),
        );

        assert!(matches!(error, FilesystemError::Backend { .. }));
        assert!(logs_contain("local filesystem backend error"));
    }

    /// A `mount_local_per_leaf` mount always requires the caller's own leaf
    /// (`host_root/<first-tail-segment>`) in the granted path — there is no
    /// safe interpretation of the bare mount path itself
    /// (that would mean "every caller's leaf", the exact shared-parent
    /// boundary `mount_local_per_leaf` exists to eliminate). Today every
    /// legitimate grant against such a mount always resolves to a
    /// leaf-prefixed target (`sandbox_user_workspace_mount_view` in
    /// `ironclaw_composition`), but that is an invariant enforced one
    /// layer up, not by this crate — so a bare-root request must fail closed
    /// here rather than silently fall back to the full shared parent.
    #[tokio::test]
    async fn leaf_scoped_mount_rejects_bare_mount_root_request() {
        let storage = tempdir().unwrap();
        let mut root = DiskFilesystem::new();
        root.mount_local_per_leaf(
            VirtualPath::new("/tmp").unwrap(),
            HostPath::from_path_buf(storage.path().to_path_buf()),
        )
        .unwrap();

        let error = root
            .read_file(&VirtualPath::new("/tmp").unwrap())
            .await
            .unwrap_err();

        assert!(
            matches!(error, FilesystemError::PathOutsideMount { .. }),
            "expected PathOutsideMount, got: {error:?}"
        );
    }

    /// The actual escape `leaf_scoped` containment exists to close: two
    /// callers share one `mount_local_per_leaf` `host_root`, each confined to
    /// their own leaf (`leaf-a`, `leaf-b`). A symlink planted inside
    /// `leaf-a` pointing at `../leaf-b/secret.txt` stays within the shared
    /// `host_root` — a plain `mount_local` containment check (host_root
    /// only) would let it resolve — but leaves `leaf-a`'s own containment
    /// root, so it must be rejected here.
    #[cfg(unix)]
    #[tokio::test]
    async fn leaf_scoped_mount_rejects_cross_leaf_symlink_escape() {
        let storage = tempdir().unwrap();
        let host_root = storage.path();

        let leaf_a = host_root.join("leaf-a");
        let leaf_b = host_root.join("leaf-b");
        std::fs::create_dir_all(&leaf_a).unwrap();
        std::fs::create_dir_all(&leaf_b).unwrap();
        std::fs::write(leaf_b.join("secret.txt"), b"leaf-b secret").unwrap();
        std::os::unix::fs::symlink("../leaf-b/secret.txt", leaf_a.join("escape.txt")).unwrap();

        let mut root = DiskFilesystem::new();
        root.mount_local_per_leaf(
            VirtualPath::new("/tmp").unwrap(),
            HostPath::from_path_buf(host_root.to_path_buf()),
        )
        .unwrap();

        let error = root
            .read_file(&VirtualPath::new("/tmp/leaf-a/escape.txt").unwrap())
            .await
            .unwrap_err();

        assert!(
            matches!(error, FilesystemError::SymlinkEscape { .. }),
            "expected SymlinkEscape, got: {error:?}"
        );
    }

    /// A retained shared-root capability may safely create the caller's
    /// server-selected leaf without ambient-path bootstrap checks.
    #[tokio::test]
    async fn leaf_scoped_mount_creates_a_brand_new_leaf_on_first_write() {
        let storage = tempdir().unwrap();
        let host_root = storage.path();

        let mut root = DiskFilesystem::new();
        root.mount_local_per_leaf(
            VirtualPath::new("/tmp").unwrap(),
            HostPath::from_path_buf(host_root.to_path_buf()),
        )
        .unwrap();

        root.write_file(
            &VirtualPath::new("/tmp/new-leaf/file.txt").unwrap(),
            b"hello",
        )
        .await
        .unwrap();

        let bytes = root
            .read_file(&VirtualPath::new("/tmp/new-leaf/file.txt").unwrap())
            .await
            .unwrap();
        assert_eq!(bytes, b"hello");
    }

    /// Same first-use bootstrap through capability-relative `create_dir_all`.
    #[tokio::test]
    async fn leaf_scoped_mount_create_dir_all_bootstraps_a_brand_new_leaf() {
        let storage = tempdir().unwrap();
        let host_root = storage.path();

        let mut root = DiskFilesystem::new();
        root.mount_local_per_leaf(
            VirtualPath::new("/tmp").unwrap(),
            HostPath::from_path_buf(host_root.to_path_buf()),
        )
        .unwrap();

        root.create_dir_all(&VirtualPath::new("/tmp/new-leaf/nested").unwrap())
            .await
            .unwrap();

        assert!(host_root.join("new-leaf").join("nested").is_dir());
    }

    /// Bootstrapping a new leaf must not reopen the cross-leaf symlink
    /// escape the write path closes: a *pre-existing* sibling leaf's
    /// symlink must still be rejected by `resolve_for_write`
    /// (`append_file`/`write_file`), not just by `read_file`.
    #[cfg(unix)]
    #[tokio::test]
    async fn leaf_scoped_mount_rejects_cross_leaf_symlink_escape_on_write() {
        let storage = tempdir().unwrap();
        let host_root = storage.path();

        let leaf_a = host_root.join("leaf-a");
        let leaf_b = host_root.join("leaf-b");
        std::fs::create_dir_all(&leaf_a).unwrap();
        std::fs::create_dir_all(&leaf_b).unwrap();
        std::os::unix::fs::symlink("../leaf-b", leaf_a.join("escape")).unwrap();

        let mut root = DiskFilesystem::new();
        root.mount_local_per_leaf(
            VirtualPath::new("/tmp").unwrap(),
            HostPath::from_path_buf(host_root.to_path_buf()),
        )
        .unwrap();

        let error = root
            .write_file(
                &VirtualPath::new("/tmp/leaf-a/escape/planted.txt").unwrap(),
                b"planted",
            )
            .await
            .unwrap_err();

        assert!(
            matches!(error, FilesystemError::SymlinkEscape { .. }),
            "expected SymlinkEscape, got: {error:?}"
        );
        assert!(!leaf_b.join("planted.txt").exists());
    }

    /// A *dangling* final symlink — the entry exists but its target does
    /// not — must still be rejected. `tokio::fs::try_exists` follows
    /// symlinks and reports `false` for a target that doesn't exist yet, so
    /// naively treating "not exists" as "brand new file in this leaf" would
    /// let `write_file`/`append_file` open through the symlink (the OS
    /// creates the target on `O_CREAT`), writing into whatever sibling leaf
    /// (or worse) the symlink points at.
    #[cfg(unix)]
    #[tokio::test]
    async fn leaf_scoped_mount_rejects_dangling_final_symlink_escape_on_write() {
        let storage = tempdir().unwrap();
        let host_root = storage.path();

        let leaf_a = host_root.join("leaf-a");
        let leaf_b = host_root.join("leaf-b");
        std::fs::create_dir_all(&leaf_a).unwrap();
        std::fs::create_dir_all(&leaf_b).unwrap();
        std::os::unix::fs::symlink("../leaf-b/planted.txt", leaf_a.join("escape.txt")).unwrap();

        let mut root = DiskFilesystem::new();
        root.mount_local_per_leaf(
            VirtualPath::new("/tmp").unwrap(),
            HostPath::from_path_buf(host_root.to_path_buf()),
        )
        .unwrap();

        let error = root
            .write_file(
                &VirtualPath::new("/tmp/leaf-a/escape.txt").unwrap(),
                b"planted",
            )
            .await
            .unwrap_err();

        assert!(
            matches!(error, FilesystemError::SymlinkEscape { .. }),
            "expected SymlinkEscape, got: {error:?}"
        );
        assert!(!leaf_b.join("planted.txt").exists());
    }

    /// Append must enforce the same no-symlink write boundary as atomic
    /// replacement. Otherwise `O_APPEND | O_CREAT` follows a planted final
    /// symlink and mutates a sibling caller's file.
    #[cfg(unix)]
    #[tokio::test]
    async fn leaf_scoped_mount_rejects_final_symlink_escape_on_append() {
        let storage = tempdir().unwrap();
        let host_root = storage.path();

        let leaf_a = host_root.join("leaf-a");
        let leaf_b = host_root.join("leaf-b");
        std::fs::create_dir_all(&leaf_a).unwrap();
        std::fs::create_dir_all(&leaf_b).unwrap();
        std::fs::write(leaf_b.join("secret.txt"), b"original").unwrap();
        std::os::unix::fs::symlink("../leaf-b/secret.txt", leaf_a.join("escape.txt")).unwrap();

        let mut root = DiskFilesystem::new();
        root.mount_local_per_leaf(
            VirtualPath::new("/tmp").unwrap(),
            HostPath::from_path_buf(host_root.to_path_buf()),
        )
        .unwrap();

        let error = root
            .append_file(
                &VirtualPath::new("/tmp/leaf-a/escape.txt").unwrap(),
                b"-planted",
            )
            .await
            .unwrap_err();

        assert!(
            matches!(error, FilesystemError::SymlinkEscape { .. }),
            "expected SymlinkEscape, got: {error:?}"
        );
        assert_eq!(
            std::fs::read(leaf_b.join("secret.txt")).unwrap(),
            b"original"
        );
    }

    /// Linux retains directory capabilities as `O_PATH` descriptors. A
    /// successful publication must not be reported as failed merely because
    /// durability sync used the traversal-only descriptor.
    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn nested_create_only_write_succeeds_without_false_post_commit_error() {
        let storage = tempdir().unwrap();
        let mut root = DiskFilesystem::new();
        root.mount_local(
            VirtualPath::new("/projects").unwrap(),
            HostPath::from_path_buf(storage.path().to_path_buf()),
        )
        .unwrap();
        let path = VirtualPath::new("/projects/nested/state.json").unwrap();

        root.put(
            &path,
            Entry::bytes(b"first".to_vec()),
            CasExpectation::Absent,
        )
        .await
        .expect("published write must report success");

        let error = root
            .put(
                &path,
                Entry::bytes(b"second".to_vec()),
                CasExpectation::Absent,
            )
            .await
            .unwrap_err();
        assert!(matches!(error, FilesystemError::VersionMismatch { .. }));
        assert_eq!(
            std::fs::read(storage.path().join("nested/state.json")).unwrap(),
            b"first"
        );
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn atomic_subtree_publication_succeeds_with_directory_durability_sync() {
        let storage = tempdir().unwrap();
        let mut root = DiskFilesystem::new();
        root.mount_local(
            VirtualPath::new("/projects").unwrap(),
            HostPath::from_path_buf(storage.path().to_path_buf()),
        )
        .unwrap();
        let prefix = VirtualPath::new("/projects/install").unwrap();

        root.create_subtree_atomic(
            &prefix,
            vec![AtomicSubtreeEntry {
                path: VirtualPath::new("/projects/install/nested/manifest.json").unwrap(),
                entry: Entry::bytes(b"manifest".to_vec()),
            }],
        )
        .await
        .expect("atomic subtree publication must report success");

        assert_eq!(
            std::fs::read(storage.path().join("install/nested/manifest.json")).unwrap(),
            b"manifest"
        );
    }
}
