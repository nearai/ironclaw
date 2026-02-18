//! Purpose-built agentic loop for benchmarks.
//!
//! Bypasses the full Agent machinery (safety layer, approval flow, sessions,
//! dispatcher iteration limits) and directly calls the LLM with tools.
//! Benchmarks have fundamentally different requirements:
//!   - No user in the loop, no approval needed
//!   - Trusted prompts (dataset content), no injection filtering
//!   - Higher iteration limits (SWE-bench tasks need 20-50 tool calls)
//!   - Only suite-specific tools (no 20+ builtin dilution)

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use ironclaw::context::JobContext;
use ironclaw::llm::{ChatMessage, LlmProvider, ToolCall, ToolCompletionRequest, ToolDefinition};
use ironclaw::tools::Tool;

use crate::error::BenchError;
use crate::results::TraceToolCall;

/// Result of running the agentic loop.
pub struct AgenticResult {
    /// Final text response from the LLM.
    pub response: String,
    /// All tool calls made during the loop.
    pub tool_calls: Vec<TraceToolCall>,
    /// Number of LLM iterations used.
    pub iterations: usize,
    /// Whether we hit the iteration cap without the LLM finishing.
    pub hit_iteration_limit: bool,
}

/// A direct agentic loop that calls the LLM with tools until it produces
/// a final text response or hits the iteration limit.
pub struct AgenticLoop {
    llm: Arc<dyn LlmProvider>,
    tools: HashMap<String, Arc<dyn Tool>>,
    max_iterations: usize,
}

impl AgenticLoop {
    pub fn new(
        llm: Arc<dyn LlmProvider>,
        tools: Vec<Arc<dyn Tool>>,
        max_iterations: usize,
    ) -> Self {
        let tool_map: HashMap<String, Arc<dyn Tool>> = tools
            .into_iter()
            .map(|t| (t.name().to_string(), t))
            .collect();
        Self {
            llm,
            tools: tool_map,
            max_iterations,
        }
    }

    /// Build LLM tool definitions from the registered tools (done once).
    fn tool_definitions(&self) -> Vec<ToolDefinition> {
        self.tools
            .values()
            .map(|t| ToolDefinition {
                name: t.name().to_string(),
                description: t.description().to_string(),
                parameters: t.parameters_schema(),
            })
            .collect()
    }

