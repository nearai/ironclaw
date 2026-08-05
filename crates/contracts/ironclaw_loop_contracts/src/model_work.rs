use serde::{Deserialize, Serialize};

use super::{
    host::{AgentLoopHostErrorKind, LoopModelRequest, LoopModelRouteSnapshot, LoopRunContext},
    model::{LoopModelGatewayError, ModelCallOutcome},
    refs::ModelProfileId,
    system_inference::{
        SystemInferenceError, SystemInferenceRequest, SystemInferenceResponse, SystemPromptSource,
        SystemTaskKind,
    },
};

const MODEL_WORK_ESTIMATED_CHARS_PER_TOKEN: u64 = 4;

/// Model-backed unit of work that needs the host policy/accounting envelope.
///
/// This is deliberately narrower than [`LoopModelRequest`]: assistant dispatch
/// and host-owned system inference share spend/policy handling, but keep their
/// execution semantics separate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelWorkRequest {
    pub kind: ModelWorkKind,
    pub model_profile_id: ModelProfileId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_model_route: Option<LoopModelRouteSnapshot>,
    pub estimated_input_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated_output_tokens: Option<u64>,
}

impl ModelWorkRequest {
    pub fn for_assistant(context: &LoopRunContext, request: &LoopModelRequest) -> Self {
        let model_profile_id = request
            .model_preference
            .as_ref()
            .unwrap_or(&context.resolved_run_profile.model_profile_id)
            .clone();
        let estimated_input_tokens = request
            .messages
            .iter()
            .map(|message| {
                (message.content_ref.as_str().len() as u64) / MODEL_WORK_ESTIMATED_CHARS_PER_TOKEN
            })
            .sum::<u64>()
            .max(64);
        Self {
            kind: ModelWorkKind::Assistant,
            model_profile_id,
            resolved_model_route: context.resolved_model_route.clone(),
            estimated_input_tokens,
            estimated_output_tokens: None,
        }
    }

    pub fn for_system_inference(
        context: &LoopRunContext,
        request: &SystemInferenceRequest,
    ) -> Self {
        let estimated_chars = request
            .identity
            .system_prompt
            .len()
            .saturating_add(request.input_text.len()) as u64;
        Self {
            kind: ModelWorkKind::SystemInference {
                task_kind: request.identity.task_kind,
                prompt_source: request.identity.prompt_source.clone(),
            },
            model_profile_id: context.resolved_run_profile.model_profile_id.clone(),
            resolved_model_route: context.resolved_model_route.clone(),
            estimated_input_tokens: (estimated_chars / MODEL_WORK_ESTIMATED_CHARS_PER_TOKEN)
                .max(64),
            estimated_output_tokens: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum ModelWorkKind {
    Assistant,
    SystemInference {
        task_kind: SystemTaskKind,
        prompt_source: SystemPromptSource,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelWorkOutcome {
    Success(ModelWorkUsage),
    Failure(AgentLoopHostErrorKind),
}

impl ModelWorkOutcome {
    pub fn from_model_call(outcome: ModelCallOutcome<'_>) -> Self {
        match outcome {
            ModelCallOutcome::Success(response) => Self::Success(ModelWorkUsage {
                output_tokens: Some(response.chunks.len() as u64),
                output_bytes: response
                    .chunks
                    .iter()
                    .map(|chunk| chunk.safe_text_delta.len() as u64)
                    .sum(),
                wall_clock_ms: 0,
            }),
            ModelCallOutcome::Failure(error) => Self::from_gateway_error(error),
        }
    }

    pub fn from_gateway_error(error: &LoopModelGatewayError) -> Self {
        Self::Failure(error.kind)
    }

    pub fn from_system_inference_result(
        result: &Result<SystemInferenceResponse, SystemInferenceError>,
    ) -> Self {
        match result {
            Ok(response) => Self::Success(ModelWorkUsage {
                output_tokens: None,
                output_bytes: response.output_text.len() as u64,
                wall_clock_ms: response.elapsed_ms,
            }),
            Err(SystemInferenceError::Cancelled) => {
                Self::Failure(AgentLoopHostErrorKind::Cancelled)
            }
            Err(SystemInferenceError::InputTooLarge) => {
                Self::Failure(AgentLoopHostErrorKind::InvalidInvocation)
            }
            Err(SystemInferenceError::Timeout) => {
                Self::Failure(AgentLoopHostErrorKind::Unavailable)
            }
            Err(SystemInferenceError::Failed { .. }) => {
                Self::Failure(AgentLoopHostErrorKind::Unavailable)
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelWorkUsage {
    pub output_tokens: Option<u64>,
    pub output_bytes: u64,
    pub wall_clock_ms: u64,
}
