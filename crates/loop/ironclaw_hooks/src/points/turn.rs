//! Context for the `after_turn` hook point.
//!
//! `AfterTurn` fires once after a turn's run reaches a terminal state. It is
//! the seam for work about the turn as a whole — the whole run has already
//! finished, so nothing a hook does here can affect the turn that fired it.
//!
//! # Unbound runs never reach this point
//!
//! The dispatch call site guarantees that this point is only fired for runs
//! that have a bound actor. Work started *by* a hook runs unbound, so firing
//! `AfterTurn` on unbound completion would let each background pass schedule
//! its own successor — an unbounded self-feeding chain with no user in the
//! loop to stop it.
//!
//! There is deliberately **no `unbound` field** on this context. A hook must
//! not be able to opt back into observing background runs by inspecting a
//! flag: observing background runs is what
//! [`crate::registry::HookPointSpec::EventTriggered`] plus the
//! `LoopCompleted` runtime event kind are for. That path is observer-only, so
//! a background observation cannot start more background work.
//!
//! Because unbound runs never reach here, [`AfterTurnHookContext::user_id`]
//! is non-optional: an actorless run has no `AfterTurn` dispatch at all.

use ironclaw_host_api::ids::{AgentId, ProjectId, TenantId, UserId};

/// Read-only context handed to an `after_turn` hook. As with the other
/// points, `#[non_exhaustive]` so additional fields can land without breaking
/// hook authors.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct AfterTurnHookContext {
    pub tenant_id: TenantId,
    /// Actor the completed run was bound to. Non-optional: the dispatch site
    /// never fires this point for actorless runs (see the module docs).
    pub user_id: UserId,
    pub agent_id: Option<AgentId>,
    pub project_id: Option<ProjectId>,
    /// `true` when the turn completed successfully; `false` for any other
    /// terminal state (failed, cancelled, …).
    pub completed: bool,
}
