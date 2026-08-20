//! The `after_turn` hook-context derivation (#7276).
//!
//! These pin which finished runs may fire the `after_turn` lifecycle point. The
//! unbound-run case is the load-bearing one: work a hook starts is ITSELF an
//! unbound run, so a derivation that accepted unbound completions would let each
//! background pass schedule its successor — an unbounded background loop that no
//! user asked for and nothing stops.

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
