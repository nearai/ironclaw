//! Core execution loop — the replacement for `run_agentic_loop()`.
//!
//! The `ExecutionLoop` owns a thread and drives it through LLM call →
//! action execution → result processing → repeat cycles. Unlike the
//! existing delegate pattern, the loop is self-contained: all behavior
//! differences between thread types are handled via capability leases
//! and policy, not delegate implementations.

use std::sync::Arc;

use tracing::{debug, warn};

use crate::capability::lease::LeaseManager;
use crate::capability::policy::PolicyEngine;
use crate::executor::context::build_step_context;
use crate::executor::intent;
use crate::executor::structured::execute_action_calls;
use crate::runtime::messaging::{SignalReceiver, ThreadOutcome, ThreadSignal};
use crate::traits::effect::{EffectExecutor, ThreadExecutionContext};
use crate::traits::llm::{LlmBackend, LlmCallConfig};
use crate::types::error::EngineError;
use crate::types::event::EventKind;
use crate::types::message::ThreadMessage;
use crate::types::step::{ExecutionTier, LlmResponse, Step, StepStatus};
use crate::types::thread::{Thread, ThreadState};

/// The core execution loop for a thread.
pub struct ExecutionLoop {
    pub thread: Thread,
    llm: Arc<dyn LlmBackend>,
    effects: Arc<dyn EffectExecutor>,
    leases: Arc<LeaseManager>,
    policy: Arc<PolicyEngine>,
    signal_rx: SignalReceiver,
    user_id: String,
}

impl ExecutionLoop {
    pub fn new(
        thread: Thread,
        llm: Arc<dyn LlmBackend>,
        effects: Arc<dyn EffectExecutor>,
        leases: Arc<LeaseManager>,
        policy: Arc<PolicyEngine>,
        signal_rx: SignalReceiver,
        user_id: String,
    ) -> Self {
        Self {
            thread,
            llm,
            effects,
            leases,
            policy,
            signal_rx,
            user_id,
        }
    }

