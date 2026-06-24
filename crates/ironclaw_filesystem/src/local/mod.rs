//! Local filesystem backend mounted into the virtual namespace.
//!
//! ## TOCTOU hardening
//!
//! Each mount opens its root directory **once** during trusted setup
//! ([`LocalFilesystem::mount_local`]) and stores the resulting
//! [`OwnedFd`](std::os::fd::OwnedFd). Every runtime operation resolves the
//! caller's virtual-path tail *relative to that fd* using race-free fd-relative
//! traversal (`openat2(RESOLVE_BENEATH)` on Linux, an `openat(O_NOFOLLOW)`
//! per-component walk elsewhere — see [`resolver`]). No operation ever
//! re-resolves an absolute host path or trusts `canonicalize`, so containment
//! within the mount root holds **by construction**: a concurrent ancestor
//! symlink swap cannot redirect an open outside the root.
//!
//! The synchronous rustix `*at` syscalls and the subsequent fd-bound IO are
//! executed on `tokio::task::spawn_blocking` to preserve the async contract.

mod resolver;

use std::os::fd::OwnedFd;
use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::{HostPath, VirtualPath};
use ironclaw_safety::sensitive_paths::is_sensitive_path_str;
use rustix::fs::{Mode, OFlags, RawMode};

use crate::{
    CasExpectation, DirEntry, Entry, FileStat, FileType, FilesystemError, FilesystemOperation,
    RecordVersion, RootFilesystem, VersionedEntry, path_prefix_matches,
};

use resolver::{
    AtFlags, ResolveError, ResolvedTail, RustixDir, RustixFileType, as_os_str, components,
    open_beneath, open_parent_dir, tail_from_components,
};

/// Default mode for files created by the local backend (0o644 before umask).
const FILE_CREATE_MODE: RawMode = 0o644;
/// Default mode for directories created by the local backend (0o755 before umask).
const DIR_CREATE_MODE: RawMode = 0o755;

/// Local filesystem backend mounted into the virtual namespace.
#[derive(Debug, Default)]
pub struct LocalFilesystem {
    mounts: Vec<LocalMount>,
}

#[derive(Debug, Clone)]
struct LocalMount {
    virtual_root: VirtualPath,
    /// Owned fd onto the mount root directory, opened once at mount time.
    /// Every operation resolves fd-relative to this handle.
    root_dir: Arc<OwnedFd>,
    /// Canonicalized host path of the mount root, captured once during trusted
    /// `mount_local` setup. Used **only** for string-based sensitivity
    /// classification of the host path (never re-opened, never used for a
    /// runtime path resolution — opens always go through `root_dir`). Storing
    /// the canonical root lets `stat` classify the *host* path rather than the
    /// virtual path, so a mount whose virtual root differs from its host root
    /// can't hide a sensitive host location (e.g. host `…/.ssh` mounted at
    /// virtual `/memory`).
    host_root: Arc<std::path::PathBuf>,
}

impl LocalFilesystem {
    pub fn new() -> Self {
        Self::default()
    }

    /// Mounts a host directory during trusted setup.
    ///
    /// This API is intentionally synchronous because it mutates in-memory mount
    /// configuration and is not part of the async runtime operation path. The
    /// root directory fd is opened here, once, under the trusted setup path.
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

        // Open the mount root directory once. This is the single trusted
        // absolute-path resolution; every later operation is fd-relative to the
        // returned handle and never touches `canonical_root` for opens.
        let root_dir = rustix::fs::open(
            &canonical_root,
            OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::RDONLY,
            Mode::empty(),
        )
        .map_err(|errno| FilesystemError::Backend {
            path: virtual_root.clone(),
            operation: FilesystemOperation::MountLocal,
            reason: errno_reason(errno),
        })?;

