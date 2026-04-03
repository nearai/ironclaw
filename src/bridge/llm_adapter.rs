//! LLM bridge adapter — wraps `LlmProvider` as `ironclaw_engine::LlmBackend`.

use std::sync::Arc;

use ironclaw_engine::{
    ActionDef, EngineError, LlmBackend, LlmCallConfig, LlmOutput, LlmResponse, ThreadMessage,
    TokenUsage,
};

use crate::llm::{ChatMessage, LlmProvider, Role, ToolCall, ToolCompletionRequest, ToolDefinition};

/// Wraps an existing `LlmProvider` to implement the engine's `LlmBackend` trait.
pub struct LlmBridgeAdapter {
    provider: Arc<dyn LlmProvider>,
    /// Optional cheaper provider for sub-calls (depth > 0).
    cheap_provider: Option<Arc<dyn LlmProvider>>,
    /// Optional callback for streaming content tokens.
    on_token: Option<Arc<dyn Fn(String) + Send + Sync>>,
}

impl LlmBridgeAdapter {
    pub fn new(
        provider: Arc<dyn LlmProvider>,
        cheap_provider: Option<Arc<dyn LlmProvider>>,
    ) -> Self {
        Self {
            provider,
            cheap_provider,
            on_token: None,
        }
    }

    /// Set the streaming token callback.
    pub fn with_on_token(mut self, cb: Arc<dyn Fn(String) + Send + Sync>) -> Self {
        self.on_token = Some(cb);
        self
    }

    fn provider_for_depth(&self, depth: u32) -> &Arc<dyn LlmProvider> {
        if depth > 0 {
            self.cheap_provider.as_ref().unwrap_or(&self.provider)
        } else {
            &self.provider
        }
    }
}

#[async_trait::async_trait]
impl LlmBackend for LlmBridgeAdapter {
    async fn complete(
        &self,
        messages: &[ThreadMessage],
        actions: &[ActionDef],
        config: &LlmCallConfig,
    ) -> Result<LlmOutput, EngineError> {
        let provider = self.provider_for_depth(config.depth);

        // Convert messages
        let chat_messages: Vec<ChatMessage> = messages.iter().map(thread_msg_to_chat).collect();

        // Convert actions to tool definitions
        let tools: Vec<ToolDefinition> = if config.force_text {
            vec![] // No tools when forcing text
        } else {
            actions.iter().map(action_def_to_tool_def).collect()
        };

        // Build request — match the existing Reasoning.respond_with_tools() defaults
        let max_tokens = config.max_tokens.unwrap_or(4096);
        let temperature = config.temperature.unwrap_or(0.7);

        if tools.is_empty() {
            // No tools: use plain completion (matches existing no-tools path)
            let mut request = crate::llm::CompletionRequest::new(chat_messages)
                .with_max_tokens(max_tokens)
                .with_temperature(temperature);
            request.metadata = config.metadata.clone();

            let response = if let Some(ref cb) = self.on_token {
                provider
                    .complete_streaming(request, cb.as_ref())
                    .await
                    .map_err(|e| EngineError::Llm {
                        reason: e.to_string(),
                    })?
            } else {
                provider
                    .complete(request)
                    .await
                    .map_err(|e| EngineError::Llm {
                        reason: e.to_string(),
                    })?
            };

            // Check for code blocks in the response (CodeAct/RLM pattern)
            let llm_response = match extract_code_block(&response.content) {
                Some(code) => LlmResponse::Code {
                    code,
                    content: Some(response.content),
                },
                None => LlmResponse::Text(response.content),
            };

            return Ok(LlmOutput {
                response: llm_response,
                usage: TokenUsage {
                    input_tokens: u64::from(response.input_tokens),
                    output_tokens: u64::from(response.output_tokens),
                    cache_read_tokens: u64::from(response.cache_read_input_tokens),
                    cache_write_tokens: u64::from(response.cache_creation_input_tokens),
                    cost_usd: 0.0,
                },
            });
        }

        // With tools: use tool completion (matches existing tools path)
        let mut request = ToolCompletionRequest::new(chat_messages, tools)
            .with_max_tokens(max_tokens)
            .with_temperature(temperature)
            .with_tool_choice("auto");
        request.metadata = config.metadata.clone();

        // Call provider (streaming when callback is set)
        let response = if let Some(ref cb) = self.on_token {
            provider
                .complete_with_tools_streaming(request, cb.as_ref())
                .await
                .map_err(|e| EngineError::Llm {
                    reason: e.to_string(),
                })?
        } else {
            provider
                .complete_with_tools(request)
                .await
                .map_err(|e| EngineError::Llm {
                    reason: e.to_string(),
                })?
        };

        // Convert response — check for code blocks (CodeAct/RLM pattern)
        let llm_response = if !response.tool_calls.is_empty() {
            LlmResponse::ActionCalls {
                calls: response
                    .tool_calls
                    .iter()
                    .map(|tc| ironclaw_engine::ActionCall {
                        id: tc.id.clone(),
                        action_name: tc.name.clone(),
                        parameters: tc.arguments.clone(),
                    })
                    .collect(),
                content: response.content.clone(),
            }
        } else {
            let text = response.content.unwrap_or_default();
            // Detect ```repl or ```python fenced code blocks
            match extract_code_block(&text) {
                Some(code) => LlmResponse::Code {
                    code,
                    content: Some(text),
                },
                None => LlmResponse::Text(text),
            }
        };

        Ok(LlmOutput {
            response: llm_response,
            usage: TokenUsage {
                input_tokens: u64::from(response.input_tokens),
                output_tokens: u64::from(response.output_tokens),
                cache_read_tokens: u64::from(response.cache_read_input_tokens),
                cache_write_tokens: u64::from(response.cache_creation_input_tokens),
                cost_usd: 0.0, // TODO: populate from provider cost data when available
            },
        })
    }

