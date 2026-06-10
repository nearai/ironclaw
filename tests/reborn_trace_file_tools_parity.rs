#[allow(dead_code)]
#[path = "support/reborn/mod.rs"]
mod reborn_support;
mod support;

use ironclaw_host_api::CapabilityId;
use ironclaw_host_runtime::{READ_FILE_CAPABILITY_ID, WRITE_FILE_CAPABILITY_ID};
use ironclaw_loop_support::{HostManagedModelMessageRole, HostManagedModelResponse};
use ironclaw_turns::{TurnStatus, run_profile::LoopHostMilestoneKind};
use reborn_support::{
    harness::{RebornBinaryE2EHarness, assert_milestone_order},
    model_replay::{
        RebornModelReplayStep, RebornScriptedProviderToolCall, RebornTraceReplayModelGateway,
    },
};

const EXPECTED_CONTENT: &str = "Hello, E2E test!";

#[tokio::test]
async fn reborn_trace_file_tools_parity() {
    let write_file = CapabilityId::new(WRITE_FILE_CAPABILITY_ID).expect("valid capability id");
    let read_file = CapabilityId::new(READ_FILE_CAPABILITY_ID).expect("valid capability id");
    let model_gateway = RebornTraceReplayModelGateway::with_scripted_steps([
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![RebornScriptedProviderToolCall::new(
                write_file.clone(),
                "call_write_file_1",
                serde_json::json!({
                    "path": "/workspace/generated/hello.txt",
                    "content": EXPECTED_CONTENT,
                }),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![RebornScriptedProviderToolCall::new(
                read_file.clone(),
                "call_read_file_1",
                serde_json::json!({
                    "path": "/workspace/generated/hello.txt",
                }),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::Response {
            response: HostManagedModelResponse::assistant_reply("file trace complete"),
            expected_tool_results: Vec::new(),
        },
    ]);
    let mut harness = RebornBinaryE2EHarness::with_host_runtime_file_capabilities(
        "room-trace-file-tools",
        model_gateway,
    )
    .await
    .expect("harness");
    harness.start();

    let submitted = harness
        .submit_text("event-trace-file-tools", "write the greeting file")
        .await
        .expect("submit text");
    harness
        .wait_for_status(submitted.run_id, TurnStatus::Completed)
        .await
        .expect("completed run");
    harness
        .assert_final_reply("file trace complete")
        .await
        .expect("final reply");

    let written_path = harness
        .host_workspace_file_path("generated/hello.txt")
        .expect("host workspace path");
    let file_content = std::fs::read_to_string(&written_path).expect("written file");
    assert_eq!(file_content, EXPECTED_CONTENT);

    let invocations = harness.capability_invocations();
    assert_eq!(invocations.len(), 2);
    assert_eq!(invocations[0].capability_id, write_file);
    assert_eq!(invocations[1].capability_id, read_file);

    let requests = harness.model_requests();
    assert_eq!(requests.len(), 3);
    assert!(
        requests[1].messages.iter().any(|message| message.role
            == HostManagedModelMessageRole::ToolResult
            && message.content.contains("result:")),
        "tool result ref should be visible to the follow-up model call"
    );
    assert_milestone_order(
        &harness.milestones(),
        |kind| matches!(kind, LoopHostMilestoneKind::CapabilityBatchCompleted { .. }),
        |kind| matches!(kind, LoopHostMilestoneKind::AssistantReplyFinalized { .. }),
    );
    harness.assert_model_exhausted();

    harness.shutdown().await;
}

#[tokio::test]
async fn reborn_trace_file_write_local_dev_approval_gate_bubbles() {
    let write_file = CapabilityId::new(WRITE_FILE_CAPABILITY_ID).expect("valid capability id");
    let model_gateway = RebornTraceReplayModelGateway::with_scripted_steps([
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![RebornScriptedProviderToolCall::new(
                write_file.clone(),
                "call_write_file_approval",
                serde_json::json!({
                    "path": "/workspace/generated/approval.txt",
                    "content": "approval required",
                }),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::Response {
            response: HostManagedModelResponse::assistant_reply("approval gate resumed"),
            expected_tool_results: Vec::new(),
        },
    ]);
    let mut harness =
        RebornBinaryE2EHarness::with_host_runtime_file_capabilities_requiring_approval(
            "room-trace-file-approval",
            model_gateway,
        )
        .await
        .expect("harness");
    harness.start();

    let submitted = harness
        .submit_text("event-trace-file-approval", "write an approval gated file")
        .await
        .expect("submit text");
    let blocked = harness
        .wait_for_status(submitted.run_id, TurnStatus::BlockedApproval)
        .await
        .expect("write blocks on local-dev approval gate");
    let gate_ref = blocked.gate_ref.expect("blocked approval gate ref");
    assert!(
        gate_ref.as_str().starts_with("gate:approval-"),
        "expected local-dev approval gate ref, got {gate_ref:?}"
    );

    let resolved = harness
        .approve_and_resume_local_dev_gate(submitted.run_id)
        .await
        .expect("approve local-dev file write gate");
    assert_eq!(resolved, gate_ref);
    harness
        .wait_for_status(submitted.run_id, TurnStatus::Completed)
        .await
        .expect("completed after approval resume");
    harness
        .assert_final_reply("approval gate resumed")
        .await
        .expect("final reply");
    let invocations = harness.capability_invocations();
    assert_eq!(invocations.len(), 2);
    assert_eq!(invocations[0].capability_id, write_file);
    assert!(invocations[0].approval_resume.is_none());
    assert_eq!(invocations[1].capability_id, write_file);
    assert!(
        invocations[1].approval_resume.is_some(),
        "approved gate should resume the original blocked capability, not ask the model for a new tool call"
    );

    harness.shutdown().await;
}
