use async_trait::async_trait;
use ironclaw_loop_contracts::{
    AgentLoopHostError, AgentLoopHostErrorKind, CapabilitySurfaceVersion, CompactionInitiator,
    LoopCompactionError, LoopCompactionMode, LoopCompactionOutcome, LoopCompactionRequest,
    LoopContextCompactionKind, LoopContextCompactionMetadata, LoopExit, LoopInlineMessage,
    LoopModelCapabilityView, LoopModelMessage, LoopProgressEvent, LoopSafeSummary,
    SystemInferenceTaskId, VisibleCapabilityRequest, VisibleCapabilitySurface,
};
use tracing::debug;

use crate::state::{
    CheckpointKind, CompactionPromptSnapshot, DeferredCompactionWatermark, IndexedMessageKind,
    LoopExecutionState, MessageIndexEntry, PendingModelRetryDirective,
};
use crate::strategies::{
    CompactionDecision, invalid_model_output_repair_control_message,
    model_error_observation_control_message, terminal_warning_control_message,
};

use super::{
    AgentLoopExecutorError, CancelCheck, CheckpointStage, ExecutorStage, HostStage, StageContext,
    apply_capability_filter, cancelled_exit, debug_host_unavailable,
    pending_approval_resume_candidate, pending_auth_resume_candidate,
    pending_external_tool_resume_candidate,
};

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct PromptStage;

struct PromptPlanningPipeline<'a> {
    ctx: StageContext<'a>,
    state: LoopExecutionState,
}

pub(super) struct PromptInput {
    pub(super) state: LoopExecutionState,
}

pub(super) struct PromptOutput {
    pub(super) state: LoopExecutionState,
    pub(super) surface: VisibleCapabilitySurface,
    pub(super) messages: Vec<ironclaw_loop_contracts::LoopModelMessage>,
    pub(super) inline_messages: Vec<LoopInlineMessage>,
    pub(super) capability_view: LoopModelCapabilityView,
    pub(super) rendered_repeated_call_warning: bool,
}

pub(super) struct ApprovalResumePromptOutput {
    pub(super) state: LoopExecutionState,
    pub(super) surface: VisibleCapabilitySurface,
    pub(super) call: ironclaw_loop_contracts::CapabilityCallCandidate,
}

pub(super) enum PromptStep {
    Prepared(Box<PromptOutput>),
    ResumeApproval(Box<ApprovalResumePromptOutput>),
    /// Re-dispatch an auth-gated capability call without a model turn.
    ///
    /// Emitted when `pending_auth_resume` is set on the incoming state. The original
    /// capability call is re-dispatched as a plain invocation (no approval token).
    /// The `pending_auth_resume` slot is cleared at every capability-outcome site
    /// (Completed, SpawnedChild, AuthRequired, error/retry paths) and at gate
    /// SkipAndContinue/Abort outcomes — never consumed via `take_if` here.
    ResumeAuth(Box<ApprovalResumePromptOutput>),
    /// Re-dispatch a client-supplied ("external") tool call without a model turn.
    ///
    /// Emitted when `pending_external_tool_resume` is set on the incoming state
    /// (the client has resumed a parked `BlockedExternalTool` run). The parked
    /// call is re-dispatched as a plain invocation; the host's external-tool
    /// decorator completes it from the run-scoped catalog's submitted output.
    ResumeExternalTool(Box<ApprovalResumePromptOutput>),
    Exit(LoopExit),
    /// Compaction-only turn: PromptCompactionStep ran (forced by the
    /// `skip_model_this_iteration` flag), no prompt was assembled, no
    /// model call this iteration. canonical.rs bypasses ModelStage +
    /// CapabilityStage + PostCapabilityStage and routes directly to
    /// StopStage.observe().
    ///
    /// ack inbound user input BEFORE stop.observe runs, mirroring the
    /// Prepared path. PromptCompactionStep::run only acks internally on
    /// Compacted; the Skipped branch (reachable when force_compact is
    /// true but message_index is empty) returns without acking — without
    /// this field the ack would silently drop.
    // Boxed to avoid a large_enum_variant warning.
    SkipModel(Box<LoopExecutionState>),
}

