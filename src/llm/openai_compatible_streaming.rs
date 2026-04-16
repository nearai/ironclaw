//! OpenAI-compatible streaming provider via `async-openai`.
//!
//! Uses the `async-openai` crate for native SSE streaming, delivering
//! token-level deltas through `StreamingChunkSender` while accumulating
//! the full response (including tool call deltas) for the return value.
//!
//! Set `LLM_BACKEND=openai_compatible_streaming` with the same env vars
//! as `openai_compatible` (`LLM_BASE_URL`, `LLM_API_KEY`, `LLM_MODEL`).

use std::collections::BTreeMap;
use std::collections::HashMap;

use async_openai::Client;
use async_openai::config::OpenAIConfig;
use async_openai::traits::RequestOptionsBuilder;
use async_openai::types::chat::FunctionCall;
use async_openai::types::chat::{
    ChatCompletionMessageToolCall, ChatCompletionMessageToolCalls,
    ChatCompletionRequestAssistantMessage, ChatCompletionRequestMessage,
    ChatCompletionRequestSystemMessage, ChatCompletionRequestToolMessage,
    ChatCompletionRequestUserMessage, ChatCompletionStreamOptions, ChatCompletionTool,
    ChatCompletionToolChoiceOption, ChatCompletionTools, CreateChatCompletionRequestArgs,
    FinishReason as AoFinishReason, FunctionObject, ToolChoiceOptions,
};
use async_trait::async_trait;
use futures::StreamExt;
use rust_decimal::Decimal;

use crate::llm::costs;
use crate::llm::error::LlmError;
use crate::llm::provider::{
    ChatMessage, CompletionRequest, CompletionResponse, FinishReason, LlmProvider, Role,
    StreamingChunkSender, ToolCall, ToolCompletionRequest, ToolCompletionResponse, ToolDefinition,
};

const PROVIDER_NAME: &str = "openai_compatible_streaming";

/// OpenAI-compatible streaming provider backed by `async-openai`.
pub struct OpenAiCompatibleStreamingProvider {
    client: Client<OpenAIConfig>,
    model: String,
    passthrough_session_header: bool,
}

impl OpenAiCompatibleStreamingProvider {
    /// Create a new streaming provider.
    ///
    /// `base_url` must include the path prefix (e.g. `http://host:8080/v1`).
    /// `api_key` may be empty for local servers that don't require auth.
    pub fn new(base_url: &str, api_key: &str, model: &str) -> Self {
        let config = OpenAIConfig::new()
            .with_api_base(base_url)
            .with_api_key(api_key);
        let client = Client::with_config(config);
        Self {
            client,
            model: model.to_string(),
            passthrough_session_header: should_passthrough_session_header(base_url),
        }
    }

    /// Resolve the model to use for a request.
    #[cfg(test)]
    fn resolve_model(&self, requested: Option<&str>) -> String {
        requested.unwrap_or(&self.model).to_string()
    }

    fn session_id_header<'a>(&self, metadata: &'a HashMap<String, String>) -> Option<&'a str> {
        if !self.passthrough_session_header {
            return None;
        }

        metadata
            .get("thread_id")
            .or_else(|| metadata.get("session_id"))
            .map(String::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
    }
}

fn should_passthrough_session_header(base_url: &str) -> bool {
    base_url.contains("/api/internal/assistant-llm")
}

// ---------------------------------------------------------------------------
// Type translation: IronClaw → async-openai
// ---------------------------------------------------------------------------

