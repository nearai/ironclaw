//! Chat delegate for the shared agentic loop.
//!
//! Replaces the execution loop from `src/agent/dispatcher.rs`. Implements
//! `LoopDelegate` to customize the shared agentic loop for interactive
//! chat sessions with:
//! - Three-phase tool execution (preflight → parallel exec → postflight)
//! - Hook-based tool interception (BeforeToolCall)
//! - Approval flow (session auto-approvals, pending approval queue)
//! - Skill-based tool attenuation
//! - Session/thread interruption detection
//! - Cost guard enforcement
//! - Context compaction on ContextLengthExceeded

use std::sync::Arc;

use tokio::sync::Mutex;
use tokio::task::JoinSet;
use uuid::Uuid;

use crate::agent::agentic_loop_engine::{
    AgenticLoopConfig, LoopDelegate, LoopOutcome, LoopSignal, ToolExecResult,
};
use crate::agent::session::{Session, ThreadState};
use crate::channels::{ChannelManager, IncomingMessage, StatusUpdate};
use crate::context::JobContext;
use crate::hooks::HookRegistry;
use crate::llm::{LlmProvider, ReasoningContext, ToolDefinition, ToolSelection};
use crate::safety::SafetyLayer;
use crate::skills::LoadedSkill;
use crate::tools::{ToolRegistry, redact_params};

/// Shared dependencies for chat execution.
#[derive(Clone)]
pub struct ChatDeps {
    pub llm: Arc<dyn LlmProvider>,
    pub safety: Arc<SafetyLayer>,
    pub tools: Arc<ToolRegistry>,
    pub hooks: Arc<HookRegistry>,
    pub channels: Arc<ChannelManager>,
    pub session: Arc<Mutex<Session>>,
    pub thread_id: Uuid,
    pub job_ctx: JobContext,
    pub auto_approve_tools: bool,
    pub active_skills: Vec<LoadedSkill>,
    pub http_interceptor: Option<Arc<dyn crate::llm::recording::HttpInterceptor>>,
}

/// Chat delegate for interactive conversational turns.
///
/// Implements the three-phase tool execution pipeline from dispatcher.rs:
/// 1. Preflight: sequential hook checks + approval gating
/// 2. Execution: parallel via JoinSet (or sequential for single tools)
/// 3. Postflight: sequential result processing in original order
pub struct ChatDelegate {
    deps: ChatDeps,
    message: IncomingMessage,
}

impl ChatDelegate {
    pub fn new(deps: ChatDeps, message: IncomingMessage) -> Self {
        Self { deps, message }
    }

    fn tools(&self) -> &ToolRegistry {
        &self.deps.tools
    }

    #[allow(dead_code)]
    fn safety(&self) -> &SafetyLayer {
        &self.deps.safety
    }

    /// Build the `AgenticLoopConfig` for chat sessions.
    pub fn loop_config(&self, max_tool_iterations: usize) -> AgenticLoopConfig {
        AgenticLoopConfig {
            max_iterations: max_tool_iterations,
            enable_planning: false, // Chat doesn't use planning
            max_tool_intent_nudges: 2,
        }
    }
}

