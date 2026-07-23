use async_trait::async_trait;
use ironclaw_turns::{
    LoopBlocked, LoopBlockedKind, LoopExit, LoopFailureKind,
    run_profile::{
        AgentLoopHostErrorKind, LoopDriverNoteKind, LoopModelCapabilityView, LoopModelRequest,
        LoopProgressEvent, LoopSafeSummary,
    },
};
use tracing::debug;

use crate::{
    state::{CheckpointKind, LoopExecutionState, PendingModelRetryDirective},
    strategies::{
        GateKind, ModelErrorSummary, RecoveryOutcome, RetryAlteration, RetryScope,
        model_error_to_failure_kind,
    },
};

use super::prompt::build_prompt_bundle_for_surface;
use super::{
    AgentLoopExecutorError, CancelCheck, CheckpointStage, ExecutorStage, FailedExitDetails,
    HostStage, StageContext, exit_id, failed_exit, honor_retry_alteration, loop_gate_kind,
    model_error_class, model_error_failure_summary, model_preference_to_host,
    sanitized_strategy_summary_or_fallback,
};

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct ModelStage;

pub(super) struct ModelInput {
    pub(super) state: LoopExecutionState,
    pub(super) messages: Vec<ironclaw_turns::run_profile::LoopModelMessage>,
    pub(super) inline_messages: Vec<ironclaw_turns::run_profile::LoopInlineMessage>,
    pub(super) surface_version: ironclaw_turns::run_profile::CapabilitySurfaceVersion,
    pub(super) capability_view: LoopModelCapabilityView,
}

pub(super) enum ModelStep {
    Response(
        Box<LoopExecutionState>,
        ironclaw_turns::run_profile::LoopModelResponse,
    ),
    RetryIteration(Box<LoopExecutionState>),
    Exit(LoopExit),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ModelRetryAction {
    RetryCall,
    RetryIteration,
}

#[async_trait]
impl ExecutorStage<ModelInput> for ModelStage {
    type Output = ModelStep;