pub(super) struct BuiltPromptBundle {
    messages: Vec<LoopModelMessage>,
    inline_messages: Vec<LoopInlineMessage>,
    compaction_message_index: Vec<LoopContextCompactionMetadata>,
    recent_window_truncation: Option<ironclaw_loop_contracts::LoopContextWindowTruncation>,
    rendered_reply_admission_control: bool,
    rendered_repeated_call_warning: bool,
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
        observe_recent_window_truncation(state, bundle.recent_window_truncation.as_ref());
        Ok(bundle)
    }

    pub(super) fn into_model_messages(
        self,
        state: &mut LoopExecutionState,
    ) -> Vec<LoopModelMessage> {
        refresh_compaction_prompt_from_index(state, &self.compaction_message_index);
        observe_recent_window_truncation(state, self.recent_window_truncation.as_ref());
        self.messages
    }

    pub(super) fn inline_messages(&self) -> Vec<LoopInlineMessage> {
        self.inline_messages.clone()
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

    fn into_model_parts(self) -> (Vec<LoopModelMessage>, Vec<LoopInlineMessage>) {
        (self.bundle.messages, self.bundle.inline_messages)
    }

    fn rendered_reply_admission_control(&self) -> bool {
        self.bundle.rendered_reply_admission_control
    }

    fn rendered_repeated_call_warning(&self) -> bool {
        self.bundle.rendered_repeated_call_warning
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
        }
    }

    async fn run(mut self) -> Result<PromptStep, AgentLoopExecutorError> {
        let surface_filter = self.ctx.planner.capability().filter(&self.state).await;
        if let Some(exit) = self.cancel_boundary().await? {
            return Ok(PromptStep::Exit(exit));
        }

        // PostCapabilityStage set skip_model_this_iteration after a byte-cap
        // trip on the prior turn. Compact here and short-circuit before
        // building the prompt bundle — no surface filter, no prompt assembly,
        // canonical.rs to route past Model/Capability/PostCapability straight
        // to stop.observe().
        if self.state.post_capability_state.skip_model_this_iteration {
            self.state.post_capability_state.skip_model_this_iteration = false;
            let compaction = PromptCompactionStep::new(self.ctx).run(self.state).await?;
            let state = match compaction {
                PromptCompactionOutcome::Exited(exit) => return Ok(PromptStep::Exit(exit)),
                PromptCompactionOutcome::Skipped(mut state) => {
                    // Compaction couldn't actually run (e.g. empty message_index) — clear
                    // both the force flag AND the initiator so a later unrelated
                    // compaction (Auto-triggered) doesn't .take() a stale
                    // CapabilityResultOverflow initiator and misattribute telemetry.
                    state.compaction_state.force_compact_on_next_iteration = false;
                    state.compaction_state.force_compact_initiator = None;
                    state
                }
                PromptCompactionOutcome::Compacted(state) => state,
            };
            return Ok(PromptStep::SkipModel(Box::new(state)));
        }

        let surface = self.visible_surface(surface_filter).await?;
        // The capability view drives call-time authorization (the model-visible
        // capability filter), which must permit every tool the model can legitimately
        // invoke this turn — not just the advertised subset. Under progressive tool
        // disclosure the surface narrows `descriptors` to the advertised set but
        // carries the full reachable catalog in `callable_capability_ids`; use that
        // wider set so bridge / forgiving-direct calls to disclosed-but-unadvertised
        // tools aren't rejected as "outside the model-visible capability view".
        // Advertising and prompt rendering still use the narrow `descriptors`.
        // `None` callable_capability_ids means no narrowing is in effect, so fall
        // back to `descriptors` (preserves non-disclosure behavior exactly). A
        // `Some(_)` set is used verbatim, even when empty.
        let visible_capability_ids = match &surface.callable_capability_ids {
            Some(callable) => callable.clone(),
            None => surface
                .descriptors
                .iter()
                .map(|descriptor| descriptor.capability_id.clone())
                .collect(),
        };
        let capability_view = LoopModelCapabilityView {
            visible_capability_ids,
        };
        self.state.surface_version = Some(surface.version.clone());
        if let Some(exit) = self.cancel_boundary().await? {
            return Ok(PromptStep::Exit(exit));
        }
        if let Some(resume) = self.state.pending_approval_resume.as_ref() {
            let call = pending_approval_resume_candidate(resume, surface.version.clone());
            return Ok(PromptStep::ResumeApproval(Box::new(
                ApprovalResumePromptOutput {
                    state: self.state,
                    surface,
                    call,
                },
            )));
        }
        // Auth-resume check runs after approval (approval takes priority; both set
        // simultaneously is impossible today, but this ordering is defensive).
        if let Some(resume) = self.state.pending_auth_resume.as_ref() {
            let call =
                pending_auth_resume_candidate(self.ctx.host, resume, surface.version.clone())
                    .await?;
            return Ok(PromptStep::ResumeAuth(Box::new(
                ApprovalResumePromptOutput {
                    state: self.state,
                    surface,
                    call,
                },
            )));
        }
        // External-tool resume: re-dispatch the parked client-tool call so the
        // host decorator completes it from the catalog's submitted output.
        if let Some(resume) = self.state.pending_external_tool_resume.as_ref() {
            let call = pending_external_tool_resume_candidate(
                self.ctx.host,
                resume,
                surface.version.clone(),
            )
            .await?;
            return Ok(PromptStep::ResumeExternalTool(Box::new(
                ApprovalResumePromptOutput {
                    state: self.state,
                    surface,
                    call,
                },
            )));
        }

        let candidate_bundle = PromptBundleCandidate::build(
            self.ctx,
            &mut self.state,
            surface.version.clone(),
            capability_view.clone(),
        )
        .await?;
        // The candidate bundle refreshed compaction_prompt from a real prompt
        // build, so a compaction completed on an earlier iteration (SkipModel
        // compaction-only turn, forced-shrink retry) can now be judged with
        // its summary included — before the strategy decides whether to
        // compact again below.
        observe_pending_compaction_effectiveness(&mut self.state);
        if let Some(exit) = self.cancel_boundary().await? {
            return Ok(PromptStep::Exit(exit));
        }

        let compaction = PromptCompactionStep::new(self.ctx).run(self.state).await?;
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
                // The rebuilt bundle's prompt estimate includes the injected
                // summary — judge the compaction that just ran against its
                // trigger-kind baseline.
                observe_pending_compaction_effectiveness(&mut self.state);
                if let Some(exit) = self.cancel_boundary().await? {
                    return Ok(PromptStep::Exit(exit));
                }
                bundle
            }
        };
        if final_bundle.rendered_reply_admission_control() {
            self.state.reply_admission_state.pending_rejection_rendered = true;
        }
        let rendered_repeated_call_warning = final_bundle.rendered_repeated_call_warning();

        let (messages, inline_messages) = final_bundle.into_model_parts();
        // Consume the one-shot completion-nudge flag: when set, its directive was
        // injected into this iteration's prompt bundle by
        // `build_prompt_bundle_for_surface` (with the full tool surface still
        // available). Clearing here bounds the nudge to exactly this iteration and
        // keeps a later model-error retry from re-injecting it.
        self.state.completion_nudge_pending = false;
        Ok(PromptStep::Prepared(Box::new(PromptOutput {
            state: self.state,
            surface,
            messages,
            inline_messages,
            capability_view,
            rendered_repeated_call_warning,
        })))
    }

    async fn cancel_boundary(&mut self) -> Result<Option<LoopExit>, AgentLoopExecutorError> {
        let cancel_check = CheckpointStage
            .cancel_if_requested(self.ctx, self.state.clone())
            .await;
        match cancel_check {
            Ok(CancelCheck::Continue(state)) => {
                self.state = *state;
                Ok(None)
            }
            Ok(CancelCheck::Exit(exit)) => Ok(Some(exit)),
            Err(error) => Err(error),
        }
    }

    async fn visible_surface(
        &self,
        surface_filter: crate::strategies::CapabilityFilter,
    ) -> Result<VisibleCapabilitySurface, AgentLoopExecutorError> {
        let map_capability_error = |error| {
            debug_host_unavailable(HostStage::Capability, &error);
            AgentLoopExecutorError::HostUnavailable {
                stage: HostStage::Capability,
            }
        };
        let mut surface = match self
            .ctx
            .host
            .current_visible_capabilities()
            .map_err(&map_capability_error)?
        {
            Some(surface) => surface,
            None => self
                .ctx
                .host
                .visible_capabilities(VisibleCapabilityRequest)
                .await
                .map_err(map_capability_error)?,
        };
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

struct PromptCompactionStep<'a> {
    ctx: StageContext<'a>,
}

