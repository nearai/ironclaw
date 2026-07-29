//! fd-rooted traversal primitives for the local disk backend.
//!
//! Every function below operates purely on file descriptors and path
//! *components* (never a joined host path string). `open_one` is the one
//! place a directory-entry lookup happens. It now **follows** a symlink that
//! stays inside the resolution root, and refuses (fails closed) one that
//! would step outside it — on Linux via a kernel-enforced
//! `openat2(RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS)` call for the (common,
//! no-symlink) fast case, falling back to this module's own bounded,
//! fd-anchored walk the instant a symlink is actually encountered — on other
//! platforms always via that same walk. Because resolution never hands back
//! a path for a later, independent syscall to re-open, there is no window
//! between "checked" and "acted on" for an attacker to swap the entry in.
//!
//! **Invariant every step in this module preserves:** no step ever resolves
//! a path *string* against anything other than an fd this module already
//! holds and has itself verified (the mount's root fd, or an fd this module
//! opened moments earlier in the same walk). A symlink target is text an
//! attacker controls; it is only ever handed to `openat`-family calls
//! anchored on such a verified fd — component by component — never to a
//! path-based syscall (`open`, `std::fs::*`, `canonicalize`) that would let
//! the kernel re-resolve it from scratch against the real host root.
//!
//! This module is deliberately self-contained: nothing here depends on
//! `DiskFilesystem`/`LocalMount`/the `RootFilesystem` impl in the parent
//! `local` module — every function operates on `BorrowedFd`/`OwnedFd` and
//! `OsStr`/`OsString` only, plus the crate's error/path vocabulary
//! (`FilesystemError`, `FilesystemOperation`, `VirtualPath`). That is also
//! why this module has no reason to ever import `tokio::fs` (or any other
//! path-based filesystem API): its whole job is to be the fd-relative
//! alternative to one.
//!
//! **Why the Linux fast path no longer lets the kernel silently follow a
//! symlink (`RESOLVE_NO_SYMLINKS` added, PR #6817 follow-up):** a bare
//! `openat2(dir, name, RESOLVE_BENEATH)` call's containment boundary is
//! `dir` — the *immediate* parent of the one component being opened — not
//! this mount's true root. That composes correctly across a chain of plain
//! (non-symlink) directories, since each step's `dir` is itself beneath its
//! own parent's already-verified boundary. It does **not** compose for a
//! *parent-relative* symlink: `dir/link -> ../sibling`, where `sibling` is
//! still safely inside the mount root but the `..` in the target text steps
//! above `dir` itself. `RESOLVE_BENEATH` rejects that with `EXDEV`
//! regardless of where `sibling` actually is, because the kernel has no way
//! to know this process's concept of "the true root" — only `RESOLVE_BENEATH`'s
//! own `dir` argument. This is exactly the shape a real pnpm `node_modules`
//! layout uses (`node_modules/@types/react ->
//! ../.pnpm/react@.../node_modules/react`), so silently deferring every
//! symlink to the kernel's own resolution would make this module reject
//! (or, worse, sometimes accept via a coincidentally-matching boundary and
//! sometimes reject) exactly the layouts the "follow in-bounds symlinks"
//! policy change exists to support — and would do so only on Linux, silently
//! diverging from the portable fallback below, which has always tracked the
//! true root correctly (see [`walk_symlink_target`]).
//!
//! The fix: `open_one_inner`'s Linux arm adds `RESOLVE_NO_SYMLINKS` to its
//! `openat2` flags. That makes the kernel refuse to follow *any* symlink at
//! all (single-component call, so "any" only ever means "this one entry"),
//! reporting `ELOOP` — at which point control falls to
//! [`follow_symlink_component`], the exact same fd-anchored, budget-bounded,
//! true-root-tracking walk the portable fallback always used. A plain
//! directory/file component still resolves in one kernel-enforced syscall,
//! at full speed; only the (uncommon) symlink case now pays for a handful of
//! extra syscalls to resolve it correctly and portably. This also closes the
//! [`SymlinkBudget`] gap tracked below: since every symlink hop, on every
//! platform, now flows through [`follow_symlink_component`]'s
//! `budget.consume()` call, the shared per-resolution cap actually bounds
//! Linux resolution too, not just the portable fallback.

use std::cell::Cell;
use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::OsStrExt;
use std::sync::atomic::{AtomicU64, Ordering};

use ironclaw_host_api::VirtualPath;
use rustix::fd::{AsFd, BorrowedFd, OwnedFd};
use rustix::fs::{AtFlags, Dev, Mode, OFlags};
use rustix::io::Errno;

use crate::{CasExpectation, FileType, FilesystemError, FilesystemOperation, RecordVersion};

static LOCAL_WRITE_TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Cap on symlink expansion during a single resolution — the number of
/// symlink hops [`follow_symlink_component`] will follow while descending a
/// walk, on *every* platform (see the module doc for why the Linux fast path
/// now reaches this too, not just the portable fallback). Chosen to match
/// the ballpark of a typical kernel `SYMLOOP_MAX` (Linux and macOS both
/// default to 40) without depending on the host's actual `sysconf` value: a
/// legitimate directory structure never nests this many symlinks, so hitting
/// the cap always means a cycle (or a pathological chain worth failing
/// closed on rather than resolving), never a benign deep structure.
const MAX_SYMLINK_DEPTH: u8 = 32;

/// A single-resolution symlink-hop budget, shared across every
/// [`follow_symlink_component`] hop [`resolve_walk`]/[`descend_creating`]/
/// [`resolve_write_leaf`] make while walking their (possibly
/// multi-component) path — never reset per-component, and never reset
/// per-platform: the Linux fast path and the portable fallback draw on the
/// exact same budget for the exact same reason (see the module doc).
///
/// Before this type existed, [`open_one`] always started a fresh
/// [`MAX_SYMLINK_DEPTH`]-hop budget (`open_one_depth(..., 0)`), and
/// `resolve_walk`/`descend_creating` called `open_one` once per path
/// component in a loop. That let a 10-component path with a
/// `MAX_SYMLINK_DEPTH`-hop chain planted at *each* component accumulate
/// roughly `10 * MAX_SYMLINK_DEPTH` total hops in one resolution — looser
/// than a per-resolution cap should allow. One `SymlinkBudget` is
/// constructed at the top of each of those three functions and threaded by
/// reference through every `open_one`/symlink-following call the walk makes,
/// so the whole resolution shares one `MAX_SYMLINK_DEPTH`-hop ceiling.
///
/// A `Cell`, not an `AtomicU8`: every caller of these functions runs the
/// whole walk synchronously on one blocking-pool thread with no `.await` in
/// between (see the module doc's TOCTOU invariant) and never shares a
/// `SymlinkBudget` across threads, so plain interior mutability is
/// sufficient and avoids the unnecessary atomic-ordering ceremony.
pub(super) struct SymlinkBudget(Cell<u8>);

impl SymlinkBudget {
    pub(super) fn new() -> Self {
        Self(Cell::new(MAX_SYMLINK_DEPTH))
    }

    /// Consumes one hop of the remaining budget; fails closed
    /// (`ResolveError::Escape`) once it is exhausted, exactly like the old
    /// `depth >= MAX_SYMLINK_DEPTH` check it replaces.
    fn consume(&self) -> Result<(), ResolveError> {
        let remaining = self.0.get();
        if remaining == 0 {
            return Err(ResolveError::Escape);
        }
        self.0.set(remaining - 1);
        Ok(())
    }
}

