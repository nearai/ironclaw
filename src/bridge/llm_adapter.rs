//! LLM bridge adapter — wraps `LlmProvider` as `ironclaw_engine::LlmBackend`.
//!
//! Code-only contract: the adapter never advertises tool definitions to the
//! provider. Actions are listed in the system prompt; the model writes Python
//! that calls them. Responses are always returned as `LlmResponse::Code`
//! (unless the caller forces `Text` via `LlmCallConfig::force_text` for
//! compaction/sub-query/etc.).

use std::sync::Arc;

use ironclaw_engine::{
    ActionDef, EngineError, LlmBackend, LlmCallConfig, LlmOutput, LlmResponse, ThreadMessage,
    TokenUsage,
};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

use crate::llm::{ChatMessage, LlmProvider, Role, ToolCall, sanitize_tool_messages};

/// Compute the USD cost of a single completion response, honoring the
/// provider's prompt-caching pricing. Mirrors the formula in
/// `src/agent/cost_guard.rs::CostGuard::record_llm_call` so engine v2's
/// `Thread::total_cost_usd` matches what `max_budget_usd` / v1's daily
/// budget enforcer would have computed:
///
/// * uncached input tokens are priced at `cost_per_token().0`;
/// * cache-read tokens are discounted by `cache_read_discount()` (10x
///   off for Anthropic, 2x for OpenAI);
/// * cache-write tokens are multiplied by `cache_write_multiplier()`
///   (1.25× for Anthropic 5m TTL, 2× for 1h);
/// * output tokens are priced at `cost_per_token().1`.
///
/// Returns 0.0 for subscription-billed providers that report
/// `cost_per_token() == (0, 0)` (e.g. OpenAI Codex via ChatGPT OAuth).
fn cost_usd_from(
    provider: &Arc<dyn LlmProvider>,
    input_tokens: u32,
    output_tokens: u32,
    cache_read_input_tokens: u32,
    cache_creation_input_tokens: u32,
) -> f64 {
    let (input_rate, output_rate) = provider.cost_per_token();

    let cached_total = cache_read_input_tokens.saturating_add(cache_creation_input_tokens);
    let uncached_input = input_tokens.saturating_sub(cached_total);

    let discount = provider.cache_read_discount();
    let effective_discount = if discount.is_zero() {
        Decimal::ONE
    } else {
        discount
    };

    let cache_read_cost = input_rate * Decimal::from(cache_read_input_tokens) / effective_discount;
    let cache_write_cost =
        input_rate * Decimal::from(cache_creation_input_tokens) * provider.cache_write_multiplier();
    let cost = input_rate * Decimal::from(uncached_input)
        + cache_read_cost
        + cache_write_cost
        + output_rate * Decimal::from(output_tokens);

    cost.to_f64().unwrap_or(0.0)
}

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
        _actions: &[ActionDef],
        config: &LlmCallConfig,
    ) -> Result<LlmOutput, EngineError> {
        let provider = self.provider_for_depth(config.depth);

        let mut chat_messages: Vec<ChatMessage> = messages.iter().map(thread_msg_to_chat).collect();
        sanitize_tool_messages(&mut chat_messages);

        let max_tokens = config.max_tokens.unwrap_or(4096);
        let temperature = config.temperature.unwrap_or(0.7);

        let mut request = crate::llm::CompletionRequest::new(chat_messages)
            .with_max_tokens(max_tokens)
            .with_temperature(temperature);
        request.metadata = config.metadata.clone();
        if let Some(ref model) = config.model {
            request.model = Some(model.clone());
        }

        let response = provider
            .complete(request)
            .await
            .map_err(|e| EngineError::Llm {
                reason: e.to_string(),
            })?;

        // Two shapes:
        // - force_text=true (compaction, llm_query, sub-queries) returns the
        //   response verbatim so an incidental ```repl fence in the prose
        //   isn't mistakenly extracted as "the answer".
        // - Everyone else: extract a fenced code block if present, else treat
        //   the whole trimmed response as Python. Monty surfaces SyntaxError /
        //   NameError naturally — the LLM reads the traceback and self-corrects.
        let llm_response = if config.force_text {
            LlmResponse::Text(response.content)
        } else {
            let code = extract_code_block(&response.content)
                .unwrap_or_else(|| response.content.trim().to_string());
            LlmResponse::Code {
                code,
                content: Some(response.content),
            }
        };

        Ok(LlmOutput {
            response: llm_response,
            usage: TokenUsage {
                input_tokens: u64::from(response.input_tokens),
                output_tokens: u64::from(response.output_tokens),
                cache_read_tokens: u64::from(response.cache_read_input_tokens),
                cache_write_tokens: u64::from(response.cache_creation_input_tokens),
                cost_usd: cost_usd_from(
                    provider,
                    response.input_tokens,
                    response.output_tokens,
                    response.cache_read_input_tokens,
                    response.cache_creation_input_tokens,
                ),
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

    // Historical messages from pre-simplification threads may still carry
    // tool_calls; preserve them so provider APIs with strict call/result
    // pairing don't reject the history.
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

/// Extract Python code from fenced code blocks in the LLM response.
///
/// Tries these markers in order: ```repl, ```python, ```py, then bare ```
/// (if the content looks like Python). Collects ALL code blocks in the
/// response and concatenates them (models sometimes split code across
/// multiple blocks with explanation text between them).
fn extract_code_block(text: &str) -> Option<String> {
    let mut all_code = Vec::new();

    for marker in ["```repl", "```python", "```py", "```"] {
        let mut search_from = 0;
        while let Some(start) = text[search_from..].find(marker) {
            let abs_start = search_from + start;
            let after_marker = abs_start + marker.len();

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

            let code_start = text[after_marker..]
                .find('\n')
                .map(|i| after_marker + i + 1)
                .unwrap_or(after_marker);

            if let Some(end) = text[code_start..].find("```") {
                let code = text[code_start..code_start + end].trim();
                if !code.is_empty() {
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

        if !all_code.is_empty() {
            break;
        }
    }

    if all_code.is_empty() {
        return None;
    }

    Some(all_code.join("\n\n"))
}

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

    for line in code.lines().take(5) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with('#') {
            return true;
        }
        if trimmed.starts_with('-')
            || trimmed.starts_with('*')
            || trimmed.starts_with('|')
            || trimmed.starts_with('>')
        {
            return false;
        }
        if trimmed.chars().next().is_some_and(|c| c.is_ascii_digit()) && trimmed.contains(". ") {
            return false;
        }
        if has_identifier_call(trimmed) {
            return true;
        }
        if trimmed.contains('=') {
            return true;
        }
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
    use crate::llm::{ToolCompletionRequest, ToolCompletionResponse};

    #[derive(Default)]
    struct CapturingProviderState {
        completion_requests: tokio::sync::Mutex<Vec<Vec<ChatMessage>>>,
        models: tokio::sync::Mutex<Vec<Option<String>>>,
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
            self.state.models.lock().await.push(req.model.clone());
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

        // Code-only contract: the adapter must never drive the provider's
        // tool-call API. If this fires, something regressed.
        async fn complete_with_tools(
            &self,
            _req: ToolCompletionRequest,
        ) -> Result<ToolCompletionResponse, LlmError> {
            unreachable!("LlmBridgeAdapter must never call complete_with_tools under code-only")
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
    async fn complete_rewrites_orphaned_action_results_before_provider_call() {
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

        // "ok" has no fence, so it becomes the Python body under the code-only
        // contract (Monty will surface a NameError for the LLM to recover from).
        match output.response {
            LlmResponse::Code { ref code, .. } => assert_eq!(code, "ok"),
            other => panic!("expected Code response, got {other:?}"),
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
    async fn complete_preserves_matched_action_results_in_history() {
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

        adapter
            .complete(
                &messages,
                &[test_action("search")],
                &LlmCallConfig::default(),
            )
            .await
            .unwrap();

        let completion_requests = state.completion_requests.lock().await;
        let sent = completion_requests.last().unwrap();

        assert_eq!(sent.len(), 3);
        assert_eq!(sent[2].role, Role::Tool);
        assert_eq!(sent[2].content, "result payload");
        assert_eq!(sent[2].tool_call_id.as_deref(), Some("call_1"));
        assert_eq!(sent[2].name.as_deref(), Some("search"));
    }

    #[tokio::test]
    async fn config_model_forwards_to_completion_request() {
        let state = Arc::new(CapturingProviderState::default());
        let provider: Arc<dyn LlmProvider> = Arc::new(CapturingProvider {
            state: state.clone(),
        });
        let adapter = LlmBridgeAdapter::new(provider, None);

        let config = ironclaw_engine::LlmCallConfig {
            model: Some("gpt-4o".into()),
            ..Default::default()
        };

        adapter
            .complete(&[ThreadMessage::user("hi")], &[], &config)
            .await
            .unwrap();

        // Actions are listed in the prompt, not passed to the API — the
        // completion request must still go through the plain `complete` path.
        adapter
            .complete(
                &[ThreadMessage::user("hi")],
                &[ActionDef {
                    name: "echo".into(),
                    description: "test".into(),
                    parameters_schema: serde_json::json!({"type": "object"}),
                    effects: vec![EffectType::ReadLocal],
                    requires_approval: false,
                }],
                &config,
            )
            .await
            .unwrap();

        let models = state.models.lock().await;
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].as_deref(), Some("gpt-4o"));
        assert_eq!(models[1].as_deref(), Some("gpt-4o"));
    }

    #[tokio::test]
    async fn config_without_model_leaves_request_model_none() {
        let state = Arc::new(CapturingProviderState::default());
        let provider: Arc<dyn LlmProvider> = Arc::new(CapturingProvider {
            state: state.clone(),
        });
        let adapter = LlmBridgeAdapter::new(provider, None);

        adapter
            .complete(
                &[ThreadMessage::user("hi")],
                &[],
                &ironclaw_engine::LlmCallConfig::default(),
            )
            .await
            .unwrap();

        let models = state.models.lock().await;
        assert_eq!(models.len(), 1);
        assert_eq!(models[0], None);
    }

    #[tokio::test]
    async fn force_text_bypasses_code_extraction() {
        let state = Arc::new(CapturingProviderState::default());
        let provider: Arc<dyn LlmProvider> = Arc::new(CapturingProvider {
            state: state.clone(),
        });
        let adapter = LlmBridgeAdapter::new(provider, None);

        let config = LlmCallConfig {
            force_text: true,
            ..Default::default()
        };

        let output = adapter
            .complete(&[ThreadMessage::user("summarize")], &[], &config)
            .await
            .unwrap();

        match output.response {
            LlmResponse::Text(t) => assert_eq!(t, "ok"),
            other => panic!("force_text must return Text, got {other:?}"),
        }
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
        assert!(code.contains("\n\n"));
    }

    #[test]
    fn mixed_thinking_and_code() {
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

    /// Regression test: the full ThreadMessage -> ChatMessage -> sanitize
    /// pipeline must preserve 1:1 correspondence between historical assistant
    /// tool_calls and Tool messages. A gap causes the LLM API to reject
    /// with "No tool output found for function call <id>".
    #[test]
    fn tool_call_result_correspondence_after_sanitize() {
        let messages: Vec<ThreadMessage> = vec![
            ThreadMessage::system("system prompt"),
            ThreadMessage::user("update all tools"),
            ThreadMessage::assistant_with_actions(
                Some(String::new()),
                vec![
                    ActionCall {
                        id: "call_AAA".into(),
                        action_name: "tool_a".into(),
                        parameters: serde_json::json!({}),
                    },
                    ActionCall {
                        id: "call_BBB".into(),
                        action_name: "tool_b".into(),
                        parameters: serde_json::json!({}),
                    },
                    ActionCall {
                        id: "call_CCC".into(),
                        action_name: "tool_c".into(),
                        parameters: serde_json::json!({}),
                    },
                ],
            ),
            ThreadMessage::action_result("call_AAA", "tool_a", "{\"ok\": true}"),
            ThreadMessage::action_result("call_BBB", "tool_b", "[no output]"),
            ThreadMessage::action_result("call_CCC", "tool_c", "{\"done\": true}"),
        ];

        let mut chat_messages: Vec<ChatMessage> = messages.iter().map(thread_msg_to_chat).collect();
        sanitize_tool_messages(&mut chat_messages);

        let mut expected_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
        for msg in &chat_messages {
            if msg.role == Role::Assistant
                && let Some(ref calls) = msg.tool_calls
            {
                for tc in calls {
                    expected_ids.insert(tc.id.clone());
                }
            }
        }

        let mut result_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
        for msg in &chat_messages {
            if msg.role == Role::Tool
                && let Some(ref id) = msg.tool_call_id
            {
                result_ids.insert(id.clone());
            }
        }

        assert_eq!(expected_ids.len(), 3, "assistant should have 3 tool calls");
        for id in &expected_ids {
            assert!(
                result_ids.contains(id),
                "tool_call {id} has no matching Tool message after sanitize — \
                 LLM API would reject with 'No tool output found'"
            );
        }
    }

    // ── Caller-level cost-tracking test ──────────────────────
    //
    // Per testing rules: "Test Through the Caller, Not Just the Helper".
    // Drives the adapter end-to-end with a provider that has known per-token
    // pricing and asserts the populated cost flows out via TokenUsage.

    /// Provider with deterministic pricing — Anthropic Sonnet rates
    /// (input $3/MTok, output $15/MTok), expressed per token.
    struct PricedProvider;

    #[async_trait]
    impl LlmProvider for PricedProvider {
        fn model_name(&self) -> &str {
            "priced-mock"
        }
        fn cost_per_token(&self) -> (Decimal, Decimal) {
            (
                rust_decimal_macros::dec!(0.000003),
                rust_decimal_macros::dec!(0.000015),
            )
        }
        async fn complete(
            &self,
            _req: crate::llm::CompletionRequest,
        ) -> Result<crate::llm::CompletionResponse, LlmError> {
            Ok(crate::llm::CompletionResponse {
                content: "hello".to_string(),
                input_tokens: 1000,
                output_tokens: 500,
                finish_reason: crate::llm::FinishReason::Stop,
                cache_read_input_tokens: 0,
                cache_creation_input_tokens: 0,
            })
        }
        async fn complete_with_tools(
            &self,
            _req: ToolCompletionRequest,
        ) -> Result<ToolCompletionResponse, LlmError> {
            unreachable!("code-only adapter must not drive complete_with_tools")
        }
    }

    /// Expected cost: 1000 * $0.000003 + 500 * $0.000015 = $0.0105
    const EXPECTED_COST_USD: f64 = 0.0105;

    #[tokio::test]
    async fn complete_populates_cost_usd_through_adapter() {
        let provider: Arc<dyn LlmProvider> = Arc::new(PricedProvider);
        let adapter = LlmBridgeAdapter::new(provider, None);

        let output = adapter
            .complete(
                &[ThreadMessage::user("hi")],
                &[],
                &LlmCallConfig::default(),
            )
            .await
            .unwrap();

        assert!(
            (output.usage.cost_usd - EXPECTED_COST_USD).abs() < 1e-9,
            "expected cost_usd ≈ {EXPECTED_COST_USD}, got {}",
            output.usage.cost_usd
        );
    }

    #[tokio::test]
    async fn complete_with_actions_listed_still_uses_plain_completion_path() {
        // Passing ActionDef entries must NOT switch the adapter onto the
        // tool-API path — the code-only contract says actions are listed
        // in the prompt, never advertised to the provider.
        let provider: Arc<dyn LlmProvider> = Arc::new(PricedProvider);
        let adapter = LlmBridgeAdapter::new(provider, None);

        let output = adapter
            .complete(
                &[ThreadMessage::user("hi")],
                &[test_action("noop")],
                &LlmCallConfig::default(),
            )
            .await
            .unwrap();

        assert!(
            (output.usage.cost_usd - EXPECTED_COST_USD).abs() < 1e-9,
            "expected cost_usd ≈ {EXPECTED_COST_USD}, got {}",
            output.usage.cost_usd
        );
    }

    #[tokio::test]
    async fn complete_routes_subcalls_through_cheap_provider_for_cost() {
        struct ZeroProvider;
        #[async_trait]
        impl LlmProvider for ZeroProvider {
            fn model_name(&self) -> &str {
                "zero-mock"
            }
            fn cost_per_token(&self) -> (Decimal, Decimal) {
                (Decimal::ZERO, Decimal::ZERO)
            }
            async fn complete(
                &self,
                _req: crate::llm::CompletionRequest,
            ) -> Result<crate::llm::CompletionResponse, LlmError> {
                Ok(crate::llm::CompletionResponse {
                    content: "ok".into(),
                    input_tokens: 1000,
                    output_tokens: 500,
                    finish_reason: crate::llm::FinishReason::Stop,
                    cache_read_input_tokens: 0,
                    cache_creation_input_tokens: 0,
                })
            }
            async fn complete_with_tools(
                &self,
                _req: ToolCompletionRequest,
            ) -> Result<ToolCompletionResponse, LlmError> {
                unreachable!()
            }
        }

        let primary: Arc<dyn LlmProvider> = Arc::new(PricedProvider);
        let cheap: Arc<dyn LlmProvider> = Arc::new(ZeroProvider);
        let adapter = LlmBridgeAdapter::new(primary, Some(cheap));

        let output = adapter
            .complete(
                &[ThreadMessage::user("hi")],
                &[],
                &LlmCallConfig {
                    depth: 1,
                    ..LlmCallConfig::default()
                },
            )
            .await
            .unwrap();

        assert_eq!(
            output.usage.cost_usd, 0.0,
            "depth>0 must use cheap provider's pricing (zero), not primary's"
        );
    }

    #[tokio::test]
    async fn complete_with_subscription_billed_provider_yields_zero_cost() {
        struct SubscriptionProvider;
        #[async_trait]
        impl LlmProvider for SubscriptionProvider {
            fn model_name(&self) -> &str {
                "subscription-mock"
            }
            fn cost_per_token(&self) -> (Decimal, Decimal) {
                (Decimal::ZERO, Decimal::ZERO)
            }
            async fn complete(
                &self,
                _req: crate::llm::CompletionRequest,
            ) -> Result<crate::llm::CompletionResponse, LlmError> {
                Ok(crate::llm::CompletionResponse {
                    content: "ok".into(),
                    input_tokens: 10_000,
                    output_tokens: 5_000,
                    finish_reason: crate::llm::FinishReason::Stop,
                    cache_read_input_tokens: 0,
                    cache_creation_input_tokens: 0,
                })
            }
            async fn complete_with_tools(
                &self,
                _req: ToolCompletionRequest,
            ) -> Result<ToolCompletionResponse, LlmError> {
                unreachable!()
            }
        }

        let provider: Arc<dyn LlmProvider> = Arc::new(SubscriptionProvider);
        let adapter = LlmBridgeAdapter::new(provider, None);

        let output = adapter
            .complete(&[ThreadMessage::user("hi")], &[], &LlmCallConfig::default())
            .await
            .unwrap();

        assert_eq!(output.usage.cost_usd, 0.0);
        assert!(output.usage.cost_usd.is_finite());
    }

    #[tokio::test]
    async fn complete_prices_cache_tokens_with_discount_and_multiplier() {
        struct AnthropicCachingProvider;
        #[async_trait]
        impl LlmProvider for AnthropicCachingProvider {
            fn model_name(&self) -> &str {
                "anthropic-caching-mock"
            }
            fn cost_per_token(&self) -> (Decimal, Decimal) {
                (
                    rust_decimal_macros::dec!(0.000003),
                    rust_decimal_macros::dec!(0.000015),
                )
            }
            fn cache_read_discount(&self) -> Decimal {
                rust_decimal_macros::dec!(10)
            }
            fn cache_write_multiplier(&self) -> Decimal {
                rust_decimal_macros::dec!(1.25)
            }
            async fn complete(
                &self,
                _req: crate::llm::CompletionRequest,
            ) -> Result<crate::llm::CompletionResponse, LlmError> {
                Ok(crate::llm::CompletionResponse {
                    content: "ok".into(),
                    input_tokens: 10_000,
                    output_tokens: 500,
                    finish_reason: crate::llm::FinishReason::Stop,
                    cache_read_input_tokens: 2_000,
                    cache_creation_input_tokens: 1_000,
                })
            }
            async fn complete_with_tools(
                &self,
                _req: ToolCompletionRequest,
            ) -> Result<ToolCompletionResponse, LlmError> {
                unreachable!()
            }
        }

        let provider: Arc<dyn LlmProvider> = Arc::new(AnthropicCachingProvider);
        let adapter = LlmBridgeAdapter::new(provider, None);

        let output = adapter
            .complete(&[ThreadMessage::user("hi")], &[], &LlmCallConfig::default())
            .await
            .unwrap();

        let expected = 0.032_85_f64;
        assert!(
            (output.usage.cost_usd - expected).abs() < 1e-9,
            "expected cost_usd ≈ {expected}, got {}",
            output.usage.cost_usd
        );

        let naive = 10_000.0 * 0.000_003 + 500.0 * 0.000_015;
        assert!(
            (output.usage.cost_usd - naive).abs() > 1e-6,
            "cost_usd {} must not match the pre-fix naive formula {}",
            output.usage.cost_usd,
            naive
        );
    }
}
