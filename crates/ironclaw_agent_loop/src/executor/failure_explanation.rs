use std::time::Duration;

use ironclaw_turns::{
    LoopFailureKind, LoopMessageRef,
    run_profile::{
        AgentLoopHostError, AgentLoopHostErrorKind, FinalizeAssistantMessage, LoopInlineMessage,
        LoopInlineMessageRole, LoopModelRequest, LoopModelResponse, LoopPromptBundleRequest,
        LoopSafeSummary, ParentLoopOutput, PromptMode,
    },
};

use crate::state::LoopExecutionState;

use super::{AgentLoopExecutorError, StageContext};

const FAILURE_EXPLANATION_MODEL_DEADLINE: Duration = Duration::from_secs(10);

/// Best-effort explanation plus state attachment: the only call path stages
/// should use. Records the terminal reason before checkpointing, finalizes the
/// explanation (when produced), and records its ref in `state.assistant_refs`
/// so checkpoint and exit evidence stay consistent.
pub(super) async fn attach_failure_explanation(
    ctx: StageContext<'_>,
    state: &mut LoopExecutionState,
    reason_kind: LoopFailureKind,
) -> Result<Option<LoopMessageRef>, AgentLoopExecutorError> {
    state.recent_failure_kinds.push(reason_kind);
    let explanation_message_ref = explain_failure(ctx, state, reason_kind).await?;
    if let Some(message_ref) = explanation_message_ref.as_ref() {
        state.assistant_refs.push(message_ref.clone());
    }
    Ok(explanation_message_ref)
}

pub(super) async fn explain_failure(
    ctx: StageContext<'_>,
    state: &LoopExecutionState,
    reason_kind: LoopFailureKind,
) -> Result<Option<LoopMessageRef>, AgentLoopExecutorError> {
    if !is_explainable(reason_kind) {
        return Ok(None);
    }
    if ctx.host.observe_cancellation().is_some() {
        tracing::debug!(
            reason_kind = reason_kind.as_str(),
            "skipping failure explanation because cancellation is already requested"
        );
        return Ok(None);
    }

    let request = match build_explanation_prompt_request(state, reason_kind) {
        Some(request) => request,
        None => return Ok(None),
    };
    let messages = match ctx.host.build_prompt_bundle(request).await {
        Ok(bundle) => bundle.messages,
        Err(error) => {
            if error.kind == AgentLoopHostErrorKind::Cancelled {
                return Err(AgentLoopExecutorError::Cancelled);
            }
            tracing::debug!(
                reason_kind = reason_kind.as_str(),
                error_kind = error.kind.as_str(),
                safe_summary = error.safe_summary.as_str(),
                "failure explanation prompt bundle build failed"
            );
            return Ok(None);
        }
    };

    let response = match await_explanation_model_call(
        ctx,
        ctx.host.stream_model(LoopModelRequest {
            inline_messages: Vec::new(),
            messages,
            surface_version: None,
            model_preference: None,
            capability_view: None,
        }),
    )
    .await
    {
        ExplanationModelCallOutcome::Completed(Ok(response)) => response,
        ExplanationModelCallOutcome::Completed(Err(error)) => {
            if error.kind == AgentLoopHostErrorKind::Cancelled {
                return Err(AgentLoopExecutorError::Cancelled);
            }
            tracing::debug!(
                reason_kind = reason_kind.as_str(),
                error_kind = error.kind.as_str(),
                safe_summary = error.safe_summary.as_str(),
                "failure explanation model call failed"
            );
            return Ok(None);
        }
        ExplanationModelCallOutcome::TimedOut => {
            tracing::debug!(
                reason_kind = reason_kind.as_str(),
                deadline_ms = FAILURE_EXPLANATION_MODEL_DEADLINE.as_millis(),
                "failure explanation model call timed out"
            );
            return Ok(None);
        }
        ExplanationModelCallOutcome::Cancelled => return Err(AgentLoopExecutorError::Cancelled),
    };

    let reply = match response.output {
        ParentLoopOutput::AssistantReply(reply) => reply,
        ParentLoopOutput::CapabilityCalls(_) => {
            tracing::debug!(
                reason_kind = reason_kind.as_str(),
                "failure explanation model returned capability calls"
            );
            return Ok(None);
        }
    };

    match ctx
        .host
        .finalize_assistant_message(FinalizeAssistantMessage { reply })
        .await
    {
        Ok(message_ref) => Ok(Some(message_ref)),
        Err(error) => {
            if error.kind == AgentLoopHostErrorKind::Cancelled {
                return Err(AgentLoopExecutorError::Cancelled);
            }
            tracing::debug!(
                reason_kind = reason_kind.as_str(),
                error_kind = error.kind.as_str(),
                safe_summary = error.safe_summary.as_str(),
                "failure explanation transcript finalize failed"
            );
            Ok(None)
        }
    }
}