impl<'a> PromptCompactionStep<'a> {
    fn new(ctx: StageContext<'a>) -> Self {
        Self { ctx }
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
            effectiveness_baseline,
        } = decision
        else {
            return Ok(PromptCompactionOutcome::Skipped(state));
        };

        let task_id = SystemInferenceTaskId::new();
        let initiator = state
            .compaction_state
            .force_compact_initiator
            .take()
            .unwrap_or(CompactionInitiator::Auto);
        let mode = if initiator == CompactionInitiator::WindowEviction {
            LoopCompactionMode::WindowEviction
        } else {
            LoopCompactionMode::Fresh
        };
        CheckpointStage
            .emit_progress(
                self.ctx,
                LoopProgressEvent::CompactionStarted { task_id, initiator },
            )
            .await;
        state = match CheckpointStage.cancel_if_requested(self.ctx, state).await? {
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
            mode,
            deadline_ms,
        };
        let compaction_result = await_compaction_with_cancellation(
            self.ctx,
            self.ctx.host.compact_loop_context(compaction_request),
        )
        .await;
        let response = match compaction_result {
            CompactionCallOutcome::Completed(Ok(LoopCompactionOutcome::Compacted(response))) => {
                response
            }
            CompactionCallOutcome::Completed(Ok(LoopCompactionOutcome::Deferred {
                safe_summary,
            })) => {
                tracing::debug!(
                    %safe_summary,
                    "agent loop compaction deferred; continuing with the existing prompt"
                );
                return defer_compaction(self.ctx, state, drop_through_seq).await;
            }
            CompactionCallOutcome::Completed(Err(LoopCompactionError::Cancelled))
            | CompactionCallOutcome::Cancelled => {
                return compaction_cancelled_exit(self.ctx, state).await;
            }
            CompactionCallOutcome::Completed(Err(error)) => {
                return compaction_failed_continue(
                    self.ctx,
                    state,
                    task_id,
                    drop_through_seq,
                    &error,
                )
                .await;
            }
        };

