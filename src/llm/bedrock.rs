//! AWS Bedrock LLM provider using the native Converse API.
//!
//! Uses `aws-sdk-bedrockruntime` to call `client.converse()` directly,
//! bypassing the OpenAI-compatible layer. Supports all Bedrock auth methods:
//! bearer token (`AWS_BEARER_TOKEN_BEDROCK`), IAM credentials, SSO profiles,
//! and instance roles — all handled transparently by the AWS SDK credential chain.

use std::collections::HashMap;
use std::sync::RwLock;

use async_trait::async_trait;
use aws_config::{BehaviorVersion, Region};
use aws_sdk_bedrockruntime::Client;
use aws_sdk_bedrockruntime::operation::converse::ConverseError;
use aws_sdk_bedrockruntime::types::{
    AnyToolChoice, AutoToolChoice, ContentBlock, ConversationRole, InferenceConfiguration, Message,
    StopReason, SystemContentBlock, Tool, ToolChoice, ToolConfiguration, ToolInputSchema,
    ToolResultBlock, ToolResultContentBlock, ToolResultStatus, ToolSpecification, ToolUseBlock,
};
use aws_smithy_types::Document;
use rust_decimal::Decimal;

use crate::config::BedrockConfig;
use crate::error::LlmError;
use crate::llm::provider::{
    CompletionRequest, CompletionResponse, FinishReason, LlmProvider, ModelMetadata, ToolCall,
    ToolCompletionRequest, ToolCompletionResponse, ToolDefinition,
};

/// AWS Bedrock provider using the native Converse API.
pub struct BedrockProvider {
    client: Client,
    /// Base model ID for display purposes (without prefix).
    display_model: String,
    /// Cross-region prefix (e.g. "us.", "global.") or empty.
    cross_region_prefix: String,
    /// Active model ID (with cross-region prefix), switchable at runtime via `set_model()`.
    active_model: RwLock<String>,
}

impl BedrockProvider {
    /// Create a new Bedrock provider from configuration.
    ///
    /// Uses `block_in_place` because the AWS SDK config loader is async
    /// but `create_llm_provider` is sync. This is safe because IronClaw
    /// uses the multi-threaded tokio runtime.
    pub fn new(config: &BedrockConfig) -> Result<Self, LlmError> {
        let cross_region_prefix = config
            .cross_region
            .as_ref()
            .map(|prefix| format!("{}.", prefix))
            .unwrap_or_default();

        let model_id = format!("{}{}", cross_region_prefix, config.model);

        let sdk_config = tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                let mut builder = aws_config::defaults(BehaviorVersion::latest())
                    .region(Region::new(config.region.clone()));
                if let Some(ref profile) = config.profile {
                    builder = builder.profile_name(profile);
                }
                builder.load().await
            })
        });

        let client = Client::new(&sdk_config);

        Ok(Self {
            client,
            display_model: config.model.clone(),
            cross_region_prefix,
            active_model: RwLock::new(model_id),
        })
    }

    /// Get the currently active model ID (with cross-region prefix).
    fn current_model_id(&self) -> String {
        match self.active_model.read() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => {
                tracing::warn!("active_model lock poisoned while reading; continuing");
                poisoned.into_inner().clone()
            }
        }
    }
}

#[async_trait]
impl LlmProvider for BedrockProvider {
    fn model_name(&self) -> &str {
        &self.display_model
    }

    fn cost_per_token(&self) -> (Decimal, Decimal) {
        // Bedrock billing is on the AWS bill, not trackable per-token here.
        (Decimal::ZERO, Decimal::ZERO)
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let model_id = self.current_model_id();

        let mut messages = request.messages;
        crate::llm::provider::sanitize_tool_messages(&mut messages);

        let (system_blocks, bedrock_messages) = convert_messages(&messages)?;

        let mut builder = self
            .client
            .converse()
            .model_id(&model_id)
            .set_system(if system_blocks.is_empty() {
                None
            } else {
                Some(system_blocks)
            })
            .set_messages(Some(bedrock_messages));

        if let Some(temp) = request.temperature {
            builder = builder.inference_config(
                InferenceConfiguration::builder()
                    .temperature(temp)
                    .set_max_tokens(request.max_tokens.map(|t| t as i32))
                    .build(),
            );
        } else if let Some(max_tokens) = request.max_tokens {
            builder = builder.inference_config(
                InferenceConfiguration::builder()
                    .max_tokens(max_tokens as i32)
                    .build(),
            );
        }

        let response = builder.send().await.map_err(|e| map_sdk_error(&e))?;

        let (text, _tool_calls) = extract_content_blocks(response.output())?;
        let (input_tokens, output_tokens) = extract_token_usage(response.usage());

        Ok(CompletionResponse {
            content: text,
            input_tokens,
            output_tokens,
            finish_reason: map_stop_reason(response.stop_reason()),
        })
    }