/// Per-resolution state every fd-opening step in this module needs, bundled
/// into one reference so the growing family of `open_one`/`open_one_inner`/
/// `follow_symlink_component`/`walk_symlink_target` signatures doesn't keep
/// gaining one more standalone parameter each time a new per-resolution
/// invariant is added (clippy's `too_many_arguments` at 8; this keeps the
/// count at 7). Both fields share the same lifecycle: constructed once at
/// the top of [`resolve_walk`]/[`descend_creating`]/[`resolve_write_leaf`]/
/// `anchor_for_target` (never per-component, never per-symlink-hop), then
/// threaded by reference through the whole walk.
///
/// - `budget`: see [`SymlinkBudget`].
/// - `anchor_dev`: the resolution anchor's own device, captured once via one
///   `fstat` (not re-`fstat`'d per component) — see [`check_same_device`]
///   (PR #6817 follow-up: macOS `RESOLVE_NO_XDEV` parity).
pub(super) struct ResolveContext<'a> {
    pub(super) budget: &'a SymlinkBudget,
    pub(super) anchor_dev: Dev,
}

/// The outcome of a failed fd-relative resolution step: either a genuine I/O
/// error (propagated as-is), or a symlink/`..`-past-root escape attempt (or
/// a symlink chain deep enough to be indistinguishable from a cycle).
///
/// `AbsoluteSymlinkTarget` (PR #6817 review follow-up) is a distinct variant
/// from the bare `Escape` every other rejection in this module reports —
/// not a different *outcome* (both still fail the resolution closed,
/// unconditionally; see [`walk_symlink_target`]'s doc comment for why an
/// absolute target is always rejected, never reinterpreted against the
/// root), only a richer one: at the one call site that produces it, this
/// module already knows exactly which symlink and exactly what target text
/// caused the rejection, and that is enough to turn an opaque "symlink
/// escapes backend mount" into something a user can actually act on
/// (replace the symlink with a relative one) — see
/// `resolve_error_to_filesystem_error` and
/// `FilesystemError::SymlinkEscape`'s `detail` field.
#[derive(Debug)]
pub(super) enum ResolveError {
    Escape,
    AbsoluteSymlinkTarget {
        symlink_name: OsString,
        target: OsString,
    },
    Io(std::io::Error),
}

pub(super) fn resolve_error_to_filesystem_error(
    path: &VirtualPath,
    operation: FilesystemOperation,
    error: ResolveError,
) -> FilesystemError {
    match error {
        ResolveError::Escape => FilesystemError::SymlinkEscape {
            path: path.clone(),
            detail: None,
        },
        ResolveError::AbsoluteSymlinkTarget {
            symlink_name,
            target,
        } => FilesystemError::SymlinkEscape {
            path: path.clone(),
            detail: Some(format!(
                "symlink {symlink_name:?} has an absolute target {target:?}; absolute \
                 symlink targets are not supported by this filesystem backend, even when \
                 the target would resolve inside the mount — replace it with a relative \
                 symlink target"
            )),
        },
        ResolveError::Io(io_err) => super::io_error(path.clone(), operation, io_err),
    }
}

fn io_err(errno: Errno) -> ResolveError {
    ResolveError::Io(errno.into())
}

/// Rejects `fd` (fail closed, `ResolveError::Escape`) if its device differs
/// from `anchor_dev` — the portable-fallback (macOS, and the Linux
/// `openat2`-unsupported fallback) equivalent of Linux's
/// `openat2(RESOLVE_BENEATH)`, which implies `RESOLVE_NO_XDEV` and rejects a
/// mount crossing at the kernel level (PR #6817 follow-up: macOS
/// `RESOLVE_NO_XDEV` parity). Without this, a bind mount or mounted volume
/// nested anywhere inside a mount's root — lexically "beneath" it, so every
/// other check in this module accepts it — was silently traversable on
/// macOS (and on the Linux fallback) while an identical layout is rejected
/// on the Linux fast path, a platform-dependent containment gap.
///
/// One extra `fstat` per opened fd, called from [`open_one_inner`]'s
/// portable `openat` branch — the single choke point every fresh open in
/// this module's portable path goes through — so every directory hop and
/// every leaf, not just the final component, is checked.
fn check_same_device(anchor_dev: Dev, fd: BorrowedFd<'_>) -> Result<(), ResolveError> {
    let stat = rustix::fs::fstat(fd).map_err(io_err)?;
    if stat.st_dev != anchor_dev {
        return Err(ResolveError::Escape);
    }
    Ok(())
}

/// Opens exactly one path component beneath `dir`, following it if it is a
/// symlink whose resolution stays inside the true mount root, and failing
/// closed (`ResolveError::Escape`) if it would step outside the root or
/// exceed [`MAX_SYMLINK_DEPTH`].
///
/// `ancestors` is every already-open, already-verified directory fd strictly
/// above `dir`, root-first (root itself is *not* included — callers that
/// start at the root pass `&[]`). It exists solely to answer a `..`
/// component inside a followed symlink's target text (see
/// [`walk_symlink_target`]) without ever performing a live, name-based
/// `openat(fd, "..")` lookup: popping this slice hands back an fd this
/// module itself opened and is still holding, which a concurrent rename
/// anywhere in the tree cannot invalidate (an open fd is immune to renames
/// of the directory entry that produced it — see the module-level TOCTOU
/// invariant). Forward (non-`..`) resolution never touches `ancestors` at
/// all.
///
/// On Linux, this is one syscall via
/// `openat2(RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS)` for the common case
/// where `name` is not itself a symlink (falling back below on `ENOSYS`, or
/// on an exhausted `EAGAIN` retry budget). `RESOLVE_NO_SYMLINKS` is the
/// deliberate difference from a bare `RESOLVE_BENEATH`: it makes the kernel
/// refuse to expand `name` even if it *is* a symlink whose resolution would
/// have stayed within `dir`'s own tree, reporting `ELOOP` instead — at which
/// point control falls through to [`follow_symlink_component`], the same
/// fd-anchored, true-root-tracking walk the portable fallback always uses.
/// See the module doc for why letting the kernel silently follow a symlink
/// itself (the old, `RESOLVE_NO_SYMLINKS`-less behavior) breaks containment
/// for a parent-relative symlink target and silently diverges from the
/// portable fallback's correct behavior.
///
/// Everywhere else (including macOS, which has no `openat2`, and the Linux
/// symlink case above), a per-component `openat` with `O_NOFOLLOW` detects a
/// symlink and this module's own bounded walk
/// ([`follow_symlink_component`]) resolves it — fd-anchored at every step,
/// per the module invariant.
///
/// **Every `open`/`openat2` call this function (transitively) makes —
/// including `open_one_beneath_no_symlinks`, the portable `openat` in
/// `open_one_inner`, and `remove_dir_all_fd`'s own directory open — passes
/// `O_NONBLOCK` (PR #6817 review follow-up).** Without it, opening a FIFO
/// with no writer present blocks the calling thread indefinitely — and
/// because every caller here runs on the tokio *blocking pool*
/// (`spawn_blocking`, see `run_blocking` in `local.rs`), that thread is gone
/// for good from the pool's perspective; a handful of concurrent requests
/// against attacker-plantable FIFOs (any writable mount, any path
/// component — a FIFO can sit at an intermediate directory-walk position
/// too, since the kernel's own FIFO-open blocking happens before the
/// `O_DIRECTORY`/`ENOTDIR` check completes) exhausts the pool and wedges the
/// whole process. `O_NONBLOCK` is a documented no-op for regular files and
/// directories (the kernel never blocks opening or reading/writing them for
/// this reason), so it costs nothing on the overwhelmingly common path; it
/// only changes behavior for FIFOs (open returns immediately — successfully
/// for O_RDONLY with no writer, or with `ENXIO` for O_WRONLY with no
/// reader — instead of blocking) and certain blocking character devices.
/// Every caller that goes on to read/write the resulting fd already
/// classifies it via an `AT_SYMLINK_NOFOLLOW` `fstat`/`statat` and rejects
/// anything that isn't the expected type before touching its data (see
/// `read_file`/`read_file_bounded` in `local.rs`), so a FIFO that manages to
/// open non-blocking is still refused — just without ever blocking a thread
/// to find that out.
pub(super) fn open_one(
    root: BorrowedFd<'_>,
    ancestors: &[OwnedFd],
    dir: BorrowedFd<'_>,
    name: &OsStr,
    oflags: OFlags,
    mode: Mode,
    ctx: &ResolveContext<'_>,
) -> Result<OwnedFd, ResolveError> {
    open_one_inner(root, ancestors, dir, name, oflags, mode, ctx)
}

