//! LLM reasoning capabilities for planning, tool selection, and evaluation.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::error::LlmError;

use crate::llm::{
    ChatMessage, CompletionRequest, LlmProvider, ToolCall, ToolCompletionRequest, ToolDefinition,
};
use crate::safety::SafetyLayer;

/// Context for reasoning operations.
pub struct ReasoningContext {
    /// Conversation history.
    pub messages: Vec<ChatMessage>,
    /// Available tools.
    pub available_tools: Vec<ToolDefinition>,
    /// Job description if working on a job.
    pub job_description: Option<String>,
    /// Current state description.
    pub current_state: Option<String>,
    /// Opaque metadata forwarded to the LLM provider (e.g. thread_id for chaining).
    pub metadata: std::collections::HashMap<String, String>,
}

impl ReasoningContext {
    /// Create a new reasoning context.
    pub fn new() -> Self {
        Self {
            messages: Vec::new(),
            available_tools: Vec::new(),
            job_description: None,
            current_state: None,
            metadata: std::collections::HashMap::new(),
        }
    }

    /// Add a message to the context.
    pub fn with_message(mut self, message: ChatMessage) -> Self {
        self.messages.push(message);
        self
    }

    /// Set messages directly (for session-based context).
    pub fn with_messages(mut self, messages: Vec<ChatMessage>) -> Self {
        self.messages = messages;
        self
    }

    /// Set available tools.
    pub fn with_tools(mut self, tools: Vec<ToolDefinition>) -> Self {
        self.available_tools = tools;
        self
    }

    /// Set job description.
    pub fn with_job(mut self, description: impl Into<String>) -> Self {
        self.job_description = Some(description.into());
        self
    }

    /// Set metadata (forwarded to the LLM provider).
    pub fn with_metadata(mut self, metadata: std::collections::HashMap<String, String>) -> Self {
        self.metadata = metadata;
        self
    }
}

impl Default for ReasoningContext {
    fn default() -> Self {
        Self::new()
    }
}

/// A planned action to take.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannedAction {
    /// Tool to use.
    pub tool_name: String,
    /// Parameters for the tool.
    pub parameters: serde_json::Value,
    /// Reasoning for this action.
    pub reasoning: String,
    /// Expected outcome.
    pub expected_outcome: String,
}

/// Result of planning.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionPlan {
    /// Overall goal understanding.
    pub goal: String,
    /// Planned sequence of actions.
    pub actions: Vec<PlannedAction>,
    /// Estimated total cost.
    pub estimated_cost: Option<f64>,
    /// Estimated total time in seconds.
    pub estimated_time_secs: Option<u64>,
    /// Confidence in the plan (0-1).
    pub confidence: f64,
}

/// Result of tool selection.
#[derive(Debug, Clone)]
pub struct ToolSelection {
    /// Selected tool name.
    pub tool_name: String,
    /// Parameters for the tool.
    pub parameters: serde_json::Value,
    /// Reasoning for the selection.
    pub reasoning: String,
    /// Alternative tools considered.
    pub alternatives: Vec<String>,
    /// The tool call ID from the LLM response.
    ///
    /// OpenAI-compatible providers assign each tool call a unique ID that must
    /// be echoed back in the corresponding tool result message. Without this,
    /// the provider cannot match results to their originating calls.
    pub tool_call_id: String,
}

/// Token usage from a single LLM call.
#[derive(Debug, Clone, Copy, Default)]
pub struct TokenUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

impl TokenUsage {
    pub fn total(&self) -> u32 {
        self.input_tokens + self.output_tokens
    }
}

/// Result of a response with potential tool calls.
///
/// Used by the agent loop to handle tool execution before returning a final response.
#[derive(Debug, Clone)]
pub enum RespondResult {
    /// A text response (no tools needed).
    Text(String),
    /// The model wants to call tools. Caller should execute them and call back.
    /// Includes the optional content from the assistant message (some models
    /// include explanatory text alongside tool calls).
    ToolCalls {
        tool_calls: Vec<ToolCall>,
        content: Option<String>,
    },
}

