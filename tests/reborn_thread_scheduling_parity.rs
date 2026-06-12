#[allow(dead_code)]
#[path = "support/reborn/mod.rs"]
mod reborn_support;
mod support;

use ironclaw_loop_support::HostManagedModelResponse;
use ironclaw_turns::TurnStatus;
use reborn_support::{
    assertions::{
        assert_completed_lifecycle, assert_history_contains_assistant,
        assert_history_contains_user, assert_history_excludes,
    },
    events::turn_event_snapshot,
    harness::{RebornBinaryE2EHarness, RecordingTestCapabilityPort},
    model_replay::RebornTraceReplayModelGateway,
};

#[tokio::test]
async fn reborn_thread_scheduling_multi_turn_state() {
    let model_gateway = RebornTraceReplayModelGateway::with_responses([
        HostManagedModelResponse::assistant_reply("remembered first turn"),
        HostManagedModelResponse::assistant_reply("remembered second turn"),
    ]);
    let mut harness = RebornBinaryE2EHarness::with_model_gateway(
        "room-thread-scheduling",
        model_gateway,
        RecordingTestCapabilityPort::echo(),
    )
    .await
    .expect("harness");
    harness.start();

    let first = harness
        .submit_text("event-thread-scheduling-1", "first turn")
        .await
        .expect("submit first turn");
    harness
        .wait_for_status(first.run_id, TurnStatus::Completed)
        .await
        .expect("first turn completed");

    let second = harness
        .submit_text("event-thread-scheduling-2", "second turn")
        .await
        .expect("submit second turn");
    harness
        .wait_for_status(second.run_id, TurnStatus::Completed)
        .await
        .expect("second turn completed");

    assert_eq!(
        first.thread_id, second.thread_id,
        "same external conversation should schedule follow-up turns on one canonical thread"
    );
    assert_ne!(
        first.run_id, second.run_id,
        "distinct inbound events should create distinct turn runs"
    );

    let history = harness
        .history_for_submitted_thread(&second)
        .await
        .expect("thread history");
    assert_history_contains_user(&history, "first turn");
    assert_history_contains_assistant(&history, "remembered first turn");
    assert_history_contains_user(&history, "second turn");
    assert_history_contains_assistant(&history, "remembered second turn");

    let first_events = turn_event_snapshot(&harness, &first)
        .await
        .expect("first turn event snapshot");
    let second_events = turn_event_snapshot(&harness, &second)
        .await
        .expect("second turn event snapshot");
    assert_completed_lifecycle(&first_events.entries);
    assert_completed_lifecycle(&second_events.entries);
    assert_eq!(harness.model_requests().len(), 2);
    harness.assert_model_exhausted();

    harness.shutdown().await;
}

#[tokio::test]
async fn reborn_thread_scheduling_drains_queued_distinct_threads() {
    let model_gateway = RebornTraceReplayModelGateway::with_responses([
        HostManagedModelResponse::assistant_reply("alpha queued reply"),
        HostManagedModelResponse::assistant_reply("beta queued reply"),
    ]);
    let mut harness = RebornBinaryE2EHarness::with_model_gateway_unscoped_worker(
        "room-scheduler-alpha",
        model_gateway,
        RecordingTestCapabilityPort::echo(),
    )
    .await
    .expect("harness");
    harness.start();

    let alpha = harness
        .submit_text_for(
            "room-scheduler-alpha",
            "alice",
            "event-scheduler-alpha",
            "alpha queued turn",
        )
        .await
        .expect("submit alpha turn");
    let beta = harness
        .submit_text_for(
            "room-scheduler-beta",
            "alice",
            "event-scheduler-beta",
            "beta queued turn",
        )
        .await
        .expect("submit beta turn");

    harness
        .wait_for_submitted_status(&alpha, TurnStatus::Completed)
        .await
        .expect("alpha completed");
    harness
        .wait_for_submitted_status(&beta, TurnStatus::Completed)
        .await
        .expect("beta completed");

    assert_ne!(
        alpha.thread_id, beta.thread_id,
        "different external conversations should schedule onto isolated threads"
    );

    let alpha_history = harness
        .history_for_submitted_thread(&alpha)
        .await
        .expect("alpha history");
    let beta_history = harness
        .history_for_submitted_thread(&beta)
        .await
        .expect("beta history");
    assert_history_contains_user(&alpha_history, "alpha queued turn");
    assert_history_excludes(&alpha_history, "beta queued turn");
    assert_history_contains_user(&beta_history, "beta queued turn");
    assert_history_excludes(&beta_history, "alpha queued turn");

    let alpha_events = turn_event_snapshot(&harness, &alpha)
        .await
        .expect("alpha turn event snapshot");
    let beta_events = turn_event_snapshot(&harness, &beta)
        .await
        .expect("beta turn event snapshot");
    assert_completed_lifecycle(&alpha_events.entries);
    assert_completed_lifecycle(&beta_events.entries);
    assert_eq!(harness.model_requests().len(), 2);
    harness.assert_model_exhausted();

    harness.shutdown().await;
}