/// Outcome of the Linux `openat2(RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS)`
/// fast-path attempt (see [`open_one`]).
#[cfg(target_os = "linux")]
enum FastOpenOutcome {
    /// Opened directly — `name` was not a symlink.
    Opened(OwnedFd),
    /// `name` is itself a symlink (kernel refused to expand it because of
    /// `RESOLVE_NO_SYMLINKS`); the caller must resolve it via
    /// [`follow_symlink_component`].
    IsSymlink,
    /// The kernel doesn't support `openat2` (`ENOSYS`), or the retry budget
    /// for a benign concurrent-rename `EAGAIN` was exhausted; the caller
    /// must fall back to the portable `O_NOFOLLOW` path below, which does
    /// not share `openat2`'s whole-path-restart-on-`EAGAIN` failure mode.
    Unsupported,
    /// A genuine error (including a real containment/mount-crossing
    /// rejection), to propagate as-is.
    Failed(ResolveError),
}

#[cfg(target_os = "linux")]
fn open_one_beneath_no_symlinks(
    dir: BorrowedFd<'_>,
    name: &OsStr,
    oflags: OFlags,
    mode: Mode,
) -> FastOpenOutcome {
    use rustix::fs::{ResolveFlags, openat2};

    // `openat2(2)` documents `EAGAIN` for "a resolution restart was
    // necessary, e.g. because of concurrent rename or unlink of a path
    // component" — a *legitimate* concurrent mutation (an editor's atomic
    // save, `git checkout`, a parallel build touching the same subtree), not
    // an attack. Without a retry, a real, benign rename racing an unrelated
    // open makes `openat2` spuriously fail and the caller sees an opaque
    // `Backend` error for an operation that would have succeeded a moment
    // later. Retry a small, fixed number of times — each retry is a fresh
    // kernel-side resolution, not a busy spin on our own state, so the bound
    // only needs to outlast one rename's duration, not any unbounded
    // contention; the loop always terminates within `MAX_AGAIN_RETRIES + 1`
    // attempts, never spins unbounded.
    const MAX_AGAIN_RETRIES: u8 = 4;
    let mut retries = 0;
    loop {
        match openat2(
            dir,
            name,
            oflags | OFlags::CLOEXEC | OFlags::NONBLOCK,
            mode,
            ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS,
        ) {
            Ok(fd) => return FastOpenOutcome::Opened(fd),
            // Kernel predates openat2 (< 5.6), or a seccomp/container policy
            // denies the syscall outright.
            Err(Errno::NOSYS) => return FastOpenOutcome::Unsupported,
            Err(Errno::AGAIN) if retries < MAX_AGAIN_RETRIES => {
                retries += 1;
                continue;
            }
            Err(Errno::AGAIN) => return FastOpenOutcome::Unsupported,
            // `RESOLVE_BENEATH` implies `RESOLVE_NO_XDEV`: a mountpoint
            // crossing anywhere in the resolution — even one that stays
            // lexically "inside" the tree, e.g. a bind mount planted under a
            // leaf directory — fails here too. Both a genuine
            // beneath-root escape and an in-tree mount crossing report
            // `EXDEV`; the kernel gives no way to tell them apart from the
            // errno alone, and both are exactly the class this function
            // exists to fail closed on, so both map to `Escape`.
            Err(Errno::XDEV) => return FastOpenOutcome::Failed(ResolveError::Escape),
            // With `RESOLVE_NO_SYMLINKS` set, `ELOOP` from a single-component
            // resolution unambiguously means "this entry is a symlink" (the
            // kernel never attempts to expand it, so a genuine cycle cannot
            // be observed here, exactly like the portable `O_NOFOLLOW` path
            // below) — hand it to the caller to follow via
            // `follow_symlink_component`.
            Err(Errno::LOOP) => return FastOpenOutcome::IsSymlink,
            Err(errno) => return FastOpenOutcome::Failed(io_err(errno)),
        }
    }
}

fn open_one_inner(
    root: BorrowedFd<'_>,
    ancestors: &[OwnedFd],
    dir: BorrowedFd<'_>,
    name: &OsStr,
    oflags: OFlags,
    mode: Mode,
    ctx: &ResolveContext<'_>,
) -> Result<OwnedFd, ResolveError> {
    #[cfg(target_os = "linux")]
    {
        // `openat2(RESOLVE_BENEATH)` implies `RESOLVE_NO_XDEV` (see
        // `open_one_beneath_no_symlinks`'s `Errno::XDEV` arm), so the kernel
        // itself already rejects a mount crossing on this fast path — no
        // additional `check_same_device` call is needed here.
        match open_one_beneath_no_symlinks(dir, name, oflags, mode) {
            FastOpenOutcome::Opened(fd) => return Ok(fd),
            FastOpenOutcome::IsSymlink => {
                return follow_symlink_component(root, ancestors, dir, name, oflags, mode, ctx);
            }
            FastOpenOutcome::Failed(error) => return Err(error),
            // Fall through to the portable per-component path below instead
            // of surfacing an opaque `Backend` error.
            FastOpenOutcome::Unsupported => {}
        }
    }
    match rustix::fs::openat(
        dir,
        name,
        oflags | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
        mode,
    ) {
        // Portable fallback (macOS always; Linux when `openat2` is
        // unsupported) — no kernel-enforced `RESOLVE_NO_XDEV` here, so this
        // module must check the opened fd's device itself before handing it
        // back (PR #6817 follow-up: macOS `RESOLVE_NO_XDEV` parity). Fails
        // closed exactly like every other containment rejection in this
        // module — never falls back to a wider root or a path-string check.
        Ok(fd) => {
            check_same_device(ctx.anchor_dev, fd.as_fd())?;
            Ok(fd)
        }
        // With `O_NOFOLLOW` set, `ELOOP` from a single-component open
        // unambiguously means "this entry is a symlink" (the kernel never
        // attempts to expand it, so a genuine cycle cannot be observed
        // here) — follow it ourselves, fd-anchored and budget-bounded.
        Err(Errno::LOOP) => follow_symlink_component(root, ancestors, dir, name, oflags, mode, ctx),
        // Some platforms (observed on macOS) report `ENOTDIR` rather than
        // `ELOOP` when `O_DIRECTORY | O_NOFOLLOW` hits a symlink — the
        // kernel checks "is this a directory" before "did NOFOLLOW block
        // it", and a symlink is never a directory itself. `ENOTDIR` is
        // ambiguous on its own (a plain non-directory file blocking descent
        // hits it too), so disambiguate with one `AT_SYMLINK_NOFOLLOW`
        // `fstatat` rather than guessing either way.
        Err(Errno::NOTDIR) if oflags.contains(OFlags::DIRECTORY) => {
            match rustix::fs::statat(dir, name, AtFlags::SYMLINK_NOFOLLOW) {
                Ok(stat)
                    if rustix::fs::FileType::from_raw_mode(stat.st_mode)
                        == rustix::fs::FileType::Symlink =>
                {
                    follow_symlink_component(root, ancestors, dir, name, oflags, mode, ctx)
                }
                _ => Err(io_err(Errno::NOTDIR)),
            }
        }
        Err(errno) => Err(io_err(errno)),
    }
}

