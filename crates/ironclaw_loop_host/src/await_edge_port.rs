//! Dependency-inversion seam for subagent await-edge delivery.
//!
//! `ironclaw_loop_host` owns `SubagentSpawnDeps` (see `subagent_spawn_port.rs`)
//! but cannot depend on `ironclaw_runner`, which owns the concrete CAS'd
//! filesystem await-edge store and resolver
//! (`crates/ironclaw_runner/src/subagent/await_edge/`). This module defines
//! the two traits that seam crosses — `AwaitEdgeWriter` (spawn-time writes,
//! consumed by `subagent_spawn_port.rs`) and `AwaitEdgeSettler` (completion-time
//! settle/resume/drain, consumed by `ironclaw_runner`'s completion path) —
//! per the design's §4.1 crate-placement ruling (permanent seam, category 2
//! of `.claude/rules/type-placement.md`, no `arch-exempt`).
//!
//! See `docs/reborn/subagent-spawn/thread-harness-design.md` §2-§5 for the
//! full CAS state machine these traits front.

use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use ironclaw_turns::{
    TurnCommittedEventObserver, TurnCoordinator, TurnError, TurnRunId, TurnScope,
};

use crate::subagent_spawn_port::AwaitedChildSetRecord;
use ironclaw_turns::run_profile::AgentLoopHostError;

/// Retryable rejection returned by [`AwaitEdgeWriter::check_scope_recovered`]
/// when a scope's boot/lazy recovery task is in flight (§5.3). Callers treat
/// this exactly like `ThreadBusy` — retry/backoff, no special-casing.
#[derive(Debug, Clone, thiserror::Error)]
#[error("subagent await-edge scope recovery in progress, retry after {retry_after_hint:?}")]
pub struct ScopeRecoveryInProgress {
    pub retry_after_hint: Duration,
}

/// Spawn-side writer seam (§3 replacement for `SubagentGateResolutionStore`).
/// Implemented in `ironclaw_runner` by `FilesystemAwaitEdgeStore` (production)
/// and here by `InMemoryAwaitEdgeWriter` (loop_host's own unit tests, no
/// filesystem/CAS semantics needed).
#[async_trait]
pub trait AwaitEdgeWriter: Send + Sync {
    /// Lazy-recovery admission gate (§5.3): called before opening a new edge
    /// for `scope`. `Err(ScopeRecoveryInProgress)` if this scope's boot/lazy
    /// recovery task is in flight. The in-memory test fixture always admits.
    async fn check_scope_recovered(
        &self,
        scope: &TurnScope,
    ) -> Result<(), ScopeRecoveryInProgress> {
        let _ = scope;
        Ok(())
    }

    /// Idempotently opens the edge (+ scope-roster touch before it, §4.5
    /// write-before-first-edge ordering) for this parent/child pair. No-ops
    /// if an edge for this exact pair is already recorded.
    async fn record_awaited_child(
        &self,
        record: AwaitedChildSetRecord,
    ) -> Result<(), AgentLoopHostError>;

    /// Rollback-only: abandon and delete the just-opened edge (§2 mode-scoped
    /// case (b) — spawn failed after the edge write; explicit teardown, not
    /// a normal terminal close).
    async fn abandon_awaited_child(
        &self,
        child_scope: &TurnScope,
        parent_run_id: TurnRunId,
        child_run_id: TurnRunId,
    ) -> Result<(), AgentLoopHostError>;
}

/// Per-child (or per-settle-group, §5.6's `gate_ref` grouping) outcome of one
/// [`AwaitEdgeSettler::on_child_terminal`] call, folded into a scope's
/// [`ResolveReport`] (§5.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolveOutcome {
    /// This settle drained its group and resumed the parent.
    Resumed,
    /// This settle completed drain/close but the parent resume was already
    /// satisfied by a prior attempt (idempotent replay, §5.2).
    Drained,
    /// The edge was abandoned (mode-scoped, §2) rather than delivered.
    Abandoned,
    /// A benign re-observation of already-closed state (`NotFound`, or
    /// `InvalidTransition` with `from` in the benign set, §5.2).
    AlreadyClosed,
    /// This child is not a subagent child (no lineage) — not an error, just
    /// a no-op observation.
    NotApplicable,
}

/// Per-scope observability counters (§5.4), accumulated across a boot pass
/// or a batch of lazy resolutions.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ResolveReport {
    pub resumed: u64,
    pub drained: u64,
    pub abandoned: u64,
    pub already_closed: u64,
    pub failed: u64,
}

impl ResolveReport {
    pub fn record(&mut self, outcome: ResolveOutcome) {
        match outcome {
            ResolveOutcome::Resumed => self.resumed += 1,
            ResolveOutcome::Drained => self.drained += 1,
            ResolveOutcome::Abandoned => self.abandoned += 1,
            ResolveOutcome::AlreadyClosed => self.already_closed += 1,
            ResolveOutcome::NotApplicable => {}
        }
    }

    pub fn record_failed(&mut self) {
        self.failed += 1;
    }
}

/// Completion-side settle seam (§3 replacement for the gate store's terminal
/// handling; implemented by `AwaitEdgeResolver` in `ironclaw_runner`). Named
/// per the design doc's §4.1/§3 explicit choice — kept distinct from the
/// crate's `*Store`/`*Writer`/`*Resolver` naming convention deliberately,
/// since the certified design doc names this exact trait `AwaitEdgeSettler`.
#[async_trait]
pub trait AwaitEdgeSettler: Send + Sync {
    /// Drives settle → (group-ready?) → write-result → resume → release →
    /// prune → delete for one child terminal event (§2, §5.2, §5.5, §8.1).
    async fn on_child_terminal(
        &self,
        event: &ironclaw_turns::TurnLifecycleEvent,
    ) -> Result<ResolveOutcome, AgentLoopHostError>;

    /// Bind the back-reference to the wrapping `TurnCoordinator` so the
    /// blocking-resume path can call back into it after a child terminates.
    /// Trait method (not left as an inherent method on the concrete
    /// resolver type) so `ironclaw_runner::runtime` can call it through
    /// `Arc<dyn AwaitEdgeSettler>` without depending on the resolver's
    /// concrete, filesystem-backend-generic type.
    fn bind_coordinator(&self, coordinator: Arc<dyn TurnCoordinator>) -> Result<(), TurnError>;

    /// Bind the capability result writer late, mirroring `bind_coordinator`'s
    /// deferred-binding pattern. Needed because some composition call sites
    /// (`ironclaw_reborn_composition::runtime`) construct the result writer
    /// *after* the await-edge resolver is assembled and erased into
    /// `Arc<dyn AwaitEdgeSettler>` (its own generic backend type parameter is
    /// no longer nameable at that point) — this lets construction and
    /// result-writer availability stay decoupled without threading the
    /// resolver's concrete, filesystem-backend-generic type further than the
    /// module that builds it.
    fn bind_result_writer(
        &self,
        result_writer: Arc<dyn crate::LoopCapabilityResultWriter>,
    ) -> Result<(), TurnError>;

    /// Adapter to the pre-existing `TurnCommittedEventObserver` seam
    /// (`ironclaw_turns::TurnLifecycleEventBus::subscribe_required` needs a
    /// `TurnCommittedEventObserver`, not this trait) — implemented as
    /// `Arc::clone(&self) as Arc<dyn TurnCommittedEventObserver>` inside the
    /// concrete type, where the concrete type is known and ordinary
    /// (non-upcasting) trait-object coercion applies.
    fn as_turn_committed_event_observer(self: Arc<Self>) -> Arc<dyn TurnCommittedEventObserver>;
}
