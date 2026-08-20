//! The curation-trigger derivation (#7276).
//!
//! These pin which finished runs may kick a memory-curation pass. The
//! unbound-run case is the load-bearing one: a curation pass is ITSELF an
//! unbound run, so a derivation that accepted unbound completions would let each
//! pass schedule its successor — an unbounded background loop, running the model
//! against a user's memory forever, that no user asked for and nothing stops.

use chrono::Utc;
use ironclaw_host_api::ids::{TenantId, ThreadId, UserId};
use ironclaw_host_api::turn::{AcceptedMessageRef, RunProfileId, TurnActor, TurnScope, TurnStatus};
use ironclaw_turn_runner::after_turn_curation::curation_signal_for_completed_run;
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
fn a_completed_user_turn_is_a_curation_trigger() {
    let state = run_state(
        TurnStatus::Completed,
        RunProfileId::interactive_default(),
        actor(),
    );

    let signal = curation_signal_for_completed_run(&state).expect("an ordinary completed turn");

    assert_eq!(signal.tenant_id.as_str(), "tenant-a");
    assert_eq!(
        signal.user_id.as_str(),
        "user-a",
        "curation is scoped to the acting human, whose memory it would curate"
    );
}

/// THE guard. A curation pass runs unbound; if its completion triggered
/// curation, every pass would schedule the next one forever.
#[test]
fn an_unbound_run_never_triggers_curation() {
    for profile in [
        RunProfileId::unbound_default(),
        RunProfileId::unbound_structured(),
    ] {
        let state = run_state(TurnStatus::Completed, profile.clone(), actor());
        assert!(
            curation_signal_for_completed_run(&state).is_none(),
            "{profile:?} must not trigger curation: a pass would schedule its own successor"
        );
    }
}

/// Memory is scoped to a human owner. A trigger-fired or host-initiated run has
/// no actor, so there is no memory to curate and nothing to scope a pass to.
#[test]
fn a_run_without_an_actor_is_not_a_trigger() {
    let state = run_state(
        TurnStatus::Completed,
        RunProfileId::interactive_default(),
        None,
    );

    assert!(curation_signal_for_completed_run(&state).is_none());
}

/// Only a run that actually finished. A failed or cancelled turn says nothing
/// about whether memory needs tidying, and counting it would drift the interval.
#[test]
fn a_non_completed_run_is_not_a_trigger() {
    for status in [
        TurnStatus::Failed,
        TurnStatus::Cancelled,
        TurnStatus::Running,
    ] {
        let state = run_state(status, RunProfileId::interactive_default(), actor());
        assert!(
            curation_signal_for_completed_run(&state).is_none(),
            "{status:?} must not trigger curation"
        );
    }
}
