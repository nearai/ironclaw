//! GitHub Copilot provider (direct HTTP with token exchange).
//!
//! The GitHub Copilot API at `api.githubcopilot.com` speaks OpenAI Chat
//! Completions format but requires a two-step authentication flow:
//! 1. A long-lived GitHub OAuth token (from device login or IDE sign-in)
//! 2. A short-lived Copilot session token (exchanged via GitHub API)
//!
//! The standard OpenAI rig-core client sends `Authorization: Bearer <token>`
//! with the raw OAuth token, which gets rejected with "Authorization header
//! is badly formatted". This provider handles the token exchange transparently.

use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use reqwest::Client;
use rust_decimal::Decimal;
use secrecy::ExposeSecret;
use serde::{Deserialize, Serialize};

use crate::llm::config::RegistryProviderConfig;
use crate::llm::costs;
use crate::llm::error::LlmError;
use crate::llm::github_copilot_auth::CopilotTokenManager;
use crate::llm::provider::{
    ChatMessage, CompletionRequest, CompletionResponse, ContentPart, FinishReason, LlmProvider,
    Role, ToolCall, ToolCompletionRequest, ToolCompletionResponse,
    strip_unsupported_completion_params, strip_unsupported_tool_params,
};

/// GitHub Copilot provider with automatic token exchange.
pub struct GithubCopilotProvider {
    client: Client,
    token_manager: Arc<CopilotTokenManager>,
    model: String,
    base_url: String,
    active_model: std::sync::RwLock<String>,
    extra_headers: Vec<(String, String)>,
    /// Parameter names that this provider does not support.
    unsupported_params: HashSet<String>,
}

impl GithubCopilotProvider {
    pub fn new(
        config: &RegistryProviderConfig,
        request_timeout_secs: u64,
    ) -> Result<Self, LlmError> {
        let oauth_token = config
            .api_key
            .as_ref()
            .map(|k| k.expose_secret().to_string())
            .ok_or_else(|| {
                tracing::error!("No API key configured for github_copilot — check GITHUB_COPILOT_TOKEN env var or secrets store");
                LlmError::AuthFailed {
                    provider: "github_copilot".to_string(),
                }
            })?;

        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(request_timeout_secs))
            .build()
            .map_err(|e| LlmError::RequestFailed {
                provider: "github_copilot".to_string(),
                reason: format!("Failed to build HTTP client: {e}"),
            })?;

        let token_manager = Arc::new(CopilotTokenManager::new(client.clone(), oauth_token));

        let base_url = if config.base_url.is_empty() {
            "https://api.githubcopilot.com".to_string()
        } else {
            config.base_url.clone()
        };

        let active_model = std::sync::RwLock::new(config.model.clone());
        let unsupported_params: HashSet<String> =
            config.unsupported_params.iter().cloned().collect();

