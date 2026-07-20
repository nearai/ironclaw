use std::collections::HashSet;
use std::ops::ControlFlow;

use async_trait::async_trait;
use ironclaw_host_api::{
    ApprovalRequestId, Blocked, CorrelationId, DenyReason, DependentRunResult, FailureKind,
    INPUT_ENCODE_HUMAN_SUMMARY, LoopRef, ModelFailureDiagnostic, ModelInputIssue, Outcome,
    Resolution, ResultProgress, ResumeToken, Suspension, ToolVerdict,
};
use ironclaw_turns::{
    LoopFailureKind, LoopGateRef, LoopResultRef,
    run_profile::{
        AuthResumeApprovalIdentity, CapabilityActivityId, CapabilityApprovalResume,
        CapabilityAuthResume, CapabilityBatchInvocation, CapabilityCallCandidate,
        CapabilityFailure, CapabilityFailureDetail, CapabilityFailureKind, CapabilityInputIssue,
        CapabilityProgress, CapabilityResultMessage, CapabilityResumeToken, ContentDigest,
        LoopDriverNoteKind, LoopProcessRef, LoopProgressEvent,
        MODEL_VISIBLE_TOOL_OBSERVATION_SCHEMA_VERSION, ModelVisibleToolObservation,
        ObservationTrust, ToolObservationDetail, ToolObservationStatus, VisibleCapabilitySurface,
    },
};

use crate::{
    state::{CapabilityOutputObservation, CheckpointKind, LoopExecutionState},
    strategies::{
        BatchPolicy, CapabilityBatchTurnSummary, CapabilityErrorClass, CapabilityErrorSummary,
        GateKind, RecoveryOutcome, RetryAlteration, SanitizedStrategySummary, TurnSummary,
    },
};

use super::{
    AgentLoopExecutorError, AwaitDependentRunGateInput, AwaitDependentRunGateStage, BatchStep,
    CancelCheck, CapabilitySurfaceIndex, CheckpointStage, ExecutorStage, FailedExitDetails,
    GateInput, GateStage, MAX_CAPABILITY_RETRIES, StageContext, TurnCompletedStep,
    append_capability_error_ref, append_capability_result_ref, append_capability_safe_summary_ref,
    attach_failure_explanation, batch_policy_kind, cancelled_exit, capability_batch_counts,
    capability_call_signature, capability_error_class, capability_error_failure_category,
    capability_failure_kind, capability_host_error,
    capability_invocation_from_auth_resume_candidate, capability_invocation_from_candidate,
    capability_is_visible, capability_summary, clear_matching_pending_auth_resume,
    clear_matching_pending_external_tool_resume, failed_exit, honor_retry_alteration,
    model_visible_capability_failure_observation, push_call_signature_once, push_completed_result,
    sanitized_strategy_summary,
};

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct CapabilityStage;

const MAX_SAFE_SUMMARY_BYTES: usize = 512;
const STRATEGY_INPUT_COULD_NOT_BE_ENCODED_SUMMARY: &str = "input could not be encoded";

pub(super) struct CapabilityInput {
    pub(super) state: LoopExecutionState,
    pub(super) surface: VisibleCapabilitySurface,
    pub(super) calls: Vec<CapabilityCallCandidate>,
}

#[async_trait]
impl ExecutorStage<CapabilityInput> for CapabilityStage {
    type Output = TurnCompletedStep;

