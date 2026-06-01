use ironclaw_threads::{
    LoadContextMessagesRequest, SessionThreadService, ThreadMessageId, ThreadScope,
};
use ironclaw_turns::{
    LoopMessageRef,
    run_profile::{
        AgentLoopHostError, AgentLoopHostErrorKind, LoopContextMessage, LoopContextSnippet,
        LoopRunContext, LoopSafeSummary,
    },
};

use crate::{context_message_to_loop_message, context_read_error, message_ref_from_context};

const TURN_OBJECTIVE_SNIPPET_REF: &str = "instruction:system.turn_objective";
const TURN_OBJECTIVE_SAFE_SUMMARY: &str = concat!(
    "The accepted user message for this run is the turn objective. ",
    "Before finalizing, check the final answer or completed work against that objective. ",
    "If the requested work is incomplete, blocked, or unverified, state that explicitly."
);

enum AcceptedTurnObjectiveRef {
    Opaque,
    TranscriptMessage {
        message_ref: LoopMessageRef,
        message_id: ThreadMessageId,
    },
}

pub(crate) async fn apply_turn_objective_context<S>(
    thread_service: &S,
    thread_scope: &ThreadScope,
    run_context: &LoopRunContext,
    messages: &mut Vec<LoopContextMessage>,
    instruction_snippets: &mut Vec<LoopContextSnippet>,
) -> Result<(), AgentLoopHostError>
where
    S: SessionThreadService + ?Sized + Send + Sync,
{
    let Some(objective_ref) = accepted_turn_objective_ref(run_context)? else {
        return Ok(());
    };
    instruction_snippets.push(turn_objective_instruction()?);

    let AcceptedTurnObjectiveRef::TranscriptMessage {
        message_ref,
        message_id,
    } = objective_ref
    else {
        return Ok(());
    };
    if messages.iter().any(|message| {
        message
            .message_ref
            .as_ref()
            .is_some_and(|existing_ref| existing_ref == &message_ref)
    }) {
        return Ok(());
    }

    let context_messages = thread_service
        .load_context_messages(LoadContextMessagesRequest {
            scope: thread_scope.clone(),
            thread_id: run_context.thread_id.clone(),
            message_ids: vec![message_id],
        })
        .await
        .map_err(context_read_error)?;
    let objective_message = context_messages
        .messages
        .into_iter()
        .find(|message| message_ref_from_context(message).as_ref() == Some(&message_ref))
        .and_then(context_message_to_loop_message)
        .ok_or_else(|| {
            AgentLoopHostError::new(
                AgentLoopHostErrorKind::InvalidInvocation,
                "accepted turn objective message is unavailable",
            )
        })?;
    messages.insert(0, objective_message);
    Ok(())
}

fn accepted_turn_objective_ref(
    run_context: &LoopRunContext,
) -> Result<Option<AcceptedTurnObjectiveRef>, AgentLoopHostError> {
    let Some(accepted_ref) = run_context.accepted_message_ref.as_ref() else {
        return Ok(None);
    };
    let value = accepted_ref.as_str();
    let Some(raw_message_id) = value.strip_prefix("msg:") else {
        return Ok(Some(AcceptedTurnObjectiveRef::Opaque));
    };
    if raw_message_id.is_empty() {
        return Err(invalid_accepted_message_ref());
    }
    let message_ref = LoopMessageRef::new(value.to_string()).map_err(|_| {
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::InvalidInvocation,
            "accepted turn objective message ref is invalid",
        )
    })?;
    let message_id = ThreadMessageId::parse(raw_message_id).map_err(|_| {
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::InvalidInvocation,
            "accepted turn objective message id is invalid",
        )
    })?;
    Ok(Some(AcceptedTurnObjectiveRef::TranscriptMessage {
        message_ref,
        message_id,
    }))
}

fn turn_objective_instruction() -> Result<LoopContextSnippet, AgentLoopHostError> {
    Ok(LoopContextSnippet {
        snippet_ref: TURN_OBJECTIVE_SNIPPET_REF.to_string(),
        safe_summary: LoopSafeSummary::new(TURN_OBJECTIVE_SAFE_SUMMARY)
            .map_err(|reason| {
                AgentLoopHostError::new(
                    AgentLoopHostErrorKind::Internal,
                    format!("turn objective instruction invalid: {reason}"),
                )
            })?
            .as_str()
            .to_string(),
        metadata: None,
    })
}

fn invalid_accepted_message_ref() -> AgentLoopHostError {
    AgentLoopHostError::new(
        AgentLoopHostErrorKind::InvalidInvocation,
        "accepted turn objective message ref is invalid",
    )
}
