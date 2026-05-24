//! Fd-relative, race-free path resolution for the local filesystem backend.
//!
//! Every operation resolves a virtual-path *tail* relative to an
//! [`OwnedFd`](std::os::fd::OwnedFd) opened once during trusted mount setup —
//! the mount root directory. The root fd is never re-derived from an absolute
//! path and no resolution step trusts `canonicalize`, so containment holds **by
//! construction**: a malicious symlink or a concurrent ancestor swap cannot
//! redirect an open outside the mount root.
//!
//! ## Portability
//!
//! - **Linux** (`target_os = "linux"`): a single `openat2(root_fd, tail, …,
//!   RESOLVE_BENEATH | RESOLVE_NO_MAGICLINKS)` syscall performs the whole walk
//!   atomically in the kernel. Any attempt to traverse out of the root (`..`,
//!   absolute path, escaping symlink, magic link) fails with `EXDEV`/`ELOOP`.
//! - **Other Unix** (macOS dev/CI): a manual per-component walk using
//!   `openat(parent_fd, comp, O_NOFOLLOW [| O_DIRECTORY for non-leaf])`. Each
//!   `openat` refuses to follow a symlink (`ELOOP`), and `..`/absolute
//!   components are rejected before the syscall. The window is closed the same
//!   way — every hop is relative to a fd we already hold, never an absolute
//!   re-resolution.
//!
//! Both share the invariant: open the root fd once, walk fd-relative, never
//! trust an absolute path again.
//!
//! ## Bind-mount / cross-device scope
//!
//! Neither `RESOLVE_BENEATH` (Linux) nor the portable per-component walk blocks
//! traversal *across a mount point* that lives **inside** the mount root. On
//! Linux this would require `RESOLVE_NO_XDEV`; on the portable walk it would
//! require an `st_dev` consistency check on every hop. We deliberately do not
//! enforce either, under the assumption that **a tenant mount root is a flat
//! storage directory that contains no nested mounts/bind-mounts**. Under that
//! assumption there is no cross-device object to traverse to, so the omission is
//! safe.
//!
//! If that assumption is ever violated — e.g. a tenant root could contain a
//! bind mount onto another tenant's storage — that nested mount *would* be a
//! real escape vector, because `RESOLVE_BENEATH` only constrains the logical
//! path (`..`/symlinks/absolute), not device boundaries. Adding
//! `RESOLVE_NO_XDEV` (Linux) plus an `st_dev` check (portable walk) is the
//! follow-up to close it; it is intentionally deferred to avoid breaking
//! legitimate same-root traversal when a tenant root happens to span devices
//! for unrelated reasons.

use std::ffi::OsStr;
use std::os::fd::OwnedFd;

use rustix::fs::{Mode, OFlags};

/// A resolved virtual-path tail, split into normalized components.
///
/// Construction validates that no component is empty, `.`, `..`, or absolute —
/// `resolve_joined` already produces clean segments, but we re-check here so
/// the walk on non-Linux platforms cannot be tricked into climbing out of the
/// root via a `..` segment, and so the contract is identical on every platform.
pub(super) struct ResolvedTail {
    components: Vec<String>,
}

impl ResolvedTail {
    /// Split a `/`-joined tail (already stripped of the virtual mount prefix)
    /// into validated components. An empty tail addresses the mount root.
    pub(super) fn parse(tail: &str) -> Result<Self, ResolveError> {
        let mut components = Vec::new();
        for segment in tail.split('/') {
            if segment.is_empty() || segment == "." {
                // `resolve_joined` trims and never emits empty/`.` segments for
                // a well-formed tail; tolerate them defensively as no-ops.
                continue;
            }
            if segment == ".." {
                return Err(ResolveError::Escape);
            }
            components.push(segment.to_string());
        }
        Ok(Self { components })
    }

    fn is_root(&self) -> bool {
        self.components.is_empty()
    }

    /// The final component (file/dir name), if the tail is non-empty.
    pub(super) fn leaf(&self) -> Option<&str> {
        self.components.last().map(String::as_str)
    }
}

/// Errors from fd-relative resolution, mapped to `FilesystemError` by the caller.
#[derive(Debug)]
pub(super) enum ResolveError {
    /// The tail attempted to traverse outside the mount root (`..`, absolute,
    /// escaping symlink, magic link, or cross-device). Maps to `SymlinkEscape`.
    Escape,
    /// A path component was not found. Maps to `NotFound`.
    NotFound,
    /// A path component that should have been a directory was a regular file (or
    /// other non-directory). This is a legitimate "not a directory" condition,
    /// NOT a symlink escape — e.g. `/workspace/file/child` where `file` is a
    /// regular file. Maps to a non-escape `Backend` error.
    NotADirectory,
    /// Any other OS error. Maps to `Backend`.
    Os(rustix::io::Errno),
}

