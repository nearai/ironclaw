//! Binary-level E2E for the no-borking-failures contract (PR #4841).
//!
//! Proves the full headline promise end-to-end against a real Reborn run:
//! a provider failure produces a sanitized, *retryable* run failure (not a
//! borking executor error), and retrying that run resumes from the preserved
//! checkpoint and completes — rather than restarting from scratch or stranding
//! the user on a dead run. The per-layer behavior is covered by unit/contract
//! tests; this test locks the whole loop→runner→store→coordinator path.

#[allow(dead_code)]
#[path = "support/reborn_parity_qa/mod.rs"]
mod parity_qa_support;
#[allow(dead_code)]
#[path = "integration/support/mod.rs"]
mod reborn_support;
mod support;

use ironclaw_host_api::CapabilityId;
use ironclaw_loop_host::{HostManagedModelErrorKind, HostManagedModelResponse};
use ironclaw_runner::failure_lane::{FailureLane, failure_lane};
use ironclaw_runner::retry_disposition::{RetryDisposition, retry_disposition};
use ironclaw_turns::{TurnRunState, TurnStatus, run_profile::LoopHostMilestoneKind};
use parity_qa_support::{
    binary_e2e::RebornBinaryE2EHarness,
    model_replay::{
        RebornModelReplayStep, RebornScriptedProviderToolCall, RebornTraceReplayModelGateway,
    },
};
use reborn_support::doubles::RecordingTestCapabilityPort;
use serde_json::json;

/// First model call fails (provider rejects the request); the run must end as a
/// sanitized, retryable `Failed`. Retrying resumes from the `BeforeModel`
/// checkpoint and the second (resumed) model call succeeds, completing the run.
#[tokio::test]
async fn reborn_model_failure_is_retryable_and_retry_resumes_to_completion() {
    let model_gateway = RebornTraceReplayModelGateway::with_scripted_steps([
        // First model call: the provider rejects the request. This maps to a
        // model-stage host-unavailable failure with no internal retry loop.
        RebornModelReplayStep::ModelError {
            kind: HostManagedModelErrorKind::InvalidRequest,
            message: "model provider rejected the request".to_string(),
        },
        // The retry's resumed model call succeeds with a final reply.
        RebornModelReplayStep::Response {
            response: HostManagedModelResponse::assistant_reply("Recovered: here is your answer."),
            expected_tool_results: Vec::new(),
        },
    ]);
    let mut harness = RebornBinaryE2EHarness::with_model_gateway(
        "room-failure-retry-resume",
        model_gateway,
        RecordingTestCapabilityPort::echo(),
    )
    .await
    .expect("harness");
    harness.start();

    // 1) The run fails with a sanitized, actionable category — not a borking
    //    `HostUnavailable` executor error reaching the user.
    let submitted = harness
        .submit_text("event-failure-retry-resume", "Answer my question")
        .await
        .expect("submit text");
    let failed = harness
        .wait_for_status(submitted.run_id, TurnStatus::Failed)
        .await
        .expect("failed run");
    assert_failure_lane_alignment(
        &failed,
        "host_stage_unavailable_model",
        FailureLane::Retriable,
        RetryDisposition::Auto,
    );

    // 2) The failed run is retryable: it preserved a resumable checkpoint. This
    //    is exactly what the projection surfaces as `retryable: true`.
    assert!(
        failed.checkpoint_id.is_some(),
        "a retryable model-stage failure must preserve a resume checkpoint"
    );

    // The failed run must not fabricate a final assistant reply.
    assert!(
        !harness.milestones().iter().any(|milestone| matches!(
            milestone.kind,
            LoopHostMilestoneKind::AssistantReplyFinalized { .. }
        )),
        "a failed model call must not fabricate a final assistant reply"
    );

    // 3) Retrying spawns a new run that resumes from the checkpoint and
    //    completes — the loop did not restart from scratch and did not strand
    //    the user on a dead run.
    let retry = harness
        .retry_turn(submitted.run_id)
        .await
        .expect("retry the failed run");
    assert_ne!(
        retry.run_id, submitted.run_id,
        "retry must spawn a distinct run"
    );
    assert_eq!(retry.status, TurnStatus::Queued);

    harness
        .wait_for_status(retry.run_id, TurnStatus::Completed)
        .await
        .expect("retry run completes");
    harness
        .assert_final_reply("Recovered: here is your answer.")
        .await
        .expect("recovered reply persisted to the thread");

    // Both scripted steps were consumed: the failing call and the recovered
    // retry call.
    assert_eq!(harness.remaining_model_responses(), 0);

    harness.shutdown().await;
}

