//! C-COMMCTX: a wired `communication_context_provider` reaches the turn
//! pipeline — the delivery-preference / connected-channel slice it resolves
//! renders into the model request on a real coordinator-path turn.
//!
//! Distinct from the outbound delivery sink (E-OUTBOUND): this covers prompt
//! context, not a delivery recorder. The service→context mapping itself is
//! unit-tested at crate tier (`ironclaw_reborn_composition::communication_context`);
//! this binary covers only that the field threads through the coordinator path
//! into the model request.

#[allow(dead_code)]
#[path = "support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
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

    // Baseline: a request WAS captured, so the negative assertion below
    // proves absence of the section, not a vacuous pass on zero requests.
    h.assert_model_request_contains("hello")
        .await
        .expect("the turn's own text must reach the captured model request");

    // Specific error check (not `is_err()`): pins the failure to the
    // "not found" path over the one captured request, ruling out an infra
    // failure or a zero-capture regression masquerading as proof of absence.
    let err = h
        .assert_model_request_contains("Outbound delivery target:")
        .await
        .expect_err("no communication section must render when no provider is wired");
    assert!(
        err.to_string()
            .starts_with("no model request contained \"Outbound delivery target:\""),
        "expected the intended \"not found\" assertion failure, got a different harness error: {err}"
    );
    assert!(
        err.to_string().contains("captured 1 request(s)"),
        "expected exactly one captured model request; got: {err}"
    );
}