/// Follows the symlink at `name` (found directly under `dir`), fd-anchored
/// and budget-bounded (see [`SymlinkBudget`]), and finishes opening its
/// ultimate target with the caller's original `oflags`/`mode`. Reached from
/// both the Linux fast path (once `RESOLVE_NO_SYMLINKS` reports `ELOOP`) and
/// the portable `O_NOFOLLOW` path — see the module doc for why both now
/// share this exact walk instead of the Linux side ever letting the kernel
/// expand a symlink itself.
fn follow_symlink_component(
    root: BorrowedFd<'_>,
    ancestors: &[OwnedFd],
    dir: BorrowedFd<'_>,
    name: &OsStr,
    oflags: OFlags,
    mode: Mode,
    ctx: &ResolveContext<'_>,
) -> Result<OwnedFd, ResolveError> {
    ctx.budget.consume()?;
    let target = rustix::fs::readlinkat(dir, name, Vec::new()).map_err(io_err)?;
    let (new_ancestors, anchor, last) =
        walk_symlink_target(root, ancestors, dir, name, target.as_bytes(), ctx)?;
    match last {
        Some(final_name) => open_one_inner(
            root,
            &new_ancestors,
            anchor.as_fd(),
            &final_name,
            oflags,
            mode,
            ctx,
        ),
        // Target had no final component (e.g. "/" or ""): it resolves to
        // the anchor directory itself.
        None => Ok(anchor),
    }
}

/// Walks every component of a symlink's `target` text except the last,
/// fd-anchored throughout, and returns the resulting ancestor stack
/// (everything above the returned directory fd, root-first), that directory
/// fd, plus the final component's own name (left un-opened — the caller
/// decides how to use it: as a final `open_one` leaf, or as a bare name for
/// `atomic_write_file`).
///
/// **Absolute targets are rejected outright (`Escape`), unconditionally —
/// this deliberately matches Linux's native `openat2(RESOLVE_BENEATH)`
/// behavior, not the "reinterpret against the mount root" scheme a first
/// draft of this module attempted.** `RESOLVE_BENEATH` disallows *any*
/// absolute symlink target, regardless of where it would resolve, precisely
/// because the kernel has no concept of "this process's virtual mount root"
/// to reinterpret it against — only `RESOLVE_IN_ROOT` does that, a
/// materially different (chroot-emulating) primitive this module does not
/// use. Reinterpreting an absolute target against the root here would (a)
/// diverge from Linux's real, kernel-enforced behavior for the identical
/// symlink, and (b) not even help in practice: a symlink created by any real
/// tool (`ln -s`, an editor, a package manager) stores the *real host
/// absolute path*, which reinterpreted against a mount's root is essentially
/// never the intended target — it is coincidence, not signal. Both platforms
/// therefore agree: an absolute symlink target is always an escape attempt.
///
/// A relative target resolves from `dir` (the directory the symlink itself
/// lives in). **`..` components are answered by popping `ancestors` — never
/// by a live, name-based `openat(fd, "..")` lookup.** An earlier version of
/// this function did exactly that: capture `cur`'s `(device, inode)`
/// identity, compare it against the root's to decide whether to refuse, and
/// only then call `openat(cur, "..", …)`. That check answers "is `cur`
/// itself the root" correctly, but it does not close the actual race: `..`
/// is *always* a live, name-based lookup, resolved by the kernel at the
/// moment of the syscall against whatever `cur`'s *current* parent directory
/// entry is — not a snapshot from whenever the identity check ran. A
/// concurrent rename of the directory `cur` refers to (e.g. `mv realdir
/// /outside/newname`, executed by another thread between the identity check
/// and the `openat` call) changes what `..` resolves to without changing
/// `cur`'s own `(device, inode)` at all, so no ordering of that check
/// relative to the `openat` call closes the window — confirmed with a
/// working exploit (a test-only sleep widening the window let a concurrent
/// rename make a read return bytes from outside the mount). Popping
/// `ancestors` instead never asks the kernel to re-derive a parent by name
/// at all: every element on the stack is an fd this module already opened
/// and is still holding open, and an open fd's target is fixed regardless of
/// what happens to the directory entries that were used to reach it — no
/// concurrent rename anywhere in the tree can change what a pop yields. "Am
/// I already at the root" becomes `ancestors.is_empty()`, a pure in-process
/// check with no syscall (and therefore no race window) at all, replacing
/// the old `fd_identity(root) == fd_identity(cur)` `fstat` comparison.
///
/// Every forward (non-`.`/`..`) component goes back through
/// [`open_one_inner`] (pushing the fd it steps off of onto the stack before
/// moving on), so a symlink nested inside another symlink's target is itself
/// resolved through this exact same escape/budget-bounded machinery, never
/// treated as plain text — and consumes budget from the same shared
/// [`SymlinkBudget`] as everything else in the resolution.
fn walk_symlink_target(
    root: BorrowedFd<'_>,
    ancestors: &[OwnedFd],
    dir: BorrowedFd<'_>,
    symlink_name: &OsStr,
    target: &[u8],
    ctx: &ResolveContext<'_>,
) -> Result<(Vec<OwnedFd>, OwnedFd, Option<OsString>), ResolveError> {
    if target.starts_with(b"/") {
        // PR #6817 review follow-up: report exactly which symlink and
        // exactly what (attacker/user-supplied, already-on-disk-in-their-
        // own-mount) target text caused the rejection, instead of the bare
        // `Escape` every other rejection in this module reports — see
        // `ResolveError::AbsoluteSymlinkTarget`'s doc comment.
        return Err(ResolveError::AbsoluteSymlinkTarget {
            symlink_name: symlink_name.to_os_string(),
            target: OsStr::from_bytes(target).to_os_string(),
        });
    }
    let mut stack: Vec<OwnedFd> = Vec::with_capacity(ancestors.len());
    for ancestor in ancestors {
        stack.push(dup_fd(ancestor.as_fd())?);
    }
    let mut cur = dup_fd(dir)?;
    let mut components = target
        .split(|byte| *byte == b'/')
        .filter(|component| !component.is_empty())
        .peekable();
    while let Some(component) = components.next() {
        let is_last = components.peek().is_none();
        let component = OsStr::from_bytes(component);
        if component == "." {
            continue;
        }
        if component == ".." {
            match stack.pop() {
                Some(parent) => cur = parent,
                // Nothing left to pop means we are already at (or above,
                // which cannot happen by construction) the root — refuse
                // exactly like the old identity check did, just without a
                // syscall to race.
                None => return Err(ResolveError::Escape),
            }
            continue;
        }
        if is_last {
            return Ok((stack, cur, Some(component.to_os_string())));
        }
        let next = open_one_inner(
            root,
            &stack,
            cur.as_fd(),
            component,
            OFlags::DIRECTORY,
            Mode::empty(),
            ctx,
        )?;
        stack.push(cur);
        cur = next;
    }
    Ok((stack, cur, None))
}