    fn model_name(&self) -> &str {
        self.provider.model_name()
    }
}

// ── Conversion helpers ──────────────────────────────────────

fn thread_msg_to_chat(msg: &ThreadMessage) -> ChatMessage {
    use ironclaw_engine::MessageRole;

    let role = match msg.role {
        MessageRole::System => Role::System,
        MessageRole::User => Role::User,
        MessageRole::Assistant => Role::Assistant,
        MessageRole::ActionResult => Role::Tool,
    };

    let mut chat = ChatMessage {
        role,
        content: msg.content.clone(),
        content_parts: Vec::new(),
        tool_call_id: msg.action_call_id.clone(),
        name: msg.action_name.clone(),
        tool_calls: None,
    };

    // Convert action calls if present (assistant message with tool calls)
    if let Some(ref calls) = msg.action_calls {
        chat.tool_calls = Some(
            calls
                .iter()
                .map(|c| ToolCall {
                    id: c.id.clone(),
                    name: c.action_name.clone(),
                    arguments: c.parameters.clone(),
                    reasoning: None,
                })
                .collect(),
        );
    }

    chat
}

fn action_def_to_tool_def(action: &ActionDef) -> ToolDefinition {
    ToolDefinition {
        name: action.name.clone(),
        description: action.description.clone(),
        parameters: action.parameters_schema.clone(),
    }
}

