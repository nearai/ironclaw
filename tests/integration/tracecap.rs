//! C-TRACECAP: `turn_event_sink` int-tier coverage.
//!
//! Wires `ironclaw_turns::InMemoryTurnEventSink` into the harness's planned
//! runtime via `.with_turn_event_sink()`, proving production's
//! `lifecycle_bus.subscribe_best_effort` seam actually publishes turn-lifecycle
//! events. Distinct from T0-SYSPROMPT (captured model requests) and
//! `ironclaw_recorded_trace_parity.rs` (recorded-response replay).

#[allow(dead_code)]
#[path = "support/mod.rs"]
mod ironclaw_support;
#[allow(dead_code)]
#[path = "../support/mod.rs"]
mod support;

use ironclaw_support::builder::IronClawIntegrationHarness;
use ironclaw_support::group::IronClawIntegrationGroup;
use ironclaw_support::reply::IronClawScriptedReply;
use ironclaw_turns::TurnEventKind;

#[tokio::test]
async fn turn_event_sink_receives_completed_event_for_a_finished_turn() {
    let harness = IronClawIntegrationHarness::test_default()
        .with_turn_event_sink()
        .script([IronClawScriptedReply::text("done")])
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

/// Negative control: without `.with_turn_event_sink()`, no sink is installed and
/// no events are recorded — proves the assertion discriminates on real wiring.
#[tokio::test]
async fn no_events_recorded_without_opting_in() {
    let harness = IronClawIntegrationHarness::test_default()
        .script([IronClawScriptedReply::text("done")])
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

/// Regression: the sink is shared across every thread in a group, so
/// `assert_turn_event_recorded` must slice `[baseline_turn_event_count..]` —
/// a second thread built after the first's Completed event must not see it.
#[tokio::test]
async fn group_thread_does_not_see_a_sibling_threads_turn_event() {
    let group = IronClawIntegrationGroup::builder()
        .with_turn_event_sink()
        .builtin_tools()
        .await
        .expect("builtin-tools group builds");

    let first = group
        .thread("conv-tracecap-first")
        .script([IronClawScriptedReply::text("done")])
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

    // Built after `first` completed, so the shared sink already holds its
    // Completed event ahead of this thread's baseline.
    let second = group
        .thread("conv-tracecap-second")
        .script([IronClawScriptedReply::text("unused")])
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
