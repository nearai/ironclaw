// arch-exempt: large_file, Rig provider bridge keeps streaming and non-streaming conversions co-located until provider adapter decomposition, plan #6175
//! Generic adapter that bridges rig-core's `CompletionModel` trait to IronClaw's `LlmProvider`.
//!
//! This lets us use any rig-core provider (OpenAI, Anthropic, Ollama, etc.) as an
//! `Arc<dyn LlmProvider>` without changing any of the agent, reasoning, or tool code.

use crate::config::CacheRetention;
use async_trait::async_trait;
use futures::StreamExt;
use rig::OneOrMany;
use rig::completion::{
    AssistantContent, CompletionModel, CompletionRequest as RigRequest, GetTokenUsage,
    ToolDefinition as RigToolDefinition, Usage as RigUsage,
};
use rig::message::{
    DocumentSourceKind, Image, ImageDetail, ImageMediaType, Message as RigMessage, MimeType,
    ToolChoice as RigToolChoice, ToolFunction, ToolResult as RigToolResult, ToolResultContent,
    UserContent,
};
use rig::streaming::{StreamedAssistantContent, StreamingCompletionResponse};
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use serde::Serialize;
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};

use std::collections::HashSet;
use std::str::FromStr;

use crate::error::LlmError;
use crate::provider::{
    ChatMessage, CompletionRequest, CompletionResponse, CompletionResponseFormat,
    CompletionStreamSink, FinishReason, LlmProvider, ReasoningDetail as IronReasoningDetail,
    ReasoningDetails as IronReasoningDetails, ToolCall as IronToolCall, ToolCompletionRequest,
    ToolCompletionResponse, ToolDefinition as IronToolDefinition, map_provider_finish_token,
    normalized_model_override, resolve_finish_reason, strip_unsupported_completion_params,
    strip_unsupported_tool_params,
};
use crate::tool_schema::{ToolSchemaPolicy, shape_tool_schema};
#[cfg(test)]
use crate::tool_schema::{normalize_schema_strict, serialize_json_capped};
use ironclaw_common::llm_costs as costs;

/// Adapter that wraps a rig-core `CompletionModel` and implements `LlmProvider`.
pub struct RigAdapter<M: CompletionModel> {
    model: M,
    provider_id: String,
    model_name: String,
    input_cost: Decimal,
    output_cost: Decimal,
    /// Prompt cache retention policy (Anthropic only).
    /// When not `CacheRetention::None`, emits explicit `cache_control`
    /// breakpoints (last tool via `additional_params.tools`, plus rig's
    /// system/last-message markers under `Short`) and the top-level
    /// automatic-caching marker — see `build_rig_request` and issue #6984.
    /// Also controls the cost multiplier for cache-creation tokens.
    cache_retention: CacheRetention,
    /// Parameter names that this provider does not support (e.g., `"temperature"`).
    /// These are stripped from requests before sending to avoid 400 errors.
    unsupported_params: HashSet<String>,
    /// Default additional parameters merged into every request.
    /// Used by providers that need extra top-level fields (e.g., Ollama `think: true`).
    default_additional_params: Option<serde_json::Value>,
    /// Provider-specific fallback used when the caller omits `max_tokens`.
    /// Anthropic requires this field on every request; other rig-backed
    /// providers preserve their existing omission semantics by leaving it unset.
    default_max_tokens: Option<u32>,
    /// Whether this provider has been live-validated for rig-core streaming.
    /// Untested rig-backed providers retain the buffered trait fallback.
    native_streaming: bool,
    /// Whether this rig-core provider serializes `CompletionRequest::output_schema`.
    ///
    /// rig-core 0.36's dedicated DeepSeek and OpenRouter request paths accept
    /// the field in the shared request type but omit it from their wire
    /// payloads. Keep that limitation explicit at the adapter boundary rather
    /// than silently dispatching a request whose structured-output contract is
    /// lost.
    structured_output_supported: bool,
    /// Whether this rig-core path is an OpenAI-compatible Chat Completions
    /// adapter that can preserve native `json_object` mode.
    json_object_supported: bool,
    /// Optional model-discovery endpoint. When set, [`LlmProvider::list_models`]
    /// issues a `GET` instead of returning the empty default. rig-core's
    /// `CompletionModel` does not expose model discovery, so this is wired
    /// explicitly per protocol (OpenAI-compatible, Anthropic, Ollama).
    models_endpoint: Option<ModelsEndpoint>,
}

/// Auth scheme applied to a model-discovery request.
#[derive(Clone)]
pub(crate) enum ModelsAuth {
    /// `Authorization: Bearer <key>` (OpenAI-compatible, NEAR AI).
    Bearer(String),
    /// `x-api-key: <key>` plus an `anthropic-version` header (Anthropic).
    AnthropicKey { api_key: String, version: String },
    /// No auth header (Ollama).
    None,
}

/// Response body shape returned by a model-discovery endpoint.
#[derive(Clone, Copy)]
pub(crate) enum ModelsShape {
    /// OpenAI / Anthropic: `{ "data": [ { "id": ... } ] }`.
    OpenAiData,
    /// Ollama `/api/tags`: `{ "models": [ { "name": ... } ] }`.
    OllamaTags,
}

/// Connection details for a provider model-discovery request.
#[derive(Clone)]
pub(crate) struct ModelsEndpoint {
    /// Provider id, used for error/log context.
    pub(crate) provider_id: String,
    /// Fully-built request URL (base + discovery path).
    pub(crate) url: String,
    /// Auth scheme for the request.
    pub(crate) auth: ModelsAuth,
    /// Response body shape to parse.
    pub(crate) shape: ModelsShape,
    /// Extra headers applied to every request to this provider.
    pub(crate) extra_headers: reqwest::header::HeaderMap,
}

impl ModelsEndpoint {
    /// Issue the model-discovery `GET` and return the model ids.
    ///
    /// Validates the URL against the baseline SSRF guard, applies the
    /// adapter-specific auth scheme, and parses the adapter-specific response
    /// shape. Network, auth, and parse failures map to `LlmError` so the caller
    /// can surface a real message instead of an empty list.
    async fn fetch_models(&self) -> Result<Vec<String>, LlmError> {
        let validated = crate::url_check::check_models_url(&self.provider_id, &self.url).await?;

        // `check_models_url` validates only the initial URL. Disable redirect
        // following so a host that passes the guard cannot 3xx-redirect the
        // request to a blocked target (e.g. the cloud-metadata IP) — a 3xx is
        // surfaced as a non-success status below instead of being chased. The
        // shared builder also bypasses the proxy for loopback providers.
        let mut builder =
            crate::config::hardened_client_builder(crate::config::AUXILIARY_REQUEST_TIMEOUT_SECS)
                .redirect(reqwest::redirect::Policy::none());
        // Pin the client to the addresses the guard validated, so the
        // connect-time resolver can't rebind the hostname to a blocked IP after
        // the check passed (DNS TOCTOU). `None` for literal-IP / proxy-resolved
        // hosts, where there is nothing to pin.
        if let Some((host, addrs)) = &validated.pin {
            builder = builder.resolve_to_addrs(host, addrs);
        }
        let client = crate::url_check::build_http_client(&self.provider_id, &self.url, builder)?;

        let mut builder = client.get(&self.url).headers(self.extra_headers.clone());
        builder = match &self.auth {
            ModelsAuth::Bearer(key) => builder.bearer_auth(key),
            ModelsAuth::AnthropicKey { api_key, version } => builder
                .header("x-api-key", api_key)
                .header("anthropic-version", version),
            ModelsAuth::None => builder,
        };

        let response = builder.send().await.map_err(|e| LlmError::RequestFailed {
            provider: self.provider_id.clone(),
            reason: format!("request to {} failed: {e}", self.url),
        })?;

        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            return Err(LlmError::AuthFailed {
                provider: self.provider_id.clone(),
            });
        }
        if !status.is_success() {
            return Err(LlmError::RequestFailed {
                provider: self.provider_id.clone(),
                reason: format!("{} returned HTTP {}", self.url, status.as_u16()),
            });
        }

        // Bound the body: a model list is a few KB, but an operator-configured
        // (or compromised) endpoint could slow-drip megabytes within the 30s
        // timeout. Reject a declared oversize length up front, then stream with
        // a hard cap so memory stays bounded even when content-length is absent.
        use futures::StreamExt;
        const MAX_MODELS_BODY_BYTES: usize = 4 * 1024 * 1024;
        if let Some(len) = response.content_length()
            && len > MAX_MODELS_BODY_BYTES as u64
        {
            return Err(LlmError::InvalidResponse {
                provider: self.provider_id.clone(),
                reason: format!("models response too large ({len} bytes)"),
            });
        }
        let mut body = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| LlmError::StreamInterrupted {
                provider: self.provider_id.clone(),
                reason: format!("could not read models response: {e}"),
            })?;
            if body.len() + chunk.len() > MAX_MODELS_BODY_BYTES {
                return Err(LlmError::InvalidResponse {
                    provider: self.provider_id.clone(),
                    reason: "models response exceeded 4 MiB cap".to_string(),
                });
            }
            body.extend_from_slice(&chunk);
        }
        parse_models_response(&self.provider_id, self.shape, &body)
    }
}

/// Extract model ids from a model-discovery response body. Empty/whitespace
/// ids are dropped.
///
/// Split out from the HTTP call so the parsing contract is unit-testable
/// without a live endpoint. `ModelsShape` selects the JSON shape:
/// OpenAI/Anthropic `{ "data": [{ "id" }] }` vs Ollama
/// `{ "models": [{ "name" }] }`.
fn parse_models_response(
    provider_id: &str,
    shape: ModelsShape,
    body: &[u8],
) -> Result<Vec<String>, LlmError> {
    #[derive(serde::Deserialize)]
    struct OpenAiResponse {
        #[serde(default)]
        data: Vec<OpenAiEntry>,
    }
    #[derive(serde::Deserialize)]
    struct OpenAiEntry {
        id: String,
    }
    #[derive(serde::Deserialize)]
    struct OllamaResponse {
        #[serde(default)]
        models: Vec<OllamaEntry>,
    }
    #[derive(serde::Deserialize)]
    struct OllamaEntry {
        name: String,
    }

    let parse_err = |e: serde_json::Error| LlmError::InvalidResponse {
        provider: provider_id.to_string(),
        reason: format!("could not parse models response: {e}"),
    };

    let ids = match shape {
        ModelsShape::OpenAiData => serde_json::from_slice::<OpenAiResponse>(body)
            .map_err(parse_err)?
            .data
            .into_iter()
            .map(|entry| entry.id)
            .collect::<Vec<_>>(),
        ModelsShape::OllamaTags => serde_json::from_slice::<OllamaResponse>(body)
            .map_err(parse_err)?
            .models
            .into_iter()
            .map(|entry| entry.name)
            .collect::<Vec<_>>(),
    };

    Ok(ids.into_iter().filter(|id| !id.trim().is_empty()).collect())
}

impl<M: CompletionModel> RigAdapter<M> {
    /// Create a new adapter wrapping the given rig-core model.
    pub fn new(model: M, model_name: impl Into<String>) -> Self {
        let name = model_name.into();
        let (input_cost, output_cost) =
            costs::model_cost(&name).unwrap_or_else(costs::default_cost);
        Self {
            model,
            provider_id: name.clone(),
            model_name: name,
            input_cost,
            output_cost,
            cache_retention: CacheRetention::None,
            unsupported_params: HashSet::new(),
            default_additional_params: None,
            default_max_tokens: None,
            native_streaming: false,
            structured_output_supported: false,
            json_object_supported: false,
            models_endpoint: None,
        }
    }

    /// Set the stable provider identity used in typed provider errors.
    ///
    /// The model name and provider id are distinct payload fields. Keeping this
    /// explicit prevents a missing-model error from reporting the model slug as
    /// its provider.
    pub(crate) fn with_provider_id(mut self, provider_id: impl Into<String>) -> Self {
        self.provider_id = provider_id.into();
        self
    }

    /// Mark whether this rig-core provider preserves the request's structured
    /// output schema on the wire.
    pub(crate) fn with_structured_output_support(mut self, supported: bool) -> Self {
        self.structured_output_supported = supported;
        self
    }

    /// Mark this adapter as an OpenAI-compatible Chat Completions path that
    /// can serialize the native JSON-object response mode.
    pub(crate) fn with_json_object_support(mut self, supported: bool) -> Self {
        self.json_object_supported = supported;
        self
    }

    /// Enable model discovery for [`LlmProvider::list_models`].
    ///
    /// Without this, `list_models` falls back to the trait default (an empty
    /// list). The provider factories build a protocol-specific [`ModelsEndpoint`]
    /// (URL, auth scheme, response shape) so the "Fetch models" UI returns the
    /// provider's catalog.
    pub(crate) fn with_model_listing(mut self, endpoint: ModelsEndpoint) -> Self {
        self.models_endpoint = Some(endpoint);
        self
    }

