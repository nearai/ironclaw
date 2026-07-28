//! Durable persistence for blocked turns under the in-memory turn-state
//! authority.
//!
//! The in-memory authority (the `TurnStateEngine` embedded in the row store)
//! removes the per-user
//! `state.json` CAS livelock by coordinating turns in one process, but it is
//! otherwise volatile: a process restart drops all runs. That is acceptable for
//! in-flight compute (short-lived, re-triggerable), but **not** for a turn
//! parked on a human gate (approval/auth) — a deploy would silently drop it and
//! the user's "Approve" would land on nothing.
//!
//! `TurnStateBlockPersistence` closes exactly that gap **off the hot path**: the
//! store persists the (small, low-frequency) snapshot only when the set of
//! gate-blocked runs changes — a run blocks on a gate, or such a run resumes /
//! terminates. Normal turn traffic (claim → complete) never touches the sink, so
//! it does not reintroduce the contention the in-memory authority removed.

use async_trait::async_trait;

use crate::TurnPersistenceSnapshot;

/// Durable sink invoked when the set of gate-blocked runs changes.
///
/// Best-effort by contract: implementations log and swallow their own errors so
/// a durable-write failure never fails an already-applied in-memory transition.
/// On process start, composition rehydrates the store from the last persisted
/// snapshot via `TurnStateEngine::from_persistence_snapshot`, so only
/// blocked/terminal runs need survive — recovering a gate-blocked turn is the
/// whole point.
#[async_trait]
pub trait TurnStateBlockPersistence: Send + Sync {
    /// Persist the current turn-state snapshot. Called only on blocked-set
    /// changes, never on the normal claim/complete hot path.
    async fn persist(&self, snapshot: &TurnPersistenceSnapshot);
}