/// Extract Python code from fenced code blocks in the LLM response.
///
/// Tries these markers in order: ```repl, ```python, ```py, then bare ```
/// (if the content looks like Python). Collects ALL code blocks in the
/// response and concatenates them (models sometimes split code across
/// multiple blocks with explanation text between them).
fn extract_code_block(text: &str) -> Option<String> {
    let mut all_code = Vec::new();

    // Try specific markers first, then bare backticks
    for marker in ["```repl", "```python", "```py", "```"] {
        let mut search_from = 0;
        while let Some(start) = text[search_from..].find(marker) {
            let abs_start = search_from + start;
            let after_marker = abs_start + marker.len();

            // For bare ```, skip if it's actually ```someotherlang
            if marker == "```" && text[after_marker..].starts_with(|c: char| c.is_alphabetic()) {
                let lang: String = text[after_marker..]
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
                    .collect();
                if !["repl", "python", "py"].contains(&lang.as_str()) {
                    search_from = after_marker;
                    continue;
                }
            }

            // Skip to next line after the marker
            let code_start = text[after_marker..]
                .find('\n')
                .map(|i| after_marker + i + 1)
                .unwrap_or(after_marker);

            // Find closing ```
            if let Some(end) = text[code_start..].find("```") {
                let code = text[code_start..code_start + end].trim();
                if !code.is_empty() {
                    all_code.push(code.to_string());
                }
                search_from = code_start + end + 3;
            } else {
                break;
            }
        }

        // If we found code with a specific marker, use it (don't fall through to bare)
        if !all_code.is_empty() {
            break;
        }
    }

    if all_code.is_empty() {
        return None;
    }

    Some(all_code.join("\n\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── extract_code_block tests ────────────────────────────

    #[test]
    fn extract_repl_block() {
        let text = "Some explanation\n```repl\nx = 1 + 2\nprint(x)\n```\nMore text";
        let code = extract_code_block(text).unwrap();
        assert_eq!(code, "x = 1 + 2\nprint(x)");
    }

    #[test]
    fn extract_python_block() {
        let text = "Let me compute:\n```python\nresult = sum([1,2,3])\n```";
        let code = extract_code_block(text).unwrap();
        assert_eq!(code, "result = sum([1,2,3])");
    }

    #[test]
    fn extract_py_block() {
        let text = "```py\nprint('hello')\n```";
        let code = extract_code_block(text).unwrap();
        assert_eq!(code, "print('hello')");
    }

    #[test]
    fn extract_bare_backtick_block() {
        let text = "Here's the code:\n```\nx = 42\nFINAL(x)\n```";
        let code = extract_code_block(text).unwrap();
        assert_eq!(code, "x = 42\nFINAL(x)");
    }

    #[test]
    fn skip_non_python_language() {
        let text = "```json\n{\"key\": \"value\"}\n```\nThat's the config.";
        assert!(extract_code_block(text).is_none());
    }

    #[test]
    fn no_code_blocks_returns_none() {
        let text = "Just a plain text response with no code.";
        assert!(extract_code_block(text).is_none());
    }

    #[test]
    fn multiple_code_blocks_concatenated() {
        let text = "\
Let me search first:\n\
```repl\nresult = web_search(query=\"test\")\nprint(result)\n```\n\
Now let's process:\n\
```repl\nFINAL(result['title'])\n```";
        let code = extract_code_block(text).unwrap();
        assert!(code.contains("web_search"));
        assert!(code.contains("FINAL"));
        // Two blocks joined by double newline
        assert!(code.contains("\n\n"));
    }

    #[test]
    fn mixed_thinking_and_code() {
        // Simulates a model that outputs explanation + code (the Hyperliquid case)
        let text = "\
Let me help you explore the relationship between Hyperliquid's price and revenue.\n\
\n\
First, let's gather some data:\n\
\n\
```python\nsearch_results = web_search(\n    query=\"Hyperliquid revenue\",\n    count=5\n)\nprint(search_results)\n```\n\
\n\
And also check the token price:\n\
\n\
```python\ntoken_data = web_search(\n    query=\"Hyperliquid token price\",\n    count=3\n)\nprint(token_data)\n```";
        let code = extract_code_block(text).unwrap();
        assert!(code.contains("web_search"));
        assert!(code.contains("Hyperliquid revenue"));
        assert!(code.contains("Hyperliquid token price"));
    }

    #[test]
    fn repl_preferred_over_bare() {
        // If both ```repl and bare ``` exist, prefer ```repl
        let text = "```\nignored\n```\n```repl\nused = True\n```";
        let code = extract_code_block(text).unwrap();
        assert_eq!(code, "used = True");
    }

    #[test]
    fn empty_code_block_skipped() {
        let text = "```python\n\n```\nThat was empty.";
        assert!(extract_code_block(text).is_none());
    }

    #[test]
    fn unclosed_block_returns_none() {
        let text = "```python\nprint('no closing fence')";
        assert!(extract_code_block(text).is_none());
    }
}