    async fn complete_with_tools(
        &self,
        request: ToolCompletionRequest,
    ) -> Result<ToolCompletionResponse, LlmError> {
        let model_id = self.current_model_id();

        let mut messages = request.messages;
        crate::llm::provider::sanitize_tool_messages(&mut messages);

        let (system_blocks, bedrock_messages) = convert_messages(&messages)?;

        let tool_config = build_tool_config(&request.tools, request.tool_choice.as_deref())?;

        let mut builder = self
            .client
            .converse()
            .model_id(&model_id)
            .set_system(if system_blocks.is_empty() {
                None
            } else {
                Some(system_blocks)
            })
            .set_messages(Some(bedrock_messages));

        if let Some(tc) = tool_config {
            builder = builder.tool_config(tc);
        }

        if let Some(temp) = request.temperature {
            builder = builder.inference_config(
                InferenceConfiguration::builder()
                    .temperature(temp)
                    .set_max_tokens(request.max_tokens.map(|t| t as i32))
                    .build(),
            );
        } else if let Some(max_tokens) = request.max_tokens {
            builder = builder.inference_config(
                InferenceConfiguration::builder()
                    .max_tokens(max_tokens as i32)
                    .build(),
            );
        }

        let response = builder.send().await.map_err(|e| map_sdk_error(&e))?;

        let (text, tool_calls) = extract_content_blocks(response.output())?;
        let (input_tokens, output_tokens) = extract_token_usage(response.usage());

        Ok(ToolCompletionResponse {
            content: if text.is_empty() { None } else { Some(text) },
            tool_calls,
            input_tokens,
            output_tokens,
            finish_reason: map_stop_reason(response.stop_reason()),
        })
    }

    async fn model_metadata(&self) -> Result<ModelMetadata, LlmError> {
        Ok(ModelMetadata {
            id: self.current_model_id(),
            context_length: None,
        })
    }

    fn active_model_name(&self) -> String {
        self.current_model_id()
    }

    fn effective_model_name(&self, _requested_model: Option<&str>) -> String {
        // Bedrock doesn't support per-request model overrides in Converse API;
        // the model is part of the request builder, not the message body.
        self.active_model_name()
    }