    /// Run the execution loop to completion.
    pub async fn run(&mut self) -> Result<ThreadOutcome, EngineError> {
        // Transition to Running
        self.thread.transition_to(ThreadState::Running, None)?;

        // Inject CodeAct/RLM system prompt if none exists
        if !self.thread.messages.iter().any(|m| m.role == crate::types::message::MessageRole::System) {
            // Get available actions for the prompt
            let active_leases = self.leases.active_for_thread(self.thread.id).await;
            let actions = self.effects.available_actions(&active_leases).await
                .unwrap_or_default();
            let system_prompt = crate::executor::prompt::build_codeact_system_prompt(&actions);
            self.thread.messages.insert(0, ThreadMessage::system(system_prompt));
        }

        let max_iterations = self.thread.config.max_iterations;
        let max_nudges = self.thread.config.max_tool_intent_nudges;
        let nudge_enabled = self.thread.config.enable_tool_intent_nudge;
        let start_time = std::time::Instant::now();
        let mut nudge_count: u32 = 0;
        let mut consecutive_errors: u32 = 0;
        let mut compaction_count: u32 = 0;

        for iteration in 0..max_iterations {
            // 1. Check signals
            match self.check_signals() {
                SignalAction::Continue => {}
                SignalAction::Stop => {
                    self.thread
                        .transition_to(ThreadState::Completed, Some("stopped by signal".into()))?;
                    return Ok(ThreadOutcome::Stopped);
                }
                SignalAction::Inject(msg) => {
                    self.thread.add_message(msg);
                }
            }

            // 2. Check budget limits
            if let Some(max_tokens) = self.thread.config.max_tokens_total
                && self.thread.total_tokens_used >= max_tokens
            {
                warn!(
                    thread_id = %self.thread.id,
                    used = self.thread.total_tokens_used,
                    limit = max_tokens,
                    "token limit exceeded"
                );
                self.thread.transition_to(
                    ThreadState::Completed,
                    Some("token limit exceeded".into()),
                )?;
                return Ok(ThreadOutcome::Failed {
                    error: format!(
                        "Token limit exceeded: {} of {} tokens",
                        self.thread.total_tokens_used, max_tokens
                    ),
                });
            }

            if let Some(max_dur) = self.thread.config.max_duration {
                let elapsed = start_time.elapsed();
                if elapsed >= max_dur {
                    warn!(
                        thread_id = %self.thread.id,
                        elapsed = ?elapsed,
                        limit = ?max_dur,
                        "thread timeout"
                    );
                    self.thread
                        .transition_to(ThreadState::Completed, Some("timeout".into()))?;
                    return Ok(ThreadOutcome::Failed {
                        error: format!("Thread timeout: {elapsed:?} of {max_dur:?}"),
                    });
                }
            }

            // 3. Check compaction
            if self.thread.config.enable_compaction {
                let ctx_limit = self.thread.config.model_context_limit;
                let threshold = self.thread.config.compaction_threshold;
                if crate::executor::compaction::should_compact(
                    &self.thread.messages,
                    ctx_limit,
                    threshold,
                ) {
                    debug!(
                        thread_id = %self.thread.id,
                        compaction_count,
                        "triggering context compaction"
                    );
                    let result = crate::executor::compaction::compact_messages(
                        &self.thread.messages,
                        &self.llm,
                        compaction_count,
                    )
                    .await?;
                    self.thread.total_tokens_used += result.tokens_used.total();
                    self.thread.messages = result.compacted_messages;
                    compaction_count += 1;
                }
            }

            // 4. Get active leases
            let active_leases = self.leases.active_for_thread(self.thread.id).await;

            // 5. Build context
            let (messages, _actions) =
                build_step_context(&self.thread.messages, &active_leases, &self.effects).await?;

            // 6. Create step
            let mut step = Step::new(self.thread.id, iteration + 1);
            step.status = StepStatus::LlmCalling;
            self.thread.add_event(EventKind::StepStarted {
                step_id: step.id,
            });

            // 7. Call LLM
            // CodeAct/RLM: send NO structured tool definitions — tools are described
            // in the system prompt as Python functions. The LLM produces text with
            // ```repl code blocks that the bridge detects and converts to LlmResponse::Code.
            // This avoids the LLM using structured tool calls instead of writing code.
            let force_text = iteration >= max_iterations.saturating_sub(1);
            let config = LlmCallConfig {
                force_text,
                depth: self.thread.config.depth,
                ..LlmCallConfig::default()
            };

            let llm_output = self.llm.complete(&messages, &[], &config).await?;
            step.tokens_used = llm_output.usage;
            self.thread.total_tokens_used += llm_output.usage.total();
            step.llm_response = Some(llm_output.response.clone());

            // 6. Handle response
            match llm_output.response {
                LlmResponse::Text(text) => {
                    // Check for FINAL() in text (regex fallback — models
                    // sometimes write FINAL() outside code blocks)
                    if let Some(answer) = extract_final_from_text(&text) {
                        self.thread
                            .add_message(ThreadMessage::assistant(text));
                        step.status = StepStatus::Completed;
                        step.completed_at = Some(chrono::Utc::now());
                        self.thread.add_event(EventKind::StepCompleted {
                            step_id: step.id,
                            tokens: step.tokens_used,
                        });
                        self.thread.step_count += 1;
                        self.thread.transition_to(
                            ThreadState::Completed,
                            Some("FINAL() in text".into()),
                        )?;
                        return Ok(ThreadOutcome::Completed {
                            response: Some(answer),
                        });
                    }

                    // Check for tool intent nudge
                    if nudge_enabled
                        && nudge_count < max_nudges
                        && intent::signals_tool_intent(&text)
                    {
                        nudge_count += 1;
                        debug!(
                            thread_id = %self.thread.id,
                            nudge_count,
                            "tool intent detected, injecting nudge"
                        );
                        self.thread
                            .add_message(ThreadMessage::assistant(text));
                        self.thread
                            .add_message(ThreadMessage::system(intent::TOOL_INTENT_NUDGE));

                        step.status = StepStatus::Completed;
                        step.completed_at = Some(chrono::Utc::now());
                        self.thread.add_event(EventKind::StepCompleted {
                            step_id: step.id,
                            tokens: step.tokens_used,
                        });
                        self.thread.step_count += 1;
                        continue;
                    }

                    // Final text response
                    self.thread
                        .add_message(ThreadMessage::assistant(text.clone()));

                    step.status = StepStatus::Completed;
                    step.completed_at = Some(chrono::Utc::now());
                    self.thread.add_event(EventKind::StepCompleted {
                        step_id: step.id,
                        tokens: step.tokens_used,
                    });
                    self.thread.step_count += 1;

                    self.thread.transition_to(
                        ThreadState::Completed,
                        Some("text response".into()),
                    )?;
                    return Ok(ThreadOutcome::Completed {
                        response: Some(text),
                    });
                }

                LlmResponse::ActionCalls { calls, content } => {
                    nudge_count = 0;

                    // Record assistant message with action calls
                    self.thread
                        .add_message(ThreadMessage::assistant_with_actions(content, calls.clone()));

                    step.status = StepStatus::Executing;

                    // Build execution context
                    let exec_ctx = ThreadExecutionContext {
                        thread_id: self.thread.id,
                        thread_type: self.thread.thread_type,
                        project_id: self.thread.project_id,
                        user_id: self.user_id.clone(),
                        step_id: step.id,
                    };

                    // Execute actions
                    let batch = execute_action_calls(
                        &calls,
                        &self.thread,
                        &self.effects,
                        &self.leases,
                        &self.policy,
                        &exec_ctx,
                        &[], // capability-level policies (TODO: resolve from registry)
                    )
                    .await?;

                    // Record events
                    for event_kind in batch.events {
                        self.thread.add_event(event_kind);
                    }

                    // Add action results as messages
                    for result in &batch.results {
                        self.thread.add_message(ThreadMessage::action_result(
                            &result.call_id,
                            &result.action_name,
                            serde_json::to_string(&result.output).unwrap_or_default(),
                        ));
                    }

                    step.action_results = batch.results;
                    step.status = StepStatus::Completed;
                    step.completed_at = Some(chrono::Utc::now());
                    self.thread.add_event(EventKind::StepCompleted {
                        step_id: step.id,
                        tokens: step.tokens_used,
                    });
                    self.thread.step_count += 1;

                    // Check if approval is needed
                    if let Some(outcome) = batch.need_approval {
                        self.thread
                            .transition_to(ThreadState::Waiting, Some("awaiting approval".into()))?;
                        return Ok(outcome);
                    }
                }

                LlmResponse::Code { code, content } => {
                    nudge_count = 0;

                    // Record assistant message with the code
                    self.thread.add_message(ThreadMessage::assistant(
                        content.unwrap_or_else(|| format!("```python\n{code}\n```")),
                    ));

                    step.status = StepStatus::Executing;
                    step.tier = ExecutionTier::Scripting;

                    // Inject Step 0 orientation preamble on first code step
                    if self.thread.step_count == 0 {
                        let preamble =
                            crate::executor::scripting::build_orientation_preamble(&self.thread);
                        self.thread.add_message(ThreadMessage::system(preamble));
                    }

                    let exec_ctx = ThreadExecutionContext {
                        thread_id: self.thread.id,
                        thread_type: self.thread.thread_type,
                        project_id: self.thread.project_id,
                        user_id: self.user_id.clone(),
                        step_id: step.id,
                    };

                    // Execute via Monty
                    let code_result = crate::executor::scripting::execute_code(
                        &code,
                        &self.thread,
                        &self.llm,
                        &self.effects,
                        &self.leases,
                        &self.policy,
                        &exec_ctx,
                        &[],
                    )
                    .await?;

                    // Track recursive LLM token usage
                    self.thread.total_tokens_used += code_result.recursive_tokens.total();

                    // Record events
                    for event_kind in code_result.events {
                        self.thread.add_event(event_kind);
                    }

                    // Add action results as messages
                    for result in &code_result.action_results {
                        self.thread.add_message(ThreadMessage::action_result(
                            &result.call_id,
                            &result.action_name,
                            serde_json::to_string(&result.output).unwrap_or_default(),
                        ));
                    }

                    step.action_results = code_result.action_results;

                    // Compact output metadata (RLM pattern: truncated, not full)
                    let metadata = crate::executor::scripting::compact_output_metadata(
                        &code_result.stdout,
                        &code_result.return_value,
                    );
                    self.thread.add_message(ThreadMessage::system(metadata));

                    step.status = StepStatus::Completed;
                    step.completed_at = Some(chrono::Utc::now());
                    self.thread.add_event(EventKind::StepCompleted {
                        step_id: step.id,
                        tokens: step.tokens_used,
                    });
                    self.thread.step_count += 1;

                    // Check FINAL() termination
                    if let Some(answer) = code_result.final_answer {
                        self.thread.transition_to(
                            ThreadState::Completed,
                            Some("FINAL() called".into()),
                        )?;
                        return Ok(ThreadOutcome::Completed {
                            response: Some(answer),
                        });
                    }

                    // Check if approval is needed
                    if let Some(outcome) = code_result.need_approval {
                        self.thread
                            .transition_to(ThreadState::Waiting, Some("awaiting approval".into()))?;
                        return Ok(outcome);
                    }

                    // Track consecutive errors for budget enforcement
                    if code_result.had_error {
                        consecutive_errors += 1;
                    } else {
                        consecutive_errors = 0;
                    }
                }
            }

            // Check consecutive error threshold after each step
            if let Some(max_errors) = self.thread.config.max_consecutive_errors
                && consecutive_errors >= max_errors
            {
                warn!(
                    thread_id = %self.thread.id,
                    consecutive_errors,
                    max_errors,
                    "consecutive error threshold exceeded"
                );
                self.thread.transition_to(
                    ThreadState::Failed,
                    Some(format!(
                        "consecutive error threshold: {consecutive_errors} errors"
                    )),
                )?;
                return Ok(ThreadOutcome::Failed {
                    error: format!(
                        "Consecutive error threshold exceeded: {consecutive_errors} of {max_errors}"
                    ),
                });
            }
        }

        // Max iterations reached
        warn!(
            thread_id = %self.thread.id,
            max_iterations,
            "max iterations reached"
        );
        self.thread.transition_to(
            ThreadState::Completed,
            Some("max iterations reached".into()),
        )?;
        Ok(ThreadOutcome::MaxIterations)
    }