fn dup_fd(fd: BorrowedFd<'_>) -> Result<OwnedFd, ResolveError> {
    rustix::io::dup(fd).map_err(io_err)
}

/// Walks `components` from `root`, following an in-bounds symlink anywhere
/// along the way (see [`open_one`]) and returns the final entry's fd plus —
/// when `components` is non-empty — the fd of its immediate parent directory
/// and its own name (needed by callers that must act on the parent, e.g.
/// `unlinkat`/`renameat`, rather than on the entry itself, which POSIX has no
/// "act on this fd regardless of its name" primitive for).
///
/// `components` empty resolves to the mount root itself (`root` duplicated),
/// with no parent — the mount root has no fd-relative parent inside this
/// mount's sandbox, by design (see `DiskFilesystem::delete`).
///
/// One [`SymlinkBudget`] is constructed here and shared across every
/// component's `open_one` call — the whole multi-component walk shares one
/// [`MAX_SYMLINK_DEPTH`]-hop ceiling, not a fresh one per component. An
/// ancestor stack (see [`open_one`]'s `ancestors` parameter) is built up
/// alongside it, so a symlink discovered at any depth can answer a `..` in
/// its own target text by popping an already-open fd from *this* walk's own
/// history, never by a live `openat(fd, "..")` lookup.
pub(super) fn resolve_walk(
    root: BorrowedFd<'_>,
    components: &[OsString],
    final_oflags: OFlags,
) -> Result<(OwnedFd, Option<(OwnedFd, OsString)>), ResolveError> {
    let Some((leaf, ancestor_components)) = components.split_last() else {
        return Ok((dup_fd(root)?, None));
    };
    let budget = SymlinkBudget::new();
    // Captured once per resolution, not re-`fstat`'d per component (PR
    // #6817 follow-up: macOS `RESOLVE_NO_XDEV` parity) — mirroring
    // `SymlinkBudget`'s own per-resolution-not-per-component construction
    // just above. Caching this at mount-open time instead (on `LocalMount`)
    // would not actually save anything: a `leaf_scoped` mount's anchor is a
    // *fresh* fd opened per call (see `anchor_for_target` in `local.rs`), so
    // its device still has to be read per resolution regardless of where the
    // non-leaf-scoped case's value came from. One `fstat` per request is the
    // real floor either way.
    let anchor_dev = rustix::fs::fstat(root).map_err(io_err)?.st_dev;
    let ctx = ResolveContext {
        budget: &budget,
        anchor_dev,
    };
    let mut cur = dup_fd(root)?;
    let mut ancestors: Vec<OwnedFd> = Vec::with_capacity(ancestor_components.len());
    for component in ancestor_components {
        let next = open_one(
            root,
            &ancestors,
            cur.as_fd(),
            component,
            OFlags::DIRECTORY,
            Mode::empty(),
            &ctx,
        )?;
        ancestors.push(cur);
        cur = next;
    }
    let fd = open_one(
        root,
        &ancestors,
        cur.as_fd(),
        leaf,
        final_oflags,
        Mode::empty(),
        &ctx,
    )?;
    Ok((fd, Some((cur, leaf.clone()))))
}

/// Walks `components` from `root`, creating any missing directory along the
/// way (`mkdir -p` semantics), and returns the final directory's fd *plus*
/// the ancestor stack that got it there (root-first, not including the
/// returned fd itself — the same shape [`open_one`]'s `ancestors` parameter
/// expects). Used by `write_file`/`append_file` (parent-only — matching the
/// previous implementation's "always create the parent chain" behavior) and
/// by `create_dir_all` (full path, leaf included).
///
/// **PR #6817 review follow-up (`mount_registry.rs`, `local.rs`,
/// `fd_resolve.rs`, `filesystem_contract.rs` review threads — one root
/// cause, fixed together here):** this function used to build the ancestor
/// stack purely for its *own* internal `..`-in-a-followed-symlink resolution
/// and then discard it, returning only the final fd. That forced
/// `write_file`/`append_file`'s own leaf resolution
/// ([`resolve_write_leaf`]/`open_one`, both called from `local.rs`) to pass
/// an empty ancestor slice, so a `..` inside a symlink discovered *at the
/// write leaf itself* failed closed instead of resolving past the parent
/// directory `descend_creating` had already opened and verified — a
/// (deliberate, documented, but no longer necessary) usability gap: it fails
/// closed, so it was never a containment hole, just a spurious rejection of
/// an otherwise-legitimate layout. Returning the stack lets every write-side
/// caller thread it into its own leaf resolution, closing the gap. Every
/// existing caller of this function that only needed the final fd
/// (`create_dir_all`, `ensure_scoped_mount_dynamic`) simply takes `.0` and
/// ignores `.1` — this is purely additive.
///
/// Each level is still resolved through [`open_one`], so a symlink swapped
/// into any not-yet-existing ancestor between one level's `mkdirat` and the
/// next level's `open_one` is rejected exactly like any other escaping
/// symlink in the walk — there is no separate "check ancestor, then mkdir,
/// then check again" gap here, because creation and the next level's
/// containment check are the same `open_one` call the next loop iteration
/// makes.
///
/// Like [`resolve_walk`], one [`SymlinkBudget`] is shared across the whole
/// multi-component walk rather than reset per component.
pub(super) fn descend_creating(
    root: BorrowedFd<'_>,
    components: &[OsString],
) -> Result<(OwnedFd, Vec<OwnedFd>), ResolveError> {
    let budget = SymlinkBudget::new();
    // See `resolve_walk`'s matching comment: captured once per resolution,
    // not per component.
    let anchor_dev = rustix::fs::fstat(root).map_err(io_err)?.st_dev;
    let ctx = ResolveContext {
        budget: &budget,
        anchor_dev,
    };
    let mut cur = dup_fd(root)?;
    let mut ancestors: Vec<OwnedFd> = Vec::new();
    for component in components {
        cur = match open_one(
            root,
            &ancestors,
            cur.as_fd(),
            component,
            OFlags::DIRECTORY,
            Mode::empty(),
            &ctx,
        ) {
            Ok(fd) => {
                ancestors.push(cur);
                fd
            }
            Err(ResolveError::Io(io_err)) if io_err.kind() == std::io::ErrorKind::NotFound => {
                match rustix::fs::mkdirat(cur.as_fd(), component.as_os_str(), new_dir_mode()) {
                    Ok(()) => {}
                    Err(Errno::EXIST) => {}
                    Err(errno) => return Err(ResolveError::Io(errno.into())),
                }
                // Re-open through the same `open_one`/`ancestors`-aware path
                // rather than assuming the freshly created entry is safe to
                // use directly: a concurrent swap between the `mkdirat`
                // above and this re-open is rejected exactly like any other
                // escaping symlink in the walk (see the doc above).
                let fd = open_one(
                    root,
                    &ancestors,
                    cur.as_fd(),
                    component,
                    OFlags::DIRECTORY,
                    Mode::empty(),
                    &ctx,
                )?;
                ancestors.push(cur);
                fd
            }
            Err(other) => return Err(other),
        };
    }
    Ok((cur, ancestors))
}