#[async_trait::async_trait]
impl LoopDelegate for ChatDelegate {
    async fn execute_tools(
        &self,
        selections: &[ToolSelection],
        _ctx: &mut ReasoningContext,
    ) -> Result<Vec<ToolExecResult>, LoopOutcome> {
        let mut results = Vec::with_capacity(selections.len());

        // === Phase 1: Preflight (sequential) ===
        // Check hooks and approval for each tool.
        enum PreflightOutcome {
            Rejected(String),
            Runnable,
        }

        let mut preflight: Vec<(usize, PreflightOutcome)> = Vec::new();
        let mut runnable_indices: Vec<usize> = Vec::new();
        let mut approval_needed: Option<(usize, &ToolSelection)> = None;

        for (idx, sel) in selections.iter().enumerate() {
            let tool_opt = self.tools().get(&sel.tool_name).await;
            let sensitive = tool_opt
                .as_ref()
                .map(|t| t.sensitive_params())
                .unwrap_or(&[]);

            // Hook: BeforeToolCall
            let hook_params = redact_params(&sel.parameters, sensitive);
            let event = crate::hooks::HookEvent::ToolCall {
                tool_name: sel.tool_name.clone(),
                parameters: hook_params,
                user_id: self.message.user_id.clone(),
                context: "chat".to_string(),
            };
            match self.deps.hooks.run(&event).await {
                Err(crate::hooks::HookError::Rejected { reason }) => {
                    preflight.push((
                        idx,
                        PreflightOutcome::Rejected(format!(
                            "Tool call rejected by hook: {}",
                            reason
                        )),
                    ));
                    continue;
                }
                Err(err) => {
                    preflight.push((
                        idx,
                        PreflightOutcome::Rejected(format!(
                            "Tool call blocked by hook policy: {}",
                            err
                        )),
                    ));
                    continue;
                }
                Ok(_) => {}
            }

            // Check approval requirement
            if !self.deps.auto_approve_tools {
                if let Some(tool) = tool_opt {
                    use crate::tools::ApprovalRequirement;
                    let needs_approval = match tool.requires_approval(&sel.parameters) {
                        ApprovalRequirement::Never => false,
                        ApprovalRequirement::UnlessAutoApproved => {
                            let sess = self.deps.session.lock().await;
                            !sess.is_tool_auto_approved(&sel.tool_name)
                        }
                        ApprovalRequirement::Always => true,
                    };

                    if needs_approval {
                        approval_needed = Some((idx, sel));
                        break; // remaining tools are deferred
                    }
                }
            }

            preflight.push((idx, PreflightOutcome::Runnable));
            runnable_indices.push(idx);
        }

        // If approval needed, return NeedApproval outcome
        if let Some((_approval_idx, sel)) = approval_needed {
            return Err(LoopOutcome::NeedApproval(serde_json::json!({
                "tool_name": sel.tool_name,
                "tool_call_id": sel.tool_call_id,
                "parameters": sel.parameters,
            })));
        }

        // === Phase 2: Parallel execution ===
        let mut exec_results: Vec<Option<ToolExecResult>> =
            (0..selections.len()).map(|_| None).collect();

        if runnable_indices.len() <= 1 {
            // Single tool: execute inline
            for &idx in &runnable_indices {
                let sel = &selections[idx];
                let result = crate::tools::execute::execute_tool_safely(
                    &*self.tools().get(&sel.tool_name).await.unwrap(),
                    &sel.parameters,
                    &self.deps.job_ctx,
                    self.tools()
                        .get(&sel.tool_name)
                        .await
                        .map(|t| t.execution_timeout())
                        .unwrap_or(std::time::Duration::from_secs(60)),
                )
                .await;

                exec_results[idx] = Some(ToolExecResult {
                    tool_call_id: sel.tool_call_id.clone(),
                    tool_name: sel.tool_name.clone(),
                    result: if result.success {
                        Ok(result.raw_output)
                    } else {
                        Err(result.raw_output)
                    },
                });
            }
        } else {
            // Multiple tools: parallel via JoinSet
            let mut join_set = JoinSet::new();
            for &idx in &runnable_indices {
                let sel = selections[idx].clone();
                let tools = Arc::clone(&self.deps.tools);
                let job_ctx = self.deps.job_ctx.clone();

                join_set.spawn(async move {
                    let tool = match tools.get(&sel.tool_name).await {
                        Some(t) => t,
                        None => {
                            return (
                                idx,
                                ToolExecResult {
                                    tool_call_id: sel.tool_call_id.clone(),
                                    tool_name: sel.tool_name.clone(),
                                    result: Err("Tool not found".to_string()),
                                },
                            );
                        }
                    };

                    let timeout = tool.execution_timeout();
                    let result = crate::tools::execute::execute_tool_safely(
                        &*tool, &sel.parameters, &job_ctx, timeout,
                    )
                    .await;

                    (
                        idx,
                        ToolExecResult {
                            tool_call_id: sel.tool_call_id.clone(),
                            tool_name: sel.tool_name.clone(),
                            result: if result.success {
                                Ok(result.raw_output)
                            } else {
                                Err(result.raw_output)
                            },
                        },
                    )
                });
            }

            while let Some(join_result) = join_set.join_next().await {
                match join_result {
                    Ok((idx, tool_result)) => {
                        exec_results[idx] = Some(tool_result);
                    }
                    Err(e) => {
                        tracing::error!("Chat tool execution task panicked: {}", e);
                    }
                }
            }
        }

        // === Phase 3: Postflight (sequential, original order) ===
        for (idx, outcome) in &preflight {
            match outcome {
                PreflightOutcome::Rejected(error_msg) => {
                    results.push(ToolExecResult {
                        tool_call_id: selections[*idx].tool_call_id.clone(),
                        tool_name: selections[*idx].tool_name.clone(),
                        result: Err(error_msg.clone()),
                    });
                }
                PreflightOutcome::Runnable => {
                    if let Some(result) = exec_results[*idx].take() {
                        results.push(result);
                    } else {
                        results.push(ToolExecResult {
                            tool_call_id: selections[*idx].tool_call_id.clone(),
                            tool_name: selections[*idx].tool_name.clone(),
                            result: Err("No result available (task panicked)".to_string()),
                        });
                    }
                }
            }
        }

        Ok(results)
    }

