//! Anthropic direct HTTP provider for API-key and OAuth authentication.
//!
//! API keys use `x-api-key`; OAuth tokens from `claude login` use
//! `Authorization: Bearer <token>`. Keeping both modes on the same direct
//! Messages API implementation ensures they share validated SSE terminal-event
//! handling instead of relying on rig-core's EOF-synthesized completion.
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
use crate::config::{CacheRetention, RegistryProviderConfig};
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

enum AnthropicAuth {
    ApiKey(SecretString),
    OAuth(std::sync::RwLock<SecretString>),
}

/// Anthropic Messages API provider using API-key or OAuth authentication.
pub(crate) struct AnthropicDirectProvider {
    client: Client,
    streaming_client: Client,
    stream_idle_timeout: Duration,
    auth: AnthropicAuth,
    provider_id: String,
    model: String,
    base_url: Option<String>,
    active_model: std::sync::RwLock<String>,
    cache_retention: CacheRetention,
    models_endpoint: Option<crate::rig_adapter::ModelsEndpoint>,
    /// Parameter names that this provider does not support.
    unsupported_params: HashSet<String>,
}

impl AnthropicDirectProvider {
    pub(crate) fn new_oauth(
        config: &RegistryProviderConfig,
        request_timeout_secs: u64,
    ) -> Result<Self, LlmError> {
        let token = config
            .oauth_token
            .clone()
            .ok_or_else(|| LlmError::AuthFailed {
                provider: "anthropic_oauth".to_string(),
            })?;

        Self::new_with_auth(
            config,
            AnthropicAuth::OAuth(std::sync::RwLock::new(token)),
            request_timeout_secs,
        )
    }

    pub(crate) fn new_api_key(
        config: &RegistryProviderConfig,
        request_timeout_secs: u64,
    ) -> Result<Self, LlmError> {
        let api_key = config.api_key.clone().ok_or_else(|| LlmError::AuthFailed {
            provider: config.provider_id.clone(),
        })?;
        Self::new_with_auth(config, AnthropicAuth::ApiKey(api_key), request_timeout_secs)
    }

