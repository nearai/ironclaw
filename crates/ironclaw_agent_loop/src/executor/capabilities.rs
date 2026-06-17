use std::collections::HashSet;
use std::ops::ControlFlow;

use async_trait::async_trait;
use ironclaw_turns::{
    LoopFailureKind, LoopResultRef,
    run_profile::{
        AuthResumeApprovalIdentity, CapabilityActivityId, CapabilityApprovalResume,
        CapabilityAuthResume, CapabilityAuthResumeReplay, CapabilityBatchInvocation,
        CapabilityCallCandidate, CapabilityFailureKind, CapabilityOutcome, CapabilityProgress,
        CapabilityResultMessage, LoopDriverNoteKind, LoopProgressEvent, VisibleCapabilitySurface,
    },
};

use crate::{
    state::{CapabilityOutputObservation, CheckpointKind, LoopExecutionState},
    strategies::{
        BatchPolicy, CapabilityBatchTurnSummary, CapabilityErrorClass, CapabilityErrorSummary,
        GateKind, RecoveryOutcome, SanitizedStrategySummary, TurnSummary,
    },
};

use super::{
    AgentLoopExecutorError, AwaitDependentRunGateInput, AwaitDependentRunGateStage, BatchStep,
    CancelCheck, CapabilitySurfaceIndex, CheckpointStage, ExecutorStage, GateInput, GateStage,
    MAX_CAPABILITY_RETRIES, StageContext, TurnCompletedStep, append_capability_error_ref,
    append_capability_result_ref, append_capability_safe_summary_ref, batch_policy_kind,
    cancelled_exit, capability_batch_counts, capability_call_signature, capability_error_class,
    capability_failure_kind, capability_host_error,
    capability_invocation_from_auth_resume_candidate, capability_invocation_from_candidate,
    capability_is_visible, capability_summary, clear_matching_pending_auth_resume, failed_exit,
    honor_retry_alteration, model_visible_capability_failure_observation, push_call_signature_once,
    push_completed_result, sanitized_strategy_summary,
};

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct CapabilityStage;

