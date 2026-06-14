//! Binary-level E2E for the no-borking-failures contract (PR #4841).
//!
//! Proves the full headline promise end-to-end against a real Reborn run:
//! a provider failure produces a sanitized, *retryable* run failure (not a
//! borking executor error), and retrying that run resumes from the preserved
//! checkpoint and completes — rather than restarting from scratch or stranding
//! the user on a dead run. The per-layer behavior is covered by unit/contract
//! tests; this test locks the whole loop→runner→store→coordinator path.

#[allow(dead_code)]
#[path = "support/reborn/mod.rs"]
mod reborn_support;
mod support;

use ironclaw_loop_support::{HostManagedModelErrorKind, HostManagedModelResponse};
use ironclaw_turns::{TurnStatus, run_profile::LoopHostMilestoneKind};
use reborn_support::{
    harness::{RebornBinaryE2EHarness, RecordingTestCapabilityPort},
    model_replay::{RebornModelReplayStep, RebornTraceReplayModelGateway},
};

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
    assert_eq!(
        failed
            .failure
            .as_ref()
            .expect("failure category")
            .category(),
        "host_stage_unavailable_model"
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
