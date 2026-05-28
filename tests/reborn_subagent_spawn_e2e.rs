#[allow(dead_code)]
#[path = "support/reborn/mod.rs"]
mod reborn_support;
mod support;

use std::time::Duration;

use ironclaw_host_api::CapabilityId;
use ironclaw_host_runtime::READ_FILE_CAPABILITY_ID;
use ironclaw_loop_support::{
    DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID, HostManagedModelMessageRole, HostManagedModelResponse,
};
use ironclaw_turns::TurnStatus;
use reborn_support::{
    config::WaitConfig,
    harness::{RebornBinaryE2EHarness, RecordingTestCapabilityPort, SubmittedTurn},
    model_replay::{
        RebornModelReplayStep, RebornScriptedProviderToolCall, RebornTraceReplayModelGateway,
    },
};

#[tokio::test]
async fn blocking_spawn_parks_parent_then_resumes_with_child_result() {
    let model_gateway = RebornTraceReplayModelGateway::with_scripted_steps([
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![spawn_call(
                "spawn_blocking",
                serde_json::json!({
                    "flavor_id": "general",
                    "task": "answer for the parent",
                }),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::DelayedResponse {
            response: HostManagedModelResponse::assistant_reply("blocking child output"),
            delay: Duration::from_millis(50),
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::Response {
            response: HostManagedModelResponse::assistant_reply("parent resumed"),
            expected_tool_results: Vec::new(),
        },
    ]);
    let mut harness = spawn_harness("room-subagent-blocking", model_gateway).await;
    harness.start();

    let submitted = harness
        .submit_text("event-subagent-blocking", "delegate and wait")
        .await
        .expect("submit root turn");
    harness
        .wait_for_status(submitted.run_id, TurnStatus::BlockedDependentRun)
        .await
        .expect("parent parks on dependent child");

    let child = await_single_child(&harness, &submitted).await;
    harness
        .wait_for_status_in_scope(child.scope.clone(), child.run_id, TurnStatus::Completed)
        .await
        .expect("blocking child completes");
    harness
        .wait_for_status(submitted.run_id, TurnStatus::Completed)
        .await
        .expect("parent resumes after child completion");
    harness
        .assert_final_reply("parent resumed")
        .await
        .expect("parent final reply");
    assert_child_thread_invariants(&submitted, &child);
    assert!(
        harness.model_requests()[2]
            .messages
            .iter()
            .any(
                |message| message.role == HostManagedModelMessageRole::ToolResult
                    && message.content.contains("Subagent completed")
            ),
        "parent resume request includes the child completion tool result: {:#?}",
        harness.model_requests()[2].messages
    );
    harness.assert_model_exhausted();
    harness.shutdown().await;
}

#[tokio::test]
async fn legacy_explicit_blocking_spawn_still_parks_parent_and_resumes() {
    let model_gateway = RebornTraceReplayModelGateway::with_scripted_steps([
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![spawn_call(
                "spawn_legacy_explicit_blocking",
                serde_json::json!({
                    "flavor_id": "general",
                    "task": "answer with legacy blocking fields",
                    "mode": "blocking",
                    "run_in_background": false,
                }),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::DelayedResponse {
            response: HostManagedModelResponse::assistant_reply("legacy blocking child output"),
            delay: Duration::from_millis(50),
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::Response {
            response: HostManagedModelResponse::assistant_reply("legacy blocking parent resumed"),
            expected_tool_results: Vec::new(),
        },
    ]);
    let mut harness = spawn_harness("room-subagent-legacy-blocking", model_gateway).await;
    harness.start();

    let submitted = harness
        .submit_text(
            "event-subagent-legacy-blocking",
            "delegate with legacy blocking",
        )
        .await
        .expect("submit root turn");
    harness
        .wait_for_status(submitted.run_id, TurnStatus::BlockedDependentRun)
        .await
        .expect("parent parks on legacy blocking child");

    let child = await_single_child(&harness, &submitted).await;
    assert_child_thread_invariants(&submitted, &child);
    harness
        .wait_for_status_in_scope(child.scope.clone(), child.run_id, TurnStatus::Completed)
        .await
        .expect("legacy blocking child completes");
    harness
        .wait_for_status(submitted.run_id, TurnStatus::Completed)
        .await
        .expect("parent resumes after legacy blocking child completion");
    harness
        .assert_final_reply("legacy blocking parent resumed")
        .await
        .expect("parent final reply");
    harness.assert_model_exhausted();
    harness.shutdown().await;
}

#[tokio::test]
async fn background_spawn_is_rejected_before_child_run_or_auth_invocation() {
    let model_gateway = RebornTraceReplayModelGateway::with_scripted_steps([
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![spawn_call(
                "spawn_background_disabled",
                serde_json::json!({
                    "flavor_id": "general",
                    "task": "run in the background",
                    "mode": "background",
                }),
            )],
            expected_tool_results: Vec::new(),
        },
    ]);
    let mut harness = spawn_harness("room-subagent-background-disabled", model_gateway).await;
    harness.start();

    let submitted = harness
        .submit_text(
            "event-subagent-background-disabled",
            "try background delegation",
        )
        .await
        .expect("submit root turn");
    let state = harness
        .wait_for_status(submitted.run_id, TurnStatus::RecoveryRequired)
        .await
        .expect("background spawn rejects the parent turn");

    assert_eq!(
        state.failure.expect("failure category").category(),
        "driver_unavailable"
    );
    assert!(
        harness
            .children_of(&submitted.scope, submitted.run_id)
            .await
            .expect("children")
            .is_empty(),
        "disabled background spawn must not create a child run"
    );
    assert!(
        harness.capability_invocations().is_empty(),
        "disabled background spawn must reject before inner authorization invocation"
    );
    harness.assert_model_exhausted();
    harness.shutdown().await;
}

#[tokio::test]
async fn blocking_spawn_waits_while_child_is_blocked_on_approval_then_resumes() {
    let model_gateway = RebornTraceReplayModelGateway::with_scripted_steps([
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![spawn_call(
                "spawn_blocking_child_approval",
                serde_json::json!({
                    "flavor_id": "general",
                    "task": "call an approval-gated tool",
                }),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![subagent_allowed_tool_call("child_approval_tool")],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::Response {
            response: HostManagedModelResponse::assistant_reply("child approved output"),
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::Response {
            response: HostManagedModelResponse::assistant_reply("parent saw approved child"),
            expected_tool_results: Vec::new(),
        },
    ]);
    let mut harness = tokio::time::timeout(
        WaitConfig::default().timeout,
        RebornBinaryE2EHarness::with_harness_blocked_evidence_unscoped_worker(
            "room-subagent-child-approval",
            model_gateway,
            RecordingTestCapabilityPort::spawn_auth_then_approval_then_allowed_tool_with_spawn_subagent(),
        ),
    )
    .await
    .expect("spawn harness timed out")
    .expect("spawn harness");
    harness.start();

    let submitted = harness
        .submit_text(
            "event-subagent-child-approval",
            "spawn a child that needs approval",
        )
        .await
        .expect("submit root turn");
    harness
        .wait_for_status(submitted.run_id, TurnStatus::BlockedDependentRun)
        .await
        .expect("parent blocks on approval-gated child");

    let child = await_single_child(&harness, &submitted).await;
    assert_child_thread_invariants(&submitted, &child);
    harness
        .wait_for_status_in_scope(
            child.scope.clone(),
            child.run_id,
            TurnStatus::BlockedApproval,
        )
        .await
        .expect("child blocks on approval");
    assert_eq!(
        harness
            .run_state(submitted.run_id)
            .await
            .expect("parent run state")
            .status,
        TurnStatus::BlockedDependentRun,
        "parent must remain parked while the child approval is pending"
    );

    harness
        .resume_blocked_turn_in_scope(child.scope.clone(), submitted.actor.clone(), child.run_id)
        .await
        .expect("resume child approval");
    harness
        .wait_for_status_in_scope(child.scope.clone(), child.run_id, TurnStatus::Completed)
        .await
        .expect("child completes after approval");
    harness
        .wait_for_status(submitted.run_id, TurnStatus::Completed)
        .await
        .expect("parent resumes after approved child completion");
    harness
        .assert_final_reply("parent saw approved child")
        .await
        .expect("parent final reply");
    assert!(
        harness
            .model_requests()
            .last()
            .expect("parent resume request")
            .messages
            .iter()
            .any(
                |message| message.role == HostManagedModelMessageRole::ToolResult
                    && message.content.contains("child approved output")
            ),
        "parent resume request includes the approved child's final output"
    );
    assert_eq!(
        harness.capability_invocations().len(),
        2,
        "spawn auth plus the child approval gate should reach the inner capability port"
    );
    harness.assert_model_exhausted();
    harness.shutdown().await;
}

#[tokio::test]
async fn parallel_blocking_spawn_resumes_once_after_last_child() {
    let model_gateway = RebornTraceReplayModelGateway::with_scripted_steps([
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![
                spawn_call(
                    "spawn_blocking_a",
                    serde_json::json!({
                        "flavor_id": "general",
                        "task": "same goal",
                    }),
                ),
                spawn_call(
                    "spawn_blocking_b",
                    serde_json::json!({
                        "flavor_id": "general",
                        "task": "same goal",
                    }),
                ),
                spawn_call(
                    "spawn_blocking_c",
                    serde_json::json!({
                        "flavor_id": "general",
                        "task": "same goal",
                    }),
                ),
            ],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::DelayedResponse {
            response: HostManagedModelResponse::assistant_reply("child one"),
            delay: Duration::from_millis(50),
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::DelayedResponse {
            response: HostManagedModelResponse::assistant_reply("child two"),
            delay: Duration::from_millis(50),
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::DelayedResponse {
            response: HostManagedModelResponse::assistant_reply("child three"),
            delay: Duration::from_millis(50),
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::Response {
            response: HostManagedModelResponse::assistant_reply("all children complete"),
            expected_tool_results: Vec::new(),
        },
    ]);
    let mut harness = spawn_harness("room-subagent-parallel-blocking", model_gateway).await;
    harness.start();

    let submitted = harness
        .submit_text("event-subagent-parallel-blocking", "spawn three children")
        .await
        .expect("submit root turn");
    harness
        .wait_for_status(submitted.run_id, TurnStatus::BlockedDependentRun)
        .await
        .expect("parent blocks on child set");

    let children = await_children(&harness, &submitted, 3).await;
    let child_run_ids = children
        .iter()
        .map(|child| child.run_id)
        .collect::<std::collections::HashSet<_>>();
    assert_eq!(child_run_ids.len(), 3, "each spawn creates a distinct run");
    for child in &children {
        assert_child_thread_invariants(&submitted, child);
    }
    harness
        .wait_for_status(submitted.run_id, TurnStatus::Completed)
        .await
        .expect("parent resumes after all children");
    harness
        .assert_final_reply("all children complete")
        .await
        .expect("parent final reply");
    assert!(
        harness.model_requests()[4]
            .messages
            .iter()
            .filter(
                |message| message.role == HostManagedModelMessageRole::ToolResult
                    && message.content.contains("Subagent completed")
            )
            .count()
            >= 3,
        "parent resume request contains all child completion results"
    );
    harness.assert_model_exhausted();
    harness.shutdown().await;
}

async fn spawn_harness(
    conversation_id: &str,
    model_gateway: RebornTraceReplayModelGateway,
) -> RebornBinaryE2EHarness {
    tokio::time::timeout(
        WaitConfig::default().timeout,
        RebornBinaryE2EHarness::with_harness_blocked_evidence_unscoped_worker(
            conversation_id,
            model_gateway,
            RecordingTestCapabilityPort::echo_with_spawn_subagent(),
        ),
    )
    .await
    .expect("spawn harness timed out")
    .expect("spawn harness")
}

fn spawn_call(
    call_id: impl Into<String>,
    arguments: serde_json::Value,
) -> RebornScriptedProviderToolCall {
    RebornScriptedProviderToolCall::new(spawn_capability_id(), call_id, arguments)
}

fn spawn_capability_id() -> CapabilityId {
    CapabilityId::new(DEFAULT_SPAWN_SUBAGENT_CAPABILITY_ID).expect("valid capability id")
}

fn subagent_allowed_tool_call(call_id: impl Into<String>) -> RebornScriptedProviderToolCall {
    RebornScriptedProviderToolCall::new(
        CapabilityId::new(READ_FILE_CAPABILITY_ID).expect("valid capability id"),
        call_id,
        serde_json::json!({"message": "hi"}),
    )
}

async fn await_single_child(
    harness: &RebornBinaryE2EHarness,
    submitted: &SubmittedTurn,
) -> ironclaw_turns::TurnRunRecord {
    let mut children = await_children(harness, submitted, 1).await;
    children.pop().expect("one child")
}

async fn await_children(
    harness: &RebornBinaryE2EHarness,
    submitted: &SubmittedTurn,
    expected: usize,
) -> Vec<ironclaw_turns::TurnRunRecord> {
    let wait = WaitConfig::default();
    let deadline = tokio::time::Instant::now() + wait.timeout;
    loop {
        let children = harness
            .children_of(&submitted.scope, submitted.run_id)
            .await
            .expect("children");
        if children.len() >= expected {
            return children;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!(
                "timed out waiting for {expected} children; observed {}",
                children.len()
            );
        }
        tokio::time::sleep(wait.poll_interval).await;
    }
}

fn assert_child_thread_invariants(parent: &SubmittedTurn, child: &ironclaw_turns::TurnRunRecord) {
    assert_eq!(child.parent_run_id, Some(parent.run_id));
    assert_eq!(child.subagent_depth, 1);
    assert_eq!(child.spawn_tree_root_run_id, Some(parent.run_id));
    assert_eq!(child.scope.tenant_id, parent.scope.tenant_id);
    assert_eq!(child.scope.agent_id, parent.scope.agent_id);
    assert_eq!(child.scope.project_id, parent.scope.project_id);
    assert_ne!(
        child.scope.thread_id, parent.scope.thread_id,
        "child must run on a distinct thread"
    );
}
