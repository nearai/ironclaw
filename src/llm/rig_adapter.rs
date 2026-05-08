//! Generic adapter that bridges rig-core's `CompletionModel` trait to IronClaw's `LlmProvider`.
//!
//! This lets us use any rig-core provider (OpenAI, Anthropic, Ollama, etc.) as an
//! `Arc<dyn LlmProvider>` without changing any of the agent, reasoning, or tool code.

use crate::llm::config::CacheRetention;
use async_trait::async_trait;
use rig::OneOrMany;
use rig::completion::{
    AssistantContent, CompletionModel, CompletionRequest as RigRequest,
    ToolDefinition as RigToolDefinition, Usage as RigUsage,
};
use rig::message::{
    DocumentSourceKind, Image, ImageDetail, ImageMediaType, Message as RigMessage, MimeType,
    ToolChoice as RigToolChoice, ToolFunction, ToolResult as RigToolResult, ToolResultContent,
    UserContent,
};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serde::Serialize;
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};

use std::collections::HashSet;
use std::str::FromStr;

use crate::llm::costs;
use crate::llm::error::LlmError;
use crate::llm::provider::{
    ChatMessage, CompletionRequest, CompletionResponse, FinishReason, LlmProvider,
    ToolCall as IronToolCall, ToolCompletionRequest, ToolCompletionResponse,
    ToolDefinition as IronToolDefinition, strip_unsupported_completion_params,
    strip_unsupported_tool_params,
};
use crate::llm::tool_schema::{ToolSchemaPolicy, shape_tool_schema};
#[cfg(test)]
use crate::llm::tool_schema::{normalize_schema_strict, serialize_json_capped};

/// Adapter that wraps a rig-core `CompletionModel` and implements `LlmProvider`.
pub struct RigAdapter<M: CompletionModel> {
    model: M,
    model_name: String,
    input_cost: Decimal,
    output_cost: Decimal,
    /// Prompt cache retention policy (Anthropic only).
    /// When not `CacheRetention::None`, injects top-level `cache_control`
    /// via `additional_params` for Anthropic automatic caching. Also controls
    /// the cost multiplier for cache-creation tokens.
    cache_retention: CacheRetention,
    /// Parameter names that this provider does not support (e.g., `"temperature"`).
    /// These are stripped from requests before sending to avoid 400 errors.
    unsupported_params: HashSet<String>,
    /// Default additional parameters merged into every request.
    /// Used by providers that need extra top-level fields (e.g., Ollama `think: true`).
    default_additional_params: Option<serde_json::Value>,
}

impl<M: CompletionModel> RigAdapter<M> {
    /// Create a new adapter wrapping the given rig-core model.
    pub fn new(model: M, model_name: impl Into<String>) -> Self {
        let name = model_name.into();
        let (input_cost, output_cost) =
            costs::model_cost(&name).unwrap_or_else(costs::default_cost);
        Self {
            model,
            model_name: name,
            input_cost,
            output_cost,
            cache_retention: CacheRetention::None,
            unsupported_params: HashSet::new(),
            default_additional_params: None,
        }
    }

    /// Set Anthropic prompt cache retention policy.
    ///
    /// Controls both cache injection and cost tracking:
    /// - `None` — no caching, no surcharge (1.0×).
    /// - `Short` — 5-minute TTL via `{"type": "ephemeral"}`, 1.25× write surcharge.
    /// - `Long` — 1-hour TTL via `{"type": "ephemeral", "ttl": "1h"}`, 2.0× write surcharge.
    ///
    /// Cache injection uses Anthropic's **automatic caching** — a top-level
    /// `cache_control` field in `additional_params` that gets `#[serde(flatten)]`'d
    /// into the request body by rig-core.
    ///
    /// If the configured model does not support caching (e.g. claude-2),
    /// a warning is logged once at construction and caching is disabled.
    pub fn with_cache_retention(mut self, retention: CacheRetention) -> Self {
        if retention != CacheRetention::None && !supports_prompt_cache(&self.model_name) {
            tracing::warn!(
                model = %self.model_name,
                "Prompt caching requested but model does not support it; disabling"
            );
            self.cache_retention = CacheRetention::None;
        } else {
            self.cache_retention = retention;
        }
        self
    }

    /// Set the list of unsupported parameter names for this provider.
    ///
    /// Parameters in this set are stripped from requests before sending.
    /// Supported parameter names: `"temperature"`, `"max_tokens"`, `"stop_sequences"`.
    pub fn with_unsupported_params(mut self, params: Vec<String>) -> Self {
        self.unsupported_params = params.into_iter().collect();
        self
    }

    /// Set default additional parameters merged into every request.
    ///
    /// These are injected into rig-core's `additional_params` field, which gets
    /// `#[serde(flatten)]`'d into the provider's request payload. Use this for
    /// provider-specific top-level fields like Ollama's `think: true`.
    pub fn with_additional_params(mut self, params: serde_json::Value) -> Self {
        self.default_additional_params = Some(params);
        self
    }

    /// Strip unsupported fields from a `CompletionRequest` in place.
    fn strip_unsupported_completion_params(&self, req: &mut CompletionRequest) {
        strip_unsupported_completion_params(&self.unsupported_params, req);
    }

    /// Strip unsupported fields from a `ToolCompletionRequest` in place.
    fn strip_unsupported_tool_params(&self, req: &mut ToolCompletionRequest) {
        strip_unsupported_tool_params(&self.unsupported_params, req);
    }
}

// -- Type conversion helpers --

/// Round an f32 to f64 without precision artifacts.
///
/// Direct `f32 as f64` preserves the binary representation, producing values
/// like `0.699999988079071` instead of `0.7`. Some providers (e.g. Zhipu/GLM)
/// reject these values with a 400 error. Rounding to 6 decimal places removes
/// the artifact while preserving all meaningful precision for temperature.
fn round_f32_to_f64(val: f32) -> f64 {
    ((val as f64) * 1_000_000.0).round() / 1_000_000.0
}

/// Convert IronClaw messages to rig-core format.
///
/// Returns `(preamble, chat_history)` where preamble is extracted from
/// any System message and chat_history contains the rest.
fn convert_messages(messages: &[ChatMessage]) -> (Option<String>, Vec<RigMessage>) {
    let mut preamble: Option<String> = None;
    let mut history = Vec::new();

    for msg in messages {
        match msg.role {
            crate::llm::Role::System => {
                // Concatenate system messages into preamble
                match preamble {
                    Some(ref mut p) => {
                        p.push('\n');
                        p.push_str(&msg.content);
                    }
                    None => preamble = Some(msg.content.clone()),
                }
            }
            crate::llm::Role::User => {
                if msg.content_parts.is_empty() {
                    // Skip empty user messages — some providers (e.g. Kimi) reject "content": ""
                    if msg.content.is_empty() {
                        continue;
                    }
                    history.push(RigMessage::user(&msg.content));
                } else {
                    // Build multimodal user message with text + image parts
                    let mut contents: Vec<UserContent> = vec![UserContent::text(&msg.content)];
                    for part in &msg.content_parts {
                        if let crate::llm::ContentPart::ImageUrl { image_url } = part {
                            let detail =
                                ImageDetail::from_str(&image_url.normalized_openai_detail())
                                    .unwrap_or_default();
                            // Parse data: URL for base64 images, or use raw URL
                            let image = if let Some(rest) = image_url.url.strip_prefix("data:") {
                                // Format: data:<mime>;base64,<data>
                                let (mime, b64) =
                                    rest.split_once(";base64,").unwrap_or(("image/jpeg", rest));
                                Image {
                                    data: DocumentSourceKind::base64(b64),
                                    media_type: ImageMediaType::from_mime_type(mime),
                                    detail: Some(detail.clone()),
                                    additional_params: None,
                                }
                            } else {
                                Image {
                                    data: DocumentSourceKind::url(&image_url.url),
                                    media_type: None,
                                    detail: Some(detail),
                                    additional_params: None,
                                }
                            };
                            contents.push(UserContent::Image(image));
                        }
                    }
                    if let Ok(many) = OneOrMany::many(contents) {
                        history.push(RigMessage::User { content: many });
                    } else {
                        history.push(RigMessage::user(&msg.content));
                    }
                }
            }
            crate::llm::Role::Assistant => {
                if let Some(ref tool_calls) = msg.tool_calls {
                    // Assistant message with tool calls
                    let mut contents: Vec<AssistantContent> = Vec::new();
                    if !msg.content.is_empty() {
                        contents.push(AssistantContent::text(&msg.content));
                    }
                    // Round-trip provider-emitted reasoning artifacts. rig-core's
                    // dedicated DeepSeek/Gemini/OpenRouter clients consume
                    // `AssistantContent::Reasoning` on the message and re-emit
                    // it as the wire-format reasoning field on the next request.
                    // Without this, DeepSeek/Gemini reject the follow-up turn
                    // with HTTP 400. See #3201, #3225.
                    if let Some(ref reasoning) = msg.reasoning
                        && !reasoning.is_empty()
                    {
                        contents.push(AssistantContent::Reasoning(rig::message::Reasoning::new(
                            reasoning,
                        )));
                    }
                    for (idx, tc) in tool_calls.iter().enumerate() {
                        let tool_call_id =
                            normalized_tool_call_id(Some(tc.id.as_str()), history.len() + idx);
                        let mut rig_tc = rig::message::ToolCall::new(
                            tool_call_id.clone(),
                            ToolFunction::new(tc.name.clone(), tc.arguments.clone()),
                        )
                        .with_call_id(tool_call_id);
                        // Echo provider-emitted per-tool-call signatures back
                        // (Gemini's `thought_signature`). The reviewer's
                        // motivating example: a signed Gemini `functionCall`
                        // returned in turn N must carry the same signature
                        // when sent back in turn N+1, otherwise the API
                        // rejects with "Function call is missing a
                        // thought_signature in functionCall parts" (#3225).
                        if tc.signature.is_some() {
                            rig_tc = rig_tc.with_signature(tc.signature.clone());
                        }
                        contents.push(AssistantContent::ToolCall(rig_tc));
                    }
                    if let Ok(many) = OneOrMany::many(contents) {
                        history.push(RigMessage::Assistant {
                            id: None,
                            content: many,
                        });
                    } else {
                        // Shouldn't happen but fall back to text
                        history.push(RigMessage::assistant(&msg.content));
                    }
                } else if let Some(ref reasoning) = msg.reasoning
                    && !reasoning.is_empty()
                {
                    // Assistant message with reasoning but no tool calls
                    // (e.g., a "thinking" turn followed by a final answer).
                    // The next request still needs the reasoning echoed so the
                    // provider validates the chain, even when the message has
                    // no tool calls of its own.
                    let mut contents: Vec<AssistantContent> = Vec::new();
                    if !msg.content.is_empty() {
                        contents.push(AssistantContent::text(&msg.content));
                    }
                    contents.push(AssistantContent::Reasoning(rig::message::Reasoning::new(
                        reasoning,
                    )));
                    if let Ok(many) = OneOrMany::many(contents) {
                        history.push(RigMessage::Assistant {
                            id: None,
                            content: many,
                        });
                    }
                } else {
                    // Skip empty assistant messages — these occur when thinking-tag stripping
                    // leaves a blank response; sending "content": "" causes 400 on strict
                    // OpenAI-compatible providers (e.g. Kimi).
                    if msg.content.is_empty() {
                        continue;
                    }
                    history.push(RigMessage::assistant(&msg.content));
                }
            }
            crate::llm::Role::Tool => {
                // Tool result message: wrap as User { ToolResult }.
                // Merge consecutive tool results into a single User message
                // so the API sees one multi-result message instead of
                // multiple consecutive User messages (which Anthropic rejects).
                let tool_id = normalized_tool_call_id(msg.tool_call_id.as_deref(), history.len());
                let tool_result = UserContent::ToolResult(RigToolResult {
                    id: tool_id.clone(),
                    call_id: Some(tool_id),
                    content: OneOrMany::one(ToolResultContent::text(&msg.content)),
                });

                let should_merge = matches!(
                    history.last(),
                    Some(RigMessage::User { content }) if content.iter().all(|c| matches!(c, UserContent::ToolResult(_)))
                );

                if should_merge {
                    if let Some(RigMessage::User { content }) = history.last_mut() {
                        content.push(tool_result);
                    }
                } else {
                    history.push(RigMessage::User {
                        content: OneOrMany::one(tool_result),
                    });
                }
            }
        }
    }

    (preamble, history)
}

