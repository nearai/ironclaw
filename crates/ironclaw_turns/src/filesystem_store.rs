//! Filesystem-backed turn-state persistence under the `/turns` mount alias.
//!
//! The one production turn-state store is
//! [`FilesystemTurnStateRowStore`](row_store::FilesystemTurnStateRowStore): it
//! persists an append-only delta journal materialized into per-record row blobs
//! under the `/turns` mount alias, backed by a bounded in-memory hot cache.
//! High-churn runner-lease heartbeats are memory-backed and overlaid onto the
//! durable rows while the process is alive.
//!
//! Tenant/user isolation is structural: the
//! [`MountView`](ironclaw_host_api::MountView) the composition layer hands the
//! [`ScopedFilesystem`] resolves alias-relative paths to a tenant/user-scoped
//! [`VirtualPath`](ironclaw_host_api::VirtualPath) before any backend dispatch,
//! so tenant isolation is not something this crate re-derives from
//! `TurnScope.tenant_id`. Within-tenant axes (agent/project/thread) stay in the
//! persisted records via `TurnScope`.
//!
//! [`FilesystemTurnStateBlockPersistence`] is the legacy single-blob durable
//! sink (`/turns/state.json`). It is retained only so the row store's
//! first-boot importer can migrate a pre-existing `inmemory-turn-state`
//! deployment's gate-parked snapshot: the sink and the row store's legacy-blob
//! importer share [`io::snapshot_path`] + [`io::snapshot_entry`], so the
//! migration is automatic.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_filesystem::{CasExpectation, RootFilesystem, ScopedFilesystem};
use ironclaw_host_api::ResourceScope;

use crate::{TurnError, TurnPersistenceSnapshot};

mod io;
mod profile_resolver;
mod projection;
mod row_store;
mod runner_lease;
pub(crate) mod turn_state_engine;

use io::{deserialize_snapshot, fs_error, snapshot_entry, snapshot_path};

pub use row_store::FilesystemTurnStateRowStore;
pub use turn_state_engine::TurnStateStoreLimits;

/// Legacy filesystem-backed durable sink for a full [`TurnPersistenceSnapshot`].
///
/// Writes the snapshot to the `/turns/state.json` alias-relative path. This is
/// the single-blob layout a pre-`#6263` `inmemory-turn-state` deployment used
/// to persist its gate-parked (approval/auth) turns. It is retained so the row
/// store's first-boot legacy-blob importer has a durable artifact to migrate;
/// no production path writes new blobs through this sink.
///
/// On process start, composition calls [`load`](Self::load) once to read the
/// last persisted snapshot for startup rehydration.
pub struct FilesystemTurnStateBlockPersistence<F>
where
    F: RootFilesystem,
{
    filesystem: Arc<ScopedFilesystem<F>>,
}

impl<F> FilesystemTurnStateBlockPersistence<F>
where
    F: RootFilesystem,
{
    /// Build a sink over the same scoped filesystem the store persists to.
    pub fn new(filesystem: Arc<ScopedFilesystem<F>>) -> Self {
        Self { filesystem }
    }

    /// Load the last persisted snapshot for startup rehydration.
    ///
    /// Returns an empty snapshot when nothing has been persisted yet (fresh
    /// tenant/user, or a store that never blocked a run).
    pub async fn load(&self) -> Result<TurnPersistenceSnapshot, TurnError> {
        let path = snapshot_path()?;
        match self.filesystem.get(&ResourceScope::system(), &path).await {
            Ok(Some(versioned)) => deserialize_snapshot(&versioned.entry.body),
            Ok(None) => Ok(TurnPersistenceSnapshot::default()),
            Err(error) => Err(fs_error(error)),
        }
    }
}

#[async_trait]
impl<F> crate::TurnStateBlockPersistence for FilesystemTurnStateBlockPersistence<F>
where
    F: RootFilesystem,
{
    async fn persist(&self, snapshot: &TurnPersistenceSnapshot) {
        // Best-effort by contract: a durable-write failure must never fail an
        // already-applied in-memory transition, so log and swallow. The
        // in-memory store remains authoritative; this snapshot only backs
        // restart recovery of gate-blocked turns.
        let path = match snapshot_path() {
            Ok(path) => path,
            Err(error) => {
                tracing::debug!(%error, "turn-state block persistence: invalid snapshot path");
                return;
            }
        };
        let entry = match snapshot_entry(snapshot) {
            Ok(entry) => entry,
            Err(error) => {
                tracing::debug!(%error, "turn-state block persistence: snapshot serialization failed");
                return;
            }
        };
        // Blind overwrite: the in-memory authority owns the truth, orders writes
        // by a monotonic sequence, and hands us the complete latest snapshot, so
        // last-writer-wins is correct here — there is no cross-process snapshot to
        // lose. A plain `put` with `CasExpectation::Any` (rather than the store's
        // read-modify-write `cas_update`) keeps this a single write off the hot
        // path.
        let scope = ResourceScope::system();
        if let Err(error) = self
            .filesystem
            .put(&scope, &path, entry, CasExpectation::Any)
            .await
        {
            tracing::debug!(%error, "turn-state block persistence: durable write failed");
        }
    }
}
