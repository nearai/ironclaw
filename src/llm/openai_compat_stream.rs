//! Streaming-capable wrapper for OpenAI-compatible Chat Completions providers.
//!
//! Wraps an existing [`LlmProvider`] (typically a [`RigAdapter`]) and overrides
//! [`complete_stream`] / [`complete_with_tools_stream`] with real SSE streaming
//! via a direct HTTP POST to the provider's `/v1/chat/completions` endpoint.
//! All non-streaming methods are forwarded to the inner provider unchanged.
//!
//! This is used for registry providers with protocol `OpenAiCompletions`
//! (OpenRouter, Groq, NVIDIA NIM, etc.) where the upstream endpoint supports
//! the standard `"stream": true` / SSE delta format.

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use eventsource_stream::Eventsource;
use futures::StreamExt;
use rust_decimal::Decimal;

use crate::llm::error::LlmError;
use crate::llm::provider::{
    sanitize_tool_messages, ChatMessage, CompletionRequest, CompletionResponse, FinishReason,
    LlmProvider, Role, ToolCall, ToolCompletionRequest, ToolCompletionResponse, ToolDefinition,
};

/// Wraps any [`LlmProvider`] backed by an OpenAI-compatible endpoint and adds
/// real token-level SSE streaming.
///
/// Non-streaming calls (`complete`, `complete_with_tools`) are delegated to
/// the inner provider. Streaming calls bypass the inner provider and POST
/// directly to `base_url/chat/completions` with `"stream": true`, then parse
/// the OpenAI SSE delta protocol.
pub struct OpenAiCompatStreamingProvider {
    inner: Arc<dyn LlmProvider>,
    client: reqwest::Client,
    base_url: String,
    api_key: String,
    model_name: String,
    /// Raw (key, value) pairs sent as additional HTTP headers on every request.
    extra_headers: Vec<(String, String)>,
    /// Parameter names that this provider does not accept (e.g. `"temperature"`).
    unsupported_params: HashSet<String>,
}