    async fn process(
        &self,
        ctx: StageContext<'_>,
        input: ModelInput,
    ) -> Result<ModelStep, AgentLoopExecutorError> {
        let mut state = input.state;
        state = match CheckpointStage.cancel_if_requested(ctx, state).await? {
            CancelCheck::Continue(state) => *state,
            CancelCheck::Exit(exit) => return Ok(ModelStep::Exit(exit)),
        };

        let model_preference =
            model_preference_to_host(ctx.planner.model().preference(&state).await)?;
        state = match CheckpointStage.cancel_if_requested(ctx, state).await? {
            CancelCheck::Continue(state) => *state,
            CancelCheck::Exit(exit) => return Ok(ModelStep::Exit(exit)),
        };
        let surface_version = input.surface_version;
        let capability_view = input.capability_view;
        let mut request = LoopModelRequest {
            messages: input.messages,
            inline_messages: input.inline_messages,
            surface_version: Some(surface_version.clone()),
            model_preference,
            capability_view: Some(capability_view.clone()),
        };
        let visible_capability_count = capability_view.visible_capability_ids.len();
        debug!(
            iteration = state.iteration,
            surface_version = surface_version.as_str(),
            visible_capability_count,
            model_preference = request
                .model_preference
                .as_ref()
                .map(|profile| profile.as_str())
                .unwrap_or("<none>"),
            message_count = request.messages.len(),
            "agent loop model request prepared"
        );

        let mut recorded_failure = false;
        // The retry guard is derived from the composed recovery strategy so
        // every accepted retry budget can reach the strategy's own Abort
        // (failure kind + diagnostics). The fall-through below only fires on
        // a strategy contract bug: retrying past its declared ceiling.
        let max_model_attempts = ctx.planner.recovery().max_total_model_attempts().max(1);
        let mut last_error_summary: Option<ModelErrorSummary> = None;
        let mut last_error_detail: Option<String> = None;
        for _ in 0..max_model_attempts {
            let model_result = ctx.host.stream_model(request.clone()).await;
            match model_result {
                Ok(response) => {
                    // A successful response proves the provider saw this
                    // request. Consume the pending controls only now; a
                    // gate-shaped error below happens before provider dispatch
                    // and must preserve them for the approved retry.
                    state.pending_model_error_observation = None;
                    state.pending_model_retry_directive = None;
                    state.terminal_warning_state.mark_delivered();
                    match &response.output {
                        ironclaw_turns::run_profile::ParentLoopOutput::AssistantReply(reply) => {
                            debug!(
                                iteration = state.iteration,
                                response_kind = "assistant_reply",
                                content_bytes = reply.content.len(),
                                "agent loop model response classified"
                            );
                        }
                        ironclaw_turns::run_profile::ParentLoopOutput::CapabilityCalls(calls) => {
                            debug!(
                                iteration = state.iteration,
                                response_kind = "capability_calls",
                                capability_call_count = calls.len(),
                                "agent loop model response classified"
                            );
                        }
                    }
                    state.recovery_state = state.recovery_state.cleared_attempts();
                    return Ok(ModelStep::Response(Box::new(state), response));
                }
                Err(error) => {
                    if error.kind == AgentLoopHostErrorKind::Cancelled {
                        return Err(AgentLoopExecutorError::Cancelled);
                    }
                    if error.kind == AgentLoopHostErrorKind::BudgetApprovalRequired
                        && let Some(gate_ref) = error.gate_ref.clone()
                    {
                        return budget_approval_blocked_exit(ctx, state, gate_ref).await;
                    }
                    // Non-gate errors were returned after the request crossed
                    // the model boundary. Do not leave stale model-error
                    // controls pending in a later prompt. A terminal warning,
                    // however, remains pending until a successful response
                    // proves the model received its recovery turn.
                    state.pending_model_error_observation = None;
                    state.pending_model_retry_directive = None;
                    let Some(class) = model_error_class(&error) else {
                        let raw_summary = error.safe_summary;
                        let (safe_summary, rejected_summary_detail) =
                            match LoopSafeSummary::new(raw_summary.clone()) {
                                Ok(summary) => (summary, None),
                                Err(validation_error) => {
                                    debug!(
                                        validation_error = %validation_error,
                                        "model host error summary rejected; using fallback"
                                    );
                                    (
                                    LoopSafeSummary::model_gateway_failed(),
                                    Some(ironclaw_turns::run_profile::sanitize_model_visible_text(
                                        raw_summary,
                                    )),
                                )
                                }
                            };
                        let detail = error.detail.or(rejected_summary_detail);
                        return Err(AgentLoopExecutorError::HostUnavailableWithDiagnostics {
                            stage: HostStage::Model,
                            kind: error.kind,
                            safe_summary,
                            reason_kind: error.reason_kind,
                            diagnostic_ref: error.diagnostic_ref,
                            detail,
                        });
                    };
                    if !recorded_failure {
                        state
                            .recent_failure_kinds
                            .push(model_error_to_failure_kind(class));
                        recorded_failure = true;
                    }
                    let upstream_detail = error.detail;
                    let (safe_summary, rejected_summary_detail) =
                        sanitized_strategy_summary_or_fallback(
                            error.safe_summary,
                            "model gateway failed",
                        );
                    let model_failure_detail = upstream_detail.or(rejected_summary_detail);
                    let summary = ModelErrorSummary {
                        class,
                        safe_summary,
                        diagnostic_ref: error.diagnostic_ref,
                    };
                    last_error_summary = Some(summary.clone());
                    last_error_detail.clone_from(&model_failure_detail);
                    let recovery_outcome = ctx
                        .planner
                        .recovery()
                        .on_model_error(&state, &summary)
                        .await;
                    let (recovery, scope, alter, observation) = match recovery_outcome {
                        RecoveryOutcome::Retry {
                            recovery,
                            scope,
                            alter,
                        } => (recovery, scope, alter, None),
                        RecoveryOutcome::ModelErrorObservation {
                            recovery,
                            scope,
                            alter,
                            observation,
                        } => (recovery, scope, alter, Some(observation)),
                        RecoveryOutcome::ToolErrorResult { .. } => {
                            return Err(AgentLoopExecutorError::PlannerContract {
                                detail: "ToolErrorResult on model error",
                            });
                        }
                        RecoveryOutcome::Abort {
                            recovery,
                            failure_kind,
                        } => {
                            state.recovery_state = recovery;
                            state.recent_failure_kinds.push(failure_kind);
                            match CheckpointStage.cancel_if_requested(ctx, state).await? {
                                CancelCheck::Continue(next) => state = *next,
                                CancelCheck::Exit(exit) => return Ok(ModelStep::Exit(exit)),
                            }
                            let checked = CheckpointStage
                                .write(ctx, state, CheckpointKind::Final)
                                .await?;
                            let mut safe_failure = model_error_failure_summary(&summary)?;
                            if let Some(detail) = model_failure_detail {
                                safe_failure = safe_failure.with_detail(detail);
                            }
                            return Ok(ModelStep::Exit(failed_exit(
                                ctx.host,
                                checked.state,
                                failure_kind,
                                Some(checked.checkpoint_id),
                                FailedExitDetails {
                                    diagnostic_ref: summary.diagnostic_ref.clone(),
                                    safe_summary: Some(safe_failure),
                                    explanation_message_ref: None,
                                },
                            )?));
                        }
                    };
                    state.recovery_state = recovery;
                    state.pending_model_error_observation = observation;
                    match CheckpointStage.cancel_if_requested(ctx, state).await? {
                        CancelCheck::Continue(next) => state = *next,
                        CancelCheck::Exit(exit) => return Ok(ModelStep::Exit(exit)),
                    }
                    let retry_action =
                        prepare_model_retry_alteration(&mut state, scope, alter.as_ref())?;
                    // Persist the consumed retry/observation budget before the
                    // next model attempt. Otherwise a worker restart reloads
                    // the pre-error BeforeModel checkpoint and grants the
                    // same recovery attempt again. For iteration retries this
                    // also makes the compaction request and pending
                    // observation survive the restart window.
                    state = CheckpointStage
                        .write(ctx, state, CheckpointKind::BeforeModel)
                        .await?
                        .state;
                    wait_for_model_retry_backoff(ctx, alter.as_ref()).await;
                    // A cancel request can wake the backoff sleep early;
                    // observe it here so cancellation exits at this boundary
                    // instead of issuing another call.
                    match CheckpointStage.cancel_if_requested(ctx, state).await? {
                        CancelCheck::Continue(next) => state = *next,
                        CancelCheck::Exit(exit) => return Ok(ModelStep::Exit(exit)),
                    }
                    CheckpointStage
                        .emit_progress(
                            ctx,
                            LoopProgressEvent::driver_note(
                                LoopDriverNoteKind::Retrying,
                                "retrying model request",
                            )
                            .map_err(|_| {
                                AgentLoopExecutorError::PlannerContract {
                                    detail: "retry progress summary was invalid",
                                }
                            })?,
                        )
                        .await;
                    if retry_action == ModelRetryAction::RetryIteration {
                        return Ok(ModelStep::RetryIteration(Box::new(state)));
                    }
                    let bundle = build_prompt_bundle_for_surface(
                        ctx,
                        &state,
                        surface_version.clone(),
                        capability_view.clone(),
                    )
                    .await?;
                    request.inline_messages = bundle.inline_messages();
                    match CheckpointStage.cancel_if_requested(ctx, state).await? {
                        CancelCheck::Continue(next) => state = *next,
                        CancelCheck::Exit(exit) => return Ok(ModelStep::Exit(exit)),
                    }
                    request.messages = bundle.into_model_messages(&mut state);
                }
            }
        }

        // Contract-bug fall-through: the strategy kept returning Retry past
        // its own declared attempt ceiling. Still surface the last observed
        // model error's diagnostics rather than a bare generic failure.
        state.recent_failure_kinds.push(LoopFailureKind::ModelError);
        let details = match &last_error_summary {
            Some(summary) => {
                let mut safe_failure = model_error_failure_summary(summary)?;
                if let Some(detail) = last_error_detail {
                    safe_failure = safe_failure.with_detail(detail);
                }
                FailedExitDetails {
                    diagnostic_ref: summary.diagnostic_ref.clone(),
                    safe_summary: Some(safe_failure),
                    explanation_message_ref: None,
                }
            }
            None => FailedExitDetails::default(),
        };
        let checked = CheckpointStage
            .write(ctx, state, CheckpointKind::Final)
            .await?;
        Ok(ModelStep::Exit(failed_exit(
            ctx.host,
            checked.state,
            LoopFailureKind::ModelError,
            Some(checked.checkpoint_id),
            details,
        )?))
    }
}

async fn budget_approval_blocked_exit(
    ctx: StageContext<'_>,
    mut state: LoopExecutionState,
    gate_ref: ironclaw_turns::LoopGateRef,
) -> Result<ModelStep, AgentLoopExecutorError> {
    state.last_gate = Some(gate_ref.clone());
    state = match CheckpointStage.cancel_if_requested(ctx, state).await? {
        CancelCheck::Continue(state) => *state,
        CancelCheck::Exit(exit) => return Ok(ModelStep::Exit(exit)),
    };
    CheckpointStage
        .emit_progress(
            ctx,
            LoopProgressEvent::GateBlocked {
                iteration: state.iteration,
                gate_kind: loop_gate_kind(GateKind::Resource),
            },
        )
        .await;
    let checked = CheckpointStage
        .write_before_block(ctx, state, &gate_ref)
        .await?;
    Ok(ModelStep::Exit(LoopExit::Blocked(LoopBlocked {
        kind: LoopBlockedKind::Resource,
        gate_ref,
        blocked_activity_id: None,
        credential_requirements: Vec::new(),
        checkpoint_id: checked.checkpoint_id,
        state_ref: checked.state_ref,
        exit_id: exit_id(ctx.host, "blocked")?,
    })))
}

fn prepare_model_retry_alteration(
    state: &mut LoopExecutionState,
    scope: RetryScope,
    alteration: Option<&RetryAlteration>,
) -> Result<ModelRetryAction, AgentLoopExecutorError> {
    honor_retry_alteration(alteration)?;
    state.pending_model_retry_directive = None;
    match alteration {
        Some(RetryAlteration::Backoff { .. }) => {}
        Some(RetryAlteration::ShrinkContext) => {
            if scope != RetryScope::Iteration {
                return Err(AgentLoopExecutorError::PlannerContract {
                    detail: "context shrink retry requires iteration scope",
                });
            }
            state.compaction_state.force_compact_on_next_iteration = true;
            return Ok(ModelRetryAction::RetryIteration);
        }
        Some(RetryAlteration::RepairInvalidModelOutput) => {
            if scope != RetryScope::Call {
                return Err(AgentLoopExecutorError::PlannerContract {
                    detail: "invalid model output repair retry requires call scope",
                });
            }
            state.pending_model_retry_directive =
                Some(PendingModelRetryDirective::RepairInvalidOutput);
        }
        Some(RetryAlteration::AdvanceFallback) | None => {}
    }

    Ok(match scope {
        RetryScope::Call => ModelRetryAction::RetryCall,
        RetryScope::Iteration => ModelRetryAction::RetryIteration,
    })
}

async fn wait_for_model_retry_backoff(ctx: StageContext<'_>, alteration: Option<&RetryAlteration>) {
    let Some(RetryAlteration::Backoff { delay_ms }) = alteration else {
        return;
    };
    // Availability backoffs run up to 60s. The retry transition is already
    // durable at this point, and cancellation wakes the delay so the caller's
    // next boundary check can produce the cancelled exit promptly.
    let sleep = tokio::time::sleep(std::time::Duration::from_millis(delay_ms.as_u64()));
    tokio::pin!(sleep);
    let cancellation = ctx.host.cancellation_requested();
    tokio::pin!(cancellation);
    tokio::select! {
        () = &mut sleep => {}
        _signal = &mut cancellation => {}
    }
}
