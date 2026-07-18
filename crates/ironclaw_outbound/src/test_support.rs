//! In-memory-backed outbound-state store constructor for tests.
//!
//! The Reborn architecture-simplification note
//! (`docs/reborn/2026-07-17-architecture-simplification-dto-dyn-local.md`, §4.3)
//! replaces hand-written `InMemory*Store` parallel implementations with the one
//! production `Filesystem*Store<F>` exercised over an in-memory backend:
//! "in-memory" stops being a store and becomes a filesystem backend
//! (`InMemoryBackend`). This helper wires that seam for the outbound-state store
//! — a `ScopedFilesystem<InMemoryBackend>` mounting `/outbound` (the alias the
//! store persists all four role subtrees under: `policies/`, `subscriptions/`,
//! `deliveries/`, `communication-preferences/`, `delivered-gate-routes/`) — so
//! tests instantiate the same store a deployment runs.
//!
//! Note on isolation: [`FilesystemOutboundStateStore`] encodes the record scope
//! (thread-scope key, hashed communication-preference key, subscription/delivery
//! key) in the path, and tenant/user isolation lives in the `MountView`. The
//! single fixed mount below therefore isolates by those in-path scope keys but
//! not by tenant/user — which matches the single-tenant test doubles the deleted
//! `InMemoryOutboundStateStore` served. Cross-tenant isolation is exercised by
//! the per-tenant-mount tests in `tests/outbound_state_store_contract.rs`.
//!
//! Unlike the deleted `InMemoryOutboundStateStore` (which implemented only
//! `CommunicationPreferenceRepository` + `OutboundStateStore`),
//! `FilesystemOutboundStateStore` implements all four outbound-store traits, so
//! a single instance from this helper can back every role — the same
//! consolidation the durable (libsql/postgres) composition already relies on.
//!
//! Gated behind `#[cfg(any(test, feature = "test-support"))]` so nothing here
//! ships in production binaries; downstream crates enable the `test-support`
//! feature from their `[dev-dependencies]`.

use std::sync::Arc;

use ironclaw_filesystem::{InMemoryBackend, ScopedFilesystem};
use ironclaw_host_api::{MountAlias, MountGrant, MountPermissions, MountView, VirtualPath};

use crate::FilesystemOutboundStateStore;

/// A fresh, volatile `ScopedFilesystem<InMemoryBackend>` mounting `/outbound` —
/// the in-memory backend seam the outbound-state store persists under.
pub fn in_memory_backed_outbound_filesystem() -> Arc<ScopedFilesystem<InMemoryBackend>> {
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/outbound").expect("static valid mount alias"), // safety: test-support scaffolding, static literal
        VirtualPath::new("/engine/outbound").expect("static valid virtual path"), // safety: test-support scaffolding, static literal
        MountPermissions::read_write_list_delete(),
    )])
    .expect("static valid outbound mount view"); // safety: test-support scaffolding, static literal
    Arc::new(ScopedFilesystem::with_fixed_view(
        Arc::new(InMemoryBackend::new()),
        mounts,
    ))
}

/// The production outbound-state store over a fresh in-memory backend — the
/// drop-in replacement for the deleted `InMemoryOutboundStateStore`. A single
/// instance implements all four outbound-store traits.
pub fn in_memory_backed_outbound_state_store() -> FilesystemOutboundStateStore<InMemoryBackend> {
    // The `disallowed_methods` lint reserves `FilesystemOutboundStateStore::new`
    // for composition's owned construction site; this test-support constructor
    // is the sanctioned volatile-backend seam for tests (arch-simplification §4.3).
    #[allow(clippy::disallowed_methods)]
    FilesystemOutboundStateStore::new(in_memory_backed_outbound_filesystem())
}
