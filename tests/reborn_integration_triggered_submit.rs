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
use reborn_support::reply::RebornScriptedReply;

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

/// C-TRIGGERED-ORIGIN contrast arm: a normal interactive user turn (through the
/// same `submit_turn` → `accept_inbound` → coordinator wire this harness always
/// uses) must record `TurnOriginKind::Inbound`, NOT `ScheduledTrigger`.
///
/// This is what makes the `triggered_submit_carries_scheduled_trigger_origin`
/// assertion above *discriminating*: without a contrasting turn on the same wire,
/// a `ScheduledTrigger` assertion could pass even if the origin were hardcoded to
/// that value everywhere. Proving the interactive turn lands on a DIFFERENT origin
/// pins that the origin is genuinely propagated from the submission path, not a
/// constant. Both origins are read through the identical
/// `coordinator.get_run_state(...).product_context.origin` boundary — the same
/// accessor the roadmap's C-TRIGGERED-ORIGIN row specifies (no new seam).
///
/// (`TurnOriginKind` has no distinct "interactive" variant — the enum is
/// `WebUi | Inbound | ScheduledTrigger`, `crates/ironclaw_turns/src/origin.rs`.
/// The harness's `submit_turn` classifies as `ProductTriggerReason::DirectChat`
/// on the Untrusted inbound path, which resolves to `Inbound`.)
#[tokio::test]
async fn interactive_submit_carries_inbound_origin_not_scheduled_trigger() {
    let harness = RebornIntegrationHarness::test_default()
        .script([RebornScriptedReply::text("ack")])
        .build()
        .await
        .expect("harness builds");

    let run_id = harness
        .submit_turn("run this interactively")
        .await
        .expect("interactive turn completes");

    let state = harness
        .coordinator
        .get_run_state(ironclaw_turns::GetRunStateRequest {
            scope: harness.turn_scope.clone(),
            run_id,
        })
        .await
        .expect("run state readable at the coordinator boundary");

    assert_eq!(
        state.product_context.map(|context| context.origin),
        Some(ironclaw_turns::TurnOriginKind::Inbound),
        "a normal interactive user turn must carry TurnOriginKind::Inbound — \
         proving the ScheduledTrigger origin on the triggered path is really \
         propagated from the submission path, not a constant",
    );
}