    fn new_with_auth(
        config: &RegistryProviderConfig,
        auth: AnthropicAuth,
        request_timeout_secs: u64,
    ) -> Result<Self, LlmError> {
        let provider_id = if matches!(&auth, AnthropicAuth::OAuth(_)) {
            "anthropic_oauth".to_string()
        } else {
            config.provider_id.clone()
        };
        let client_base_url = if config.base_url.is_empty() {
            ANTHROPIC_API_URL
        } else {
            &config.base_url
        };
        let client = crate::url_check::build_http_client(
            &provider_id,
            client_base_url,
            crate::config::hardened_client_builder(request_timeout_secs),
        )?;
        let streaming_client = crate::url_check::build_http_client(
            &provider_id,
            client_base_url,
            crate::config::hardened_streaming_client_builder(),
        )?;

        let active_model = std::sync::RwLock::new(config.model.clone());
        let base_url = if config.base_url.is_empty() {
            None
        } else {
            Some(config.base_url.clone())
        };

        let unsupported_params: HashSet<String> =
            config.unsupported_params.iter().cloned().collect();
        let models_endpoint = match &auth {
            AnthropicAuth::ApiKey(api_key) => {
                let base = if config.base_url.is_empty() {
                    "https://api.anthropic.com".to_string()
                } else {
                    config.base_url.trim_end_matches('/').to_string()
                };
                let discovery_base = if base.ends_with("/v1") || base.contains("/v1/") {
                    base
                } else {
                    format!("{base}/v1")
                };
                Some(crate::rig_adapter::ModelsEndpoint {
                    provider_id: provider_id.clone(),
                    url: format!("{discovery_base}/models"),
                    auth: crate::rig_adapter::ModelsAuth::AnthropicKey {
                        api_key: api_key.expose_secret().to_string(),
                        version: ANTHROPIC_API_VERSION.to_string(),
                    },
                    shape: crate::rig_adapter::ModelsShape::OpenAiData,
                    extra_headers: reqwest::header::HeaderMap::new(),
                })
            }
            AnthropicAuth::OAuth(_) => None,
        };

        Ok(Self {
            client,
            streaming_client,
            stream_idle_timeout: Duration::from_secs(request_timeout_secs),
            auth,
            provider_id,
            model: config.model.clone(),
            base_url,
            active_model,
            cache_retention: config.cache_retention,
            models_endpoint,
            unsupported_params,
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
            if base.ends_with("/v1") {
                format!("{base}/messages")
            } else {
                format!("{base}/v1/messages")
            }
        } else {
            ANTHROPIC_API_URL.to_string()
        }
    }

    fn authenticated_request(&self, client: &Client, url: &str) -> reqwest::RequestBuilder {
        self.apply_auth(
            client
                .post(url)
                .header("anthropic-version", ANTHROPIC_API_VERSION)
                .header("Content-Type", "application/json"),
        )
    }

    fn apply_auth(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.auth {
            AnthropicAuth::ApiKey(api_key) => request.header("x-api-key", api_key.expose_secret()),
            AnthropicAuth::OAuth(token) => {
                let token = match token.read() {
                    Ok(guard) => guard.expose_secret().to_string(),
                    Err(poisoned) => poisoned.into_inner().expose_secret().to_string(),
                };
                request
                    .bearer_auth(token)
                    .header("anthropic-beta", ANTHROPIC_OAUTH_BETA)
            }
        }
    }

    /// Update the stored token after a successful Keychain refresh.
    fn update_oauth_token(&self, new_token: SecretString) {
        if let AnthropicAuth::OAuth(token) = &self.auth {
            match token.write() {
                Ok(mut guard) => *guard = new_token,
                Err(poisoned) => *poisoned.into_inner() = new_token,
            }
        }
    }

    fn uses_oauth(&self) -> bool {
        matches!(self.auth, AnthropicAuth::OAuth(_))
    }

    fn error_adapter(&self) -> crate::error::ProductionModelAdapter {
        if self.uses_oauth() {
            crate::error::ProductionModelAdapter::AnthropicOauth
        } else {
            crate::error::ProductionModelAdapter::AnthropicApiKey
        }
    }

    fn cache_control(&self) -> Option<AnthropicCacheControl> {
        match self.cache_retention {
            CacheRetention::None => None,
            CacheRetention::Short => Some(AnthropicCacheControl {
                control_type: "ephemeral",
                ttl: None,
            }),
            CacheRetention::Long => Some(AnthropicCacheControl {
                control_type: "ephemeral",
                ttl: Some("1h"),
            }),
        }
    }

    fn convert_tools(&self, tools: Vec<ToolDefinition>) -> Vec<AnthropicTool> {
        tools
            .into_iter()
            .map(|tool| {
                let mut description = tool.description;
                let input_schema = if self.uses_oauth() {
                    tool.parameters
                } else {
                    crate::tool_schema::shape_tool_schema(
                        crate::tool_schema::ToolSchemaPolicy::StrictOpenAi,
                        &tool.parameters,
                        &mut description,
                    )
                };
                AnthropicTool {
                    name: tool.name,
                    description,
                    input_schema,
                }
            })
            .collect()
    }

    async fn send_request<R: for<'de> Deserialize<'de>>(
        &self,
        body: &AnthropicRequest,
    ) -> Result<R, LlmError> {
        let url = self.api_url();

        tracing::debug!(provider = %self.provider_id, "Sending request to Anthropic: {url}");

        let response = self
            .authenticated_request(&self.client, &url)
            .json(body)
            .send()
            .await
            .map_err(|e| LlmError::RequestFailed {
                provider: self.provider_id.clone(),
                reason: e.to_string(),
            })?;

        let status = response.status();

        if !status.is_success() {
            // Parse Retry-After header before consuming the body.
            let retry_after = crate::retry::retry_after_for_status(
                status.as_u16(),
                response.headers().get("retry-after"),
            );

            let response_text = crate::error::read_bounded_provider_error_body(response)
                .await
                .map(|body| String::from_utf8_lossy(&body).into_owned())
                .unwrap_or_else(|e| format!("(failed to read error body: {e})"));

            if status.as_u16() == 401 && self.uses_oauth() {
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
                            provider: self.provider_id.clone(),
                            reason: e.to_string(),
                        })?;
                    let retry_status = retry.status();
                    if retry_status.is_success() {
                        // Persist the refreshed token so subsequent requests
                        // don't hit 401 again (fixes #1136).
                        self.update_oauth_token(fresh_token);
                        tracing::info!("Anthropic OAuth token refreshed from credential store");

                        let text = retry.text().await.map_err(|e| LlmError::RequestFailed {
                            provider: self.provider_id.clone(),
                            reason: format!("Failed to read response body: {}", e),
                        })?;
                        return serde_json::from_str(&text).map_err(|e| {
                            let truncated = ironclaw_common::truncate_for_preview(&text, 512);
                            LlmError::InvalidResponse {
                                provider: self.provider_id.clone(),
                                reason: format!("JSON parse error: {}. Raw: {}", e, truncated),
                            }
                        });
                    }
                    let retry_after = crate::retry::retry_after_for_status(
                        retry_status.as_u16(),
                        retry.headers().get("retry-after"),
                    );
                    let retry_text = crate::error::read_bounded_provider_error_body(retry)
                        .await
                        .map(|body| String::from_utf8_lossy(&body).into_owned())
                        .unwrap_or_else(|e| format!("(failed to read error body: {e})"));
                    tracing::warn!(
                        "Anthropic OAuth 401 retry with refreshed token also failed ({})",
                        retry_status
                    );
                    return Err(crate::error::map_provider_http_error(
                        crate::error::ProviderHttpError {
                            adapter: self.error_adapter(),
                            model: &self.active_model_name(),
                            status: retry_status.as_u16(),
                            body: &retry_text,
                            retry_after,
                        },
                    ));
                }
                return Err(LlmError::AuthFailed {
                    provider: self.provider_id.clone(),
                });
            }
            if status.as_u16() == 401 {
                return Err(LlmError::AuthFailed {
                    provider: self.provider_id.clone(),
                });
            }
            return Err(crate::error::map_provider_http_error(
                crate::error::ProviderHttpError {
                    adapter: self.error_adapter(),
                    model: &self.active_model_name(),
                    status: status.as_u16(),
                    body: &response_text,
                    retry_after,
                },
            ));
        }

        let response_text = response.text().await.map_err(|e| LlmError::RequestFailed {
            provider: self.provider_id.clone(),
            reason: format!("Failed to read response body: {}", e),
        })?;

        tracing::debug!(
            "Anthropic response: status={}, bytes={}",
            status,
            response_text.len()
        );

        serde_json::from_str(&response_text).map_err(|e| {
            let truncated = ironclaw_common::truncate_for_preview(&response_text, 512);
            LlmError::InvalidResponse {
                provider: self.provider_id.clone(),
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
        let mut response = self.send_streaming_http_request(&url, body).await?;

        if response.status().as_u16() == 401 && self.uses_oauth() {
            drop(response);
            // OAuth tokens from `claude login` expire in ~8-12h. Match the
            // buffered request path by allowing Claude Code's asynchronous
            // credential-store refresh to settle, then retry exactly once.
            tokio::time::sleep(Duration::from_millis(500)).await;
            let Some(fresh) = refresh_claude_oauth_token() else {
                return Err(LlmError::AuthFailed {
                    provider: self.provider_id.clone(),
                });
            };
            let fresh_token = SecretString::from(fresh);
            response = self
                .send_streaming_oauth_http_request(&url, body, fresh_token.expose_secret())
                .await?;
            if response.status().is_success() {
                self.update_oauth_token(fresh_token);
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
                    provider: self.provider_id.clone(),
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
                    adapter: self.error_adapter(),
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
                    provider: self.provider_id.clone(),
                    reason: format!(
                        "SSE stream was idle for {} seconds",
                        self.stream_idle_timeout.as_secs()
                    ),
                })?;
            let Some(event) = next else {
                break;
            };
            let event = event.map_err(|error| LlmError::StreamInterrupted {
                provider: self.provider_id.clone(),
                reason: format!("Failed to read SSE stream: {error}"),
            })?;
            ingest_anthropic_event(
                &mut streamed,
                &event.event,
                &event.data,
                sink.as_ref(),
                &self.provider_id,
            )
            .await?;
            if streamed.terminal {
                break;
            }
        }
        if !streamed.terminal {
            return Err(LlmError::StreamInterrupted {
                provider: self.provider_id.clone(),
                reason: "stream ended before message_stop or a stop reason".to_string(),
            });
        }
        streamed.finish(&self.provider_id)
    }

    async fn send_streaming_http_request(
        &self,
        url: &str,
        body: &AnthropicRequest,
    ) -> Result<reqwest::Response, LlmError> {
        tokio::time::timeout(
            self.stream_idle_timeout,
            self.authenticated_request(&self.streaming_client, url)
                .json(body)
                .send(),
        )
        .await
        .map_err(|_| LlmError::RequestFailed {
            provider: self.provider_id.clone(),
            reason: format!(
                "timed out waiting {}s for streaming response headers",
                self.stream_idle_timeout.as_secs()
            ),
        })?
        .map_err(|e| LlmError::RequestFailed {
            provider: self.provider_id.clone(),
            reason: e.to_string(),
        })
    }

    async fn send_streaming_oauth_http_request(
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
            provider: self.provider_id.clone(),
            reason: format!(
                "timed out waiting {}s for streaming response headers",
                self.stream_idle_timeout.as_secs()
            ),
        })?
        .map_err(|e| LlmError::RequestFailed {
            provider: self.provider_id.clone(),
            reason: e.to_string(),
        })
    }
}

