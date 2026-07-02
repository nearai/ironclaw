//! Reborn integration test — mid-turn cancellation (E-GATEWAY seam).
//!
//! Proves the cancel path end-to-end at the int tier: the model call parks at
//! the vendor-SDK seam, the test cancels the in-flight run, releases the park,
//! and the run reaches `TurnStatus::Cancelled` (not `Completed`). Exercises the
//! parking provider (`park_model`) and `cancel_run`. Cancellation is observed
//! by the loop-driver host's own default `TurnStateRunCancellationFactory`
//! (`group.rs` leaves the optional `cancellation_factory` as `None`), not a
//! wired coordinator fan-out.

#[allow(dead_code)]
#[path = "support/reborn/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
mod support;

use std::time::Duration;

use ironclaw_turns::TurnStatus;
use reborn_support::builder::RebornIntegrationHarness;
use reborn_support::reply::RebornScriptedReply;
use reborn_support::scripted_provider::ParkingModelGate;

#[tokio::test]
async fn cancels_a_parked_mid_turn_run() {
    let gate = ParkingModelGate::new();
    let harness = RebornIntegrationHarness::test_default()
        .park_model(gate.clone())
        .script([RebornScriptedReply::text("should never be finalized")])
        .build()
        .await
        .expect("harness builds");

    // Submit without waiting; the model call parks inside the loop.
    let run_id = harness
        .submit_turn_async("do a long thing")
        .await
        .expect("turn submitted");
    tokio::time::timeout(Duration::from_secs(10), gate.wait_until_parked())
        .await
        .expect("model call parks before the timeout");

    // Cancel while parked, then release so the loop resumes and observes the
    // cancellation at its next checkpoint.
    harness.cancel_run(run_id).await.expect("cancel accepted");
    gate.release();

    harness
        .wait_for_status(run_id, TurnStatus::Cancelled)
        .await
        .expect("parked run reaches Cancelled after cancel");
}