    fn set_model(&self, model: &str) -> Result<(), LlmError> {
        let new_id = format!("{}{}", self.cross_region_prefix, model);
        match self.active_model.write() {
            Ok(mut guard) => {
                *guard = new_id;
            }
            Err(poisoned) => {
                tracing::warn!("active_model lock poisoned while writing; continuing");
                *poisoned.into_inner() = new_id;
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Message conversion
// ---------------------------------------------------------------------------

/// Convert IronClaw `ChatMessage` list into Bedrock system blocks + messages.
///
/// Key differences from OpenAI/Anthropic protocol:
/// 1. System messages are extracted and passed separately.
/// 2. Tool results (role=Tool) become `ContentBlock::ToolResult` inside User messages.
/// 3. Consecutive tool results are merged into a single User message.
/// 4. Bedrock requires strict user/assistant alternation.
fn convert_messages(
    messages: &[crate::llm::provider::ChatMessage],
) -> Result<(Vec<SystemContentBlock>, Vec<Message>), LlmError> {
    use crate::llm::provider::Role;

    let mut system_blocks = Vec::new();
    let mut bedrock_messages: Vec<Message> = Vec::new();
    let mut pending_tool_results: Vec<ContentBlock> = Vec::new();

    for msg in messages {
        match msg.role {
            Role::System => {
                if !msg.content.is_empty() {
                    system_blocks.push(SystemContentBlock::Text(msg.content.clone()));
                }
            }
            Role::User => {
                // Flush any pending tool results as a User message first
                flush_tool_results(&mut pending_tool_results, &mut bedrock_messages)?;

                let content = vec![ContentBlock::Text(msg.content.clone())];
                push_message(&mut bedrock_messages, ConversationRole::User, content)?;
            }
            Role::Assistant => {
                // Flush any pending tool results before an assistant message
                flush_tool_results(&mut pending_tool_results, &mut bedrock_messages)?;

                let mut content = Vec::new();

                // Add text content if non-empty
                if !msg.content.is_empty() {
                    content.push(ContentBlock::Text(msg.content.clone()));
                }

                // Add tool use blocks if present
                if let Some(ref tool_calls) = msg.tool_calls {
                    for tc in tool_calls {
                        let input_doc = json_to_document(&tc.arguments);
                        let tool_use = ToolUseBlock::builder()
                            .tool_use_id(&tc.id)
                            .name(&tc.name)
                            .input(input_doc)
                            .build()
                            .map_err(|e| LlmError::RequestFailed {
                                provider: "bedrock".to_string(),
                                reason: format!("Failed to build ToolUseBlock: {}", e),
                            })?;
                        content.push(ContentBlock::ToolUse(tool_use));
                    }
                }

                if !content.is_empty() {
                    push_message(&mut bedrock_messages, ConversationRole::Assistant, content)?;
                }
            }
            Role::Tool => {
                // Accumulate tool results — they'll be flushed as a User message
                let tool_call_id = msg.tool_call_id.as_deref().unwrap_or("unknown");

                let status = if msg.content.contains("\"is_error\":true")
                    || msg.content.contains("Error:")
                {
                    Some(ToolResultStatus::Error)
                } else {
                    Some(ToolResultStatus::Success)
                };

                let tool_result = ToolResultBlock::builder()
                    .tool_use_id(tool_call_id)
                    .content(ToolResultContentBlock::Text(msg.content.clone()))
                    .set_status(status)
                    .build()
                    .map_err(|e| LlmError::RequestFailed {
                        provider: "bedrock".to_string(),
                        reason: format!("Failed to build ToolResultBlock: {}", e),
                    })?;

                pending_tool_results.push(ContentBlock::ToolResult(tool_result));
            }
        }
    }

    // Flush any remaining tool results
    flush_tool_results(&mut pending_tool_results, &mut bedrock_messages)?;

    Ok((system_blocks, bedrock_messages))
}

/// Flush accumulated tool result blocks as a single User message.
fn flush_tool_results(
    pending: &mut Vec<ContentBlock>,
    messages: &mut Vec<Message>,
) -> Result<(), LlmError> {
    if pending.is_empty() {
        return Ok(());
    }

    let content: Vec<ContentBlock> = std::mem::take(pending);
    push_message(messages, ConversationRole::User, content)?;

    Ok(())
}

/// Push a message, enforcing Bedrock's alternation requirement.
///
/// If the last message has the same role, merge the content blocks into it
/// rather than creating a consecutive same-role message.
fn push_message(
    messages: &mut Vec<Message>,
    role: ConversationRole,
    content: Vec<ContentBlock>,
) -> Result<(), LlmError> {
    if content.is_empty() {
        return Ok(());
    }

    // Check if we need to merge with the previous message of the same role
    if let Some(last) = messages.last()
        && *last.role() == role
    {
        // Remove the last message, merge content, and re-push
        let prev = messages.pop().ok_or_else(|| LlmError::RequestFailed {
            provider: "bedrock".to_string(),
            reason: "Unexpected empty message list during merge".to_string(),
        })?;
        let mut merged = prev.content().to_vec();
        merged.extend(content);
        let msg = Message::builder()
            .role(role)
            .set_content(Some(merged))
            .build()
            .map_err(|e| LlmError::RequestFailed {
                provider: "bedrock".to_string(),
                reason: format!("Failed to build merged Message: {}", e),
            })?;
        messages.push(msg);
        return Ok(());
    }

    let msg = Message::builder()
        .role(role)
        .set_content(Some(content))
        .build()
        .map_err(|e| LlmError::RequestFailed {
            provider: "bedrock".to_string(),
            reason: format!("Failed to build Message: {}", e),
        })?;
    messages.push(msg);

    Ok(())
}

// ---------------------------------------------------------------------------
// Tool configuration
// ---------------------------------------------------------------------------

/// Build Bedrock `ToolConfiguration` from IronClaw tool definitions.
fn build_tool_config(
    tools: &[ToolDefinition],
    tool_choice: Option<&str>,
) -> Result<Option<ToolConfiguration>, LlmError> {
    if tools.is_empty() {
        return Ok(None);
    }

    let bedrock_tools: Vec<Tool> = tools
        .iter()
        .map(|td| {
            let input_schema = ToolInputSchema::Json(json_to_document(&td.parameters));
            let spec = ToolSpecification::builder()
                .name(&td.name)
                .description(&td.description)
                .input_schema(input_schema)
                .build()
                .map_err(|e| LlmError::RequestFailed {
                    provider: "bedrock".to_string(),
                    reason: format!("Failed to build ToolSpecification: {}", e),
                })?;
            Ok(Tool::ToolSpec(spec))
        })
        .collect::<Result<Vec<_>, LlmError>>()?;

    let choice = match tool_choice {
        Some("none") => {
            // If tool_choice is "none", don't send tool config at all
            return Ok(None);
        }
        Some("required") => Some(ToolChoice::Any(AnyToolChoice::builder().build())),
        // "auto" or anything else
        _ => Some(ToolChoice::Auto(AutoToolChoice::builder().build())),
    };

    let mut builder = ToolConfiguration::builder().set_tools(Some(bedrock_tools));
    if let Some(c) = choice {
        builder = builder.tool_choice(c);
    }

    let config = builder.build().map_err(|e| LlmError::RequestFailed {
        provider: "bedrock".to_string(),
        reason: format!("Failed to build ToolConfiguration: {}", e),
    })?;

    Ok(Some(config))
}

// ---------------------------------------------------------------------------
// Response extraction
// ---------------------------------------------------------------------------

/// Extract text content and tool calls from the Converse response output.
fn extract_content_blocks(
    output: Option<&aws_sdk_bedrockruntime::types::ConverseOutput>,
) -> Result<(String, Vec<ToolCall>), LlmError> {
    let output = output.ok_or_else(|| LlmError::RequestFailed {
        provider: "bedrock".to_string(),
        reason: "Converse response has no output".to_string(),
    })?;

    let message = output.as_message().map_err(|_| LlmError::RequestFailed {
        provider: "bedrock".to_string(),
        reason: "Converse output is not a message".to_string(),
    })?;

    let mut text_parts = Vec::new();
    let mut tool_calls = Vec::new();

    for block in message.content() {
        match block {
            ContentBlock::Text(t) => {
                text_parts.push(t.clone());
            }
            ContentBlock::ToolUse(tu) => {
                tool_calls.push(ToolCall {
                    id: tu.tool_use_id().to_string(),
                    name: tu.name().to_string(),
                    arguments: document_to_json(tu.input()),
                });
            }
            // Ignore reasoning, citations, images, etc.
            _ => {}
        }
    }

    Ok((text_parts.join(""), tool_calls))
}

/// Extract token usage from the response, converting i32 → u32 safely.
fn extract_token_usage(usage: Option<&aws_sdk_bedrockruntime::types::TokenUsage>) -> (u32, u32) {
    match usage {
        Some(u) => (
            u32::try_from(u.input_tokens()).unwrap_or(0),
            u32::try_from(u.output_tokens()).unwrap_or(0),
        ),
        None => (0, 0),
    }
}

/// Map Bedrock `StopReason` to IronClaw `FinishReason`.
fn map_stop_reason(reason: &StopReason) -> FinishReason {
    match reason {
        StopReason::EndTurn | StopReason::StopSequence => FinishReason::Stop,
        StopReason::ToolUse => FinishReason::ToolUse,
        StopReason::MaxTokens | StopReason::ModelContextWindowExceeded => FinishReason::Length,
        StopReason::ContentFiltered | StopReason::GuardrailIntervened => {
            FinishReason::ContentFilter
        }
        _ => FinishReason::Unknown,
    }
}

// ---------------------------------------------------------------------------
// Error mapping
// ---------------------------------------------------------------------------

/// Map AWS SDK errors to `LlmError`.
fn map_sdk_error<R: std::fmt::Debug>(
    error: &aws_sdk_bedrockruntime::error::SdkError<ConverseError, R>,
) -> LlmError {
    use aws_sdk_bedrockruntime::error::SdkError;

    match error {
        SdkError::ServiceError(service_err) => {
            let msg = match service_err.err() {
                ConverseError::ModelTimeoutException(e) => {
                    format!("Model timeout: {}", e.message().unwrap_or("unknown"))
                }
                ConverseError::ModelNotReadyException(e) => {
                    format!("Model not ready: {}", e.message().unwrap_or("unknown"))
                }
                ConverseError::ThrottlingException(e) => {
                    format!("Throttled: {}", e.message().unwrap_or("unknown"))
                }
                ConverseError::ValidationException(e) => {
                    format!("Validation error: {}", e.message().unwrap_or("unknown"))
                }
                ConverseError::AccessDeniedException(e) => {
                    format!("Access denied: {}", e.message().unwrap_or("unknown"))
                }
                ConverseError::ResourceNotFoundException(e) => {
                    format!("Resource not found: {}", e.message().unwrap_or("unknown"))
                }
                ConverseError::ModelErrorException(e) => {
                    format!("Model error: {}", e.message().unwrap_or("unknown"))
                }
                ConverseError::InternalServerException(e) => {
                    format!(
                        "Internal server error: {}",
                        e.message().unwrap_or("unknown")
                    )
                }
                ConverseError::ServiceUnavailableException(e) => {
                    format!("Service unavailable: {}", e.message().unwrap_or("unknown"))
                }
                _ => format!("Bedrock service error: {}", service_err.err()),
            };
            LlmError::RequestFailed {
                provider: "bedrock".to_string(),
                reason: msg,
            }
        }
        SdkError::TimeoutError(_) => LlmError::RequestFailed {
            provider: "bedrock".to_string(),
            reason: "Request timed out".to_string(),
        },
        SdkError::DispatchFailure(e) => LlmError::RequestFailed {
            provider: "bedrock".to_string(),
            reason: format!("Connection error: {:?}", e),
        },
        _ => LlmError::RequestFailed {
            provider: "bedrock".to_string(),
            reason: format!("AWS SDK error: {}", error),
        },
    }
}

// ---------------------------------------------------------------------------
// Document ↔ serde_json::Value conversion
// ---------------------------------------------------------------------------

/// Convert `serde_json::Value` to `aws_smithy_types::Document`.
pub(crate) fn json_to_document(value: &serde_json::Value) -> Document {
    match value {
        serde_json::Value::Null => Document::Null,
        serde_json::Value::Bool(b) => Document::Bool(*b),
        serde_json::Value::Number(n) => {
            if let Some(u) = n.as_u64() {
                Document::Number(aws_smithy_types::Number::PosInt(u))
            } else if let Some(i) = n.as_i64() {
                Document::Number(aws_smithy_types::Number::NegInt(i))
            } else if let Some(f) = n.as_f64() {
                Document::Number(aws_smithy_types::Number::Float(f))
            } else {
                Document::Null
            }
        }
        serde_json::Value::String(s) => Document::String(s.clone()),
        serde_json::Value::Array(arr) => {
            Document::Array(arr.iter().map(json_to_document).collect())
        }
        serde_json::Value::Object(obj) => {
            let map: HashMap<String, Document> = obj
                .iter()
                .map(|(k, v)| (k.clone(), json_to_document(v)))
                .collect();
            Document::Object(map)
        }
    }
}

/// Convert `aws_smithy_types::Document` to `serde_json::Value`.
pub(crate) fn document_to_json(doc: &Document) -> serde_json::Value {
    match doc {
        Document::Null => serde_json::Value::Null,
        Document::Bool(b) => serde_json::Value::Bool(*b),
        Document::Number(n) => match n {
            aws_smithy_types::Number::PosInt(u) => {
                serde_json::Value::Number(serde_json::Number::from(*u))
            }
            aws_smithy_types::Number::NegInt(i) => {
                serde_json::Value::Number(serde_json::Number::from(*i))
            }
            aws_smithy_types::Number::Float(f) => serde_json::Number::from_f64(*f)
                .map(serde_json::Value::Number)
                .unwrap_or(serde_json::Value::Null),
        },
        Document::String(s) => serde_json::Value::String(s.clone()),
        Document::Array(arr) => {
            serde_json::Value::Array(arr.iter().map(document_to_json).collect())
        }
        Document::Object(obj) => {
            let map: serde_json::Map<String, serde_json::Value> = obj
                .iter()
                .map(|(k, v)| (k.clone(), document_to_json(v)))
                .collect();
            serde_json::Value::Object(map)
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::provider::{ChatMessage, Role};

    #[test]
    fn test_json_to_document_round_trip() {
        let json = serde_json::json!({
            "name": "test",
            "count": 42,
            "negative": -7,
            "ratio": 3.125,
            "active": true,
            "nothing": null,
            "tags": ["a", "b"],
            "nested": {"x": 1}
        });

        let doc = json_to_document(&json);
        let back = document_to_json(&doc);

        assert_eq!(json, back);
    }

    #[test]
    fn test_json_to_document_empty_object() {
        let json = serde_json::json!({});
        let doc = json_to_document(&json);
        let back = document_to_json(&doc);
        assert_eq!(json, back);
    }

    #[test]
    fn test_convert_messages_system_extraction() {
        let messages = vec![
            ChatMessage::system("You are helpful."),
            ChatMessage::system("Be concise."),
            ChatMessage::user("Hello"),
        ];

        let (system, msgs) = convert_messages(&messages).unwrap();

        assert_eq!(system.len(), 2);
        assert_eq!(msgs.len(), 1);
        assert_eq!(*msgs[0].role(), ConversationRole::User);
    }

    #[test]
    fn test_convert_messages_basic_conversation() {
        let messages = vec![
            ChatMessage::user("Hi"),
            ChatMessage::assistant("Hello!"),
            ChatMessage::user("How are you?"),
        ];

        let (system, msgs) = convert_messages(&messages).unwrap();

        assert!(system.is_empty());
        assert_eq!(msgs.len(), 3);
        assert_eq!(*msgs[0].role(), ConversationRole::User);
        assert_eq!(*msgs[1].role(), ConversationRole::Assistant);
        assert_eq!(*msgs[2].role(), ConversationRole::User);
    }

    #[test]
    fn test_convert_messages_tool_results_merge_into_user() {
        let tc = crate::llm::provider::ToolCall {
            id: "call_1".to_string(),
            name: "echo".to_string(),
            arguments: serde_json::json!({"text": "hi"}),
        };
        let tc2 = crate::llm::provider::ToolCall {
            id: "call_2".to_string(),
            name: "time".to_string(),
            arguments: serde_json::json!({}),
        };

        let messages = vec![
            ChatMessage::user("Do things"),
            ChatMessage::assistant_with_tool_calls(None, vec![tc, tc2]),
            ChatMessage::tool_result("call_1", "echo", "hi back"),
            ChatMessage::tool_result("call_2", "time", "12:00"),
        ];

        let (_, msgs) = convert_messages(&messages).unwrap();

        // user, assistant (with tool_use), user (with merged tool_results)
        assert_eq!(msgs.len(), 3);
        assert_eq!(*msgs[2].role(), ConversationRole::User);
        // The merged user message should have 2 content blocks (both ToolResult)
        assert_eq!(msgs[2].content().len(), 2);
        assert!(msgs[2].content()[0].is_tool_result());
        assert!(msgs[2].content()[1].is_tool_result());
    }

    #[test]
    fn test_convert_messages_consecutive_users_merge() {
        let messages = vec![ChatMessage::user("First"), ChatMessage::user("Second")];

        let (_, msgs) = convert_messages(&messages).unwrap();

        // Should merge into a single User message with 2 text blocks
        assert_eq!(msgs.len(), 1);
        assert_eq!(*msgs[0].role(), ConversationRole::User);
        assert_eq!(msgs[0].content().len(), 2);
    }

    #[test]
    fn test_convert_messages_assistant_with_tool_calls() {
        let tc = crate::llm::provider::ToolCall {
            id: "call_1".to_string(),
            name: "search".to_string(),
            arguments: serde_json::json!({"query": "test"}),
        };

        let messages = vec![
            ChatMessage::user("Search for test"),
            ChatMessage::assistant_with_tool_calls(Some("Let me search.".to_string()), vec![tc]),
        ];

        let (_, msgs) = convert_messages(&messages).unwrap();

        assert_eq!(msgs.len(), 2);
        assert_eq!(*msgs[1].role(), ConversationRole::Assistant);
        // Should have text + tool_use
        assert_eq!(msgs[1].content().len(), 2);
        assert!(msgs[1].content()[0].is_text());
        assert!(msgs[1].content()[1].is_tool_use());
    }

    #[test]
    fn test_convert_messages_empty_assistant_content_with_tool_calls() {
        let tc = crate::llm::provider::ToolCall {
            id: "call_1".to_string(),
            name: "echo".to_string(),
            arguments: serde_json::json!({}),
        };

        let messages = vec![
            ChatMessage::user("Go"),
            ChatMessage::assistant_with_tool_calls(None, vec![tc]),
        ];

        let (_, msgs) = convert_messages(&messages).unwrap();

        assert_eq!(msgs.len(), 2);
        // Empty text should not add a Text block
        let assistant_content = msgs[1].content();
        assert_eq!(assistant_content.len(), 1);
        assert!(assistant_content[0].is_tool_use());
    }

    #[test]
    fn test_build_tool_config_empty_tools() {
        let result = build_tool_config(&[], None).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_build_tool_config_none_choice() {
        let result = build_tool_config(&[], Some("none")).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_build_tool_config_with_tools() {
        let tools = vec![ToolDefinition {
            name: "echo".to_string(),
            description: "Echoes input".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "text": {"type": "string"}
                }
            }),
        }];

        let result = build_tool_config(&tools, Some("auto")).unwrap();
        assert!(result.is_some());
    }

    #[test]
    fn test_map_stop_reason() {
        assert_eq!(map_stop_reason(&StopReason::EndTurn), FinishReason::Stop);
        assert_eq!(
            map_stop_reason(&StopReason::StopSequence),
            FinishReason::Stop
        );
        assert_eq!(map_stop_reason(&StopReason::ToolUse), FinishReason::ToolUse);
        assert_eq!(
            map_stop_reason(&StopReason::MaxTokens),
            FinishReason::Length
        );
        assert_eq!(
            map_stop_reason(&StopReason::ContentFiltered),
            FinishReason::ContentFilter
        );
    }

    #[test]
    fn test_model_id_with_cross_region() {
        // Simulate what the constructor does
        let prefix = "us.";
        let model = "anthropic.claude-opus-4-6-v1";
        let model_id = format!("{}{}", prefix, model);
        assert_eq!(model_id, "us.anthropic.claude-opus-4-6-v1");
    }

    #[test]
    fn test_model_id_without_cross_region() {
        let prefix = "";
        let model = "anthropic.claude-opus-4-6-v1";
        let model_id = format!("{}{}", prefix, model);
        assert_eq!(model_id, "anthropic.claude-opus-4-6-v1");
    }

    #[test]
    fn test_convert_messages_tool_result_after_regular_user() {
        // Edge case: tool result appears after a user message (from sanitize_tool_messages rewrite)
        // This shouldn't happen normally but we should handle it gracefully
        let messages = vec![
            ChatMessage::user("Hello"),
            ChatMessage {
                role: Role::Tool,
                content: "result".to_string(),
                tool_call_id: Some("call_1".to_string()),
                name: Some("echo".to_string()),
                tool_calls: None,
            },
        ];

        let (_, msgs) = convert_messages(&messages).unwrap();

        // User + tool result (as user) = should merge into one User message
        assert_eq!(msgs.len(), 1);
        assert_eq!(*msgs[0].role(), ConversationRole::User);
    }

    #[test]
    fn test_extract_token_usage_present() {
        let usage = aws_sdk_bedrockruntime::types::TokenUsage::builder()
            .input_tokens(150)
            .output_tokens(42)
            .total_tokens(192)
            .build()
            .unwrap();
        let (input, output) = extract_token_usage(Some(&usage));
        assert_eq!(input, 150);
        assert_eq!(output, 42);
    }

    #[test]
    fn test_extract_token_usage_none() {
        let (input, output) = extract_token_usage(None);
        assert_eq!(input, 0);
        assert_eq!(output, 0);
    }

    #[test]
    fn test_extract_token_usage_negative_clamps_to_zero() {
        // Bedrock uses i32; negative values should not panic
        let usage = aws_sdk_bedrockruntime::types::TokenUsage::builder()
            .input_tokens(-1)
            .output_tokens(-5)
            .total_tokens(0)
            .build()
            .unwrap();
        let (input, output) = extract_token_usage(Some(&usage));
        assert_eq!(input, 0);
        assert_eq!(output, 0);
    }

    #[test]
    fn test_json_to_document_nested_arrays() {
        let json = serde_json::json!([[1, 2], [3, 4]]);
        let doc = json_to_document(&json);
        let back = document_to_json(&doc);
        assert_eq!(json, back);
    }

    #[test]
    fn test_json_to_document_large_numbers() {
        let json = serde_json::json!({
            "big_pos": u64::MAX,
            "big_neg": i64::MIN,
        });
        let doc = json_to_document(&json);
        let back = document_to_json(&doc);
        assert_eq!(json, back);
    }

    #[test]
    fn test_full_tool_round_trip_conversation() {
        // Simulate a complete tool-use conversation:
        // system → user → assistant(tool_calls) → tool_results → user follow-up
        let tc1 = crate::llm::provider::ToolCall {
            id: "call_abc".to_string(),
            name: "get_weather".to_string(),
            arguments: serde_json::json!({"city": "NYC"}),
        };
        let tc2 = crate::llm::provider::ToolCall {
            id: "call_def".to_string(),
            name: "get_time".to_string(),
            arguments: serde_json::json!({"tz": "EST"}),
        };

        let messages = vec![
            ChatMessage::system("You are a helpful assistant."),
            ChatMessage::user("What's the weather and time in NYC?"),
            ChatMessage::assistant_with_tool_calls(
                Some("Let me check both.".to_string()),
                vec![tc1, tc2],
            ),
            ChatMessage::tool_result("call_abc", "get_weather", "72°F and sunny"),
            ChatMessage::tool_result("call_def", "get_time", "3:45 PM EST"),
            ChatMessage::user("Thanks! What about tomorrow?"),
        ];

        let (system, msgs) = convert_messages(&messages).unwrap();

        // 1 system block
        assert_eq!(system.len(), 1);

        // Messages: user, assistant(text+2 tool_use), user(2 tool_results + follow-up text merged)
        // The follow-up user message "Thanks!" merges into the tool_results User message
        // because Bedrock requires strict user/assistant alternation.
        assert_eq!(msgs.len(), 3);

        // msg[0]: user "What's the weather..."
        assert_eq!(*msgs[0].role(), ConversationRole::User);
        assert_eq!(msgs[0].content().len(), 1);
        assert!(msgs[0].content()[0].is_text());

        // msg[1]: assistant with text + 2 tool_use blocks
        assert_eq!(*msgs[1].role(), ConversationRole::Assistant);
        assert_eq!(msgs[1].content().len(), 3); // text + 2 tool_use
        assert!(msgs[1].content()[0].is_text());
        assert!(msgs[1].content()[1].is_tool_use());
        assert!(msgs[1].content()[2].is_tool_use());

        // Verify tool_use IDs and arguments survived conversion
        let tu1 = msgs[1].content()[1].as_tool_use().unwrap();
        assert_eq!(tu1.tool_use_id(), "call_abc");
        assert_eq!(tu1.name(), "get_weather");
        let args1 = document_to_json(tu1.input());
        assert_eq!(args1, serde_json::json!({"city": "NYC"}));

        let tu2 = msgs[1].content()[2].as_tool_use().unwrap();
        assert_eq!(tu2.tool_use_id(), "call_def");
        assert_eq!(tu2.name(), "get_time");

        // msg[2]: user with 2 tool_result blocks + merged follow-up text
        // Tool results are User-role, and "Thanks!" is also User-role, so they merge.
        assert_eq!(*msgs[2].role(), ConversationRole::User);
        assert_eq!(msgs[2].content().len(), 3); // 2 tool_results + 1 text
        assert!(msgs[2].content()[0].is_tool_result());
        assert!(msgs[2].content()[1].is_tool_result());
        assert!(msgs[2].content()[2].is_text());

        // Verify tool_result IDs and content
        let tr1 = msgs[2].content()[0].as_tool_result().unwrap();
        assert_eq!(tr1.tool_use_id(), "call_abc");
        assert_eq!(tr1.content().len(), 1);

        let tr2 = msgs[2].content()[1].as_tool_result().unwrap();
        assert_eq!(tr2.tool_use_id(), "call_def");
    }

    #[test]
    fn test_convert_messages_empty_input() {
        let (system, msgs) = convert_messages(&[]).unwrap();
        assert!(system.is_empty());
        assert!(msgs.is_empty());
    }

    #[test]
    fn test_convert_messages_system_only() {
        let messages = vec![ChatMessage::system("You are helpful.")];
        let (system, msgs) = convert_messages(&messages).unwrap();
        assert_eq!(system.len(), 1);
        assert!(msgs.is_empty());
    }

    #[test]
    fn test_build_tool_config_required_choice() {
        let tools = vec![ToolDefinition {
            name: "echo".to_string(),
            description: "Echoes".to_string(),
            parameters: serde_json::json!({"type": "object"}),
        }];

        let result = build_tool_config(&tools, Some("required")).unwrap();
        assert!(result.is_some());
    }

    #[test]
    fn test_map_stop_reason_all_variants() {
        assert_eq!(
            map_stop_reason(&StopReason::GuardrailIntervened),
            FinishReason::ContentFilter
        );
        assert_eq!(
            map_stop_reason(&StopReason::ModelContextWindowExceeded),
            FinishReason::Length
        );
    }
}
