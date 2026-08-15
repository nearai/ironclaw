//! Prepared-context submission port for chat completions (unbound-turns
//! design §4 / one-engine §7 phase 1 adoption).
//!
//! Every non-streaming request without declared client tools is accepted
//! through the engine's shared `accept_prepared_context` door: the message
//! list seeds faithfully, the output contract journals beside it, and the
//! submit derives the unbound run profile at admission. This crate owns the
//! lane decision and the OpenAI-wire → engine-vocabulary mapping; validation
//! is the accept door's own `validate_prepared_seed_content` (the ONE
//! authoritative validator — no mirrored bounds), run BEFORE the idempotency
//! reservation so a body the door would refuse never burns the caller's key.
//! The port implementation lives in composition, which holds the thread
//! service and coordinator.

use async_trait::async_trait;
use ironclaw_host_api::prepared_context::OutputContract;
use ironclaw_product_contracts::inbound::ProductInboundAck;
use ironclaw_threads::agent_message::{
    AgentMessage, AgentMessageRole, ContentPart, ToolCallContent, ToolResultContent,
    ToolResultOutcome,
};

use crate::OpenAiCompatHttpError;
use crate::chat::{OpenAiChatCompletionRequest, OpenAiChatMessage, OpenAiChatMessageRole};
use crate::refs::OpenAiCompatActorScope;

/// One prepared-lane submission, already in the engine vocabulary: the wire
/// mapping and validation happened in the workflow BEFORE the idempotency
/// reservation, so composition only carries it to the door. The public
/// completion id doubles as the unbound thread id, exactly as the
/// conversation lane used it, so retrieval and ref binding stay id-stable.
#[derive(Debug, Clone)]
pub struct OpenAiCompatPreparedTurnRequest {
    pub scope: OpenAiCompatActorScope,
    pub public_id: String,
    pub system_prompt: String,
    pub messages: Vec<AgentMessage>,
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
            // Same in-process-call pattern as `prepared_seed_from_chat` below:
            // call the accept door's own schema validator directly, BEFORE
            // the idempotency reservation, so an oversized or pathologically
            // nested schema never burns the caller's key. No mirrored bound.
            ironclaw_threads::validate_output_schema(&schema).map_err(|error| {
                OpenAiCompatHttpError::invalid_request(Some(match error {
                    ironclaw_threads::SessionThreadError::InvalidPreparedContext { reason } => {
                        reason
                    }
                    other => other.to_string(),
                }))
            })?;
            Ok(Some(OutputContract::JsonSchema { schema }))
        }
        _ => Err(OpenAiCompatHttpError::invalid_request(Some(
            "response_format.type".to_string(),
        ))),
    }
}

/// The prepared-lane decision for a NON-STREAMING request: every request
/// without declared client tools takes the door (`Some`), honoring its
/// declared output contract or the assistant-message default. Requests
/// declaring live client tools stay on the conversation lane (the
/// external-tool park/resume flow lives there); combining client tools with
/// a structured output contract is rejected loudly instead of half-honored,
/// as is streaming with a JSON output contract.
pub(crate) fn prepared_lane_output(
    request: &OpenAiChatCompletionRequest,
) -> Result<Option<OutputContract>, OpenAiCompatHttpError> {
    let output = parse_response_format(request.response_format.as_ref())?;
    let declares_tools = request
        .tools
        .as_ref()
        .is_some_and(|tools| !tools.is_empty());
    // `tool_choice` with no `tools` declared has nothing to route on either
    // lane: the conversation lane's client-tool park/resume flow needs a
    // declared tool, and the prepared lane never consumes model_only_tools
    // at all. Silently taking the prepared lane here would drop the
    // caller's tool_choice entirely, so reject loudly instead.
    if !declares_tools && request.tool_choice.is_some() {
        return Err(OpenAiCompatHttpError::invalid_request(Some(
            "tool_choice without tools is not supported".to_string(),
        )));
    }
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
        // subscription is conversation-scoped); unbound-run streaming
        // arrives with the run-observation façade.
        return Ok(None);
    }
    if declares_tools {
        return Ok(None);
    }
    Ok(Some(output.unwrap_or_default()))
}