impl OpenAiCompatStreamingProvider {
    pub fn new(
        inner: Arc<dyn LlmProvider>,
        api_key: String,
        base_url: String,
        model_name: String,
        extra_headers: Vec<(String, String)>,
        unsupported_params: HashSet<String>,
    ) -> Result<Self, reqwest::Error> {
        // `connect_timeout` bounds the TCP handshake; `timeout` bounds the
        // total duration of a single streaming request (including reading
        // the full SSE stream) so a hung upstream cannot leak tasks forever.
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(30))
            .timeout(Duration::from_secs(600))
            .build()?;
        Ok(Self {
            inner,
            client,
            base_url,
            api_key,
            model_name,
            extra_headers,
            unsupported_params,
        })
    }

    fn completions_url(&self) -> String {
        // Empty base_url → OpenAI default (matches rig-core behavior).
        // Every provider's base_url already includes the API version prefix
        // (e.g. `/v1`, `/api/v1`, `/v1beta/openai`), so just append the path.
        let base = self.base_url.trim_end_matches('/');
        let base = if base.is_empty() {
            "https://api.openai.com/v1"
        } else {
            base
        };
        format!("{}/chat/completions", base)
    }

    /// POST `body` (with `"stream": true` already set) to the completions
    /// endpoint, parse the SSE delta stream, and return the accumulated result.
    async fn stream_request(
        &self,
        body: serde_json::Value,
        on_chunk: &mut (dyn FnMut(String) + Send),
    ) -> Result<OaiStreamResult, LlmError> {
        let url = self.completions_url();

        let mut builder = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .header("Accept", "text/event-stream");

        for (k, v) in &self.extra_headers {
            builder = builder.header(k.as_str(), v.as_str());
        }

        let response = builder.json(&body).send().await.map_err(|e| {
            LlmError::RequestFailed {
                provider: "openai_compat".to_string(),
                reason: e.to_string(),
            }
        })?;

        let status = response.status();
        if !status.is_success() {
            let code = status.as_u16();
            let retry_after = Some(crate::llm::retry::parse_retry_after(
                response.headers().get("retry-after"),
            ));
            let text = response.text().await.unwrap_or_default();
            let truncated = crate::agent::truncate_for_preview(&text, 512);
            return Err(match code {
                401 | 403 => LlmError::AuthFailed {
                    provider: "openai_compat".to_string(),
                },
                429 => LlmError::RateLimited {
                    provider: "openai_compat".to_string(),
                    retry_after,
                },
                _ => LlmError::RequestFailed {
                    provider: "openai_compat".to_string(),
                    reason: format!("HTTP {}: {}", status, truncated),
                },
            });
        }

        let mut result = OaiStreamResult::default();
        // BTreeMap keyed by tool_call index — OpenAI streams tool_call arguments
        // as incremental string deltas that must be concatenated in order.
        let mut tool_acc: BTreeMap<u32, PartialTool> = BTreeMap::new();

        let stream = response
            .bytes_stream()
            .map(|chunk| chunk.map_err(|e| e.to_string()));
        let mut event_stream = stream.eventsource();

        while let Some(event) = event_stream.next().await {
            let event = event.map_err(|e| LlmError::RequestFailed {
                provider: "openai_compat".to_string(),
                reason: format!("SSE stream error: {}", e),
            })?;

            let data = event.data.trim();
            if data == "[DONE]" {
                break;
            }
            if data.is_empty() {
                continue;
            }

            let parsed: serde_json::Value = match serde_json::from_str(data) {
                Ok(v) => v,
                Err(_) => continue,
            };

            if let Some(choices) = parsed.get("choices").and_then(|c| c.as_array())
                && let Some(choice) = choices.first()
            {
                if let Some(fr) = choice.get("finish_reason").and_then(|v| v.as_str()) {
                    result.finish_reason = match fr {
                        "stop" => FinishReason::Stop,
                        "length" => FinishReason::Length,
                        "tool_calls" => FinishReason::ToolUse,
                        "content_filter" => FinishReason::ContentFilter,
                        _ => result.finish_reason,
                    };
                }

                if let Some(delta) = choice.get("delta") {
                    if let Some(content) = delta.get("content").and_then(|c| c.as_str())
                        && !content.is_empty()
                    {
                        result.content.push_str(content);
                        on_chunk(content.to_string());
                    }

                    if let Some(tcs) = delta.get("tool_calls").and_then(|tc| tc.as_array()) {
                        for tc in tcs {
                            let idx = tc
                                .get("index")
                                .and_then(|v| v.as_u64())
                                .unwrap_or(0) as u32;
                            let entry = tool_acc.entry(idx).or_default();
                            if let Some(id) = tc.get("id").and_then(|v| v.as_str())
                                && !id.is_empty()
                            {
                                entry.id = id.to_string();
                            }
                            if let Some(func) = tc.get("function") {
                                if let Some(name) =
                                    func.get("name").and_then(|v| v.as_str())
                                    && !name.is_empty()
                                {
                                    entry.name = name.to_string();
                                }
                                if let Some(args) =
                                    func.get("arguments").and_then(|v| v.as_str())
                                {
                                    entry.arguments.push_str(args);
                                }
                            }
                        }
                    }
                }
            }

            // Usage is typically in the last chunk when stream_options.include_usage is set.
            if let Some(usage) = parsed.get("usage") {
                result.input_tokens = saturate_u32(
                    usage
                        .get("prompt_tokens")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0),
                );
                result.output_tokens = saturate_u32(
                    usage
                        .get("completion_tokens")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0),
                );
            }
        }

        result.tool_calls = tool_acc
            .into_values()
            .filter(|p| !p.name.is_empty())
            .map(|p| {
                // Prefer parsed JSON; on parse failure preserve the raw string
                // (wrapped as JSON string) so the downstream tool executor can
                // surface the actual malformed payload instead of a silent {}.
                let arguments = match serde_json::from_str::<serde_json::Value>(&p.arguments) {
                    Ok(v) => v,
                    Err(e) => {
                        tracing::warn!(
                            tool = %p.name,
                            error = %e,
                            raw = %p.arguments,
                            "Failed to parse streamed tool_call arguments as JSON; preserving raw text",
                        );
                        serde_json::Value::String(p.arguments.clone())
                    }
                };
                ToolCall {
                    id: p.id,
                    name: p.name,
                    arguments,
                    reasoning: None,
                }
            })
            .collect();

        Ok(result)
    }
}

