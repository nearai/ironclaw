//! Reborn integration test — mid-turn cancellation + related failure paths
//! (E-GATEWAY seam, C-ERRORS).
//!
//! Proves the cancel path end-to-end: the model call parks at the vendor-SDK
//! seam, the test cancels the in-flight run, releases the park, and the run
//! reaches `TurnStatus::Cancelled` (not `Completed`). Cancellation is observed
//! by the loop-driver host's default `TurnStateRunCancellationFactory`, not a
//! wired coordinator fan-out.
//!
//! Also covers C-ERRORS: a leaked-permit regression guard (precedent: PR
//! #5206's RAII `ReservationGuard` bugs), thread-busy rejection, and a
//! non-retryable provider-`Err` reaching a categorized `TurnStatus::Failed`.

#[allow(dead_code)]
#[path = "support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

use std::time::Duration;

use ironclaw_product_adapters::ProductInboundAck;
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

/// A second submit on a thread whose first run is still active (parked) must
/// be rejected — `InboundTurnOutcome::RejectedBusy` — not silently queued or
/// accepted. Releases the park afterward so no parked task leaks past the test.
#[tokio::test]
async fn busy_reject_when_thread_already_has_an_active_run() {
    let gate = ParkingModelGate::new();
    let harness = RebornIntegrationHarness::test_default()
        .park_model(gate.clone())
        .script([RebornScriptedReply::text("first turn done")])
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

    let ack = harness
        .submit_turn_ack("interrupt with something else")
        .await
        .expect("the busy-reject submit itself does not error");
    assert!(
        matches!(ack, ProductInboundAck::RejectedBusy { .. }),
        "expected RejectedBusy while the thread has an active run, got {ack:?}"
    );

    gate.release();
    harness
        .wait_for_status(run_id, TurnStatus::Completed)
        .await
        .expect("first turn still completes after the rejected second submit");
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
