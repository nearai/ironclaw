//! Thread manager — top-level orchestrator for thread lifecycle.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::RwLock;
use tracing::{debug, error};

use crate::capability::lease::LeaseManager;
use crate::capability::planner::LeasePlanner;
use crate::capability::policy::PolicyEngine;
use crate::capability::registry::CapabilityRegistry;
use crate::executor::ExecutionLoop;
use crate::runtime::messaging::{self, SignalSender, ThreadOutcome, ThreadSignal};
use crate::runtime::tree::ThreadTree;
use crate::traits::effect::EffectExecutor;
use crate::traits::llm::LlmBackend;
use crate::traits::store::Store;
use crate::types::error::EngineError;
use crate::types::message::ThreadMessage;
use crate::types::project::ProjectId;
use crate::types::thread::{Thread, ThreadConfig, ThreadId, ThreadState, ThreadType};

/// Handle to a running thread for checking results.
struct RunningThread {
    signal_tx: SignalSender,
    handle: tokio::task::JoinHandle<Result<ThreadOutcome, EngineError>>,
}

/// Top-level orchestrator for thread lifecycle.
///
/// Manages thread spawning, supervision, signaling, and tree relationships.
pub struct ThreadManager {
    llm: Arc<dyn LlmBackend>,
    effects: Arc<dyn EffectExecutor>,
    store: Arc<dyn Store>,
    pub capabilities: Arc<CapabilityRegistry>,
    pub leases: Arc<LeaseManager>,
    pub policy: Arc<PolicyEngine>,
    lease_planner: LeasePlanner,
    tree: RwLock<ThreadTree>,
    running: Arc<RwLock<HashMap<ThreadId, RunningThread>>>,
    completed: Arc<RwLock<HashMap<ThreadId, ThreadOutcome>>>,
    /// Broadcast channel for thread events (for live status updates).
    event_tx: tokio::sync::broadcast::Sender<crate::types::event::ThreadEvent>,
}

impl ThreadManager {
    pub fn new(
        llm: Arc<dyn LlmBackend>,
        effects: Arc<dyn EffectExecutor>,
        store: Arc<dyn Store>,
        capabilities: Arc<CapabilityRegistry>,
        leases: Arc<LeaseManager>,
        policy: Arc<PolicyEngine>,
    ) -> Self {
        let (event_tx, _) = tokio::sync::broadcast::channel(256);
        Self {
            llm,
            effects,
            store,
            capabilities,
            leases,
            policy,
            lease_planner: LeasePlanner::new(),
            tree: RwLock::new(ThreadTree::new()),
            running: Arc::new(RwLock::new(HashMap::new())),
            completed: Arc::new(RwLock::new(HashMap::new())),
            event_tx,
        }
    }

    /// Subscribe to thread events for live status updates.
    pub fn subscribe_events(
        &self,
    ) -> tokio::sync::broadcast::Receiver<crate::types::event::ThreadEvent> {
        self.event_tx.subscribe()
    }

    /// Spawn a new thread and start executing it.
    ///
    /// Grants default capability leases for all registered capabilities.
    /// Returns the thread ID immediately; the thread runs in a background task.
    ///
    /// `initial_messages` provides conversation history from prior threads
    /// (for context continuity across turns in the same conversation).
    pub async fn spawn_thread(
        &self,
        goal: impl Into<String>,
        thread_type: ThreadType,
        project_id: ProjectId,
        config: ThreadConfig,
        parent_id: Option<ThreadId>,
        user_id: impl Into<String>,
    ) -> Result<ThreadId, EngineError> {
        self.spawn_thread_with_history(
            goal,
            thread_type,
            project_id,
            config,
            parent_id,
            user_id,
            Vec::new(),
        )
        .await
    }