impl From<rustix::io::Errno> for ResolveError {
    fn from(errno: rustix::io::Errno) -> Self {
        match errno {
            // `ELOOP`/`EXDEV`: openat2(RESOLVE_BENEATH) or an `O_NOFOLLOW` leaf
            // open refused to traverse a symlink / cross a device boundary. These
            // are the only errnos that genuinely indicate a containment escape.
            rustix::io::Errno::LOOP | rustix::io::Errno::XDEV => ResolveError::Escape,
            // `ENOTDIR`: a path component that we tried to descend through is not
            // a directory. On Linux `openat2` returns `ELOOP` (handled above) for
            // a symlinked component, so `ENOTDIR` here means a *regular file*
            // ancestor — a normal "not a directory" error, never an escape.
            //
            // On the non-Linux per-component walk an `O_NOFOLLOW` open of a
            // symlink also yields `ELOOP` (not `ENOTDIR`), so the same reasoning
            // holds; the walk additionally disambiguates explicitly via
            // `fstatat` (see `open_beneath_walk`) before this conversion is hit.
            rustix::io::Errno::NOTDIR => ResolveError::NotADirectory,
            rustix::io::Errno::NOENT => ResolveError::NotFound,
            other => ResolveError::Os(other),
        }
    }
}

/// Open the file/dir addressed by `tail`, relative to `root_fd`, race-free.
///
/// `oflags` are the access flags for the *final* component (e.g. `O_RDONLY`,
/// `O_DIRECTORY`). `O_NOFOLLOW` and `O_CLOEXEC` are always added so a symlink at
/// the leaf is rejected rather than followed. `mode` is only consulted when
/// `oflags` contains `O_CREAT`.
///
/// An empty tail returns a fresh fd onto the mount root itself.
pub(super) fn open_beneath(
    root_fd: &OwnedFd,
    tail: &ResolvedTail,
    oflags: OFlags,
    mode: Mode,
) -> Result<OwnedFd, ResolveError> {
    if tail.is_root() {
        // Re-open the root directory itself (dup-with-flags via openat ".").
        return Ok(rustix::fs::openat(
            root_fd,
            ".",
            OFlags::DIRECTORY | OFlags::CLOEXEC | (oflags & !OFlags::NOFOLLOW),
            Mode::empty(),
        )?);
    }

    #[cfg(target_os = "linux")]
    {
        open_beneath_linux(root_fd, tail, oflags, mode)
    }

    #[cfg(not(target_os = "linux"))]
    {
        open_beneath_walk(root_fd, tail, oflags, mode)
    }
}

/// Linux fast path: one `openat2` with `RESOLVE_BENEATH | RESOLVE_NO_MAGICLINKS`.
#[cfg(target_os = "linux")]
fn open_beneath_linux(
    root_fd: &OwnedFd,
    tail: &ResolvedTail,
    oflags: OFlags,
    mode: Mode,
) -> Result<OwnedFd, ResolveError> {
    use rustix::fs::ResolveFlags;

    let rel = tail.components.join("/");
    let resolve = ResolveFlags::BENEATH | ResolveFlags::NO_MAGICLINKS;
    rustix::fs::openat2(
        root_fd,
        rel.as_str(),
        oflags | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        mode,
        resolve,
    )
    .map_err(ResolveError::from)
}

