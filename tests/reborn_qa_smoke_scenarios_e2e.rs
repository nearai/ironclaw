#[allow(dead_code)]
#[path = "support/reborn/mod.rs"]
mod reborn_support;
mod support;

use std::time::Duration;

use ironclaw_host_api::CapabilityId;
use ironclaw_host_runtime::{
    APPLY_PATCH_CAPABILITY_ID, JSON_CAPABILITY_ID, LIST_DIR_CAPABILITY_ID, READ_FILE_CAPABILITY_ID,
    SHELL_CAPABILITY_ID, SKILL_INSTALL_CAPABILITY_ID, SKILL_LIST_CAPABILITY_ID,
    SKILL_REMOVE_CAPABILITY_ID, TIME_CAPABILITY_ID, TRIGGER_CREATE_CAPABILITY_ID,
    TRIGGER_LIST_CAPABILITY_ID, TRIGGER_REMOVE_CAPABILITY_ID, WRITE_FILE_CAPABILITY_ID,
};
use ironclaw_loop_support::{
    DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID, HostManagedModelMessageRole, HostManagedModelResponse,
};
use ironclaw_turns::TurnStatus;
use reborn_support::{
    config::WaitConfig,
    extension_surface::{
        BUNDLED_EXTENSION_CAPABILITY_IDS, BUNDLED_EXTENSION_IDS, EXTENSION_ACTIVATE_CAPABILITY_ID,
        EXTENSION_INSTALL_CAPABILITY_ID, EXTENSION_LIFECYCLE_CAPABILITY_IDS,
        EXTENSION_REMOVE_CAPABILITY_ID, EXTENSION_SEARCH_CAPABILITY_ID,
    },
    github as github_support,
    harness::{RebornBinaryE2EHarness, RecordingTestCapabilityPort},
    model_replay::{
        RebornModelReplayStep, RebornScriptedProviderToolCall, RebornTraceReplayModelGateway,
    },
};

const COVERED_QA_SCENARIOS: &[&str] = &[
    "three_step_time_write_read_summary",
    "session_continuity_write_read_append",
    "automation_heartbeat_smoke",
    "paused_cron_automation_smoke",
    "subagent_capability_smoke",
    "skill_discovery_smoke",
    "skill_invocation_smoke",
    "browser_integration_smoke",
    "local_browser_interaction_smoke",
    "mcp_discovery_smoke",
    "plugin_capability_smoke",
    "github_capability_smoke",
    "document_artifact_smoke",
    "spreadsheet_artifact_smoke",
    "presentation_artifact_smoke",
    "image_generation_smoke",
    "error_handling_smoke",
    "long_running_process_smoke",
    "repo_read_only_review_smoke",
    "approval_boundary_smoke",
    "patch_isolation_smoke",
    "cleanup_verification_smoke",
];

#[test]
fn every_pasted_qa_scenario_has_reborn_e2e_coverage() {
    reborn_support::qa_scenarios::assert_all_covered(COVERED_QA_SCENARIOS);
}

