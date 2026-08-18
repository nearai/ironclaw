//! Host-owned structured-output guidance and finalization prompt material.
//!
//! The durable output contract is injected into the work-phase system
//! instruction and reused by the native finalization request. The loop never
//! exposes a synthetic result capability and the candidate remains ephemeral
//! until the ordinary transcript finalization boundary.

use async_trait::async_trait;
use ironclaw_host_api::output::OutputContract;
use ironclaw_loop_contracts::{
    AgentLoopHostError, AgentLoopHostErrorKind, LoopInlineMessage, LoopInlineMessageBody,
    LoopInlineMessageRole, LoopPromptBundleRequest, LoopPromptPort, LoopRunContext,
};
use std::sync::Arc;

pub const STRUCTURED_OUTPUT_GUIDANCE_PROMPT: &str =
    include_str!("../prompts/structured_output_guidance.md");
pub const STRUCTURED_OUTPUT_FINALIZATION_PROMPT: &str =
    include_str!("../prompts/structured_output_finalization.md");

/// Render the host-owned work-phase instruction with the durable schema.
pub fn structured_output_guidance(schema: &serde_json::Value) -> String {
    format!(
        "{}\n\nDeclared output schema (the final JSON must validate against it):\n{}",
        STRUCTURED_OUTPUT_GUIDANCE_PROMPT.trim(),
        schema
    )
}

fn json_object_guidance() -> String {
    format!(
        "{}\n\nDeclared output mode: the final response must be one valid JSON object. Do not emit Markdown fences, explanations, or additional text.",
        STRUCTURED_OUTPUT_GUIDANCE_PROMPT.trim()
    )
}

/// Add the immutable run's structured-output guidance to a prompt request.
/// Assistant-message runs are byte-for-byte unchanged.
pub fn add_structured_output_guidance(
    output_contract: &OutputContract,
    mut request: LoopPromptBundleRequest,
) -> Result<LoopPromptBundleRequest, AgentLoopHostError> {
    let guidance_text = match output_contract {
        OutputContract::JsonSchema { schema, .. } => structured_output_guidance(schema),
        OutputContract::JsonObject => json_object_guidance(),
        OutputContract::AssistantMessage => return Ok(request),
    };
    let guidance = LoopInlineMessageBody::new(guidance_text).map_err(|reason| {
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::Invalid,
            format!("structured-output guidance is invalid: {reason}"),
        )
    })?;
    request.inline_messages.insert(
        0,
        LoopInlineMessage {
            role: LoopInlineMessageRole::System,
            safe_body: guidance,
        },
    );
    Ok(request)
}

/// Prompt decorator for structured-output runs.  It is host-owned and
/// loop-family agnostic; the agent loop only sees the ordinary prompt port.
pub struct StructuredOutputLoopPromptPort {
    inner: Arc<dyn LoopPromptPort>,
    run_context: LoopRunContext,
}

impl StructuredOutputLoopPromptPort {
    pub fn new(inner: Arc<dyn LoopPromptPort>, run_context: LoopRunContext) -> Self {
        Self { inner, run_context }
    }
}

#[async_trait]
impl LoopPromptPort for StructuredOutputLoopPromptPort {
    async fn build_prompt_bundle(
        &self,
        request: LoopPromptBundleRequest,
    ) -> Result<ironclaw_loop_contracts::LoopPromptBundle, AgentLoopHostError> {
        let request = add_structured_output_guidance(&self.run_context.output_contract, request)?;
        self.inner.build_prompt_bundle(request).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_loop_contracts::{LoopPromptBundleRequest, PromptMode};

    fn request() -> LoopPromptBundleRequest {
        LoopPromptBundleRequest {
            mode: PromptMode::TextOnly,
            context_cursor: None,
            surface_version: None,
            capability_view: None,
            checkpoint_state_ref: None,
            max_messages: Some(8),
            inline_messages: Vec::new(),
        }
    }

    #[test]
    fn structured_contract_adds_one_system_guidance_message_with_schema() {
        let request = add_structured_output_guidance(
            &OutputContract::JsonSchema {
                name: "suggestions".to_string(),
                schema: serde_json::json!({"type": "object", "required": ["items"]}),
            },
            request(),
        )
        .expect("guidance is valid");

        assert_eq!(request.inline_messages.len(), 1);
        assert_eq!(
            request.inline_messages[0].role,
            LoopInlineMessageRole::System
        );
        assert!(
            request.inline_messages[0]
                .safe_body
                .as_str()
                .contains("\"required\":[\"items\"]")
        );
    }

    #[test]
    fn assistant_message_contract_does_not_change_prompt_request() {
        let request = request();
        let guided =
            add_structured_output_guidance(&OutputContract::AssistantMessage, request.clone())
                .expect("assistant contract remains valid");
        assert_eq!(guided, request);
    }

    #[test]
    fn schema_at_admission_bound_fits_guidance_message_bound() {
        let schema_bytes = ironclaw_threads::PREPARED_OUTPUT_SCHEMA_MAX_BYTES;
        let schema = serde_json::json!({
            "x": "a".repeat(schema_bytes - br#"{"x":""}"#.len())
        });
        ironclaw_threads::validate_output_schema(&schema)
            .expect("schema at the admission bound should be accepted");

        let guided = add_structured_output_guidance(
            &OutputContract::JsonSchema {
                name: "bounded".to_string(),
                schema,
            },
            request(),
        )
        .expect("guidance wrapper must fit the inline-message bound");
        assert_eq!(guided.inline_messages.len(), 1);
    }
}
