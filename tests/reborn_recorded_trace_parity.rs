#[allow(dead_code)]
#[path = "support/reborn/mod.rs"]
mod reborn_support;
mod support;

use ironclaw_loop_support::{HostManagedModelMessageRole, HostManagedModelResponse};
use ironclaw_turns::{TurnStatus, run_profile::LoopHostMilestoneKind};
use reborn_support::{
    harness::{
        RebornBinaryE2EHarness, RecordingTestCapabilityPort, assert_milestone_order,
        trace_tool_call_response,
    },
    model_replay::RebornTraceReplayModelGateway,
};

#[tokio::test]
async fn reborn_recorded_trace_parity() {
    let model_gateway = RebornTraceReplayModelGateway::with_responses([
        trace_tool_call_response(),
        HostManagedModelResponse::assistant_reply("trace final reply"),
    ]);
    let mut harness = RebornBinaryE2EHarness::with_model_gateway(
        "room-recorded-trace",
        model_gateway,
        RecordingTestCapabilityPort::echo(),
    )
    .await
    .expect("harness");
    harness.start();

    let submitted = harness
        .submit_text("event-recorded-trace", "use a tool")
        .await
        .expect("submit text");
    harness
        .wait_for_status(submitted.run_id, TurnStatus::Completed)
        .await
        .expect("completed run");
    harness
        .assert_final_reply("trace final reply")
        .await
        .expect("final reply");

    assert_eq!(harness.model_requests().len(), 2);
    assert_eq!(harness.remaining_model_responses(), 0);
    assert_eq!(harness.capability_invocations().len(), 1);
    let requests = harness.model_requests();
    assert!(
        requests[1].messages.iter().any(|message| message.role
            == HostManagedModelMessageRole::ToolResult
            && message.content.contains("result:test-echo-1")),
        "tool result should be visible to the follow-up model call"
    );
    assert_milestone_order(
        &harness.milestones(),
        |kind| matches!(kind, LoopHostMilestoneKind::CapabilityBatchCompleted { .. }),
        |kind| matches!(kind, LoopHostMilestoneKind::AssistantReplyFinalized { .. }),
    );

    harness.shutdown().await;
}
