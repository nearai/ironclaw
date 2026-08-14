//! Prepared-context submission port for chat completions (unbound-turns
//! design §4 / one-engine §7 phase 1 adoption).
//!
//! A request whose contract today's conversation path silently mistreats —
//! `response_format` json schema (parsed and dropped) or assistant/tool
//! history (flattened into one JSON string) — is accepted through the
//! engine's shared `accept_prepared_context` door instead: the message list
//! seeds faithfully, the output contract journals beside it, and the refless
//! submit derives the unbound run profile at admission. This crate owns only
//! the port vocabulary and the lane decision; the implementation lives in
//! composition, which holds the thread service and coordinator.

use async_trait::async_trait;
use ironclaw_host_api::prepared_context::OutputContract;
use ironclaw_product_contracts::inbound::ProductInboundAck;

use crate::OpenAiCompatHttpError;
use crate::chat::{OpenAiChatCompletionRequest, OpenAiChatMessage, OpenAiChatMessageRole};
use crate::refs::OpenAiCompatActorScope;

/// One prepared-lane submission: everything composition needs to seed the
/// caller's message list and submit the refless turn. The public completion
/// id doubles as the unbound thread id, exactly as the conversation lane
/// used it, so retrieval and ref binding stay id-stable.
#[derive(Debug, Clone)]
pub struct OpenAiCompatPreparedTurnRequest {
    pub scope: OpenAiCompatActorScope,
    pub public_id: String,
    pub messages: Vec<OpenAiChatMessage>,
    pub output: OutputContract,
    pub requested_model: Option<String>,
}

/// Accept + submit through the prepared-context door. Returns the SAME
/// `ProductInboundAck::Accepted` shape the conversation submit produces so
/// the ref-store replay/idempotency machinery downstream is untouched.
#[async_trait]
pub trait OpenAiCompatPreparedTurnPort: Send + Sync {
    async fn accept_and_submit(
        &self,
        request: OpenAiCompatPreparedTurnRequest,
    ) -> Result<ProductInboundAck, OpenAiCompatHttpError>;
}

/// Parse `response_format` into the engine output contract.
///
/// - absent / `{"type":"text"}` → `None` (assistant-message contract)
/// - `{"type":"json_schema","json_schema":{"schema":…}}` → strict schema
/// - `{"type":"json_object"}` → the permissive object schema
/// - anything else → typed invalid-request (never silently dropped)
pub(crate) fn parse_response_format(
    response_format: Option<&serde_json::Value>,
) -> Result<Option<OutputContract>, OpenAiCompatHttpError> {
    let Some(value) = response_format else {
        return Ok(None);
    };
    let kind = value.get("type").and_then(serde_json::Value::as_str);
    match kind {
        Some("text") => Ok(None),
        Some("json_object") => Ok(Some(OutputContract::JsonSchema {
            schema: serde_json::json!({"type": "object"}),
        })),
        Some("json_schema") => {
            let schema = value
                .get("json_schema")
                .and_then(|body| body.get("schema"))
                .cloned()
                .ok_or_else(|| {
                    OpenAiCompatHttpError::invalid_request(Some(
                        "response_format.json_schema.schema".to_string(),
                    ))
                })?;
            Ok(Some(OutputContract::JsonSchema { schema }))
        }
        _ => Err(OpenAiCompatHttpError::invalid_request(Some(
            "response_format.type".to_string(),
        ))),
    }
}

/// Does the request carry assistant tool-call / tool-result history?
pub(crate) fn has_tool_history(messages: &[OpenAiChatMessage]) -> bool {
    messages.iter().any(|message| {
        matches!(message.role, OpenAiChatMessageRole::Tool)
            || message
                .tool_calls
                .as_ref()
                .is_some_and(|calls| !calls.is_empty())
    })
}

/// The prepared-lane decision for a NON-STREAMING request. The lane exists
/// exactly where the conversation path mistreats the request's contract:
/// a declared output schema, or tool history that would otherwise be
/// flattened. Requests declaring live client tools stay on the conversation
/// lane (the external-tool park/resume flow lives there); combining client
/// tools with a structured output contract is rejected loudly instead of
/// half-honored.
pub(crate) fn prepared_lane_output(
    request: &OpenAiChatCompletionRequest,
) -> Result<Option<OutputContract>, OpenAiCompatHttpError> {
    let output = parse_response_format(request.response_format.as_ref())?;
    let declares_tools = request
        .tools
        .as_ref()
        .is_some_and(|tools| !tools.is_empty());
    if output.is_some() && declares_tools {
        return Err(OpenAiCompatHttpError::invalid_request(Some(
            "response_format with tools is not supported yet".to_string(),
        )));
    }
    if request.stream.unwrap_or(false) {
        if output.is_some() {
            return Err(OpenAiCompatHttpError::invalid_request(Some(
                "response_format json output with stream is not supported yet".to_string(),
            )));
        }
        // Streaming keeps the conversation lane wholesale (its projection
        // subscription is conversation-scoped); tool history streams exactly
        // as it did before.
        return Ok(None);
    }
    if let Some(output) = output {
        return Ok(Some(output));
    }
    if has_tool_history(&request.messages) && !declares_tools {
        return Ok(Some(OutputContract::AssistantMessage));
    }
    Ok(None)
}

