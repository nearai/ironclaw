//! NEAR AI Chat API provider implementation.
//!
//! This provider uses the NEAR AI chat-api which provides a unified interface
//! to multiple LLM models (OpenAI, Anthropic, etc.) with user authentication.

use std::sync::Arc;

use async_trait::async_trait;
use reqwest::Client;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;
use secrecy::ExposeSecret;
use serde::{Deserialize, Serialize};

use crate::config::NearAiConfig;
use crate::error::LlmError;
use crate::llm::provider::{
    ChatMessage, CompletionRequest, CompletionResponse, FinishReason, LlmProvider, Role, ToolCall,
    ToolCompletionRequest, ToolCompletionResponse,
};
use crate::llm::session::SessionManager;

/// Information about an available model from NEAR AI API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    /// Model identifier.
    #[serde(alias = "id", alias = "model")]
    pub name: String,
    /// Optional provider name.
    #[serde(default)]
    pub provider: Option<String>,
}

/// NEAR AI Chat API provider.
pub struct NearAiProvider {
    client: Client,
    config: NearAiConfig,
    session: Arc<SessionManager>,
}

impl NearAiProvider {
    /// Create a new NEAR AI provider with a session manager.
    pub fn new(config: NearAiConfig, session: Arc<SessionManager>) -> Self {
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .unwrap_or_else(|_| Client::new());

        Self {
            client,
            config,
            session,
        }
    }

    fn api_url(&self, path: &str) -> String {
        format!(
            "{}/v1/{}",
            self.config.base_url,
            path.trim_start_matches('/')
        )
    }

    /// Fetch available models from the NEAR AI API.
    pub async fn list_models(&self) -> Result<Vec<ModelInfo>, LlmError> {
        use secrecy::ExposeSecret;

        let token = self.session.get_token().await?;
        let url = self.api_url("model/list");

        tracing::debug!("Fetching models from: {}", url);

        let response = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", token.expose_secret()))
            .send()
            .await
            .map_err(|e| LlmError::RequestFailed {
                provider: "nearai".to_string(),
                reason: format!("Failed to fetch models: {}", e),
            })?;

        let status = response.status();
        let response_text = response.text().await.unwrap_or_default();

        if !status.is_success() {
            // Check for session expiration
            if status.as_u16() == 401 {
                return Err(LlmError::SessionExpired {
                    provider: "nearai".to_string(),
                });
            }

            return Err(LlmError::RequestFailed {
                provider: "nearai".to_string(),
                reason: format!("HTTP {}: {}", status, response_text),
            });
        }

        // Parse the response - NEAR AI returns {"limit": N, "models": [...]}
        // Each model object may have the name in different fields
        #[derive(Deserialize)]
        struct ModelMetadata {
            #[serde(default)]
            name: Option<String>,
            #[serde(default, alias = "modelName", alias = "model_name")]
            model_name: Option<String>,
        }

        #[derive(Deserialize)]
        struct ModelEntry {
            #[serde(default)]
            name: Option<String>,
            #[serde(default)]
            id: Option<String>,
            #[serde(default)]
            model: Option<String>,
            #[serde(default, alias = "modelName", alias = "model_name")]
            model_name: Option<String>,
            #[serde(default, alias = "modelId", alias = "model_id")]
            model_id: Option<String>,
            #[serde(default)]
            metadata: Option<ModelMetadata>,
        }

        impl ModelEntry {
            fn get_name(&self) -> Option<String> {
                self.name
                    .clone()
                    .or_else(|| self.id.clone())
                    .or_else(|| self.model.clone())
                    .or_else(|| self.model_name.clone())
                    .or_else(|| self.model_id.clone())
                    .or_else(|| self.metadata.as_ref().and_then(|m| m.name.clone()))
                    .or_else(|| self.metadata.as_ref().and_then(|m| m.model_name.clone()))
            }
        }

        #[derive(Deserialize)]
        struct ModelsResponse {
            #[serde(default)]
            models: Option<Vec<ModelEntry>>,
            #[serde(default)]
            data: Option<Vec<ModelEntry>>,
        }

        if let Ok(resp) = serde_json::from_str::<ModelsResponse>(&response_text) {
            if let Some(entries) = resp.models.or(resp.data) {
                let models: Vec<ModelInfo> = entries
                    .into_iter()
                    .filter_map(|e| e.get_name().map(|name| ModelInfo { name, provider: None }))
                    .collect();
                if !models.is_empty() {
                    return Ok(models);
                }
            }
        }

        // Try direct array format
        if let Ok(entries) = serde_json::from_str::<Vec<ModelEntry>>(&response_text) {
            let models: Vec<ModelInfo> = entries
                .into_iter()
                .filter_map(|e| e.get_name().map(|name| ModelInfo { name, provider: None }))
                .collect();
            if !models.is_empty() {
                return Ok(models);
            }
        }

        // Couldn't find model names in response
        Err(LlmError::InvalidResponse {
            provider: "nearai".to_string(),
            reason: format!(
                "No model names found in response: {}",
                &response_text[..response_text.len().min(300)]
            ),
        })
    }

