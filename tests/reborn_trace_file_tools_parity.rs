#[allow(dead_code)]
#[path = "support/reborn/mod.rs"]
mod reborn_support;
mod support;

use ironclaw_host_api::CapabilityId;
use ironclaw_host_runtime::WRITE_FILE_CAPABILITY_ID;
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
    let model_gateway = RebornTraceReplayModelGateway::with_scripted_steps([
        RebornModelReplayStep::ProviderToolCalls(vec![RebornScriptedProviderToolCall::new(
            write_file.clone(),
            "call_write_file_1",
            serde_json::json!({
                "path": "/workspace/generated/hello.txt",
                "content": EXPECTED_CONTENT,
            }),
        )]),
        RebornModelReplayStep::Response(HostManagedModelResponse::assistant_reply(
            "file trace complete",
        )),
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
    assert_eq!(invocations.len(), 1);
    assert_eq!(invocations[0].capability_id, write_file);

    let requests = harness.model_requests();
    assert_eq!(requests.len(), 2);
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
