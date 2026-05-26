use async_trait::async_trait;
use ironclaw_turns::{
    LoopExit, LoopFailureKind,
    run_profile::{
        AgentLoopHostErrorKind, LoopDriverNoteKind, LoopModelCapabilityView, LoopModelRequest,
        LoopProgressEvent, LoopSafeSummary,
    },
};
use tracing::debug;

use crate::{
    state::{CheckpointKind, LoopExecutionState},
    strategies::{ModelErrorSummary, RecoveryOutcome},
};

use super::prompt::build_prompt_bundle_for_surface;
use super::{
    AgentLoopExecutorError, CancelCheck, CheckpointStage, ExecutorStage, HostStage,
    MAX_MODEL_RETRIES, StageContext, failed_exit, honor_retry_alteration, model_error_class,
    model_preference_to_host, sanitized_strategy_summary,
};

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct ModelStage;

pub(super) struct ModelInput {
    pub(super) state: LoopExecutionState,
    pub(super) messages: Vec<ironclaw_turns::run_profile::LoopModelMessage>,
    pub(super) surface_version: ironclaw_turns::run_profile::CapabilitySurfaceVersion,
    pub(super) capability_view: LoopModelCapabilityView,
}

pub(super) enum ModelStep {
    Response(
        Box<LoopExecutionState>,
        ironclaw_turns::run_profile::LoopModelResponse,
    ),
    Exit(LoopExit),
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
        for _ in 0..MAX_MODEL_RETRIES {
            match ctx.host.stream_model(request.clone()).await {
                Ok(response) => {
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
                    let Some(class) = model_error_class(&error) else {
                        return Err(AgentLoopExecutorError::HostUnavailableWithDiagnostics {
                            stage: HostStage::Model,
                            kind: error.kind,
                            safe_summary: LoopSafeSummary::new(error.safe_summary)
                                .unwrap_or_else(|_| LoopSafeSummary::model_gateway_failed()),
                            diagnostic_ref: error.diagnostic_ref,
                        });
                    };
                    if !recorded_failure {
                        state.recent_failure_kinds.push(LoopFailureKind::ModelError);
                        recorded_failure = true;
                    }
                    let summary = ModelErrorSummary {
                        class,
                        safe_summary: sanitized_strategy_summary(error.safe_summary)?,
                        diagnostic_ref: error.diagnostic_ref,
                    };
                    match ctx
                        .planner
                        .recovery()
                        .on_model_error(&state, &summary)
                        .await
                    {
                        RecoveryOutcome::Retry {
                            recovery, alter, ..
                        } => {
                            state.recovery_state = recovery;
                            match CheckpointStage.cancel_if_requested(ctx, state).await? {
                                CancelCheck::Continue(next) => state = *next,
                                CancelCheck::Exit(exit) => return Ok(ModelStep::Exit(exit)),
                            }
                            honor_retry_alteration(alter.as_ref())?;
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
                            let messages = build_prompt_bundle_for_surface(
                                ctx,
                                &state,
                                surface_version.clone(),
                                capability_view.clone(),
                            )
                            .await?;
                            match CheckpointStage.cancel_if_requested(ctx, state).await? {
                                CancelCheck::Continue(next) => state = *next,
                                CancelCheck::Exit(exit) => return Ok(ModelStep::Exit(exit)),
                            }
                            request.messages = messages;
                        }
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
                            match CheckpointStage.cancel_if_requested(ctx, state).await? {
                                CancelCheck::Continue(next) => state = *next,
                                CancelCheck::Exit(exit) => return Ok(ModelStep::Exit(exit)),
                            }
                            let checked = CheckpointStage
                                .write(ctx, state, CheckpointKind::Final)
                                .await?;
                            return Ok(ModelStep::Exit(failed_exit(
                                ctx.host,
                                checked.state,
                                failure_kind,
                                Some(checked.checkpoint_id),
                            )?));
                        }
                    }
                }
            }
        }

        let checked = CheckpointStage
            .write(ctx, state, CheckpointKind::Final)
            .await?;
        Ok(ModelStep::Exit(failed_exit(
            ctx.host,
            checked.state,
            LoopFailureKind::DriverBug,
            Some(checked.checkpoint_id),
        )?))
    }
}
