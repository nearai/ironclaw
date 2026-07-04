//! Reborn integration test — mid-turn cancellation + related failure paths
//! (E-GATEWAY seam, C-ERRORS).
//!
//! Proves the cancel path end-to-end at the int tier: the model call parks at
//! the vendor-SDK seam, the test cancels the in-flight run, releases the park,
//! and the run reaches `TurnStatus::Cancelled` (not `Completed`). Exercises the
//! parking provider (`park_model`) and `cancel_run`. Cancellation is observed
//! by the loop-driver host's own default `TurnStateRunCancellationFactory`
//! (`group.rs` leaves the optional `cancellation_factory` as `None`), not a
//! wired coordinator fan-out.
//!
//! The tests below extend the same seam with C-ERRORS coverage: a
//! leaked-permit regression guard on the cancel path (precedent: PR #5206's
//! RAII `ReservationGuard` bugs), thread-busy rejection, and a non-retryable
//! provider-`Err` reaching a categorized `TurnStatus::Failed`.

#[allow(dead_code)]
#[path = "support/reborn/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
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

/// Regression guard: cancelling a parked run must release whatever
/// per-actor/tenant admission slot or run-concurrency permit it held. If
/// cancellation leaked one (precedent: PR #5206's leaked WASM
/// permit/reservation bugs), a second turn on the SAME thread right after
/// would either hang past `wait_for_status`'s internal deadline or come back
/// `RejectedBusy` instead of completing.
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

    // The gate's channels are already consumed after the first park+release
    // cycle, so this second call passes through the same `ParkingLlm` instantly
    // (see `ParkingModelGate`'s "second call does not block" guarantee).
    harness
        .submit_turn("do another thing")
        .await
        .expect("second turn on the same thread completes after the first was cancelled");
    harness
        .assert_reply_contains("second turn done")
        .await
        .expect("second turn's reply persisted");
}

/// A second submit on a thread whose first run is still active (parked, not
/// yet cancelled) must be rejected — `InboundTurnOutcome::RejectedBusy`
/// (`ironclaw_product_workflow::inbound_turn`) — not silently queued or
/// accepted. Releases the park afterward so the first run still completes and
/// the harness doesn't leak a parked task past the test.
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

/// A raw provider `Err` that the real `ironclaw_llm` decorator chain classifies
/// as non-retryable (`LlmError::ContextLengthExceeded`, excluded from
/// `ironclaw_llm::retry::is_retryable`) must reach the model a `TurnStatus::Failed`
/// run categorized `"model_error"` (`LoopFailureKind::ModelError`), not silently
/// retry forever or surface as a different failure category.
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
        "model_error",
        "expected LoopFailureKind::ModelError, got {failure:?}"
    );
}

/// Regression guard, `Failed`-path sibling of
/// `cancelled_run_does_not_block_a_second_turn_on_the_same_thread`: whatever
/// per-thread busy/admission lock a run holds must be released when the run
/// reaches `TurnStatus::Failed`, not just `Cancelled`. If that release leaked
/// (same "wedge class" as PR #5206's leaked WASM permit/reservation bugs), a
/// second submit on the SAME thread right after would come back
/// `RejectedBusy` instead of being admitted.
///
/// This cannot mirror the cancel-path test's shape exactly (second turn
/// reaching `Completed`): `fail_model()` swaps in `ErrLlm` — see
/// `tests/support/reborn/scripted_provider.rs` — as this thread's ENTIRE raw
/// model provider, unconditionally, for every future call (no per-call
/// counting; `ErrLlm::complete`/`complete_with_tools` always return
/// `LlmError::ContextLengthExceeded`). There is also no builder seam to swap
/// in a fresh, successful script for a second turn on the same thread: a
/// second `RebornIntegrationGroup::thread(...)` build for the same
/// `conversation_id` would resolve the same `TurnScope` and panic on
/// `ScopeRegistryGateway::register`'s duplicate-registration guard (see
/// `tests/support/reborn/scope_gateway.rs::duplicate_register_for_same_scope_panics`).
/// So the second turn here also fails (same `"model_error"` category) — the
/// regression signal is that it is *admitted* (`Accepted`, not `RejectedBusy`)
/// and reaches its own terminal status promptly rather than hanging or being
/// silently dropped, proving the lock was genuinely released and not just
/// accepted-then-stuck.
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
        "model_error",
        "expected LoopFailureKind::ModelError on the second run too, got {failure:?}"
    );
}