#[async_trait]
impl LlmProvider for AnthropicDirectProvider {
    fn provider_id(&self) -> String {
        self.provider_id.clone()
    }

    async fn complete(&self, mut req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let model = req
            .take_model_override()
            .unwrap_or_else(|| self.active_model_name());
        self.strip_unsupported_completion_params(&mut req);
        let mut messages = req.messages;
        crate::provider::sanitize_tool_messages(&mut messages);
        let (system, messages) = convert_messages(messages);
        let max_tokens = req.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS);

        let request = AnthropicRequest {
            stream: false,
            cache_control: self.cache_control(),
            thinking: thinking_for_request(&model, max_tokens, req.temperature, false),
            model,
            messages,
            system,
            max_tokens,
            temperature: req.temperature,
            stop_sequences: req.stop_sequences,
            tools: None,
            tool_choice: None,
        };

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
        let mut messages = req.messages;
        crate::provider::sanitize_tool_messages(&mut messages);
        let (system, messages) = convert_messages(messages);
        let max_tokens = req.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS);
        let request = AnthropicRequest {
            stream: true,
            cache_control: self.cache_control(),
            thinking: thinking_for_request(&model, max_tokens, req.temperature, false),
            model,
            messages,
            system,
            max_tokens,
            temperature: req.temperature,
            stop_sequences: req.stop_sequences,
            tools: None,
            tool_choice: None,
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
        let mut messages = req.messages;
        crate::provider::sanitize_tool_messages(&mut messages);
        let (system, messages) = convert_messages(messages);

        let tools = self.convert_tools(req.tools);
        let tool_choice = convert_anthropic_tool_choice(req.tool_choice);
        let max_tokens = req.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS);

        // Suppress thinking for tool-capable requests to avoid signature round-trip issues.
        // Anthropic requires signed thinking blocks to be echoed back on subsequent tool_result
        // turns; without round-tripping the signature, the next turn fails. Reasoning is
        // preserved on text-only turns via `complete()`.
        let has_tools = !tools.is_empty();
        let opt_tools = if has_tools { Some(tools) } else { None };

        let request = AnthropicRequest {
            stream: false,
            cache_control: self.cache_control(),
            thinking: if has_tools {
                None
            } else {
                thinking_for_request(&model, max_tokens, req.temperature, false)
            },
            model,
            messages,
            system,
            max_tokens,
            temperature: req.temperature,
            stop_sequences: req.stop_sequences,
            tools: opt_tools,
            tool_choice,
        };

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
        let mut messages = req.messages;
        crate::provider::sanitize_tool_messages(&mut messages);
        let (system, messages) = convert_messages(messages);
        let tools = self.convert_tools(req.tools);
        let tool_choice = convert_anthropic_tool_choice(req.tool_choice);
        let max_tokens = req.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS);
        let has_tools = !tools.is_empty();
        let request = AnthropicRequest {
            stream: true,
            cache_control: self.cache_control(),
            thinking: if has_tools {
                None
            } else {
                thinking_for_request(&model, max_tokens, req.temperature, false)
            },
            model,
            messages,
            system,
            max_tokens,
            temperature: req.temperature,
            stop_sequences: req.stop_sequences,
            tools: has_tools.then_some(tools),
            tool_choice,
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

    async fn list_models(&self) -> Result<Vec<String>, LlmError> {
        match &self.models_endpoint {
            Some(endpoint) => endpoint.fetch_models().await,
            None => Ok(Vec::new()),
        }
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        let model = self.active_model_name();
        costs::model_cost(&model).unwrap_or_else(costs::default_cost)
    }

    fn cache_write_multiplier(&self) -> Decimal {
        match self.cache_retention {
            CacheRetention::None => Decimal::ONE,
            CacheRetention::Short => Decimal::new(125, 2),
            CacheRetention::Long => Decimal::TWO,
        }
    }

    fn cache_read_discount(&self) -> Decimal {
        if self.cache_retention == CacheRetention::None {
            Decimal::ONE
        } else {
            Decimal::TEN
        }
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
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_control: Option<AnthropicCacheControl>,
    model: String,
    messages: Vec<AnthropicMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stop_sequences: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking: Option<AnthropicThinking>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<AnthropicTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<AnthropicToolChoice>,
}

#[derive(Debug, Serialize)]
struct AnthropicCacheControl {
    #[serde(rename = "type")]
    control_type: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    ttl: Option<&'static str>,
}

#[derive(Debug, Serialize)]
struct AnthropicMessage {
    role: String,
    content: AnthropicContent,
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
    Text { text: String },
    #[serde(rename = "image")]
    Image { source: AnthropicImageSource },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: String,
    },
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
    fn finish(mut self, provider_id: &str) -> Result<Self, LlmError> {
        for state in self.tool_call_parts.values() {
            if state.id.is_empty() || state.name.is_empty() {
                return Err(LlmError::InvalidResponse {
                    provider: provider_id.to_string(),
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
                            provider: provider_id.to_string(),
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
    provider_id: &str,
) -> Result<(), LlmError> {
    let body: serde_json::Value =
        serde_json::from_str(data).map_err(|error| LlmError::InvalidResponse {
            provider: provider_id.to_string(),
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
                provider: provider_id.to_string(),
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
                            blocks.push(AnthropicContentBlock::Text { text: msg.content });
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
                        blocks.push(AnthropicContentBlock::Text { text: msg.content });
                    }
                    for tc in tool_calls {
                        blocks.push(AnthropicContentBlock::ToolUse {
                            id: tc.id,
                            name: tc.name,
                            input: tc.arguments,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct RecordingSink(std::sync::Mutex<Vec<String>>);

    #[async_trait]
    impl CompletionStreamSink for RecordingSink {
        async fn text_delta(&self, delta: String) {
            self.0
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .push(delta);
        }
    }

    #[tokio::test]
    async fn anthropic_stream_emits_text_and_preserves_terminal_tools_and_usage() {
        let sink = RecordingSink::default();
        let mut response = AnthropicStreamingResponse::default();
        ingest_anthropic_event(
            &mut response,
            "message_start",
            r#"{"type":"message_start","message":{"usage":{"input_tokens":11,"cache_read_input_tokens":3}}}"#,
            &sink,
            "anthropic_oauth",
        )
        .await
        .expect("message start");
        ingest_anthropic_event(
            &mut response,
            "content_block_delta",
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hello "}}"#,
            &sink,
            "anthropic_oauth",
        )
        .await
        .expect("text delta");
        assert_eq!(
            sink.0
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .as_slice(),
            ["hello "]
        );
        assert!(!response.terminal, "text must arrive before completion");
        ingest_anthropic_event(
            &mut response,
            "content_block_start",
            r#"{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"call-1","name":"weather","input":{}}}"#,
            &sink,
            "anthropic_oauth",
        )
        .await
        .expect("tool start");
        for partial_json in ["{\"city\":\"", "Istanbul\"}"] {
            ingest_anthropic_event(
                &mut response,
                "content_block_delta",
                &format!(
                    r#"{{"type":"content_block_delta","index":1,"delta":{{"type":"input_json_delta","partial_json":{}}}}}"#,
                    serde_json::to_string(partial_json).expect("partial JSON string")
                ),
                &sink,
                "anthropic_oauth",
            )
            .await
            .expect("tool delta");
        }
        ingest_anthropic_event(
            &mut response,
            "message_delta",
            r#"{"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":7}}"#,
            &sink,
            "anthropic_oauth",
        )
        .await
        .expect("terminal delta");
        let response = response.finish("anthropic_oauth").expect("complete stream");
        assert_eq!(response.content, "hello ");
        assert_eq!(response.stop_reason.as_deref(), Some("tool_use"));
        assert_eq!(response.usage.input_tokens, 11);
        assert_eq!(response.usage.output_tokens, 7);
        assert_eq!(response.usage.cache_read_input_tokens, 3);
        assert_eq!(response.tool_calls.len(), 1);
        assert_eq!(response.tool_calls[0].name, "weather");
        assert_eq!(
            response.tool_calls[0].arguments,
            serde_json::json!({"city":"Istanbul"})
        );
    }

    #[test]
    fn anthropic_stream_rejects_tool_state_missing_id_or_name() {
        for (id, name) in [("", "weather"), ("call-1", "")] {
            let mut response = AnthropicStreamingResponse::default();
            response.tool_call_parts.insert(
                0,
                AnthropicStreamingToolCall {
                    id: id.to_string(),
                    name: name.to_string(),
                    input_json: "{}".to_string(),
                },
            );

            assert!(matches!(
                response.finish("anthropic_oauth"),
                Err(LlmError::InvalidResponse { provider, reason })
                    if provider == "anthropic_oauth"
                        && reason == "streamed tool_use block is missing its id or name"
            ));
        }
    }

    #[test]
    fn anthropic_stream_rejects_malformed_accumulated_tool_arguments() {
        let mut response = AnthropicStreamingResponse::default();
        response.tool_call_parts.insert(
            0,
            AnthropicStreamingToolCall {
                id: "call-1".to_string(),
                name: "weather".to_string(),
                input_json: r#"{"city":"Istanbul""#.to_string(),
            },
        );

        match response.finish("anthropic_oauth") {
            Err(LlmError::InvalidResponse { provider, reason }) => {
                assert_eq!(provider, "anthropic_oauth");
                assert!(reason.starts_with("streamed tool arguments are invalid JSON: "));
            }
            other => panic!("expected invalid streamed tool arguments, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn complete_preserves_missing_retry_after_on_headerless_502() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("loopback listener");
        let base_url = format!(
            "http://{}",
            listener.local_addr().expect("loopback address")
        );
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept request");
            let mut request = vec![0_u8; 4096];
            let _ = socket.read(&mut request).await.expect("read request");
            let body = r#"{"error":{"message":"upstream unavailable"}}"#;
            let response = format!(
                "HTTP/1.1 502 Bad Gateway\r\ncontent-type: application/json\r\n\
                 content-length: {}\r\n\r\n{body}",
                body.len()
            );
            socket
                .write_all(response.as_bytes())
                .await
                .expect("write error response");
        });

        let mut config = RegistryProviderConfig::generic(
            crate::registry::ProviderProtocol::Anthropic,
            "anthropic_oauth",
            None,
            base_url,
            "claude-test",
        );
        config.oauth_token = Some(SecretString::from("test-token".to_string()));
        let provider = AnthropicDirectProvider::new_oauth(&config, 5).expect("provider");
        let error = provider
            .complete(CompletionRequest::new(vec![ChatMessage::user("hello")]))
            .await
            .expect_err("scripted provider error");
        server.await.expect("loopback server");

        assert!(matches!(
            error,
            LlmError::BadGateway {
                provider,
                status: 502,
                retry_after: None,
            } if provider == "anthropic_oauth"
        ));
    }

    #[tokio::test]
    async fn api_key_provider_preserves_anthropic_model_discovery() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("loopback listener");
        let base_url = format!(
            "http://{}/v1",
            listener.local_addr().expect("loopback address")
        );
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept request");
            let mut request = vec![0_u8; 4096];
            let read = socket.read(&mut request).await.expect("read request");
            let request = String::from_utf8_lossy(&request[..read]);
            assert!(request.starts_with("GET /v1/models HTTP/1.1"));
            let lowercase_request = request.to_ascii_lowercase();
            assert!(lowercase_request.contains("x-api-key: test-api-key"));
            assert!(lowercase_request.contains("anthropic-version: 2023-06-01"));
            assert!(!lowercase_request.contains("authorization:"));

            let body = r#"{"data":[{"id":"claude-one"},{"id":"claude-two"}]}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\n\
                 content-length: {}\r\n\r\n{body}",
                body.len()
            );
            socket
                .write_all(response.as_bytes())
                .await
                .expect("write models response");
        });

        let config = RegistryProviderConfig::generic(
            crate::registry::ProviderProtocol::Anthropic,
            "anthropic",
            Some(SecretString::from("test-api-key".to_string())),
            base_url,
            "claude-test",
        );
        let provider = AnthropicDirectProvider::new_api_key(&config, 5).expect("provider");
        assert_eq!(
            provider.list_models().await.expect("model discovery"),
            ["claude-one", "claude-two"]
        );
        server.await.expect("loopback server");
    }

    #[test]
    fn api_key_provider_preserves_rig_schema_and_cache_cost_contracts() {
        let mut config = RegistryProviderConfig::generic(
            crate::registry::ProviderProtocol::Anthropic,
            "anthropic",
            Some(SecretString::from("test-api-key".to_string())),
            "https://api.anthropic.com",
            "claude-test",
        );
        config.cache_retention = CacheRetention::Long;
        let provider = AnthropicDirectProvider::new_api_key(&config, 5).expect("provider");
        let tools = provider.convert_tools(vec![ToolDefinition {
            name: "weather".to_string(),
            description: "Read weather".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "city": {"type": "string"}
                }
            }),
        }]);

        assert_eq!(tools[0].input_schema["additionalProperties"], false);
        assert_eq!(
            tools[0].input_schema["required"],
            serde_json::json!(["city"])
        );
        assert_eq!(
            tools[0].input_schema["properties"]["city"]["type"],
            serde_json::json!(["string", "null"])
        );
        assert_eq!(provider.cache_write_multiplier(), Decimal::TWO);
        assert_eq!(provider.cache_read_discount(), Decimal::TEN);
    }

    #[tokio::test]
    async fn complete_streaming_rejects_eof_without_terminal_event() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("loopback listener");
        let base_url = format!(
            "http://{}",
            listener.local_addr().expect("loopback address")
        );
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept request");
            let mut request = vec![0_u8; 4096];
            let _ = socket.read(&mut request).await.expect("read request");
            let body = concat!(
                "event: message_start\n",
                "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":1}}}\n\n",
                "event: content_block_delta\n",
                "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"partial\"}}\n\n"
            );
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\n\
                 content-length: {}\r\n\r\n{body}",
                body.len()
            );
            socket
                .write_all(response.as_bytes())
                .await
                .expect("write streaming response");
        });

        let mut config = RegistryProviderConfig::generic(
            crate::registry::ProviderProtocol::Anthropic,
            "anthropic_oauth",
            None,
            base_url,
            "claude-test",
        );
        config.oauth_token = Some(SecretString::from("test-token".to_string()));
        let provider = AnthropicDirectProvider::new_oauth(&config, 5).expect("provider");
        let sink = Arc::new(RecordingSink::default());
        let error = provider
            .complete_streaming(
                CompletionRequest::new(vec![ChatMessage::user("hello")]),
                sink.clone(),
            )
            .await
            .expect_err("unterminated stream must fail");
        server.await.expect("loopback server");

        assert!(matches!(
            error,
            LlmError::StreamInterrupted { provider, reason }
                if provider == "anthropic_oauth"
                    && reason == "stream ended before message_stop or a stop reason"
        ));
        assert_eq!(
            sink.0
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .as_slice(),
            ["partial"]
        );
    }

    #[test]
    fn context_overflow_413_maps_to_context_length_exceeded() {
        // A raw HTTP 413 (payload too large) must become ContextLengthExceeded
        // so the loop's context-shrink recovery fires.
        match context_length_error_for_status(413, "Request Entity Too Large") {
            Some(LlmError::ContextLengthExceeded { .. }) => {}
            other => panic!("expected ContextLengthExceeded, got {other:?}"),
        }
    }

    #[test]
    fn context_overflow_400_body_maps_to_context_length_exceeded() {
        let body = r#"{"type":"error","error":{"type":"invalid_request_error","message":"prompt is too long: 234872 tokens > 200000 maximum"}}"#;
        match context_length_error_for_status(400, body) {
            Some(LlmError::ContextLengthExceeded { used, limit }) => {
                assert_eq!(used, 234872);
                assert_eq!(limit, 200000);
            }
            other => panic!("expected ContextLengthExceeded, got {other:?}"),
        }
    }

    #[test]
    fn unrelated_400_is_not_context_overflow() {
        // A plain bad-request (e.g. invalid request shape) must NOT be
        // classified as context overflow — the caller falls through to
        // RequestFailed.
        assert!(
            context_length_error_for_status(400, r#"{"error":{"message":"invalid request body"}}"#)
                .is_none()
        );
    }

    #[test]
    fn unrelated_5xx_is_not_context_overflow() {
        assert!(context_length_error_for_status(503, "service unavailable").is_none());
    }

    #[test]
    fn test_convert_messages_extracts_system() {
        let messages = vec![
            ChatMessage::system("You are helpful."),
            ChatMessage::user("Hello"),
        ];
        let (system, msgs) = convert_messages(messages);
        assert_eq!(system, Some("You are helpful.".to_string()));
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].role, "user");
    }

    #[test]
    fn test_convert_messages_multiple_systems() {
        let messages = vec![
            ChatMessage::system("System 1"),
            ChatMessage::system("System 2"),
            ChatMessage::user("Hello"),
        ];
        let (system, msgs) = convert_messages(messages);
        assert_eq!(system, Some("System 1\n\nSystem 2".to_string()));
        assert_eq!(msgs.len(), 1);
    }

    #[test]
    fn test_convert_messages_user_image_becomes_base64_image_block() {
        let messages = vec![ChatMessage::user_with_parts(
            "what is this?",
            vec![ContentPart::ImageUrl {
                image_url: crate::provider::ImageUrl {
                    url: "data:image/png;base64,AQIDBA==".to_string(),
                    detail: None,
                },
            }],
        )];
        let (_system, msgs) = convert_messages(messages);
        assert_eq!(msgs.len(), 1);
        // Text rides as the first block, the image as a base64 `image` block.
        let value = serde_json::to_value(&msgs[0]).expect("serialize");
        let blocks = value["content"].as_array().expect("content blocks");
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0]["type"], "text");
        assert_eq!(blocks[0]["text"], "what is this?");
        assert_eq!(blocks[1]["type"], "image");
        assert_eq!(blocks[1]["source"]["type"], "base64");
        assert_eq!(blocks[1]["source"]["media_type"], "image/png");
        assert_eq!(blocks[1]["source"]["data"], "AQIDBA==");
    }

    #[test]
    fn test_convert_messages_text_only_user_stays_a_string() {
        let messages = vec![ChatMessage::user("just text")];
        let (_system, msgs) = convert_messages(messages);
        let value = serde_json::to_value(&msgs[0]).expect("serialize");
        // No inline images → compact string content, not a blocks array.
        assert_eq!(value["content"], "just text");
    }

    #[test]
    fn test_convert_messages_tool_calls() {
        let tool_calls = vec![ToolCall {
            id: "call_1".to_string(),
            name: "search".to_string(),
            arguments: serde_json::json!({"q": "test"}),
            reasoning: None,
            signature: None,
            arguments_parse_error: None,
        }];
        let messages = vec![
            ChatMessage::user("Search for test"),
            ChatMessage::assistant_with_tool_calls(Some("Let me search.".to_string()), tool_calls),
            ChatMessage::tool_result("call_1", "search", "found it"),
        ];
        let (system, msgs) = convert_messages(messages);
        assert!(system.is_none());
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[1].role, "assistant");
        // Tool result should be a user message
        assert_eq!(msgs[2].role, "user");
    }

    #[test]
    fn test_extract_response_text_only() {
        let response = AnthropicResponse {
            content: vec![AnthropicResponseBlock::Text {
                text: "Hello!".to_string(),
            }],
            stop_reason: Some("end_turn".to_string()),
            usage: AnthropicUsage {
                input_tokens: 10,
                output_tokens: 5,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 0,
            },
        };
        let extracted = extract_response_content(&response);
        assert_eq!(extracted.content, Some("Hello!".to_string()));
        assert!(extracted.tool_calls.is_empty());
    }

    #[test]
    fn test_extract_response_with_tool_use() {
        let response = AnthropicResponse {
            content: vec![
                AnthropicResponseBlock::Text {
                    text: "Let me search.".to_string(),
                },
                AnthropicResponseBlock::ToolUse {
                    id: "call_1".to_string(),
                    name: "search".to_string(),
                    input: serde_json::json!({"q": "test"}),
                },
            ],
            stop_reason: Some("tool_use".to_string()),
            usage: AnthropicUsage {
                input_tokens: 20,
                output_tokens: 15,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 0,
            },
        };
        let extracted = extract_response_content(&response);
        assert_eq!(extracted.content, Some("Let me search.".to_string()));
        assert_eq!(extracted.tool_calls.len(), 1);
        assert_eq!(extracted.tool_calls[0].name, "search");
    }

    #[test]
    fn test_extract_response_preserves_thinking_as_reasoning() {
        let response = AnthropicResponse {
            content: vec![
                AnthropicResponseBlock::Thinking {
                    thinking: Some("Raw thinking".to_string()),
                    summary: Some("Summarized thinking".to_string()),
                    _signature: Some("sig".to_string()),
                },
                AnthropicResponseBlock::Text {
                    text: "Done.".to_string(),
                },
            ],
            stop_reason: Some("end_turn".to_string()),
            usage: AnthropicUsage {
                input_tokens: 20,
                output_tokens: 15,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 0,
            },
        };
        let extracted = extract_response_content(&response);
        assert_eq!(extracted.content, Some("Done.".to_string()));
        assert_eq!(extracted.reasoning, Some("Summarized thinking".to_string()));
    }

    #[test]
    fn test_extract_response_uses_thinking_when_summary_absent() {
        let response = AnthropicResponse {
            content: vec![
                AnthropicResponseBlock::Thinking {
                    thinking: Some("Raw thinking fallback".to_string()),
                    summary: None,
                    _signature: Some("sig".to_string()),
                },
                AnthropicResponseBlock::Text {
                    text: "Done.".to_string(),
                },
            ],
            stop_reason: Some("end_turn".to_string()),
            usage: AnthropicUsage {
                input_tokens: 20,
                output_tokens: 15,
                cache_creation_input_tokens: 0,
                cache_read_input_tokens: 0,
            },
        };

        let extracted = extract_response_content(&response);

        assert_eq!(extracted.content, Some("Done.".to_string()));
        assert_eq!(
            extracted.reasoning,
            Some("Raw thinking fallback".to_string())
        );
    }

    /// Regression test for #1136: token field must be mutable via RwLock
    /// so that a refreshed token persists across subsequent requests.
    #[test]
    fn test_token_update_persists() {
        let original = SecretString::from("old_token".to_string());
        let token = std::sync::RwLock::new(original);

        // Read the original
        assert_eq!(token.read().unwrap().expose_secret(), "old_token");

        // Simulate a successful refresh
        let refreshed = SecretString::from("new_token".to_string());
        *token.write().unwrap() = refreshed;

        // Subsequent reads see the updated token
        assert_eq!(token.read().unwrap().expose_secret(), "new_token");
    }
}
