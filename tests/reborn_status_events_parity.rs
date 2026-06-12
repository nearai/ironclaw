#[allow(dead_code)]
#[path = "support/reborn/mod.rs"]
mod reborn_support;
mod support;

use ironclaw_host_api::CapabilityId;
use ironclaw_loop_support::{HostManagedModelMessageRole, HostManagedModelResponse};
use ironclaw_turns::{TurnEventKind, TurnStatus, run_profile::LoopHostMilestoneKind};
use reborn_support::{
    assertions::assert_event_order,
    events::{turn_event_snapshot, turn_event_updates},
    harness::{RebornBinaryE2EHarness, RecordingTestCapabilityPort, assert_milestone_order},
    model_replay::{
        RebornModelReplayStep, RebornScriptedProviderToolCall, RebornTraceReplayModelGateway,
    },
};

const TEST_CAPABILITY_ID: &str = "test.echo";

#[tokio::test]
async fn reborn_status_events_parity() {
    let test_echo = CapabilityId::new(TEST_CAPABILITY_ID).expect("valid capability id");
    let model_gateway = RebornTraceReplayModelGateway::with_scripted_steps([
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![
                RebornScriptedProviderToolCall::new(
                    test_echo.clone(),
                    "call-status-one",
                    serde_json::json!({"message": "one"}),
                ),
                RebornScriptedProviderToolCall::new(
                    test_echo.clone(),
                    "call-status-two",
                    serde_json::json!({"message": "two"}),
                ),
                RebornScriptedProviderToolCall::new(
                    test_echo.clone(),
                    "call-status-three",
                    serde_json::json!({"message": "three"}),
                ),
            ],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::Response {
            response: HostManagedModelResponse::assistant_reply("status events complete"),
            expected_tool_results: Vec::new(),
        },
    ]);
    let mut harness = RebornBinaryE2EHarness::with_model_gateway(
        "room-status-events",
        model_gateway,
        RecordingTestCapabilityPort::echo(),
    )
    .await
    .expect("harness");
    harness.start();

    let submitted = harness
        .submit_text("event-status-events", "run three status event tools")
        .await
        .expect("submit text");
    harness
        .wait_for_status(submitted.run_id, TurnStatus::Completed)
        .await
        .expect("completed run");
    harness
        .assert_final_reply("status events complete")
        .await
        .expect("final reply");

    let snapshot = turn_event_snapshot(&harness, &submitted)
        .await
        .expect("turn event snapshot");
    assert!(
        !snapshot.truncated,
        "status event snapshot should not truncate"
    );
    assert_event_order(
        &snapshot.entries,
        &[
            TurnEventKind::Submitted,
            TurnEventKind::RunnerClaimed,
            TurnEventKind::Completed,
        ],
    );
    assert!(
        snapshot
            .entries
            .iter()
            .all(|event| event.run_id == submitted.run_id),
        "projection should only include events for the submitted run"
    );
    assert!(
        snapshot
            .entries
            .iter()
            .any(|event| event.kind == TurnEventKind::Completed
                && event.status == TurnStatus::Completed),
        "completed lifecycle event should carry completed run status"
    );

    let empty_update = turn_event_updates(&harness, &submitted, Some(snapshot.next_cursor), 100)
        .await
        .expect("turn event update after cursor");
    assert!(
        empty_update.entries.is_empty(),
        "event projection cursor should not replay already-drained events"
    );

    let invocations = harness.capability_invocations();
    assert_eq!(invocations.len(), 3);
    assert!(
        invocations
            .iter()
            .all(|invocation| invocation.capability_id == test_echo)
    );

    let requests = harness.model_requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(tool_result_count(&requests[1]), 3);
    assert_milestone_order(
        &harness.milestones(),
        |kind| matches!(kind, LoopHostMilestoneKind::CapabilityBatchCompleted { .. }),
        |kind| matches!(kind, LoopHostMilestoneKind::AssistantReplyFinalized { .. }),
    );

    harness.shutdown().await;
}

fn tool_result_count(request: &ironclaw_loop_support::HostManagedModelRequest) -> usize {
    request
        .messages
        .iter()
        .filter(|message| message.role == HostManagedModelMessageRole::ToolResult)
        .count()
}
