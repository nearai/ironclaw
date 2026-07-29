mod fd_resolve;
mod mount_registry;

use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::OsStrExt;

use async_trait::async_trait;
use ironclaw_host_api::VirtualPath;
use ironclaw_safety::sensitive_paths::is_sensitive_path_str;
use rustix::fd::{AsFd, OwnedFd};
use rustix::fs::{AtFlags, Mode, OFlags};

use self::fd_resolve::{
    ResolveContext, ResolveError, SymlinkBudget, atomic_write_file, descend_creating,
    map_file_type, new_file_mode, open_one, read_all, remove_dir_all_fd,
    resolve_error_to_filesystem_error, resolve_walk, resolve_write_leaf, write_all,
};
use self::mount_registry::{LocalMount, MountTarget};
use crate::{
    CasExpectation, DirEntry, Entry, FileStat, FilesystemError, FilesystemOperation, RecordVersion,
    RootFilesystem, VersionedEntry,
};

/// The on-disk `RootFilesystem` backend, mounted into the virtual namespace.
///
/// The name states the **storage medium** — disk, a peer of `InMemoryBackend`,
/// `LibSqlRootFilesystem`, and `PostgresRootFilesystem` — not a deployment mode.
/// Renamed from `LocalFilesystem` because `Local` read like a deployment tier
/// while this is simply the disk backend a `DeploymentConfig` may select
/// (arch-simplification §4.4 Bucket 2).
///
/// Mount registration and routing (`mount_local`, `mount_local_per_leaf`,
/// `ensure_scoped_mount_dynamic`, `resolve_mount_target`, `LocalMount`,
/// `MountTarget`) live in [`mount_registry`](self::mount_registry) — a second
/// `impl DiskFilesystem` block in its own file. This impl block stays here:
/// the `RootFilesystem` trait impl (below) is Rust's one-impl-block-per-trait
/// rule, so it cannot itself be split across files.
#[derive(Debug, Default)]
pub struct DiskFilesystem {
    mounts: std::sync::RwLock<Vec<LocalMount>>,
    /// Every virtual root ever passed to
    /// [`ensure_scoped_mount_dynamic`](DiskFilesystem::ensure_scoped_mount_dynamic),
    /// retained permanently (never evicted, unlike `mounts`). Backs the
    /// PR #6817 cross-tenant fix in `mount_registry`'s `resolve_mount_target`:
    /// once a virtual root has ever been narrowed, every future resolution
    /// under it must find a live matching dynamic mount or fail closed,
    /// rather than silently matching a wider ancestor mount whose
    /// containment boundary the narrowing was meant to replace. Only
    /// strings, not fds, so it does not reopen the `RLIMIT_NOFILE` leak
    /// `MAX_DYNAMIC_MOUNTS` exists to bound.
    narrow_scoped_roots: std::sync::RwLock<std::collections::HashSet<String>>,
}

impl DiskFilesystem {
    pub fn new() -> Self {
        Self::default()
    }
}

fn dup_owned_fd(fd: rustix::fd::BorrowedFd<'_>) -> Result<OwnedFd, ResolveError> {
    rustix::io::dup(fd).map_err(|errno| ResolveError::Io(errno.into()))
}

