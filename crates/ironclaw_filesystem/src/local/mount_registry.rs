//! Mount-registry layer for the local disk backend: registering (static and
//! dynamic) mounts, and routing a [`VirtualPath`] to the [`MountTarget`] a
//! caller resolves it against.
//!
//! Extracted out of `local.rs` (mirroring the existing `local/fd_resolve.rs`
//! extraction) once FIX 1 (wiring `ensure_scoped_mount` into the production
//! request path) and FIX 2 (bounding its dynamic-mount fd growth) added
//! enough logic to this exact area that it earned its own file. Unlike
//! `fd_resolve.rs`, this module is NOT independent of `DiskFilesystem` — it
//! *is* `DiskFilesystem`'s mount bookkeeping, split into a second `impl
//! DiskFilesystem` block in its own file (Rust allows an inherent `impl`
//! to be split across multiple blocks/files; it does not allow that for a
//! single trait impl, which is why `RootFilesystem for DiskFilesystem`
//! itself — including the one-line `ensure_scoped_mount` trait method that
//! delegates to [`DiskFilesystem::ensure_scoped_mount_dynamic`] here — stays
//! in `local.rs`).
//!
//! What moved here: [`DiskFilesystem::mount_local`]/
//! [`mount_local_per_leaf`](DiskFilesystem::mount_local_per_leaf) (boot-time
//! static mount registration), [`DiskFilesystem::ensure_scoped_mount_dynamic`]
//! (per-request dynamic mount registration, LRU-bounded), the routing helper
//! [`DiskFilesystem::resolve_mount_target`], and the [`LocalMount`]/
//! [`MountTarget`] types themselves. What stayed in `local.rs`:
//! `anchor_for_target` (needs `MountTarget`'s fields, but is about
//! *resolving inside* an already-routed mount, not registry bookkeeping) and
//! the `RootFilesystem` trait impl (every operation's entry point).

