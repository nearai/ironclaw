use std::collections::HashSet;

use async_trait::async_trait;
use ironclaw_turns::{
    LoopFailureKind, LoopResultRef,
    run_profile::{
        CapabilityBatchInvocation, CapabilityCallCandidate, CapabilityFailureKind,
        CapabilityOutcome, CapabilityResultMessage, LoopDriverNoteKind, LoopProgressEvent,
        VisibleCapabilitySurface,
    },
};

use crate::{
    state::{CheckpointKind, LoopExecutionState},
    strategies::{
        BatchPolicy, CapabilityErrorClass, CapabilityErrorSummary, GateKind, RecoveryOutcome,
        SanitizedStrategySummary, TurnSummary,
    },
};

use super::{
    AgentLoopExecutorError, AwaitDependentRunGateInput, AwaitDependentRunGateStage, BatchStep,
    CancelCheck, CapabilitySurfaceIndex, CheckpointStage, ExecutorStage, GateInput, GateStage,
    MAX_CAPABILITY_RETRIES, StageContext, TurnCompletedStep, append_capability_error_ref,
    append_capability_result_ref, append_capability_safe_summary_ref, batch_policy_kind,
    cancelled_exit, capability_batch_counts, capability_error_class, capability_failure_kind,
    capability_host_error, capability_invocation_from_candidate, capability_is_visible,
    capability_summary, failed_exit, honor_retry_alteration, push_call_signature_once,
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
        let surface = &input.surface;
        let surface_index = CapabilitySurfaceIndex::new(surface);
        let calls = input.calls;
        state.stop_state.last_batch_total = 0;
        state.stop_state.terminate_hints_in_last_batch = 0;

        let mut visible_calls = Vec::new();
        let mut denied_calls = Vec::new();
        for call in calls {
            if capability_is_visible(&surface_index, &call) {
                visible_calls.push(call);
                continue;
            }

            denied_calls.push(call);
        }

        let summaries = visible_calls
            .iter()
            .map(|call| capability_summary(&surface_index, call))
            .collect::<Vec<_>>();
        let policy = ctx.planner.batch().policy(&state, &summaries);
        let stop_on_first_suspension = matches!(policy, BatchPolicy::Sequential);
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
                .handle_capability_error(ctx, state, call, summary)
                .await?
            {
                BatchStep::Continue(next) => state = *next,
                BatchStep::Exit(exit) => return Ok(TurnCompletedStep::Exit(exit)),
            }
        }

        state.stop_state.last_batch_total = visible_calls.len() as u32;
        if visible_calls.is_empty() {
            return self.completed_turn(ctx, state, result_refs_start).await;
        }

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

        let batch = ctx
            .host
            .invoke_capability_batch(CapabilityBatchInvocation {
                invocations: visible_calls
                    .iter()
                    .cloned()
                    .map(capability_invocation_from_candidate)
                    .collect(),
                stop_on_first_suspension,
            })
            .await
            .map_err(capability_host_error)?;

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
                        append_capability_result_ref(ctx.host, &call, &result).await?;
                        push_completed_result(&mut state, result);
                    }
                    CapabilityOutcome::SpawnedChildRun {
                        result_ref,
                        safe_summary,
                        ..
                    } => {
                        push_call_signature_once(&mut state, &mut signatures, &call)?;
                        append_spawned_child_result(
                            ctx.host,
                            &mut state,
                            &call,
                            result_ref,
                            safe_summary,
                        )
                        .await?;
                    }
                    CapabilityOutcome::AwaitDependentRun {
                        gate_ref,
                        result_ref,
                        safe_summary,
                    } if coalesced_gate_step
                        .as_ref()
                        .is_some_and(|(gate, _)| gate == &gate_ref) =>
                    {
                        push_call_signature_once(&mut state, &mut signatures, &call)?;
                        let result = CapabilityResultMessage {
                            result_ref,
                            safe_summary,
                            terminate_hint: false,
                        };
                        append_capability_result_ref(ctx.host, &call, &result).await?;
                        push_completed_result(&mut state, result);
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
                    .handle_capability_outcome(ctx, state, call, outcome)
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
                        },
                    )
                    .await?
                {
                    BatchStep::Continue(next) => {
                        return self.completed_turn(ctx, *next, result_refs_start).await;
                    }
                    BatchStep::Exit(exit) => return Ok(TurnCompletedStep::Exit(exit)),
                }
            }
        } else {
            for (call, outcome) in visible_calls.into_iter().zip(outcomes) {
                push_call_signature_once(&mut state, &mut signatures, &call)?;
                match self
                    .handle_capability_outcome(ctx, state, call, outcome)
                    .await?
                {
                    BatchStep::Continue(next) => {
                        state = *next;
                    }
                    BatchStep::Exit(exit) => return Ok(TurnCompletedStep::Exit(exit)),
                }
            }
        }

        self.completed_turn(ctx, state, result_refs_start).await
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
    let detail = truncate_summary_detail(detail.as_str(), MAX_SAFE_SUMMARY_BYTES - prefix.len());
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
    ) -> Result<TurnCompletedStep, AgentLoopExecutorError> {
        let state = match CheckpointStage.cancel_if_requested(ctx, state).await? {
            CancelCheck::Continue(state) => *state,
            CancelCheck::Exit(exit) => return Ok(TurnCompletedStep::Exit(exit)),
        };
        let summary =
            TurnSummary::after_capability_batch(state.result_refs[result_refs_start..].to_vec());
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
    ) -> Result<BatchStep, AgentLoopExecutorError> {
        match outcome {
            CapabilityOutcome::Completed(result) => {
                append_capability_result_ref(ctx.host, &call, &result).await?;
                push_completed_result(&mut state, result);
                Ok(BatchStep::Continue(Box::new(state)))
            }
            CapabilityOutcome::SpawnedChildRun {
                result_ref,
                safe_summary,
                ..
            } => {
                append_spawned_child_result(ctx.host, &mut state, &call, result_ref, safe_summary)
                    .await?;
                Ok(BatchStep::Continue(Box::new(state)))
            }
            CapabilityOutcome::ApprovalRequired { gate_ref, .. } => {
                GateStage
                    .process(
                        ctx,
                        GateInput {
                            state,
                            call,
                            kind: GateKind::Approval,
                            gate_ref,
                        },
                    )
                    .await
            }
            CapabilityOutcome::AuthRequired { gate_ref, .. } => {
                GateStage
                    .process(
                        ctx,
                        GateInput {
                            state,
                            call,
                            kind: GateKind::Auth,
                            gate_ref,
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
                        },
                    )
                    .await
            }
            CapabilityOutcome::AwaitDependentRun {
                gate_ref,
                result_ref,
                safe_summary,
            } => {
                let resolved_result = CapabilityResultMessage {
                    result_ref,
                    safe_summary,
                    terminate_hint: false,
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
                self.handle_capability_error(ctx, state, call, summary)
                    .await
            }
            CapabilityOutcome::Failed(failure) => {
                if failure.error_kind == CapabilityFailureKind::Cancelled {
                    return self.cancelled_after_checkpoint(ctx, state).await;
                }
                state
                    .recent_failure_kinds
                    .push(capability_failure_kind(&failure.error_kind));
                let summary = CapabilityErrorSummary {
                    class: capability_error_class(&failure.error_kind),
                    safe_summary: capability_failed_summary(
                        &failure.error_kind,
                        failure.safe_summary,
                    )?,
                    diagnostic_ref: None,
                };
                self.handle_capability_error(ctx, state, call, summary)
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
    ) -> Result<BatchStep, AgentLoopExecutorError> {
        for _ in 0..MAX_CAPABILITY_RETRIES {
            match ctx
                .planner
                .recovery()
                .on_capability_error(&state, &summary)
                .await
            {
                RecoveryOutcome::ToolErrorResult { recovery } => {
                    state.recovery_state = recovery;
                    append_capability_error_ref(ctx.host, &mut state, &call, &summary).await?;
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
                    append_capability_error_ref(ctx.host, &mut state, &call, &summary).await?;
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
                    let retry = ctx
                        .host
                        .invoke_capability(capability_invocation_from_candidate(call.clone()))
                        .await
                        .map_err(capability_host_error)?;
                    match retry {
                        CapabilityOutcome::Failed(failure) => {
                            if failure.error_kind == CapabilityFailureKind::Cancelled {
                                return self.cancelled_after_checkpoint(ctx, state).await;
                            }
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
                            return Box::pin(
                                self.handle_capability_outcome(ctx, state, call, promoted),
                            )
                            .await;
                        }
                    }
                }
            }
        }

        append_capability_error_ref(ctx.host, &mut state, &call, &summary).await?;
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
}

async fn append_spawned_child_result(
    host: &(dyn ironclaw_turns::run_profile::AgentLoopDriverHost + Send + Sync),
    state: &mut LoopExecutionState,
    call: &CapabilityCallCandidate,
    result_ref: LoopResultRef,
    safe_summary: String,
) -> Result<(), AgentLoopExecutorError> {
    let safe_summary = sanitized_strategy_summary(safe_summary)?.into_inner();
    let result = CapabilityResultMessage {
        result_ref,
        safe_summary,
        terminate_hint: false,
    };
    append_capability_result_ref(host, call, &result).await?;
    push_completed_result(state, result);
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
        }
    }

    fn completed(result: &str) -> CapabilityOutcome {
        CapabilityOutcome::Completed(CapabilityResultMessage {
            result_ref: LoopResultRef::new(format!("result:{result}")).unwrap(),
            safe_summary: "summary".to_string(),
            terminate_hint: false,
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
}