    /// Execute a single tool call, returning the result as a string.
    async fn execute_tool_call(&self, call: &ToolCall) -> (String, TraceToolCall) {
        let start = Instant::now();
        let ctx = JobContext::default();

        let (content, success) = match self.tools.get(&call.name) {
            Some(tool) => match tool.execute(call.arguments.clone(), &ctx).await {
                Ok(output) => {
                    let text = match &output.result {
                        serde_json::Value::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                    (text, true)
                }
                Err(e) => (format!("Error: {e}"), false),
            },
            None => (format!("Error: unknown tool '{}'", call.name), false),
        };

        let trace = TraceToolCall {
            name: call.name.clone(),
            duration_ms: start.elapsed().as_millis() as u64,
            success,
        };

        (content, trace)
    }

    /// Run the agentic loop: LLM call -> tool execution -> repeat.
    pub async fn run(
        &self,
        system_prompt: &str,
        user_prompt: &str,
    ) -> Result<AgenticResult, BenchError> {
        let tool_defs = self.tool_definitions();
        let mut messages = vec![
            ChatMessage::system(system_prompt),
            ChatMessage::user(user_prompt),
        ];
        let mut all_tool_calls: Vec<TraceToolCall> = Vec::new();
        let mut tools_have_been_used = false;

        for iteration in 0..self.max_iterations {
            let request = ToolCompletionRequest::new(messages.clone(), tool_defs.clone())
                .with_temperature(0.0);

            let response = self.llm.complete_with_tools(request).await.map_err(|e| {
                BenchError::TaskFailed {
                    task_id: "agentic-loop".to_string(),
                    reason: format!("LLM call failed on iteration {iteration}: {e}"),
                }
            })?;

            if !response.tool_calls.is_empty() {
                tools_have_been_used = true;

                // Add the assistant message with tool calls to conversation
                messages.push(ChatMessage::assistant_with_tool_calls(
                    response.content.clone(),
                    response.tool_calls.clone(),
                ));

                // Execute each tool call and add results
                for call in &response.tool_calls {
                    let (result_content, trace) = self.execute_tool_call(call).await;
                    all_tool_calls.push(trace);
                    messages.push(ChatMessage::tool_result(
                        &call.id,
                        &call.name,
                        result_content,
                    ));
                }
                continue;
            }

            // Text-only response
            let text = response.content.unwrap_or_default();

            // If the LLM responded with text before ever using tools, nudge it.
            // Give it a few chances (iteration < 3) before accepting.
            if !tools_have_been_used && iteration < 3 {
                messages.push(ChatMessage::assistant(&text));
                messages.push(ChatMessage::user(
                    "You have not used any tools yet. Please use the available tools \
                     to explore the repository, understand the problem, and make the \
                     necessary code changes. Start by reading relevant source files.",
                ));
                continue;
            }

            // Done: LLM gave a text response after using tools (or gave up).
            return Ok(AgenticResult {
                response: text,
                tool_calls: all_tool_calls,
                iterations: iteration + 1,
                hit_iteration_limit: false,
            });
        }

        // Hit the iteration limit
        // Extract the last assistant message if any
        let last_response = messages
            .iter()
            .rev()
            .find(|m| m.role == ironclaw::llm::Role::Assistant)
            .map(|m| m.content.clone())
            .unwrap_or_default();

        Ok(AgenticResult {
            response: last_response,
            tool_calls: all_tool_calls,
            iterations: self.max_iterations,
            hit_iteration_limit: true,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use ironclaw::context::JobContext;
    use ironclaw::llm::{
        CompletionRequest, CompletionResponse, FinishReason, ToolCompletionResponse,
    };
    use ironclaw::tools::{ToolError, ToolOutput};
    use rust_decimal::Decimal;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    // -- Fake LLM that returns text immediately --

    struct TextOnlyLlm {
        response: String,
    }

    #[async_trait]
    impl LlmProvider for TextOnlyLlm {
        fn model_name(&self) -> &str {
            "fake"
        }
        fn cost_per_token(&self) -> (Decimal, Decimal) {
            (Decimal::ZERO, Decimal::ZERO)
        }
        async fn complete(
            &self,
            _req: CompletionRequest,
        ) -> Result<CompletionResponse, ironclaw::error::LlmError> {
            Ok(CompletionResponse {
                content: self.response.clone(),
                input_tokens: 10,
                output_tokens: 5,
                finish_reason: FinishReason::Stop,
                response_id: None,
            })
        }
        async fn complete_with_tools(
            &self,
            _req: ToolCompletionRequest,
        ) -> Result<ToolCompletionResponse, ironclaw::error::LlmError> {
            Ok(ToolCompletionResponse {
                content: Some(self.response.clone()),
                tool_calls: vec![],
                input_tokens: 10,
                output_tokens: 5,
                finish_reason: FinishReason::Stop,
                response_id: None,
            })
        }
    }

    // -- Fake LLM that calls a tool first, then responds with text --

    struct ToolThenTextLlm {
        call_count: AtomicUsize,
    }

    #[async_trait]
    impl LlmProvider for ToolThenTextLlm {
        fn model_name(&self) -> &str {
            "fake-tool"
        }
        fn cost_per_token(&self) -> (Decimal, Decimal) {
            (Decimal::ZERO, Decimal::ZERO)
        }
        async fn complete(
            &self,
            _req: CompletionRequest,
        ) -> Result<CompletionResponse, ironclaw::error::LlmError> {
            unreachable!()
        }
        async fn complete_with_tools(
            &self,
            _req: ToolCompletionRequest,
        ) -> Result<ToolCompletionResponse, ironclaw::error::LlmError> {
            let n = self.call_count.fetch_add(1, Ordering::SeqCst);
            if n == 0 {
                // First call: request a tool call
                Ok(ToolCompletionResponse {
                    content: None,
                    tool_calls: vec![ToolCall {
                        id: "call_1".to_string(),
                        name: "echo".to_string(),
                        arguments: serde_json::json!({"message": "hello"}),
                    }],
                    input_tokens: 10,
                    output_tokens: 5,
                    finish_reason: FinishReason::ToolUse,
                    response_id: None,
                })
            } else {
                // Second call: text response
                Ok(ToolCompletionResponse {
                    content: Some("Done fixing the bug.".to_string()),
                    tool_calls: vec![],
                    input_tokens: 20,
                    output_tokens: 10,
                    finish_reason: FinishReason::Stop,
                    response_id: None,
                })
            }
        }
    }

    // -- Fake LLM that always requests tool calls (to test iteration limit) --

    struct InfiniteToolLlm;

    #[async_trait]
    impl LlmProvider for InfiniteToolLlm {
        fn model_name(&self) -> &str {
            "infinite"
        }
        fn cost_per_token(&self) -> (Decimal, Decimal) {
            (Decimal::ZERO, Decimal::ZERO)
        }
        async fn complete(
            &self,
            _req: CompletionRequest,
        ) -> Result<CompletionResponse, ironclaw::error::LlmError> {
            unreachable!()
        }
        async fn complete_with_tools(
            &self,
            _req: ToolCompletionRequest,
        ) -> Result<ToolCompletionResponse, ironclaw::error::LlmError> {
            Ok(ToolCompletionResponse {
                content: None,
                tool_calls: vec![ToolCall {
                    id: "call_x".to_string(),
                    name: "echo".to_string(),
                    arguments: serde_json::json!({"message": "loop"}),
                }],
                input_tokens: 5,
                output_tokens: 5,
                finish_reason: FinishReason::ToolUse,
                response_id: None,
            })
        }
    }

    // -- Simple echo tool for tests --

    struct EchoTool;

    #[async_trait]
    impl Tool for EchoTool {
        fn name(&self) -> &str {
            "echo"
        }
        fn description(&self) -> &str {
            "Echoes back the message"
        }
        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({
                "type": "object",
                "properties": {
                    "message": {"type": "string"}
                },
                "required": ["message"]
            })
        }
        async fn execute(
            &self,
            params: serde_json::Value,
            _ctx: &JobContext,
        ) -> Result<ToolOutput, ToolError> {
            let msg = params
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            Ok(ToolOutput::text(msg, Duration::from_millis(1)))
        }
        fn requires_sanitization(&self) -> bool {
            false
        }
    }

    fn echo_tools() -> Vec<Arc<dyn Tool>> {
        vec![Arc::new(EchoTool) as Arc<dyn Tool>]
    }

    #[tokio::test]
    async fn test_agentic_loop_text_response() {
        // LLM returns text immediately. Since no tools are used and iteration < 3,
        // the loop should nudge. After 3 nudges, it accepts the text response.
        let llm = Arc::new(TextOnlyLlm {
            response: "I cannot use tools.".to_string(),
        });
        let loop_ = AgenticLoop::new(llm, echo_tools(), 10);
        let result = loop_.run("system", "user prompt").await.unwrap();

        // Accepted after nudging phase (iteration 3, so 4 iterations: 0, 1, 2, 3)
        assert_eq!(result.response, "I cannot use tools.");
        assert!(result.tool_calls.is_empty());
        assert!(!result.hit_iteration_limit);
    }

    #[tokio::test]
    async fn test_agentic_loop_tool_then_response() {
        let llm = Arc::new(ToolThenTextLlm {
            call_count: AtomicUsize::new(0),
        });
        let loop_ = AgenticLoop::new(llm, echo_tools(), 10);
        let result = loop_.run("system", "fix the bug").await.unwrap();

        assert_eq!(result.response, "Done fixing the bug.");
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].name, "echo");
        assert!(result.tool_calls[0].success);
        assert_eq!(result.iterations, 2);
        assert!(!result.hit_iteration_limit);
    }

    #[tokio::test]
    async fn test_agentic_loop_max_iterations() {
        let llm = Arc::new(InfiniteToolLlm);
        let loop_ = AgenticLoop::new(llm, echo_tools(), 5);
        let result = loop_.run("system", "fix it").await.unwrap();

        assert!(result.hit_iteration_limit);
        assert_eq!(result.iterations, 5);
        assert_eq!(result.tool_calls.len(), 5); // one tool call per iteration
    }

    #[tokio::test]
    async fn test_agentic_loop_unknown_tool() {
        // LLM requests a tool that doesn't exist
        struct UnknownToolLlm;

        #[async_trait]
        impl LlmProvider for UnknownToolLlm {
            fn model_name(&self) -> &str {
                "fake"
            }
            fn cost_per_token(&self) -> (Decimal, Decimal) {
                (Decimal::ZERO, Decimal::ZERO)
            }
            async fn complete(
                &self,
                _req: CompletionRequest,
            ) -> Result<CompletionResponse, ironclaw::error::LlmError> {
                unreachable!()
            }
            async fn complete_with_tools(
                &self,
                req: ToolCompletionRequest,
            ) -> Result<ToolCompletionResponse, ironclaw::error::LlmError> {
                // Check if there's already a tool result in the messages (meaning we already called the unknown tool)
                let has_tool_result = req
                    .messages
                    .iter()
                    .any(|m| m.role == ironclaw::llm::Role::Tool);
                if has_tool_result {
                    return Ok(ToolCompletionResponse {
                        content: Some("I see the tool failed.".to_string()),
                        tool_calls: vec![],
                        input_tokens: 5,
                        output_tokens: 5,
                        finish_reason: FinishReason::Stop,
                        response_id: None,
                    });
                }
                Ok(ToolCompletionResponse {
                    content: None,
                    tool_calls: vec![ToolCall {
                        id: "call_1".to_string(),
                        name: "nonexistent_tool".to_string(),
                        arguments: serde_json::json!({}),
                    }],
                    input_tokens: 5,
                    output_tokens: 5,
                    finish_reason: FinishReason::ToolUse,
                    response_id: None,
                })
            }
        }

        let llm = Arc::new(UnknownToolLlm);
        let loop_ = AgenticLoop::new(llm, echo_tools(), 10);
        let result = loop_.run("system", "test").await.unwrap();

        assert_eq!(result.tool_calls.len(), 1);
        assert!(!result.tool_calls[0].success);
        assert_eq!(result.response, "I see the tool failed.");
    }
}