#[tokio::test]
async fn qa_three_step_time_write_read_and_session_continuity_workflows() {
    let time = cap(TIME_CAPABILITY_ID);
    let write_file = cap(WRITE_FILE_CAPABILITY_ID);
    let read_file = cap(READ_FILE_CAPABILITY_ID);
    let apply_patch = cap(APPLY_PATCH_CAPABILITY_ID);
    let steps = [
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![call(
                &time,
                "qa_time_now",
                serde_json::json!({"operation": "now", "timezone": "UTC"}),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![call(
                &write_file,
                "qa_write_time",
                serde_json::json!({
                    "path": "/workspace/qa-reborn-loop.txt",
                    "content": "time result was observed before this write\n"
                }),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![call(
                &read_file,
                "qa_read_time",
                serde_json::json!({"path": "/workspace/qa-reborn-loop.txt"}),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![call(
                &write_file,
                "qa_write_alpha",
                serde_json::json!({
                    "path": "/workspace/qa-session-continuity.md",
                    "content": "session marker alpha\n"
                }),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![call(
                &read_file,
                "qa_read_alpha",
                serde_json::json!({"path": "/workspace/qa-session-continuity.md"}),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![call(
                &apply_patch,
                "qa_append_beta",
                serde_json::json!({
                    "path": "/workspace/qa-session-continuity.md",
                    "old_string": "session marker alpha\n",
                    "new_string": "session marker alpha\nsession marker beta\n"
                }),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![call(
                &read_file,
                "qa_read_beta",
                serde_json::json!({"path": "/workspace/qa-session-continuity.md"}),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::Response {
            response: HostManagedModelResponse::assistant_reply(
                "qa file and session continuity workflows complete",
            ),
            expected_tool_results: Vec::new(),
        },
    ];
    let mut harness = run_qa_harness(
        "room-qa-file-session",
        "event-qa-file-session",
        "run QA file and session continuity workflows",
        steps,
    )
    .await;

    let invocations = harness.capability_invocations();
    assert_eq!(
        capability_order(&invocations),
        vec![
            TIME_CAPABILITY_ID,
            WRITE_FILE_CAPABILITY_ID,
            READ_FILE_CAPABILITY_ID,
            WRITE_FILE_CAPABILITY_ID,
            READ_FILE_CAPABILITY_ID,
            APPLY_PATCH_CAPABILITY_ID,
            READ_FILE_CAPABILITY_ID,
        ]
    );
    assert_eq!(
        std::fs::read_to_string(
            harness
                .host_workspace_file_path("qa-reborn-loop.txt")
                .expect("qa loop path")
        )
        .expect("qa loop content"),
        "time result was observed before this write\n"
    );
    assert_eq!(
        std::fs::read_to_string(
            harness
                .host_workspace_file_path("qa-session-continuity.md")
                .expect("session continuity path")
        )
        .expect("session continuity content"),
        "session marker alpha\nsession marker beta\n"
    );
    let requests = harness.model_requests();
    assert!(requests.len() >= 2, "expected at least 2 model requests");
    assert_eq!(
        tool_result_count(&requests[1]),
        1,
        "time result must be visible before the dependent write request"
    );
    harness.shutdown().await;
}

#[tokio::test]
async fn qa_trigger_automation_smokes_create_view_and_cleanup() {
    let trigger_create = cap(TRIGGER_CREATE_CAPABILITY_ID);
    let trigger_list = cap(TRIGGER_LIST_CAPABILITY_ID);
    let trigger_remove = cap(TRIGGER_REMOVE_CAPABILITY_ID);
    let steps = [
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![call(
                &trigger_create,
                "qa_heartbeat_create",
                serde_json::json!({
                    "name": "qa-reborn-heartbeat-smoke",
                    "prompt": "reborn heartbeat smoke",
                    "cron": "*/2 * * * *",
                    "timezone": "UTC"
                }),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![call(
                &trigger_list,
                "qa_heartbeat_view",
                serde_json::json!({"limit": 10}),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![call(
                &trigger_create,
                "qa_cron_create",
                serde_json::json!({
                    "name": "qa-reborn-cron-smoke",
                    "prompt": "summarize repo status",
                    "cron": "0 9 * * 1",
                    "timezone": "UTC"
                }),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![call(
                &trigger_list,
                "qa_cron_view",
                serde_json::json!({"limit": 10}),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![call(
                &trigger_remove,
                "qa_heartbeat_remove",
                serde_json::json!({"trigger_id": "01J00000000000000000000009"}),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![call(
                &trigger_remove,
                "qa_cron_remove",
                serde_json::json!({"trigger_id": "01J00000000000000000000010"}),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::Response {
            response: HostManagedModelResponse::assistant_reply(
                "qa automation trigger smoke complete",
            ),
            expected_tool_results: Vec::new(),
        },
    ];
    let mut harness = run_qa_harness(
        "room-qa-triggers",
        "event-qa-triggers",
        "run QA trigger automation smoke workflows",
        steps,
    )
    .await;

    let results = harness.capability_results();
    assert_eq!(results.len(), 6);
    assert_eq!(
        results[0].output["trigger"]["name"],
        serde_json::json!("qa-reborn-heartbeat-smoke")
    );
    assert_eq!(
        results[2].output["trigger"]["name"],
        serde_json::json!("qa-reborn-cron-smoke")
    );
    assert!(
        results[1].output.to_string().contains("reborn-heartbeat"),
        "heartbeat trigger should be visible before cleanup"
    );
    assert!(
        results[3]
            .output
            .to_string()
            .contains("qa-reborn-cron-smoke"),
        "cron trigger should be visible before cleanup"
    );
    assert!(
        !results[3].output.to_string().contains("\"paused\""),
        "current Reborn triggers expose scheduled/removed state, not a paused state"
    );
    assert_eq!(results[4].output["removed"], serde_json::json!(false));
    assert_eq!(results[5].output["removed"], serde_json::json!(false));
    harness.shutdown().await;
}

#[tokio::test]
async fn qa_subagent_capability_smoke_uses_child_run() {
    let spawn_subagent = cap(DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID);
    let model_gateway = RebornTraceReplayModelGateway::with_scripted_steps([
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![call(
                &spawn_subagent,
                "qa_spawn_docs_checks",
                serde_json::json!({
                    "flavor_id": "general",
                    "task": "check repo testing docs and security docs independently"
                }),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::Response {
            response: HostManagedModelResponse::assistant_reply(
                "child found testing docs and security docs",
            ),
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::Response {
            response: HostManagedModelResponse::assistant_reply("qa subagent smoke complete"),
            expected_tool_results: Vec::new(),
        },
    ]);
    let mut harness = RebornBinaryE2EHarness::with_harness_blocked_evidence_unscoped_worker(
        "room-qa-subagent",
        model_gateway,
        RecordingTestCapabilityPort::echo_with_spawn_subagent(),
    )
    .await
    .expect("subagent harness");
    harness.start();

    let submitted = harness
        .submit_text(
            "event-qa-subagent",
            "split repo testing docs and security docs checks",
        )
        .await
        .expect("submit subagent smoke");
    harness
        .wait_for_status_with_config(submitted.run_id, TurnStatus::BlockedDependentRun, qa_wait())
        .await
        .expect("parent should block on child");
    let children = harness
        .children_of(&submitted.scope, submitted.run_id)
        .await
        .expect("children");
    assert_eq!(
        children.len(),
        1,
        "subagent smoke should create one child run"
    );
    harness
        .wait_for_status_in_scope_with_config(
            children[0].scope.clone(),
            children[0].run_id,
            TurnStatus::Completed,
            qa_wait(),
        )
        .await
        .expect("child completes");
    harness
        .wait_for_status_with_config(submitted.run_id, TurnStatus::Completed, qa_wait())
        .await
        .expect("parent resumes");
    harness
        .assert_final_reply("qa subagent smoke complete")
        .await
        .expect("final reply");
    harness.assert_model_exhausted();
    harness.shutdown().await;
}

#[tokio::test]
async fn qa_skill_discovery_and_invocation_smokes_are_read_only_until_invoked() {
    let skill_install = cap(SKILL_INSTALL_CAPABILITY_ID);
    let skill_list = cap(SKILL_LIST_CAPABILITY_ID);
    let skill_remove = cap(SKILL_REMOVE_CAPABILITY_ID);
    let steps = [
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![call(
                &skill_list,
                "qa_skill_discovery_empty",
                serde_json::json!({}),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![call(
                &skill_install,
                "qa_skill_install_review",
                serde_json::json!({
                    "name": "qa-read-only-review",
                    "content": "---\nname: qa-read-only-review\ndescription: Read-only mini review of current status or diff.\n---\nInspect status and report findings without editing.\n"
                }),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![call(
                &skill_list,
                "qa_skill_list_after_install",
                serde_json::json!({}),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![call(
                &skill_remove,
                "qa_skill_remove_review",
                serde_json::json!({"name": "qa-read-only-review"}),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::Response {
            response: HostManagedModelResponse::assistant_reply("qa skill smoke complete"),
            expected_tool_results: Vec::new(),
        },
    ];
    let mut harness = run_qa_harness(
        "room-qa-skills",
        "event-qa-skills",
        "run QA skill discovery and invocation smoke",
        steps,
    )
    .await;

    let results = harness.capability_results();
    assert!(results.len() >= 4, "expected at least 4 capability results");
    assert_eq!(results[1].output["installed"], serde_json::json!(true));
    assert!(
        results[2]
            .output
            .to_string()
            .contains("qa-read-only-review"),
        "installed skill should appear in skill discovery"
    );
    assert_eq!(results[3].output["removed"], serde_json::json!(true));
    harness.shutdown().await;
}

#[tokio::test]
async fn qa_error_process_repo_patch_and_cleanup_smokes() {
    let json = cap(JSON_CAPABILITY_ID);
    let shell = cap(SHELL_CAPABILITY_ID);
    let write_file = cap(WRITE_FILE_CAPABILITY_ID);
    let read_file = cap(READ_FILE_CAPABILITY_ID);
    let list_dir = cap(LIST_DIR_CAPABILITY_ID);
    let apply_patch = cap(APPLY_PATCH_CAPABILITY_ID);
    let steps = [
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![call(
                &json,
                "qa_invalid_json",
                serde_json::json!({"operation": "parse", "data": "{invalid"}),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![call(
                &json,
                "qa_valid_json",
                serde_json::json!({"operation": "validate", "data": "{\"ok\":true}"}),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![call(
                &shell,
                "qa_pwd",
                serde_json::json!({"command": "pwd", "timeout": 5}),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![call(
                &shell,
                "qa_ticks",
                serde_json::json!({
                    "command": "for i in 1 2 3; do echo tick; sleep 0.1; done",
                    "timeout": 5
                }),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![call(
                &shell,
                "qa_repo_status",
                serde_json::json!({
                    "command": "git status --short && git branch --show-current",
                    "timeout": 5
                }),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![call(
                &write_file,
                "qa_patch_write",
                serde_json::json!({
                    "path": "/workspace/qa-patch-isolation.txt",
                    "content": "alpha\nbeta\ngamma\n"
                }),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![call(
                &apply_patch,
                "qa_patch_beta",
                serde_json::json!({
                    "path": "/workspace/qa-patch-isolation.txt",
                    "old_string": "beta",
                    "new_string": "BETA"
                }),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![call(
                &read_file,
                "qa_patch_read",
                serde_json::json!({"path": "/workspace/qa-patch-isolation.txt"}),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![
                call(
                    &write_file,
                    "qa_create_cleanup_a",
                    serde_json::json!({
                        "path": "/workspace/qa-cleanup-smoke/a",
                        "content": "a\n"
                    }),
                ),
                call(
                    &write_file,
                    "qa_create_cleanup_b",
                    serde_json::json!({
                        "path": "/workspace/qa-cleanup-smoke/b",
                        "content": "b\n"
                    }),
                ),
                call(
                    &write_file,
                    "qa_create_cleanup_c",
                    serde_json::json!({
                        "path": "/workspace/qa-cleanup-smoke/c",
                        "content": "c\n"
                    }),
                ),
            ],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![call(
                &list_dir,
                "qa_cleanup_list_before",
                serde_json::json!({"path": "/workspace/qa-cleanup-smoke"}),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::Response {
            response: HostManagedModelResponse::assistant_reply(
                "qa error process repo patch cleanup smoke complete",
            ),
            expected_tool_results: Vec::new(),
        },
    ];
    let mut harness = run_qa_harness(
        "room-qa-error-process-cleanup",
        "event-qa-error-process-cleanup",
        "run QA error, process, repo review, patch, and cleanup smokes",
        steps,
    )
    .await;

    let results = harness.capability_results();
    assert!(
        results.iter().any(|result| {
            result.capability_id == json && result.output["valid"] == serde_json::json!(true)
        }),
        "successful JSON validation result should be captured after invalid JSON failure"
    );
    assert!(
        results.iter().any(|result| {
            result.capability_id == shell
                && result.output["output"]
                    .as_str()
                    .is_some_and(|output| output.matches("tick").count() == 3)
        }),
        "long-running process smoke should print tick three times"
    );
    let patch_path = harness
        .host_workspace_file_path("qa-patch-isolation.txt")
        .expect("patch path");
    assert_eq!(
        std::fs::read_to_string(&patch_path).expect("patched file"),
        "alpha\nBETA\ngamma\n"
    );
    let cleanup_dir = harness
        .host_workspace_file_path("qa-cleanup-smoke")
        .expect("cleanup path");
    let mut listed = std::fs::read_dir(&cleanup_dir)
        .expect("cleanup dir")
        .map(|entry| {
            entry
                .expect("cleanup entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect::<Vec<_>>();
    listed.sort();
    assert_eq!(listed, vec!["a", "b", "c"]);
    for file_name in &listed {
        std::fs::remove_file(cleanup_dir.join(file_name)).expect("remove cleanup artifact");
    }
    std::fs::remove_dir(&cleanup_dir).expect("remove cleanup dir");
    std::fs::remove_file(&patch_path).expect("remove patch isolation file");
    assert!(!cleanup_dir.exists(), "cleanup directory should be removed");
    assert!(
        !patch_path.exists(),
        "patch isolation file should be removed"
    );
    harness.shutdown().await;
}

#[tokio::test]
async fn qa_extension_lifecycle_tools_search_install_activate_and_remove_e2e() {
    let search = cap(EXTENSION_SEARCH_CAPABILITY_ID);
    let install = cap(EXTENSION_INSTALL_CAPABILITY_ID);
    let activate = cap(EXTENSION_ACTIVATE_CAPABILITY_ID);
    let remove = cap(EXTENSION_REMOVE_CAPABILITY_ID);
    let model_gateway = RebornTraceReplayModelGateway::with_scripted_steps([
        RebornModelReplayStep::AssertProviderToolsThenProviderToolCalls {
            capability_ids: capability_ids(EXTENSION_LIFECYCLE_CAPABILITY_IDS),
            calls: vec![call(
                &search,
                "qa_extension_search_github",
                serde_json::json!({"query": "github"}),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![call(
                &install,
                "qa_extension_install_github",
                serde_json::json!({"extension_id": "github"}),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![call(
                &activate,
                "qa_extension_activate_github",
                serde_json::json!({"extension_id": "github"}),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::AssertProviderToolsThenProviderToolCalls {
            capability_ids: github_support::capability_ids().expect("github capability ids"),
            calls: vec![call(
                &remove,
                "qa_extension_remove_github",
                serde_json::json!({"extension_id": "github"}),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::Response {
            response: HostManagedModelResponse::assistant_reply(
                "qa extension lifecycle smoke complete",
            ),
            expected_tool_results: Vec::new(),
        },
    ]);
    let mut harness = RebornBinaryE2EHarness::with_host_runtime_extension_lifecycle_capabilities(
        "room-qa-extension-lifecycle",
        model_gateway,
    )
    .await
    .expect("extension lifecycle harness");
    harness.start();

    let submitted = harness
        .submit_text(
            "event-qa-extension-lifecycle",
            "search, install, activate, and remove a Reborn extension",
        )
        .await
        .expect("submit extension lifecycle smoke");
    harness
        .wait_for_status_with_config(submitted.run_id, TurnStatus::Completed, qa_wait())
        .await
        .expect("extension lifecycle smoke completes");
    harness
        .assert_final_reply("qa extension lifecycle smoke complete")
        .await
        .expect("final reply");

    let results = harness.capability_results();
    assert!(
        results.iter().any(|result| {
            result.capability_id == search && result.output["payload"]["count"].as_u64() == Some(1)
        }),
        "extension search should find the GitHub package"
    );
    assert!(
        results.iter().any(|result| {
            result.capability_id == install
                && result.output["payload"]["installed"] == serde_json::json!(true)
        }),
        "extension install should succeed"
    );
    assert!(
        results.iter().any(|result| {
            result.capability_id == activate
                && result.output["payload"]["activated"] == serde_json::json!(true)
        }),
        "extension activate should succeed"
    );
    assert!(
        results.iter().any(|result| {
            result.capability_id == remove
                && result.output["payload"]["removed"] == serde_json::json!(true)
        }),
        "extension remove should succeed"
    );
    harness.assert_model_exhausted();
    harness.shutdown().await;
}

#[tokio::test]
async fn qa_activating_bundled_extensions_exposes_complete_model_surface_e2e() {
    let install = cap(EXTENSION_INSTALL_CAPABILITY_ID);
    let activate = cap(EXTENSION_ACTIVATE_CAPABILITY_ID);
    let install_calls = BUNDLED_EXTENSION_IDS
        .iter()
        .map(|extension_id| {
            call(
                &install,
                &format!("qa_install_{extension_id}"),
                serde_json::json!({"extension_id": extension_id}),
            )
        })
        .collect::<Vec<_>>();
    let activate_calls = BUNDLED_EXTENSION_IDS
        .iter()
        .map(|extension_id| {
            call(
                &activate,
                &format!("qa_activate_{extension_id}"),
                serde_json::json!({"extension_id": extension_id}),
            )
        })
        .collect::<Vec<_>>();
    let model_gateway = RebornTraceReplayModelGateway::with_scripted_steps([
        RebornModelReplayStep::ProviderToolCalls {
            calls: install_calls,
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::ProviderToolCalls {
            calls: activate_calls,
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::AssertProviderToolsThenResponse {
            capability_ids: capability_ids(BUNDLED_EXTENSION_CAPABILITY_IDS),
            response: HostManagedModelResponse::assistant_reply(
                "qa bundled extension surface complete",
            ),
            expected_tool_results: Vec::new(),
        },
    ]);
    let mut harness = RebornBinaryE2EHarness::with_host_runtime_extension_lifecycle_capabilities(
        "room-qa-bundled-extension-surface",
        model_gateway,
    )
    .await
    .expect("bundled extension surface harness");
    harness.start();

    let submitted = harness
        .submit_text(
            "event-qa-bundled-extension-surface",
            "activate every bundled Reborn extension and verify the model surface",
        )
        .await
        .expect("submit bundled extension surface smoke");
    harness
        .wait_for_status_with_config(submitted.run_id, TurnStatus::Completed, qa_wait())
        .await
        .expect("bundled extension surface smoke completes");
    harness
        .assert_final_reply("qa bundled extension surface complete")
        .await
        .expect("final reply");

    let invocations = harness.capability_invocations();
    assert_eq!(
        invocations
            .iter()
            .filter(|invocation| invocation.capability_id == install)
            .count(),
        BUNDLED_EXTENSION_IDS.len(),
        "each bundled extension should be installed through the lifecycle tool"
    );
    assert_eq!(
        invocations
            .iter()
            .filter(|invocation| invocation.capability_id == activate)
            .count(),
        BUNDLED_EXTENSION_IDS.len(),
        "each bundled extension should be activated through the lifecycle tool"
    );
    harness.assert_model_exhausted();
    harness.shutdown().await;
}

#[tokio::test]
async fn qa_plugin_browser_mcp_artifact_and_image_capability_surface_smokes() {
    let unsupported_capabilities = [
        "browser.open",
        "browser.click",
        "document.create_docx",
        "spreadsheet.create_xlsx",
        "presentation.create_pptx",
        "image.generate",
    ];
    let model_gateway =
        RebornTraceReplayModelGateway::with_scripted_steps([RebornModelReplayStep::Response {
            response: HostManagedModelResponse::assistant_reply(
                "qa unavailable plugin artifact surface smoke complete",
            ),
            expected_tool_results: Vec::new(),
        }]);
    let mut harness = RebornBinaryE2EHarness::with_host_runtime_qa_smoke_capabilities(
        "room-qa-capability-surface",
        model_gateway,
    )
    .await
    .expect("qa surface harness");
    harness.start();

    let submitted = harness
        .submit_text(
            "event-qa-capability-surface",
            "inspect browser, MCP, plugin, artifact, and image capability availability",
        )
        .await
        .expect("submit surface smoke");
    harness
        .wait_for_status_with_config(submitted.run_id, TurnStatus::Completed, qa_wait())
        .await
        .expect("surface smoke completes");
    harness
        .assert_final_reply("qa unavailable plugin artifact surface smoke complete")
        .await
        .expect("final reply");

    let first_request = harness
        .model_requests()
        .into_iter()
        .next()
        .expect("model request");
    let prompt_surface = first_request
        .messages
        .iter()
        .map(|message| message.content.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    for unsupported in unsupported_capabilities {
        assert!(
            !prompt_surface.contains(unsupported),
            "{unsupported} should not be advertised by the Reborn first-party QA surface"
        );
    }
    harness.assert_model_exhausted();
    harness.shutdown().await;
}

#[tokio::test]
async fn qa_github_capability_smoke_discovers_actions_without_live_github() {
    let model_gateway = RebornTraceReplayModelGateway::with_scripted_steps([
        RebornModelReplayStep::AssertProviderToolsThenResponse {
            capability_ids: github_support::capability_ids().expect("github capability ids"),
            response: HostManagedModelResponse::assistant_reply("qa github smoke complete"),
            expected_tool_results: Vec::new(),
        },
    ]);
    let mut harness = RebornBinaryE2EHarness::with_host_runtime_github_issue_capabilities(
        "room-qa-github",
        model_gateway,
    )
    .await
    .expect("github harness");
    harness.start();

    let submitted = harness
        .submit_text(
            "event-qa-github",
            "inspect GitHub capability availability without contacting GitHub",
        )
        .await
        .expect("submit github smoke");
    harness
        .wait_for_status_with_config(submitted.run_id, TurnStatus::Completed, qa_wait())
        .await
        .expect("github smoke completes");
    harness
        .assert_final_reply("qa github smoke complete")
        .await
        .expect("final reply");
    assert!(
        harness.runtime_http_requests().is_empty(),
        "GitHub discovery smoke must not contact GitHub"
    );
    assert!(
        harness.network_http_requests().is_empty(),
        "GitHub discovery smoke must not contact GitHub"
    );
    harness.assert_model_exhausted();
    harness.shutdown().await;
}

async fn run_qa_harness(
    conversation_id: &str,
    event_id: &str,
    prompt: &str,
    steps: impl IntoIterator<Item = RebornModelReplayStep>,
) -> RebornBinaryE2EHarness {
    let model_gateway = RebornTraceReplayModelGateway::with_scripted_steps(steps);
    let mut harness = RebornBinaryE2EHarness::with_host_runtime_qa_smoke_capabilities(
        conversation_id,
        model_gateway,
    )
    .await
    .expect("qa smoke harness");
    harness.start();
    let submitted = harness
        .submit_text(event_id, prompt)
        .await
        .expect("submit qa smoke");
    harness
        .wait_for_status_with_config(submitted.run_id, TurnStatus::Completed, qa_wait())
        .await
        .expect("qa smoke completes");
    harness.assert_model_exhausted();
    harness
}

fn cap(id: &str) -> CapabilityId {
    CapabilityId::new(id).expect("valid capability id")
}

fn capability_ids(ids: &[&str]) -> Vec<CapabilityId> {
    ids.iter().map(|id| cap(id)).collect()
}

fn qa_wait() -> WaitConfig {
    WaitConfig {
        timeout: Duration::from_secs(60),
        poll_interval: Duration::from_millis(20),
    }
}

fn call(
    capability_id: &CapabilityId,
    call_id: &str,
    arguments: serde_json::Value,
) -> RebornScriptedProviderToolCall {
    RebornScriptedProviderToolCall::new(capability_id.clone(), call_id, arguments)
}

fn capability_order(
    invocations: &[ironclaw_turns::run_profile::CapabilityInvocation],
) -> Vec<&str> {
    invocations
        .iter()
        .map(|invocation| invocation.capability_id.as_str())
        .collect()
}

fn tool_result_count(request: &ironclaw_loop_support::HostManagedModelRequest) -> usize {
    request
        .messages
        .iter()
        .filter(|message| message.role == HostManagedModelMessageRole::ToolResult)
        .count()
}
