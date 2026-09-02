use std::{collections::HashSet, ops::ControlFlow};

use ironclaw_host_api::turn::CapabilityActivityId;
use ironclaw_host_api::{
    resolution::{Blocked, Resolution, Suspension, ToolVerdict},
    result_meta::{CapabilityRecoveryHint, FailureKind, SameCallRetryConstraint},
};
use ironclaw_loop_contracts::{
    CapabilityApprovalResume, CapabilityCallCandidate, CapabilityFailureDetail, LoopDriverNoteKind,
    LoopFailureKind, LoopProgressEvent, LoopRecoveryClass, LoopRecoveryDisposition,
    LoopRecoveryStage, ModelVisibleToolObservation, ObservationTrust, ToolObservationDetail,
    ToolObservationStatus, ToolRecoveryObservation,
};

use crate::{
    state::{CapabilityCallSignature, InvocationCharge, LoopExecutionState},
    strategies::{
        CapabilityBatchTurnSummary, CapabilityErrorSummary, GateKind, RecoveryOutcome,
        RetryAlteration, SanitizedStrategySummary, capability_error_to_failure_kind,
    },
};

use super::capabilities::{CapabilityStage, OutcomeStep};
use super::capability_failure::{
    capability_denied_observation_summary, capability_denied_summary, capability_failed_summary,
    capability_failure_from_recoverable, capability_port_error_observation,
    capability_port_error_summary, denial_detail_text, deny_reason_tag, deny_recovery,
    model_observation_diagnostic_detail,
};
use super::capability_helpers::{capability_call_signature, capability_invocation_from_candidate};
use super::capability_outcomes::{
    SelectedParallelTerminal, append_completed_capability_result, append_spawned_child_result,
    finish_selected_parallel_terminal,
};
use super::capability_records::{
    capability_result_from_outcome, child_result_from_outcome, dependent_run_result_message,
    loop_gate_ref_from_origin, loop_process_ref_from_origin,
};
use super::{
    AgentLoopExecutorError, AwaitDependentRunGateInput, AwaitDependentRunGateStage, BatchStep,
    CheckpointStage, ExecutorStage, FailedExitDetails, GateInput, GateStage,
    MAX_CAPABILITY_RETRIES, StageContext, TurnCompletedStep, append_capability_error_ref,
    approval_resume_from_gate, attach_failure_explanation, auth_resume_from_gate,
    cancelled_reason_from_signal, capability_error_failure_category, capability_host_error,
    capability_port_error_is_terminal, clear_matching_pending_approval_resume,
    clear_matching_pending_auth_resume, clear_matching_pending_external_tool_resume, failed_exit,
    honor_capability_retry_alteration, model_visible_capability_failure_observation,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum CapabilityRetryMode {
    Allow,
    Suppress,
}

pub(super) struct CapabilityErrorHandling {
    pub(super) summary: CapabilityErrorSummary,
    pub(super) model_observation: Option<ModelVisibleToolObservation>,
    pub(super) retry_mode: CapabilityRetryMode,
}

fn outcome_step_from_consumed_step(step: BatchStep) -> OutcomeStep {
    match step {
        BatchStep::Continue(next) => OutcomeStep::Continue(next),
        BatchStep::Exit(exit) => OutcomeStep::Exit { exit, state: None },
    }
}

impl CapabilityStage {
    /// Shared denied-resume short-circuit for approval and external-tool gates.
    ///
    /// Partitions `visible_calls` by the parked call's `activity_id`. For the
    /// matching call, synthesises a model-visible `GateDeclined` failure via
    /// `handle_capability_error` and uses `planner_summary` as the
    /// planner-visible strategy summary (must pass
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
    /// - Approval-gate denial: `state.pending_approval_resume = None` before
    ///   calling; `planner_summary = "approval gate denied by user"`.
    /// - External-tool cancellation: `state.pending_external_tool_resume =
    ///   None` before calling; `planner_summary = "external tool gate
    ///   cancelled by client"`.
    ///
    /// Auth-gate denial remains host-visible and does not use this helper.
    ///
    /// Both summaries are compile-time `&'static str` and are validated by
    /// `SanitizedStrategySummary::from_trusted_static` at the call site.
    // arch-exempt: too_many_args, denied-resume short-circuit threads the capability-batch dispatch context; needs a dispatch-context bundle, plan #4954
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn short_circuit_denied_resume(
        &self,
        ctx: StageContext<'_>,
        mut state: LoopExecutionState,
        signatures: &mut HashSet<CapabilityCallSignature>,
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
            // A denied resume starts as a local terminalization without
            // dispatching the parked capability, so do not record it as an
            // executed call unless recovery explicitly retries and the host
            // returns an outcome.
            let signature = capability_call_signature(&call)?;
            CheckpointStage
                .emit_progress(
                    ctx,
                    LoopProgressEvent::CapabilityActivityFailed {
                        activity_id: denied_activity_id,
                        capability_id: call.capability_id.clone(),
                        reason_kind: FailureKind::GateDeclined,
                        // Gate denial carries no host-authored message; the
                        // model-visible text is produced separately below.
                        safe_summary: None,
                    },
                )
                .await;
            let failure = ironclaw_loop_contracts::CapabilityFailure {
                error_kind: FailureKind::GateDeclined,
                safe_summary: "The user declined the capability request.".to_string(),
                detail: CapabilityFailureDetail::Diagnostic {
                    text: "The capability did not run because the user declined its approval request. Revise the approach or ask the user before trying again.".to_string(),
                },
            };
            state
                .recent_failure_kinds
                .push(capability_error_to_failure_kind(failure.error_kind));
            let model_observation = Some(model_visible_capability_failure_observation(&failure));
            let summary = CapabilityErrorSummary {
                kind: failure.error_kind,
                safe_summary: SanitizedStrategySummary::from_trusted_static(planner_summary),
            };
            match Box::pin(self.handle_capability_error(
                ctx,
                state,
                call,
                signature,
                CapabilityErrorHandling {
                    summary,
                    model_observation,
                    retry_mode: CapabilityRetryMode::Allow,
                },
                signatures,
                capability_batch,
            ))
            .await?
            {
                OutcomeStep::Continue(next) => state = *next,
                OutcomeStep::Exit {
                    exit,
                    state: Some(terminal_state),
                } => {
                    let finalized = finish_selected_parallel_terminal(
                        ctx,
                        *terminal_state,
                        SelectedParallelTerminal::Loop(exit),
                    )
                    .await?;
                    return Ok(ControlFlow::Break(finalized));
                }
                OutcomeStep::Exit { state: None, .. } => {
                    return Err(AgentLoopExecutorError::PlannerContract {
                        detail: "denied-resume exit did not return its mutated state",
                    });
                }
            }
        }

        // Return surviving state + remaining calls to the caller.
        // The caller checks remaining_calls.is_empty() and calls completed_turn
        // when there is nothing left to dispatch.
        Ok(ControlFlow::Continue((state, remaining_calls)))
    }
}

