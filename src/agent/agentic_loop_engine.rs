//! Shared agentic loop engine.
//!
//! Extracts the common LLM call → tool execution → result processing → repeat
//! loop from `dispatcher.rs`, `worker.rs`, and `worker/runtime.rs` into a
//! single reusable engine. Consumers customize behavior through the
//! `LoopDelegate` trait.
//!
//! # Design decisions
//!
//! - **Trait object, not generics**: Uses `&dyn LoopDelegate` to avoid
//!   monomorphization bloat for 3 consumers (which would defeat the purpose).
//! - **Approval lives in the delegate**: The dispatcher's 3-phase preflight is
//!   complex and chat-specific. The loop calls `delegate.execute_tools()` and
//!   the chat delegate handles approval internally.
//! - **Planning is a pre-loop phase**: `delegate.run_plan()` runs before the
//!   main reactive loop. Not all delegates support it.

use serde_json::Value;

use crate::error::Error;
use crate::llm::{ChatMessage, Reasoning, ReasoningContext, RespondResult, ToolSelection};
use crate::safety::SafetyLayer;

/// Signal from the delegate's environment (cancellation, injected messages).
pub enum LoopSignal {
    /// Continue the loop normally.
    Continue,
    /// Stop the loop (cancellation, timeout).
    Stop,
    /// Inject a user message into the context and continue.
    InjectMessage(String),
}

/// Outcome of the agentic loop.
pub enum LoopOutcome {
    /// Completed with a final text response.
    Response(String),
    /// A tool requires user approval (chat-specific).
    NeedApproval(Value),
    /// The loop was stopped by an external signal.
    Stopped,
    /// Completed without a text response (background job marked complete).
    Completed,
}

/// Result of a single tool execution.
pub struct ToolExecResult {
    /// The tool call ID (for building the result message).
    pub tool_call_id: String,
    /// The tool name.
    pub tool_name: String,
    /// The result string (output or error message).
    pub result: Result<String, String>,
}

/// Configuration for the agentic loop.
pub struct AgenticLoopConfig {
    /// Maximum number of tool-calling iterations before forcing a text response.
    pub max_iterations: usize,
    /// Whether to run the planning phase before the reactive loop.
    pub enable_planning: bool,
    /// Maximum consecutive tool-intent nudges before giving up.
    pub max_tool_intent_nudges: u32,
}

impl Default for AgenticLoopConfig {
    fn default() -> Self {
        Self {
            max_iterations: 50,
            enable_planning: false,
            max_tool_intent_nudges: 2,
        }
    }
}

/// Strategy trait for customizing the agentic loop per consumer.
///
/// Each consumer (chat dispatcher, background job worker, container runtime)
/// implements this trait to control tool execution, event posting, completion
/// detection, and signal handling — without duplicating the core loop logic.
#[async_trait::async_trait]
pub trait LoopDelegate: Send + Sync {
    /// Execute a batch of tool calls. Returns results in the same order.
    ///
    /// The delegate decides whether to execute sequentially or in parallel.
    /// For chat, this includes the approval flow. For jobs, this is parallel
    /// via JoinSet. For containers, this is sequential.
    async fn execute_tools(
        &self,
        selections: &[ToolSelection],
        ctx: &mut ReasoningContext,
    ) -> Result<Vec<ToolExecResult>, LoopOutcome>;

    /// Handle a text-only response from the LLM.
    ///
    /// Returns `Some(LoopOutcome)` to exit the loop, or `None` to continue.
    /// - Chat: returns `Some(Response)` immediately (first text = done).
    /// - Job/Container: checks `llm_signals_completion()`, returns `Some` if
    ///   complete, `None` to continue.
    async fn handle_text_response(
        &self,
        text: &str,
        ctx: &mut ReasoningContext,
    ) -> Option<LoopOutcome>;

    /// Check for external signals (cancellation, user messages, interrupts).
    async fn check_signals(&self) -> LoopSignal;

    /// Called after tool results are processed (for event sourcing, SSE, etc.).
    async fn on_tool_results(&self, results: &[ToolExecResult]);

    /// Called before each LLM call (for status updates).
    async fn on_before_llm_call(&self, iteration: usize);