        Ok(Self {
            client,
            token_manager,
            model: config.model.clone(),
            base_url,
            active_model,
            extra_headers: config.extra_headers.clone(),
            unsupported_params,
        })
    }

    fn api_url(&self) -> String {
        let base = self.base_url.trim_end_matches('/');
        format!("{base}/chat/completions")
    }

    /// Strip unsupported fields from a `CompletionRequest` in place.
    fn strip_unsupported_completion_params(&self, req: &mut CompletionRequest) {
        strip_unsupported_completion_params(&self.unsupported_params, req);
    }

    /// Strip unsupported fields from a `ToolCompletionRequest` in place.
    fn strip_unsupported_tool_params(&self, req: &mut ToolCompletionRequest) {
        strip_unsupported_tool_params(&self.unsupported_params, req);
    }

    async fn send_request<R: for<'de> Deserialize<'de>>(
        &self,
        body: &impl Serialize,
    ) -> Result<R, LlmError> {
        let url = self.api_url();
        // Distinguish permanent auth errors (non-retryable) from transient
        // network failures (retryable) so RetryProvider handles them correctly.
        let token = self.token_manager.get_token().await.map_err(|e| {
            tracing::warn!(error = %e, "Copilot: token exchange failed");
            match &e {
                crate::llm::github_copilot_auth::GithubCopilotAuthError::AccessDenied
                | crate::llm::github_copilot_auth::GithubCopilotAuthError::Expired => {
                    LlmError::AuthFailed {
                        provider: "github_copilot".to_string(),
                    }
                }
                _ => LlmError::RequestFailed {
                    provider: "github_copilot".to_string(),
                    reason: format!("Token exchange failed: {e}"),
                },
            }
        })?;

        let mut request = self
            .client
            .post(&url)
            .bearer_auth(token.expose_secret())
            .header("Content-Type", "application/json")
            .header("Openai-Intent", "conversation-edits")
            .header("x-initiator", "user");

        // Inject Copilot identity headers
        for (key, value) in &self.extra_headers {
            request = request.header(key.as_str(), value.as_str());
        }

        // Claude models require the anthropic-beta header for structured
        // reasoning fields (reasoning_text/reasoning_opaque) instead of
        // raw <think> tags in content.
        let is_claude = self.model.to_lowercase().contains("claude");
        if is_claude {
            request =
                request.header("anthropic-beta", "interleaved-thinking-2025-05-14");
        }
        tracing::debug!(
            model = %self.model,
            is_claude = is_claude,
            "Copilot: sending request"
        );

        let response = request.json(body).send().await.map_err(|e| {
            tracing::warn!(error = %e, "Copilot: HTTP request failed");
            LlmError::RequestFailed {
                provider: "github_copilot".to_string(),
                reason: e.to_string(),
            }
        })?;

        let status = response.status();

        if !status.is_success() {
            // Use shared retry-after parser (supports HTTP-date, default 60s)
            let retry_after = Some(crate::llm::retry::parse_retry_after(
                response.headers().get(reqwest::header::RETRY_AFTER),
            ));

            let response_text = response
                .text()
                .await
                .unwrap_or_else(|e| format!("(failed to read error body: {e})"));

            tracing::warn!(
                status = %status,
                body = %crate::agent::truncate_for_preview(&response_text, 256),
                "Copilot: API error response"
            );

            if status.as_u16() == 401 {
                // Invalidate the cached session token so the next attempt
                // (driven by RetryProvider) gets a fresh one. We don't retry
                // inline to avoid nested retries with the outer RetryProvider.
                tracing::warn!("Copilot: 401 Unauthorized — invalidating session token for retry");
                self.token_manager.invalidate().await;
                return Err(LlmError::RequestFailed {
                    provider: "github_copilot".to_string(),
                    reason: "HTTP 401 Unauthorized".to_string(),
                });
            }
            if status.as_u16() == 429 {
                tracing::warn!(retry_after = ?retry_after, "Copilot: rate limited");
                return Err(LlmError::RateLimited {
                    provider: "github_copilot".to_string(),
                    retry_after,
                });
            }
            let truncated = crate::agent::truncate_for_preview(&response_text, 512);
            return Err(LlmError::RequestFailed {
                provider: "github_copilot".to_string(),
                reason: format!("HTTP {status}: {truncated}"),
            });
        }

        let response_text = response.text().await.map_err(|e| LlmError::RequestFailed {
            provider: "github_copilot".to_string(),
            reason: format!("Failed to read response body: {e}"),
        })?;

        tracing::trace!(
            body_len = response_text.len(),
            body_preview = %crate::agent::truncate_for_preview(&response_text, 1024),
            "Copilot: raw response body"
        );

        serde_json::from_str(&response_text).map_err(|e| {
            let truncated = crate::agent::truncate_for_preview(&response_text, 512);
            tracing::warn!(
                error = %e,
                body = %truncated,
                "Copilot: failed to parse response JSON"
            );
            LlmError::InvalidResponse {
                provider: "github_copilot".to_string(),
                reason: format!("JSON parse error: {e}. Raw: {truncated}"),
            }
        })
    }
}

