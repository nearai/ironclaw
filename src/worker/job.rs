//! Job delegate for background scheduler jobs.
//!
//! Replaces the execution loop from `src/agent/worker.rs`. Implements
//! `LoopDelegate` to customize the shared agentic loop for background
//! job execution with:
//! - Parallel tool execution via JoinSet
//! - Event sourcing to database
//! - SSE broadcast for live job streaming
//! - Completion detection via `llm_signals_completion()`
//! - Optional planning phase
//! - Signal handling (stop, ping, user messages via mpsc channel)

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::task::JoinSet;
use uuid::Uuid;

use crate::agent::agentic_loop_engine::{
    AgenticLoopConfig, LoopDelegate, LoopOutcome, LoopSignal, ToolExecResult,
};
use crate::agent::WorkerMessage;
use crate::channels::web::types::SseEvent;
use crate::context::ContextManager;
use crate::db::Database;
use crate::error::Error;
use crate::hooks::HookRegistry;
use crate::llm::{ChatMessage, LlmProvider, Reasoning, ReasoningContext, ToolSelection};
use crate::safety::SafetyLayer;
use crate::tools::{ApprovalContext, ToolRegistry};

/// Shared dependencies for job execution (equivalent to old `WorkerDeps`).
#[derive(Clone)]
pub struct JobDeps {
    pub context_manager: Arc<ContextManager>,
    pub llm: Arc<dyn LlmProvider>,
    pub safety: Arc<SafetyLayer>,
    pub tools: Arc<ToolRegistry>,
    pub store: Option<Arc<dyn Database>>,
    pub hooks: Arc<HookRegistry>,
    pub timeout: Duration,
    pub use_planning: bool,
    pub sse_tx: Option<tokio::sync::broadcast::Sender<SseEvent>>,
    pub approval_context: Option<ApprovalContext>,
    pub http_interceptor: Option<Arc<dyn crate::llm::recording::HttpInterceptor>>,
}

/// Job delegate for background scheduler jobs.
pub struct JobDelegate {
    job_id: Uuid,
    deps: JobDeps,
    rx: tokio::sync::Mutex<mpsc::Receiver<WorkerMessage>>,
}

impl JobDelegate {
    pub fn new(job_id: Uuid, deps: JobDeps, rx: mpsc::Receiver<WorkerMessage>) -> Self {
        Self {
            job_id,
            deps,
            rx: tokio::sync::Mutex::new(rx),
        }
    }

    fn tools(&self) -> &ToolRegistry {
        &self.deps.tools
    }

