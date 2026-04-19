//! LLM bridge adapter — wraps `LlmProvider` as `ironclaw_engine::LlmBackend`.

use std::sync::Arc;

use ironclaw_engine::{
    ActionDef, EngineError, LlmBackend, LlmCallConfig, LlmOutput, LlmResponse, ThreadMessage,
    TokenUsage,
};
use rust_decimal::prelude::ToPrimitive;

use crate::llm::{
    ChatMessage, LlmProvider, Role, ToolCall, ToolCompletionRequest, ToolDefinition,
    sanitize_tool_messages,
};

/// Convert the provider's per-call cost into the f64 the engine accumulates.
/// Returns 0.0 for unknown providers (e.g. subscription-billed Codex) to match
/// their `cost_per_token() == (0, 0)` contract.
fn cost_usd_from(provider: &Arc<dyn LlmProvider>, input_tokens: u32, output_tokens: u32) -> f64 {
    provider
        .calculate_cost(input_tokens, output_tokens)
        .to_f64()
        .unwrap_or(0.0)
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
            if let Some(ref model) = config.model {
                request.model = Some(model.clone());
            }

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
                    cost_usd: cost_usd_from(
                        provider,
                        response.input_tokens,
                        response.output_tokens,
                    ),
                },
            });
        }

        // With tools: use tool completion (matches existing tools path)
        let mut request = ToolCompletionRequest::new(chat_messages, tools)
            .with_max_tokens(max_tokens)
            .with_temperature(temperature)
            .with_tool_choice("auto");
        request.metadata = config.metadata.clone();
        if let Some(ref model) = config.model {
            request.model = Some(model.clone());
        }

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
            let mut calls: Vec<ironclaw_engine::ActionCall> = response
                .tool_calls
                .iter()
                .map(|tc| ironclaw_engine::ActionCall {
                    id: tc.id.clone(),
                    action_name: tc.name.clone(),
                    parameters: tc.arguments.clone(),
                })
                .collect();

            // Resolve `{{call_id.field}}` template references in tool call
            // parameters. Some models (e.g. Qwen) emit these when making
            // parallel tool calls that reference results from prior calls.
            if calls.iter().any(|c| json_has_template_refs(&c.parameters)) {
                let tool_results = build_tool_result_index(messages);
                if !tool_results.is_empty() {
                    for call in &mut calls {
                        if json_has_template_refs(&call.parameters) {
                            call.parameters =
                                resolve_template_refs_in_json(&call.parameters, &tool_results);
                        }
                    }
                }
            }

            LlmResponse::ActionCalls {
                calls,
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
                cost_usd: cost_usd_from(provider, response.input_tokens, response.output_tokens),
            },
        })
    }

    fn model_name(&self) -> &str {
        self.provider.model_name()
    }
}

// ── Tool-call template reference resolution ────────────────
//
// Some OpenAI-format models (e.g. Qwen) emit template references like
// `{{chatcmpl-tool-<id>.<field>}}` in parallel tool call arguments,
// expecting the runtime to resolve them from prior tool results. We
// resolve these by looking up the referenced call_id in the conversation
// history and extracting the requested JSON field from the result.

/// Regex-free lightweight scan for `{{<call_id>.<field>}}` patterns.
/// Resolves references iteratively. If an unresolvable reference is
/// encountered, resolution stops and earlier successful substitutions
/// are preserved (partial resolution). Returns the original string
/// unchanged if no `{{` markers are found.
fn resolve_template_refs(value: &str, tool_results: &[(String, serde_json::Value)]) -> String {
    if !value.contains("{{") {
        return value.to_string();
    }

    let mut result = value.to_string();
    let mut search_from = 0;
    // Iteratively resolve all `{{..}}` patterns (limit iterations to prevent infinite loops)
    for _ in 0..50 {
        let Some(rel_start) = result[search_from..].find("{{") else {
            break;
        };
        let start = search_from + rel_start;
        let Some(rel_end) = result[start..].find("}}") else {
            break;
        };
        let end = start + rel_end;
        let ref_str = &result[start + 2..end]; // e.g. "chatcmpl-tool-9816a462feb22da1.project_id"

        let resolved = if let Some(dot_pos) = ref_str.rfind('.') {
            let call_id = &ref_str[..dot_pos];
            let field = &ref_str[dot_pos + 1..];
            tool_results
                .iter()
                .find(|(id, _)| id == call_id)
                .and_then(|(_, json)| json.get(field))
                .map(|v| match v {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                })
        } else {
            None
        };

        match resolved {
            Some(val) => {
                let val_len = val.len();
                result.replace_range(start..end + 2, &val);
                // Advance past the replacement to prevent second-order injection:
                // resolved values containing `{{...}}` must not be re-scanned.
                search_from = start + val_len;
            }
            None => {
                // Can't resolve — skip past this `{{` to avoid infinite loop on the same pattern
                search_from = start + 2;
            }
        }
    }
    result
}

