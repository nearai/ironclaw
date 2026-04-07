//! Core execution loop — the replacement for `run_agentic_loop()`.
//!
//! The `ExecutionLoop` owns a thread and drives it through LLM call →
//! action execution → result processing → repeat cycles. Unlike the
//! existing delegate pattern, the loop is self-contained: all behavior
//! differences between thread types are handled via capability leases
//! and policy, not delegate implementations.

use std::sync::Arc;

use tracing::debug;

use crate::capability::lease::LeaseManager;
use crate::capability::policy::PolicyEngine;
use crate::runtime::messaging::{SignalReceiver, ThreadOutcome};
use crate::traits::effect::EffectExecutor;
use crate::traits::llm::LlmBackend;
use crate::types::error::EngineError;
use crate::types::event::EventKind;
use crate::types::message::ThreadMessage;
use crate::types::step::Step;
use crate::types::thread::{Thread, ThreadState};

const RUNTIME_CHECKPOINT_METADATA_KEY: &str = "runtime_checkpoint";

/// Persisted state from a prior execution, used to resume threads.
/// The Python orchestrator manages loop counters internally; Rust only
/// needs the opaque `persisted_state` blob to hand back on resume.
#[derive(Default)]
struct RuntimeCheckpoint {
    persisted_state: serde_json::Value,
}