/// For a `leaf_scoped` mount, opens the caller's own leaf directory
/// (`target.components[0]`) as a fresh anchor fd and returns it alongside
/// the remaining tail components: every subsequent walk resolves
/// `RESOLVE_BENEATH` *this* anchor, not the wide, shared mount root, so an
/// in-bounds symlink can never step from one caller's leaf into a sibling
/// leaf. Non-leaf-scoped mounts pass the mount root straight through,
/// unchanged.
///
/// `create_if_missing` mirrors [`descend_creating`]'s bootstrap semantics —
/// a brand-new leaf's directory does not exist yet on its first write, so
/// write paths must create it as they anchor; read paths must not (a read
/// against a leaf that has never been written must still report
/// `NotFound`/`MountNotFound`, not silently fabricate the directory).
fn anchor_for_target(
    target: &MountTarget,
    create_if_missing: bool,
) -> Result<(OwnedFd, Vec<OsString>), ResolveError> {
    if !target.leaf_scoped {
        return Ok((
            dup_owned_fd(target.root_fd.as_fd())?,
            target.components.clone(),
        ));
    }
    let Some((leaf, rest)) = target.components.split_first() else {
        // `resolve_mount_target` already fails a leaf-scoped mount closed on
        // an empty tail (`PathOutsideMount`) before a `MountTarget` is ever
        // built, and `VirtualPath::new` (the type's sole constructor, used
        // even by `Deserialize`) strips every `.` segment before any path
        // reaches this crate, so a `.`-only tail like `/tmp/.` can never
        // arrive here either — confirmed by
        // `leaf_scoped_mount_rejects_dot_only_bare_mount_root_request`.
        // Fail closed anyway rather than widening to the shared root: a
        // leaf-scoped mount has no safe anchor without a leaf, and this arm
        // should never depend on an invariant enforced by a different
        // crate to stay safe.
        return Err(ResolveError::Escape);
    };
    let leaf_components = std::slice::from_ref(leaf);
    let anchor = if create_if_missing {
        descend_creating(target.root_fd.as_fd(), leaf_components)?.0
    } else {
        // PR #6817 follow-up (macOS `RESOLVE_NO_XDEV` parity): the shared
        // mount root's own device, captured once here — the same
        // once-per-resolution shape `resolve_walk`/`descend_creating` use
        // internally for their own anchor.
        let anchor_dev = rustix::fs::fstat(target.root_fd.as_fd())
            .map_err(|errno| ResolveError::Io(errno.into()))?
            .st_dev;
        let budget = SymlinkBudget::new();
        let ctx = ResolveContext {
            budget: &budget,
            anchor_dev,
        };
        open_one(
            target.root_fd.as_fd(),
            &[],
            target.root_fd.as_fd(),
            leaf,
            OFlags::DIRECTORY,
            Mode::empty(),
            &ctx,
        )?
    };
    Ok((anchor, rest.to_vec()))
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

    async fn read_file(&self, path: &VirtualPath) -> Result<Vec<u8>, FilesystemError> {
        let target = self.resolve_mount_target(path)?;
        let path = path.clone();
        run_blocking(path.clone(), FilesystemOperation::ReadFile, move || {
            let (anchor, rest) = anchor_for_target(&target, false).map_err(|error| {
                resolve_error_to_filesystem_error(&path, FilesystemOperation::ReadFile, error)
            })?;
            let (fd, _parent) =
                resolve_walk(anchor.as_fd(), &rest, OFlags::RDONLY).map_err(|error| {
                    resolve_error_to_filesystem_error(&path, FilesystemOperation::ReadFile, error)
                })?;
            let stat = rustix::fs::fstat(&fd).map_err(|errno| {
                io_error(path.clone(), FilesystemOperation::ReadFile, errno.into())
            })?;
            if rustix::fs::FileType::from_raw_mode(stat.st_mode)
                != rustix::fs::FileType::RegularFile
            {
                return Err(FilesystemError::Backend {
                    path: path.clone(),
                    operation: FilesystemOperation::ReadFile,
                    reason: "not a file".to_string(),
                });
            }
            read_all(fd)
                .map_err(|error| io_error(path.clone(), FilesystemOperation::ReadFile, error))
        })
        .await
    }

    async fn read_file_bounded(
        &self,
        path: &VirtualPath,
        max_bytes: usize,
    ) -> Result<Option<Vec<u8>>, FilesystemError> {
        let target = self.resolve_mount_target(path)?;
        let path = path.clone();
        run_blocking(path.clone(), FilesystemOperation::ReadFile, move || {
            let (anchor, rest) = anchor_for_target(&target, false).map_err(|error| {
                resolve_error_to_filesystem_error(&path, FilesystemOperation::ReadFile, error)
            })?;
            let (fd, _parent) =
                resolve_walk(anchor.as_fd(), &rest, OFlags::RDONLY).map_err(|error| {
                    resolve_error_to_filesystem_error(&path, FilesystemOperation::ReadFile, error)
                })?;
            let stat = rustix::fs::fstat(&fd).map_err(|errno| {
                io_error(path.clone(), FilesystemOperation::ReadFile, errno.into())
            })?;
            if rustix::fs::FileType::from_raw_mode(stat.st_mode)
                != rustix::fs::FileType::RegularFile
            {
                return Err(FilesystemError::Backend {
                    path: path.clone(),
                    operation: FilesystemOperation::ReadFile,
                    reason: "not a file".to_string(),
                });
            }
            if stat.st_size < 0 || stat.st_size as u64 > max_bytes as u64 {
                return Ok(None);
            }
            let bytes = read_all(fd)
                .map_err(|error| io_error(path.clone(), FilesystemOperation::ReadFile, error))?;
            if bytes.len() > max_bytes {
                return Ok(None);
            }
            Ok(Some(bytes))
        })
        .await
    }

    async fn write_file(&self, path: &VirtualPath, bytes: &[u8]) -> Result<(), FilesystemError> {
        self.write_file_with_cas(path, bytes, CasExpectation::Any)
            .await
    }

    async fn append_file(&self, path: &VirtualPath, bytes: &[u8]) -> Result<(), FilesystemError> {
        let target = self.resolve_mount_target(path)?;
        let bytes = bytes.to_vec();
        let path = path.clone();
        run_blocking(path.clone(), FilesystemOperation::AppendFile, move || {
            let (anchor, rest) = anchor_for_target(&target, true).map_err(|error| {
                resolve_error_to_filesystem_error(&path, FilesystemOperation::AppendFile, error)
            })?;
            let (parent_components, leaf) = split_leaf(&rest, &path)?;
            let (parent_fd, parent_ancestors) =
                descend_creating(anchor.as_fd(), &parent_components).map_err(|error| {
                    resolve_error_to_filesystem_error(&path, FilesystemOperation::AppendFile, error)
                })?;
            // `open_one` already follows an in-bounds symlink at `leaf`
            // transparently (unlike `write_file`'s rename-based atomic
            // install, a plain `open`-and-append writes straight through
            // one), so no separate `resolve_write_leaf` chase is needed here.
            // `parent_ancestors` is `parent_fd`'s own ancestor stack
            // (PR #6817 review follow-up — `descend_creating` now returns
            // it): a `..` in a symlink discovered at `leaf` resolves past
            // `parent_fd`'s own parent instead of failing closed.
            // PR #6817 follow-up (macOS `RESOLVE_NO_XDEV` parity): `anchor`'s
            // own device, captured once here — mirroring `resolve_walk`/
            // `descend_creating`'s internal once-per-resolution capture,
            // since this leaf open is itself a one-component resolution
            // anchored at `anchor` (via `parent_fd`).
            let anchor_dev = rustix::fs::fstat(anchor.as_fd())
                .map_err(|errno| ResolveError::Io(errno.into()))
                .map_err(|error| {
                    resolve_error_to_filesystem_error(&path, FilesystemOperation::AppendFile, error)
                })?
                .st_dev;
            let budget = SymlinkBudget::new();
            let ctx = ResolveContext {
                budget: &budget,
                anchor_dev,
            };
            let fd = open_one(
                anchor.as_fd(),
                &parent_ancestors,
                parent_fd.as_fd(),
                &leaf,
                OFlags::WRONLY | OFlags::APPEND | OFlags::CREATE,
                new_file_mode(),
                &ctx,
            )
            .map_err(|error| {
                resolve_error_to_filesystem_error(&path, FilesystemOperation::AppendFile, error)
            })?;
            write_all(fd, &bytes)
                .map_err(|error| io_error(path.clone(), FilesystemOperation::AppendFile, error))
        })
        .await
    }

    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        self.list_dir_bounded(path, usize::MAX).await
    }

    async fn list_dir_bounded(
        &self,
        path: &VirtualPath,
        max_entries: usize,
    ) -> Result<Vec<DirEntry>, FilesystemError> {
        let target = self.resolve_mount_target(path)?;
        let path = path.clone();
        run_blocking(path.clone(), FilesystemOperation::ListDir, move || {
            let (anchor, rest) = anchor_for_target(&target, false).map_err(|error| {
                resolve_error_to_filesystem_error(&path, FilesystemOperation::ListDir, error)
            })?;
            let (fd, _parent) =
                resolve_walk(anchor.as_fd(), &rest, OFlags::RDONLY).map_err(|error| {
                    resolve_error_to_filesystem_error(&path, FilesystemOperation::ListDir, error)
                })?;
            let mut listing = rustix::fs::Dir::read_from(fd.as_fd()).map_err(|errno| {
                io_error(path.clone(), FilesystemOperation::ListDir, errno.into())
            })?;
            let mut entries = Vec::new();
            while entries.len() < max_entries {
                let Some(raw_entry) = listing.next() else {
                    break;
                };
                let raw_entry = raw_entry.map_err(|errno| {
                    io_error(path.clone(), FilesystemOperation::ListDir, errno.into())
                })?;
                let name_bytes = raw_entry.file_name().to_bytes();
                if name_bytes == b"." || name_bytes == b".." {
                    continue;
                }
                let name = OsStr::from_bytes(name_bytes);
                let name_str = name.to_string_lossy().to_string();
                let entry_path = VirtualPath::new(format!(
                    "{}/{}",
                    path.as_str().trim_end_matches('/'),
                    name_str
                ))?;
                // `AT_SYMLINK_NOFOLLOW`: report a symlink child as a symlink,
                // never resolve through it to describe whatever it points
                // at (which may be outside the mount entirely).
                let stat = rustix::fs::statat(fd.as_fd(), name, AtFlags::SYMLINK_NOFOLLOW)
                    .map_err(|errno| {
                        io_error(entry_path.clone(), FilesystemOperation::Stat, errno.into())
                    })?;
                entries.push(DirEntry {
                    name: name_str,
                    path: entry_path,
                    file_type: map_file_type(rustix::fs::FileType::from_raw_mode(stat.st_mode)),
                });
            }
            entries.sort_by(|left, right| left.name.cmp(&right.name));
            Ok(entries)
        })
        .await
    }

    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        let target = self.resolve_mount_target(path)?;
        let path = path.clone();
        run_blocking(path.clone(), FilesystemOperation::Stat, move || {
            let (anchor, rest) = anchor_for_target(&target, false).map_err(|error| {
                resolve_error_to_filesystem_error(&path, FilesystemOperation::Stat, error)
            })?;
            let (fd, _parent) =
                resolve_walk(anchor.as_fd(), &rest, OFlags::RDONLY).map_err(|error| {
                    resolve_error_to_filesystem_error(&path, FilesystemOperation::Stat, error)
                })?;
            let stat = rustix::fs::fstat(&fd)
                .map_err(|errno| io_error(path.clone(), FilesystemOperation::Stat, errno.into()))?;
            let len = if stat.st_size < 0 {
                0
            } else {
                stat.st_size as u64
            };
            Ok(FileStat {
                path: path.clone(),
                file_type: map_file_type(rustix::fs::FileType::from_raw_mode(stat.st_mode)),
                len,
                modified: stat_modified(stat.st_mtime, stat.st_mtime_nsec),
                // No host path to check anymore (by design — see the module
                // doc): the string-only, filesystem-access-free
                // `is_sensitive_path_str` checks the same filename patterns
                // (`.env`, `.pem`, …) against the virtual path's leaf
                // component, which is identical to the host path's leaf
                // component for every mount (mounting only ever renames the
                // path *prefix*).
                sensitive: is_sensitive_path_str(path.as_str()),
            })
        })
        .await
    }

    async fn delete(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        let target = self.resolve_mount_target(path)?;
        if target.components.is_empty() {
            // Removing an entire mount's root by virtual-path traversal was
            // never an intentional capability (the old path-based
            // implementation happened to allow it as a side effect of
            // `resolve_existing` resolving the bare mount root). The
            // fd-rooted resolver has no parent fd for the mount root itself
            // — by design, it never holds an fd outside `root_fd` — so this
            // fails closed instead.
            return Err(FilesystemError::PathOutsideMount { path: path.clone() });
        }
        let path = path.clone();
        run_blocking(path.clone(), FilesystemOperation::Delete, move || {
            let (anchor, rest) = anchor_for_target(&target, false).map_err(|error| {
                resolve_error_to_filesystem_error(&path, FilesystemOperation::Delete, error)
            })?;
            if rest.is_empty() {
                // Same "cannot delete the resolution root" policy as the
                // bare-mount-root check above, restated post-anchor: for a
                // leaf-scoped mount, `rest` empty means the request named
                // the caller's own leaf directory itself (anchoring already
                // consumed it as `target.components[0]`), which has no
                // fd-relative parent inside this resolution either.
                return Err(FilesystemError::PathOutsideMount { path: path.clone() });
            }
            let (fd, parent) =
                resolve_walk(anchor.as_fd(), &rest, OFlags::RDONLY).map_err(|error| {
                    resolve_error_to_filesystem_error(&path, FilesystemOperation::Delete, error)
                })?;
            let Some((parent_fd, name)) = parent else {
                return Err(FilesystemError::PathOutsideMount { path: path.clone() });
            };
            drop(fd);
            // Determine the *entry's own* type via an `AT_SYMLINK_NOFOLLOW`
            // stat against `parent_fd`/`name` — not `fstat` of `fd` (which,
            // now that `open_one` follows in-bounds symlinks, may have
            // opened straight through a symlink to a directory target).
            // `std::fs::remove_dir_all`/`remove_file` never follow a
            // symlink at the entry being removed — a symlink is always
            // unlinked as itself, never traversed into — and this module
            // promises the same contract; using `fd`'s (possibly-followed)
            // type here would recurse into and delete the *target*
            // directory's contents before failing on the final
            // `unlinkat(..., REMOVEDIR)` (which POSIX refuses on a symlink).
            let entry_stat =
                rustix::fs::statat(parent_fd.as_fd(), &name, AtFlags::SYMLINK_NOFOLLOW).map_err(
                    |errno| io_error(path.clone(), FilesystemOperation::Delete, errno.into()),
                )?;
            if rustix::fs::FileType::from_raw_mode(entry_stat.st_mode)
                == rustix::fs::FileType::Directory
            {
                remove_dir_all_fd(anchor.as_fd(), parent_fd.as_fd(), &name)
                    .map_err(|error| io_error(path.clone(), FilesystemOperation::Delete, error))
            } else {
                rustix::fs::unlinkat(parent_fd.as_fd(), &name, AtFlags::empty()).map_err(|errno| {
                    io_error(path.clone(), FilesystemOperation::Delete, errno.into())
                })
            }
        })
        .await
    }

    async fn create_dir_all(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        let target = self.resolve_mount_target(path)?;
        let path = path.clone();
        run_blocking(path.clone(), FilesystemOperation::CreateDirAll, move || {
            let (anchor, rest) = anchor_for_target(&target, true).map_err(|error| {
                resolve_error_to_filesystem_error(&path, FilesystemOperation::CreateDirAll, error)
            })?;
            descend_creating(anchor.as_fd(), &rest)
                .map(|(_fd, _ancestors)| ())
                .map_err(|error| {
                    resolve_error_to_filesystem_error(
                        &path,
                        FilesystemOperation::CreateDirAll,
                        error,
                    )
                })
        })
        .await
    }

    /// One-line delegation to [`DiskFilesystem::ensure_scoped_mount_dynamic`]
    /// (defined in [`mount_registry`](self::mount_registry), a second
    /// `impl DiskFilesystem` block in its own file). The actual logic lives
    /// there because Rust allows an inherent `impl` to be split across
    /// multiple blocks/files but not a single trait impl — this
    /// `impl RootFilesystem for DiskFilesystem` block is one block, so every
    /// one of its methods must be written here, even when (as here) the body
    /// is a single call out to mount-registry logic that lives elsewhere.
    async fn ensure_scoped_mount(&self, virtual_root: &VirtualPath) -> Result<(), FilesystemError> {
        self.ensure_scoped_mount_dynamic(virtual_root).await
    }
}