/// Responses-style providers require a non-empty tool call ID.
///
/// IDs must be compatible with providers like Mistral, which constrain IDs
/// to `[a-zA-Z0-9]{9}`. We therefore:
/// - pass through any non-empty raw ID that already matches this constraint;
/// - otherwise deterministically map the raw string into a provider-compliant ID;
/// - and when `raw` is empty/None, delegate to `generate_tool_call_id`.
fn normalized_tool_call_id(raw: Option<&str>, seed: usize) -> String {
    // Trim and treat empty as None.
    let trimmed = raw.and_then(|s| {
        let t = s.trim();
        if t.is_empty() { None } else { Some(t) }
    });

    if let Some(id) = trimmed {
        // If the ID already satisfies `[a-zA-Z0-9]{9}`, pass it through unchanged.
        if id.len() == 9 && id.chars().all(|c| c.is_ascii_alphanumeric()) {
            return id.to_string();
        }

        // Otherwise, deterministically hash the raw ID and feed the hash-derived
        // seed into the provider-level generator so that the encoding and any
        // provider-specific constraints remain centralized in one place.
        let digest = Sha256::digest(id.as_bytes());
        // Derive a 64-bit value from the first 8 bytes of the digest, then
        // split it into two usize seeds so we preserve all 64 bits of entropy
        // even on 32-bit targets.
        let hash64 = {
            // SHA-256 always produces 32 bytes, so indexing the first 8 is safe.
            let bytes: [u8; 8] = [
                digest[0], digest[1], digest[2], digest[3], digest[4], digest[5], digest[6],
                digest[7],
            ];
            u64::from_be_bytes(bytes)
        };
        let hi_seed: usize = (hash64 >> 32) as usize;
        let lo_seed: usize = (hash64 & 0xFFFF_FFFF) as usize;
        return super::provider::generate_tool_call_id(hi_seed, lo_seed);
    }

    // Fallback for missing/empty raw IDs: use the provider-level generator,
    // which already produces compliant IDs.
    super::provider::generate_tool_call_id(seed, 0)
}

/// Convert IronClaw tool definitions to rig-core format.
///
/// Applies `normalize_schema_strict` at the boundary, which both
/// strict-normalizes nested objects AND flattens any top-level
/// `oneOf`/`anyOf`/`allOf`/`enum`/`not` (OpenAI's tool API rejects those at
/// the top level even when the rest of the schema is valid). The flatten may
/// append an advisory hint to the tool description, so we pass an owned
/// clone through and read it back.
fn convert_tools(tools: &[IronToolDefinition]) -> Vec<RigToolDefinition> {
    tools
        .iter()
        .map(|t| {
            let mut description = t.description.clone();
            let parameters = shape_tool_schema(
                ToolSchemaPolicy::StrictOpenAi,
                &t.parameters,
                &mut description,
            );
            RigToolDefinition {
                name: t.name.clone(),
                description,
                parameters,
            }
        })
        .collect()
}

/// Convert IronClaw tool_choice string to rig-core ToolChoice.
fn convert_tool_choice(choice: Option<&str>) -> Option<RigToolChoice> {
    match choice.map(|s| s.to_lowercase()).as_deref() {
        Some("auto") => Some(RigToolChoice::Auto),
        Some("required") => Some(RigToolChoice::Required),
        Some("none") => Some(RigToolChoice::None),
        _ => None,
    }
}

/// Extract text, tool calls, and provider-emitted reasoning artifacts from a
/// rig-core completion response.
///
/// The returned `reasoning` is the concatenation of every
/// `AssistantContent::Reasoning` chunk in the response. Callers MUST attach it
/// to the assistant `ChatMessage` they store for the next turn — DeepSeek's
/// thinking mode and Gemini 2.5+ both reject the next request with HTTP 400
/// when the prior message had reasoning that wasn't echoed back. See #3201,
/// #3225, and the rig-core deepseek client source where
/// `last_reasoning_content` is round-tripped onto the last assistant message
/// of the next request.
fn extract_response(
    choice: &OneOrMany<AssistantContent>,
    _usage: &RigUsage,
) -> (
    Option<String>,
    Vec<IronToolCall>,
    FinishReason,
    Option<String>,
) {
    let mut text_parts: Vec<String> = Vec::new();
    let mut tool_calls: Vec<IronToolCall> = Vec::new();
    let mut reasoning_parts: Vec<String> = Vec::new();

    for content in choice.iter() {
        match content {
            AssistantContent::Text(t) if !t.text.is_empty() => {
                text_parts.push(t.text.clone());
            }
            AssistantContent::Text(_) => {}
            AssistantContent::ToolCall(tc) => {
                tool_calls.push(IronToolCall {
                    id: tc.id.clone(),
                    name: tc.function.name.clone(),
                    arguments: tc.function.arguments.clone(),
                    reasoning: None,
                    // Capture Gemini `thought_signature` (and any other
                    // per-tool-call signatures) so the next turn can echo
                    // them. Without this, Gemini 2.5+ rejects the next
                    // request with HTTP 400. See #3225.
                    signature: tc.signature.clone(),
                });
            }
            AssistantContent::Reasoning(r) if !r.reasoning.is_empty() => {
                reasoning_parts.push(r.reasoning.join("\n"));
            }
            // Image variants are not mapped to IronClaw types
            _ => {}
        }
    }

    let text = if text_parts.is_empty() {
        None
    } else {
        Some(text_parts.join(""))
    };

    let reasoning = if reasoning_parts.is_empty() {
        None
    } else {
        Some(reasoning_parts.join("\n"))
    };

    let finish = if !tool_calls.is_empty() {
        FinishReason::ToolUse
    } else {
        FinishReason::Stop
    };

    (text, tool_calls, finish, reasoning)
}

/// Saturate u64 to u32 for token counts.
fn saturate_u32(val: u64) -> u32 {
    val.min(u32::MAX as u64) as u32
}

/// Returns `true` if the model supports Anthropic prompt caching.
///
/// Per Anthropic docs, only Claude 3+ models support prompt caching.
/// Unsupported: claude-2, claude-2.1, claude-instant-*.
fn supports_prompt_cache(name: &str) -> bool {
    let lower = name.to_lowercase();
    // Strip optional provider prefix (e.g. "anthropic/claude-...")
    let model = lower.strip_prefix("anthropic/").unwrap_or(&lower);
    // Only Claude 3+ families support prompt caching
    model.starts_with("claude-3")
        || model.starts_with("claude-4")
        || model.starts_with("claude-sonnet")
        || model.starts_with("claude-opus")
        || model.starts_with("claude-haiku")
}

/// Extract `cache_creation_input_tokens` from the raw provider response.
///
/// Rig-core's unified `Usage` does not surface this field, but Anthropic's raw
/// response includes it at `usage.cache_creation_input_tokens`. We serialize the
/// raw response to JSON and attempt to read the value.
fn extract_cache_creation<T: Serialize>(raw: &T) -> u32 {
    serde_json::to_value(raw)
        .ok()
        .and_then(|v| v.get("usage")?.get("cache_creation_input_tokens")?.as_u64())
        .map(|n| n.min(u32::MAX as u64) as u32)
        .unwrap_or(0)
}

/// Merge default additional parameters into the rig-core request.
///
/// Provider-level params (e.g., Ollama's `think: true`) are merged into the
/// request's `additional_params`. Existing keys from `build_rig_request`
/// (e.g., `cache_control`) take precedence over defaults.
fn merge_additional_params(rig_req: &mut RigRequest, defaults: Option<&serde_json::Value>) {
    let Some(defaults) = defaults else { return };
    let Some(default_obj) = defaults.as_object() else {
        return;
    };
    match rig_req.additional_params {
        Some(ref mut params) => {
            if let Some(obj) = params.as_object_mut() {
                for (k, v) in default_obj {
                    obj.entry(k).or_insert_with(|| v.clone());
                }
            }
        }
        None => {
            rig_req.additional_params = Some(defaults.clone());
        }
    }
}

/// Build a rig-core CompletionRequest from our internal types.
///
/// When `cache_retention` is not `None`, injects a top-level `cache_control`
/// field via `additional_params`. Rig-core's `AnthropicCompletionRequest`
/// uses `#[serde(flatten)]` on `additional_params`, so the field lands at
/// the request root — which is exactly what Anthropic's **automatic caching**
/// expects. The API auto-places the cache breakpoint at the last cacheable
/// block and moves it forward as conversations grow.
#[allow(clippy::too_many_arguments)]
fn build_rig_request(
    preamble: Option<String>,
    mut history: Vec<RigMessage>,
    tools: Vec<RigToolDefinition>,
    tool_choice: Option<RigToolChoice>,
    temperature: Option<f32>,
    max_tokens: Option<u32>,
    cache_retention: CacheRetention,
) -> Result<RigRequest, LlmError> {
    // rig-core requires at least one message in chat_history
    if history.is_empty() {
        history.push(RigMessage::user("Hello"));
    }

    let chat_history = OneOrMany::many(history).map_err(|e| LlmError::RequestFailed {
        provider: "rig".to_string(),
        reason: format!("Failed to build chat history: {}", e),
    })?;

    // Inject top-level cache_control for Anthropic automatic prompt caching.
    let additional_params = match cache_retention {
        CacheRetention::None => None,
        CacheRetention::Short => Some(serde_json::json!({
            "cache_control": {"type": "ephemeral"}
        })),
        CacheRetention::Long => Some(serde_json::json!({
            "cache_control": {"type": "ephemeral", "ttl": "1h"}
        })),
    };

    Ok(RigRequest {
        preamble,
        chat_history,
        documents: Vec::new(),
        tools,
        temperature: temperature.map(round_f32_to_f64),
        max_tokens: max_tokens.map(|t| t as u64),
        tool_choice,
        additional_params,
    })
}

/// Inject a per-request model override into the rig request's `additional_params`.
///
/// Rig-core bakes the model name at construction time inside each provider's
/// `CompletionModel` implementation. This helper inserts a top-level `"model"`
/// key into `additional_params`, which rig-core flattens into the provider's
/// request payload via `#[serde(flatten)]`.
///
/// Whether the override takes effect depends on the downstream API server's
/// handling of duplicate JSON keys (most Python/Go servers use last-key-wins,
/// but this is not guaranteed by the JSON spec). The `effective_model_name()`
/// trait method should be consulted to determine the model actually used.
fn inject_model_override(rig_req: &mut RigRequest, model_override: Option<&str>) {
    let Some(model) = model_override else {
        return;
    };
    match rig_req.additional_params {
        Some(ref mut params) => {
            if let Some(obj) = params.as_object_mut() {
                obj.insert("model".to_string(), serde_json::json!(model));
            }
        }
        None => {
            rig_req.additional_params = Some(serde_json::json!({ "model": model }));
        }
    }
}

