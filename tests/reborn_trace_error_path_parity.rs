#[allow(dead_code)]
#[path = "support/reborn_parity_qa/mod.rs"]
mod parity_qa_support;
#[allow(dead_code)]
#[path = "integration/support/mod.rs"]
mod reborn_support;
mod support;

use ironclaw_host_api::CapabilityId;
use ironclaw_host_runtime::READ_FILE_CAPABILITY_ID;
use ironclaw_turns::{TurnStatus, run_profile::LoopHostMilestoneKind};
use parity_qa_support::{
    binary_e2e::RebornBinaryE2EHarness,
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
    let failure = state.failure.expect("failure category");
    assert_eq!(failure.category(), "driver_protocol_violation");
    // The durable failure record must name WHICH loop-exit protocol rule was
    // broken (loop-failure matrix §5a.6): the driver's Failed exit could not be
    // evidence-verified, and that specific violation kind survives on the
    // sanitized failure detail instead of collapsing into the bare category.
    assert_eq!(
        failure.detail(),
        Some("loop exit violation: unverified_failure_evidence"),
        "the specific loop-exit violation kind must survive on the durable failure detail"
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
///
/// Contract update (#6284 item 1): an unadvertised-capability reference is a
/// stale/invalid model request, which is now model-fixable — the loop retries
/// at iteration scope with a rebuilt capability surface before failing. The
/// script therefore supplies the same unadvertised call for the initial
/// attempt plus both retries, and the exhausted run fails with the precise
/// `model_stale_request` category instead of the old opaque
/// `host_stage_unavailable_model`. Rejection-before-invocation is unchanged:
/// no attempt ever reaches the capability port.
#[tokio::test]
async fn reborn_trace_unadvertised_capability_is_rejected() {
    let read_file = CapabilityId::new(READ_FILE_CAPABILITY_ID).expect("valid capability id");
    let unadvertised_call = |call_id: &str| RebornModelReplayStep::ProviderToolCalls {
        calls: vec![RebornScriptedProviderToolCall::new(
            read_file.clone(),
            call_id,
            serde_json::json!({
                "path": "/workspace/should-not-be-visible.txt",
            }),
        )],
        expected_tool_results: Vec::new(),
    };
    let model_gateway = RebornTraceReplayModelGateway::with_scripted_steps([
        // Initial attempt plus both iteration-scoped stale-request retries,
        // exhausting DefaultRecoveryStrategy::max_attempts_per_class (2).
        unadvertised_call("call_read_file_unadvertised"),
        unadvertised_call("call_read_file_unadvertised_retry_1"),
        unadvertised_call("call_read_file_unadvertised_retry_2"),
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
    // WS-3 upgraded the opaque `driver_unavailable` category to a
    // stage-scoped category; #6284 item 1 upgraded it again to the precise,
    // model-fixable `model_stale_request` after in-loop surface-rebuild
    // retries exhaust.
    assert_eq!(
        state.failure.expect("failure category").category(),
        "model_stale_request"
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
