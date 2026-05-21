#[allow(dead_code)]
#[path = "support/reborn/mod.rs"]
mod reborn_support;
mod support;

use ironclaw_host_api::CapabilityId;
use ironclaw_host_runtime::{
    APPLY_PATCH_CAPABILITY_ID, HTTP_CAPABILITY_ID, JSON_CAPABILITY_ID, READ_FILE_CAPABILITY_ID,
    TIME_CAPABILITY_ID,
};
use ironclaw_loop_support::{HostManagedModelMessageRole, HostManagedModelResponse};
use ironclaw_turns::{TurnStatus, run_profile::LoopHostMilestoneKind};
use reborn_support::{
    harness::{RebornBinaryE2EHarness, assert_milestone_order},
    model_replay::{
        RebornModelReplayStep, RebornScriptedProviderToolCall, RebornTraceReplayModelGateway,
    },
};

const PATCHED_CONTENT: &str = "alpha\npatched\nomega\n";

#[tokio::test]
async fn reborn_trace_core_builtin_tools_parity() {
    let time = CapabilityId::new(TIME_CAPABILITY_ID).expect("valid capability id");
    let json = CapabilityId::new(JSON_CAPABILITY_ID).expect("valid capability id");
    let http = CapabilityId::new(HTTP_CAPABILITY_ID).expect("valid capability id");
    let read_file = CapabilityId::new(READ_FILE_CAPABILITY_ID).expect("valid capability id");
    let apply_patch = CapabilityId::new(APPLY_PATCH_CAPABILITY_ID).expect("valid capability id");
    let model_gateway = RebornTraceReplayModelGateway::with_scripted_steps([
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![
                RebornScriptedProviderToolCall::new(
                    time.clone(),
                    "call_time_diff",
                    serde_json::json!({
                        "operation": "diff",
                        "input": "2026-05-12T13:00:00Z",
                        "timestamp2": "2026-05-12T15:30:00Z",
                    }),
                ),
                RebornScriptedProviderToolCall::new(
                    json.clone(),
                    "call_json_query",
                    serde_json::json!({
                        "operation": "query",
                        "data": {"items": [{"name": "alpha"}]},
                        "path": "items[0].name",
                    }),
                ),
                RebornScriptedProviderToolCall::new(
                    http.clone(),
                    "call_http_get",
                    serde_json::json!({
                        "url": "https://api.example.test/v1/items",
                        "headers": {"x-request-id": "reborn-core-builtins"},
                        "timeout_ms": 2500,
                    }),
                ),
            ],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![RebornScriptedProviderToolCall::new(
                read_file.clone(),
                "call_read_patch_target",
                serde_json::json!({"path": "/workspace/patch-target.txt"}),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![RebornScriptedProviderToolCall::new(
                apply_patch.clone(),
                "call_apply_patch",
                serde_json::json!({
                    "path": "/workspace/patch-target.txt",
                    "old_string": "needs-patch",
                    "new_string": "patched",
                }),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::Response {
            response: HostManagedModelResponse::assistant_reply("core builtins trace complete"),
            expected_tool_results: Vec::new(),
        },
    ]);
    let mut harness = RebornBinaryE2EHarness::with_host_runtime_core_builtin_capabilities(
        "room-trace-core-builtins",
        model_gateway,
    )
    .await
    .expect("harness");
    seed_workspace(&harness);
    harness.start();

    let submitted = harness
        .submit_text("event-trace-core-builtins", "exercise core builtins")
        .await
        .expect("submit text");
    harness
        .wait_for_status(submitted.run_id, TurnStatus::Completed)
        .await
        .expect("completed run");
    harness
        .assert_final_reply("core builtins trace complete")
        .await
        .expect("final reply");

    let patched_path = harness
        .host_workspace_file_path("patch-target.txt")
        .expect("patch target path");
    assert_eq!(
        std::fs::read_to_string(patched_path).expect("patched file"),
        PATCHED_CONTENT
    );

    let invocations = harness.capability_invocations();
    assert_eq!(invocations.len(), 5);
    assert_eq!(invocations[0].capability_id, time);
    assert_eq!(invocations[1].capability_id, json);
    assert_eq!(invocations[2].capability_id, http);
    assert_eq!(invocations[3].capability_id, read_file);
    assert_eq!(invocations[4].capability_id, apply_patch);

    let requests = harness.model_requests();
    assert_eq!(requests.len(), 4);
    assert_eq!(tool_result_count(&requests[1]), 3);
    assert_eq!(tool_result_count(&requests[2]), 4);
    assert_eq!(tool_result_count(&requests[3]), 5);
    assert!(
        requests[3].messages.iter().any(|message| message.role
            == HostManagedModelMessageRole::ToolResult
            && message.content.contains("result:")),
        "apply_patch result ref should be visible before final reply"
    );

    assert_milestone_order(
        &harness.milestones(),
        |kind| matches!(kind, LoopHostMilestoneKind::CapabilityBatchCompleted { .. }),
        |kind| matches!(kind, LoopHostMilestoneKind::AssistantReplyFinalized { .. }),
    );

    tokio::task::yield_now().await;
    harness.shutdown().await;
}

fn seed_workspace(harness: &RebornBinaryE2EHarness) {
    std::fs::write(
        harness
            .host_workspace_file_path("patch-target.txt")
            .expect("patch target path"),
        "alpha\nneeds-patch\nomega\n",
    )
    .expect("write patch target");
}

fn tool_result_count(request: &ironclaw_loop_support::HostManagedModelRequest) -> usize {
    request
        .messages
        .iter()
        .filter(|message| message.role == HostManagedModelMessageRole::ToolResult)
        .count()
}