        if response.redacted_leak_count > 0 {
            CheckpointStage
                .emit_progress(
                    self.ctx,
                    LoopProgressEvent::CompactionLeakDetected {
                        task_id,
                        reason_kind: LoopSafeSummary::new("redacted")
                            .unwrap_or_else(|_| LoopSafeSummary::model_gateway_failed()),
                        redacted_leak_count: response.redacted_leak_count,
                    },
                )
                .await;
        }
        state = match CheckpointStage.cancel_if_requested(self.ctx, state).await? {
            CancelCheck::Continue(state) => *state,
            CancelCheck::Exit(exit) => {
                return Ok(PromptCompactionOutcome::Exited(exit));
            }
        };

        state.compaction_state.last_compacted_through_seq = Some(drop_through_seq);
        if state
            .compaction_state
            .window_eviction
            .as_ref()
            .is_some_and(|watermark| watermark.omitted_through_sequence <= drop_through_seq)
        {
            state.compaction_state.window_eviction = None;
        }
        state.compaction_state.last_deferred = None;
        state.compaction_state.force_compact_on_next_iteration = false;
        state
            .compaction_prompt
            .retain_after_sequence(drop_through_seq);
        // Circuit-breaker accounting is deferred: the retained prompt
        // estimate here excludes the injected summary (the rebuilt bundle
        // isn't known yet), so judging effectiveness now would mark a
        // compaction whose huge summary keeps the prompt oversized as
        // "effective". Stash the trigger-kind-specific baseline; the
        // executor consumes it via observe_pending_compaction_effectiveness
        // once the prompt bundle is next rebuilt and observed_prompt_tokens
        // includes the summary. After `INEFFECTIVE_COMPACTION_TRIP_LIMIT`
        // consecutive ineffective runs the strategies stop threshold-
        // triggered compaction for the remainder of the run — Claude Code
        // measured ~250K wasted API calls/day from exactly this
        // compact-recompact doom loop before adding a breaker.
        state.compaction_state.pending_effectiveness_baseline = Some(effectiveness_baseline);
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
        Ok(PromptCompactionOutcome::Compacted(checked.state))
    }
}

enum CompactionCallOutcome {
    Completed(Result<LoopCompactionOutcome, ironclaw_loop_contracts::LoopCompactionError>),
    Cancelled,
}

/// Races the compaction call against run cancellation only. The deadline is
/// enforced solely by the inner `ModelGatewayBackedSystemInferencePort`
/// timeout (which surfaces as `SystemInferenceError::Timeout` ->
/// `LoopCompactionError::InferenceFailed`); a second, outer timeout here
/// would drop the call future on the same deadline and detach the
/// `GuardedSystemInferencePort` worker it spawned, leaking a task per
/// timed-out compaction.
async fn await_compaction_with_cancellation<F>(
    ctx: StageContext<'_>,
    call: F,
) -> CompactionCallOutcome
where
    F: std::future::Future<Output = Result<LoopCompactionOutcome, LoopCompactionError>>,
{
    let call = call;
    tokio::pin!(call);
    let cancellation = ctx.host.cancellation_requested();
    tokio::pin!(cancellation);

    tokio::select! {
        result = &mut call => CompactionCallOutcome::Completed(result),
        _signal = &mut cancellation => {
            CompactionCallOutcome::Cancelled
        }
    }
}