impl DiskFilesystem {
    async fn write_file_with_cas(
        &self,
        path: &VirtualPath,
        bytes: &[u8],
        cas: CasExpectation,
    ) -> Result<(), FilesystemError> {
        let target = self.resolve_mount_target(path)?;
        let bytes = bytes.to_vec();
        let path = path.clone();
        run_blocking(path.clone(), FilesystemOperation::WriteFile, move || {
            let (anchor, rest) = anchor_for_target(&target, true).map_err(|error| {
                resolve_error_to_filesystem_error(&path, FilesystemOperation::WriteFile, error)
            })?;
            let (parent_components, leaf) = split_leaf(&rest, &path)?;
            let (parent_fd, parent_ancestors) =
                descend_creating(anchor.as_fd(), &parent_components).map_err(|error| {
                    resolve_error_to_filesystem_error(&path, FilesystemOperation::WriteFile, error)
                })?;
            // `rename`/`link` (how `atomic_write_file` installs bytes) never
            // follow a symlink at the destination name — resolve any
            // in-bounds symlink chain at `leaf` ourselves first so the
            // install lands at the symlink's ultimate target, not over the
            // symlink entry itself. `parent_ancestors` is `parent_fd`'s own
            // ancestor stack (PR #6817 review follow-up — `descend_creating`
            // now returns it): a `..` in a symlink discovered at `leaf`
            // resolves past `parent_fd`'s own parent instead of failing
            // closed.
            let (write_parent_fd, write_leaf) =
                resolve_write_leaf(anchor.as_fd(), &parent_ancestors, parent_fd.as_fd(), &leaf)
                    .map_err(|error| {
                        resolve_error_to_filesystem_error(
                            &path,
                            FilesystemOperation::WriteFile,
                            error,
                        )
                    })?;
            atomic_write_file(&path, write_parent_fd.as_fd(), &write_leaf, &bytes, cas)
        })
        .await
    }
}

