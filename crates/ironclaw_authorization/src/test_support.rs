//! In-memory-backed capability-lease store constructor for tests.
//!
//! The Reborn architecture-simplification note
//! (`docs/reborn/2026-07-17-architecture-simplification-dto-dyn-local.md`, §4.3)
//! replaces the hand-written `InMemory*Store` parallel implementations with the
//! one production `Filesystem*Store<F>` exercised over an in-memory backend:
//! "in-memory" stops being a store and becomes a filesystem backend
//! (`InMemoryBackend`). This helper wires that seam once — a
//! `ScopedFilesystem<InMemoryBackend>` mounted at `/authorization` (covering the
//! lease store's `/authorization/**` path prefix) — so tests instantiate the same
//! store a deployment runs.
//!
//! Gated behind `#[cfg(any(test, feature = "test-support"))]` so nothing here
//! ships in production binaries; downstream crates enable the `test-support`
//! feature from their `[dev-dependencies]`.

use std::sync::Arc;

use ironclaw_filesystem::{InMemoryBackend, ScopedFilesystem};
use ironclaw_host_api::{MountAlias, MountGrant, MountPermissions, MountView, VirtualPath};

use crate::CapabilityLeaseStore;

/// A fresh, volatile `ScopedFilesystem<InMemoryBackend>` mounted at
/// `/authorization` — the in-memory backend seam the lease store uses in tests.
pub fn in_memory_backed_authorization_filesystem() -> Arc<ScopedFilesystem<InMemoryBackend>> {
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/authorization").expect("static valid mount alias"), // safety: test-support scaffolding, static literal
        VirtualPath::new("/engine/authorization").expect("static valid virtual path"), // safety: test-support scaffolding, static literal
        MountPermissions::read_write_list_delete(),
    )])
    .expect("static valid authorization mount view"); // safety: test-support scaffolding, static literal
    Arc::new(ScopedFilesystem::with_fixed_view(
        Arc::new(InMemoryBackend::new()),
        mounts,
    ))
}

/// The production capability-lease store over a fresh in-memory backend — the
/// drop-in replacement for the deleted `InMemoryCapabilityLeaseStore`.
pub fn in_memory_backed_capability_lease_store() -> CapabilityLeaseStore<InMemoryBackend> {
    CapabilityLeaseStore::new(in_memory_backed_authorization_filesystem())
}