    /// Spawn a thread with initial conversation history.
    #[allow(clippy::too_many_arguments)]
    pub async fn spawn_thread_with_history(
        &self,
        goal: impl Into<String>,
        thread_type: ThreadType,
        project_id: ProjectId,
        config: ThreadConfig,
        parent_id: Option<ThreadId>,
        user_id: impl Into<String>,
        initial_messages: Vec<crate::types::message::ThreadMessage>,
    ) -> Result<ThreadId, EngineError> {
        let mut thread = Thread::new(goal, thread_type, project_id, config);
        if let Some(pid) = parent_id {
            thread = thread.with_parent(pid);
        }
        let thread_id = thread.id;
        let user_id = user_id.into();
        if let Some(metadata) = thread.metadata.as_object_mut() {
            metadata.insert("user_id".into(), serde_json::Value::String(user_id.clone()));
        }

        // Register in tree
        if let Some(pid) = parent_id {
            self.tree.write().await.add_child(pid, thread_id);
        }

        // Grant explicit capability leases based on thread type.
        for grant in self
            .lease_planner
            .plan_for_thread(thread_type, &self.capabilities)
        {
            let lease = self
                .leases
                .grant(
                    thread_id,
                    grant.capability_name,
                    grant.granted_actions,
                    None,
                    None,
                )
                .await;
            self.store.save_lease(&lease).await?;
            thread.capability_leases.push(lease.id);
        }

        // Add conversation history from prior threads (for context continuity)
        for msg in initial_messages {
            thread.messages.push(msg);
        }

        // Add the goal as the current user message so the LLM has context
        thread.add_message(crate::types::message::ThreadMessage::user(&thread.goal));

        // Persist
        self.store.save_thread(&thread).await?;

        self.start_thread(thread, user_id, false).await
    }

    /// Resume a persisted waiting or suspended thread.
    pub async fn resume_thread(
        &self,
        thread_id: ThreadId,
        user_id: impl Into<String>,
        injected_message: Option<ThreadMessage>,
        approval_event: Option<(String, bool)>,
    ) -> Result<(), EngineError> {
        if self.is_running(thread_id).await {
            return Err(EngineError::Thread(
                crate::types::error::ThreadError::AlreadyRunning(thread_id),
            ));
        }

        let mut thread = self
            .store
            .load_thread(thread_id)
            .await?
            .ok_or(EngineError::ThreadNotFound(thread_id))?;

        if !matches!(
            thread.state,
            crate::types::thread::ThreadState::Waiting
                | crate::types::thread::ThreadState::Suspended
        ) {
            return Err(EngineError::Store {
                reason: format!(
                    "thread {thread_id} is not resumable from {:?}",
                    thread.state
                ),
            });
        }

        if let Some((call_id, approved)) = approval_event {
            let event = crate::types::event::ThreadEvent::new(
                thread_id,
                crate::types::event::EventKind::ApprovalReceived { call_id, approved },
            );
            let _ = self.event_tx.send(event.clone());
            thread.events.push(event);
            thread.updated_at = chrono::Utc::now();
        }

        if let Some(message) = injected_message {
            thread.add_message(message);
        }

        self.store.save_thread(&thread).await?;
        self.start_thread(thread, user_id.into(), true).await?;
        Ok(())
    }

