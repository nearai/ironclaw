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
    PendingInputAck, StageContext, apply_capability_filter, cancelled_exit, debug_host_unavailable,
    failed_exit,
};

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct PromptStage;

struct PromptPlanningPipeline<'a> {
    ctx: StageContext<'a>,
    state: LoopExecutionState,
    pending_input_ack: PendingInputAck,
}

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
    messages: Vec<LoopModelMessage>,
    compaction_message_index: Vec<LoopContextCompactionMetadata>,
    rendered_reply_admission_control: bool,
}

impl BuiltPromptBundle {
    async fn build_and_refresh_compaction_prompt(
        ctx: StageContext<'_>,
        state: &mut LoopExecutionState,
        surface_version: CapabilitySurfaceVersion,
        capability_view: LoopModelCapabilityView,
    ) -> Result<Self, AgentLoopExecutorError> {
        let bundle =
            build_prompt_bundle_for_surface(ctx, state, surface_version, capability_view).await?;
        refresh_compaction_prompt_from_index(state, &bundle.compaction_message_index);
        Ok(bundle)
    }

    pub(super) fn into_model_messages(
        self,
        state: &mut LoopExecutionState,
    ) -> Vec<LoopModelMessage> {
        refresh_compaction_prompt_from_index(state, &self.compaction_message_index);
        self.messages
    }
}

struct PromptBundleCandidate {
    bundle: BuiltPromptBundle,
}

impl PromptBundleCandidate {
    async fn build(
        ctx: StageContext<'_>,
        state: &mut LoopExecutionState,
        surface_version: CapabilitySurfaceVersion,
        capability_view: LoopModelCapabilityView,
    ) -> Result<Self, AgentLoopExecutorError> {
        let bundle = BuiltPromptBundle::build_and_refresh_compaction_prompt(
            ctx,
            state,
            surface_version,
            capability_view,
        )
        .await?;
        Ok(Self { bundle })
    }

    fn into_final_without_rebuild(self) -> FinalPromptBundle {
        FinalPromptBundle {
            bundle: self.bundle,
        }
    }
}

struct FinalPromptBundle {
    bundle: BuiltPromptBundle,
}

impl FinalPromptBundle {
    async fn rebuild_after_successful_compaction(
        ctx: StageContext<'_>,
        state: &mut LoopExecutionState,
        surface_version: CapabilitySurfaceVersion,
        capability_view: LoopModelCapabilityView,
    ) -> Result<Self, AgentLoopExecutorError> {
        let bundle = BuiltPromptBundle::build_and_refresh_compaction_prompt(
            ctx,
            state,
            surface_version,
            capability_view,
        )
        .await?;
        Ok(Self { bundle })
    }

    fn into_messages(self) -> Vec<LoopModelMessage> {
        self.bundle.messages
    }

    fn rendered_reply_admission_control(&self) -> bool {
        self.bundle.rendered_reply_admission_control
    }
}

#[async_trait]
impl ExecutorStage<PromptInput> for PromptStage {
    type Output = PromptStep;

    async fn process(
        &self,
        ctx: StageContext<'_>,
        input: PromptInput,
    ) -> Result<PromptStep, AgentLoopExecutorError> {
        PromptPlanningPipeline::new(ctx, input).run().await
    }
}

impl<'a> PromptPlanningPipeline<'a> {
    fn new(ctx: StageContext<'a>, input: PromptInput) -> Self {
        Self {
            ctx,
            state: input.state,
            pending_input_ack: input.pending_input_ack,
        }
    }

    async fn run(mut self) -> Result<PromptStep, AgentLoopExecutorError> {
        let surface_filter = self.ctx.planner.capability().filter(&self.state).await;
        if let Some(exit) = self.cancel_boundary().await? {
            return Ok(PromptStep::Exit(exit));
        }

        let surface = self.visible_surface(surface_filter).await?;
        let capability_view = LoopModelCapabilityView {
            visible_capability_ids: surface
                .descriptors
                .iter()
                .map(|descriptor| descriptor.capability_id.clone())
                .collect(),
        };
        self.state.surface_version = Some(surface.version.clone());
        if let Some(exit) = self.cancel_boundary().await? {
            return Ok(PromptStep::Exit(exit));
        }

        let candidate_bundle = PromptBundleCandidate::build(
            self.ctx,
            &mut self.state,
            surface.version.clone(),
            capability_view.clone(),
        )
        .await?;
        if let Some(exit) = self.cancel_boundary().await? {
            return Ok(PromptStep::Exit(exit));
        }

        let compaction = PromptCompactionStep::new(self.ctx, &mut self.pending_input_ack)
            .run(self.state)
            .await?;
        let final_bundle = match compaction {
            PromptCompactionOutcome::Exited(exit) => return Ok(PromptStep::Exit(exit)),
            PromptCompactionOutcome::Skipped(state) => {
                self.state = state;
                candidate_bundle.into_final_without_rebuild()
            }
            PromptCompactionOutcome::Compacted(state) => {
                self.state = state;
                let bundle = FinalPromptBundle::rebuild_after_successful_compaction(
                    self.ctx,
                    &mut self.state,
                    surface.version.clone(),
                    capability_view.clone(),
                )
                .await?;
                if let Some(exit) = self.cancel_boundary().await? {
                    return Ok(PromptStep::Exit(exit));
                }
                bundle
            }
        };
        if final_bundle.rendered_reply_admission_control() {
            self.state.reply_admission_state.pending_rejection_rendered = true;
        }

        Ok(PromptStep::Prepared(Box::new(PromptOutput {
            state: self.state,
            pending_input_ack: self.pending_input_ack,
            surface,
            messages: final_bundle.into_messages(),
            capability_view,
        })))
    }

