//! Shared test double for turn-state storage.
//!
//! [`in_memory_turn_state_store`] is the single, workspace-wide replacement for
//! the former public in-memory turn-state store: a [`TurnStateRowStore`]
//! over a volatile [`InMemoryBackend`], at the store's single write-behind
//! durability mode (#6263 Step 5b — there is no longer a durability-mode
//! choice). Gate-park, terminal, and new-run transitions are still
//! synchronously durable (they are recoverability-critical, see
//! [`crate::is_recoverability_critical`]); other transitions commit at memory
//! speed and flush asynchronously. A test that reopens a fresh store instance
//! over the same backend (to exercise restart/rehydration) must drain the
//! prior instance first — drop it after its last critical op, or call its
//! `drain()` — so a pending non-critical async tail is not silently lost
//! before the reopen reads.

use std::sync::Arc;

use ironclaw_filesystem::{InMemoryBackend, RootFilesystem, ScopedFilesystem};
use ironclaw_host_api::{MountAlias, MountGrant, MountPermissions, MountView, VirtualPath};

use crate::TurnStateRowStore;

/// Build the volatile, process-local turn-state store double.
///
/// A fresh [`InMemoryBackend`] per call, so distinct stores are isolated. To
/// exercise reopen / rehydration, build a shared backend with
/// [`in_memory_turns_filesystem`] and open two stores over it.
pub fn in_memory_turn_state_store() -> TurnStateRowStore<InMemoryBackend> {
    // The lenient (default) mode — the same shape production composition wires
    // via `TurnStateRowStore::new`.
    TurnStateRowStore::new(in_memory_turns_filesystem())
}

/// A fresh scoped `/turns` filesystem over a volatile [`InMemoryBackend`].
///
/// Reuse the returned handle to open more than one
/// [`TurnStateRowStore`] over the *same durable bytes* — the
/// canonical way to cover restart / rehydration now that the row store
/// rehydrates from durable rows rather than a handed-in snapshot.
pub fn in_memory_turns_filesystem() -> Arc<ScopedFilesystem<InMemoryBackend>> {
    scoped_turns_filesystem(Arc::new(InMemoryBackend::new()))
}

/// Wrap a specific backend in the scoped `/turns` mount the row store expects.
/// Handy when a test needs to hold the backend itself — e.g. a bare
/// [`InMemoryBackend`], or an
/// [`ironclaw_filesystem::FaultInjecting`]`<InMemoryBackend>` so a store fault
/// path runs through the real store's `FilesystemError -> TurnError` mapping.
pub fn scoped_turns_filesystem<F: RootFilesystem>(backend: Arc<F>) -> Arc<ScopedFilesystem<F>> {
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/turns").expect("turns alias"),
        VirtualPath::new("/turns").expect("turns target"),
        MountPermissions::read_write_list_delete(),
    )])
    .expect("turns mount view");
    Arc::new(ScopedFilesystem::with_fixed_view(backend, mounts))
}