/// Walk a JSON value and resolve any `{{call_id.field}}` template references
/// found in string values.
fn resolve_template_refs_in_json(
    value: &serde_json::Value,
    tool_results: &[(String, serde_json::Value)],
) -> serde_json::Value {
    match value {
        serde_json::Value::String(s) => {
            let resolved = resolve_template_refs(s, tool_results);
            serde_json::Value::String(resolved)
        }
        serde_json::Value::Object(map) => {
            let resolved: serde_json::Map<String, serde_json::Value> = map
                .iter()
                .map(|(k, v)| (k.clone(), resolve_template_refs_in_json(v, tool_results)))
                .collect();
            serde_json::Value::Object(resolved)
        }
        serde_json::Value::Array(arr) => {
            let resolved: Vec<serde_json::Value> = arr
                .iter()
                .map(|v| resolve_template_refs_in_json(v, tool_results))
                .collect();
            serde_json::Value::Array(resolved)
        }
        other => other.clone(),
    }
}

/// Build a lookup table of (call_id -> parsed JSON) from tool result messages
/// in the conversation.
fn build_tool_result_index(messages: &[ThreadMessage]) -> Vec<(String, serde_json::Value)> {
    messages
        .iter()
        .filter(|m| m.role == ironclaw_engine::MessageRole::ActionResult)
        .filter_map(|m| {
            let call_id = m.action_call_id.as_deref()?;
            // Try to parse the content as JSON; fall back to wrapping as a string
            let json = serde_json::from_str(&m.content)
                .unwrap_or_else(|_| serde_json::Value::String(m.content.clone()));
            Some((call_id.to_string(), json))
        })
        .collect()
}