/// A `RespondResult` bundled with the token usage from the LLM call that produced it.
#[derive(Debug, Clone)]
pub struct RespondOutput {
    pub result: RespondResult,
    pub usage: TokenUsage,
}

/// Reasoning engine for the agent.
pub struct Reasoning {
    llm: Arc<dyn LlmProvider>,
    #[allow(dead_code)] // Will be used for sanitizing tool outputs
    safety: Arc<SafetyLayer>,
    /// Optional workspace for loading identity/system prompts.
    workspace_system_prompt: Option<String>,
}

impl Reasoning {
    /// Create a new reasoning engine.
    pub fn new(llm: Arc<dyn LlmProvider>, safety: Arc<SafetyLayer>) -> Self {
        Self {
            llm,
            safety,
            workspace_system_prompt: None,
        }
    }

    /// Set a custom system prompt from workspace identity files.
    ///
    /// This is typically loaded from workspace.system_prompt() which combines
    /// AGENTS.md, SOUL.md, USER.md, and IDENTITY.md into a unified prompt.
    pub fn with_system_prompt(mut self, prompt: String) -> Self {
        if !prompt.is_empty() {
            self.workspace_system_prompt = Some(prompt);
        }
        self
    }

    /// Generate a plan for completing a goal.
    pub async fn plan(&self, context: &ReasoningContext) -> Result<ActionPlan, LlmError> {
        let system_prompt = self.build_planning_prompt(context);

        let mut messages = vec![ChatMessage::system(system_prompt)];
        messages.extend(context.messages.clone());

        if let Some(ref job) = context.job_description {
            messages.push(ChatMessage::user(format!(
                "Please create a plan to complete this job:\n\n{}",
                job
            )));
        }

        let request = CompletionRequest::new(messages)
            .with_max_tokens(2048)
            .with_temperature(0.3);

        let response = self.llm.complete(request).await?;

        // Parse the plan from the response
        self.parse_plan(&response.content)
    }

    /// Select the best tool for the current situation.
    pub async fn select_tool(
        &self,
        context: &ReasoningContext,
    ) -> Result<Option<ToolSelection>, LlmError> {
        let tools = self.select_tools(context).await?;
        Ok(tools.into_iter().next())
    }

    /// Select tools to execute (may return multiple for parallel execution).
    ///
    /// The LLM may return multiple tool calls if it determines they can be
    /// executed in parallel. This enables more efficient job completion.
    pub async fn select_tools(
        &self,
        context: &ReasoningContext,
    ) -> Result<Vec<ToolSelection>, LlmError> {
        if context.available_tools.is_empty() {
            return Ok(vec![]);
        }

        let mut request =
            ToolCompletionRequest::new(context.messages.clone(), context.available_tools.clone())
                .with_max_tokens(1024)
                .with_tool_choice("auto");
        request.metadata = context.metadata.clone();

        let response = self.llm.complete_with_tools(request).await?;

        let reasoning = response.content.unwrap_or_default();

        let selections: Vec<ToolSelection> = response
            .tool_calls
            .into_iter()
            .map(|tool_call| ToolSelection {
                tool_name: tool_call.name,
                parameters: tool_call.arguments,
                reasoning: reasoning.clone(),
                alternatives: vec![],
                tool_call_id: tool_call.id,
            })
            .collect();

        Ok(selections)
    }

