#[allow(dead_code)]
#[path = "support/reborn_parity_qa/mod.rs"]
mod parity_qa_support;
#[allow(dead_code)]
#[path = "integration/support/mod.rs"]
mod reborn_support;
mod support;

use ironclaw_host_api::CapabilityId;
use ironclaw_host_runtime::READ_FILE_CAPABILITY_ID;
use ironclaw_loop_host::{HostManagedModelMessageRole, HostManagedModelResponse};
use ironclaw_turns::{TurnStatus, run_profile::LoopHostMilestoneKind};
use parity_qa_support::{
    binary_e2e::RebornBinaryE2EHarness,
    model_replay::{
        RebornModelReplayStep, RebornScriptedProviderToolCall, RebornTraceReplayModelGateway,
    },
};
use reborn_support::doubles::RecordingTestCapabilityPort;

/// Exercises read_file with a missing `path` parameter, proving malformed real
/// built-in tool input is returned to the model and the turn can recover.
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
        RebornModelReplayStep::Response {
            response: HostManagedModelResponse::assistant_reply(
                "I could not read the file because its path was missing.",
            ),
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
        .wait_for_status(submitted.run_id, TurnStatus::Completed)
        .await
        .expect("completed run");
    assert!(state.failure.is_none());

    let invocations = harness.capability_invocations();
    assert_eq!(invocations.len(), 1);
    assert_eq!(invocations[0].capability_id, read_file);

    let requests = harness.model_requests();
    assert_eq!(requests.len(), 2);
    assert!(
        requests[1].messages.iter().any(|message| {
            message.role == HostManagedModelMessageRole::ToolResult
                && message.content.contains("path")
        }),
        "the retrying model turn must observe why read_file input was invalid"
    );
    assert_eq!(harness.remaining_model_responses(), 0);
    assert!(harness.milestones().iter().any(|milestone| matches!(
        milestone.kind,
        LoopHostMilestoneKind::CapabilityBatchCompleted { .. }
    )));
    assert!(
        harness.milestones().iter().any(|milestone| matches!(
            milestone.kind,
            LoopHostMilestoneKind::AssistantReplyFinalized { .. }
        )),
        "the model's recovery reply should be finalized"
    );

    harness.shutdown().await;
}

/// A model-actionable `InvalidInput` remains inside the same run: the immediate
/// next model request receives the typed failure, the model changes its input,
/// the corrected call succeeds, and only then is the final reply persisted.
#[tokio::test]
async fn reborn_trace_invalid_input_recovers_with_changed_action() {
    let echo = CapabilityId::new("test.echo").expect("valid capability id");
    let model_gateway = RebornTraceReplayModelGateway::with_scripted_steps([
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![RebornScriptedProviderToolCall::new(
                echo.clone(),
                "call_echo_invalid_input",
                serde_json::json!({"message": ""}),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::ProviderToolCallsForRequest {
            request_contains: "invalid_input".to_string(),
            calls: vec![RebornScriptedProviderToolCall::new(
                echo.clone(),
                "call_echo_corrected_input",
                serde_json::json!({"message": "corrected"}),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::ResponseForRequest {
            request_contains: "echo: hi".to_string(),
            response: HostManagedModelResponse::assistant_reply(
                "I corrected the invalid input and completed the call.",
            ),
            expected_tool_results: Vec::new(),
        },
    ]);
    let mut harness = RebornBinaryE2EHarness::with_model_gateway(
        "room-trace-invalid-input-recovery",
        model_gateway,
        RecordingTestCapabilityPort::invalid_input_then_echo(),
    )
    .await
    .expect("harness");
    harness.start();

    let submitted = harness
        .submit_text(
            "event-trace-invalid-input-recovery",
            "Call the test capability and correct invalid input if needed",
        )
        .await
        .expect("submit text");
    let state = harness
        .wait_for_status(submitted.run_id, TurnStatus::Completed)
        .await
        .expect("invalid input remains model-recoverable");
    assert!(state.failure.is_none());
    harness
        .assert_final_reply("I corrected the invalid input and completed the call.")
        .await
        .expect("recovery reply persisted");

    let invocations = harness.capability_invocations();
    assert_eq!(invocations.len(), 2);
    assert!(
        invocations
            .iter()
            .all(|invocation| invocation.capability_id == echo)
    );
    let requests = harness.model_requests();
    assert_eq!(requests.len(), 3);
    let recovery_request = serde_json::to_string(&requests[1]).expect("request serializes");
    for expected in ["invalid_input", "capability input failed validation"] {
        assert!(
            recovery_request.contains(expected),
            "recovery request must contain {expected:?}: {recovery_request}"
        );
    }
    harness.assert_model_exhausted();

    harness.shutdown().await;
}

/// Exercises the model replay guard for scripted calls whose capability was not
/// advertised by the active surface. The harness exposes only write_file, then
/// the trace asks for read_file, so `provider_tool_calls_response` must reject
/// the model output before registration and retry with corrective context.
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
        RebornModelReplayStep::Response {
            response: HostManagedModelResponse::assistant_reply(
                "I cannot call read_file because it is not available in this turn.",
            ),
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
        .wait_for_status(submitted.run_id, TurnStatus::Completed)
        .await
        .expect("completed run");
    assert!(state.failure.is_none());

    assert!(
        harness.capability_invocations().is_empty(),
        "unadvertised capability should fail before invocation"
    );
    assert_eq!(harness.remaining_model_responses(), 0);
    let requests = harness.model_requests();
    assert_eq!(requests.len(), 2);
    assert!(
        requests[1].messages.iter().any(|message| {
            message.role == HostManagedModelMessageRole::System
                && message.content.contains("structurally invalid")
                && message.content.contains("available tool")
        }),
        "the recovery turn must receive the invalid-output repair instruction"
    );
    assert!(
        !harness.milestones().iter().any(|milestone| matches!(
            milestone.kind,
            LoopHostMilestoneKind::CapabilityBatchCompleted { .. }
        )),
        "unadvertised capability should fail before capability batch execution"
    );

    harness.shutdown().await;
}
