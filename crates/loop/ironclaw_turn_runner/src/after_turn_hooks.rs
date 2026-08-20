//! Deriving the `after_turn` hook context from a terminal run (#7276).
//!
//! The hook framework owns the point, the sink trait, and the dispatch
//! discipline; what lives HERE is the one thing only this tier can do: decide,
//! from a `TurnRunState`, whether a finished run may fire the point at all.
//!
//! Two guards, enforced centrally so no individual hook has to remember them:
//!
//! - **Unbound runs never fire the point.** Work started BY a hook runs
//!   unbound, so firing on unbound completion would let each background pass
//!   schedule its own successor — a self-sustaining loop nobody asked for and
//!   nothing stops. `AfterTurnMemoryRecorder` skips unbound runs for the
//!   adjacent reason (their exchange is caller data, not a user observation).
//! - **A run with no actor is skipped.** There is nothing to attribute
//!   follow-on work to, and [`AfterTurnHookContext::user_id`] is non-optional
//!   precisely because of this guard.
//!
//! Any TERMINAL state of an ordinary actor-bearing run fires the point;
//! `completed` distinguishes success from failure or cancellation, and a hook
//! that only cares about successes checks that flag. The call site invokes this
//! only after the run has reached a terminal state, so a non-terminal status is
//! not a case this function is asked to judge — it derives a context from
//! whatever status the state carries and lets `completed` speak for it.

use ironclaw_hooks::points::AfterTurnHookContext;
use ironclaw_turns::{TurnRunState, TurnStatus};
use tracing::debug;

/// Derive the `after_turn` context from a terminal run, or `None` when this run
/// must not fire the point.
pub fn after_turn_hook_context(state: &TurnRunState) -> Option<AfterTurnHookContext> {
    if state.resolved_run_profile_id.is_unbound() {
        debug!("after-turn hooks: unbound run; not a hook trigger");
        return None;
    }
    let actor = state.actor.as_ref()?;
    Some(AfterTurnHookContext::new(
        state.scope.tenant_id.clone(),
        actor.user_id.clone(),
        state.scope.agent_id.clone(),
        state.scope.project_id.clone(),
        state.status == TurnStatus::Completed,
    ))
}
