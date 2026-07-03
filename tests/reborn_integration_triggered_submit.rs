//! Reborn integration-test framework — E-TRIGGERED-SUBMIT driving test.
//!
//! Proves the trusted-trigger submission seam end-to-end: this exercises the
//! real `TrustedTriggerFireSubmitter` (via
//! `RebornIntegrationHarness::submit_triggered_turn`), proving that
//! `TurnOriginKind::ScheduledTrigger` propagates all the way into the
//! persisted run state observable at the coordinator boundary
//! (`TurnCoordinator::get_run_state`).
//!
//! Three slices, one wire:
//! - `triggered_submit_carries_scheduled_trigger_origin` — submission accepted,
//!   run state carries the scheduled-trigger origin (unscripted: the run then
//!   fails benignly on the scope-miss sentinel, which this slice never reads).
//! - `interactive_submit_carries_inbound_origin_not_scheduled_trigger` — the
//!   discriminating contrast arm (C-TRIGGERED-ORIGIN).
//! - `triggered_run_completes_and_persists_reply_in_trigger_thread` — drives a
//!   triggered run to completion via `submit_triggered_turn_scripted` and pins
//!   the int-tier-observable delivery contract (reply persisted in the
//!   trigger's own thread; see that test's docs for why the outbound push leg
//!   is out of reach at this tier).
//!
//! The Slack push/delivery-routing matrix stays with the services-shell spike
//! (C-TRIGGERED-DELIVERY defer).

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
/// a `ScheduledTrigger` assertion could pass even if the origin were hardcoded
/// everywhere. Both origins are read through the identical
/// `coordinator.get_run_state(...).product_context.origin` boundary.
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

/// Post-fire delivery semantics at int tier: a triggered run driven to
/// completion persists its final reply in the TRIGGER's own thread, readable
/// through the same thread-history boundary interactive replies use.
///
/// At this tier a completed run's final reply does NOT route through
/// `ProductAdapter::render_outbound` (and therefore never reaches an
/// `OutboundDeliverySink`) — the only production constructor of
/// `ProductOutboundDeliveryRequest` is the Slack delivery services-shell
/// (`slack_delivery.rs`, feature-gated), which no harness composition wires.
/// The int-tier-observable delivery contract is therefore: reply finalized +
/// persisted in the trigger's own thread — the same state production's
/// `deliver_triggered_run` reads before pushing. The push leg itself stays
/// with the services-shell spike (C-TRIGGERED-DELIVERY defer).
#[tokio::test]
async fn triggered_run_completes_and_persists_reply_in_trigger_thread() {
    let harness = RebornIntegrationHarness::test_default()
        .build()
        .await
        .expect("harness builds");

    let submission = harness
        .submit_triggered_turn_scripted(
            "run the scheduled digest",
            [RebornScriptedReply::text("scheduled digest complete")],
        )
        .await
        .expect("scripted triggered submit accepted");

    harness
        .wait_for_status_in_scope(
            &submission.turn_scope,
            submission.run_id,
            ironclaw_turns::TurnStatus::Completed,
        )
        .await
        .expect("triggered run completes on the shared scheduler");

    harness
        .thread_harness
        .assert_final_reply(
            submission.turn_scope.thread_id.clone(),
            "scheduled digest complete",
        )
        .await
        .expect("triggered run's final reply persisted in the trigger's own thread");
}
