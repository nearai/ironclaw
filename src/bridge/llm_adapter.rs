//! LLM bridge adapter — wraps `LlmProvider` as `ironclaw_engine::LlmBackend`.

use std::sync::Arc;

use ironclaw_engine::{
    ActionDef, EngineError, LlmBackend, LlmCallConfig, LlmOutput, LlmResponse, ThreadMessage,
    TokenUsage,
};

use crate::llm::{
    ChatMessage, LlmProvider, Role, ToolCall, ToolCompletionRequest, ToolDefinition,
    sanitize_tool_messages,
};

/// Wraps an existing `LlmProvider` to implement the engine's `LlmBackend` trait.
pub struct LlmBridgeAdapter {
    provider: Arc<dyn LlmProvider>,
    /// Optional cheaper provider for sub-calls (depth > 0).
    cheap_provider: Option<Arc<dyn LlmProvider>>,
}

impl LlmBridgeAdapter {
    pub fn new(
        provider: Arc<dyn LlmProvider>,
        cheap_provider: Option<Arc<dyn LlmProvider>>,
    ) -> Self {
        Self {
            provider,
            cheap_provider,
        }
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
        let mut chat_messages: Vec<ChatMessage> = messages.iter().map(thread_msg_to_chat).collect();
        sanitize_tool_messages(&mut chat_messages);

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

            let response = provider
                .complete(request)
                .await
                .map_err(|e| EngineError::Llm {
                    reason: e.to_string(),
                })?;

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

        // Call provider
        let response =
            provider
                .complete_with_tools(request)
                .await
                .map_err(|e| EngineError::Llm {
                    reason: e.to_string(),
                })?;

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
                    // For bare ``` blocks (no explicit language tag) only
                    // accept content that actually looks like Python. Without
                    // this guard, the agent's example markdown blocks
                    // (lists, tables, plain prose) get misclassified as code
                    // and explode in the Monty parser with SyntaxError —
                    // which the LLM then has to recover from.
                    if marker == "```" && !looks_like_python(code) {
                        search_from = code_start + end + 3;
                        continue;
                    }
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

/// Heuristic check that a bare ``` block contains Python rather than
/// markdown / prose / a different language.
///
/// Accepts: assignments (`x =`), function calls (`name(`), Python keywords
/// (`import`, `from`, `def`, `class`, `if`, `for`, `while`, `return`,
/// `print`, `FINAL`, `try`, `with`, `pass`, `raise`, `yield`, `lambda`),
/// or comments (`#`).
///
/// Rejects: lines starting with `-`, `*`, `|`, `>`, `:`, digits followed by
/// `.` (markdown lists, tables, blockquotes, headings, numbered lists),
/// bare prose, etc.
/// Returns true when `line` contains an identifier-style function call
/// (an identifier or attribute path immediately followed by `(`).
///
/// Avoids the false positives `trimmed.contains('(')` produced for markdown
/// links like `[text](url)` and prose like "See (docs)" — neither has an
/// alphanumeric/underscore character directly before the `(`.
fn has_identifier_call(line: &str) -> bool {
    let bytes = line.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'(' && i > 0 {
            let prev = bytes[i - 1];
            if prev.is_ascii_alphanumeric() || prev == b'_' {
                return true;
            }
        }
    }
    false
}

fn looks_like_python(code: &str) -> bool {
    const PY_KEYWORDS: &[&str] = &[
        "import", "from", "def", "class", "if", "for", "while", "return", "print", "FINAL", "try",
        "with", "pass", "raise", "yield", "lambda", "elif", "else", "async", "await", "global",
        "nonlocal", "assert", "break", "continue", "del", "not", "and", "or", "is", "in",
    ];

    // Check the first few non-empty lines — at least one must look Python-y.
    for line in code.lines().take(5) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        // Comments are valid Python.
        if trimmed.starts_with('#') {
            return true;
        }
        // Markdown markers are NOT Python.
        if trimmed.starts_with('-')
            || trimmed.starts_with('*')
            || trimmed.starts_with('|')
            || trimmed.starts_with('>')
        {
            return false;
        }
        // Markdown numbered list "1. foo" is NOT Python (a Python statement
        // starting with a literal int is `123` followed by an operator, not
        // `123. text`).
        if trimmed.chars().next().is_some_and(|c| c.is_ascii_digit()) && trimmed.contains(". ") {
            return false;
        }
        // Function call: an identifier (or attribute path) followed by `(`,
        // e.g. `foo(...)`, `obj.method(...)`. We require the `(` to be
        // preceded by an identifier char so markdown links like `[text](url)`
        // and prose like "See (docs)" don't get classified as Python.
        if has_identifier_call(trimmed) {
            return true;
        }
        // Assignment: `name = ...` (but not `==` comparisons in prose).
        if trimmed.contains('=') {
            return true;
        }
        // First word matches a Python keyword.
        let first_word: String = trimmed
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if PY_KEYWORDS.contains(&first_word.as_str()) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use async_trait::async_trait;
    use rust_decimal::Decimal;

    use ironclaw_engine::{ActionCall, ActionDef, EffectType, LlmResponse, ThreadMessage};

    use crate::error::LlmError;
    use crate::llm::ToolCompletionResponse;

    #[derive(Default)]
    struct CapturingProviderState {
        completion_requests: tokio::sync::Mutex<Vec<Vec<ChatMessage>>>,
        tool_requests: tokio::sync::Mutex<Vec<Vec<ChatMessage>>>,
    }

    struct CapturingProvider {
        state: Arc<CapturingProviderState>,
    }

    #[async_trait]
    impl LlmProvider for CapturingProvider {
        fn model_name(&self) -> &str {
            "capturing-provider"
        }

        fn cost_per_token(&self) -> (Decimal, Decimal) {
            (Decimal::ZERO, Decimal::ZERO)
        }

        async fn complete(
            &self,
            req: crate::llm::CompletionRequest,
        ) -> Result<crate::llm::CompletionResponse, LlmError> {
            self.state
                .completion_requests
                .lock()
                .await
                .push(req.messages);

            Ok(crate::llm::CompletionResponse {
                content: "ok".to_string(),
                input_tokens: 1,
                output_tokens: 1,
                finish_reason: crate::llm::FinishReason::Stop,
                cache_read_input_tokens: 0,
                cache_creation_input_tokens: 0,
            })
        }

        async fn complete_with_tools(
            &self,
            req: ToolCompletionRequest,
        ) -> Result<ToolCompletionResponse, LlmError> {
            self.state.tool_requests.lock().await.push(req.messages);

            Ok(ToolCompletionResponse {
                content: Some("ok".to_string()),
                tool_calls: Vec::new(),
                input_tokens: 1,
                output_tokens: 1,
                finish_reason: crate::llm::FinishReason::Stop,
                cache_read_input_tokens: 0,
                cache_creation_input_tokens: 0,
            })
        }
    }

    fn test_action(name: &str) -> ActionDef {
        ActionDef {
            name: name.to_string(),
            description: format!("Test action {name}"),
            parameters_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
            effects: vec![EffectType::ReadExternal],
            requires_approval: false,
        }
    }

    #[tokio::test]
    async fn complete_with_tools_rewrites_orphaned_action_results_before_provider_call() {
        let state = Arc::new(CapturingProviderState::default());
        let provider: Arc<dyn LlmProvider> = Arc::new(CapturingProvider {
            state: state.clone(),
        });
        let adapter = LlmBridgeAdapter::new(provider, None);
        let messages = vec![
            ThreadMessage::user("Find the docs"),
            ThreadMessage::assistant("I checked a tool earlier."),
            ThreadMessage::action_result("call_missing", "search", "result payload"),
        ];

        let output = adapter
            .complete(
                &messages,
                &[test_action("search")],
                &LlmCallConfig::default(),
            )
            .await
            .unwrap();

        match output.response {
            LlmResponse::Text(ref text) => assert_eq!(text, "ok"),
            other => panic!("expected text response, got {other:?}"),
        }

        let tool_requests = state.tool_requests.lock().await;
        let sent = tool_requests.last().unwrap();

        assert_eq!(sent.len(), 3);
        assert_eq!(sent[2].role, Role::User);
        assert_eq!(sent[2].content, "[Tool `search` returned: result payload]");
        assert!(sent[2].tool_call_id.is_none());
        assert!(sent[2].name.is_none());
    }

    #[tokio::test]
    async fn complete_without_tools_rewrites_orphaned_action_results_before_provider_call() {
        let state = Arc::new(CapturingProviderState::default());
        let provider: Arc<dyn LlmProvider> = Arc::new(CapturingProvider {
            state: state.clone(),
        });
        let adapter = LlmBridgeAdapter::new(provider, None);
        let messages = vec![
            ThreadMessage::user("Find the docs"),
            ThreadMessage::assistant("I checked a tool earlier."),
            ThreadMessage::action_result("call_missing", "search", "result payload"),
        ];

        let output = adapter
            .complete(&messages, &[], &LlmCallConfig::default())
            .await
            .unwrap();

        match output.response {
            LlmResponse::Text(ref text) => assert_eq!(text, "ok"),
            other => panic!("expected text response, got {other:?}"),
        }

        let completion_requests = state.completion_requests.lock().await;
        let sent = completion_requests.last().unwrap();

        assert_eq!(sent.len(), 3);
        assert_eq!(sent[2].role, Role::User);
        assert_eq!(sent[2].content, "[Tool `search` returned: result payload]");
        assert!(sent[2].tool_call_id.is_none());
        assert!(sent[2].name.is_none());
    }

    #[tokio::test]
    async fn complete_with_tools_preserves_matched_action_results() {
        let state = Arc::new(CapturingProviderState::default());
        let provider: Arc<dyn LlmProvider> = Arc::new(CapturingProvider {
            state: state.clone(),
        });
        let adapter = LlmBridgeAdapter::new(provider, None);
        let messages = vec![
            ThreadMessage::user("Find the docs"),
            ThreadMessage::assistant_with_actions(
                Some("Using search".to_string()),
                vec![ActionCall {
                    id: "call_1".to_string(),
                    action_name: "search".to_string(),
                    parameters: serde_json::json!({"q": "docs"}),
                }],
            ),
            ThreadMessage::action_result("call_1", "search", "result payload"),
        ];

        let output = adapter
            .complete(
                &messages,
                &[test_action("search")],
                &LlmCallConfig::default(),
            )
            .await
            .unwrap();

        match output.response {
            LlmResponse::Text(ref text) => assert_eq!(text, "ok"),
            other => panic!("expected text response, got {other:?}"),
        }

        let tool_requests = state.tool_requests.lock().await;
        let sent = tool_requests.last().unwrap();

        assert_eq!(sent.len(), 3);
        assert_eq!(sent[2].role, Role::Tool);
        assert_eq!(sent[2].content, "result payload");
        assert_eq!(sent[2].tool_call_id.as_deref(), Some("call_1"));
        assert_eq!(sent[2].name.as_deref(), Some("search"));
    }

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
        // Bare ``` blocks are accepted ONLY when the content looks like
        // Python (assignment, function call, keyword, or comment). The
        // `looks_like_python` heuristic prevents the LLM's example markdown
        // from being misclassified as code (which used to crash Monty
        // with a SyntaxError on `- TICKER: SIZE, ...` style content).
        let text = "Here's the code:\n```\nx = 42\nFINAL(x)\n```";
        let code = extract_code_block(text).unwrap();
        assert_eq!(code, "x = 42\nFINAL(x)");
    }

