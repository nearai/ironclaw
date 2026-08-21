use std::collections::HashSet;
use std::ops::ControlFlow;

use async_trait::async_trait;
use futures::{StreamExt, stream::FuturesUnordered};
use ironclaw_host_api::turn::CapabilityActivityId;
use ironclaw_host_api::turn::{LoopGateRef, LoopResultRef};
use ironclaw_host_api::{
    decision::DenyReason,
    dispatch::INPUT_ENCODE_HUMAN_SUMMARY,
    ids::{ApprovalRequestId, CorrelationId},
    resolution::{
        Blocked, DependentRunResult, Outcome, Resolution, ResolutionBatch, Suspension, ToolVerdict,
    },
    result_meta::{
        CapabilityRecoveryHint, FailureKind, LoopRef, ModelFailureDiagnostic, ModelInputIssue,
        ResultProgress, ResumeToken, SameCallRetryConstraint,
    },
};
use ironclaw_loop_contracts::{
    AuthResumeApprovalIdentity, CapabilityApprovalResume, CapabilityAuthResume,
    CapabilityCallCandidate, CapabilityFailure, CapabilityFailureDetail, CapabilityInputIssue,
    CapabilityProgress, CapabilityResultMessage, CapabilityResumeToken, ContentDigest,
    LoopDriverNoteKind, LoopExit, LoopFailureKind, LoopProcessRef, LoopProgressEvent,
    LoopRecoveryClass, LoopRecoveryDisposition, LoopRecoveryStage, LoopRequest, LoopRequestBatch,
    MODEL_VISIBLE_TOOL_OBSERVATION_SCHEMA_VERSION, ModelVisibleToolObservation, ObservationTrust,
    ToolObservationDetail, ToolObservationStatus, ToolRecoveryObservation,
    VisibleCapabilitySurface,
};

use super::{
    AgentLoopExecutorError, AwaitDependentRunGateInput, AwaitDependentRunGateStage, BatchStep,
    CancelCheck, CapabilitySurfaceIndex, CheckpointStage, ExecutorStage, FailedExitDetails,
    GateInput, GateStage, MAX_CAPABILITY_RETRIES, StageContext, TurnCompletedStep,
    append_capability_error_ref, append_capability_result_ref, append_capability_safe_summary_ref,
    attach_failure_explanation, batch_policy_kind, cancelled_exit_with_reason,
    cancelled_reason_from_signal, capability_batch_counts, capability_call_signature,
    capability_error_failure_category, capability_host_error,
    capability_invocation_from_auth_resume_candidate, capability_invocation_from_candidate,
    capability_is_visible, capability_port_error_is_terminal, clear_matching_pending_auth_resume,
    clear_matching_pending_external_tool_resume, failed_exit, gate_tool_result_summary,
    honor_capability_retry_alteration, model_visible_capability_failure_observation,
    push_call_signature_once, push_completed_result, sanitized_strategy_summary_or_fallback,
};
use crate::{
    state::{CheckpointKind, InvocationCharge, LoopExecutionState},
    strategies::{
        BatchPolicy, CapabilityBatchTurnSummary, CapabilityErrorSummary, GateKind, RecoveryOutcome,
        RetryAlteration, SanitizedStrategySummary, TurnSummary, capability_error_to_failure_kind,
    },
};

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct CapabilityStage;

const MAX_SAFE_SUMMARY_BYTES: usize = 512;
const STRATEGY_INPUT_COULD_NOT_BE_ENCODED_SUMMARY: &str = "input could not be encoded";

const MAX_PARALLEL_CAPABILITY_INVOCATIONS: usize = 4;
pub(super) struct CapabilityInput {
    pub(super) state: LoopExecutionState,
    pub(super) surface: VisibleCapabilitySurface,
    pub(super) calls: Vec<CapabilityCallCandidate>,
}

/// Outcome-processing step for the capability batch drain.
///
/// Unlike `BatchStep`, the `Exit` arm carries the state the outcome was
/// processed against, so every processed sibling's mutations (assistant/result
/// refs, seen output digests, recent failure bookkeeping) can be merged back
/// into the shared drain state even when the outcome exits. `state` is `None`
/// only where the exit was produced by a stage that consumed the state itself
/// (gate stages, cancellation checks): those exits carry their own durable
/// checkpoint, and the run ends (or blocks) on them regardless.
enum OutcomeStep {
    Continue(Box<LoopExecutionState>),
    Exit {
        exit: LoopExit,
        state: Option<Box<LoopExecutionState>>,
    },
}

/// Convert a `BatchStep` produced by a stage that consumed the state itself
/// (gate stages, cancellation checks) into an [`OutcomeStep`]. The exit's
/// durable state lives in the checkpoint the stage wrote, so no merged state
/// is carried back (`None`).
fn outcome_step_from_consumed_step(step: BatchStep) -> OutcomeStep {
    match step {
        BatchStep::Continue(next) => OutcomeStep::Continue(next),
        BatchStep::Exit(exit) => OutcomeStep::Exit { exit, state: None },
    }
}

/// Rebuild a selected terminal sibling exit against a freshly staged Final
/// checkpoint so its checkpoint carries every processed sibling's mutations
/// (siblings after the exit were processed and merged after the exit's own
/// checkpoint was staged). The original reason/kind metadata and safe summary
/// are preserved; explanation refs are re-derived from the merged state's
/// assistant refs — the exit's own explanation was already appended to them
/// during its outcome processing.
fn rebuild_terminal_exit_against_checkpoint(
    ctx: StageContext<'_>,
    exit: LoopExit,
    state: LoopExecutionState,
    checkpoint_id: ironclaw_host_api::turn::TurnCheckpointId,
) -> Result<LoopExit, AgentLoopExecutorError> {
    match exit {
        LoopExit::Failed(failed) => failed_exit(
            ctx.host,
            state,
            failed.reason_kind,
            Some(checkpoint_id),
            FailedExitDetails {
                safe_summary: failed.safe_summary,
                explanation_message_ref: None,
            },
        ),
        LoopExit::Cancelled(cancelled) => {
            cancelled_exit_with_reason(ctx.host, state, cancelled.reason_kind, Some(checkpoint_id))
        }
        LoopExit::Completed(_) | LoopExit::Blocked(_) => {
            Err(AgentLoopExecutorError::PlannerContract {
                detail: "selected sibling exit was not terminal failure or cancellation",
            })
        }
    }
}

enum InvokedCapabilityOutcome {
    Resolution(Resolution),
    TerminalError(ironclaw_loop_contracts::AgentLoopHostError),
}

struct InvokedCapabilityBatch {
    outcomes: Vec<InvokedCapabilityOutcome>,
    /// The bounded scheduler stopped admitting calls after a gate or typed
    /// cancellation. This is broader than the host's suspension-only flag.
    truncated_launch_window: bool,
}

struct InvokedCapabilityBatchError {
    error: Box<ironclaw_loop_contracts::AgentLoopHostError>,
    launched_count: usize,
}

