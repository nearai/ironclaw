//! Generic OpenAI Responses API client.
//!
//! Implements `LlmProvider` using the public `/responses` wire protocol for
//! API-key-authenticated OpenAI-compatible endpoints.

use async_trait::async_trait;
use reqwest::Client;
use rust_decimal::Decimal;
use secrecy::ExposeSecret;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::LlmError;
use crate::llm::config::RegistryProviderConfig;
use crate::llm::provider::{
    ChatMessage, CompletionRequest, CompletionResponse, ContentPart, FinishReason, LlmProvider,
    ModelMetadata, Role, ToolCall, ToolCompletionRequest, ToolCompletionResponse, ToolDefinition,
};

const PROVIDER_NAME: &str = "openai_responses";

pub struct OpenAiResponsesProvider {
    client: Client,
    model: String,
    api_base_url: String,
    api_key: String,
    extra_headers: reqwest::header::HeaderMap,
}

impl OpenAiResponsesProvider {
    pub fn new(
        config: &RegistryProviderConfig,
        request_timeout_secs: u64,
    ) -> Result<Self, LlmError> {
        let api_key = config
            .api_key
            .as_ref()
            .map(|k| k.expose_secret().to_string())
            .ok_or_else(|| LlmError::AuthFailed {
                provider: config.provider_id.clone(),
            })?;

        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(request_timeout_secs))
            .build()
            .map_err(|e| LlmError::RequestFailed {
                provider: config.provider_id.clone(),
                reason: format!("Failed to create HTTP client: {e}"),
            })?;

        let extra_headers = build_extra_headers(&config.extra_headers, &config.provider_id);

        Ok(Self {
            client,
            model: config.model.clone(),
            api_base_url: config.base_url.trim_end_matches('/').to_string(),
            api_key,
            extra_headers,
        })
    }

    fn build_request_body(
        &self,
        messages: &[ChatMessage],
        tools: Option<&[ToolDefinition]>,
    ) -> Value {
        let instructions: String = messages
            .iter()
            .filter(|m| m.role == Role::System)
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("\n\n");

        let input: Vec<Value> = messages
            .iter()
            .enumerate()
            .filter(|(_, m)| m.role != Role::System)
            .flat_map(|(i, m)| convert_message(m, i))
            .collect();

        let mut body = json!({
            "model": self.model,
            "store": false,
            "stream": true,
            "input": input,
            "text": { "verbosity": "medium" },
            "include": ["reasoning.encrypted_content"],
        });

        if !instructions.is_empty() {
            body["instructions"] = Value::String(instructions);
        }

        if let Some(tools) = tools
            && !tools.is_empty()
        {
            body["tools"] = Value::Array(tools.iter().map(convert_tool_definition).collect());
            body["tool_choice"] = Value::String("auto".to_string());
            body["parallel_tool_calls"] = Value::Bool(true);
        }

        body
    }

    async fn send_request(&self, body: Value) -> Result<ParsedResponse, LlmError> {
        let url = format!("{}/responses", self.api_base_url);
        let mut request = self
            .client
            .post(&url)
            .bearer_auth(&self.api_key)
            .header(reqwest::header::ACCEPT, "text/event-stream")
            .header(reqwest::header::CONTENT_TYPE, "application/json");

        if !self.extra_headers.is_empty() {
            request = request.headers(self.extra_headers.clone());
        }

        tracing::debug!(url = %url, model = %self.model, "Sending generic Responses API request");

        let response = request
            .json(&body)
            .send()
            .await
            .map_err(|e| LlmError::RequestFailed {
                provider: PROVIDER_NAME.to_string(),
                reason: format!("HTTP request failed: {e}"),
            })?;

        let status = response.status();
        if !status.is_success() {
            let retry_after = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(parse_retry_after);
            let body_text = response.text().await.unwrap_or_default();
            if status == reqwest::StatusCode::UNAUTHORIZED {
                return Err(LlmError::AuthFailed {
                    provider: PROVIDER_NAME.to_string(),
                });
            }
            if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                return Err(LlmError::RateLimited {
                    provider: PROVIDER_NAME.to_string(),
                    retry_after,
                });
            }
            return Err(LlmError::RequestFailed {
                provider: PROVIDER_NAME.to_string(),
                reason: format!("HTTP {status}: {body_text}"),
            });
        }

        let body_bytes = response
            .bytes()
            .await
            .map_err(|e| LlmError::RequestFailed {
                provider: PROVIDER_NAME.to_string(),
                reason: format!("Failed to read response body: {e}"),
            })?;

        let body_text = String::from_utf8_lossy(&body_bytes);
        parse_sse_response(&body_text)
    }
}

