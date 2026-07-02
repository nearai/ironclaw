//! C-COMMCTX: a wired `communication_context_provider` reaches the turn
//! pipeline — the delivery-preference / connected-channel slice it resolves
//! renders into the model request on a real coordinator-path turn.
//!
//! Distinct from the outbound delivery **sink** (E-OUTBOUND, a sibling lane):
//! this covers prompt **context** (delivery preferences/targets), not a delivery
//! recorder. The production `RuntimeCommunicationContextProvider`'s
//! facade→context mapping is densely unit-tested at crate tier
//! (`ironclaw_reborn_composition::communication_context`); this binary covers
//! only the int-tier wiring gap — that the `communication_context_provider`
//! field threads through the coordinator path into the model request.

#[allow(dead_code)]
#[path = "support/reborn/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
mod support;

use reborn_support::builder::RebornIntegrationHarness;
use reborn_support::comm_context::RecordingCommunicationContextProvider;
use reborn_support::reply::RebornScriptedReply;

/// A configured delivery target + connected channel supplied by the wired
/// provider both appear in the model-visible request, proving the communication
/// slice reached the turn pipeline (not just the provider in isolation).
#[tokio::test]
async fn communication_context_slice_reaches_model_request() {
    let provider = RecordingCommunicationContextProvider::with_target_and_channel(
        "reborn-commctx-target",
        "slack",
        "reborn-commctx-channel",
    );
    let h = RebornIntegrationHarness::test_default()
        .with_communication_context_provider(provider)
        .script([RebornScriptedReply::text("ok")])
        .build()
        .await
        .expect("harness builds");
    h.submit_turn("hello").await.expect("turn completes");
    h.assert_model_request_contains("Outbound delivery target: reborn-commctx-target (slack)")
        .await
        .expect("delivery-target slice must reach the model request");
    h.assert_model_request_contains("Connected channels: reborn-commctx-channel")
        .await
        .expect("connected-channel slice must reach the model request");
}

/// Guard: with no provider wired, no communication section is rendered — pins
/// that the assertion above is not matching an incidental prompt fragment and
/// that the default path is behavior-identical (no comm slice).
#[tokio::test]
async fn no_communication_section_without_provider() {
    let h = RebornIntegrationHarness::test_default()
        .script([RebornScriptedReply::text("ok")])
        .build()
        .await
        .expect("harness builds");
    h.submit_turn("hello").await.expect("turn completes");
    assert!(
        h.assert_model_request_contains("Outbound delivery target:")
            .await
            .is_err(),
        "no communication section must render when no provider is wired"
    );
}
