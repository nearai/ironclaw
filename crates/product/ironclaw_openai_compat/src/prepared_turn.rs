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

/// Does the request replay a transcript rather than open a fresh chat?
/// Any assistant or tool row, or more than one user turn, means the caller
/// owns the history — the stateless-replay shape the prepared lane seeds
/// faithfully (the retired path flattened it into one JSON string).
pub(crate) fn has_replayed_history(messages: &[OpenAiChatMessage]) -> bool {
    let mut user_turns = 0usize;
    for message in messages {
        match message.role {
            OpenAiChatMessageRole::Tool | OpenAiChatMessageRole::Assistant => return true,
            OpenAiChatMessageRole::User => user_turns += 1,
            OpenAiChatMessageRole::System | OpenAiChatMessageRole::Developer => {}
        }
        if message
            .tool_calls
            .as_ref()
            .is_some_and(|calls| !calls.is_empty())
        {
            return true;
        }
    }
    user_turns > 1
}

/// The prepared-lane decision for a NON-STREAMING request. The lane exists
/// exactly where the conversation path mistreats the request's contract:
/// a declared output schema, or replayed history (assistant/tool rows or
/// multiple user turns) that would otherwise be flattened into one JSON
/// string. Requests declaring live client tools stay on the conversation
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
    if has_replayed_history(&request.messages) && !declares_tools {
        return Ok(Some(OutputContract::AssistantMessage));
    }
    Ok(None)
}

/// Mirrors of the accept door's deterministic bounds
/// (`ironclaw_llm::agent_message`, re-exported via
/// `ironclaw_threads::agent_message`); this crate cannot depend on the llm
/// crate in production, so the values are pinned by the equality test below.
pub(crate) const PREPARED_TEXT_PART_MAX_BYTES: usize = 64 * 1024;
pub(crate) const PREPARED_LIST_MAX_BYTES: usize = 256 * 1024;
pub(crate) const PREPARED_TOOL_ARGUMENTS_MAX_BYTES: usize = 64 * 1024;
pub(crate) const PREPARED_LIST_MAX_MESSAGES: usize = 128;

