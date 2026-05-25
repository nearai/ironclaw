#[allow(dead_code)]
#[path = "support/reborn/mod.rs"]
mod reborn_support;
mod support;

use std::collections::BTreeSet;

use ironclaw_host_api::CapabilityId;
use ironclaw_host_runtime::{
    APPLY_PATCH_CAPABILITY_ID, ECHO_CAPABILITY_ID, GLOB_CAPABILITY_ID, GREP_CAPABILITY_ID,
    HTTP_CAPABILITY_ID, JSON_CAPABILITY_ID, LIST_DIR_CAPABILITY_ID, READ_FILE_CAPABILITY_ID,
    SHELL_CAPABILITY_ID, SKILL_INSTALL_CAPABILITY_ID, SKILL_LIST_CAPABILITY_ID,
    SKILL_REMOVE_CAPABILITY_ID, TIME_CAPABILITY_ID, WRITE_FILE_CAPABILITY_ID,
    builtin_first_party_package,
};
use ironclaw_loop_support::{HostManagedModelMessageRole, HostManagedModelResponse};
use ironclaw_turns::{TurnStatus, run_profile::LoopHostMilestoneKind};
use reborn_support::{
    harness::{RebornBinaryE2EHarness, assert_milestone_order},
    model_replay::{
        RebornModelReplayStep, RebornScriptedProviderToolCall, RebornTraceReplayModelGateway,
    },
};

const REBORN_FIRST_PARTY_E2E_COVERED_CAPABILITIES: &[&str] = &[
    ECHO_CAPABILITY_ID,
    TIME_CAPABILITY_ID,
    JSON_CAPABILITY_ID,
    HTTP_CAPABILITY_ID,
    SHELL_CAPABILITY_ID,
    READ_FILE_CAPABILITY_ID,
    WRITE_FILE_CAPABILITY_ID,
    LIST_DIR_CAPABILITY_ID,
    GLOB_CAPABILITY_ID,
    GREP_CAPABILITY_ID,
    APPLY_PATCH_CAPABILITY_ID,
    SKILL_LIST_CAPABILITY_ID,
    SKILL_INSTALL_CAPABILITY_ID,
    SKILL_REMOVE_CAPABILITY_ID,
];

const SKILL_NAME: &str = "reborn-skill-e2e";

#[test]
fn reborn_builtin_first_party_capability_e2e_coverage_is_complete() {
    let declared = builtin_first_party_package()
        .expect("built-in first-party package builds")
        .capabilities
        .into_iter()
        .map(|capability| capability.id.as_str().to_string())
        .collect::<BTreeSet<_>>();
    let covered = REBORN_FIRST_PARTY_E2E_COVERED_CAPABILITIES
        .iter()
        .map(|capability| (*capability).to_string())
        .collect::<BTreeSet<_>>();

    assert_eq!(
        declared, covered,
        "each built-in first-party capability must have Reborn e2e coverage"
    );
}

#[tokio::test]
async fn reborn_trace_process_first_party_tools_parity() {
    let echo = CapabilityId::new(ECHO_CAPABILITY_ID).expect("valid capability id");
    let shell = CapabilityId::new(SHELL_CAPABILITY_ID).expect("valid capability id");
    let model_gateway = RebornTraceReplayModelGateway::with_scripted_steps([
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![RebornScriptedProviderToolCall::new(
                echo.clone(),
                "call_echo_first_party",
                serde_json::json!({"message": "reborn echo e2e"}),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::Response {
            response: HostManagedModelResponse::assistant_reply("process tools trace complete"),
            expected_tool_results: Vec::new(),
        },
    ]);
    let mut harness = RebornBinaryE2EHarness::with_host_runtime_process_capabilities(
        "room-trace-process-first-party-tools",
        model_gateway,
    )
    .await
    .expect("harness");
    harness.start();

    let submitted = harness
        .submit_text(
            "event-trace-process-first-party-tools",
            "exercise process first-party tools",
        )
        .await
        .expect("submit text");
    harness
        .wait_for_status(submitted.run_id, TurnStatus::Completed)
        .await
        .expect("completed run");
    harness
        .assert_final_reply("process tools trace complete")
        .await
        .expect("final reply");

    let invocations = harness.capability_invocations();
    assert_eq!(invocations.len(), 1);
    assert_eq!(invocations[0].capability_id, echo);

    let results = harness.capability_results();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].capability_id, echo);
    assert_eq!(results[0].output, serde_json::json!("reborn echo e2e"));

    let requests = harness.model_requests();
    assert_eq!(requests.len(), 2);
    // The loop approval-gates shell execution; the product-live adapter e2e
    // covers direct shell execution while this test guards model-surface parity.
    assert!(
        requests[0]
            .messages
            .iter()
            .any(|message| message.content.contains(shell.as_str())),
        "shell must be advertised on the Reborn model-facing first-party surface"
    );
    assert_eq!(tool_result_count(&requests[1]), 1);
    assert_milestone_order(
        &harness.milestones(),
        |kind| matches!(kind, LoopHostMilestoneKind::CapabilityBatchCompleted { .. }),
        |kind| matches!(kind, LoopHostMilestoneKind::AssistantReplyFinalized { .. }),
    );
    harness.assert_model_exhausted();

    harness.shutdown().await;
}