// ── Internal helpers ─────────────────────────────────────────────────────────

#[derive(Debug, Default)]
struct PartialTool {
    id: String,
    name: String,
    arguments: String,
}

#[derive(Debug)]
struct OaiStreamResult {
    content: String,
    tool_calls: Vec<ToolCall>,
    finish_reason: FinishReason,
    input_tokens: u32,
    output_tokens: u32,
}

impl Default for OaiStreamResult {
    fn default() -> Self {
        Self {
            content: String::new(),
            tool_calls: Vec::new(),
            finish_reason: FinishReason::Unknown,
            input_tokens: 0,
            output_tokens: 0,
        }
    }
}

fn saturate_u32(v: u64) -> u32 {
    v.min(u32::MAX as u64) as u32
}

/// Serialize IronClaw [`ChatMessage`]s into OpenAI Chat Completions JSON format.
fn messages_to_json(messages: &[ChatMessage]) -> Vec<serde_json::Value> {
    messages
        .iter()
        .map(|msg| {
            let role = match msg.role {
                Role::System => "system",
                Role::User => "user",
                Role::Assistant => "assistant",
                Role::Tool => "tool",
            };

            // Multimodal: serialize content as an array of parts; text-only: plain string.
            // Assistant messages with tool_calls and empty text use null content.
            let content: serde_json::Value = if !msg.content_parts.is_empty() {
                let mut parts =
                    vec![serde_json::json!({"type": "text", "text": msg.content})];
                for p in &msg.content_parts {
                    parts.push(serde_json::to_value(p).unwrap_or_default());
                }
                serde_json::Value::Array(parts)
            } else if role == "assistant"
                && msg.tool_calls.is_some()
                && msg.content.is_empty()
            {
                serde_json::Value::Null
            } else {
                serde_json::Value::String(msg.content.clone())
            };

            let mut obj = serde_json::json!({"role": role, "content": content});

            if let Some(id) = &msg.tool_call_id {
                obj["tool_call_id"] = serde_json::json!(id);
            }
            if let Some(name) = &msg.name {
                obj["name"] = serde_json::json!(name);
            }
            if let Some(tcs) = &msg.tool_calls {
                let arr: Vec<serde_json::Value> = tcs
                    .iter()
                    .map(|tc| {
                        serde_json::json!({
                            "id": tc.id,
                            "type": "function",
                            "function": {
                                "name": tc.name,
                                "arguments": tc.arguments.to_string(),
                            },
                        })
                    })
                    .collect();
                obj["tool_calls"] = serde_json::Value::Array(arr);
            }

            obj
        })
        .collect()
}

/// Serialize IronClaw [`ToolDefinition`]s into OpenAI Chat Completions JSON format.
///
/// Schemas are run through [`normalize_schema_strict`] so top-level
/// `oneOf`/`anyOf`/`allOf`/`enum`/`not` (which OpenAI rejects with
/// `invalid_function_parameters`) are flattened into a permissive object
/// envelope. The non-streaming rig-based path normalizes via the same helper
/// inside `RigAdapter::convert_tools`; this keeps the streaming path in sync.
fn tools_to_json(tools: &[ToolDefinition]) -> Vec<serde_json::Value> {
    tools
        .iter()
        .map(|t| {
            let mut description = t.description.clone();
            let parameters =
                crate::llm::rig_adapter::normalize_schema_strict(&t.parameters, &mut description);
            serde_json::json!({
                "type": "function",
                "function": {
                    "name": t.name,
                    "description": description,
                    "parameters": parameters,
                },
            })
        })
        .collect()
}

// ── LlmProvider impl ─────────────────────────────────────────────────────────

