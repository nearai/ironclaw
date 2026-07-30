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

use ironclaw_event_projections::TimelineEntryKind;
use ironclaw_host_api::CapabilityId;
use ironclaw_loop_host::{HostManagedModelErrorKind, HostManagedModelResponse};
use ironclaw_turns::{TurnRunId, TurnRunState, TurnStatus, run_profile::LoopHostMilestoneKind};
use parity_qa_support::{
    binary_e2e::RebornBinaryE2EHarness,
    model_replay::{
        RebornModelReplayStep, RebornScriptedProviderToolCall, RebornTraceReplayModelGateway,
    },
};
use reborn_support::doubles::RecordingTestCapabilityPort;
use serde_json::json;

/// A stale model request injected at the model gateway is recoverable inside
/// the same run: the loop re-drives the model from `BeforeModel`, consumes a
/// second model turn, and persists the recovered reply without requiring an
/// external retry.
#[tokio::test]
async fn reborn_single_stale_model_request_redrives_in_loop_to_completion() {
    let model_gateway = RebornTraceReplayModelGateway::with_scripted_steps([
        RebornModelReplayStep::ModelError {
            kind: HostManagedModelErrorKind::StaleRequest,
            message: "capability surface changed before model dispatch".to_string(),
        },
        RebornModelReplayStep::Response {
            response: HostManagedModelResponse::assistant_reply(
                "Recovered after retrying the stale model request.",
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
        .assert_final_reply("Recovered after retrying the stale model request.")
        .await
        .expect("recovered reply persisted to the thread");

    assert_eq!(
        harness.model_requests().len(),
        2,
        "one stale request must cause exactly one same-run model re-drive"
    );
    assert_eq!(harness.remaining_model_responses(), 0);
    assert_single_durable_recovery(
        &harness,
        submitted.run_id,
        "model",
        "model_stale_request",
        "retried",
    )
    .await;

    harness.shutdown().await;
}

/// Repeated stale model requests exhaust in-loop recovery; the run must end as
/// a sanitized, retryable `Failed`. Retrying resumes from the `BeforeModel`
/// checkpoint and the next model call succeeds, completing the run.
#[tokio::test]
async fn reborn_model_failure_is_retryable_and_retry_resumes_to_completion() {
    let model_gateway = RebornTraceReplayModelGateway::with_scripted_steps([
        // Typed stale requests are retried twice in-loop, then receive one
        // observation-assisted attempt. A fourth consecutive failure aborts
        // and leaves the successful response for the externally resumed run.
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
        RebornModelReplayStep::ModelError {
            kind: HostManagedModelErrorKind::StaleRequest,
            message: "model provider rejected the observation-assisted retry".to_string(),
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
    assert_failure_retry_contract(&failed, "model_stale_request", true);

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

    // All scripted steps were consumed: four in-loop failures and the recovered
    // external retry call.
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
    assert_failure_retry_contract(&failed, "interrupted_unexpectedly", true);
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

/// A caller-shaped capability-port failure is surfaced to the model and the
/// same run completes. The recovery numerator must cross the real progress
/// port and runner milestone adapter into the durable runtime projection once.
#[tokio::test]
async fn reborn_capability_failure_recovers_model_visibly_with_one_durable_event() {
    let model_gateway = RebornTraceReplayModelGateway::with_scripted_steps([
        RebornModelReplayStep::ProviderToolCalls {
            calls: vec![RebornScriptedProviderToolCall::new(
                CapabilityId::new("test.echo").expect("valid capability id"),
                "call-capability-recovers",
                json!({"message": "please use the test capability"}),
            )],
            expected_tool_results: Vec::new(),
        },
        RebornModelReplayStep::ResponseForRequest {
            request_contains: "input_encode".to_string(),
            response: HostManagedModelResponse::assistant_reply(
                "Recovered after the model-visible capability failure.",
            ),
            expected_tool_results: Vec::new(),
        },
    ]);
    let mut harness = RebornBinaryE2EHarness::with_model_gateway(
        "room-capability-model-visible-recovery",
        model_gateway,
        RecordingTestCapabilityPort::recoverable_port_error(),
    )
    .await
    .expect("harness");
    harness.start();

    let submitted = harness
        .submit_text(
            "event-capability-model-visible-recovery",
            "Use the test capability and recover from a caller-shaped failure",
        )
        .await
        .expect("submit text");
    let completed = harness
        .wait_for_status(submitted.run_id, TurnStatus::Completed)
        .await
        .expect("caller-shaped capability failure remains model-recoverable");
    assert!(completed.failure.is_none());
    harness
        .assert_final_reply("Recovered after the model-visible capability failure.")
        .await
        .expect("recovered capability reply persisted to the thread");
    assert_eq!(
        harness.capability_invocations().len(),
        1,
        "model-visible recovery must not duplicate the failed capability call"
    );
    assert_single_durable_recovery(
        &harness,
        submitted.run_id,
        "capability",
        "input_encode",
        "model_visible",
    )
    .await;

    harness.shutdown().await;
}

/// The first run reaches the capability stage and the capability host returns
/// a terminal host fault (`Unavailable` — caller-shaped port errors now
/// recover in-loop by fate instead of ending the run). The runner must still
/// fail the run with a retryable capability-stage category, preserve the
/// checkpoint, and allow a retry to resume to a final answer.
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
    assert_failure_retry_contract(&failed, "host_stage_unavailable_capability", true);
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
    assert_eq!(
        harness.capability_invocations().len(),
        1,
        "the retry must continue from the checkpoint without replaying the failed side effect"
    );
    assert_eq!(harness.remaining_model_responses(), 0);

    harness.shutdown().await;
}

async fn assert_single_durable_recovery(
    harness: &RebornBinaryE2EHarness,
    run_id: TurnRunId,
    expected_stage: &str,
    expected_class: &str,
    expected_disposition: &str,
) {
    let projection = harness
        .runtime_projection(run_id)
        .await
        .expect("durable runtime events replay through the production projection");
    let recovery_entries = projection
        .timeline
        .entries
        .iter()
        .filter(|entry| {
            entry.kind == TimelineEntryKind::FailureRecovered
                && entry.invocation_id.as_uuid() == run_id.as_uuid()
        })
        .collect::<Vec<_>>();

    assert_eq!(
        recovery_entries.len(),
        1,
        "one applied recovery must emit exactly one durable numerator event"
    );
    let recovery = recovery_entries[0];
    assert_eq!(
        recovery.recovery_stage.as_deref(),
        Some(expected_stage),
        "recovery stage must remain authoritative through projection"
    );
    assert_eq!(
        recovery.recovery_class.as_deref(),
        Some(expected_class),
        "recovery class must remain authoritative through projection"
    );
    assert_eq!(
        recovery.recovery_disposition.as_deref(),
        Some(expected_disposition),
        "recovery disposition must remain authoritative through projection"
    );
    assert!(
        projection.timeline.entries.iter().any(|entry| {
            entry.kind == TimelineEntryKind::AssistantReplyFinalized
                && entry.invocation_id.as_uuid() == run_id.as_uuid()
        }),
        "the same durable projection must contain the caller-visible finalized reply"
    );
}

fn assert_failure_retry_contract(
    state: &TurnRunState,
    expected_category: &str,
    expected_retryable: bool,
) {
    let failure = state.failure.as_ref().expect("failure category");
    let category = failure.category();
    let retryable = state.checkpoint_id.is_some();

    assert_eq!(
        category, expected_category,
        "sanitized failure category: {failure:?}"
    );
    assert_eq!(retryable, expected_retryable, "{category}: retryability");
}