        self.mounts.push(LocalMount {
            virtual_root,
            root_dir: Arc::new(root_dir),
            host_root: Arc::new(canonical_root),
        });
        Ok(())
    }

    /// Resolve `path` to its owning mount and the validated tail relative to the
    /// mount root. Replaces the old `resolve_joined` + `host_root.join` path:
    /// we return the *tail*, never an absolute joined host path.
    fn resolve_mount_tail(
        &self,
        path: &VirtualPath,
        operation: FilesystemOperation,
    ) -> Result<(&LocalMount, ResolvedTail), FilesystemError> {
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

        let resolved = ResolvedTail::parse(tail)
            .map_err(|error| map_resolve_error(error, path.clone(), operation))?;
        Ok((mount, resolved))
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
                path: path.clone(),
                entry: Entry::bytes(body),
                version: RecordVersion::from_backend(0),
            })),
            Err(FilesystemError::NotFound { .. }) => Ok(None),
            Err(error) => Err(error),
        }
    }

    async fn read_file(&self, path: &VirtualPath) -> Result<Vec<u8>, FilesystemError> {
        let (mount, tail) = self.resolve_mount_tail(path, FilesystemOperation::ReadFile)?;
        let root = Arc::clone(&mount.root_dir);
        let vpath = path.clone();
        run_blocking(path.clone(), FilesystemOperation::ReadFile, move || {
            let fd = open_beneath(&root, &tail, OFlags::RDONLY, Mode::empty())
                .map_err(|e| map_resolve_error(e, vpath.clone(), FilesystemOperation::ReadFile))?;
            let mut file = std::fs::File::from(fd);
            let mut bytes = Vec::new();
            std::io::Read::read_to_end(&mut file, &mut bytes)
                .map_err(|e| io_error(vpath.clone(), FilesystemOperation::ReadFile, e))?;
            Ok(bytes)
        })
        .await
    }

    async fn read_file_bounded(
        &self,
        path: &VirtualPath,
        max_bytes: usize,
    ) -> Result<Option<Vec<u8>>, FilesystemError> {
        let (mount, tail) = self.resolve_mount_tail(path, FilesystemOperation::ReadFile)?;
        let root = Arc::clone(&mount.root_dir);
        let vpath = path.clone();
        run_blocking(path.clone(), FilesystemOperation::ReadFile, move || {
            let fd = open_beneath(&root, &tail, OFlags::RDONLY, Mode::empty())
                .map_err(|e| map_resolve_error(e, vpath.clone(), FilesystemOperation::ReadFile))?;
            // fstat the SAME fd we will read — no second path lookup.
            let stat = rustix::fs::fstat(&fd).map_err(|e| {
                errno_to_filesystem(e, vpath.clone(), FilesystemOperation::ReadFile)
            })?;
            if file_type_from_raw_mode(stat.st_mode) != FileType::File {
                return Err(FilesystemError::Backend {
                    path: vpath.clone(),
                    operation: FilesystemOperation::ReadFile,
                    reason: "not a file".to_string(),
                });
            }
            if stat.st_size as u64 > max_bytes as u64 {
                return Ok(None);
            }
            use std::io::Read as _;
            let file = std::fs::File::from(fd);
            let cap = max_bytes.min(stat.st_size as usize);
            let mut bytes = Vec::with_capacity(cap);
            file.take((max_bytes as u64).saturating_add(1))
                .read_to_end(&mut bytes)
                .map_err(|e| io_error(vpath.clone(), FilesystemOperation::ReadFile, e))?;
            if bytes.len() > max_bytes {
                return Ok(None);
            }
            Ok(Some(bytes))
        })
        .await
    }

    async fn write_file(&self, path: &VirtualPath, bytes: &[u8]) -> Result<(), FilesystemError> {
        let (mount, tail) = self.resolve_mount_tail(path, FilesystemOperation::WriteFile)?;
        let root = Arc::clone(&mount.root_dir);
        let vpath = path.clone();
        let bytes = bytes.to_vec();
        run_blocking(path.clone(), FilesystemOperation::WriteFile, move || {
            // Establish the parent directory hierarchy fd-relative, then open
            // the leaf in the resolved parent with O_NOFOLLOW so a symlink at
            // the leaf is rejected rather than followed.
            ensure_parent_dirs(&root, &tail, &vpath, FilesystemOperation::WriteFile)?;
            let (parent_fd, leaf) = open_parent_dir(&root, &tail)
                .map_err(|e| map_resolve_error(e, vpath.clone(), FilesystemOperation::WriteFile))?;
            let fd = rustix::fs::openat(
                &parent_fd,
                as_os_str(&leaf),
                OFlags::WRONLY
                    | OFlags::CREATE
                    | OFlags::TRUNC
                    | OFlags::NOFOLLOW
                    | OFlags::CLOEXEC,
                Mode::from_raw_mode(FILE_CREATE_MODE),
            )
            .map_err(|e| errno_to_filesystem(e, vpath.clone(), FilesystemOperation::WriteFile))?;
            let mut file = std::fs::File::from(fd);
            std::io::Write::write_all(&mut file, &bytes)
                .map_err(|e| io_error(vpath.clone(), FilesystemOperation::WriteFile, e))?;
            Ok(())
        })
        .await
    }

    async fn append_file(&self, path: &VirtualPath, bytes: &[u8]) -> Result<(), FilesystemError> {
        let (mount, tail) = self.resolve_mount_tail(path, FilesystemOperation::AppendFile)?;
        let root = Arc::clone(&mount.root_dir);
        let vpath = path.clone();
        let bytes = bytes.to_vec();
        run_blocking(path.clone(), FilesystemOperation::AppendFile, move || {
            ensure_parent_dirs(&root, &tail, &vpath, FilesystemOperation::AppendFile)?;
            let (parent_fd, leaf) = open_parent_dir(&root, &tail).map_err(|e| {
                map_resolve_error(e, vpath.clone(), FilesystemOperation::AppendFile)
            })?;
            let fd = rustix::fs::openat(
                &parent_fd,
                as_os_str(&leaf),
                OFlags::WRONLY
                    | OFlags::CREATE
                    | OFlags::APPEND
                    | OFlags::NOFOLLOW
                    | OFlags::CLOEXEC,
                Mode::from_raw_mode(FILE_CREATE_MODE),
            )
            .map_err(|e| errno_to_filesystem(e, vpath.clone(), FilesystemOperation::AppendFile))?;
            let mut file = std::fs::File::from(fd);
            std::io::Write::write_all(&mut file, &bytes)
                .map_err(|e| io_error(vpath.clone(), FilesystemOperation::AppendFile, e))?;
            std::io::Write::flush(&mut file)
                .map_err(|e| io_error(vpath.clone(), FilesystemOperation::AppendFile, e))?;
            Ok(())
        })
        .await
    }

    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        let (mount, tail) = self.resolve_mount_tail(path, FilesystemOperation::ListDir)?;
        let root = Arc::clone(&mount.root_dir);
        let vpath = path.clone();
        let mut entries = run_blocking(path.clone(), FilesystemOperation::ListDir, move || {
            let dir_fd = open_beneath(
                &root,
                &tail,
                OFlags::DIRECTORY | OFlags::RDONLY,
                Mode::empty(),
            )
            .map_err(|e| map_resolve_error(e, vpath.clone(), FilesystemOperation::ListDir))?;
            let mut dir = RustixDir::read_from(&dir_fd)
                .map_err(|e| errno_to_filesystem(e, vpath.clone(), FilesystemOperation::ListDir))?;
            let mut raw: Vec<(String, FileType)> = Vec::new();
            for entry in dir.by_ref() {
                let entry = entry.map_err(|e| {
                    errno_to_filesystem(e, vpath.clone(), FilesystemOperation::ListDir)
                })?;
                let name = entry.file_name().to_string_lossy().to_string();
                if name == "." || name == ".." {
                    continue;
                }
                raw.push((name, file_type_from_rustix(entry.file_type())));
            }
            Ok(raw)
        })
        .await?;

        // Build virtual paths on the async side; keep the blocking closure
        // focused on syscalls.
        entries.sort_by(|left, right| left.0.cmp(&right.0));
        let mut result = Vec::with_capacity(entries.len());
        for (name, file_type) in entries {
            let entry_path =
                VirtualPath::new(format!("{}/{}", path.as_str().trim_end_matches('/'), name))?;
            result.push(DirEntry {
                name,
                path: entry_path,
                file_type,
            });
        }
        Ok(result)
    }

    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        let (mount, tail) = self.resolve_mount_tail(path, FilesystemOperation::Stat)?;
        let root = Arc::clone(&mount.root_dir);
        // Classify sensitivity on the *host* path (canonical mount root + the
        // already-validated tail), not the virtual path. For a mount whose
        // virtual root differs from its host root, the virtual path can omit or
        // add path segments that change classification — e.g. host `…/.ssh`
        // mounted at virtual `/memory` would otherwise be reported non-sensitive.
        // This is pure string assembly from a root captured once during trusted
        // mount setup plus the validated tail components: NO filesystem
        // resolution happens here, so the fd-safe TOCTOU invariant is preserved
        // (we never canonicalize attacker-influenced input after the fstat).
        let sensitive = is_sensitive_host_path(&mount.host_root, &tail);
        let vpath = path.clone();
        run_blocking(path.clone(), FilesystemOperation::Stat, move || {
            // Open the resolved entry race-free, then fstat the fd. Files open
            // with O_RDONLY; directories may require O_DIRECTORY on some
            // platforms (EISDIR), so retry with it.
            let fd = match open_beneath(&root, &tail, OFlags::RDONLY, Mode::empty()) {
                Ok(fd) => fd,
                Err(ResolveError::Os(rustix::io::Errno::ISDIR)) => open_beneath(
                    &root,
                    &tail,
                    OFlags::DIRECTORY | OFlags::RDONLY,
                    Mode::empty(),
                )
                .map_err(|e| map_resolve_error(e, vpath.clone(), FilesystemOperation::Stat))?,
                Err(e) => {
                    return Err(map_resolve_error(
                        e,
                        vpath.clone(),
                        FilesystemOperation::Stat,
                    ));
                }
            };
            let stat = rustix::fs::fstat(&fd)
                .map_err(|e| errno_to_filesystem(e, vpath.clone(), FilesystemOperation::Stat))?;
            let file_type = file_type_from_raw_mode(stat.st_mode);
            let modified = system_time_from_stat(stat.st_mtime as i64, stat.st_mtime_nsec as i64);
            // `sensitive` is advisory metadata, not an access gate. It is
            // classified above on the host path (computed string-only from the
            // trusted canonical mount root + validated tail) rather than the
            // virtual path: see the `is_sensitive_host_path` call in `stat`.
            // Crucially this performs ZERO host-path filesystem resolution —
            // using the canonicalizing `is_sensitive_path` would reintroduce a
            // path lookup on attacker-influenced input *after* the fd-safe
            // `fstat`, recreating the TOCTOU window this backend eliminates.
            Ok(FileStat {
                path: vpath.clone(),
                file_type,
                len: stat.st_size as u64,
                modified,
                sensitive,
            })
        })
        .await
    }

    async fn delete(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        let (mount, tail) = self.resolve_mount_tail(path, FilesystemOperation::Delete)?;
        let root = Arc::clone(&mount.root_dir);
        let vpath = path.clone();
        run_blocking(path.clone(), FilesystemOperation::Delete, move || {
            let (parent_fd, leaf) = open_parent_dir(&root, &tail)
                .map_err(|e| map_resolve_error(e, vpath.clone(), FilesystemOperation::Delete))?;
            delete_at(&parent_fd, std::ffi::OsStr::new(&leaf), &vpath)
        })
        .await
    }

    async fn create_dir_all(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        let (mount, tail) = self.resolve_mount_tail(path, FilesystemOperation::CreateDirAll)?;
        let root = Arc::clone(&mount.root_dir);
        let vpath = path.clone();
        run_blocking(path.clone(), FilesystemOperation::CreateDirAll, move || {
            mkdir_all(&root, &tail, &vpath)
        })
        .await
    }
}