use std::{
    ffi::OsString,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use ironclaw_host_api::{HostPath, VirtualPath};
use rustix::fd::{AsFd, OwnedFd};
use rustix::fs::{Mode, OFlags};

use super::DiskFilesystem;
use crate::{FilesystemError, FilesystemOperation, path_prefix_matches};

/// Upper bound on how many *dynamically*-registered scoped mounts (see
/// [`DiskFilesystem::ensure_scoped_mount_dynamic`]) this backend keeps open
/// directory fds for at once. Boot-time static mounts
/// ([`DiskFilesystem::mount_local`]/
/// [`mount_local_per_leaf`](DiskFilesystem::mount_local_per_leaf)) are never
/// counted against this bound or evicted — there are only a handful of them,
/// fixed at process startup, and every future virtual path under one
/// resolves through it, so evicting one would break resolution for its whole
/// subtree with no way to recreate it from a later request.
///
/// Once `ensure_scoped_mount` is reachable from the production request path
/// (the fix this constant ships alongside), one dynamic mount is registered
/// per distinct `MountGrant::target` a caller resolves through — in
/// practice, one per active (tenant, user) scope touching a
/// wide-mount-backed grant like `/skills`. Left unbounded, that is one open
/// directory fd per distinct scope for the process's entire lifetime: a
/// slow leak against `RLIMIT_NOFILE` on a long-lived, multi-tenant host.
///
/// 512 is chosen against a default Linux soft `RLIMIT_NOFILE` of 1024:
/// even if every slot is a live fd, this cache alone cannot claim more than
/// half of a stock default limit, leaving headroom for sockets, log files,
/// the boot-time static mounts, and every other fd the process needs.
/// Exceeding the bound evicts the least-recently-touched dynamic entry
/// rather than failing the request — see
/// [`DiskFilesystem::ensure_scoped_mount_dynamic`].
const MAX_DYNAMIC_MOUNTS: usize = 512;

/// Upper bound on how many path components a single [`VirtualPath`] tail may
/// resolve into beneath a mount (PR #6817 review follow-up — "unbounded
/// ancestor-fd retention").
///
/// `resolve_walk`/`descend_creating` (`local/fd_resolve.rs`) hold one open
/// ancestor directory fd per path component *simultaneously* for the
/// duration of a single resolution — they need every one of them live at
/// once so a `..` inside a followed symlink can answer by popping an
/// already-open fd rather than a live, racy `openat(fd, "..")` lookup (see
/// `fd_resolve.rs`'s module doc). Unlike [`MAX_DYNAMIC_MOUNTS`] (which bounds
/// a *cache* with an LRU-eviction release valve), there was no cap at all on
/// a single request's own component count: every component of `path` is
/// caller-supplied (a `VirtualPath` has no length or depth limit of its
/// own — see `ironclaw_host_api::path`), so an attacker could send one
/// pathologically deep path and force this backend to hold thousands of
/// open fds for the lifetime of that one resolution — exhaustion that is
/// process-wide (`RLIMIT_NOFILE`), not scoped to the offending request.
///
/// **Fails closed, never wide.** This check runs in
/// [`resolve_mount_route`](DiskFilesystem::resolve_mount_route) *before* any
/// fd is opened — the earliest possible point, before `MountTarget` is even
/// constructed — and rejects with [`FilesystemError::PathTooDeep`] outright.
/// There is deliberately no fallback path (no "widen to a shorter prefix",
/// no "truncate and continue"): the PR history already has one proven
/// cross-tenant escape from a bound that silently fell back to a wider
/// scope on eviction (the now-removed LRU-fallback shape a different
/// structure in this crate used before PR #6817) — this cap does the
/// opposite by construction: exceeding it is always a hard `Err`, never a
/// change in which (narrower or wider) boundary a request resolves against.
///
/// 2048 is chosen well above both any real virtual-path shape this crate's
/// own callers ever construct (`/projects/tenants/<t>/users/<u>/skills/...`
/// tops out at a double-digit component count even for a deeply nested
/// project) *and* the deepest deliberately-deep tree this crate's own test
/// suite builds via `create_dir_all`/`delete`
/// (`local_backend_delete_of_tree_exceeding_max_depth_fails_cleanly` goes to
/// 600+2 components, one past `MAX_REMOVE_DIR_DEPTH`, to pin the *separate*
/// recursive-delete stack-depth cap) — so this cap only ever fires on a
/// component count no legitimate caller or existing regression test
/// approaches, while still bounding a single resolution's simultaneous fd
/// cost to a small, fixed number rather than an attacker-chosen one.
pub(super) const MAX_PATH_COMPONENTS: usize = 2048;

/// Process-wide logical clock for dynamic-mount LRU recency. A simple
/// monotonically increasing counter (not wall-clock time) is enough: only
/// the *relative order* of touches across entries in one `DiskFilesystem`'s
/// mount list matters for picking an eviction victim, and a counter avoids
/// any dependency on clock resolution or monotonicity guarantees. Shared
/// across every `DiskFilesystem` instance in the process — harmless, since
/// each instance only ever compares its own entries' values against each
/// other, never against another instance's.
static DYNAMIC_MOUNT_CLOCK: AtomicU64 = AtomicU64::new(0);

fn next_touch() -> u64 {
    DYNAMIC_MOUNT_CLOCK.fetch_add(1, Ordering::Relaxed)
}

#[derive(Debug)]
pub(super) struct LocalMount {
    pub(super) virtual_root: VirtualPath,
    /// An open directory descriptor on the mount's canonical host root,
    /// opened once at mount time. Every request resolves *from this fd*
    /// (or, for a `leaf_scoped` mount, from a fresh per-call anchor fd
    /// opened beneath it — see `anchor_for_target`), component by
    /// component, following an in-bounds symlink and refusing an escaping
    /// one atomically — via the single-syscall `openat2(RESOLVE_BENEATH)`
    /// on Linux, or this crate's own bounded fd-anchored walk everywhere
    /// else. A symlink swapped in after any earlier check is never handed
    /// to a later, independent path-based syscall, closing the
    /// pathname-check-then-separate-syscall TOCTOU window `resolve_existing`
    /// / `resolve_for_write` / `resolve_for_create_dir_all` used to leave
    /// open.
    ///
    /// Wrapped in `Arc` (not re-opened per request): cloning an `Arc<OwnedFd>`
    /// shares the same underlying open file description rather than
    /// `dup`-ing a new fd, which is exactly what we want here — directory
    /// fds used only for relative `openat`/`mkdirat`/`unlinkat`/`fstatat`
    /// lookups carry no mutable per-fd state (no seek offset, no O_APPEND
    /// cursor) that a concurrent "clone" could corrupt, so many callers
    /// reading `self.mounts` concurrently and cloning this `Arc` is safe
    /// without any lock.
    pub(super) root_fd: Arc<OwnedFd>,
    /// When `true`, this mount is shared by many callers who are each only
    /// ever granted a single leaf subtree of it (one `MountGrant` target
    /// per caller, narrowed by the composition-layer `MountView` resolver —
    /// e.g. the sandboxed-profile `/workspace` mount, where every user's
    /// `MountView` target is `/workspace/<digest>`).
    ///
    /// `leaf_scoped` still needs a *narrower* containment boundary than an
    /// ordinary mount: now that `open_one` follows an in-bounds symlink
    /// instead of rejecting every symlink outright, a symlink planted in one
    /// caller's leaf that resolves into a sibling leaf would stay "beneath"
    /// the wide, shared mount root and pass containment there. So for a
    /// `leaf_scoped` mount, every request is anchored not at `root_fd` but
    /// at a fresh fd opened *at the caller's own leaf directory* (see
    /// `anchor_for_target`) before any further walking happens —
    /// `RESOLVE_BENEATH` (or the portable fallback's escape check) then
    /// enforces containment against that narrower anchor, so a symlink can
    /// never step from one caller's leaf into a sibling leaf. `leaf_scoped`
    /// additionally still preserves the original policy that a request
    /// against the bare mount root (no leaf segment at all) must fail closed
    /// rather than resolving to the shared parent every caller's leaf lives
    /// under. See [`DiskFilesystem::resolve_mount_target`].
    pub(super) leaf_scoped: bool,
    /// `true` for a mount registered by
    /// [`DiskFilesystem::ensure_scoped_mount_dynamic`] (a caller-scoped
    /// anchor, e.g. one per (tenant, user) `/skills` grant); `false` for a
    /// boot-time [`mount_local`](DiskFilesystem::mount_local)/
    /// [`mount_local_per_leaf`](DiskFilesystem::mount_local_per_leaf) mount.
    /// Only `dynamic` entries count against [`MAX_DYNAMIC_MOUNTS`] and are
    /// eligible for LRU eviction — static entries are exempt (see
    /// `MAX_DYNAMIC_MOUNTS`'s doc for why).
    pub(super) dynamic: bool,
    /// Logical-clock recency stamp for LRU eviction among `dynamic` entries
    /// only (see [`next_touch`]); meaningless and never read for a static
    /// entry. An `AtomicU64` rather than a plain field so
    /// [`DiskFilesystem::resolve_mount_target`] can update recency through a
    /// shared `&self`/read-lock reference on every request that resolves
    /// through this mount, without needing the write lock just to record
    /// "this was used" — the field is interior-mutable, the `Vec` slot
    /// itself is not relocated by a touch.
    pub(super) last_used: AtomicU64,
}

/// A mount plus the path components to walk under its `root_fd`. Carries no
/// host path — every subsequent step is fd-relative.
pub(super) struct MountTarget {
    pub(super) root_fd: Arc<OwnedFd>,
    pub(super) components: Vec<OsString>,
    /// Mirrors [`LocalMount::leaf_scoped`] — carried through so `local.rs`'s
    /// `anchor_for_target` can anchor containment at the caller's own leaf
    /// instead of the shared mount root now that `open_one` follows
    /// in-bounds symlinks. Without this, a symlink planted inside one
    /// caller's leaf that resolves into a sibling leaf would pass
    /// containment (both stay "beneath" the wide, shared mount root) even
    /// though it must not.
    pub(super) leaf_scoped: bool,
}

impl DiskFilesystem {
    /// Mounts a host directory during trusted setup.
    ///
    /// This API is intentionally synchronous because it mutates in-memory mount
    /// configuration and is not part of the async runtime operation path. Async
    /// file operations after mount setup use fd-relative syscalls run on the
    /// blocking pool.
    pub fn mount_local(
        &mut self,
        virtual_root: VirtualPath,
        host_root: HostPath,
    ) -> Result<(), FilesystemError> {
        self.mount_local_impl(virtual_root, host_root, false)
    }

    /// Mounts a host directory shared across many callers, each of whom is
    /// only ever granted (via their own `MountView`) a single leaf subtree
    /// of it — e.g. the `HostedSingleTenantVolumeSandboxed` profile's
    /// `/workspace` mount, whose shared parent holds every user's leaf
    /// sandbox-workspace directory. See [`LocalMount::leaf_scoped`] for why
    /// this no longer needs a distinct containment boundary from
    /// [`mount_local`](Self::mount_local).
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
        // `&mut self`, so `get_mut` never actually contends a lock — this
        // still goes through the same `RwLock<Vec<LocalMount>>` storage
        // `ensure_scoped_mount_dynamic` uses for its `&self` dynamic
        // registration, rather than keeping two separate storage shapes in
        // sync.
        let mounts = self
            .mounts
            .get_mut()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if mounts
            .iter()
            .any(|mount| mount.virtual_root.as_str() == virtual_root.as_str())
        {
            return Err(FilesystemError::MountConflict { path: virtual_root });
        }

        let (canonical_root, root_fd) = open_mount_root(&virtual_root, &host_root)?;
        let _ = canonical_root;

        mounts.push(LocalMount {
            virtual_root,
            root_fd: Arc::new(root_fd),
            leaf_scoped,
            dynamic: false,
            last_used: AtomicU64::new(0),
        });
        Ok(())
    }

    fn has_mount(&self, virtual_root: &VirtualPath) -> bool {
        let mounts = self
            .mounts
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        mounts
            .iter()
            .any(|mount| mount.virtual_root.as_str() == virtual_root.as_str())
    }

    /// Routes `path` to its mount and splits the tail into path components
    /// under that mount's `root_fd`. No filesystem access happens here —
    /// this is pure string/virtual-path routing, and is not part of the
    /// TOCTOU surface: the actual containment enforcement happens
    /// fd-relatively in `open_one` as each returned component is walked.
    ///
    /// This is the public routing entry point every real filesystem
    /// operation (`read_file`, `write_file`, ... in `local.rs`) resolves
    /// through, so it is where the PR #6817 fix applies: see
    /// [`Self::narrowing_lost`] for the fail-closed check layered on top of
    /// the raw routing in [`Self::resolve_mount_route`]. Internal bootstrap
    /// callers that must legitimately resolve through a *wider* ancestor
    /// mount before a narrower one exists — namely
    /// `ensure_scoped_mount_dynamic` descending from the parent mount to
    /// open its own narrow anchor — call `resolve_mount_route` directly to
    /// bypass this check.
    pub(super) fn resolve_mount_target(
        &self,
        path: &VirtualPath,
    ) -> Result<MountTarget, FilesystemError> {
        let (target, matched_virtual_root) = self.resolve_mount_route(path)?;
        if self.narrowing_lost(path, &matched_virtual_root) {
            // A virtual root at least as specific as this path was, at some
            // point, established as this backend's own containment boundary
            // for that scope (`ensure_scoped_mount_dynamic`) — but no mount
            // that specific is live right now, only a shorter/wider
            // ancestor (`matched_virtual_root`). Matching the ancestor here
            // would silently re-widen containment for whatever caller
            // narrowed this scope, reopening the same-storage-root
            // cross-tenant symlink escape narrowing exists to close (PR
            // #6817). Fail loudly instead: the caller must re-establish
            // narrowing (`ensure_scoped_mount`) before this path resolves
            // again, exactly like a first-time access.
            return Err(FilesystemError::MountNotFound { path: path.clone() });
        }
        Ok(target)
    }

    /// `true` when `path` requires narrowing (some virtual root at least as
    /// specific as `matched_virtual_root` was previously established via
    /// `ensure_scoped_mount_dynamic`) but the mount that actually matched in
    /// `resolve_mount_route` is not that narrow root itself — i.e. narrowing
    /// was lost (typically: LRU-evicted) and never re-established.
    fn narrowing_lost(&self, path: &VirtualPath, matched_virtual_root: &str) -> bool {
        let narrow_roots = self
            .narrow_scoped_roots
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        narrow_roots.iter().any(|root| {
            root.len() > matched_virtual_root.len() && path_prefix_matches(root, path.as_str())
        })
    }

    /// The raw longest-prefix routing `resolve_mount_target` wraps with the
    /// narrowing-lost check — used directly, without that check, by
    /// `ensure_scoped_mount_dynamic`'s own bootstrap resolution (it must be
    /// able to resolve through a wider ancestor mount precisely in order to
    /// *create* the narrower one; requiring the narrow mount to already
    /// exist to resolve through it would be circular). Returns the matched
    /// mount's own `virtual_root` alongside the target so callers can tell
    /// which mount actually satisfied the routing.
    fn resolve_mount_route(
        &self,
        path: &VirtualPath,
    ) -> Result<(MountTarget, String), FilesystemError> {
        let mounts = self
            .mounts
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mount = mounts
            .iter()
            .filter(|mount| path_prefix_matches(mount.virtual_root.as_str(), path.as_str()))
            .max_by_key(|mount| mount.virtual_root.as_str().len())
            .ok_or_else(|| FilesystemError::MountNotFound { path: path.clone() })?;

        // Recency touch for LRU eviction (see `MAX_DYNAMIC_MOUNTS`). Every
        // real filesystem operation routes through here, so this is the true
        // usage signal — a no-op for a static mount (its `last_used` is
        // never read), and for a dynamic mount it keeps an actively-used
        // scope's entry from looking idle just because no *new* registration
        // happened recently.
        if mount.dynamic {
            mount.last_used.store(next_touch(), Ordering::Relaxed);
        }

        let matched_virtual_root = mount.virtual_root.as_str().to_string();
        let tail = path
            .as_str()
            .strip_prefix(mount.virtual_root.as_str())
            .unwrap_or_default()
            .trim_start_matches('/');

        if tail.is_empty() && mount.leaf_scoped {
            // A leaf-scoped mount has no safe target for the bare mount path
            // itself — that would be "every caller's leaf", the shared-parent
            // boundary this mount kind exists to eliminate. The
            // composition-layer `MountView` always supplies a leaf, but that
            // invariant is enforced one layer up, so fail closed here.
            return Err(FilesystemError::PathOutsideMount { path: path.clone() });
        }

        let mut components = Vec::new();
        for segment in tail.split('/').filter(|segment| !segment.is_empty()) {
            // The virtual-path layer (`ScopedPath::new`) already rejects
            // literal `..` segments before a caller-controlled path ever
            // reaches this crate, but `VirtualPath` itself does not enforce
            // that (this crate's own tests construct arbitrary
            // `VirtualPath` values), and every component below is handed
            // directly to `openat`/`mkdirat` — which *do* interpret a `..`
            // component as "go to the parent directory". Reject it here,
            // defensively, before any fd work: this is the one place a
            // literal `..` could turn into a real directory-fd escape.
            if segment == ".." {
                return Err(FilesystemError::PathOutsideMount { path: path.clone() });
            }
            if segment == "." {
                continue;
            }
            // Bound the per-resolution ancestor-fd budget before any fd is
            // opened (`MAX_PATH_COMPONENTS` — PR #6817 review follow-up).
            // Fails closed with a dedicated error, never widens or
            // truncates: see the constant's doc comment for why a fallback
            // shape is exactly what this must not do.
            if components.len() >= MAX_PATH_COMPONENTS {
                return Err(FilesystemError::PathTooDeep {
                    path: path.clone(),
                    max_components: MAX_PATH_COMPONENTS,
                });
            }
            components.push(OsString::from(segment));
        }

        Ok((
            MountTarget {
                root_fd: Arc::clone(&mount.root_fd),
                components,
                leaf_scoped: mount.leaf_scoped,
            },
            matched_virtual_root,
        ))
    }

    /// Dynamically registers a mount rooted exactly *at* `virtual_root`, if
    /// one is not already registered there — idempotent, so a repeated call
    /// for the same `virtual_root` (the usual shape: called on every request
    /// for a given scope, automatically via
    /// [`ScopedFilesystem`](crate::ScopedFilesystem)'s permission-resolution
    /// path) is a cheap no-op after the first. Unlike
    /// [`mount_local`](Self::mount_local) (boot-time only, exclusive
    /// `&mut self`), this takes `&self` so it can be called per request from
    /// behind a shared `Arc<DiskFilesystem>`. Named distinctly from the
    /// `RootFilesystem::ensure_scoped_mount` trait method (`local.rs`,
    /// one-line delegation to this) rather than sharing the name: an
    /// inherent and a trait method of the same name on the same type would
    /// still resolve correctly (inherent wins), but a distinct name here
    /// avoids relying on a reader noticing that shadowing rather than
    /// misreading the trait impl's delegating call as infinite recursion.
    ///
    /// This is the mechanism that closes a same-storage-root cross-tenant/
    /// cross-user symlink escape for a mount whose containment root is wider
    /// than the subtree a specific caller is actually granted (e.g. `/projects`
    /// mounted once over the whole local-dev storage root, while a caller's
    /// `/skills` grant only authorizes `/projects/tenants/<t>/users/<u>/skills`).
    /// The composition layer already knows that exact boundary — it is the
    /// `MountGrant::target` a scope-aware `MountView` builder computes from
    /// typed `ResourceScope` fields, not something this crate derives by
    /// counting path segments. Registering a *second*, narrower mount at that
    /// literal target makes [`resolve_mount_target`](Self::resolve_mount_target)'s
    /// existing longest-prefix-wins matching pick it over the wide mount for
    /// anything under it, so `RESOLVE_BENEATH` (or the portable fallback)
    /// enforces containment against the caller's own subtree — exactly that
    /// subtree, no more — rather than the shared parent every caller's
    /// subtree lives under.
    ///
    /// No host path is taken as input: `virtual_root` is resolved through the
    /// *existing* (necessarily wider) mount that already covers it, via the
    /// same fd-rooted `descend_creating` every other write path in this
    /// crate uses (creating the directory if this is a brand-new leaf's first
    /// access, exactly like `descend_creating`'s other callers) — never a
    /// second, independently-resolved `std::fs` path lookup. The resulting,
    /// already-open fd becomes the new mount's `root_fd` directly.
    pub(super) async fn ensure_scoped_mount_dynamic(
        &self,
        virtual_root: &VirtualPath,
    ) -> Result<(), FilesystemError> {
        // Record *before* the idempotent short-circuit and before any
        // eviction can happen: from this point on, `virtual_root` is a
        // containment boundary some caller relies on, permanently — even
        // across a later LRU eviction of the live entry below. See
        // `DiskFilesystem::narrow_scoped_roots` and
        // `resolve_mount_target`/`narrowing_lost` (PR #6817 fix).
        self.narrow_scoped_roots
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(virtual_root.as_str().to_string());

        if self.has_mount(virtual_root) {
            return Ok(());
        }
        let virtual_root = virtual_root.clone();
        // Raw routing, not `resolve_mount_target`: this call resolves
        // through the (necessarily wider) *ancestor* mount to descend into
        // `virtual_root` and open its own anchor fd — `virtual_root` itself
        // has no live mount yet, that is exactly what this function is
        // registering. Going through `resolve_mount_target` here would
        // wrongly fail closed on `virtual_root`'s own just-recorded
        // narrowing requirement.
        let (target, _matched_virtual_root) = self.resolve_mount_route(&virtual_root)?;
        let path = virtual_root.clone();
        let anchor_fd =
            super::run_blocking(path.clone(), FilesystemOperation::MountLocal, move || {
                super::fd_resolve::descend_creating(target.root_fd.as_fd(), &target.components)
                    .map(|(fd, _ancestors)| fd)
                    .map_err(|error| {
                        super::fd_resolve::resolve_error_to_filesystem_error(
                            &path,
                            FilesystemOperation::MountLocal,
                            error,
                        )
                    })
            })
            .await?;

        let mut mounts = self
            .mounts
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // Re-check under the write lock: two concurrent first-callers for
        // the same scope must not both push a mount for the same
        // `virtual_root` (that would leave two entries with the same
        // longest-prefix key — harmless for correctness since both point at
        // the same host directory, but wasteful and worth avoiding).
        if mounts
            .iter()
            .any(|mount| mount.virtual_root.as_str() == virtual_root.as_str())
        {
            return Ok(());
        }

        // Bound the dynamic-mount population (`MAX_DYNAMIC_MOUNTS`) before
        // adding one more: evict the least-recently-touched dynamic entry if
        // we are at capacity. Still under the same write-lock guard as the
        // push below, so no other caller can register a mount, race this
        // eviction, or observe a moment where capacity is exceeded.
        //
        // Safety of eviction: dropping a `LocalMount` here only drops the
        // registry's own `Arc<OwnedFd>` reference. A request already
        // in-flight against this mount captured its own clone of that `Arc`
        // in a `MountTarget` (`resolve_mount_target` does `Arc::clone`, not a
        // move) before this eviction could ever run — the write lock held
        // here cannot be held concurrently with `resolve_mount_target`'s read
        // lock, but the in-flight request's *clone* was already taken and
        // handed off to the blocking closure by an earlier, already-released
        // read-lock critical section. So eviction only ever drops the
        // registry's reference; the underlying open file description stays
        // open until every clone — including any in-flight one — is dropped,
        // per ordinary `Arc` semantics. No in-use fd is ever closed by this.
        let dynamic_count = mounts.iter().filter(|mount| mount.dynamic).count();
        if dynamic_count >= MAX_DYNAMIC_MOUNTS {
            let evict_index = mounts
                .iter()
                .enumerate()
                .filter(|(_, mount)| mount.dynamic)
                .min_by_key(|(_, mount)| mount.last_used.load(Ordering::Relaxed))
                .map(|(index, _)| index);
            if let Some(evict_index) = evict_index {
                mounts.remove(evict_index);
            }
        }

        mounts.push(LocalMount {
            virtual_root,
            root_fd: Arc::new(anchor_fd),
            leaf_scoped: false,
            dynamic: true,
            last_used: AtomicU64::new(next_touch()),
        });
        Ok(())
    }
}

