//! In-memory-backed run-state / approval-request store constructors for tests.
//!
//! The Reborn architecture-simplification note
//! (`docs/reborn/2026-07-17-architecture-simplification-dto-dyn-local.md`, §4.3)
//! replaces the hand-written `InMemory*Store` parallel implementations with the
//! one production `Filesystem*Store<F>` exercised over an in-memory backend:
//! "in-memory" stops being a store and becomes a filesystem backend
//! (`InMemoryBackend`). These helpers wire that seam once — a
//! `ScopedFilesystem<InMemoryBackend>` mounting both `/run-state` and
//! `/approvals` (the two aliases this crate persists under) — so tests
//! instantiate the same store a deployment runs.
//!
//! Note on isolation: the run-state/approval stores encode
//! agent/project/mission/thread in the path (structural under any mount) while
//! tenant/user isolation lives in the `MountView`. The single fixed mount below
//! therefore isolates by agent/project/mission/thread but not by tenant/user —
//! which matches single-tenant state-machine tests; cross-tenant isolation is
//! exercised by the per-tenant-mount tests in the contract suites.
//!
//! Run-state and approval records live under sibling aliases on **one** backend,
//! so a single `in_memory_backed_run_state_filesystem()` feeds both stores — an
//! approval resolution that reads the blocked run and its approval record sees a
//! consistent view.
//!
//! Gated behind `#[cfg(any(test, feature = "test-support"))]` and disabled by
//! default. Downstream crates should enable `test-support` only from their
//! `[dev-dependencies]`.

use std::sync::Arc;

use ironclaw_filesystem::{InMemoryBackend, ScopedFilesystem};
use ironclaw_host_api::{MountAlias, MountGrant, MountPermissions, MountView, VirtualPath};

use crate::{FilesystemApprovalRequestStore, FilesystemRunStateStore};

/// A fresh, volatile `ScopedFilesystem<InMemoryBackend>` mounting both
/// `/run-state` and `/approvals` — the in-memory backend seam the run-state and
/// approval-request stores share in tests.
pub fn in_memory_backed_run_state_filesystem() -> Arc<ScopedFilesystem<InMemoryBackend>> {
    let mounts = MountView::new(vec![
        MountGrant::new(
            MountAlias::new("/run-state").expect("static valid mount alias"), // safety: test-support scaffolding, static literal
            VirtualPath::new("/engine/run-state").expect("static valid virtual path"), // safety: test-support scaffolding, static literal
            MountPermissions::read_write_list_delete(),
        ),
        MountGrant::new(
            MountAlias::new("/approvals").expect("static valid mount alias"), // safety: test-support scaffolding, static literal
            VirtualPath::new("/engine/approvals").expect("static valid virtual path"), // safety: test-support scaffolding, static literal
            MountPermissions::read_write_list_delete(),
        ),
    ])
    .expect("static valid run-state mount view"); // safety: test-support scaffolding, static literal
    Arc::new(ScopedFilesystem::with_fixed_view(
        Arc::new(InMemoryBackend::new()),
        mounts,
    ))
}

/// The production run-state store over a fresh in-memory backend — the drop-in
/// replacement for the deleted `InMemoryRunStateStore`.
pub fn in_memory_backed_run_state_store() -> FilesystemRunStateStore<InMemoryBackend> {
    FilesystemRunStateStore::new(in_memory_backed_run_state_filesystem())
}

/// The production approval-request store over a fresh in-memory backend — the
/// drop-in replacement for the deleted `InMemoryApprovalRequestStore`.
pub fn in_memory_backed_approval_request_store() -> FilesystemApprovalRequestStore<InMemoryBackend>
{
    FilesystemApprovalRequestStore::new(in_memory_backed_run_state_filesystem())
}
