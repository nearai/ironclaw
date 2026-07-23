//! In-memory-backed budget-gate store constructor for tests.
//!
//! The Reborn architecture-simplification note
//! (`docs/reborn/2026-07-17-architecture-simplification-dto-dyn-local.md`, §4.3)
//! replaces hand-written `InMemory*Store` parallel implementations with the one
//! production `Filesystem*Store<F>` exercised over an in-memory backend:
//! "in-memory" stops being a store and becomes a filesystem backend
//! (`InMemoryBackend`). This helper wires that seam for the budget-gate store —
//! a `ScopedFilesystem<InMemoryBackend>` mounting `/resources` (the alias the
//! store persists its `/resources/budget-gates.json` snapshot under) — so tests
//! instantiate the same store a deployment runs.
//!
//! Note on isolation: [`BudgetGateStore`] serializes a single snapshot
//! file per `/resources` mount, keyed internally by `BudgetGateId`; it carries no
//! scope in the path, so tenant/user isolation lives entirely in the
//! `MountView`. The single fixed mount below therefore behaves exactly like the
//! deleted `InMemoryBudgetGateStore` did — one shared snapshot regardless of the
//! per-op `ResourceScope` — which is what the single-scope gate-lifecycle tests
//! (open/get/resolve/list/expire under one scope) need. Cross-tenant isolation is
//! exercised by the per-tenant-mount tests in `filesystem_store.rs`.
//!
//! Terminal retention is disabled ([`with_terminal_retention(None)`]) so tests
//! can inspect resolved/expired gates without a time window — matching the
//! retain-forever semantics of the deleted in-memory store. Production
//! composition keeps the default bounded retention via plain
//! [`BudgetGateStore::new`].
//!
//! Gated behind `#[cfg(any(test, feature = "test-support"))]` and disabled by
//! default. Downstream crates should enable `test-support` only from their
//! `[dev-dependencies]`.

use std::sync::Arc;

use ironclaw_filesystem::{InMemoryBackend, ScopedFilesystem};
use ironclaw_host_api::{MountAlias, MountGrant, MountPermissions, MountView, VirtualPath};

use crate::BudgetGateStore;

/// A fresh, volatile `ScopedFilesystem<InMemoryBackend>` mounting `/resources` —
/// the in-memory backend seam the budget-gate store persists under.
pub fn in_memory_backed_resources_filesystem() -> Arc<ScopedFilesystem<InMemoryBackend>> {
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/resources").expect("static valid mount alias"), // safety: test-support scaffolding, static literal
        VirtualPath::new("/engine/resources").expect("static valid virtual path"), // safety: test-support scaffolding, static literal
        MountPermissions::read_write_list_delete(),
    )])
    .expect("static valid resources mount view"); // safety: test-support scaffolding, static literal
    Arc::new(ScopedFilesystem::with_fixed_view(
        Arc::new(InMemoryBackend::new()),
        mounts,
    ))
}

/// The production budget-gate store over a fresh in-memory backend — the drop-in
/// replacement for the deleted `InMemoryBudgetGateStore`. Terminal retention is
/// disabled so tests can inspect resolved/expired gates.
pub fn in_memory_backed_budget_gate_store() -> BudgetGateStore<InMemoryBackend> {
    BudgetGateStore::new(in_memory_backed_resources_filesystem()).with_terminal_retention(None)
}
