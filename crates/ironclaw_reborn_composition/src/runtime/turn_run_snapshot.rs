//! Turn-run persistence-snapshot abstraction shared by the local-dev
//! approval/auth interaction locators (`LocalDevApprovalTurnRunLocator` in
//! `runtime.rs`, `LocalDevAuthInteractionReadModel` in
//! `runtime/auth_interaction.rs`).
//!
//! Exists so a `test-support` caller can substitute the turn-state store a
//! locator reads from without those locators depending on the specific
//! concrete `LocalDevTurnStateStore` type: `RebornIntegrationGroup`'s
//! real runs execute against its own `shared.turn_store`
//! (`FilesystemTurnStateStore<HarnessTurnBackend>`, built by
//! `build_default_planned_runtime`) — a DIFFERENT store than
//! `RebornServices.local_runtime.turn_state`, which is this crate's own
//! `build_reborn_services` composition. Production wiring is unaffected:
//! `build_reborn_runtime` still passes `Arc::clone(&local_runtime.turn_state)`
//! as the source, which implements this trait via the blanket impls below, so
//! its snapshot behavior is byte-identical to before this seam existed — this
//! module only replaces a hardcoded field type with a trait-object one.

use async_trait::async_trait;
use ironclaw_turns::{TurnError, TurnPersistenceSnapshot};

#[async_trait]
pub(crate) trait TurnRunSnapshotSource: Send + Sync {
    async fn turn_run_snapshot(&self) -> Result<TurnPersistenceSnapshot, TurnError>;
}

// Durable filesystem store: async fallible snapshot. Generic over any
// `RootFilesystem` backend so both `LocalDevTurnStateStore` (production/
// local-dev, when it resolves to `FilesystemTurnStateStore<LocalDevRootFilesystem>`)
// and a caller's own store (e.g. `RebornIntegrationGroup`'s
// `FilesystemTurnStateStore<HarnessTurnBackend>`) implement this identically.
// Unconditional (not cfg-gated on which backend `LocalDevTurnStateStore`
// happens to alias to in this build): `FilesystemTurnStateStore::persistence_snapshot`
// is always defined, and this impl targets a different concrete type per `F`
// than the `InMemoryTurnStateStore` impl below, so the two never conflict.
#[async_trait]
impl<F> TurnRunSnapshotSource for ironclaw_turns::FilesystemTurnStateStore<F>
where
    F: ironclaw_filesystem::RootFilesystem + Send + Sync + 'static,
{
    async fn turn_run_snapshot(&self) -> Result<TurnPersistenceSnapshot, TurnError> {
        self.persistence_snapshot().await
    }
}

// In-memory authority: sync infallible snapshot. Also unconditional, for the
// same reason as the impl above.
#[async_trait]
impl TurnRunSnapshotSource for ironclaw_turns::InMemoryTurnStateStore {
    async fn turn_run_snapshot(&self) -> Result<TurnPersistenceSnapshot, TurnError> {
        Ok(self.persistence_snapshot())
    }
}
