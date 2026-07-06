//! Reply admission strategy contract.

use async_trait::async_trait;
use ironclaw_common::provider_transcript::is_only_provider_transcript_artifact_lines;
use ironclaw_turns::run_profile::{
    AssistantReply, LoopInlineMessage, LoopInlineMessageBody, LoopInlineMessageRole,
};

use crate::state::{LoopExecutionState, ReplyAdmissionRejection};

pub(crate) const REPLY_ADMISSION_STOP_CONDITION_CONTROL_TEXT: &str =
    "loop control reply rejected stop condition not met continue";

/// Classifies model replies before they are finalized into the transcript.
///
/// A reply accepted here becomes a user-visible assistant message. A rejected
/// reply remains loop-private candidate state and the executor continues from
/// typed state instead of persisting the reply as final transcript content.
#[async_trait]
pub(crate) trait ReplyAdmissionStrategy: Send + Sync {
    async fn admit_reply(
        &self,
        state: &LoopExecutionState,
        reply: &AssistantReply,
    ) -> ReplyAdmissionOutcome;
}

#[allow(dead_code)]
fn _assert_object_safe(_: &dyn ReplyAdmissionStrategy) {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ReplyAdmissionOutcome {
    AcceptFinal,
    RejectFinal { rejection: ReplyAdmissionRejection },
}

/// Baseline admission policy for assistant replies.
///
/// Normal non-empty replies are final. Empty replies and provider-history
/// artifacts are rejected so weak-model echoes do not become user-visible final
/// answers.
#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultReplyAdmissionStrategy;

#[async_trait]
impl ReplyAdmissionStrategy for DefaultReplyAdmissionStrategy {
    async fn admit_reply(
        &self,
        _state: &LoopExecutionState,
        reply: &AssistantReply,
    ) -> ReplyAdmissionOutcome {
        if is_non_final_reply_artifact(&reply.content) {
            return ReplyAdmissionOutcome::RejectFinal {
                rejection: ReplyAdmissionRejection::stop_condition_not_met(),
            };
        }
        ReplyAdmissionOutcome::AcceptFinal
    }
}

fn is_non_final_reply_artifact(content: &str) -> bool {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return true;
    }
    // Only treat content as a replayed provider-transcript artifact when it is
    // MULTI-line. A genuine one-line answer that merely names a `__`-bearing tool
    // (e.g. "Tool result from web__fetch: near.ai returned 200 OK") is a real
    // final reply, not an echo of provider history — rejecting it on the
    // single-line shape was eating valid answers. A weak model echoing replayed
    // transcript reproduces multiple history lines, which this still catches.
    trimmed
        .lines()
        .filter(|line| !line.trim().is_empty())
        .count()
        > 1
        && is_only_provider_transcript_artifact_lines(content)
}

pub(crate) fn reply_admission_control_message(
    rejection: &ReplyAdmissionRejection,
) -> LoopInlineMessage {
    LoopInlineMessage {
        role: LoopInlineMessageRole::System,
        safe_body: LoopInlineMessageBody::new(reply_admission_control_text(rejection))
            .expect("static loop-control text is non-empty and safe"), // safety: static safe ASCII words.
    }
}

fn reply_admission_control_text(rejection: &ReplyAdmissionRejection) -> &'static str {
    match rejection.reason_code {
        crate::state::ReplyAdmissionRejectionReason::StopConditionNotMet => {
            REPLY_ADMISSION_STOP_CONDITION_CONTROL_TEXT
        }
    }
}

#[cfg(test)]
mod tests {
    use ironclaw_turns::run_profile::AssistantReply;

    use super::*;
    use crate::test_support::test_run_context;

    #[test]
    fn reply_admission_strategy_is_object_safe() {
        _assert_object_safe(&DefaultReplyAdmissionStrategy);
    }

    #[tokio::test]
    async fn default_reply_admission_accepts_final_reply() {
        let context = test_run_context("default-reply-admission");
        let state = LoopExecutionState::initial_for_run(&context);
        let reply = AssistantReply {
            content: "done".to_string(),
        };

        let outcome = DefaultReplyAdmissionStrategy
            .admit_reply(&state, &reply)
            .await;

        assert_eq!(outcome, ReplyAdmissionOutcome::AcceptFinal);
    }

    #[tokio::test]
    async fn default_reply_admission_rejects_empty_reply() {
        let context = test_run_context("default-reply-admission-empty");
        let state = LoopExecutionState::initial_for_run(&context);
        let reply = AssistantReply {
            content: "  \n".to_string(),
        };

        let outcome = DefaultReplyAdmissionStrategy
            .admit_reply(&state, &reply)
            .await;

        assert!(matches!(outcome, ReplyAdmissionOutcome::RejectFinal { .. }));
    }

    #[tokio::test]
    async fn default_reply_admission_rejects_flattened_tool_history_echo() {
        // A weak model echoing replayed provider history reproduces MULTIPLE
        // transcript lines — that multi-line, all-artifact shape is still rejected.
        let context = test_run_context("default-reply-admission-tool-history");
        let state = LoopExecutionState::initial_for_run(&context);
        let reply = AssistantReply {
            content:
                "Previous tool event: demo__echo was invoked.\nTool result from demo__echo: hi"
                    .to_string(),
        };

        let outcome = DefaultReplyAdmissionStrategy
            .admit_reply(&state, &reply)
            .await;

        assert!(matches!(outcome, ReplyAdmissionOutcome::RejectFinal { .. }));
    }

    #[tokio::test]
    async fn default_reply_admission_accepts_single_line_answer_naming_a_tool() {
        // F3: a one-line final answer that names a `__`-bearing tool is a real
        // reply, not a replayed-transcript artifact — it must be admitted.
        let context = test_run_context("default-reply-admission-single-line-tool");
        let state = LoopExecutionState::initial_for_run(&context);
        let reply = AssistantReply {
            content: "Tool result from web__fetch: near.ai returned 200 OK.".to_string(),
        };

        let outcome = DefaultReplyAdmissionStrategy
            .admit_reply(&state, &reply)
            .await;

        assert_eq!(outcome, ReplyAdmissionOutcome::AcceptFinal);
    }

    #[tokio::test]
    async fn default_reply_admission_accepts_reply_that_mentions_tool_history_in_context() {
        let context = test_run_context("default-reply-admission-tool-history-context");
        let state = LoopExecutionState::initial_for_run(&context);
        let reply = AssistantReply {
            content: "I checked the prior tool result and the task is done.".to_string(),
        };

        let outcome = DefaultReplyAdmissionStrategy
            .admit_reply(&state, &reply)
            .await;

        assert_eq!(outcome, ReplyAdmissionOutcome::AcceptFinal);
    }

    #[tokio::test]
    async fn default_reply_admission_accepts_natural_language_tool_result_sentence() {
        let context = test_run_context("default-reply-admission-tool-result-sentence");
        let state = LoopExecutionState::initial_for_run(&context);
        let reply = AssistantReply {
            content: "Tool result from my_tool: success".to_string(),
        };

        let outcome = DefaultReplyAdmissionStrategy
            .admit_reply(&state, &reply)
            .await;

        assert_eq!(outcome, ReplyAdmissionOutcome::AcceptFinal);
    }
}
