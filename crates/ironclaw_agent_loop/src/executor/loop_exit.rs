use async_trait::async_trait;
use ironclaw_turns::{
    LoopExit, LoopMessageRef,
    run_profile::{AssistantReply, FinalizeAssistantMessage},
};

use crate::{
    state::{CheckpointKind, LoopExecutionState},
    strategies::StopKind,
};

use super::{
    AgentLoopExecutorError, CheckpointStage, ExecutorStage, FailedExitDetails, StageContext,
    completed_exit, explain_failure, failed_exit,
};

const NO_PROGRESS_FALLBACK_REPLY: &str = concat!(
    "I stopped because I was repeating the same step without making progress. ",
    "The recent tool activity shows the repeated calls, results, and any failure summaries. ",
    "Try again with a narrower request, or fix the failed tool/resource and rerun it."
);

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
                let reply_ref = finalize_no_progress_fallback(ctx).await?;
                state.assistant_refs.push(reply_ref);
                let checked = CheckpointStage
                    .write(ctx, state, CheckpointKind::Final)
                    .await?;
                completed_exit(ctx.host, checked.state, Some(checked.checkpoint_id))
            }
            StopKind::Aborted(failure_kind) => {
                let mut state = state;
                let explanation_message_ref = explain_failure(ctx, &state, failure_kind).await;
                if let Some(message_ref) = explanation_message_ref.as_ref() {
                    state.assistant_refs.push(message_ref.clone());
                }
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

async fn finalize_no_progress_fallback(
    ctx: StageContext<'_>,
) -> Result<LoopMessageRef, AgentLoopExecutorError> {
    let reply_ref = ctx
        .host
        .finalize_assistant_message(FinalizeAssistantMessage {
            reply: AssistantReply {
                content: NO_PROGRESS_FALLBACK_REPLY.to_string(),
            },
        })
        .await
        .map_err(|_| AgentLoopExecutorError::HostUnavailable {
            stage: super::HostStage::Transcript,
        })?;
    Ok(reply_ref)
}