/// Run a synchronous, fd-relative filesystem closure on the blocking pool,
/// mapping a join failure to a `Backend` error. Preserves the async contract
/// for the otherwise-synchronous rustix `*at` syscalls.
async fn run_blocking<T, F>(
    path: VirtualPath,
    operation: FilesystemOperation,
    f: F,
) -> Result<T, FilesystemError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, FilesystemError> + Send + 'static,
{
    match tokio::task::spawn_blocking(f).await {
        Ok(result) => result,
        Err(join_error) => Err(FilesystemError::Backend {
            path,
            operation,
            reason: format!("blocking task failed: {join_error}"),
        }),
    }
}

/// Ensure the parent directory chain of `tail` exists, fd-relative and
/// race-free, so a subsequent leaf open finds its parent. Mirrors the old
/// `create_dir_all(parent)` behavior in `resolve_for_write`, but without ever
/// re-resolving an absolute path.
fn ensure_parent_dirs(
    root: &OwnedFd,
    tail: &ResolvedTail,
    path: &VirtualPath,
    operation: FilesystemOperation,
) -> Result<(), FilesystemError> {
    let comps = components(tail);
    if comps.len() <= 1 {
        // Leaf lives directly in the mount root; nothing to create.
        return Ok(());
    }
    let parent = tail_from_components(comps[..comps.len() - 1].to_vec());
    mkdir_all(root, &parent, path).map_err(|e| match e {
        // re-tag a not-found to the caller's op for accurate attribution.
        FilesystemError::NotFound { path, .. } => FilesystemError::NotFound { path, operation },
        other => other,
    })
}