enum SelectedParallelTerminal {
    Loop(LoopExit),
    Host(ironclaw_loop_contracts::AgentLoopHostError),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CapabilityRetryMode {
    Allow,
    Suppress,
}

struct CapabilityErrorHandling {
    summary: CapabilityErrorSummary,
    model_observation: Option<ModelVisibleToolObservation>,
    retry_mode: CapabilityRetryMode,
}
async fn finish_selected_parallel_terminal(
    ctx: StageContext<'_>,
    state: LoopExecutionState,
    terminal: SelectedParallelTerminal,
) -> Result<TurnCompletedStep, AgentLoopExecutorError> {
    let checked = CheckpointStage
        .write(ctx, state, CheckpointKind::Final)
        .await?;
    match terminal {
        SelectedParallelTerminal::Loop(exit) => {
            let exit = rebuild_terminal_exit_against_checkpoint(
                ctx,
                exit,
                checked.state,
                checked.checkpoint_id,
            )?;
            Ok(TurnCompletedStep::Exit(exit))
        }
        SelectedParallelTerminal::Host(error) => Err(capability_host_error(error)),
    }
}

impl InvokedCapabilityBatch {
    fn from_resolution_batch(batch: ResolutionBatch) -> Self {
        Self {
            outcomes: batch
                .resolutions
                .into_iter()
                .map(InvokedCapabilityOutcome::Resolution)
                .collect(),
            truncated_launch_window: batch.stopped_on_suspension,
        }
    }
}
fn resolution_stops_parallel_launch(resolution: &Resolution) -> bool {
    resolution.parks()
        || matches!(
            resolution,
            Resolution::Done(outcome)
                if matches!(
                    &outcome.verdict,
                    ToolVerdict::RecoverableFailure { error_kind, .. }
                        if *error_kind == FailureKind::Cancelled
                )
        )
}

impl CapabilityStage {
    async fn invoke_batch(
        &self,
        ctx: StageContext<'_>,
        policy: BatchPolicy,
        invocations: Vec<LoopRequest>,
    ) -> Result<InvokedCapabilityBatch, InvokedCapabilityBatchError> {
        let ordered = invocations.len() >= 2
            && policy == BatchPolicy::Parallel
            && ctx.host.requires_ordered_batch_invocation(&invocations);
        if invocations.len() < 2 || policy != BatchPolicy::Parallel || ordered {
            return ctx
                .host
                .invoke_capability_batch(LoopRequestBatch {
                    invocations,
                    stop_on_first_suspension: matches!(policy, BatchPolicy::Sequential) || ordered,
                })
                .await
                .map(InvokedCapabilityBatch::from_resolution_batch)
                .map_err(|error| InvokedCapabilityBatchError {
                    error: Box::new(error),
                    launched_count: 0,
                });
        }

        let invocation_count = invocations.len();
        let mut indexed_invocations = invocations.into_iter().enumerate();
        let invoke = |(index, invocation)| async move {
            (index, ctx.host.invoke_capability(invocation).await)
        };
        let mut pending = FuturesUnordered::new();
        let mut launched = 0_usize;
        for _ in 0..MAX_PARALLEL_CAPABILITY_INVOCATIONS {
            let Some(indexed_invocation) = indexed_invocations.next() else {
                break;
            };
            pending.push(invoke(indexed_invocation));
            launched += 1;
        }

        let mut outcomes = (0..invocation_count)
            .map(|_| None)
            .collect::<Vec<
                Option<Result<Resolution, ironclaw_loop_contracts::AgentLoopHostError>>,
            >>();
        let mut stop_launching = false;
        let mut terminal_error_seen = false;
        while let Some((index, result)) = pending.next().await {
            match &result {
                Ok(resolution) => stop_launching |= resolution_stops_parallel_launch(resolution),
                Err(error) => {
                    terminal_error_seen |= capability_port_error_is_terminal(error.kind);
                }
            }
            outcomes[index] = Some(result);

            if !stop_launching
                && !terminal_error_seen
                && let Some(indexed_invocation) = indexed_invocations.next()
            {
                pending.push(invoke(indexed_invocation));
                launched += 1;
            }
        }

        let mut normalized = Vec::with_capacity(launched);
        for outcome in outcomes.into_iter().take(launched) {
            let outcome = match outcome {
                Some(Ok(resolution)) => InvokedCapabilityOutcome::Resolution(resolution),
                Some(Err(error)) if capability_port_error_is_terminal(error.kind) => {
                    InvokedCapabilityOutcome::TerminalError(error)
                }
                Some(Err(error)) => {
                    InvokedCapabilityOutcome::Resolution(recoverable_port_error_resolution(error))
                }
                None => {
                    return Err(InvokedCapabilityBatchError {
                        error: Box::new(ironclaw_loop_contracts::AgentLoopHostError::new(
                            ironclaw_loop_contracts::AgentLoopHostErrorKind::Internal,
                            "parallel capability invocation completed without an indexed outcome",
                        )),
                        launched_count: launched,
                    });
                }
            };
            normalized.push(outcome);
        }

        Ok(InvokedCapabilityBatch {
            outcomes: normalized,
            truncated_launch_window: (stop_launching || terminal_error_seen)
                && launched < invocation_count,
        })
    }
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
        let surface_index = CapabilitySurfaceIndex::new(&input.surface);
        let calls = input.calls;
        let denied_auth_activity_id = state
            .pending_auth_resume
            .as_ref()
            .filter(|pending| {
                matches!(
                    pending.disposition,
                    Some(ironclaw_host_api::turn::GateResumeDisposition::Denied)
                )
            })
            .map(|pending| pending.activity_id_for_resume());

        let mut visible_calls = Vec::new();
        let mut denied_calls = Vec::new();
        for call in calls {
            // A denied auth gate terminalizes the exact already-admitted
            // invocation. It is not a new capability dispatch, so removal from
            // the current surface must not strand the durable BlockedAuth
            // record. The loop-host and CapabilityHost still validate the
            // saved activity, scope, actor, and capability identity before
            // mutating it.
            if denied_auth_activity_id == Some(call.activity_id)
                || capability_is_visible(&surface_index, &call)
            {
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
                kind: FailureKind::PolicyDenied,
                safe_summary: SanitizedStrategySummary::from_trusted_static(
                    "capability is not visible in the filtered surface",
                ),
            };
            match Box::pin(self.handle_capability_error(
                ctx,
                state,
                call,
                CapabilityErrorHandling {
                    summary,
                    model_observation: None,
                    retry_mode: CapabilityRetryMode::Allow,
                },
                &mut capability_batch,
            ))
            .await?
            {
                OutcomeStep::Continue(next) => state = *next,
                OutcomeStep::Exit {
                    exit,
                    state: Some(terminal_state),
                } => {
                    return finish_selected_parallel_terminal(
                        ctx,
                        *terminal_state,
                        SelectedParallelTerminal::Loop(exit),
                    )
                    .await;
                }
                OutcomeStep::Exit { state: None, .. } => {
                    return Err(AgentLoopExecutorError::PlannerContract {
                        detail: "capability error exit did not return its mutated state",
                    });
                }
            }
        }

