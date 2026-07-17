//! In-memory-backed process store constructors for tests.
//!
//! The Reborn architecture-simplification note
//! (`docs/reborn/2026-07-17-architecture-simplification-dto-dyn-local.md`, §4.3)
//! replaces the hand-written `InMemory*Store` parallel implementations with the
//! one production `Filesystem*Store<F>` exercised over an in-memory backend:
//! "in-memory" stops being a store and becomes a filesystem backend
//! (`InMemoryBackend`). These helpers wire that seam once — a
//! `ScopedFilesystem<InMemoryBackend>` mounted at `/processes` — so tests
//! instantiate the same store a deployment runs.
//!
//! Note on sub-scope isolation: `FilesystemProcessStore` encodes
//! agent/project/mission/thread in the path (structural under any mount), while
//! tenant/user isolation lives in the `MountView`. The single fixed mount below
//! therefore isolates by agent/project/mission/thread but not by tenant/user —
//! which matches single-tenant state-machine tests; cross-tenant isolation is
//! exercised by the `filesystem_process_store_isolates_two_tenants_*` tests,
//! which mount per tenant/user.
//!
//! Gated behind `#[cfg(any(test, feature = "test-support"))]` so nothing here
//! ships in production binaries; downstream crates enable the `test-support`
//! feature from their `[dev-dependencies]`.

use std::sync::Arc;

use ironclaw_filesystem::{InMemoryBackend, ScopedFilesystem};
use ironclaw_host_api::{MountAlias, MountGrant, MountPermissions, MountView, VirtualPath};

use crate::{FilesystemProcessResultStore, FilesystemProcessStore, ProcessServices};

/// A fresh, volatile `ScopedFilesystem<InMemoryBackend>` mounted at `/processes`
/// — the in-memory backend seam every process store uses in tests.
pub fn in_memory_backed_processes_filesystem() -> Arc<ScopedFilesystem<InMemoryBackend>> {
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/processes").expect("static valid mount alias"), // safety: test-support scaffolding, static literal
        VirtualPath::new("/engine/processes").expect("static valid virtual path"), // safety: test-support scaffolding, static literal
        MountPermissions::read_write_list_delete(),
    )])
    .expect("static valid processes mount view"); // safety: test-support scaffolding, static literal
    Arc::new(ScopedFilesystem::with_fixed_view(
        Arc::new(InMemoryBackend::new()),
        mounts,
    ))
}

/// The production process store over a fresh in-memory backend — the drop-in
/// replacement for the deleted `InMemoryProcessStore`.
pub fn in_memory_backed_process_store() -> FilesystemProcessStore<InMemoryBackend> {
    FilesystemProcessStore::new(in_memory_backed_processes_filesystem())
}

/// The production process result store over a fresh in-memory backend — the
/// drop-in replacement for the deleted `InMemoryProcessResultStore`.
pub fn in_memory_backed_process_result_store() -> FilesystemProcessResultStore<InMemoryBackend> {
    FilesystemProcessResultStore::new(in_memory_backed_processes_filesystem())
}

/// A [`ProcessServices`] whose lifecycle and result stores share **one**
/// in-memory-backed `/processes` filesystem — the drop-in replacement for the
/// deleted `ProcessServices::in_memory()`. Use this (not the two standalone
/// helpers) when a test starts a process and reads back its result, since both
/// stores must resolve against the same backend.
pub fn in_memory_backed_process_services() -> ProcessServices<
    FilesystemProcessStore<InMemoryBackend>,
    FilesystemProcessResultStore<InMemoryBackend>,
> {
    ProcessServices::filesystem(in_memory_backed_processes_filesystem())
}
