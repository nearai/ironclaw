//! Test-support constructors for this crate's filesystem-backed stores.
//!
//! Gated behind `#[cfg(any(test, feature = "test-support"))]`: disabled by
//! default; downstream crates enable it only from `[dev-dependencies]`
//! (`ironclaw_loop_host = { …, features = ["test-support"] }`), mirroring the
//! `ironclaw_run_state` test-support pattern. Centralizes the
//! `FilesystemCheckpointStateStore<InMemoryBackend>` seam that was previously
//! copy-pasted across a dozen test modules (§4.3: exercise the one production
//! store over a volatile backend — never a hand-written `InMemory*Store`).

use std::sync::Arc;

use ironclaw_filesystem::{InMemoryBackend, ScopedFilesystem};
use ironclaw_host_api::{MountAlias, MountGrant, MountPermissions, MountView, VirtualPath};

use crate::FilesystemCheckpointStateStore;

/// The production checkpoint-state store over a fresh, volatile
/// `InMemoryBackend`, mounted at the same `/checkpoint-state` alias production
/// composition uses.
pub fn in_memory_backed_checkpoint_state_store()
-> Arc<FilesystemCheckpointStateStore<InMemoryBackend>> {
    let mounts = MountView::new(vec![MountGrant::new(
        MountAlias::new("/checkpoint-state").expect("static valid mount alias"), // safety: test-support scaffolding, static literal
        VirtualPath::new("/checkpoint-state").expect("static valid virtual path"), // safety: test-support scaffolding, static literal
        MountPermissions::read_write_list_delete(),
    )])
    .expect("static valid checkpoint-state mount view"); // safety: test-support scaffolding, static literal
    Arc::new(FilesystemCheckpointStateStore::new(Arc::new(
        ScopedFilesystem::with_fixed_view(Arc::new(InMemoryBackend::new()), mounts),
    )))
}