const MAX_SAFE_SUMMARY_BYTES: usize = 512;

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
        // parked capability (still-missing credential → re-block → infinite loop).
        // Surface a model-visible Authorization failure (retry forbidden) for the
        // denied call and let unrelated calls in the same batch proceed normally.
        //
        // We call `handle_capability_error` directly (not via
        // `handle_capability_outcome(Failed(Authorization, ...))`) because
        // `capability_failed_summary` builds a prefix containing `"authorization:"`
        // which is banned by `validate_loop_safe_summary`.
        if let Some(pending) = state.pending_auth_resume.as_ref().filter(|p| {
            matches!(
                p.disposition.as_ref(),
                Some(ironclaw_turns::GateResumeDisposition::Denied)
            )
        }) {
            let denied_cap_id = pending.capability_id.clone();
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
                    denied_cap_id,
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
        // the parked capability (re-dispatch → re-block → infinite loop).
        // Mirror the auth-gate pattern above: surface a model-visible
        // Authorization failure for only the denied call, let other parallel
        // calls in the same batch proceed normally.
        if let Some(pending) = state.pending_approval_resume.as_ref().filter(|p| {
            matches!(
                p.disposition.as_ref(),
                Some(ironclaw_turns::GateResumeDisposition::Denied)
            )
        }) {
            let denied_cap_id = pending.capability_id.clone();
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
                    denied_cap_id,
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

        if batch.outcomes.is_empty()
            || batch.outcomes.len() > visible_calls.len()
            || (!batch.stopped_on_suspension && batch.outcomes.len() != visible_calls.len())
        {
            return Err(AgentLoopExecutorError::PlannerContract {
                detail: "capability batch outcome count does not match invocations",
            });
        }

        let (result_count, denied_count, gated_count, failed_count) =
            capability_batch_counts(&batch.outcomes);
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

        let outcomes = batch.outcomes;
        // Multiple AwaitDependentRun outcomes that share a single gate_ref
        // must coalesce into ONE gate exit: each outcome's result_ref is
        // appended as a completed result (so the parent observes every
        // child's result on resume) and a single GateStage step transitions
        // the loop to BlockedDependentRun. Firing one gate step per outcome
        // would create duplicate gate records and race the resume attempts.
        let coalesced_gate_step = if !batch.stopped_on_suspension {
            shared_await_dependent_gate(&visible_calls, &outcomes)
        } else {
            None
        };
        if !batch.stopped_on_suspension {
            // Non-suspended batches record completed (and coalesced-await)
            // outcomes before handling any remaining gates so partial parallel
            // progress is durable in any later suspension checkpoint.
            let mut pending_outcomes = Vec::new();
            for (call, outcome) in visible_calls.into_iter().zip(outcomes) {
                match outcome {
                    CapabilityOutcome::Completed(result) => {
                        push_call_signature_once(&mut state, &mut signatures, &call)?;
                        clear_matching_pending_approval_resume(&mut state, &call);
                        clear_matching_pending_auth_resume(&mut state, &call);
                        append_completed_capability_result(
                            ctx.host,
                            &mut state,
                            &call,
                            result,
                            &mut capability_batch,
                        )
                        .await?;
                    }
                    CapabilityOutcome::SpawnedChildRun {
                        result_ref,
                        safe_summary,
                        byte_len,
                        ..
                    } => {
                        push_call_signature_once(&mut state, &mut signatures, &call)?;
                        clear_matching_pending_approval_resume(&mut state, &call);
                        clear_matching_pending_auth_resume(&mut state, &call);
                        append_spawned_child_result(
                            ctx.host,
                            &mut state,
                            &call,
                            result_ref,
                            safe_summary,
                            byte_len,
                            &mut capability_batch,
                        )
                        .await?;
                    }
                    CapabilityOutcome::AwaitDependentRun {
                        gate_ref,
                        result_ref,
                        safe_summary,
                        byte_len,
                    } if coalesced_gate_step
                        .as_ref()
                        .is_some_and(|(gate, _)| gate == &gate_ref) =>
                    {
                        push_call_signature_once(&mut state, &mut signatures, &call)?;
                        clear_matching_pending_approval_resume(&mut state, &call);
                        clear_matching_pending_auth_resume(&mut state, &call);
                        let result = CapabilityResultMessage {
                            result_ref,
                            safe_summary,
                            progress: CapabilityProgress::MadeProgress,
                            terminate_hint: false,
                            byte_len,
                            output_digest: None,
                        };
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
            for (call, outcome) in visible_calls.into_iter().zip(outcomes) {
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
    let detail = sanitized_strategy_summary(safe_summary)?;
    let detail = truncate_summary_detail(
        detail.as_str(),
        MAX_SAFE_SUMMARY_BYTES.saturating_sub(prefix.len()),
    );
    sanitized_strategy_summary(format!("{prefix}{detail}"))
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
        outcome: CapabilityOutcome,
        capability_batch: &mut CapabilityBatchTurnSummary,
    ) -> Result<BatchStep, AgentLoopExecutorError> {
        match outcome {
            CapabilityOutcome::Completed(result) => {
                clear_matching_pending_approval_resume(&mut state, &call);
                clear_matching_pending_auth_resume(&mut state, &call);
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
            CapabilityOutcome::SpawnedChildRun {
                result_ref,
                safe_summary,
                byte_len,
                ..
            } => {
                clear_matching_pending_approval_resume(&mut state, &call);
                clear_matching_pending_auth_resume(&mut state, &call);
                append_spawned_child_result(
                    ctx.host,
                    &mut state,
                    &call,
                    result_ref,
                    safe_summary,
                    byte_len,
                    capability_batch,
                )
                .await?;
                Ok(BatchStep::Continue(Box::new(state)))
            }
            CapabilityOutcome::ApprovalRequired {
                gate_ref,
                approval_resume,
                ..
            } => {
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
            CapabilityOutcome::AuthRequired {
                gate_ref,
                credential_requirements,
                auth_resume,
                ..
            } => {
                // When the invocation already passed an approval gate, carry that
                // identity into the auth resume contract before handing off to the
                // generic gate persistence stage.
                //
                // Extract BEFORE clearing so the data is still present.
                let prior_approval = state
                    .pending_approval_resume
                    .as_ref()
                    .filter(|r| r.capability_id == call.capability_id)
                    .map(|r| r.to_approval_resume());
                // Clearing here keeps the clear-on-every-outcome invariant; for auth
                // outcomes GateStage re-populates the record when it blocks.
                clear_matching_pending_approval_resume(&mut state, &call);
                clear_matching_pending_auth_resume(&mut state, &call);
                let auth_resume = auth_resume_for_gate(auth_resume, prior_approval.as_ref());
                GateStage
                    .process(
                        ctx,
                        GateInput {
                            state,
                            call,
                            kind: GateKind::Auth,
                            gate_ref,
                            credential_requirements,
                            approval_resume: prior_approval,
                            auth_resume,
                        },
                    )
                    .await
            }
            CapabilityOutcome::ResourceBlocked { gate_ref, .. } => {
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
            CapabilityOutcome::AwaitDependentRun {
                gate_ref,
                result_ref,
                safe_summary,
                byte_len,
            } => {
                let resolved_result = CapabilityResultMessage {
                    result_ref,
                    safe_summary,
                    progress: CapabilityProgress::MadeProgress,
                    terminate_hint: false,
                    byte_len,
                    output_digest: None,
                };
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
            CapabilityOutcome::SpawnedProcess(handle) => {
                self.fail_unsupported_process_wait(ctx, state, &call, &handle.process_ref)
                    .await
            }
            CapabilityOutcome::Denied(denied) => {
                state
                    .recent_failure_kinds
                    .push(LoopFailureKind::PolicyDenied);
                let summary = CapabilityErrorSummary {
                    class: CapabilityErrorClass::PolicyDenied,
                    safe_summary: capability_denied_summary(
                        denied.reason_kind.as_str(),
                        denied.safe_summary,
                    )?,
                    diagnostic_ref: None,
                };
                self.handle_capability_error(ctx, state, call, summary, None, capability_batch)
                    .await
            }
            CapabilityOutcome::Failed(failure) => {
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
                    let checked = CheckpointStage
                        .write(ctx, state, CheckpointKind::Final)
                        .await?;
                    return Ok(BatchStep::Exit(failed_exit(
                        ctx.host,
                        checked.state,
                        failure_kind,
                        Some(checked.checkpoint_id),
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
                        CapabilityOutcome::Failed(failure) => {
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
        let checked = CheckpointStage
            .write(ctx, state, CheckpointKind::Final)
            .await?;
        Ok(BatchStep::Exit(failed_exit(
            ctx.host,
            checked.state,
            LoopFailureKind::DriverBug,
            Some(checked.checkpoint_id),
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
        let checked = CheckpointStage
            .write(ctx, state, CheckpointKind::Final)
            .await?;
        Ok(BatchStep::Exit(failed_exit(
            ctx.host,
            checked.state,
            LoopFailureKind::CapabilityProtocolError,
            Some(checked.checkpoint_id),
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
    /// Partitions `visible_calls` by `denied_cap_id`.  For every call that
    /// matches the denied capability, synthesises a model-visible
    /// `Authorization` failure (retry `Forbidden`) via `handle_capability_error`
    /// and uses `planner_summary` as the planner-visible strategy summary
    /// (must pass `validate_loop_safe_summary`).
    ///
    /// Returns `ControlFlow::Break(step)` if `handle_capability_error` produced
    /// an `Exit` (caller should propagate it immediately), or
    /// `ControlFlow::Continue((state, remaining_calls))` with the surviving
    /// state and the calls that did *not* match the denied capability.  The
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
        denied_cap_id: ironclaw_host_api::CapabilityId,
        denied_activity_id: Option<CapabilityActivityId>,
        planner_summary: &'static str,
        visible_calls: Vec<CapabilityCallCandidate>,
    ) -> Result<
        ControlFlow<TurnCompletedStep, (LoopExecutionState, Vec<CapabilityCallCandidate>)>,
        AgentLoopExecutorError,
    > {
        let (denied_calls, remaining_calls): (Vec<_>, Vec<_>) = visible_calls
            .into_iter()
            .partition(|call| call.capability_id == denied_cap_id);

        let mut denied_activity_id = denied_activity_id;
        for call in denied_calls {
            push_call_signature_once(&mut state, signatures, &call)?;
            if let Some(activity_id) = denied_activity_id.take() {
                CheckpointStage
                    .emit_progress(
                        ctx,
                        LoopProgressEvent::CapabilityActivityFailed {
                            activity_id,
                            capability_id: call.capability_id.clone(),
                            reason_kind: CapabilityFailureKind::Authorization,
                        },
                    )
                    .await;
            }
            let failure = ironclaw_turns::run_profile::CapabilityFailure {
                error_kind: CapabilityFailureKind::Authorization,
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
    let prior_replay = || CapabilityAuthResumeReplay {
        input: prior_approval.input.clone(),
        estimate: prior_approval.estimate.clone(),
    };

    match auth_resume.as_mut() {
        Some(resume) => {
            resume.resume_token = prior_approval.resume_token.clone();
            resume.prior_approval.get_or_insert_with(prior_identity);
            resume.replay.get_or_insert_with(prior_replay);
            auth_resume
        }
        None => Some(CapabilityAuthResume {
            resume_token: prior_approval.resume_token.clone(),
            prior_approval: Some(prior_identity()),
            replay: Some(prior_replay()),
        }),
    }
}

async fn append_spawned_child_result(
    host: &(dyn ironclaw_turns::run_profile::AgentLoopDriverHost + Send + Sync),
    state: &mut LoopExecutionState,
    call: &CapabilityCallCandidate,
    result_ref: LoopResultRef,
    safe_summary: String,
    byte_len: u64,
    capability_batch: &mut CapabilityBatchTurnSummary,
) -> Result<(), AgentLoopExecutorError> {
    let safe_summary = sanitized_strategy_summary(safe_summary)?.into_inner();
    let result = CapabilityResultMessage {
        result_ref,
        safe_summary,
        progress: CapabilityProgress::MadeProgress,
        terminate_hint: false,
        byte_len,
        output_digest: None,
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
    outcomes: &[CapabilityOutcome],
) -> Option<(ironclaw_turns::LoopGateRef, CapabilityCallCandidate)> {
    let mut shared_gate: Option<ironclaw_turns::LoopGateRef> = None;
    let mut first_call: Option<CapabilityCallCandidate> = None;
    let mut count = 0_usize;
    for (call, outcome) in calls.iter().zip(outcomes.iter()) {
        match outcome {
            CapabilityOutcome::AwaitDependentRun { gate_ref, .. } => {
                if let Some(existing) = shared_gate.as_ref() {
                    if existing != gate_ref {
                        return None;
                    }
                } else {
                    shared_gate = Some(gate_ref.clone());
                    first_call = Some(call.clone());
                }
                count += 1;
            }
            other if other.is_suspension() => {
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
        run_profile::{CapabilityInputRef, CapabilitySurfaceVersion},
    };

    fn call(input: &str) -> CapabilityCallCandidate {
        let capability_id = ironclaw_host_api::CapabilityId::new("test.cap").unwrap();
        CapabilityCallCandidate {
            surface_version: CapabilitySurfaceVersion::new("test-v1").unwrap(),
            capability_id: capability_id.clone(),
            effective_capability_ids: vec![capability_id],
            input_ref: CapabilityInputRef::new(format!("input:{input}")).unwrap(),
            provider_replay: None,
        }
    }

    fn await_dependent(gate: &str, result: &str) -> CapabilityOutcome {
        CapabilityOutcome::AwaitDependentRun {
            gate_ref: LoopGateRef::new(gate).unwrap(),
            result_ref: LoopResultRef::new(format!("result:{result}")).unwrap(),
            safe_summary: "summary".to_string(),
            byte_len: 0,
        }
    }

    fn completed(result: &str) -> CapabilityOutcome {
        CapabilityOutcome::Completed(CapabilityResultMessage {
            result_ref: LoopResultRef::new(format!("result:{result}")).unwrap(),
            safe_summary: "summary".to_string(),
            progress: CapabilityProgress::MadeProgress,
            terminate_hint: false,
            byte_len: 0,
            output_digest: None,
        })
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
            CapabilityOutcome::ApprovalRequired {
                gate_ref: LoopGateRef::new("gate:approval").unwrap(),
                safe_summary: "approval".to_string(),
                approval_resume: None,
            },
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
}
