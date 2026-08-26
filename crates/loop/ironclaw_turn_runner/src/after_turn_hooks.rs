//! Deriving the `after_turn` hook context from a terminal run (#7276).
//!
//! The hook framework owns the point, the sink trait, and the dispatch
//! discipline; what lives HERE is the one thing only this tier can do: decide,
//! from a `TurnRunState`, whether a finished run may fire the point at all.
//!
//! Two guards, enforced centrally so no individual hook has to remember them:
//!
//! - **Only a real conversation turn fires the point.** The admitted set is an
//!   ALLOWLIST of the profiles a human-in-the-loop conversation resolves to,
//!   not a denylist of everything else, because the point may start follow-on
//!   work with the run's actor's authority and no user present. A denylist
//!   fails open: the next non-conversation profile anyone adds inherits the
//!   right to trigger background work simply by not having been listed.
//!   [`is_conversation_turn_profile`] documents each excluded family.
//! - **A run with no actor is skipped.** There is nothing to attribute
//!   follow-on work to, and [`AfterTurnHookContext::user_id`] is non-optional
//!   precisely because of this guard.
//!
//! - **Only a TERMINAL state fires the point.** A blocked run (approval, auth,
//!   resource, dependent run, external tool) has not finished: it will be
//!   resumed and reach a terminal state later, so firing on the blocked state
//!   too would deliver the SAME turn to every hook twice — a curation-style
//!   hook would count one turn twice and a notifying hook would announce a
//!   turn that is still running. The predicate is
//!   [`TurnStatus::is_terminal`], never a local list, so a status added to the
//!   kernel is judged by the kernel's own definition here.
//!
//! Any terminal state of an ordinary actor-bearing conversation run fires the
//! point; `completed` distinguishes success from failure or cancellation, and a
//! hook that only cares about successes checks that flag.

use ironclaw_hooks::points::AfterTurnHookContext;
use ironclaw_host_api::turn::RunProfileId;
use ironclaw_turns::{TurnRunState, TurnStatus};
use tracing::debug;

use crate::planned_driver_factory::PLANNED_DEFAULT_PROFILE_ID;

/// True only for the run profiles an ordinary conversation turn resolves to.
///
/// The three admitted ids are the whole conversation surface:
/// `reborn-planned-default` is what production resolves for a WebUI or channel
/// turn (both submit with no requested profile, and the planned resolver's
/// implicit default is this id); `interactive_default` is the same role under
/// the non-planned resolver; `default` is the generic alias compositions and
/// harnesses resolve when they register no planned profile at all.
///
/// Deliberately NOT admitted:
///
/// - `unbound_default` / `unbound_structured` — work started BY a hook runs
///   unbound, so admitting these would let each background pass schedule its
///   own successor, an unbounded self-feeding chain with no user to stop it.
/// - `scheduled_trigger` — a trusted trigger fire keeps its creator as the
///   actor and passes the actor guard, but no human is present: firing here
///   would let a background schedule alone drive write-capable follow-on work.
/// - `reborn-planned-subagent` — a child run is conversation-adjacent
///   machinery, not a turn. One user turn can spawn many children, so firing
///   per child would multiply every interval a hook counts.
/// - Anything else, including deployment-registered profiles. An unknown
///   profile is unknown authority; it must opt in here explicitly.
fn is_conversation_turn_profile(profile: &RunProfileId) -> bool {
    profile.is_interactive_default()
        || profile == &RunProfileId::default_profile()
        || profile.as_str() == PLANNED_DEFAULT_PROFILE_ID
}

/// Derive the `after_turn` context from a terminal run, or `None` when this run
/// must not fire the point.
pub fn after_turn_hook_context(state: &TurnRunState) -> Option<AfterTurnHookContext> {
    // Terminality first: the point is "this turn is over". A gated run that is
    // later resumed would otherwise fire it once on the block and once on the
    // real ending — the same turn delivered twice.
    if !state.status.is_terminal() {
        debug!(
            status = ?state.status,
            "after-turn hooks: run is not terminal; not a hook trigger"
        );
        return None;
    }
    if !is_conversation_turn_profile(&state.resolved_run_profile_id) {
        debug!(
            profile = state.resolved_run_profile_id.as_str(),
            "after-turn hooks: not a conversation turn; not a hook trigger"
        );
        return None;
    }
    let actor = state.actor.as_ref()?;
    Some(AfterTurnHookContext::new(
        state.scope.tenant_id.clone(),
        state.run_id,
        actor.user_id.clone(),
        state.scope.agent_id.clone(),
        state.scope.project_id.clone(),
        state.status == TurnStatus::Completed,
    ))
}
