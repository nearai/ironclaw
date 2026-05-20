#[allow(dead_code)]
#[path = "support/reborn/mod.rs"]
mod reborn_support;
mod support;

use ironclaw_loop_support::HostManagedModelResponse;
use ironclaw_turns::{TurnStatus, run_profile::LoopHostMilestoneKind};
use reborn_support::{
    harness::{
        RebornBinaryE2EHarness, RecordingTestCapabilityPort, assert_milestone_order,
        trace_tool_call_response,
    },
    model_replay::RebornTraceReplayModelGateway,
};

#[tokio::test]
async fn reborn_approval_traces_parity() {
    let model_gateway = RebornTraceReplayModelGateway::with_responses([
        trace_tool_call_response(),
        HostManagedModelResponse::assistant_reply("approval resumed reply"),
    ]);
    let mut harness = RebornBinaryE2EHarness::with_harness_blocked_evidence(
        "room-approval-trace",
        model_gateway,
        RecordingTestCapabilityPort::approval_then_echo(),
    )
    .await
    .expect("harness");
    harness.start();

    let submitted = harness
        .submit_text("event-approval-trace", "needs approval")
        .await
        .expect("submit text");
    let blocked = harness
        .wait_for_status(submitted.run_id, TurnStatus::BlockedApproval)
        .await
        .expect("blocked approval");
    assert!(
        blocked.gate_ref.is_some(),
        "blocked run should expose gate ref"
    );
    assert!(
        harness
            .run_state(submitted.run_id)
            .await
            .expect("run state")
            .status
            != TurnStatus::Completed,
        "blocked run must not complete before resume"
    );

    harness
        .resume_blocked_turn(submitted.run_id)
        .await
        .expect("resume");
    harness
        .wait_for_status(submitted.run_id, TurnStatus::Completed)
        .await
        .expect("completed after resume");
    harness
        .assert_final_reply("approval resumed reply")
        .await
        .expect("final reply");
    assert_eq!(harness.capability_invocations().len(), 1);
    assert_milestone_order(
        &harness.milestones(),
        |kind| matches!(kind, LoopHostMilestoneKind::GateBlocked { .. }),
        |kind| matches!(kind, LoopHostMilestoneKind::AssistantReplyFinalized { .. }),
    );

    harness.shutdown().await;
}

#[tokio::test]
async fn reborn_approval_cancel_traces_parity() {
    let model_gateway = RebornTraceReplayModelGateway::with_responses([
        trace_tool_call_response(),
        HostManagedModelResponse::assistant_reply("must not be emitted after cancel"),
    ]);
    let mut harness = RebornBinaryE2EHarness::with_harness_blocked_evidence(
        "room-approval-cancel-trace",
        model_gateway,
        RecordingTestCapabilityPort::approval_then_echo(),
    )
    .await
    .expect("harness");
    harness.start();

    let submitted = harness
        .submit_text("event-approval-cancel-trace", "needs approval then cancel")
        .await
        .expect("submit text");
    let blocked = harness
        .wait_for_status(submitted.run_id, TurnStatus::BlockedApproval)
        .await
        .expect("blocked approval");
    assert!(
        blocked.gate_ref.is_some(),
        "blocked run should expose gate ref before cancellation"
    );

    harness
        .cancel_blocked_turn(submitted.run_id)
        .await
        .expect("cancel blocked approval run");
    harness
        .wait_for_status(submitted.run_id, TurnStatus::Cancelled)
        .await
        .expect("cancelled after approval block");

    let state = harness
        .run_state(submitted.run_id)
        .await
        .expect("cancelled run state");
    assert_eq!(state.status, TurnStatus::Cancelled);
    assert_eq!(harness.capability_invocations().len(), 1);
    assert_eq!(harness.model_requests().len(), 1);
    assert_eq!(harness.remaining_model_responses(), 1);
    assert!(
        !harness.milestones().iter().any(|milestone| matches!(
            milestone.kind,
            LoopHostMilestoneKind::AssistantReplyFinalized { .. }
        )),
        "cancelled approval run must not finalize an assistant reply"
    );
    assert!(
        harness
            .milestones()
            .iter()
            .any(|milestone| matches!(milestone.kind, LoopHostMilestoneKind::GateBlocked { .. })),
        "cancelled approval run should still record gate-blocked evidence"
    );

    harness.shutdown().await;
}
