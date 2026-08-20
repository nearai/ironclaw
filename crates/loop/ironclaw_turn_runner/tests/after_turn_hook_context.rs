//! The `after_turn` hook-context derivation (#7276).
//!
//! These pin which finished runs may fire the `after_turn` lifecycle point. The
//! admitted set is an allowlist of conversation profiles, and the excluded ones
//! are each excluded for a reason a test states: work a hook starts is ITSELF an
//! unbound run (so accepting unbound completions would let each background pass
//! schedule its successor forever); a scheduled-trigger fire keeps its creator
//! as actor but has no human present; a subagent child is machinery, not a turn,
//! and one user turn can spawn many.

use chrono::Utc;
use ironclaw_host_api::ids::{TenantId, ThreadId, UserId};
use ironclaw_host_api::turn::{AcceptedMessageRef, RunProfileId, TurnActor, TurnScope, TurnStatus};
use ironclaw_turn_runner::after_turn_hooks::after_turn_hook_context;
use ironclaw_turns::{EventCursor, TurnId, TurnRunId, TurnRunState};

fn run_state(status: TurnStatus, profile: RunProfileId, actor: Option<TurnActor>) -> TurnRunState {
    let scope = TurnScope::new_with_owner(
        TenantId::new("tenant-a").expect("tenant id"),
        None,
        None,
        ThreadId::new("thread-a").expect("thread id"),
        Some(UserId::new("user-a").expect("user id")),
    );
    TurnRunState {
        scope,
        actor,
        turn_id: TurnId::new(),
        run_id: TurnRunId::new(),
        status,
        accepted_message_ref: AcceptedMessageRef::new("msg:accepted").expect("valid ref"),
        output_contract: ironclaw_host_api::output::OutputContract::AssistantMessage,
        resolved_run_profile_id: profile,
        resolved_run_profile_version: ironclaw_host_api::turn::RunProfileVersion::new(1),
        allow_steering: false,
        resolved_model_route: None,
        model_usage: None,
        execution_outcome: None,
        received_at: Utc::now(),
        checkpoint_id: None,
        gate_ref: None,
        blocked_activity_id: None,
        credential_requirements: Vec::new(),
        failure: None,
        event_cursor: EventCursor(0),
        product_context: None,
        resume_disposition: None,
    }
}

fn actor() -> Option<TurnActor> {
    Some(TurnActor::new(UserId::new("user-a").expect("user id")))
}

#[test]
fn a_completed_user_turn_yields_a_hook_context() {
    let state = run_state(
        TurnStatus::Completed,
        RunProfileId::interactive_default(),
        actor(),
    );

    let ctx = after_turn_hook_context(&state).expect("an ordinary completed turn");

    assert_eq!(ctx.tenant_id.as_str(), "tenant-a");
    assert_eq!(
        ctx.user_id.as_str(),
        "user-a",
        "follow-on work is attributed to the acting human, and scoped to them"
    );
    assert!(ctx.completed, "a Completed run reports success to hooks");
}

/// THE guard. Hook-started background work runs unbound; if its completion fired
/// the point, every pass would schedule the next one forever.
#[test]
fn an_unbound_run_never_fires_the_point() {
    for profile in [
        RunProfileId::unbound_default(),
        RunProfileId::unbound_structured(),
    ] {
        let state = run_state(TurnStatus::Completed, profile.clone(), actor());
        assert!(
            after_turn_hook_context(&state).is_none(),
            "{profile:?} must not fire after_turn: a pass would schedule its own successor"
        );
    }
}

/// A trusted scheduled-trigger fire is the case a denylist got wrong: it keeps
/// its creator as the `TurnActor` and runs an ordinary (non-unbound) profile, so
/// both original guards passed it. Nobody is present at a fire, and the point
/// may start write-capable follow-on work with that actor's authority — so the
/// profile itself has to be the thing that excludes it.
#[test]
fn a_scheduled_trigger_fire_never_fires_the_point() {
    let state = run_state(
        TurnStatus::Completed,
        RunProfileId::scheduled_trigger(),
        actor(),
    );

    assert!(
        after_turn_hook_context(&state).is_none(),
        "a background schedule alone must not drive hook-started work"
    );
}

/// A subagent child run is conversation-adjacent machinery, not a turn. One user
/// turn can spawn many children, so firing per child would multiply whatever
/// interval a hook counts.
#[test]
fn a_subagent_child_run_never_fires_the_point() {
    let profile = ironclaw_turn_runner::planned_driver_factory::subagent_planned_profile_id()
        .expect("the subagent profile id is valid");
    let state = run_state(TurnStatus::Completed, profile, actor());

    assert!(after_turn_hook_context(&state).is_none());
}

/// The production conversation profile: what a WebUI or channel turn actually
/// resolves to (both submit with no requested profile, and the planned
/// resolver's implicit default is this id). If this stopped firing, curation
/// would go silently dead in production while every other test still passed.
#[test]
fn the_production_conversation_profile_fires_the_point() {
    let profile = ironclaw_turn_runner::planned_driver_factory::planned_default_profile_id()
        .expect("the planned default profile id is valid");
    let state = run_state(TurnStatus::Completed, profile, actor());

    assert!(after_turn_hook_context(&state).is_some());
}

/// The context carries the triggering run, which is what lets a hook derive a
/// per-trigger identity without inventing a counter: distinct per run, and
/// replayed as-is by a crash-retry of the same run.
#[test]
fn the_context_carries_the_triggering_run_id() {
    let state = run_state(
        TurnStatus::Completed,
        RunProfileId::interactive_default(),
        actor(),
    );

    let ctx = after_turn_hook_context(&state).expect("an ordinary completed turn");

    assert_eq!(ctx.run_id, state.run_id);
}

/// A trigger-fired or host-initiated run has no actor, so there is nothing to
/// attribute follow-on work to — which is why the context's `user_id` is
/// non-optional.
#[test]
fn a_run_without_an_actor_does_not_fire_the_point() {
    let state = run_state(
        TurnStatus::Completed,
        RunProfileId::interactive_default(),
        None,
    );

    assert!(after_turn_hook_context(&state).is_none());
}

/// The point is about the turn as a whole, so EVERY terminal state of an
/// ordinary actor-bearing run reaches it — a hook that cares only about
/// successes reads `completed`. Making that the hook's call, rather than the
/// derivation's, is what lets one point serve both "tidy up after a good turn"
/// and "notice this turn failed".
#[test]
fn a_failed_or_cancelled_run_fires_the_point_with_completed_false() {
    for status in [TurnStatus::Failed, TurnStatus::Cancelled] {
        let state = run_state(status, RunProfileId::interactive_default(), actor());
        let ctx = after_turn_hook_context(&state)
            .unwrap_or_else(|| panic!("{status:?} yields a context"));
        assert!(
            !ctx.completed,
            "{status:?} is terminal but not a success; hooks must be able to tell"
        );
    }
}

/// The derivation judges the SCOPE of a run, never its terminality: the call
/// site fires it only after a run has reached a terminal state. A non-terminal
/// status therefore never reaches here in production — the contract pinned is
/// that the derivation reports such a run as not-completed rather than
/// inventing a success.
#[test]
fn a_non_terminal_status_is_never_reported_as_completed() {
    let state = run_state(
        TurnStatus::Running,
        RunProfileId::interactive_default(),
        actor(),
    );

    let ctx =
        after_turn_hook_context(&state).expect("scope guards pass; terminality is not judged");
    assert!(!ctx.completed);
}