    /// Evaluate whether a task was completed successfully.
    pub async fn evaluate_success(
        &self,
        context: &ReasoningContext,
        result: &str,
    ) -> Result<SuccessEvaluation, LlmError> {
        let system_prompt = r#"You are an evaluation assistant. Your job is to determine if a task was completed successfully.

Analyze the task description and the result, then provide:
1. Whether the task was successful (true/false)
2. A confidence score (0-1)
3. Detailed reasoning
4. Any issues found
5. Suggestions for improvement

Respond in JSON format:
{
    "success": true/false,
    "confidence": 0.0-1.0,
    "reasoning": "...",
    "issues": ["..."],
    "suggestions": ["..."]
}"#;

        let mut messages = vec![ChatMessage::system(system_prompt)];

        if let Some(ref job) = context.job_description {
            messages.push(ChatMessage::user(format!(
                "Task description:\n{}\n\nResult:\n{}",
                job, result
            )));
        } else {
            messages.push(ChatMessage::user(format!(
                "Result to evaluate:\n{}",
                result
            )));
        }

        let request = CompletionRequest::new(messages)
            .with_max_tokens(1024)
            .with_temperature(0.1);

        let response = self.llm.complete(request).await?;

        self.parse_evaluation(&response.content)
    }

    /// Generate a response to a user message.
    ///
    /// If tools are available in the context, uses tool completion mode.
    /// This is a convenience wrapper around `respond_with_tools()` that formats
    /// tool calls as text for simple cases. Use `respond_with_tools()` when you
    /// need to actually execute tool calls in an agentic loop.
    pub async fn respond(&self, context: &ReasoningContext) -> Result<String, LlmError> {
        let output = self.respond_with_tools(context).await?;
        match output.result {
            RespondResult::Text(text) => Ok(text),
            RespondResult::ToolCalls {
                tool_calls: calls, ..
            } => {
                // Format tool calls as text (legacy behavior for non-agentic callers)
                let tool_info: Vec<String> = calls
                    .iter()
                    .map(|tc| format!("`{}({})`", tc.name, tc.arguments))
                    .collect();
                Ok(format!("[Calling tools: {}]", tool_info.join(", ")))
            }
        }
    }

    /// Generate a response that may include tool calls, with token usage tracking.
    ///
    /// Returns `RespondOutput` containing the result and token usage from the LLM call.
    /// The caller should use `usage` to track cost/budget against the job.
    pub async fn respond_with_tools(
        &self,
        context: &ReasoningContext,
    ) -> Result<RespondOutput, LlmError> {
        let system_prompt = self.build_conversation_prompt(context);

        let mut messages = vec![ChatMessage::system(system_prompt)];
        messages.extend(context.messages.clone());

        // If we have tools, use tool completion mode
        if !context.available_tools.is_empty() {
            let mut request = ToolCompletionRequest::new(messages, context.available_tools.clone())
                .with_max_tokens(4096)
                .with_temperature(0.7)
                .with_tool_choice("auto");
            request.metadata = context.metadata.clone();

            let response = self.llm.complete_with_tools(request).await?;
            let usage = TokenUsage {
                input_tokens: response.input_tokens,
                output_tokens: response.output_tokens,
            };

            // If there were tool calls, return them for execution
            if !response.tool_calls.is_empty() {
                return Ok(RespondOutput {
                    result: RespondResult::ToolCalls {
                        tool_calls: response.tool_calls,
                        content: response.content,
                    },
                    usage,
                });
            }

            let content = response
                .content
                .unwrap_or_else(|| "I'm not sure how to respond to that.".to_string());

            // Some models (e.g. GLM-4.7) emit tool calls as XML tags in content
            // instead of using the structured tool_calls field. Try to recover
            // them before giving up and returning plain text.
            let recovered = recover_tool_calls_from_content(&content, &context.available_tools);
            if !recovered.is_empty() {
                let cleaned = clean_response(&content);
                return Ok(RespondOutput {
                    result: RespondResult::ToolCalls {
                        tool_calls: recovered,
                        content: if cleaned.is_empty() {
                            None
                        } else {
                            Some(cleaned)
                        },
                    },
                    usage,
                });
            }

            Ok(RespondOutput {
                result: RespondResult::Text(clean_response(&content)),
                usage,
            })
        } else {
            // No tools, use simple completion
            let mut request = CompletionRequest::new(messages)
                .with_max_tokens(4096)
                .with_temperature(0.7);
            request.metadata = context.metadata.clone();

            let response = self.llm.complete(request).await?;
            Ok(RespondOutput {
                result: RespondResult::Text(clean_response(&response.content)),
                usage: TokenUsage {
                    input_tokens: response.input_tokens,
                    output_tokens: response.output_tokens,
                },
            })
        }
    }

