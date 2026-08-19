//! Anthropic OAuth provider (direct HTTP, `Authorization: Bearer`).
//!
//! This provider exists because the `rig-core` Anthropic client hardcodes the
//! `x-api-key` header, which is rejected by Anthropic's OAuth tokens from
//! `claude login`. OAuth tokens require `Authorization: Bearer <token>` instead.
//!
//! Pattern follows `nearai_chat.rs`: direct HTTP calls via `reqwest::Client`.

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use eventsource_stream::Eventsource;
use futures::StreamExt;
use reqwest::Client;
use rust_decimal::Decimal;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};

use crate::anthropic_thinking::{AnthropicThinking, thinking_for_request};
use crate::config::RegistryProviderConfig;
use crate::error::LlmError;
use crate::provider::{
    ChatMessage, CompletionRequest, CompletionResponse, CompletionStreamSink, ContentPart,
    FinishReason, LlmProvider, Role, ToolCall, ToolCompletionRequest, ToolCompletionResponse,
    ToolDefinition, strip_unsupported_completion_params, strip_unsupported_tool_params,
};
use ironclaw_common::llm_costs as costs;

/// Read a fresh `claude login` OAuth token from the OS credential store.
///
/// Mirrors `ironclaw::config::ClaudeCodeConfig::extract_oauth_token` but is
/// inlined here so this crate doesn't depend on the main binary. Used to
/// retry once after a 401 if the user's OAuth token has been rotated by
/// Claude Code's background refresh.
fn refresh_claude_oauth_token() -> Option<String> {
    if cfg!(target_os = "macos") {
        match std::process::Command::new("security")
            .args([
                "find-generic-password",
                "-s",
                "Claude Code-credentials",
                "-w",
            ])
            .output()
        {
            Ok(output) if output.status.success() => {
                if let Ok(json) = String::from_utf8(output.stdout) {
                    return parse_oauth_access_token(json.trim());
                }
            }
            _ => {}
        }
    }
    if let Some(home) = dirs::home_dir() {
        let creds_path = home.join(".claude").join(".credentials.json");
        if let Ok(json) = std::fs::read_to_string(&creds_path) {
            return parse_oauth_access_token(&json);
        }
    }
    None
}

fn parse_oauth_access_token(json: &str) -> Option<String> {
    let creds: serde_json::Value = serde_json::from_str(json).ok()?;
    let token = creds["claudeAiOauth"]["accessToken"].as_str()?;
    if !token.starts_with("sk-ant-oat") {
        tracing::debug!("Ignoring credential store token with unexpected prefix");
        return None;
    }
    Some(token.to_string())
}
/// Map an HTTP error status + response body to a context-length error when it
/// indicates the prompt exceeded the model's context window.
///
/// Returns `Some(LlmError::ContextLengthExceeded { .. })` for HTTP 413 or for
/// an HTTP 400 whose body matches a context-overflow pattern, and `None`
/// otherwise. Delegates to the shared `crate::error::context_length_error`
/// helper so detection stays consistent across direct-HTTP providers.
#[cfg(test)]
fn context_length_error_for_status(status_code: u16, response_text: &str) -> Option<LlmError> {
    crate::error::context_length_error(status_code, response_text)
}

const ANTHROPIC_API_URL: &str = "https://api.anthropic.com/v1/messages";
/// OAuth beta requires 2023-06-01; the 2024-10-22 version is not valid with the beta flag.
const ANTHROPIC_API_VERSION: &str = "2023-06-01";
/// Required beta flag to enable OAuth Bearer auth on api.anthropic.com.
/// Without this header, the API returns 401 "OAuth authentication is currently not supported."
const ANTHROPIC_OAUTH_BETA: &str = "oauth-2025-04-20";
const DEFAULT_MAX_TOKENS: u32 = 8192;

/// Anthropic provider using OAuth Bearer authentication.
pub(crate) struct AnthropicOAuthProvider {
    client: Client,
    streaming_client: Client,
    stream_idle_timeout: Duration,
    /// OAuth token, wrapped in RwLock so it can be updated after a successful
    /// Keychain refresh (fixes #1136: stale token reuse after expiry).
    token: std::sync::RwLock<SecretString>,
    model: String,
    base_url: Option<String>,
    active_model: std::sync::RwLock<String>,
    /// Parameter names that this provider does not support.
    unsupported_params: HashSet<String>,
    /// Anthropic prompt-cache retention; drives the explicit `cache_control`
    /// breakpoints (system prompt, last tool, last message block). See #6984.
    cache_retention: crate::config::CacheRetention,
}

impl AnthropicOAuthProvider {
    pub(crate) fn new(config: &RegistryProviderConfig) -> Result<Self, LlmError> {
        let token = config
            .oauth_token
            .clone()
            .ok_or_else(|| LlmError::AuthFailed {
                provider: "anthropic_oauth".to_string(),
            })?;

        let client =
            crate::config::hardened_client_builder(crate::config::DEFAULT_REQUEST_TIMEOUT_SECS)
                .build()
                .map_err(|e| LlmError::RequestFailed {
                    provider: "anthropic_oauth".to_string(),
                    reason: format!("Failed to build HTTP client: {}", e),
                })?;
        let streaming_client = crate::config::hardened_streaming_client_builder()
            .build()
            .map_err(|e| LlmError::RequestFailed {
                provider: "anthropic_oauth".to_string(),
                reason: format!("Failed to build streaming HTTP client: {e}"),
            })?;

        let active_model = std::sync::RwLock::new(config.model.clone());
        let base_url = if config.base_url.is_empty() {
            None
        } else {
            Some(config.base_url.clone())
        };

        let unsupported_params: HashSet<String> =
            config.unsupported_params.iter().cloned().collect();

        let cache_retention =
            crate::rig_adapter::effective_cache_retention(config.cache_retention, &config.model);

        Ok(Self {
            client,
            streaming_client,
            stream_idle_timeout: Duration::from_secs(crate::config::DEFAULT_REQUEST_TIMEOUT_SECS),
            token: std::sync::RwLock::new(token),
            model: config.model.clone(),
            base_url,
            active_model,
            unsupported_params,
            cache_retention,
        })
    }