/// Classify a failed descend `openat(O_DIRECTORY | O_NOFOLLOW)` on a component
/// that already exists. On macOS both a symlinked and a regular-file component
/// surface as `ENOTDIR` (`O_NOFOLLOW` refuses to follow the symlink), so errno
/// alone is ambiguous. We `fstatat(AT_SYMLINK_NOFOLLOW)` the component, relative
/// to the fd we already hold (no absolute-path re-resolution), to decide:
/// symlink → `SymlinkEscape`, otherwise → a non-escape "not a directory"
/// `Backend` error. Errnos other than `ENOTDIR`/`ELOOP` use the standard map.
fn classify_descend_errno(
    errno: rustix::io::Errno,
    parent_fd: std::os::fd::BorrowedFd<'_>,
    component: &str,
    path: &VirtualPath,
    operation: FilesystemOperation,
) -> FilesystemError {
    match errno {
        rustix::io::Errno::NOTDIR | rustix::io::Errno::LOOP => {
            match rustix::fs::statat(parent_fd, as_os_str(component), AtFlags::SYMLINK_NOFOLLOW) {
                Ok(stat) => {
                    if file_type_from_raw_mode(stat.st_mode) == FileType::Symlink {
                        FilesystemError::SymlinkEscape { path: path.clone() }
                    } else {
                        FilesystemError::Backend {
                            path: path.clone(),
                            operation,
                            reason: "not a directory".to_string(),
                        }
                    }
                }
                Err(other) => errno_to_filesystem(other, path.clone(), operation),
            }
        }
        other => errno_to_filesystem(other, path.clone(), operation),
    }
}