fn translate_messages(
    messages: Vec<ChatMessage>,
) -> Result<Vec<ChatCompletionRequestMessage>, LlmError> {
    let mut out = Vec::with_capacity(messages.len());
    for msg in messages {
        let m = match msg.role {
            Role::System => {
                ChatCompletionRequestMessage::System(ChatCompletionRequestSystemMessage {
                    content: msg.content.into(),
                    name: None,
                })
            }
            Role::User => ChatCompletionRequestMessage::User(ChatCompletionRequestUserMessage {
                content: msg.content.into(),
                name: None,
            }),
            Role::Assistant => {
                let tool_calls = msg.tool_calls.map(|tcs| {
                    tcs.into_iter()
                        .map(|tc| {
                            ChatCompletionMessageToolCalls::Function(
                                ChatCompletionMessageToolCall {
                                    id: tc.id,
                                    function: FunctionCall {
                                        name: tc.name,
                                        arguments: tc.arguments.to_string(),
                                    },
                                },
                            )
                        })
                        .collect()
                });
                // Anthropic rejects empty text content blocks, so when the
                // assistant message is purely tool calls (no text), set
                // content to None instead of Some("").
                let content = if msg.content.is_empty() && tool_calls.is_some() {
                    None
                } else {
                    Some(msg.content.into())
                };
                ChatCompletionRequestMessage::Assistant(ChatCompletionRequestAssistantMessage {
                    content,
                    tool_calls,
                    ..Default::default()
                })
            }
            Role::Tool => {
                let tool_call_id = msg.tool_call_id.unwrap_or_else(|| "unknown".to_string());
                ChatCompletionRequestMessage::Tool(ChatCompletionRequestToolMessage {
                    content: msg.content.into(),
                    tool_call_id,
                })
            }
        };
        out.push(m);
    }
    Ok(out)
}

fn translate_tools(tools: Vec<ToolDefinition>) -> Vec<ChatCompletionTools> {
    tools
        .into_iter()
        .map(|t| {
            ChatCompletionTools::Function(ChatCompletionTool {
                function: FunctionObject {
                    name: t.name,
                    description: Some(t.description),
                    parameters: Some(t.parameters),
                    strict: None,
                },
            })
        })
        .collect()
}

fn map_finish_reason(reason: Option<AoFinishReason>, has_tool_calls: bool) -> FinishReason {
    match reason {
        Some(AoFinishReason::Stop) => FinishReason::Stop,
        Some(AoFinishReason::Length) => FinishReason::Length,
        Some(AoFinishReason::ToolCalls) => FinishReason::ToolUse,
        Some(AoFinishReason::ContentFilter) => FinishReason::ContentFilter,
        _ => {
            if has_tool_calls {
                FinishReason::ToolUse
            } else {
                FinishReason::Unknown
            }
        }
    }
}

fn extract_tool_calls(tool_calls: Option<Vec<ChatCompletionMessageToolCalls>>) -> Vec<ToolCall> {
    tool_calls
        .unwrap_or_default()
        .into_iter()
        .filter_map(|tc| match tc {
            ChatCompletionMessageToolCalls::Function(f) => {
                let arguments = serde_json::from_str(&f.function.arguments)
                    .unwrap_or(serde_json::Value::Object(Default::default()));
                Some(ToolCall {
                    id: f.id,
                    name: f.function.name,
                    arguments,
                    reasoning: None,
                })
            }
            _ => None,
        })
        .collect()
}

