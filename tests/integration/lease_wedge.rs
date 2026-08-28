//! Reborn integration test — tool-path lease-expiry wedge coverage (issue
//! #5476, Row-D runtime robustness).
//!
//! The model path already has `ParkingModelGate`/`ParkingLlm` for mid-turn
//! cancel coverage (`tests/integration/cancel.rs`), but until now nothing
//! could park the tool/capability-dispatch path — so a scenario where a tool
//! call outlives its run's scheduler lease was untestable. This proves the
//! scheduler's real lease-recovery sweep (`recover_expired_leases`, 10s
//! production cadence, shortened here via
//! `with_lease_recovery_interval_for_test` so the test doesn't wait on the
//! production tick) reaps a wedged run into a terminal, observable state
//! instead of leaving it `Running` forever.

#[allow(dead_code)]
#[path = "support/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

use std::time::Duration;

use ironclaw_turns::TurnStatus;
use reborn_support::builder::RebornIntegrationHarness;
use reborn_support::doubles::ParkingCapabilityGate;
use reborn_support::reply::RebornScriptedReply;
use reborn_support::scripted_provider::ParkingModelGate;
use serde_json::json;

const HTTP_TOOL_URL: &str = "https://api.example.test/v1/items";

/// A wedged tool call (parked mid-dispatch, never released) outlives a
/// deliberately shortened test-only lease TTL well before its run's next
/// heartbeat is due, so the scheduler's real (test-shortened) lease-recovery
/// tick must reap it: `TurnStatus::Failed` with the `lease_expired` category,
/// not an unbounded hang.
#[tokio::test]
async fn wedged_tool_call_is_reaped_by_lease_expiry_not_left_running_forever() {
    let gate = ParkingCapabilityGate::new();
    let _guard = gate.release_guard();

    let harness = RebornIntegrationHarness::test_default()
        .with_live_shell()
        .park_tool_dispatch(gate.clone())
        .with_runner_lease_ttl_for_test(chrono::Duration::milliseconds(200))
        .with_lease_recovery_interval_for_test(Duration::from_millis(50))
        .script([RebornScriptedReply::tool_call(
            "builtin.http",
            json!({"url": HTTP_TOOL_URL}),
        )])
        .build()
        .await
        .expect("harness builds");

    let run_id = harness
        .submit_turn_async("fetch a url")
        .await
        .expect("turn submitted");

    tokio::time::timeout(Duration::from_secs(10), gate.wait_until_parked())
        .await
        .expect("tool dispatch parks before the timeout");

    // Never release: the tool call outlives its short, test-only lease.
    let state = tokio::time::timeout(
        Duration::from_secs(10),
        harness.wait_for_status(run_id, TurnStatus::Failed),
    )
    .await
    .expect("wedged run is reaped by lease-expiry recovery before the timeout")
    .expect("wedged run is reaped by lease-expiry recovery, not left Running forever");
    assert_eq!(
        state.failure.as_ref().map(|failure| failure.category()),
        Some("lease_expired"),
        "recovered run must be tagged with the lease_expired failure category"
    );
    assert!(
        state.checkpoint_id.is_some(),
        "a lease-expired possible side effect must retain its durable BeforeSideEffect checkpoint"
    );
    assert_eq!(
        gate.dispatch_count(),
        1,
        "lease recovery must not redispatch a possibly attempted tool call"
    );
}

