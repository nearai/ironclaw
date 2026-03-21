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
    pub fn new(config: &RegistryProviderConfig) -> Result<Self, LlmError> {
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
            .timeout(std::time::Duration::from_secs(120))
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
        let token = self.token_manager.get_token().await.map_err(|e| {
            tracing::warn!(error = %e, "Copilot: token exchange failed");
            LlmError::AuthFailed {
                provider: "github_copilot".to_string(),
            }
        })?;

        let mut request = self
            .client
            .post(&url)
            .bearer_auth(token.expose_secret())
            .header("Content-Type", "application/json");

        // Inject Copilot identity headers
        for (key, value) in &self.extra_headers {
            request = request.header(key.as_str(), value.as_str());
        }

        let response = request.json(body).send().await.map_err(|e| {
            tracing::warn!(error = %e, "Copilot: HTTP request failed");
            LlmError::RequestFailed {
                provider: "github_copilot".to_string(),
                reason: e.to_string(),
            }
        })?;

        let status = response.status();

        if !status.is_success() {
            let retry_after = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok())
                .map(std::time::Duration::from_secs);

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
                // performs a fresh token exchange.
                tracing::warn!(
                    "Copilot: 401 Unauthorized — invalidating cached session token for retry"
                );
                self.token_manager.invalidate().await;
                return Err(LlmError::AuthFailed {
                    provider: "github_copilot".to_string(),
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
            tools: None,
            tool_choice: None,
        };

        let response: OpenAiResponse = self.send_request(&request).await?;
        let choice =
            response
                .choices
                .into_iter()
                .next()
                .ok_or_else(|| LlmError::InvalidResponse {
                    provider: "github_copilot".to_string(),
                    reason: "No choices in response".to_string(),
                })?;

        let (content, _tool_calls) = extract_choice_content(&choice);

        let finish_reason = match choice.finish_reason.as_deref() {
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
            tools: if tools.is_empty() { None } else { Some(tools) },
            tool_choice,
        };

        let response: OpenAiResponse = self.send_request(&request).await?;
        let choice =
            response
                .choices
                .into_iter()
                .next()
                .ok_or_else(|| LlmError::InvalidResponse {
                    provider: "github_copilot".to_string(),
                    reason: "No choices in response".to_string(),
                })?;

        let (content, tool_calls) = extract_choice_content(&choice);

        let finish_reason = match choice.finish_reason.as_deref() {
            Some("stop") => FinishReason::Stop,
            Some("length") => FinishReason::Length,
            Some("tool_calls") => FinishReason::ToolUse,
            Some("content_filter") => FinishReason::ContentFilter,
            _ => {
                if !tool_calls.is_empty() {
                    FinishReason::ToolUse
                } else {
                    FinishReason::Unknown
                }
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
            },
            Role::User => {
                let content = if msg.content_parts.is_empty() {
                    Some(OpenAiContent::Text(msg.content))
                } else {
                    let mut parts = vec![OpenAiContentPart::Text { text: msg.content }];
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
                OpenAiMessage {
                    role: "assistant".to_string(),
                    content,
                    tool_calls,
                    tool_call_id: None,
                    name: None,
                }
            }
            Role::Tool => OpenAiMessage {
                role: "tool".to_string(),
                content: Some(OpenAiContent::Text(msg.content)),
                tool_calls: None,
                tool_call_id: msg.tool_call_id,
                name: msg.name,
            },
        })
        .collect()
}

/// Extract text and tool calls from an OpenAI response choice.
fn extract_choice_content(choice: &OpenAiChoice) -> (Option<String>, Vec<ToolCall>) {
    let content = choice.message.content.clone();
    let tool_calls = choice
        .message
        .tool_calls
        .as_ref()
        .map(|calls| {
            calls
                .iter()
                .map(|tc| ToolCall {
                    id: tc.id.clone(),
                    name: tc.function.name.clone(),
                    arguments: serde_json::from_str(&tc.function.arguments)
                        .unwrap_or(serde_json::Value::Object(serde_json::Map::new())),
                })
                .collect()
        })
        .unwrap_or_default();

    (content, tool_calls)
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
    fn test_extract_choice_text_only() {
        let choice = OpenAiChoice {
            message: OpenAiResponseMessage {
                content: Some("Hello!".to_string()),
                tool_calls: None,
            },
            finish_reason: Some("stop".to_string()),
        };
        let (content, tool_calls) = extract_choice_content(&choice);
        assert_eq!(content, Some("Hello!".to_string()));
        assert!(tool_calls.is_empty());
    }

    #[test]
    fn test_extract_choice_with_tool_calls() {
        let choice = OpenAiChoice {
            message: OpenAiResponseMessage {
                content: Some("Let me search.".to_string()),
                tool_calls: Some(vec![OpenAiResponseToolCall {
                    id: "call_1".to_string(),
                    function: OpenAiResponseFunction {
                        name: "search".to_string(),
                        arguments: r#"{"q":"test"}"#.to_string(),
                    },
                }]),
            },
            finish_reason: Some("tool_calls".to_string()),
        };
        let (content, tool_calls) = extract_choice_content(&choice);
        assert_eq!(content, Some("Let me search.".to_string()));
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0].name, "search");
        assert_eq!(tool_calls[0].arguments["q"], "test");
    }
}
