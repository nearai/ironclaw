//! Local trigger-fire access store.
//!
//! This is a Reborn-owned bootstrap access store, separate from the WebChat
//! identity store. It owns only the local access records used to satisfy the
//! fire-time trigger authorization contract for local/operator-managed
//! deployments. Backends may persist those records through the host filesystem
//! abstraction or through the legacy local-dev libSQL sidecar.
//!
//! These records are not the general agent/project membership source of truth.
//! Multi-tenant runtimes must wire a real membership-backed trigger access
//! checker instead of this bootstrap store.

#[cfg(feature = "filesystem-local-trigger-access")]
mod filesystem;
#[cfg(feature = "webui-user-store")]
mod libsql;
mod types;

#[cfg(feature = "filesystem-local-trigger-access")]
pub use filesystem::RebornFilesystemLocalTriggerAccessStore;
#[cfg(feature = "webui-user-store")]
pub use libsql::RebornLibSqlLocalTriggerAccessStore;
pub use types::{
    LocalTriggerAccessReconciliation, LocalTriggerAccessRole, LocalTriggerAccessSeed,
    LocalTriggerAccessSource, LocalTriggerAccessStore, RebornLocalTriggerAccessStoreError,
};