    /// Strip unsupported fields from a `CompletionRequest` in place.
    fn strip_unsupported_completion_params(&self, req: &mut CompletionRequest) {
        strip_unsupported_completion_params(&self.unsupported_params, req);
    }

    /// Strip unsupported fields from a `ToolCompletionRequest` in place.
    fn strip_unsupported_tool_params(&self, req: &mut ToolCompletionRequest) {
        strip_unsupported_tool_params(&self.unsupported_params, req);
    }

    fn api_url(&self) -> String {
        if let Some(ref base) = self.base_url {
            let base = base.trim_end_matches('/');
            format!("{}/v1/messages", base)
        } else {
            ANTHROPIC_API_URL.to_string()
        }
    }

    /// Read the current token from the RwLock.
    fn current_token(&self) -> String {
        match self.token.read() {
            Ok(guard) => guard.expose_secret().to_string(),
            Err(poisoned) => poisoned.into_inner().expose_secret().to_string(),
        }
    }

    /// Update the stored token after a successful Keychain refresh.
    fn update_token(&self, new_token: SecretString) {
        match self.token.write() {
            Ok(mut guard) => *guard = new_token,
            Err(poisoned) => *poisoned.into_inner() = new_token,
        }
    }

    async fn send_request<R: for<'de> Deserialize<'de>>(
        &self,
        body: &AnthropicRequest,
    ) -> Result<R, LlmError> {
        let url = self.api_url();

        tracing::debug!("Sending request to Anthropic OAuth: {}", url);

        let response = self
            .client
            .post(&url)
            .bearer_auth(self.current_token())
            .header("anthropic-version", ANTHROPIC_API_VERSION)
            .header("anthropic-beta", ANTHROPIC_OAUTH_BETA)
            .header("Content-Type", "application/json")
            .json(body)
            .send()
            .await
            .map_err(|e| LlmError::RequestFailed {
                provider: "anthropic_oauth".to_string(),
                reason: e.to_string(),
            })?;

        let status = response.status();

        if !status.is_success() {
            // Parse Retry-After header before consuming the body.
            let retry_after = crate::retry::retry_after_for_status(
                status.as_u16(),
                response.headers().get("retry-after"),
            );

            let response_text = response
                .text()
                .await
                .unwrap_or_else(|e| format!("(failed to read error body: {e})"));

            if status.as_u16() == 401 {
                // OAuth tokens from `claude login` expire in ~8-12h. Attempt
                // to re-extract a fresh token from the OS credential store
                // (macOS Keychain / Linux credentials file) before giving up.
                //
                // Brief delay to give Claude Code time to complete its async
                // Keychain refresh write (fixes race in #1136).
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;

                if let Some(fresh) = refresh_claude_oauth_token() {
                    let fresh_token = SecretString::from(fresh);
                    // Retry once with the refreshed token
                    let retry = self
                        .client
                        .post(&url)
                        .bearer_auth(fresh_token.expose_secret())
                        .header("anthropic-version", ANTHROPIC_API_VERSION)
                        .header("anthropic-beta", ANTHROPIC_OAUTH_BETA)
                        .header("Content-Type", "application/json")
                        .json(body)
                        .send()
                        .await
                        .map_err(|e| LlmError::RequestFailed {
                            provider: "anthropic_oauth".to_string(),
                            reason: e.to_string(),
                        })?;
                    let retry_status = retry.status();
                    if retry_status.is_success() {
                        // Persist the refreshed token so subsequent requests
                        // don't hit 401 again (fixes #1136).
                        self.update_token(fresh_token);
                        tracing::info!("Anthropic OAuth token refreshed from credential store");

                        let text = retry.text().await.map_err(|e| LlmError::RequestFailed {
                            provider: "anthropic_oauth".to_string(),
                            reason: format!("Failed to read response body: {}", e),
                        })?;
                        return serde_json::from_str(&text).map_err(|e| {
                            let truncated = ironclaw_common::truncate_for_preview(&text, 512);
                            LlmError::InvalidResponse {
                                provider: "anthropic_oauth".to_string(),
                                reason: format!("JSON parse error: {}. Raw: {}", e, truncated),
                            }
                        });
                    }
                    let retry_after = crate::retry::retry_after_for_status(
                        retry_status.as_u16(),
                        retry.headers().get("retry-after"),
                    );
                    let retry_text = retry
                        .text()
                        .await
                        .unwrap_or_else(|e| format!("(failed to read error body: {e})"));
                    tracing::warn!(
                        "Anthropic OAuth 401 retry with refreshed token also failed ({})",
                        retry_status
                    );
                    return Err(crate::error::map_provider_http_error(
                        crate::error::ProviderHttpError {
                            adapter: crate::error::ProductionModelAdapter::AnthropicOauth,
                            model: &self.active_model_name(),
                            status: retry_status.as_u16(),
                            body: &retry_text,
                            retry_after,
                        },
                    ));
                }
                return Err(LlmError::AuthFailed {
                    provider: "anthropic_oauth".to_string(),
                });
            }
            return Err(crate::error::map_provider_http_error(
                crate::error::ProviderHttpError {
                    adapter: crate::error::ProductionModelAdapter::AnthropicOauth,
                    model: &self.active_model_name(),
                    status: status.as_u16(),
                    body: &response_text,
                    retry_after,
                },
            ));
        }

        let response_text = response.text().await.map_err(|e| LlmError::RequestFailed {
            provider: "anthropic_oauth".to_string(),
            reason: format!("Failed to read response body: {}", e),
        })?;

        tracing::debug!(
            "Anthropic OAuth response: status={}, bytes={}",
            status,
            response_text.len()
        );

        serde_json::from_str(&response_text).map_err(|e| {
            let truncated = ironclaw_common::truncate_for_preview(&response_text, 512);
            LlmError::InvalidResponse {
                provider: "anthropic_oauth".to_string(),
                reason: format!("JSON parse error: {}. Raw: {}", e, truncated),
            }
        })
    }

