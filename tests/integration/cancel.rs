//! Reborn integration test — mid-turn cancellation + related failure paths
//! (E-GATEWAY seam, C-ERRORS).
//!
//! Proves the cancel path end-to-end: the model call parks at the vendor-SDK
//! seam, the test cancels the in-flight run, releases the park, and the run
//! reaches `TurnStatus::Cancelled` (not `Completed`). Cancellation is observed
//! by the loop-driver host's default `AgentTurnRunCancellationFactory`, not a
//! wired coordinator fan-out.
//!
//! Also covers C-ERRORS: a leaked-permit regression guard (precedent: PR
//! #5206's RAII `ReservationGuard` bugs) and a non-retryable provider-`Err`
//! reaching a categorized `TurnStatus::Failed`. (The busy-thread submit
//! scenario lives in `steering.rs` now that a busy submit queues as steering
//! input instead of rejecting.)

#[allow(dead_code)]
#[path = "support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

use std::time::Duration;

use ironclaw_product::ProductInboundAck;
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

/// Regression guard: cancelling a parked run must release its per-actor/
/// tenant admission permit. If leaked (precedent: PR #5206's WASM
/// permit/reservation bugs), a second turn on the SAME thread would hang or
/// come back `RejectedBusy` instead of completing.
#[tokio::test]
async fn cancelled_run_does_not_block_a_second_turn_on_the_same_thread() {
    let gate = ParkingModelGate::new();
    let harness = RebornIntegrationHarness::test_default()
        .park_model(gate.clone())
        .script([
            RebornScriptedReply::text("should never be finalized"),
            RebornScriptedReply::text("second turn done"),
        ])
        .build()
        .await
        .expect("harness builds");

    let run_id = harness
        .submit_turn_async("do a long thing")
        .await
        .expect("first turn submitted");
    tokio::time::timeout(Duration::from_secs(10), gate.wait_until_parked())
        .await
        .expect("model call parks before the timeout");
    harness.cancel_run(run_id).await.expect("cancel accepted");
    gate.release();
    harness
        .wait_for_status(run_id, TurnStatus::Cancelled)
        .await
        .expect("parked run reaches Cancelled after cancel");

    // The gate's channels are already consumed, so this second call passes
    // through the same `ParkingLlm` instantly (`ParkingModelGate`'s "second
    // call does not block" guarantee).
    harness
        .submit_turn("do another thing")
        .await
        .expect("second turn on the same thread completes after the first was cancelled");
    harness
        .assert_reply_contains("second turn done")
        .await
        .expect("second turn's reply persisted");
}

/// A raw provider `Err` classified non-retryable by `ironclaw_llm`
/// (`LlmError::ContextLengthExceeded`, excluded from `is_retryable`) must
/// reach `TurnStatus::Failed` after bounded context-shrink recovery is
/// exhausted, categorized `"model_context_overflow"` by the batch-2 provider
/// fidelity mapping (not the generic `"model_error"`), and must not retry
/// forever.
#[tokio::test]
async fn mid_turn_provider_error_reaches_failed_with_model_error_category() {
    let harness = RebornIntegrationHarness::test_default()
        .fail_model()
        .build()
        .await
        .expect("harness builds");

    let run_id = harness
        .submit_turn_async("do something")
        .await
        .expect("turn submitted");
    let state = harness
        .wait_for_status(run_id, TurnStatus::Failed)
        .await
        .expect("run reaches Failed after a non-retryable provider error");
    let failure = state
        .failure
        .as_ref()
        .expect("a Failed run must carry a failure detail");
    assert_eq!(
        failure.category(),
        "model_context_overflow",
        "expected the context-overflow fidelity category (ContextLengthExceeded), got {failure:?}"
    );
}