/// `mkdir -p` walk: create each component fd-relative with `mkdirat`, ignoring
/// `EEXIST` (idempotent), then descend via `openat(O_DIRECTORY|O_NOFOLLOW)`. A
/// symlinked component is rejected as `SymlinkEscape` (disambiguated from a
/// benign regular-file component via `fstatat`; see `classify_descend_errno`).
fn mkdir_all(
    root: &OwnedFd,
    tail: &ResolvedTail,
    path: &VirtualPath,
) -> Result<(), FilesystemError> {
    use std::os::fd::AsFd;

    let comps = components(tail);
    if comps.is_empty() {
        return Ok(());
    }
    let mut current: Option<OwnedFd> = None;
    for component in comps {
        let parent_fd = current.as_ref().map(AsFd::as_fd).unwrap_or(root.as_fd());
        match rustix::fs::mkdirat(
            parent_fd,
            as_os_str(component),
            Mode::from_raw_mode(DIR_CREATE_MODE),
        ) {
            Ok(()) => {}
            Err(rustix::io::Errno::EXIST) => {} // idempotent
            Err(errno) => {
                return Err(errno_to_filesystem(
                    errno,
                    path.clone(),
                    FilesystemOperation::CreateDirAll,
                ));
            }
        }
        // Descend into the (now-existing) component with O_NOFOLLOW so a
        // pre-existing symlink in place of a dir is rejected, not followed.
        let next = rustix::fs::openat(
            parent_fd,
            as_os_str(component),
            OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::RDONLY,
            Mode::empty(),
        )
        .map_err(|errno| {
            classify_descend_errno(
                errno,
                parent_fd,
                component,
                path,
                FilesystemOperation::CreateDirAll,
            )
        })?;
        current = Some(next);
    }
    Ok(())
}

