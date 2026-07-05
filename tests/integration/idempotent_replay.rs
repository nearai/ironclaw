//! `ProductWorkflow::submit_inbound` idempotency — real filesystem ledger.
//!
//! Every crate-tier idempotency test (`fake_ledger_expiration_reclaims_in_flight_fingerprint`,
//! `in_memory_idempotency_ledger_*`, `duplicate_envelope_replays_prior_outcome`) drives replay
//! through an in-memory fake ledger. This harness wires a REAL, filesystem-backed
//! `FilesystemIdempotencyLedger` into `DefaultProductWorkflow` — this test proves a second
//! real read-modify-write cycle against it short-circuits to `Duplicate` without minting a
//! second turn/run, rather than asserting that behavior against a fake.

#[allow(dead_code)]
#[path = "support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

use ironclaw_product_adapters::{ProductInboundAck, ProductTriggerReason, ProductWorkflow};
use ironclaw_turns::TurnStatus;
use reborn_support::builder::RebornIntegrationHarness;
use reborn_support::reply::RebornScriptedReply;

#[tokio::test]
async fn duplicate_inbound_event_replays_prior_ack_without_resubmitting() {
    let harness = RebornIntegrationHarness::test_default()
        .script([RebornScriptedReply::text("done")])
        .build()
        .await
        .expect("harness builds");

    let envelope = harness
        .ingress
        .verified_text_envelope_with_trigger(
            "evt-replay-fixed",
            &harness.actor_id,
            &harness.conversation_id,
            "say hi",
            ProductTriggerReason::DirectChat,
        )
        .expect("envelope");

    let first_ack = harness
        .workflow
        .submit_inbound(envelope.clone())
        .await
        .expect("first submit accepted");
    let ProductInboundAck::Accepted {
        submitted_run_id: run_id,
        ..
    } = &first_ack
    else {
        panic!("expected Accepted, got {first_ack:?}");
    };
    harness
        .wait_for_status(*run_id, TurnStatus::Completed)
        .await
        .expect("first turn completes");

    // Same external_event_id (and every other fingerprint component) — the real
    // FilesystemIdempotencyLedger must find the settled reservation and replay it
    // rather than minting a second turn/run.
    let second_ack = harness
        .workflow
        .submit_inbound(envelope)
        .await
        .expect("duplicate submit does not error");

    match second_ack {
        ProductInboundAck::Duplicate { prior } => {
            assert_eq!(
                *prior, first_ack,
                "duplicate replay must return the exact prior outcome, not a fresh submission"
            );
        }
        other => panic!("expected Duplicate ack for a replayed external_event_id, got {other:?}"),
    }
}
