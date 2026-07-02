//! C-TRACECAP: `turn_event_sink` int-tier coverage (rev-3 Tier-2, A1 audit).
//!
//! Production wires a best-effort turn-lifecycle sink via
//! `lifecycle_bus.subscribe_best_effort(sink)` in
//! `build_default_planned_runtime_inner` (`crates/ironclaw_reborn/src/runtime.rs:613-619`),
//! fed in real deployments by `CompositeTurnEventSink` over
//! `[TraceCaptureTurnEventSink, ..]` (`crates/ironclaw_reborn_composition/src/runtime.rs:3229-3290`)
//! — the entry point to the 0%-covered `ironclaw_reborn_traces` crate. That
//! seam was never exercised by any Reborn test: `DefaultPlannedRuntimeParts.turn_event_sink`
//! was `None` in every harness/group construction.
//!
//! This test wires `ironclaw_turns::InMemoryTurnEventSink` — a real, already-shipped
//! production `TurnEventSink` impl with zero callers anywhere in the codebase
//! today — into the harness's planned runtime via `.with_turn_event_sink()`, and
//! proves `subscribe_best_effort` actually publishes to it for a real completed
//! turn. Distinct from T0-SYSPROMPT's `TraceLlm` captured model requests (a
//! different seam: what the model saw, not what the turn coordinator published)
//! and from `reborn_recorded_trace_parity.rs` (recorded-response replay).

#[allow(dead_code)]
#[path = "support/reborn/mod.rs"]
mod reborn_support;
#[allow(dead_code)]
mod support;

use ironclaw_turns::TurnEventKind;
use reborn_support::builder::RebornIntegrationHarness;
use reborn_support::group::RebornIntegrationGroup;
use reborn_support::reply::RebornScriptedReply;

#[tokio::test]
async fn turn_event_sink_receives_completed_event_for_a_finished_turn() {
    let harness = RebornIntegrationHarness::test_default()
        .with_turn_event_sink()
        .script([RebornScriptedReply::text("done")])
        .build()
        .await
        .expect("harness builds");

    harness
        .submit_turn("do something")
        .await
        .expect("turn completes");

    harness
        .assert_turn_event_recorded(TurnEventKind::Completed)
        .await
        .expect("turn-lifecycle sink recorded the Completed event for the finished turn");
}

/// Negative control: a harness that never calls `.with_turn_event_sink()` has no
/// sink installed, so `DefaultPlannedRuntimeParts.turn_event_sink` stays `None`
/// (matching every pre-existing reborn test) and no events are recorded. Proves
/// the assertion is discriminating on real wiring, not a tautology.
#[tokio::test]
async fn no_events_recorded_without_opting_in() {
    let harness = RebornIntegrationHarness::test_default()
        .script([RebornScriptedReply::text("done")])
        .build()
        .await
        .expect("harness builds");

    harness
        .submit_turn("do something")
        .await
        .expect("turn completes");

    let err = harness
        .assert_turn_event_recorded(TurnEventKind::Completed)
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("no recorded turn event of kind"),
        "expected error about missing turn event, got: {err}"
    );
}

/// Regression: the sink is shared across every thread in a `.with_turn_event_sink()`
/// group (it has no per-thread channel), so `assert_turn_event_recorded` must slice
/// `[baseline_turn_event_count..]` to see only THIS thread's events. Drives a first
/// thread to Completed, then builds a second thread AFTER that (so its baseline
/// already includes the first thread's event) and never submits a turn on it. If
/// the baseline slice were missing, the second thread's assertion would incorrectly
/// pass on the first thread's Completed event.
#[tokio::test]
async fn group_thread_does_not_see_a_sibling_threads_turn_event() {
    let group = RebornIntegrationGroup::builder()
        .with_turn_event_sink()
        .builtin_tools()
        .await
        .expect("builtin-tools group builds");

    let first = group
        .thread("conv-tracecap-first")
        .script([RebornScriptedReply::text("done")])
        .build()
        .await
        .expect("first thread builds");
    first
        .submit_turn("do something")
        .await
        .expect("first turn completes");
    first
        .assert_turn_event_recorded(TurnEventKind::Completed)
        .await
        .expect("first thread recorded its own Completed event");

    // Built after `first` completed, so the shared sink already holds `first`'s
    // Completed event ahead of this thread's baseline.
    let second = group
        .thread("conv-tracecap-second")
        .script([RebornScriptedReply::text("unused")])
        .build()
        .await
        .expect("second thread builds");

    let err = second
        .assert_turn_event_recorded(TurnEventKind::Completed)
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("no recorded turn event of kind"),
        "second thread never submitted a turn, so it must not see the first \
         thread's Completed event; got: {err}"
    );
}
