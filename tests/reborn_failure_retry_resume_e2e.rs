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

/// A single stale capability surface is recoverable inside the same run: the
/// loop re-drives the model from `BeforeModel`, consumes a second model turn,
/// and persists the recovered reply without requiring an external retry.
#[tokio::test]
async fn reborn_single_stale_model_request_redrives_in_loop_to_completion() {
    let model_gateway = RebornTraceReplayModelGateway::with_scripted_steps([
        RebornModelReplayStep::ModelError {
            kind: HostManagedModelErrorKind::StaleRequest,
            message: "capability surface changed before model dispatch".to_string(),
        },
        RebornModelReplayStep::Response {
            response: HostManagedModelResponse::assistant_reply(
                "Recovered after refreshing the capability surface.",
            ),
            expected_tool_results: Vec::new(),
        },
    ]);
    let mut harness = RebornBinaryE2EHarness::with_model_gateway(
        "room-single-stale-model-redrive",
        model_gateway,
        RecordingTestCapabilityPort::echo(),
    )
    .await
    .expect("harness");
    harness.start();

    let submitted = harness
        .submit_text(
            "event-single-stale-model-redrive",
            "Answer using the current capability surface",
        )
        .await
        .expect("submit text");
    harness
        .wait_for_status(submitted.run_id, TurnStatus::Completed)
        .await
        .expect("one stale request is recovered in-loop");
    harness
        .assert_final_reply("Recovered after refreshing the capability surface.")
        .await
        .expect("recovered reply persisted to the thread");

    assert_eq!(
        harness.model_requests().len(),
        2,
        "one stale request must cause exactly one same-run model re-drive"
    );
    assert_eq!(harness.remaining_model_responses(), 0);

    harness.shutdown().await;
}

/// Repeated stale model requests exhaust in-loop recovery; the run must end as
/// a sanitized, retryable `Failed`. Retrying resumes from the `BeforeModel`
/// checkpoint and the next model call succeeds, completing the run.
#[tokio::test]
async fn reborn_model_failure_is_retryable_and_retry_resumes_to_completion() {
    let model_gateway = RebornTraceReplayModelGateway::with_scripted_steps([
        // Typed stale requests are retried twice in-loop. Three consecutive
        // failures exhaust that budget and leave the successful response for
        // the externally resumed run.
        RebornModelReplayStep::ModelError {
            kind: HostManagedModelErrorKind::StaleRequest,
            message: "model provider rejected the request".to_string(),
        },
        RebornModelReplayStep::ModelError {
            kind: HostManagedModelErrorKind::StaleRequest,
            message: "model provider rejected the retried request".to_string(),
        },
        RebornModelReplayStep::ModelError {
            kind: HostManagedModelErrorKind::StaleRequest,
            message: "model provider rejected the final in-loop retry".to_string(),
        },
        // The externally retried run resumes and succeeds with a final reply.
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
        "model_stale_request",
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

    // All scripted steps were consumed: three in-loop failures and the
    // recovered external retry call.
    assert_eq!(harness.remaining_model_responses(), 0);

    harness.shutdown().await;
}

#[tokio::test]
async fn reborn_invalid_model_request_fails_without_in_loop_retry() {
    let model_gateway = RebornTraceReplayModelGateway::with_scripted_steps([
        RebornModelReplayStep::ModelError {
            kind: HostManagedModelErrorKind::InvalidRequest,
            message: "model request is deterministically invalid".to_string(),
        },
        RebornModelReplayStep::Response {
            response: HostManagedModelResponse::assistant_reply("must remain unused"),
            expected_tool_results: Vec::new(),
        },
    ]);
    let mut harness = RebornBinaryE2EHarness::with_model_gateway(
        "room-invalid-model-request",
        model_gateway,
        RecordingTestCapabilityPort::echo(),
    )
    .await
    .expect("harness");
    harness.start();

    let submitted = harness
        .submit_text("event-invalid-model-request", "Answer my question")
        .await
        .expect("submit text");
    harness
        .wait_for_status(submitted.run_id, TurnStatus::Failed)
        .await
        .expect("invalid request fails the run");

    assert_eq!(
        harness.remaining_model_responses(),
        1,
        "deterministic InvalidRequest must not consume an in-loop retry"
    );

    harness.shutdown().await;
}

/// A host-managed model call can surface `Cancelled` without any cooperative
/// cancel signal. `planned_driver` maps that executor error to
/// `interrupted_unexpectedly`, and the binary runner preserves that category
/// on the durable failure record (§5a.5 closed — it previously overwrote it
/// with the generic `driver_failed`).
#[tokio::test]
async fn reborn_inflight_model_cancelled_preserves_interrupted_unexpectedly() {
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
    // §5a.5 closed: `map_executor_error` produces "interrupted_unexpectedly"
    // and `sanitized_driver_failure` now preserves it end-to-end, so the
    // durable run failure carries the original category instead of the
    // masking "driver_failed".
    assert_failure_lane_alignment(
        &failed,
        "interrupted_unexpectedly",
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

    assert_eq!(
        category, expected_category,
        "sanitized failure category: {failure:?}"
    );
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