    async fn process(
        &self,
        ctx: StageContext<'_>,
        input: CapabilityInput,
    ) -> Result<TurnCompletedStep, AgentLoopExecutorError> {
        let mut state = input.state;
        let result_refs_start = state.result_refs.len();
        let mut capability_batch = CapabilityBatchTurnSummary::default();
        let surface = &input.surface;
        let surface_index = CapabilitySurfaceIndex::new(surface);
        let calls = input.calls;

        let mut visible_calls = Vec::new();
        let mut denied_calls = Vec::new();
        for call in calls {
            if capability_is_visible(&surface_index, &call) {
                visible_calls.push(call);
                continue;
            }

            denied_calls.push(call);
        }

        match CheckpointStage.cancel_if_requested(ctx, state).await? {
            CancelCheck::Continue(next) => state = *next,
            CancelCheck::Exit(exit) => return Ok(TurnCompletedStep::Exit(exit)),
        }

        state = CheckpointStage
            .write(ctx, state, CheckpointKind::BeforeSideEffect)
            .await?
            .state;
        match CheckpointStage.cancel_if_requested(ctx, state).await? {
            CancelCheck::Continue(next) => state = *next,
            CancelCheck::Exit(exit) => return Ok(TurnCompletedStep::Exit(exit)),
        }

        let mut signatures = HashSet::new();
        for call in denied_calls {
            push_call_signature_once(&mut state, &mut signatures, &call)?;
            state
                .recent_failure_kinds
                .push(LoopFailureKind::PolicyDenied);
            let summary = CapabilityErrorSummary {
                class: CapabilityErrorClass::PolicyDenied,
                safe_summary: SanitizedStrategySummary::from_trusted_static(
                    "capability is not visible in the filtered surface",
                ),
                diagnostic_ref: None,
            };
            match self
                .handle_capability_error(ctx, state, call, summary, None, &mut capability_batch)
                .await?
            {
                BatchStep::Continue(next) => state = *next,
                BatchStep::Exit(exit) => return Ok(TurnCompletedStep::Exit(exit)),
            }
        }

        if visible_calls.is_empty() {
            return self
                .completed_turn(ctx, state, result_refs_start, capability_batch)
                .await;
        }

        // A run resumed from a user-DENIED auth gate must not re-dispatch the
        // parked capability (still-missing credential -> re-block -> infinite loop).
        // Surface a model-visible gate-declined failure (retry forbidden) for
        // the denied call and let unrelated calls in the same batch proceed
        // normally.
        //
        // We call `handle_capability_error` directly so the planner-visible
        // summary can stay distinct from the stable product-facing declined
        // reason token.
        if let Some(pending) = state.pending_auth_resume.as_ref().filter(|p| {
            matches!(
                p.disposition.as_ref(),
                Some(ironclaw_turns::GateResumeDisposition::Denied)
            )
        }) {
            let denied_activity_id = pending.activity_id_for_resume();
            // Take ownership now that we've confirmed the disposition is Denied.
            // The unconditional take() below also covers the defensive case where
            // auth_denied_calls is empty — preventing a stale Denied disposition
            // from leaking into the fall-through batch.
            state.pending_auth_resume = None;
            match self
                .short_circuit_denied_resume(
                    ctx,
                    state,
                    &mut signatures,
                    &mut capability_batch,
                    denied_activity_id,
                    "auth gate denied by user",
                    visible_calls,
                )
                .await?
            {
                ControlFlow::Break(exit) => return Ok(exit),
                ControlFlow::Continue((next, remaining)) => {
                    state = next;
                    visible_calls = remaining;
                }
            }
            if visible_calls.is_empty() {
                return self
                    .completed_turn(ctx, state, result_refs_start, capability_batch)
                    .await;
            }
        }

        // A run resumed from a user-DENIED approval gate must not re-dispatch
        // the parked capability (re-dispatch -> re-block -> infinite loop).
        // Mirror the auth-gate pattern above: surface a model-visible
        // gate-declined failure for only the denied call, let other parallel
        // calls in the same batch proceed normally.
        if let Some(pending) = state.pending_approval_resume.as_ref().filter(|p| {
            matches!(
                p.disposition.as_ref(),
                Some(ironclaw_turns::GateResumeDisposition::Denied)
            )
        }) {
            let denied_activity_id = pending.activity_id_for_resume();
            // Clear the slot unconditionally — even if the partition yields no
            // matching calls, a stale Denied disposition must not bleed into the
            // fall-through batch.
            state.pending_approval_resume = None;
            match self
                .short_circuit_denied_resume(
                    ctx,
                    state,
                    &mut signatures,
                    &mut capability_batch,
                    denied_activity_id,
                    "approval gate denied by user",
                    visible_calls,
                )
                .await?
            {
                ControlFlow::Break(exit) => return Ok(exit),
                ControlFlow::Continue((next, remaining)) => {
                    state = next;
                    visible_calls = remaining;
                }
            }
            if visible_calls.is_empty() {
                return self
                    .completed_turn(ctx, state, result_refs_start, capability_batch)
                    .await;
            }
        }

        // A run resumed from a cancelled/denied external-tool gate must not
        // re-dispatch the parked client tool (no output was submitted →
        // re-park → infinite loop). Surface a model-visible failure for the
        // denied call and let other parallel calls proceed.
        if let Some(pending) = state.pending_external_tool_resume.as_ref().filter(|p| {
            matches!(
                p.disposition.as_ref(),
                Some(ironclaw_turns::GateResumeDisposition::Denied)
            )
        }) {
            let denied_activity_id = pending.activity_id_for_resume();
            state.pending_external_tool_resume = None;
            match self
                .short_circuit_denied_resume(
                    ctx,
                    state,
                    &mut signatures,
                    &mut capability_batch,
                    denied_activity_id,
                    "external tool gate cancelled by client",
                    visible_calls,
                )
                .await?
            {
                ControlFlow::Break(exit) => return Ok(exit),
                ControlFlow::Continue((next, remaining)) => {
                    state = next;
                    visible_calls = remaining;
                }
            }
            if visible_calls.is_empty() {
                return self
                    .completed_turn(ctx, state, result_refs_start, capability_batch)
                    .await;
            }
        }

        // Compute batch policy from the final set of calls that will actually
        // reach invoke_capability_batch (post auth-deny partition if applicable).
        let summaries = visible_calls
            .iter()
            .map(|call| capability_summary(&surface_index, call))
            .collect::<Vec<_>>();
        let policy = ctx.planner.batch().policy(&state, &summaries);
        let stop_on_first_suspension = matches!(policy, BatchPolicy::Sequential);

        capability_batch = CapabilityBatchTurnSummary::for_invocation_count(visible_calls.len());

        CheckpointStage
            .emit_progress(
                ctx,
                LoopProgressEvent::CapabilityBatchStarted {
                    iteration: state.iteration,
                    call_count: visible_calls.len() as u32,
                    policy: batch_policy_kind(policy),
                },
            )
            .await;

        let mut pending_approval_resume = state.pending_approval_resume.clone();
        let mut pending_auth_resume = state.pending_auth_resume.clone();
        let batch_result = ctx
            .host
            .invoke_capability_batch(CapabilityBatchInvocation {
                invocations: visible_calls
                    .iter()
                    .cloned()
                    .map(|call| {
                        // Auth-resume takes precedence: when the run is parked
                        // at a BlockedAuth checkpoint that also carried prior
                        // approval identity, re-dispatch through the auth-resume
                        // path so the original invocation_id is reused.
                        //
                        // Consume the slot on first match so that a batch with two
                        // calls to the same capability_id does not tag both as
                        // auth-resume (which would reuse one resume_token across
                        // distinct calls — a correctness and security bug).  Mirror
                        // the approval path immediately below which uses take_if.
                        if let Some(auth) = pending_auth_resume
                            .take_if(|auth| auth.capability_id == call.capability_id)
                        {
                            return capability_invocation_from_auth_resume_candidate(call, &auth);
                        }
                        let resume = pending_approval_resume
                            .take_if(|resume| resume.capability_id == call.capability_id)
                            .map(|resume| resume.to_approval_resume());
                        capability_invocation_from_candidate(call, resume)
                    })
                    .collect(),
                stop_on_first_suspension,
            })
            .await;

        let batch = match batch_result {
            Ok(batch) => batch,
            Err(ref error)
                if error.kind
                    == ironclaw_turns::run_profile::AgentLoopHostErrorKind::StaleSurface =>
            {
                let stale_summary = SanitizedStrategySummary::from_trusted_static(
                    "capability surface changed before execution; re-issue the call",
                );
                for call in visible_calls {
                    push_call_signature_once(&mut state, &mut signatures, &call)?;
                    state
                        .recent_failure_kinds
                        .push(LoopFailureKind::PolicyDenied);
                    let summary = CapabilityErrorSummary {
                        class: CapabilityErrorClass::PolicyDenied,
                        safe_summary: stale_summary.clone(),
                        diagnostic_ref: None,
                    };
                    match self
                        .handle_capability_error(
                            ctx,
                            state,
                            call,
                            summary,
                            None,
                            &mut capability_batch,
                        )
                        .await?
                    {
                        BatchStep::Continue(next) => state = *next,
                        BatchStep::Exit(exit) => return Ok(TurnCompletedStep::Exit(exit)),
                    }
                }
                return self
                    .completed_turn(ctx, state, result_refs_start, capability_batch)
                    .await;
            }
            Err(error) => return Err(capability_host_error(error)),
        };

        if batch.resolutions.is_empty()
            || batch.resolutions.len() > visible_calls.len()
            || (!batch.stopped_on_suspension && batch.resolutions.len() != visible_calls.len())
        {
            return Err(AgentLoopExecutorError::PlannerContract {
                detail: "capability batch outcome count does not match invocations",
            });
        }

        let (result_count, denied_count, gated_count, failed_count) =
            capability_batch_counts(&batch.resolutions);
        CheckpointStage
            .emit_progress(
                ctx,
                LoopProgressEvent::CapabilityBatchCompleted {
                    iteration: state.iteration,
                    result_count,
                    denied_count,
                    gated_count,
                    failed_count,
                },
            )
            .await;

        let resolutions = batch.resolutions;
        // Multiple AwaitDependentRun outcomes that share a single gate_ref
        // must coalesce into ONE gate exit: each outcome's result_ref is
        // appended as a completed result (so the parent observes every
        // child's result on resume) and a single GateStage step transitions
        // the loop to BlockedDependentRun. Firing one gate step per outcome
        // would create duplicate gate records and race the resume attempts.
        let coalesced_gate_step = if !batch.stopped_on_suspension {
            shared_await_dependent_gate(&visible_calls, &resolutions)
        } else {
            None
        };
        if !batch.stopped_on_suspension {
            // Non-suspended batches record completed (and coalesced-await)
            // outcomes before handling any remaining gates so partial parallel
            // progress is durable in any later suspension checkpoint.
            let mut pending_outcomes = Vec::new();
            for (call, resolution) in visible_calls.into_iter().zip(resolutions) {
                match resolution {
                    Resolution::Done(outcome) if outcome.verdict.is_success() => {
                        push_call_signature_once(&mut state, &mut signatures, &call)?;
                        clear_matching_pending_approval_resume(&mut state, &call);
                        clear_matching_pending_auth_resume(&mut state, &call);
                        clear_matching_pending_external_tool_resume(&mut state, &call);
                        let result = capability_result_from_outcome(&outcome)?;
                        append_completed_capability_result(
                            ctx.host,
                            &mut state,
                            &call,
                            result,
                            &mut capability_batch,
                        )
                        .await?;
                    }
                    Resolution::Done(outcome)
                        if matches!(outcome.verdict, ToolVerdict::ChildSpawned { .. }) =>
                    {
                        push_call_signature_once(&mut state, &mut signatures, &call)?;
                        clear_matching_pending_approval_resume(&mut state, &call);
                        clear_matching_pending_auth_resume(&mut state, &call);
                        clear_matching_pending_external_tool_resume(&mut state, &call);
                        let input = child_result_from_outcome(&outcome)?;
                        append_spawned_child_result(
                            ctx.host,
                            &mut state,
                            &call,
                            input,
                            &mut capability_batch,
                        )
                        .await?;
                    }
                    Resolution::Suspended(Suspension::DependentRun { waypoint, result })
                        if coalesced_gate_step.as_ref().is_some_and(|(gate, _)| {
                            waypoint.origin.as_ref().map(LoopRef::as_str) == Some(gate.as_str())
                        }) =>
                    {
                        push_call_signature_once(&mut state, &mut signatures, &call)?;
                        clear_matching_pending_approval_resume(&mut state, &call);
                        clear_matching_pending_auth_resume(&mut state, &call);
                        clear_matching_pending_external_tool_resume(&mut state, &call);
                        let result = dependent_run_result_message(&result)?;
                        append_completed_capability_result(
                            ctx.host,
                            &mut state,
                            &call,
                            result,
                            &mut capability_batch,
                        )
                        .await?;
                    }
                    other => {
                        pending_outcomes.push((call, other));
                    }
                }
            }
            // Drain non-await/non-completed outcomes (denied, failed, other
            // gates) BEFORE the coalesced gate fires. The shared-gate fast
            // path early-returns via `completed_turn` on `BatchStep::Continue`,
            // so anything left in `pending_outcomes` after the gate step would
            // be silently dropped — losing signature bookkeeping and side
            // effects for outcomes the parent must observe on resume.
            for (call, outcome) in pending_outcomes {
                push_call_signature_once(&mut state, &mut signatures, &call)?;
                match self
                    .handle_capability_outcome(ctx, state, call, outcome, &mut capability_batch)
                    .await?
                {
                    BatchStep::Continue(next) => {
                        state = *next;
                    }
                    BatchStep::Exit(exit) => return Ok(TurnCompletedStep::Exit(exit)),
                }
            }
            if let Some((shared_gate_ref, first_call)) = coalesced_gate_step {
                match GateStage
                    .process(
                        ctx,
                        GateInput {
                            state,
                            call: first_call,
                            kind: GateKind::AwaitDependentRun,
                            gate_ref: shared_gate_ref,
                            credential_requirements: Vec::new(),
                            approval_resume: None,
                            auth_resume: None,
                        },
                    )
                    .await?
                {
                    BatchStep::Continue(next) => {
                        return self
                            .completed_turn(ctx, *next, result_refs_start, capability_batch)
                            .await;
                    }
                    BatchStep::Exit(exit) => return Ok(TurnCompletedStep::Exit(exit)),
                }
            }
        } else {
            for (call, resolution) in visible_calls.into_iter().zip(resolutions) {
                push_call_signature_once(&mut state, &mut signatures, &call)?;
                match self
                    .handle_capability_outcome(ctx, state, call, resolution, &mut capability_batch)
                    .await?
                {
                    BatchStep::Continue(next) => {
                        state = *next;
                    }
                    BatchStep::Exit(exit) => return Ok(TurnCompletedStep::Exit(exit)),
                }
            }
        }

        self.completed_turn(ctx, state, result_refs_start, capability_batch)
            .await
    }
}