    #[test]
    fn bare_backtick_markdown_list_is_rejected() {
        let text = "Example positions file:\n```\n- AAPL: 500 shares, entry $175\n- TSLA: 200 shares, entry $260\n```";
        assert!(
            extract_code_block(text).is_none(),
            "markdown list inside bare ``` should NOT be treated as Python"
        );
    }

    #[test]
    fn bare_backtick_markdown_table_is_rejected() {
        let text = "Schema:\n```\n| col | type |\n| --- | --- |\n| id  | int  |\n```";
        assert!(
            extract_code_block(text).is_none(),
            "markdown table inside bare ``` should NOT be treated as Python"
        );
    }

    #[test]
    fn bare_backtick_prose_is_rejected() {
        let text = "Here's a quote:\n```\nThe quick brown fox jumps over the lazy dog.\n```";
        assert!(
            extract_code_block(text).is_none(),
            "prose inside bare ``` should NOT be treated as Python"
        );
    }

    #[test]
    fn bare_backtick_markdown_link_is_rejected() {
        // Regression test for PR #1736 review (Copilot, 3057247912):
        // `looks_like_python` previously matched any line containing `(`,
        // which classified markdown links like `[text](url)` and prose
        // like "See (docs)" as Python and forwarded them to Monty as code.
        let link_text = "Read more:\n```\n[the docs](https://example.com)\n```";
        assert!(
            extract_code_block(link_text).is_none(),
            "markdown link inside bare ``` should NOT be treated as Python"
        );

        let parens_prose = "Note:\n```\nSee (docs) for details on the API.\n```";
        assert!(
            extract_code_block(parens_prose).is_none(),
            "prose with parenthetical inside bare ``` should NOT be treated as Python"
        );
    }

    #[test]
    fn bare_backtick_python_with_comment() {
        let text = "```\n# fetch the data\nresult = fetch()\nFINAL(result)\n```";
        let code = extract_code_block(text).unwrap();
        assert!(code.contains("fetch()"));
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