fn io_reason(error: std::io::Error) -> String {
    error.kind().to_string()
}

/// Canonicalizes `host_root` and opens it `O_DIRECTORY | O_NOFOLLOW`, the
/// shared "turn a host path into a verified mount root fd" step both
/// `mount_local_impl` (boot-time, static mounts) and
/// `ensure_scoped_mount_dynamic` (per-request, dynamic mounts) need. The
/// returned canonical `PathBuf` is not retained by either caller — only the
/// fd is; this crate never re-resolves a path string against anything after
/// mount time (see the `fd_resolve` module doc).
fn open_mount_root(
    virtual_root: &VirtualPath,
    host_root: &HostPath,
) -> Result<(std::path::PathBuf, OwnedFd), FilesystemError> {
    let canonical_root =
        std::fs::canonicalize(host_root.as_path()).map_err(|error| FilesystemError::Backend {
            path: virtual_root.clone(),
            operation: FilesystemOperation::MountLocal,
            reason: io_reason(error),
        })?;

    if !canonical_root.is_dir() {
        return Err(FilesystemError::Backend {
            path: virtual_root.clone(),
            operation: FilesystemOperation::MountLocal,
            reason: "host root is not a directory".to_string(),
        });
    }

    let root_fd = rustix::fs::open(
        &canonical_root,
        OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::empty(),
    )
    .map_err(|errno| FilesystemError::Backend {
        path: virtual_root.clone(),
        operation: FilesystemOperation::MountLocal,
        reason: io_reason(errno.into()),
    })?;

    Ok((canonical_root, root_fd))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RootFilesystem;
    use tempfile::tempdir;

    /// A `mount_local_per_leaf` mount's containment boundary is the caller's
    /// own leaf (`host_root/<first-tail-segment>`), derived from the tail —
    /// there is no safe containment root for the bare mount path itself
    /// (that would mean "every caller's leaf", the exact shared-parent
    /// boundary `mount_local_per_leaf` exists to eliminate). Today every
    /// legitimate grant against such a mount always resolves to a
    /// leaf-prefixed target (`sandbox_user_workspace_mount_view` in
    /// `ironclaw_reborn_composition`), but that is an invariant enforced one
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

    /// Same containment gap as `leaf_scoped_mount_rejects_bare_mount_root_request`,
    /// reached via a `.`-only tail instead of an empty one: `tail.is_empty()`
    /// is checked *before* `.` segments are stripped out of the tail below,
    /// so `/tmp/.` produces a non-empty raw tail (`"."`) that sails past the
    /// bare-root guard and only then normalizes to zero components. Without
    /// this fix, `anchor_for_target` in `local.rs` treats that as "no leaf"
    /// and hands back the wide, shared mount root instead of failing closed
    /// — `list_dir("/tmp/.")` would enumerate every caller's leaf directory
    /// names under the shared root, exactly the boundary this mount kind
    /// exists to eliminate.
    #[tokio::test]
    async fn leaf_scoped_mount_rejects_dot_only_bare_mount_root_request() {
        let storage = tempdir().unwrap();
        let mut root = DiskFilesystem::new();
        root.mount_local_per_leaf(
            VirtualPath::new("/tmp").unwrap(),
            HostPath::from_path_buf(storage.path().to_path_buf()),
        )
        .unwrap();

        let error = root
            .list_dir(&VirtualPath::new("/tmp/.").unwrap())
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

    /// First-use path for a brand-new leaf: nothing under `host_root` exists
    /// yet for this caller, so the nearest *existing* ancestor of the target
    /// is the shared `host_root` itself, not the (not-yet-created)
    /// containment root `host_root/<leaf>`. Regression for the bug where
    /// `ensure_existing_ancestor_contained` rejected that shared root as an
    /// escape, permanently blocking every new leaf's first write.
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

    /// Same first-use bootstrap, but through `create_dir_all` rather than
    /// `write_file` — the two callers of `ensure_existing_ancestor_contained`
    /// must both accept the shared `host_root` as a bootstrap ancestor.
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
    /// not — must still be rejected. Naively treating "target doesn't
    /// resolve" as "brand new file in this leaf" would let `write_file`/
    /// `append_file` open through the symlink (the OS creates the target on
    /// `O_CREAT`), writing into whatever sibling leaf (or worse) the symlink
    /// points at. `atomic_write_file`'s pre-install probe (`open_one` with
    /// `O_NOFOLLOW`) is what catches this now: it never resolves the
    /// dangling target at all, so "does the target exist" never comes up.
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

    fn dynamic_mount_count(root: &DiskFilesystem) -> usize {
        root.mounts
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .filter(|mount| mount.dynamic)
            .count()
    }

    /// FIX 2 regression: registering more distinct scopes than
    /// `MAX_DYNAMIC_MOUNTS` must evict rather than grow unbounded — proving
    /// the fd-leak bound actually holds, not just that the constant exists.
    /// Registers `MAX_DYNAMIC_MOUNTS + 5` distinct scoped mounts (each opens
    /// its own directory fd) and asserts the live dynamic-mount population
    /// never exceeds the cap.
    #[tokio::test]
    async fn ensure_scoped_mount_caps_dynamic_mount_population() {
        let storage = tempdir().unwrap();
        let mut root = DiskFilesystem::new();
        root.mount_local(
            VirtualPath::new("/projects").unwrap(),
            HostPath::from_path_buf(storage.path().to_path_buf()),
        )
        .unwrap();

        for i in 0..(MAX_DYNAMIC_MOUNTS + 5) {
            let target = VirtualPath::new(format!("/projects/scope-{i}")).unwrap();
            root.ensure_scoped_mount_dynamic(&target)
                .await
                .unwrap_or_else(|error| panic!("ensure_scoped_mount({i}) failed: {error:?}"));
            assert!(
                dynamic_mount_count(&root) <= MAX_DYNAMIC_MOUNTS,
                "dynamic mount population exceeded MAX_DYNAMIC_MOUNTS after registering scope {i}"
            );
        }

        assert_eq!(dynamic_mount_count(&root), MAX_DYNAMIC_MOUNTS);
    }

    /// FIX 2 regression, second half: a scope evicted to make room for newer
    /// ones must still resolve correctly once its narrowing is properly
    /// re-established — eviction only drops the registry's own
    /// `Arc<OwnedFd>` clone (see `ensure_scoped_mount_dynamic`'s eviction
    /// comment), never an fd a concurrent or later request is relying on,
    /// nor the underlying host directory or its contents.
    ///
    /// Updated for the PR #6817 fix: a bare `read_file`/`write_file` call no
    /// longer transparently re-widens through the ancestor mount after
    /// eviction (see `evicted_narrow_mount_fails_closed_instead_of_reopening_cross_tenant_symlink_escape`
    /// below) — that silent widening *was* the bug. The realistic "reused"
    /// path, matching every real caller (`ScopedFilesystem` always calls
    /// `ensure_scoped_mount` immediately before each op), is to
    /// re-`ensure_scoped_mount_dynamic` the scope first; this test proves
    /// that path still transparently re-opens the evicted scope and behaves
    /// exactly like the first registration.
    #[tokio::test]
    async fn ensure_scoped_mount_evicted_scope_still_resolves_after_reregistration() {
        let storage = tempdir().unwrap();
        let mut root = DiskFilesystem::new();
        root.mount_local(
            VirtualPath::new("/projects").unwrap(),
            HostPath::from_path_buf(storage.path().to_path_buf()),
        )
        .unwrap();

        let first_scope = VirtualPath::new("/projects/scope-0").unwrap();
        root.ensure_scoped_mount_dynamic(&first_scope)
            .await
            .expect("register scope-0 first");
        root.write_file(
            &VirtualPath::new("/projects/scope-0/marker.txt").unwrap(),
            b"first-registration",
        )
        .await
        .expect("write through scope-0 before eviction");

        // Push scope-0 out: fill past the cap with fresh scopes, none of
        // which ever touch scope-0 again, so it is the oldest-by-recency
        // entry and the eviction victim.
        for i in 1..=MAX_DYNAMIC_MOUNTS {
            let target = VirtualPath::new(format!("/projects/scope-{i}")).unwrap();
            root.ensure_scoped_mount_dynamic(&target)
                .await
                .unwrap_or_else(|error| panic!("ensure_scoped_mount({i}) failed: {error:?}"));
        }
        assert_eq!(dynamic_mount_count(&root), MAX_DYNAMIC_MOUNTS);

        // A bare read against the evicted scope, without re-establishing
        // narrowing, must now fail closed (PR #6817 fix) rather than
        // silently resolving through the wide `/projects` ancestor.
        let error = root
            .read_file(&VirtualPath::new("/projects/scope-0/marker.txt").unwrap())
            .await
            .expect_err("read against an evicted, non-re-narrowed scope must fail closed");
        assert!(
            matches!(error, FilesystemError::MountNotFound { .. }),
            "expected MountNotFound after eviction with no re-narrowing, got: {error:?}"
        );

        // The realistic reuse path: re-establish narrowing, exactly as
        // `ScopedFilesystem` does before every op. scope-0's file is still
        // on disk (eviction only drops the fd *cache* entry, never the
        // underlying host directory or its contents), so re-registering
        // must transparently re-open it.
        root.ensure_scoped_mount_dynamic(&first_scope)
            .await
            .expect("re-register scope-0 after eviction");
        let bytes = root
            .read_file(&VirtualPath::new("/projects/scope-0/marker.txt").unwrap())
            .await
            .expect("scope-0 must resolve correctly once narrowing is re-established");
        assert_eq!(bytes, b"first-registration");

        root.write_file(
            &VirtualPath::new("/projects/scope-0/marker.txt").unwrap(),
            b"second-registration",
        )
        .await
        .expect("write through re-registered scope-0");
        let bytes = root
            .read_file(&VirtualPath::new("/projects/scope-0/marker.txt").unwrap())
            .await
            .unwrap();
        assert_eq!(bytes, b"second-registration");
        assert_eq!(dynamic_mount_count(&root), MAX_DYNAMIC_MOUNTS);
    }

    /// PROVEN cross-tenant escape (PR #6817), permanent regression test.
    ///
    /// `ensure_scoped_mount_dynamic` narrows containment to exactly one
    /// caller's own scope (here `/projects/scope-a`) so a symlink escaping
    /// that scope — even while staying under the wider `/projects` mount —
    /// is rejected. But that narrow mount is one of `MAX_DYNAMIC_MOUNTS`
    /// LRU-bounded entries: if it is evicted (one attacker can self-trigger
    /// this by forcing `MAX_DYNAMIC_MOUNTS` registrations) before the
    /// narrowed caller's own operation resolves, `resolve_mount_target` used
    /// to silently fall back to the wider `/projects` mount — reopening the
    /// exact escape narrowing exists to close, with no error and no signal
    /// that narrowing was lost.
    ///
    /// This exercises the *real* production trigger (an actual
    /// `MAX_DYNAMIC_MOUNTS`-entry LRU eviction), not a hand-removed mount
    /// entry.
    #[cfg(unix)]
    #[tokio::test]
    async fn evicted_narrow_mount_fails_closed_instead_of_reopening_cross_tenant_symlink_escape() {
        let storage = tempdir().unwrap();
        let host_root = storage.path();

        let scope_a = host_root.join("scope-a");
        let scope_b = host_root.join("scope-b");
        std::fs::create_dir_all(&scope_a).unwrap();
        std::fs::create_dir_all(&scope_b).unwrap();
        std::fs::write(scope_b.join("secret.txt"), b"scope-b secret").unwrap();
        std::os::unix::fs::symlink("../scope-b/secret.txt", scope_a.join("escape.txt")).unwrap();

        let mut root = DiskFilesystem::new();
        root.mount_local(
            VirtualPath::new("/projects").unwrap(),
            HostPath::from_path_buf(host_root.to_path_buf()),
        )
        .unwrap();

        let narrow_target = VirtualPath::new("/projects/scope-a").unwrap();
        let escape_path = VirtualPath::new("/projects/scope-a/escape.txt").unwrap();

        // Narrow mount live: containment is anchored at scope-a's own leaf,
        // so the symlink escaping to scope-b is rejected.
        root.ensure_scoped_mount_dynamic(&narrow_target)
            .await
            .expect("establish narrow mount for scope-a");
        let error = root.read_file(&escape_path).await.unwrap_err();
        assert!(
            matches!(error, FilesystemError::SymlinkEscape { .. }),
            "expected SymlinkEscape with the narrow mount live, got: {error:?}"
        );

        // Force scope-a's narrow mount out of the real LRU cache without
        // ever re-narrowing for it — the exact production trigger: a busy
        // multi-tenant host (or one attacker) registering
        // `MAX_DYNAMIC_MOUNTS` distinct scopes evicts an unrelated caller's
        // narrow mount mid-request.
        for i in 0..MAX_DYNAMIC_MOUNTS {
            let target = VirtualPath::new(format!("/projects/filler-{i}")).unwrap();
            root.ensure_scoped_mount_dynamic(&target)
                .await
                .unwrap_or_else(|error| {
                    panic!("ensure_scoped_mount(filler-{i}) failed: {error:?}")
                });
        }
        assert!(
            !root.has_mount(&narrow_target),
            "scope-a's narrow mount must have been evicted by the fill loop"
        );

        // The identical read, with narrowing lost and never re-established,
        // must now fail closed rather than silently falling back to the
        // wide `/projects` mount and returning scope-b's file content.
        let error = root.read_file(&escape_path).await.expect_err(
            "read after narrow-mount eviction must fail closed, not resolve through the wider mount",
        );
        assert!(
            matches!(error, FilesystemError::MountNotFound { .. }),
            "expected MountNotFound (fail-closed) after narrowing was lost, got: {error:?}"
        );
    }
}