/// Resolves `leaf` under `parent` to the `(parent, leaf-name)` pair a write
/// should actually land at, chasing a chain of in-bounds symlinks *at the
/// leaf position* (bounded by [`MAX_SYMLINK_DEPTH`]).
///
/// `ancestors` is `parent`'s own ancestor stack (root-first, not including
/// `parent` itself) — normally the second element of
/// [`descend_creating`]'s return value, since `parent` is typically exactly
/// the directory `descend_creating` just resolved/created. It is required
/// for the same reason [`open_one`]'s `ancestors` parameter is: a `..` in a
/// leaf-position symlink's target must be answered by popping an
/// already-open fd, never by a live `openat(fd, "..")` lookup (see
/// [`walk_symlink_target`]).
///
/// `rename`/`link` — how [`atomic_write_file`] installs bytes — never follow
/// a symlink at the destination name; they replace/create the directory
/// entry itself. That is exactly right for the entry-replacement case, but
/// wrong for "write through an in-bounds symlink," which callers now expect
/// (a benign in-bounds symlink at a write target is no longer rejected — see
/// the module-level policy change). This function bridges the gap: it
/// resolves the symlink chain itself (fd-anchored, via the same
/// [`walk_symlink_target`] escape/depth-bounded machinery `open_one` uses)
/// down to the ultimate non-symlink `(parent, leaf)` pair, so the caller's
/// `atomic_write_file` rename/link installs the bytes at the symlink's
/// *target*, not over the symlink entry itself. An escaping symlink, or a
/// chain exceeding the depth cap, fails closed (`Escape`) exactly like every
/// other resolution step in this module.
///
/// `append_file`/`write_file`'s non-atomic siblings don't need this: they
/// `open_one` the leaf directly, and `open_one` already follows an in-bounds
/// symlink transparently.
pub(super) fn resolve_write_leaf(
    root: BorrowedFd<'_>,
    ancestors: &[OwnedFd],
    parent: BorrowedFd<'_>,
    leaf: &OsStr,
) -> Result<(OwnedFd, OsString), ResolveError> {
    let budget = SymlinkBudget::new();
    // See `resolve_walk`'s matching comment: captured once per resolution,
    // not per component.
    let anchor_dev = rustix::fs::fstat(root).map_err(io_err)?.st_dev;
    let ctx = ResolveContext {
        budget: &budget,
        anchor_dev,
    };
    let mut cur_ancestors: Vec<OwnedFd> = Vec::with_capacity(ancestors.len());
    for ancestor in ancestors {
        cur_ancestors.push(dup_fd(ancestor.as_fd())?);
    }
    let mut cur_parent = dup_fd(parent)?;
    let mut cur_leaf = leaf.to_os_string();
    loop {
        let is_symlink =
            match rustix::fs::statat(cur_parent.as_fd(), &cur_leaf, AtFlags::SYMLINK_NOFOLLOW) {
                Ok(stat) => {
                    rustix::fs::FileType::from_raw_mode(stat.st_mode)
                        == rustix::fs::FileType::Symlink
                }
                // Not found (nothing at the leaf yet) or any other stat error:
                // nothing to chase through — hand the pair back as-is and let
                // the caller's own probe/creation logic report it.
                Err(_) => false,
            };
        if !is_symlink {
            return Ok((cur_parent, cur_leaf));
        }
        // Consumes from the same shared budget `walk_symlink_target` below
        // draws on for any non-final components of this hop's target text —
        // one ceiling for the whole chase, matching `resolve_walk`/
        // `descend_creating`'s per-resolution (not per-hop) budget.
        ctx.budget.consume()?;
        let target =
            rustix::fs::readlinkat(cur_parent.as_fd(), &cur_leaf, Vec::new()).map_err(io_err)?;
        let (new_ancestors, new_parent, new_leaf) = walk_symlink_target(
            root,
            &cur_ancestors,
            cur_parent.as_fd(),
            &cur_leaf,
            target.as_bytes(),
            &ctx,
        )?;
        let Some(new_leaf) = new_leaf else {
            // Symlink target has no final path component (e.g. "/" or ""):
            // there is no meaningful write-target name. Fail closed rather
            // than guess.
            return Err(ResolveError::Escape);
        };
        cur_ancestors = new_ancestors;
        cur_parent = new_parent;
        cur_leaf = new_leaf;
    }
}

/// Maximum nesting depth [`remove_dir_all_fd`] will descend into before
/// failing closed, rather than recursing without limit.
///
/// This is genuinely recursive Rust code running on a tokio blocking-pool
/// thread — not a regression (`std::fs::remove_dir_all` is also recursive
/// and has no cap of its own), but a deep tree can now be created entirely
/// inside a sandboxed shell's own writable mount, i.e. by someone who will
/// deliberately try to break it. 512 levels comfortably survives on a
/// default thread stack (each frame here is a handful of small locals, no
/// large stack arrays) while still failing far short of any real stack
/// limit if a caller does manage to create a tree this deep. `remove_dir_contents`
/// only `unlinkat`s a symlink entry directly (`AtFlags::empty()`, never
/// following it) and never recurses through one — it decides "is this a
/// directory to recurse into, or a leaf entry to unlink" from an
/// `AT_SYMLINK_NOFOLLOW` `statat` on the entry itself, so a symlink is never
/// mistaken for the (possibly in-bounds, now-followable) directory it might
/// point at. That check is unaffected by this module's new
/// symlink-following policy — depth here is still bounded purely by real,
/// non-symlink directory nesting.
const MAX_REMOVE_DIR_DEPTH: usize = 512;

/// Recursively removes `name` (found directly under `parent`) and everything
/// beneath it, never following a symlink into whatever it points at — a
/// symlinked child is unlinked as itself, exactly like `std::fs::remove_dir_all`,
/// never traversed into.
///
/// This does **not** use [`open_one`] (PR #6817 review follow-up): a caller
/// (`DiskFilesystem::delete`, and this function's own recursion via
/// `remove_dir_contents`) classifies `name` as a real directory via an
/// `AT_SYMLINK_NOFOLLOW` `statat` *before* calling in here — but `open_one`
/// deliberately follows an in-bounds symlink, so a concurrent writer that
/// swaps `name` for a symlink to another in-bounds directory in the gap
/// between that classification and this function's own lookup would have
/// this function recurse into and delete the symlink's *target*, only
/// failing (safely, but too late) on the trailing `unlinkat(REMOVEDIR)`
/// below once the target's contents are already gone. Opening with
/// `O_DIRECTORY | O_NOFOLLOW` directly instead makes the open itself the
/// (re-)classification: a symlink now fails the open with `ELOOP`/`ENOTDIR`
/// and is never traversed, closing the gap structurally rather than relying
/// on the classify-and-open pair being too fast to race in practice.
pub(super) fn remove_dir_all_fd(
    root: BorrowedFd<'_>,
    parent: BorrowedFd<'_>,
    name: &OsStr,
) -> Result<(), std::io::Error> {
    remove_dir_all_fd_bounded(root, parent, name, 0)
}

fn remove_dir_all_fd_bounded(
    root: BorrowedFd<'_>,
    parent: BorrowedFd<'_>,
    name: &OsStr,
    depth: usize,
) -> Result<(), std::io::Error> {
    if depth >= MAX_REMOVE_DIR_DEPTH {
        return Err(std::io::Error::other(format!(
            "directory tree exceeds maximum removal depth of {MAX_REMOVE_DIR_DEPTH} levels; refusing to delete"
        )));
    }
    let dir_fd = rustix::fs::openat(
        parent,
        name,
        OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
    )
    .map_err(std::io::Error::from)?;
    remove_dir_contents(root, dir_fd.as_fd(), depth + 1)?;
    rustix::fs::unlinkat(parent, name, AtFlags::REMOVEDIR).map_err(std::io::Error::from)
}