/// The other side of the same boundary: a run whose lease expires while it is
/// parked *before a model call* has committed no external effect, so recovery
/// requeues it and it finishes. The user sees a normal answer, not a dead run.
///
/// The wedged-tool test above and this one differ only in where the run was
/// standing when its lease lapsed, which is exactly the distinction the
/// recorded checkpoint kind carries: `BeforeSideEffect` (there) still fails
/// closed, `BeforeModel` (here) is reclaimed after one full lease TTL of grace.
///
/// Reclaim must also fence the stale executor: the replacement never starts
/// while the abandoned worker is still alive (its heartbeat is definitively
/// rejected against the requeued process, which cancels it), so the resumed
/// run completing is itself proof the stale attempt was awaited — and its
/// "stale worker output" must never appear in the persisted reply.
#[tokio::test]
async fn run_parked_before_a_model_call_is_resumed_after_lease_expiry_not_failed() {
    let gate = ParkingModelGate::new();

    let harness = RebornIntegrationHarness::test_default()
        .park_model(gate.clone())
        .with_runner_lease_ttl_for_test(chrono::Duration::milliseconds(200))
        .with_lease_recovery_interval_for_test(Duration::from_millis(50))
        .script([
            // Consumed by the resumed run, which is the reply the user sees.
            RebornScriptedReply::text("recovered and finished"),
            // Would be consumed by the abandoned worker if it were ever
            // released to run again; the fence must prevent that.
            RebornScriptedReply::text("stale worker output"),
        ])
        .build()
        .await
        .expect("harness builds");

    let run_id = harness
        .submit_turn_async("answer a question")
        .await
        .expect("turn submitted");

    tokio::time::timeout(Duration::from_secs(10), gate.wait_until_parked())
        .await
        .expect("model call parks before the timeout");

    // Never released before recovery: the lease lapses under a worker that is
    // still holding the run, which is precisely the ambiguous case the grace
    // window exists to resolve. The resumed run completes only after the
    // supervisor has cancelled the stale attempt (the reclaim fence), so the
    // stale worker is awaited by construction.
    let state = tokio::time::timeout(
        Duration::from_secs(60),
        harness.wait_for_status(run_id, TurnStatus::Completed),
    )
    .await
    .expect("requeued run completes before the timeout")
    .expect("a before-model checkpoint is resumed, not failed");
    assert!(
        state.failure.is_none(),
        "the user must never see a failure for a run that was safely resumable, got {:?}",
        state.failure
    );

    harness
        .assert_reply_contains("recovered and finished")
        .await
        .expect("the resumed run's reply is the one persisted");

    // Release the abandoned worker as the incident would have. Two fences may
    // stop it, and which one fires is timing-dependent: the supervisor cancels
    // a stale executor at the first definitive lease-lost heartbeat, and if the
    // worker outruns that, the transcript port refuses its lease-fenced writes.
    // Either way nothing it produces may land. Give any delayed write path a
    // bounded window to land wrongly, then assert at the seams the user reads.
    gate.release();
    tokio::time::sleep(Duration::from_millis(500)).await;

    let state = harness
        .run_state(run_id)
        .await
        .expect("the recovered run's state is readable");
    assert_eq!(
        state.status,
        TurnStatus::Completed,
        "a stale worker must not move a run the journal already completed"
    );
    assert!(
        state.failure.is_none(),
        "a stale worker must not stamp a failure on a completed run, got {:?}",
        state.failure
    );
    // The transcript is the seam the user actually reads: a worker whose lease
    // the journal already reclaimed must not be able to append a second,
    // unattributed answer next to the recovered one.
    harness
        .assert_conversation_history_lacks("stale worker output")
        .await
        .expect("the stale worker's output must never reach the transcript");
    harness
        .assert_reply_contains("recovered and finished")
        .await
        .expect("the recovered reply survives the stale worker");
}

/// #7603 — the `BeforeModel` checkpoint written after a tool call is
/// load-bearing for automatic crash recovery. Do not batch it away.
///
/// This is the third case in the family above, and the one that is easy to
/// break. The run does two tool calls and is then stopped mid-flight during
/// the model call of iteration 2 — after a side effect, not before one.
///
/// Why it recovers today: iteration 2's `BeforeModel` write replaces
/// `BeforeSideEffect` as the newest checkpoint kind on the process row, and
/// that kind is exactly what `apply_recover_expired` reads to decide whether a
/// lease-expired run may be requeued (`replays_side_effect()` is `true` for
/// `BeforeSideEffect`, which also denies the grace window). Skip that write
/// and the marker never clears, so the scheduler fails the run closed instead
/// of resuming it.
///
/// That is not a hypothetical. A first cut of #7603 batched `BeforeModel`
/// purely on an iteration interval, and this test caught it:
///
/// ```text
/// expected Completed but run reached terminal status Failed;
/// failure=Some(SanitizedFailure { category: "lease_expired", detail: None })
/// ```
///
/// The fix constrains batching so it never leaves a `BeforeSideEffect` as the
/// newest checkpoint. If a future change reintroduces unconditional batching
/// — or relaxes the `replays_side_effect` / grace / `reclaimable` predicates —
/// this test fails, and it should. Removing the constraint safely means
/// tracking side-effect-outstanding explicitly on the process row: #7707.
///
/// What this asserts at the seams a user can observe:
///
/// - The run is reclaimed and finishes, rather than dying as `lease_expired`.
/// - Neither HTTP call is dispatched twice. Recovery continues past the
///   capability instead of replaying it, keeping the fail-closed side-effect
///   contract intact — there is no durable tool-idempotency table under it.
/// - Only the model call is redone.
#[tokio::test]
async fn run_interrupted_after_a_side_effect_still_auto_recovers_without_repeating_it() {
    // This scenario drives two tool calls plus a resumed run, so its future is
    // large enough to overflow libtest's default 2 MiB thread stack in debug
    // builds. Box it onto the heap; CI sets no `RUST_MIN_STACK`.
    Box::pin(run_interrupted_after_a_side_effect_body()).await;
}