/// Returns true if any string value in the JSON contains `{{` template refs.
fn json_has_template_refs(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::String(s) => s.contains("{{"),
        serde_json::Value::Object(map) => map.values().any(json_has_template_refs),
        serde_json::Value::Array(arr) => arr.iter().any(json_has_template_refs),
        _ => false,
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

        async fn complete_with_tools(
            &self,
            req: ToolCompletionRequest,
        ) -> Result<ToolCompletionResponse, LlmError> {
            self.state.models.lock().await.push(req.model.clone());
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

        // Plain completion path (no tools)
        adapter
            .complete(&[ThreadMessage::user("hi")], &[], &config)
            .await
            .unwrap();

        // Tool completion path
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

    /// Regression test: the full ThreadMessage -> ChatMessage -> sanitize
    /// pipeline must preserve 1:1 correspondence between assistant
    /// tool_calls and Tool messages. A gap causes the LLM API to reject
    /// with "No tool output found for function call <id>".
    #[test]
    fn tool_call_result_correspondence_after_sanitize() {
        // Simulate messages that include a "[no output]" placeholder
        // (the fix for null tool output).
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
            // call_BBB had null output; Python now sends "[no output]"
            ThreadMessage::action_result("call_BBB", "tool_b", "[no output]"),
            ThreadMessage::action_result("call_CCC", "tool_c", "{\"done\": true}"),
        ];

        let mut chat_messages: Vec<ChatMessage> = messages.iter().map(thread_msg_to_chat).collect();
        sanitize_tool_messages(&mut chat_messages);

        // Collect tool_call IDs from assistant messages
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

        // Collect tool_call_ids from Tool messages
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

    // ── Template reference resolution tests ────────────────────

    #[test]
    fn resolve_template_refs_simple_field() {
        let tool_results = vec![(
            "chatcmpl-tool-abc123".to_string(),
            serde_json::json!({"project_id": "068f67da-49b6", "name": "My Project"}),
        )];

        let input = "{{chatcmpl-tool-abc123.project_id}}";
        assert_eq!(resolve_template_refs(input, &tool_results), "068f67da-49b6");
    }

    #[test]
    fn resolve_template_refs_embedded_in_string() {
        let tool_results = vec![("call-1".to_string(), serde_json::json!({"id": "proj-42"}))];

        let input = "Project ID is {{call-1.id}} here";
        assert_eq!(
            resolve_template_refs(input, &tool_results),
            "Project ID is proj-42 here"
        );
    }

    #[test]
    fn resolve_template_refs_no_match_unchanged() {
        let tool_results = vec![("call-1".to_string(), serde_json::json!({"id": "proj-42"}))];

        let input = "{{call-unknown.id}}";
        // Can't resolve — returns unchanged
        assert_eq!(resolve_template_refs(input, &tool_results), input);
    }

    #[test]
    fn resolve_template_refs_no_templates_passthrough() {
        let input = "plain string with no templates";
        assert_eq!(resolve_template_refs(input, &[]), input);
    }

    #[test]
    fn resolve_template_refs_numeric_value() {
        let tool_results = vec![("call-1".to_string(), serde_json::json!({"count": 42}))];

        let input = "{{call-1.count}}";
        assert_eq!(resolve_template_refs(input, &tool_results), "42");
    }

    #[test]
    fn resolve_template_refs_in_json_deep() {
        let tool_results = vec![(
            "chatcmpl-tool-9816".to_string(),
            serde_json::json!({"project_id": "068f67da"}),
        )];

        let input = serde_json::json!({
            "name": "Daily Monitoring",
            "project_id": "{{chatcmpl-tool-9816.project_id}}",
            "nested": {
                "ref": "{{chatcmpl-tool-9816.project_id}}"
            },
            "list": ["{{chatcmpl-tool-9816.project_id}}", "static"],
            "number": 42
        });

        let resolved = resolve_template_refs_in_json(&input, &tool_results);
        assert_eq!(resolved["project_id"], "068f67da");
        assert_eq!(resolved["nested"]["ref"], "068f67da");
        assert_eq!(resolved["list"][0], "068f67da");
        assert_eq!(resolved["list"][1], "static");
        assert_eq!(resolved["number"], 42);
        assert_eq!(resolved["name"], "Daily Monitoring");
    }

    #[test]
    fn resolve_template_refs_no_second_order_injection() {
        // If a resolved value itself contains {{...}}, it must NOT be resolved.
        // This prevents second-order template injection from tool output.
        let tool_results = vec![
            (
                "call-1".to_string(),
                serde_json::json!({"payload": "{{call-2.secret}}"}),
            ),
            (
                "call-2".to_string(),
                serde_json::json!({"secret": "LEAKED"}),
            ),
        ];

        let input = "result: {{call-1.payload}}";
        let resolved = resolve_template_refs(input, &tool_results);
        // The resolved value contains {{call-2.secret}} literally — it must NOT be resolved further.
        assert_eq!(resolved, "result: {{call-2.secret}}");
    }

    #[test]
    fn resolve_template_refs_skips_unresolvable_continues_later() {
        // An unresolvable ref should not prevent resolving later valid refs.
        let tool_results = vec![("call-1".to_string(), serde_json::json!({"id": "42"}))];

        let input = "{{unknown.field}} then {{call-1.id}}";
        let resolved = resolve_template_refs(input, &tool_results);
        assert_eq!(resolved, "{{unknown.field}} then 42");
    }

    #[test]
    fn build_tool_result_index_from_messages() {
        let messages = vec![
            ThreadMessage::user("hello"),
            ThreadMessage::action_result(
                "call-1",
                "memory_write",
                r#"{"project_id": "068f67da", "name": "Test"}"#,
            ),
            ThreadMessage::assistant("done"),
            ThreadMessage::action_result("call-2", "memory_write", "plain text result"),
        ];

        let index = build_tool_result_index(&messages);
        assert_eq!(index.len(), 2);
        assert_eq!(index[0].0, "call-1");
        assert_eq!(index[0].1["project_id"], "068f67da");
        assert_eq!(index[1].0, "call-2");
        // Non-JSON content wrapped as string
        assert_eq!(
            index[1].1,
            serde_json::Value::String("plain text result".to_string())
        );
    }

    #[test]
    fn json_has_template_refs_detection() {
        assert!(json_has_template_refs(&serde_json::json!("{{call.field}}")));
        assert!(json_has_template_refs(&serde_json::json!({"a": "{{x.y}}"})));
        assert!(json_has_template_refs(&serde_json::json!(["{{x.y}}"])));
        assert!(!json_has_template_refs(&serde_json::json!("no refs")));
        assert!(!json_has_template_refs(&serde_json::json!(42)));
        assert!(!json_has_template_refs(&serde_json::json!({"a": "b"})));
    }

    // ── Caller-level template ref resolution test ────────────
    //
    // Per testing rules: "Test Through the Caller, Not Just the Helper".
    // This test drives LlmBridgeAdapter::complete() with a conversation
    // that contains tool results and an LLM response referencing them
    // via {{call_id.field}} patterns. Verifies the resolution happens
    // at the adapter level, not just in the helper functions.

    /// Mock LLM provider that returns tool calls with template refs in
    /// their parameters, simulating Qwen-style parallel call behavior.
    struct TemplateRefProvider;

    #[async_trait]
    impl LlmProvider for TemplateRefProvider {
        fn model_name(&self) -> &str {
            "template-ref-mock"
        }
        fn cost_per_token(&self) -> (Decimal, Decimal) {
            (Decimal::ZERO, Decimal::ZERO)
        }
        async fn complete(
            &self,
            _req: crate::llm::CompletionRequest,
        ) -> Result<crate::llm::CompletionResponse, LlmError> {
            unreachable!("should use complete_with_tools")
        }
        async fn complete_with_tools(
            &self,
            _req: ToolCompletionRequest,
        ) -> Result<ToolCompletionResponse, LlmError> {
            // Simulate: LLM returns a mission_create call that references
            // a prior tool result's project_id via template ref.
            Ok(ToolCompletionResponse {
                content: Some("Creating mission in the new project".to_string()),
                tool_calls: vec![crate::llm::ToolCall {
                    id: "call-2".to_string(),
                    name: "mission_create".to_string(),
                    arguments: serde_json::json!({
                        "name": "Daily Monitor",
                        "goal": "Monitor things",
                        "project_id": "{{call-1.project_id}}"
                    }),
                    reasoning: None,
                }],
                input_tokens: 10,
                output_tokens: 10,
                finish_reason: crate::llm::FinishReason::ToolUse,
                cache_read_input_tokens: 0,
                cache_creation_input_tokens: 0,
            })
        }
    }

    #[tokio::test]
    async fn complete_resolves_template_refs_through_adapter() {
        let provider: Arc<dyn LlmProvider> = Arc::new(TemplateRefProvider);
        let adapter = LlmBridgeAdapter::new(provider, None);

        // Conversation history: user asked to create a project, tool returned
        // a result with project_id, now the LLM wants to create a mission
        // referencing that project_id.
        let messages = vec![
            ThreadMessage::user("Create a project and a daily mission"),
            ThreadMessage::assistant_with_actions(
                Some("I'll create the project first".to_string()),
                vec![ActionCall {
                    id: "call-1".into(),
                    action_name: "memory_write".into(),
                    parameters: serde_json::json!({"target": "projects/test/AGENTS.md"}),
                }],
            ),
            ThreadMessage::action_result(
                "call-1",
                "memory_write",
                r#"{"project_id": "068f67da-49b6-4f6c-9463-8d243c2cff6c", "status": "ok"}"#,
            ),
        ];

        let output = adapter
            .complete(
                &messages,
                &[test_action("mission_create")],
                &LlmCallConfig::default(),
            )
            .await
            .unwrap();

        // The adapter should have resolved {{call-1.project_id}} to the UUID.
        match output.response {
            LlmResponse::ActionCalls { calls, .. } => {
                assert_eq!(calls.len(), 1);
                let project_id = calls[0].parameters["project_id"].as_str().unwrap();
                assert_eq!(
                    project_id, "068f67da-49b6-4f6c-9463-8d243c2cff6c",
                    "Template ref should be resolved to actual UUID"
                );
                assert_eq!(calls[0].parameters["name"], "Daily Monitor");
            }
            other => panic!("Expected ActionCalls, got: {other:?}"),
        }
    }

    // ── Caller-level cost-tracking test ──────────────────────
    //
    // Per testing rules: "Test Through the Caller, Not Just the Helper".
    // model_cost() returning the right Decimal is necessary but not
    // sufficient — the gap that motivated this test was that
    // LlmBridgeAdapter hardcoded `cost_usd: 0.0` and never consulted the
    // provider's calculate_cost(), so Thread::total_cost_usd never
    // accumulated and `max_budget_usd` gates were inert. This test drives
    // the adapter end-to-end with a provider that has known per-token
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
            Ok(ToolCompletionResponse {
                content: Some("hello".to_string()),
                tool_calls: Vec::new(),
                input_tokens: 1000,
                output_tokens: 500,
                finish_reason: crate::llm::FinishReason::Stop,
                cache_read_input_tokens: 0,
                cache_creation_input_tokens: 0,
            })
        }
    }

    /// Expected cost: 1000 * $0.000003 + 500 * $0.000015 = $0.0105
    const EXPECTED_COST_USD: f64 = 0.0105;

    #[tokio::test]
    async fn complete_no_tools_populates_cost_usd_through_adapter() {
        let provider: Arc<dyn LlmProvider> = Arc::new(PricedProvider);
        let adapter = LlmBridgeAdapter::new(provider, None);

        let output = adapter
            .complete(
                &[ThreadMessage::user("hi")],
                &[], // no actions => no-tools path
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
    async fn complete_with_tools_populates_cost_usd_through_adapter() {
        let provider: Arc<dyn LlmProvider> = Arc::new(PricedProvider);
        let adapter = LlmBridgeAdapter::new(provider, None);

        let output = adapter
            .complete(
                &[ThreadMessage::user("hi")],
                &[test_action("noop")], // forces with-tools path
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
        // Sub-calls (depth > 0) must be priced with the cheap provider, not
        // the primary. Otherwise nested CodeAct calls inflate the parent
        // thread's cost by the wrong rate and `max_budget_usd` gates fire
        // against the wrong total.
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
}
