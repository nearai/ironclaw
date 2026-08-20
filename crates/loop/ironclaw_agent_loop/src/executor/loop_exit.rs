use async_trait::async_trait;
use ironclaw_host_api::turn::{ModelInvalidOutputDetailReason, SanitizedFailure, TurnOriginKind};
use ironclaw_loop_contracts::{
    AgentLoopDriverHost, LoopExit, LoopFailureKind, LoopInlineMessage, LoopInlineMessageBody,
    LoopInlineMessageRole,
};

use crate::{
    state::{CheckpointKind, LoopExecutionState},
    strategies::StopKind,
};

use super::{
    AgentLoopExecutorError, CancelCheck, CheckpointStage, ExecutorStage, FailedExitDetails,
    StageContext, attach_failure_explanation, completed_exit, failed_exit,
    is_typed_nothing_to_report,
};

/// Instruction injected by the tools-capable completion nudge — drive the model
/// to *finish* the task (writing any required output artifact with its tools)
/// before answering, rather than merely synthesize prose from work already done.
/// This is delivered as an inline message on an ordinary loop iteration with
/// the full tool surface still available.
pub(super) const COMPLETION_NUDGE: &str = include_str!("../../prompts/completion_nudge.md");

/// Hard cap on tools-capable completion nudges per run. Each nudge re-enters the
/// loop for at least one more model call (plus any tool calls the model makes),
/// so this bounds the extra work a stuck run can generate.
pub(super) const COMPLETION_NUDGE_LIMIT: u32 = 2;

/// Whether an admitted assistant reply "trailed off" without a real closing
/// answer: empty after trimming, or ending in a colon (the model narrated a next
/// step — "Let me write the file:" — but emitted no tool call, so the turn ended
/// mid-intent). Mirrors nearai-bench's `trailed_off_without_answer` so the
/// in-loop nudge fires on the same signal the out-of-loop bench nudge used.
pub(super) struct ReplyCompletionSignals {
    pub(super) trailed_off: bool,
    pub(super) empty: bool,
    pub(super) ended_with_question: bool,
}

pub(super) fn reply_completion_signals(content: &str) -> ReplyCompletionSignals {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return ReplyCompletionSignals {
            trailed_off: true,
            empty: true,
            ended_with_question: false,
        };
    }
    let last_line = trimmed.lines().next_back().unwrap_or(trimmed).trim_start();
    // A block quote at the end is displayed content, not narration of a next
    // action. Routine confirmations commonly introduce the exact message
    // prefix and then quote it (for example `> Daily summary:`); treating that
    // literal colon as a trail-off caused two completion nudges and three
    // user-visible confirmations from one run.
    ReplyCompletionSignals {
        trailed_off: !last_line.starts_with('>') && trimmed.ends_with(':'),
        empty: false,
        ended_with_question: last_line.ends_with('?'),
    }
}

#[cfg(test)]
pub(super) fn reply_trailed_off(content: &str) -> bool {
    reply_completion_signals(content).trailed_off
}

pub(super) fn scheduled_trigger_run(host: &(dyn AgentLoopDriverHost + Send + Sync)) -> bool {
    host.run_context()
        .product_context
        .as_ref()
        .is_some_and(|context| context.origin == TurnOriginKind::ScheduledTrigger)
}

fn invalid_scheduled_final_output(
    host: &(dyn AgentLoopDriverHost + Send + Sync),
    state: &LoopExecutionState,
) -> Option<ModelInvalidOutputDetailReason> {
    if !scheduled_trigger_run(host) {
        return None;
    }
    if state.last_reply_empty {
        return Some(ModelInvalidOutputDetailReason::EmptyAssistantResponse);
    }
    state
        .last_reply_ended_with_question
        .then_some(ModelInvalidOutputDetailReason::UnattendedQuestionEndingResponse)
}

/// Inline control message carrying the completion-nudge instruction. Delivered
/// as a `User` turn so the model treats it as a fresh directive to act on with
/// its tools. A malformed static body surfaces as a planner-contract error.
pub(super) fn completion_nudge_control_message() -> Result<LoopInlineMessage, AgentLoopExecutorError>
{
    let safe_body =
        LoopInlineMessageBody::new(COMPLETION_NUDGE.trim().to_string()).map_err(|_| {
            AgentLoopExecutorError::PlannerContract {
                detail: "completion-nudge control text was invalid",
            }
        })?;
    Ok(LoopInlineMessage {
        role: LoopInlineMessageRole::User,
        safe_body,
    })
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
        if matches!(kind, StopKind::GracefulStop)
            && let Some(detail_reason) = invalid_scheduled_final_output(ctx.host, &state)
        {
            let mut state = match CheckpointStage.cancel_if_requested(ctx, state).await? {
                CancelCheck::Continue(state) => *state,
                CancelCheck::Exit(exit) => return Ok(exit),
            };
            let failure_kind = LoopFailureKind::InvalidModelOutput;
            let explanation_message_ref =
                attach_failure_explanation(ctx, &mut state, failure_kind).await?;
            let checked = CheckpointStage
                .write(ctx, state, CheckpointKind::Final)
                .await?;
            return failed_exit(
                ctx.host,
                checked.state,
                failure_kind,
                Some(checked.checkpoint_id),
                FailedExitDetails {
                    safe_summary: Some(
                        SanitizedFailure::from_trusted_static(failure_kind.as_str())
                            .with_detail(detail_reason.safe_summary()),
                    ),
                    explanation_message_ref,
                },
            );
        }
        match kind {
            StopKind::GracefulStop => {
                let checked = CheckpointStage
                    .write(ctx, state, CheckpointKind::Final)
                    .await?;
                let state = if is_typed_nothing_to_report(ctx.host, &checked.state) {
                    match CheckpointStage
                        .cancel_if_requested(ctx, checked.state)
                        .await?
                    {
                        CancelCheck::Continue(state) => *state,
                        CancelCheck::Exit(exit) => return Ok(exit),
                    }
                } else {
                    checked.state
                };
                completed_exit(ctx.host, state, Some(checked.checkpoint_id))
            }
            StopKind::NoProgressDetected => {
                let mut state = state;
                // A recurrent no-progress stop is a runtime failure, not a
                // conversational completion. The bounded warning turn already
                // gave the model its final recovery opportunity with the normal
                // capability surface, so finalize the typed failure directly.
                let explanation_message_ref = attach_failure_explanation(
                    ctx,
                    &mut state,
                    LoopFailureKind::NoProgressDetected,
                )
                .await?;
                let checked = CheckpointStage
                    .write(ctx, state, CheckpointKind::Final)
                    .await?;
                failed_exit(
                    ctx.host,
                    checked.state,
                    LoopFailureKind::NoProgressDetected,
                    Some(checked.checkpoint_id),
                    FailedExitDetails {
                        safe_summary: None,
                        explanation_message_ref,
                    },
                )
            }
            StopKind::Aborted(failure_kind) => {
                let mut state = match CheckpointStage.cancel_if_requested(ctx, state).await? {
                    CancelCheck::Continue(state) => *state,
                    CancelCheck::Exit(exit) => return Ok(exit),
                };
                let explanation_message_ref =
                    attach_failure_explanation(ctx, &mut state, failure_kind).await?;
                let checked = CheckpointStage
                    .write(ctx, state, CheckpointKind::Final)
                    .await?;
                failed_exit(
                    ctx.host,
                    checked.state,
                    failure_kind,
                    Some(checked.checkpoint_id),
                    FailedExitDetails {
                        safe_summary: None,
                        explanation_message_ref,
                    },
                )
            }
        }
    }
}