    fn build_planning_prompt(&self, context: &ReasoningContext) -> String {
        let tools_desc = if context.available_tools.is_empty() {
            "No tools available.".to_string()
        } else {
            context
                .available_tools
                .iter()
                .map(|t| format!("- {}: {}", t.name, t.description))
                .collect::<Vec<_>>()
                .join("\n")
        };

        format!(
            r#"You are a planning assistant for an autonomous agent. Your job is to create detailed, actionable plans.

Available tools:
{tools_desc}

When creating a plan:
1. Break down the goal into specific, achievable steps
2. Select the most appropriate tool for each step
3. Consider dependencies between steps
4. Estimate costs and time realistically
5. Identify potential failure points

Respond with a JSON plan in this format:
{{
    "goal": "Clear statement of the goal",
    "actions": [
        {{
            "tool_name": "tool_to_use",
            "parameters": {{}},
            "reasoning": "Why this action",
            "expected_outcome": "What should happen"
        }}
    ],
    "estimated_cost": 0.0,
    "estimated_time_secs": 0,
    "confidence": 0.0-1.0
}}"#
        )
    }

    fn build_conversation_prompt(&self, context: &ReasoningContext) -> String {
        let tools_section = if context.available_tools.is_empty() {
            String::new()
        } else {
            let tool_list: Vec<String> = context
                .available_tools
                .iter()
                .map(|t| format!("  - {}: {}", t.name, t.description))
                .collect();
            format!(
                "\n\n## Available Tools\nYou have access to these tools:\n{}\n\nCall tools when they would help accomplish the task.",
                tool_list.join("\n")
            )
        };

        // Include workspace identity prompt if available
        let identity_section = if let Some(ref identity) = self.workspace_system_prompt {
            format!("\n\n---\n\n{}", identity)
        } else {
            String::new()
        };

        format!(
            r#"You are NEAR AI Agent, an autonomous assistant.

## Response Format

If you need to think through a problem, wrap your thinking in <thinking> tags. Everything outside these tags goes directly to the user.

Example:
<thinking>
Let me consider the options...
Option 1: ...
Option 2: ...
I'll go with option 1.
</thinking>
Here's the solution: [actual response to user]

## Guidelines
- Be concise and direct
- Use markdown formatting where helpful
- For code, use appropriate code blocks with language tags
- Call tools when they would help accomplish the task{}

The user sees ONLY content outside <thinking> tags.{}"#,
            tools_section, identity_section
        )
    }

    fn parse_plan(&self, content: &str) -> Result<ActionPlan, LlmError> {
        // Try to extract JSON from the response
        let json_str = extract_json(content).unwrap_or(content);

        serde_json::from_str(json_str).map_err(|e| LlmError::InvalidResponse {
            provider: self.llm.model_name().to_string(),
            reason: format!("Failed to parse plan: {}", e),
        })
    }

    fn parse_evaluation(&self, content: &str) -> Result<SuccessEvaluation, LlmError> {
        let json_str = extract_json(content).unwrap_or(content);

        serde_json::from_str(json_str).map_err(|e| LlmError::InvalidResponse {
            provider: self.llm.model_name().to_string(),
            reason: format!("Failed to parse evaluation: {}", e),
        })
    }
}

/// Result of success evaluation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuccessEvaluation {
    pub success: bool,
    pub confidence: f64,
    pub reasoning: String,
    #[serde(default)]
    pub issues: Vec<String>,
    #[serde(default)]
    pub suggestions: Vec<String>,
}

/// Extract JSON from text that might contain other content.
fn extract_json(text: &str) -> Option<&str> {
    // Find the first { and last } to extract JSON
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    if start < end {
        Some(&text[start..=end])
    } else {
        None
    }
}