/// Splits `components` into its parent directory components and its final
/// (leaf) component, for operations that create or open a specific file —
/// distinct from [`resolve_walk`], which is used by read-only operations
/// that may legitimately target the bare mount root.
fn split_leaf(
    components: &[OsString],
    path: &VirtualPath,
) -> Result<(Vec<OsString>, OsString), FilesystemError> {
    match components.split_last() {
        Some((leaf, parent)) => Ok((parent.to_vec(), leaf.clone())),
        None => Err(FilesystemError::PathOutsideMount { path: path.clone() }),
    }
}

/// Runs a synchronous, fd-rooted resolve-and-act closure on the blocking
/// pool, and flattens the `JoinError` case into a `FilesystemError`.
///
/// This is the structural fix's shape: everything inside `body` — walking
/// component-by-component from the mount's open root fd, and then acting on
/// the fd that walk produced — runs without ever crossing back through the
/// async scheduler. There is no `.await` between "resolve" and "act" for a
/// TOCTOU race to land in, because there is no longer a separate "resolve"
/// step that hands back a path string for a later, independent syscall to
/// re-resolve. The resolved fd itself, not a path, is what every subsequent
/// operation in `body` touches.
async fn run_blocking<T, F>(
    path: VirtualPath,
    operation: FilesystemOperation,
    body: F,
) -> Result<T, FilesystemError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, FilesystemError> + Send + 'static,
{
    match tokio::task::spawn_blocking(body).await {
        Ok(result) => result,
        Err(join_error) => Err(FilesystemError::Backend {
            path,
            operation,
            reason: format!("local filesystem blocking task panicked: {join_error}"),
        }),
    }
}

