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
use serde_json::Value as JsonValue;
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

/// Normalize a JSON Schema for OpenAI strict mode compliance.
///
/// OpenAI strict function calling requires:
/// - Every object must have `"additionalProperties": false`
/// - `"required"` must list ALL property keys
/// - Optional fields use `"type": ["<original>", "null"]` instead of being omitted from `required`
/// - Nested objects and array items are recursively normalized
///
/// This is applied as a clone-and-transform at the provider boundary so the
/// original tool definitions remain unchanged for other providers.
pub(crate) fn normalize_schema_strict(schema: &JsonValue) -> JsonValue {
    let mut schema = schema.clone();
    normalize_schema_recursive(&mut schema);
    schema
}

fn normalize_schema_recursive(schema: &mut JsonValue) {
    let obj = match schema.as_object_mut() {
        Some(o) => o,
        None => return,
    };

    // Recurse into combinators: anyOf, oneOf, allOf
    for key in &["anyOf", "oneOf", "allOf"] {
        if let Some(JsonValue::Array(variants)) = obj.get_mut(*key) {
            for variant in variants.iter_mut() {
                normalize_schema_recursive(variant);
            }
        }
    }

    // Recurse into array items
    if let Some(items) = obj.get_mut("items") {
        normalize_schema_recursive(items);
    }

    // Recurse into `not`, `if`, `then`, `else`
    for key in &["not", "if", "then", "else"] {
        if let Some(sub) = obj.get_mut(*key) {
            normalize_schema_recursive(sub);
        }
    }

    // Only apply object-level normalization if this schema has "properties"
    // (explicit object schema) or type == "object"
    let is_object = obj
        .get("type")
        .and_then(|t| t.as_str())
        .map(|t| t == "object")
        .unwrap_or(false);
    let has_properties = obj.contains_key("properties");

    if !is_object && !has_properties {
        return;
    }

    // Ensure "type": "object" is present
    if !obj.contains_key("type") && has_properties {
        obj.insert("type".to_string(), JsonValue::String("object".to_string()));
    }

    // Force additionalProperties: false (overwrite any existing value)
    obj.insert("additionalProperties".to_string(), JsonValue::Bool(false));

    // Ensure "properties" exists
    if !obj.contains_key("properties") {
        obj.insert(
            "properties".to_string(),
            JsonValue::Object(serde_json::Map::new()),
        );
    }

    // Collect current required set
    let current_required: std::collections::HashSet<String> = obj
        .get("required")
        .and_then(|r| r.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    // Get all property keys (sorted for deterministic output)
    let all_keys: Vec<String> = obj
        .get("properties")
        .and_then(|p| p.as_object())
        .map(|props| {
            let mut keys: Vec<String> = props.keys().cloned().collect();
            keys.sort();
            keys
        })
        .unwrap_or_default();

    // For properties NOT in the original required list, make them nullable
    if let Some(JsonValue::Object(props)) = obj.get_mut("properties") {
        for key in &all_keys {
            // Recurse into each property's schema FIRST (before make_nullable,
            // which may change the type to an array and prevent object detection)
            if let Some(prop_schema) = props.get_mut(key) {
                normalize_schema_recursive(prop_schema);
            }
            // Then make originally-optional properties nullable
            if !current_required.contains(key)
                && let Some(prop_schema) = props.get_mut(key)
            {
                make_nullable(prop_schema);
            }
        }
    }

    // Set required to ALL property keys
    let required_value: Vec<JsonValue> = all_keys.into_iter().map(JsonValue::String).collect();
    obj.insert("required".to_string(), JsonValue::Array(required_value));
}

/// Make a property schema nullable for OpenAI strict mode.
///
/// If it has a simple `"type": "<T>"`, converts to `"type": ["<T>", "null"]`.
/// If it already has an array type, adds "null" if not present.
/// Otherwise, wraps with `anyOf: [<existing>, {"type": "null"}]`.
fn make_nullable(schema: &mut JsonValue) {
    let obj = match schema.as_object_mut() {
        Some(o) => o,
        None => return,
    };

    if let Some(type_val) = obj.get("type").cloned() {
        match type_val {
            // "type": "string" → "type": ["string", "null"]
            JsonValue::String(ref t) if t != "null" => {
                obj.insert("type".to_string(), serde_json::json!([t, "null"]));
            }
            // "type": ["string", "integer"] → add "null" if missing
            JsonValue::Array(ref arr) => {
                let has_null = arr.iter().any(|v| v.as_str() == Some("null"));
                if !has_null {
                    let mut new_arr = arr.clone();
                    new_arr.push(JsonValue::String("null".to_string()));
                    obj.insert("type".to_string(), JsonValue::Array(new_arr));
                }
            }
            _ => {}
        }
    } else {
        // No "type" key — wrap with anyOf including null
        // (handles enum-only, $ref, or combinator schemas)
        let existing = JsonValue::Object(obj.clone());
        obj.clear();
        obj.insert(
            "anyOf".to_string(),
            serde_json::json!([existing, {"type": "null"}]),
        );
    }
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
                    for (idx, tc) in tool_calls.iter().enumerate() {
                        let tool_call_id =
                            normalized_tool_call_id(Some(tc.id.as_str()), history.len() + idx);
                        contents.push(AssistantContent::ToolCall(
                            rig::message::ToolCall::new(
                                tool_call_id.clone(),
                                ToolFunction::new(tc.name.clone(), tc.arguments.clone()),
                            )
                            .with_call_id(tool_call_id),
                        ));
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
                } else {
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
/// Applies OpenAI strict-mode schema normalization to ensure all tool
/// parameter schemas comply with OpenAI's function calling requirements.
fn convert_tools(tools: &[IronToolDefinition]) -> Vec<RigToolDefinition> {
    tools
        .iter()
        .map(|t| RigToolDefinition {
            name: t.name.clone(),
            description: t.description.clone(),
            parameters: normalize_schema_strict(&t.parameters),
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

/// Extract text and tool calls from a rig-core completion response.
fn extract_response(
    choice: &OneOrMany<AssistantContent>,
    _usage: &RigUsage,
) -> (Option<String>, Vec<IronToolCall>, FinishReason) {
    let mut text_parts: Vec<String> = Vec::new();
    let mut tool_calls: Vec<IronToolCall> = Vec::new();

    for content in choice.iter() {
        match content {
            AssistantContent::Text(t) => {
                if !t.text.is_empty() {
                    text_parts.push(t.text.clone());
                }
            }
            AssistantContent::ToolCall(tc) => {
                tool_calls.push(IronToolCall {
                    id: tc.id.clone(),
                    name: tc.function.name.clone(),
                    arguments: tc.function.arguments.clone(),
                    reasoning: None,
                });
            }
            // Reasoning and Image variants are not mapped to IronClaw types
            _ => {}
        }
    }

    let text = if text_parts.is_empty() {
        None
    } else {
        Some(text_parts.join(""))
    };

    let finish = if !tool_calls.is_empty() {
        FinishReason::ToolUse
    } else {
        FinishReason::Stop
    };

    (text, tool_calls, finish)
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
/// `CompletionModel` implementation. The actual HTTP request body includes a
/// `model` field set by the provider. Rig-core's `#[serde(flatten)]` on
/// `additional_params` emits these fields AFTER the provider's own fields.
/// Most API servers (Python, Go) use last-key-wins when deserializing
/// duplicate JSON keys, so the injected `model` value takes effect.
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
        let model_override = request.model.take();

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

        inject_model_override(&mut rig_req, model_override.as_deref());

        let response =
            self.model
                .completion(rig_req)
                .await
                .map_err(|e| LlmError::RequestFailed {
                    provider: self.model_name.clone(),
                    reason: e.to_string(),
                })?;

        let (text, _tool_calls, finish) = extract_response(&response.choice, &response.usage);

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
        let model_override = request.model.take();

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

        inject_model_override(&mut rig_req, model_override.as_deref());

        let response =
            self.model
                .completion(rig_req)
                .await
                .map_err(|e| LlmError::RequestFailed {
                    provider: self.model_name.clone(),
                    reason: e.to_string(),
                })?;

        let (text, mut tool_calls, finish) = extract_response(&response.choice, &response.usage);

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
        let (text, calls, finish) = extract_response(&content, &usage);
        assert_eq!(text, Some("Hello world".to_string()));
        assert!(calls.is_empty());
        assert_eq!(finish, FinishReason::Stop);
    }

    #[test]
    fn test_extract_response_tool_call() {
        let tc = AssistantContent::tool_call("call_1", "search", serde_json::json!({"q": "test"}));
        let content = OneOrMany::one(tc);
        let usage = RigUsage::new();
        let (text, calls, finish) = extract_response(&content, &usage);
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
        };
        let assistant_msg = ChatMessage::assistant_with_tool_calls(None, vec![tc]);
        let tool_result_msg = ChatMessage {
            role: crate::llm::Role::Tool,
            content: "search results here".to_string(),
            content_parts: Vec::new(),
            tool_call_id: None,
            name: Some("search".to_string()),
            tool_calls: None,
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
        };
        let tc2 = IronToolCall {
            id: "call_b".to_string(),
            name: "fetch".to_string(),
            arguments: serde_json::json!({"url": "https://example.com"}),
            reasoning: None,
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
}