    async fn start_thread(
        &self,
        thread: Thread,
        user_id: String,
        is_resume: bool,
    ) -> Result<ThreadId, EngineError> {
        let thread_id = thread.id;

        // Create signal channel
        let (tx, rx) = messaging::signal_channel(32);

        // Build execution loop
        let llm = Arc::clone(&self.llm);
        let effects = Arc::clone(&self.effects);
        let leases = Arc::clone(&self.leases);
        let policy = Arc::clone(&self.policy);

        let store_for_retrieval = Arc::clone(&self.store);
        let retrieval = crate::memory::RetrievalEngine::new(store_for_retrieval);

        let exec_loop = ExecutionLoop::new(thread, llm, effects, leases, policy, rx, user_id)
            .with_capabilities(Arc::clone(&self.capabilities))
            .with_event_tx(self.event_tx.clone())
            .with_retrieval(retrieval)
            .with_store(Arc::clone(&self.store));

        // Spawn background task
        let store_for_task = Arc::clone(&self.store);
        let llm_for_reflection = Arc::clone(&self.llm);
        let caps_for_reflection = Arc::clone(&self.capabilities);
        let event_tx = self.event_tx.clone();
        let running = Arc::clone(&self.running);
        let completed = Arc::clone(&self.completed);
        let handle = tokio::spawn(async move {
            let mut exec = exec_loop;
            let result = exec.run().await;
            debug!(thread_id = %thread_id, "thread execution finished");

            // Helper to emit events on both the thread and broadcast channel
            let emit = |thread: &mut crate::types::thread::Thread,
                        kind: crate::types::event::EventKind| {
                let event = crate::types::event::ThreadEvent::new(thread.id, kind);
                let _ = event_tx.send(event.clone());
                thread.events.push(event);
                thread.updated_at = chrono::Utc::now();
            };

            // Run retrospective trace analysis (non-LLM, always runs)
            let mut trace = crate::executor::trace::build_trace(&exec.thread);
            if !trace.issues.is_empty() {
                crate::executor::trace::log_trace_summary(&trace);
            }

            // Run LLM reflection if enabled and thread completed
            if exec.thread.config.enable_reflection
                && exec.thread.state == crate::types::thread::ThreadState::Completed
            {
                // Transition: Completed → Reflecting
                if let Err(e) = exec.thread.transition_to(
                    crate::types::thread::ThreadState::Reflecting,
                    Some("starting reflection".into()),
                ) {
                    tracing::warn!(thread_id = %thread_id, "failed to transition to Reflecting: {e}");
                } else {
                    emit(
                        &mut exec.thread,
                        crate::types::event::EventKind::ReflectionStarted,
                    );

                    match crate::reflection::reflect(
                        &exec.thread,
                        &llm_for_reflection,
                        &store_for_task,
                        &caps_for_reflection,
                    )
                    .await
                    {
                        Ok(reflection) => {
                            let doc_types: Vec<String> = reflection
                                .docs
                                .iter()
                                .map(|d| format!("{:?}", d.doc_type))
                                .collect();

                            emit(
                                &mut exec.thread,
                                crate::types::event::EventKind::ReflectionComplete {
                                    docs_produced: reflection.docs.len(),
                                    doc_types,
                                    tokens_used: reflection.tokens_used.total(),
                                },
                            );

                            // Attach reflection results to the trace
                            crate::executor::trace::attach_reflection(&mut trace, &reflection);

                            for doc in &reflection.docs {
                                if let Err(e) = store_for_task.save_memory_doc(doc).await {
                                    tracing::warn!(
                                        thread_id = %thread_id,
                                        doc_title = %doc.title,
                                        "failed to save reflection doc: {e}"
                                    );
                                }
                            }
                        }
                        Err(e) => {
                            emit(
                                &mut exec.thread,
                                crate::types::event::EventKind::ReflectionFailed {
                                    error: e.to_string(),
                                },
                            );
                        }
                    }

                    // Transition: Reflecting → Done
                    if let Err(e) = exec.thread.transition_to(
                        crate::types::thread::ThreadState::Done,
                        Some("reflection finished".into()),
                    ) {
                        tracing::warn!(
                            thread_id = %thread_id,
                            "failed to transition to Done after reflection: {e}"
                        );
                    }
                }
            }

            // Write trace file if enabled (after reflection, so it's included)
            if crate::executor::trace::is_trace_enabled() {
                crate::executor::trace::log_trace_summary(&trace);
                crate::executor::trace::write_trace(&trace);
            }

            if let Err(e) = store_for_task.append_events(&exec.thread.events).await {
                tracing::warn!(
                    thread_id = %thread_id,
                    "failed to persist thread events: {e}"
                );
            }

            // Save final thread state to store
            if let Err(e) = store_for_task.save_thread(&exec.thread).await {
                tracing::warn!(
                    thread_id = %thread_id,
                    "failed to save final thread state: {e}"
                );
            }

            let outcome = match result {
                Ok(outcome) => outcome,
                Err(error) => ThreadOutcome::Failed {
                    error: error.to_string(),
                },
            };
            completed.write().await.insert(thread_id, outcome.clone());
            running.write().await.remove(&thread_id);
            Ok(outcome)
        });

        self.running.write().await.insert(
            thread_id,
            RunningThread {
                signal_tx: tx,
                handle,
            },
        );

        if is_resume {
            debug!(thread_id = %thread_id, "resumed thread");
        }

        Ok(thread_id)
    }