fn capability_failed_summary(
    error_kind: &CapabilityFailureKind,
    safe_summary: String,
) -> Result<SanitizedStrategySummary, AgentLoopExecutorError> {
    prefixed_capability_summary(
        format!("capability failed with {}: ", error_kind.as_str()),
        safe_summary,
    )
}

fn capability_denied_summary(
    reason_kind: &str,
    safe_summary: String,
) -> Result<SanitizedStrategySummary, AgentLoopExecutorError> {
    prefixed_capability_summary(
        format!("capability denied with {reason_kind}: "),
        safe_summary,
    )
}

fn prefixed_capability_summary(
    prefix: String,
    safe_summary: String,
) -> Result<SanitizedStrategySummary, AgentLoopExecutorError> {
    let safe_summary = strategy_safe_capability_summary_detail(safe_summary);
    let detail = sanitized_strategy_summary(safe_summary)?;
    let detail = truncate_summary_detail(
        detail.as_str(),
        MAX_SAFE_SUMMARY_BYTES.saturating_sub(prefix.len()),
    );
    sanitized_strategy_summary(format!("{prefix}{detail}"))
}

fn strategy_safe_capability_summary_detail(safe_summary: String) -> String {
    if safe_summary == INPUT_ENCODE_HUMAN_SUMMARY {
        STRATEGY_INPUT_COULD_NOT_BE_ENCODED_SUMMARY.to_string()
    } else {
        safe_summary
    }
}

fn truncate_summary_detail(detail: &str, max_bytes: usize) -> &str {
    if detail.len() <= max_bytes {
        return detail;
    }
    let mut end = max_bytes;
    while end > 0 && !detail.is_char_boundary(end) {
        end -= 1;
    }
    &detail[..end]
}

impl CapabilityStage {
    async fn completed_turn(
        &self,
        ctx: StageContext<'_>,
        state: LoopExecutionState,
        result_refs_start: usize,
        capability_batch: CapabilityBatchTurnSummary,
    ) -> Result<TurnCompletedStep, AgentLoopExecutorError> {
        let state = match CheckpointStage.cancel_if_requested(ctx, state).await? {
            CancelCheck::Continue(state) => *state,
            CancelCheck::Exit(exit) => return Ok(TurnCompletedStep::Exit(exit)),
        };
        let summary = TurnSummary::after_capability_batch(
            state.result_refs[result_refs_start..].to_vec(),
            capability_batch,
        );
        Ok(TurnCompletedStep::Continue {
            state: Box::new(state),
            summary,
        })
    }

    async fn handle_capability_outcome(
        &self,
        ctx: StageContext<'_>,
        mut state: LoopExecutionState,
        call: CapabilityCallCandidate,
        resolution: Resolution,
        capability_batch: &mut CapabilityBatchTurnSummary,
    ) -> Result<BatchStep, AgentLoopExecutorError> {
        // Exhaustive over `Resolution`, no wildcard (§11.9). `Done` re-splits on
        // its typed `ToolVerdict`; every gate/suspension arm reconstructs the loop
        // ref from the channel's preserved `origin`. Model-visible content comes
        // from PR-B (`ToolVerdict::RecoverableFailure.diagnostic`, `Denial`); the
        // dependent-run staged result comes from `Suspension::dependent_result()`.
        match resolution {
            Resolution::Done(outcome) => match outcome.verdict {
                ToolVerdict::Success => {
                    clear_matching_pending_approval_resume(&mut state, &call);
                    clear_matching_pending_auth_resume(&mut state, &call);
                    clear_matching_pending_external_tool_resume(&mut state, &call);
                    let result = capability_result_from_outcome(&outcome)?;
                    append_completed_capability_result(
                        ctx.host,
                        &mut state,
                        &call,
                        result,
                        capability_batch,
                    )
                    .await?;
                    Ok(BatchStep::Continue(Box::new(state)))
                }
                ToolVerdict::ChildSpawned { .. } => {
                    clear_matching_pending_approval_resume(&mut state, &call);
                    clear_matching_pending_auth_resume(&mut state, &call);
                    clear_matching_pending_external_tool_resume(&mut state, &call);
                    let input = child_result_from_outcome(&outcome)?;
                    append_spawned_child_result(
                        ctx.host,
                        &mut state,
                        &call,
                        input,
                        capability_batch,
                    )
                    .await?;
                    Ok(BatchStep::Continue(Box::new(state)))
                }
                ToolVerdict::RecoverableFailure {
                    ref error_kind,
                    ref diagnostic,
                } => {
                    let failure = capability_failure_from_recoverable(
                        error_kind,
                        diagnostic.as_ref(),
                        &outcome,
                    );
                    if failure.error_kind == CapabilityFailureKind::Cancelled {
                        return self.cancelled_after_checkpoint(ctx, state).await;
                    }
                    state
                        .recent_failure_kinds
                        .push(capability_failure_kind(&failure.error_kind));
                    let model_observation =
                        Some(model_visible_capability_failure_observation(&failure));
                    let summary = CapabilityErrorSummary {
                        class: capability_error_class(&failure.error_kind),
                        safe_summary: capability_failed_summary(
                            &failure.error_kind,
                            failure.safe_summary,
                        )?,
                        diagnostic_ref: None,
                    };
                    self.handle_capability_error(
                        ctx,
                        state,
                        call,
                        summary,
                        model_observation,
                        capability_batch,
                    )
                    .await
                }
            },
            Resolution::Denied(denial) => {
                state
                    .recent_failure_kinds
                    .push(LoopFailureKind::PolicyDenied);
                let reason = denial
                    .reason_kind
                    .map(deny_reason_tag)
                    .unwrap_or("policy_denied");
                let safe_summary = denial
                    .summary
                    .map(|summary| summary.as_str().to_string())
                    .unwrap_or_default();
                let summary = CapabilityErrorSummary {
                    class: CapabilityErrorClass::PolicyDenied,
                    safe_summary: capability_denied_summary(reason, safe_summary)?,
                    diagnostic_ref: None,
                };
                self.handle_capability_error(ctx, state, call, summary, None, capability_batch)
                    .await
            }
            Resolution::Blocked(Blocked::Approval(waypoint)) => {
                let gate_ref = loop_gate_ref_from_origin(waypoint.origin.as_ref())?;
                let approval_resume =
                    approval_resume_from_gate(&gate_ref, waypoint.resume.as_ref(), &call);
                GateStage
                    .process(
                        ctx,
                        GateInput {
                            state,
                            call,
                            kind: GateKind::Approval,
                            gate_ref,
                            credential_requirements: Vec::new(),
                            approval_resume,
                            auth_resume: None,
                        },
                    )
                    .await
            }
            Resolution::Blocked(Blocked::Auth(waypoint)) => {
                let gate_ref = loop_gate_ref_from_origin(waypoint.origin.as_ref())?;
                // When the invocation already passed an approval gate, carry that
                // identity into the auth resume contract before handing off to the
                // generic gate persistence stage. Extract BEFORE clearing.
                let prior_approval = state
                    .pending_approval_resume
                    .as_ref()
                    .filter(|r| r.capability_id == call.capability_id)
                    .map(|r| r.to_approval_resume());
                clear_matching_pending_approval_resume(&mut state, &call);
                clear_matching_pending_auth_resume(&mut state, &call);
                clear_matching_pending_external_tool_resume(&mut state, &call);
                let auth_resume =
                    auth_resume_from_gate(waypoint.resume.as_ref(), prior_approval.as_ref());
                // `credential_requirements` now ride the host `GateRecord::Auth`
                // (§5.2.9), not this model-visible channel; the runner re-reads them
                // from the record at the blocked exit to rebuild
                // `TurnRunRecord.credential_requirements`.
                GateStage
                    .process(
                        ctx,
                        GateInput {
                            state,
                            call,
                            kind: GateKind::Auth,
                            gate_ref,
                            credential_requirements: Vec::new(),
                            approval_resume: prior_approval,
                            auth_resume,
                        },
                    )
                    .await
            }
            Resolution::Blocked(Blocked::Resource(waypoint)) => {
                let gate_ref = loop_gate_ref_from_origin(waypoint.origin.as_ref())?;
                GateStage
                    .process(
                        ctx,
                        GateInput {
                            state,
                            call,
                            kind: GateKind::Resource,
                            gate_ref,
                            credential_requirements: Vec::new(),
                            approval_resume: None,
                            auth_resume: None,
                        },
                    )
                    .await
            }
            Resolution::Suspended(Suspension::ExternalTool(waypoint)) => {
                let gate_ref = loop_gate_ref_from_origin(waypoint.origin.as_ref())?;
                // The model called a client-supplied tool: park the run and return
                // control to the API client. No resume payload — the client submits
                // the tool output on resume.
                GateStage
                    .process(
                        ctx,
                        GateInput {
                            state,
                            call,
                            kind: GateKind::ExternalTool,
                            gate_ref,
                            credential_requirements: Vec::new(),
                            approval_resume: None,
                            auth_resume: None,
                        },
                    )
                    .await
            }
            Resolution::Suspended(Suspension::DependentRun { waypoint, result }) => {
                let gate_ref = loop_gate_ref_from_origin(waypoint.origin.as_ref())?;
                let resolved_result = dependent_run_result_message(&result)?;
                AwaitDependentRunGateStage
                    .process(
                        ctx,
                        AwaitDependentRunGateInput {
                            state,
                            call,
                            gate_ref,
                            resolved_result,
                        },
                    )
                    .await
            }
            Resolution::Suspended(Suspension::Process(waypoint)) => {
                let process_ref = loop_process_ref_from_origin(waypoint.origin.as_ref())?;
                self.fail_unsupported_process_wait(ctx, state, &call, &process_ref)
                    .await
            }
        }
    }

