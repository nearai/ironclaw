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
        .with_builtin_http_tools()
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
}