/// Portable per-component walk for non-Linux Unix (macOS dev/CI).
///
/// Each intermediate component is opened with `O_DIRECTORY | O_NOFOLLOW`. The
/// leaf is opened with the caller's `oflags | O_NOFOLLOW`. Because every hop is
/// relative to a fd we already hold, there is no absolute-path re-resolution and
/// the walk is race-free by construction.
///
/// ## Errno disambiguation
///
/// On macOS an intermediate `openat(O_DIRECTORY | O_NOFOLLOW)` fails with
/// `ENOTDIR` for **both** a symlinked ancestor and a regular-file ancestor
/// (`O_NOFOLLOW` refuses to follow the symlink, so the kernel sees a non-dir at
/// that slot in either case). Errno alone therefore cannot tell a symlink escape
/// from a benign "not a directory" condition. When an intermediate open fails
/// with `ENOTDIR` (or `ELOOP`) we `fstatat(AT_SYMLINK_NOFOLLOW)` the offending
/// component — without following it — to classify it: a symlink is a containment
/// `Escape`, anything else is a normal `NotADirectory`. The `fstatat` is on the
/// same `parent_fd` we already hold, so it introduces no absolute-path
/// re-resolution and no new TOCTOU window.
#[cfg(not(target_os = "linux"))]
fn open_beneath_walk(
    root_fd: &OwnedFd,
    tail: &ResolvedTail,
    oflags: OFlags,
    mode: Mode,
) -> Result<OwnedFd, ResolveError> {
    use std::os::fd::AsFd;

    let mut parent: Option<OwnedFd> = None;
    let last = tail.components.len() - 1;
    for (index, component) in tail.components.iter().enumerate() {
        let parent_fd = parent.as_ref().map(AsFd::as_fd).unwrap_or(root_fd.as_fd());
        let next = if index == last {
            rustix::fs::openat(
                parent_fd,
                component.as_str(),
                oflags | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                mode,
            )
            .map_err(|errno| classify_walk_errno(errno, parent_fd, component))?
        } else {
            rustix::fs::openat(
                parent_fd,
                component.as_str(),
                OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|errno| classify_walk_errno(errno, parent_fd, component))?
        };
        parent = Some(next);
    }
    // `parent` is always `Some` here: a non-root tail has >= 1 component.
    parent.ok_or(ResolveError::Escape)
}

/// Classify an `errno` from a failed `openat(… O_NOFOLLOW)` on the portable
/// walk (intermediate or leaf component). `ENOTDIR`/`ELOOP` are ambiguous on
/// macOS — a symlinked component opened `O_NOFOLLOW` surfaces as `ENOTDIR` (with
/// `O_DIRECTORY`) or `ELOOP`, and a regular-file component opened `O_DIRECTORY`
/// also surfaces as `ENOTDIR` — so we `fstatat(AT_SYMLINK_NOFOLLOW)` the
/// component, relative to the fd we already hold (never an absolute path), to
/// decide: symlink → `Escape`, otherwise → `NotADirectory`. Any other errno
/// passes through the standard mapping.
#[cfg(not(target_os = "linux"))]
fn classify_walk_errno(
    errno: rustix::io::Errno,
    parent_fd: std::os::fd::BorrowedFd<'_>,
    component: &str,
) -> ResolveError {
    use rustix::fs::{AtFlags, FileType};

    match errno {
        rustix::io::Errno::NOTDIR | rustix::io::Errno::LOOP => {
            match rustix::fs::statat(parent_fd, component, AtFlags::SYMLINK_NOFOLLOW) {
                Ok(stat) => {
                    if FileType::from_raw_mode(stat.st_mode) == FileType::Symlink {
                        ResolveError::Escape
                    } else {
                        ResolveError::NotADirectory
                    }
                }
                // The component vanished between the open and the stat: treat as
                // not-found rather than guessing.
                Err(rustix::io::Errno::NOENT) => ResolveError::NotFound,
                Err(other) => ResolveError::from(other),
            }
        }
        other => ResolveError::from(other),
    }
}

/// Open the parent directory of `tail` race-free, returning the dir fd and the
/// leaf name. Used by write/append/delete/create which operate on the parent.
///
/// Errors if `tail` is the mount root (no parent within the mount).
pub(super) fn open_parent_dir(
    root_fd: &OwnedFd,
    tail: &ResolvedTail,
) -> Result<(OwnedFd, String), ResolveError> {
    let leaf = tail.leaf().ok_or(ResolveError::Escape)?.to_string();
    let parent_components = &tail.components[..tail.components.len() - 1];
    let parent_tail = ResolvedTail {
        components: parent_components.to_vec(),
    };
    let dir = open_beneath(
        root_fd,
        &parent_tail,
        OFlags::DIRECTORY | OFlags::RDONLY,
        Mode::empty(),
    )?;
    Ok((dir, leaf))
}

/// Borrow a name as an `OsStr` for rustix `*at` calls without allocating.
pub(super) fn as_os_str(name: &str) -> &OsStr {
    OsStr::new(name)
}

/// Components of the tail, for callers that need to walk (`create_dir_all`).
pub(super) fn components(tail: &ResolvedTail) -> &[String] {
    &tail.components
}

/// Build a `ResolvedTail` directly from owned components (for sub-walks).
pub(super) fn tail_from_components(components: Vec<String>) -> ResolvedTail {
    ResolvedTail { components }
}

/// Re-export for convenience so the ops module can construct fd-relative opens.
pub(super) use rustix::fs::{Dir as RustixDir, FileType as RustixFileType};

#[allow(unused_imports)]
pub(super) use rustix::fs::AtFlags;
