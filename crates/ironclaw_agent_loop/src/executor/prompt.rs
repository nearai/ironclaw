use async_trait::async_trait;
use ironclaw_turns::LoopFailureKind;
use ironclaw_turns::{
    LoopExit,
    run_profile::{
        CapabilitySurfaceVersion, CompactionInitiator, LoopCompactionError, LoopCompactionMode,
        LoopCompactionRequest, LoopContextCompactionKind, LoopContextCompactionMetadata,
        LoopModelCapabilityView, LoopModelMessage, LoopProgressEvent, LoopSafeSummary,
        SystemInferenceTaskId, VisibleCapabilityRequest, VisibleCapabilitySurface,
    },
};
use std::time::Duration;
use tracing::debug;

use crate::state::{
    CheckpointKind, CompactionPromptSnapshot, IndexedMessageKind, LoopExecutionState,
    MessageIndexEntry,
};
use crate::strategies::CompactionDecision;

use super::{
    AgentLoopExecutorError, CancelCheck, CheckpointStage, ExecutorStage, HostStage,
    PendingInputAck, StageContext, apply_capability_filter, failed_exit,
};

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct PromptStage;

#[derive(Debug, Default, Clone, Copy)]
struct PromptPlanningPipeline;

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

pub(super) struct BuiltPromptBundle {
    pub(super) messages: Vec<LoopModelMessage>,
    pub(super) compaction_message_index: Vec<LoopContextCompactionMetadata>,
}

#[async_trait]
impl ExecutorStage<PromptInput> for PromptStage {
    type Output = PromptStep;

    async fn process(
        &self,
        ctx: StageContext<'_>,
        input: PromptInput,
    ) -> Result<PromptStep, AgentLoopExecutorError> {
        PromptPlanningPipeline.process(ctx, input).await
    }
}

impl PromptPlanningPipeline {
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

        let candidate_bundle = build_prompt_bundle_for_surface(
            ctx,
            &state,
            surface.version.clone(),
            capability_view.clone(),
        )
        .await?;
        apply_compaction_index_from_prompt_bundle(
            &mut state,
            &candidate_bundle.compaction_message_index,
        );
        state = match CheckpointStage
            .cancel_if_requested_after_pending_input_ack(ctx, state, &mut pending_input_ack)
            .await?
        {
            CancelCheck::Continue(state) => *state,
            CancelCheck::Exit(exit) => return Ok(PromptStep::Exit(exit)),
        };

        let compaction = maybe_compact_prompt_context(ctx, state, &mut pending_input_ack).await?;
        if let Some(exit) = compaction.exit {
            return Ok(PromptStep::Exit(exit));
        }
        state = compaction.state;

        let final_bundle = if compaction.compacted {
            let bundle = build_prompt_bundle_for_surface(
                ctx,
                &state,
                surface.version.clone(),
                capability_view.clone(),
            )
            .await?;
            apply_compaction_index_from_prompt_bundle(&mut state, &bundle.compaction_message_index);
            state = match CheckpointStage
                .cancel_if_requested_after_pending_input_ack(ctx, state, &mut pending_input_ack)
                .await?
            {
                CancelCheck::Continue(state) => *state,
                CancelCheck::Exit(exit) => return Ok(PromptStep::Exit(exit)),
            };
            bundle
        } else {
            candidate_bundle
        };

        Ok(PromptStep::Prepared(Box::new(PromptOutput {
            state,
            pending_input_ack,
            surface,
            messages: final_bundle.messages,
            capability_view,
        })))
    }
}

struct PromptCompactionOutput {
    state: LoopExecutionState,
    exit: Option<LoopExit>,
    compacted: bool,
}