/// Remove `name` within the directory referenced by `parent_fd`, race-free.
/// Uses `fstatat(AT_SYMLINK_NOFOLLOW)` to classify the entry without following a
/// symlink, then `unlinkat` (recursing fd-relative for directories).
///
/// `name` is an [`OsStr`] (raw bytes), not a `str`: on Unix directory entries
/// are arbitrary byte sequences that need not be valid UTF-8. Lossy conversion
/// (e.g. `to_string_lossy`) would substitute U+FFFD for invalid bytes and make
/// the `statat`/`unlinkat` target a *different*, non-existent name — silently
/// failing to remove the real entry. Carrying the raw `OsStr` end to end keeps
/// recursive delete byte-exact, matching the prior `remove_dir_all` behavior.
fn delete_at(
    parent_fd: &OwnedFd,
    name: &std::ffi::OsStr,
    path: &VirtualPath,
) -> Result<(), FilesystemError> {
    let stat = rustix::fs::statat(parent_fd, name, AtFlags::SYMLINK_NOFOLLOW)
        .map_err(|e| errno_to_filesystem(e, path.clone(), FilesystemOperation::Delete))?;
    if file_type_from_raw_mode(stat.st_mode) == FileType::Directory {
        // Open the child dir fd-relative (O_NOFOLLOW), recurse, then rmdir it.
        let child = rustix::fs::openat(
            parent_fd,
            name,
            OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::RDONLY,
            Mode::empty(),
        )
        .map_err(|e| errno_to_filesystem(e, path.clone(), FilesystemOperation::Delete))?;
        let mut dir = RustixDir::read_from(&child)
            .map_err(|e| errno_to_filesystem(e, path.clone(), FilesystemOperation::Delete))?;
        // Collect raw entry names as `OsString` (preserving non-UTF8 bytes) so
        // the recursive unlink targets the true on-disk name.
        let mut children: Vec<std::ffi::OsString> = Vec::new();
        for entry in dir.by_ref() {
            let entry = entry
                .map_err(|e| errno_to_filesystem(e, path.clone(), FilesystemOperation::Delete))?;
            let child_name = entry.file_name();
            if child_name == c"." || child_name == c".." {
                continue;
            }
            children.push(os_string_from_cstr(child_name));
        }
        drop(dir);
        for child_name in &children {
            delete_at(&child, child_name, path)?;
        }
        rustix::fs::unlinkat(parent_fd, name, AtFlags::REMOVEDIR)
            .map_err(|e| errno_to_filesystem(e, path.clone(), FilesystemOperation::Delete))?;
    } else {
        rustix::fs::unlinkat(parent_fd, name, AtFlags::empty())
            .map_err(|e| errno_to_filesystem(e, path.clone(), FilesystemOperation::Delete))?;
    }
    Ok(())
}

/// Convert a rustix dirent name (`&CStr`, raw bytes) into an `OsString` without
/// lossy UTF-8 substitution, preserving non-UTF8 byte sequences exactly.
fn os_string_from_cstr(name: &std::ffi::CStr) -> std::ffi::OsString {
    use std::os::unix::ffi::OsStrExt;
    std::ffi::OsStr::from_bytes(name.to_bytes()).to_os_string()
}

// ─── Helpers ──────────────────────────────────────────────────────────────

/// Classify the *host* path of a resolved entry as sensitive, string-only.
///
/// Joins the canonical mount root (captured once at trusted `mount_local` time)
/// with the already-validated tail components and runs the pure pattern matcher
/// [`is_sensitive_path_str`]. This deliberately does **not** call
/// [`ironclaw_safety::sensitive_paths::is_sensitive_path`], which canonicalizes
/// on disk — doing so would reopen the TOCTOU window the fd-relative backend
/// closes. Because the tail components are already escape-validated (no `..`,
/// no absolute, no `.`), the join cannot climb above the mount root.
fn is_sensitive_host_path(host_root: &std::path::Path, tail: &ResolvedTail) -> bool {
    let mut host_path = host_root.to_path_buf();
    for component in components(tail) {
        host_path.push(component);
    }
    is_sensitive_path_str(&host_path.to_string_lossy())
}