#[async_trait]
impl LlmProvider for OpenAiCompatStreamingProvider {
    fn model_name(&self) -> &str {
        &self.model_name
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        self.inner.cost_per_token()
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        self.inner.complete(request).await
    }

    async fn complete_with_tools(
        &self,
        request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        self.inner.complete_with_tools(request).await
    }

    async fn complete_stream(
        &self,
        mut req: CompletionRequest,
        on_chunk: &mut (dyn FnMut(String) + Send),
    ) -> Result<CompletionResponse, LlmError> {
        let model = req
            .take_model_override()
            .unwrap_or_else(|| self.model_name.clone());
        // Match RigAdapter behavior: rewrite orphaned tool_result messages as
        // user messages so OpenAI-compatible endpoints do not reject the
        // request with 400 "messages with role 'tool' must be a response to
        // a preceeding message with 'tool_calls'".
        sanitize_tool_messages(&mut req.messages);
        let messages = messages_to_json(&req.messages);

        let mut body = serde_json::json!({
            "model": model,
            "messages": messages,
            "stream": true,
            "stream_options": {"include_usage": true},
        });

        if !self.unsupported_params.contains("temperature") {
            if let Some(t) = req.temperature {
                body["temperature"] = serde_json::json!(t);
            }
        }
        if !self.unsupported_params.contains("max_tokens") {
            if let Some(mt) = req.max_tokens {
                body["max_tokens"] = serde_json::json!(mt);
            }
        }
        if !self.unsupported_params.contains("stop_sequences")
            && let Some(stop) = req.stop_sequences
            && !stop.is_empty()
        {
            body["stop"] = serde_json::json!(stop);
        }

        let result = self.stream_request(body, on_chunk).await?;

        Ok(CompletionResponse {
            content: result.content,
            finish_reason: result.finish_reason,
            input_tokens: result.input_tokens,
            output_tokens: result.output_tokens,
            cache_read_input_tokens: 0,
            cache_creation_input_tokens: 0,
        })
    }

    async fn complete_with_tools_stream(
        &self,
        mut req: ToolCompletionRequest,
        on_chunk: &mut (dyn FnMut(String) + Send),
    ) -> Result<ToolCompletionResponse, LlmError> {
        let model = req
            .take_model_override()
            .unwrap_or_else(|| self.model_name.clone());
        sanitize_tool_messages(&mut req.messages);
        let messages = messages_to_json(&req.messages);
        let tools = tools_to_json(&req.tools);

        let mut body = serde_json::json!({
            "model": model,
            "messages": messages,
            "tools": tools,
            "stream": true,
            "stream_options": {"include_usage": true},
        });

        if let Some(tc) = req.tool_choice {
            body["tool_choice"] = serde_json::json!(tc);
        }
        if !self.unsupported_params.contains("temperature") {
            if let Some(t) = req.temperature {
                body["temperature"] = serde_json::json!(t);
            }
        }
        if !self.unsupported_params.contains("max_tokens") {
            if let Some(mt) = req.max_tokens {
                body["max_tokens"] = serde_json::json!(mt);
            }
        }
        if !self.unsupported_params.contains("stop_sequences")
            && let Some(stop) = req.stop_sequences
            && !stop.is_empty()
        {
            body["stop"] = serde_json::json!(stop);
        }

        let result = self.stream_request(body, on_chunk).await?;

        let content = if !result.content.is_empty() {
            Some(result.content)
        } else {
            None
        };

        Ok(ToolCompletionResponse {
            content,
            tool_calls: result.tool_calls,
            finish_reason: result.finish_reason,
            input_tokens: result.input_tokens,
            output_tokens: result.output_tokens,
            cache_read_input_tokens: 0,
            cache_creation_input_tokens: 0,
        })
    }

    async fn list_models(&self) -> Result<Vec<String>, LlmError> {
        self.inner.list_models().await
    }

    fn active_model_name(&self) -> String {
        self.inner.active_model_name()
    }

    fn set_model(&self, model: &str) -> Result<(), LlmError> {
        self.inner.set_model(model)
    }
}