/// Clean up LLM response by stripping model-internal tags and reasoning patterns.
///
/// Some models (GLM-4.7, etc.) emit XML-tagged internal state like
/// Try to extract tool calls from content text where the model emitted them
/// as XML tags instead of using the structured tool_calls field.
///
/// Handles these formats:
/// - `<tool_call>tool_name</tool_call>` (bare name)
/// - `<tool_call>{"name":"x","arguments":{}}</tool_call>` (JSON)
/// - `<|tool_call|>...<|/tool_call|>` (pipe-delimited variant)
/// - `<function_call>...</function_call>` (function_call variant)
///
/// Only returns calls whose name matches an available tool.
fn recover_tool_calls_from_content(
    content: &str,
    available_tools: &[ToolDefinition],
) -> Vec<ToolCall> {
    let tool_names: std::collections::HashSet<&str> =
        available_tools.iter().map(|t| t.name.as_str()).collect();
    let mut calls = Vec::new();

    for (open, close) in &[
        ("<tool_call>", "</tool_call>"),
        ("<|tool_call|>", "<|/tool_call|>"),
        ("<function_call>", "</function_call>"),
        ("<|function_call|>", "<|/function_call|>"),
    ] {
        let mut remaining = content;
        while let Some(start) = remaining.find(open) {
            let inner_start = start + open.len();
            let after = &remaining[inner_start..];
            let Some(end) = after.find(close) else {
                break;
            };
            let inner = after[..end].trim();
            remaining = &after[end + close.len()..];

            if inner.is_empty() {
                continue;
            }

            // Try JSON first: {"name":"x","arguments":{}}
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(inner)
                && let Some(name) = parsed.get("name").and_then(|v| v.as_str())
                && tool_names.contains(name)
            {
                let arguments = parsed
                    .get("arguments")
                    .cloned()
                    .unwrap_or(serde_json::Value::Object(Default::default()));
                calls.push(ToolCall {
                    id: format!("recovered_{}", calls.len()),
                    name: name.to_string(),
                    arguments,
                });
                continue;
            }

            // Bare tool name (e.g. "<tool_call>tool_list</tool_call>")
            let name = inner.trim();
            if tool_names.contains(name) {
                calls.push(ToolCall {
                    id: format!("recovered_{}", calls.len()),
                    name: name.to_string(),
                    arguments: serde_json::Value::Object(Default::default()),
                });
            }
        }
    }

    calls
}

/// `<tool_call>tool_list</tool_call>` or `<|tool_call|>` in the content field
/// instead of using the standard OpenAI tool_calls array. We strip all of
/// these before the response reaches channels/users.
fn clean_response(text: &str) -> String {
    let text = strip_internal_tags(text);
    strip_reasoning_patterns(&text)
}

/// Tags that are model-internal and should never reach users.
const INTERNAL_TAGS: &[&str] = &["thinking", "tool_call", "function_call", "tool_calls"];

/// Strip all model-internal XML tags from LLM output.
///
/// Handles standard XML tags (`<tag>...</tag>`) and pipe-delimited variants
/// (`<|tag|>...<|/tag|>`) used by some models (e.g. GLM-4.7).
fn strip_internal_tags(text: &str) -> String {
    let mut result = text.to_string();
    for tag in INTERNAL_TAGS {
        result = strip_xml_tag(&result, tag);
        result = strip_pipe_tag(&result, tag);
    }
    // Collapse triple+ newlines left behind by removed blocks
    while result.contains("\n\n\n") {
        result = result.replace("\n\n\n", "\n\n");
    }
    result.trim().to_string()
}

