//! LLM reasoning capabilities for planning, tool selection, and evaluation.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::error::LlmError;
use crate::llm::{
    ChatMessage, CompletionRequest, LlmProvider, ToolCompletionRequest, ToolDefinition,
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
}

impl ReasoningContext {
    /// Create a new reasoning context.
    pub fn new() -> Self {
        Self {
            messages: Vec::new(),
            available_tools: Vec::new(),
            job_description: None,
            current_state: None,
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
}

/// Reasoning engine for the agent.
pub struct Reasoning {
    llm: Arc<dyn LlmProvider>,
    #[allow(dead_code)] // Will be used for sanitizing tool outputs
    safety: Arc<SafetyLayer>,
}

impl Reasoning {
    /// Create a new reasoning engine.
    pub fn new(llm: Arc<dyn LlmProvider>, safety: Arc<SafetyLayer>) -> Self {
        Self { llm, safety }
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

        let request =
            ToolCompletionRequest::new(context.messages.clone(), context.available_tools.clone())
                .with_max_tokens(1024)
                .with_tool_choice("auto");

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
    pub async fn respond(&self, context: &ReasoningContext) -> Result<String, LlmError> {
        let system_prompt = self.build_conversation_prompt(context);

        let mut messages = vec![ChatMessage::system(system_prompt)];
        messages.extend(context.messages.clone());

        // If we have tools, use tool completion mode
        if !context.available_tools.is_empty() {
            let request = ToolCompletionRequest::new(messages, context.available_tools.clone())
                .with_max_tokens(4096)
                .with_temperature(0.7)
                .with_tool_choice("auto");

            let response = self.llm.complete_with_tools(request).await?;

            // If there were tool calls, the content is usually just internal reasoning
            // Don't show it - just acknowledge the tool calls (actual execution handled by caller)
            if !response.tool_calls.is_empty() {
                let tool_info: Vec<String> = response
                    .tool_calls
                    .iter()
                    .map(|tc| format!("`{}({})`", tc.name, tc.arguments))
                    .collect();
                return Ok(format!("[Calling tools: {}]", tool_info.join(", ")));
            }

            // No tool calls - clean up the response
            let content = response
                .content
                .unwrap_or_else(|| "I'm not sure how to respond to that.".to_string());
            Ok(clean_response(&content))
        } else {
            // No tools, use simple completion
            let request = CompletionRequest::new(messages)
                .with_max_tokens(4096)
                .with_temperature(0.7);

            let response = self.llm.complete(request).await?;
            Ok(clean_response(&response.content))
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
                "\n\n## Available Tools\nYou have access to these tools:\n{}\n\nCall tools directly when needed - don't announce what you're going to do.",
                tool_list.join("\n")
            )
        };

        format!(
            r#"You are NEAR AI Agent, an autonomous assistant.

CRITICAL: Never output your internal reasoning or thinking process. Your response must contain ONLY the final answer or action.

FORBIDDEN patterns (never start with these):
- "The user wants..." / "The user is asking..."
- "I need to..." / "I should..." / "I will..."
- "Let me think..." / "Let me first..."
- "This is a request to..."
- Any self-narration about what you're doing

CORRECT behavior:
- Answer questions directly
- Call tools without announcing it
- Ask clarifying questions if genuinely needed
- Provide code/content without preamble{}

## Format
- Be concise
- Use markdown where helpful
- Code blocks with language tags"#,
            tools_section
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

/// Clean up LLM response by stripping thinking tags and reasoning patterns.
fn clean_response(text: &str) -> String {
    let text = strip_thinking_tags(text);
    strip_reasoning_patterns(&text)
}

/// Strip `<thinking>...</thinking>` blocks from LLM output.
///
/// Some models (especially Claude with extended thinking) include internal
/// reasoning in thinking tags. We strip these before showing to users.
fn strip_thinking_tags(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut remaining = text;

    while let Some(start) = remaining.find("<thinking>") {
        // Add everything before the tag
        result.push_str(&remaining[..start]);

        // Find the closing tag
        if let Some(end_offset) = remaining[start..].find("</thinking>") {
            // Skip past the closing tag (start + offset + tag length)
            let end = start + end_offset + "</thinking>".len();
            remaining = &remaining[end..];
        } else {
            // No closing tag found, discard everything from here
            // (malformed, but handle gracefully by not including the unclosed tag)
            remaining = "";
            break;
        }
    }

    // Add any remaining content after the last thinking block
    result.push_str(remaining);

    // Clean up any double newlines left behind
    let mut cleaned = result.trim().to_string();
    while cleaned.contains("\n\n\n") {
        cleaned = cleaned.replace("\n\n\n", "\n\n");
    }

    cleaned
}

/// Strip common reasoning/thinking patterns from the start of responses.
///
/// Models sometimes output their thinking process as plain text despite
/// instructions not to. This strips common patterns like "The user wants...",
/// "Let me think...", "I need to...", etc.
fn strip_reasoning_patterns(text: &str) -> String {
    let text = text.trim();

    // Patterns that indicate internal reasoning (case-insensitive check)
    let reasoning_prefixes = [
        "the user wants",
        "the user is asking",
        "the user would like",
        "i need to",
        "i should",
        "i will",
        "i'll",
        "let me think",
        "let me first",
        "let me check",
        "let me look",
        "let me explore",
        "let me search",
        "this is a request",
        "this request",
        "to answer this",
        "to help with this",
        "first, i",
        "okay, so",
        "alright, ",
    ];

    // Find where reasoning ends and actual content begins
    // Look for paragraph breaks or sentences that start the actual response
    let lines: Vec<&str> = text.lines().collect();
    let mut skip_until = 0;

    for (i, line) in lines.iter().enumerate() {
        let lower = line.to_lowercase();

        // Check if this line starts with a reasoning pattern
        let is_reasoning = reasoning_prefixes
            .iter()
            .any(|p| lower.trim_start().starts_with(p));

        if is_reasoning {
            skip_until = i + 1;
        } else if !line.trim().is_empty() && skip_until <= i {
            // Found non-reasoning content, stop looking
            break;
        }
    }

    if skip_until > 0 && skip_until < lines.len() {
        // Skip the reasoning lines and return the rest
        let result = lines[skip_until..].join("\n").trim().to_string();
        if !result.is_empty() {
            return result;
        }
    }

    // If we'd strip everything, just return the original (better than empty)
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
        let output = strip_thinking_tags(input);
        assert_eq!(output, "Hello, user!");
    }

    #[test]
    fn test_strip_thinking_tags_multiple() {
        let input =
            "<thinking>First thought</thinking>Hello<thinking>Second thought</thinking> world!";
        let output = strip_thinking_tags(input);
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
        let output = strip_thinking_tags(input);
        assert_eq!(output, "Here is my response to your question.");
    }

    #[test]
    fn test_strip_thinking_tags_no_tags() {
        let input = "Just a normal response without thinking tags.";
        let output = strip_thinking_tags(input);
        assert_eq!(output, "Just a normal response without thinking tags.");
    }

    #[test]
    fn test_strip_thinking_tags_unclosed() {
        // Malformed: unclosed tag should strip from there to end
        let input = "Hello <thinking>this never closes";
        let output = strip_thinking_tags(input);
        assert_eq!(output, "Hello");
    }

    #[test]
    fn test_strip_reasoning_patterns_basic() {
        let input = "The user wants me to implement something.\n\nHere's the implementation:";
        let output = strip_reasoning_patterns(input);
        assert_eq!(output, "Here's the implementation:");
    }

    #[test]
    fn test_strip_reasoning_patterns_multiline() {
        let input = r#"The user is asking about Telegram.
I need to think about what this involves.
Let me first check the existing code.

Here's what I found in the codebase."#;
        let output = strip_reasoning_patterns(input);
        assert_eq!(output, "Here's what I found in the codebase.");
    }

    #[test]
    fn test_strip_reasoning_no_patterns() {
        let input = "Here's a direct answer to your question.";
        let output = strip_reasoning_patterns(input);
        assert_eq!(output, "Here's a direct answer to your question.");
    }

    #[test]
    fn test_strip_reasoning_preserves_all_if_only_reasoning() {
        // If stripping would leave nothing, keep the original
        let input = "The user wants to know X.";
        let output = strip_reasoning_patterns(input);
        assert_eq!(output, "The user wants to know X.");
    }

    #[test]
    fn test_clean_response_combined() {
        let input =
            "<thinking>Internal thought</thinking>I need to check this.\n\nActual response here.";
        let output = clean_response(input);
        assert_eq!(output, "Actual response here.");
    }
}