fn remove_dir_contents(
    root: BorrowedFd<'_>,
    dir: BorrowedFd<'_>,
    depth: usize,
) -> Result<(), std::io::Error> {
    let mut entries = Vec::new();
    {
        let listing = rustix::fs::Dir::read_from(dir)?;
        for entry in listing {
            let entry = entry?;
            let name_bytes = entry.file_name().to_bytes();
            if name_bytes == b"." || name_bytes == b".." {
                continue;
            }
            let name = OsStr::from_bytes(name_bytes).to_os_string();
            let stat = rustix::fs::statat(dir, &name, AtFlags::SYMLINK_NOFOLLOW)?;
            let is_dir = rustix::fs::FileType::from_raw_mode(stat.st_mode)
                == rustix::fs::FileType::Directory;
            entries.push((name, is_dir));
        }
    }
    for (name, is_dir) in entries {
        if is_dir {
            remove_dir_all_fd_bounded(root, dir, &name, depth)?;
        } else {
            rustix::fs::unlinkat(dir, &name, AtFlags::empty())?;
        }
    }
    Ok(())
}

fn resolve_error_to_io(error: ResolveError) -> std::io::Error {
    match error {
        ResolveError::Escape => std::io::Error::other("symlink escape"),
        ResolveError::AbsoluteSymlinkTarget {
            symlink_name,
            target,
        } => std::io::Error::other(format!(
            "symlink {symlink_name:?} has an absolute target {target:?}, which is not \
             supported"
        )),
        ResolveError::Io(io_err) => io_err,
    }
}

pub(super) fn read_all(fd: OwnedFd) -> Result<Vec<u8>, std::io::Error> {
    use std::io::Read;
    let mut file = std::fs::File::from(fd);
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(bytes)
}

pub(super) fn write_all(fd: OwnedFd, bytes: &[u8]) -> Result<(), std::io::Error> {
    use std::io::Write;
    let mut file = std::fs::File::from(fd);
    file.write_all(bytes)?;
    file.flush()
}

pub(super) fn new_file_mode() -> Mode {
    Mode::RUSR | Mode::WUSR | Mode::RGRP | Mode::WGRP | Mode::ROTH | Mode::WOTH
}

fn new_dir_mode() -> Mode {
    Mode::RWXU | Mode::RWXG | Mode::RWXO
}

pub(super) fn map_file_type(kind: rustix::fs::FileType) -> FileType {
    match kind {
        rustix::fs::FileType::RegularFile => FileType::File,
        rustix::fs::FileType::Directory => FileType::Directory,
        rustix::fs::FileType::Symlink => FileType::Symlink,
        _ => FileType::Other,
    }
}

