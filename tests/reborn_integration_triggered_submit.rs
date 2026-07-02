//! Reborn integration-test framework — E-TRIGGERED-SUBMIT driving test.
//!
//! Proves the trusted-trigger submission seam end-to-end: this exercises the
//! real `TrustedTriggerFireSubmitter` (via
//! `RebornIntegrationHarness::submit_triggered_turn`), proving that
//! `TurnOriginKind::ScheduledTrigger` propagates all the way into the
//! persisted run state observable at the coordinator boundary
//! (`TurnCoordinator::get_run_state`).
//!
//! This is a smoke slice, not the exhaustive matrix: it asserts only that the
//! submission is accepted and that the resulting run state carries the
//! scheduled-trigger origin. It does not drive the scripted model — the
//! triggered turn executes under its own resolved scope, which intentionally
//! has no registered gateway, so this test asserts submission + persisted
//! `product_context` only, not turn completion. The exhaustive origin/delivery
//! matrix (surface types, adapters, delivery routing) is separate later
//! coverage under C-TRIGGERED-ORIGIN / C-TRIGGERED-DELIVERY.

// The support tree is large and shared; a single-test file exercises only a
// slice of it, so suppress dead-code warnings on the includes (matches
// `reborn_qa_recorded_behavior.rs`).
#[allow(dead_code)]
#[path = "support/reborn/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
mod support;

use reborn_support::builder::RebornIntegrationHarness;

#[tokio::test]
async fn triggered_submit_carries_scheduled_trigger_origin() {
    let harness = RebornIntegrationHarness::test_default()
        .build()
        .await
        .expect("harness builds");

    let submission = harness
        .submit_triggered_turn("run the scheduled reminder")
        .await
        .expect("triggered submit accepted");

    let state = harness
        .coordinator
        .get_run_state(ironclaw_turns::GetRunStateRequest {
            scope: submission.turn_scope,
            run_id: submission.run_id,
        })
        .await
        .expect("run state readable at the coordinator boundary");

    assert_eq!(
        state.product_context.map(|context| context.origin),
        Some(ironclaw_turns::TurnOriginKind::ScheduledTrigger),
        "a turn submitted through the trusted-trigger submitter must carry \
         TurnOriginKind::ScheduledTrigger, proving the real trusted-trigger \
         origin wire is exercised end to end",
    );
}