/// Credentials sibling of the context-overflow test above (issue #6284 item
/// 1 — precise model-path failure categories): a raw provider auth failure
/// (`LlmError::AuthFailed`, non-retryable) must reach `TurnStatus::Failed`
/// with the PINNED, user-actionable `model_credentials_unavailable` category
/// — "fix the provider API key" — not the generic
/// `model_unavailable`/`host_stage_unavailable_model` collapse. Asserts at
/// the persisted `SanitizedFailure` seam through the full production path:
/// provider `Err` -> `map_provider_error` (`CredentialUnavailable`) -> loop
/// `HostUnavailableWithDiagnostics{Model}` -> runner
/// `host_stage_failure_category` -> persisted failure.
#[tokio::test]
async fn mid_turn_auth_provider_error_reaches_failed_with_credentials_category() {
    let harness = RebornIntegrationHarness::test_default()
        .fail_model_auth()
        // Recording is additive: placing it after the failing mode must not
        // replace the selected provider behavior.
        .record_model_calls_for_test()
        .with_turn_event_sink()
        .build()
        .await
        .expect("harness builds");

    let run_id = harness
        .submit_turn_async("do something")
        .await
        .expect("turn submitted");
    let state = harness
        .wait_for_status(run_id, TurnStatus::Failed)
        .await
        .expect("run reaches Failed after a non-retryable provider auth error");
    let failure = state
        .failure
        .as_ref()
        .expect("a Failed run must carry a failure detail");
    assert_eq!(
        failure.category(),
        "model_credentials_unavailable",
        "expected the pinned credentials category (AuthFailed), got {failure:?}"
    );
    let detail = failure
        .detail()
        .expect("credentials failure must carry the provider cause for the explainer");
    assert!(
        detail.contains("Authentication failed"),
        "detail should describe the auth failure, got {detail:?}"
    );
    harness
        .assert_interactive_model_provider_call_count(1)
        .await
        .expect("invalid credentials must not blindly retry the provider");
    harness
        .assert_text_model_provider_call_count(0)
        .await
        .expect("terminal auth handling must not invoke a model explainer");
    harness
        .assert_only_tools_invoked(&[])
        .await
        .expect("terminal auth handling must not dispatch a tool");
    harness
        .assert_model_message_content_occurrences("model error observation", 0)
        .await
        .expect("the failed model must not be credited with seeing an observation");
    harness
        .assert_turn_event_recorded(ironclaw_turns::TurnEventKind::Failed)
        .await
        .expect("the credentials failure is durably published");
}

/// Regression guard, `Failed`-path sibling of
/// `cancelled_run_does_not_block_a_second_turn_on_the_same_thread`: the
/// per-thread busy/admission lock must release on `TurnStatus::Failed`, not
/// just `Cancelled` (same "wedge class" as PR #5206's leaked WASM
/// permit/reservation bugs) — a leak would make a second submit on the SAME
/// thread come back `RejectedBusy`.
///
/// The second turn also fails (same `"model_context_overflow"` category), not
/// completes: `fail_model()` swaps in `ErrLlm` as the thread's entire raw
/// model provider permanently (no per-call counting), and there is no
/// builder seam to swap in a fresh script for a second turn on the same
/// thread (a second `group.thread(...)` for the same `conversation_id` would
/// panic on `ScopeRegistryGateway::register`'s duplicate-registration guard).
/// The regression signal is that it is *admitted* (`Accepted`, not
/// `RejectedBusy`) and reaches its own terminal status promptly, proving the
/// lock was genuinely released.
#[tokio::test]
async fn failed_run_does_not_block_a_second_turn_on_the_same_thread() {
    let harness = RebornIntegrationHarness::test_default()
        .fail_model()
        .build()
        .await
        .expect("harness builds");

    let run_id = harness
        .submit_turn_async("do something")
        .await
        .expect("first turn submitted");
    harness
        .wait_for_status(run_id, TurnStatus::Failed)
        .await
        .expect("first turn reaches Failed after a non-retryable provider error");

    let ack = harness
        .submit_turn_ack("do another thing")
        .await
        .expect("the second submit itself does not error");
    assert!(
        matches!(ack, ProductInboundAck::Accepted { .. }),
        "expected the second submit to be accepted after the first run Failed \
         (busy lock released), got {ack:?}"
    );
    let run_id_2 = match ack {
        ProductInboundAck::Accepted {
            submitted_run_id, ..
        } => submitted_run_id,
        other => unreachable!("checked Accepted above, got {other:?}"),
    };

    let state = harness
        .wait_for_status(run_id_2, TurnStatus::Failed)
        .await
        .expect(
            "second turn on the same thread still reaches a terminal status \
             after the first run Failed and released the busy lock",
        );
    let failure = state
        .failure
        .as_ref()
        .expect("a Failed run must carry a failure detail");
    assert_eq!(
        failure.category(),
        "model_context_overflow",
        "expected the context-overflow fidelity category on the second run too, got {failure:?}"
    );
}