/// Cheap deterministic pre-validation for a prepared-lane request, run
/// BEFORE the idempotency reservation so an invalid body never burns the
/// caller's idempotency key (the same ordering the conversation lane's
/// payload build provides). The accept door remains the authoritative
/// validator; everything here is a faithful mirror of its DETERMINISTIC
/// rejections — counts, byte budgets, call-id grammar, tool-name mapping,
/// and tool-call/result pairing — so a body the door would refuse never
/// reaches the reservation.
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
    if messages.len() > crate::chat_workflow::MAX_CHAT_COMPLETION_MESSAGES {
        return Err(OpenAiCompatHttpError::invalid_request(Some(format!(
            "messages exceeds the {} message limit",
            crate::chat_workflow::MAX_CHAT_COMPLETION_MESSAGES
        ))));
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

    // Byte budgets and pairing, mirroring the seed shapes the mapping
    // produces: system/developer rows fold into ONE system prompt; every
    // other message becomes one text part; tool-call arguments serialize
    // as parsed JSON when they parse, else as the raw string.
    fn joined_text_len(content: Option<&serde_json::Value>) -> usize {
        match content {
            None => 0,
            Some(serde_json::Value::String(text)) => text.len(),
            Some(serde_json::Value::Array(parts)) => parts
                .iter()
                .map(|part| {
                    part.get("text")
                        .and_then(serde_json::Value::as_str)
                        .map(str::len)
                        .unwrap_or(0)
                })
                .sum::<usize>()
                .saturating_add(parts.len().saturating_sub(1).saturating_mul(2)),
            Some(_) => 0,
        }
    }
    let mut seeded_messages = 0usize;
    let mut system_bytes = 0usize;
    let mut total_bytes = 0usize;
    let mut open_calls: Vec<&str> = Vec::new();
    for message in messages {
        let text_len = joined_text_len(message.content.as_ref());
        match message.role {
            OpenAiChatMessageRole::System | OpenAiChatMessageRole::Developer => {
                system_bytes = system_bytes.saturating_add(text_len);
                continue;
            }
            _ => {}
        }
        seeded_messages += 1;
        if text_len > PREPARED_TEXT_PART_MAX_BYTES {
            return Err(OpenAiCompatHttpError::invalid_request(Some(format!(
                "messages[].content exceeds the {PREPARED_TEXT_PART_MAX_BYTES} byte text budget"
            ))));
        }
        total_bytes = total_bytes.saturating_add(text_len);
        if matches!(message.role, OpenAiChatMessageRole::Tool) {
            let call_id = message.tool_call_id.as_deref().unwrap_or_default();
            let Some(open_at) = open_calls.iter().position(|open| *open == call_id) else {
                return Err(OpenAiCompatHttpError::invalid_request(Some(
                    "messages[].tool_call_id does not answer a prior assistant tool call"
                        .to_string(),
                )));
            };
            open_calls.remove(open_at);
        }
        for call in message.tool_calls.iter().flatten() {
            let serialized_arguments =
                serde_json::from_str::<serde_json::Value>(&call.function.arguments)
                    .map(|value| value.to_string())
                    .unwrap_or_else(|_| {
                        serde_json::Value::String(call.function.arguments.clone()).to_string()
                    });
            if serialized_arguments.len() > PREPARED_TOOL_ARGUMENTS_MAX_BYTES {
                return Err(OpenAiCompatHttpError::invalid_request(Some(format!(
                    "messages[].tool_calls[].function.arguments exceeds the \
                     {PREPARED_TOOL_ARGUMENTS_MAX_BYTES} byte budget"
                ))));
            }
            total_bytes = total_bytes.saturating_add(serialized_arguments.len());
            open_calls.push(call.id.as_str());
        }
    }
    if !open_calls.is_empty() {
        return Err(OpenAiCompatHttpError::invalid_request(Some(
            "messages[].tool_calls without a matching tool result".to_string(),
        )));
    }
    if seeded_messages > PREPARED_LIST_MAX_MESSAGES {
        return Err(OpenAiCompatHttpError::invalid_request(Some(format!(
            "messages exceeds the {PREPARED_LIST_MAX_MESSAGES} seeded message limit"
        ))));
    }
    if system_bytes > PREPARED_TEXT_PART_MAX_BYTES {
        return Err(OpenAiCompatHttpError::invalid_request(Some(format!(
            "system/developer content exceeds the {PREPARED_TEXT_PART_MAX_BYTES} byte budget"
        ))));
    }
    if total_bytes > PREPARED_LIST_MAX_BYTES {
        return Err(OpenAiCompatHttpError::invalid_request(Some(format!(
            "messages exceed the {PREPARED_LIST_MAX_BYTES} byte request budget"
        ))));
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
    fn multi_turn_text_history_takes_the_prepared_lane() {
        let mut request = base_request(vec![user_message("first question")]);
        request.messages.push(OpenAiChatMessage {
            role: OpenAiChatMessageRole::Assistant,
            content: Some(json!("first answer")),
            name: None,
            tool_call_id: None,
            tool_calls: None,
        });
        request.messages.push(user_message("follow-up"));
        assert!(matches!(
            prepared_lane_output(&request).expect("lane"),
            Some(OutputContract::AssistantMessage)
        ));

        // Two user turns with no assistant row is still replayed history.
        let request = base_request(vec![user_message("part one"), user_message("part two")]);
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
    fn mirrored_bounds_match_the_accept_door_constants() {
        use ironclaw_threads::agent_message as door;
        assert_eq!(
            PREPARED_TEXT_PART_MAX_BYTES,
            door::AGENT_MESSAGE_TEXT_PART_MAX_BYTES
        );
        assert_eq!(PREPARED_LIST_MAX_BYTES, door::AGENT_MESSAGE_LIST_MAX_BYTES);
        assert_eq!(
            PREPARED_TOOL_ARGUMENTS_MAX_BYTES,
            door::AGENT_MESSAGE_TOOL_ARGUMENTS_MAX_BYTES
        );
        assert_eq!(
            PREPARED_LIST_MAX_MESSAGES,
            door::AGENT_MESSAGE_LIST_MAX_MESSAGES
        );
    }

    #[test]
    fn pre_validation_mirrors_the_door_bounds_and_pairing() {
        // 129 seeded messages: rejected before any reservation could burn.
        let messages: Vec<OpenAiChatMessage> = (0..129).map(|_| user_message("hi")).collect();
        assert!(prepared_pre_validate(&messages).is_err());

        // Oversized single text part.
        let messages = vec![user_message(&"x".repeat(PREPARED_TEXT_PART_MAX_BYTES + 1))];
        assert!(prepared_pre_validate(&messages).is_err());

        // A tool result answering no prior call.
        let messages = vec![
            user_message("go"),
            OpenAiChatMessage {
                role: OpenAiChatMessageRole::Tool,
                content: Some(json!("result")),
                name: None,
                tool_call_id: Some("call_orphan".to_string()),
                tool_calls: None,
            },
        ];
        assert!(prepared_pre_validate(&messages).is_err());

        // A tool call with no answering result.
        let messages = vec![
            user_message("go"),
            OpenAiChatMessage {
                role: OpenAiChatMessageRole::Assistant,
                content: None,
                name: None,
                tool_call_id: None,
                tool_calls: Some(vec![crate::chat::OpenAiChatToolCall {
                    id: "call_unanswered".to_string(),
                    kind: crate::chat::OpenAiChatToolKind::Function,
                    function: crate::chat::OpenAiChatToolCallFunction {
                        name: "lookup".to_string(),
                        arguments: "{}".to_string(),
                    },
                }]),
            },
        ];
        assert!(prepared_pre_validate(&messages).is_err());

        // A well-formed paired round passes.
        let messages = vec![
            user_message("go"),
            OpenAiChatMessage {
                role: OpenAiChatMessageRole::Assistant,
                content: None,
                name: None,
                tool_call_id: None,
                tool_calls: Some(vec![crate::chat::OpenAiChatToolCall {
                    id: "call_1".to_string(),
                    kind: crate::chat::OpenAiChatToolKind::Function,
                    function: crate::chat::OpenAiChatToolCallFunction {
                        name: "lookup".to_string(),
                        arguments: "{\"q\":1}".to_string(),
                    },
                }]),
            },
            OpenAiChatMessage {
                role: OpenAiChatMessageRole::Tool,
                content: Some(json!("found it")),
                name: None,
                tool_call_id: Some("call_1".to_string()),
                tool_calls: None,
            },
            user_message("continue"),
        ];
        prepared_pre_validate(&messages).expect("paired history passes pre-validation");
    }

    #[test]
    fn unknown_response_format_is_rejected_loudly() {
        let mut request = base_request(vec![user_message("go")]);
        request.response_format = Some(json!({"type": "xml"}));
        assert!(parse_response_format(request.response_format.as_ref()).is_err());
    }
}