    async fn send_streaming_request(
        &self,
        body: &AnthropicRequest,
        sink: Arc<dyn CompletionStreamSink>,
    ) -> Result<AnthropicStreamingResponse, LlmError> {
        let url = self.api_url();
        let current_token = self.current_token();
        let mut response = self
            .send_streaming_http_request(&url, body, &current_token)
            .await?;

        if response.status().as_u16() == 401 {
            drop(response);
            // OAuth tokens from `claude login` expire in ~8-12h. Match the
            // buffered request path by allowing Claude Code's asynchronous
            // credential-store refresh to settle, then retry exactly once.
            tokio::time::sleep(Duration::from_millis(500)).await;
            let Some(fresh) = refresh_claude_oauth_token() else {
                return Err(LlmError::AuthFailed {
                    provider: "anthropic_oauth".to_string(),
                });
            };
            let fresh_token = SecretString::from(fresh);
            response = self
                .send_streaming_http_request(&url, body, fresh_token.expose_secret())
                .await?;
            if response.status().is_success() {
                self.update_token(fresh_token);
                tracing::info!("Anthropic OAuth token refreshed from credential store");
            }
        }

        let status = response.status();
        if !status.is_success() {
            let retry_after = crate::retry::retry_after_for_status(
                status.as_u16(),
                response.headers().get("retry-after"),
            );
            if status.as_u16() == 401 {
                return Err(LlmError::AuthFailed {
                    provider: "anthropic_oauth".to_string(),
                });
            }
            // silent-ok: the bounded provider error body is diagnostic only;
            // status-based classification remains authoritative if reading it fails.
            let response_body = tokio::time::timeout(
                Duration::from_secs(5),
                crate::error::read_bounded_provider_error_body(response),
            )
            .await
            .ok()
            .and_then(Result::ok)
            .unwrap_or_default();
            let response_text = String::from_utf8_lossy(&response_body);
            return Err(crate::error::map_provider_http_error(
                crate::error::ProviderHttpError {
                    adapter: crate::error::ProductionModelAdapter::AnthropicOauth,
                    model: &self.active_model_name(),
                    status: status.as_u16(),
                    body: response_text.as_ref(),
                    retry_after,
                },
            ));
        }

        let mut events = response
            .bytes_stream()
            .map(|chunk| chunk.map_err(|error| error.to_string()))
            .eventsource();
        let mut streamed = AnthropicStreamingResponse::default();
        loop {
            let next = tokio::time::timeout(self.stream_idle_timeout, events.next())
                .await
                .map_err(|_| LlmError::StreamInterrupted {
                    provider: "anthropic_oauth".to_string(),
                    reason: format!(
                        "SSE stream was idle for {} seconds",
                        self.stream_idle_timeout.as_secs()
                    ),
                })?;
            let Some(event) = next else {
                break;
            };
            let event = event.map_err(|error| LlmError::StreamInterrupted {
                provider: "anthropic_oauth".to_string(),
                reason: format!("Failed to read SSE stream: {error}"),
            })?;
            ingest_anthropic_event(&mut streamed, &event.event, &event.data, sink.as_ref()).await?;
            if streamed.terminal {
                break;
            }
        }
        if !streamed.terminal {
            return Err(LlmError::StreamInterrupted {
                provider: "anthropic_oauth".to_string(),
                reason: "stream ended before message_stop or a stop reason".to_string(),
            });
        }
        streamed.finish()
    }

    async fn send_streaming_http_request(
        &self,
        url: &str,
        body: &AnthropicRequest,
        token: &str,
    ) -> Result<reqwest::Response, LlmError> {
        tokio::time::timeout(
            self.stream_idle_timeout,
            self.streaming_client
                .post(url)
                .bearer_auth(token)
                .header("anthropic-version", ANTHROPIC_API_VERSION)
                .header("anthropic-beta", ANTHROPIC_OAUTH_BETA)
                .header("Content-Type", "application/json")
                .json(body)
                .send(),
        )
        .await
        .map_err(|_| LlmError::RequestFailed {
            provider: "anthropic_oauth".to_string(),
            reason: format!(
                "timed out waiting {}s for streaming response headers",
                self.stream_idle_timeout.as_secs()
            ),
        })?
        .map_err(|e| LlmError::RequestFailed {
            provider: "anthropic_oauth".to_string(),
            reason: e.to_string(),
        })
    }
}

#[async_trait]
impl LlmProvider for AnthropicOAuthProvider {
    fn provider_id(&self) -> String {
        "anthropic_oauth".to_string()
    }

    async fn complete(&self, mut req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let model = req
            .take_model_override()
            .unwrap_or_else(|| self.active_model_name());
        self.strip_unsupported_completion_params(&mut req);
        let output_config = anthropic_output_config(req.response_format.as_ref())?;
        let (system, messages) = convert_messages(req.messages);
        let max_tokens = req.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS);