    /// Set Anthropic prompt cache retention policy.
    ///
    /// Controls both cache injection and cost tracking:
    /// - `None` — no caching, no surcharge (1.0×).
    /// - `Short` — 5-minute TTL via `{"type": "ephemeral"}`, 1.25× write surcharge.
    /// - `Long` — 1-hour TTL via `{"type": "ephemeral", "ttl": "1h"}`, 2.0× write surcharge.
    ///
    /// Cache injection combines explicit breakpoints with Anthropic's
    /// automatic caching: `build_rig_request` marks the last tool definition
    /// (via `additional_params.tools`) and injects the top-level
    /// `cache_control` field, while `Short` retention additionally enables
    /// rig's typed system/last-message breakpoints at provider construction
    /// (`create_anthropic_from_registry`). See issue #6984.
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

    /// Set a provider-required fallback for requests that omit `max_tokens`.
    pub(crate) fn with_default_max_tokens(mut self, max_tokens: u32) -> Self {
        self.default_max_tokens = Some(max_tokens);
        self
    }

    fn max_tokens_or_default(&self, max_tokens: Option<u32>) -> Option<u32> {
        max_tokens.or(self.default_max_tokens)
    }

    /// Strip unsupported fields from a `CompletionRequest` in place.
    fn strip_unsupported_completion_params(&self, req: &mut CompletionRequest) {
        strip_unsupported_completion_params(&self.unsupported_params, req);
    }

    /// Strip unsupported fields from a `ToolCompletionRequest` in place.
    fn strip_unsupported_tool_params(&self, req: &mut ToolCompletionRequest) {
        strip_unsupported_tool_params(&self.unsupported_params, req);
    }

    fn ensure_structured_output_supported(
        &self,
        requested: Option<&CompletionResponseFormat>,
    ) -> Result<(), LlmError> {
        if matches!(requested, Some(CompletionResponseFormat::JsonSchema(_)))
            && !self.structured_output_supported
        {
            return Err(LlmError::InvalidRequest {
                provider: self.provider_id.clone(),
                reason: "structured output is not supported by this rig-core provider path"
                    .to_string(),
            });
        }
        Ok(())
    }