#[tokio::test]
async fn reborn_trace_skill_management_first_party_tools_parity() {
    let skill_install =
        CapabilityId::new(SKILL_INSTALL_CAPABILITY_ID).expect("valid capability id");
    let skill_list = CapabilityId::new(SKILL_LIST_CAPABILITY_ID).expect("valid capability id");
    let skill_remove = CapabilityId::new(SKILL_REMOVE_CAPABILITY_ID).expect("valid capability id");
    let skill_content = skill_md(SKILL_NAME, "Reborn skill management e2e");
    let model_gateway = RebornTraceReplayModelGateway::with_scripted_steps([
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![RebornScriptedProviderToolCall::new(
                skill_install.clone(),
                "call_skill_install_first_party",
                serde_json::json!({
                    "name": SKILL_NAME,
                    "content": skill_content,
                }),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![RebornScriptedProviderToolCall::new(
                skill_list.clone(),
                "call_skill_list_after_install",
                serde_json::json!({}),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![RebornScriptedProviderToolCall::new(
                skill_remove.clone(),
                "call_skill_remove_first_party",
                serde_json::json!({"name": SKILL_NAME}),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![RebornScriptedProviderToolCall::new(
                skill_list.clone(),
                "call_skill_list_after_remove",
                serde_json::json!({}),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::Response {
            response: HostManagedModelResponse::assistant_reply(
                "skill management tools trace complete",
            ),
            expected_tool_results: Vec::new(),
        },
    ]);
    let mut harness = RebornBinaryE2EHarness::with_host_runtime_skill_management_capabilities(
        "room-trace-skill-management-first-party-tools",
        model_gateway,
    )
    .await
    .expect("harness");
    harness.start();

    let submitted = harness
        .submit_text(
            "event-trace-skill-management-first-party-tools",
            "exercise skill management first-party tools",
        )
        .await
        .expect("submit text");
    harness
        .wait_for_status(submitted.run_id, TurnStatus::Completed)
        .await
        .expect("completed run");
    harness
        .assert_final_reply("skill management tools trace complete")
        .await
        .expect("final reply");

    let invocations = harness.capability_invocations();
    assert_eq!(invocations.len(), 4);
    assert_eq!(invocations[0].capability_id, skill_install);
    assert_eq!(invocations[1].capability_id, skill_list);
    assert_eq!(invocations[2].capability_id, skill_remove);
    assert_eq!(invocations[3].capability_id, skill_list);

    let results = harness.capability_results();
    assert_eq!(results.len(), 4);
    assert_eq!(results[0].capability_id, skill_install);
    assert_eq!(results[0].output["installed"], serde_json::json!(true));
    assert_eq!(results[0].output["name"], serde_json::json!(SKILL_NAME));
    assert_skill_list_contains(&results[1].output, SKILL_NAME);
    assert_eq!(results[2].capability_id, skill_remove);
    assert_eq!(results[2].output["removed"], serde_json::json!(true));
    assert_eq!(results[2].output["name"], serde_json::json!(SKILL_NAME));
    assert_skill_list_excludes(&results[3].output, SKILL_NAME);

    let requests = harness.model_requests();
    assert_eq!(requests.len(), 5);
    assert_eq!(tool_result_count(&requests[1]), 1);
    assert_eq!(tool_result_count(&requests[2]), 2);
    assert_eq!(tool_result_count(&requests[3]), 3);
    assert_eq!(tool_result_count(&requests[4]), 4);
    assert_milestone_order(
        &harness.milestones(),
        |kind| matches!(kind, LoopHostMilestoneKind::CapabilityBatchCompleted { .. }),
        |kind| matches!(kind, LoopHostMilestoneKind::AssistantReplyFinalized { .. }),
    );
    harness.assert_model_exhausted();

    harness.shutdown().await;
}

fn skill_md(name: &str, description: &str) -> String {
    format!("---\nname: {name}\ndescription: {description}\n---\nSkill body for {name}.\n")
}

fn tool_result_count(request: &ironclaw_loop_support::HostManagedModelRequest) -> usize {
    request
        .messages
        .iter()
        .filter(|message| message.role == HostManagedModelMessageRole::ToolResult)
        .count()
}

fn assert_skill_list_contains(output: &serde_json::Value, expected: &str) {
    assert!(
        skill_names(output).contains(&expected),
        "expected skill list to include {expected:?}, got {output:?}"
    );
}

fn assert_skill_list_excludes(output: &serde_json::Value, unexpected: &str) {
    assert!(
        skill_names(output).iter().all(|name| *name != unexpected),
        "expected skill list to exclude {unexpected:?}, got {output:?}"
    );
}

fn skill_names(output: &serde_json::Value) -> Vec<&str> {
    output["skills"]
        .as_array()
        .expect("skill list output should contain skills array")
        .iter()
        .filter_map(|skill| skill["name"].as_str())
        .collect()
}