/// Strip `<tag>...</tag>` and `<tag ...>...</tag>` blocks from text.
fn strip_xml_tag(text: &str, tag: &str) -> String {
    let open_exact = format!("<{}>", tag);
    let open_prefix = format!("<{} ", tag); // for <tag attr="...">
    let close = format!("</{}>", tag);

    let mut result = String::with_capacity(text.len());
    let mut remaining = text;

    loop {
        // Find the next opening tag (exact or with attributes)
        let exact_pos = remaining.find(&open_exact);
        let prefix_pos = remaining.find(&open_prefix);
        let start = match (exact_pos, prefix_pos) {
            (Some(a), Some(b)) => a.min(b),
            (Some(a), None) => a,
            (None, Some(b)) => b,
            (None, None) => break,
        };

        // Add everything before the tag
        result.push_str(&remaining[..start]);

        // Find the end of the opening tag (the closing >)
        let after_open = &remaining[start..];
        let open_end = match after_open.find('>') {
            Some(pos) => start + pos + 1,
            None => break, // malformed, stop
        };

        // Find the closing tag
        if let Some(close_offset) = remaining[open_end..].find(&close) {
            let end = open_end + close_offset + close.len();
            remaining = &remaining[end..];
        } else {
            // No closing tag, discard from here (malformed)
            remaining = "";
            break;
        }
    }

    result.push_str(remaining);
    result
}

/// Strip `<|tag|>...<|/tag|>` pipe-delimited blocks from text.
///
/// Some models (e.g. certain Chinese LLMs) use this format instead of
/// standard XML tags.
fn strip_pipe_tag(text: &str, tag: &str) -> String {
    let open = format!("<|{}|>", tag);
    let close = format!("<|/{}|>", tag);

    let mut result = String::with_capacity(text.len());
    let mut remaining = text;

    while let Some(start) = remaining.find(&open) {
        result.push_str(&remaining[..start]);

        if let Some(close_offset) = remaining[start..].find(&close) {
            let end = start + close_offset + close.len();
            remaining = &remaining[end..];
        } else {
            remaining = "";
            break;
        }
    }

    result.push_str(remaining);
    result
}