    /// Send a stop signal to a running thread.
    pub async fn stop_thread(&self, thread_id: ThreadId) -> Result<(), EngineError> {
        let running = self.running.read().await;
        if let Some(rt) = running.get(&thread_id) {
            let _ = rt.signal_tx.send(ThreadSignal::Stop).await;
            Ok(())
        } else {
            Err(EngineError::ThreadNotFound(thread_id))
        }
    }

    /// Inject a user message into a running thread.
    pub async fn inject_message(
        &self,
        thread_id: ThreadId,
        message: ThreadMessage,
    ) -> Result<(), EngineError> {
        let running = self.running.read().await;
        if let Some(rt) = running.get(&thread_id) {
            let _ = rt
                .signal_tx
                .send(ThreadSignal::InjectMessage(message))
                .await;
            Ok(())
        } else {
            Err(EngineError::ThreadNotFound(thread_id))
        }
    }

    /// Check if a thread is still running.
    pub async fn is_running(&self, thread_id: ThreadId) -> bool {
        let running = self.running.read().await;
        running
            .get(&thread_id)
            .is_some_and(|rt| !rt.handle.is_finished())
    }

    /// Wait for a thread to finish and return its outcome.
    /// Removes the thread from the running set.
    pub async fn join_thread(&self, thread_id: ThreadId) -> Result<ThreadOutcome, EngineError> {
        if let Some(outcome) = self.completed.write().await.remove(&thread_id) {
            return Ok(outcome);
        }

        let rt = {
            let mut running = self.running.write().await;
            running.remove(&thread_id)
        };

        match rt {
            Some(rt) => match rt.handle.await {
                Ok(result) => result,
                Err(e) => {
                    error!(thread_id = %thread_id, "thread task panicked: {e}");
                    Ok(ThreadOutcome::Failed {
                        error: format!("thread task panicked: {e}"),
                    })
                }
            },
            None => Err(EngineError::ThreadNotFound(thread_id)),
        }
    }

    /// Get children of a thread.
    pub async fn children_of(&self, thread_id: ThreadId) -> Vec<ThreadId> {
        let tree = self.tree.read().await;
        tree.children_of(thread_id).to_vec()
    }

    /// Get the parent of a thread.
    pub async fn parent_of(&self, thread_id: ThreadId) -> Option<ThreadId> {
        let tree = self.tree.read().await;
        tree.parent_of(thread_id)
    }

    /// Clean up finished threads from the running set.
    pub async fn cleanup_finished(&self) -> Vec<ThreadId> {
        let mut running = self.running.write().await;
        let finished: Vec<ThreadId> = running
            .iter()
            .filter(|(_, rt)| rt.handle.is_finished())
            .map(|(id, _)| *id)
            .collect();
        for id in &finished {
            running.remove(id);
        }
        finished
    }

    /// Automatically resume checkpointed non-foreground threads.
    pub async fn resume_background_threads(
        &self,
        project_id: ProjectId,
    ) -> Result<Vec<ThreadId>, EngineError> {
        let threads = self.store.list_threads(project_id).await?;
        let mut resumed = Vec::new();

        for thread in threads {
            if thread.state != ThreadState::Suspended {
                continue;
            }
            if thread.thread_type != ThreadType::Research {
                continue;
            }
            if thread.metadata.get("runtime_checkpoint").is_none() {
                continue;
            }
            let Some(user_id) = thread
                .metadata
                .get("user_id")
                .and_then(|value| value.as_str())
                .filter(|user_id| !user_id.is_empty())
            else {
                continue;
            };

            self.resume_thread(thread.id, user_id.to_string(), None, None)
                .await?;
            resumed.push(thread.id);
        }

        Ok(resumed)
    }

