//! Executor stage that gates assistant replies before transcript finalization.
//!
//! The model port can return either capability calls or an assistant reply. A
//! reply is only user-visible after the assistant-reply stage persists it into
//! the transcript. This stage sits immediately before that persistence point
//! and gives the loop family one deterministic admission decision:
//!
//! - accept the candidate and continue to normal finalization
//! - reject the candidate, record typed loop-private state, and continue the
//!   loop without writing the candidate as an assistant message
//!
//! Keeping rejected candidates out of the transcript is the important boundary:
//! prompt materialization can render a deterministic control event from the
//! typed rejection state on the next iteration, but stop policy and persistence
//! do not have to infer intent from assistant prose.

use async_trait::async_trait;
use ironclaw_turns::run_profile::AssistantReply;

use crate::{
    state::{LoopExecutionState, ReplyAdmissionRejection},
    strategies::ReplyAdmissionOutcome,
};

use super::{AgentLoopExecutorError, ExecutorStage, StageContext};

#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct ReplyAdmissionStage;

/// Candidate reply plus the current whole-loop state.
///
/// The reply has not been finalized yet. Callers must route every
/// `ParentLoopOutput::AssistantReply` through this input before invoking
/// `AssistantReplyStage`.
pub(super) struct ReplyAdmissionInput {
    pub(super) state: LoopExecutionState,
    pub(super) reply: AssistantReply,
}

/// Admission result consumed by the canonical executor.
///
/// `Accept` carries the original reply forward to transcript finalization.
/// `Reject` carries only updated loop state; the rejected text is intentionally
/// not exposed as a final assistant-message ref.
pub(super) enum ReplyAdmissionStep {
    Accept {
        state: Box<LoopExecutionState>,
        reply: AssistantReply,
    },
    Reject {
        state: Box<LoopExecutionState>,
    },
}

#[async_trait]
impl ExecutorStage<ReplyAdmissionInput> for ReplyAdmissionStage {
    type Output = ReplyAdmissionStep;

    async fn process(
        &self,
        ctx: StageContext<'_>,
        input: ReplyAdmissionInput,
    ) -> Result<ReplyAdmissionStep, AgentLoopExecutorError> {
        let mut state = input.state;
        match ctx
            .planner
            .reply_admission()
            .admit_reply(&state, &input.reply)
            .await
        {
            ReplyAdmissionOutcome::AcceptFinal => {
                // A real final reply supersedes any earlier rejected candidate.
                // Clear the pending control state so the next turn starts clean.
                state.reply_admission_state.pending_rejection = None;
                state.reply_admission_state.pending_rejection_rendered = false;
                Ok(ReplyAdmissionStep::Accept {
                    state: Box::new(state),
                    reply: input.reply,
                })
            }
            ReplyAdmissionOutcome::RejectFinal { rejection } => {
                record_rejection(&mut state, rejection);
                Ok(ReplyAdmissionStep::Reject {
                    state: Box::new(state),
                })
            }
        }
    }
}

fn record_rejection(state: &mut LoopExecutionState, rejection: ReplyAdmissionRejection) {
    // This is compact, resumable metadata only. Do not store the raw rejected
    // reply here; raw model output belongs to provider traces/transcript
    // systems, not checkpointed loop-control state.
    state.reply_admission_state.rejected_reply_candidates = state
        .reply_admission_state
        .rejected_reply_candidates
        .saturating_add(1);
    state.reply_admission_state.pending_rejection = Some(rejection);
    state.reply_admission_state.pending_rejection_rendered = false;
}