#[async_trait]
impl LlmProvider for GithubCopilotProvider {
    async fn complete(&self, mut req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let model = req.model.take().unwrap_or_else(|| self.active_model_name());
        self.strip_unsupported_completion_params(&mut req);
        let messages = convert_messages(req.messages);

        let request = OpenAiRequest {
            model,
            messages,
            max_tokens: req.max_tokens,
            temperature: req.temperature,
            stop: req.stop_sequences,
            tools: None,
            tool_choice: None,
        };

        let response: OpenAiResponse = self.send_request(&request).await?;
        if response.choices.is_empty() {
            return Err(LlmError::EmptyResponse {
                provider: "github_copilot".to_string(),
            });
        }

        let (content, _tool_calls, _provider_metadata) = merge_choices(&response.choices);

        let finish_reason = match response.choices[0].finish_reason.as_deref() {
            Some("stop") => FinishReason::Stop,
            Some("length") => FinishReason::Length,
            Some("tool_calls") => FinishReason::ToolUse,
            Some("content_filter") => FinishReason::ContentFilter,
            _ => FinishReason::Unknown,
        };

        Ok(CompletionResponse {
            content: content.unwrap_or_default(),
            finish_reason,
            input_tokens: response
                .usage
                .as_ref()
                .map(|u| u.prompt_tokens)
                .unwrap_or(0),
            output_tokens: response
                .usage
                .as_ref()
                .map(|u| u.completion_tokens)
                .unwrap_or(0),
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: 0,
        })
    }

