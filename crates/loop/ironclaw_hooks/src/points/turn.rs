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
//!
//! # Terminal coverage today: successfully-applied exits only
//!
//! A documented limitation, not an accident. The point is dispatched from the
//! turn-run executor after it APPLIES a run's loop exit. The other ways a run
//! reaches a terminal state — a driver invocation that fails, an exit whose
//! application fails, and the scheduler's own failure terminalization — return
//! through separate recovery paths that do not dispatch it. So a hook counting
//! turns sees every ordinary ending (completed, cancelled, and failures the
//! executor applied) and misses runs terminalized by those fallbacks.
//!
//! For the curation-style hooks this point exists for, missing a failure path
//! is a slightly-late chore, never a wrong one. Routing every terminalization
//! through one seam is the right shape and is tracked as a follow-up on
//! #7770.

use ironclaw_host_api::ids::{AgentId, ProjectId, TenantId, UserId};
use ironclaw_host_api::turn::TurnRunId;

/// Read-only context handed to an `after_turn` hook. As with the other
/// points, `#[non_exhaustive]` so additional fields can land without breaking
/// hook authors.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct AfterTurnHookContext {
    pub tenant_id: TenantId,
    /// The terminal run that fired the point. It gives a hook a per-trigger
    /// identity it does not have to invent: stable across crash-retries of the
    /// same trigger (the retry replays the same run), distinct for every other
    /// trigger, and needing no durable counter to stay that way.
    pub run_id: TurnRunId,
    /// Actor the completed run was bound to. Non-optional: the dispatch site
    /// never fires this point for actorless runs (see the module docs).
    pub user_id: UserId,
    pub agent_id: Option<AgentId>,
    pub project_id: Option<ProjectId>,
    /// `true` when the turn completed successfully; `false` for any other
    /// terminal state (failed, cancelled, …).
    pub completed: bool,
}

impl AfterTurnHookContext {
    /// Build the context. Required because the struct is `#[non_exhaustive]`:
    /// the dispatch call site lives in `ironclaw_turn_runner`, outside this
    /// crate, so it cannot use a struct literal.
    ///
    /// `user_id` is taken by value and non-optional by design — see the module
    /// docs: a run with no bound actor never reaches this point at all.
    pub fn new(
        tenant_id: TenantId,
        run_id: TurnRunId,
        user_id: UserId,
        agent_id: Option<AgentId>,
        project_id: Option<ProjectId>,
        completed: bool,
    ) -> Self {
        Self {
            tenant_id,
            run_id,
            user_id,
            agent_id,
            project_id,
            completed,
        }
    }
}
