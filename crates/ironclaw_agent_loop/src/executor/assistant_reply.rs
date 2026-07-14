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
        let reply = apply_capability_final_reply_presentation(&state, input.reply);
        let output_tokens = input
            .usage
            .map(|usage| usage.output_tokens)
            .unwrap_or_else(|| estimate_output_tokens(&reply));
        let reply_ref = ctx
            .host
            .finalize_assistant_message(FinalizeAssistantMessage { reply })
            .await
            .map_err(|_| AgentLoopExecutorError::HostUnavailable {
                stage: HostStage::Transcript,
            })?;
        state.assistant_refs.push(reply_ref.clone());
        state.pending_final_reply_presentations = crate::state::BoundedRing::new();
        state.recent_output_token_counts.push(output_tokens);
        // NOTE: cumulative model usage is accumulated once per model response in
        // the canonical executor (before the output branch), so it is NOT
        // accumulated again here — doing so would double-count assistant-reply
        // turns. `input.usage` is still used above for the output-token window.
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

pub(super) fn apply_capability_final_reply_presentation(
    state: &LoopExecutionState,
    mut reply: AssistantReply,
) -> AssistantReply {
    if let Some(presentation) = state.pending_final_reply_presentations.iter().next_back() {
        reply.content = presentation.safe_reply().to_string();
    }
    reply
}

fn estimate_output_tokens(reply: &AssistantReply) -> u32 {
    if reply.content.is_empty() {
        return 0;
    }
    let estimated = reply.content.len().div_ceil(4).max(1);
    estimated.min(u32::MAX as usize) as u32
}

#[cfg(test)]
mod tests {
    use ironclaw_host_api::CapabilityFinalReplyPresentation;

    use super::*;

    #[test]
    fn capability_owned_final_presentation_replaces_model_authored_result_prose() {
        let context = crate::test_support::test_run_context("safe-final-reply");
        let mut state = LoopExecutionState::initial_for_run(&context);
        state.pending_final_reply_presentations.push(
            CapabilityFinalReplyPresentation::new("Routine created\nSchedule: recurring")
                .expect("safe presentation"),
        );

        let reply = apply_capability_final_reply_presentation(
            &state,
            AssistantReply {
                content: "trigger_id=secret-id, cron=*/5 * * * *".to_string(),
            },
        );

        assert_eq!(reply.content, "Routine created\nSchedule: recurring");
    }
}
