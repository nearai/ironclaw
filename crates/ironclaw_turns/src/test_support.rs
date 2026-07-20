//! Shared test double for turn-state storage.
//!
//! [`in_memory_turn_state_store`] is the single, workspace-wide replacement for
//! the former public in-memory turn-state store: a [`FilesystemTurnStateRowStore`]
//! over a volatile [`InMemoryBackend`] at the default
//! [`WriteThrough`](crate::TurnStateDurabilityPolicy::WriteThrough) durability.
//! Because every transition commits synchronously under `WriteThrough`, its
//! read-after-write and reopen-durability semantics are identical to the old
//! in-memory authority — the same engine (`TurnStateEngine`) runs inside it.
//!
//! Keeping every test on this one constructor means a later step can flip the
//! whole fleet's durability policy in exactly one place.

use std::sync::Arc;

use ironclaw_filesystem::{InMemoryBackend, ScopedFilesystem};
use ironclaw_host_api::{MountAlias, MountGrant, MountPermissions, MountView, VirtualPath};

use crate::FilesystemTurnStateRowStore;

/// Build the volatile, process-local turn-state store double.
///
/// A fresh [`InMemoryBackend`] per call, so distinct stores are isolated. To
/// exercise reopen / rehydration, build a shared backend with
/// [`in_memory_turns_filesystem`] and open two stores over it.
pub fn in_memory_turn_state_store() -> FilesystemTurnStateRowStore<InMemoryBackend> {
    // The lenient (default) mode — the same shape production composition wires
    // via `FilesystemTurnStateStoreKind::row`.
    FilesystemTurnStateRowStore::new(in_memory_turns_filesystem())
}

/// A fresh scoped `/turns` filesystem over a volatile [`InMemoryBackend`].
///
/// Reuse the returned handle to open more than one
/// [`FilesystemTurnStateRowStore`] over the *same durable bytes* — the
/// canonical way to cover restart / rehydration now that the row store
/// rehydrates from durable rows rather than a handed-in snapshot.
pub fn in_memory_turns_filesystem() -> Arc<ScopedFilesystem<InMemoryBackend>> {
    scoped_turns_filesystem(Arc::new(InMemoryBackend::new()))
}

/// Wrap a specific [`InMemoryBackend`] in the scoped `/turns` mount the row
/// store expects. Handy when a test needs to hold the backend itself.
pub fn scoped_turns_filesystem(
    backend: Arc<InMemoryBackend>,
) -> Arc<ScopedFilesystem<InMemoryBackend>> {
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/turns").expect("turns alias"),
        VirtualPath::new("/turns").expect("turns target"),
        MountPermissions::read_write_list_delete(),
    )])
    .expect("turns mount view");
    Arc::new(ScopedFilesystem::with_fixed_view(backend, mounts))
}
