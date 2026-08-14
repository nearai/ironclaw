use async_trait::async_trait;
use ironclaw_loop_contracts::{AssistantReply, FinalizeAssistantMessage};

use crate::{state::LoopExecutionState, strategies::TurnSummary};

use super::{
    AgentLoopExecutorError, CancelCheck, CheckpointStage, ExecutorStage, StageContext,
    TurnCompletedStep, transcript_host_error,
};

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct AssistantReplyStage;

pub(super) struct AssistantReplyInput {
    pub(super) state: LoopExecutionState,
    pub(super) reply: AssistantReply,
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
        // Record whether this reply trailed off without a real closing answer so
        // the stop handling can decide a graceful stop warrants a tools-capable
        // completion nudge. Captured before `reply` is moved into the transcript.
        let completion_signals = super::reply_completion_signals(&input.reply.content);
        state.last_reply_trailed_off = completion_signals.trailed_off;
        state.last_reply_empty = completion_signals.empty;
        state.last_reply_ended_with_question = completion_signals.ended_with_question;
        let reply_ref = ctx
            .host
            .finalize_assistant_message(FinalizeAssistantMessage { reply: input.reply })
            .await
            .map_err(transcript_host_error)?;
        state.assistant_refs.push(reply_ref.clone());
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