async fn compaction_cancelled_exit(
    ctx: StageContext<'_>,
    state: LoopExecutionState,
) -> Result<PromptCompactionOutcome, AgentLoopExecutorError> {
    let checked = CheckpointStage
        .write(ctx, state, CheckpointKind::Final)
        .await?;
    let exit = cancelled_exit(ctx.host, checked.state, Some(checked.checkpoint_id))?;
    Ok(PromptCompactionOutcome::Exited(exit))
}

async fn compaction_failed_continue(
    ctx: StageContext<'_>,
    state: LoopExecutionState,
    task_id: SystemInferenceTaskId,
    drop_through_seq: u64,
    error: &LoopCompactionError,
) -> Result<PromptCompactionOutcome, AgentLoopExecutorError> {
    let reason_kind = loop_compaction_reason(error);
    tracing::debug!(
        task_id = ?task_id,
        %reason_kind,
        "compaction failed; continuing run with uncompacted prompt"
    );
    CheckpointStage
        .emit_progress(
            ctx,
            LoopProgressEvent::CompactionFailed {
                task_id,
                reason_kind,
            },
        )
        .await;
    defer_compaction(ctx, state, drop_through_seq).await
}

/// Shared tail for both compaction-deferral paths (explicit `Deferred`
/// outcome and failure-fallback continue): clears the force-compact flag,
/// records the deferred watermark, and honors cancellation. The
/// mutate-then-cancel-check order is intentional — the watermark persists
/// via the `Final` checkpoint even if the run is cancelled right after.
async fn defer_compaction(
    ctx: StageContext<'_>,
    state: LoopExecutionState,
    drop_through_seq: u64,
) -> Result<PromptCompactionOutcome, AgentLoopExecutorError> {
    let mut state = state;
    state.compaction_state.force_compact_on_next_iteration = false;
    state.compaction_state.last_deferred = Some(DeferredCompactionWatermark {
        through_seq: drop_through_seq,
        prompt_fingerprint: state.compaction_prompt.fingerprint(),
    });
    state = match CheckpointStage.cancel_if_requested(ctx, state).await? {
        CancelCheck::Continue(state) => *state,
        CancelCheck::Exit(exit) => return Ok(PromptCompactionOutcome::Exited(exit)),
    };
    Ok(PromptCompactionOutcome::Skipped(state))
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
    if matches!(
        state.pending_model_retry_directive,
        Some(PendingModelRetryDirective::RepairInvalidOutput)
    ) {
        context_request
            .inline_messages
            .push(invalid_model_output_repair_control_message());
    }
    if let Some(observation) = state.pending_model_error_observation.as_ref() {
        context_request.inline_messages.push(
            model_error_observation_control_message(observation).map_err(|error| {
                debug!(%error, "model-error observation control text rejected");
                AgentLoopExecutorError::PlannerContract {
                    detail: "model-error observation control text was invalid",
                }
            })?,
        );
    }
    if let Some(observation) = state.terminal_warning_state.pending() {
        context_request.inline_messages.push(
            terminal_warning_control_message(observation).map_err(|error| {
                debug!(%error, "terminal-warning control text rejected");
                AgentLoopExecutorError::PlannerContract {
                    detail: "terminal warning control text was invalid",
                }
            })?,
        );
    }
    // Tools-capable completion nudge scheduled by the stop handling on the prior
    // turn: inject its directive so the model finishes the task (e.g. writes a
    // required output file) this iteration. The flag is consumed by
    // `PromptStage::run` after the bundle is built.
    if state.completion_nudge_pending {
        context_request
            .inline_messages
            .push(super::completion_nudge_control_message()?);
    }
    let inline_messages = context_request.inline_messages.clone();
    let prompt_mode = context_request.mode;
    let rendered_reply_admission_control = context_plan.emitted_admission_control;
    let rendered_repeated_call_warning = context_plan.emitted_repeated_call_warning;
    let prompt_bundle = ctx
        .host
        .build_prompt_bundle(context_request)
        .await
        .map_err(|error| {
            debug_host_unavailable(HostStage::Prompt, &error);
            prompt_host_error(error)
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
        inline_messages,
        compaction_message_index: prompt_bundle.compaction_message_index,
        recent_window_truncation: prompt_bundle.recent_window_truncation,
        rendered_reply_admission_control,
        rendered_repeated_call_warning,
    })
}