fn file_type_from_rustix(ft: RustixFileType) -> FileType {
    if ft == RustixFileType::RegularFile {
        FileType::File
    } else if ft == RustixFileType::Directory {
        FileType::Directory
    } else if ft == RustixFileType::Symlink {
        FileType::Symlink
    } else {
        FileType::Other
    }
}

fn file_type_from_raw_mode(mode: RawMode) -> FileType {
    file_type_from_rustix(RustixFileType::from_raw_mode(mode))
}

fn system_time_from_stat(secs: i64, nsecs: i64) -> Option<std::time::SystemTime> {
    let base = std::time::UNIX_EPOCH;
    if secs >= 0 {
        Some(base + std::time::Duration::new(secs as u64, nsecs.clamp(0, 999_999_999) as u32))
    } else {
        base.checked_sub(std::time::Duration::from_secs(secs.unsigned_abs()))
    }
}

/// Map a [`ResolveError`] to a [`FilesystemError`], preserving the
/// no-host-path-in-errors invariant (only the virtual path is carried).
fn map_resolve_error(
    error: ResolveError,
    path: VirtualPath,
    operation: FilesystemOperation,
) -> FilesystemError {
    match error {
        ResolveError::Escape => FilesystemError::SymlinkEscape { path },
        ResolveError::NotFound => FilesystemError::NotFound { path, operation },
        ResolveError::NotADirectory => FilesystemError::Backend {
            path,
            operation,
            reason: "not a directory".to_string(),
        },
        ResolveError::Os(errno) => errno_to_filesystem(errno, path, operation),
    }
}

/// Map a raw rustix `Errno` to a [`FilesystemError`].
fn errno_to_filesystem(
    errno: rustix::io::Errno,
    path: VirtualPath,
    operation: FilesystemOperation,
) -> FilesystemError {
    tracing::debug!(
        virtual_path = path.as_str(),
        %operation,
        errno = ?errno,
        "local filesystem backend errno"
    );
    match errno {
        // Only `ELOOP`/`EXDEV` indicate a genuine containment escape: every
        // `openat` in this backend uses `O_NOFOLLOW`, so a symlinked component is
        // reported as `ELOOP` and a cross-device hop as `EXDEV`.
        rustix::io::Errno::LOOP | rustix::io::Errno::XDEV => {
            FilesystemError::SymlinkEscape { path }
        }
        // `ENOTDIR` means a path component is a regular file where a directory
        // was expected (e.g. `/workspace/file/child` where `file` is a file).
        // With `O_NOFOLLOW` a symlink would surface as `ELOOP`, never `ENOTDIR`,
        // so this is a normal "not a directory" error — NOT a symlink escape.
        // Mapping it to `SymlinkEscape` would be a behavioral regression and
        // would pollute escape telemetry.
        rustix::io::Errno::NOTDIR => FilesystemError::Backend {
            path,
            operation,
            reason: "not a directory".to_string(),
        },
        rustix::io::Errno::NOENT => FilesystemError::NotFound { path, operation },
        other => FilesystemError::Backend {
            path,
            operation,
            reason: errno_reason(other),
        },
    }
}

fn io_error(
    path: VirtualPath,
    operation: FilesystemOperation,
    error: std::io::Error,
) -> FilesystemError {
    // A missing path is an expected condition, not a backend error — return it
    // without emitting a "backend error" debug line, so probing for absent
    // files does not flood the logs. Only genuine backend failures are logged.
    // (Ported from main's local.rs improvement during the openat2 refactor merge.)
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

fn errno_reason(errno: rustix::io::Errno) -> String {
    std::io::Error::from(errno).kind().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RootFilesystem;
    use ironclaw_host_api::HostPath;
    use tempfile::tempdir;

    // Ported from the pre-refactor `local.rs` during the openat2 merge: a
    // missing path must surface `NotFound` WITHOUT emitting a "local
    // filesystem backend error" debug line (probing absent files is expected,
    // not a backend failure).
    #[tokio::test]
    #[tracing_test::traced_test]
    async fn missing_local_paths_do_not_log_backend_error() {
        let storage = tempdir().unwrap();
        let mut root = LocalFilesystem::new();
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

    // A genuine (non-NotFound) backend error must still be classified as
    // `Backend` AND logged.
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
}
