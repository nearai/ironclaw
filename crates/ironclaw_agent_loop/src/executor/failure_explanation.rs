use ironclaw_turns::{
    LoopFailureKind, LoopMessageRef,
    run_profile::{
        FinalizeAssistantMessage, LoopInlineMessage, LoopInlineMessageRole, LoopModelRequest,
        LoopPromptBundleRequest, LoopSafeSummary, ParentLoopOutput,
    },
};

use crate::state::LoopExecutionState;

use super::StageContext;

pub(super) async fn explain_failure(
    ctx: StageContext<'_>,
    state: &LoopExecutionState,
    reason_kind: LoopFailureKind,
) -> Option<LoopMessageRef> {
    if !is_explainable(reason_kind) {
        return None;
    }
    if ctx.host.observe_cancellation().is_some() {
        tracing::debug!(
            reason_kind = reason_kind.as_str(),
            "skipping failure explanation because cancellation is already requested"
        );
        return None;
    }

    let request = match build_explanation_prompt_request(ctx, state, reason_kind).await {
        Some(request) => request,
        None => return None,
    };
    let messages = match ctx.host.build_prompt_bundle(request).await {
        Ok(bundle) => bundle.messages,
        Err(error) => {
            tracing::debug!(
                reason_kind = reason_kind.as_str(),
                error_kind = error.kind.as_str(),
                safe_summary = error.safe_summary.as_str(),
                "failure explanation prompt bundle build failed"
            );
            return None;
        }
    };

    let response = match ctx
        .host
        .stream_model(LoopModelRequest {
            messages,
            surface_version: None,
            model_preference: None,
            capability_view: None,
        })
        .await
    {
        Ok(response) => response,
        Err(error) => {
            tracing::debug!(
                reason_kind = reason_kind.as_str(),
                error_kind = error.kind.as_str(),
                safe_summary = error.safe_summary.as_str(),
                "failure explanation model call failed"
            );
            return None;
        }
    };

    let reply = match response.output {
        ParentLoopOutput::AssistantReply(reply) => reply,
        ParentLoopOutput::CapabilityCalls(_) => {
            tracing::debug!(
                reason_kind = reason_kind.as_str(),
                "failure explanation model returned capability calls"
            );
            return None;
        }
    };

    match ctx
        .host
        .finalize_assistant_message(FinalizeAssistantMessage { reply })
        .await
    {
        Ok(message_ref) => Some(message_ref),
        Err(error) => {
            tracing::debug!(
                reason_kind = reason_kind.as_str(),
                error_kind = error.kind.as_str(),
                safe_summary = error.safe_summary.as_str(),
                "failure explanation transcript finalize failed"
            );
            None
        }
    }
}

async fn build_explanation_prompt_request(
    ctx: StageContext<'_>,
    state: &LoopExecutionState,
    reason_kind: LoopFailureKind,
) -> Option<LoopPromptBundleRequest> {
    let mut request = ctx
        .planner
        .context()
        .plan_context_request(state)
        .await
        .request;
    request.surface_version = None;
    request.capability_view = None;
    request.checkpoint_state_ref = None;
    request.inline_messages.push(LoopInlineMessage {
        role: LoopInlineMessageRole::System,
        safe_body: safe_summary(failure_context(state, reason_kind), reason_kind)?,
    });
    request.inline_messages.push(LoopInlineMessage {
        role: LoopInlineMessageRole::User,
        safe_body: safe_summary(final_instruction(reason_kind), reason_kind)?,
    });
    Some(request)
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
