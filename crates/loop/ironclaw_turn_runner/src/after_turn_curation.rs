//! Deriving the memory-curation trigger signal from a terminal run (#7276).
//!
//! The vocabulary — [`AfterTurnCurationSignal`] and the port the product tier
//! implements — lives in `ironclaw_loop_contracts`, because this crate and
//! `ironclaw_assistant` deliberately do not depend on each other (WS1.7 removed
//! the production edge). What lives HERE is the one thing only this tier can
//! do: decide, from a `TurnRunState`, whether a finished run is a curation
//! trigger at all.
//!
//! Two guards, and the first is the important one:
//!
//! - **Unbound runs never trigger curation.** A curation pass is itself an
//!   unbound run, so triggering on unbound completion would let a pass schedule
//!   its own successor — a self-sustaining loop nobody asked for and nothing
//!   stops. `AfterTurnMemoryRecorder` skips unbound runs for the adjacent
//!   reason (their exchange is caller data, not a user observation).
//! - **A run with no actor is skipped.** Memory is scoped to a human owner;
//!   without one there is no memory to curate.

use ironclaw_loop_contracts::AfterTurnCurationSignal;
use ironclaw_turns::{TurnRunState, TurnStatus};
use tracing::debug;

/// Derive the signal from a terminal run, or `None` when this run must not
/// trigger curation.
pub fn curation_signal_for_completed_run(state: &TurnRunState) -> Option<AfterTurnCurationSignal> {
    if state.status != TurnStatus::Completed {
        return None;
    }
    if state.resolved_run_profile_id.is_unbound() {
        debug!("after-turn curation: unbound run; not a curation trigger");
        return None;
    }
    let actor = state.actor.as_ref()?;
    Some(AfterTurnCurationSignal {
        tenant_id: state.scope.tenant_id.clone(),
        user_id: actor.user_id.clone(),
        agent_id: state.scope.agent_id.clone(),
        project_id: state.scope.project_id.clone(),
    })
}