    /// Send a request with automatic session renewal on 401.
    async fn send_request<T: Serialize + std::fmt::Debug, R: for<'de> Deserialize<'de>>(
        &self,
        path: &str,
        body: &T,
    ) -> Result<R, LlmError> {
        // Try the request, handling session expiration
        match self.send_request_inner(path, body).await {
            Ok(result) => Ok(result),
            Err(LlmError::SessionExpired { .. }) => {
                // Session expired, attempt renewal and retry once
                self.session.handle_auth_failure().await?;
                self.send_request_inner(path, body).await
            }
            Err(e) => Err(e),
        }
    }

    /// Inner request implementation without retry logic.
    async fn send_request_inner<T: Serialize + std::fmt::Debug, R: for<'de> Deserialize<'de>>(
        &self,
        path: &str,
        body: &T,
    ) -> Result<R, LlmError> {
        let url = self.api_url(path);
        let token = self.session.get_token().await?;

        tracing::debug!("Sending request to NEAR AI: {}", url);
        tracing::debug!("Request body: {:?}", body);

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", token.expose_secret()))
            .header("Content-Type", "application/json")
            .json(body)
            .send()
            .await
            .map_err(|e| {
                tracing::error!("NEAR AI request failed: {}", e);
                e
            })?;

        let status = response.status();
        let response_text = response.text().await.unwrap_or_default();

        tracing::debug!("NEAR AI response status: {}", status);
        tracing::debug!("NEAR AI response body: {}", response_text);

        if !status.is_success() {
            // Check for session expiration (401 with specific message patterns)
            if status.as_u16() == 401 {
                let is_session_expired = response_text.to_lowercase().contains("session")
                    && (response_text.to_lowercase().contains("expired")
                        || response_text.to_lowercase().contains("invalid"));

                if is_session_expired {
                    return Err(LlmError::SessionExpired {
                        provider: "nearai".to_string(),
                    });
                }

                // Generic 401 without session expiration indication
                return Err(LlmError::AuthFailed {
                    provider: "nearai".to_string(),
                });
            }

            // Try to parse as JSON error
            if let Ok(error) = serde_json::from_str::<NearAiErrorResponse>(&response_text) {
                if status.as_u16() == 429 {
                    return Err(LlmError::RateLimited {
                        provider: "nearai".to_string(),
                        retry_after: None,
                    });
                }
                return Err(LlmError::RequestFailed {
                    provider: "nearai".to_string(),
                    reason: error.error,
                });
            }

            return Err(LlmError::RequestFailed {
                provider: "nearai".to_string(),
                reason: format!("HTTP {}: {}", status, response_text),
            });
        }

        // Try to parse as our expected type
        match serde_json::from_str::<R>(&response_text) {
            Ok(parsed) => Ok(parsed),
            Err(e) => {
                tracing::debug!("Response is not expected JSON format: {}", e);
                tracing::debug!("Will try alternative parsing in caller");
                Err(LlmError::InvalidResponse {
                    provider: "nearai".to_string(),
                    reason: format!("Parse error: {}. Raw: {}", e, response_text),
                })
            }
        }
    }
}

