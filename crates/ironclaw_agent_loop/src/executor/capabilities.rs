use std::collections::HashSet;

use async_trait::async_trait;
use ironclaw_turns::{
    LoopFailureKind,
    run_profile::{
        CapabilityBatchInvocation, CapabilityCallCandidate, CapabilityFailureKind,
        CapabilityOutcome, LoopDriverNoteKind, LoopProgressEvent, VisibleCapabilitySurface,
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
    AgentLoopExecutorError, BatchStep, CancelCheck, CapabilitySurfaceIndex, CheckpointStage,
    ExecutorStage, GateInput, GateStage, MAX_CAPABILITY_RETRIES, StageContext, TurnCompletedStep,
    append_capability_error_ref, append_capability_result_ref, append_capability_safe_summary_ref,
    batch_policy_kind, cancelled_exit, capability_batch_counts, capability_error_class,
    capability_failure_kind, capability_host_error, capability_invocation_from_candidate,
    capability_is_visible, capability_summary, failed_exit, honor_retry_alteration,
    push_call_signature_once, push_completed_result, sanitized_strategy_summary,
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
        if !batch.stopped_on_suspension {
            // Non-suspended batches record completed outcomes before handling
            // possible gates/errors so partial parallel progress is durable in
            // any later suspension checkpoint. Keep this in sync with the
            // Completed arm in handle_capability_outcome.
            let mut pending_outcomes = Vec::new();
            for (call, outcome) in visible_calls.into_iter().zip(outcomes) {
                if let CapabilityOutcome::Completed(result) = outcome {
                    push_call_signature_once(&mut state, &mut signatures, &call)?;
                    append_capability_result_ref(ctx.host, &call, &result).await?;
                    push_completed_result(&mut state, result);
                } else {
                    pending_outcomes.push((call, outcome));
                }
            }
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
