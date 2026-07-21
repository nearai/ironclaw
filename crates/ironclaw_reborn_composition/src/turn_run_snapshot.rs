//! Turn-run persistence-snapshot abstraction shared by every reader of live
//! turn-run state in this crate: the local-dev approval/auth interaction
//! locators (`SnapshotApprovalTurnRunLocator` in `runtime.rs`,
//! `SnapshotAuthInteractionReadModel` in `runtime/auth_interaction.rs`) and
//! the trigger poller's active-run lookup (`SnapshotActiveRunLookup` in
//! `trigger_poller/active_run_lookup.rs`).
//!
//! Exists so a `test-support` caller can substitute the turn-state store a
//! locator reads from without those locators depending on the specific
//! concrete `ComposedTurnStateStore` type: `RebornIntegrationGroup`'s
//! real runs execute against its own `shared.turn_store`
//! (`FilesystemTurnStateRowStore<HarnessTurnBackend>`, built by
//! `build_default_planned_runtime`) — a DIFFERENT store than
//! `RebornServices.local_runtime.turn_state`, which is this crate's own
//! `build_reborn_services` composition. Production wiring is unaffected:
//! `build_reborn_runtime` still passes `Arc::clone(&local_runtime.turn_state)`
//! as the source, which implements this trait via the blanket impl below, so
//! its snapshot behavior is byte-identical to before this seam existed — this
//! module only replaces a hardcoded field type with a trait-object one.
//!
//! Returns the raw `ironclaw_turns::TurnError`; each consumer maps it into
//! its own domain error at its own boundary (`ProductWorkflowError` for the
//! approval/auth locators, `TriggerError` for the trigger poller) rather than
//! this shared substrate trait picking a consumer's error type.

use async_trait::async_trait;
use ironclaw_turns::{TurnError, TurnPersistenceSnapshot};

#[async_trait]
pub(crate) trait TurnRunSnapshotSource: Send + Sync {
    async fn turn_run_snapshot(&self) -> Result<TurnPersistenceSnapshot, TurnError>;
}

// The one production turn-state store. Generic over any `RootFilesystem`
// backend so both `ComposedTurnStateStore` (production/local-dev, which
// resolves to `FilesystemTurnStateRowStore<CompositeRootFilesystem>` or
// `<InMemoryBackend>`) and a caller's own store (e.g.
// `RebornIntegrationGroup`'s `FilesystemTurnStateRowStore<HarnessTurnBackend>`)
// implement this identically.
#[async_trait]
impl<F> TurnRunSnapshotSource for ironclaw_turns::FilesystemTurnStateRowStore<F>
where
    F: ironclaw_filesystem::RootFilesystem + Send + Sync + 'static,
{
    async fn turn_run_snapshot(&self) -> Result<TurnPersistenceSnapshot, TurnError> {
        self.persistence_snapshot().await
    }
}