/// Split messages into system instructions and non-system input messages.
/// The OpenAI Responses API expects system prompts in an `instructions` field,
/// not as a message with role "system" in the input array.
fn split_messages(messages: Vec<ChatMessage>) -> (Option<String>, Vec<NearAiMessage>) {
    let mut instructions: Vec<String> = Vec::new();
    let mut input: Vec<NearAiMessage> = Vec::new();

    for msg in messages {
        if msg.role == Role::System {
            instructions.push(msg.content);
        } else {
            input.push(msg.into());
        }
    }

    let instructions = if instructions.is_empty() {
        None
    } else {
        Some(instructions.join("\n\n"))
    };

    (instructions, input)
}

#[async_trait]
impl LlmProvider for NearAiProvider {
    async fn complete(&self, req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let (instructions, input) = split_messages(req.messages);

        let request = NearAiRequest {
            model: self.config.model.clone(),
            instructions,
            input,
            temperature: req.temperature,
            max_output_tokens: req.max_tokens,
            stream: Some(false),
            tools: None,
        };

        // Try to get structured response, fall back to alternative formats
        let response: NearAiResponse = match self.send_request("responses", &request).await {
            Ok(r) => r,
            Err(LlmError::InvalidResponse { reason, .. }) if reason.contains("Raw: ") => {
                // Extract the raw JSON from the error
                let raw_text = reason.split("Raw: ").nth(1).unwrap_or("");

                // Try parsing as alternative response format
                if let Ok(alt) = serde_json::from_str::<NearAiAltResponse>(raw_text) {
                    tracing::info!("NEAR AI returned alternative response format");
                    let text = extract_text_from_output(&alt.output);
                    let usage = alt.usage.unwrap_or(NearAiUsage {
                        input_tokens: 0,
                        output_tokens: 0,
                    });
                    return Ok(CompletionResponse {
                        content: text,
                        finish_reason: FinishReason::Stop,
                        input_tokens: usage.input_tokens,
                        output_tokens: usage.output_tokens,
                    });
                }

                // Check if it's a JSON string (quoted)
                let text = if raw_text.starts_with('"') {
                    serde_json::from_str::<String>(raw_text)
                        .unwrap_or_else(|_| raw_text.to_string())
                } else {
                    raw_text.to_string()
                };

                tracing::info!("NEAR AI returned plain text response");
                return Ok(CompletionResponse {
                    content: text,
                    finish_reason: FinishReason::Stop,
                    input_tokens: 0,
                    output_tokens: 0,
                });
            }
            Err(e) => return Err(e),
        };

        tracing::debug!("NEAR AI response: {:?}", response);

        // Extract text from response output
        // Try multiple formats since API response shape may vary
        let text = response
            .output
            .iter()
            .filter_map(|item| {
                tracing::debug!(
                    "Processing output item: type={}, text={:?}",
                    item.item_type,
                    item.text
                );
                if item.item_type == "message" {
                    // First check for direct text field on item
                    if let Some(ref text) = item.text {
                        return Some(text.clone());
                    }
                    // Then check content array
                    item.content.as_ref().map(|contents| {
                        contents
                            .iter()
                            .filter_map(|c| {
                                tracing::debug!(
                                    "Content item: type={}, text={:?}",
                                    c.content_type,
                                    c.text
                                );
                                // Accept various content types that might contain text
                                match c.content_type.as_str() {
                                    "output_text" | "text" => c.text.clone(),
                                    _ => None,
                                }
                            })
                            .collect::<Vec<_>>()
                            .join("")
                    })
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
            .join("");

        if text.is_empty() {
            tracing::warn!(
                "Empty response from NEAR AI. Raw output: {:?}",
                response.output
            );
        }

        Ok(CompletionResponse {
            content: text,
            finish_reason: FinishReason::Stop,
            input_tokens: response.usage.input_tokens,
            output_tokens: response.usage.output_tokens,
        })
    }

    async fn complete_with_tools(
        &self,
        req: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        let (instructions, input) = split_messages(req.messages);

        let tools: Vec<NearAiTool> = req
            .tools
            .into_iter()
            .map(|t| NearAiTool {
                tool_type: "function".to_string(),
                name: t.name,
                description: Some(t.description),
                parameters: Some(t.parameters),
            })
            .collect();

        let request = NearAiRequest {
            model: self.config.model.clone(),
            instructions,
            input,
            temperature: req.temperature,
            max_output_tokens: req.max_tokens,
            stream: Some(false),
            tools: if tools.is_empty() { None } else { Some(tools) },
        };

        // Try to get structured response, fall back to alternative formats
        let response: NearAiResponse = match self.send_request("responses", &request).await {
            Ok(r) => r,
            Err(LlmError::InvalidResponse { reason, .. }) if reason.contains("Raw: ") => {
                let raw_text = reason.split("Raw: ").nth(1).unwrap_or("");

                // Try parsing as alternative response format
                if let Ok(alt) = serde_json::from_str::<NearAiAltResponse>(raw_text) {
                    let text = extract_text_from_output(&alt.output);
                    let tool_calls = extract_tool_calls_from_output(&alt.output);
                    let usage = alt.usage.unwrap_or(NearAiUsage {
                        input_tokens: 0,
                        output_tokens: 0,
                    });

                    let finish_reason = if tool_calls.is_empty() {
                        FinishReason::Stop
                    } else {
                        FinishReason::ToolUse
                    };

                    tracing::info!(
                        "NEAR AI returned alternative response format ({} tool calls)",
                        tool_calls.len()
                    );

                    return Ok(ToolCompletionResponse {
                        content: if text.is_empty() { None } else { Some(text) },
                        tool_calls,
                        finish_reason,
                        input_tokens: usage.input_tokens,
                        output_tokens: usage.output_tokens,
                    });
                }

                let text = if raw_text.starts_with('"') {
                    serde_json::from_str::<String>(raw_text)
                        .unwrap_or_else(|_| raw_text.to_string())
                } else {
                    raw_text.to_string()
                };

                tracing::info!("NEAR AI returned plain text response (tool request)");
                return Ok(ToolCompletionResponse {
                    content: Some(text),
                    tool_calls: vec![],
                    finish_reason: FinishReason::Stop,
                    input_tokens: 0,
                    output_tokens: 0,
                });
            }
            Err(e) => return Err(e),
        };

        // Extract text and tool calls from response
        let mut text = String::new();
        let mut tool_calls = Vec::new();

        for item in &response.output {
            if item.item_type == "message" {
                if let Some(contents) = &item.content {
                    for content in contents {
                        if content.content_type == "output_text" {
                            if let Some(t) = &content.text {
                                text.push_str(t);
                            }
                        }
                    }
                }
            } else if item.item_type == "function_call" {
                if let (Some(name), Some(call_id)) = (&item.name, &item.call_id) {
                    // Parse arguments JSON string into Value
                    let arguments = item
                        .arguments
                        .as_ref()
                        .and_then(|s| serde_json::from_str(s).ok())
                        .unwrap_or(serde_json::Value::Object(Default::default()));

                    tool_calls.push(ToolCall {
                        id: call_id.clone(),
                        name: name.clone(),
                        arguments,
                    });
                }
            }
        }

        let finish_reason = if tool_calls.is_empty() {
            FinishReason::Stop
        } else {
            FinishReason::ToolUse
        };

        Ok(ToolCompletionResponse {
            content: if text.is_empty() { None } else { Some(text) },
            tool_calls,
            finish_reason,
            input_tokens: response.usage.input_tokens,
            output_tokens: response.usage.output_tokens,
        })
    }

    fn model_name(&self) -> &str {
        &self.config.model
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        // Default costs - could be model-specific in the future
        // These are approximate and may vary by model
        (dec!(0.000003), dec!(0.000015))
    }

    async fn list_models(&self) -> Result<Vec<String>, LlmError> {
        // Use the inherent method and extract IDs
        let models = NearAiProvider::list_models(self).await?;
        Ok(models.into_iter().map(|m| m.name).collect())
    }
}

// NEAR AI API types

/// Request format for NEAR AI Responses API.
/// See: https://docs.near.ai/api
#[derive(Debug, Serialize)]
struct NearAiRequest {
    /// Model identifier (e.g., "fireworks::accounts/fireworks/models/llama-v3p1-405b-instruct")
    model: String,
    /// System instructions (replaces sending system role in input)
    #[serde(skip_serializing_if = "Option::is_none")]
    instructions: Option<String>,
    /// Input messages (user/assistant/tool only, NOT system)
    input: Vec<NearAiMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_output_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<NearAiTool>>,
}

#[derive(Debug, Serialize, Deserialize)]
struct NearAiMessage {
    role: String,
    content: String,
}

impl From<ChatMessage> for NearAiMessage {
    fn from(msg: ChatMessage) -> Self {
        let role = match msg.role {
            Role::System => "system",
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Tool => "tool",
        };
        Self {
            role: role.to_string(),
            content: msg.content,
        }
    }
}

#[derive(Debug, Serialize)]
struct NearAiTool {
    #[serde(rename = "type")]
    tool_type: String,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    parameters: Option<serde_json::Value>,
}

/// Primary response format (output array style)
#[derive(Debug, Deserialize)]
struct NearAiResponse {
    #[allow(dead_code)]
    id: String,
    output: Vec<NearAiOutputItem>,
    usage: NearAiUsage,
}

/// Alternative response format (OpenAI-compatible style)
#[derive(Debug, Deserialize)]
struct NearAiAltResponse {
    #[allow(dead_code)]
    id: String,
    #[allow(dead_code)]
    object: Option<String>,
    #[allow(dead_code)]
    status: Option<String>,
    /// The actual output content
    output: Option<serde_json::Value>,
    /// Usage stats
    usage: Option<NearAiUsage>,
}

#[derive(Debug, Deserialize)]
struct NearAiOutputItem {
    #[serde(rename = "type")]
    item_type: String,
    #[serde(default)]
    content: Option<Vec<NearAiContent>>,
    // Direct text field (some response formats)
    #[serde(default)]
    text: Option<String>,
    // For function calls
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    call_id: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

#[derive(Debug, Deserialize)]
struct NearAiContent {
    #[serde(rename = "type")]
    content_type: String,
    #[serde(default)]
    text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct NearAiUsage {
    input_tokens: u32,
    output_tokens: u32,
}

#[derive(Debug, Deserialize)]
struct NearAiErrorResponse {
    error: String,
}

/// Extract text content from various output formats.
fn extract_text_from_output(output: &Option<serde_json::Value>) -> String {
    let Some(output) = output else {
        return String::new();
    };

    // If output is a string, return it directly
    if let Some(s) = output.as_str() {
        return s.to_string();
    }

    // If output is an array, try to extract text from items
    if let Some(arr) = output.as_array() {
        let texts: Vec<String> = arr
            .iter()
            .filter_map(|item| {
                // Skip function_call items
                if item.get("type").and_then(|t| t.as_str()) == Some("function_call") {
                    return None;
                }
                // Check for direct text field
                if let Some(text) = item.get("text").and_then(|t| t.as_str()) {
                    return Some(text.to_string());
                }
                // Check for content array with text
                if let Some(content) = item.get("content").and_then(|c| c.as_array()) {
                    let content_texts: Vec<String> = content
                        .iter()
                        .filter_map(|c| c.get("text").and_then(|t| t.as_str()).map(String::from))
                        .collect();
                    if !content_texts.is_empty() {
                        return Some(content_texts.join(""));
                    }
                }
                // Check for content as string
                if let Some(content) = item.get("content").and_then(|c| c.as_str()) {
                    return Some(content.to_string());
                }
                None
            })
            .collect();
        return texts.join("");
    }

    // If output is an object, try common fields
    if let Some(obj) = output.as_object() {
        if let Some(text) = obj.get("text").and_then(|t| t.as_str()) {
            return text.to_string();
        }
        if let Some(content) = obj.get("content").and_then(|c| c.as_str()) {
            return content.to_string();
        }
        if let Some(message) = obj.get("message").and_then(|m| m.as_str()) {
            return message.to_string();
        }
    }

    // Fallback: return JSON representation
    tracing::warn!("Could not extract text from output: {:?}", output);
    output.to_string()
}

/// Extract tool calls from alternative output format.
fn extract_tool_calls_from_output(output: &Option<serde_json::Value>) -> Vec<ToolCall> {
    let Some(output) = output else {
        return vec![];
    };

    let Some(arr) = output.as_array() else {
        return vec![];
    };

    arr.iter()
        .filter_map(|item| {
            // Look for function_call type items
            let item_type = item.get("type").and_then(|t| t.as_str())?;
            if item_type != "function_call" {
                return None;
            }

            let name = item.get("name").and_then(|n| n.as_str())?;
            let call_id = item
                .get("call_id")
                .and_then(|c| c.as_str())
                .unwrap_or("unknown");

            // Arguments can be a string (JSON) or already an object
            let arguments = if let Some(args_str) = item.get("arguments").and_then(|a| a.as_str()) {
                serde_json::from_str(args_str)
                    .unwrap_or(serde_json::Value::Object(Default::default()))
            } else if let Some(args_obj) = item.get("arguments") {
                args_obj.clone()
            } else {
                serde_json::Value::Object(Default::default())
            };

            Some(ToolCall {
                id: call_id.to_string(),
                name: name.to_string(),
                arguments,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_conversion() {
        let msg = ChatMessage::user("Hello");
        let nearai_msg: NearAiMessage = msg.into();
        assert_eq!(nearai_msg.role, "user");
        assert_eq!(nearai_msg.content, "Hello");
    }

    #[test]
    fn test_system_message_conversion() {
        let msg = ChatMessage::system("You are helpful");
        let nearai_msg: NearAiMessage = msg.into();
        assert_eq!(nearai_msg.role, "system");
    }

    #[test]
    fn test_split_messages_with_system() {
        let messages = vec![
            ChatMessage::system("You are a helpful assistant"),
            ChatMessage::user("Hello"),
            ChatMessage::assistant("Hi there!"),
        ];
        let (instructions, input) = split_messages(messages);
        assert_eq!(
            instructions,
            Some("You are a helpful assistant".to_string())
        );
        assert_eq!(input.len(), 2);
        assert_eq!(input[0].role, "user");
        assert_eq!(input[1].role, "assistant");
    }

    #[test]
    fn test_split_messages_no_system() {
        let messages = vec![
            ChatMessage::user("Hello"),
            ChatMessage::assistant("Hi there!"),
        ];
        let (instructions, input) = split_messages(messages);
        assert!(instructions.is_none());
        assert_eq!(input.len(), 2);
    }

    #[test]
    fn test_split_messages_multiple_system() {
        let messages = vec![
            ChatMessage::system("First instruction"),
            ChatMessage::system("Second instruction"),
            ChatMessage::user("Hello"),
        ];
        let (instructions, input) = split_messages(messages);
        assert_eq!(
            instructions,
            Some("First instruction\n\nSecond instruction".to_string())
        );
        assert_eq!(input.len(), 1);
    }
}