#[async_trait]
impl LlmProvider for OpenAiResponsesProvider {
    fn model_name(&self) -> &str {
        &self.model
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        (Decimal::ZERO, Decimal::ZERO)
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let body = self.build_request_body(&request.messages, None);
        let parsed = self.send_request(body).await?;
        Ok(CompletionResponse {
            content: parsed.text_content,
            input_tokens: parsed.input_tokens,
            output_tokens: parsed.output_tokens,
            finish_reason: parsed.finish_reason,
            cache_read_input_tokens: 0,
            cache_creation_input_tokens: 0,
        })
    }

    async fn complete_with_tools(
        &self,
        request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        let body = self.build_request_body(&request.messages, Some(&request.tools));
        let parsed = self.send_request(body).await?;
        let finish_reason = if !parsed.tool_calls.is_empty() {
            FinishReason::ToolUse
        } else {
            parsed.finish_reason
        };
        Ok(ToolCompletionResponse {
            content: (!parsed.text_content.is_empty()).then_some(parsed.text_content),
            tool_calls: parsed.tool_calls,
            input_tokens: parsed.input_tokens,
            output_tokens: parsed.output_tokens,
            finish_reason,
            cache_read_input_tokens: 0,
            cache_creation_input_tokens: 0,
        })
    }

    async fn model_metadata(&self) -> Result<ModelMetadata, LlmError> {
        Ok(ModelMetadata {
            id: self.model.clone(),
            context_length: None,
        })
    }

    fn effective_model_name(&self, _requested_model: Option<&str>) -> String {
        self.model.clone()
    }
}

fn build_extra_headers(
    pairs: &[(String, String)],
    provider_id: &str,
) -> reqwest::header::HeaderMap {
    let mut headers = reqwest::header::HeaderMap::new();
    for (key, value) in pairs {
        let name = match reqwest::header::HeaderName::from_bytes(key.as_bytes()) {
            Ok(n) => n,
            Err(e) => {
                tracing::warn!(
                    provider = provider_id,
                    header = %key,
                    error = %e,
                    "Skipping extra header: invalid name"
                );
                continue;
            }
        };
        let value = match reqwest::header::HeaderValue::from_str(value) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(
                    provider = provider_id,
                    header = %key,
                    error = %e,
                    "Skipping extra header: invalid value"
                );
                continue;
            }
        };
        headers.insert(name, value);
    }
    headers
}

fn parse_retry_after(raw: &str) -> Option<std::time::Duration> {
    if let Ok(secs) = raw.trim().parse::<u64>() {
        return Some(std::time::Duration::from_secs(secs));
    }
    if let Ok(dt) = chrono::DateTime::parse_from_rfc2822(raw.trim()) {
        let now = chrono::Utc::now();
        let delta = dt.signed_duration_since(now);
        return Some(std::time::Duration::from_secs(
            delta.num_seconds().max(0) as u64
        ));
    }
    None
}

fn convert_message(msg: &ChatMessage, index: usize) -> Vec<Value> {
    match msg.role {
        Role::System => Vec::new(),
        Role::User => {
            let content = if !msg.content_parts.is_empty() {
                msg.content_parts
                    .iter()
                    .map(|part| match part {
                        ContentPart::Text { text } => json!({
                            "type": "input_text",
                            "text": text,
                        }),
                        ContentPart::ImageUrl { image_url } => json!({
                            "type": "input_image",
                            "image_url": image_url.url,
                        }),
                    })
                    .collect::<Vec<_>>()
            } else {
                vec![json!({
                    "type": "input_text",
                    "text": msg.content,
                })]
            };

            vec![json!({
                "type": "message",
                "role": "user",
                "content": content,
            })]
        }
        Role::Assistant => {
            if let Some(tool_calls) = &msg.tool_calls {
                let mut items = Vec::new();
                if !msg.content.is_empty() {
                    items.push(json!({
                        "type": "message",
                        "role": "assistant",
                        "id": format!("msg_{index}"),
                        "status": "completed",
                        "content": [{
                            "type": "output_text",
                            "text": msg.content,
                            "annotations": [],
                        }],
                    }));
                }
                items.extend(tool_calls.iter().map(|tc| {
                    let args = if tc.arguments.is_string() {
                        tc.arguments.as_str().unwrap_or("{}").to_string()
                    } else {
                        tc.arguments.to_string()
                    };
                    json!({
                        "type": "function_call",
                        "call_id": tc.id,
                        "name": tc.name,
                        "arguments": args,
                    })
                }));
                items
            } else {
                vec![json!({
                    "type": "message",
                    "role": "assistant",
                    "id": format!("msg_{index}"),
                    "status": "completed",
                    "content": [{
                        "type": "output_text",
                        "text": msg.content,
                        "annotations": [],
                    }],
                })]
            }
        }
        Role::Tool => vec![json!({
            "type": "function_call_output",
            "call_id": msg.tool_call_id.as_deref().unwrap_or("unknown"),
            "output": msg.content,
        })],
    }
}