    /// Called after each LLM call with token usage info.
    async fn on_after_llm_call(&self, input_tokens: u32, output_tokens: u32, cost: f64);

    /// Get the current tool definitions (may change between iterations).
    async fn tool_definitions(&self) -> Vec<crate::llm::ToolDefinition>;

    /// Run the optional planning phase. Only called if `config.enable_planning`.
    async fn run_plan(
        &self,
        _reasoning: &Reasoning,
        _ctx: &mut ReasoningContext,
    ) -> Result<bool, Error> {
        // Default: no planning
        Ok(false)
    }
}

/// The single agentic loop. All three consumers call this.
///
/// Replaces the duplicated loops in `dispatcher.rs`, `worker.rs`, and
/// `worker/runtime.rs` with a single implementation.
pub async fn run_agentic_loop(
    delegate: &dyn LoopDelegate,
    reasoning: &Reasoning,
    ctx: &mut ReasoningContext,
    safety: &SafetyLayer,
    config: &AgenticLoopConfig,
) -> Result<LoopOutcome, Error> {
    // Optional planning phase
    if config.enable_planning {
        let plan_completed = delegate.run_plan(reasoning, ctx).await?;
        if plan_completed {
            return Ok(LoopOutcome::Completed);
        }
    }

    let force_text_at = config.max_iterations;
    let nudge_at = config.max_iterations.saturating_sub(1);
    let mut iteration = 0;
    let mut consecutive_tool_intent_nudges: u32 = 0;

    loop {
        iteration += 1;

        // Hard ceiling safety net
        if iteration > config.max_iterations + 1 {
            return Err(crate::error::LlmError::InvalidResponse {
                provider: "agent".to_string(),
                reason: format!(
                    "Exceeded maximum tool iterations ({})",
                    config.max_iterations
                ),
            }
            .into());
        }

        // Check for external signals (cancellation, injected messages)
        match delegate.check_signals().await {
            LoopSignal::Continue => {}
            LoopSignal::Stop => return Ok(LoopOutcome::Stopped),
            LoopSignal::InjectMessage(msg) => {
                ctx.messages.push(ChatMessage::user(msg));
            }
        }

        // Inject nudge message when approaching iteration limit
        if iteration == nudge_at {
            ctx.messages.push(ChatMessage::system(
                "You are approaching the tool call limit. \
                 Provide your best final answer on the next response \
                 using the information you have gathered so far. \
                 Do not call any more tools.",
            ));
        }

        let force_text = iteration >= force_text_at;
        ctx.force_text = force_text;

        // Refresh tool definitions each iteration
        if !force_text {
            ctx.available_tools = delegate.tool_definitions().await;
        } else {
            ctx.available_tools = Vec::new();
            tracing::info!(
                iteration,
                "Forcing text-only response (iteration limit reached)"
            );
        }

        // Notify delegate before LLM call
        delegate.on_before_llm_call(iteration).await;

        // Call LLM
        let output = reasoning.respond_with_tools(ctx).await?;

        // Notify delegate after LLM call
        delegate
            .on_after_llm_call(
                output.usage.input_tokens,
                output.usage.output_tokens,
                0.0, // cost computed by delegate
            )
            .await;

        match output.result {
            RespondResult::Text(text) => {
                // Tool intent nudge: if the LLM expressed intent to use tools
                // but didn't actually call any, nudge it to use them.
                if !force_text
                    && !ctx.available_tools.is_empty()
                    && consecutive_tool_intent_nudges < config.max_tool_intent_nudges
                    && crate::llm::llm_signals_tool_intent(&text)
                {
                    consecutive_tool_intent_nudges += 1;
                    tracing::info!(
                        iteration,
                        "LLM expressed tool intent without calling a tool, nudging"
                    );
                    ctx.messages.push(ChatMessage::assistant(&text));
                    ctx.messages
                        .push(ChatMessage::user(crate::llm::TOOL_INTENT_NUDGE));
                    continue;
                }

                // Delegate decides whether this text completes the loop
                if let Some(outcome) = delegate.handle_text_response(&text, ctx).await {
                    return Ok(outcome);
                }

                // If delegate returns None, continue looping
                ctx.messages.push(ChatMessage::assistant(&text));
            }

            RespondResult::ToolCalls {
                tool_calls,
                content,
            } => {
                consecutive_tool_intent_nudges = 0;

                // Add assistant message with tool_calls to context
                ctx.messages.push(ChatMessage::assistant_with_tool_calls(
                    content,
                    tool_calls.clone(),
                ));

                // Convert ToolCalls to ToolSelections for execution
                let selections: Vec<ToolSelection> = tool_calls
                    .iter()
                    .map(|tc| ToolSelection {
                        tool_call_id: tc.id.clone(),
                        tool_name: tc.name.clone(),
                        parameters: tc.arguments.clone(),
                        reasoning: String::new(),
                        alternatives: Vec::new(),
                    })
                    .collect();

                // Delegate executes tools (handles approval, parallelism, etc.)
                let results = match delegate.execute_tools(&selections, ctx).await {
                    Ok(results) => results,
                    Err(outcome) => return Ok(outcome), // e.g. NeedApproval
                };

                // Process results: sanitize and add to context
                for result in &results {
                    let output_str = match &result.result {
                        Ok(s) => s.clone(),
                        Err(e) => format!("Error: {}", e),
                    };

                    // Sanitize tool output through safety layer
                    let sanitized = safety.sanitize_tool_output(&result.tool_name, &output_str);
                    let wrapped = safety.wrap_for_llm(
                        &result.tool_name,
                        &sanitized.content,
                        sanitized.was_modified,
                    );

                    ctx.messages.push(ChatMessage::tool_result(
                        &result.tool_call_id,
                        &result.tool_name,
                        wrapped,
                    ));
                }

                // Notify delegate of results (for event sourcing, SSE, etc.)
                delegate.on_tool_results(&results).await;
            }
        }
    }
}