async fn run_interrupted_after_a_side_effect_body() {
    const FIRST_URL: &str = "https://api.example.test/v1/batched/first";
    const SECOND_URL: &str = "https://api.example.test/v1/batched/second";

    // Iterations 0 and 1 each call a tool; iteration 2's model call is the one
    // that parks, so the run stops after a side effect rather than before one.
    let gate = ParkingModelGate::parking_call(2);

    let harness = RebornIntegrationHarness::test_default()
        .with_builtin_http_tools()
        .park_model(gate.clone())
        .with_runner_lease_ttl_for_test(chrono::Duration::milliseconds(200))
        .with_lease_recovery_interval_for_test(Duration::from_millis(50))
        .script([
            RebornScriptedReply::tool_call("builtin.http", json!({"url": FIRST_URL})),
            RebornScriptedReply::tool_call("builtin.http", json!({"url": SECOND_URL})),
            // Consumed by the resumed run: the reply the user actually sees.
            RebornScriptedReply::text("recovered and finished"),
            // Only reachable by the abandoned worker, which must never land.
            RebornScriptedReply::text("stale worker output"),
        ])
        .build()
        .await
        .expect("harness builds");

    let run_id = harness
        .submit_turn_async("fetch both urls then answer")
        .await
        .expect("turn submitted");

    tokio::time::timeout(Duration::from_secs(10), gate.wait_until_parked())
        .await
        .expect("the third model call parks before the timeout");

    let state = tokio::time::timeout(
        Duration::from_secs(60),
        harness.wait_for_status(run_id, TurnStatus::Completed),
    )
    .await
    .expect("the requeued run completes before the timeout")
    .expect(
        "a run stopped after a tool call must stay auto-recoverable; \
         if this is `lease_expired`, the post-side-effect BeforeModel \
         checkpoint was batched away (see #7603/#7707)",
    );
    assert!(
        state.failure.is_none(),
        "a run whose newest checkpoint is a post-side-effect BeforeModel is resumable, got {:?}",
        state.failure
    );

    harness
        .assert_reply_contains("recovered and finished")
        .await
        .expect("the resumed run's reply is the one persisted");

    // The core safety property: the resume replayed a model call, not a side
    // effect. Two scripted tool calls, two dispatches — not three.
    harness
        .assert_egress_count(2)
        .await
        .expect("resuming across a batched-away checkpoint must not re-dispatch a tool call");
    harness
        .assert_egress_url_order(&[FIRST_URL, SECOND_URL])
        .await
        .expect("each side effect happened exactly once, in order");
    harness
        .assert_tool_invocation_count("builtin.http", 2)
        .await
        .expect("the capability was invoked once per scripted call");

    // Release the abandoned worker as the incident would have, then re-assert
    // at the seams the user reads.
    gate.release();
    tokio::time::sleep(Duration::from_millis(500)).await;

    harness
        .assert_egress_count(2)
        .await
        .expect("the stale worker must not dispatch a third request");
    harness
        .assert_conversation_history_lacks("stale worker output")
        .await
        .expect("the stale worker's output must never reach the transcript");
}
