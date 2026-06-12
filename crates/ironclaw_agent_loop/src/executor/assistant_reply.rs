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
        let output_tokens = input
            .usage
            .map(|usage| usage.output_tokens)
            .unwrap_or_else(|| estimate_output_tokens(&input.reply));
        let reply_ref = ctx
            .host
            .finalize_assistant_message(FinalizeAssistantMessage { reply: input.reply })
            .await
            .map_err(|_| AgentLoopExecutorError::HostUnavailable {
                stage: HostStage::Transcript,
            })?;
        state.assistant_refs.push(reply_ref.clone());
        state.recent_output_token_counts.push(output_tokens);
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

fn estimate_output_tokens(reply: &AssistantReply) -> u32 {
    if reply.content.is_empty() {
        return 0;
    }
    let estimated = reply.content.len().div_ceil(4).max(1);
    estimated.min(u32::MAX as usize) as u32
}