    async fn handle_capability_error(
        &self,
        ctx: StageContext<'_>,
        mut state: LoopExecutionState,
        call: CapabilityCallCandidate,
        mut summary: CapabilityErrorSummary,
        mut model_observation: Option<ironclaw_turns::run_profile::ModelVisibleToolObservation>,
        capability_batch: &mut CapabilityBatchTurnSummary,
    ) -> Result<BatchStep, AgentLoopExecutorError> {
        // Snapshot resume-origin flags for this call BEFORE clearing the pending
        // slots.
        //
        // Safety invariants:
        //   S1: A resume-origin failure must never surface as scope_mismatch /
        //       terminal "Capability: unavailable".
        //   S2: A side-effecting capability must never be silently re-executed by
        //       a retry — the first resume dispatch already hit the backend.
        //
        // Part C-sub-A (primary guard): when this failure originated from an
        // approval-resume OR auth-resume dispatch (`is_resume_origin == true`), we
        // intercept any `RecoveryOutcome::Retry` outcome below and redirect it to
        // `ToolErrorResult` instead.  This:
        //   - Kills scope_mismatch (S1): no retry ever reaches the cross-run
        //     input_ref without the resume context.
        //   - Prevents double-exec (S2): the backend is not invoked a second time.
        //   - Surfaces the real error to the model so the user can re-approve /
        //     re-authenticate.
        //
        // Auth-resume note: `PendingAuthResume` carries `input_ref` only (no
        // inline `input` value); a non-resume retry dispatched through
        // `capability_invocation_from_candidate(call.clone(), None)` would reach
        // the product adapter's `ensure_ref_scoped_to_run` check without the auth
        // context and fail with `ScopeMismatch`.  The same surface-and-continue
        // redirect is therefore the correct fix for both resume origins.
        //
        // Part A (belt-and-suspenders): if a retry IS dispatched (only possible
        // when `is_resume_origin == false`, i.e. non-resume path), we always pass
        // `None` as before.  If this logic ever changes to allow a resume-origin
        // retry, the approval/auth context must be threaded into
        // `capability_invocation_from_candidate` so the retry cannot reach the host
        // without its resume context.
        let captured_approval_resume: Option<CapabilityApprovalResume> = state
            .pending_approval_resume
            .as_ref()
            .filter(|r| r.capability_id == call.capability_id)
            .map(|r| r.to_approval_resume());
        let captured_auth_resume_origin: bool = state
            .pending_auth_resume
            .as_ref()
            .is_some_and(|r| r.capability_id == call.capability_id);
        let is_resume_origin = captured_approval_resume.is_some() || captured_auth_resume_origin;

        clear_matching_pending_approval_resume(&mut state, &call);
        clear_matching_pending_auth_resume(&mut state, &call);
        clear_matching_pending_external_tool_resume(&mut state, &call);
        for _ in 0..MAX_CAPABILITY_RETRIES {
            match ctx
                .planner
                .recovery()
                .on_capability_error(&state, &summary)
                .await
            {
                RecoveryOutcome::ToolErrorResult { recovery } => {
                    state.recovery_state = recovery;
                    append_blocked_capability_error_result(
                        ctx.host,
                        &mut state,
                        &call,
                        &summary,
                        model_observation.clone(),
                        capability_batch,
                    )
                    .await?;
                    match CheckpointStage.cancel_if_requested(ctx, state).await? {
                        CancelCheck::Continue(next) => state = *next,
                        CancelCheck::Exit(exit) => return Ok(BatchStep::Exit(exit)),
                    }
                    return Ok(BatchStep::Continue(Box::new(state)));
                }
                RecoveryOutcome::Abort {
                    recovery,
                    failure_kind,
                } => {
                    state.recovery_state = recovery;
                    append_blocked_capability_error_result(
                        ctx.host,
                        &mut state,
                        &call,
                        &summary,
                        model_observation.clone(),
                        capability_batch,
                    )
                    .await?;
                    match CheckpointStage.cancel_if_requested(ctx, state).await? {
                        CancelCheck::Continue(next) => state = *next,
                        CancelCheck::Exit(exit) => return Ok(BatchStep::Exit(exit)),
                    }
                    let explanation_message_ref =
                        attach_failure_explanation(ctx, &mut state, failure_kind).await?;
                    let checked = CheckpointStage
                        .write(ctx, state, CheckpointKind::Final)
                        .await?;
                    return Ok(BatchStep::Exit(failed_exit(
                        ctx.host,
                        checked.state,
                        failure_kind,
                        Some(checked.checkpoint_id),
                        FailedExitDetails {
                            diagnostic_ref: summary.diagnostic_ref.clone(),
                            safe_summary: Some(capability_error_failure_category(summary.class)?),
                            explanation_message_ref,
                        },
                    )?));
                }
                RecoveryOutcome::Retry {
                    recovery, alter, ..
                } => {
                    state.recovery_state = recovery;

                    // Part C-sub-A: a resume-origin retryable failure must not be
                    // silently re-dispatched.  The first dispatch already contacted
                    // the backend (side-effect risk) and a retry without the
                    // approval/auth context would cause scope_mismatch.  Surface
                    // the real error to the model as a clean tool error and
                    // continue the loop so the user can re-approve / re-auth.
                    if is_resume_origin {
                        append_blocked_capability_error_result(
                            ctx.host,
                            &mut state,
                            &call,
                            &summary,
                            model_observation,
                            capability_batch,
                        )
                        .await?;
                        match CheckpointStage.cancel_if_requested(ctx, state).await? {
                            CancelCheck::Continue(next) => state = *next,
                            CancelCheck::Exit(exit) => return Ok(BatchStep::Exit(exit)),
                        }
                        return Ok(BatchStep::Continue(Box::new(state)));
                    }

                    match CheckpointStage.cancel_if_requested(ctx, state).await? {
                        CancelCheck::Continue(next) => state = *next,
                        CancelCheck::Exit(exit) => return Ok(BatchStep::Exit(exit)),
                    }
                    if matches!(alter, Some(RetryAlteration::RepairInvalidModelOutput)) {
                        return Err(AgentLoopExecutorError::PlannerContract {
                            detail: "invalid model output repair retry is model-only",
                        });
                    }
                    honor_retry_alteration(alter.as_ref())?;
                    CheckpointStage
                        .emit_progress(
                            ctx,
                            LoopProgressEvent::driver_note(
                                LoopDriverNoteKind::Retrying,
                                "retrying capability invocation",
                            )
                            .map_err(|_| {
                                AgentLoopExecutorError::PlannerContract {
                                    detail: "retry progress summary was invalid",
                                }
                            })?,
                        )
                        .await;
                    // Part A: Non-resume-origin retry.  `is_resume_origin` is
                    // `false` here (the Part C-sub-A guard above short-circuited
                    // for both approval-resume and auth-resume cases), so passing
                    // `None` is correct and safe — there is no cross-run input_ref
                    // to protect.
                    let retry_result = ctx
                        .host
                        .invoke_capability(capability_invocation_from_candidate(call.clone(), None))
                        .await;
                    let retry = match retry_result {
                        Ok(outcome) => outcome,
                        Err(ref error)
                            if error.kind
                                == ironclaw_turns::run_profile::AgentLoopHostErrorKind::StaleSurface =>
                        {
                            summary = CapabilityErrorSummary {
                                class: CapabilityErrorClass::PolicyDenied,
                                safe_summary: SanitizedStrategySummary::from_trusted_static(
                                    "capability surface changed before execution; re-issue the call",
                                ),
                                diagnostic_ref: None,
                            };
                            model_observation = None;
                            continue;
                        }
                        Err(error) => return Err(capability_host_error(error)),
                    };
                    match retry {
                        Resolution::Done(outcome)
                            if matches!(
                                outcome.verdict,
                                ToolVerdict::RecoverableFailure { .. }
                            ) =>
                        {
                            let failure = match &outcome.verdict {
                                ToolVerdict::RecoverableFailure {
                                    error_kind,
                                    diagnostic,
                                } => capability_failure_from_recoverable(
                                    error_kind,
                                    diagnostic.as_ref(),
                                    &outcome,
                                ),
                                _ => unreachable!("guarded to RecoverableFailure"),
                            };
                            if failure.error_kind == CapabilityFailureKind::Cancelled {
                                return self.cancelled_after_checkpoint(ctx, state).await;
                            }
                            model_observation =
                                Some(model_visible_capability_failure_observation(&failure));
                            summary = CapabilityErrorSummary {
                                class: capability_error_class(&failure.error_kind),
                                safe_summary: capability_failed_summary(
                                    &failure.error_kind,
                                    failure.safe_summary,
                                )?,
                                diagnostic_ref: None,
                            };
                        }
                        promoted => {
                            return Box::pin(self.handle_capability_outcome(
                                ctx,
                                state,
                                call,
                                promoted,
                                capability_batch,
                            ))
                            .await;
                        }
                    }
                }
            }
        }

        append_blocked_capability_error_result(
            ctx.host,
            &mut state,
            &call,
            &summary,
            model_observation,
            capability_batch,
        )
        .await?;
        // Route through the single failure-explanation chokepoint so the
        // recent-failure-kind record and (when the kind is explainable) the
        // explanation message ref are produced consistently with the other
        // failed-exit sites instead of being pushed inline here.
        let failure_kind = exhausted_capability_failure_kind(summary.class);
        let explanation_message_ref =
            attach_failure_explanation(ctx, &mut state, failure_kind).await?;
        let checked = CheckpointStage
            .write(ctx, state, CheckpointKind::Final)
            .await?;
        Ok(BatchStep::Exit(failed_exit(
            ctx.host,
            checked.state,
            failure_kind,
            Some(checked.checkpoint_id),
            FailedExitDetails {
                diagnostic_ref: summary.diagnostic_ref.clone(),
                safe_summary: Some(capability_error_failure_category(summary.class)?),
                explanation_message_ref,
            },
        )?))
    }

