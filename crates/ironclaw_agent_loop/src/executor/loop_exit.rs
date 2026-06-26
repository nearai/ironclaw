use async_trait::async_trait;
use ironclaw_turns::{
    LoopExit, LoopFailureKind, LoopMessageRef,
    run_profile::{
        FinalizeAssistantMessage, LoopInlineMessage, LoopInlineMessageRole,
        LoopModelCapabilityView, LoopModelRequest, LoopSafeSummary, ParentLoopOutput,
    },
};

use crate::{
    state::{CheckpointKind, LoopExecutionState},
    strategies::{ReplyAdmissionOutcome, StopKind},
};

use super::{
    AgentLoopExecutorError, CheckpointStage, ExecutorStage, HostStage, StageContext,
    completed_exit, failed_exit, model_preference_to_host,
};

/// Instruction injected by the final-answer nudge — drive the model to produce a
/// closing answer with no tools available. Template lives in a prompt file so it
/// stays reviewable and versioned with the rest of the prompt surface.
pub(super) const FINAL_ANSWER_NUDGE: &str = include_str!("../../prompts/final_answer_nudge.md");

/// Driver-specific "final-answer" nudge: when the loop would otherwise end a turn
/// with no real assistant answer (empty/trailed-off reply, model-call budget
/// exhausted, or no-progress detected), issue ONE extra **tool-free** model call
/// asking the model to synthesize a closing answer from the work done, and return
/// the finalized reply ref. This is the reborn equivalent of the legacy loop's
/// `on_tool_intent_nudge` / force-text-recovery.
///
/// Gated by `SteeringPolicy.allow_driver_specific_nudges` (off in production) and
/// capped at one nudge per run. Returns `Ok(None)` when disabled, capped, or the
/// model still declines to answer — callers then keep their existing behavior.
/// Does NOT push to `state.assistant_refs` (the caller owns that, to stay
/// consistent with each exit path's checkpoint ordering).
pub(super) async fn try_final_answer_nudge(
    ctx: StageContext<'_>,
    state: &mut LoopExecutionState,
) -> Result<Option<LoopMessageRef>, AgentLoopExecutorError> {
    if !ctx
        .host
        .run_context()
        .resolved_run_profile
        .steering_policy
        .allow_driver_specific_nudges
    {
        return Ok(None);
    }
    if state.final_answer_nudges_used >= 1 {
        return Ok(None);
    }

    // Build the prompt-context request, then suppress tools. Clearing
    // `surface_version`/`capability_view` here only strips tools from the prompt
    // *text*; the empty capability view set on the model request below (not these
    // `None`s) is what actually forces a tool-free provider call. See the comment
    // on `LoopModelRequest.capability_view` construction further down.
    let context_plan = ctx.planner.context().plan_context_request(state).await;
    let mut request = context_plan.request;
    request.surface_version = None;
    request.capability_view = None;
    let safe_body = LoopSafeSummary::new(FINAL_ANSWER_NUDGE.trim().to_string()).map_err(|_| {
        AgentLoopExecutorError::PlannerContract {
            detail: "final-answer nudge body was invalid",
        }
    })?;
    request.inline_messages.push(LoopInlineMessage {
        role: LoopInlineMessageRole::User,
        safe_body,
    });
    let bundle = ctx.host.build_prompt_bundle(request).await.map_err(|_| {
        AgentLoopExecutorError::HostUnavailable {
            stage: HostStage::Prompt,
        }
    })?;
    // Count it before the call so a failure can't be retried into a loop.
    state.final_answer_nudges_used += 1;

    let model_preference = model_preference_to_host(ctx.planner.model().preference(state).await)?;
    // An *empty* capability view (not `None`) is what actually forces a tool-free
    // model call: the reborn gateway attaches tools whenever the loop port holds a
    // capability port, filtered by this view — an empty visible set filters the
    // surface to zero tools, so the provider gets a text-only request and must
    // answer in prose. `surface_version: None` only strips tools from the prompt
    // *text*, not from the provider tool array, so it is not sufficient on its own.
    let model_request = LoopModelRequest {
        messages: bundle.messages,
        surface_version: None,
        model_preference,
        capability_view: Some(LoopModelCapabilityView {
            visible_capability_ids: Vec::new(),
        }),
    };
    let response = ctx.host.stream_model(model_request).await.map_err(|_| {
        AgentLoopExecutorError::HostUnavailable {
            stage: HostStage::Model,
        }
    })?;

    let usage = response.usage;
    match response.output {
        ParentLoopOutput::AssistantReply(reply) => {
            // Route the nudged reply through the SAME admission policy as a
            // normal assistant reply, rather than a bespoke `!is_empty()` check:
            // this keeps `DefaultReplyAdmissionStrategy`'s protections (blank
            // text, provider-transcript artifacts) as the single gate before
            // anything is finalized into the transcript.
            match ctx
                .planner
                .reply_admission()
                .admit_reply(state, &reply)
                .await
            {
                ReplyAdmissionOutcome::AcceptFinal => {
                    // Preserve the canonical assistant-reply accounting so the
                    // diminishing-returns window sees the nudge turn's output
                    // tokens (matches `AssistantReplyStage`).
                    let output_tokens = usage
                        .map(|u| u.output_tokens)
                        .unwrap_or_else(|| estimate_output_tokens(&reply.content));
                    let reply_ref = ctx
                        .host
                        .finalize_assistant_message(FinalizeAssistantMessage { reply })
                        .await
                        .map_err(|_| AgentLoopExecutorError::HostUnavailable {
                            stage: HostStage::Transcript,
                        })?;
                    state.recent_output_token_counts.push(output_tokens);
                    Ok(Some(reply_ref))
                }
                // Admission rejected it (empty / artifact) — give up; the caller
                // falls back to its existing exit (typed no-progress failure, or
                // the budget path's fail-closed terminal).
                ReplyAdmissionOutcome::RejectFinal { .. } => Ok(None),
            }
        }
        // Model emitted capability calls despite the tool-free surface — give up.
        _ => Ok(None),
    }
}