        let mut request = AnthropicRequest {
            stream: false,
            thinking: thinking_for_request(&model, max_tokens, req.temperature, false),
            model,
            messages,
            system: system.map(AnthropicSystem::Text),
            max_tokens,
            temperature: req.temperature,
            tools: None,
            tool_choice: None,
            output_config,
        };

        apply_cache_breakpoints(&mut request, self.cache_retention);

        let response: AnthropicResponse = self.send_request(&request).await?;
        let extracted = extract_response_content(&response);

        let finish_reason = match response.stop_reason.as_deref() {
            Some("end_turn") | Some("stop") => FinishReason::Stop,
            Some("max_tokens") => FinishReason::Length,
            Some("tool_use") => FinishReason::ToolUse,
            _ => FinishReason::Unknown,
        };

        Ok(CompletionResponse {
            content: extracted.content.unwrap_or_default(),
            finish_reason,
            input_tokens: response.usage.input_tokens,
            output_tokens: response.usage.output_tokens,
            reasoning: extracted.reasoning,
            cache_creation_input_tokens: response.usage.cache_creation_input_tokens,
            cache_read_input_tokens: response.usage.cache_read_input_tokens,
        })
    }

    async fn complete_streaming(
        &self,
        mut req: CompletionRequest,
        sink: Arc<dyn CompletionStreamSink>,
    ) -> Result<CompletionResponse, LlmError> {
        let model = req
            .take_model_override()
            .unwrap_or_else(|| self.active_model_name());
        self.strip_unsupported_completion_params(&mut req);
        let output_config = anthropic_output_config(req.response_format.as_ref())?;
        let (system, messages) = convert_messages(req.messages);
        let max_tokens = req.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS);
        let request = AnthropicRequest {
            stream: true,
            thinking: thinking_for_request(&model, max_tokens, req.temperature, false),
            model,
            messages,
            system: system.map(AnthropicSystem::Text),
            max_tokens,
            temperature: req.temperature,
            tools: None,
            tool_choice: None,
            output_config,
        };
        let response = self.send_streaming_request(&request, sink).await?;
        Ok(CompletionResponse {
            content: response.content,
            finish_reason: map_anthropic_finish_reason(response.stop_reason.as_deref(), false),
            input_tokens: response.usage.input_tokens,
            output_tokens: response.usage.output_tokens,
            reasoning: response.reasoning,
            cache_creation_input_tokens: response.usage.cache_creation_input_tokens,
            cache_read_input_tokens: response.usage.cache_read_input_tokens,
        })
    }

    async fn complete_with_tools(
        &self,
        mut req: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        let model = req
            .take_model_override()
            .unwrap_or_else(|| self.active_model_name());
        self.strip_unsupported_tool_params(&mut req);
        let output_config = anthropic_output_config(req.response_format.as_ref())?;
        let (system, messages) = convert_messages(req.messages);

        let tools = convert_anthropic_tools(req.tools);
        let tool_choice = convert_anthropic_tool_choice(req.tool_choice);
        let max_tokens = req.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS);

        // Suppress thinking for tool-capable requests to avoid signature round-trip issues.
        // Anthropic requires signed thinking blocks to be echoed back on subsequent tool_result
        // turns; without round-tripping the signature, the next turn fails. Reasoning is
        // preserved on text-only turns via `complete()`.
        let has_tools = !tools.is_empty();
        let opt_tools = if has_tools { Some(tools) } else { None };

        let mut request = AnthropicRequest {
            stream: false,
            thinking: if has_tools {
                None
            } else {
                thinking_for_request(&model, max_tokens, req.temperature, false)
            },
            model,
            messages,
            system: system.map(AnthropicSystem::Text),
            max_tokens,
            temperature: req.temperature,
            tools: opt_tools,
            tool_choice,
            output_config,
        };

        apply_cache_breakpoints(&mut request, self.cache_retention);

        let response: AnthropicResponse = self.send_request(&request).await?;
        let extracted = extract_response_content(&response);

        let finish_reason = match response.stop_reason.as_deref() {
            Some("end_turn") | Some("stop") => FinishReason::Stop,
            Some("max_tokens") => FinishReason::Length,
            Some("tool_use") => FinishReason::ToolUse,
            _ => {
                if !extracted.tool_calls.is_empty() {
                    FinishReason::ToolUse
                } else {
                    FinishReason::Unknown
                }
            }
        };

        Ok(ToolCompletionResponse {
            content: extracted.content,
            tool_calls: extracted.tool_calls,
            finish_reason,
            input_tokens: response.usage.input_tokens,
            output_tokens: response.usage.output_tokens,
            cache_creation_input_tokens: response.usage.cache_creation_input_tokens,
            reasoning: extracted.reasoning,
            reasoning_details: None,
            cache_read_input_tokens: response.usage.cache_read_input_tokens,
        })
    }

    async fn complete_with_tools_streaming(
        &self,
        mut req: ToolCompletionRequest,
        sink: Arc<dyn CompletionStreamSink>,
    ) -> Result<ToolCompletionResponse, LlmError> {
        let model = req
            .take_model_override()
            .unwrap_or_else(|| self.active_model_name());
        self.strip_unsupported_tool_params(&mut req);
        let output_config = anthropic_output_config(req.response_format.as_ref())?;
        let (system, messages) = convert_messages(req.messages);
        let tools = convert_anthropic_tools(req.tools);
        let tool_choice = convert_anthropic_tool_choice(req.tool_choice);
        let max_tokens = req.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS);
        let has_tools = !tools.is_empty();
        let request = AnthropicRequest {
            stream: true,
            thinking: if has_tools {
                None
            } else {
                thinking_for_request(&model, max_tokens, req.temperature, false)
            },
            model,
            messages,
            system: system.map(AnthropicSystem::Text),
            max_tokens,
            temperature: req.temperature,
            tools: has_tools.then_some(tools),
            tool_choice,
            output_config,
        };
        let response = self.send_streaming_request(&request, sink).await?;
        let has_tool_calls = !response.tool_calls.is_empty();
        Ok(ToolCompletionResponse {
            content: (!response.content.is_empty()).then_some(response.content),
            tool_calls: response.tool_calls,
            finish_reason: map_anthropic_finish_reason(
                response.stop_reason.as_deref(),
                has_tool_calls,
            ),
            input_tokens: response.usage.input_tokens,
            output_tokens: response.usage.output_tokens,
            cache_creation_input_tokens: response.usage.cache_creation_input_tokens,
            cache_read_input_tokens: response.usage.cache_read_input_tokens,
            reasoning: response.reasoning,
            reasoning_details: None,
        })
    }

    fn model_name(&self) -> &str {
        &self.model
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        let model = self.active_model_name();
        costs::model_cost(&model).unwrap_or_else(costs::default_cost)
    }

    fn active_model_name(&self) -> String {
        match self.active_model.read() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    fn set_model(&self, model: &str) -> Result<(), LlmError> {
        match self.active_model.write() {
            Ok(mut guard) => {
                *guard = model.to_string();
            }
            Err(poisoned) => {
                *poisoned.into_inner() = model.to_string();
            }
        }
        Ok(())
    }
}