/// The core execution loop for a thread.
pub struct ExecutionLoop {
    pub thread: Thread,
    llm: Arc<dyn LlmBackend>,
    effects: Arc<dyn EffectExecutor>,
    leases: Arc<LeaseManager>,
    policy: Arc<PolicyEngine>,
    signal_rx: SignalReceiver,
    /// Stored for potential future use (e.g. user-scoped prompt overlays).
    _user_id: String,
    /// Optional capability registry for resolving capability-level policies.
    capabilities: Option<Arc<crate::capability::registry::CapabilityRegistry>>,
    /// Optional broadcast sender for live event streaming.
    event_tx: Option<tokio::sync::broadcast::Sender<crate::types::event::ThreadEvent>>,
    /// Optional retrieval engine for injecting prior knowledge into context.
    retrieval: Option<crate::memory::RetrievalEngine>,
    /// Optional Store for runtime prompt overlay loading and skill retrieval.
    store: Option<Arc<dyn crate::traits::store::Store>>,
    /// Runtime platform metadata for self-awareness in system prompts.
    platform_info: Option<crate::executor::prompt::PlatformInfo>,
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
            _user_id: user_id,
            capabilities: None,
            event_tx: None,
            retrieval: None,
            store: None,
            platform_info: None,
        }
    }

    /// Set the event broadcast sender for live status updates.
    pub fn with_event_tx(
        mut self,
        tx: tokio::sync::broadcast::Sender<crate::types::event::ThreadEvent>,
    ) -> Self {
        self.event_tx = Some(tx);
        self
    }

    /// Set the capability registry for resolving capability-level policies.
    pub fn with_capabilities(
        mut self,
        capabilities: Arc<crate::capability::registry::CapabilityRegistry>,
    ) -> Self {
        self.capabilities = Some(capabilities);
        self
    }

    /// Set the retrieval engine for injecting prior knowledge into context.
    pub fn with_retrieval(mut self, retrieval: crate::memory::RetrievalEngine) -> Self {
        self.retrieval = Some(retrieval);
        self
    }

    /// Set the Store for runtime prompt overlay loading and skill retrieval.
    pub fn with_store(mut self, store: Arc<dyn crate::traits::store::Store>) -> Self {
        self.store = Some(store);
        self
    }

    /// Set platform metadata for self-awareness in system prompts.
    pub fn with_platform_info(mut self, info: crate::executor::prompt::PlatformInfo) -> Self {
        self.platform_info = Some(info);
        self
    }

    /// Add an event to the thread and broadcast it for live status updates.
    fn emit_event(&mut self, kind: EventKind) {
        let event = crate::types::event::ThreadEvent::new(self.thread.id, kind);
        if let Some(ref tx) = self.event_tx {
            let _ = tx.send(event.clone());
        }
        self.thread.events.push(event);
        self.thread.updated_at = chrono::Utc::now();
    }

    fn load_runtime_checkpoint(&self) -> RuntimeCheckpoint {
        let persisted_state = self
            .thread
            .metadata
            .get(RUNTIME_CHECKPOINT_METADATA_KEY)
            .and_then(|value| value.get("persisted_state"))
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));

        RuntimeCheckpoint { persisted_state }
    }

    fn clear_runtime_checkpoint(&mut self) {
        if let Some(metadata) = self.thread.metadata.as_object_mut() {
            metadata.remove(RUNTIME_CHECKPOINT_METADATA_KEY);
        }
        self.thread.updated_at = chrono::Utc::now();
    }

    async fn persist_runtime_state(
        &self,
        step: Option<&Step>,
        persisted_event_count: &mut usize,
    ) -> Result<(), EngineError> {
        let Some(store) = self.store.as_ref() else {
            return Ok(());
        };

        // All three store writes are independent — run them in parallel.
        let step_fut = async {
            if let Some(step) = step {
                store.save_step(step).await
            } else {
                Ok(())
            }
        };

        let new_event_count = self.thread.events.len();
        let events_fut = async {
            if *persisted_event_count < new_event_count {
                store
                    .append_events(&self.thread.events[*persisted_event_count..])
                    .await
            } else {
                Ok(())
            }
        };

        let thread_fut = store.save_thread(&self.thread);

        let (step_res, events_res, thread_res) = tokio::join!(step_fut, events_fut, thread_fut);
        step_res?;
        events_res?;
        thread_res?;

        *persisted_event_count = new_event_count;
        Ok(())
    }

    /// Run the execution loop to completion.
    pub async fn run(&mut self) -> Result<ThreadOutcome, EngineError> {
        let mut persisted_event_count = self.thread.events.len();
        let checkpoint = self.load_runtime_checkpoint();

        // Transition to Running if this is a fresh start or restart from a resumable state.
        if self.thread.state != ThreadState::Running {
            self.thread.transition_to(ThreadState::Running, None)?;
        }

        // Pre-fetch shared memory docs once — used by both prompt overlay and
        // orchestrator loading, avoiding a duplicate Store query.
        let system_docs = if let Some(store) = self.store.as_ref() {
            match store.list_shared_memory_docs(self.thread.project_id).await {
                Ok(docs) => docs,
                Err(e) => {
                    debug!("failed to load shared docs for orchestrator: {e}");
                    Vec::new()
                }
            }
        } else {
            Vec::new()
        };

        // Inject CodeAct/RLM system prompt if none exists
        if !self
            .thread
            .messages
            .iter()
            .any(|m| m.role == crate::types::message::MessageRole::System)
        {
            // Fetch active leases (needed for action list)
            let active_leases = self.leases.active_for_thread(self.thread.id).await;
            let actions = match self.effects.available_actions(&active_leases).await {
                Ok(a) => a,
                Err(e) => {
                    debug!(thread_id = %self.thread.id, "failed to load actions for system prompt: {e}");
                    Vec::new()
                }
            };
            // Build prompt using pre-fetched docs (no extra Store query)
            let system_prompt = crate::executor::prompt::build_codeact_system_prompt_with_docs(
                &actions,
                &system_docs,
                self.platform_info.as_ref(),
            );

            // Skill selection and injection happens in the Python orchestrator
            // via __list_skills__() host function — not here in Rust.

            self.thread
                .messages
                .insert(0, ThreadMessage::system(system_prompt));
        }
        self.persist_runtime_state(None, &mut persisted_event_count)
            .await?;

        // Load versioned Python orchestrator using pre-fetched docs.
        // Self-modification is disabled by default — only the compiled-in v0
        // runs unless explicitly opted in via ORCHESTRATOR_SELF_MODIFY=true.
        let allow_self_modify = std::env::var("ORCHESTRATOR_SELF_MODIFY")
            .map(|v| v == "true" || v == "1")
            .unwrap_or(false);
        let (orchestrator_code, orchestrator_version) =
            crate::executor::orchestrator::load_orchestrator_from_docs(
                &system_docs,
                allow_self_modify,
            );

        debug!(
            thread_id = %self.thread.id,
            orchestrator_version,
            "running Python orchestrator"
        );

        // Store version in thread metadata for rollback tracking
        if let Some(metadata) = self.thread.metadata.as_object_mut() {
            metadata.insert(
                "orchestrator_version".into(),
                serde_json::json!(orchestrator_version),
            );
        }

        // Execute the Python orchestrator with host function dispatch
        let result = crate::executor::orchestrator::execute_orchestrator(
            &orchestrator_code,
            &mut self.thread,
            &self.llm,
            &self.effects,
            &self.leases,
            &self.policy,
            &mut self.signal_rx,
            self.event_tx.as_ref(),
            self.retrieval.as_ref(),
            self.store.as_ref(),
            &checkpoint.persisted_state,
        )
        .await;

        // Post-cleanup: persist final state, track failures for auto-rollback
        match result {
            Ok(orch_result) => {
                // Reset failure counter on success
                if let Some(store) = self.store.as_ref() {
                    crate::executor::orchestrator::reset_orchestrator_failures(
                        store,
                        self.thread.project_id,
                    )
                    .await;
                }
                let _ = &orch_result.tokens_used;

                self.clear_runtime_checkpoint();
                self.persist_runtime_state(None, &mut persisted_event_count)
                    .await?;
                Ok(orch_result.outcome)
            }
            Err(e) => {
                debug!(
                    thread_id = %self.thread.id,
                    error = %e,
                    orchestrator_version,
                    "orchestrator execution failed"
                );

                // Record failure for auto-rollback tracking
                if let Some(store) = self.store.as_ref() {
                    crate::executor::orchestrator::record_orchestrator_failure(
                        store,
                        self.thread.project_id,
                        orchestrator_version,
                    )
                    .await;

                    // Emit rollback event if this version will be skipped next time
                    // (failure count was just incremented, so check >= threshold - 1)
                    if orchestrator_version > 0 {
                        self.emit_event(EventKind::OrchestratorRollback {
                            from_version: orchestrator_version,
                            to_version: orchestrator_version.saturating_sub(1),
                            reason: format!("execution failed: {e}"),
                        });
                    }
                }

                // Transition to failed if not already in a terminal state
                if self.thread.state != ThreadState::Completed
                    && self.thread.state != ThreadState::Failed
                    && self.thread.state != ThreadState::Done
                {
                    let _ = self.thread.transition_to(
                        ThreadState::Failed,
                        Some(format!("orchestrator error: {e}")),
                    );
                }
                self.clear_runtime_checkpoint();
                self.persist_runtime_state(None, &mut persisted_event_count)
                    .await?;
                Ok(ThreadOutcome::Failed {
                    error: format!("Orchestrator error: {e}"),
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    /// Extract a FINAL() answer from the LLM's text response.
    ///
    /// Matches `FINAL(...)` anywhere in the text, handling:
    /// - Single-line: `FINAL("the answer")`
    /// - Multi-line: `FINAL("""\n...\n""")`
    /// - With or without quotes
    fn extract_final_from_text(text: &str) -> Option<String> {
        let marker = "FINAL(";
        let start = text.find(marker)?;
        let content_start = start + marker.len();
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
                        let answer = remaining[..i].trim(); // safety: i is from char_indices(), always a valid boundary
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
    use super::*;
    use crate::runtime::messaging::ThreadSignal;
    use crate::traits::effect::ThreadExecutionContext;
    use crate::traits::llm::{LlmCallConfig, LlmOutput};
    use crate::types::capability::{ActionDef, CapabilityLease, EffectType, GrantedActions};
    use crate::types::project::ProjectId;
    use crate::types::step::LlmResponse;
    use crate::types::step::{ActionResult, TokenUsage};
    use crate::types::thread::{ThreadConfig, ThreadType};

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
        let thread = Thread::new(
            "test goal",
            ThreadType::Foreground,
            project_id,
            "test-user",
            config,
        );
        let tid = thread.id;

        let llm = Arc::new(MockLlm::new(llm_responses));
        let effects = Arc::new(MockEffects::new(vec![test_action()], effect_results));
        let leases = Arc::new(LeaseManager::new());
        let policy = Arc::new(PolicyEngine::new());

        // Grant a default lease
        leases
            .grant(tid, "test_cap", GrantedActions::All, None, None)
            .await
            .unwrap();

        let (tx, rx) = crate::runtime::messaging::signal_channel(16);

        let exec = ExecutionLoop::new(thread, llm, effects, leases, policy, rx, "test-user".into());
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
        // The orchestrator syncs intermediate messages to internal_messages;
        // the visible transcript (messages) gets only the final response.
        let all_msgs = exec.thread.messages.len() + exec.thread.internal_messages.len();
        assert!(all_msgs >= 3);
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
        assert!(
            exec.thread
                .messages
                .iter()
                .any(|m| m.content == "injected!")
        );
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
        // Nudge message lives in internal_messages (orchestrator working state)
        let all_messages: Vec<&crate::types::message::ThreadMessage> = exec
            .thread
            .messages
            .iter()
            .chain(exec.thread.internal_messages.iter())
            .collect();
        assert!(
            all_messages
                .iter()
                .any(|m| m.content.contains("did not include any tool calls"))
        );
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
            vec![code_response(
                "result = test_tool()\nprint(result)\nFINAL('got result')",
            )],
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
        // The output metadata from first step lives in internal_messages
        let all_messages: Vec<&crate::types::message::ThreadMessage> = exec
            .thread
            .messages
            .iter()
            .chain(exec.thread.internal_messages.iter())
            .collect();
        assert!(all_messages.iter().any(|m| m.content.contains("x = 30")));
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
        assert!(
            matches!(outcome, ThreadOutcome::Completed { response: Some(r) } if r == "recovered")
        );
        assert_eq!(exec.thread.step_count, 2);
        // First step should have error in output metadata
        assert!(
            exec.thread
                .messages
                .iter()
                .any(|m| { m.content.contains("NameError") || m.content.contains("Error") })
        );
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
                code_response(
                    "answer = llm_query('What is 2+2?')\nFINAL(f'Sub-agent said: {answer}')",
                ),
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

    // ── call_id propagation through orchestrator pipeline ────
    //
    // These tests verify the end-to-end flow: LLM returns ActionCalls with
    // call_ids → orchestrator executes them → ActionResult messages on the
    // thread have correct call_ids (not empty). This catches the class of
    // bugs that caused OpenAI/Codex HTTP 400 rejections.

    #[tokio::test]
    async fn action_result_messages_have_correct_call_id() {
        // LLM returns a tool call, then a text response
        let (mut exec, _tx) = make_loop(
            vec![
                action_response("test_tool", "call_xK9mZq123"),
                text_response("Done!"),
            ],
            vec![Ok(ActionResult {
                call_id: String::new(), // EffectExecutor returns empty
                action_name: "test_tool".into(),
                output: serde_json::json!({"data": "result"}),
                is_error: false,
                duration: Duration::from_millis(5),
            })],
            ThreadConfig::default(),
        )
        .await;

        exec.run().await.unwrap();

        // Find the ActionResult message in the internal orchestrator transcript
        let action_results: Vec<_> = exec
            .thread
            .internal_messages
            .iter()
            .filter(|m| m.role == crate::types::message::MessageRole::ActionResult)
            .collect();

        assert!(
            !action_results.is_empty(),
            "thread should have at least one internal ActionResult message"
        );

        for msg in &action_results {
            let call_id = msg.action_call_id.as_deref().unwrap_or("");
            assert!(
                !call_id.is_empty(),
                "ActionResult message must have non-empty call_id, got empty for tool '{}'",
                msg.action_name.as_deref().unwrap_or("?")
            );
        }
    }

    /// Verify that the ActionExecuted event carries the call_id from the LLM.
    #[tokio::test]
    async fn action_executed_events_carry_call_id() {
        let (mut exec, _tx) = make_loop(
            vec![
                action_response("test_tool", "call_evt_id_42"),
                text_response("ok"),
            ],
            vec![Ok(ActionResult {
                call_id: String::new(),
                action_name: "test_tool".into(),
                output: serde_json::json!({}),
                is_error: false,
                duration: Duration::from_millis(1),
            })],
            ThreadConfig::default(),
        )
        .await;

        exec.run().await.unwrap();

        let exec_events: Vec<_> = exec
            .thread
            .events
            .iter()
            .filter_map(|e| match &e.kind {
                EventKind::ActionExecuted { call_id, .. } => Some(call_id.clone()),
                _ => None,
            })
            .collect();

        assert!(!exec_events.is_empty(), "should have ActionExecuted events");
        for call_id in &exec_events {
            assert!(
                !call_id.is_empty(),
                "ActionExecuted event must have non-empty call_id"
            );
        }
    }

    /// When a tool call fails (no lease), the internal ActionResult message and
    /// ActionFailed event must still carry the original call_id.
    #[tokio::test]
    async fn failed_action_preserves_call_id_in_message_and_event() {
        let project_id = ProjectId::new();
        let thread = Thread::new(
            "test",
            ThreadType::Foreground,
            project_id,
            "test-user",
            ThreadConfig::default(),
        );
        let tid = thread.id;

        // Create a tool that requires a separate capability
        let missing_action = ActionDef {
            name: "restricted_tool".into(),
            description: "A tool with no lease".into(),
            parameters_schema: serde_json::json!({"type": "object"}),
            effects: vec![EffectType::WriteExternal],
            requires_approval: false,
        };

        let llm = Arc::new(MockLlm::new(vec![
            // LLM calls a tool the thread has no lease for
            LlmOutput {
                response: LlmResponse::ActionCalls {
                    calls: vec![crate::types::step::ActionCall {
                        id: "call_nolease_xyz".into(),
                        action_name: "restricted_tool".into(),
                        parameters: serde_json::json!({}),
                    }],
                    content: None,
                },
                usage: TokenUsage::default(),
            },
            text_response("I couldn't access that tool"),
        ]));
        let effects = Arc::new(MockEffects::new(vec![missing_action], vec![]));
        let leases = Arc::new(LeaseManager::new());
        let policy = Arc::new(PolicyEngine::new());

        // Grant a lease that does NOT cover "restricted_tool"
        leases
            .grant(tid, "basic_cap", GrantedActions::All, None, None)
            .await
            .unwrap();

        let (_tx, rx) = crate::runtime::messaging::signal_channel(16);
        let mut exec =
            ExecutionLoop::new(thread, llm, effects, leases, policy, rx, "test-user".into());

        exec.run().await.unwrap();

        // Check internal ActionResult messages
        let action_results: Vec<_> = exec
            .thread
            .internal_messages
            .iter()
            .filter(|m| m.role == crate::types::message::MessageRole::ActionResult)
            .collect();

        for msg in &action_results {
            let call_id = msg.action_call_id.as_deref().unwrap_or("");
            assert!(
                !call_id.is_empty(),
                "even failed ActionResult must have call_id"
            );
        }

        // Check ActionFailed events
        let fail_events: Vec<_> = exec
            .thread
            .events
            .iter()
            .filter_map(|e| match &e.kind {
                EventKind::ActionFailed {
                    call_id,
                    action_name,
                    ..
                } => Some((call_id.clone(), action_name.clone())),
                _ => None,
            })
            .collect();

        for (call_id, _name) in &fail_events {
            assert!(!call_id.is_empty(), "ActionFailed event must have call_id");
        }
    }

    /// Verify the trace analyzer does NOT flag any issues on a clean
    /// action execution (no empty call_ids).
    #[tokio::test]
    async fn trace_analysis_clean_after_successful_tool_use() {
        let (mut exec, _tx) = make_loop(
            vec![
                action_response("test_tool", "call_clean_id"),
                text_response("All done"),
            ],
            vec![Ok(ActionResult {
                call_id: String::new(),
                action_name: "test_tool".into(),
                output: serde_json::json!({"status": "ok"}),
                is_error: false,
                duration: Duration::from_millis(3),
            })],
            ThreadConfig::default(),
        )
        .await;

        exec.run().await.unwrap();

        let trace = crate::executor::trace::build_trace(&exec.thread);
        let empty_id_issues: Vec<_> = trace
            .issues
            .iter()
            .filter(|i| i.category == "empty_call_id")
            .collect();

        assert!(
            empty_id_issues.is_empty(),
            "clean execution should have no empty_call_id issues, got: {empty_id_issues:?}"
        );
    }
}
