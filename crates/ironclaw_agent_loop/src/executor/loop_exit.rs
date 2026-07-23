use async_trait::async_trait;
use ironclaw_turns::{
    LoopExit, LoopFailureKind,
    run_profile::{LoopInlineMessage, LoopInlineMessageBody, LoopInlineMessageRole},
};

use crate::{
    state::{CheckpointKind, LoopExecutionState},
    strategies::StopKind,
};

use super::{
    AgentLoopExecutorError, CancelCheck, CheckpointStage, ExecutorStage, FailedExitDetails,
    StageContext, attach_failure_explanation, completed_exit, failed_exit,
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
pub(super) fn reply_trailed_off(content: &str) -> bool {
    let trimmed = content.trim();
    trimmed.is_empty() || trimmed.ends_with(':')
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
        match kind {
            StopKind::GracefulStop => {
                let checked = CheckpointStage
                    .write(ctx, state, CheckpointKind::Final)
                    .await?;
                completed_exit(ctx.host, checked.state, Some(checked.checkpoint_id))
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
                        diagnostic_ref: None,
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
                        diagnostic_ref: None,
                        safe_summary: None,
                        explanation_message_ref,
                    },
                )
            }
        }
    }
}