/// Execute a tool with timeout and serialize the result.
///
/// Shared helper that replaces the 4 duplicated copies of:
/// timeout → tool.execute → serialize
///
/// Note: parameter validation is done separately by the consumer
/// via `safety.validator().validate_tool_params()`.
pub async fn execute_tool_with_safety(
    tool: &dyn crate::tools::Tool,
    params: &serde_json::Value,
    job_ctx: &crate::context::JobContext,
    timeout: std::time::Duration,
) -> Result<String, String> {
    // Execute with timeout
    match tokio::time::timeout(timeout, tool.execute(params.clone(), job_ctx)).await {
        Ok(Ok(output)) => Ok(serde_json::to_string_pretty(&output.result)
            .unwrap_or_else(|_| output.result.to_string())),
        Ok(Err(e)) => Err(format!("Tool error: {}", e)),
        Err(_) => Err(format!(
            "Tool execution timed out after {} seconds",
            timeout.as_secs()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_agentic_loop_config_defaults() {
        let config = AgenticLoopConfig::default();
        assert_eq!(config.max_iterations, 50);
        assert!(!config.enable_planning);
        assert_eq!(config.max_tool_intent_nudges, 2);
    }

    #[test]
    fn test_loop_signal_variants() {
        // Ensure all variants are constructable
        let _ = LoopSignal::Continue;
        let _ = LoopSignal::Stop;
        let _ = LoopSignal::InjectMessage("test".to_string());
    }

    #[test]
    fn test_loop_outcome_variants() {
        let _ = LoopOutcome::Response("done".to_string());
        let _ = LoopOutcome::Stopped;
        let _ = LoopOutcome::Completed;
        let _ = LoopOutcome::NeedApproval(serde_json::json!({}));
    }

    #[test]
    fn test_tool_exec_result() {
        let result = ToolExecResult {
            tool_call_id: "call_1".to_string(),
            tool_name: "test_tool".to_string(),
            result: Ok("output".to_string()),
        };
        assert_eq!(result.tool_name, "test_tool");
        assert!(result.result.is_ok());

        let err_result = ToolExecResult {
            tool_call_id: "call_2".to_string(),
            tool_name: "bad_tool".to_string(),
            result: Err("failed".to_string()),
        };
        assert!(err_result.result.is_err());
    }
}