    async fn fail_unsupported_process_wait(
        &self,
        ctx: StageContext<'_>,
        mut state: LoopExecutionState,
        call: &CapabilityCallCandidate,
        _process_ref: &ironclaw_turns::run_profile::LoopProcessRef,
    ) -> Result<BatchStep, AgentLoopExecutorError> {
        append_capability_safe_summary_ref(
            ctx.host,
            &mut state,
            call,
            "capability process wait is not supported".to_string(),
        )
        .await?;
        let explanation_message_ref =
            attach_failure_explanation(ctx, &mut state, LoopFailureKind::CapabilityProtocolError)
                .await?;
        let checked = CheckpointStage
            .write(ctx, state, CheckpointKind::Final)
            .await?;
        Ok(BatchStep::Exit(failed_exit(
            ctx.host,
            checked.state,
            LoopFailureKind::CapabilityProtocolError,
            Some(checked.checkpoint_id),
            FailedExitDetails {
                diagnostic_ref: None,
                safe_summary: None,
                explanation_message_ref,
            },
        )?))
    }

    async fn cancelled_after_checkpoint(
        &self,
        ctx: StageContext<'_>,
        state: LoopExecutionState,
    ) -> Result<BatchStep, AgentLoopExecutorError> {
        // Called when a capability invocation surfaced `CapabilityFailureKind::Cancelled`
        // and no `LoopCancellationSignal` is in scope, so the cooperative-boundary
        // reason cannot be derived from a signal. `cancelled_exit` hardcodes
        // `LoopCancelledReasonKind::HostCancellation` which currently coarsens
        // every reason variant; if `LoopCancelledReasonKind` gains finer-grained
        // variants this site must switch to `cancelled_exit_with_reason` with the
        // capability-specific reason.
        let checked = CheckpointStage
            .write(ctx, state, CheckpointKind::Final)
            .await?;
        Ok(BatchStep::Exit(cancelled_exit(
            ctx.host,
            checked.state,
            Some(checked.checkpoint_id),
        )?))
    }

    /// Shared denied-resume short-circuit for both auth and approval gates.
    ///
    /// Partitions `visible_calls` by the parked call's `activity_id`. For the
    /// matching call, synthesises a model-visible `GateDeclined` failure (retry
    /// `Forbidden`) via `handle_capability_error` and uses `planner_summary` as
    /// the planner-visible strategy summary (must pass
    /// `validate_loop_safe_summary`).
    ///
    /// Returns `ControlFlow::Break(step)` if `handle_capability_error` produced
    /// an `Exit` (caller should propagate it immediately), or
    /// `ControlFlow::Continue((state, remaining_calls))` with the surviving
    /// state and the calls that did *not* match the parked activity.  The
    /// caller is responsible for checking whether `remaining_calls` is empty
    /// and calling `completed_turn` when it is.
    ///
    /// # Callers
    ///
    /// - Auth-gate denial: `state.pending_auth_resume = None` before calling;
    ///   `planner_summary = "auth gate denied by user"`.
    /// - Approval-gate denial: `state.pending_approval_resume = None` before
    ///   calling; `planner_summary = "approval gate denied by user"`.
    ///
    /// Both summaries are compile-time `&'static str` and are validated by
    /// `SanitizedStrategySummary::from_trusted_static` at the call site.
    // arch-exempt: too_many_args, denied-resume short-circuit threads the capability-batch dispatch context (ctx/state/signatures/batch); needs a dispatch-context bundle, plan #4954
    #[allow(clippy::too_many_arguments)]
    async fn short_circuit_denied_resume(
        &self,
        ctx: StageContext<'_>,
        mut state: LoopExecutionState,
        signatures: &mut HashSet<crate::state::CapabilityCallSignature>,
        capability_batch: &mut CapabilityBatchTurnSummary,
        denied_activity_id: CapabilityActivityId,
        planner_summary: &'static str,
        visible_calls: Vec<CapabilityCallCandidate>,
    ) -> Result<
        ControlFlow<TurnCompletedStep, (LoopExecutionState, Vec<CapabilityCallCandidate>)>,
        AgentLoopExecutorError,
    > {
        let (denied_calls, remaining_calls): (Vec<_>, Vec<_>) = visible_calls
            .into_iter()
            .partition(|call| call.activity_id == denied_activity_id);

        for call in denied_calls {
            push_call_signature_once(&mut state, signatures, &call)?;
            CheckpointStage
                .emit_progress(
                    ctx,
                    LoopProgressEvent::CapabilityActivityFailed {
                        activity_id: denied_activity_id,
                        capability_id: call.capability_id.clone(),
                        reason_kind: CapabilityFailureKind::GateDeclined,
                        // Gate denial carries no host-authored message; the
                        // model-visible text is produced separately below.
                        safe_summary: None,
                    },
                )
                .await;
            let failure = ironclaw_turns::run_profile::CapabilityFailure {
                error_kind: CapabilityFailureKind::GateDeclined,
                // Intentionally empty: model-visible text comes from
                // `model_visible_capability_failure_observation` and the
                // planner summary from `from_trusted_static` below.
                safe_summary: String::new(),
                detail: None,
            };
            state
                .recent_failure_kinds
                .push(capability_failure_kind(&failure.error_kind));
            let model_observation = Some(model_visible_capability_failure_observation(&failure));
            let summary = CapabilityErrorSummary {
                class: capability_error_class(&failure.error_kind),
                safe_summary: SanitizedStrategySummary::from_trusted_static(planner_summary),
                diagnostic_ref: None,
            };
            match self
                .handle_capability_error(
                    ctx,
                    state,
                    call,
                    summary,
                    model_observation,
                    capability_batch,
                )
                .await?
            {
                BatchStep::Continue(next) => state = *next,
                BatchStep::Exit(exit) => {
                    return Ok(ControlFlow::Break(TurnCompletedStep::Exit(exit)));
                }
            }
        }

        // Return surviving state + remaining calls to the caller.
        // The caller checks remaining_calls.is_empty() and calls completed_turn
        // when there is nothing left to dispatch.
        Ok(ControlFlow::Continue((state, remaining_calls)))
    }
}

