//! Reborn integration test — mid-run queued-message steering (E-GATEWAY seam).
//!
//! Proves the queued-steering journey end-to-end through the real stack: a
//! message submitted while the thread's run is mid-model-call is accepted as
//! `DeferredBusy` and stored `Queued` (not rejected), the running loop drains
//! it as steering input, the transcript row flips `Queued` → `Submitted`, and
//! — the load-bearing assertion — the steering text reaches a subsequent
//! model request through the real prompt rebuild, shaping the final reply.
//!
//! The parked model call doubles as the end-of-turn race: the steering input
//! arrives while the run's FINAL scripted model call is in flight, so it is
//! only observable at the reply-only exit boundary. Without the follow-up
//! drain consuming `Steering` inputs (and without the drain-time ack that
//! makes the row model-visible before the next prompt build), the run would
//! complete with the message stranded or silently unseen.

#[allow(dead_code)]
#[path = "support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

use std::time::Duration;

use ironclaw_product::ProductInboundAck;
use ironclaw_threads::MessageStatus;
use ironclaw_turns::TurnStatus;
use reborn_support::builder::RebornIntegrationHarness;
use reborn_support::reply::RebornScriptedReply;
use reborn_support::scripted_provider::ParkingModelGate;

/// The full mid-run steering journey:
///
/// 1. Turn A parks inside its (final) model call.
/// 2. Message B submitted on the busy thread → `DeferredBusy` ack, transcript
///    row `Queued` and bound to run A — never `RejectedBusy`.
/// 3. Release the park: run A's reply-only exit drains B as steering input,
///    forces one more model iteration, and completes.
/// 4. B's text appears in a model request (the real prompt rebuild carried
///    it), the final reply is the steered script entry, and B's transcript
///    row is `Submitted`, still bound to run A.
#[tokio::test]
async fn queued_steering_message_reaches_the_model_mid_run() {
    let gate = ParkingModelGate::new();
    let harness = RebornIntegrationHarness::test_default()
        .park_model(gate.clone())
        .script([
            RebornScriptedReply::text("first reply before steering"),
            RebornScriptedReply::text("steered reply"),
        ])
        .build()
        .await
        .expect("harness builds");

    let run_id = harness
        .submit_turn_async("start a long task")
        .await
        .expect("turn A submitted");
    tokio::time::timeout(Duration::from_secs(10), gate.wait_until_parked())
        .await
        .expect("model call parks before the timeout");

    // Submit B while A is mid-model-call: accepted and queued as steering.
    let ack = harness
        .submit_turn_ack("steer the task toward blue")
        .await
        .expect("busy submit does not error");
    let ProductInboundAck::DeferredBusy { active_run_id, .. } = ack else {
        panic!("expected DeferredBusy while the thread has an active run, got {ack:?}");
    };
    assert_eq!(
        active_run_id, run_id,
        "the deferred ack must name run A as the consuming run"
    );
    let queued = harness
        .user_message_record("steer the task toward blue")
        .await
        .expect("queued message persisted");
    assert_eq!(
        queued.status,
        MessageStatus::Queued,
        "the busy-thread message must be stored Queued, not RejectedBusy"
    );
    assert_eq!(
        queued.turn_run_id.as_deref(),
        Some(run_id.to_string().as_str()),
        "the Queued row must be bound to run A"
    );

    // Release the park: A's reply-only exit must drain B and keep iterating.
    gate.release();
    harness
        .wait_for_status(run_id, TurnStatus::Completed)
        .await
        .expect("run A completes after consuming the steering input");

    harness
        .assert_model_saw_user_message("steer the task toward blue")
        .await
        .expect("the steering text must reach a model request");
    harness
        .assert_reply_contains("steered reply")
        .await
        .expect("the final reply is the post-steering script entry");
    let consumed = harness
        .user_message_record("steer the task toward blue")
        .await
        .expect("steering message still in transcript");
    assert_eq!(
        consumed.status,
        MessageStatus::Submitted,
        "the consumed steering row must flip Queued → Submitted"
    );
    assert_eq!(
        consumed.turn_run_id.as_deref(),
        Some(run_id.to_string().as_str()),
        "the Submitted row keeps its binding to run A"
    );
}