/// `pub(super)`: also called from the [`fd_resolve`] submodule, which has no
/// dependency on `DiskFilesystem` itself but does need this shared
/// `io::Error -> FilesystemError` mapping (the majority of call sites are
/// still here, in the `RootFilesystem` impl above).
pub(super) fn io_error(
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

fn stat_modified(secs: i64, nanos: impl TryInto<u32>) -> Option<std::time::SystemTime> {
    let nanos = nanos.try_into().unwrap_or(0);
    if secs >= 0 {
        std::time::SystemTime::UNIX_EPOCH.checked_add(std::time::Duration::new(secs as u64, nanos))
    } else {
        std::time::SystemTime::UNIX_EPOCH.checked_sub(std::time::Duration::new((-secs) as u64, 0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_host_api::HostPath;
    use tempfile::tempdir;

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

    fn make_fifo(path: &std::path::Path) {
        // Deliberately *not* `rustix::fs::mkfifoat` (review suggestion):
        // that function is `#[cfg(not(apple, ...))]` in the pinned rustix
        // `1.1.4` — unavailable on macOS, a platform this module's own
        // fallback path explicitly supports (see the module doc). Shelling
        // out to the portable `mkfifo(1)` binary (present on every
        // Unix-like CI/dev image this crate targets) is the one that
        // actually works everywhere `cargo test` runs for this crate.
        let status = std::process::Command::new("mkfifo")
            .arg(path)
            .status()
            .expect("mkfifo command must be available on this platform");
        assert!(status.success(), "mkfifo failed for {path:?}");
    }

    /// PR #6817 review follow-up: `resolve_walk`'s leaf open used to be a
    /// plain blocking `OFlags::RDONLY` open with no `O_NONBLOCK`. A FIFO with
    /// no writer, planted under any writable mount, blocks the tokio
    /// blocking-pool thread that opens it *indefinitely* — repeated, that
    /// exhausts the pool and wedges the whole process. This pins that
    /// `read_file` must not block on a no-writer FIFO: it must either error
    /// promptly (the expected outcome — the FIFO fails the "must be a
    /// regular file" check right after open) within a small, generous
    /// timeout, never hang.
    #[tokio::test(flavor = "multi_thread")]
    async fn read_file_on_fifo_with_no_writer_does_not_block_forever() {
        let storage = tempdir().unwrap();
        let fifo_path = storage.path().join("blocking.fifo");
        make_fifo(&fifo_path);

        let mut root = DiskFilesystem::new();
        root.mount_local(
            VirtualPath::new("/projects").unwrap(),
            HostPath::from_path_buf(storage.path().to_path_buf()),
        )
        .unwrap();

        let outcome = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            root.read_file(&VirtualPath::new("/projects/blocking.fifo").unwrap()),
        )
        .await;

        assert!(
            outcome.is_ok(),
            "read_file on a FIFO with no writer must not block the blocking-pool \
             thread indefinitely (timed out — this is the FIFO-DoS regression)"
        );
        // The FIFO is rejected as "not a file" once opened non-blocking — the
        // point being pinned here is that we *get an answer promptly*, not
        // which particular error variant it is.
        assert!(outcome.unwrap().is_err());
    }

    /// Same DoS shape as `read_file_on_fifo_with_no_writer_does_not_block_forever`,
    /// but for the write path: `append_file`'s leaf `open_one` call
    /// (`O_WRONLY | O_APPEND | O_CREATE`) is just as exposed — opening a FIFO
    /// for write-only with no reader present blocks under the same missing
    /// `O_NONBLOCK`. A fix that only covers the read side is not a fix.
    #[tokio::test(flavor = "multi_thread")]
    async fn append_file_on_fifo_with_no_reader_does_not_block_forever() {
        let storage = tempdir().unwrap();
        let fifo_path = storage.path().join("blocking-write.fifo");
        make_fifo(&fifo_path);

        let mut root = DiskFilesystem::new();
        root.mount_local(
            VirtualPath::new("/projects").unwrap(),
            HostPath::from_path_buf(storage.path().to_path_buf()),
        )
        .unwrap();

        let outcome = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            root.append_file(
                &VirtualPath::new("/projects/blocking-write.fifo").unwrap(),
                b"payload",
            ),
        )
        .await;

        assert!(
            outcome.is_ok(),
            "append_file on a FIFO with no reader must not block the blocking-pool \
             thread indefinitely (timed out — this is the FIFO-DoS regression)"
        );
        assert!(outcome.unwrap().is_err());
    }
}