fn clear_matching_pending_approval_resume(
    state: &mut LoopExecutionState,
    call: &CapabilityCallCandidate,
) {
    if state
        .pending_approval_resume
        .as_ref()
        .is_some_and(|resume| resume.capability_id == call.capability_id)
    {
        state.pending_approval_resume = None;
    }
}

fn exhausted_capability_failure_kind(class: CapabilityErrorClass) -> LoopFailureKind {
    match class {
        CapabilityErrorClass::PolicyDenied => LoopFailureKind::PolicyDenied,
        CapabilityErrorClass::InputInvalid => LoopFailureKind::ModelError,
        CapabilityErrorClass::Transient
        | CapabilityErrorClass::Permanent
        | CapabilityErrorClass::OperationFailed
        | CapabilityErrorClass::Unavailable
        | CapabilityErrorClass::Internal => LoopFailureKind::CapabilityProtocolError,
    }
}

fn auth_resume_for_gate(
    mut auth_resume: Option<CapabilityAuthResume>,
    prior_approval: Option<&CapabilityApprovalResume>,
) -> Option<CapabilityAuthResume> {
    let Some(prior_approval) = prior_approval else {
        return auth_resume;
    };

    let prior_identity = || AuthResumeApprovalIdentity {
        approval_request_id: prior_approval.approval_request_id,
        correlation_id: prior_approval.correlation_id,
    };

    match auth_resume.as_mut() {
        Some(resume) => {
            resume.resume_token = prior_approval.resume_token.clone();
            resume.prior_approval.get_or_insert_with(prior_identity);
            auth_resume
        }
        None => Some(CapabilityAuthResume {
            resume_token: prior_approval.resume_token.clone(),
            prior_approval: Some(prior_identity()),
        }),
    }
}

// ---------------------------------------------------------------------------
// host_api::Resolution -> loop vocabulary reconstruction (§5.3 Stage 2 flip).
//
// The loop-facing result IS `Resolution` now; these total helpers reconstruct
// the loop-side values the existing downstream stages consume, from the
// channel's preserved `origin` refs and PR-B model-visible content. The
// producer always populates `origin` (the mapping preserves it), so a missing
// one is an internal contract violation, not a recoverable model error.
// ---------------------------------------------------------------------------

fn loop_result_ref_from_origin(
    origin: Option<&LoopRef>,
) -> Result<LoopResultRef, AgentLoopExecutorError> {
    origin
        .and_then(|loop_ref| LoopResultRef::new(loop_ref.as_str()).ok())
        .ok_or(AgentLoopExecutorError::PlannerContract {
            detail: "capability resolution is missing its loop result origin",
        })
}

fn loop_gate_ref_from_origin(
    origin: Option<&LoopRef>,
) -> Result<LoopGateRef, AgentLoopExecutorError> {
    origin
        .and_then(|loop_ref| LoopGateRef::new(loop_ref.as_str()).ok())
        .ok_or(AgentLoopExecutorError::PlannerContract {
            detail: "capability resolution is missing its loop gate origin",
        })
}

fn loop_process_ref_from_origin(
    origin: Option<&LoopRef>,
) -> Result<LoopProcessRef, AgentLoopExecutorError> {
    origin
        .and_then(|loop_ref| LoopProcessRef::new(loop_ref.as_str()).ok())
        .ok_or(AgentLoopExecutorError::PlannerContract {
            detail: "capability resolution is missing its loop process origin",
        })
}

/// Reconstruct the byte-stable approval identity from the deterministic
/// `gate:approval-{id}` routing ref, so the fingerprinted approval lease claimed
/// on resume is identical to the pre-flip one.
fn approval_request_id_from_loop_gate_ref(gate_ref: &LoopGateRef) -> Option<ApprovalRequestId> {
    gate_ref
        .as_str()
        .strip_prefix("gate:approval-")
        .and_then(|id| ApprovalRequestId::parse(id).ok())
}

fn capability_progress_from(progress: ResultProgress) -> CapabilityProgress {
    match progress {
        ResultProgress::Unknown => CapabilityProgress::Unknown,
        ResultProgress::MadeProgress => CapabilityProgress::MadeProgress,
        ResultProgress::NoChange => CapabilityProgress::NoChange,
        ResultProgress::Blocked => CapabilityProgress::Blocked,
    }
}

fn capability_result_from_outcome(
    outcome: &Outcome,
) -> Result<CapabilityResultMessage, AgentLoopExecutorError> {
    Ok(CapabilityResultMessage {
        result_ref: loop_result_ref_from_origin(outcome.refs.origin.as_ref())?,
        safe_summary: outcome.summary.as_str().to_string(),
        progress: capability_progress_from(outcome.progress),
        terminate_hint: outcome.terminate_hint.should_terminate(),
        byte_len: outcome.refs.byte_len,
        model_observation: result_reference_observation_from_outcome(outcome),
        output_digest: outcome
            .refs
            .output_digest
            .map(|digest| ContentDigest(digest.value())),
    })
}

fn child_result_from_outcome(
    outcome: &Outcome,
) -> Result<ChildResultAppendInput, AgentLoopExecutorError> {
    Ok(ChildResultAppendInput {
        result_ref: loop_result_ref_from_origin(outcome.refs.origin.as_ref())?,
        safe_summary: outcome.summary.as_str().to_string(),
        byte_len: outcome.refs.byte_len,
        model_observation: result_reference_observation_from_outcome(outcome),
    })
}

/// Rebuild the `ResultReference` model observation from a completed [`Outcome`],
/// carrying the #5838 first-look inline preview content the model reads without a
/// follow-up `result_read`. Reconstructed from the channel's real
/// [`ModelResultPreview`] (`refs.preview`) — the delimiter/JSON-bearing content
/// that the collapse now preserves (it rode the caption `SafeSummary` before,
/// which dropped it). `None` when no preview is staged, in which case
/// `append_capability_result_ref` synthesizes a bare success observation.
fn result_reference_observation_from_outcome(
    outcome: &Outcome,
) -> Option<ModelVisibleToolObservation> {
    let preview = outcome.refs.preview.as_ref()?;
    let meta = &outcome.refs.preview_meta;
    // The observation references the preview's OWN result: `preview_meta`'s
    // referenced ref when it differs (a `result_read` presenting another result),
    // else the outcome's own preserved origin.
    let result_ref = meta
        .referenced_result_ref
        .as_ref()
        .or(outcome.refs.origin.as_ref())?
        .as_str()
        .to_string();
    Some(ModelVisibleToolObservation {
        schema_version: MODEL_VISIBLE_TOOL_OBSERVATION_SCHEMA_VERSION,
        status: ToolObservationStatus::Success,
        // The observation's OWN producer-authored summary (carried through the
        // collapse in `preview_meta`), NOT the generic outcome caption: it holds
        // the truncation/continuation hint ("preview truncated, use result_read …")
        // that a completed result message's `safe_summary` ("capability completed")
        // does not. Falls back to the outcome caption when the producer authored no
        // observation summary (or it failed the caption contract).
        summary: meta
            .summary
            .as_ref()
            .map(|summary| summary.as_str().to_string())
            .unwrap_or_else(|| outcome.summary.as_str().to_string()),
        detail: ToolObservationDetail::ResultReference {
            result_ref,
            byte_len: outcome.refs.byte_len,
            preview: Some(preview.as_str().to_string()),
            // Continuation metadata for a truncated first-look preview; falls back
            // to the full inline size for a complete preview.
            total_bytes: meta.total_bytes.or(Some(outcome.refs.byte_len)),
            next_offset: meta.next_offset,
            item_count: meta.item_count,
        },
        artifacts: Vec::new(),
        recovery: None,
        trust: ObservationTrust::UntrustedToolOutput,
    })
}