// --- Anthropic Messages API types ---

#[derive(Debug, Serialize)]
struct AnthropicRequest {
    stream: bool,
    model: String,
    messages: Vec<AnthropicMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<AnthropicSystem>,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking: Option<AnthropicThinking>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<AnthropicTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<AnthropicToolChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output_config: Option<AnthropicOutputConfig>,
}

#[derive(Debug, Serialize)]
struct AnthropicOutputConfig {
    format: AnthropicOutputFormat,
}

#[derive(Debug, Serialize)]
struct AnthropicOutputFormat {
    #[serde(rename = "type")]
    format_type: &'static str,
    schema: serde_json::Value,
}

fn anthropic_output_config(
    response_format: Option<&crate::provider::CompletionResponseFormat>,
) -> Result<Option<AnthropicOutputConfig>, LlmError> {
    match response_format {
        None => Ok(None),
        Some(crate::provider::CompletionResponseFormat::JsonSchema(response_format)) => {
            Ok(Some(AnthropicOutputConfig {
                format: AnthropicOutputFormat {
                    format_type: "json_schema",
                    schema: response_format.schema.clone(),
                },
            }))
        }
        Some(crate::provider::CompletionResponseFormat::JsonObject) => {
            Err(LlmError::InvalidRequest {
                provider: "anthropic_oauth".to_string(),
                reason: "native JSON-object response mode is not supported by Anthropic OAuth"
                    .to_string(),
            })
        }
    }
}

#[derive(Debug, Serialize)]
struct AnthropicMessage {
    role: String,
    content: AnthropicContent,
}

/// Anthropic system prompt: a plain string (legacy wire shape, kept when
/// caching is off) or content blocks so the last block can carry a
/// `cache_control` breakpoint.
#[derive(Debug, Serialize)]
#[serde(untagged)]
enum AnthropicSystem {
    Text(String),
    Blocks(Vec<AnthropicSystemBlock>),
}

#[derive(Debug, Serialize)]
struct AnthropicSystemBlock {
    #[serde(rename = "type")]
    block_type: &'static str,
    text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_control: Option<serde_json::Value>,
}

/// Anthropic content can be a simple string or a list of content blocks.
#[derive(Debug, Serialize)]
#[serde(untagged)]
enum AnthropicContent {
    Text(String),
    Blocks(Vec<AnthropicContentBlock>),
}

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
enum AnthropicContentBlock {
    #[serde(rename = "text")]
    Text {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<serde_json::Value>,
    },
    #[serde(rename = "image")]
    Image {
        source: AnthropicImageSource,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<serde_json::Value>,
    },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<serde_json::Value>,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<serde_json::Value>,
    },
}

impl AnthropicContentBlock {
    /// Stamp a `cache_control` breakpoint on this block.
    fn set_cache_control(&mut self, marker: serde_json::Value) {
        match self {
            Self::Text { cache_control, .. }
            | Self::Image { cache_control, .. }
            | Self::ToolUse { cache_control, .. }
            | Self::ToolResult { cache_control, .. } => *cache_control = Some(marker),
        }
    }
}

/// Inline base64 image source for an Anthropic `image` content block.
#[derive(Debug, Serialize)]
struct AnthropicImageSource {
    #[serde(rename = "type")]
    source_type: &'static str,
    media_type: String,
    data: String,
}

#[derive(Debug, Serialize)]
struct AnthropicTool {
    name: String,
    description: String,
    input_schema: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_control: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
struct AnthropicToolChoice {
    #[serde(rename = "type")]
    choice_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AnthropicResponse {
    content: Vec<AnthropicResponseBlock>,
    #[serde(default)]
    stop_reason: Option<String>,
    usage: AnthropicUsage,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum AnthropicResponseBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "thinking")]
    Thinking {
        #[serde(default)]
        thinking: Option<String>,
        #[serde(default)]
        summary: Option<String>,
        #[serde(default, rename = "signature")]
        _signature: Option<String>,
    },
    #[serde(rename = "redacted_thinking")]
    RedactedThinking {},
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    #[serde(other)]
    Other,
}