fn map_tool_choice(choice: &str) -> Option<ChatCompletionToolChoiceOption> {
    match choice {
        "auto" => Some(ChatCompletionToolChoiceOption::Mode(
            ToolChoiceOptions::Auto,
        )),
        "required" => Some(ChatCompletionToolChoiceOption::Mode(
            ToolChoiceOptions::Required,
        )),
        "none" => Some(ChatCompletionToolChoiceOption::Mode(
            ToolChoiceOptions::None,
        )),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Tool-call delta accumulator (indexed by stream position, not id) — R6
// ---------------------------------------------------------------------------

/// Accumulates streamed tool-call deltas in emission order.
///
/// The OpenAI streaming API emits tool-call fragments with an `index` field
/// that identifies which tool call the fragment belongs to. We use a `BTreeMap`
/// keyed by `index` so the final iteration order matches the model's emission
/// order — not the opaque `id` string (which could be random). This satisfies
/// requirement R6.
#[derive(Default)]
struct ToolCallAccumulator {
    /// Map from stream index → (id, name, arguments_buffer).
    calls: BTreeMap<u32, (String, String, String)>,
}

impl ToolCallAccumulator {
    fn accumulate(
        &mut self,
        index: u32,
        id: Option<String>,
        name: Option<String>,
        arguments: Option<String>,
    ) {
        let entry = self.calls.entry(index).or_default();
        if let Some(id) = id {
            if !id.is_empty() {
                entry.0 = id;
            }
        }
        if let Some(name) = name {
            if !name.is_empty() {
                entry.1 = name;
            }
        }
        if let Some(args) = arguments {
            entry.2.push_str(&args);
        }
    }

    fn into_tool_calls(self) -> Vec<ToolCall> {
        self.calls
            .into_values()
            .map(|(id, name, args_str)| {
                let arguments = serde_json::from_str(&args_str)
                    .unwrap_or(serde_json::Value::Object(Default::default()));
                ToolCall {
                    id,
                    name,
                    arguments,
                    reasoning: None,
                }
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------
// LlmProvider implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl LlmProvider for OpenAiCompatibleStreamingProvider {
    fn model_name(&self) -> &str {
        &self.model
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        costs::model_cost(&self.model).unwrap_or_else(costs::default_cost)
    }

    async fn complete(
        &self,
        mut request: CompletionRequest,
    ) -> Result<CompletionResponse, LlmError> {
        let model = request
            .take_model_override()
            .unwrap_or_else(|| self.model.clone());
        let session_id = self
            .session_id_header(&request.metadata)
            .map(str::to_string);
        let messages = translate_messages(request.messages)?;

        let mut builder = CreateChatCompletionRequestArgs::default();
        builder.model(&model).messages(messages);
        if let Some(t) = request.temperature {
            builder.temperature(t);
        }
        if let Some(m) = request.max_tokens {
            #[allow(deprecated)]
            builder.max_tokens(m);
        }
        if let Some(ref stops) = request.stop_sequences {
            builder.stop(stops.clone());
        }

        let req = builder.build().map_err(|e| LlmError::RequestFailed {
            provider: PROVIDER_NAME.to_string(),
            reason: format!("Failed to build request: {e}"),
        })?;

        let mut chat = self.client.chat();
        if let Some(session_id) = session_id.as_deref() {
            chat =
                chat.header("x-session-id", session_id)
                    .map_err(|e| LlmError::RequestFailed {
                        provider: PROVIDER_NAME.to_string(),
                        reason: format!("Failed to set x-session-id header: {e}"),
                    })?;
        }

        let response = chat
            .create(req)
            .await
            .map_err(|e| LlmError::RequestFailed {
                provider: PROVIDER_NAME.to_string(),
                reason: format!("{e}"),
            })?;

        let choice =
            response
                .choices
                .into_iter()
                .next()
                .ok_or_else(|| LlmError::EmptyResponse {
                    provider: PROVIDER_NAME.to_string(),
                })?;

        let content = choice.message.content.unwrap_or_default();
        let finish_reason = map_finish_reason(choice.finish_reason, false);
        let (input_tokens, output_tokens) = response
            .usage
            .map(|u| (u.prompt_tokens, u.completion_tokens))
            .unwrap_or((0, 0));

        Ok(CompletionResponse {
            content,
            finish_reason,
            input_tokens,
            output_tokens,
            cache_read_input_tokens: 0,
            cache_creation_input_tokens: 0,
        })
    }

    async fn complete_with_tools(
        &self,
        mut request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        let model = request
            .take_model_override()
            .unwrap_or_else(|| self.model.clone());
        let session_id = self
            .session_id_header(&request.metadata)
            .map(str::to_string);
        let messages = translate_messages(request.messages)?;
        let tools = translate_tools(request.tools);

        let mut builder = CreateChatCompletionRequestArgs::default();
        builder.model(&model).messages(messages);
        if !tools.is_empty() {
            builder.tools(tools);
        }
        if let Some(ref tc) = request.tool_choice {
            if let Some(choice) = map_tool_choice(tc) {
                builder.tool_choice(choice);
            }
        }
        if let Some(t) = request.temperature {
            builder.temperature(t);
        }
        if let Some(m) = request.max_tokens {
            #[allow(deprecated)]
            builder.max_tokens(m);
        }
        if let Some(ref stops) = request.stop_sequences {
            builder.stop(stops.clone());
        }

        let req = builder.build().map_err(|e| LlmError::RequestFailed {
            provider: PROVIDER_NAME.to_string(),
            reason: format!("Failed to build request: {e}"),
        })?;

        let mut chat = self.client.chat();
        if let Some(session_id) = session_id.as_deref() {
            chat =
                chat.header("x-session-id", session_id)
                    .map_err(|e| LlmError::RequestFailed {
                        provider: PROVIDER_NAME.to_string(),
                        reason: format!("Failed to set x-session-id header: {e}"),
                    })?;
        }

        let response = chat
            .create(req)
            .await
            .map_err(|e| LlmError::RequestFailed {
                provider: PROVIDER_NAME.to_string(),
                reason: format!("{e}"),
            })?;

        let choice =
            response
                .choices
                .into_iter()
                .next()
                .ok_or_else(|| LlmError::EmptyResponse {
                    provider: PROVIDER_NAME.to_string(),
                })?;

        let tool_calls = extract_tool_calls(choice.message.tool_calls);
        let finish_reason = map_finish_reason(choice.finish_reason, !tool_calls.is_empty());
        let content = choice.message.content;
        let (input_tokens, output_tokens) = response
            .usage
            .map(|u| (u.prompt_tokens, u.completion_tokens))
            .unwrap_or((0, 0));

        Ok(ToolCompletionResponse {
            content,
            tool_calls,
            finish_reason,
            input_tokens,
            output_tokens,
            cache_read_input_tokens: 0,
            cache_creation_input_tokens: 0,
        })
    }

    // -----------------------------------------------------------------------
    // Streaming paths — the raison d'être of this provider
    // -----------------------------------------------------------------------

    async fn complete_streaming(
        &self,
        mut request: CompletionRequest,
        chunk_tx: StreamingChunkSender,
    ) -> Result<CompletionResponse, LlmError> {
        let model = request
            .take_model_override()
            .unwrap_or_else(|| self.model.clone());
        let session_id = self
            .session_id_header(&request.metadata)
            .map(str::to_string);
        let messages = translate_messages(request.messages)?;

        let mut builder = CreateChatCompletionRequestArgs::default();
        builder
            .model(&model)
            .messages(messages)
            .stream_options(ChatCompletionStreamOptions {
                include_usage: Some(true),
                include_obfuscation: None,
            });
        if let Some(t) = request.temperature {
            builder.temperature(t);
        }
        if let Some(m) = request.max_tokens {
            #[allow(deprecated)]
            builder.max_tokens(m);
        }
        if let Some(ref stops) = request.stop_sequences {
            builder.stop(stops.clone());
        }

        let req = builder.build().map_err(|e| LlmError::RequestFailed {
            provider: PROVIDER_NAME.to_string(),
            reason: format!("Failed to build streaming request: {e}"),
        })?;

        let mut chat = self.client.chat();
        if let Some(session_id) = session_id.as_deref() {
            chat =
                chat.header("x-session-id", session_id)
                    .map_err(|e| LlmError::RequestFailed {
                        provider: PROVIDER_NAME.to_string(),
                        reason: format!("Failed to set x-session-id header: {e}"),
                    })?;
        }

        let mut stream = chat
            .create_stream(req)
            .await
            .map_err(|e| LlmError::RequestFailed {
                provider: PROVIDER_NAME.to_string(),
                reason: format!("Failed to create stream: {e}"),
            })?;

        let mut full_content = String::new();
        let mut finish_reason = FinishReason::Unknown;
        let mut input_tokens: u32 = 0;
        let mut output_tokens: u32 = 0;

        while let Some(result) = stream.next().await {
            let response = result.map_err(|e| LlmError::RequestFailed {
                provider: PROVIDER_NAME.to_string(),
                reason: format!("Stream error: {e}"),
            })?;

            // Usage is reported in the final chunk (R7)
            if let Some(usage) = response.usage {
                input_tokens = usage.prompt_tokens;
                output_tokens = usage.completion_tokens;
            }

            for choice in &response.choices {
                if let Some(ref reason) = choice.finish_reason {
                    finish_reason = map_finish_reason(Some(*reason), false);
                }
                if let Some(ref content) = choice.delta.content {
                    full_content.push_str(content);
                    if chunk_tx.send(content.clone()).await.is_err() {
                        tracing::debug!("Streaming receiver dropped, finishing stream");
                        return Ok(CompletionResponse {
                            content: full_content,
                            finish_reason,
                            input_tokens,
                            output_tokens,
                            cache_read_input_tokens: 0,
                            cache_creation_input_tokens: 0,
                        });
                    }
                }
            }
        }

        Ok(CompletionResponse {
            content: full_content,
            finish_reason,
            input_tokens,
            output_tokens,
            cache_read_input_tokens: 0,
            cache_creation_input_tokens: 0,
        })
    }

    async fn complete_with_tools_streaming(
        &self,
        mut request: ToolCompletionRequest,
        chunk_tx: StreamingChunkSender,
    ) -> Result<ToolCompletionResponse, LlmError> {
        let model = request
            .take_model_override()
            .unwrap_or_else(|| self.model.clone());
        let session_id = self
            .session_id_header(&request.metadata)
            .map(str::to_string);
        let messages = translate_messages(request.messages)?;
        let tools = translate_tools(request.tools);

        let mut builder = CreateChatCompletionRequestArgs::default();
        builder
            .model(&model)
            .messages(messages)
            .stream_options(ChatCompletionStreamOptions {
                include_usage: Some(true),
                include_obfuscation: None,
            });
        if !tools.is_empty() {
            builder.tools(tools);
        }
        if let Some(ref tc) = request.tool_choice {
            if let Some(choice) = map_tool_choice(tc) {
                builder.tool_choice(choice);
            }
        }
        if let Some(t) = request.temperature {
            builder.temperature(t);
        }
        if let Some(m) = request.max_tokens {
            #[allow(deprecated)]
            builder.max_tokens(m);
        }
        if let Some(ref stops) = request.stop_sequences {
            builder.stop(stops.clone());
        }

        let req = builder.build().map_err(|e| LlmError::RequestFailed {
            provider: PROVIDER_NAME.to_string(),
            reason: format!("Failed to build streaming request: {e}"),
        })?;

        let mut chat = self.client.chat();
        if let Some(session_id) = session_id.as_deref() {
            chat =
                chat.header("x-session-id", session_id)
                    .map_err(|e| LlmError::RequestFailed {
                        provider: PROVIDER_NAME.to_string(),
                        reason: format!("Failed to set x-session-id header: {e}"),
                    })?;
        }

        let mut stream = chat
            .create_stream(req)
            .await
            .map_err(|e| LlmError::RequestFailed {
                provider: PROVIDER_NAME.to_string(),
                reason: format!("Failed to create stream: {e}"),
            })?;

        let mut full_content = String::new();
        let mut tool_acc = ToolCallAccumulator::default();
        let mut finish_reason = FinishReason::Unknown;
        let mut input_tokens: u32 = 0;
        let mut output_tokens: u32 = 0;
        let mut receiver_alive = true;

        while let Some(result) = stream.next().await {
            let response = result.map_err(|e| LlmError::RequestFailed {
                provider: PROVIDER_NAME.to_string(),
                reason: format!("Stream error: {e}"),
            })?;

            if let Some(usage) = response.usage {
                input_tokens = usage.prompt_tokens;
                output_tokens = usage.completion_tokens;
            }

            for choice in &response.choices {
                if let Some(ref reason) = choice.finish_reason {
                    finish_reason = map_finish_reason(Some(*reason), false);
                }

                // Text delta
                if let Some(ref content) = choice.delta.content {
                    full_content.push_str(content);
                    if receiver_alive && chunk_tx.send(content.clone()).await.is_err() {
                        tracing::debug!("Streaming receiver dropped, continuing to accumulate");
                        receiver_alive = false;
                    }
                }

                // Tool-call deltas — accumulate by stream index (R6)
                if let Some(ref tc_deltas) = choice.delta.tool_calls {
                    for tc_delta in tc_deltas {
                        let idx = tc_delta.index;
                        let id = tc_delta.id.clone();
                        let (name, args) = tc_delta
                            .function
                            .as_ref()
                            .map(|f| (f.name.clone(), f.arguments.clone()))
                            .unwrap_or((None, None));
                        tool_acc.accumulate(idx, id, name, args);
                    }
                }
            }
        }

        let tool_calls = tool_acc.into_tool_calls();
        if !tool_calls.is_empty() && finish_reason != FinishReason::ToolUse {
            finish_reason = FinishReason::ToolUse;
        }

        let content = if full_content.is_empty() {
            None
        } else {
            Some(full_content)
        };

        Ok(ToolCompletionResponse {
            content,
            tool_calls,
            finish_reason,
            input_tokens,
            output_tokens,
            cache_read_input_tokens: 0,
            cache_creation_input_tokens: 0,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accumulator_preserves_emission_order_by_index() {
        let mut acc = ToolCallAccumulator::default();
        acc.accumulate(1, Some("id_b".into()), Some("tool_b".into()), None);
        acc.accumulate(1, None, None, Some(r#"{"x":1"#.into()));
        acc.accumulate(1, None, None, Some("}".into()));
        acc.accumulate(0, Some("id_a".into()), Some("tool_a".into()), None);
        acc.accumulate(0, None, None, Some(r#"{"y":2}"#.into()));

        let calls = acc.into_tool_calls();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].name, "tool_a");
        assert_eq!(calls[0].id, "id_a");
        assert_eq!(calls[0].arguments, serde_json::json!({"y": 2}));
        assert_eq!(calls[1].name, "tool_b");
        assert_eq!(calls[1].id, "id_b");
        assert_eq!(calls[1].arguments, serde_json::json!({"x": 1}));
    }

    #[test]
    fn accumulator_handles_malformed_json_gracefully() {
        let mut acc = ToolCallAccumulator::default();
        acc.accumulate(
            0,
            Some("id".into()),
            Some("tool".into()),
            Some("not json".into()),
        );
        let calls = acc.into_tool_calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].arguments, serde_json::json!({}));
    }

    #[test]
    fn accumulator_empty_produces_no_calls() {
        let acc = ToolCallAccumulator::default();
        assert!(acc.into_tool_calls().is_empty());
    }

    #[test]
    fn translate_messages_all_roles() {
        let messages = vec![
            ChatMessage {
                role: Role::System,
                content: "You are helpful.".to_string(),
                tool_calls: None,
                tool_call_id: None,
                name: None,
                content_parts: vec![],
            },
            ChatMessage {
                role: Role::User,
                content: "Hello".to_string(),
                tool_calls: None,
                tool_call_id: None,
                name: None,
                content_parts: vec![],
            },
            ChatMessage {
                role: Role::Assistant,
                content: "Hi there!".to_string(),
                tool_calls: None,
                tool_call_id: None,
                name: None,
                content_parts: vec![],
            },
            ChatMessage {
                role: Role::Tool,
                content: r#"{"result": "ok"}"#.to_string(),
                tool_calls: None,
                tool_call_id: Some("call_123".to_string()),
                name: None,
                content_parts: vec![],
            },
        ];
        let result = translate_messages(messages);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().len(), 4);
    }

    #[test]
    fn translate_tools_round_trips() {
        let tools = vec![ToolDefinition {
            name: "get_weather".to_string(),
            description: "Get weather for a city".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "city": {"type": "string"}
                }
            }),
        }];
        let ao_tools = translate_tools(tools);
        assert_eq!(ao_tools.len(), 1);
        match &ao_tools[0] {
            ChatCompletionTools::Function(t) => {
                assert_eq!(t.function.name, "get_weather");
            }
            _ => panic!("expected Function variant"),
        }
    }

    #[test]
    fn finish_reason_mapping() {
        assert_eq!(
            map_finish_reason(Some(AoFinishReason::Stop), false),
            FinishReason::Stop
        );
        assert_eq!(
            map_finish_reason(Some(AoFinishReason::Length), false),
            FinishReason::Length
        );
        assert_eq!(
            map_finish_reason(Some(AoFinishReason::ToolCalls), false),
            FinishReason::ToolUse
        );
        assert_eq!(
            map_finish_reason(Some(AoFinishReason::ContentFilter), false),
            FinishReason::ContentFilter
        );
        assert_eq!(map_finish_reason(None, true), FinishReason::ToolUse);
        assert_eq!(map_finish_reason(None, false), FinishReason::Unknown);
    }

    #[test]
    fn provider_new_sets_model_name() {
        let provider =
            OpenAiCompatibleStreamingProvider::new("http://localhost:8080/v1", "sk-test", "gpt-4o");
        assert_eq!(provider.model_name(), "gpt-4o");
    }

    #[test]
    fn internal_proxy_base_url_enables_session_header_passthrough() {
        assert!(should_passthrough_session_header(
            "http://lobsterpool:3000/api/internal/assistant-llm"
        ));
        assert!(should_passthrough_session_header(
            "http://lobsterpool:3000/api/internal/assistant-llm/v1"
        ));
    }

    #[test]
    fn external_base_url_disables_session_header_passthrough() {
        assert!(!should_passthrough_session_header(
            "https://api.openai.com/v1"
        ));
        assert!(!should_passthrough_session_header(
            "https://openrouter.ai/api/v1"
        ));
    }

    #[test]
    fn session_header_uses_thread_id_only_for_internal_proxy() {
        let internal = OpenAiCompatibleStreamingProvider::new(
            "http://lobsterpool:3000/api/internal/assistant-llm",
            "sk-test",
            "gpt-4o",
        );
        let external = OpenAiCompatibleStreamingProvider::new(
            "https://api.openai.com/v1",
            "sk-test",
            "gpt-4o",
        );
        let metadata = HashMap::from([("thread_id".to_string(), "thread-123".to_string())]);

        assert_eq!(internal.session_id_header(&metadata), Some("thread-123"));
        assert_eq!(external.session_id_header(&metadata), None);
    }

    #[test]
    fn provider_cost_per_token_uses_cost_table() {
        let provider =
            OpenAiCompatibleStreamingProvider::new("http://localhost:8080/v1", "sk-test", "gpt-4o");
        let (input, output) = provider.cost_per_token();
        assert!(input >= Decimal::ZERO);
        assert!(output >= Decimal::ZERO);
    }

    #[test]
    fn resolve_model_uses_override_when_provided() {
        let provider =
            OpenAiCompatibleStreamingProvider::new("http://localhost:8080/v1", "sk-test", "gpt-4o");
        assert_eq!(provider.resolve_model(Some("gpt-4o-mini")), "gpt-4o-mini");
        assert_eq!(provider.resolve_model(None), "gpt-4o");
    }

    #[test]
    fn extract_tool_calls_from_response() {
        let ao_calls = vec![
            ChatCompletionMessageToolCalls::Function(ChatCompletionMessageToolCall {
                id: "call_1".into(),
                function: FunctionCall {
                    name: "get_weather".into(),
                    arguments: r#"{"city":"NYC"}"#.into(),
                },
            }),
            ChatCompletionMessageToolCalls::Function(ChatCompletionMessageToolCall {
                id: "call_2".into(),
                function: FunctionCall {
                    name: "get_time".into(),
                    arguments: r#"{"tz":"UTC"}"#.into(),
                },
            }),
        ];
        let calls = extract_tool_calls(Some(ao_calls));
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].name, "get_weather");
        assert_eq!(calls[0].arguments, serde_json::json!({"city": "NYC"}));
        assert_eq!(calls[1].name, "get_time");
    }

    #[test]
    fn extract_tool_calls_none_returns_empty() {
        let calls = extract_tool_calls(None);
        assert!(calls.is_empty());
    }

    #[test]
    fn map_tool_choice_variants() {
        assert!(matches!(
            map_tool_choice("auto"),
            Some(ChatCompletionToolChoiceOption::Mode(
                ToolChoiceOptions::Auto
            ))
        ));
        assert!(matches!(
            map_tool_choice("required"),
            Some(ChatCompletionToolChoiceOption::Mode(
                ToolChoiceOptions::Required
            ))
        ));
        assert!(matches!(
            map_tool_choice("none"),
            Some(ChatCompletionToolChoiceOption::Mode(
                ToolChoiceOptions::None
            ))
        ));
        assert!(map_tool_choice("other").is_none());
    }
}