    async fn cancel_boundary(&mut self) -> Result<Option<LoopExit>, AgentLoopExecutorError> {
        let original_state = self.state.clone();
        let state = std::mem::replace(
            &mut self.state,
            LoopExecutionState::initial_for_run(self.ctx.host.run_context()),
        );
        let cancel_check = CheckpointStage
            .cancel_if_requested_after_pending_input_ack(
                self.ctx,
                state,
                &mut self.pending_input_ack,
            )
            .await;
        self.state = match cancel_check {
            Ok(CancelCheck::Continue(state)) => *state,
            Ok(CancelCheck::Exit(exit)) => return Ok(Some(exit)),
            Err(error) => {
                self.state = original_state;
                return Err(error);
            }
        };
        Ok(None)
    }

    async fn visible_surface(
        &self,
        surface_filter: crate::strategies::CapabilityFilter,
    ) -> Result<VisibleCapabilitySurface, AgentLoopExecutorError> {
        let mut surface = self
            .ctx
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
                iteration = self.state.iteration,
                surface_version = %surface.version,
                visible_capability_count = surface.descriptors.len(),
                visible_capability_sample = ?visible_capability_sample,
                "agent loop prompt capability surface prepared"
            );
        }
        Ok(surface)
    }
}

enum PromptCompactionOutcome {
    Skipped(LoopExecutionState),
    Compacted(LoopExecutionState),
    Exited(LoopExit),
}

struct PromptCompactionStep<'a, 'b> {
    ctx: StageContext<'a>,
    pending_input_ack: &'b mut PendingInputAck,
}

impl<'a, 'b> PromptCompactionStep<'a, 'b> {
    fn new(ctx: StageContext<'a>, pending_input_ack: &'b mut PendingInputAck) -> Self {
        Self {
            ctx,
            pending_input_ack,
        }
    }

    async fn run(
        self,
        mut state: LoopExecutionState,
    ) -> Result<PromptCompactionOutcome, AgentLoopExecutorError> {
        let decision = self
            .ctx
            .planner
            .compaction()
            .should_compact(&state, self.ctx.host.run_context());

        let CompactionDecision::Trigger {
            drop_through_seq,
            preserve_tail_tokens,
            deadline_ms,
        } = decision
        else {
            return Ok(PromptCompactionOutcome::Skipped(state));
        };

        let task_id = SystemInferenceTaskId::new();
        CheckpointStage
            .emit_progress(
                self.ctx,
                LoopProgressEvent::CompactionStarted {
                    task_id,
                    initiator: CompactionInitiator::Auto,
                },
            )
            .await;
        state = match CheckpointStage
            .cancel_if_requested_after_pending_input_ack(self.ctx, state, self.pending_input_ack)
            .await?
        {
            CancelCheck::Continue(state) => *state,
            CancelCheck::Exit(exit) => {
                return Ok(PromptCompactionOutcome::Exited(exit));
            }
        };

        let compaction_request = LoopCompactionRequest {
            task_id,
            thread_id: self.ctx.host.run_context().thread_id.clone(),
            last_compacted_through_seq: state.compaction_state.last_compacted_through_seq,
            drop_through_seq,
            preserve_tail_tokens,
            mode: LoopCompactionMode::Fresh,
            deadline_ms,
        };
        let compaction_result = await_compaction_with_cancellation(
            self.ctx,
            Duration::from_millis(deadline_ms),
            self.ctx.host.compact_loop_context(compaction_request),
        )
        .await;
        let response = match compaction_result {
            CompactionCallOutcome::Completed(Ok(response)) => response,
            CompactionCallOutcome::Completed(Err(LoopCompactionError::Cancelled))
            | CompactionCallOutcome::Cancelled => {
                return compaction_cancelled_exit(self.ctx, state, self.pending_input_ack).await;
            }
            CompactionCallOutcome::Completed(Err(error)) => {
                return compaction_failed_exit(
                    self.ctx,
                    state,
                    self.pending_input_ack,
                    task_id,
                    &error,
                )
                .await;
            }
            CompactionCallOutcome::TimedOut => {
                let error = LoopCompactionError::InferenceFailed {
                    safe_summary: safe("compaction deadline exceeded"),
                };
                return compaction_failed_exit(
                    self.ctx,
                    state,
                    self.pending_input_ack,
                    task_id,
                    &error,
                )
                .await;
            }
        };

        state = match CheckpointStage
            .cancel_if_requested_after_pending_input_ack(self.ctx, state, self.pending_input_ack)
            .await?
        {
            CancelCheck::Continue(state) => *state,
            CancelCheck::Exit(exit) => {
                return Ok(PromptCompactionOutcome::Exited(exit));
            }
        };

        state.compaction_state.last_compacted_through_seq = Some(drop_through_seq);
        state.compaction_state.force_compact_on_next_iteration = false;
        state
            .compaction_prompt
            .retain_after_sequence(drop_through_seq);
        CheckpointStage
            .emit_progress(
                self.ctx,
                LoopProgressEvent::CompactionCompleted {
                    task_id,
                    compression_ratio_ppm: response.compression_ratio_ppm,
                },
            )
            .await;
        let checked = CheckpointStage
            .write(self.ctx, state, CheckpointKind::BeforeModel)
            .await?;
        self.pending_input_ack.ack(self.ctx.host).await?;
        Ok(PromptCompactionOutcome::Compacted(checked.state))
    }
}