/// Rebuild the staged dependent-child result the parent observes on resume from
/// the inline [`DependentRunResult`] (Stage 1b) — no host-storage read.
fn dependent_run_result_message(
    result: &DependentRunResult,
) -> Result<CapabilityResultMessage, AgentLoopExecutorError> {
    let result_ref = loop_result_ref_from_origin(result.origin.as_ref())?;
    // Forward the child's staged observation caption (#6287 IronLoop). The
    // mapping preserves a bounded `SafeSummary` caption on
    // `DependentRunResult.observation` — "model_observation now rides the inline
    // observation preview (was dropped entirely)". Hardcoding `None` here re-drops
    // it, so `append_capability_result_ref` falls back to a bare synthesized
    // success observation and the resumed parent loses both the caption and the
    // staged result reference. Surface it as a `ResultReference` observation
    // pointing at the staged child result. The full inline first-look preview
    // content stays host-owned and is the completed-`Outcome` path, not this
    // suspension channel.
    let model_observation =
        result
            .observation
            .as_ref()
            .map(|caption| ModelVisibleToolObservation {
                schema_version: MODEL_VISIBLE_TOOL_OBSERVATION_SCHEMA_VERSION,
                status: ToolObservationStatus::Success,
                summary: caption.as_str().to_string(),
                detail: ToolObservationDetail::ResultReference {
                    result_ref: result_ref.as_str().to_string(),
                    byte_len: result.byte_len,
                    preview: None,
                    total_bytes: None,
                    next_offset: None,
                    item_count: None,
                },
                artifacts: Vec::new(),
                recovery: None,
                trust: ObservationTrust::UntrustedToolOutput,
            });
    Ok(CapabilityResultMessage {
        result_ref,
        safe_summary: result.summary.as_str().to_string(),
        progress: CapabilityProgress::MadeProgress,
        terminate_hint: false,
        byte_len: result.byte_len,
        output_digest: None,
        model_observation,
    })
}

fn capability_failure_from_recoverable(
    error_kind: &FailureKind,
    diagnostic: Option<&ModelFailureDiagnostic>,
    outcome: &Outcome,
) -> CapabilityFailure {
    CapabilityFailure {
        error_kind: capability_failure_kind_from(error_kind),
        safe_summary: outcome.summary.as_str().to_string(),
        detail: diagnostic.map(capability_failure_detail_from),
    }
}

fn capability_failure_kind_from(kind: &FailureKind) -> CapabilityFailureKind {
    use serde::Deserialize;
    use serde::de::{IntoDeserializer, value::StrDeserializer};
    // `FailureKind` and `CapabilityFailureKind` share the same stable tag set
    // plus an open `Unknown`; deserialize routes any tag (incl. unknowns) losslessly.
    let deserializer: StrDeserializer<'_, serde::de::value::Error> =
        kind.as_str().into_deserializer();
    CapabilityFailureKind::deserialize(deserializer).unwrap_or(CapabilityFailureKind::Internal)
}

fn capability_failure_detail_from(diagnostic: &ModelFailureDiagnostic) -> CapabilityFailureDetail {
    match diagnostic {
        ModelFailureDiagnostic::InvalidInput { issues } => CapabilityFailureDetail::InvalidInput {
            issues: issues
                .as_slice()
                .iter()
                .map(capability_input_issue_from)
                .collect(),
        },
        ModelFailureDiagnostic::Diagnostic { text } => CapabilityFailureDetail::Diagnostic {
            text: text.as_str().to_string(),
        },
        ModelFailureDiagnostic::HostRemediation { text } => {
            CapabilityFailureDetail::HostRemediation { text: text.clone() }
        }
    }
}

fn capability_input_issue_from(issue: &ModelInputIssue) -> CapabilityInputIssue {
    CapabilityInputIssue {
        path: issue.path.as_str().to_string(),
        code: issue.code,
        expected: issue
            .expected
            .as_ref()
            .map(|value| value.as_str().to_string()),
        received: issue
            .received
            .as_ref()
            .map(|value| value.as_str().to_string()),
        schema_path: issue
            .schema_path
            .as_ref()
            .map(|value| value.as_str().to_string()),
    }
}

fn deny_reason_tag(reason: DenyReason) -> &'static str {
    match reason {
        DenyReason::MissingGrant => "missing_grant",
        DenyReason::InvalidPath => "invalid_path",
        DenyReason::PathOutsideMount => "path_outside_mount",
        DenyReason::UnknownCapability => "unknown_capability",
        DenyReason::UnknownSecret => "unknown_secret",
        DenyReason::NetworkDenied => "network_denied",
        DenyReason::BudgetDenied => "budget_denied",
        DenyReason::ApprovalDenied => "approval_denied",
        DenyReason::PolicyDenied => "policy_denied",
        DenyReason::ResourceLimitExceeded => "resource_limit_exceeded",
        DenyReason::InternalInvariantViolation => "internal_invariant_violation",
    }
}

/// Reconstruct the loop-facing approval resume from the gate waypoint: the resume
/// token echoed back, the byte-stable approval id from the routing ref, the
/// call's own input ref (advisory — the host reconstitutes the authoritative one
/// from its replay store on resume), and a fresh correlation id (observability
/// only; not in the idempotency key or lease).
fn approval_resume_from_gate(
    gate_ref: &LoopGateRef,
    resume_token: Option<&ResumeToken>,
    call: &CapabilityCallCandidate,
) -> Option<CapabilityApprovalResume> {
    let resume_token = CapabilityResumeToken::new(resume_token?.as_str()).ok()?;
    let approval_request_id = approval_request_id_from_loop_gate_ref(gate_ref)?;
    Some(CapabilityApprovalResume {
        approval_request_id,
        resume_token,
        correlation_id: CorrelationId::new(),
        input_ref: call.input_ref.clone(),
    })
}

/// Reconstruct the loop-facing auth resume from the gate waypoint's token, then
/// fold in any prior-approval identity (kept on the wire this slice; its host-side
/// move is deferred to §5.3 Stage 2a-ii).
fn auth_resume_from_gate(
    resume_token: Option<&ResumeToken>,
    prior_approval: Option<&CapabilityApprovalResume>,
) -> Option<CapabilityAuthResume> {
    let base = resume_token
        .and_then(|token| CapabilityResumeToken::new(token.as_str()).ok())
        .map(|resume_token| CapabilityAuthResume {
            resume_token,
            prior_approval: None,
        });
    auth_resume_for_gate(base, prior_approval)
}

struct ChildResultAppendInput {
    result_ref: LoopResultRef,
    safe_summary: String,
    byte_len: u64,
    model_observation: Option<ModelVisibleToolObservation>,
}

async fn append_spawned_child_result(
    host: &(dyn ironclaw_turns::run_profile::AgentLoopDriverHost + Send + Sync),
    state: &mut LoopExecutionState,
    call: &CapabilityCallCandidate,
    input: ChildResultAppendInput,
    capability_batch: &mut CapabilityBatchTurnSummary,
) -> Result<(), AgentLoopExecutorError> {
    let safe_summary = sanitized_strategy_summary(input.safe_summary)?.into_inner();
    let result = CapabilityResultMessage {
        result_ref: input.result_ref,
        safe_summary,
        progress: CapabilityProgress::MadeProgress,
        terminate_hint: false,
        byte_len: input.byte_len,
        output_digest: None,
        model_observation: input.model_observation,
    };
    append_completed_capability_result(host, state, call, result, capability_batch).await
}

async fn append_blocked_capability_error_result(
    host: &(dyn ironclaw_turns::run_profile::AgentLoopDriverHost + Send + Sync),
    state: &mut LoopExecutionState,
    call: &CapabilityCallCandidate,
    summary: &CapabilityErrorSummary,
    model_observation: Option<ironclaw_turns::run_profile::ModelVisibleToolObservation>,
    capability_batch: &mut CapabilityBatchTurnSummary,
) -> Result<(), AgentLoopExecutorError> {
    append_capability_error_ref(host, state, call, summary, model_observation).await?;
    if capability_batch.invocation_count > 0
        && call.provider_replay.is_some()
        && let Ok(signature) = capability_call_signature(call)
    {
        capability_batch.record_result(signature, CapabilityProgress::Blocked, false);
    }
    Ok(())
}

