use async_trait::async_trait;
use ironclaw_turns::{
    LoopExit,
    run_profile::{
        CapabilitySurfaceVersion, LoopModelCapabilityView, LoopModelMessage, LoopProgressEvent,
        VisibleCapabilityRequest, VisibleCapabilitySurface,
    },
};
use tracing::debug;

use crate::state::LoopExecutionState;

use super::{
    AgentLoopExecutorError, CancelCheck, CheckpointStage, ExecutorStage, HostStage,
    PendingInputAck, StageContext, apply_capability_filter,
};

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct PromptStage;

pub(super) struct PromptInput {
    pub(super) state: LoopExecutionState,
    pub(super) pending_input_ack: PendingInputAck,
}

pub(super) struct PromptOutput {
    pub(super) state: LoopExecutionState,
    pub(super) pending_input_ack: PendingInputAck,
    pub(super) surface: VisibleCapabilitySurface,
    pub(super) messages: Vec<ironclaw_turns::run_profile::LoopModelMessage>,
    pub(super) capability_view: LoopModelCapabilityView,
}

pub(super) enum PromptStep {
    Prepared(Box<PromptOutput>),
    Exit(LoopExit),
}

#[async_trait]
impl ExecutorStage<PromptInput> for PromptStage {
    type Output = PromptStep;

    async fn process(
        &self,
        ctx: StageContext<'_>,
        input: PromptInput,
    ) -> Result<PromptStep, AgentLoopExecutorError> {
        let mut state = input.state;
        let mut pending_input_ack = input.pending_input_ack;

        let surface_filter = ctx.planner.capability().filter(&state).await;
        state = match CheckpointStage
            .cancel_if_requested_after_pending_input_ack(ctx, state, &mut pending_input_ack)
            .await?
        {
            CancelCheck::Continue(state) => *state,
            CancelCheck::Exit(exit) => return Ok(PromptStep::Exit(exit)),
        };

        let mut surface = ctx
            .host
            .visible_capabilities(VisibleCapabilityRequest)
            .await
            .map_err(|_| AgentLoopExecutorError::HostUnavailable {
                stage: HostStage::Capability,
            })?;
        apply_capability_filter(&mut surface, &surface_filter);
        if tracing::enabled!(tracing::Level::DEBUG) {
            let visible_capability_sample = surface
                .descriptors
                .iter()
                .take(20)
                .map(|descriptor| descriptor.capability_id.as_str())
                .collect::<Vec<_>>();
            debug!(
                iteration = state.iteration,
                surface_version = %surface.version,
                visible_capability_count = surface.descriptors.len(),
                visible_capability_sample = ?visible_capability_sample,
                "agent loop prompt capability surface prepared"
            );
        }
        let capability_view = LoopModelCapabilityView {
            visible_capability_ids: surface
                .descriptors
                .iter()
                .map(|descriptor| descriptor.capability_id.clone())
                .collect(),
        };
        state.surface_version = Some(surface.version.clone());
        state = match CheckpointStage
            .cancel_if_requested_after_pending_input_ack(ctx, state, &mut pending_input_ack)
            .await?
        {
            CancelCheck::Continue(state) => *state,
            CancelCheck::Exit(exit) => return Ok(PromptStep::Exit(exit)),
        };

        let messages = build_prompt_bundle_for_surface(
            ctx,
            &state,
            Some(surface.version.clone()),
            Some(capability_view.clone()),
        )
        .await?;
        state = match CheckpointStage
            .cancel_if_requested_after_pending_input_ack(ctx, state, &mut pending_input_ack)
            .await?
        {
            CancelCheck::Continue(state) => *state,
            CancelCheck::Exit(exit) => return Ok(PromptStep::Exit(exit)),
        };
        Ok(PromptStep::Prepared(Box::new(PromptOutput {
            state,
            pending_input_ack,
            surface,
            messages,
            capability_view,
        })))
    }
}

pub(super) async fn build_prompt_bundle_for_surface(
    ctx: StageContext<'_>,
    state: &LoopExecutionState,
    surface_version: Option<CapabilitySurfaceVersion>,
    capability_view: Option<LoopModelCapabilityView>,
) -> Result<Vec<LoopModelMessage>, AgentLoopExecutorError> {
    let mut context_request = ctx.planner.context().plan_context_request(state).await;
    context_request.surface_version = surface_version;
    context_request.capability_view = capability_view;
    let prompt_mode = context_request.mode;
    let prompt_bundle = ctx
        .host
        .build_prompt_bundle(context_request)
        .await
        .map_err(|_| AgentLoopExecutorError::HostUnavailable {
            stage: HostStage::Prompt,
        })?;
    CheckpointStage
        .emit_progress(
            ctx,
            LoopProgressEvent::PromptBundleBuilt {
                iteration: state.iteration,
                bundle_ref: prompt_bundle.bundle_ref.clone(),
                mode: prompt_mode,
                surface_version: prompt_bundle.surface_version.clone(),
                message_count: prompt_bundle.messages.len() as u32,
                identity_message_count: prompt_bundle.identity_message_count,
                instruction_snippet_count: prompt_bundle.instruction_snippet_count,
            },
        )
        .await;

    Ok(prompt_bundle.messages)
}
