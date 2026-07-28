//! Turn-run persistence-snapshot abstraction shared by every reader of live
//! turn-run state in this crate: the local-dev approval/auth interaction
//! locators (`SnapshotApprovalTurnRunLocator` in `runtime.rs`,
//! `SnapshotAuthInteractionReadModel` in `runtime/auth_interaction.rs`) and
//! the trigger poller's active-run lookup (`SnapshotActiveRunLookup` in
//! `trigger_poller/active_run_lookup.rs`).
//!
//! Exists so a `test-support` caller can substitute the turn-state store a
//! locator reads from without those locators depending on the specific
//! concrete `TurnStateRowStore<CompositeRootFilesystem>` type:
//! `RebornIntegrationGroup`'s real runs execute against its own
//! `shared.turn_store` (`TurnStateRowStore<HarnessTurnBackend>`,
//! built by `build_default_planned_runtime`) — a DIFFERENT store than the one
//! assembled by `build_runtime_substrate`. This module replaces a hardcoded
//! concrete field type with a trait-object snapshot source; every real
//! turn-state implementation remains the same `TurnStateRowStore<F>`
//! over the configured filesystem backend.
//!
//! Returns the raw `ironclaw_turns::TurnError`; each consumer maps it into
//! its own domain error at its own boundary (`ProductSurfaceFailure` for the
//! approval/auth locators, `TriggerError` for the trigger poller) rather than
//! this shared substrate trait picking a consumer's error type.

use async_trait::async_trait;
use ironclaw_turns::{TurnError, TurnPersistenceSnapshot};
use std::sync::{Arc, RwLock};

#[async_trait]
pub(crate) trait TurnRunSnapshotSource: Send + Sync {
    async fn turn_run_snapshot(&self) -> Result<TurnPersistenceSnapshot, TurnError>;
}

pub(crate) struct RebindableTurnRunSnapshotSource {
    source: Arc<RwLock<Arc<dyn TurnRunSnapshotSource>>>,
}

impl RebindableTurnRunSnapshotSource {
    pub(crate) fn new(source: Arc<RwLock<Arc<dyn TurnRunSnapshotSource>>>) -> Self {
        Self { source }
    }
}

#[async_trait]
impl TurnRunSnapshotSource for RebindableTurnRunSnapshotSource {
    async fn turn_run_snapshot(&self) -> Result<TurnPersistenceSnapshot, TurnError> {
        let source = self
            .source
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        source.turn_run_snapshot().await
    }
}

// The one turn-state store. Generic over any `RootFilesystem` backend, so the
// composition row store and a caller's own row store (for example
// `RebornIntegrationGroup`'s `TurnStateRowStore<HarnessTurnBackend>`)
// implement this identically.
#[async_trait]
impl<F> TurnRunSnapshotSource for ironclaw_turns::TurnStateRowStore<F>
where
    F: ironclaw_filesystem::RootFilesystem + Send + Sync + 'static,
{
    async fn turn_run_snapshot(&self) -> Result<TurnPersistenceSnapshot, TurnError> {
        self.persistence_snapshot().await
    }
}