/// Translate the OpenAI message list into the engine vocabulary:
/// system/developer text becomes the prepared system prompt; user,
/// assistant, and tool turns become `AgentMessage`s (tool history maps
/// onto the same `external_tool.{name}` capability identity the live
/// client-tool lane registers). Inline images and non-text parts are
/// rejected loudly — the conversation lane keeps serving those.
pub(crate) fn agent_messages_from_chat(
    messages: &[OpenAiChatMessage],
) -> Result<(String, Vec<AgentMessage>), OpenAiCompatHttpError> {
    fn text_content(content: Option<&serde_json::Value>) -> Result<String, OpenAiCompatHttpError> {
        match content {
            None => Ok(String::new()),
            Some(serde_json::Value::String(text)) => Ok(text.clone()),
            Some(serde_json::Value::Array(parts)) => {
                let mut sections = Vec::new();
                for part in parts {
                    match part.get("type").and_then(serde_json::Value::as_str) {
                        Some("text") => {
                            let text = part
                                .get("text")
                                .and_then(serde_json::Value::as_str)
                                .ok_or_else(|| {
                                    OpenAiCompatHttpError::invalid_request(Some(
                                        "messages[].content[].text".to_string(),
                                    ))
                                })?;
                            sections.push(text.to_string());
                        }
                        _ => {
                            return Err(OpenAiCompatHttpError::invalid_request(Some(
                                "only text content parts are supported on this request shape"
                                    .to_string(),
                            )));
                        }
                    }
                }
                Ok(sections.join("\n\n"))
            }
            Some(_) => Err(OpenAiCompatHttpError::invalid_request(Some(
                "messages[].content".to_string(),
            ))),
        }
    }

    fn external_capability(
        name: &str,
    ) -> Result<ironclaw_host_api::ids::CapabilityId, OpenAiCompatHttpError> {
        ironclaw_host_api::ids::CapabilityId::new(format!(
            "external_tool.{}",
            name.to_ascii_lowercase()
        ))
        .map_err(|error| {
            tracing::debug!(
                ?error,
                tool_name = name,
                "tool name cannot be represented as a capability id"
            );
            OpenAiCompatHttpError::invalid_request(Some(format!(
                "tool name {name:?} cannot be represented as a capability id: {error}"
            )))
        })
    }

    let mut system_sections: Vec<String> = Vec::new();
    let mut converted: Vec<AgentMessage> = Vec::new();
    for message in messages {
        match message.role {
            OpenAiChatMessageRole::System | OpenAiChatMessageRole::Developer => {
                let text = text_content(message.content.as_ref())?;
                if !text.is_empty() {
                    system_sections.push(text);
                }
            }
            OpenAiChatMessageRole::User => {
                converted.push(AgentMessage {
                    role: AgentMessageRole::User,
                    content: vec![ContentPart::text(text_content(message.content.as_ref())?)],
                });
            }
            OpenAiChatMessageRole::Assistant => {
                let mut parts = Vec::new();
                let text = text_content(message.content.as_ref())?;
                if !text.is_empty() {
                    parts.push(ContentPart::text(text));
                }
                for call in message.tool_calls.iter().flatten() {
                    // silent-ok: caller-authored replay args are
                    // display-degradable — a non-JSON arguments string still
                    // carries useful content as a raw string, so this falls
                    // back rather than rejecting the whole message.
                    let arguments =
                        serde_json::from_str(&call.function.arguments).unwrap_or_else(|error| {
                            tracing::debug!(
                                ?error,
                                call_id = %call.id,
                                "tool call arguments are not valid JSON; carrying as a raw string"
                            );
                            serde_json::Value::String(call.function.arguments.clone())
                        });
                    parts.push(ContentPart::ToolCall(ToolCallContent {
                        call_id: call.id.clone(),
                        capability: external_capability(&call.function.name)?,
                        arguments,
                    }));
                }
                if !parts.is_empty() {
                    converted.push(AgentMessage {
                        role: AgentMessageRole::Assistant,
                        content: parts,
                    });
                }
            }
            OpenAiChatMessageRole::Tool => {
                let call_id = message.tool_call_id.clone().ok_or_else(|| {
                    OpenAiCompatHttpError::invalid_request(Some(
                        "messages[].tool_call_id".to_string(),
                    ))
                })?;
                converted.push(AgentMessage {
                    role: AgentMessageRole::Tool,
                    content: vec![ContentPart::ToolResult(ToolResultContent {
                        call_id,
                        outcome: ToolResultOutcome::Text {
                            text: text_content(message.content.as_ref())?,
                        },
                        is_error: false,
                    })],
                });
            }
        }
    }
    if converted.is_empty() {
        return Err(OpenAiCompatHttpError::invalid_request(Some(
            "messages".to_string(),
        )));
    }
    Ok((system_sections.join("\n\n"), converted))
}