fn prompt_host_error(error: AgentLoopHostError) -> AgentLoopExecutorError {
    if error.kind == AgentLoopHostErrorKind::Cancelled {
        return AgentLoopExecutorError::Cancelled;
    }

    let raw_summary = error.safe_summary;
    let (safe_summary, rejected_summary_detail) = match LoopSafeSummary::new(raw_summary.clone()) {
        Ok(summary) => (summary, None),
        Err(validation_error) => {
            debug!(
                validation_error = %validation_error,
                "prompt host error summary rejected; using fallback"
            );
            (
                LoopSafeSummary::tool_failure_details_redacted(),
                Some(ironclaw_loop_contracts::sanitize_model_visible_text(
                    raw_summary,
                )),
            )
        }
    };

    AgentLoopExecutorError::HostUnavailableWithDiagnostics {
        stage: HostStage::Prompt,
        kind: error.kind,
        safe_summary,
        reason_kind: error.reason_kind,
        detail: error.detail.or(rejected_summary_detail),
    }
}

/// Consumes a completed compaction's pending effectiveness baseline against
/// the freshly rebuilt prompt estimate (which now includes the injected
/// summary) and updates the circuit-breaker accounting.
///
/// Callers must only invoke this after `compaction_prompt` was refreshed from
/// a real prompt bundle; a no-op when no compaction is awaiting judgement.
fn observe_pending_compaction_effectiveness(state: &mut LoopExecutionState) {
    let Some(baseline) = state.compaction_state.pending_effectiveness_baseline.take() else {
        return;
    };
    let post_compaction_prompt_tokens = state.compaction_prompt.observed_prompt_tokens;
    let circuit_was_open = state.compaction_state.compaction_circuit_open;
    // `take()` already cleared the pending slot, so the cloned successor state
    // carries no stale baseline.
    state.compaction_state = state
        .compaction_state
        .with_compaction_effectiveness_observed(post_compaction_prompt_tokens, baseline);
    if !circuit_was_open && state.compaction_state.compaction_circuit_open {
        // debug!, not warn!: internal loop diagnostics — info!/warn! render in
        // the REPL/TUI and corrupt the interactive display.
        debug!(
            consecutive_ineffective_compactions =
                state.compaction_state.consecutive_ineffective_compactions,
            post_compaction_prompt_tokens,
            effectiveness_baseline_tokens = baseline.tokens(),
            "context compaction circuit breaker opened after repeated ineffective compactions; threshold-triggered compaction disabled for the remainder of this run"
        );
    }
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
                LoopContextCompactionKind::ToolResult => IndexedMessageKind::ToolResult,
                LoopContextCompactionKind::System => IndexedMessageKind::System,
                LoopContextCompactionKind::Summary => IndexedMessageKind::Summary,
                LoopContextCompactionKind::Other => IndexedMessageKind::Other,
            },
            estimated_tokens: entry.estimated_tokens,
        })
        .collect();
    state.compaction_prompt = CompactionPromptSnapshot::from_message_index(message_index);
}

fn observe_recent_window_truncation(
    state: &mut LoopExecutionState,
    truncation: Option<&ironclaw_loop_contracts::LoopContextWindowTruncation>,
) {
    let Some(truncation) = truncation else {
        return;
    };
    if !matches!(
        truncation.omitted_through_kind,
        LoopContextCompactionKind::User | LoopContextCompactionKind::ToolResult
    ) || Some(truncation.omitted_through_sequence)
        <= state.compaction_state.last_compacted_through_seq
    {
        return;
    }
    state.compaction_state.window_eviction = Some(truncation.clone());
    let prompt_fingerprint = state.compaction_prompt.fingerprint();
    if state
        .compaction_state
        .last_deferred
        .is_some_and(|deferred| deferred.prompt_fingerprint == prompt_fingerprint)
    {
        return;
    }
    if !state.compaction_state.force_compact_on_next_iteration {
        state.compaction_state.force_compact_on_next_iteration = true;
        state.compaction_state.force_compact_initiator = Some(CompactionInitiator::WindowEviction);
    }
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
