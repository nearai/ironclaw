use std::{collections::HashSet, ops::ControlFlow};

use async_trait::async_trait;
use ironclaw_host_api::{
    resolution::{Resolution, Suspension, ToolVerdict},
    result_meta::{FailureKind, LoopRef},
};
use ironclaw_loop_contracts::{
    BatchPolicyKind, CapabilityCallCandidate, LoopExit, LoopFailureKind, LoopProgressEvent,
    VisibleCapabilitySurface,
};

use super::capability_batch::{
    InvokedCapabilityBatch, InvokedCapabilityBatchError, InvokedCapabilityOutcome,
};
use super::capability_dispatch::{CapabilityErrorHandling, CapabilityRetryMode};
use super::capability_failure::{capability_port_error_observation, capability_port_error_summary};
use super::capability_helpers::capability_call_signature;
use super::capability_outcomes::{
    SelectedParallelTerminal, append_completed_capability_result, append_spawned_child_result,
    finish_selected_parallel_terminal, persist_later_gate_outcome, shared_await_dependent_gate,
};
use super::capability_records::{
    capability_result_from_outcome, child_result_from_outcome, dependent_run_result_message,
};
use super::gates::{gate_outcome_kind, gate_outcome_writes_before_block};
use super::{
    AgentLoopExecutorError, BatchStep, CancelCheck, CapabilitySurfaceIndex, CheckpointStage,
    ExecutorStage, GateInput, GateStage, StageContext, TurnCompletedStep, capability_batch_counts,
    capability_host_error, capability_invocation_from_auth_resume_candidate,
    capability_invocation_from_candidate, capability_is_visible, capability_port_error_is_terminal,
    clear_matching_pending_approval_resume, clear_matching_pending_auth_resume,
    clear_matching_pending_external_tool_resume,
};
use crate::{
    state::{CheckpointKind, InvocationCharge, LoopExecutionState},
    strategies::{
        CapabilityBatchTurnSummary, CapabilityErrorSummary, GateKind, SanitizedStrategySummary,
        capability_error_to_failure_kind,
    },
};

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct CapabilityStage;

pub(super) enum OutcomeStep {
    Continue(Box<LoopExecutionState>),
    Exit {
        exit: LoopExit,
        state: Option<Box<LoopExecutionState>>,
    },
}

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
            // This call was rejected locally by the filtered capability
            // surface, so build its signature for error handling without
            // recording it as an executed call.
            let signature = capability_call_signature(&call)?;
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
                signature,
                CapabilityErrorHandling {
                    summary,
                    model_observation: None,
                    retry_mode: CapabilityRetryMode::Allow,
                },
                &mut signatures,
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
                // The over-budget suffix is never dispatched to the host, so
                // its signature must not enter the executed-call ring.
                let signature = capability_call_signature(&call)?;
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
                    signature,
                    CapabilityErrorHandling {
                        summary,
                        model_observation: None,
                        // Never dispatch a call this batch already decided is
                        // over budget, even if the recovery strategy would
                        // otherwise retry it.
                        retry_mode: CapabilityRetryMode::Suppress,
                    },
                    &mut signatures,
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
        let policy = BatchPolicyKind::Parallel;

        capability_batch = CapabilityBatchTurnSummary::for_invocation_count(visible_calls.len());
        // Budget accounting: reserve the admitted launch window above, then
        // settle it against the authoritative launched count below.

        CheckpointStage
            .emit_progress(
                ctx,
                LoopProgressEvent::CapabilityBatchStarted {
                    iteration: state.iteration,
                    call_count: visible_calls.len() as u32,
                    policy,
                },
            )
            .await;

        let visible_call_signatures = visible_calls
            .iter()
            .map(capability_call_signature)
            .collect::<Result<Vec<_>, _>>()?;

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
                for (call, signature) in visible_calls.into_iter().zip(visible_call_signatures) {
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
                        signature,
                        CapabilityErrorHandling {
                            summary,
                            model_observation: None,
                            retry_mode: CapabilityRetryMode::Allow,
                        },
                        &mut signatures,
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
                for (call, signature) in visible_calls.into_iter().zip(visible_call_signatures) {
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
                        signature,
                        CapabilityErrorHandling {
                            summary: summary.clone(),
                            model_observation: Some(observation.clone()),
                            retry_mode: CapabilityRetryMode::Allow,
                        },
                        &mut signatures,
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
            .zip(visible_call_signatures)
            .zip(outcomes)
            .enumerate()
            .map(|(index, ((call, signature), outcome))| (index, call, signature, outcome))
            .collect::<Vec<_>>();

        // Only outcomes returned by the host represent launched calls. Keep
        // canonical signatures for the unlaunched suffix out of the recent
        // call ring while preserving the one-computation-per-call fast path.
        for (_, _, signature, _) in &indexed_outcomes {
            if signatures.insert(signature.clone()) {
                state.recent_call_signatures.push(signature.clone());
            }
        }

        // Durable successful work is recorded before any gate or terminal
        // outcome can return. Dependent-run siblings sharing one gate are
        // likewise materialized here, then coalesced into one gate below.
        let mut pending_outcomes = Vec::new();
        let mut coalesced_gate_index = None;
        for (index, call, signature, outcome) in indexed_outcomes {
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
                        signature,
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
                        signature,
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
                        signature,
                        result,
                        &mut capability_batch,
                    )
                    .await?;
                }
                other => pending_outcomes.push((index, call, signature, other)),
            }
        }

        // One gate can own the batch's resumable checkpoint. Defer the first
        // gate in input order until every launched sibling is durably drained;
        // later gates become explicit model-visible pending outcomes.
        let mut first_gate = None;
        let mut sibling_outcomes = Vec::with_capacity(pending_outcomes.len());
        for item in pending_outcomes {
            let is_gate = matches!(
                &item.3,
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
            .and_then(|(_, _, _, outcome)| match outcome {
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
            && let Some((_, call, _, InvokedCapabilityOutcome::Resolution(resolution))) =
                first_gate.take()
        {
            persist_later_gate_outcome(ctx, &mut state, call, resolution).await?;
        }
        // If cancellation was already requested while the concurrent window
        // was in flight, make the deferred gate model-visible before any
        // sibling handler observes that signal and returns its Final exit.
        if ctx.host.observe_cancellation().is_some()
            && let Some((_, call, _, InvokedCapabilityOutcome::Resolution(resolution))) =
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
        for (index, call, signature, outcome) in sibling_outcomes {
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
                            (call, signature),
                            resolution,
                            &mut signatures,
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
            first_signature,
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
                        (first_call, first_signature),
                        first_gate_outcome,
                        &mut signatures,
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

#[cfg(test)]
mod tests;
