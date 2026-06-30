#[allow(dead_code)]
#[path = "support/reborn/mod.rs"]
mod reborn_support;
mod support;

use ironclaw_host_api::CapabilityId;
use ironclaw_host_runtime::READ_FILE_CAPABILITY_ID;
use ironclaw_turns::{TurnStatus, run_profile::LoopHostMilestoneKind};
use reborn_support::{
    harness::RebornBinaryE2EHarness,
    model_replay::{
        RebornModelReplayStep, RebornScriptedProviderToolCall, RebornTraceReplayModelGateway,
    },
};

/// Exercises read_file with a missing `path` parameter, proving malformed real
/// built-in tool input is persisted as a terminal Reborn run failure.
#[tokio::test]
async fn reborn_trace_error_path_parity() {
    let read_file = CapabilityId::new(READ_FILE_CAPABILITY_ID).expect("valid capability id");
    let model_gateway = RebornTraceReplayModelGateway::with_scripted_steps([
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![RebornScriptedProviderToolCall::new(
                read_file.clone(),
                "call_read_file_missing_path",
                serde_json::json!({}),
            )],
            expected_tool_results: Vec::new(),
        },
    ]);
    let mut harness = RebornBinaryE2EHarness::with_host_runtime_file_capabilities(
        "room-trace-error-path",
        model_gateway,
    )
    .await
    .expect("harness");
    harness.start();

    let submitted = harness
        .submit_text("event-trace-error-path", "Read a file for me")
        .await
        .expect("submit text");
    let state = harness
        .wait_for_status(submitted.run_id, TurnStatus::Failed)
        .await
        .expect("failed run");
    assert_eq!(
        state.failure.expect("failure category").category(),
        "driver_protocol_violation"
    );

    let invocations = harness.capability_invocations();
    assert_eq!(invocations.len(), 1);
    assert_eq!(invocations[0].capability_id, read_file);

    let requests = harness.model_requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(harness.remaining_model_responses(), 0);
    assert!(harness.milestones().iter().any(|milestone| matches!(
        milestone.kind,
        LoopHostMilestoneKind::CapabilityBatchCompleted { .. }
    )));
    assert!(
        !harness.milestones().iter().any(|milestone| matches!(
            milestone.kind,
            LoopHostMilestoneKind::AssistantReplyFinalized { .. }
        )),
        "invalid tool input should not fabricate a final assistant reply"
    );

    harness.shutdown().await;
}

/// Exercises the model replay guard for scripted calls whose capability was not
/// advertised by the active surface. The harness exposes only write_file, then
/// the trace asks for read_file, so `provider_tool_calls_response` must fail
/// before any provider tool call is registered or invoked.
#[tokio::test]
async fn reborn_trace_unadvertised_capability_is_rejected() {
    let read_file = CapabilityId::new(READ_FILE_CAPABILITY_ID).expect("valid capability id");
    let model_gateway = RebornTraceReplayModelGateway::with_scripted_steps([
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![RebornScriptedProviderToolCall::new(
                read_file,
                "call_read_file_unadvertised",
                serde_json::json!({
                    "path": "/workspace/should-not-be-visible.txt",
                }),
            )],
            expected_tool_results: Vec::new(),
        },
    ]);
    let mut harness = RebornBinaryE2EHarness::with_host_runtime_write_only(
        "room-trace-unadvertised-capability",
        model_gateway,
    )
    .await
    .expect("harness");
    harness.start();

    let submitted = harness
        .submit_text("event-trace-unadvertised-capability", "Read a file for me")
        .await
        .expect("submit text");
    let state = harness
        .wait_for_status(submitted.run_id, TurnStatus::Failed)
        .await
        .expect("failed run");
    assert_eq!(
        state.failure.expect("failure category").category(),
        "driver_unavailable"
    );

    assert!(
        harness.capability_invocations().is_empty(),
        "unadvertised capability should fail before invocation"
    );
    assert_eq!(harness.remaining_model_responses(), 0);
    assert!(
        !harness.milestones().iter().any(|milestone| matches!(
            milestone.kind,
            LoopHostMilestoneKind::CapabilityBatchCompleted { .. }
        )),
        "unadvertised capability should fail before capability batch execution"
    );

    harness.shutdown().await;
}