fn convert_tool_definition(tool: &ToolDefinition) -> Value {
    use crate::llm::rig_adapter::normalize_schema_strict;

    json!({
        "type": "function",
        "name": tool.name,
        "description": tool.description,
        "parameters": normalize_schema_strict(&tool.parameters),
    })
}

#[derive(Debug)]
struct ParsedResponse {
    text_content: String,
    tool_calls: Vec<ToolCall>,
    input_tokens: u32,
    output_tokens: u32,
    finish_reason: FinishReason,
}

#[derive(Debug, Deserialize)]
struct SseEvent {
    #[serde(rename = "type")]
    event_type: String,
    #[serde(flatten)]
    data: Value,
}

#[derive(Debug, Default)]
struct FunctionCallState {
    call_id: String,
    name: String,
    arguments: String,
}

fn parse_sse_response(body: &str) -> Result<ParsedResponse, LlmError> {
    let mut text_content = String::new();
    let mut tool_calls = Vec::new();
    let mut input_tokens = 0;
    let mut output_tokens = 0;
    let mut finish_reason = FinishReason::Stop;
    let mut active_function_calls: std::collections::HashMap<String, FunctionCallState> =
        std::collections::HashMap::new();
    let mut response_status: Option<String> = None;

    for line in body.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with(':') {
            continue;
        }
        let Some(data) = line.strip_prefix("data: ") else {
            continue;
        };
        if data == "[DONE]" {
            break;
        }
        let event: SseEvent =
            serde_json::from_str(data).map_err(|e| LlmError::InvalidResponse {
                provider: PROVIDER_NAME.to_string(),
                reason: format!("Failed to parse SSE event JSON: {e}"),
            })?;

        match event.event_type.as_str() {
            "response.output_text.delta" => {
                if let Some(delta) = event.data.get("delta").and_then(|d| d.as_str()) {
                    text_content.push_str(delta);
                }
            }
            "response.output_item.added" => {
                let item = event.data.get("item").unwrap_or(&event.data);
                if item.get("type").and_then(|t| t.as_str()) == Some("function_call") {
                    let item_id = item
                        .get("id")
                        .or_else(|| item.get("item_id"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let call_id = item
                        .get("call_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let name = item
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    active_function_calls
                        .entry(item_id)
                        .or_insert(FunctionCallState {
                            call_id,
                            name,
                            arguments: String::new(),
                        });
                }
            }
            "response.function_call_arguments.delta" => {
                if let Some(item_id) = event.data.get("item_id").and_then(|v| v.as_str())
                    && let Some(state) = active_function_calls.get_mut(item_id)
                    && let Some(delta) = event.data.get("delta").and_then(|d| d.as_str())
                {
                    state.arguments.push_str(delta);
                }
            }
            "response.function_call_arguments.done" => {
                if let Some(item_id) = event.data.get("item_id").and_then(|v| v.as_str())
                    && let Some(state) = active_function_calls.get_mut(item_id)
                    && let Some(arguments) = event.data.get("arguments").and_then(|v| v.as_str())
                {
                    state.arguments = arguments.to_string();
                }
            }
            "response.output_item.done" => {
                let item = event.data.get("item").unwrap_or(&event.data);
                if item.get("type").and_then(|t| t.as_str()) == Some("function_call") {
                    let item_id = item
                        .get("id")
                        .or_else(|| item.get("item_id"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let state = active_function_calls.get(&item_id);
                    let call_id = item
                        .get("call_id")
                        .and_then(|v| v.as_str())
                        .or_else(|| state.map(|s| s.call_id.as_str()))
                        .unwrap_or("")
                        .to_string();
                    let name = item
                        .get("name")
                        .and_then(|v| v.as_str())
                        .or_else(|| state.map(|s| s.name.as_str()))
                        .unwrap_or("")
                        .to_string();
                    let arguments = item
                        .get("arguments")
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string)
                        .or_else(|| state.map(|s| s.arguments.clone()))
                        .unwrap_or_default();
                    let arguments = serde_json::from_str(&arguments)
                        .unwrap_or_else(|_| Value::String(arguments));
                    tool_calls.push(ToolCall {
                        id: call_id,
                        name,
                        arguments,
                    });
                    active_function_calls.remove(&item_id);
                }
            }
            "response.completed" => {
                if let Some(response) = event.data.get("response") {
                    response_status = response
                        .get("status")
                        .and_then(|v| v.as_str())
                        .map(ToString::to_string);
                    input_tokens = response
                        .get("usage")
                        .and_then(|u| u.get("input_tokens"))
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0) as u32;
                    output_tokens = response
                        .get("usage")
                        .and_then(|u| u.get("output_tokens"))
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0) as u32;
                }
            }
            "response.failed" => {
                return Err(LlmError::RequestFailed {
                    provider: PROVIDER_NAME.to_string(),
                    reason: format!("Responses API returned failed event: {}", event.data),
                });
            }
            _ => {}
        }
    }

    if response_status.as_deref() == Some("incomplete") {
        finish_reason = FinishReason::Length;
    } else if response_status.as_deref() == Some("failed") {
        finish_reason = FinishReason::Unknown;
    }

    Ok(ParsedResponse {
        text_content,
        tool_calls,
        input_tokens,
        output_tokens,
        finish_reason,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::provider::{ChatMessage, ToolDefinition};

    #[test]
    fn build_request_body_uses_responses_format_for_text() {
        let provider = OpenAiResponsesProvider {
            client: Client::new(),
            model: "gpt-5.4".to_string(),
            api_base_url: "https://example.com/v1".to_string(),
            api_key: "test-key".to_string(),
            extra_headers: reqwest::header::HeaderMap::new(),
        };
        let body = provider.build_request_body(
            &[
                ChatMessage::system("You are helpful."),
                ChatMessage::user("hello"),
            ],
            None,
        );
        assert_eq!(body["model"], "gpt-5.4");
        assert_eq!(body["stream"], true);
        assert_eq!(body["instructions"], "You are helpful.");
        assert_eq!(body["input"][0]["type"], "message");
        assert_eq!(body["input"][0]["role"], "user");
    }

    #[test]
    fn build_request_body_includes_tools_for_responses() {
        let provider = OpenAiResponsesProvider {
            client: Client::new(),
            model: "gpt-5.4".to_string(),
            api_base_url: "https://example.com/v1".to_string(),
            api_key: "test-key".to_string(),
            extra_headers: reqwest::header::HeaderMap::new(),
        };
        let tool = ToolDefinition {
            name: "time".to_string(),
            description: "Get current time".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "utc_offset": { "type": "string" }
                }
            }),
        };
        let body =
            provider.build_request_body(&[ChatMessage::user("what time is it?")], Some(&[tool]));
        assert_eq!(body["tools"][0]["type"], "function");
        assert_eq!(body["tool_choice"], "auto");
        assert_eq!(body["parallel_tool_calls"], true);
    }

    #[test]
    fn parse_sse_response_extracts_text_and_tool_calls() {
        let sse_body = r#"data: {"type":"response.output_item.added","item":{"type":"function_call","id":"fc_1","call_id":"call_abc","name":"search"}}

data: {"type":"response.function_call_arguments.delta","item_id":"fc_1","delta":"{\"query\":"}

data: {"type":"response.function_call_arguments.done","item_id":"fc_1","arguments":"{\"query\":\"test\"}"}

data: {"type":"response.output_item.done","item":{"type":"function_call","id":"fc_1","call_id":"call_abc","name":"search","arguments":"{\"query\":\"test\"}"}}

data: {"type":"response.output_text.delta","delta":"Done"}

data: {"type":"response.completed","response":{"status":"completed","usage":{"input_tokens":11,"output_tokens":7}}}
"#;

        let parsed = parse_sse_response(sse_body).expect("responses SSE should parse");
        assert_eq!(parsed.text_content, "Done");
        assert_eq!(parsed.input_tokens, 11);
        assert_eq!(parsed.output_tokens, 7);
        assert_eq!(parsed.tool_calls.len(), 1);
        assert_eq!(parsed.tool_calls[0].id, "call_abc");
        assert_eq!(parsed.tool_calls[0].name, "search");
        assert_eq!(parsed.tool_calls[0].arguments["query"], "test");
    }
}