/// Atomically installs `bytes` as `leaf` under `parent`, via a temp file
/// created in the same directory and then renamed (`CasExpectation::Any`) or
/// hard-linked into place (`CasExpectation::Absent`) — fd-relative
/// (`renameat`/`linkat` against `parent`, an already fd-resolved,
/// already-verified directory) instead of path-relative.
///
/// Callers that want "write through an in-bounds symlink" must resolve the
/// symlink chain at `leaf` themselves first (see [`resolve_write_leaf`]) and
/// pass the *resolved* `(parent, leaf)` pair here — `rename`/`link` never
/// follow a symlink at the destination name (they replace/create the
/// directory entry itself), so by the time bytes are installed, `leaf` must
/// already name the real, non-symlink target.
pub(super) fn atomic_write_file(
    virtual_path: &VirtualPath,
    parent: BorrowedFd<'_>,
    leaf: &OsStr,
    bytes: &[u8],
    cas: CasExpectation,
) -> Result<(), FilesystemError> {
    // A pre-existing entry at `leaf` was already resolved by the caller
    // (`resolve_write_leaf`) if it was a symlink, so by construction `leaf`
    // here never names a symlink itself — this probe exists only to
    // distinguish "create" from "overwrite" for the CAS/temp-file dance
    // below, not to police symlinks (that already happened upstream). Both
    // run inside the same non-yielding blocking closure, so there is no
    // `.await` for a swap to land between the probe and the install.
    match rustix::fs::statat(parent, leaf, AtFlags::SYMLINK_NOFOLLOW) {
        Ok(_existing) => {}
        Err(Errno::NOENT) => {}
        Err(errno) => {
            return Err(super::io_error(
                virtual_path.clone(),
                FilesystemOperation::WriteFile,
                errno.into(),
            ));
        }
    }

    let counter = LOCAL_WRITE_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut temp_name = OsString::from(".");
    temp_name.push(leaf);
    temp_name.push(format!(".tmp.{counter}"));

    let temp_fd = rustix::fs::openat(
        parent,
        &temp_name,
        OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::CLOEXEC,
        new_file_mode(),
    )
    .map_err(|errno| {
        super::io_error(
            virtual_path.clone(),
            FilesystemOperation::WriteFile,
            errno.into(),
        )
    })?;

    let write_result = (|| -> std::io::Result<()> {
        use std::io::Write;
        let mut file = std::fs::File::from(temp_fd);
        file.write_all(bytes)?;
        file.sync_all()
    })();
    if let Err(error) = write_result {
        let _ = rustix::fs::unlinkat(parent, &temp_name, AtFlags::empty());
        return Err(super::io_error(
            virtual_path.clone(),
            FilesystemOperation::WriteFile,
            error,
        ));
    }

    let install_result = match cas {
        CasExpectation::Any => {
            rustix::fs::renameat(parent, &temp_name, parent, leaf).map_err(|errno| {
                super::io_error(
                    virtual_path.clone(),
                    FilesystemOperation::WriteFile,
                    errno.into(),
                )
            })
        }
        CasExpectation::Absent => {
            match rustix::fs::linkat(parent, &temp_name, parent, leaf, AtFlags::empty()) {
                Ok(()) => {
                    let _ = rustix::fs::unlinkat(parent, &temp_name, AtFlags::empty());
                    Ok(())
                }
                Err(Errno::EXIST) => {
                    let _ = rustix::fs::unlinkat(parent, &temp_name, AtFlags::empty());
                    Err(FilesystemError::VersionMismatch {
                        path: virtual_path.clone(),
                        expected: None,
                        found: Some(RecordVersion::from_backend(0)),
                    })
                }
                Err(errno) => {
                    let _ = rustix::fs::unlinkat(parent, &temp_name, AtFlags::empty());
                    Err(super::io_error(
                        virtual_path.clone(),
                        FilesystemOperation::WriteFile,
                        errno.into(),
                    ))
                }
            }
        }
        CasExpectation::Version(_) => Err(FilesystemError::Unsupported {
            path: virtual_path.clone(),
            operation: FilesystemOperation::WriteFile,
        }),
    };

    install_result?;

    // Best-effort durability: fsync the parent directory so the rename/link
    // above survives a crash. Not part of the containment/TOCTOU surface —
    // failure here is reported but the write itself already succeeded.
    let parent_file = std::fs::File::from(dup_fd(parent).map_err(resolve_error_to_io).map_err(
        |error| super::io_error(virtual_path.clone(), FilesystemOperation::WriteFile, error),
    )?);
    parent_file.sync_all().map_err(|error| {
        super::io_error(virtual_path.clone(), FilesystemOperation::WriteFile, error)
    })
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::symlink;

    use rustix::fd::AsFd;
    use rustix::fs::{Mode, OFlags, openat};
    use tempfile::tempdir;

    use super::remove_dir_all_fd;

    /// Deterministic, platform-independent pin on the mount-crossing check's
    /// own contract (PR #6817 follow-up, macOS `RESOLVE_NO_XDEV` parity):
    /// given an anchor device that differs from an opened fd's device, the
    /// resolver must fail closed with `ResolveError::Escape` — never fall
    /// back to a path-string check or a wider root. This does not exercise a
    /// *real* mount crossing (see the `#[cfg(target_os = "macos")]` test
    /// below for that); it pins `check_same_device`'s own logic so the
    /// contract is covered even on a CI runner where a real bind mount can't
    /// be constructed.
    #[test]
    fn check_same_device_rejects_mismatched_device() {
        let storage = tempdir().unwrap();
        let fd = openat(
            rustix::fs::CWD,
            storage.path(),
            OFlags::DIRECTORY | OFlags::RDONLY,
            Mode::empty(),
        )
        .unwrap();
        let real_dev = rustix::fs::fstat(&fd).unwrap().st_dev;

        // Same device: must pass.
        assert!(super::check_same_device(real_dev, fd.as_fd()).is_ok());

        // A device value that cannot match the real one: must fail closed.
        let bogus_dev = real_dev.wrapping_add(1).max(1);
        assert!(matches!(
            super::check_same_device(bogus_dev, fd.as_fd()),
            Err(super::ResolveError::Escape)
        ));
    }

    /// Real cross-device traversal regression for the macOS/portable-fallback
    /// `RESOLVE_NO_XDEV` parity gap (PR #6817 follow-up): on Linux,
    /// `openat2(RESOLVE_BENEATH)` implies `RESOLVE_NO_XDEV` and rejects a
    /// bind-mount crossing inside the resolution root — but the portable
    /// fallback `open_one_inner` uses on macOS (and as the Linux
    /// `openat2`-unsupported fallback) had zero `st_dev` check, so a mounted
    /// volume nested inside a mount's root was traversable there and
    /// rejected on Linux for an identical layout.
    ///
    /// This mounts a real APFS disk image (via `hdiutil`, no root required)
    /// at a subdirectory of a tempdir-backed resolution root, then asserts
    /// `resolve_walk` refuses to walk into it. A genuine, kernel-level device
    /// boundary — not a simulated one.
    #[cfg(target_os = "macos")]
    #[test]
    fn resolve_walk_rejects_real_mount_crossing_on_macos() {
        use std::os::unix::fs::MetadataExt;
        use std::process::Command;

        let storage = tempdir().unwrap();
        let root_path = storage.path();
        let mount_point = root_path.join("crossing");
        std::fs::create_dir_all(&mount_point).unwrap();

        let dmg_dir = tempdir().unwrap();
        let dmg_path = dmg_dir.path().join("xdev_test.dmg");

        let create_status = Command::new("hdiutil")
            .args([
                "create",
                "-size",
                "5m",
                "-fs",
                "APFS",
                "-volname",
                "xdevtest",
                dmg_path.to_str().unwrap(),
            ])
            .status()
            .expect("hdiutil create must run on macOS test hosts");
        assert!(create_status.success(), "hdiutil create failed");

        // `hdiutil create` appends `.dmg` if not already present in some
        // macOS versions' output naming; locate the actual produced file.
        let actual_dmg = if dmg_path.exists() {
            dmg_path
        } else {
            dmg_dir.path().join("xdev_test.dmg.dmg")
        };

        let attach_status = Command::new("hdiutil")
            .args([
                "attach",
                actual_dmg.to_str().unwrap(),
                "-mountpoint",
                mount_point.to_str().unwrap(),
                "-nobrowse",
            ])
            .status()
            .expect("hdiutil attach must run on macOS test hosts");
        assert!(attach_status.success(), "hdiutil attach failed");

        // Ensure the volume is detached even if an assertion below panics.
        struct DetachGuard(std::path::PathBuf);
        impl Drop for DetachGuard {
            fn drop(&mut self) {
                let _ = Command::new("hdiutil")
                    .args(["detach", self.0.to_str().unwrap(), "-force"])
                    .status();
            }
        }
        let _detach = DetachGuard(mount_point.clone());

        // Sanity-check the mount actually crossed a device boundary before
        // trusting the resolver's rejection of it.
        let root_dev = std::fs::metadata(root_path).unwrap().dev();
        let mounted_dev = std::fs::metadata(&mount_point).unwrap().dev();
        assert_ne!(
            root_dev, mounted_dev,
            "test setup did not actually cross a device boundary"
        );

        let root_fd = openat(
            rustix::fs::CWD,
            root_path,
            OFlags::DIRECTORY | OFlags::RDONLY,
            Mode::empty(),
        )
        .unwrap();

        let result = super::resolve_walk(
            root_fd.as_fd(),
            &[
                std::ffi::OsString::from("crossing"),
                std::ffi::OsString::from("anything"),
            ],
            OFlags::RDONLY,
        );

        assert!(
            matches!(result, Err(super::ResolveError::Escape)),
            "resolving into a mounted volume nested inside the resolution root must fail \
             closed with Escape, matching Linux's RESOLVE_NO_XDEV behavior — got {result:?}"
        );
    }

    /// Deterministic (non-timing-dependent) regression for the recursive
    /// `delete` symlink race flagged in PR #6817 review: `remove_dir_all_fd`
    /// is only ever called by a caller that has already classified `name` as
    /// a real directory via a `SYMLINK_NOFOLLOW` `statat` — but this
    /// function then re-looks-up `name` itself via `open_one`, which
    /// deliberately follows in-bounds symlinks. Rather than racing that gap
    /// against a background thread (tried, and could not reliably win it —
    /// the classify-then-open window in `DiskFilesystem::delete` is a
    /// handful of back-to-back syscalls with no intervening work), this
    /// pins the function's *own* contract directly: called with `name`
    /// already pointing at a symlink to a sibling directory (modeling "the
    /// race already landed"), it must never recurse into and delete that
    /// sibling's contents.
    #[test]
    fn remove_dir_all_fd_never_deletes_through_a_symlink_at_the_target_name() {
        let storage = tempdir().unwrap();
        let root_path = storage.path();

        let target_dir = root_path.join("target-dir");
        std::fs::create_dir_all(&target_dir).unwrap();
        let precious = target_dir.join("precious.txt");
        std::fs::write(&precious, b"PRECIOUS-CONTENT").unwrap();

        // `victim` is a symlink to `target-dir`, not a real directory —
        // modeling the post-swap state a caller's earlier
        // `statat(SYMLINK_NOFOLLOW)` classification can no longer see by
        // the time this function's own `open_one` call runs.
        symlink("target-dir", root_path.join("victim")).unwrap();

        let root_fd = openat(
            rustix::fs::CWD,
            root_path,
            OFlags::DIRECTORY | OFlags::RDONLY,
            Mode::empty(),
        )
        .unwrap();
        let parent_fd = openat(
            &root_fd,
            ".",
            OFlags::DIRECTORY | OFlags::RDONLY,
            Mode::empty(),
        )
        .unwrap();

        // Whether this returns `Ok` or `Err` is not the contract being
        // pinned here — either is an acceptable way to refuse recursing
        // through the symlink. Losing `precious.txt` is not.
        let _ = remove_dir_all_fd(
            root_fd.as_fd(),
            parent_fd.as_fd(),
            std::ffi::OsStr::new("victim"),
        );

        assert_eq!(
            std::fs::read(&precious).unwrap(),
            b"PRECIOUS-CONTENT",
            "remove_dir_all_fd must never delete a sibling directory's contents by \
             following a symlink planted at the name it was asked to remove"
        );
    }
}