/// Cheap deterministic pre-validation for a prepared-lane request, run
/// BEFORE the idempotency reservation so an invalid body never burns the
/// caller's idempotency key (the same ordering the conversation lane's
/// payload build provides). The accept door remains the authoritative
/// validator; everything here is a faithful subset.
pub(crate) fn prepared_pre_validate(
    messages: &[OpenAiChatMessage],
) -> Result<(), OpenAiCompatHttpError> {
    fn text_shape_ok(content: Option<&serde_json::Value>) -> bool {
        match content {
            None | Some(serde_json::Value::String(_)) => true,
            Some(serde_json::Value::Array(parts)) => parts
                .iter()
                .all(|part| part.get("type").and_then(serde_json::Value::as_str) == Some("text")),
            Some(_) => false,
        }
    }
    // Mirrors `ironclaw_safety::validate_provider_token` (the authoritative
    // check at the accept door): 1..=512 bytes of [A-Za-z0-9_\-.:].
    fn call_id_ok(value: &str) -> bool {
        !value.is_empty()
            && value.len() <= 512
            && value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b':'))
    }
    if messages.is_empty() {
        return Err(OpenAiCompatHttpError::invalid_request(Some(
            "messages".to_string(),
        )));
    }
    for message in messages {
        if !text_shape_ok(message.content.as_ref()) {
            return Err(OpenAiCompatHttpError::invalid_request(Some(
                "only text content parts are supported with structured output or tool history"
                    .to_string(),
            )));
        }
        if matches!(message.role, OpenAiChatMessageRole::Tool) {
            let Some(call_id) = message.tool_call_id.as_deref() else {
                return Err(OpenAiCompatHttpError::invalid_request(Some(
                    "messages[].tool_call_id".to_string(),
                )));
            };
            if !call_id_ok(call_id) {
                return Err(OpenAiCompatHttpError::invalid_request(Some(
                    "messages[].tool_call_id".to_string(),
                )));
            }
        }
        for call in message.tool_calls.iter().flatten() {
            if !call_id_ok(&call.id) {
                return Err(OpenAiCompatHttpError::invalid_request(Some(
                    "messages[].tool_calls[].id".to_string(),
                )));
            }
            if ironclaw_host_api::ids::CapabilityId::new(format!(
                "external_tool.{}",
                call.function.name.to_ascii_lowercase()
            ))
            .is_err()
            {
                return Err(OpenAiCompatHttpError::invalid_request(Some(
                    "messages[].tool_calls[].function.name".to_string(),
                )));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn base_request(messages: Vec<OpenAiChatMessage>) -> OpenAiChatCompletionRequest {
        OpenAiChatCompletionRequest {
            model: "gpt-test".to_string(),
            messages,
            stream: None,
            temperature: None,
            top_p: None,
            max_tokens: None,
            max_completion_tokens: None,
            stop: None,
            tools: None,
            tool_choice: None,
            response_format: None,
            user: None,
            metadata: None,
        }
    }

    fn user_message(text: &str) -> OpenAiChatMessage {
        OpenAiChatMessage {
            role: OpenAiChatMessageRole::User,
            content: Some(json!(text)),
            name: None,
            tool_call_id: None,
            tool_calls: None,
        }
    }

    #[test]
    fn plain_requests_stay_on_the_conversation_lane() {
        let request = base_request(vec![user_message("hi")]);
        assert!(prepared_lane_output(&request).expect("lane").is_none());
    }

    #[test]
    fn json_schema_maps_to_the_engine_output_contract() {
        let mut request = base_request(vec![user_message("classify")]);
        request.response_format = Some(json!({
            "type": "json_schema",
            "json_schema": {"name": "s", "schema": {"type": "object"}}
        }));
        match prepared_lane_output(&request).expect("lane") {
            Some(OutputContract::JsonSchema { schema }) => {
                assert_eq!(schema, json!({"type": "object"}));
            }
            other => panic!("expected a json schema contract, got {other:?}"),
        }
    }

    #[test]
    fn json_object_maps_to_the_permissive_object_schema() {
        let mut request = base_request(vec![user_message("classify")]);
        request.response_format = Some(json!({"type": "json_object"}));
        assert!(matches!(
            prepared_lane_output(&request).expect("lane"),
            Some(OutputContract::JsonSchema { .. })
        ));
    }

    #[test]
    fn tool_history_without_declared_tools_takes_the_prepared_lane() {
        let mut request = base_request(vec![user_message("continue")]);
        request.messages.push(OpenAiChatMessage {
            role: OpenAiChatMessageRole::Tool,
            content: Some(json!("result body")),
            name: None,
            tool_call_id: Some("call_1".to_string()),
            tool_calls: None,
        });
        assert!(matches!(
            prepared_lane_output(&request).expect("lane"),
            Some(OutputContract::AssistantMessage)
        ));
    }

    #[test]
    fn declared_tools_keep_the_conversation_lane_and_reject_schema_combos() {
        let mut request = base_request(vec![user_message("go")]);
        request.tools = Some(vec![]);
        assert!(prepared_lane_output(&request).expect("lane").is_none());

        request.tools = Some(vec![crate::chat::OpenAiChatTool {
            kind: crate::chat::OpenAiChatToolKind::Function,
            function: crate::chat::OpenAiChatFunction {
                name: "lookup".to_string(),
                description: None,
                parameters: None,
                strict: None,
            },
        }]);
        request.response_format = Some(json!({"type": "json_object"}));
        assert!(prepared_lane_output(&request).is_err());
    }

    #[test]
    fn streaming_rejects_json_output_and_keeps_the_conversation_lane() {
        let mut request = base_request(vec![user_message("go")]);
        request.stream = Some(true);
        request.response_format = Some(json!({"type": "json_object"}));
        assert!(prepared_lane_output(&request).is_err());

        let mut request = base_request(vec![user_message("go")]);
        request.stream = Some(true);
        assert!(prepared_lane_output(&request).expect("lane").is_none());
    }

    #[test]
    fn unknown_response_format_is_rejected_loudly() {
        let mut request = base_request(vec![user_message("go")]);
        request.response_format = Some(json!({"type": "xml"}));
        assert!(parse_response_format(request.response_format.as_ref()).is_err());
    }
}