#[derive(Debug, Default, Deserialize)]
struct AnthropicUsage {
    #[serde(default)]
    input_tokens: u32,
    #[serde(default)]
    output_tokens: u32,
    #[serde(default)]
    cache_creation_input_tokens: u32,
    #[serde(default)]
    cache_read_input_tokens: u32,
}

#[derive(Debug, Default)]
struct AnthropicStreamingToolCall {
    id: String,
    name: String,
    input_json: String,
}

#[derive(Debug, Default)]
struct AnthropicStreamingResponse {
    content: String,
    reasoning_parts: Vec<String>,
    tool_call_parts: BTreeMap<u64, AnthropicStreamingToolCall>,
    reasoning: Option<String>,
    tool_calls: Vec<ToolCall>,
    stop_reason: Option<String>,
    usage: AnthropicUsage,
    terminal: bool,
}

impl AnthropicStreamingResponse {
    fn finish(mut self) -> Result<Self, LlmError> {
        for state in self.tool_call_parts.values() {
            if state.id.is_empty() || state.name.is_empty() {
                return Err(LlmError::InvalidResponse {
                    provider: "anthropic_oauth".to_string(),
                    reason: "streamed tool_use block is missing its id or name".to_string(),
                });
            }
        }
        self.tool_calls = std::mem::take(&mut self.tool_call_parts)
            .into_values()
            .map(|state| {
                let arguments = if state.input_json.is_empty() {
                    serde_json::json!({})
                } else {
                    serde_json::from_str(&state.input_json).map_err(|error| {
                        LlmError::InvalidResponse {
                            provider: "anthropic_oauth".to_string(),
                            reason: format!("streamed tool arguments are invalid JSON: {error}"),
                        }
                    })?
                };
                Ok(ToolCall {
                    id: state.id,
                    name: state.name,
                    arguments,
                    reasoning: None,
                    signature: None,
                    arguments_parse_error: None,
                })
            })
            .collect::<Result<Vec<_>, LlmError>>()?;
        self.reasoning = (!self.reasoning_parts.is_empty()).then(|| self.reasoning_parts.join(""));
        Ok(self)
    }
}

async fn ingest_anthropic_event(
    response: &mut AnthropicStreamingResponse,
    event_name: &str,
    data: &str,
    sink: &dyn CompletionStreamSink,
) -> Result<(), LlmError> {
    let body: serde_json::Value =
        serde_json::from_str(data).map_err(|error| LlmError::InvalidResponse {
            provider: "anthropic_oauth".to_string(),
            reason: format!(
                "stream JSON parse error: {error}. Raw: {}",
                ironclaw_common::truncate_for_preview(data, 512)
            ),
        })?;
    let event_type = body
        .get("type")
        .and_then(|value| value.as_str())
        .unwrap_or(event_name);
    match event_type {
        "message_start" => {
            if let Some(usage) = body.get("message").and_then(|message| message.get("usage")) {
                update_anthropic_usage(&mut response.usage, usage);
            }
        }
        "content_block_start" => {
            let index = body
                .get("index")
                .and_then(|value| value.as_u64())
                .unwrap_or(0);
            let block = body
                .get("content_block")
                .unwrap_or(&serde_json::Value::Null);
            match block.get("type").and_then(|value| value.as_str()) {
                Some("text") => {
                    if let Some(text) = block.get("text").and_then(|value| value.as_str())
                        && !text.is_empty()
                    {
                        response.content.push_str(text);
                        sink.text_delta(text.to_string()).await;
                    }
                }
                Some("tool_use") => {
                    response.tool_call_parts.insert(
                        index,
                        AnthropicStreamingToolCall {
                            id: block
                                .get("id")
                                .and_then(|value| value.as_str())
                                .unwrap_or_default()
                                .to_string(),
                            name: block
                                .get("name")
                                .and_then(|value| value.as_str())
                                .unwrap_or_default()
                                .to_string(),
                            input_json: String::new(),
                        },
                    );
                }
                _ => {}
            }
        }
        "content_block_delta" => {
            let index = body
                .get("index")
                .and_then(|value| value.as_u64())
                .unwrap_or(0);
            let delta = body.get("delta").unwrap_or(&serde_json::Value::Null);
            match delta.get("type").and_then(|value| value.as_str()) {
                Some("text_delta") => {
                    if let Some(text) = delta.get("text").and_then(|value| value.as_str())
                        && !text.is_empty()
                    {
                        response.content.push_str(text);
                        sink.text_delta(text.to_string()).await;
                    }
                }
                Some("thinking_delta") => {
                    if let Some(thinking) = delta.get("thinking").and_then(|value| value.as_str()) {
                        response.reasoning_parts.push(thinking.to_string());
                    }
                }
                Some("input_json_delta") => {
                    if let Some(partial) =
                        delta.get("partial_json").and_then(|value| value.as_str())
                    {
                        response
                            .tool_call_parts
                            .entry(index)
                            .or_default()
                            .input_json
                            .push_str(partial);
                    }
                }
                _ => {}
            }
        }
        "message_delta" => {
            if let Some(stop_reason) = body
                .get("delta")
                .and_then(|delta| delta.get("stop_reason"))
                .and_then(|value| value.as_str())
            {
                response.stop_reason = Some(stop_reason.to_string());
                response.terminal = true;
            }
            if let Some(usage) = body.get("usage") {
                update_anthropic_usage(&mut response.usage, usage);
            }
        }
        "message_stop" => response.terminal = true,
        "error" => {
            let reason = body
                .get("error")
                .and_then(|error| error.get("message"))
                .and_then(|value| value.as_str())
                .unwrap_or("Anthropic streaming error");
            return Err(LlmError::RequestFailed {
                provider: "anthropic_oauth".to_string(),
                reason: reason.to_string(),
            });
        }
        _ => {}
    }
    Ok(())
}