enum ExplanationModelCallOutcome {
    Completed(Result<LoopModelResponse, AgentLoopHostError>),
    TimedOut,
    Cancelled,
}

async fn await_explanation_model_call<F>(
    ctx: StageContext<'_>,
    call: F,
) -> ExplanationModelCallOutcome
where
    F: std::future::Future<Output = Result<LoopModelResponse, AgentLoopHostError>>,
{
    tokio::pin!(call);
    let timeout = tokio::time::sleep(FAILURE_EXPLANATION_MODEL_DEADLINE);
    tokio::pin!(timeout);
    let cancellation = ctx.host.cancellation_requested();
    tokio::pin!(cancellation);

    tokio::select! {
        result = &mut call => ExplanationModelCallOutcome::Completed(result),
        _ = &mut timeout => ExplanationModelCallOutcome::TimedOut,
        _signal = &mut cancellation => ExplanationModelCallOutcome::Cancelled,
    }
}

fn build_explanation_prompt_request(
    state: &LoopExecutionState,
    reason_kind: LoopFailureKind,
) -> Option<LoopPromptBundleRequest> {
    Some(LoopPromptBundleRequest {
        mode: PromptMode::TextOnly,
        context_cursor: None,
        surface_version: None,
        capability_view: None,
        checkpoint_state_ref: None,
        max_messages: Some(0),
        inline_messages: vec![
            LoopInlineMessage {
                role: LoopInlineMessageRole::System,
                safe_body: safe_summary(failure_context(state, reason_kind), reason_kind)?,
            },
            LoopInlineMessage {
                role: LoopInlineMessageRole::User,
                safe_body: safe_summary(final_instruction(reason_kind), reason_kind)?,
            },
        ],
    })
}

fn failure_context(state: &LoopExecutionState, reason_kind: LoopFailureKind) -> String {
    let recent_failures = state
        .recent_failure_kinds
        .iter()
        .map(|kind| kind.as_str())
        .collect::<Vec<_>>();
    let recent_failures = if recent_failures.is_empty() {
        "none".to_string()
    } else {
        recent_failures.join(" ")
    };
    format!(
        "Failure context reason {} iteration {} finalized assistant messages {} recent failures {}",
        reason_kind.as_str(),
        state.iteration,
        state.assistant_refs.len(),
        recent_failures
    )
}

fn final_instruction(reason_kind: LoopFailureKind) -> String {
    format!(
        "The run is ending due to {}. Write a short honest message to the user explaining what happened what was completed so far and what they can do next. Do not invent details.",
        reason_kind.as_str()
    )
}

fn safe_summary(value: String, reason_kind: LoopFailureKind) -> Option<LoopSafeSummary> {
    match LoopSafeSummary::new(value) {
        Ok(summary) => Some(summary),
        Err(error) => {
            tracing::debug!(
                reason_kind = reason_kind.as_str(),
                validation_error = error,
                "failure explanation prompt text was not loop safe"
            );
            None
        }
    }
}

fn is_explainable(reason_kind: LoopFailureKind) -> bool {
    matches!(
        reason_kind,
        LoopFailureKind::CapabilityProtocolError
            | LoopFailureKind::IterationLimit
            | LoopFailureKind::PolicyDenied
            | LoopFailureKind::NoProgressDetected
            | LoopFailureKind::CompactionUnavailable
            | LoopFailureKind::InvalidModelOutput
    )
}