impl CapabilityStage {
    // arch-exempt: too_many_args, outcome handling threads the per-batch signature set through the canonical retry path; needs a dispatch-context bundle, plan #4954
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn handle_capability_outcome(
        &self,
        ctx: StageContext<'_>,
        mut state: LoopExecutionState,
        call_with_signature: (CapabilityCallCandidate, CapabilityCallSignature),
        resolution: Resolution,
        signatures: &mut HashSet<CapabilityCallSignature>,
        capability_batch: &mut CapabilityBatchTurnSummary,
        retry_mode: CapabilityRetryMode,
    ) -> Result<OutcomeStep, AgentLoopExecutorError> {
        let (call, signature) = call_with_signature;
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
                        signature,
                        result,
                        capability_batch,
                    )
                    .await?;
                    Ok(OutcomeStep::Continue(Box::new(state)))
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
                        signature,
                        input,
                        capability_batch,
                    )
                    .await?;
                    Ok(OutcomeStep::Continue(Box::new(state)))
                }
                ToolVerdict::RecoverableFailure {
                    ref error_kind,
                    ref diagnostic,
                } => {
                    let failure =
                        capability_failure_from_recoverable(error_kind, diagnostic, &outcome);
                    if failure.error_kind == FailureKind::Cancelled {
                        return self.cancelled_for_batch_drain(ctx, state);
                    }
                    state
                        .recent_failure_kinds
                        .push(capability_error_to_failure_kind(failure.error_kind));
                    let model_observation =
                        Some(model_visible_capability_failure_observation(&failure));
                    let summary = CapabilityErrorSummary {
                        kind: failure.error_kind,
                        safe_summary: capability_failed_summary(
                            failure.error_kind,
                            failure.safe_summary,
                        ),
                    };
                    Box::pin(self.handle_capability_error(
                        ctx,
                        state,
                        call,
                        signature,
                        CapabilityErrorHandling {
                            summary,
                            model_observation,
                            retry_mode,
                        },
                        signatures,
                        capability_batch,
                    ))
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
                    kind: FailureKind::PolicyDenied,
                    safe_summary: capability_denied_summary(reason, safe_summary.clone()),
                };
                // Denials used to pass `None` here, so nothing actionable
                // reached the model: no recovery, no retry constraint, no
                // repairs. The reason #6781 made specific was legible only as
                // text inside the summary. Carry it as structured recovery so
                // the model can tell "authenticate" from "ask a human" from
                // "this is permanently refused" (#6284 item 4).
                let (same_call_retry, recovery_hint) =
                    denial.reason_kind.map(deny_recovery).unwrap_or((
                        SameCallRetryConstraint::Forbidden,
                        CapabilityRecoveryHint::ReviseApproach,
                    ));
                let observation = ModelVisibleToolObservation {
                    schema_version:
                        ironclaw_loop_contracts::MODEL_VISIBLE_TOOL_OBSERVATION_SCHEMA_VERSION,
                    status: ToolObservationStatus::Error,
                    summary: capability_denied_observation_summary(reason),
                    detail: ToolObservationDetail::GenericFailure {
                        failure_kind: FailureKind::PolicyDenied,
                        detail: Some(denial_detail_text(reason, &safe_summary)),
                    },
                    artifacts: Vec::new(),
                    recovery: Some(ToolRecoveryObservation::new(same_call_retry, recovery_hint)),
                    trust: ObservationTrust::UntrustedToolOutput,
                };
                Box::pin(self.handle_capability_error(
                    ctx,
                    state,
                    call,
                    signature,
                    CapabilityErrorHandling {
                        summary,
                        model_observation: Some(observation),
                        retry_mode,
                    },
                    signatures,
                    capability_batch,
                ))
                .await
            }
            Resolution::Blocked(Blocked::Approval(waypoint)) => {
                let gate_ref = loop_gate_ref_from_origin(waypoint.origin.as_ref())?;
                let approval_resume =
                    approval_resume_from_gate(&gate_ref, waypoint.resume.as_ref(), &call);
                Ok(outcome_step_from_consumed_step(
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
                        .await?,
                ))
            }
            Resolution::Blocked(Blocked::Auth(waypoint)) => {
                let gate_ref = loop_gate_ref_from_origin(waypoint.origin.as_ref())?;
                // When the invocation already passed an approval gate, carry that
                // identity into the auth resume contract before handing off to the
                // generic gate persistence stage. Extract BEFORE clearing.
                let prior_approval = state
                    .pending_approval_resume
                    .as_ref()
                    .filter(|resume| resume.activity_id == call.activity_id)
                    .map(|r| r.to_approval_resume());
                clear_matching_pending_approval_resume(&mut state, &call);
                clear_matching_pending_auth_resume(&mut state, &call);
                clear_matching_pending_external_tool_resume(&mut state, &call);
                let auth_resume = auth_resume_from_gate(
                    &gate_ref,
                    waypoint.resume.as_ref(),
                    prior_approval.as_ref(),
                );
                // `credential_requirements` now ride the host `GateRecord::Auth`
                // (§5.2.9), not this model-visible channel; the runner re-reads them
                // from the record at the blocked exit to rebuild
                // `TurnRunRecord.credential_requirements`.
                Ok(outcome_step_from_consumed_step(
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
                        .await?,
                ))
            }
            Resolution::Blocked(Blocked::Resource(waypoint)) => {
                let gate_ref = loop_gate_ref_from_origin(waypoint.origin.as_ref())?;
                Ok(outcome_step_from_consumed_step(
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
                        .await?,
                ))
            }
            Resolution::Suspended(Suspension::ExternalTool(waypoint)) => {
                let gate_ref = loop_gate_ref_from_origin(waypoint.origin.as_ref())?;
                // The model called a client-supplied tool: park the run and return
                // control to the API client. No resume payload — the client submits
                // the tool output on resume.
                Ok(outcome_step_from_consumed_step(
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
                        .await?,
                ))
            }
            Resolution::Suspended(Suspension::DependentRun { waypoint, result }) => {
                let gate_ref = loop_gate_ref_from_origin(waypoint.origin.as_ref())?;
                let resolved_result = dependent_run_result_message(&result)?;
                Ok(outcome_step_from_consumed_step(
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
                        .await?,
                ))
            }
            Resolution::Suspended(Suspension::Process(waypoint)) => {
                let process_ref = loop_process_ref_from_origin(waypoint.origin.as_ref())?;
                self.fail_unsupported_process_wait(ctx, state, &call, &process_ref)
                    .await
            }
        }
    }
}