        if visible_calls.is_empty() {
            return self
                .completed_turn(ctx, state, result_refs_start, capability_batch)
                .await;
        }

        // A run resumed from a user-DENIED approval gate must not re-dispatch
        // the parked capability (re-dispatch -> re-block -> infinite loop).
        // Mirror the auth-gate pattern above: surface a model-visible
        // gate-declined failure for only the denied call, let other parallel
        // calls in the same batch proceed normally.
        if let Some(pending) = state.pending_approval_resume.as_ref().filter(|p| {
            matches!(
                p.disposition.as_ref(),
                Some(ironclaw_host_api::turn::GateResumeDisposition::Denied)
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
                Some(ironclaw_host_api::turn::GateResumeDisposition::Denied)
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

        // A single model turn must not admit more calls than the remaining
        // per-run resource budget allows. `BudgetStage` only hard-stops the
        // run between outer-loop iterations (budget.rs), so without this cap
        // one oversized batch could dispatch (and charge)
        // `visible_calls.len()` invocations even when the remaining
        // allowance is smaller. `try_charge_invocations` reserves and
        // commits the admitted count in the same call; every call beyond it
        // is not dispatched — it gets a model-visible blocked result via the
        // same denied-calls machinery used above, so tool_use/tool_result
        // pairing still holds for the whole batch. After dispatch, the
        // reservation is settled against the host's authoritative launched
        // count so a truncated launch window does not consume budget for its
        // unlaunched suffix. Once the counter reaches the cap, the next
        // `BudgetStage` iteration hard stops the run through the existing
        // `hard_budget_exit`.
        let resource_budget_policy = ctx
            .host
            .run_context()
            .resolved_run_profile
            .resource_budget_policy
            .clone();
        let invocation_charge = state
            .budget_ledger
            .try_charge_invocations(visible_calls.len(), &resource_budget_policy);
        let over_budget_calls: Vec<CapabilityCallCandidate> = match invocation_charge {
            InvocationCharge::Charged => Vec::new(),
            InvocationCharge::Partial { admitted } => visible_calls.split_off(admitted),
            InvocationCharge::Exhausted => std::mem::take(&mut visible_calls),
        };
        if !over_budget_calls.is_empty() {
            for call in over_budget_calls {
                push_call_signature_once(&mut state, &mut signatures, &call)?;
                let summary = CapabilityErrorSummary {
                    kind: FailureKind::Resource,
                    safe_summary: SanitizedStrategySummary::from_trusted_static(
                        "the run's capability-invocation budget is exhausted for this turn; \
                         stop issuing further capability calls",
                    ),
                };
                state
                    .recent_failure_kinds
                    .push(capability_error_to_failure_kind(summary.kind));
                match Box::pin(self.handle_capability_error(
                    ctx,
                    state,
                    call,
                    CapabilityErrorHandling {
                        summary,
                        model_observation: None,
                        // Never dispatch a call this batch already decided is
                        // over budget, even if the recovery strategy would
                        // otherwise retry it.
                        retry_mode: CapabilityRetryMode::Suppress,
                    },
                    &mut capability_batch,
                ))
                .await?
                {
                    OutcomeStep::Continue(next) => state = *next,
                    OutcomeStep::Exit {
                        exit,
                        state: Some(terminal_state),
                    } => {
                        return finish_selected_parallel_terminal(
                            ctx,
                            *terminal_state,
                            SelectedParallelTerminal::Loop(exit),
                        )
                        .await;
                    }
                    OutcomeStep::Exit { state: None, .. } => {
                        return Err(AgentLoopExecutorError::PlannerContract {
                            detail: "capability budget exit did not return its mutated state",
                        });
                    }
                }
            }
            if visible_calls.is_empty() {
                return self
                    .completed_turn(ctx, state, result_refs_start, capability_batch)
                    .await;
            }
        }

        // Multiple calls in one model response are the model's declaration
        // that the calls are semantically independent. The host may still
        // require ordered batch entry for operational or policy reasons.
        let policy = BatchPolicy::Parallel;

        capability_batch = CapabilityBatchTurnSummary::for_invocation_count(visible_calls.len());
        // Budget accounting: reserve the admitted launch window above, then
        // settle it against the authoritative launched count below.

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
        let invocations = visible_calls
            .iter()
            .cloned()
            .map(|call| {
                // Auth-resume takes precedence: when the run is parked
                // at a BlockedAuth checkpoint that also carried prior
                // approval identity, re-dispatch through the auth-resume
                // path so the original invocation_id is reused.
                //
                // Consume only the parked activity's slot. Two calls may share
                // one capability id; capability identity is therefore too
                // coarse for resume-token ownership.
                if let Some(auth) =
                    pending_auth_resume.take_if(|auth| auth.activity_id == call.activity_id)
                {
                    return capability_invocation_from_auth_resume_candidate(call, &auth);
                }
                let resume = pending_approval_resume
                    .take_if(|resume| resume.activity_id == call.activity_id)
                    .map(|resume| resume.to_approval_resume());
                capability_invocation_from_candidate(call, resume)
            })
            .collect();
        let batch_result = self.invoke_batch(ctx, policy, invocations).await;
        if let Ok(batch) = &batch_result
            && (batch.outcomes.is_empty()
                || batch.outcomes.len() > visible_calls.len()
                || (!batch.truncated_launch_window && batch.outcomes.len() != visible_calls.len()))
        {
            return Err(AgentLoopExecutorError::PlannerContract {
                detail: "capability batch outcome count does not match invocations",
            });
        }
        let launched_count = match &batch_result {
            Ok(batch) => batch.outcomes.len(),
            Err(failure) => failure.launched_count,
        };
        if !state
            .budget_ledger
            .settle_invocation_reservation(visible_calls.len(), launched_count)
        {
            return Err(AgentLoopExecutorError::PlannerContract {
                detail: "capability batch launch count cannot settle its budget reservation",
            });
        }

        let batch = match batch_result {
            Ok(batch) => batch,
            Err(InvokedCapabilityBatchError { ref error, .. })
                if error.kind == ironclaw_loop_contracts::AgentLoopHostErrorKind::StaleSurface =>
            {
                let stale_summary = SanitizedStrategySummary::from_trusted_static(
                    "capability surface changed before execution; re-issue the call",
                );
                for call in visible_calls {
                    push_call_signature_once(&mut state, &mut signatures, &call)?;
                    state
                        .recent_failure_kinds
                        .push(LoopFailureKind::PolicyDenied);
                    // The honest kind for a surface raced by a refresh; its
                    // fate stays ModelVisible (re-issue against the fresh
                    // surface) and its wire failure category stays
                    // `capability_policy_denied`, matching the retired mint.
                    let summary = CapabilityErrorSummary {
                        kind: FailureKind::StaleSurface,
                        safe_summary: stale_summary.clone(),
                    };
                    match Box::pin(self.handle_capability_error(
                        ctx,
                        state,
                        call,
                        CapabilityErrorHandling {
                            summary,
                            model_observation: None,
                            retry_mode: CapabilityRetryMode::Allow,
                        },
                        &mut capability_batch,
                    ))
                    .await?
                    {
                        OutcomeStep::Continue(next) => state = *next,
                        OutcomeStep::Exit {
                            exit,
                            state: Some(terminal_state),
                        } => {
                            return finish_selected_parallel_terminal(
                                ctx,
                                *terminal_state,
                                SelectedParallelTerminal::Loop(exit),
                            )
                            .await;
                        }
                        OutcomeStep::Exit { state: None, .. } => {
                            return Err(AgentLoopExecutorError::PlannerContract {
                                detail: "stale-surface exit did not return its mutated state",
                            });
                        }
                    }
                }
                return self
                    .completed_turn(ctx, state, result_refs_start, capability_batch)
                    .await;
            }
            // A caller-shaped port error (unauthorized, scope mismatch,
            // invalid invocation, policy denied, ...) must not end the run:
            // surface it to the model as a tool error for every call in the
            // batch — mirroring the StaleSurface arm above — and let the
            // recovery strategy route it by `FailureKind::fate`. Only genuine
            // host faults keep the terminal `capability_host_error` path
            // below.
            Err(InvokedCapabilityBatchError { error, .. })
                if !capability_port_error_is_terminal(error.kind) =>
            {
                let summary = capability_port_error_summary(&error);
                let observation = capability_port_error_observation(&error);
                for call in visible_calls {
                    push_call_signature_once(&mut state, &mut signatures, &call)?;
                    state
                        .recent_failure_kinds
                        .push(capability_error_to_failure_kind(summary.kind));
                    // Boxed: this second inlined `handle_capability_error`
                    // call site would otherwise grow the (already enormous)
                    // executor future past the test-thread stack.
                    match Box::pin(self.handle_capability_error(
                        ctx,
                        state,
                        call,
                        CapabilityErrorHandling {
                            summary: summary.clone(),
                            model_observation: Some(observation.clone()),
                            retry_mode: CapabilityRetryMode::Allow,
                        },
                        &mut capability_batch,
                    ))
                    .await?
                    {
                        OutcomeStep::Continue(next) => state = *next,
                        OutcomeStep::Exit {
                            exit,
                            state: Some(terminal_state),
                        } => {
                            return finish_selected_parallel_terminal(
                                ctx,
                                *terminal_state,
                                SelectedParallelTerminal::Loop(exit),
                            )
                            .await;
                        }
                        OutcomeStep::Exit { state: None, .. } => {
                            return Err(AgentLoopExecutorError::PlannerContract {
                                detail: "batch port-error exit did not return its mutated state",
                            });
                        }
                    }
                }
                return self
                    .completed_turn(ctx, state, result_refs_start, capability_batch)
                    .await;
            }
            Err(failure) => return Err(capability_host_error(*failure.error)),
        };

        let InvokedCapabilityBatch {
            outcomes,
            truncated_launch_window,
        } = batch;
        let has_terminal_error = outcomes
            .iter()
            .any(|outcome| matches!(outcome, InvokedCapabilityOutcome::TerminalError(_)));
        let (result_count, denied_count, gated_count, mut failed_count) =
            capability_batch_counts(outcomes.iter().filter_map(|outcome| match outcome {
                InvokedCapabilityOutcome::Resolution(resolution) => Some(resolution),
                InvokedCapabilityOutcome::TerminalError(_) => None,
            }));
        failed_count += outcomes
            .iter()
            .filter(|outcome| matches!(outcome, InvokedCapabilityOutcome::TerminalError(_)))
            .count() as u32;
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

        let resolution_snapshot = if has_terminal_error {
            None
        } else {
            Some(
                outcomes
                    .iter()
                    .filter_map(|outcome| match outcome {
                        InvokedCapabilityOutcome::Resolution(resolution) => {
                            Some(resolution.clone())
                        }
                        InvokedCapabilityOutcome::TerminalError(_) => None,
                    })
                    .collect::<Vec<_>>(),
            )
        };
        let coalesced_gate_step = if truncated_launch_window {
            None
        } else {
            resolution_snapshot
                .as_deref()
                .and_then(|resolutions| shared_await_dependent_gate(&visible_calls, resolutions))
        };
        let unlaunched_calls = &visible_calls[outcomes.len()..];
        let unlaunched_approval_resume =
            state
                .pending_approval_resume
                .as_ref()
                .is_some_and(|resume| {
                    unlaunched_calls
                        .iter()
                        .any(|call| call.activity_id == resume.activity_id)
                });
        let unlaunched_auth_resume = state.pending_auth_resume.as_ref().is_some_and(|resume| {
            unlaunched_calls
                .iter()
                .any(|call| call.activity_id == resume.activity_id)
        });
        let unlaunched_external_tool_resume = state
            .pending_external_tool_resume
            .as_ref()
            .is_some_and(|resume| {
                unlaunched_calls
                    .iter()
                    .any(|call| call.activity_id == resume.activity_id)
            });

        let indexed_outcomes = visible_calls
            .into_iter()
            .zip(outcomes)
            .enumerate()
            .map(|(index, (call, outcome))| (index, call, outcome))
            .collect::<Vec<_>>();

        // Every launched call signature must precede any gate or terminal
        // checkpoint. A resumed run must never forget work that was already
        // admitted merely because an earlier outcome selected the run exit.
        for (_, call, _) in &indexed_outcomes {
            push_call_signature_once(&mut state, &mut signatures, call)?;
        }

        // Durable successful work is recorded before any gate or terminal
        // outcome can return. Dependent-run siblings sharing one gate are
        // likewise materialized here, then coalesced into one gate below.
        let mut pending_outcomes = Vec::new();
        let mut coalesced_gate_index = None;
        for (index, call, outcome) in indexed_outcomes {
            match outcome {
                InvokedCapabilityOutcome::Resolution(Resolution::Done(outcome))
                    if outcome.verdict.is_success() =>
                {
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
                InvokedCapabilityOutcome::Resolution(Resolution::Done(outcome))
                    if matches!(outcome.verdict, ToolVerdict::ChildSpawned { .. }) =>
                {
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
                InvokedCapabilityOutcome::Resolution(Resolution::Suspended(
                    Suspension::DependentRun { waypoint, result },
                )) if coalesced_gate_step.as_ref().is_some_and(|(gate, _)| {
                    waypoint.origin.as_ref().map(LoopRef::as_str) == Some(gate.as_str())
                }) =>
                {
                    coalesced_gate_index.get_or_insert(index);
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
                other => pending_outcomes.push((index, call, other)),
            }
        }

        // One gate can own the batch's resumable checkpoint. Defer the first
        // gate in input order until every launched sibling is durably drained;
        // later gates become explicit model-visible pending outcomes.
        let mut first_gate = None;
        let mut sibling_outcomes = Vec::with_capacity(pending_outcomes.len());
        for item in pending_outcomes {
            let is_gate = matches!(
                &item.2,
                InvokedCapabilityOutcome::Resolution(resolution)
                    if gate_outcome_writes_before_block(resolution)
            );
            if first_gate.is_none() && is_gate {
                first_gate = Some(item);
            } else {
                sibling_outcomes.push(item);
            }
        }
        let gate_seen = first_gate.is_some() || coalesced_gate_index.is_some();
        // A truncated prefix must not overwrite a same-kind resume slot owned
        // by an unlaunched activity. Complete the prefix gate model-visibly;
        // the parked sibling retains its token for the next executor pass.
        let first_gate_conflicts_with_unlaunched_resume = first_gate
            .as_ref()
            .and_then(|(_, _, outcome)| match outcome {
                InvokedCapabilityOutcome::Resolution(resolution) => gate_outcome_kind(resolution),
                InvokedCapabilityOutcome::TerminalError(_) => None,
            })
            .is_some_and(|kind| match kind {
                GateKind::Approval => unlaunched_approval_resume,
                GateKind::Auth => unlaunched_auth_resume,
                GateKind::ExternalTool => unlaunched_external_tool_resume,
                GateKind::Resource | GateKind::AwaitDependentRun => false,
            });
        if first_gate_conflicts_with_unlaunched_resume
            && let Some((_, call, InvokedCapabilityOutcome::Resolution(resolution))) =
                first_gate.take()
        {
            persist_later_gate_outcome(ctx, &mut state, call, resolution).await?;
        }
        // If cancellation was already requested while the concurrent window
        // was in flight, make the deferred gate model-visible before any
        // sibling handler observes that signal and returns its Final exit.
        if ctx.host.observe_cancellation().is_some()
            && let Some((_, call, InvokedCapabilityOutcome::Resolution(resolution))) =
                first_gate.take()
        {
            persist_later_gate_outcome(ctx, &mut state, call, resolution).await?;
        }

        // A drain adjacent to a terminal host error or any gate must not
        // launch replacement capability calls: all original calls have already
        // run concurrently, and retrying here can duplicate side effects.
        let retry_mode = if has_terminal_error || gate_seen {
            CapabilityRetryMode::Suppress
        } else {
            CapabilityRetryMode::Allow
        };
        let mut selected: Option<(usize, SelectedParallelTerminal)> = None;
        for (index, call, outcome) in sibling_outcomes {
            match outcome {
                InvokedCapabilityOutcome::TerminalError(error) => {
                    if selected
                        .as_ref()
                        .is_none_or(|(selected_index, _)| index < *selected_index)
                    {
                        selected = Some((index, SelectedParallelTerminal::Host(error)));
                    }
                }
                InvokedCapabilityOutcome::Resolution(resolution)
                    if gate_outcome_writes_before_block(&resolution) =>
                {
                    persist_later_gate_outcome(ctx, &mut state, call, resolution).await?;
                }
                InvokedCapabilityOutcome::Resolution(resolution) => {
                    let snapshot = state.clone();
                    match self
                        .handle_capability_outcome(
                            ctx,
                            snapshot,
                            call,
                            resolution,
                            &mut capability_batch,
                            retry_mode,
                        )
                        .await?
                    {
                        OutcomeStep::Continue(next) => state = *next,
                        OutcomeStep::Exit {
                            exit,
                            state: mutated,
                        } => {
                            let Some(mutated) = mutated else {
                                return Err(AgentLoopExecutorError::PlannerContract {
                                    detail: "non-gate capability exit did not return its mutated state",
                                });
                            };
                            state = *mutated;
                            if selected
                                .as_ref()
                                .is_none_or(|(selected_index, _)| index < *selected_index)
                            {
                                selected = Some((index, SelectedParallelTerminal::Loop(exit)));
                            }
                        }
                    }
                }
            }
        }

        if let Some((
            gate_index,
            first_call,
            InvokedCapabilityOutcome::Resolution(first_gate_outcome),
        )) = first_gate
        {
            let cancellation_selected = matches!(
                selected.as_ref(),
                Some((_, SelectedParallelTerminal::Loop(LoopExit::Cancelled(_))))
            );
            if cancellation_selected
                || selected
                    .as_ref()
                    .is_some_and(|(selected_index, _)| *selected_index < gate_index)
            {
                persist_later_gate_outcome(ctx, &mut state, first_call, first_gate_outcome).await?;
            } else {
                match self
                    .handle_capability_outcome(
                        ctx,
                        state,
                        first_call,
                        first_gate_outcome,
                        &mut capability_batch,
                        retry_mode,
                    )
                    .await?
                {
                    OutcomeStep::Continue(next) => state = *next,
                    OutcomeStep::Exit { exit, .. } => {
                        return Ok(TurnCompletedStep::Exit(exit));
                    }
                }
            }
        }

        if let Some((shared_gate_ref, first_call)) = coalesced_gate_step {
            let gate_index =
                coalesced_gate_index.ok_or(AgentLoopExecutorError::PlannerContract {
                    detail: "coalesced dependent-run gate lost its input index",
                })?;
            let cancellation_selected = matches!(
                selected.as_ref(),
                Some((_, SelectedParallelTerminal::Loop(LoopExit::Cancelled(_))))
            );
            if !cancellation_selected
                && selected
                    .as_ref()
                    .is_none_or(|(selected_index, _)| *selected_index >= gate_index)
            {
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
        }
        if let Some((_, terminal)) = selected {
            return finish_selected_parallel_terminal(ctx, state, terminal).await;
        }
        if has_terminal_error {
            return Err(AgentLoopExecutorError::PlannerContract {
                detail: "terminal capability error batch had no selected terminal outcome",
            });
        }

        self.completed_turn(ctx, state, result_refs_start, capability_batch)
            .await
    }
}

/// Strategy-visible summary for a capability-stage port `Err` whose kind is
/// NOT a terminal host fault (`capability_port_error_is_terminal` == false).
/// The kind projection is owned by `AgentLoopHostErrorKind::failure_kind`;
/// the summary text fail-softs through `capability_failed_summary` (a summary
/// that trips the strict validator degrades to a canned fallback instead of
/// borking the run).
fn capability_port_error_summary(
    error: &ironclaw_loop_contracts::AgentLoopHostError,
) -> CapabilityErrorSummary {
    let kind = error.kind.failure_kind();
    CapabilityErrorSummary {
        kind,
        safe_summary: capability_failed_summary(kind, error.safe_summary.clone()),
    }
}

/// Model-visible observation for a recoverable capability-stage port `Err`.
/// Carries the port error's secret-scrubbed `detail` (when present) so the
/// model can retry or explain instead of guessing from the kind alone.
fn capability_failure_from_port_error(
    error: &ironclaw_loop_contracts::AgentLoopHostError,
) -> CapabilityFailure {
    let detail = error.detail.clone().unwrap_or_else(|| {
        ironclaw_loop_contracts::sanitize_model_visible_text(error.safe_summary.clone())
    });
    CapabilityFailure {
        error_kind: error.kind.failure_kind(),
        safe_summary: error.safe_summary.clone(),
        detail: CapabilityFailureDetail::Diagnostic { text: detail },
    }
}

fn recoverable_port_error_resolution(
    error: ironclaw_loop_contracts::AgentLoopHostError,
) -> Resolution {
    let failure = capability_failure_from_port_error(&error);
    ironclaw_loop_contracts::resolution::failed(
        failure.error_kind,
        failure.safe_summary,
        failure.detail,
    )
}

/// Model-visible observation for a recoverable capability-stage port `Err`.
/// Carries the port error's secret-scrubbed `detail` (when present) so the
/// model can retry or explain instead of guessing from the kind alone.
fn capability_port_error_observation(
    error: &ironclaw_loop_contracts::AgentLoopHostError,
) -> ModelVisibleToolObservation {
    model_visible_capability_failure_observation(&capability_failure_from_port_error(error))
}

fn capability_failed_summary(
    error_kind: FailureKind,
    safe_summary: String,
) -> SanitizedStrategySummary {
    prefixed_capability_summary(
        format!("capability failed with {}: ", error_kind.as_str()),
        safe_summary,
    )
}

fn capability_denied_summary(reason_kind: &str, safe_summary: String) -> SanitizedStrategySummary {
    prefixed_capability_summary(
        format!("capability denied with {reason_kind}: "),
        safe_summary,
    )
}

fn prefixed_capability_summary(prefix: String, safe_summary: String) -> SanitizedStrategySummary {
    let safe_summary = strategy_safe_capability_summary_detail(safe_summary);
    // Fail soft: a capability summary that fails strict validation degrades to
    // a fixed fallback instead of aborting the run. The real cause still rides
    // the model-visible Diagnostic detail via the paired observation, so no
    // information is lost by degrading the card summary here.
    let (detail, _) = sanitized_strategy_summary_or_fallback(
        safe_summary,
        "the tool failure details were redacted",
    );
    let detail = truncate_summary_detail(
        detail.as_str(),
        MAX_SAFE_SUMMARY_BYTES.saturating_sub(prefix.len()),
    );
    // The combined summary must also fail soft: the prefix itself can trip the
    // validator (`Failed(Authorization)` yields "capability failed with
    // authorization: ", and "authorization:" is a banned marker). A recoverable
    // tool failure must never turn terminal because of its own card label.
    let (summary, _) = sanitized_strategy_summary_or_fallback(
        format!("{prefix}{detail}"),
        "the tool failure details were redacted",
    );
    summary
}

/// Extract the secret-scrubbed model-visible diagnostic text from a tool
/// observation, if any, so a terminal capability failure can carry the real
/// cause on `SanitizedFailure.detail` for the failure explainer.
fn model_observation_diagnostic_detail(
    observation: Option<&ModelVisibleToolObservation>,
) -> Option<String> {
    match observation.map(|observation| &observation.detail) {
        Some(ToolObservationDetail::GenericFailure {
            detail: Some(detail),
            ..
        }) => Some(detail.clone()),
        _ => None,
    }
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
        retry_mode: CapabilityRetryMode,
    ) -> Result<OutcomeStep, AgentLoopExecutorError> {
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
                        CapabilityErrorHandling {
                            summary,
                            model_observation,
                            retry_mode,
                        },
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
                    CapabilityErrorHandling {
                        summary,
                        model_observation: Some(observation),
                        retry_mode,
                    },
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

    async fn handle_capability_error(
        &self,
        ctx: StageContext<'_>,
        mut state: LoopExecutionState,
        call: CapabilityCallCandidate,
        handling: CapabilityErrorHandling,
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
                    append_blocked_capability_error_result(
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
                    append_blocked_capability_error_result(
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
                        append_blocked_capability_error_result(
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
                        Ok(outcome) => outcome,
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
                                _ => unreachable!("guarded to RecoverableFailure"),
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
                                call,
                                promoted,
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
        append_blocked_capability_error_result(
            ctx.host,
            &mut state,
            &call,
            &summary,
            model_observation,
        )
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

    async fn fail_unsupported_process_wait(
        &self,
        ctx: StageContext<'_>,
        mut state: LoopExecutionState,
        call: &CapabilityCallCandidate,
        _process_ref: &ironclaw_loop_contracts::LoopProcessRef,
    ) -> Result<OutcomeStep, AgentLoopExecutorError> {
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
        let exit = failed_exit(
            ctx.host,
            state.clone(),
            LoopFailureKind::CapabilityProtocolError,
            None,
            FailedExitDetails {
                safe_summary: None,
                explanation_message_ref,
            },
        )?;
        Ok(OutcomeStep::Exit {
            exit,
            state: Some(Box::new(state)),
        })
    }

    fn cancelled_for_batch_drain(
        &self,
        ctx: StageContext<'_>,
        state: LoopExecutionState,
    ) -> Result<OutcomeStep, AgentLoopExecutorError> {
        self.cancelled_for_batch_drain_with_reason(
            ctx,
            state,
            ironclaw_loop_contracts::LoopCancelledReasonKind::HostCancellation,
        )
    }

    fn cancelled_for_batch_drain_with_reason(
        &self,
        ctx: StageContext<'_>,
        state: LoopExecutionState,
        reason_kind: ironclaw_loop_contracts::LoopCancelledReasonKind,
    ) -> Result<OutcomeStep, AgentLoopExecutorError> {
        // The unified batch drain merges every launched sibling before writing
        // the one authoritative Final checkpoint.
        let exit = cancelled_exit_with_reason(ctx.host, state.clone(), reason_kind, None)?;
        Ok(OutcomeStep::Exit {
            exit,
            state: Some(Box::new(state)),
        })
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
                CapabilityErrorHandling {
                    summary,
                    model_observation,
                    retry_mode: CapabilityRetryMode::Allow,
                },
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

fn clear_matching_pending_approval_resume(
    state: &mut LoopExecutionState,
    call: &CapabilityCallCandidate,
) {
    if state
        .pending_approval_resume
        .as_ref()
        .is_some_and(|resume| resume.activity_id == call.activity_id)
    {
        state.pending_approval_resume = None;
    }
}

fn auth_resume_for_gate(
    gate_ref: &LoopGateRef,
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
            resume.resume_token = Some(prior_approval.resume_token.clone());
            resume.prior_approval.get_or_insert_with(prior_identity);
            auth_resume
        }
        None => Some(CapabilityAuthResume::resolved(
            gate_ref.clone(),
            prior_approval.resume_token.clone(),
            Some(prior_identity()),
        )),
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
/// [`ModelResultPreview`] (`refs.preview`) and its independent continuation
/// metadata. Metadata-only observations are reconstructed when preview safety
/// suppresses the text; `None` is reserved for outcomes with neither preview nor
/// continuation metadata, where `append_capability_result_ref` synthesizes a bare
/// success observation.
fn result_reference_observation_from_outcome(
    outcome: &Outcome,
) -> Option<ModelVisibleToolObservation> {
    let preview = outcome.refs.preview.as_ref();
    let meta = &outcome.refs.preview_meta;
    if preview.is_none() && meta.is_empty() {
        return None;
    }
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
            preview: preview.map(|preview| preview.as_str().to_string()),
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
    diagnostic: &ModelFailureDiagnostic,
    outcome: &Outcome,
) -> CapabilityFailure {
    CapabilityFailure {
        // The verdict already carries the unified kind; no tag round-trip.
        error_kind: *error_kind,
        safe_summary: outcome.summary.as_str().to_string(),
        detail: capability_failure_detail_from(diagnostic),
    }
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

/// What the model should do about a denial, and whether re-issuing the same
/// call could ever work.
///
/// Denials reached the model with `model_observation: None` — no recovery, no
/// retry constraint, no repairs — so a denial meaning *authenticate and this
/// succeeds* was indistinguishable from a permanent block (#6284 item 4).
/// #6781 made the *reason* specific; this makes the reason **actionable**.
///
/// Exhaustive and wildcard-free: a new [`DenyReason`] cannot compile until its
/// next move is chosen.
fn deny_recovery(reason: DenyReason) -> (SameCallRetryConstraint, CapabilityRecoveryHint) {
    match reason {
        // A credential unlocks it — the same call succeeds afterwards.
        DenyReason::UnknownSecret => (
            SameCallRetryConstraint::Forbidden,
            CapabilityRecoveryHint::AuthenticateThenRetry,
        ),
        // A human unlocks it. Worth asking; the same call may then succeed.
        DenyReason::ApprovalDenied => (
            SameCallRetryConstraint::Forbidden,
            CapabilityRecoveryHint::RequestApproval,
        ),
        // A grant is missing. Not the model's to fix by rewording.
        DenyReason::MissingGrant => (
            SameCallRetryConstraint::Forbidden,
            CapabilityRecoveryHint::RequestApproval,
        ),
        // Not this caller's capability; re-calling cannot succeed.
        DenyReason::UnknownCapability => (
            SameCallRetryConstraint::Forbidden,
            CapabilityRecoveryHint::UseDifferentCapability,
        ),
        // The model named a path it can correct.
        DenyReason::InvalidPath | DenyReason::PathOutsideMount => (
            SameCallRetryConstraint::RequiresChangedInput,
            CapabilityRecoveryHint::CorrectArgumentsBeforeRetry,
        ),
        // Capacity, not permission: waiting is the move.
        DenyReason::BudgetDenied | DenyReason::ResourceLimitExceeded => (
            SameCallRetryConstraint::AllowedAfterDelay,
            CapabilityRecoveryHint::WaitThenRetry,
        ),
        // Permanently refused.
        DenyReason::NetworkDenied | DenyReason::PolicyDenied => (
            SameCallRetryConstraint::Forbidden,
            CapabilityRecoveryHint::ReviseApproach,
        ),
        // A host fault, not a refusal. Nothing the model can unlock, but the
        // same call is not forbidden on principle.
        DenyReason::InternalInvariantViolation => (
            SameCallRetryConstraint::NotUseful,
            CapabilityRecoveryHint::RespectFailureConstraint,
        ),
    }
}

/// Fixed, host-authored one-line summary for a denial observation.
///
/// Deliberately not the capability's own text: the summary channel is strictly
/// validated and a denial's `safe_summary` may degrade to a placeholder. The
/// untrusted cause rides `detail` instead, where the lenient validator applies.
fn capability_denied_observation_summary(reason_kind: &str) -> String {
    format!("The capability was denied ({reason_kind}).")
}

/// The denial's own text for the model-visible detail channel. Empty summaries
/// become an actionable host-authored sentence rather than a category-only
/// observation.
fn denial_detail_text(reason_kind: &str, safe_summary: &str) -> String {
    let trimmed = safe_summary.trim();
    if trimmed.is_empty() {
        format!(
            "The host denied this capability with reason {reason_kind}; follow the recovery guidance before choosing the next action."
        )
    } else {
        trimmed.to_string()
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
    gate_ref: &LoopGateRef,
    resume_token: Option<&ResumeToken>,
    prior_approval: Option<&CapabilityApprovalResume>,
) -> Option<CapabilityAuthResume> {
    let base = resume_token
        .and_then(|token| CapabilityResumeToken::new(token.as_str()).ok())
        .map(|resume_token| CapabilityAuthResume::resolved(gate_ref.clone(), resume_token, None));
    auth_resume_for_gate(gate_ref, base, prior_approval)
}

struct ChildResultAppendInput {
    result_ref: LoopResultRef,
    safe_summary: String,
    byte_len: u64,
    model_observation: Option<ModelVisibleToolObservation>,
}

async fn append_spawned_child_result(
    host: &(dyn ironclaw_loop_contracts::AgentLoopDriverHost + Send + Sync),
    state: &mut LoopExecutionState,
    call: &CapabilityCallCandidate,
    input: ChildResultAppendInput,
    capability_batch: &mut CapabilityBatchTurnSummary,
) -> Result<(), AgentLoopExecutorError> {
    // Keep this write seam fail-soft even though `Resolution` normally arrives
    // with a validated `SafeSummary`: a malformed adapter value must degrade the
    // label, not end the run.
    let (safe_summary, _) =
        sanitized_strategy_summary_or_fallback(input.safe_summary, "spawned a child run");
    let safe_summary = safe_summary.into_inner();
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

/// Errored calls are deliberately NOT recorded into `observed_signatures`:
/// that list means COMPLETED calls (its only consumer is the
/// structured-result stop strategy, which completes the run on the result
/// tool's completed signature and aborts on all-failed batches). Recording
/// errors here completed structured runs off a FAILED validation attempt —
/// with no durable result — and masked the all-failed abort.
async fn append_blocked_capability_error_result(
    host: &(dyn ironclaw_loop_contracts::AgentLoopDriverHost + Send + Sync),
    state: &mut LoopExecutionState,
    call: &CapabilityCallCandidate,
    summary: &CapabilityErrorSummary,
    model_observation: Option<ironclaw_loop_contracts::ModelVisibleToolObservation>,
) -> Result<(), AgentLoopExecutorError> {
    append_capability_error_ref(host, state, call, summary, model_observation).await
}

async fn append_completed_capability_result(
    host: &(dyn ironclaw_loop_contracts::AgentLoopDriverHost + Send + Sync),
    state: &mut LoopExecutionState,
    call: &CapabilityCallCandidate,
    result: CapabilityResultMessage,
    capability_batch: &mut CapabilityBatchTurnSummary,
) -> Result<(), AgentLoopExecutorError> {
    append_capability_result_ref(host, call, &result).await?;
    let signature = capability_call_signature(call)?;
    // Repeated output is not terminal evidence. The host-reported progress and
    // digest remain part of the result contract, while loop steering relies on
    // consecutive call signatures and deterministic limits remain the backstop.
    capability_batch.record_result(signature, result.terminate_hint);
    push_completed_result(state, &call.capability_id, result);
    Ok(())
}

/// The gate kind this outcome would stage a BeforeBlock checkpoint for, or
/// `None` when the outcome is not gate-writing.
///
/// Single source of truth for the gate-writing variant set:
/// [`gate_outcome_writes_before_block`] and [`persist_later_gate_outcome`]
/// both derive from this mapping, so a future gate-writing `Resolution`
/// variant can never be added to the predicate without the later-sibling
/// persistence handling it (a divergence would otherwise surface only at
/// runtime as the terminal `PlannerContract` error). `DependentRun` maps to
/// `GateKind::AwaitDependentRun` but is persisted with its concrete result.
fn gate_outcome_kind(resolution: &Resolution) -> Option<GateKind> {
    match resolution {
        Resolution::Blocked(Blocked::Approval(_)) => Some(GateKind::Approval),
        Resolution::Blocked(Blocked::Auth(_)) => Some(GateKind::Auth),
        Resolution::Blocked(Blocked::Resource(_)) => Some(GateKind::Resource),
        Resolution::Suspended(Suspension::ExternalTool(_)) => Some(GateKind::ExternalTool),
        Resolution::Suspended(Suspension::DependentRun { .. }) => Some(GateKind::AwaitDependentRun),
        _ => None,
    }
}

/// Whether handling this outcome through [`handle_capability_outcome`] stages
/// a BeforeBlock checkpoint — the gate-writing outcomes the drain must treat
/// as candidates for the batch's single gate exit.
fn gate_outcome_writes_before_block(resolution: &Resolution) -> bool {
    gate_outcome_kind(resolution).is_some()
}

/// Complete a later sibling gate model-visibly without allocating another
/// resumable gate slot. The first gate in input order owns the batch's single
/// BeforeBlock checkpoint; later calls return a durable "pending" observation
/// so no provider call is left without a result and the model can retry after
/// the first gate resolves. A dependent run already has a concrete result, so
/// preserve that result instead of replacing it with a pending summary.
async fn persist_later_gate_outcome(
    ctx: StageContext<'_>,
    state: &mut LoopExecutionState,
    call: CapabilityCallCandidate,
    resolution: Resolution,
) -> Result<(), AgentLoopExecutorError> {
    // A later sibling may itself be a resumed call. Its new pending/concrete
    // result consumes the prior resume token; only the first gate may remain
    // resumable in the checkpoint this batch returns.
    clear_matching_pending_approval_resume(state, &call);
    clear_matching_pending_auth_resume(state, &call);
    clear_matching_pending_external_tool_resume(state, &call);
    // A dependent run already has a concrete result; persist that result
    // instead of a pending summary.
    if let Resolution::Suspended(Suspension::DependentRun { result, .. }) = &resolution {
        let result = dependent_run_result_message(result)?;
        append_capability_result_ref(ctx.host, &call, &result).await?;
        push_completed_result(state, &call.capability_id, result);
        return Ok(());
    }
    let Some(kind) = gate_outcome_kind(&resolution) else {
        return Err(AgentLoopExecutorError::PlannerContract {
            detail: "persist_later_gate_outcome called for a non-gate outcome",
        });
    };
    append_capability_safe_summary_ref(
        ctx.host,
        state,
        &call,
        gate_tool_result_summary(kind, "pending"),
    )
    .await
}

fn shared_await_dependent_gate(
    calls: &[CapabilityCallCandidate],
    resolutions: &[Resolution],
) -> Option<(
    ironclaw_host_api::turn::LoopGateRef,
    CapabilityCallCandidate,
)> {
    let mut shared_gate: Option<ironclaw_host_api::turn::LoopGateRef> = None;
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
    use ironclaw_host_api::turn::{LoopGateRef, LoopResultRef};
    use ironclaw_loop_contracts::{CapabilityInputRef, CapabilitySurfaceVersion, resolution};

    fn call(input: &str) -> CapabilityCallCandidate {
        let capability_id = ironclaw_host_api::ids::CapabilityId::new("test.cap").unwrap();
        CapabilityCallCandidate {
            activity_id: ironclaw_host_api::turn::CapabilityActivityId::new(),
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
    fn gate_outcome_kind_maps_every_gate_writing_variant() {
        let cases = [
            (
                resolution::approval_required(
                    LoopGateRef::new("gate:kind-approval").unwrap(),
                    "approval".to_string(),
                    None,
                )
                .resolution,
                Some(GateKind::Approval),
            ),
            (
                resolution::auth_required(
                    LoopGateRef::new("gate:kind-auth").unwrap(),
                    Vec::new(),
                    "auth".to_string(),
                    None,
                )
                .resolution,
                Some(GateKind::Auth),
            ),
            (
                resolution::resource_blocked(
                    LoopGateRef::new("gate:kind-resource").unwrap(),
                    "resource".to_string(),
                )
                .resolution,
                Some(GateKind::Resource),
            ),
            (
                resolution::external_tool_pending(
                    LoopGateRef::new("gate:kind-external-tool").unwrap(),
                    "external tool".to_string(),
                )
                .resolution,
                Some(GateKind::ExternalTool),
            ),
            (
                await_dependent("gate:kind-dependent", "r"),
                Some(GateKind::AwaitDependentRun),
            ),
            (completed("r-none"), None),
        ];
        for (outcome, expected) in cases {
            assert_eq!(
                gate_outcome_kind(&outcome),
                expected,
                "gate_outcome_kind must agree with gate_outcome_writes_before_block"
            );
            assert_eq!(
                gate_outcome_writes_before_block(&outcome),
                expected.is_some()
            );
        }
    }

    #[test]
    fn prefixed_capability_summary_does_not_underflow_when_prefix_is_too_long() {
        let prefix = "x".repeat(MAX_SAFE_SUMMARY_BYTES + 1);
        let summary = prefixed_capability_summary(prefix, "detail".to_string());

        // An oversized combination degrades to the fixed fallback instead of
        // becoming a terminal PlannerContract error.
        assert_eq!(summary.as_str(), "the tool failure details were redacted");
    }

    #[test]
    fn prefixed_capability_summary_degrades_marker_bearing_prefix_without_borking() {
        // Regression: `Failed(Authorization)` builds the prefix "capability
        // failed with authorization: ", whose "authorization:" substring is a
        // banned marker — this used to return a terminal PlannerContract error
        // before the model ever saw the tool failure.
        let summary = capability_failed_summary(
            FailureKind::Authorization,
            "the provider token has expired".to_string(),
        );

        assert_eq!(summary.as_str(), "the tool failure details were redacted");
    }

    #[test]
    fn prefixed_capability_summary_rephrases_fixed_input_encode_summary() {
        let summary = prefixed_capability_summary(
            "capability failed with invalid_input: ".to_string(),
            INPUT_ENCODE_HUMAN_SUMMARY.to_string(),
        );

        assert_eq!(
            summary.as_str(),
            "capability failed with invalid_input: input could not be encoded"
        );
    }
}

// arch-exempt: large_file, pre-existing large file minimally touched for the §5.3 Stage 2a-i replay-payload move (field/store wiring + tests), plan #6175