/// Map + validate a prepared-lane body with the accept door's own validator.
/// Runs BEFORE the idempotency reservation; the returned pair rides the port
/// request so the mapping happens exactly once.
pub(crate) fn prepared_seed_from_chat(
    messages: &[OpenAiChatMessage],
) -> Result<(String, Vec<AgentMessage>), OpenAiCompatHttpError> {
    let (system_prompt, mapped) = agent_messages_from_chat(messages)?;
    ironclaw_threads::validate_prepared_seed_content(&system_prompt, &mapped).map_err(|error| {
        match error {
            ironclaw_threads::SessionThreadError::InvalidPreparedContext { reason } => {
                OpenAiCompatHttpError::invalid_request(Some(reason))
            }
            other => OpenAiCompatHttpError::invalid_request(Some(other.to_string())),
        }
    })?;
    Ok((system_prompt, mapped))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::OpenAiCompatErrorType;
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

    fn tool_round() -> Vec<OpenAiChatMessage> {
        vec![
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
        ]
    }

    #[test]
    fn invalid_tool_name_reports_the_capability_id_cause() {
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
                        name: "look up!".to_string(),
                        arguments: "{}".to_string(),
                    },
                }]),
            },
        ];
        let error = agent_messages_from_chat(&messages).expect_err("invalid tool name");
        // Wire shape stays a sanitized 400 invalid_request regardless of the
        // underlying capability-id validation cause (never exposed raw to the
        // client); the cause itself is bound and carried into the request
        // string / debug! log rather than dropped by the map_err closure —
        // see the non-`|_|` binding at the call site above.
        assert_eq!(error.status_code(), 400);
        assert_eq!(
            error.body().error.error_type(),
            OpenAiCompatErrorType::InvalidRequestError
        );
    }

    #[test]
    fn plain_requests_take_the_prepared_lane_with_the_default_contract() {
        let request = base_request(vec![user_message("hi")]);
        assert!(matches!(
            prepared_lane_output(&request).expect("lane"),
            Some(OutputContract::AssistantMessage)
        ));
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
    fn declared_tools_keep_the_conversation_lane_and_reject_schema_combos() {
        let mut request = base_request(vec![user_message("go")]);
        request.tools = Some(vec![]);
        assert!(matches!(
            prepared_lane_output(&request).expect("lane"),
            Some(OutputContract::AssistantMessage)
        ));

        request.tools = Some(vec![crate::chat::OpenAiChatTool {
            kind: crate::chat::OpenAiChatToolKind::Function,
            function: crate::chat::OpenAiChatFunction {
                name: "lookup".to_string(),
                description: None,
                parameters: None,
                strict: None,
            },
        }]);
        assert!(prepared_lane_output(&request).expect("lane").is_none());

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

    /// The accept door's own schema bounds run here, in-process, BEFORE the
    /// idempotency reservation in `chat_workflow.rs` — see
    /// `oversized_declared_json_schema_is_rejected_before_reserving_the_idempotency_key`
    /// in `tests/chat_workflow_handlers_contract.rs` for the route-level
    /// proof that no key gets burned.
    #[test]
    fn oversized_json_schema_is_rejected_before_reaching_the_prepared_lane() {
        let big_enum: Vec<String> = (0..(ironclaw_threads::PREPARED_OUTPUT_SCHEMA_MAX_BYTES / 8))
            .map(|i| format!("v{i:06}"))
            .collect();
        let response_format = json!({
            "type": "json_schema",
            "json_schema": {"name": "s", "schema": {"type": "string", "enum": big_enum}}
        });
        let error =
            parse_response_format(Some(&response_format)).expect_err("oversized schema rejected");
        assert_eq!(error.status_code(), 400);
    }

    #[test]
    fn over_deep_json_schema_is_rejected_before_reaching_the_prepared_lane() {
        let mut schema = json!("leaf");
        for _ in 0..(ironclaw_threads::PREPARED_OUTPUT_SCHEMA_MAX_DEPTH + 1) {
            schema = json!([schema]);
        }
        let response_format = json!({
            "type": "json_schema",
            "json_schema": {"name": "s", "schema": schema}
        });
        let error =
            parse_response_format(Some(&response_format)).expect_err("over-deep schema rejected");
        assert_eq!(error.status_code(), 400);
    }

    #[test]
    fn declared_tool_choice_without_tools_is_rejected_loudly() {
        let mut request = base_request(vec![user_message("go")]);
        request.tool_choice = Some(json!("auto"));
        let error = prepared_lane_output(&request).expect_err("tool_choice without tools rejected");
        assert_eq!(error.status_code(), 400);
    }

    #[test]
    fn text_part_missing_text_field_is_rejected_not_degraded() {
        let messages = vec![OpenAiChatMessage {
            role: OpenAiChatMessageRole::User,
            content: Some(json!([{"type": "text"}])),
            name: None,
            tool_call_id: None,
            tool_calls: None,
        }];
        let error = agent_messages_from_chat(&messages)
            .expect_err("a text part without a text field is malformed input");
        assert_eq!(error.status_code(), 400);
    }

    #[test]
    fn non_json_tool_call_arguments_carry_as_a_raw_string() {
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
                        arguments: "not json".to_string(),
                    },
                }]),
            },
        ];
        let (_, mapped) = agent_messages_from_chat(&messages).expect("degraded, not rejected");
        match &mapped[1].content[0] {
            ContentPart::ToolCall(call) => {
                assert_eq!(call.arguments, json!("not json"));
            }
            other => panic!("expected a tool call, got {other:?}"),
        }
    }

    #[test]
    fn mapping_folds_system_rows_and_pairs_tool_history() {
        let mut messages = vec![OpenAiChatMessage {
            role: OpenAiChatMessageRole::System,
            content: Some(json!("be terse")),
            name: None,
            tool_call_id: None,
            tool_calls: None,
        }];
        messages.extend(tool_round());
        let (system_prompt, mapped) =
            prepared_seed_from_chat(&messages).expect("mapped and validated");
        assert_eq!(system_prompt, "be terse");
        assert_eq!(mapped.len(), 4);
        assert_eq!(mapped[1].role, AgentMessageRole::Assistant);
        assert!(matches!(mapped[1].content[0], ContentPart::ToolCall(_)));
        assert_eq!(mapped[2].role, AgentMessageRole::Tool);
    }

    /// The door's validator runs pre-reservation: bodies it would refuse are
    /// rejected here (these shapes are the DOOR's rules — no local mirror to
    /// drift; the assertions only prove the wiring).
    #[test]
    fn door_validation_runs_on_the_mapped_seed() {
        // Over the door's message budget.
        let messages: Vec<OpenAiChatMessage> = (0..200).map(|_| user_message("hi")).collect();
        assert!(prepared_seed_from_chat(&messages).is_err());

        // A tool result answering no prior call.
        let messages = vec![
            user_message("go"),
            OpenAiChatMessage {
                role: OpenAiChatMessageRole::Tool,
                content: Some(json!("orphan")),
                name: None,
                tool_call_id: Some("call_orphan".to_string()),
                tool_calls: None,
            },
        ];
        assert!(prepared_seed_from_chat(&messages).is_err());

        // A well-formed paired round passes.
        prepared_seed_from_chat(&tool_round()).expect("paired history validates");
    }
}
