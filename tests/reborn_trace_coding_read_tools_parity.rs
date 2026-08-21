#[allow(dead_code)]
#[path = "support/reborn_parity_qa/mod.rs"]
mod parity_qa_support;
#[allow(dead_code)]
#[path = "integration/support/mod.rs"]
mod reborn_support;
mod support;

use ironclaw_host_api::ids::CapabilityId;
use ironclaw_host_runtime::{CODING_READ_CAPABILITY_ID, GLOB_CAPABILITY_ID, GREP_CAPABILITY_ID};
use ironclaw_loop_contracts::LoopHostMilestoneKind;
use ironclaw_loop_host::{HostManagedModelMessageRole, HostManagedModelResponse};
use ironclaw_turns::TurnStatus;
use parity_qa_support::{
    binary_e2e::{RebornBinaryE2EHarness, assert_milestone_order},
    model_replay::{
        RebornModelReplayStep, RebornScriptedProviderToolCall, RebornTraceReplayModelGateway,
    },
};

const ALPHA_CONTENT: &str = "Project Alpha contains DETERMINISTIC_MARKER_3702.";
const BETA_CONTENT: &str = "Project Beta has no marker.";

#[tokio::test]
async fn reborn_trace_coding_read_tools_parity() {
    let read_dir = CapabilityId::new(CODING_READ_CAPABILITY_ID).expect("valid capability id");
    let glob = CapabilityId::new(GLOB_CAPABILITY_ID).expect("valid capability id");
    let grep = CapabilityId::new(GREP_CAPABILITY_ID).expect("valid capability id");
    let model_gateway = RebornTraceReplayModelGateway::with_scripted_steps([
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![
                RebornScriptedProviderToolCall::new(
                    read_dir.clone(),
                    "call_read_dir_notes",
                    serde_json::json!({
                        "path": "/workspace/notes",
                    }),
                ),
                RebornScriptedProviderToolCall::new(
                    glob.clone(),
                    "call_glob_notes",
                    serde_json::json!({
                        "path": "notes/*.md",
                    }),
                ),
                RebornScriptedProviderToolCall::new(
                    grep.clone(),
                    "call_grep_marker",
                    serde_json::json!({
                        "path": "/workspace",
                        "pattern": "DETERMINISTIC_MARKER_3702",
                    }),
                ),
            ],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::Response {
            response: HostManagedModelResponse::assistant_reply("coding read tools trace complete"),
            expected_tool_results: Vec::new(),
        },
    ]);
    let mut harness = RebornBinaryE2EHarness::with_host_runtime_coding_read_capabilities(
        "room-trace-coding-read-tools",
        model_gateway,
    )
    .await
    .expect("harness");
    seed_workspace(&harness);
    harness.start();

    let submitted = harness
        .submit_text("event-trace-coding-read-tools", "inspect workspace notes")
        .await
        .expect("submit text");
    harness
        .wait_for_status(submitted.run_id, TurnStatus::Completed)
        .await
        .expect("completed run");
    harness
        .assert_final_reply("coding read tools trace complete")
        .await
        .expect("final reply");

    assert_eq!(
        std::fs::read_to_string(harness.host_workspace_file_path("notes/alpha.md").unwrap())
            .expect("alpha note"),
        ALPHA_CONTENT
    );

    let invocations = harness.capability_invocations();
    assert_eq!(invocations.len(), 3);
    assert_eq!(invocations[0].capability_id, read_dir);
    assert_eq!(invocations[1].capability_id, glob);
    assert_eq!(invocations[2].capability_id, grep);

    let results = harness.capability_results();
    assert_eq!(results.len(), 3);
    assert_eq!(results[0].capability_id, read_dir);
    assert_entries_include(&results[0].output, &["alpha.md", "beta.md"]);
    assert_eq!(results[1].capability_id, glob);
    assert_files_include(&results[1].output, &["alpha.md", "beta.md"]);
    assert_files_exclude(&results[1].output, &["README.txt"]);
    assert_eq!(results[2].capability_id, grep);
    let grep_output = results[2].output["output"]
        .as_str()
        .expect("grep output text");
    assert!(
        grep_output.contains("1:Project Alpha contains DETERMINISTIC_MARKER_3702."),
        "grep must surface the deterministic marker match row, got {grep_output}"
    );

    let requests = harness.model_requests();
    assert_eq!(requests.len(), 2);
    let tool_results = requests[1]
        .messages
        .iter()
        .filter(|message| message.role == HostManagedModelMessageRole::ToolResult)
        .collect::<Vec<_>>();
    assert_eq!(tool_results.len(), 3);
    assert!(
        tool_results
            .iter()
            .all(|message| message.content.contains("result:"))
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
    let notes_dir = harness
        .host_workspace_file_path("notes")
        .expect("notes directory path");
    std::fs::create_dir_all(&notes_dir).expect("create notes directory");
    std::fs::write(notes_dir.join("alpha.md"), ALPHA_CONTENT).expect("write alpha note");
    std::fs::write(notes_dir.join("beta.md"), BETA_CONTENT).expect("write beta note");
    std::fs::write(
        harness
            .host_workspace_file_path("README.txt")
            .expect("readme path"),
        "non-markdown file",
    )
    .expect("write readme");
}

fn assert_entries_include(output: &serde_json::Value, expected: &[&str]) {
    let text = output["output"].as_str().expect("directory read output");
    for expected_entry in expected {
        assert!(
            text.contains(expected_entry),
            "expected directory read output to include {expected_entry:?}, got {text:?}"
        );
    }
}

fn assert_files_include(output: &serde_json::Value, expected: &[&str]) {
    let text = output["output"].as_str().expect("glob output");
    for expected_file in expected {
        assert!(
            text.contains(expected_file),
            "expected glob output to include {expected_file:?}, got {text:?}"
        );
    }
}

fn assert_files_exclude(output: &serde_json::Value, expected: &[&str]) {
    let text = output["output"].as_str().expect("glob output");
    for unexpected_file in expected {
        assert!(
            !text.contains(unexpected_file),
            "expected glob output to exclude {unexpected_file:?}, got {text:?}"
        );
    }
}