    /// Reconcile persisted non-terminal threads after process startup.
    ///
    /// The current engine does not support mid-thread replay/resume, so any
    /// thread left in a non-terminal state is marked failed-safe.
    pub async fn recover_project_threads(
        &self,
        project_id: ProjectId,
    ) -> Result<Vec<ThreadId>, EngineError> {
        const PENDING_APPROVAL_METADATA_KEY: &str = "pending_approval";
        const RUNTIME_CHECKPOINT_METADATA_KEY: &str = "runtime_checkpoint";
        let threads = self.store.list_threads(project_id).await?;
        let mut recovered = Vec::new();

        for mut thread in threads {
            if thread.state.is_terminal() || thread.state == ThreadState::Completed {
                continue;
            }

            if thread.state == ThreadState::Waiting
                && thread.metadata.get(PENDING_APPROVAL_METADATA_KEY).is_some()
            {
                continue;
            }

            if thread
                .metadata
                .get(RUNTIME_CHECKPOINT_METADATA_KEY)
                .is_some()
                && matches!(thread.state, ThreadState::Running | ThreadState::Suspended)
            {
                if thread.state == ThreadState::Running {
                    thread.transition_to(
                        ThreadState::Suspended,
                        Some("engine restart; resumable from checkpoint".into()),
                    )?;
                }
                self.store.append_events(&thread.events).await?;
                self.store.save_thread(&thread).await?;
                recovered.push(thread.id);
                continue;
            }

            if thread
                .transition_to(
                    ThreadState::Failed,
                    Some("engine restart before thread completion".into()),
                )
                .is_ok()
            {
                self.store.append_events(&thread.events).await?;
                self.store.save_thread(&thread).await?;
                recovered.push(thread.id);
            }
        }

        Ok(recovered)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traits::llm::{LlmCallConfig, LlmOutput};
    use crate::types::capability::{ActionDef, Capability, CapabilityLease, EffectType};
    use crate::types::event::ThreadEvent;
    use crate::types::memory::{DocId, MemoryDoc};
    use crate::types::project::Project;
    use crate::types::step::{ActionResult, LlmResponse, Step, TokenUsage};
    use crate::types::thread::ThreadState;
    use std::sync::Mutex;
    use std::time::Duration;

    // ── Mocks ───────────────────────────────────────────────

    struct MockLlm {
        responses: Mutex<Vec<LlmOutput>>,
    }

    impl MockLlm {
        fn text(msg: &str) -> Arc<Self> {
            Arc::new(Self {
                responses: Mutex::new(vec![LlmOutput {
                    response: LlmResponse::Text(msg.into()),
                    usage: TokenUsage::default(),
                }]),
            })
        }
    }

    #[async_trait::async_trait]
    impl LlmBackend for MockLlm {
        async fn complete(
            &self,
            _: &[crate::types::message::ThreadMessage],
            _: &[ActionDef],
            _: &LlmCallConfig,
        ) -> Result<LlmOutput, EngineError> {
            let mut r = self.responses.lock().unwrap();
            if r.is_empty() {
                Ok(LlmOutput {
                    response: LlmResponse::Text("done".into()),
                    usage: TokenUsage::default(),
                })
            } else {
                Ok(r.remove(0))
            }
        }

        fn model_name(&self) -> &str {
            "mock"
        }
    }

    struct MockEffects;

    #[async_trait::async_trait]
    impl EffectExecutor for MockEffects {
        async fn execute_action(
            &self,
            _: &str,
            _: serde_json::Value,
            _: &CapabilityLease,
            _: &crate::traits::effect::ThreadExecutionContext,
        ) -> Result<ActionResult, EngineError> {
            Ok(ActionResult {
                call_id: String::new(),
                action_name: String::new(),
                output: serde_json::json!({}),
                is_error: false,
                duration: Duration::from_millis(1),
            })
        }

        async fn available_actions(
            &self,
            _: &[CapabilityLease],
        ) -> Result<Vec<ActionDef>, EngineError> {
            Ok(vec![])
        }
    }

    struct MockStore {
        threads: RwLock<HashMap<ThreadId, Thread>>,
        events: RwLock<HashMap<ThreadId, Vec<ThreadEvent>>>,
    }

    impl MockStore {
        fn new() -> Self {
            Self {
                threads: RwLock::new(HashMap::new()),
                events: RwLock::new(HashMap::new()),
            }
        }
    }

    #[async_trait::async_trait]
    impl Store for MockStore {
        async fn save_thread(&self, thread: &Thread) -> Result<(), EngineError> {
            self.threads.write().await.insert(thread.id, thread.clone());
            Ok(())
        }
        async fn load_thread(&self, id: ThreadId) -> Result<Option<Thread>, EngineError> {
            Ok(self.threads.read().await.get(&id).cloned())
        }
        async fn list_threads(&self, project_id: ProjectId) -> Result<Vec<Thread>, EngineError> {
            Ok(self
                .threads
                .read()
                .await
                .values()
                .filter(|thread| thread.project_id == project_id)
                .cloned()
                .collect())
        }
        async fn update_thread_state(
            &self,
            _: ThreadId,
            _: ThreadState,
        ) -> Result<(), EngineError> {
            Ok(())
        }
        async fn save_step(&self, _: &Step) -> Result<(), EngineError> {
            Ok(())
        }
        async fn load_steps(&self, _: ThreadId) -> Result<Vec<Step>, EngineError> {
            Ok(vec![])
        }
        async fn append_events(&self, events: &[ThreadEvent]) -> Result<(), EngineError> {
            let mut stored = self.events.write().await;
            for event in events {
                stored
                    .entry(event.thread_id)
                    .or_default()
                    .push(event.clone());
            }
            Ok(())
        }
        async fn load_events(&self, thread_id: ThreadId) -> Result<Vec<ThreadEvent>, EngineError> {
            Ok(self
                .events
                .read()
                .await
                .get(&thread_id)
                .cloned()
                .unwrap_or_default())
        }
        async fn save_project(&self, _: &Project) -> Result<(), EngineError> {
            Ok(())
        }
        async fn load_project(&self, _: ProjectId) -> Result<Option<Project>, EngineError> {
            Ok(None)
        }
        async fn save_memory_doc(&self, _: &MemoryDoc) -> Result<(), EngineError> {
            Ok(())
        }
        async fn load_memory_doc(&self, _: DocId) -> Result<Option<MemoryDoc>, EngineError> {
            Ok(None)
        }
        async fn list_memory_docs(&self, _: ProjectId) -> Result<Vec<MemoryDoc>, EngineError> {
            Ok(vec![])
        }
        async fn save_lease(&self, _: &CapabilityLease) -> Result<(), EngineError> {
            Ok(())
        }
        async fn load_active_leases(
            &self,
            _: ThreadId,
        ) -> Result<Vec<CapabilityLease>, EngineError> {
            Ok(vec![])
        }
        async fn revoke_lease(
            &self,
            _: crate::types::capability::LeaseId,
            _: &str,
        ) -> Result<(), EngineError> {
            Ok(())
        }
        async fn save_mission(
            &self,
            _: &crate::types::mission::Mission,
        ) -> Result<(), EngineError> {
            Ok(())
        }
        async fn load_mission(
            &self,
            _: crate::types::mission::MissionId,
        ) -> Result<Option<crate::types::mission::Mission>, EngineError> {
            Ok(None)
        }
        async fn list_missions(
            &self,
            _: ProjectId,
        ) -> Result<Vec<crate::types::mission::Mission>, EngineError> {
            Ok(vec![])
        }
        async fn update_mission_status(
            &self,
            _: crate::types::mission::MissionId,
            _: crate::types::mission::MissionStatus,
        ) -> Result<(), EngineError> {
            Ok(())
        }
    }

    fn make_manager(llm: Arc<dyn LlmBackend>) -> ThreadManager {
        let mut caps = CapabilityRegistry::new();
        caps.register(Capability {
            name: "test".into(),
            description: "Test capability".into(),
            actions: vec![ActionDef {
                name: "test_tool".into(),
                description: "Test".into(),
                parameters_schema: serde_json::json!({}),
                effects: vec![EffectType::ReadLocal],
                requires_approval: false,
            }],
            knowledge: vec![],
            policies: vec![],
        });

        ThreadManager::new(
            llm,
            Arc::new(MockEffects),
            Arc::new(MockStore::new()),
            Arc::new(caps),
            Arc::new(LeaseManager::new()),
            Arc::new(PolicyEngine::new()),
        )
    }

    fn make_manager_with_store(llm: Arc<dyn LlmBackend>, store: Arc<MockStore>) -> ThreadManager {
        let mut caps = CapabilityRegistry::new();
        caps.register(Capability {
            name: "test".into(),
            description: "Test capability".into(),
            actions: vec![ActionDef {
                name: "test_tool".into(),
                description: "Test".into(),
                parameters_schema: serde_json::json!({}),
                effects: vec![EffectType::ReadLocal],
                requires_approval: false,
            }],
            knowledge: vec![],
            policies: vec![],
        });

        ThreadManager::new(
            llm,
            Arc::new(MockEffects),
            store,
            Arc::new(caps),
            Arc::new(LeaseManager::new()),
            Arc::new(PolicyEngine::new()),
        )
    }

    // ── Tests ───────────────────────────────────────────────

    #[tokio::test]
    async fn spawn_and_join() {
        let mgr = make_manager(MockLlm::text("Hello!"));
        let project = ProjectId::new();

        let tid = mgr
            .spawn_thread(
                "test",
                ThreadType::Foreground,
                project,
                ThreadConfig::default(),
                None,
                "user",
            )
            .await
            .unwrap();

        let outcome = mgr.join_thread(tid).await.unwrap();
        assert!(matches!(outcome, ThreadOutcome::Completed { response: Some(r) } if r == "Hello!"));
    }

    #[tokio::test]
    async fn stop_thread_works() {
        // LLM that returns many action responses
        let responses: Vec<LlmOutput> = (0..100)
            .map(|i| LlmOutput {
                response: LlmResponse::ActionCalls {
                    calls: vec![crate::types::step::ActionCall {
                        id: format!("c{i}"),
                        action_name: "test_tool".into(),
                        parameters: serde_json::json!({}),
                    }],
                    content: None,
                },
                usage: TokenUsage::default(),
            })
            .collect();

        let mgr = make_manager(Arc::new(MockLlm {
            responses: Mutex::new(responses),
        }));
        let project = ProjectId::new();

        let tid = mgr
            .spawn_thread(
                "test",
                ThreadType::Foreground,
                project,
                ThreadConfig::default(),
                None,
                "user",
            )
            .await
            .unwrap();

        // Give it a moment to start, then stop
        tokio::time::sleep(Duration::from_millis(10)).await;
        let _ = mgr.stop_thread(tid).await;

        let outcome = mgr.join_thread(tid).await.unwrap();
        assert!(matches!(
            outcome,
            ThreadOutcome::Stopped | ThreadOutcome::Completed { .. } | ThreadOutcome::MaxIterations
        ));
    }

    #[tokio::test]
    async fn parent_child_tree() {
        let mgr = make_manager(MockLlm::text("parent done"));
        let project = ProjectId::new();

        let parent = mgr
            .spawn_thread(
                "parent",
                ThreadType::Foreground,
                project,
                ThreadConfig::default(),
                None,
                "user",
            )
            .await
            .unwrap();

        let child = mgr
            .spawn_thread(
                "child",
                ThreadType::Research,
                project,
                ThreadConfig::default(),
                Some(parent),
                "user",
            )
            .await
            .unwrap();

        assert_eq!(mgr.parent_of(child).await, Some(parent));
        assert_eq!(mgr.children_of(parent).await, vec![child]);
    }

    #[tokio::test]
    async fn recover_project_threads_marks_non_terminal_as_failed() {
        let store = Arc::new(MockStore::new());
        let project = ProjectId::new();

        let mut running = Thread::new(
            "running",
            ThreadType::Foreground,
            project,
            ThreadConfig::default(),
        );
        running.transition_to(ThreadState::Running, None).unwrap();
        store.save_thread(&running).await.unwrap();

        let mut completed = Thread::new(
            "done",
            ThreadType::Foreground,
            project,
            ThreadConfig::default(),
        );
        completed
            .transition_to(ThreadState::Failed, Some("already terminal".into()))
            .unwrap();
        store.save_thread(&completed).await.unwrap();

        let mgr = make_manager_with_store(MockLlm::text("ignored"), Arc::clone(&store));
        let recovered = mgr.recover_project_threads(project).await.unwrap();

        assert_eq!(recovered, vec![running.id]);
        let saved = store.load_thread(running.id).await.unwrap().unwrap();
        assert_eq!(saved.state, ThreadState::Failed);
        let events = store.load_events(running.id).await.unwrap();
        assert!(!events.is_empty());
    }

    #[tokio::test]
    async fn recover_project_threads_preserves_waiting_approval_threads() {
        let store = Arc::new(MockStore::new());
        let project = ProjectId::new();

        let mut waiting = Thread::new(
            "awaiting approval",
            ThreadType::Foreground,
            project,
            ThreadConfig::default(),
        );
        waiting.transition_to(ThreadState::Running, None).unwrap();
        waiting
            .transition_to(ThreadState::Waiting, Some("approval".into()))
            .unwrap();
        waiting.metadata = serde_json::json!({
            "pending_approval": {
                "request_id": "req-1",
                "action_name": "shell",
                "call_id": "call-1"
            }
        });
        store.save_thread(&waiting).await.unwrap();

        let mgr = make_manager_with_store(MockLlm::text("ignored"), Arc::clone(&store));
        let recovered = mgr.recover_project_threads(project).await.unwrap();

        assert!(recovered.is_empty());
        let saved = store.load_thread(waiting.id).await.unwrap().unwrap();
        assert_eq!(saved.state, ThreadState::Waiting);
    }

    #[tokio::test]
    async fn recover_project_threads_suspends_checkpointed_threads() {
        let store = Arc::new(MockStore::new());
        let project = ProjectId::new();

        let mut running = Thread::new(
            "resume me",
            ThreadType::Foreground,
            project,
            ThreadConfig::default(),
        );
        running.transition_to(ThreadState::Running, None).unwrap();
        running.metadata = serde_json::json!({
            "runtime_checkpoint": {
                "persisted_state": {"last_return": 7},
                "nudge_count": 0,
                "consecutive_errors": 0,
                "compaction_count": 0
            }
        });
        store.save_thread(&running).await.unwrap();

        let mgr = make_manager_with_store(MockLlm::text("ignored"), Arc::clone(&store));
        let recovered = mgr.recover_project_threads(project).await.unwrap();

        assert_eq!(recovered, vec![running.id]);
        let saved = store.load_thread(running.id).await.unwrap().unwrap();
        assert_eq!(saved.state, ThreadState::Suspended);
    }

    #[tokio::test]
    async fn resume_background_threads_restarts_suspended_research_threads() {
        let store = Arc::new(MockStore::new());
        let project = ProjectId::new();

        let mut research = Thread::new(
            "background research",
            ThreadType::Research,
            project,
            ThreadConfig::default(),
        );
        research.transition_to(ThreadState::Running, None).unwrap();
        research.metadata = serde_json::json!({
            "user_id": "owner",
            "runtime_checkpoint": {
                "persisted_state": {},
                "nudge_count": 0,
                "consecutive_errors": 0,
                "compaction_count": 0
            }
        });
        research
            .transition_to(
                ThreadState::Suspended,
                Some("engine restart; resumable from checkpoint".into()),
            )
            .unwrap();
        store.save_thread(&research).await.unwrap();

        let mgr = make_manager_with_store(MockLlm::text("done"), Arc::clone(&store));
        let resumed = mgr.resume_background_threads(project).await.unwrap();
        assert_eq!(resumed, vec![research.id]);

        let outcome = mgr.join_thread(research.id).await.unwrap();
        assert!(matches!(outcome, ThreadOutcome::Completed { .. }));
    }
}