    async fn handle_text_response(
        &self,
        text: &str,
        _ctx: &mut ReasoningContext,
    ) -> Option<LoopOutcome> {
        // Chat delegate: first text response = done (strip internal markers)
        let sanitized = strip_internal_tool_call_text(text);
        Some(LoopOutcome::Response(sanitized))
    }

    async fn check_signals(&self) -> LoopSignal {
        // Check for thread interruption
        let sess = self.deps.session.lock().await;
        if let Some(thread) = sess.threads.get(&self.deps.thread_id)
            && thread.state == ThreadState::Interrupted
        {
            return LoopSignal::Stop;
        }
        LoopSignal::Continue
    }

    async fn on_tool_results(&self, results: &[ToolExecResult]) {
        // Record results in session thread
        let mut sess = self.deps.session.lock().await;
        if let Some(thread) = sess.threads.get_mut(&self.deps.thread_id)
            && let Some(turn) = thread.last_turn_mut()
        {
            for r in results {
                match &r.result {
                    Ok(output) => {
                        turn.record_tool_result(serde_json::json!(output));
                    }
                    Err(e) => {
                        turn.record_tool_error(e.to_string());
                    }
                }
            }
        }
    }

    async fn on_before_llm_call(&self, iteration: usize) {
        let _ = self
            .deps
            .channels
            .send_status(
                &self.message.channel,
                StatusUpdate::Thinking("Calling LLM...".into()),
                &self.message.metadata,
            )
            .await;
        tracing::debug!("Chat iteration {}", iteration);
    }

    async fn on_after_llm_call(&self, input_tokens: u32, output_tokens: u32, cost: f64) {
        tracing::debug!(
            "Chat LLM call: {} in / {} out (${:.6})",
            input_tokens,
            output_tokens,
            cost,
        );
    }

    async fn tool_definitions(&self) -> Vec<ToolDefinition> {
        let tool_defs = self.tools().tool_definitions().await;
        if !self.deps.active_skills.is_empty() {
            crate::skills::attenuate_tools(&tool_defs, &self.deps.active_skills).tools
        } else {
            tool_defs
        }
    }
}

/// Strip internal "[Called tool ...]" text that can leak when provider flattening
/// converts tool_calls to plain text and the LLM echoes it back.
fn strip_internal_tool_call_text(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    for line in text.lines() {
        if !line.starts_with("[Called tool ")
            && !line.starts_with("[Tool result:")
            && !line.starts_with("[Calling tool ")
        {
            if !result.is_empty() {
                result.push('\n');
            }
            result.push_str(line);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_internal_tool_call_text() {
        let input = "Hello\n[Called tool read_file]\nWorld";
        assert_eq!(strip_internal_tool_call_text(input), "Hello\nWorld");
    }

    #[test]
    fn test_strip_no_internal_markers() {
        let input = "Clean response with no markers.";
        assert_eq!(strip_internal_tool_call_text(input), input);
    }

    #[test]
    fn test_strip_multiple_markers() {
        let input = "[Called tool search]\n[Tool result: found]\nDone";
        assert_eq!(strip_internal_tool_call_text(input), "Done");
    }
}