    fn log_event(&self, event_type: &str, data: serde_json::Value) {
        if let Some(ref tx) = self.deps.sse_tx {
            // Map event_type to the appropriate SseEvent variant
            let event = match event_type {
                "message" => {
                    let role = data
                        .get("role")
                        .and_then(|v| v.as_str())
                        .unwrap_or("assistant")
                        .to_string();
                    let content = data
                        .get("content")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    SseEvent::JobMessage {
                        job_id: self.job_id.to_string(),
                        role,
                        content,
                    }
                }
                "tool_result" => {
                    let tool_name = data
                        .get("tool_name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let output = data
                        .get("status")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown")
                        .to_string();
                    SseEvent::JobToolResult {
                        job_id: self.job_id.to_string(),
                        tool_name,
                        output,
                    }
                }
                _ => SseEvent::JobStatus {
                    job_id: self.job_id.to_string(),
                    message: serde_json::to_string(&data).unwrap_or_default(),
                },
            };
            let _ = tx.send(event);
        }
    }

    /// Build the `AgenticLoopConfig` for this job.
    pub fn loop_config(&self) -> AgenticLoopConfig {
        AgenticLoopConfig {
            max_iterations: 50, // overridden from job metadata if available
            enable_planning: self.deps.use_planning,
            max_tool_intent_nudges: 2,
        }
    }
}

#[async_trait::async_trait]
impl LoopDelegate for JobDelegate {
    async fn execute_tools(
        &self,
        selections: &[ToolSelection],
        _ctx: &mut ReasoningContext,
    ) -> Result<Vec<ToolExecResult>, LoopOutcome> {
        let mut results = Vec::with_capacity(selections.len());

        // Parallel execution via JoinSet
        let mut join_set = JoinSet::new();
        for sel in selections {
            let tools = Arc::clone(&self.deps.tools);
            let ctx_mgr = Arc::clone(&self.deps.context_manager);
            let job_id = self.job_id;
            let tool_name = sel.tool_name.clone();
            let tool_call_id = sel.tool_call_id.clone();
            let params = sel.parameters.clone();

            join_set.spawn(async move {
                let tool = match tools.get(&tool_name).await {
                    Some(t) => t,
                    None => {
                        return ToolExecResult {
                            tool_call_id,
                            tool_name,
                            result: Err("Tool not found".to_string()),
                        };
                    }
                };

                let job_ctx = match ctx_mgr.get_context(job_id).await {
                    Ok(ctx) => ctx,
                    Err(e) => {
                        return ToolExecResult {
                            tool_call_id,
                            tool_name,
                            result: Err(format!("Context error: {}", e)),
                        };
                    }
                };

                let timeout = tool.execution_timeout();
                let result =
                    crate::tools::execute::execute_tool_safely(&*tool, &params, &job_ctx, timeout)
                        .await;

                ToolExecResult {
                    tool_call_id,
                    tool_name,
                    result: if result.success {
                        Ok(result.raw_output)
                    } else {
                        Err(result.raw_output)
                    },
                }
            });
        }

        // Collect results preserving order
        while let Some(result) = join_set.join_next().await {
            match result {
                Ok(tool_result) => results.push(tool_result),
                Err(e) => {
                    tracing::error!("Tool execution panicked: {}", e);
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
        // Log the text response
        self.log_event(
            "message",
            serde_json::json!({
                "role": "assistant",
                "content": text,
            }),
        );

        // Check for completion signals
        if crate::util::llm_signals_completion(text) {
            tracing::info!("Job {} completion signal detected", self.job_id);
            Some(LoopOutcome::Response(text.to_string()))
        } else {
            // Continue looping — the LLM responded with text but hasn't
            // signaled completion. Keep going.
            None
        }
    }

    async fn check_signals(&self) -> LoopSignal {
        let mut rx: tokio::sync::MutexGuard<'_, mpsc::Receiver<WorkerMessage>> = self.rx.lock().await;
        while let Ok(msg) = rx.try_recv() {
            match msg {
                WorkerMessage::Stop => return LoopSignal::Stop,
                WorkerMessage::Ping => {}
                WorkerMessage::Start => {}
                WorkerMessage::UserMessage(content) => {
                    return LoopSignal::InjectMessage(content);
                }
            }
        }
        LoopSignal::Continue
    }

    async fn on_tool_results(&self, results: &[ToolExecResult]) {
        for r in results {
            let status = if r.result.is_ok() { "success" } else { "error" };
            self.log_event(
                "tool_result",
                serde_json::json!({
                    "tool_name": r.tool_name,
                    "tool_call_id": r.tool_call_id,
                    "status": status,
                }),
            );
        }
    }

    async fn on_before_llm_call(&self, iteration: usize) {
        tracing::debug!("Job {} iteration {}", self.job_id, iteration);
    }

    async fn on_after_llm_call(&self, input_tokens: u32, output_tokens: u32, _cost: f64) {
        tracing::debug!(
            "Job {} LLM call: {} in / {} out",
            self.job_id,
            input_tokens,
            output_tokens,
        );
    }

    async fn tool_definitions(&self) -> Vec<crate::llm::ToolDefinition> {
        self.tools().tool_definitions().await
    }

    async fn run_plan(
        &self,
        reasoning: &Reasoning,
        ctx: &mut ReasoningContext,
    ) -> Result<bool, Error> {
        if !self.deps.use_planning {
            return Ok(false);
        }

        match reasoning.plan(ctx).await {
            Ok(plan) => {
                tracing::info!(
                    "Created plan for job {}: {} actions, {:.0}% confidence",
                    self.job_id,
                    plan.actions.len(),
                    plan.confidence * 100.0
                );

                // Add plan to context as assistant message
                ctx.messages.push(ChatMessage::assistant(format!(
                    "I've created a plan: {}\n\nSteps:\n{}",
                    plan.goal,
                    plan.actions
                        .iter()
                        .enumerate()
                        .map(|(i, a)| format!("{}. {} - {}", i + 1, a.tool_name, a.reasoning))
                        .collect::<Vec<_>>()
                        .join("\n")
                )));

                self.log_event(
                    "message",
                    serde_json::json!({
                        "role": "assistant",
                        "content": format!("Plan: {}", plan.goal),
                    }),
                );

                // Plan was created but loop should still run
                Ok(false)
            }
            Err(e) => {
                tracing::warn!(
                    "Planning failed for job {}, falling back to direct selection: {}",
                    self.job_id,
                    e
                );
                Ok(false)
            }
        }
    }
}