    async fn complete_with_tools(
        &self,
        mut req: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        let model = req.model.take().unwrap_or_else(|| self.active_model_name());
        self.strip_unsupported_tool_params(&mut req);
        let messages = convert_messages(req.messages);

        let tools: Vec<OpenAiTool> = req
            .tools
            .into_iter()
            .map(|t| OpenAiTool {
                tool_type: "function".to_string(),
                function: OpenAiFunction {
                    name: t.name,
                    description: t.description,
                    parameters: t.parameters,
                },
            })
            .collect();

        let tool_choice = req.tool_choice.map(|tc| match tc.as_str() {
            "auto" | "required" | "none" => serde_json::Value::String(tc),
            specific => serde_json::json!({
                "type": "function",
                "function": {"name": specific}
            }),
        });

        let request = OpenAiRequest {
            model,
            messages,
            max_tokens: req.max_tokens,
            temperature: req.temperature,
            stop: req.stop_sequences,
            tools: if tools.is_empty() { None } else { Some(tools) },
            tool_choice,
        };

        let response: OpenAiResponse = self.send_request(&request).await?;
        if response.choices.is_empty() {
            return Err(LlmError::EmptyResponse {
                provider: "github_copilot".to_string(),
            });
        }

        let (content, tool_calls, provider_metadata) = merge_choices(&response.choices);

        // Determine finish_reason from all choices — prefer tool_calls > stop
        let finish_reason = if !tool_calls.is_empty() {
            FinishReason::ToolUse
        } else {
            // Use the first choice's finish_reason as fallback
            match response.choices[0].finish_reason.as_deref() {
                Some("stop") => FinishReason::Stop,
                Some("length") => FinishReason::Length,
                Some("tool_calls") => FinishReason::ToolUse,
                Some("content_filter") => FinishReason::ContentFilter,
                _ => FinishReason::Unknown,
            }
        };

        Ok(ToolCompletionResponse {
            content,
            tool_calls,
            finish_reason,
            input_tokens: response
                .usage
                .as_ref()
                .map(|u| u.prompt_tokens)
                .unwrap_or(0),
            output_tokens: response
                .usage
                .as_ref()
                .map(|u| u.completion_tokens)
                .unwrap_or(0),
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: 0,
            provider_metadata,
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

// --- OpenAI Chat Completions API types ---

#[derive(Debug, Serialize)]
struct OpenAiRequest {
    model: String,
    messages: Vec<OpenAiMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stop: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<OpenAiTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
struct OpenAiMessage {
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<OpenAiContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<OpenAiToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    /// Copilot: reasoning text for multi-turn Claude conversations.
    /// Only sent when `reasoning_opaque` is also present.
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_text: Option<String>,
    /// Copilot: opaque reasoning blob for multi-turn continuity.
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_opaque: Option<String>,
}

/// OpenAI content can be a plain string or an array of parts (for multimodal).
#[derive(Debug, Serialize)]
#[serde(untagged)]
enum OpenAiContent {
    Text(String),
    Parts(Vec<OpenAiContentPart>),
}

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
enum OpenAiContentPart {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image_url")]
    ImageUrl { image_url: OpenAiImageUrl },
}

#[derive(Debug, Serialize)]
struct OpenAiImageUrl {
    url: String,
}

#[derive(Debug, Serialize)]
struct OpenAiToolCall {
    id: String,
    #[serde(rename = "type")]
    call_type: String,
    function: OpenAiToolCallFunction,
}

#[derive(Debug, Serialize)]
struct OpenAiToolCallFunction {
    name: String,
    arguments: String,
}

#[derive(Debug, Serialize)]
struct OpenAiTool {
    #[serde(rename = "type")]
    tool_type: String,
    function: OpenAiFunction,
}

#[derive(Debug, Serialize)]
struct OpenAiFunction {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct OpenAiResponse {
    choices: Vec<OpenAiChoice>,
    #[serde(default)]
    usage: Option<OpenAiUsage>,
}

#[derive(Debug, Deserialize)]
struct OpenAiChoice {
    message: OpenAiResponseMessage,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAiResponseMessage {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<OpenAiResponseToolCall>>,
    /// Copilot-specific: structured reasoning text from Claude models.
    #[serde(default)]
    reasoning_text: Option<String>,
    /// Copilot-specific: opaque blob for multi-turn reasoning continuity.
    #[serde(default)]
    reasoning_opaque: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenAiResponseToolCall {
    id: String,
    function: OpenAiResponseFunction,
}

#[derive(Debug, Deserialize)]
struct OpenAiResponseFunction {
    name: String,
    arguments: String,
}

#[derive(Debug, Deserialize)]
struct OpenAiUsage {
    #[serde(default)]
    prompt_tokens: u32,
    #[serde(default)]
    completion_tokens: u32,
}

/// Convert IronClaw messages to OpenAI Chat Completions format.
fn convert_messages(messages: Vec<ChatMessage>) -> Vec<OpenAiMessage> {
    messages
        .into_iter()
        .map(|msg| match msg.role {
            Role::System => OpenAiMessage {
                role: "system".to_string(),
                content: Some(OpenAiContent::Text(msg.content)),
                tool_calls: None,
                tool_call_id: None,
                name: None,
                reasoning_text: None,
                reasoning_opaque: None,
            },
            Role::User => {
                let content = if msg.content_parts.is_empty() {
                    Some(OpenAiContent::Text(msg.content))
                } else {
                    let mut parts = Vec::with_capacity(1 + msg.content_parts.len());
                    if !msg.content.is_empty() {
                        parts.push(OpenAiContentPart::Text { text: msg.content });
                    }
                    for part in msg.content_parts {
                        match part {
                            ContentPart::Text { text } => {
                                parts.push(OpenAiContentPart::Text { text });
                            }
                            ContentPart::ImageUrl { image_url } => {
                                parts.push(OpenAiContentPart::ImageUrl {
                                    image_url: OpenAiImageUrl { url: image_url.url },
                                });
                            }
                        }
                    }
                    Some(OpenAiContent::Parts(parts))
                };
                OpenAiMessage {
                    role: "user".to_string(),
                    content,
                    tool_calls: None,
                    tool_call_id: None,
                    name: None,
                    reasoning_text: None,
                    reasoning_opaque: None,
                }
            }
            Role::Assistant => {
                let tool_calls = msg.tool_calls.map(|calls| {
                    calls
                        .into_iter()
                        .map(|tc| OpenAiToolCall {
                            id: tc.id,
                            call_type: "function".to_string(),
                            function: OpenAiToolCallFunction {
                                name: tc.name,
                                arguments: tc.arguments.to_string(),
                            },
                        })
                        .collect()
                });
                let content = if msg.content.is_empty() {
                    None
                } else {
                    Some(OpenAiContent::Text(msg.content))
                };

                // Round-trip reasoning fields from provider_metadata.
                // Per Copilot API: only send reasoning_text when
                // reasoning_opaque is also present.
                let reasoning_opaque = msg
                    .provider_metadata
                    .get("reasoning_opaque")
                    .cloned();
                let reasoning_text = if reasoning_opaque.is_some() {
                    msg.provider_metadata
                        .get("reasoning_text")
                        .cloned()
                } else {
                    None
                };

                OpenAiMessage {
                    role: "assistant".to_string(),
                    content,
                    tool_calls,
                    tool_call_id: None,
                    name: None,
                    reasoning_text,
                    reasoning_opaque,
                }
            }
            Role::Tool => OpenAiMessage {
                role: "tool".to_string(),
                content: Some(OpenAiContent::Text(msg.content)),
                tool_calls: None,
                tool_call_id: msg.tool_call_id,
                name: msg.name,
                reasoning_text: None,
                reasoning_opaque: None,
            },
        })
        .collect()
}

/// Merge content, tool calls, and provider metadata from ALL response choices.
///
/// The Copilot API (especially for Claude models) sometimes splits responses
/// across multiple choices: one with text/thinking content, another with tool
/// calls. This function merges them all into a single result.
///
/// When the Copilot API returns `reasoning_text`/`reasoning_opaque` (Claude models),
/// these are surfaced in the returned metadata map so they can be stored on
/// `ChatMessage::provider_metadata` for round-tripping on subsequent turns.
fn merge_choices(
    choices: &[OpenAiChoice],
) -> (Option<String>, Vec<ToolCall>, std::collections::HashMap<String, String>) {
    let mut merged_content: Option<String> = None;
    let mut merged_tool_calls: Vec<ToolCall> = Vec::new();
    let mut provider_metadata = std::collections::HashMap::new();
    // Track the best finish_reason across choices (tool_calls > stop > others)
    let mut saw_tool_calls_finish = false;

    for (idx, choice) in choices.iter().enumerate() {
        // Merge content: concatenate non-empty content from all choices
        if let Some(ref c) = choice.message.content
            && !c.is_empty()
        {
            match &mut merged_content {
                Some(existing) => {
                    existing.push('\n');
                    existing.push_str(c);
                }
                None => {
                    merged_content = Some(c.clone());
                }
            }
        }

        // Merge tool calls from all choices
        if let Some(ref calls) = choice.message.tool_calls {
            let reasoning_for_tools = choice.message.reasoning_text.clone();
            for tc in calls {
                merged_tool_calls.push(ToolCall {
                    id: tc.id.clone(),
                    name: tc.function.name.clone(),
                    arguments: serde_json::from_str(&tc.function.arguments)
                        .unwrap_or(serde_json::Value::Object(serde_json::Map::new())),
                    reasoning: reasoning_for_tools.clone(),
                });
            }
        }

        // Capture provider metadata from any choice that has it
        if let Some(ref rt) = choice.message.reasoning_text
            && !rt.is_empty()
        {
            tracing::debug!(
                reasoning_text_len = rt.len(),
                choice_idx = idx,
                "Copilot: received reasoning_text from model"
            );
            provider_metadata.insert("reasoning_text".to_string(), rt.clone());
        }
        if let Some(ref ro) = choice.message.reasoning_opaque
            && !ro.is_empty()
        {
            tracing::debug!(
                reasoning_opaque_len = ro.len(),
                choice_idx = idx,
                "Copilot: received reasoning_opaque from model"
            );
            provider_metadata.insert("reasoning_opaque".to_string(), ro.clone());
        }

        if choice.finish_reason.as_deref() == Some("tool_calls") {
            saw_tool_calls_finish = true;
        }
    }

    if choices.len() > 1 {
        tracing::debug!(
            num_choices = choices.len(),
            merged_content_len = merged_content.as_ref().map(|c| c.len()).unwrap_or(0),
            merged_tool_calls = merged_tool_calls.len(),
            saw_tool_calls_finish,
            "Copilot: merged multiple response choices"
        );
    }

    (merged_content, merged_tool_calls, provider_metadata)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_convert_messages_basic() {
        let messages = vec![
            ChatMessage::system("You are helpful."),
            ChatMessage::user("Hello"),
            ChatMessage::assistant("Hi there!"),
        ];
        let converted = convert_messages(messages);
        assert_eq!(converted.len(), 3);
        assert_eq!(converted[0].role, "system");
        assert_eq!(converted[1].role, "user");
        assert_eq!(converted[2].role, "assistant");
    }

    #[test]
    fn test_convert_messages_tool_calls() {
        let tool_calls = vec![ToolCall {
            id: "call_1".to_string(),
            name: "search".to_string(),
            arguments: serde_json::json!({"q": "test"}),
            reasoning: None,
        }];
        let messages = vec![
            ChatMessage::user("Search"),
            ChatMessage::assistant_with_tool_calls(Some("Searching...".to_string()), tool_calls),
            ChatMessage::tool_result("call_1", "search", "found it"),
        ];
        let converted = convert_messages(messages);
        assert_eq!(converted.len(), 3);
        assert!(converted[1].tool_calls.is_some());
        assert_eq!(converted[2].role, "tool");
        assert_eq!(converted[2].tool_call_id, Some("call_1".to_string()));
    }

    #[test]
    fn test_merge_choices_text_only() {
        let choices = vec![OpenAiChoice {
            message: OpenAiResponseMessage {
                content: Some("Hello!".to_string()),
                tool_calls: None,
                reasoning_text: None,
                reasoning_opaque: None,
            },
            finish_reason: Some("stop".to_string()),
        }];
        let (content, tool_calls, _provider_metadata) = merge_choices(&choices);
        assert_eq!(content, Some("Hello!".to_string()));
        assert!(tool_calls.is_empty());
    }

    #[test]
    fn test_merge_choices_with_tool_calls() {
        let choices = vec![OpenAiChoice {
            message: OpenAiResponseMessage {
                content: Some("Let me search.".to_string()),
                tool_calls: Some(vec![OpenAiResponseToolCall {
                    id: "call_1".to_string(),
                    function: OpenAiResponseFunction {
                        name: "search".to_string(),
                        arguments: r#"{"q":"test"}"#.to_string(),
                    },
                }]),
                reasoning_text: None,
                reasoning_opaque: None,
            },
            finish_reason: Some("tool_calls".to_string()),
        }];
        let (content, tool_calls, _provider_metadata) = merge_choices(&choices);
        assert_eq!(content, Some("Let me search.".to_string()));
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].name, "search");
        assert_eq!(tool_calls[0].arguments["q"], "test");
    }

    #[test]
    fn test_merge_choices_multi_choice_copilot_style() {
        // Copilot API returns text/thinking in one choice, tool calls in another
        let choices = vec![
            OpenAiChoice {
                message: OpenAiResponseMessage {
                    content: Some("On it, spinning up a sandbox.".to_string()),
                    tool_calls: None,
                    reasoning_text: None,
                    reasoning_opaque: None,
                },
                finish_reason: Some("tool_calls".to_string()),
            },
            OpenAiChoice {
                message: OpenAiResponseMessage {
                    content: None,
                    tool_calls: Some(vec![OpenAiResponseToolCall {
                        id: "call_abc".to_string(),
                        function: OpenAiResponseFunction {
                            name: "create_job".to_string(),
                            arguments: r#"{"task":"hello world"}"#.to_string(),
                        },
                    }]),
                    reasoning_text: None,
                    reasoning_opaque: None,
                },
                finish_reason: Some("tool_calls".to_string()),
            },
        ];
        let (content, tool_calls, _provider_metadata) = merge_choices(&choices);
        assert_eq!(content, Some("On it, spinning up a sandbox.".to_string()));
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].name, "create_job");
        assert_eq!(tool_calls[0].id, "call_abc");
    }
}
