//! In-memory-backed approval store constructors for tests.
//!
//! The IronClaw architecture-simplification note
//! (`docs/ironclaw/2026-07-17-architecture-simplification-dto-dyn-local.md`, §4.3)
//! replaces the hand-written `InMemory*Store` parallel implementations with the
//! one production `Filesystem*Store<F>` exercised over an in-memory backend:
//! "in-memory" stops being a store and becomes a filesystem backend
//! (`InMemoryBackend`). These helpers wire that seam once — a
//! `ScopedFilesystem<InMemoryBackend>` mounted at `/approvals` (covering every
//! approval store's `/approvals/**` path prefix) — so tests instantiate the same
//! store a deployment runs.
//!
//! Gated behind `#[cfg(any(test, feature = "test-support"))]` so nothing here
//! ships in production binaries; downstream crates enable the `test-support`
//! feature from their `[dev-dependencies]`.

use std::sync::Arc;

use ironclaw_filesystem::{InMemoryBackend, ScopedFilesystem};
use ironclaw_host_api::{MountAlias, MountGrant, MountPermissions, MountView, VirtualPath};

use crate::{
    FilesystemAutoApproveSettingStore, FilesystemCapabilityPermissionOverrideStore,
    FilesystemPersistentApprovalPolicyStore,
};

/// A fresh, volatile `ScopedFilesystem<InMemoryBackend>` mounted at `/approvals`
/// — the in-memory backend seam every approval store uses in tests.
pub fn in_memory_backed_approvals_filesystem() -> Arc<ScopedFilesystem<InMemoryBackend>> {
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/approvals").expect("static valid mount alias"), // safety: test-support scaffolding, static literal
        VirtualPath::new("/engine/approvals").expect("static valid virtual path"), // safety: test-support scaffolding, static literal
        MountPermissions::read_write_list_delete(),
    )])
    .expect("static valid approvals mount view"); // safety: test-support scaffolding, static literal
    Arc::new(ScopedFilesystem::with_fixed_view(
        Arc::new(InMemoryBackend::new()),
        mounts,
    ))
}

/// The production auto-approve store over a fresh in-memory backend.
pub fn in_memory_backed_auto_approve_setting_store()
-> FilesystemAutoApproveSettingStore<InMemoryBackend> {
    FilesystemAutoApproveSettingStore::new(in_memory_backed_approvals_filesystem())
}

/// The production persistent-approval-policy store over a fresh in-memory
/// backend.
pub fn in_memory_backed_persistent_approval_policy_store()
-> FilesystemPersistentApprovalPolicyStore<InMemoryBackend> {
    FilesystemPersistentApprovalPolicyStore::new(in_memory_backed_approvals_filesystem())
}

/// The production capability-permission-override store over a fresh in-memory
/// backend.
pub fn in_memory_backed_capability_permission_override_store()
-> FilesystemCapabilityPermissionOverrideStore<InMemoryBackend> {
    FilesystemCapabilityPermissionOverrideStore::new(in_memory_backed_approvals_filesystem())
}