fn update_anthropic_usage(usage: &mut AnthropicUsage, value: &serde_json::Value) {
    if let Some(tokens) = value.get("input_tokens").and_then(|value| value.as_u64()) {
        usage.input_tokens = tokens.min(u32::MAX as u64) as u32;
    }
    if let Some(tokens) = value.get("output_tokens").and_then(|value| value.as_u64()) {
        usage.output_tokens = tokens.min(u32::MAX as u64) as u32;
    }
    if let Some(tokens) = value
        .get("cache_creation_input_tokens")
        .and_then(|value| value.as_u64())
    {
        usage.cache_creation_input_tokens = tokens.min(u32::MAX as u64) as u32;
    }
    if let Some(tokens) = value
        .get("cache_read_input_tokens")
        .and_then(|value| value.as_u64())
    {
        usage.cache_read_input_tokens = tokens.min(u32::MAX as u64) as u32;
    }
}

fn map_anthropic_finish_reason(reason: Option<&str>, has_tool_calls: bool) -> FinishReason {
    match reason {
        Some("end_turn" | "stop_sequence" | "stop") => FinishReason::Stop,
        Some("max_tokens") => FinishReason::Length,
        Some("tool_use") => FinishReason::ToolUse,
        _ if has_tool_calls => FinishReason::ToolUse,
        _ => FinishReason::Unknown,
    }
}

/// Build Anthropic `image` content blocks from a user message's multimodal
/// parts. Only inline base64 `data:` images are forwarded (the Anthropic
/// messages API also accepts `url` sources, but the model gateway always emits
/// `data:` URLs); anything else is skipped so the text still reaches the model.
fn user_image_blocks(parts: &[ContentPart]) -> Vec<AnthropicContentBlock> {
    parts
        .iter()
        .filter_map(|part| match part {
            ContentPart::ImageUrl { image_url } => {
                let (media_type, data) = image_url.decode_data_url()?;
                Some(AnthropicContentBlock::Image {
                    cache_control: None,
                    source: AnthropicImageSource {
                        source_type: "base64",
                        media_type: media_type.to_string(),
                        data: data.to_string(),
                    },
                })
            }
            ContentPart::Text { .. } => None,
        })
        .collect()
}

/// Place the explicit Anthropic `cache_control` breakpoints (issue #6984):
/// the system prompt, the last tool definition, and the last content block of
/// the last message. Mirrors pi's placement so the tool/system prefix and the
/// growing conversation each cache independently. All markers carry the same
/// TTL, satisfying Anthropic's longer-TTL-first ordering rule. No-op when
/// retention is `None`, preserving the legacy wire shape (plain-string
/// system, no markers).
fn apply_cache_breakpoints(
    request: &mut AnthropicRequest,
    retention: crate::config::CacheRetention,
) {
    let Some(marker) = retention.cache_control_json() else {
        return;
    };

    // convert_messages only ever produces the string form, but restore any
    // other value untouched rather than dropping it on the floor.
    request.system = match request.system.take() {
        Some(AnthropicSystem::Text(text)) => {
            Some(AnthropicSystem::Blocks(vec![AnthropicSystemBlock {
                block_type: "text",
                text,
                cache_control: Some(marker.clone()),
            }]))
        }
        other => other,
    };

    if let Some(tools) = request.tools.as_mut()
        && let Some(last) = tools.last_mut()
    {
        last.cache_control = Some(marker.clone());
    }

    if let Some(last_message) = request.messages.last_mut() {
        match &mut last_message.content {
            // Empty text blocks cannot carry cache_control (API rejects
            // them), so an empty trailing message keeps the string form.
            AnthropicContent::Text(text) if !text.is_empty() => {
                last_message.content =
                    AnthropicContent::Blocks(vec![AnthropicContentBlock::Text {
                        text: std::mem::take(text),
                        cache_control: Some(marker),
                    }]);
            }
            AnthropicContent::Text(_) => {}
            AnthropicContent::Blocks(blocks) => {
                if let Some(last_block) = blocks.last_mut() {
                    last_block.set_cache_control(marker);
                }
            }
        }
    }
}

fn convert_anthropic_tools(tools: Vec<ToolDefinition>) -> Vec<AnthropicTool> {
    tools
        .into_iter()
        .map(|tool| AnthropicTool {
            name: tool.name,
            description: tool.description,
            input_schema: tool.parameters,
            // Markers are applied per-request by `apply_cache_breakpoints`;
            // the legacy shape carries none.
            cache_control: None,
        })
        .collect()
}

fn convert_anthropic_tool_choice(choice: Option<String>) -> Option<AnthropicToolChoice> {
    choice.map(|choice| match choice.as_str() {
        "auto" => AnthropicToolChoice {
            choice_type: "auto".to_string(),
            name: None,
        },
        "required" => AnthropicToolChoice {
            choice_type: "any".to_string(),
            name: None,
        },
        "none" => AnthropicToolChoice {
            choice_type: "none".to_string(),
            name: None,
        },
        specific => AnthropicToolChoice {
            choice_type: "tool".to_string(),
            name: Some(specific.to_string()),
        },
    })
}