/// Strip any remaining reasoning that wasn't in proper <thinking> tags.
///
/// This is a simple fallback for models that don't follow the <thinking> tag
/// instruction. It looks for paragraph breaks where actual content follows.
fn strip_reasoning_patterns(text: &str) -> String {
    let text = text.trim();
    if text.is_empty() {
        return text.to_string();
    }

    // If text already looks clean (starts with actual content), return as-is
    // Actual content often starts with: markdown, code blocks, direct statements
    let first_char = text.chars().next().unwrap_or(' ');
    if first_char == '#' || first_char == '`' || first_char == '*' || first_char == '-' {
        return text.to_string();
    }

    // Look for paragraph break followed by actual content
    // Often models output: "thinking...\n\nActual response"
    if let Some(idx) = text.find("\n\n") {
        let after_break = text[idx + 2..].trim();
        if !after_break.is_empty() {
            let first_after = after_break.chars().next().unwrap_or(' ');
            // If it starts with typical response markers, use content after break
            if first_after == '#'
                || first_after == '`'
                || first_after == '*'
                || first_after == '-'
                || after_break.to_lowercase().starts_with("here")
                || after_break.to_lowercase().starts_with("i'd")
                || after_break.to_lowercase().starts_with("sure")
            {
                return after_break.to_string();
            }
        }
    }

    // Return original if no clear split found
    text.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_json() {
        let text = r#"Here's the plan:
{"goal": "test", "actions": []}
That's my plan."#;

        let json = extract_json(text).unwrap();
        assert!(json.starts_with('{'));
        assert!(json.ends_with('}'));
    }

    #[test]
    fn test_reasoning_context_builder() {
        let context = ReasoningContext::new()
            .with_message(ChatMessage::user("Hello"))
            .with_job("Test job");

        assert_eq!(context.messages.len(), 1);
        assert!(context.job_description.is_some());
    }

    #[test]
    fn test_strip_thinking_tags_basic() {
        let input = "<thinking>Let me think about this...</thinking>Hello, user!";
        let output = strip_internal_tags(input);
        assert_eq!(output, "Hello, user!");
    }

    #[test]
    fn test_strip_thinking_tags_multiple() {
        let input =
            "<thinking>First thought</thinking>Hello<thinking>Second thought</thinking> world!";
        let output = strip_internal_tags(input);
        assert_eq!(output, "Hello world!");
    }

    #[test]
    fn test_strip_thinking_tags_multiline() {
        let input = r#"<thinking>
I need to consider:
1. What the user wants
2. How to respond
</thinking>
Here is my response to your question."#;
        let output = strip_internal_tags(input);
        assert_eq!(output, "Here is my response to your question.");
    }

    #[test]
    fn test_strip_thinking_tags_no_tags() {
        let input = "Just a normal response without thinking tags.";
        let output = strip_internal_tags(input);
        assert_eq!(output, "Just a normal response without thinking tags.");
    }

    #[test]
    fn test_strip_thinking_tags_unclosed() {
        // Malformed: unclosed tag should strip from there to end
        let input = "Hello <thinking>this never closes";
        let output = strip_internal_tags(input);
        assert_eq!(output, "Hello");
    }

    #[test]
    fn test_strip_tool_call_tags() {
        // GLM-4.7 emits this garbage instead of using the tool_calls array
        let input = "<tool_call>tool_list</tool_call>";
        let output = strip_internal_tags(input);
        assert_eq!(output, "");
    }

    #[test]
    fn test_strip_tool_call_with_surrounding_text() {
        let input = "Here is my answer.\n\n<tool_call>\n{\"name\": \"search\", \"arguments\": {}}\n</tool_call>";
        let output = strip_internal_tags(input);
        assert_eq!(output, "Here is my answer.");
    }

    #[test]
    fn test_strip_multiple_internal_tags() {
        let input = "<thinking>Let me think</thinking>Hello!\n<tool_call>some_tool</tool_call>";
        let output = strip_internal_tags(input);
        assert_eq!(output, "Hello!");
    }

    #[test]
    fn test_strip_function_call_tags() {
        let input = "Response text<function_call>{\"name\": \"foo\"}</function_call>";
        let output = strip_internal_tags(input);
        assert_eq!(output, "Response text");
    }

    #[test]
    fn test_strip_tool_calls_plural() {
        let input = "<tool_calls>[{\"id\": \"1\"}]</tool_calls>Actual response.";
        let output = strip_internal_tags(input);
        assert_eq!(output, "Actual response.");
    }

    #[test]
    fn test_strip_pipe_delimited_tags() {
        let input = "<|tool_call|>{\"name\": \"search\"}<|/tool_call|>Hello!";
        let output = strip_internal_tags(input);
        assert_eq!(output, "Hello!");
    }

    #[test]
    fn test_strip_pipe_delimited_thinking() {
        let input = "<|thinking|>reasoning here<|/thinking|>The answer is 42.";
        let output = strip_internal_tags(input);
        assert_eq!(output, "The answer is 42.");
    }

    #[test]
    fn test_strip_xml_tag_with_attributes() {
        let input = "<tool_call type=\"function\">search()</tool_call>Done.";
        let output = strip_internal_tags(input);
        assert_eq!(output, "Done.");
    }

    #[test]
    fn test_clean_response_preserves_normal_content() {
        let input = "The function tool_call_handler works great. No tags here!";
        let output = clean_response(input);
        assert_eq!(
            output,
            "The function tool_call_handler works great. No tags here!"
        );
    }

    #[test]
    fn test_strip_reasoning_paragraph_break() {
        // Content after paragraph break with "here" marker
        let input = "Some thinking here.\n\nHere's the answer:";
        let output = strip_reasoning_patterns(input);
        assert_eq!(output, "Here's the answer:");
    }

    #[test]
    fn test_strip_reasoning_markdown_after_break() {
        // Content after paragraph break starting with markdown
        let input = "Some reasoning.\n\n**The Solution**\n- Item 1";
        let output = strip_reasoning_patterns(input);
        assert_eq!(output, "**The Solution**\n- Item 1");
    }

    #[test]
    fn test_strip_reasoning_preserves_markdown_start() {
        // If response starts with markdown, keep as-is
        let input = "**What type of tool?**\n- Option 1\n- Option 2";
        let output = strip_reasoning_patterns(input);
        assert_eq!(output, "**What type of tool?**\n- Option 1\n- Option 2");
    }

    #[test]
    fn test_strip_reasoning_preserves_code_start() {
        // If response starts with code block, keep as-is
        let input = "```rust\nfn main() {}\n```";
        let output = strip_reasoning_patterns(input);
        assert_eq!(output, "```rust\nfn main() {}\n```");
    }

    #[test]
    fn test_strip_reasoning_no_paragraph_break() {
        // Without clear paragraph break, return original
        let input = "Some text without clear separation.";
        let output = strip_reasoning_patterns(input);
        assert_eq!(output, "Some text without clear separation.");
    }

    #[test]
    fn test_clean_response_combined() {
        // Combines thinking tags + paragraph break fallback
        let input = "<thinking>Internal thought</thinking>Some text.\n\nHere's the answer.";
        let output = clean_response(input);
        assert_eq!(output, "Here's the answer.");
    }

    // -- recover_tool_calls_from_content tests --

    fn make_tools(names: &[&str]) -> Vec<ToolDefinition> {
        names
            .iter()
            .map(|n| ToolDefinition {
                name: n.to_string(),
                description: String::new(),
                parameters: serde_json::json!({}),
            })
            .collect()
    }

    #[test]
    fn test_recover_bare_tool_name() {
        let tools = make_tools(&["tool_list", "tool_auth"]);
        let content = "<tool_call>tool_list</tool_call>";
        let calls = recover_tool_calls_from_content(content, &tools);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "tool_list");
        assert_eq!(calls[0].arguments, serde_json::json!({}));
    }

    #[test]
    fn test_recover_json_tool_call() {
        let tools = make_tools(&["memory_search"]);
        let content =
            r#"<tool_call>{"name": "memory_search", "arguments": {"query": "test"}}</tool_call>"#;
        let calls = recover_tool_calls_from_content(content, &tools);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "memory_search");
        assert_eq!(calls[0].arguments, serde_json::json!({"query": "test"}));
    }

    #[test]
    fn test_recover_pipe_delimited() {
        let tools = make_tools(&["tool_list"]);
        let content = "<|tool_call|>tool_list<|/tool_call|>";
        let calls = recover_tool_calls_from_content(content, &tools);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "tool_list");
    }

    #[test]
    fn test_recover_unknown_tool_ignored() {
        let tools = make_tools(&["tool_list"]);
        let content = "<tool_call>nonexistent_tool</tool_call>";
        let calls = recover_tool_calls_from_content(content, &tools);
        assert!(calls.is_empty());
    }

    #[test]
    fn test_recover_no_tags() {
        let tools = make_tools(&["tool_list"]);
        let content = "Just a normal response.";
        let calls = recover_tool_calls_from_content(content, &tools);
        assert!(calls.is_empty());
    }

    #[test]
    fn test_recover_multiple_tool_calls() {
        let tools = make_tools(&["tool_list", "tool_auth"]);
        let content = "<tool_call>tool_list</tool_call>\n<tool_call>tool_auth</tool_call>";
        let calls = recover_tool_calls_from_content(content, &tools);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].name, "tool_list");
        assert_eq!(calls[1].name, "tool_auth");
    }

    #[test]
    fn test_recover_function_call_variant() {
        let tools = make_tools(&["shell"]);
        let content =
            r#"<function_call>{"name": "shell", "arguments": {"cmd": "ls"}}</function_call>"#;
        let calls = recover_tool_calls_from_content(content, &tools);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "shell");
    }

    #[test]
    fn test_recover_with_surrounding_text() {
        let tools = make_tools(&["tool_list"]);
        let content = "Let me check.\n\n<tool_call>tool_list</tool_call>\n\nDone.";
        let calls = recover_tool_calls_from_content(content, &tools);
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "tool_list");
    }
}
