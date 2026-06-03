use async_trait::async_trait;
use ironclaw_turns::run_profile::{AssistantReply, FinalizeAssistantMessage, LoopModelUsage};

use crate::{state::LoopExecutionState, strategies::TurnSummary};

use super::{
    AgentLoopExecutorError, CancelCheck, CheckpointStage, ExecutorStage, HostStage, StageContext,
    TurnCompletedStep,
};

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct AssistantReplyStage;

pub(super) struct AssistantReplyInput {
    pub(super) state: LoopExecutionState,
    pub(super) reply: AssistantReply,
    pub(super) usage: Option<LoopModelUsage>,
}

#[async_trait]
impl ExecutorStage<AssistantReplyInput> for AssistantReplyStage {
    type Output = TurnCompletedStep;

    async fn process(
        &self,
        ctx: StageContext<'_>,
        input: AssistantReplyInput,
    ) -> Result<TurnCompletedStep, AgentLoopExecutorError> {
        let mut state = input.state;
        let reply_ref = ctx
            .host
            .finalize_assistant_message(FinalizeAssistantMessage { reply: input.reply })
            .await
            .map_err(|_| AgentLoopExecutorError::HostUnavailable {
                stage: HostStage::Transcript,
            })?;
        state.assistant_refs.push(reply_ref.clone());
        if let Some(usage) = input.usage {
            state.recent_output_token_counts.push(usage.output_tokens);
        }
        state = match CheckpointStage.cancel_if_requested(ctx, state).await? {
            CancelCheck::Continue(state) => *state,
            CancelCheck::Exit(exit) => return Ok(TurnCompletedStep::Exit(exit)),
        };

        Ok(TurnCompletedStep::Continue {
            state: Box::new(state),
            summary: TurnSummary::reply_only(reply_ref),
        })
    }
}