async fn append_completed_capability_result(
    host: &(dyn ironclaw_turns::run_profile::AgentLoopDriverHost + Send + Sync),
    state: &mut LoopExecutionState,
    call: &CapabilityCallCandidate,
    result: CapabilityResultMessage,
    capability_batch: &mut CapabilityBatchTurnSummary,
) -> Result<(), AgentLoopExecutorError> {
    append_capability_result_ref(host, call, &result).await?;
    let signature = capability_call_signature(call)?;
    // Output-aware progress: if this exact call (same signature) produced an
    // output we have already observed this run, it advanced nothing — NoChange.
    // A first-seen output is MadeProgress. Without a digest (synthetic results or
    // older hosts) fall back to the host-reported progress. The membership check
    // MUST run before recording the observation, or a first occurrence would
    // immediately look "seen".
    let progress = match result.output_digest {
        Some(output_digest) => {
            let already_seen = state
                .seen_capability_output_digests
                .iter()
                .any(|observation| {
                    observation.signature == signature && observation.output_digest == output_digest
                });
            if already_seen {
                CapabilityProgress::NoChange
            } else {
                state
                    .seen_capability_output_digests
                    .push(CapabilityOutputObservation {
                        signature: signature.clone(),
                        output_digest,
                    });
                CapabilityProgress::MadeProgress
            }
        }
        None => result.progress,
    };
    capability_batch.record_result(signature, progress, result.terminate_hint);
    push_completed_result(state, &call.capability_id, result);
    Ok(())
}

fn shared_await_dependent_gate(
    calls: &[CapabilityCallCandidate],
    resolutions: &[Resolution],
) -> Option<(ironclaw_turns::LoopGateRef, CapabilityCallCandidate)> {
    let mut shared_gate: Option<ironclaw_turns::LoopGateRef> = None;
    let mut first_call: Option<CapabilityCallCandidate> = None;
    let mut count = 0_usize;
    for (call, resolution) in calls.iter().zip(resolutions.iter()) {
        match resolution {
            Resolution::Suspended(Suspension::DependentRun { waypoint, .. }) => {
                // Coalesce on the preserved originating loop gate ref; a missing
                // origin (never produced by the mapping) can't be coalesced.
                let gate_ref = waypoint
                    .origin
                    .as_ref()
                    .and_then(|origin| LoopGateRef::new(origin.as_str()).ok())?;
                if let Some(existing) = shared_gate.as_ref() {
                    if existing != &gate_ref {
                        return None;
                    }
                } else {
                    shared_gate = Some(gate_ref);
                    first_call = Some(call.clone());
                }
                count += 1;
            }
            // Any other parked work — a re-entrant gate (`Blocked`) or a
            // non-dependent-run suspension (process/external-tool) — means the
            // batch cannot coalesce into a single dependent-run gate. `parks()`,
            // not `is_suspension()`, to also catch `Blocked` (H1).
            resolution if resolution.parks() => {
                return None;
            }
            _ => {}
        }
    }
    // Only coalesce when at least two AwaitDependentRun outcomes share the
    // same gate — that is the case the fast path exists for. A single
    // AwaitDependentRun (with or without sibling completed outcomes) has no
    // coalescing benefit, and routing through this path would diverge the
    // completed-first durability ordering the non-suspended branch
    // guarantees. Fall back to the per-outcome path for single-await batches.
    if count <= 1 {
        return None;
    }
    shared_gate.zip(first_call)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_turns::{
        LoopGateRef, LoopResultRef,
        run_profile::{CapabilityInputRef, CapabilitySurfaceVersion, resolution},
    };

    fn call(input: &str) -> CapabilityCallCandidate {
        let capability_id = ironclaw_host_api::CapabilityId::new("test.cap").unwrap();
        CapabilityCallCandidate {
            activity_id: ironclaw_turns::CapabilityActivityId::new(),
            surface_version: CapabilitySurfaceVersion::new("test-v1").unwrap(),
            capability_id: capability_id.clone(),
            effective_capability_ids: vec![capability_id],
            input_ref: CapabilityInputRef::new(format!("input:{input}")).unwrap(),
            provider_replay: None,
        }
    }

    // The fixtures build the exact `Resolution` the producer constructors
    // emit so `shared_await_dependent_gate` sees the flip's channel shape
    // (origin preserved on the channel).
    fn await_dependent(gate: &str, result: &str) -> Resolution {
        resolution::await_dependent_run(
            LoopGateRef::new(gate).unwrap(),
            LoopResultRef::new(format!("result:{result}")).unwrap(),
            "summary".to_string(),
            0,
            None,
        )
        .resolution
    }

    fn completed(result: &str) -> Resolution {
        resolution::completed(
            LoopResultRef::new(format!("result:{result}")).unwrap(),
            "summary".to_string(),
            CapabilityProgress::MadeProgress,
            false,
            0,
            None,
            None,
        )
    }

    #[test]
    fn returns_some_for_two_outcomes_sharing_one_gate() {
        let calls = vec![call("a"), call("b")];
        let outcomes = vec![
            await_dependent("gate:batch-1", "r1"),
            await_dependent("gate:batch-1", "r2"),
        ];
        let result = shared_await_dependent_gate(&calls, &outcomes);
        assert!(result.is_some());
        let (gate, first) = result.unwrap();
        assert_eq!(gate.as_str(), "gate:batch-1");
        assert_eq!(first.input_ref.as_str(), "input:a");
    }

    #[test]
    fn returns_none_for_divergent_gate_refs() {
        let calls = vec![call("a"), call("b")];
        let outcomes = vec![
            await_dependent("gate:a", "r1"),
            await_dependent("gate:b", "r2"),
        ];
        assert!(shared_await_dependent_gate(&calls, &outcomes).is_none());
    }

    #[test]
    fn returns_none_for_single_await_with_completed_sibling() {
        // Single AwaitDependentRun has no coalescing benefit; fall back to
        // the per-outcome path for completed-first durability ordering.
        let calls = vec![call("a"), call("b")];
        let outcomes = vec![await_dependent("gate:1", "r1"), completed("r2")];
        assert!(shared_await_dependent_gate(&calls, &outcomes).is_none());
    }

    #[test]
    fn returns_none_when_non_await_suspension_present() {
        let calls = vec![call("a"), call("b")];
        let outcomes = vec![
            await_dependent("gate:1", "r1"),
            resolution::approval_required(
                LoopGateRef::new("gate:approval").unwrap(),
                "approval".to_string(),
                None,
            )
            .resolution,
        ];
        assert!(shared_await_dependent_gate(&calls, &outcomes).is_none());
    }

    #[test]
    fn returns_none_for_empty_outcomes() {
        assert!(shared_await_dependent_gate(&[], &[]).is_none());
    }

    #[test]
    fn returns_some_for_two_awaits_with_completed_between() {
        let calls = vec![call("a"), call("b"), call("c")];
        let outcomes = vec![
            await_dependent("gate:batch-2", "r1"),
            completed("r2"),
            await_dependent("gate:batch-2", "r3"),
        ];
        let result = shared_await_dependent_gate(&calls, &outcomes);
        assert!(result.is_some());
        let (gate, _) = result.unwrap();
        assert_eq!(gate.as_str(), "gate:batch-2");
    }

    #[test]
    fn prefixed_capability_summary_does_not_underflow_when_prefix_is_too_long() {
        let prefix = "x".repeat(MAX_SAFE_SUMMARY_BYTES + 1);
        let result = prefixed_capability_summary(prefix, "detail".to_string());

        assert!(matches!(
            result,
            Err(AgentLoopExecutorError::PlannerContract { detail })
                if detail == "host returned unsafe strategy summary"
        ));
    }

    #[test]
    fn prefixed_capability_summary_rephrases_fixed_input_encode_summary() {
        let summary = prefixed_capability_summary(
            "capability failed with invalid_input: ".to_string(),
            INPUT_ENCODE_HUMAN_SUMMARY.to_string(),
        )
        .expect("fixed input encode summary should be strategy-safe");

        assert_eq!(
            summary.as_str(),
            "capability failed with invalid_input: input could not be encoded"
        );
    }
}

// arch-exempt: large_file, pre-existing large file minimally touched for the §5.3 Stage 2a-i replay-payload move (field/store wiring + tests), plan #6175
