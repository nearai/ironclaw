use ironclaw_safety::{
    ProviderValidationError, validate_optional_provider_metadata_text,
    validate_provider_arguments as validate_safety_provider_arguments, validate_provider_identity,
    validate_provider_token, validate_provider_tool_name as validate_safety_provider_tool_name,
};
use ironclaw_turns::run_profile::{AgentLoopHostError, AgentLoopHostErrorKind, ProviderToolCall};

pub(super) use ironclaw_safety::PROVIDER_TOOL_NAME_MAX_BYTES;

pub(super) fn validate_provider_tool_call(
    tool_call: &ProviderToolCall,
) -> Result<(), AgentLoopHostError> {
    validate_provider_identity(&tool_call.provider_id, "provider id", 512)
        .map_err(invalid_invocation)?;
    validate_provider_identity(&tool_call.provider_model_id, "provider model id", 512)
        .map_err(invalid_invocation)?;
    let turn_id = tool_call.turn_id.as_deref().ok_or_else(|| {
        AgentLoopHostError::new(
            AgentLoopHostErrorKind::InvalidInvocation,
            "provider tool call is missing a provider turn id",
        )
    })?;
    validate_provider_token(turn_id, "provider turn id", 512).map_err(invalid_invocation)?;
    validate_provider_token(&tool_call.id, "provider call id", 512).map_err(invalid_invocation)?;
    validate_provider_tool_name(&tool_call.name)?;
    validate_provider_arguments(&tool_call.arguments)?;
    validate_optional_provider_metadata_text(
        tool_call.response_reasoning.as_deref(),
        "provider response reasoning",
        4096,
    )
    .map_err(invalid_invocation)?;
    validate_optional_provider_metadata_text(
        tool_call.reasoning.as_deref(),
        "provider reasoning",
        4096,
    )
    .map_err(invalid_invocation)?;
    validate_optional_provider_metadata_text(
        tool_call.signature.as_deref(),
        "provider signature",
        4096,
    )
    .map_err(invalid_invocation)?;
    Ok(())
}

pub(super) fn validate_provider_tool_name(value: &str) -> Result<(), AgentLoopHostError> {
    validate_safety_provider_tool_name(value).map_err(invalid_invocation)
}

pub(super) fn validate_provider_arguments(
    arguments: &serde_json::Value,
) -> Result<(), AgentLoopHostError> {
    validate_safety_provider_arguments(arguments).map_err(invalid_invocation)
}

fn invalid_invocation(error: ProviderValidationError) -> AgentLoopHostError {
    AgentLoopHostError::new(AgentLoopHostErrorKind::InvalidInvocation, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_tool_call_validation_rejects_provider_unsafe_tool_name() {
        let mut call = provider_tool_call();
        call.name = "demo.echo".to_string();

        let error = validate_provider_tool_call(&call).expect_err("unsafe name rejected");
        assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
    }

    #[test]
    fn provider_tool_call_validation_requires_turn_id() {
        let mut call = provider_tool_call();
        call.turn_id = None;

        let error = validate_provider_tool_call(&call).expect_err("missing turn id rejected");
        assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
    }

    #[test]
    fn provider_tool_call_validation_rejects_sensitive_metadata() {
        let mut call = provider_tool_call();
        let api_key = format!("sk-proj-{}", "a".repeat(24));
        call.arguments = serde_json::json!({"password": api_key});
        assert!(validate_provider_tool_call(&call).is_err());

        let mut call = provider_tool_call();
        call.reasoning = Some("provider error included traceback".to_string());
        assert!(validate_provider_tool_call(&call).is_err());
    }

    #[test]
    fn provider_tool_call_validation_allows_multiline_argument_text() {
        let mut call = provider_tool_call();
        call.arguments = serde_json::json!({
            "content": "---\nname: pasted-skill\n---\n\nMention an API key placeholder, but no secret.\n"
        });

        validate_provider_tool_call(&call).expect("multiline argument text should be valid");
    }

    #[test]
    fn provider_tool_call_validation_rejects_non_whitespace_argument_controls() {
        let mut call = provider_tool_call();
        call.arguments = serde_json::json!({"content":"line one\u{0001}line two"});

        let error = validate_provider_tool_call(&call)
            .expect_err("non-whitespace control character should fail");
        assert_eq!(error.kind, AgentLoopHostErrorKind::InvalidInvocation);
    }

    fn provider_tool_call() -> ProviderToolCall {
        ProviderToolCall {
            provider_id: "provider".to_string(),
            provider_model_id: "model".to_string(),
            turn_id: Some("turn_1".to_string()),
            id: "call_1".to_string(),
            name: "demo__echo".to_string(),
            arguments: serde_json::json!({"message":"hello"}),
            response_reasoning: None,
            reasoning: None,
            signature: None,
        }
    }
}