enum CompactionCallOutcome {
    Completed(Result<ironclaw_turns::run_profile::LoopCompactionResponse, LoopCompactionError>),
    TimedOut,
    Cancelled,
}

async fn await_compaction_with_cancellation<F>(
    ctx: StageContext<'_>,
    deadline: Duration,
    call: F,
) -> CompactionCallOutcome
where
    F: std::future::Future<
            Output = Result<
                ironclaw_turns::run_profile::LoopCompactionResponse,
                LoopCompactionError,
            >,
        >,
{
    let call = call;
    tokio::pin!(call);
    let timeout = tokio::time::sleep(deadline);
    tokio::pin!(timeout);
    let cancellation = ctx.host.cancellation_requested();
    tokio::pin!(cancellation);

    tokio::select! {
        result = &mut call => CompactionCallOutcome::Completed(result),
        _ = &mut timeout => CompactionCallOutcome::TimedOut,
        _signal = &mut cancellation => {
            CompactionCallOutcome::Cancelled
        }
    }
}

async fn compaction_cancelled_exit(
    ctx: StageContext<'_>,
    state: LoopExecutionState,
    pending_input_ack: &mut PendingInputAck,
) -> Result<PromptCompactionOutcome, AgentLoopExecutorError> {
    let checked = CheckpointStage
        .write(ctx, state, CheckpointKind::Final)
        .await?;
    pending_input_ack.ack(ctx.host).await?;
    let exit = cancelled_exit(ctx.host, checked.state, Some(checked.checkpoint_id))?;
    Ok(PromptCompactionOutcome::Exited(exit))
}

async fn compaction_failed_exit(
    ctx: StageContext<'_>,
    state: LoopExecutionState,
    pending_input_ack: &mut PendingInputAck,
    task_id: SystemInferenceTaskId,
    error: &LoopCompactionError,
) -> Result<PromptCompactionOutcome, AgentLoopExecutorError> {
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
    Ok(PromptCompactionOutcome::Exited(exit))
}

pub(super) async fn build_prompt_bundle_for_surface(
    ctx: StageContext<'_>,
    state: &LoopExecutionState,
    surface_version: CapabilitySurfaceVersion,
    capability_view: LoopModelCapabilityView,
) -> Result<BuiltPromptBundle, AgentLoopExecutorError> {
    let context_plan = ctx.planner.context().plan_context_request(state).await;
    let mut context_request = context_plan.request;
    context_request.surface_version = Some(surface_version);
    context_request.capability_view = Some(capability_view);
    let prompt_mode = context_request.mode;
    let rendered_reply_admission_control = context_plan.emitted_admission_control;
    let prompt_bundle = ctx
        .host
        .build_prompt_bundle(context_request)
        .await
        .map_err(|error| {
            debug_host_unavailable(HostStage::Prompt, &error);
            AgentLoopExecutorError::HostUnavailable {
                stage: HostStage::Prompt,
            }
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
        rendered_reply_admission_control,
    })
}

fn refresh_compaction_prompt_from_index(
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
        LoopCompactionError::UnsupportedMode => "unsupported mode",
        LoopCompactionError::InputTooLarge => "input too large",
        LoopCompactionError::SecurityRejected { .. } => "security rejected",
        LoopCompactionError::InferenceFailed { .. } => "inference failed",
        LoopCompactionError::Cancelled => "cancelled",
        LoopCompactionError::PersistenceFailed { .. } => "persistence failed",
    };
    LoopSafeSummary::new(value).unwrap_or_else(|_| LoopSafeSummary::model_gateway_failed())
}

fn safe(value: &'static str) -> LoopSafeSummary {
    LoopSafeSummary::new(value).unwrap_or_else(|_| LoopSafeSummary::model_gateway_failed())
}