impl CapabilityStage {
    // arch-exempt: too_many_args, capability retry must share the per-batch signature set with its caller; needs a dispatch-context bundle, plan #4954
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn handle_capability_error(
        &self,
        ctx: StageContext<'_>,
        mut state: LoopExecutionState,
        call: CapabilityCallCandidate,
        signature: CapabilityCallSignature,
        handling: CapabilityErrorHandling,
        signatures: &mut HashSet<CapabilityCallSignature>,
        capability_batch: &mut CapabilityBatchTurnSummary,
    ) -> Result<OutcomeStep, AgentLoopExecutorError> {
        let CapabilityErrorHandling {
            mut summary,
            mut model_observation,
            retry_mode,
        } = handling;
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
            .filter(|resume| resume.activity_id == call.activity_id)
            .map(|r| r.to_approval_resume());
        let captured_auth_resume_origin: bool = state
            .pending_auth_resume
            .as_ref()
            .is_some_and(|resume| resume.activity_id == call.activity_id);
        let is_resume_origin = captured_approval_resume.is_some() || captured_auth_resume_origin;

        clear_matching_pending_approval_resume(&mut state, &call);
        clear_matching_pending_auth_resume(&mut state, &call);
        clear_matching_pending_external_tool_resume(&mut state, &call);
        // Resolved once for any retry dispatch below; the budget cannot
        // change mid-call.
        let resource_budget_policy = ctx
            .host
            .run_context()
            .resolved_run_profile
            .resource_budget_policy
            .clone();
        for _ in 0..MAX_CAPABILITY_RETRIES {
            let outcome = ctx
                .planner
                .recovery()
                .on_capability_error(&state, &summary, model_observation.as_ref())
                .await;
            let outcome = match outcome {
                RecoveryOutcome::Retry { recovery, .. }
                    if is_resume_origin || retry_mode == CapabilityRetryMode::Suppress =>
                {
                    RecoveryOutcome::ToolErrorResult { recovery }
                }
                other => other,
            };
            match outcome {
                RecoveryOutcome::ModelErrorObservation { .. } => {
                    return Err(AgentLoopExecutorError::PlannerContract {
                        detail: "ModelErrorObservation on capability error",
                    });
                }
                RecoveryOutcome::UserVisibleTerminal { .. } => {
                    return Err(AgentLoopExecutorError::PlannerContract {
                        detail: "UserVisibleTerminal on capability error",
                    });
                }
                RecoveryOutcome::ToolErrorResult { recovery } => {
                    state.recovery_state = recovery;
                    append_capability_error_ref(
                        ctx.host,
                        &mut state,
                        &call,
                        &summary,
                        model_observation.clone(),
                    )
                    .await?;
                    CheckpointStage
                        .emit_recovery(
                            ctx,
                            &mut state,
                            LoopRecoveryStage::Capability,
                            LoopRecoveryClass::Capability(summary.kind),
                            LoopRecoveryDisposition::ModelVisible,
                        )
                        .await?;
                    if let Some(signal) = ctx.host.observe_cancellation() {
                        return self.cancelled_for_batch_drain_with_reason(
                            ctx,
                            state,
                            cancelled_reason_from_signal(&signal),
                        );
                    }
                    return Ok(OutcomeStep::Continue(Box::new(state)));
                }
                RecoveryOutcome::Abort {
                    recovery,
                    failure_kind,
                } => {
                    state.recovery_state = recovery;
                    let terminal_detail =
                        model_observation_diagnostic_detail(model_observation.as_ref());
                    append_capability_error_ref(
                        ctx.host,
                        &mut state,
                        &call,
                        &summary,
                        model_observation.clone(),
                    )
                    .await?;
                    if let Some(signal) = ctx.host.observe_cancellation() {
                        return self.cancelled_for_batch_drain_with_reason(
                            ctx,
                            state,
                            cancelled_reason_from_signal(&signal),
                        );
                    }
                    let explanation_message_ref =
                        attach_failure_explanation(ctx, &mut state, failure_kind).await?;
                    let mut safe_failure = capability_error_failure_category(summary.kind)?;
                    if let Some(detail) = terminal_detail {
                        safe_failure = safe_failure.with_detail(detail);
                    }
                    let exit = failed_exit(
                        ctx.host,
                        state.clone(),
                        failure_kind,
                        None,
                        FailedExitDetails {
                            safe_summary: Some(safe_failure),
                            explanation_message_ref,
                        },
                    )?;
                    return Ok(OutcomeStep::Exit {
                        exit,
                        state: Some(Box::new(state)),
                    });
                }
                RecoveryOutcome::Retry {
                    recovery, alter, ..
                } => {
                    if let Some(signal) = ctx.host.observe_cancellation() {
                        return self.cancelled_for_batch_drain_with_reason(
                            ctx,
                            state,
                            cancelled_reason_from_signal(&signal),
                        );
                    }
                    if matches!(alter, Some(RetryAlteration::RepairInvalidModelOutput)) {
                        return Err(AgentLoopExecutorError::PlannerContract {
                            detail: "invalid model output repair retry is model-only",
                        });
                    }
                    honor_capability_retry_alteration(alter.as_ref())?;
                    // Budget accounting: every invocation that reaches
                    // dispatch counts, whatever its outcome (same rule as
                    // the initial batch dispatch above) — this retry
                    // dispatch would be a real capability invocation
                    // against the host, not a replay. Charge it through the
                    // same ledger chokepoint the initial batch uses,
                    // BEFORE dispatch. When the run's capability-invocation
                    // budget is already exhausted, do not re-dispatch at
                    // all — fall through to the same model-visible
                    // blocked-result path `ToolErrorResult` uses above, so
                    // a retry can never silently exceed
                    // `ResourceBudgetPolicy::max_capability_invocations`.
                    // The next `BudgetStage` iteration then hard-stops the
                    // run through the existing `CapabilityInvocationLimit`
                    // exit — no new exit path.
                    if state
                        .budget_ledger
                        .try_charge_invocations(1, &resource_budget_policy)
                        == InvocationCharge::Exhausted
                    {
                        state.recovery_state = recovery;
                        append_capability_error_ref(
                            ctx.host,
                            &mut state,
                            &call,
                            &summary,
                            model_observation.clone(),
                        )
                        .await?;
                        CheckpointStage
                            .emit_recovery(
                                ctx,
                                &mut state,
                                LoopRecoveryStage::Capability,
                                LoopRecoveryClass::Capability(summary.kind),
                                LoopRecoveryDisposition::ModelVisible,
                            )
                            .await?;
                        if let Some(signal) = ctx.host.observe_cancellation() {
                            return self.cancelled_for_batch_drain_with_reason(
                                ctx,
                                state,
                                cancelled_reason_from_signal(&signal),
                            );
                        }
                        return Ok(OutcomeStep::Continue(Box::new(state)));
                    }
                    state.recovery_state = recovery;
                    CheckpointStage
                        .emit_recovery(
                            ctx,
                            &mut state,
                            LoopRecoveryStage::Capability,
                            LoopRecoveryClass::Capability(summary.kind),
                            LoopRecoveryDisposition::Retried,
                        )
                        .await?;
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
                        Ok(outcome) => {
                            // The retry reached the host and returned a
                            // resolution, so this is the canonical point at
                            // which its signature becomes an executed call.
                            // The per-stage set prevents a first host outcome
                            // followed by a retry from recording twice.
                            if signatures.insert(signature.clone()) {
                                state.recent_call_signatures.push(signature.clone());
                            }
                            outcome
                        }
                        Err(ref error)
                            if error.kind
                                == ironclaw_loop_contracts::AgentLoopHostErrorKind::StaleSurface =>
                        {
                            summary = CapabilityErrorSummary {
                                kind: FailureKind::StaleSurface,
                                safe_summary: SanitizedStrategySummary::from_trusted_static(
                                    "capability surface changed before execution; re-issue the call",
                                ),
                            };
                            model_observation = None;
                            continue;
                        }
                        // Caller-shaped port error on the retry dispatch:
                        // re-enter the recovery loop with the new summary so
                        // the strategy routes it by fate (mirrors the
                        // StaleSurface arm above) instead of ending the run.
                        Err(error) if !capability_port_error_is_terminal(error.kind) => {
                            summary = capability_port_error_summary(&error);
                            model_observation = Some(capability_port_error_observation(&error));
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
                                    error_kind, diagnostic, &outcome,
                                ),
                                _ => unreachable!("guarded to RecoverableFailure"), // safety: the enclosing match only enters this branch for a recoverable failure.
                            };
                            if failure.error_kind == FailureKind::Cancelled {
                                return self.cancelled_for_batch_drain(ctx, state);
                            }
                            model_observation =
                                Some(model_visible_capability_failure_observation(&failure));
                            summary = CapabilityErrorSummary {
                                kind: failure.error_kind,
                                safe_summary: capability_failed_summary(
                                    failure.error_kind,
                                    failure.safe_summary,
                                ),
                            };
                        }
                        promoted => {
                            return Box::pin(self.handle_capability_outcome(
                                ctx,
                                state,
                                (call, signature),
                                promoted,
                                signatures,
                                capability_batch,
                                retry_mode,
                            ))
                            .await;
                        }
                    }
                }
            }
        }

        let terminal_detail = model_observation_diagnostic_detail(model_observation.as_ref());
        append_capability_error_ref(ctx.host, &mut state, &call, &summary, model_observation)
            .await?;
        // Route through the single failure-explanation chokepoint so the
        // recent-failure-kind record and (when the kind is explainable) the
        // explanation message ref are produced consistently with the other
        // failed-exit sites instead of being pushed inline here.
        let failure_kind = capability_error_to_failure_kind(summary.kind);
        let explanation_message_ref =
            attach_failure_explanation(ctx, &mut state, failure_kind).await?;
        let mut safe_failure = capability_error_failure_category(summary.kind)?;
        if let Some(detail) = terminal_detail {
            safe_failure = safe_failure.with_detail(detail);
        }
        let exit = failed_exit(
            ctx.host,
            state.clone(),
            failure_kind,
            None,
            FailedExitDetails {
                safe_summary: Some(safe_failure),
                explanation_message_ref,
            },
        )?;
        Ok(OutcomeStep::Exit {
            exit,
            state: Some(Box::new(state)),
        })
    }
}