async fn maybe_compact_prompt_context(
    ctx: StageContext<'_>,
    mut state: LoopExecutionState,
    pending_input_ack: &mut PendingInputAck,
) -> Result<PromptCompactionOutput, AgentLoopExecutorError> {
    let decision = ctx
        .planner
        .compaction()
        .should_compact(&state, ctx.host.run_context());

    let CompactionDecision::Trigger {
        drop_through_seq,
        preserve_tail_tokens,
        deadline_ms,
    } = decision
    else {
        return Ok(PromptCompactionOutput {
            state,
            exit: None,
            compacted: false,
        });
    };

    let task_id = SystemInferenceTaskId::new();
    CheckpointStage
        .emit_progress(
            ctx,
            LoopProgressEvent::CompactionStarted {
                task_id,
                initiator: CompactionInitiator::Auto,
            },
        )
        .await;
    let compaction_result = tokio::time::timeout(
        Duration::from_millis(deadline_ms),
        ctx.host.compact_loop_context(LoopCompactionRequest {
            task_id,
            thread_id: ctx.host.run_context().thread_id.clone(),
            last_compacted_through_seq: state.compaction_state.last_compacted_through_seq,
            drop_through_seq,
            preserve_tail_tokens,
            mode: LoopCompactionMode::Fresh,
            deadline_ms,
        }),
    )
    .await;
    let response = match compaction_result {
        Ok(Ok(response)) => response,
        Ok(Err(error)) => {
            return compaction_failed_exit(ctx, state, pending_input_ack, task_id, &error).await;
        }
        Err(_) => {
            let error = LoopCompactionError::InferenceFailed {
                safe_summary: safe("compaction deadline exceeded"),
            };
            return compaction_failed_exit(ctx, state, pending_input_ack, task_id, &error).await;
        }
    };

    state.compaction_state.last_compacted_through_seq = Some(drop_through_seq);
    state.compaction_state.force_compact_on_next_iteration = false;
    state
        .compaction_prompt
        .retain_after_sequence(drop_through_seq);
    CheckpointStage
        .emit_progress(
            ctx,
            LoopProgressEvent::CompactionCompleted {
                task_id,
                compression_ratio_ppm: response.compression_ratio_ppm,
            },
        )
        .await;
    let checked = CheckpointStage
        .write(ctx, state, CheckpointKind::BeforeModel)
        .await?;
    pending_input_ack.ack(ctx.host).await?;
    Ok(PromptCompactionOutput {
        state: checked.state,
        exit: None,
        compacted: true,
    })
}

async fn compaction_failed_exit(
    ctx: StageContext<'_>,
    state: LoopExecutionState,
    pending_input_ack: &mut PendingInputAck,
    task_id: SystemInferenceTaskId,
    error: &LoopCompactionError,
) -> Result<PromptCompactionOutput, AgentLoopExecutorError> {
    CheckpointStage
        .emit_progress(
            ctx,
            LoopProgressEvent::CompactionFailed {
                task_id,
                reason_kind: loop_compaction_reason(error),
            },
        )
        .await;
    let checked = CheckpointStage
        .write(ctx, state, CheckpointKind::Final)
        .await?;
    pending_input_ack.ack(ctx.host).await?;
    let exit = failed_exit(
        ctx.host,
        checked.state,
        LoopFailureKind::CompactionUnavailable,
        Some(checked.checkpoint_id),
    )?;
    Ok(PromptCompactionOutput {
        state: LoopExecutionState::initial_for_run(ctx.host.run_context()),
        exit: Some(exit),
        compacted: false,
    })
}

pub(super) async fn build_prompt_bundle_for_surface(
    ctx: StageContext<'_>,
    state: &LoopExecutionState,
    surface_version: CapabilitySurfaceVersion,
    capability_view: LoopModelCapabilityView,
) -> Result<BuiltPromptBundle, AgentLoopExecutorError> {
    let mut context_request = ctx.planner.context().plan_context_request(state).await;
    context_request.surface_version = Some(surface_version);
    context_request.capability_view = Some(capability_view);
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

    Ok(BuiltPromptBundle {
        messages: prompt_bundle.messages,
        compaction_message_index: prompt_bundle.compaction_message_index,
    })
}

pub(super) fn apply_compaction_index_from_prompt_bundle(
    state: &mut LoopExecutionState,
    index: &[LoopContextCompactionMetadata],
) {
    let message_index = index
        .iter()
        .map(|entry| MessageIndexEntry {
            sequence: entry.sequence,
            kind: match entry.kind {
                LoopContextCompactionKind::User => IndexedMessageKind::User,
                LoopContextCompactionKind::Assistant => IndexedMessageKind::Assistant,
                LoopContextCompactionKind::System => IndexedMessageKind::System,
                LoopContextCompactionKind::Summary => IndexedMessageKind::Summary,
                LoopContextCompactionKind::Other => IndexedMessageKind::Other,
            },
            estimated_tokens: entry.estimated_tokens,
        })
        .collect();
    state.compaction_prompt = CompactionPromptSnapshot::from_message_index(message_index);
}

fn loop_compaction_reason(error: &LoopCompactionError) -> LoopSafeSummary {
    let value = match error {
        LoopCompactionError::InvalidCutPoint => "invalid cut point",
        LoopCompactionError::InputTooLarge => "input too large",
        LoopCompactionError::SecurityRejected { .. } => "security rejected",
        LoopCompactionError::InferenceFailed { .. } => "inference failed",
        LoopCompactionError::PersistenceFailed { .. } => "persistence failed",
    };
    LoopSafeSummary::new(value).unwrap_or_else(|_| LoopSafeSummary::model_gateway_failed())
}

fn safe(value: &'static str) -> LoopSafeSummary {
    LoopSafeSummary::new(value).unwrap_or_else(|_| LoopSafeSummary::model_gateway_failed())
}