/// A host-managed model call can surface `Cancelled` without any cooperative
/// cancel signal. `planned_driver` maps that executor error to
/// `interrupted_unexpectedly`; the binary runner currently projects the
/// non-allowlisted driver failure as the generic driver category.
#[tokio::test]
async fn reborn_inflight_model_cancelled_projects_driver_failed_divergence() {
    let model_gateway =
        RebornTraceReplayModelGateway::with_scripted_steps([RebornModelReplayStep::ModelError {
            kind: HostManagedModelErrorKind::Cancelled,
            message: "model provider cancelled the in-flight request".to_string(),
        }]);
    let mut harness = RebornBinaryE2EHarness::with_model_gateway(
        "room-model-cancelled-divergence",
        model_gateway,
        RecordingTestCapabilityPort::echo(),
    )
    .await
    .expect("harness");
    harness.start();

    let submitted = harness
        .submit_text(
            "event-model-cancelled-divergence",
            "Answer after a cancelled model call",
        )
        .await
        .expect("submit text");
    let failed = harness
        .wait_for_status(submitted.run_id, TurnStatus::Failed)
        .await
        .expect("failed run");
    // matrix-divergence: `map_executor_error` produces
    // "interrupted_unexpectedly", but `TurnRunnerWorker::sanitized_driver_failure`
    // only preserves allowlisted driver reason kinds and currently projects the
    // binary run category as "driver_failed".
    assert_failure_lane_alignment(
        &failed,
        "driver_failed",
        FailureLane::Retriable,
        RetryDisposition::UserInitiated,
    );
    assert!(
        failed.checkpoint_id.is_some(),
        "a model-stage cancellation before a trustworthy LoopExit still preserves the BeforeModel checkpoint"
    );
    assert!(
        !harness.milestones().iter().any(|milestone| matches!(
            milestone.kind,
            LoopHostMilestoneKind::AssistantReplyFinalized { .. }
        )),
        "a cancelled model call must not fabricate a final assistant reply"
    );
    assert_eq!(harness.remaining_model_responses(), 0);

    harness.shutdown().await;
}

/// The first run reaches the capability stage and the capability host returns a
/// permanent invocation error. The runner must still fail the run with a
/// retryable capability-stage category, preserve the checkpoint, and allow a
/// retry to resume to a final answer.
#[tokio::test]
async fn reborn_capability_failure_is_retryable_and_retry_resumes_to_completion() {
    let model_gateway = RebornTraceReplayModelGateway::with_scripted_steps([
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![RebornScriptedProviderToolCall::new(
                CapabilityId::new("test.echo").expect("valid capability id"),
                "call-capability-fails",
                json!({"message": "please use the test capability"}),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::Response {
            response: HostManagedModelResponse::assistant_reply(
                "Recovered after capability failure.",
            ),
            expected_tool_results: Vec::new(),
        },
    ]);
    let mut harness = RebornBinaryE2EHarness::with_model_gateway(
        "room-capability-failure-retry-resume",
        model_gateway,
        RecordingTestCapabilityPort::invocation_error(),
    )
    .await
    .expect("harness");
    harness.start();

    let submitted = harness
        .submit_text(
            "event-capability-failure-retry-resume",
            "Use the test capability and then answer",
        )
        .await
        .expect("submit text");
    let failed = harness
        .wait_for_status(submitted.run_id, TurnStatus::Failed)
        .await
        .expect("failed run");
    assert_failure_lane_alignment(
        &failed,
        "host_stage_unavailable_capability",
        FailureLane::Retriable,
        RetryDisposition::Auto,
    );
    assert!(
        failed.checkpoint_id.is_some(),
        "a retryable capability-stage failure must preserve a resume checkpoint"
    );
    assert_eq!(
        harness.capability_invocations().len(),
        1,
        "the failed first run must reach exactly one capability invocation"
    );
    assert!(
        !harness.milestones().iter().any(|milestone| matches!(
            milestone.kind,
            LoopHostMilestoneKind::AssistantReplyFinalized { .. }
        )),
        "a failed capability invocation must not fabricate a final assistant reply"
    );

    let retry = harness
        .retry_turn(submitted.run_id)
        .await
        .expect("retry the failed run");
    assert_ne!(
        retry.run_id, submitted.run_id,
        "retry must spawn a distinct run"
    );
    assert_eq!(retry.status, TurnStatus::Queued);

    harness
        .wait_for_status(retry.run_id, TurnStatus::Completed)
        .await
        .expect("retry run completes");
    harness
        .assert_final_reply("Recovered after capability failure.")
        .await
        .expect("recovered reply persisted to the thread");
    assert_eq!(harness.remaining_model_responses(), 0);

    harness.shutdown().await;
}

fn assert_failure_lane_alignment(
    state: &TurnRunState,
    expected_category: &str,
    expected_lane: FailureLane,
    expected_disposition: RetryDisposition,
) {
    let failure = state.failure.as_ref().expect("failure category");
    let category = failure.category();
    let retryable = state.checkpoint_id.is_some();

    assert_eq!(category, expected_category, "sanitized failure category");
    assert_eq!(
        failure_lane(category, retryable),
        expected_lane,
        "{category}: FailureLane must match the emitted category + retryable signal"
    );
    let disposition = retry_disposition(category, retryable);
    assert_eq!(
        disposition, expected_disposition,
        "{category}: RetryDisposition must match the emitted category + retryable signal"
    );
    assert_eq!(
        disposition.failure_lane(),
        expected_lane,
        "{category}: RetryDisposition must imply the same FailureLane"
    );
}