    /// Check for pending signals without blocking.
    fn check_signals(&mut self) -> SignalAction {
        match self.signal_rx.try_recv() {
            Ok(ThreadSignal::Stop) => SignalAction::Stop,
            Ok(ThreadSignal::InjectMessage(msg)) => SignalAction::Inject(msg),
            Ok(ThreadSignal::Suspend) => {
                // For now, treat suspend as stop. Phase 3 adds proper suspend/resume.
                SignalAction::Stop
            }
            Ok(ThreadSignal::Resume) | Ok(ThreadSignal::ChildCompleted { .. }) => {
                SignalAction::Continue
            }
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => SignalAction::Continue,
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                // Channel closed — the manager dropped our sender. Treat as stop.
                SignalAction::Stop
            }
        }
    }
}

enum SignalAction {
    Continue,
    Stop,
    Inject(ThreadMessage),
}

/// Extract a FINAL() answer from the LLM's text response.
///
/// Matches `FINAL(...)` anywhere in the text, handling:
/// - Single-line: `FINAL("the answer")`
/// - Multi-line: `FINAL("""\n...\n""")`
/// - With or without quotes
///
/// This is the regex fallback from the official RLM implementation
/// (`find_final_answer` in parsing.py) for when the model writes
/// FINAL() outside a code block.
fn extract_final_from_text(text: &str) -> Option<String> {
    // Find FINAL( — could be at start of line or after whitespace
    let marker = "FINAL(";
    let start = text.find(marker)?;
    let content_start = start + marker.len();

    // Extract everything after FINAL( up to the matching closing paren
    // Handle nested parens and triple-quoted strings
    let remaining = &text[content_start..];

    // Try triple-quoted string first: FINAL("""...""")
    if remaining.starts_with("\"\"\"") {
        let inner_start = 3;
        if let Some(end) = remaining[inner_start..].find("\"\"\"") {
            let answer = remaining[inner_start..inner_start + end].trim();
            if !answer.is_empty() {
                return Some(answer.to_string());
            }
        }
    }

    // Try single/double quoted: FINAL("...") or FINAL('...')
    if remaining.starts_with('"') || remaining.starts_with('\'') {
        let quote = remaining.as_bytes()[0] as char;
        if let Some(end) = remaining[1..].find(quote) {
            let answer = &remaining[1..1 + end];
            if !answer.is_empty() {
                return Some(answer.to_string());
            }
        }
    }

    // Unquoted: FINAL(some content here) — find matching close paren
    let mut depth = 1;
    for (i, ch) in remaining.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    let answer = remaining[..i].trim();
                    if !answer.is_empty() {
                        return Some(answer.to_string());
                    }
                    return None;
                }
            }
            _ => {}
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::capability::{ActionDef, CapabilityLease, EffectType};
    use crate::types::project::ProjectId;
    use crate::types::step::{ActionResult, TokenUsage};
    use crate::types::thread::{ThreadConfig, ThreadType};
    use crate::traits::llm::{LlmCallConfig, LlmOutput};

    use std::sync::Mutex;
    use std::time::Duration;

    // ── Mock LLM ────────────────────────────────────────────

    struct MockLlm {
        responses: Mutex<Vec<LlmOutput>>,
    }

    impl MockLlm {
        fn new(responses: Vec<LlmOutput>) -> Self {
            Self {
                responses: Mutex::new(responses),
            }
        }
    }

    #[async_trait::async_trait]
    impl LlmBackend for MockLlm {
        async fn complete(
            &self,
            _messages: &[ThreadMessage],
            _actions: &[ActionDef],
            _config: &LlmCallConfig,
        ) -> Result<LlmOutput, EngineError> {
            let mut responses = self.responses.lock().unwrap();
            if responses.is_empty() {
                Ok(LlmOutput {
                    response: LlmResponse::Text("(no more responses)".into()),
                    usage: TokenUsage::default(),
                })
            } else {
                Ok(responses.remove(0))
            }
        }

        fn model_name(&self) -> &str {
            "mock"
        }
    }

    // ── Mock EffectExecutor ─────────────────────────────────

    struct MockEffects {
        results: Mutex<Vec<Result<ActionResult, EngineError>>>,
        actions: Vec<ActionDef>,
    }

    impl MockEffects {
        fn new(actions: Vec<ActionDef>, results: Vec<Result<ActionResult, EngineError>>) -> Self {
            Self {
                results: Mutex::new(results),
                actions,
            }
        }
    }

    #[async_trait::async_trait]
    impl EffectExecutor for MockEffects {
        async fn execute_action(
            &self,
            _action_name: &str,
            _parameters: serde_json::Value,
            _lease: &CapabilityLease,
            _context: &ThreadExecutionContext,
        ) -> Result<ActionResult, EngineError> {
            let mut results = self.results.lock().unwrap();
            if results.is_empty() {
                Ok(ActionResult {
                    call_id: String::new(),
                    action_name: String::new(),
                    output: serde_json::json!({"result": "ok"}),
                    is_error: false,
                    duration: Duration::from_millis(1),
                })
            } else {
                results.remove(0)
            }
        }

        async fn available_actions(
            &self,
            _leases: &[CapabilityLease],
        ) -> Result<Vec<ActionDef>, EngineError> {
            Ok(self.actions.clone())
        }
    }

    // ── Helpers ─────────────────────────────────────────────

    fn text_response(text: &str) -> LlmOutput {
        LlmOutput {
            response: LlmResponse::Text(text.into()),
            usage: TokenUsage {
                input_tokens: 100,
                output_tokens: 50,
                ..Default::default()
            },
        }
    }

    fn action_response(action_name: &str, call_id: &str) -> LlmOutput {
        LlmOutput {
            response: LlmResponse::ActionCalls {
                calls: vec![crate::types::step::ActionCall {
                    id: call_id.into(),
                    action_name: action_name.into(),
                    parameters: serde_json::json!({}),
                }],
                content: None,
            },
            usage: TokenUsage {
                input_tokens: 100,
                output_tokens: 50,
                ..Default::default()
            },
        }
    }

    fn test_action() -> ActionDef {
        ActionDef {
            name: "test_tool".into(),
            description: "A test tool".into(),
            parameters_schema: serde_json::json!({"type": "object"}),
            effects: vec![EffectType::ReadLocal],
            requires_approval: false,
        }
    }

    async fn make_loop(
        llm_responses: Vec<LlmOutput>,
        effect_results: Vec<Result<ActionResult, EngineError>>,
        config: ThreadConfig,
    ) -> (ExecutionLoop, crate::runtime::messaging::SignalSender) {
        let project_id = ProjectId::new();
        let thread = Thread::new("test goal", ThreadType::Foreground, project_id, config);
        let tid = thread.id;

        let llm = Arc::new(MockLlm::new(llm_responses));
        let effects = Arc::new(MockEffects::new(vec![test_action()], effect_results));
        let leases = Arc::new(LeaseManager::new());
        let policy = Arc::new(PolicyEngine::new());

        // Grant a default lease
        leases
            .grant(tid, "test_cap", vec![], None, None)
            .await;

        let (tx, rx) = crate::runtime::messaging::signal_channel(16);

        let exec = ExecutionLoop::new(
            thread,
            llm,
            effects,
            leases,
            policy,
            rx,
            "test-user".into(),
        );
        (exec, tx)
    }

    // ── Tests ───────────────────────────────────────────────

    #[tokio::test]
    async fn text_response_completes() {
        let (mut exec, _tx) = make_loop(
            vec![text_response("Hello!")],
            vec![],
            ThreadConfig::default(),
        )
        .await;

        let outcome = exec.run().await.unwrap();
        assert!(matches!(outcome, ThreadOutcome::Completed { response: Some(r) } if r == "Hello!"));
        assert!(exec.thread.state.is_terminal() || exec.thread.state == ThreadState::Completed);
        assert_eq!(exec.thread.step_count, 1);
        assert!(exec.thread.total_tokens_used > 0);
    }

    #[tokio::test]
    async fn action_then_text() {
        let (mut exec, _tx) = make_loop(
            vec![
                action_response("test_tool", "call_1"),
                text_response("Done!"),
            ],
            vec![Ok(ActionResult {
                call_id: "call_1".into(),
                action_name: "test_tool".into(),
                output: serde_json::json!({"data": "result"}),
                is_error: false,
                duration: Duration::from_millis(5),
            })],
            ThreadConfig::default(),
        )
        .await;

        let outcome = exec.run().await.unwrap();
        assert!(matches!(outcome, ThreadOutcome::Completed { response: Some(r) } if r == "Done!"));
        assert_eq!(exec.thread.step_count, 2);
        // Should have: system(nudge not counted), assistant+actions, action_result, assistant
        assert!(exec.thread.messages.len() >= 3);
    }

    #[tokio::test]
    async fn max_iterations_reached() {
        // LLM always returns actions, so it never exits naturally
        let many_actions: Vec<LlmOutput> = (0..5)
            .map(|i| action_response("test_tool", &format!("call_{i}")))
            .collect();

        let many_results: Vec<Result<ActionResult, EngineError>> = (0..5)
            .map(|i| {
                Ok(ActionResult {
                    call_id: format!("call_{i}"),
                    action_name: "test_tool".into(),
                    output: serde_json::json!({"i": i}),
                    is_error: false,
                    duration: Duration::from_millis(1),
                })
            })
            .collect();

        let config = ThreadConfig {
            max_iterations: 3,
            ..ThreadConfig::default()
        };

        let (mut exec, _tx) = make_loop(many_actions, many_results, config).await;

        let outcome = exec.run().await.unwrap();
        // The last iteration forces text mode, and MockLlm returns action_response
        // which gets treated as the 3rd iteration, then on the 3rd iteration force_text
        // is set. But MockLlm ignores force_text. So we get MaxIterations after 3 iterations.
        // Actually, max_iterations=3, and force_text is set when iteration >= max-1 = 2,
        // so iteration 2 (0-indexed) has force_text. The MockLlm still returns action calls,
        // so we loop 3 times and exit.
        assert!(matches!(
            outcome,
            ThreadOutcome::MaxIterations | ThreadOutcome::Completed { .. }
        ));
        assert!(exec.thread.step_count <= 3);
    }

    #[tokio::test]
    async fn stop_signal_exits() {
        // LLM would loop forever, but we send a stop signal
        let many_actions: Vec<LlmOutput> = (0..100)
            .map(|i| action_response("test_tool", &format!("call_{i}")))
            .collect();

        let many_results: Vec<Result<ActionResult, EngineError>> = (0..100)
            .map(|i| {
                Ok(ActionResult {
                    call_id: format!("call_{i}"),
                    action_name: "test_tool".into(),
                    output: serde_json::json!({}),
                    is_error: false,
                    duration: Duration::from_millis(1),
                })
            })
            .collect();

        let (mut exec, tx) = make_loop(many_actions, many_results, ThreadConfig::default()).await;

        // Send stop before first iteration
        tx.send(ThreadSignal::Stop).await.unwrap();

        let outcome = exec.run().await.unwrap();
        assert!(matches!(outcome, ThreadOutcome::Stopped));
    }

    #[tokio::test]
    async fn inject_message_appears_in_context() {
        let (mut exec, tx) = make_loop(
            vec![text_response("Got your message")],
            vec![],
            ThreadConfig::default(),
        )
        .await;

        tx.send(ThreadSignal::InjectMessage(ThreadMessage::user(
            "injected!",
        )))
        .await
        .unwrap();

        let outcome = exec.run().await.unwrap();
        assert!(matches!(outcome, ThreadOutcome::Completed { .. }));
        assert!(exec
            .thread
            .messages
            .iter()
            .any(|m| m.content == "injected!"));
    }

    #[tokio::test]
    async fn tool_intent_nudge_injected() {
        let (mut exec, _tx) = make_loop(
            vec![
                text_response("Let me search for that"),
                text_response("The answer is 42"),
            ],
            vec![],
            ThreadConfig {
                enable_tool_intent_nudge: true,
                max_tool_intent_nudges: 2,
                ..ThreadConfig::default()
            },
        )
        .await;

        let outcome = exec.run().await.unwrap();
        assert!(
            matches!(outcome, ThreadOutcome::Completed { response: Some(r) } if r == "The answer is 42")
        );
        assert_eq!(exec.thread.step_count, 2);
        // Should have nudge system message
        assert!(exec
            .thread
            .messages
            .iter()
            .any(|m| m.content.contains("didn't make an action call")));
    }

    #[tokio::test]
    async fn events_are_recorded() {
        let (mut exec, _tx) = make_loop(
            vec![text_response("Hello!")],
            vec![],
            ThreadConfig::default(),
        )
        .await;

        exec.run().await.unwrap();

        let _event_kinds: Vec<String> = exec
            .thread
            .events
            .iter()
            .map(|e| format!("{:?}", std::mem::discriminant(&e.kind)))
            .collect();

        // Should have: StateChanged(Created->Running), StepStarted, MessageAdded,
        // StepCompleted, StateChanged(Running->Completed)
        assert!(exec.thread.events.len() >= 4);

        // Verify first event is state change to Running
        assert!(matches!(
            &exec.thread.events[0].kind,
            EventKind::StateChanged {
                from: ThreadState::Created,
                to: ThreadState::Running,
                ..
            }
        ));
    }

    // ── CodeAct / RLM tests ─────────────────────────────────

    fn code_response(code: &str) -> LlmOutput {
        LlmOutput {
            response: LlmResponse::Code {
                code: code.into(),
                content: Some(format!("```repl\n{code}\n```")),
            },
            usage: TokenUsage {
                input_tokens: 100,
                output_tokens: 80,
                ..Default::default()
            },
        }
    }

    #[tokio::test]
    async fn codeact_simple_final() {
        // LLM outputs Python code that calls FINAL()
        let (mut exec, _tx) = make_loop(
            vec![code_response("FINAL('The answer is 42')")],
            vec![],
            ThreadConfig::default(),
        )
        .await;

        let outcome = exec.run().await.unwrap();
        assert!(
            matches!(outcome, ThreadOutcome::Completed { response: Some(r) } if r == "The answer is 42")
        );
        assert_eq!(exec.thread.step_count, 1);
    }

    #[tokio::test]
    async fn codeact_tool_call_then_final() {
        // LLM outputs code that calls a tool, then uses the result
        let (mut exec, _tx) = make_loop(
            vec![
                code_response("result = test_tool()\nprint(result)\nFINAL('got result')"),
            ],
            vec![Ok(ActionResult {
                call_id: "code_call_1".into(),
                action_name: "test_tool".into(),
                output: serde_json::json!({"data": "hello from tool"}),
                is_error: false,
                duration: Duration::from_millis(5),
            })],
            ThreadConfig::default(),
        )
        .await;

        let outcome = exec.run().await.unwrap();
        assert!(
            matches!(outcome, ThreadOutcome::Completed { response: Some(r) } if r == "got result")
        );
        // Should have at least 1 action result recorded
        assert!(!exec.thread.messages.is_empty());
    }

    #[tokio::test]
    async fn codeact_pure_python_computation() {
        // LLM outputs pure Python with no tool calls — just computation + FINAL
        let (mut exec, _tx) = make_loop(
            vec![code_response(
                "numbers = [1, 2, 3, 4, 5]\ntotal = sum(numbers)\nFINAL(f'Sum is {total}')",
            )],
            vec![],
            ThreadConfig::default(),
        )
        .await;

        let outcome = exec.run().await.unwrap();
        assert!(
            matches!(outcome, ThreadOutcome::Completed { response: Some(r) } if r == "Sum is 15")
        );
    }

    #[tokio::test]
    async fn codeact_multi_step() {
        // First iteration: code runs but no FINAL — returns output
        // Second iteration: LLM sees output and calls FINAL
        let (mut exec, _tx) = make_loop(
            vec![
                code_response("x = 10 + 20\nprint(f'x = {x}')"),
                code_response("FINAL('done, x was 30')"),
            ],
            vec![],
            ThreadConfig::default(),
        )
        .await;

        let outcome = exec.run().await.unwrap();
        assert!(
            matches!(outcome, ThreadOutcome::Completed { response: Some(r) } if r == "done, x was 30")
        );
        assert_eq!(exec.thread.step_count, 2);
        // The output metadata from first step should be in messages
        assert!(exec.thread.messages.iter().any(|m| m.content.contains("x = 30")));
    }

    #[tokio::test]
    async fn codeact_error_recovery() {
        // First iteration: code has an error (NameError)
        // Second iteration: LLM sees the error and fixes it
        let (mut exec, _tx) = make_loop(
            vec![
                code_response("result = undefined_var + 1"),
                code_response("FINAL('recovered')"),
            ],
            vec![],
            ThreadConfig::default(),
        )
        .await;

        let outcome = exec.run().await.unwrap();
        assert!(matches!(outcome, ThreadOutcome::Completed { response: Some(r) } if r == "recovered"));
        assert_eq!(exec.thread.step_count, 2);
        // First step should have error in output metadata
        assert!(exec.thread.messages.iter().any(|m| {
            m.content.contains("NameError") || m.content.contains("Error")
        }));
    }

    #[tokio::test]
    async fn codeact_context_variables_available() {
        // Code accesses the `goal` and `context` variables injected by the engine
        let (mut exec, _tx) = make_loop(
            vec![code_response(
                "FINAL(f'Goal: {goal}, Messages: {len(context)}')",
            )],
            vec![],
            ThreadConfig::default(),
        )
        .await;

        let outcome = exec.run().await.unwrap();
        // Should have access to goal="test goal" and context (list of messages)
        match outcome {
            ThreadOutcome::Completed { response: Some(r) } => {
                assert!(r.contains("Goal: test goal"), "got: {r}");
                assert!(r.contains("Messages:"), "got: {r}");
            }
            other => panic!("expected Completed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn codeact_multiple_tool_calls_in_loop() {
        // Code calls a tool 3 times in a for loop
        let (mut exec, _tx) = make_loop(
            vec![code_response(
                "results = []\nfor i in range(3):\n    r = test_tool()\n    results.append(r)\nFINAL(f'Got {len(results)} results')",
            )],
            vec![
                Ok(ActionResult {
                    call_id: "code_call_1".into(),
                    action_name: "test_tool".into(),
                    output: serde_json::json!({"i": 0}),
                    is_error: false,
                    duration: Duration::from_millis(1),
                }),
                Ok(ActionResult {
                    call_id: "code_call_2".into(),
                    action_name: "test_tool".into(),
                    output: serde_json::json!({"i": 1}),
                    is_error: false,
                    duration: Duration::from_millis(1),
                }),
                Ok(ActionResult {
                    call_id: "code_call_3".into(),
                    action_name: "test_tool".into(),
                    output: serde_json::json!({"i": 2}),
                    is_error: false,
                    duration: Duration::from_millis(1),
                }),
            ],
            ThreadConfig::default(),
        )
        .await;

        let outcome = exec.run().await.unwrap();
        assert!(
            matches!(outcome, ThreadOutcome::Completed { response: Some(r) } if r == "Got 3 results")
        );
    }

    #[tokio::test]
    async fn codeact_llm_query_recursive() {
        // Code calls llm_query() — which calls the MockLlm recursively.
        // The MockLlm will return the next response in its queue for the sub-call.
        let (mut exec, _tx) = make_loop(
            vec![
                // First response: code that calls llm_query
                code_response("answer = llm_query('What is 2+2?')\nFINAL(f'Sub-agent said: {answer}')"),
                // This text response will be consumed by the llm_query sub-call
                // (MockLlm pops from the same queue)
            ],
            vec![],
            ThreadConfig::default(),
        )
        .await;

        let outcome = exec.run().await.unwrap();
        // llm_query will get "(no more responses)" since the queue only had
        // the code response. That's fine — it tests the plumbing.
        match outcome {
            ThreadOutcome::Completed { response: Some(r) } => {
                assert!(r.contains("Sub-agent said:"), "got: {r}");
            }
            other => panic!("expected Completed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn codeact_final_in_text_response() {
        // LLM outputs FINAL() as plain text (not in a code block)
        // This is the Hyperliquid case — model writes explanation + FINAL()
        let (mut exec, _tx) = make_loop(
            vec![text_response(
                "Based on my analysis, the answer is clear.\n\nFINAL(\"Revenue grows with volume\")",
            )],
            vec![],
            ThreadConfig::default(),
        )
        .await;

        let outcome = exec.run().await.unwrap();
        assert!(
            matches!(outcome, ThreadOutcome::Completed { response: Some(ref r) } if r == "Revenue grows with volume"),
            "got: {outcome:?}"
        );
    }

    #[tokio::test]
    async fn codeact_final_triple_quoted_in_text() {
        // FINAL with triple-quoted multi-line string in plain text
        let (mut exec, _tx) = make_loop(
            vec![text_response(
                "Here's the summary:\n\nFINAL(\"\"\"\nLine 1\nLine 2\nLine 3\n\"\"\")",
            )],
            vec![],
            ThreadConfig::default(),
        )
        .await;

        let outcome = exec.run().await.unwrap();
        match outcome {
            ThreadOutcome::Completed { response: Some(r) } => {
                assert!(r.contains("Line 1"), "got: {r}");
                assert!(r.contains("Line 3"), "got: {r}");
            }
            other => panic!("expected Completed, got {other:?}"),
        }
    }

    // ── extract_final_from_text unit tests ──────────────────

    #[test]
    fn final_double_quoted() {
        let text = "some text\nFINAL(\"the answer\")";
        assert_eq!(extract_final_from_text(text).unwrap(), "the answer");
    }

    #[test]
    fn final_single_quoted() {
        let text = "FINAL('hello world')";
        assert_eq!(extract_final_from_text(text).unwrap(), "hello world");
    }

    #[test]
    fn final_triple_quoted() {
        let text = "FINAL(\"\"\"\nmulti\nline\n\"\"\")";
        assert_eq!(extract_final_from_text(text).unwrap(), "multi\nline");
    }

    #[test]
    fn final_unquoted() {
        let text = "FINAL(42)";
        assert_eq!(extract_final_from_text(text).unwrap(), "42");
    }

    #[test]
    fn final_with_nested_parens() {
        let text = "FINAL(f'result is {len(items)}')";
        assert_eq!(
            extract_final_from_text(text).unwrap(),
            "f'result is {len(items)}'"
        );
    }

    #[test]
    fn no_final_returns_none() {
        assert!(extract_final_from_text("just regular text").is_none());
    }

    #[test]
    fn final_after_long_text() {
        let text = "A very long explanation...\n\n🔚 Final Thought\n\nFINAL(\"the conclusion\")";
        assert_eq!(extract_final_from_text(text).unwrap(), "the conclusion");
    }
}