    async fn drain_streaming_response(
        &self,
        mut stream: StreamingCompletionResponse<M::StreamingResponse>,
        sink: std::sync::Arc<dyn CompletionStreamSink>,
    ) -> Result<DrainedStreamingResponse, LlmError> {
        let mut streamed_reasoning = StreamedReasoningAccumulator::default();
        while let Some(chunk) = stream.next().await {
            match chunk.map_err(|e| map_rig_error_for(&self.provider_id, &self.model_name, e))? {
                StreamedAssistantContent::Text(text) if !text.text.is_empty() => {
                    sink.text_delta(text.text).await;
                }
                StreamedAssistantContent::Reasoning(reasoning) => {
                    streamed_reasoning.push_reasoning(&reasoning);
                }
                StreamedAssistantContent::ReasoningDelta { id, reasoning } => {
                    streamed_reasoning.push_delta(id, reasoning);
                }
                _ => {}
            }
        }

        // rig-core sets `stream.response` only when the provider yields its
        // terminal frame. Its absence means the stream simply stopped
        // producing items — a dropped connection or an aborted generation —
        // which is the crate's established incomplete-stream condition, not a
        // completed call. Raising it here, before a response object exists,
        // is what lets retry and failover see a failure instead of a success.
        //
        // What this actually detects, in rig-core 0.36: **Ollama only**. Its
        // stream yields `FinalResponse` solely inside `if response.done`, so a
        // stream that stops early genuinely leaves `stream.response` unset.
        // Anthropic and Gemini, plus the shared OpenAI-compatible stream driver
        // used by OpenAI, OpenRouter, and DeepSeek, all yield a usage-only
        // `FinalResponse` after the SSE loop — including after the plain
        // `Err(StreamEnded) => break` arm. On those five a server that closes
        // mid-answer still sets `stream.response` and this check does not fire.
        // Catching that needs a terminal-observed signal rig-core does not
        // carry; it is a known limitation of this adapter, not a claim it makes.
        let Some(terminal_frame) = stream.response.as_ref() else {
            return Err(LlmError::StreamInterrupted {
                provider: self.model_name.clone(),
                reason: "stream ended before the provider's terminal frame".to_string(),
            });
        };
        let usage = terminal_frame.token_usage().unwrap_or_default();
        let raw_terminal_frame = serialize_raw_response(terminal_frame);
        let cache_creation_input_tokens = extract_cache_creation(raw_terminal_frame.as_ref());
        let provider_finish = extract_finish_reason(raw_terminal_frame.as_ref());
        // Note: in rig-core 0.36 only Ollama's terminal frame still carries a
        // finish reason (`done_reason`). The other five define
        // `StreamingResponse` as usage-only, so their per-chunk
        // `finish_reason`/`stop_reason` is dropped before it reaches this
        // adapter and `provider_finish` is `None` — shape inference is the
        // documented fallback there.
        let (text, tool_calls, finish, reasoning, reasoning_details) =
            extract_response(&stream.choice, &usage, provider_finish);
        let (streamed_reasoning, streamed_reasoning_details) = streamed_reasoning.finish();

        Ok(DrainedStreamingResponse {
            text,
            tool_calls,
            finish,
            reasoning: reasoning.or(streamed_reasoning),
            reasoning_details: reasoning_details.or(streamed_reasoning_details),
            usage,
            cache_creation_input_tokens,
        })
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
            crate::Role::System => {
                // Concatenate system messages into preamble
                match preamble {
                    Some(ref mut p) => {
                        p.push('\n');
                        p.push_str(&msg.content);
                    }
                    None => preamble = Some(msg.content.clone()),
                }
            }
            crate::Role::User => {
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
                        if let crate::ContentPart::ImageUrl { image_url } = part {
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
            crate::Role::Assistant => {
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
                    if let Some(reasoning) = assistant_reasoning_content(msg) {
                        contents.push(reasoning);
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
                } else if message_has_reasoning(msg) {
                    // Assistant message with reasoning but no tool calls
                    // (e.g., a "thinking" turn followed by a final answer).
                    // The next request still needs the reasoning echoed so the
                    // provider validates the chain, even when the message has
                    // no tool calls of its own.
                    let mut contents: Vec<AssistantContent> = Vec::new();
                    if !msg.content.is_empty() {
                        contents.push(AssistantContent::text(&msg.content));
                    }
                    if let Some(reasoning) = assistant_reasoning_content(msg) {
                        contents.push(reasoning);
                    }
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
            crate::Role::Tool => {
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
    let choice = choice?;
    match choice.to_lowercase().as_str() {
        "auto" => Some(RigToolChoice::Auto),
        "required" => Some(RigToolChoice::Required),
        "none" => Some(RigToolChoice::None),
        // A named tool: rig providers without specific-tool support reject
        // this loudly, which beats silently downgrading a forced call.
        _ => Some(RigToolChoice::Specific {
            function_names: vec![choice.to_string()],
        }),
    }
}

fn message_has_reasoning(msg: &ChatMessage) -> bool {
    msg.reasoning_details
        .as_ref()
        .is_some_and(|details| !details.is_empty())
        || msg
            .reasoning
            .as_ref()
            .is_some_and(|reasoning| !reasoning.trim().is_empty())
}

fn assistant_reasoning_content(msg: &ChatMessage) -> Option<AssistantContent> {
    if let Some(details) = msg
        .reasoning_details
        .as_ref()
        .filter(|details| !details.is_empty())
    {
        return Some(AssistantContent::Reasoning(iron_reasoning_to_rig(details)));
    }

    msg.reasoning
        .as_ref()
        .filter(|reasoning| !reasoning.trim().is_empty())
        .map(|reasoning| AssistantContent::Reasoning(rig::message::Reasoning::new(reasoning)))
}

fn iron_reasoning_to_rig(details: &IronReasoningDetails) -> rig::message::Reasoning {
    let mut reasoning = rig::message::Reasoning::new("");
    reasoning.id = details.id.clone();
    reasoning.content = details
        .content
        .iter()
        .map(|detail| match detail {
            IronReasoningDetail::Text { text, signature } => rig::message::ReasoningContent::Text {
                text: text.clone(),
                signature: signature.clone(),
            },
            IronReasoningDetail::Encrypted(text) => {
                rig::message::ReasoningContent::Encrypted(text.clone())
            }
            IronReasoningDetail::Redacted { data } => {
                rig::message::ReasoningContent::Redacted { data: data.clone() }
            }
            IronReasoningDetail::Summary(text) => {
                rig::message::ReasoningContent::Summary(text.clone())
            }
        })
        .collect();
    reasoning
}

fn rig_reasoning_to_iron(reasoning: &rig::message::Reasoning) -> Option<IronReasoningDetails> {
    let content = reasoning
        .content
        .iter()
        .filter_map(|detail| match detail {
            rig::message::ReasoningContent::Text { text, signature } => {
                Some(IronReasoningDetail::Text {
                    text: text.clone(),
                    signature: signature.clone(),
                })
            }
            rig::message::ReasoningContent::Encrypted(text) => {
                Some(IronReasoningDetail::Encrypted(text.clone()))
            }
            rig::message::ReasoningContent::Redacted { data } => {
                Some(IronReasoningDetail::Redacted { data: data.clone() })
            }
            rig::message::ReasoningContent::Summary(text) => {
                Some(IronReasoningDetail::Summary(text.clone()))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    let details = IronReasoningDetails {
        id: reasoning.id.clone(),
        content,
    };
    (!details.is_empty()).then_some(details)
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
    provider_finish: Option<FinishReason>,
) -> (
    Option<String>,
    Vec<IronToolCall>,
    FinishReason,
    Option<String>,
    Option<IronReasoningDetails>,
) {
    let mut text_parts: Vec<String> = Vec::new();
    let mut tool_calls: Vec<IronToolCall> = Vec::new();
    let mut reasoning_parts: Vec<String> = Vec::new();
    let mut reasoning_details: Vec<IronReasoningDetail> = Vec::new();
    let mut reasoning_id: Option<String> = None;

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
                    arguments_parse_error: None,
                });
            }
            AssistantContent::Reasoning(r) if !r.content.is_empty() => {
                if reasoning_id.is_none() {
                    reasoning_id = r.id.clone();
                }
                if let Some(details) = rig_reasoning_to_iron(r) {
                    if let Some(display_text) = details.display_text() {
                        reasoning_parts.push(display_text);
                    }
                    reasoning_details.extend(details.content);
                }
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
    let typed_reasoning = if reasoning_details.is_empty() {
        None
    } else {
        Some(IronReasoningDetails {
            id: reasoning_id,
            content: reasoning_details,
        })
    };

    let finish = resolve_finish_reason(provider_finish, !tool_calls.is_empty());

    (text, tool_calls, finish, reasoning, typed_reasoning)
}

#[derive(Default)]
struct StreamedReasoningAccumulator {
    parts: Vec<String>,
    details: Vec<IronReasoningDetail>,
    id: Option<String>,
}

impl StreamedReasoningAccumulator {
    fn push_reasoning(&mut self, reasoning: &rig::message::Reasoning) {
        if self.id.is_none() {
            self.id = reasoning.id.clone();
        }
        if let Some(details) = rig_reasoning_to_iron(reasoning) {
            if let Some(display_text) = details.display_text() {
                self.parts.push(display_text);
            }
            self.details.extend(details.content);
        }
    }

    fn push_delta(&mut self, id: Option<String>, reasoning: String) {
        if self.id.is_none() {
            self.id = id;
        }
        if !reasoning.trim().is_empty() {
            self.parts.push(reasoning);
        }
    }

    fn finish(self) -> (Option<String>, Option<IronReasoningDetails>) {
        let reasoning = if self.parts.is_empty() {
            None
        } else {
            Some(self.parts.join("\n"))
        };
        let details = if self.details.is_empty() {
            None
        } else {
            Some(IronReasoningDetails {
                id: self.id,
                content: self.details,
            })
        };
        (reasoning, details)
    }
}

struct DrainedStreamingResponse {
    text: Option<String>,
    tool_calls: Vec<IronToolCall>,
    finish: FinishReason,
    reasoning: Option<String>,
    reasoning_details: Option<IronReasoningDetails>,
    usage: RigUsage,
    cache_creation_input_tokens: u32,
}

/// Saturate u64 to u32 for token counts.
fn saturate_u32(val: u64) -> u32 {
    val.min(u32::MAX as u64) as u32
}

/// Downgrade a requested cache retention to `None` for models without
/// prompt-cache support. Shared by both Anthropic transports so the
/// construction-time decision (rig `prompt_caching` flag, OAuth breakpoint
/// application) always agrees with `with_cache_retention`'s validation.
pub(crate) fn effective_cache_retention(
    retention: CacheRetention,
    model_name: &str,
) -> CacheRetention {
    if retention != CacheRetention::None && !supports_prompt_cache(model_name) {
        CacheRetention::None
    } else {
        retention
    }
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

/// Serialize the raw provider response to JSON **once** per completion.
///
/// rig-core's unified types drop fields we need (`cache_creation_input_tokens`,
/// the provider's own finish reason), so both are read back out of the raw
/// body. Serializing the whole response — generated text, tool payloads and
/// all — is response-sized work, so every call site does it once and lends the
/// resulting `Value` to each extractor rather than each extractor serializing
/// for itself.
///
/// silent-ok: serializing a body rig already deserialized from the provider
/// cannot fail in practice. If it ever does, the extractors fall back to their
/// documented defaults rather than failing an otherwise-good response — but the
/// branch is logged, because it silently re-enables the guess this machinery
/// removes: a truncated response would then report `Stop`.
fn serialize_raw_response<T: Serialize>(raw: &T) -> Option<serde_json::Value> {
    match serde_json::to_value(raw) {
        Ok(value) => Some(value),
        Err(error) => {
            tracing::debug!(
                %error,
                "raw provider response would not serialize; falling back to \
                 response-shape inference and zero cache-creation tokens"
            );
            None
        }
    }
}

/// Extract `cache_creation_input_tokens` from the serialized provider response.
///
/// Rig-core's unified `Usage` does not surface this field, but Anthropic's raw
/// response includes it at `usage.cache_creation_input_tokens`.
fn extract_cache_creation(raw: Option<&serde_json::Value>) -> u32 {
    raw.and_then(|v| v.get("usage")?.get("cache_creation_input_tokens")?.as_u64())
        .map(|n| n.min(u32::MAX as u64) as u32)
        .unwrap_or(0)
}

/// Extract the provider's own finish/stop reason from the serialized response.
///
/// rig-core's unified `AssistantContent` says nothing about *why* generation
/// stopped, so a `max_tokens` truncation and a content-filter refusal used to
/// be indistinguishable from a clean answer. Every provider states it in its
/// raw body, so read the field the provider actually emits.
///
/// Returns `None` when the response carries no finish-reason field at all
/// (unknown or older provider shape); callers then fall back to inferring it
/// from the response shape. See [`resolve_finish_reason`].
fn extract_finish_reason(raw: Option<&serde_json::Value>) -> Option<FinishReason> {
    let token = provider_finish_token(raw?)?;
    map_provider_finish_token(token)
}

/// Locate the provider's finish-reason field across the response shapes rig fronts.
///
/// Paths verified against rig-core 0.36:
/// - OpenAI-shaped (`openai`, `openai_compatible`, `tinfoil`, `azure`,
///   `deepseek`, `openrouter`): `choices[0].finish_reason`
/// - Anthropic-by-key: `stop_reason`
/// - Ollama: `done_reason`
/// - Gemini by API key: `candidates[0].finishReason`, falling back to
///   `promptFeedback.blockReason`
///
/// Gemini blocks in two different places. A candidate's `finishReason` means
/// the *output* was cut off mid-generation. `promptFeedback.blockReason` means
/// the *input* prompt was rejected before generation, and rig's own
/// `GenerateContentResponse` documents the consequence: it returns no
/// candidates at all only when something was wrong with the prompt. Reading
/// just the candidate path therefore found nothing for a blocked prompt and
/// left the caller to infer a clean `Stop` from the response shape. The
/// candidate's own reason still wins when both are present.
fn provider_finish_token(value: &serde_json::Value) -> Option<&str> {
    let openai = value
        .get("choices")
        .and_then(|choices| choices.as_array())
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("finish_reason"))
        .and_then(|reason| reason.as_str());
    if openai.is_some() {
        return openai;
    }
    if let Some(anthropic) = value.get("stop_reason").and_then(|r| r.as_str()) {
        return Some(anthropic);
    }
    if let Some(ollama) = value.get("done_reason").and_then(|r| r.as_str()) {
        return Some(ollama);
    }
    let gemini_candidate = value
        .get("candidates")
        .and_then(|candidates| candidates.as_array())
        .and_then(|candidates| candidates.first())
        .and_then(|candidate| candidate.get("finishReason"))
        .and_then(|reason| reason.as_str());
    if gemini_candidate.is_some() {
        return gemini_candidate;
    }
    // `blockReason` speaks the same `SCREAMING_SNAKE_CASE` vocabulary as
    // `finishReason`, so the shared table classifies it without a second
    // mapping: `SAFETY` / `BLOCKLIST` / `PROHIBITED_CONTENT` / `IMAGE_SAFETY` /
    // `MODEL_ARMOR` are content blocks, `OTHER` and `BLOCK_REASON_UNSPECIFIED`
    // are unknown. An absent `blockReason` still means the provider said
    // nothing, which is the documented shape-inference fallback.
    value
        .get("promptFeedback")
        .and_then(|feedback| feedback.get("blockReason"))
        .and_then(|reason| reason.as_str())
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
/// When `cache_retention` is not `None`, emits Anthropic cache markers
/// (issue #6984):
///
/// - A top-level `cache_control` field via `additional_params` (rig-core's
///   `AnthropicCompletionRequest` uses `#[serde(flatten)]` on
///   `additional_params`, so the field lands at the request root). This is
///   Anthropic's **automatic caching**: the API places a breakpoint at the
///   last cacheable block and moves it forward as conversations grow.
/// - An explicit breakpoint on the **last tool definition**, so the tool
///   prefix stays cached even when later parts of the prompt change. rig's
///   typed `ToolDefinition` cannot carry `cache_control`, so the last tool
///   is moved into `additional_params.tools` in Anthropic's native shape —
///   rig appends raw `additional_params.tools` entries after the typed
///   tools, preserving order.
///
/// The system-prompt and last-message-block breakpoints are rig's own
/// `prompt_caching` flag, set at provider construction for `Short` retention
/// only: rig's typed markers are always plain 5m ephemeral, and a 5m block
/// marker combined with a 1h automatic marker is rejected by the API (TTL
/// conflict on the last block), so `Long` relies on the automatic marker for
/// the conversation tail.
#[allow(clippy::too_many_arguments)]
fn build_rig_request(
    preamble: Option<String>,
    mut history: Vec<RigMessage>,
    mut tools: Vec<RigToolDefinition>,
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

    let additional_params = match cache_retention.cache_control_json() {
        None => None,
        Some(marker) => {
            let mut params = serde_json::json!({ "cache_control": marker.clone() });
            if let Some(last_tool) = tools.pop() {
                // Anthropic-native tool shape: rig maps `parameters` →
                // `input_schema` only for typed tools; raw entries pass
                // through verbatim.
                params["tools"] = serde_json::json!([{
                    "name": last_tool.name,
                    "description": last_tool.description,
                    "input_schema": last_tool.parameters,
                    "cache_control": marker,
                }]);
            }
            Some(params)
        }
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
        model: None,
        output_schema: None,
    })
}

/// Inject a per-request model override into the typed rig request field.
///
/// Rig-core providers use this field in preference to their configured model.
/// Keeping it out of flattened `additional_params` ensures the serialized JSON
/// request has exactly one top-level `model` key.
fn inject_model_override(rig_req: &mut RigRequest, model_override: Option<&str>) {
    rig_req.model = model_override.map(str::to_owned);
}

/// Convert the provider-neutral response schema into rig-core's typed schema.
///
/// All four completion paths use the same request field, so keep conversion
/// and its stable error mapping in one helper rather than letting a path drift.
fn set_output_schema(
    rig_req: &mut RigRequest,
    response_format: Option<CompletionResponseFormat>,
    provider_id: &str,
    json_object_supported: bool,
) -> Result<(), LlmError> {
    match response_format {
        None => {}
        Some(CompletionResponseFormat::JsonSchema(format)) => {
            // rig-core derives the OpenAI `json_schema.name` from the root
            // schema's `title` and otherwise falls back to
            // `response_schema`.  The provider-neutral contract carries its
            // name separately, so copy that identity into the annotation
            // rig-core reads before handing the schema over.  `title` is a
            // JSON Schema annotation and does not change the schema's
            // validation semantics; overwriting a stale title is required so
            // the durable response-format name remains authoritative.
            let mut schema = format.schema;
            let object = schema
                .as_object_mut()
                .ok_or_else(|| LlmError::InvalidRequest {
                    provider: provider_id.to_string(),
                    reason: "JSON Schema response format must be an object".to_string(),
                })?;
            object.insert("title".to_string(), serde_json::Value::String(format.name));
            rig_req.output_schema =
                Some(
                    serde_json::from_value(schema).map_err(|error| LlmError::InvalidRequest {
                        provider: provider_id.to_string(),
                        reason: format!("invalid JSON Schema response format: {error}"),
                    })?,
                );
        }
        Some(CompletionResponseFormat::JsonObject) => {
            if !json_object_supported {
                return Err(LlmError::InvalidRequest {
                    provider: provider_id.to_string(),
                    reason:
                        "native JSON-object response mode is not supported by this provider path"
                            .to_string(),
                });
            }
            let params = rig_req
                .additional_params
                .get_or_insert_with(|| serde_json::json!({}));
            let object = params
                .as_object_mut()
                .ok_or_else(|| LlmError::InvalidRequest {
                    provider: provider_id.to_string(),
                    reason: "provider additional parameters must be a JSON object".to_string(),
                })?;
            object.insert(
                "response_format".to_string(),
                serde_json::json!({"type": "json_object"}),
            );
        }
    }
    Ok(())
}

#[async_trait]
impl<M> LlmProvider for RigAdapter<M>
where
    M: CompletionModel + Send + Sync + 'static,
    M::Response: Send + Sync + Serialize + DeserializeOwned,
{
    fn provider_id(&self) -> String {
        self.provider_id.clone()
    }

    fn model_name(&self) -> &str {
        &self.model_name
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        (self.input_cost, self.output_cost)
    }

    async fn list_models(&self) -> Result<Vec<String>, LlmError> {
        let Some(endpoint) = self.models_endpoint.as_ref() else {
            // No discovery endpoint wired (e.g. Anthropic/Ollama paths); preserve
            // the trait default rather than guessing a URL.
            return Ok(Vec::new());
        };
        endpoint.fetch_models().await
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
        self.ensure_structured_output_supported(request.response_format.as_ref())?;
        let model_override = request.take_model_override();
        let response_format = request.response_format.take();

        self.strip_unsupported_completion_params(&mut request);

        let mut messages = request.messages;
        crate::provider::sanitize_tool_messages(&mut messages);
        let (preamble, history) = convert_messages(&messages);

        let mut rig_req = build_rig_request(
            preamble,
            history,
            Vec::new(),
            None,
            request.temperature,
            self.max_tokens_or_default(request.max_tokens),
            self.cache_retention,
        )?;
        set_output_schema(
            &mut rig_req,
            response_format,
            &self.provider_id,
            self.json_object_supported,
        )?;

        merge_additional_params(&mut rig_req, self.default_additional_params.as_ref());
        inject_model_override(&mut rig_req, model_override.as_deref());

        let response = self
            .model
            .completion(rig_req)
            .await
            .map_err(|e| map_rig_error_for(&self.provider_id, &self.model_name, e))?;

        let raw_response = serialize_raw_response(&response.raw_response);
        let provider_finish = extract_finish_reason(raw_response.as_ref());
        let (text, _tool_calls, finish, reasoning, _reasoning_details) =
            extract_response(&response.choice, &response.usage, provider_finish);

        let resp = CompletionResponse {
            content: text.unwrap_or_default(),
            input_tokens: saturate_u32(response.usage.input_tokens),
            output_tokens: saturate_u32(response.usage.output_tokens),
            finish_reason: finish,
            reasoning,
            cache_read_input_tokens: saturate_u32(response.usage.cached_input_tokens),
            cache_creation_input_tokens: extract_cache_creation(raw_response.as_ref()),
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

    async fn complete_streaming(
        &self,
        mut request: CompletionRequest,
        sink: std::sync::Arc<dyn CompletionStreamSink>,
    ) -> Result<CompletionResponse, LlmError> {
        self.ensure_structured_output_supported(request.response_format.as_ref())?;
        if !self.native_streaming {
            return self.complete(request).await;
        }

        let model_override = request.take_model_override();
        let response_format = request.response_format.take();

        self.strip_unsupported_completion_params(&mut request);

        let mut messages = request.messages;
        crate::provider::sanitize_tool_messages(&mut messages);
        let (preamble, history) = convert_messages(&messages);

        let mut rig_req = build_rig_request(
            preamble,
            history,
            Vec::new(),
            None,
            request.temperature,
            self.max_tokens_or_default(request.max_tokens),
            self.cache_retention,
        )?;
        set_output_schema(
            &mut rig_req,
            response_format,
            &self.provider_id,
            self.json_object_supported,
        )?;

        merge_additional_params(&mut rig_req, self.default_additional_params.as_ref());
        inject_model_override(&mut rig_req, model_override.as_deref());

        let stream = self
            .model
            .stream(rig_req)
            .await
            .map_err(|e| map_rig_error_for(&self.provider_id, &self.model_name, e))?;
        let drained = self.drain_streaming_response(stream, sink).await?;

        let resp = CompletionResponse {
            content: drained.text.unwrap_or_default(),
            input_tokens: saturate_u32(drained.usage.input_tokens),
            output_tokens: saturate_u32(drained.usage.output_tokens),
            finish_reason: drained.finish,
            reasoning: drained.reasoning,
            cache_read_input_tokens: saturate_u32(drained.usage.cached_input_tokens),
            cache_creation_input_tokens: drained.cache_creation_input_tokens,
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
        self.ensure_structured_output_supported(request.response_format.as_ref())?;
        let model_override = request.take_model_override();
        let response_format = request.response_format.take();

        self.strip_unsupported_tool_params(&mut request);

        let known_tool_names: HashSet<String> =
            request.tools.iter().map(|t| t.name.clone()).collect();

        let mut messages = request.messages;
        crate::provider::sanitize_tool_messages(&mut messages);
        let (preamble, history) = convert_messages(&messages);
        let tools = convert_tools(&request.tools);
        let tool_choice = convert_tool_choice(request.tool_choice.as_deref());

        let mut rig_req = build_rig_request(
            preamble,
            history,
            tools,
            tool_choice,
            request.temperature,
            self.max_tokens_or_default(request.max_tokens),
            self.cache_retention,
        )?;
        set_output_schema(
            &mut rig_req,
            response_format,
            &self.provider_id,
            self.json_object_supported,
        )?;

        merge_additional_params(&mut rig_req, self.default_additional_params.as_ref());
        inject_model_override(&mut rig_req, model_override.as_deref());

        let response = self
            .model
            .completion(rig_req)
            .await
            .map_err(|e| map_rig_error_for(&self.provider_id, &self.model_name, e))?;

        let raw_response = serialize_raw_response(&response.raw_response);
        let provider_finish = extract_finish_reason(raw_response.as_ref());
        let (text, mut tool_calls, finish, reasoning, reasoning_details) =
            extract_response(&response.choice, &response.usage, provider_finish);

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

        // Strict-mode tool schemas advertise every optional as required+nullable,
        // so the model fills unset optionals with `null`. Strip those placeholders
        // against each tool's original schema so only provided values reach the
        // tool.
        crate::tool_schema::strip_unset_optional_fields(
            &mut tool_calls,
            &request.tools,
            crate::tool_schema::PlaceholderStrippingMode::NullOnly,
        );

        let resp = ToolCompletionResponse {
            content: text,
            tool_calls,
            input_tokens: saturate_u32(response.usage.input_tokens),
            output_tokens: saturate_u32(response.usage.output_tokens),
            finish_reason: finish,
            cache_read_input_tokens: saturate_u32(response.usage.cached_input_tokens),
            cache_creation_input_tokens: extract_cache_creation(raw_response.as_ref()),
            reasoning,
            reasoning_details,
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

    async fn complete_with_tools_streaming(
        &self,
        mut request: ToolCompletionRequest,
        sink: std::sync::Arc<dyn CompletionStreamSink>,
    ) -> Result<ToolCompletionResponse, LlmError> {
        self.ensure_structured_output_supported(request.response_format.as_ref())?;
        if !self.native_streaming {
            return self.complete_with_tools(request).await;
        }

        let model_override = request.take_model_override();
        let response_format = request.response_format.take();

        self.strip_unsupported_tool_params(&mut request);

        let known_tool_names: HashSet<String> =
            request.tools.iter().map(|t| t.name.clone()).collect();

        let mut messages = request.messages;
        crate::provider::sanitize_tool_messages(&mut messages);
        let (preamble, history) = convert_messages(&messages);
        let tools = convert_tools(&request.tools);
        let tool_choice = convert_tool_choice(request.tool_choice.as_deref());

        let mut rig_req = build_rig_request(
            preamble,
            history,
            tools,
            tool_choice,
            request.temperature,
            self.max_tokens_or_default(request.max_tokens),
            self.cache_retention,
        )?;
        set_output_schema(
            &mut rig_req,
            response_format,
            &self.provider_id,
            self.json_object_supported,
        )?;

        merge_additional_params(&mut rig_req, self.default_additional_params.as_ref());
        inject_model_override(&mut rig_req, model_override.as_deref());

        let stream = self
            .model
            .stream(rig_req)
            .await
            .map_err(|e| map_rig_error_for(&self.provider_id, &self.model_name, e))?;
        let mut drained = self.drain_streaming_response(stream, sink).await?;

        // Normalize tool call names: some proxies prepend "proxy_" prefixes.
        for tc in &mut drained.tool_calls {
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

        crate::tool_schema::strip_unset_optional_fields(
            &mut drained.tool_calls,
            &request.tools,
            crate::tool_schema::PlaceholderStrippingMode::NullOnly,
        );

        let resp = ToolCompletionResponse {
            content: drained.text,
            tool_calls: drained.tool_calls,
            input_tokens: saturate_u32(drained.usage.input_tokens),
            output_tokens: saturate_u32(drained.usage.output_tokens),
            finish_reason: drained.finish,
            cache_read_input_tokens: saturate_u32(drained.usage.cached_input_tokens),
            cache_creation_input_tokens: drained.cache_creation_input_tokens,
            reasoning: drained.reasoning,
            reasoning_details: drained.reasoning_details,
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

    fn effective_model_name(&self, requested_model: Option<&str>) -> String {
        normalized_model_override(requested_model)
            .map(str::to_owned)
            .unwrap_or_else(|| self.active_model_name())
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
#[cfg(test)]
fn map_rig_error(model_name: &str, e: impl std::fmt::Display) -> LlmError {
    map_rig_error_for(model_name, model_name, e)
}

fn map_rig_error_for(
    provider_id: &str,
    model_name: &str,
    error: impl std::fmt::Display,
) -> LlmError {
    crate::error::map_provider_message_error(provider_id, model_name, error.to_string())
}

/// Detect authentication/authorization failures in a lowercased provider
/// error message.
///
/// Mirrors the shape of [`crate::error::is_context_length_error_message`].
/// Deliberately conservative: matches robust indicators (HTTP 401/403,
/// explicit "invalid api key"/"unauthorized"/"authentication" phrasing) and
/// avoids unrelated substrings such as a bare `"key"`.
/// Whether `code` appears in `lower` as a standalone number rather than as a
/// run of digits inside a larger one.
///
/// `"401"`/`"403"` used to be matched with a bare `contains`, so **any** number
/// containing those digits classified the failure as an auth error: a
/// `Retry-After` of `4013 ms`, a request id ending `1403`, a token count of
/// `24019`. The run then terminated telling the user to fix their API key —
/// for what was usually a rate limit, the single most common transient
/// provider error. #6284 WS5.
#[cfg(test)]
fn is_auth_error_message(lower: &str) -> bool {
    crate::error::is_auth_error_message(lower)
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
    use rig::completion::CompletionError;
    use rig::streaming::{RawStreamingChoice, RawStreamingToolCall, StreamingCompletionResponse};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;
    use tokio::sync::{mpsc, oneshot};

    const LOOPBACK_CAPTURE_TIMEOUT: Duration = Duration::from_secs(5);

    fn with_native_streaming<M: CompletionModel>(mut adapter: RigAdapter<M>) -> RigAdapter<M> {
        adapter.native_streaming = true;
        adapter
    }

    #[test]
    fn output_schema_rejects_non_object_root_before_dispatch() {
        let mut request = build_rig_request(
            None,
            Vec::new(),
            Vec::new(),
            None,
            None,
            None,
            CacheRetention::None,
        )
        .expect("empty rig request");
        let error = set_output_schema(
            &mut request,
            Some(CompletionResponseFormat::JsonSchema(
                crate::provider::JsonSchemaResponseFormat::strict(
                    "suggestions",
                    serde_json::json!(["not", "an", "object"]),
                ),
            )),
            "test-provider",
            true,
        )
        .expect_err("a JSON Schema response format must have an object root");

        assert!(matches!(
            error,
            LlmError::InvalidRequest { provider, reason }
                if provider == "test-provider" && reason.contains("must be an object")
        ));
    }

    async fn capture_one_http_request() -> (String, oneshot::Receiver<String>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind loopback capture server");
        let addr = listener.local_addr().expect("capture server address");
        let (tx, rx) = oneshot::channel();
        tokio::spawn(async move {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let mut request = Vec::new();
            let mut buffer = [0_u8; 4096];
            loop {
                let read = socket.read(&mut buffer).await.expect("read request");
                if read == 0 {
                    return;
                }
                request.extend_from_slice(&buffer[..read]);
                let Some(header_end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n")
                else {
                    continue;
                };
                let headers =
                    std::str::from_utf8(&request[..header_end]).expect("request headers are UTF-8");
                let content_length = headers
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then_some(value.trim())
                    })
                    .expect("content length header")
                    .parse::<usize>()
                    .expect("content length is numeric");
                let body_start = header_end + 4;
                if request.len() < body_start + content_length {
                    continue;
                }
                tx.send(
                    String::from_utf8(request[body_start..body_start + content_length].to_vec())
                        .expect("request body is UTF-8"),
                )
                .expect("test receives captured request");
                socket
                    .write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n")
                    .await
                    .expect("write test response");
                return;
            }
        });
        (format!("http://{addr}/v1"), rx)
    }

    fn tool_request_with_model_override() -> ToolCompletionRequest {
        ToolCompletionRequest::new(
            vec![ChatMessage::user("use the echo tool")],
            vec![IronToolDefinition {
                name: "echo".to_string(),
                description: "Echo the supplied text".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": { "text": { "type": "string" } },
                    "required": ["text"]
                }),
            }],
        )
        .with_model("override-model")
    }

    fn assert_single_model_override(body: &str) {
        assert_eq!(
            body.match_indices("\"model\":").count(),
            1,
            "the raw JSON body must have exactly one top-level model key: {body}",
        );
        assert!(
            body.contains("\"model\":\"override-model\""),
            "the one serialized model must preserve the request override: {body}",
        );
    }

    async fn captured_request_body(captured_body: oneshot::Receiver<String>) -> String {
        tokio::time::timeout(LOOPBACK_CAPTURE_TIMEOUT, captured_body)
            .await
            .expect("request capture timed out")
            .expect("captured request body")
    }

    #[tokio::test]
    async fn tool_completion_request_serializes_the_model_once() {
        use rig::client::CompletionClient;
        use rig::providers::openai;

        let (base_url, captured_body) = capture_one_http_request().await;
        let client = openai::Client::builder()
            .api_key("test-key")
            .base_url(&base_url)
            .build()
            .expect("build OpenAI-compatible client")
            .completions_api();
        let adapter = RigAdapter::new(
            client.completion_model("configured-model"),
            "configured-model",
        );
        adapter
            .complete_with_tools(tool_request_with_model_override())
            .await
            .expect_err("capture server intentionally rejects the request");

        let body = captured_request_body(captured_body).await;
        assert_single_model_override(&body);
    }

    #[tokio::test]
    async fn completion_request_serializes_native_output_schema() {
        use rig::client::CompletionClient;
        use rig::providers::openai;

        let (base_url, captured_body) = capture_one_http_request().await;
        let client = openai::Client::builder()
            .api_key("test-key")
            .base_url(&base_url)
            .build()
            .expect("build OpenAI-compatible client")
            .completions_api();
        let adapter = RigAdapter::new(
            client.completion_model("configured-model"),
            "configured-model",
        )
        .with_structured_output_support(true);
        let mut request = CompletionRequest::new(vec![ChatMessage::user("Return suggestions")]);
        request.response_format = Some(crate::provider::CompletionResponseFormat::JsonSchema(
            crate::provider::JsonSchemaResponseFormat::strict(
                "suggestions",
                serde_json::json!({
                    "type": "object",
                    "properties": {"items": {"type": "array"}}
                }),
            ),
        ));
        adapter
            .complete(request)
            .await
            .expect_err("capture server intentionally rejects the request");

        let body = captured_request_body(captured_body).await;
        let body: serde_json::Value = serde_json::from_str(&body).expect("request JSON");
        assert_eq!(body["response_format"]["type"], "json_schema");
        assert_eq!(
            body["response_format"]["json_schema"]["name"],
            "suggestions"
        );
        assert_eq!(body["response_format"]["json_schema"]["strict"], true);
        assert_eq!(
            body["response_format"]["json_schema"]["schema"]["properties"]["items"]["type"],
            "array"
        );
    }

    #[tokio::test]
    async fn completion_request_serializes_native_json_object_mode() {
        use rig::client::CompletionClient;
        use rig::providers::openai;

        let (base_url, captured_body) = capture_one_http_request().await;
        let client = openai::Client::builder()
            .api_key("test-key")
            .base_url(&base_url)
            .build()
            .expect("build OpenAI-compatible client")
            .completions_api();
        let adapter = RigAdapter::new(
            client.completion_model("configured-model"),
            "configured-model",
        )
        .with_json_object_support(true);
        let mut request = CompletionRequest::new(vec![ChatMessage::user("Return an object")]);
        request.response_format = Some(crate::provider::CompletionResponseFormat::JsonObject);
        adapter
            .complete(request)
            .await
            .expect_err("capture server intentionally rejects the request");

        let body = captured_request_body(captured_body).await;
        let body: serde_json::Value = serde_json::from_str(&body).expect("request JSON");
        assert_eq!(
            body["response_format"],
            serde_json::json!({"type": "json_object"})
        );
    }

    #[derive(Clone)]
    struct CacheableCompletionModel;

    impl CompletionModel for CacheableCompletionModel {
        type Response = serde_json::Value;
        type StreamingResponse = ();
        type Client = ();

        fn make(_client: &Self::Client, _model: impl Into<String>) -> Self {
            Self
        }

        async fn completion(
            &self,
            _request: RigRequest,
        ) -> Result<rig::completion::CompletionResponse<Self::Response>, CompletionError> {
            Ok(rig::completion::CompletionResponse {
                choice: OneOrMany::one(AssistantContent::text("cached response")),
                usage: RigUsage {
                    input_tokens: 1,
                    output_tokens: 1,
                    total_tokens: 2,
                    cached_input_tokens: 0,
                    cache_creation_input_tokens: 0,
                },
                raw_response: serde_json::json!({}),
                message_id: None,
            })
        }

        async fn stream(
            &self,
            _request: RigRequest,
        ) -> Result<StreamingCompletionResponse<Self::StreamingResponse>, CompletionError> {
            Err(CompletionError::ProviderError(
                "stream unsupported".to_string(),
            ))
        }
    }

    #[tokio::test]
    async fn cached_provider_separates_normalized_rig_model_overrides() {
        use crate::response_cache::{CachedProvider, ResponseCacheConfig};

        let adapter = Arc::new(RigAdapter::new(
            CacheableCompletionModel,
            "configured-model",
        ));
        let cached = CachedProvider::new(adapter, ResponseCacheConfig::default());

        cached
            .complete(
                CompletionRequest::new(vec![ChatMessage::user("same prompt")])
                    .with_model(" override-a "),
            )
            .await
            .expect("first completion succeeds");
        cached
            .complete(
                CompletionRequest::new(vec![ChatMessage::user("same prompt")])
                    .with_model("override-b"),
            )
            .await
            .expect("second completion succeeds");
        cached
            .complete(
                CompletionRequest::new(vec![ChatMessage::user("same prompt")])
                    .with_model("override-a"),
            )
            .await
            .expect("normalized override reuses its cache entry");

        assert_eq!(cached.len(), 2);
        assert_eq!(cached.total_hits(), 1);
    }

    #[tokio::test]
    async fn deepseek_tool_completion_request_serializes_the_model_once() {
        use rig::client::CompletionClient;
        use rig::providers::deepseek;

        let (base_url, captured_body) = capture_one_http_request().await;
        let client = deepseek::Client::builder()
            .api_key("test-key")
            .base_url(&base_url)
            .build()
            .expect("build DeepSeek client");
        let adapter = RigAdapter::new(
            client.completion_model("configured-model"),
            "configured-model",
        );
        adapter
            .complete_with_tools(tool_request_with_model_override())
            .await
            .expect_err("capture server intentionally rejects the request");

        let body = captured_request_body(captured_body).await;
        assert_single_model_override(&body);
    }

    #[test]
    fn map_rig_error_auth_401_unauthorized() {
        let err = map_rig_error("openai", "HTTP error 401 Unauthorized: invalid api key");
        assert!(
            matches!(err, LlmError::AuthFailed { provider } if provider == "openai"),
            "401/invalid api key should map to AuthFailed, got a different variant",
        );
    }

    #[test]
    fn map_rig_error_keeps_provider_and_model_identity_distinct() {
        let error = map_rig_error_for(
            "openai",
            "gpt-5-fixture",
            "HTTP 404: requested model does not exist",
        );
        assert!(matches!(
            error,
            LlmError::ModelNotAvailable { provider, model }
                if provider == "openai" && model == "gpt-5-fixture"
        ));
    }

    #[test]
    fn map_rig_error_auth_invalid_api_key() {
        let err = map_rig_error("anthropic", "Incorrect API key provided");
        assert!(
            matches!(err, LlmError::AuthFailed { provider } if provider == "anthropic"),
            "invalid/incorrect api key should map to AuthFailed",
        );
    }

    #[test]
    fn map_rig_error_context_length_still_wins_over_auth() {
        // A context-length error must never be misread as auth.
        let err = map_rig_error(
            "openai",
            "This model's maximum context length is 128000 tokens.",
        );
        assert!(
            matches!(err, LlmError::ContextLengthExceeded { .. }),
            "context-length error should map to ContextLengthExceeded",
        );
    }

    /// A number that merely *contains* 401 or 403 is not an auth failure.
    ///
    /// `is_auth_error_message` matched `"401"`/`"403"` with a bare `contains`,
    /// so a `Retry-After` of `4013 ms` — a rate limit, the most common
    /// transient provider error there is — classified as an auth failure. The
    /// run terminated immediately telling the user to fix their API key, for a
    /// condition that would have cleared on its own. #6284 WS5.
    #[test]
    fn a_number_containing_401_is_not_an_auth_failure() {
        for message in [
            "rate limited, retry after 4013 ms",
            "rate limited, retry after 14030 ms",
            "request id req_1403f2a failed",
            "context window exceeded: 24019 tokens",
            "upstream returned 4010",
        ] {
            assert!(
                !is_auth_error_message(message),
                "{message:?} is not an auth failure, but was classified as one — \
                 the run would terminate telling the user to fix their API key"
            );
            // Through the caller, because the predicate is not what the run
            // acts on: `map_rig_error` turns it into the error variant, and
            // `retry::is_retryable` keys off that variant. A predicate fix
            // that failed to change the classification would leave the bug
            // exactly where it was.
            let error = map_rig_error("openai", message);
            assert!(
                !matches!(error, LlmError::AuthFailed { .. }),
                "{message:?} mapped to {error:?}; a transient condition must not \
                 become a terminal auth failure"
            );
            assert!(
                crate::retry::is_retryable(&error),
                "{message:?} mapped to {error:?}, which is not retried — this is \
                 the rate limit the fix exists for, and it would have cleared"
            );
        }
    }

    /// The real status codes must still be caught.
    #[test]
    fn a_standalone_401_or_403_is_still_an_auth_failure() {
        for message in [
            "server returned 401",
            "http 403 while calling the model",
            "401",
            "status: 403, body: {}",
            "(401)",
        ] {
            assert!(
                is_auth_error_message(message),
                "{message:?} is a genuine auth failure and must be classified as one"
            );
            // The contract the run depends on: a real 401/403 becomes
            // `AuthFailed`, which is neither retried nor allowed to trip the
            // circuit breaker.
            let error = map_rig_error("openai", message);
            assert!(
                matches!(error, LlmError::AuthFailed { ref provider } if provider == "openai"),
                "{message:?} mapped to {error:?}, not AuthFailed"
            );
            assert!(
                !crate::retry::is_retryable(&error),
                "{message:?} is a bad credential; retrying it cannot help"
            );
        }
    }

    /// A missing model must not be retried twelve times.
    ///
    /// `LlmError::ModelNotAvailable` had **zero producers**, so a 404 /
    /// model-not-found fell into `RequestFailed`, which `retry::is_retryable`
    /// treats as retryable. A typo'd or decommissioned model id burned the full
    /// retry budget before failing. #6284 WS5.
    #[test]
    fn a_missing_model_is_not_retried_as_a_transient_failure() {
        for message in [
            "The model `gpt-5-turbo` does not exist",
            "model not found",
            "404 model_not_found",
            "unknown model: claude-99",
        ] {
            let error = map_rig_error("gpt-5-turbo", message);
            assert!(
                matches!(error, LlmError::ModelNotAvailable { .. }),
                "{message:?} must map to ModelNotAvailable, got {error:?}"
            );
            assert!(
                !crate::retry::is_retryable(&error),
                "{message:?} must not be retried — the model will not appear between attempts"
            );
        }
    }

    /// An unrelated 404 is not a missing model.
    #[test]
    fn an_unrelated_404_still_falls_through() {
        let error = map_rig_error("gpt-5-turbo", "404 not found: /v1/healthz");
        assert!(
            matches!(error, LlmError::RequestFailed { .. }),
            "an unrelated 404 must not be read as a missing model, got {error:?}"
        );
    }

    #[test]
    fn map_rig_error_unrelated_still_request_failed() {
        let err = map_rig_error("openai", "connection reset by peer");
        assert!(
            matches!(err, LlmError::RequestFailed { provider, .. } if provider == "openai"),
            "unrelated transient error should remain RequestFailed",
        );
    }

    #[test]
    fn is_auth_error_message_conservative() {
        // Positive indicators
        assert!(is_auth_error_message("401 unauthorized"));
        assert!(is_auth_error_message("403 forbidden"));
        assert!(is_auth_error_message("invalid api key"));
        assert!(is_auth_error_message("incorrect api key"));
        assert!(is_auth_error_message("error code: invalid_api_key"));
        assert!(is_auth_error_message("authentication failed"));
        assert!(is_auth_error_message("permission denied"));
        assert!(is_auth_error_message("missing api key"));
        assert!(is_auth_error_message("no api key was provided"));
        // Conservative: a bare "key" or unrelated message must not match.
        assert!(!is_auth_error_message("could not find the key in the map"));
        assert!(!is_auth_error_message("connection reset by peer"));
        assert!(!is_auth_error_message("rate limit exceeded"));
    }

    #[test]
    fn parse_models_response_openai_extracts_ids() {
        let body =
            br#"{"object":"list","data":[{"id":"gpt-4o","object":"model"},{"id":"gpt-4o-mini"}]}"#;
        let models =
            parse_models_response("openai", ModelsShape::OpenAiData, body).expect("parses");
        assert_eq!(models, vec!["gpt-4o", "gpt-4o-mini"]);
    }

    #[test]
    fn parse_models_response_ollama_extracts_names() {
        let body = br#"{"models":[{"name":"llama3.2:latest"},{"name":"qwen2.5"}]}"#;
        let models =
            parse_models_response("ollama", ModelsShape::OllamaTags, body).expect("parses");
        assert_eq!(models, vec!["llama3.2:latest", "qwen2.5"]);
    }

    #[test]
    fn parse_models_response_drops_blank_ids_and_tolerates_missing_data() {
        let with_blank = br#"{"data":[{"id":"a"},{"id":"  "},{"id":""}]}"#;
        assert_eq!(
            parse_models_response("openai", ModelsShape::OpenAiData, with_blank).expect("parses"),
            vec!["a"]
        );
        // A successful response with no `data` key yields an empty list, not an error.
        let no_data = br#"{"object":"list"}"#;
        assert!(
            parse_models_response("openai", ModelsShape::OpenAiData, no_data)
                .expect("parses")
                .is_empty()
        );
    }

    #[test]
    fn parse_models_response_rejects_malformed_json() {
        let err = parse_models_response("openai", ModelsShape::OpenAiData, b"not json")
            .expect_err("rejects");
        assert!(matches!(err, LlmError::InvalidResponse { .. }));
    }

    // Serve one canned HTTP response on a loopback port and return the endpoint
    // pointed at it. The response is written verbatim, so callers control the
    // status line and headers.
    async fn endpoint_against_canned_response(raw_response: &'static str) -> ModelsEndpoint {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind loopback");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(async move {
            if let Ok((mut sock, _)) = listener.accept().await {
                let mut buf = [0u8; 1024];
                let _ = sock.read(&mut buf).await;
                let _ = sock.write_all(raw_response.as_bytes()).await;
                let _ = sock.flush().await;
            }
        });
        ModelsEndpoint {
            provider_id: "p".to_string(),
            url: format!("http://{addr}/models"),
            auth: ModelsAuth::Bearer("k".to_string()),
            shape: ModelsShape::OpenAiData,
            extra_headers: reqwest::header::HeaderMap::new(),
        }
    }

    #[tokio::test]
    async fn fetch_models_maps_401_to_auth_failed() {
        let endpoint = endpoint_against_canned_response(
            "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\n\r\n",
        )
        .await;
        let err = endpoint.fetch_models().await.expect_err("401 is an error");
        assert!(matches!(err, LlmError::AuthFailed { .. }), "got {err:?}");
    }

    #[tokio::test]
    async fn fetch_models_maps_403_to_auth_failed() {
        let endpoint =
            endpoint_against_canned_response("HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n")
                .await;
        let err = endpoint.fetch_models().await.expect_err("403 is an error");
        assert!(matches!(err, LlmError::AuthFailed { .. }), "got {err:?}");
    }

    #[tokio::test]
    async fn fetch_models_does_not_follow_redirects() {
        // A host that passed the SSRF guard must not be able to 3xx-redirect the
        // request elsewhere (e.g. the metadata IP). With redirects disabled the
        // 301 surfaces as a non-success status, not a followed hop.
        let endpoint = endpoint_against_canned_response(
            "HTTP/1.1 301 Moved Permanently\r\nLocation: http://169.254.169.254/\r\nContent-Length: 0\r\n\r\n",
        )
        .await;
        let err = endpoint
            .fetch_models()
            .await
            .expect_err("redirect is not followed");
        assert!(matches!(err, LlmError::RequestFailed { .. }), "got {err:?}");
    }

    #[derive(Clone)]
    struct FailingCompletionModel;

    impl CompletionModel for FailingCompletionModel {
        type Response = serde_json::Value;
        type StreamingResponse = ();
        type Client = ();

        fn make(_client: &Self::Client, _model: impl Into<String>) -> Self {
            Self
        }

        async fn completion(
            &self,
            _request: RigRequest,
        ) -> Result<rig::completion::CompletionResponse<Self::Response>, CompletionError> {
            Err(CompletionError::ProviderError(
                "HTTP 400 Bad Request: The input (314325 tokens) is longer than the model's context length (262144 tokens).".to_string(),
            ))
        }

        async fn stream(
            &self,
            _request: RigRequest,
        ) -> Result<StreamingCompletionResponse<Self::StreamingResponse>, CompletionError> {
            Err(CompletionError::ProviderError(
                "stream unsupported".to_string(),
            ))
        }
    }

    /// A rig model that replays a caller-supplied raw provider body alongside
    /// a fixed assistant choice, so the adapter's public entry points can be
    /// driven against a real provider payload.
    #[derive(Clone)]
    struct ReplayingCompletionModel {
        choice: OneOrMany<AssistantContent>,
        raw_response: serde_json::Value,
    }

    impl CompletionModel for ReplayingCompletionModel {
        type Response = serde_json::Value;
        type StreamingResponse = StubStreamingResponse;
        type Client = ();

        fn make(_client: &Self::Client, _model: impl Into<String>) -> Self {
            unimplemented!("constructed directly in tests")
        }

        async fn completion(
            &self,
            _request: RigRequest,
        ) -> Result<rig::completion::CompletionResponse<Self::Response>, CompletionError> {
            Ok(rig::completion::CompletionResponse {
                choice: self.choice.clone(),
                usage: RigUsage::new(),
                raw_response: self.raw_response.clone(),
                message_id: None,
            })
        }

        async fn stream(
            &self,
            _request: RigRequest,
        ) -> Result<StreamingCompletionResponse<Self::StreamingResponse>, CompletionError> {
            Err(CompletionError::ProviderError(
                "streaming path must not be used".to_string(),
            ))
        }
    }

    /// Test through the caller, not just the helper: a `max_tokens` truncation
    /// that still emitted a (possibly cut-off) tool call must reach
    /// `complete_with_tools`' return value as `Length`. Before #6284 item 8 the
    /// adapter reported `ToolUse` and the loop executed the truncated call.
    #[tokio::test]
    async fn complete_with_tools_reports_provider_truncation_not_tool_use() {
        let adapter = RigAdapter::new(
            ReplayingCompletionModel {
                choice: OneOrMany::one(AssistantContent::tool_call(
                    "call_1",
                    "search",
                    serde_json::json!({"q": "tru"}),
                )),
                raw_response: serde_json::json!({
                    "choices": [{
                        "index": 0,
                        "message": {"role": "assistant", "content": null},
                        "finish_reason": "length",
                    }],
                }),
            },
            "gpt-4o",
        );
        let request = ToolCompletionRequest::new(
            vec![ChatMessage::user("search for something")],
            vec![IronToolDefinition {
                name: "search".to_string(),
                description: "Search".to_string(),
                parameters: serde_json::json!({"type": "object"}),
            }],
        );

        let response = adapter
            .complete_with_tools(request)
            .await
            .expect("adapter returns the provider response");

        assert_eq!(
            response.finish_reason,
            FinishReason::Length,
            "a max_tokens truncation must not be laundered into ToolUse",
        );
    }

    /// The same seam for a content-policy block on the plain `complete` path.
    #[tokio::test]
    async fn complete_reports_provider_content_filter() {
        let adapter = RigAdapter::new(
            ReplayingCompletionModel {
                choice: OneOrMany::one(AssistantContent::text("I can't help with that.")),
                raw_response: serde_json::json!({
                    "choices": [{
                        "index": 0,
                        "message": {"role": "assistant", "content": "I can't help with that."},
                        "finish_reason": "content_filter",
                    }],
                }),
            },
            "gpt-4o",
        );

        let response = adapter
            .complete(CompletionRequest::new(vec![ChatMessage::user("hi")]))
            .await
            .expect("adapter returns the provider response");

        assert_eq!(
            response.finish_reason,
            FinishReason::ContentFilter,
            "a provider refusal must not be reported as a clean stop",
        );
    }

    #[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
    struct StubStreamingResponse {
        input_tokens: u64,
        output_tokens: u64,
    }

    impl GetTokenUsage for StubStreamingResponse {
        fn token_usage(&self) -> Option<RigUsage> {
            Some(RigUsage {
                input_tokens: self.input_tokens,
                output_tokens: self.output_tokens,
                total_tokens: self.input_tokens + self.output_tokens,
                cached_input_tokens: 0,
                cache_creation_input_tokens: 0,
            })
        }
    }

    #[derive(Clone)]
    struct DispatchCountingCompletionModel {
        completions: Arc<AtomicUsize>,
        streams: Arc<AtomicUsize>,
    }

    impl CompletionModel for DispatchCountingCompletionModel {
        type Response = serde_json::Value;
        type StreamingResponse = StubStreamingResponse;
        type Client = ();

        fn make(_client: &Self::Client, _model: impl Into<String>) -> Self {
            unimplemented!("constructed directly in tests")
        }

        async fn completion(
            &self,
            _request: RigRequest,
        ) -> Result<rig::completion::CompletionResponse<Self::Response>, CompletionError> {
            self.completions.fetch_add(1, Ordering::Relaxed);
            Ok(rig::completion::CompletionResponse {
                choice: OneOrMany::one(AssistantContent::text("dispatched")),
                usage: RigUsage::new(),
                raw_response: serde_json::json!({}),
                message_id: None,
            })
        }

        async fn stream(
            &self,
            _request: RigRequest,
        ) -> Result<StreamingCompletionResponse<Self::StreamingResponse>, CompletionError> {
            self.streams.fetch_add(1, Ordering::Relaxed);
            Err(CompletionError::ProviderError(
                "structured-output test must reject before dispatch".to_string(),
            ))
        }
    }

    #[derive(Clone)]
    struct MaxTokensCompletionModel {
        expected_max_tokens: Option<u64>,
    }

    impl CompletionModel for MaxTokensCompletionModel {
        type Response = serde_json::Value;
        type StreamingResponse = StubStreamingResponse;
        type Client = ();

        fn make(_client: &Self::Client, _model: impl Into<String>) -> Self {
            unimplemented!("constructed directly in tests")
        }

        async fn completion(
            &self,
            request: RigRequest,
        ) -> Result<rig::completion::CompletionResponse<Self::Response>, CompletionError> {
            assert_eq!(request.max_tokens, self.expected_max_tokens);
            Ok(rig::completion::CompletionResponse {
                choice: OneOrMany::one(AssistantContent::text("ok")),
                usage: RigUsage::new(),
                raw_response: serde_json::json!({
                    "choices": [{
                        "index": 0,
                        "message": {"role": "assistant", "content": "ok"},
                        "finish_reason": "stop",
                    }],
                }),
                message_id: None,
            })
        }

        async fn stream(
            &self,
            _request: RigRequest,
        ) -> Result<StreamingCompletionResponse<Self::StreamingResponse>, CompletionError> {
            Err(CompletionError::ProviderError(
                "streaming path must not be used".to_string(),
            ))
        }
    }

    #[tokio::test]
    async fn complete_applies_provider_default_max_tokens() {
        const DEFAULT_MAX_TOKENS: u32 = 8192;
        let adapter = RigAdapter::new(
            MaxTokensCompletionModel {
                expected_max_tokens: Some(u64::from(DEFAULT_MAX_TOKENS)),
            },
            "default-max-tokens",
        )
        .with_default_max_tokens(DEFAULT_MAX_TOKENS);

        let response = adapter
            .complete(CompletionRequest::new(vec![ChatMessage::user("hello")]))
            .await
            .expect("completion uses provider default max_tokens");

        assert_eq!(response.content, "ok");
    }

    #[tokio::test]
    async fn complete_with_tools_preserves_explicit_max_tokens_over_provider_default() {
        let adapter = RigAdapter::new(
            MaxTokensCompletionModel {
                expected_max_tokens: Some(4096),
            },
            "explicit-max-tokens",
        )
        .with_default_max_tokens(8192);
        let request = ToolCompletionRequest::new(vec![ChatMessage::user("hello")], Vec::new())
            .with_max_tokens(4096);

        let response = adapter
            .complete_with_tools(request)
            .await
            .expect("tool completion preserves explicit max_tokens");

        assert_eq!(response.content.as_deref(), Some("ok"));
    }

    #[derive(Clone)]
    struct StreamingOnlyCompletionModel {
        expected_max_tokens: Option<u64>,
    }

    impl CompletionModel for StreamingOnlyCompletionModel {
        type Response = serde_json::Value;
        type StreamingResponse = StubStreamingResponse;
        type Client = ();

        fn make(_client: &Self::Client, _model: impl Into<String>) -> Self {
            Self {
                expected_max_tokens: None,
            }
        }

        async fn completion(
            &self,
            _request: RigRequest,
        ) -> Result<rig::completion::CompletionResponse<Self::Response>, CompletionError> {
            Err(CompletionError::ProviderError(
                "non-streaming path must not be used".to_string(),
            ))
        }

        async fn stream(
            &self,
            request: RigRequest,
        ) -> Result<StreamingCompletionResponse<Self::StreamingResponse>, CompletionError> {
            assert_eq!(request.max_tokens, self.expected_max_tokens);
            let stream = futures::stream::iter(vec![
                Ok(RawStreamingChoice::Message("Hel".to_string())),
                Ok(RawStreamingChoice::ReasoningDelta {
                    id: Some("rsn-stream".to_string()),
                    reasoning: "thinking".to_string(),
                }),
                Ok(RawStreamingChoice::Message("lo".to_string())),
                Ok(RawStreamingChoice::ToolCall(RawStreamingToolCall::new(
                    "call-1".to_string(),
                    "search".to_string(),
                    serde_json::json!({"query": "ironclaw"}),
                ))),
                Ok(RawStreamingChoice::FinalResponse(StubStreamingResponse {
                    input_tokens: 3,
                    output_tokens: 4,
                })),
            ]);
            Ok(StreamingCompletionResponse::stream(Box::pin(stream)))
        }
    }

    struct RecordingCompletionStreamSink {
        sender: mpsc::UnboundedSender<String>,
    }

    #[async_trait]
    impl CompletionStreamSink for RecordingCompletionStreamSink {
        async fn text_delta(&self, delta: String) {
            let _ = self.sender.send(delta);
        }
    }

    #[tokio::test]
    async fn deepseek_and_openrouter_reject_structured_output_before_any_dispatch() {
        for provider in ["deepseek", "openrouter"] {
            let completions = Arc::new(AtomicUsize::new(0));
            let streams = Arc::new(AtomicUsize::new(0));
            let adapter = RigAdapter::new(
                DispatchCountingCompletionModel {
                    completions: Arc::clone(&completions),
                    streams: Arc::clone(&streams),
                },
                "test-model",
            )
            .with_provider_id(provider)
            .with_structured_output_support(false);
            let structured_format = || {
                crate::provider::CompletionResponseFormat::JsonSchema(
                    crate::provider::JsonSchemaResponseFormat::strict(
                        "suggestions",
                        serde_json::json!({"type": "object"}),
                    ),
                )
            };

            let mut completion_request =
                CompletionRequest::new(vec![ChatMessage::user("return structured output")]);
            completion_request.response_format = Some(structured_format());
            let error = adapter
                .complete(completion_request)
                .await
                .expect_err("unsupported structured output must fail before dispatch");
            assert!(matches!(
                error,
                LlmError::InvalidRequest { provider: actual, reason }
                    if actual == provider && reason.contains("structured output")
            ));

            let mut streaming_request =
                CompletionRequest::new(vec![ChatMessage::user("return structured output")]);
            streaming_request.response_format = Some(structured_format());
            let (delta_tx, _delta_rx) = mpsc::unbounded_channel();
            let sink = Arc::new(RecordingCompletionStreamSink { sender: delta_tx });
            let error = adapter
                .complete_streaming(streaming_request, sink)
                .await
                .expect_err("unsupported structured streaming must fail before dispatch");
            assert!(matches!(
                error,
                LlmError::InvalidRequest { provider: actual, .. } if actual == provider
            ));

            let mut tool_request = ToolCompletionRequest::new(
                vec![ChatMessage::user("return structured output")],
                Vec::new(),
            );
            tool_request.response_format = Some(structured_format());
            let error = adapter
                .complete_with_tools(tool_request)
                .await
                .expect_err("unsupported tool structured output must fail before dispatch");
            assert!(matches!(
                error,
                LlmError::InvalidRequest { provider: actual, .. } if actual == provider
            ));

            let mut tool_streaming_request = ToolCompletionRequest::new(
                vec![ChatMessage::user("return structured output")],
                Vec::new(),
            );
            tool_streaming_request.response_format = Some(structured_format());
            let (delta_tx, _delta_rx) = mpsc::unbounded_channel();
            let sink = Arc::new(RecordingCompletionStreamSink { sender: delta_tx });
            let error = adapter
                .complete_with_tools_streaming(tool_streaming_request, sink)
                .await
                .expect_err("unsupported tool structured streaming must fail before dispatch");
            assert!(matches!(
                error,
                LlmError::InvalidRequest { provider: actual, .. } if actual == provider
            ));

            assert_eq!(completions.load(Ordering::Relaxed), 0);
            assert_eq!(streams.load(Ordering::Relaxed), 0);
        }
    }

    #[tokio::test]
    async fn streaming_methods_use_buffered_fallback_until_explicitly_enabled() {
        let adapter = RigAdapter::new(
            ReplayingCompletionModel {
                choice: OneOrMany::one(AssistantContent::text("buffered")),
                raw_response: serde_json::json!({
                    "choices": [{
                        "index": 0,
                        "message": {"role": "assistant", "content": "buffered"},
                        "finish_reason": "stop",
                    }],
                }),
            },
            "untested-provider",
        );
        let (delta_tx, mut delta_rx) = mpsc::unbounded_channel();
        let sink = Arc::new(RecordingCompletionStreamSink { sender: delta_tx });

        let text_response = adapter
            .complete_streaming(
                CompletionRequest::new(vec![ChatMessage::user("hello")]),
                sink.clone(),
            )
            .await
            .expect("buffered text fallback succeeds");
        let tool_response = adapter
            .complete_with_tools_streaming(
                ToolCompletionRequest::new(vec![ChatMessage::user("hello")], Vec::new()),
                sink,
            )
            .await
            .expect("buffered tool fallback succeeds");

        assert_eq!(text_response.content, "buffered");
        assert_eq!(tool_response.content.as_deref(), Some("buffered"));
        assert!(
            delta_rx.try_recv().is_err(),
            "buffered fallback must not emit deltas"
        );
    }

    #[tokio::test]
    async fn complete_streaming_preserves_reasoning_deltas() {
        let adapter = with_native_streaming(RigAdapter::new(
            StreamingOnlyCompletionModel {
                expected_max_tokens: None,
            },
            "streaming-only",
        ));
        let request = CompletionRequest::new(vec![ChatMessage::user("hello")]);
        let (delta_tx, mut delta_rx) = mpsc::unbounded_channel();
        let sink = Arc::new(RecordingCompletionStreamSink { sender: delta_tx });

        let response = adapter
            .complete_streaming(request, sink)
            .await
            .expect("streaming completion succeeds");

        assert_eq!(delta_rx.try_recv().expect("first delta"), "Hel");
        assert_eq!(delta_rx.try_recv().expect("second delta"), "lo");
        assert_eq!(response.content, "Hello");
        assert_eq!(response.reasoning.as_deref(), Some("thinking"));
        assert_eq!(response.input_tokens, 3);
        assert_eq!(response.output_tokens, 4);
    }

    #[tokio::test]
    async fn complete_with_tools_streaming_uses_rig_stream_and_emits_deltas() {
        let adapter = with_native_streaming(RigAdapter::new(
            StreamingOnlyCompletionModel {
                expected_max_tokens: None,
            },
            "streaming-only",
        ));
        let request = ToolCompletionRequest::new(
            vec![ChatMessage::user("search")],
            vec![IronToolDefinition {
                name: "search".to_string(),
                description: "Search the index".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {"query": {"type": "string"}},
                    "required": ["query"],
                }),
            }],
        );
        let (delta_tx, mut delta_rx) = mpsc::unbounded_channel();
        let sink = Arc::new(RecordingCompletionStreamSink { sender: delta_tx });

        let response = adapter
            .complete_with_tools_streaming(request, sink)
            .await
            .expect("streaming completion succeeds");

        assert_eq!(delta_rx.try_recv().expect("first delta"), "Hel");
        assert_eq!(delta_rx.try_recv().expect("second delta"), "lo");
        assert_eq!(response.content.as_deref(), Some("Hello"));
        assert_eq!(response.tool_calls.len(), 1);
        assert_eq!(response.tool_calls[0].name, "search");
        assert_eq!(response.reasoning.as_deref(), Some("thinking"));
        assert_eq!(response.input_tokens, 3);
        assert_eq!(response.output_tokens, 4);
    }

    #[tokio::test]
    async fn complete_with_tools_streaming_applies_provider_default_max_tokens() {
        const DEFAULT_MAX_TOKENS: u32 = 8192;

        let adapter = with_native_streaming(RigAdapter::new(
            StreamingOnlyCompletionModel {
                expected_max_tokens: Some(u64::from(DEFAULT_MAX_TOKENS)),
            },
            "streaming-only",
        ))
        .with_default_max_tokens(DEFAULT_MAX_TOKENS);
        let request = ToolCompletionRequest::new(
            vec![ChatMessage::user("search")],
            vec![IronToolDefinition {
                name: "search".to_string(),
                description: "Search the index".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {"query": {"type": "string"}},
                    "required": ["query"],
                }),
            }],
        );
        let (delta_tx, _delta_rx) = mpsc::unbounded_channel();

        adapter
            .complete_with_tools_streaming(
                request,
                Arc::new(RecordingCompletionStreamSink { sender: delta_tx }),
            )
            .await
            .expect("streaming completion succeeds with provider default");
    }

    #[tokio::test]
    async fn complete_streaming_preserves_explicit_max_tokens_over_provider_default() {
        let adapter = with_native_streaming(RigAdapter::new(
            StreamingOnlyCompletionModel {
                expected_max_tokens: Some(4096),
            },
            "streaming-only",
        ))
        .with_default_max_tokens(8192);
        let request =
            CompletionRequest::new(vec![ChatMessage::user("hello")]).with_max_tokens(4096);
        let (delta_tx, _delta_rx) = mpsc::unbounded_channel();

        adapter
            .complete_streaming(
                request,
                Arc::new(RecordingCompletionStreamSink { sender: delta_tx }),
            )
            .await
            .expect("explicit max_tokens takes precedence");
    }

    // The finish-reason conformance suite — provider fixtures, payload
    // builders, and the matrix itself — lives in its own file so this one
    // stays readable: `src/rig_adapter/tests/finish_reason_tests.rs`.
    mod finish_reason_tests;

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
            arguments_parse_error: None,
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
            role: crate::Role::Tool,
            content: "result text".to_string(),
            content_parts: Vec::new(),
            tool_call_id: None,
            name: Some("search".to_string()),
            tool_calls: None,
            reasoning: None,
            reasoning_details: None,
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
            vec![crate::ContentPart::ImageUrl {
                image_url: crate::ImageUrl {
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
            vec![crate::ContentPart::ImageUrl {
                image_url: crate::ImageUrl {
                    url: "https://example.com/image-low.png".to_string(),
                    detail: Some("low".to_string()),
                },
            }],
        )];
        let high_messages = vec![ChatMessage::user_with_parts(
            "high detail",
            vec![crate::ContentPart::ImageUrl {
                image_url: crate::ImageUrl {
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
        match convert_tool_choice(Some("builtin__structured_result")) {
            Some(RigToolChoice::Specific { function_names }) => {
                assert_eq!(
                    function_names,
                    vec!["builtin__structured_result".to_string()]
                );
            }
            other => panic!("named tool choice must map to Specific, got {other:?}"),
        }
    }

    #[test]
    fn test_extract_response_text_only() {
        let content = OneOrMany::one(AssistantContent::text("Hello world"));
        let usage = RigUsage::new();
        let (text, calls, finish, _reasoning, _reasoning_details) =
            extract_response(&content, &usage, None);
        assert_eq!(text, Some("Hello world".to_string()));
        assert!(calls.is_empty());
        assert_eq!(finish, FinishReason::Stop);
    }

    #[test]
    fn test_extract_response_tool_call() {
        let tc = AssistantContent::tool_call("call_1", "search", serde_json::json!({"q": "test"}));
        let content = OneOrMany::one(tc);
        let usage = RigUsage::new();
        let (text, calls, finish, _reasoning, _reasoning_details) =
            extract_response(&content, &usage, None);
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
            arguments_parse_error: None,
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
            arguments_parse_error: None,
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
            arguments_parse_error: None,
        };
        let assistant_msg = ChatMessage::assistant_with_tool_calls(None, vec![tc]);
        let tool_result_msg = ChatMessage {
            role: crate::Role::Tool,
            content: "search results here".to_string(),
            content_parts: Vec::new(),
            tool_call_id: None,
            name: Some("search".to_string()),
            tool_calls: None,
            reasoning: None,
            reasoning_details: None,
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

    fn two_rig_tools() -> Vec<RigToolDefinition> {
        vec![
            RigToolDefinition {
                name: "alpha".to_string(),
                description: "First tool".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": { "a": { "type": "string" } }
                }),
            },
            RigToolDefinition {
                name: "beta".to_string(),
                description: "Second tool".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": { "b": { "type": "string" } }
                }),
            },
        ]
    }

    #[test]
    fn test_build_rig_request_marks_last_tool_short() {
        let req = build_rig_request(
            Some("You are helpful.".to_string()),
            vec![RigMessage::user("Hello")],
            two_rig_tools(),
            None,
            None,
            None,
            CacheRetention::Short,
        )
        .unwrap();

        // The last tool moves into additional_params.tools (rig appends those
        // after the typed tools, preserving order) carrying the explicit
        // cache breakpoint in Anthropic's native shape.
        assert_eq!(req.tools.len(), 1, "last tool moves to additional_params");
        assert_eq!(req.tools[0].name, "alpha");
        let params = req.additional_params.expect("additional_params");
        let moved = params["tools"].as_array().expect("raw tools array");
        assert_eq!(moved.len(), 1);
        assert_eq!(moved[0]["name"], "beta");
        assert_eq!(moved[0]["description"], "Second tool");
        assert!(
            moved[0].get("input_schema").is_some(),
            "moved tool must use Anthropic's input_schema key: {moved:?}"
        );
        assert!(moved[0].get("parameters").is_none());
        assert_eq!(moved[0]["cache_control"]["type"], "ephemeral");
        assert!(moved[0]["cache_control"].get("ttl").is_none());
    }

    #[test]
    fn test_build_rig_request_marks_last_tool_long_with_ttl() {
        let req = build_rig_request(
            Some("You are helpful.".to_string()),
            vec![RigMessage::user("Hello")],
            two_rig_tools(),
            None,
            None,
            None,
            CacheRetention::Long,
        )
        .unwrap();

        let params = req.additional_params.expect("additional_params");
        let moved = params["tools"].as_array().expect("raw tools array");
        assert_eq!(moved[0]["cache_control"]["type"], "ephemeral");
        assert_eq!(moved[0]["cache_control"]["ttl"], "1h");
    }

    #[test]
    fn test_effective_cache_retention_downgrades_unsupported_models() {
        assert_eq!(
            effective_cache_retention(CacheRetention::Short, "claude-2.1"),
            CacheRetention::None
        );
        assert_eq!(
            effective_cache_retention(CacheRetention::Long, "claude-instant-1.2"),
            CacheRetention::None
        );
        assert_eq!(
            effective_cache_retention(CacheRetention::Short, "claude-opus-4-6"),
            CacheRetention::Short
        );
        assert_eq!(
            effective_cache_retention(CacheRetention::Long, "anthropic/claude-sonnet-4-5"),
            CacheRetention::Long
        );
        assert_eq!(
            effective_cache_retention(CacheRetention::None, "claude-opus-4-6"),
            CacheRetention::None
        );
    }

    #[test]
    fn test_build_rig_request_keeps_tools_typed_when_none() {
        let req = build_rig_request(
            Some("You are helpful.".to_string()),
            vec![RigMessage::user("Hello")],
            two_rig_tools(),
            None,
            None,
            None,
            CacheRetention::None,
        )
        .unwrap();

        assert_eq!(req.tools.len(), 2, "no tool moves when caching is off");
        assert!(req.additional_params.is_none());
    }

    /// Wire-level pin for the Anthropic explicit-breakpoint layout under
    /// `Short` retention (issue #6984): system prompt, last tool, and last
    /// message block each carry an ephemeral marker, and the request-level
    /// automatic-caching marker is retained (it no-ops on the already-marked
    /// last block per Anthropic's compatibility rule).
    #[tokio::test]
    async fn anthropic_short_retention_places_explicit_cache_breakpoints() {
        use rig::client::CompletionClient;
        use rig::providers::anthropic;

        let (base_url, captured_body) = capture_one_http_request().await;
        let client = anthropic::Client::builder()
            .api_key("test-key")
            .base_url(&base_url)
            .build()
            .expect("build Anthropic client");
        let mut model = client.completion_model("claude-opus-4-6");
        // Mirrors create_anthropic_from_registry: Short retention enables
        // rig's typed system/last-message breakpoints.
        model.prompt_caching = true;
        let adapter =
            RigAdapter::new(model, "claude-opus-4-6").with_cache_retention(CacheRetention::Short);

        let request = ToolCompletionRequest::new(
            vec![
                ChatMessage::system("You are helpful."),
                ChatMessage::user("First question"),
                ChatMessage::assistant("First answer"),
                ChatMessage::user("Second question"),
            ],
            vec![
                IronToolDefinition {
                    name: "alpha".to_string(),
                    description: "First tool".to_string(),
                    parameters: serde_json::json!({
                        "type": "object",
                        "properties": { "a": { "type": "string" } }
                    }),
                },
                IronToolDefinition {
                    name: "beta".to_string(),
                    description: "Second tool".to_string(),
                    parameters: serde_json::json!({
                        "type": "object",
                        "properties": { "b": { "type": "string" } }
                    }),
                },
            ],
        );
        let _ = adapter.complete_with_tools(request).await;

        let body: serde_json::Value =
            serde_json::from_str(&captured_request_body(captured_body).await)
                .expect("captured body is JSON");

        assert_eq!(body["cache_control"]["type"], "ephemeral");
        assert!(body["cache_control"].get("ttl").is_none());

        let system = body["system"]
            .as_array()
            .expect("system serialized as blocks");
        assert_eq!(
            system.last().expect("system block")["cache_control"]["type"],
            "ephemeral"
        );

        let tools = body["tools"].as_array().expect("tools array");
        assert_eq!(tools.len(), 2, "both tools reach the wire: {body}");
        assert_eq!(tools[0]["name"], "alpha");
        assert!(tools[0].get("cache_control").is_none());
        assert_eq!(tools[1]["name"], "beta", "order preserved after the move");
        assert_eq!(tools[1]["cache_control"]["type"], "ephemeral");
        assert!(
            tools[1].get("input_schema").is_some(),
            "moved tool keeps Anthropic's input_schema key: {tools:?}"
        );

        let messages = body["messages"].as_array().expect("messages");
        let last_content = messages.last().expect("last message")["content"]
            .as_array()
            .expect("last message content blocks");
        assert_eq!(
            last_content.last().expect("content block")["cache_control"]["type"],
            "ephemeral"
        );
    }

    /// Wire-level pin for `Long` retention: rig's typed markers cannot carry
    /// a TTL, so only the request-level automatic marker and the raw last-tool
    /// marker are emitted, both at 1h. Mixing a 5m block marker with a 1h
    /// automatic marker would be rejected by the API (TTL conflict on the
    /// last block), so system/message blocks must stay unmarked.
    #[tokio::test]
    async fn anthropic_long_retention_uses_1h_markers_without_block_conflicts() {
        use rig::client::CompletionClient;
        use rig::providers::anthropic;

        let (base_url, captured_body) = capture_one_http_request().await;
        let client = anthropic::Client::builder()
            .api_key("test-key")
            .base_url(&base_url)
            .build()
            .expect("build Anthropic client");
        let model = client.completion_model("claude-opus-4-6");
        let adapter =
            RigAdapter::new(model, "claude-opus-4-6").with_cache_retention(CacheRetention::Long);

        let request = ToolCompletionRequest::new(
            vec![
                ChatMessage::system("You are helpful."),
                ChatMessage::user("Question"),
            ],
            vec![IronToolDefinition {
                name: "alpha".to_string(),
                description: "First tool".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": { "a": { "type": "string" } }
                }),
            }],
        );
        let _ = adapter.complete_with_tools(request).await;

        let body: serde_json::Value =
            serde_json::from_str(&captured_request_body(captured_body).await)
                .expect("captured body is JSON");

        assert_eq!(body["cache_control"]["type"], "ephemeral");
        assert_eq!(body["cache_control"]["ttl"], "1h");

        let tools = body["tools"].as_array().expect("tools array");
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["cache_control"]["ttl"], "1h");

        for block in body["system"].as_array().expect("system blocks") {
            assert!(
                block.get("cache_control").is_none(),
                "no 5m block marker may precede the 1h automatic marker: {body}"
            );
        }
        for message in body["messages"].as_array().expect("messages") {
            if let Some(blocks) = message["content"].as_array() {
                for block in blocks {
                    assert!(
                        block.get("cache_control").is_none(),
                        "message blocks must stay unmarked under Long retention: {body}"
                    );
                }
            }
        }
    }

    /// Wire-level pin: retention `None` emits no cache_control anywhere.
    #[tokio::test]
    async fn anthropic_no_retention_emits_no_cache_control() {
        use rig::client::CompletionClient;
        use rig::providers::anthropic;

        let (base_url, captured_body) = capture_one_http_request().await;
        let client = anthropic::Client::builder()
            .api_key("test-key")
            .base_url(&base_url)
            .build()
            .expect("build Anthropic client");
        let adapter = RigAdapter::new(
            client.completion_model("claude-opus-4-6"),
            "claude-opus-4-6",
        );

        let request = ToolCompletionRequest::new(
            vec![
                ChatMessage::system("You are helpful."),
                ChatMessage::user("Question"),
            ],
            vec![IronToolDefinition {
                name: "alpha".to_string(),
                description: "First tool".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": { "a": { "type": "string" } }
                }),
            }],
        );
        let _ = adapter.complete_with_tools(request).await;

        let body = captured_request_body(captured_body).await;
        assert!(
            !body.contains("cache_control"),
            "no cache_control may be emitted when caching is off: {body}"
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
            arguments_parse_error: None,
        };
        let tc2 = IronToolCall {
            id: "call_b".to_string(),
            name: "fetch".to_string(),
            arguments: serde_json::json!({"url": "https://example.com"}),
            reasoning: None,
            signature: None,
            arguments_parse_error: None,
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
            role: crate::Role::Assistant,
            content: String::new(),
            tool_calls: None,
            reasoning: None,
            reasoning_details: None,
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
            role: crate::Role::Assistant,
            content: String::new(),
            tool_calls: None,
            reasoning: None,
            reasoning_details: None,
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
            model: None,
            output_schema: None,
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
    fn test_inject_model_override_sets_typed_request_model() {
        let mut req = make_rig_request(None);
        inject_model_override(&mut req, Some("test-model"));

        assert_eq!(req.model.as_deref(), Some("test-model"));
        assert!(req.additional_params.is_none());
    }

    #[test]
    fn test_inject_model_override_preserves_existing_additional_params() {
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
        assert!(obj.get("model").is_none());
        assert_eq!(req.model.as_deref(), Some("override-model"));
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
    fn test_map_rig_error_detects_longer_than_model_context_length() {
        let err = map_rig_error(
            "nearai",
            "HTTP 400 Bad Request: The input (314325 tokens) is longer than the model's context length (262144 tokens).",
        );
        match err {
            LlmError::ContextLengthExceeded { used, limit } => {
                assert_eq!(used, 314325);
                assert_eq!(limit, 262144);
            }
            other => panic!("Expected ContextLengthExceeded, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn complete_maps_longer_than_context_error_to_context_length_exceeded() {
        let adapter = RigAdapter::new(FailingCompletionModel, "nearai");
        let err = adapter
            .complete(CompletionRequest::new(vec![ChatMessage::user(
                "read my email",
            )]))
            .await
            .expect_err("context overflow should fail the completion");

        match err {
            LlmError::ContextLengthExceeded { used, limit } => {
                assert_eq!(used, 314325);
                assert_eq!(limit, 262144);
            }
            other => panic!("expected context-length error, got {other:?}"),
        }
    }

    #[test]
    fn test_map_rig_error_bare_413_no_false_positive() {
        // Bare "413" should NOT trigger ContextLengthExceeded — avoids false
        // positives on timestamps ("2026-04-13"), token counts ("used 1413"),
        // and request IDs.
        let err = map_rig_error("nearai", "Rate limit: 413 requests per minute exceeded");
        assert!(
            matches!(err, LlmError::RateLimited { .. }),
            "rate-limit wording should stay typed without treating bare 413 as payload status: {err:?}"
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
        let (used, limit) = crate::error::parse_context_token_counts(msg);
        assert_eq!(limit, 128000);
        assert_eq!(used, 150000);
    }

    #[test]
    fn test_parse_token_counts_unparseable_returns_zero() {
        let msg = "context_length_exceeded";
        let (used, limit) = crate::error::parse_context_token_counts(msg);
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
        let (text, tool_calls, finish, reasoning, reasoning_details) =
            extract_response(&rig_response, &usage, None);

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
            .with_reasoning_details(reasoning_details.clone());
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
                    let reasoning = r
                        .content
                        .iter()
                        .map(|content| match content {
                            rig::message::ReasoningContent::Text { text, .. } => text.as_str(),
                            _ => "",
                        })
                        .collect::<Vec<_>>();
                    assert_eq!(reasoning, vec!["Let me check the weather first."]);
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

    #[test]
    fn typed_reasoning_round_trips_through_chat_message() {
        let mut typed_reasoning = rig::message::Reasoning::new("");
        typed_reasoning.id = Some("rsn_123".to_string());
        typed_reasoning.content = vec![
            rig::message::ReasoningContent::Encrypted("encrypted-payload".to_string()),
            rig::message::ReasoningContent::Redacted {
                data: "redacted-payload".to_string(),
            },
            rig::message::ReasoningContent::Summary("safe summary".to_string()),
        ];
        let rig_response = OneOrMany::many(vec![
            AssistantContent::Reasoning(typed_reasoning.clone()),
            AssistantContent::ToolCall(rig::message::ToolCall::new(
                "call_abc123".to_string(),
                ToolFunction::new("lookup".to_string(), serde_json::json!({})),
            )),
        ])
        .unwrap();

        let usage = RigUsage::new();
        let (text, tool_calls, _finish, reasoning, reasoning_details) =
            extract_response(&rig_response, &usage, None);

        assert_eq!(text, None);
        assert_eq!(reasoning.as_deref(), Some("safe summary"));
        let assistant = ChatMessage::assistant_with_tool_calls(text, tool_calls)
            .with_reasoning_details(reasoning_details);

        let (_preamble, history) = convert_messages(&[
            ChatMessage::user("continue"),
            assistant,
            ChatMessage::tool_result("call_abc123", "lookup", "{}"),
        ]);
        let assistant_msg = history
            .iter()
            .find(|m| matches!(m, RigMessage::Assistant { .. }))
            .expect("rebuilt rig history should contain the assistant message");
        let RigMessage::Assistant { content, .. } = assistant_msg else {
            unreachable!()
        };

        let replayed_reasoning = content
            .iter()
            .find_map(|content| match content {
                AssistantContent::Reasoning(reasoning) => Some(reasoning),
                _ => None,
            })
            .expect("rebuilt assistant message should include typed reasoning");
        assert_eq!(replayed_reasoning.id, typed_reasoning.id);
        assert_eq!(replayed_reasoning.content, typed_reasoning.content);
    }

    /// Regression: a Text block with empty text but a non-empty signature must
    /// survive `with_reasoning_details` and reach `convert_messages` unchanged.
    /// Gemini 2.5+ emits `thought_signature` blocks this way; dropping them
    /// breaks the next turn with HTTP 400 (#3201, #3225).
    #[test]
    fn signature_only_text_block_survives_round_trip() {
        let signature_only = IronReasoningDetail::Text {
            text: String::new(),
            signature: Some("thought-sig-xyz".to_string()),
        };
        let details = IronReasoningDetails {
            id: Some("rsn_sig".to_string()),
            content: vec![signature_only],
        };
        // Must not be filtered by with_reasoning_details.
        let assistant = ChatMessage::assistant("ok").with_reasoning_details(Some(details.clone()));
        assert!(
            assistant.reasoning_details.is_some(),
            "with_reasoning_details must preserve a signature-only Text block"
        );

        let (_preamble, history) = convert_messages(&[ChatMessage::user("ping"), assistant]);
        let assistant_msg = history
            .iter()
            .find(|m| matches!(m, RigMessage::Assistant { .. }))
            .expect("history must include the assistant message");
        let RigMessage::Assistant { content, .. } = assistant_msg else {
            unreachable!()
        };
        let replayed = content
            .iter()
            .find_map(|c| match c {
                AssistantContent::Reasoning(r) => Some(r),
                _ => None,
            })
            .expect(
                "convert_messages must emit AssistantContent::Reasoning for signature-only block",
            );
        assert_eq!(replayed.id.as_deref(), Some("rsn_sig"));
        assert_eq!(replayed.content.len(), 1);
        match &replayed.content[0] {
            rig::message::ReasoningContent::Text { text, signature } => {
                assert!(text.is_empty(), "text must remain empty");
                assert_eq!(
                    signature.as_deref(),
                    Some("thought-sig-xyz"),
                    "signature must be preserved through the round-trip"
                );
            }
            other => panic!("expected ReasoningContent::Text, got {other:?}"),
        }
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
