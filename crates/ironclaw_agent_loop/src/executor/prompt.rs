use super::*;

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

        let mut context_request = ctx.planner.context().plan_context_request(&state).await;
        context_request.surface_version = Some(surface.version.clone());
        context_request.capability_view = Some(capability_view.clone());
        let prompt_mode = context_request.mode;
        state = match CheckpointStage
            .cancel_if_requested_after_pending_input_ack(ctx, state, &mut pending_input_ack)
            .await?
        {
            CancelCheck::Continue(state) => *state,
            CancelCheck::Exit(exit) => return Ok(PromptStep::Exit(exit)),
        };
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
            messages: prompt_bundle.messages,
            capability_view,
        })))
    }
}