/// Convert ChatMessage list to Anthropic format.
///
/// Extracts system messages to the top-level `system` parameter (Anthropic
/// doesn't allow system messages in the `messages` array). Tool-call/tool-result
/// pairs are converted to content blocks.
fn convert_messages(messages: Vec<ChatMessage>) -> (Option<String>, Vec<AnthropicMessage>) {
    let mut system_parts: Vec<String> = Vec::new();
    let mut anthropic_msgs: Vec<AnthropicMessage> = Vec::new();

    for msg in messages {
        match msg.role {
            Role::System => {
                if !msg.content.is_empty() {
                    system_parts.push(msg.content);
                }
            }
            Role::User => {
                let content = match user_image_blocks(&msg.content_parts) {
                    // Text-only (or no inline images): keep the compact string form.
                    blocks if blocks.is_empty() => AnthropicContent::Text(msg.content),
                    // Multimodal: text block first (when present), then images.
                    image_blocks => {
                        let mut blocks = Vec::with_capacity(1 + image_blocks.len());
                        if !msg.content.is_empty() {
                            blocks.push(AnthropicContentBlock::Text {
                                text: msg.content,
                                cache_control: None,
                            });
                        }
                        blocks.extend(image_blocks);
                        AnthropicContent::Blocks(blocks)
                    }
                };
                anthropic_msgs.push(AnthropicMessage {
                    role: "user".to_string(),
                    content,
                });
            }
            Role::Assistant => {
                if let Some(tool_calls) = msg.tool_calls {
                    // Assistant message with tool calls → content blocks
                    let mut blocks: Vec<AnthropicContentBlock> = Vec::new();
                    if !msg.content.is_empty() {
                        blocks.push(AnthropicContentBlock::Text {
                            text: msg.content,
                            cache_control: None,
                        });
                    }
                    for tc in tool_calls {
                        blocks.push(AnthropicContentBlock::ToolUse {
                            id: tc.id,
                            name: tc.name,
                            input: tc.arguments,
                            cache_control: None,
                        });
                    }
                    anthropic_msgs.push(AnthropicMessage {
                        role: "assistant".to_string(),
                        content: AnthropicContent::Blocks(blocks),
                    });
                } else {
                    anthropic_msgs.push(AnthropicMessage {
                        role: "assistant".to_string(),
                        content: AnthropicContent::Text(msg.content),
                    });
                }
            }
            Role::Tool => {
                let Some(tool_call_id) = msg.tool_call_id else {
                    tracing::warn!("Skipping Tool message without tool_call_id");
                    continue;
                };
                // Tool results go into a user message with tool_result blocks
                let block = AnthropicContentBlock::ToolResult {
                    tool_use_id: tool_call_id,
                    content: msg.content,
                    cache_control: None,
                };
                // If the last message is already a user message of *only*
                // tool-result blocks, append to it (Anthropic requires
                // consecutive tool results in one user message). Crucially, do
                // not merge into a multimodal user prompt (text + image
                // blocks) — that would fold a tool result into a different
                // conversational turn.
                if let Some(last) = anthropic_msgs.last_mut()
                    && last.role == "user"
                    && let AnthropicContent::Blocks(ref mut blocks) = last.content
                    && blocks
                        .iter()
                        .all(|b| matches!(b, AnthropicContentBlock::ToolResult { .. }))
                {
                    blocks.push(block);
                    continue;
                }
                anthropic_msgs.push(AnthropicMessage {
                    role: "user".to_string(),
                    content: AnthropicContent::Blocks(vec![block]),
                });
            }
        }
    }

    let system = if system_parts.is_empty() {
        None
    } else {
        Some(system_parts.join("\n\n"))
    };

    (system, anthropic_msgs)
}

#[derive(Debug, Default)]
struct ExtractedAnthropicResponse {
    content: Option<String>,
    reasoning: Option<String>,
    tool_calls: Vec<ToolCall>,
}

/// Extract text content and tool calls from an Anthropic response.
fn extract_response_content(response: &AnthropicResponse) -> ExtractedAnthropicResponse {
    let mut text_parts: Vec<String> = Vec::new();
    let mut reasoning_parts: Vec<String> = Vec::new();
    let mut tool_calls: Vec<ToolCall> = Vec::new();

    for block in &response.content {
        match block {
            AnthropicResponseBlock::Text { text } => {
                text_parts.push(text.clone());
            }
            AnthropicResponseBlock::Thinking {
                thinking,
                summary,
                _signature: _,
            } => {
                if let Some(reasoning) = summary
                    .as_deref()
                    .filter(|summary| !summary.trim().is_empty())
                    .or_else(|| {
                        thinking
                            .as_deref()
                            .filter(|thinking| !thinking.trim().is_empty())
                    })
                {
                    reasoning_parts.push(reasoning.to_string());
                }
            }
            AnthropicResponseBlock::RedactedThinking {} | AnthropicResponseBlock::Other => {}
            AnthropicResponseBlock::ToolUse { id, name, input } => {
                tool_calls.push(ToolCall {
                    id: id.clone(),
                    name: name.clone(),
                    arguments: input.clone(),
                    reasoning: None,
                    signature: None,
                    arguments_parse_error: None,
                });
            }
        }
    }

    let content = if text_parts.is_empty() {
        None
    } else {
        Some(text_parts.join(""))
    };

    let reasoning = if reasoning_parts.is_empty() {
        None
    } else {
        Some(reasoning_parts.join("\n"))
    };

    ExtractedAnthropicResponse {
        content,
        reasoning,
        tool_calls,
    }
}

// The transport/cache-breakpoint test suite lives in its own file so this
// one stays inside the file-size budget: `src/anthropic_oauth/tests.rs`.
#[cfg(test)]
mod tests;