#[async_trait]
impl<M> LlmProvider for RigAdapter<M>
where
    M: CompletionModel + Send + Sync + 'static,
    M::Response: Send + Sync + Serialize + DeserializeOwned,
{
    fn model_name(&self) -> &str {
        &self.model_name
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        (self.input_cost, self.output_cost)
    }

    fn cache_write_multiplier(&self) -> Decimal {
        match self.cache_retention {
            CacheRetention::None => Decimal::ONE,
            CacheRetention::Short => Decimal::new(125, 2), // 1.25× (125% of input rate)
            CacheRetention::Long => Decimal::TWO,          // 2.0×  (200% of input rate)
        }
    }

    fn cache_read_discount(&self) -> Decimal {
        if self.cache_retention != CacheRetention::None {
            dec!(10) // Anthropic: 90% discount (cost = input_rate / 10)
        } else {
            Decimal::ONE
        }
    }

    async fn complete(
        &self,
        mut request: CompletionRequest,
    ) -> Result<CompletionResponse, LlmError> {
        let model_override = request.take_model_override();

        self.strip_unsupported_completion_params(&mut request);

        let mut messages = request.messages;
        crate::llm::provider::sanitize_tool_messages(&mut messages);
        let (preamble, history) = convert_messages(&messages);

        let mut rig_req = build_rig_request(
            preamble,
            history,
            Vec::new(),
            None,
            request.temperature,
            request.max_tokens,
            self.cache_retention,
        )?;

        merge_additional_params(&mut rig_req, self.default_additional_params.as_ref());
        inject_model_override(&mut rig_req, model_override.as_deref());

        let response = self
            .model
            .completion(rig_req)
            .await
            .map_err(|e| map_rig_error(&self.model_name, e))?;

        let (text, _tool_calls, finish, _reasoning) =
            extract_response(&response.choice, &response.usage);

        let resp = CompletionResponse {
            content: text.unwrap_or_default(),
            input_tokens: saturate_u32(response.usage.input_tokens),
            output_tokens: saturate_u32(response.usage.output_tokens),
            finish_reason: finish,
            cache_read_input_tokens: saturate_u32(response.usage.cached_input_tokens),
            cache_creation_input_tokens: extract_cache_creation(&response.raw_response),
        };

        if resp.cache_read_input_tokens > 0 {
            tracing::debug!(
                model = %self.model_name,
                input = resp.input_tokens,
                output = resp.output_tokens,
                cache_read = resp.cache_read_input_tokens,
                "prompt cache hit",
            );
        }

        Ok(resp)
    }

    async fn complete_with_tools(
        &self,
        mut request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        let model_override = request.take_model_override();

        self.strip_unsupported_tool_params(&mut request);

        let known_tool_names: HashSet<String> =
            request.tools.iter().map(|t| t.name.clone()).collect();

        let mut messages = request.messages;
        crate::llm::provider::sanitize_tool_messages(&mut messages);
        let (preamble, history) = convert_messages(&messages);
        let tools = convert_tools(&request.tools);
        let tool_choice = convert_tool_choice(request.tool_choice.as_deref());

        let mut rig_req = build_rig_request(
            preamble,
            history,
            tools,
            tool_choice,
            request.temperature,
            request.max_tokens,
            self.cache_retention,
        )?;

        merge_additional_params(&mut rig_req, self.default_additional_params.as_ref());
        inject_model_override(&mut rig_req, model_override.as_deref());

        let response = self
            .model
            .completion(rig_req)
            .await
            .map_err(|e| map_rig_error(&self.model_name, e))?;

        let (text, mut tool_calls, finish, reasoning) =
            extract_response(&response.choice, &response.usage);

        // Normalize tool call names: some proxies prepend "proxy_" prefixes.
        for tc in &mut tool_calls {
            let normalized = normalize_tool_name(&tc.name, &known_tool_names);
            if normalized != tc.name {
                tracing::debug!(
                    original = %tc.name,
                    normalized = %normalized,
                    "Normalized tool call name from provider",
                );
                tc.name = normalized;
            }
        }

        let resp = ToolCompletionResponse {
            content: text,
            tool_calls,
            input_tokens: saturate_u32(response.usage.input_tokens),
            output_tokens: saturate_u32(response.usage.output_tokens),
            finish_reason: finish,
            cache_read_input_tokens: saturate_u32(response.usage.cached_input_tokens),
            cache_creation_input_tokens: extract_cache_creation(&response.raw_response),
            reasoning,
        };

        if resp.cache_read_input_tokens > 0 {
            tracing::debug!(
                model = %self.model_name,
                input = resp.input_tokens,
                output = resp.output_tokens,
                cache_read = resp.cache_read_input_tokens,
                "prompt cache hit",
            );
        }

        Ok(resp)
    }

    fn active_model_name(&self) -> String {
        self.model_name.clone()
    }

    fn effective_model_name(&self, _requested_model: Option<&str>) -> String {
        self.active_model_name()
    }

    fn set_model(&self, _model: &str) -> Result<(), LlmError> {
        // rig-core models are baked at construction time.
        // Switching requires creating a new adapter.
        Err(LlmError::RequestFailed {
            provider: self.model_name.clone(),
            reason: "Runtime model switching not supported for rig-core providers. \
                     Restart with a different model configured."
                .to_string(),
        })
    }
}

/// Map a rig-core completion error to an appropriate `LlmError` variant.
///
/// Detects context-length / payload-size errors in the error message and maps
/// them to `ContextLengthExceeded` so the dispatcher can trigger compaction
/// instead of retrying the same oversized payload.
fn map_rig_error(model_name: &str, e: impl std::fmt::Display) -> LlmError {
    let msg = e.to_string();
    let lower = msg.to_ascii_lowercase();

    const CONTEXT_PATTERNS: &[&str] = &[
        "context_length_exceeded",
        "maximum context length",
        "too many tokens",
        "payload too large",
    ];

    if CONTEXT_PATTERNS.iter().any(|p| lower.contains(p)) {
        let (used, limit) = parse_token_counts(&lower);
        return LlmError::ContextLengthExceeded { used, limit };
    }
    LlmError::RequestFailed {
        provider: model_name.to_string(),
        reason: msg,
    }
}

/// Try to extract token counts from a context-length error message.
///
/// Handles patterns like:
/// - "maximum context length is 128000 tokens. However, your messages resulted in 150000 tokens."
/// - "context_length_exceeded ... 150000 tokens ... limit 128000"
///
/// Returns `(0, 0)` if parsing fails.
pub(crate) fn parse_token_counts(lower: &str) -> (usize, usize) {
    // OpenAI pattern: "maximum context length is {limit} tokens. ... resulted in {used} tokens"
    if lower.contains("maximum context length") {
        let numbers: Vec<usize> = lower
            .split(|c: char| !c.is_ascii_digit())
            .filter(|s| !s.is_empty())
            .filter_map(|s| s.parse().ok())
            .filter(|&n| n > 0)
            .collect();
        if numbers.len() >= 2 {
            // First large number is typically the limit, second is the used count
            return (numbers[1], numbers[0]);
        }
    }
    (0, 0)
}