/// Fallback output-token estimate when the provider reports no usage, mirroring
/// `AssistantReplyStage`'s estimate so accounting stays consistent.
fn estimate_output_tokens(content: &str) -> u32 {
    if content.is_empty() {
        return 0;
    }
    let estimated = content.len().div_ceil(4).max(1);
    estimated.min(u32::MAX as usize) as u32
}

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct ExitStage;

pub(super) struct ExitInput {
    pub(super) state: LoopExecutionState,
    pub(super) kind: StopKind,
}

#[async_trait]
impl ExecutorStage<ExitInput> for ExitStage {
    type Output = LoopExit;

    async fn process(
        &self,
        ctx: StageContext<'_>,
        input: ExitInput,
    ) -> Result<LoopExit, AgentLoopExecutorError> {
        self.for_stop(ctx, input.state, input.kind).await
    }
}

impl ExitStage {
    async fn for_stop(
        &self,
        ctx: StageContext<'_>,
        state: LoopExecutionState,
        kind: StopKind,
    ) -> Result<LoopExit, AgentLoopExecutorError> {
        match kind {
            StopKind::GracefulStop => {
                let checked = CheckpointStage
                    .write(ctx, state, CheckpointKind::Final)
                    .await?;
                completed_exit(ctx.host, checked.state, Some(checked.checkpoint_id))
            }
            StopKind::NoProgressDetected => {
                let mut state = state;
                // A no-progress stop is a runtime *failure*, not a conversational
                // completion. Where the driver-specific nudge is enabled and the
                // model synthesizes a real closing answer, complete with that
                // answer (preserves the #4837 final-answer-nudge benchmark path,
                // bit-for-bit). Otherwise finalize a typed no-progress failure that
                // the product layer renders deterministically — never a canned
                // "I stopped" reply finalized as a successful turn.
                // The nudge owns its own output-token accounting (it pushes to
                // `recent_output_token_counts` on AcceptFinal); the caller only
                // owns `assistant_refs`. Keep the checkpoint write single and
                // shared across both outcomes.
                let completed = match try_final_answer_nudge(ctx, &mut state).await? {
                    Some(reply_ref) => {
                        state.assistant_refs.push(reply_ref);
                        true
                    }
                    None => false,
                };
                let checked = CheckpointStage
                    .write(ctx, state, CheckpointKind::Final)
                    .await?;
                if completed {
                    completed_exit(ctx.host, checked.state, Some(checked.checkpoint_id))
                } else {
                    failed_exit(
                        ctx.host,
                        checked.state,
                        LoopFailureKind::NoProgressDetected,
                        Some(checked.checkpoint_id),
                    )
                }
            }
            StopKind::Aborted(failure_kind) => {
                let checked = CheckpointStage
                    .write(ctx, state, CheckpointKind::Final)
                    .await?;
                failed_exit(
                    ctx.host,
                    checked.state,
                    failure_kind,
                    Some(checked.checkpoint_id),
                )
            }
        }
    }
}