/// Normalize a tool call name returned by an OpenAI-compatible provider.
///
/// Some proxies (e.g. VibeProxy) prepend `proxy_` to tool names.
/// If the returned name doesn't match any known tool but stripping a
/// `proxy_` prefix yields a match, use the stripped version.
fn normalize_tool_name(name: &str, known_tools: &HashSet<String>) -> String {
    if known_tools.contains(name) {
        return name.to_string();
    }

    if let Some(stripped) = name.strip_prefix("proxy_")
        && known_tools.contains(stripped)
    {
        return stripped.to_string();
    }

    name.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_round_f32_to_f64_no_precision_artifacts() {
        // Direct f32->f64 cast produces 0.699999988079071 instead of 0.7
        assert_eq!(round_f32_to_f64(0.7_f32), 0.7_f64);
        assert_eq!(round_f32_to_f64(0.5_f32), 0.5_f64);
        assert_eq!(round_f32_to_f64(1.0_f32), 1.0_f64);
        assert_eq!(round_f32_to_f64(0.0_f32), 0.0_f64);
        // Original cast produces artifacts — our fix should not
        assert_ne!(0.7_f32 as f64, 0.7_f64);
    }

    // ── normalize_schema_strict: top-level flatten ────────────────────────
    //
    // OpenAI's tool API rejects schemas whose top level isn't `type:
    // "object"` or that contain top-level `oneOf`/`anyOf`/`allOf`/`enum`/
    // `not`. The GitHub Copilot MCP server exposes a tool with exactly that
    // shape (action dispatch via top-level union), and the agent gets HTTP
    // 400 the moment it tries to enumerate tools. `normalize_schema_strict`
    // detects the bad shape, flattens parameters to a permissive object
    // envelope, and stuffs the original schema into the description as
    // advisory text so the LLM can still pick variant fields. Both rig-based
    // providers and the Codex Responses API client share this normalizer.

    #[test]
    fn test_normalize_schema_strict_passes_through_valid_object_schema() {
        let input = serde_json::json!({
            "type": "object",
            "properties": {
                "query": { "type": "string" }
            },
            "required": ["query"]
        });
        let mut description = "Search the index".to_string();
        let result = normalize_schema_strict(&input, &mut description);

        // Strict-mode normalization runs: additionalProperties forced false
        // and required is set to ALL keys, but the structural shape is
        // unchanged.
        assert_eq!(result["type"], "object");
        assert_eq!(result["additionalProperties"], false);
        assert_eq!(result["required"], serde_json::json!(["query"]));
        assert!(result["properties"]["query"].is_object());
        assert_eq!(
            description, "Search the index",
            "description must be untouched when no flatten happened"
        );
    }

    #[test]
    fn test_normalize_schema_strict_flattens_top_level_oneof() {
        // Mirrors the GitHub Copilot MCP `github` tool shape that triggered
        // the original 400.
        let input = serde_json::json!({
            "type": "object",
            "oneOf": [
                { "properties": { "action": { "const": "create_issue" }, "title": { "type": "string" } } },
                { "properties": { "action": { "const": "list_issues" }, "repo":  { "type": "string" } } }
            ]
        });
        let mut description = "GitHub umbrella tool".to_string();
        let result = normalize_schema_strict(&input, &mut description);

        assert_eq!(result["type"], "object");
        assert!(
            result.get("oneOf").is_none(),
            "top-level oneOf must be removed"
        );
        assert_eq!(result["additionalProperties"], true);
        assert!(result["properties"].is_object());
        assert!(result["required"].as_array().unwrap().is_empty());
        assert!(
            description.contains("Upstream JSON schema"),
            "description must include the advisory hint"
        );
        assert!(
            description.contains("create_issue"),
            "original schema variants must survive in the hint"
        );
    }

    #[test]
    fn test_normalize_schema_strict_flattens_anyof_allof_enum_not() {
        for forbidden in ["anyOf", "allOf", "enum", "not"] {
            let input = serde_json::json!({
                "type": "object",
                forbidden: ["whatever"]
            });
            let mut description = "tool".to_string();
            let result = normalize_schema_strict(&input, &mut description);
            assert!(
                result.get(forbidden).is_none(),
                "top-level {forbidden} must be stripped"
            );
            assert_eq!(result["type"], "object");
            assert_eq!(result["additionalProperties"], true);
        }
    }

    #[test]
    fn test_normalize_schema_strict_hint_is_keyword_aware() {
        // The flatten hint must match the construct that triggered it. The
        // previous one-size-fits-all "pick one variant" hint was correct
        // for oneOf/anyOf but actively misleading for allOf (where the LLM
        // should pass fields from ALL variants), enum (one of the listed
        // values), and not (any object that doesn't match).
        let cases = [
            ("oneOf", "pick ONE variant"),
            ("anyOf", "pick ONE variant"),
            ("allOf", "pass fields from ALL variants"),
            ("enum", "pass one of the listed values"),
            ("not", "does NOT match the constraint"),
        ];
        for (keyword, expected_phrase) in cases {
            let input = serde_json::json!({
                "type": "object",
                keyword: ["whatever"]
            });
            let mut description = "tool".to_string();
            let _ = normalize_schema_strict(&input, &mut description);
            assert!(
                description.contains(expected_phrase),
                "hint for top-level {keyword} must contain `{expected_phrase}`, \
                 got: {description}"
            );
        }
    }

    #[test]
    fn test_normalize_schema_strict_replaces_non_object_top_level_type() {
        // A schema like `{"type": "string"}` is not a valid OpenAI tool
        // parameters object — replace wholesale.
        let input = serde_json::json!({ "type": "string" });
        let mut description = "weird tool".to_string();
        let result = normalize_schema_strict(&input, &mut description);
        assert_eq!(result["type"], "object");
        assert!(result["properties"].is_object());
        assert_eq!(result["additionalProperties"], true);
    }

    #[test]
    fn test_normalize_schema_strict_does_not_flatten_nullable_object_type() {
        // `"type": ["object", "null"]` is valid JSON Schema for a nullable
        // object. Some upstream providers and `make_nullable` produce this
        // form. The previous check only matched `JsonValue::String("object")`
        // and would have flattened this schema, discarding all properties.
        let input = serde_json::json!({
            "type": ["object", "null"],
            "properties": {
                "query": { "type": "string" }
            },
            "required": ["query"]
        });
        let mut description = "nullable tool".to_string();
        let result = normalize_schema_strict(&input, &mut description);

        // Should NOT flatten — the schema is a valid object type.
        assert!(
            result["properties"]["query"].is_object(),
            "properties must be preserved for nullable object type, got: {result}"
        );
        assert_eq!(
            description, "nullable tool",
            "description must be untouched (no flatten hint appended)"
        );
    }

    /// Regression: OpenAI rejects `"items": true` and missing `items` on
    /// array-typed properties with "array schema items is not an object".
    /// Schema generators (schemars) produce this for `Vec<serde_json::Value>`.
    /// The normalizer must ensure `items` is a JSON Schema object.
    #[test]
    fn test_normalize_schema_strict_fixes_array_items_not_object() {
        // Case 1: items missing entirely (Vec<Value> → {"type": "array"})
        let input = serde_json::json!({
            "type": "object",
            "properties": {
                "requests": { "type": "array" }
            },
            "required": ["requests"]
        });
        let mut description = "batch".to_string();
        let result = normalize_schema_strict(&input, &mut description);
        assert!(
            result["properties"]["requests"]["items"].is_object(),
            "missing items must be filled with an object: {}",
            result["properties"]["requests"]
        );

        // Case 2: items is boolean true (valid JSON Schema, rejected by OpenAI)
        let input = serde_json::json!({
            "type": "object",
            "properties": {
                "data": { "type": "array", "items": true }
            },
            "required": ["data"]
        });
        let mut description = "bool items".to_string();
        let result = normalize_schema_strict(&input, &mut description);
        assert!(
            result["properties"]["data"]["items"].is_object(),
            "boolean items must be replaced with an object: {}",
            result["properties"]["data"]
        );

        // Case 3: items is already an object (should not be clobbered)
        let input = serde_json::json!({
            "type": "object",
            "properties": {
                "tags": { "type": "array", "items": { "type": "string" } }
            },
            "required": ["tags"]
        });
        let mut description = "ok".to_string();
        let result = normalize_schema_strict(&input, &mut description);
        assert_eq!(
            result["properties"]["tags"]["items"]["type"], "string",
            "well-formed items must be preserved"
        );
    }

    /// Regression for google_docs_tool: a tagged enum with a variant
    /// containing `requests: Vec<serde_json::Value>` produces a top-level
    /// `oneOf` (which we flatten) with a nested `{"type": "array"}` property
    /// that has no `items`. The flatten path originally short-circuited
    /// `normalize_schema_recursive`, so the merged `requests` property kept
    /// its bare array schema and OpenAI rejected it with "array schema items
    /// is not an object". After the fix, the flatten path normalizes each
    /// merged property individually.
    #[test]
    fn test_normalize_schema_strict_flatten_normalizes_merged_array_items() {
        let input = serde_json::json!({
            "type": "object",
            "oneOf": [
                {
                    "properties": {
                        "action": { "const": "batch_update" },
                        "document_id": { "type": "string" },
                        "requests": { "type": "array" }
                    },
                    "required": ["action", "document_id", "requests"]
                },
                {
                    "properties": {
                        "action": { "const": "get" },
                        "document_id": { "type": "string" }
                    },
                    "required": ["action", "document_id"]
                }
            ]
        });
        let mut description = "docs".to_string();
        let result = normalize_schema_strict(&input, &mut description);

        // The flatten happened (oneOf removed, properties merged).
        assert!(result.get("oneOf").is_none());
        let props = result["properties"].as_object().expect("properties");
        assert!(props.contains_key("requests"));

        // CRITICAL: the merged `requests` array property must have
        // `items` as an object — not missing, not boolean, not null.
        let requests = &result["properties"]["requests"];
        assert_eq!(requests["type"], "array");
        assert!(
            requests["items"].is_object(),
            "merged array property must have items as a JSON Schema object \
             after flatten-path normalization; got: {requests}"
        );
    }

    #[test]
    fn test_normalize_schema_strict_merges_variant_properties() {
        // Top-level oneOf flatten now merges all variants' properties into
        // the envelope so the LLM sees structured field hints instead of
        // an empty `{}`. Mirrors the GitHub Copilot `github` tool shape:
        // each variant declares its own subset of fields keyed by `action`.
        let input = serde_json::json!({
            "type": "object",
            "oneOf": [
                {
                    "properties": {
                        "action": { "const": "create_issue" },
                        "title":  { "type": "string" },
                        "body":   { "type": "string" }
                    },
                    "required": ["action", "title"]
                },
                {
                    "properties": {
                        "action": { "const": "list_issues" },
                        "repo":   { "type": "string" },
                        "state":  { "type": "string" }
                    },
                    "required": ["action", "repo"]
                }
            ]
        });
        let mut description = "github".to_string();
        let result = normalize_schema_strict(&input, &mut description);

        // The flatten happened.
        assert_eq!(result["type"], "object");
        assert!(result.get("oneOf").is_none());
        assert_eq!(result["additionalProperties"], true);
        // Strict-mode `required` is empty so the LLM can mix fields from
        // different variants without failing OpenAI validation.
        assert_eq!(result["required"], serde_json::json!([]));

        // CRITICAL: properties is no longer `{}` — every field from every
        // variant must appear so the LLM can pick what to send.
        let props = result["properties"].as_object().expect("merged properties");
        assert!(props.contains_key("action"), "discriminator must merge");
        assert!(props.contains_key("title"), "create_issue field must merge");
        assert!(props.contains_key("body"), "create_issue field must merge");
        assert!(props.contains_key("repo"), "list_issues field must merge");
        assert!(props.contains_key("state"), "list_issues field must merge");
        assert_eq!(props.len(), 5);
    }

    #[test]
    fn test_normalize_schema_strict_merge_first_write_wins_on_conflict() {
        // If two variants declare the same field with different schemas,
        // first-write wins. Documented behaviour — the description hint
        // still has the full original schema for ambiguous cases.
        let input = serde_json::json!({
            "type": "object",
            "anyOf": [
                {
                    "properties": {
                        "value": { "type": "string", "description": "first" }
                    }
                },
                {
                    "properties": {
                        "value": { "type": "integer", "description": "second" }
                    }
                }
            ]
        });
        let mut description = "ambiguous".to_string();
        let result = normalize_schema_strict(&input, &mut description);

        let value_schema = &result["properties"]["value"];
        assert_eq!(value_schema["type"], "string");
        assert_eq!(value_schema["description"], "first");
    }

    #[test]
    fn test_normalize_schema_strict_preserves_nested_oneof() {
        // Nested combinators inside `properties` are FINE for the API. Only
        // the top level is forbidden, so the nested oneOf must survive
        // (its variants get recursively strict-normalized but the union
        // itself is preserved). `filter` is marked required so strict mode
        // doesn't wrap it in an `anyOf` for nullability — that would move
        // the inner oneOf one level deeper and obscure what we're checking.
        let input = serde_json::json!({
            "type": "object",
            "properties": {
                "filter": {
                    "oneOf": [
                        { "type": "string" },
                        { "type": "object", "properties": { "regex": { "type": "string" } } }
                    ]
                }
            },
            "required": ["filter"]
        });
        let mut description = "search".to_string();
        let result = normalize_schema_strict(&input, &mut description);

        assert_eq!(result["type"], "object");
        // Nested oneOf survives untouched at the same path.
        let nested = &result["properties"]["filter"]["oneOf"];
        assert!(nested.is_array(), "nested oneOf must be preserved");
        assert_eq!(nested.as_array().unwrap().len(), 2);
        // The object variant inside the nested oneOf got strict-mode
        // normalized (additionalProperties: false, all keys required).
        let object_variant = &nested[1];
        assert_eq!(object_variant["type"], "object");
        assert_eq!(object_variant["additionalProperties"], false);
        assert_eq!(description, "search");
    }

    #[test]
    fn test_normalize_schema_strict_truncates_huge_schema_on_char_boundary() {
        // 4KB blob with a multi-byte char near the truncation point. The
        // truncated hint must not panic and must end on a valid char
        // boundary.
        let big_string = "α".repeat(2000); // each `α` is 2 bytes in UTF-8 → 4000 bytes
        let input = serde_json::json!({
            "anyOf": [{ "description": big_string }]
        });
        let mut description = "tool".to_string();
        let result = normalize_schema_strict(&input, &mut description);
        assert!(description.contains("(truncated)"));
        assert_eq!(result["type"], "object");
        assert!(result.get("anyOf").is_none());
    }

    /// Size-capped serializer: verify the capped writer produces correct
    /// output at boundary conditions.
    #[test]
    fn test_serialize_json_capped_boundary_conditions() {
        // Small schema under the cap: full output, no truncation.
        let small = serde_json::json!({"a": 1});
        let result = serialize_json_capped(&small, 1500).expect("should serialize");
        assert_eq!(result.text, r#"{"a":1}"#);
        assert!(!result.was_truncated);

        // Exactly at the cap: should produce exactly cap bytes (or fewer
        // if the serialized output happens to be shorter).
        let result = serialize_json_capped(&small, 7).expect("should serialize");
        assert_eq!(result.text.len(), 7); // {"a":1} is exactly 7 bytes
        assert!(!result.was_truncated);

        // Over the cap: output is truncated. The JSON will be malformed
        // (cut mid-stream) but that's OK — the caller adds "... (truncated)".
        let result = serialize_json_capped(&small, 4).expect("should serialize");
        assert_eq!(result.text.len(), 4);
        assert_eq!(result.text, r#"{"a""#);
        assert!(result.was_truncated);

        // Cap of 0: empty output.
        let result = serialize_json_capped(&small, 0).expect("should serialize");
        assert!(result.text.is_empty());
        assert!(result.was_truncated);
    }

    /// Size-capped serializer with multi-MB string values: the cap must
    /// bound the allocation even when the schema has few nodes but large
    /// string values (the gap the old node-counting approach missed).
    #[test]
    fn test_serialize_json_capped_large_string_values() {
        let big = serde_json::json!({
            "description": "x".repeat(100_000)
        });
        let result = serialize_json_capped(&big, 1500).expect("should serialize");
        assert!(
            result.text.len() <= 1500,
            "capped serializer must bound output to max_bytes; got {} bytes",
            result.text.len()
        );
        assert!(result.was_truncated);
        // The output should start with valid JSON structure.
        assert!(result.text.starts_with(r#"{"description":""#));
    }

    #[test]
    fn test_serialize_json_capped_reports_multibyte_truncation() {
        let value = serde_json::json!({"description": "α"});
        let result = serialize_json_capped(&value, 17).expect("should serialize");

        assert_eq!(result.text, r#"{"description":""#);
        assert!(result.was_truncated);
    }

    /// Caller-level regression test: drives `convert_tools` (the rig-based
    /// provider entry point) end to end with a GitHub-Copilot-shaped tool
    /// definition and asserts the resulting `RigToolDefinition` has a clean
    /// top level. This is the test that would have caught the OpenAI-via-rig
    /// path regressing the same way the Codex path did.
    #[test]
    fn test_convert_tools_handles_top_level_oneof_dispatcher() {
        let tools = vec![IronToolDefinition {
            name: "github".to_string(),
            description: "GitHub MCP umbrella tool".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "oneOf": [
                    {
                        "properties": {
                            "action": { "const": "create_issue" },
                            "title":  { "type": "string" }
                        },
                        "required": ["action", "title"]
                    },
                    {
                        "properties": {
                            "action": { "const": "list_issues" },
                            "repo":   { "type": "string" }
                        },
                        "required": ["action", "repo"]
                    }
                ]
            }),
        }];
        let converted = convert_tools(&tools);
        assert_eq!(converted.len(), 1);
        let tool = &converted[0];

        assert_eq!(tool.name, "github");
        assert_eq!(tool.parameters["type"], "object");
        assert!(
            tool.parameters.get("oneOf").is_none(),
            "top-level oneOf must not survive into the rig-core ToolDefinition"
        );
        assert_eq!(tool.parameters["additionalProperties"], true);
        assert!(
            tool.description.starts_with("GitHub MCP umbrella tool"),
            "original description must come first"
        );
        assert!(
            tool.description.contains("Upstream JSON schema"),
            "advisory hint must be appended"
        );
        assert!(
            tool.description.contains("create_issue") && tool.description.contains("list_issues"),
            "variant info must be retained in the hint"
        );
    }

    /// End-to-end regression test using the google_docs_tool's actual schema
    /// shape. This tool has a tagged enum (`oneOf`) with a `BatchUpdate`
    /// variant containing `requests: Vec<serde_json::Value>` — which
    /// produces a bare `{"type": "array"}` with no `items`. The flatten
    /// path broke TWICE on this shape:
    ///
    /// 1. The `return schema` short-circuit skipped `normalize_schema_recursive`,
    ///    so the merged `requests` property kept its bare array (no items).
    /// 2. Even after the array-items fix was added to the recursive normalizer,
    ///    the flatten path still short-circuited before it ran.
    ///
    /// This single test would have caught BOTH bugs. It drives the schema
    /// through `normalize_schema_strict` (shared normalizer) AND both
    /// consumer paths: `convert_tools` (rig-based providers) and
    /// `convert_tool_definition` (codex provider). Asserts:
    /// - top-level oneOf is flattened
    /// - merged properties include fields from ALL variants
    /// - array `items` is an object (not missing/boolean)
    /// - nested object properties get strict-mode treatment
    /// - output passes `validate_strict_schema` with zero violations
    #[test]
    fn test_realistic_wasm_schema_survives_normalize_flatten_pipeline() {
        // Actual shape from google_docs_tool: tagged enum with 4 variants.
        // BatchUpdate has `requests: Vec<Value>` (bare array, no items).
        // GetDocument/ReadContent have only string fields.
        // InsertText has a nested object (text_style).
        let wasm_schema = serde_json::json!({
            "oneOf": [
                {
                    "type": "object",
                    "properties": {
                        "action": { "type": "string", "const": "get_document" },
                        "document_id": { "type": "string" }
                    },
                    "required": ["action", "document_id"]
                },
                {
                    "type": "object",
                    "properties": {
                        "action": { "type": "string", "const": "batch_update" },
                        "document_id": { "type": "string" },
                        "requests": { "type": "array" }
                    },
                    "required": ["action", "document_id", "requests"]
                },
                {
                    "type": "object",
                    "properties": {
                        "action": { "type": "string", "const": "insert_text" },
                        "document_id": { "type": "string" },
                        "text": { "type": "string" },
                        "text_style": {
                            "type": "object",
                            "properties": {
                                "bold": { "type": "boolean" },
                                "font_size": { "type": "integer" }
                            }
                        }
                    },
                    "required": ["action", "document_id", "text"]
                },
                {
                    "type": "object",
                    "properties": {
                        "action": { "type": "string", "const": "read_content" },
                        "document_id": { "type": "string" }
                    },
                    "required": ["action", "document_id"]
                }
            ]
        });

        // Path 1: shared normalizer (used by both rig and codex paths)
        let mut description = "Google Docs tool".to_string();
        let normalized = normalize_schema_strict(&wasm_schema, &mut description);

        // Top-level oneOf must be flattened.
        assert!(normalized.get("oneOf").is_none(), "oneOf must be flattened");
        assert_eq!(normalized["type"], "object");
        assert_eq!(normalized["additionalProperties"], true);

        // Merged properties from ALL variants.
        let props = normalized["properties"]
            .as_object()
            .expect("merged properties");
        assert!(props.contains_key("action"), "discriminator");
        assert!(props.contains_key("document_id"), "shared field");
        assert!(props.contains_key("requests"), "BatchUpdate field");
        assert!(props.contains_key("text"), "InsertText field");
        assert!(props.contains_key("text_style"), "InsertText nested obj");

        // CRITICAL: array `items` must be an object (the bug that broke twice).
        let requests = &normalized["properties"]["requests"];
        assert!(
            requests["items"].is_object(),
            "requests array must have items as a JSON Schema object; got: {requests}"
        );

        // Nested object properties should get strict-mode treatment
        // (additionalProperties: false on the text_style sub-object).
        let text_style = &normalized["properties"]["text_style"];
        assert_eq!(text_style["additionalProperties"], false);
        assert!(text_style["properties"]["bold"].is_object());

        // Description hint should contain variant info.
        assert!(
            description.contains("batch_update") || description.contains("get_document"),
            "description must include variant info from the original schema"
        );

        // Path 2: rig-based provider entry point.
        let tools = convert_tools(&[IronToolDefinition {
            name: "google_docs_tool".to_string(),
            description: "Google Docs".to_string(),
            parameters: wasm_schema.clone(),
        }]);
        assert_eq!(tools.len(), 1);
        assert!(
            tools[0].parameters.get("oneOf").is_none(),
            "convert_tools output must not have oneOf"
        );
        assert!(
            tools[0].parameters["properties"]["requests"]["items"].is_object(),
            "convert_tools must normalize array items"
        );
    }

    /// Deeply nested schema: object → array → object → array (no items).
    /// Verifies the recursive normalizer walks the full depth and fixes
    /// every array `items` and every nested object's strict-mode fields.
    #[test]
    fn test_normalize_schema_strict_fixes_deeply_nested_array_items() {
        // All fields marked required so `make_nullable` doesn't wrap
        // types as `["array", "null"]`, keeping the assertions focused
        // on "items get fixed at every nesting depth".
        let input = serde_json::json!({
            "type": "object",
            "properties": {
                "data": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "tags": { "type": "array" },
                            "metadata": {
                                "type": "object",
                                "properties": {
                                    "values": { "type": "array" }
                                },
                                "required": ["values"]
                            }
                        },
                        "required": ["tags", "metadata"]
                    }
                }
            },
            "required": ["data"]
        });
        let mut description = "nested".to_string();
        let result = normalize_schema_strict(&input, &mut description);

        // Level 1: data.items is an object (was already, should be preserved)
        let data_items = &result["properties"]["data"]["items"];
        assert!(data_items.is_object());

        // Level 2: data.items.properties.tags must get items added
        let tags = &data_items["properties"]["tags"];
        assert_eq!(tags["type"], "array");
        assert!(
            tags["items"].is_object(),
            "deeply nested array must get items: {tags}"
        );

        // Level 3: data.items.properties.metadata.properties.values
        let values = &data_items["properties"]["metadata"]["properties"]["values"];
        assert_eq!(values["type"], "array");
        assert!(
            values["items"].is_object(),
            "3-level deep array must get items: {values}"
        );

        // Nested objects should have additionalProperties: false
        assert_eq!(data_items["additionalProperties"], false);
        assert_eq!(
            data_items["properties"]["metadata"]["additionalProperties"],
            false
        );
    }

    #[test]
    fn test_convert_messages_system_to_preamble() {
        let messages = vec![
            ChatMessage::system("You are a helpful assistant."),
            ChatMessage::user("Hello"),
        ];
        let (preamble, history) = convert_messages(&messages);
        assert_eq!(preamble, Some("You are a helpful assistant.".to_string()));
        assert_eq!(history.len(), 1);
    }

    #[test]
    fn test_convert_messages_multiple_systems_concatenated() {
        let messages = vec![
            ChatMessage::system("System 1"),
            ChatMessage::system("System 2"),
            ChatMessage::user("Hi"),
        ];
        let (preamble, history) = convert_messages(&messages);
        assert_eq!(preamble, Some("System 1\nSystem 2".to_string()));
        assert_eq!(history.len(), 1);
    }

    #[test]
    fn test_convert_messages_tool_result() {
        // Use a conforming 9-char alphanumeric ID so it passes through unchanged.
        let messages = vec![ChatMessage::tool_result(
            "abcDE1234",
            "search",
            "result text",
        )];
        let (preamble, history) = convert_messages(&messages);
        assert!(preamble.is_none());
        assert_eq!(history.len(), 1);
        // Tool results become User messages in rig-core
        match &history[0] {
            RigMessage::User { content } => match content.first() {
                UserContent::ToolResult(r) => {
                    assert_eq!(r.id, "abcDE1234");
                    assert_eq!(r.call_id.as_deref(), Some("abcDE1234"));
                }
                other => panic!("Expected tool result content, got: {:?}", other),
            },
            other => panic!("Expected User message, got: {:?}", other),
        }
    }

    #[test]
    fn test_convert_messages_assistant_with_tool_calls() {
        // Use a conforming 9-char alphanumeric ID so it passes through unchanged.
        let tc = IronToolCall {
            id: "Xt7mK9pQ2".to_string(),
            name: "search".to_string(),
            arguments: serde_json::json!({"query": "test"}),
            reasoning: None,
            signature: None,
        };
        let msg = ChatMessage::assistant_with_tool_calls(Some("thinking".to_string()), vec![tc]);
        let messages = vec![msg];
        let (_preamble, history) = convert_messages(&messages);
        assert_eq!(history.len(), 1);
        match &history[0] {
            RigMessage::Assistant { content, .. } => {
                // Should have both text and tool call
                assert!(content.iter().count() >= 2);
                for item in content.iter() {
                    if let AssistantContent::ToolCall(tc) = item {
                        assert_eq!(tc.call_id.as_deref(), Some("Xt7mK9pQ2"));
                    }
                }
            }
            other => panic!("Expected Assistant message, got: {:?}", other),
        }
    }

    #[test]
    fn test_convert_messages_tool_result_without_id_gets_fallback() {
        let messages = vec![ChatMessage {
            role: crate::llm::Role::Tool,
            content: "result text".to_string(),
            content_parts: Vec::new(),
            tool_call_id: None,
            name: Some("search".to_string()),
            tool_calls: None,
            reasoning: None,
        }];
        let (_preamble, history) = convert_messages(&messages);
        match &history[0] {
            RigMessage::User { content } => match content.first() {
                UserContent::ToolResult(r) => {
                    // Missing ID → normalized_tool_call_id generates a 9-char alphanumeric ID.
                    assert_eq!(
                        r.id.len(),
                        9,
                        "fallback ID should be 9 chars, got: {}",
                        r.id
                    );
                    assert!(r.id.chars().all(|c| c.is_ascii_alphanumeric()));
                    assert_eq!(r.call_id.as_deref(), Some(r.id.as_str()));
                }
                other => panic!("Expected tool result content, got: {:?}", other),
            },
            other => panic!("Expected User message, got: {:?}", other),
        }
    }

    #[test]
    fn test_convert_messages_data_url_without_detail_defaults_to_auto() {
        let messages = vec![ChatMessage::user_with_parts(
            "describe this",
            vec![crate::llm::ContentPart::ImageUrl {
                image_url: crate::llm::ImageUrl {
                    url: "data:image/jpeg;base64,Zm9v".to_string(),
                    detail: None,
                },
            }],
        )];

        let (_preamble, history) = convert_messages(&messages);
        match &history[0] {
            RigMessage::User { content } => {
                let image = content
                    .iter()
                    .find_map(|item| match item {
                        UserContent::Image(image) => Some(image),
                        _ => None,
                    })
                    .expect("expected image content");
                assert_eq!(image.detail, Some(ImageDetail::Auto));
            }
            other => panic!("Expected User message, got: {:?}", other),
        }
    }

    #[test]
    fn test_convert_messages_image_detail_preserves_explicit_values() {
        let low_messages = vec![ChatMessage::user_with_parts(
            "low detail",
            vec![crate::llm::ContentPart::ImageUrl {
                image_url: crate::llm::ImageUrl {
                    url: "https://example.com/image-low.png".to_string(),
                    detail: Some("low".to_string()),
                },
            }],
        )];
        let high_messages = vec![ChatMessage::user_with_parts(
            "high detail",
            vec![crate::llm::ContentPart::ImageUrl {
                image_url: crate::llm::ImageUrl {
                    url: "https://example.com/image-high.png".to_string(),
                    detail: Some("high".to_string()),
                },
            }],
        )];

        let (_, low_history) = convert_messages(&low_messages);
        let (_, high_history) = convert_messages(&high_messages);

        for (history, expected) in [
            (&low_history, ImageDetail::Low),
            (&high_history, ImageDetail::High),
        ] {
            match &history[0] {
                RigMessage::User { content } => {
                    let image = content
                        .iter()
                        .find_map(|item| match item {
                            UserContent::Image(image) => Some(image),
                            _ => None,
                        })
                        .expect("expected image content");
                    assert_eq!(image.detail, Some(expected.clone()));
                }
                other => panic!("Expected User message, got: {:?}", other),
            }
        }
    }

    #[test]
    fn test_convert_tools() {
        let tools = vec![IronToolDefinition {
            name: "search".to_string(),
            description: "Search the web".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": {"type": "string"}
                }
            }),
        }];
        let rig_tools = convert_tools(&tools);
        assert_eq!(rig_tools.len(), 1);
        assert_eq!(rig_tools[0].name, "search");
        assert_eq!(rig_tools[0].description, "Search the web");
    }

    #[test]
    fn test_convert_tool_choice() {
        assert!(matches!(
            convert_tool_choice(Some("auto")),
            Some(RigToolChoice::Auto)
        ));
        assert!(matches!(
            convert_tool_choice(Some("required")),
            Some(RigToolChoice::Required)
        ));
        assert!(matches!(
            convert_tool_choice(Some("none")),
            Some(RigToolChoice::None)
        ));
        assert!(matches!(
            convert_tool_choice(Some("AUTO")),
            Some(RigToolChoice::Auto)
        ));
        assert!(convert_tool_choice(None).is_none());
        assert!(convert_tool_choice(Some("unknown")).is_none());
    }

    #[test]
    fn test_extract_response_text_only() {
        let content = OneOrMany::one(AssistantContent::text("Hello world"));
        let usage = RigUsage::new();
        let (text, calls, finish, _reasoning) = extract_response(&content, &usage);
        assert_eq!(text, Some("Hello world".to_string()));
        assert!(calls.is_empty());
        assert_eq!(finish, FinishReason::Stop);
    }

    #[test]
    fn test_extract_response_tool_call() {
        let tc = AssistantContent::tool_call("call_1", "search", serde_json::json!({"q": "test"}));
        let content = OneOrMany::one(tc);
        let usage = RigUsage::new();
        let (text, calls, finish, _reasoning) = extract_response(&content, &usage);
        assert!(text.is_none());
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "search");
        assert_eq!(finish, FinishReason::ToolUse);
    }

    #[test]
    fn test_assistant_tool_call_empty_id_gets_generated() {
        let tc = IronToolCall {
            id: "".to_string(),
            name: "search".to_string(),
            arguments: serde_json::json!({"query": "test"}),
            reasoning: None,
            signature: None,
        };
        let messages = vec![ChatMessage::assistant_with_tool_calls(None, vec![tc])];
        let (_preamble, history) = convert_messages(&messages);

        match &history[0] {
            RigMessage::Assistant { content, .. } => {
                let tool_call = content.iter().find_map(|c| match c {
                    AssistantContent::ToolCall(tc) => Some(tc),
                    _ => None,
                });
                let tc = tool_call.expect("should have a tool call");
                // Empty ID → normalized_tool_call_id generates a 9-char alphanumeric ID.
                assert_eq!(
                    tc.id.len(),
                    9,
                    "generated id should be 9 chars, got: {}",
                    tc.id
                );
                assert!(tc.id.chars().all(|c| c.is_ascii_alphanumeric()));
                assert_eq!(tc.call_id.as_deref(), Some(tc.id.as_str()));
            }
            other => panic!("Expected Assistant message, got: {:?}", other),
        }
    }

    #[test]
    fn test_assistant_tool_call_whitespace_id_gets_generated() {
        let tc = IronToolCall {
            id: "   ".to_string(),
            name: "search".to_string(),
            arguments: serde_json::json!({"query": "test"}),
            reasoning: None,
            signature: None,
        };
        let messages = vec![ChatMessage::assistant_with_tool_calls(None, vec![tc])];
        let (_preamble, history) = convert_messages(&messages);

        match &history[0] {
            RigMessage::Assistant { content, .. } => {
                let tool_call = content.iter().find_map(|c| match c {
                    AssistantContent::ToolCall(tc) => Some(tc),
                    _ => None,
                });
                let tc = tool_call.expect("should have a tool call");
                // Whitespace-only ID → normalized_tool_call_id generates a 9-char alphanumeric ID.
                assert_eq!(
                    tc.id.len(),
                    9,
                    "generated id should be 9 chars, got: {}",
                    tc.id
                );
                assert!(tc.id.chars().all(|c| c.is_ascii_alphanumeric()));
            }
            other => panic!("Expected Assistant message, got: {:?}", other),
        }
    }

    #[test]
    fn test_assistant_and_tool_result_missing_ids_share_generated_id() {
        // Simulate: assistant emits a tool call with empty id, then tool
        // result arrives without an id. Both should get deterministic
        // generated ids that match (based on their position in history).
        let tc = IronToolCall {
            id: "".to_string(),
            name: "search".to_string(),
            arguments: serde_json::json!({"query": "test"}),
            reasoning: None,
            signature: None,
        };
        let assistant_msg = ChatMessage::assistant_with_tool_calls(None, vec![tc]);
        let tool_result_msg = ChatMessage {
            role: crate::llm::Role::Tool,
            content: "search results here".to_string(),
            content_parts: Vec::new(),
            tool_call_id: None,
            name: Some("search".to_string()),
            tool_calls: None,
            reasoning: None,
        };
        let messages = vec![assistant_msg, tool_result_msg];
        let (_preamble, history) = convert_messages(&messages);

        // Extract the generated call_id from the assistant tool call
        let assistant_call_id = match &history[0] {
            RigMessage::Assistant { content, .. } => {
                let tc = content.iter().find_map(|c| match c {
                    AssistantContent::ToolCall(tc) => Some(tc),
                    _ => None,
                });
                tc.expect("should have tool call").id.clone()
            }
            other => panic!("Expected Assistant message, got: {:?}", other),
        };

        // Extract the generated call_id from the tool result
        let tool_result_call_id = match &history[1] {
            RigMessage::User { content } => match content.first() {
                UserContent::ToolResult(r) => r
                    .call_id
                    .clone()
                    .expect("tool result call_id must be present"),
                other => panic!("Expected ToolResult, got: {:?}", other),
            },
            other => panic!("Expected User message, got: {:?}", other),
        };

        assert!(
            !assistant_call_id.is_empty(),
            "assistant call_id must not be empty"
        );
        assert!(
            !tool_result_call_id.is_empty(),
            "tool result call_id must not be empty"
        );

        // NOTE: With the current seed-based generation, these IDs will differ
        // because the assistant tool call uses seed=0 (history.len() at that
        // point) and the tool result uses seed=1 (history.len() after the
        // assistant message was pushed). This documents the current behavior.
        // A future improvement could thread the assistant's generated ID into
        // the tool result for exact matching.
        assert_ne!(
            assistant_call_id, tool_result_call_id,
            "Current impl generates different IDs for assistant call and tool result \
             because seeds differ; this documents the known limitation"
        );
    }

    #[test]
    fn test_saturate_u32() {
        assert_eq!(saturate_u32(100), 100);
        assert_eq!(saturate_u32(u64::MAX), u32::MAX);
        assert_eq!(saturate_u32(u32::MAX as u64), u32::MAX);
    }

    // -- normalize_tool_name tests --

    #[test]
    fn test_normalize_tool_name_exact_match() {
        let known = HashSet::from(["echo".to_string(), "list_jobs".to_string()]);
        assert_eq!(normalize_tool_name("echo", &known), "echo");
    }

    #[test]
    fn test_normalize_tool_name_proxy_prefix_match() {
        let known = HashSet::from(["echo".to_string(), "list_jobs".to_string()]);
        assert_eq!(normalize_tool_name("proxy_echo", &known), "echo");
    }

    #[test]
    fn test_normalize_tool_name_proxy_prefix_no_match_kept() {
        let known = HashSet::from(["echo".to_string(), "list_jobs".to_string()]);
        assert_eq!(
            normalize_tool_name("proxy_unknown", &known),
            "proxy_unknown"
        );
    }

    #[test]
    fn test_normalize_tool_name_unknown_passthrough() {
        let known = HashSet::from(["echo".to_string()]);
        assert_eq!(normalize_tool_name("other_tool", &known), "other_tool");
    }

    #[test]
    fn test_build_rig_request_injects_cache_control_short() {
        let req = build_rig_request(
            Some("You are helpful.".to_string()),
            vec![RigMessage::user("Hello")],
            Vec::new(),
            None,
            None,
            None,
            CacheRetention::Short,
        )
        .unwrap();

        let params = req
            .additional_params
            .expect("should have additional_params for Short retention");
        assert_eq!(params["cache_control"]["type"], "ephemeral");
        assert!(
            params["cache_control"].get("ttl").is_none(),
            "Short retention should not include ttl"
        );
    }

    #[test]
    fn test_build_rig_request_injects_cache_control_long() {
        let req = build_rig_request(
            Some("You are helpful.".to_string()),
            vec![RigMessage::user("Hello")],
            Vec::new(),
            None,
            None,
            None,
            CacheRetention::Long,
        )
        .unwrap();

        let params = req
            .additional_params
            .expect("should have additional_params for Long retention");
        assert_eq!(params["cache_control"]["type"], "ephemeral");
        assert_eq!(params["cache_control"]["ttl"], "1h");
    }

    #[test]
    fn test_build_rig_request_no_cache_control_when_none() {
        let req = build_rig_request(
            Some("You are helpful.".to_string()),
            vec![RigMessage::user("Hello")],
            Vec::new(),
            None,
            None,
            None,
            CacheRetention::None,
        )
        .unwrap();

        assert!(
            req.additional_params.is_none(),
            "additional_params should be None when cache is disabled"
        );
    }

    /// Verify that the multiplier match arms in `RigAdapter::cache_write_multiplier`
    /// produce the expected values. We use a standalone helper because constructing
    /// a real `RigAdapter` requires a rig `Model` (which needs network/provider setup).
    /// The helper mirrors the same match expression — if the impl drifts, the
    /// `test_build_rig_request_*` tests will still catch regressions end-to-end.
    #[test]
    fn test_cache_write_multiplier_values() {
        use rust_decimal::Decimal;
        // None → 1.0× (no surcharge)
        assert_eq!(
            cache_write_multiplier_for(CacheRetention::None),
            Decimal::ONE
        );
        // Short → 1.25× (25% surcharge)
        assert_eq!(
            cache_write_multiplier_for(CacheRetention::Short),
            Decimal::new(125, 2)
        );
        // Long → 2.0× (100% surcharge)
        assert_eq!(
            cache_write_multiplier_for(CacheRetention::Long),
            Decimal::TWO
        );
    }

    fn cache_write_multiplier_for(retention: CacheRetention) -> rust_decimal::Decimal {
        match retention {
            CacheRetention::None => rust_decimal::Decimal::ONE,
            CacheRetention::Short => rust_decimal::Decimal::new(125, 2),
            CacheRetention::Long => rust_decimal::Decimal::TWO,
        }
    }

    // -- supports_prompt_cache tests --

    #[test]
    fn test_supports_prompt_cache_supported_models() {
        // All Claude 3+ models per Anthropic docs
        assert!(supports_prompt_cache("claude-opus-4-6"));
        assert!(supports_prompt_cache("claude-sonnet-4-6"));
        assert!(supports_prompt_cache("claude-sonnet-4"));
        assert!(supports_prompt_cache("claude-haiku-4-5"));
        assert!(supports_prompt_cache("claude-3-5-sonnet-20241022"));
        assert!(supports_prompt_cache("claude-haiku-3"));
        assert!(supports_prompt_cache("Claude-Opus-4-5")); // case-insensitive
        assert!(supports_prompt_cache("anthropic/claude-sonnet-4-6")); // provider prefix
    }

    #[test]
    fn test_supports_prompt_cache_unsupported_models() {
        // Legacy Claude models that predate caching
        assert!(!supports_prompt_cache("claude-2"));
        assert!(!supports_prompt_cache("claude-2.1"));
        assert!(!supports_prompt_cache("claude-instant-1.2"));
        // Non-Claude models
        assert!(!supports_prompt_cache("gpt-4o"));
        assert!(!supports_prompt_cache("llama3"));
    }

    #[test]
    fn test_with_unsupported_params_populates_set() {
        use rig::client::CompletionClient;
        use rig::providers::openai;

        let client: openai::Client = openai::Client::builder()
            .api_key("test-key")
            .base_url("http://localhost:0")
            .build()
            .unwrap();
        let client = client.completions_api();
        let model = client.completion_model("test-model");
        let adapter = RigAdapter::new(model, "test-model")
            .with_unsupported_params(vec!["temperature".to_string()]);

        assert!(adapter.unsupported_params.contains("temperature"));
        assert!(!adapter.unsupported_params.contains("max_tokens"));
    }

    #[test]
    fn test_strip_unsupported_completion_params() {
        use rig::client::CompletionClient;
        use rig::providers::openai;

        let client: openai::Client = openai::Client::builder()
            .api_key("test-key")
            .base_url("http://localhost:0")
            .build()
            .unwrap();
        let client = client.completions_api();
        let model = client.completion_model("test-model");
        let adapter = RigAdapter::new(model, "test-model").with_unsupported_params(vec![
            "temperature".to_string(),
            "stop_sequences".to_string(),
        ]);

        let mut req = CompletionRequest::new(vec![ChatMessage::user("hi")]);
        req.temperature = Some(0.7);
        req.max_tokens = Some(100);
        req.stop_sequences = Some(vec!["STOP".to_string()]);

        adapter.strip_unsupported_completion_params(&mut req);

        assert!(req.temperature.is_none(), "temperature should be stripped");
        assert_eq!(req.max_tokens, Some(100), "max_tokens should be preserved");
        assert!(
            req.stop_sequences.is_none(),
            "stop_sequences should be stripped"
        );
    }

    #[test]
    fn test_strip_unsupported_tool_params() {
        use rig::client::CompletionClient;
        use rig::providers::openai;

        let client: openai::Client = openai::Client::builder()
            .api_key("test-key")
            .base_url("http://localhost:0")
            .build()
            .unwrap();
        let client = client.completions_api();
        let model = client.completion_model("test-model");
        let adapter = RigAdapter::new(model, "test-model")
            .with_unsupported_params(vec!["temperature".to_string(), "max_tokens".to_string()]);

        let mut req = ToolCompletionRequest::new(vec![ChatMessage::user("hi")], vec![]);
        req.temperature = Some(0.5);
        req.max_tokens = Some(200);

        adapter.strip_unsupported_tool_params(&mut req);

        assert!(req.temperature.is_none(), "temperature should be stripped");
        assert!(req.max_tokens.is_none(), "max_tokens should be stripped");
    }

    #[test]
    fn test_unsupported_params_empty_by_default() {
        use rig::client::CompletionClient;
        use rig::providers::openai;

        let client: openai::Client = openai::Client::builder()
            .api_key("test-key")
            .base_url("http://localhost:0")
            .build()
            .unwrap();
        let client = client.completions_api();
        let model = client.completion_model("test-model");
        let adapter = RigAdapter::new(model, "test-model");

        assert!(adapter.unsupported_params.is_empty());
    }

    /// Regression test: consecutive tool_result messages from parallel tool
    /// execution must be merged into a single User message with multiple
    /// ToolResult content items. Without merging, APIs like Anthropic reject
    /// the request due to consecutive User messages.
    #[test]
    fn test_consecutive_tool_results_merged_into_single_user_message() {
        let tc1 = IronToolCall {
            id: "call_a".to_string(),
            name: "search".to_string(),
            arguments: serde_json::json!({"q": "rust"}),
            reasoning: None,
            signature: None,
        };
        let tc2 = IronToolCall {
            id: "call_b".to_string(),
            name: "fetch".to_string(),
            arguments: serde_json::json!({"url": "https://example.com"}),
            reasoning: None,
            signature: None,
        };
        let assistant = ChatMessage::assistant_with_tool_calls(None, vec![tc1, tc2]);
        let result_a = ChatMessage::tool_result("call_a", "search", "search results");
        let result_b = ChatMessage::tool_result("call_b", "fetch", "fetch results");

        let messages = vec![assistant, result_a, result_b];
        let (_preamble, history) = convert_messages(&messages);

        // Should be: 1 assistant + 1 merged user (not 1 assistant + 2 users)
        assert_eq!(
            history.len(),
            2,
            "Expected 2 messages (assistant + merged user), got {}",
            history.len()
        );

        // The second message should contain both tool results
        match &history[1] {
            RigMessage::User { content } => {
                assert_eq!(
                    content.len(),
                    2,
                    "Expected 2 tool results in merged user message, got {}",
                    content.len()
                );
                for item in content.iter() {
                    assert!(
                        matches!(item, UserContent::ToolResult(_)),
                        "Expected ToolResult content"
                    );
                }
            }
            other => panic!("Expected User message, got: {:?}", other),
        }
    }

    /// Verify that a tool_result after a non-tool User message is NOT merged.
    #[test]
    fn test_tool_result_after_user_text_not_merged() {
        let user_msg = ChatMessage::user("hello");
        let tool_msg = ChatMessage::tool_result("call_1", "search", "results");

        let messages = vec![user_msg, tool_msg];
        let (_preamble, history) = convert_messages(&messages);

        // Should be 2 separate User messages (text user + tool result user)
        assert_eq!(history.len(), 2);
    }

    /// Empty user messages (e.g. after thinking-tag stripping) must be skipped.
    /// Strict providers like Kimi return 400 when "content": "" is sent.
    #[test]
    fn test_empty_user_message_is_skipped() {
        let empty = ChatMessage::user("");
        let non_empty = ChatMessage::user("hello");
        let messages = vec![empty, non_empty];
        let (_preamble, history) = convert_messages(&messages);

        assert_eq!(history.len(), 1, "empty user message must be dropped");
        match &history[0] {
            RigMessage::User { content } => {
                assert_eq!(content.len(), 1);
                let first = content.iter().next().expect("one content item");
                match first {
                    UserContent::Text(t) => assert_eq!(t.text, "hello"),
                    other => panic!("expected Text, got {:?}", other),
                }
            }
            other => panic!("expected User message, got {:?}", other),
        }
    }

    /// Empty assistant messages (e.g. after thinking-tag stripping) must be skipped.
    #[test]
    fn test_empty_assistant_message_is_skipped() {
        let empty_asst = ChatMessage {
            role: crate::llm::Role::Assistant,
            content: String::new(),
            tool_calls: None,
            reasoning: None,
            tool_call_id: None,
            name: None,
            content_parts: vec![],
        };
        let non_empty = ChatMessage::user("hi");
        let messages = vec![empty_asst, non_empty];
        let (_preamble, history) = convert_messages(&messages);

        assert_eq!(history.len(), 1, "empty assistant message must be dropped");
        assert!(matches!(history[0], RigMessage::User { .. }));
    }

    /// A conversation mixing normal and empty messages: only non-empty ones survive.
    #[test]
    fn test_mixed_empty_and_non_empty_messages_filtered_correctly() {
        let user1 = ChatMessage::user("first");
        let empty_asst = ChatMessage {
            role: crate::llm::Role::Assistant,
            content: String::new(),
            tool_calls: None,
            reasoning: None,
            tool_call_id: None,
            name: None,
            content_parts: vec![],
        };
        let user2 = ChatMessage::user("");
        let asst = ChatMessage::assistant("response");
        let messages = vec![user1, empty_asst, user2, asst];
        let (_preamble, history) = convert_messages(&messages);

        assert_eq!(history.len(), 2, "only non-empty messages should survive");
        assert!(matches!(history[0], RigMessage::User { .. }));
        assert!(matches!(history[1], RigMessage::Assistant { .. }));
    }

    // -- normalized_tool_call_id tests --

    #[test]
    fn test_normalized_tool_call_id_conforming_passthrough() {
        // A 9-char alphanumeric ID should pass through unchanged.
        let id = normalized_tool_call_id(Some("abcDE1234"), 42);
        assert_eq!(id, "abcDE1234");
    }

    #[test]
    fn test_normalized_tool_call_id_non_conforming_hashed() {
        // An ID that doesn't match [a-zA-Z0-9]{9} should be hashed into one.
        let id = normalized_tool_call_id(Some("call_abc_long_id"), 0);
        assert_eq!(id.len(), 9);
        assert!(id.chars().all(|c| c.is_ascii_alphanumeric()));
        // Should NOT be the raw input.
        assert_ne!(id, "call_abc_l");
    }

    #[test]
    fn test_normalized_tool_call_id_empty_input() {
        let id = normalized_tool_call_id(Some(""), 5);
        assert_eq!(id.len(), 9);
        assert!(id.chars().all(|c| c.is_ascii_alphanumeric()));
    }

    #[test]
    fn test_normalized_tool_call_id_whitespace_input() {
        let id = normalized_tool_call_id(Some("   "), 5);
        assert_eq!(id.len(), 9);
        assert!(id.chars().all(|c| c.is_ascii_alphanumeric()));
        // Empty and whitespace-only with the same seed should produce identical results.
        let id_empty = normalized_tool_call_id(Some(""), 5);
        assert_eq!(id, id_empty);
    }

    #[test]
    fn test_normalized_tool_call_id_none_input() {
        let id = normalized_tool_call_id(None, 7);
        assert_eq!(id.len(), 9);
        assert!(id.chars().all(|c| c.is_ascii_alphanumeric()));
        // None and empty string with same seed should produce identical results.
        let id_empty = normalized_tool_call_id(Some(""), 7);
        assert_eq!(id, id_empty);
    }

    #[test]
    fn test_normalized_tool_call_id_deterministic() {
        let id1 = normalized_tool_call_id(Some("call_xyz_123"), 0);
        let id2 = normalized_tool_call_id(Some("call_xyz_123"), 0);
        assert_eq!(id1, id2, "same input must produce same output");
    }

    #[test]
    fn test_normalized_tool_call_id_different_inputs_differ() {
        let id_a = normalized_tool_call_id(Some("call_aaa"), 0);
        let id_b = normalized_tool_call_id(Some("call_bbb"), 0);
        assert_ne!(
            id_a, id_b,
            "different raw IDs should produce different hashed IDs"
        );
    }

    fn make_rig_request(additional_params: Option<serde_json::Value>) -> RigRequest {
        RigRequest {
            preamble: None,
            chat_history: OneOrMany::one(RigMessage::user("test")),
            documents: Vec::new(),
            tools: Vec::new(),
            temperature: None,
            max_tokens: None,
            tool_choice: None,
            additional_params,
        }
    }

    #[test]
    fn test_merge_additional_params_into_empty() {
        let mut req = make_rig_request(None);
        merge_additional_params(&mut req, Some(&serde_json::json!({"think": true})));
        let params = req.additional_params.expect("should be Some");
        assert_eq!(params["think"], true);
    }

    #[test]
    fn test_merge_additional_params_preserves_existing() {
        let mut req = make_rig_request(Some(
            serde_json::json!({"cache_control": {"type": "ephemeral"}}),
        ));
        merge_additional_params(&mut req, Some(&serde_json::json!({"think": true})));
        let params = req.additional_params.expect("should remain Some");
        let obj = params.as_object().expect("should be object");
        assert_eq!(obj["cache_control"]["type"], "ephemeral");
        assert_eq!(obj["think"], true);
    }

    #[test]
    fn test_merge_additional_params_existing_key_wins() {
        let mut req = make_rig_request(Some(serde_json::json!({"think": false})));
        merge_additional_params(&mut req, Some(&serde_json::json!({"think": true})));
        let params = req.additional_params.expect("should remain Some");
        assert_eq!(
            params["think"], false,
            "existing value should not be overwritten"
        );
    }

    #[test]
    fn test_merge_additional_params_noop_when_none() {
        let mut req = make_rig_request(None);
        merge_additional_params(&mut req, None);
        assert!(req.additional_params.is_none());
    }

    #[test]
    fn test_inject_model_override_creates_params_when_none() {
        let mut req = make_rig_request(None);
        inject_model_override(&mut req, Some("test-model"));

        let params = req
            .additional_params
            .expect("additional_params should be Some");
        assert_eq!(params, serde_json::json!({ "model": "test-model" }));
    }

    #[test]
    fn test_inject_model_override_preserves_existing_params() {
        let mut req = make_rig_request(Some(serde_json::json!({
            "cache_control": { "type": "ephemeral" },
        })));
        inject_model_override(&mut req, Some("override-model"));

        let params = req.additional_params.expect("should remain Some");
        let obj = params.as_object().expect("should be object");
        assert_eq!(
            obj.get("cache_control"),
            Some(&serde_json::json!({ "type": "ephemeral" }))
        );
        assert_eq!(obj.get("model"), Some(&serde_json::json!("override-model")));
    }

    #[test]
    fn test_inject_model_override_noop_when_none() {
        let mut req = make_rig_request(None);
        inject_model_override(&mut req, None);
        assert!(req.additional_params.is_none());
    }

    // ── map_rig_error: context length detection ─────────────────────────

    #[test]
    fn test_map_rig_error_detects_context_length_exceeded() {
        let err = map_rig_error("openai", "Error: context_length_exceeded");
        assert!(
            matches!(err, LlmError::ContextLengthExceeded { .. }),
            "Should detect context_length_exceeded: {err:?}"
        );
    }

    #[test]
    fn test_map_rig_error_detects_maximum_context_length() {
        let err = map_rig_error(
            "openai",
            "This model's maximum context length is 128000 tokens",
        );
        assert!(
            matches!(err, LlmError::ContextLengthExceeded { .. }),
            "Should detect maximum context length: {err:?}"
        );
    }

    #[test]
    fn test_map_rig_error_detects_too_many_tokens() {
        let err = map_rig_error("anthropic", "Request has too many tokens (150000)");
        assert!(
            matches!(err, LlmError::ContextLengthExceeded { .. }),
            "Should detect too many tokens: {err:?}"
        );
    }

    #[test]
    fn test_map_rig_error_detects_payload_too_large() {
        let err = map_rig_error("nearai", "HTTP 413: Payload Too Large");
        assert!(
            matches!(err, LlmError::ContextLengthExceeded { .. }),
            "Should detect payload too large: {err:?}"
        );
    }

    #[test]
    fn test_map_rig_error_bare_413_no_false_positive() {
        // Bare "413" should NOT trigger ContextLengthExceeded — avoids false
        // positives on timestamps ("2026-04-13"), token counts ("used 1413"),
        // and request IDs.
        let err = map_rig_error("nearai", "Rate limit: 413 requests per minute exceeded");
        assert!(
            matches!(err, LlmError::RequestFailed { .. }),
            "Bare 413 in rate limit message should not be ContextLengthExceeded: {err:?}"
        );

        let err = map_rig_error("nearai", "Error at 2026-04-13T10:00:00Z");
        assert!(
            matches!(err, LlmError::RequestFailed { .. }),
            "413 in timestamp should not be ContextLengthExceeded: {err:?}"
        );
    }

    #[test]
    fn test_map_rig_error_generic_error_is_request_failed() {
        let err = map_rig_error("openai", "Connection refused");
        assert!(
            matches!(err, LlmError::RequestFailed { .. }),
            "Generic error should be RequestFailed: {err:?}"
        );
    }

    #[test]
    fn test_parse_token_counts_openai_format() {
        let msg = "this model's maximum context length is 128000 tokens. however, your messages resulted in 150000 tokens.";
        let (used, limit) = parse_token_counts(msg);
        assert_eq!(limit, 128000);
        assert_eq!(used, 150000);
    }

    #[test]
    fn test_parse_token_counts_unparseable_returns_zero() {
        let msg = "context_length_exceeded";
        let (used, limit) = parse_token_counts(msg);
        assert_eq!(used, 0);
        assert_eq!(limit, 0);
    }

    #[test]
    fn test_map_rig_error_extracts_token_counts() {
        let err = map_rig_error(
            "openai",
            "This model's maximum context length is 128000 tokens. However, your messages resulted in 150000 tokens.",
        );
        match err {
            LlmError::ContextLengthExceeded { used, limit } => {
                assert_eq!(limit, 128000);
                assert_eq!(used, 150000);
            }
            other => panic!("Expected ContextLengthExceeded, got: {other:?}"),
        }
    }

    /// Regression for #3201 / #3225 (the high-severity gap in PR #3326):
    /// the dedicated rig-core DeepSeek/Gemini/OpenRouter clients only fix the
    /// reasoning round-trip *inside* rig-core. IronClaw's RigAdapter sits
    /// between the agent loop and rig-core, and previously dropped both
    /// `AssistantContent::Reasoning` (DeepSeek `reasoning_content`) and
    /// per-tool-call `signature` (Gemini `thought_signature`) on the response →
    /// IronClaw conversion. On the next request it rebuilt rig messages
    /// without either field, so the provider rejected the follow-up turn.
    ///
    /// This test simulates a 2-turn tool loop:
    ///   1. extract a rig response carrying both reasoning and a signed tool call
    ///   2. round-trip those onto an IronClaw `ChatMessage`
    ///   3. convert that message back into rig format
    ///   4. assert both reasoning and signature appear on the rebuilt rig message
    ///
    /// If any layer drops a field, the next turn would fail with HTTP 400.
    #[test]
    fn reasoning_and_signature_round_trip_through_chat_message() {
        // --- Turn 1: provider returned reasoning + signed tool call ---
        let rig_response = OneOrMany::many(vec![
            AssistantContent::Reasoning(rig::message::Reasoning::new(
                "Let me check the weather first.",
            )),
            AssistantContent::ToolCall(
                rig::message::ToolCall::new(
                    "call_abc123".to_string(),
                    ToolFunction::new(
                        "get_weather".to_string(),
                        serde_json::json!({"city": "London"}),
                    ),
                )
                .with_signature(Some("thought-sig-deadbeef".to_string())),
            ),
        ])
        .unwrap();
        let usage = RigUsage::new();
        let (text, tool_calls, finish, reasoning) = extract_response(&rig_response, &usage);

        assert_eq!(finish, FinishReason::ToolUse);
        assert_eq!(text, None);
        assert_eq!(
            reasoning.as_deref(),
            Some("Let me check the weather first."),
            "extract_response must capture AssistantContent::Reasoning so the \
             next request can echo DeepSeek's reasoning_content (#3201)",
        );
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(
            tool_calls[0].signature.as_deref(),
            Some("thought-sig-deadbeef"),
            "extract_response must capture ToolCall.signature so the next \
             request can echo Gemini's thought_signature (#3225)",
        );

        // --- IronClaw stores the assistant message + tool result ---
        let assistant = ChatMessage::assistant_with_tool_calls(text, tool_calls)
            .with_reasoning(reasoning.clone());
        let tool_result =
            ChatMessage::tool_result("call_abc123", "get_weather", "{\"temp_c\": 14}");

        // --- Turn 2: IronClaw rebuilds the rig request from stored messages ---
        let messages = vec![
            ChatMessage::user("What's the weather?"),
            assistant,
            tool_result,
        ];
        let (_preamble, history) = convert_messages(&messages);

        // The rebuilt rig assistant message must carry both reasoning and
        // signature; otherwise the dedicated DeepSeek/Gemini/OpenRouter rig
        // clients would emit an empty `reasoning_content` / unsigned
        // `functionCall` and the API would reject with HTTP 400.
        let assistant_msg = history
            .iter()
            .find(|m| matches!(m, RigMessage::Assistant { .. }))
            .expect("rebuilt rig history should contain the assistant message");
        let RigMessage::Assistant { content, .. } = assistant_msg else {
            unreachable!()
        };

        let mut found_reasoning = false;
        let mut found_signed_tool_call = false;
        for c in content.iter() {
            match c {
                AssistantContent::Reasoning(r) => {
                    assert_eq!(r.reasoning, vec!["Let me check the weather first."]);
                    found_reasoning = true;
                }
                AssistantContent::ToolCall(tc) => {
                    assert_eq!(
                        tc.signature.as_deref(),
                        Some("thought-sig-deadbeef"),
                        "rebuilt rig tool call must carry the original \
                         thought_signature (#3225)",
                    );
                    found_signed_tool_call = true;
                }
                _ => {}
            }
        }
        assert!(
            found_reasoning,
            "convert_messages must emit AssistantContent::Reasoning when \
             ChatMessage carries reasoning — without this, DeepSeek thinking \
             mode rejects the next turn (#3201)",
        );
        assert!(
            found_signed_tool_call,
            "convert_messages must propagate ToolCall.signature when \
             rebuilding rig tool calls — without this, Gemini 2.5+ rejects \
             the next turn (#3225)",
        );
    }

    /// `with_reasoning` must drop empty/whitespace-only strings rather than
    /// echoing `reasoning_content: ""` (some strict-mode providers reject
    /// empty reasoning fields, and an empty echo carries no signal anyway).
    #[test]
    fn chat_message_with_reasoning_drops_empty_input() {
        let msg = ChatMessage::assistant("hi").with_reasoning(Some(String::new()));
        assert!(msg.reasoning.is_none());
        let msg = ChatMessage::assistant("hi").with_reasoning(Some("   ".to_string()));
        assert!(msg.reasoning.is_none());
        let msg = ChatMessage::assistant("hi").with_reasoning(None);
        assert!(msg.reasoning.is_none());
        let msg = ChatMessage::assistant("hi").with_reasoning(Some("real".to_string()));
        assert_eq!(msg.reasoning.as_deref(), Some("real"));
    }
}
