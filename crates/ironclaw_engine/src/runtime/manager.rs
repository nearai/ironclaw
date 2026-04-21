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
use crate::runtime::lease_refresh::reconcile_dynamic_tool_lease;
use crate::runtime::messaging::{self, SignalSender, ThreadOutcome, ThreadSignal};
use crate::runtime::tree::ThreadTree;
use crate::traits::effect::EffectExecutor;
use crate::traits::llm::LlmBackend;
use crate::traits::store::Store;
use crate::types::error::EngineError;
use crate::types::message::{MessageRole, ThreadMessage};
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
            None,
            thread_type,
            project_id,
            config,
            parent_id,
            user_id,
            Vec::new(),
            serde_json::Map::new(),
        )
        .await
    }

    /// Spawn a new thread with an explicit sidebar title.
    ///
    /// Callers with a semantic short label (e.g. mission name) should
    /// use this; everything else can rely on `spawn_thread` + the
    /// read-side fallback that derives a short title from `goal`.
    #[allow(clippy::too_many_arguments)]
    pub async fn spawn_thread_with_title(
        &self,
        goal: impl Into<String>,
        title: Option<String>,
        thread_type: ThreadType,
        project_id: ProjectId,
        config: ThreadConfig,
        parent_id: Option<ThreadId>,
        user_id: impl Into<String>,
    ) -> Result<ThreadId, EngineError> {
        self.spawn_thread_with_history(
            goal,
            title,
            thread_type,
            project_id,
            config,
            parent_id,
            user_id,
            Vec::new(),
            serde_json::Map::new(),
        )
        .await
    }

    /// Spawn a thread with initial conversation history.
    ///
    /// `initial_metadata` is applied to the thread's metadata map *before* the
    /// background execution task starts, so the executor's in-memory `Thread`
    /// observes those keys on the first step. This is the only correct way to
    /// stamp metadata that the very first orchestrator step needs to read
    /// (e.g. `source_channel` for `mission_create` notify-channel defaulting,
    /// or `user_timezone` for cron resolution). Setting metadata after spawn
    /// via `set_thread_metadata` is a race — the spawned task owns its own
    /// in-memory copy of the `Thread`, and the late update only lands on the
    /// persisted copy that the running task never re-reads.
    #[allow(clippy::too_many_arguments)]
    pub async fn spawn_thread_with_history(
        &self,
        goal: impl Into<String>,
        title: Option<String>,
        thread_type: ThreadType,
        project_id: ProjectId,
        config: ThreadConfig,
        parent_id: Option<ThreadId>,
        user_id: impl Into<String>,
        initial_messages: Vec<crate::types::message::ThreadMessage>,
        initial_metadata: serde_json::Map<String, serde_json::Value>,
    ) -> Result<ThreadId, EngineError> {
        let user_id = user_id.into();
        let mut thread = Thread::new(goal, thread_type, project_id, &user_id, config);
        if let Some(pid) = parent_id {
            thread = thread.with_parent(pid);
        }
        // Set the title before save_thread + start_thread so the
        // executor's in-memory thread observes it atomically.
        thread.title = title;
        let thread_id = thread.id;

        // Apply initial metadata before save_thread + start_thread so the
        // executor's in-memory thread observes it on the first step.
        if !initial_metadata.is_empty()
            && let Some(obj) = thread.metadata.as_object_mut()
        {
            for (k, v) in initial_metadata {
                obj.insert(k, v);
            }
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
                .await?;
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
        resolved_call_id: Option<String>,
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

        // Tenant isolation: verify the requesting user owns this thread.
        let uid: String = user_id.into();
        if !thread.is_owned_by(&uid) {
            return Err(EngineError::AccessDenied {
                user_id: uid,
                entity: format!("thread {thread_id}"),
            });
        }

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

        if let Some(ref call_id) = resolved_call_id {
            let preserve_assistant_call = injected_message.as_ref().is_some_and(|message| {
                message.role == MessageRole::ActionResult
                    && message.action_call_id.as_deref() == Some(call_id.as_str())
            });
            thread.messages.retain(|existing| {
                if preserve_assistant_call {
                    !is_resolved_action_result_message(existing, call_id)
                } else {
                    !is_resolved_call_message(existing, call_id)
                }
            });
        }

        if let Some(message) = injected_message {
            thread.add_internal_message(message.clone());
            thread.add_message(message);
        }

        // Waiting threads paused on approval/auth should resume from the
        // newly injected context rather than replaying the old checkpointed
        // interrupt. Suspended threads keep their checkpoint for restart.
        if thread.state == crate::types::thread::ThreadState::Waiting
            && let Some(metadata) = thread.metadata.as_object_mut()
        {
            metadata.remove("runtime_checkpoint");
        }

        self.store.save_thread(&thread).await?;
        self.start_thread(thread, uid, true).await?;
        Ok(())
    }

    async fn start_thread(
        &self,
        mut thread: Thread,
        user_id: String,
        is_resume: bool,
    ) -> Result<ThreadId, EngineError> {
        let thread_id = thread.id;

        reconcile_dynamic_tool_lease(
            &mut thread,
            &self.effects,
            &self.leases,
            Some(&self.store),
            &self.lease_planner,
        )
        .await?;

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
        let running = Arc::clone(&self.running);
        let completed = Arc::clone(&self.completed);
        let handle = tokio::spawn(async move {
            let mut exec = exec_loop;
            let result = exec.run().await;
            debug!(thread_id = %thread_id, "thread execution finished");

            // Run retrospective trace analysis (non-LLM, always runs).
            // Issues are picked up by the self-improvement mission via event listener.
            let trace = crate::executor::trace::build_trace(&exec.thread);
            if !trace.issues.is_empty() {
                crate::executor::trace::log_trace_summary(&trace);
            }

            // Transition Completed → Done
            if exec.thread.state == crate::types::thread::ThreadState::Completed
                && let Err(e) = exec
                    .thread
                    .transition_to(crate::types::thread::ThreadState::Done, None)
            {
                tracing::debug!(thread_id = %thread_id, "failed to transition to Done: {e}");
            }

            // Trace recording is handled centrally by `RecordingLlm` in the
            // host crate (gated by `IRONCLAW_RECORD_TRACE`). The engine no
            // longer writes its own JSON trace file.

            if let Err(e) = store_for_task.append_events(&exec.thread.events).await {
                tracing::debug!(
                    thread_id = %thread_id,
                    "failed to persist thread events: {e}"
                );
            }

            // Save final thread state to store
            if let Err(e) = store_for_task.save_thread(&exec.thread).await {
                tracing::debug!(
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
    pub async fn stop_thread(&self, thread_id: ThreadId, user_id: &str) -> Result<(), EngineError> {
        // Validate ownership before allowing stop.
        if let Some(thread) = self.store.load_thread(thread_id).await?
            && !thread.is_owned_by(user_id)
        {
            return Err(EngineError::AccessDenied {
                user_id: user_id.to_string(),
                entity: format!("thread {thread_id}"),
            });
        }
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
        user_id: &str,
        message: ThreadMessage,
    ) -> Result<(), EngineError> {
        // Validate ownership before allowing injection.
        if let Some(thread) = self.store.load_thread(thread_id).await?
            && !thread.is_owned_by(user_id)
        {
            return Err(EngineError::AccessDenied {
                user_id: user_id.to_string(),
                entity: format!("thread {thread_id}"),
            });
        }
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

    /// Set a metadata key on the persisted thread record.
    ///
    /// Note: this updates the **store**, not the in-memory `Thread` that an
    /// already-running `ExecutionLoop` is reading from. Callers that need the
    /// next executor step to observe the new value must apply this *before*
    /// the executor task is spawned (initial-create path) or before
    /// `resume_thread`, which reloads from the store.
    pub async fn set_thread_metadata(
        &self,
        thread_id: ThreadId,
        key: &str,
        value: &str,
    ) -> Result<(), EngineError> {
        let mut thread = self
            .store
            .load_thread(thread_id)
            .await
            .map_err(|e| EngineError::Store {
                reason: format!("set_thread_metadata: load failed: {e}"),
            })?
            .ok_or(EngineError::ThreadNotFound(thread_id))?;
        if let Some(obj) = thread.metadata.as_object_mut() {
            obj.insert(
                key.to_string(),
                serde_json::Value::String(value.to_string()),
            );
        }
        self.store
            .save_thread(&thread)
            .await
            .map_err(|e| EngineError::Store {
                reason: format!("set_thread_metadata: save failed: {e}"),
            })?;
        Ok(())
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
            Some(rt) => {
                let result = match rt.handle.await {
                    Ok(result) => result,
                    Err(e) => {
                        error!(thread_id = %thread_id, "thread task panicked: {e}");
                        Ok(ThreadOutcome::Failed {
                            error: format!("thread task panicked: {e}"),
                        })
                    }
                };
                self.completed.write().await.remove(&thread_id);
                result
            }
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
        // System operation: resume all suspended research threads regardless of user.
        let threads = self.store.list_all_threads(project_id).await?;
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
            if thread.user_id.is_empty() {
                continue;
            }

            self.resume_thread(thread.id, thread.user_id.clone(), None, None, None)
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
        // System operation: recover all non-terminal threads regardless of user.
        let threads = self.store.list_all_threads(project_id).await?;
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

fn is_resolved_call_message(message: &ThreadMessage, call_id: &str) -> bool {
    if message.role == MessageRole::ActionResult
        && message.action_call_id.as_deref() == Some(call_id)
    {
        return true;
    }

    message.role == MessageRole::Assistant
        && message
            .action_calls
            .as_ref()
            .is_some_and(|calls| calls.iter().any(|call| call.id == call_id))
}

fn is_resolved_action_result_message(message: &ThreadMessage, call_id: &str) -> bool {
    message.role == MessageRole::ActionResult && message.action_call_id.as_deref() == Some(call_id)
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

    struct DynamicEffects {
        actions: RwLock<Vec<ActionDef>>,
        calls: RwLock<Vec<String>>,
        install_reveals: RwLock<Option<Vec<ActionDef>>>,
    }

    impl DynamicEffects {
        fn new(actions: Vec<ActionDef>) -> Arc<Self> {
            Arc::new(Self {
                actions: RwLock::new(actions),
                calls: RwLock::new(Vec::new()),
                install_reveals: RwLock::new(None),
            })
        }

        async fn set_actions(&self, actions: Vec<ActionDef>) {
            *self.actions.write().await = actions;
        }

        async fn set_install_reveals(&self, actions: Vec<ActionDef>) {
            *self.install_reveals.write().await = Some(actions);
        }

        async fn recorded_calls(&self) -> Vec<String> {
            self.calls.read().await.clone()
        }
    }

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

    #[async_trait::async_trait]
    impl EffectExecutor for DynamicEffects {
        async fn execute_action(
            &self,
            action_name: &str,
            _: serde_json::Value,
            _: &CapabilityLease,
            _: &crate::traits::effect::ThreadExecutionContext,
        ) -> Result<ActionResult, EngineError> {
            self.calls.write().await.push(action_name.to_string());
            if action_name == "tool_install"
                && let Some(actions) = self.install_reveals.read().await.clone()
            {
                *self.actions.write().await = actions;
            }
            Ok(ActionResult {
                call_id: String::new(),
                action_name: action_name.to_string(),
                output: serde_json::json!({}),
                is_error: false,
                duration: Duration::from_millis(1),
            })
        }

        async fn available_actions(
            &self,
            _: &[CapabilityLease],
        ) -> Result<Vec<ActionDef>, EngineError> {
            Ok(self.actions.read().await.clone())
        }
    }

    struct MockStore {
        threads: RwLock<HashMap<ThreadId, Thread>>,
        events: RwLock<HashMap<ThreadId, Vec<ThreadEvent>>>,
        leases: RwLock<HashMap<crate::types::capability::LeaseId, CapabilityLease>>,
    }

    impl MockStore {
        fn new() -> Self {
            Self {
                threads: RwLock::new(HashMap::new()),
                events: RwLock::new(HashMap::new()),
                leases: RwLock::new(HashMap::new()),
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
        async fn list_threads(
            &self,
            project_id: ProjectId,
            user_id: &str,
        ) -> Result<Vec<Thread>, EngineError> {
            Ok(self
                .threads
                .read()
                .await
                .values()
                .filter(|thread| thread.project_id == project_id && thread.user_id == user_id)
                .cloned()
                .collect())
        }
        async fn list_all_threads(
            &self,
            project_id: ProjectId,
        ) -> Result<Vec<Thread>, EngineError> {
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
        async fn list_memory_docs(
            &self,
            _: ProjectId,
            _: &str,
        ) -> Result<Vec<MemoryDoc>, EngineError> {
            Ok(vec![])
        }
        async fn list_memory_docs_by_owner(&self, _: &str) -> Result<Vec<MemoryDoc>, EngineError> {
            Ok(vec![])
        }
        async fn save_lease(&self, lease: &CapabilityLease) -> Result<(), EngineError> {
            self.leases.write().await.insert(lease.id, lease.clone());
            Ok(())
        }
        async fn load_active_leases(
            &self,
            thread_id: ThreadId,
        ) -> Result<Vec<CapabilityLease>, EngineError> {
            Ok(self
                .leases
                .read()
                .await
                .values()
                .filter(|lease| lease.thread_id == thread_id && lease.is_valid())
                .cloned()
                .collect())
        }
        async fn revoke_lease(
            &self,
            lease_id: crate::types::capability::LeaseId,
            _: &str,
        ) -> Result<(), EngineError> {
            if let Some(lease) = self.leases.write().await.get_mut(&lease_id) {
                lease.revoked = true;
            }
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
            _: &str,
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

    fn make_manager_with_effects(
        llm: Arc<dyn LlmBackend>,
        store: Arc<MockStore>,
        effects: Arc<dyn EffectExecutor>,
    ) -> ThreadManager {
        let mut caps = CapabilityRegistry::new();
        caps.register(Capability {
            name: "tools".into(),
            description: "Tools".into(),
            actions: vec![ActionDef {
                name: "tool_install".into(),
                description: "Install a tool".into(),
                parameters_schema: serde_json::json!({}),
                effects: vec![EffectType::WriteLocal],
                requires_approval: false,
            }],
            knowledge: vec![],
            policies: vec![],
        });

        ThreadManager::new(
            llm,
            effects,
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
    async fn spawn_thread_defaults_title_to_none() {
        // Regression: without explicit title, the persisted thread has
        // title = None — so the gateway falls back to deriving from goal
        // rather than showing stale data.
        let mgr = make_manager(MockLlm::text("done"));
        let tid = mgr
            .spawn_thread(
                "some long goal",
                ThreadType::Foreground,
                ProjectId::new(),
                ThreadConfig::default(),
                None,
                "user",
            )
            .await
            .unwrap();
        let _ = mgr.join_thread(tid).await.unwrap();
        let loaded = mgr.store.load_thread(tid).await.unwrap().unwrap();
        assert_eq!(loaded.title, None);
    }

    #[tokio::test]
    async fn spawn_thread_with_title_persists_title() {
        // Regression: mission-spawned threads pass `Some(mission.name)` so
        // the sidebar shows the short label instead of the multi-paragraph
        // meta-prompt that lives in `goal`.
        let mgr = make_manager(MockLlm::text("done"));
        let long_goal = "a".repeat(2000);
        let tid = mgr
            .spawn_thread_with_title(
                &long_goal,
                Some("Daily summary".to_string()),
                ThreadType::Mission,
                ProjectId::new(),
                ThreadConfig::default(),
                None,
                "user",
            )
            .await
            .unwrap();
        let _ = mgr.join_thread(tid).await.unwrap();
        let loaded = mgr.store.load_thread(tid).await.unwrap().unwrap();
        assert_eq!(loaded.title.as_deref(), Some("Daily summary"));
        assert_eq!(loaded.goal.len(), 2000);
    }

    #[tokio::test]
    async fn spawn_thread_with_history_persists_derived_title() {
        // Regression: foreground gateway threads pass a derived short
        // title; without this, the sidebar would show the full first
        // user message.
        let mgr = make_manager(MockLlm::text("done"));
        let long_message =
            "first line of a user message\nsecond line that should be ignored".to_string();
        let derived = Thread::derive_title_from_message(&long_message);
        assert_eq!(derived.as_deref(), Some("first line of a user message"));

        let tid = mgr
            .spawn_thread_with_history(
                long_message,
                derived.clone(),
                ThreadType::Foreground,
                ProjectId::new(),
                ThreadConfig::default(),
                None,
                "user",
                Vec::new(),
                serde_json::Map::new(),
            )
            .await
            .unwrap();
        let _ = mgr.join_thread(tid).await.unwrap();
        let loaded = mgr.store.load_thread(tid).await.unwrap().unwrap();
        assert_eq!(loaded.title, derived);
    }

    #[tokio::test]
    async fn resume_reconciles_tool_lease_with_newly_available_actions() {
        let store = Arc::new(MockStore::new());
        let effects = DynamicEffects::new(vec![ActionDef {
            name: "tool_install".into(),
            description: "Install a tool".into(),
            parameters_schema: serde_json::json!({}),
            effects: vec![EffectType::WriteLocal],
            requires_approval: false,
        }]);
        let mgr = make_manager_with_effects(MockLlm::text("done"), store, effects.clone());

        let thread_id = ThreadId::new();
        let mut thread = Thread::new(
            "use notion",
            ThreadType::Foreground,
            ProjectId::new(),
            "user",
            ThreadConfig::default(),
        );
        thread.id = thread_id;
        thread.state = ThreadState::Waiting;
        mgr.store.save_thread(&thread).await.unwrap();

        let lease = mgr
            .leases
            .grant(
                thread_id,
                "tools",
                crate::types::capability::GrantedActions::Specific(vec!["tool_install".into()]),
                None,
                None,
            )
            .await
            .unwrap();
        mgr.store.save_lease(&lease).await.unwrap();

        effects
            .set_actions(vec![
                ActionDef {
                    name: "tool_install".into(),
                    description: "Install a tool".into(),
                    parameters_schema: serde_json::json!({}),
                    effects: vec![EffectType::WriteLocal],
                    requires_approval: false,
                },
                ActionDef {
                    name: "notion_search".into(),
                    description: "Search Notion".into(),
                    parameters_schema: serde_json::json!({}),
                    effects: vec![EffectType::ReadExternal],
                    requires_approval: false,
                },
            ])
            .await;

        mgr.resume_thread(thread_id, "user", None, None, None)
            .await
            .unwrap();
        let _ = mgr.join_thread(thread_id).await.unwrap();

        let refreshed = mgr
            .leases
            .find_lease_for_action(thread_id, "notion_search")
            .await;
        assert!(
            refreshed.is_some(),
            "resume should refresh tools lease for newly available actions"
        );
    }

    #[tokio::test]
    async fn spawn_reconciles_tool_lease_with_stale_capability_snapshot() {
        let store = Arc::new(MockStore::new());
        let effects = DynamicEffects::new(vec![
            ActionDef {
                name: "tool_install".into(),
                description: "Install a tool".into(),
                parameters_schema: serde_json::json!({}),
                effects: vec![EffectType::WriteLocal],
                requires_approval: false,
            },
            ActionDef {
                name: "notion_search".into(),
                description: "Search Notion".into(),
                parameters_schema: serde_json::json!({}),
                effects: vec![EffectType::ReadExternal],
                requires_approval: false,
            },
        ]);
        let mgr = make_manager_with_effects(MockLlm::text("done"), store, effects);

        let tid = mgr
            .spawn_thread(
                "use notion",
                ThreadType::Foreground,
                ProjectId::new(),
                ThreadConfig::default(),
                None,
                "user",
            )
            .await
            .unwrap();

        let lease = mgr.leases.find_lease_for_action(tid, "notion_search").await;
        assert!(
            lease.is_some(),
            "spawn should refresh tools lease for actions exposed after the capability snapshot"
        );
    }

    #[tokio::test]
    async fn running_thread_can_install_then_use_new_tool_without_user_bounce() {
        let store = Arc::new(MockStore::new());
        let initial_actions = vec![ActionDef {
            name: "tool_install".into(),
            description: "Install a tool".into(),
            parameters_schema: serde_json::json!({}),
            effects: vec![EffectType::WriteLocal],
            requires_approval: false,
        }];
        let revealed_actions = vec![
            ActionDef {
                name: "tool_install".into(),
                description: "Install a tool".into(),
                parameters_schema: serde_json::json!({}),
                effects: vec![EffectType::WriteLocal],
                requires_approval: false,
            },
            ActionDef {
                name: "notion_search".into(),
                description: "Search Notion".into(),
                parameters_schema: serde_json::json!({}),
                effects: vec![EffectType::ReadExternal],
                requires_approval: false,
            },
        ];
        let effects = DynamicEffects::new(initial_actions);
        effects.set_install_reveals(revealed_actions).await;
        let llm = Arc::new(MockLlm {
            responses: Mutex::new(vec![
                LlmOutput {
                    response: LlmResponse::ActionCalls {
                        calls: vec![crate::types::step::ActionCall {
                            id: "call_install".into(),
                            action_name: "tool_install".into(),
                            parameters: serde_json::json!({"name": "notion"}),
                        }],
                        content: None,
                    },
                    usage: TokenUsage::default(),
                },
                LlmOutput {
                    response: LlmResponse::ActionCalls {
                        calls: vec![crate::types::step::ActionCall {
                            id: "call_search".into(),
                            action_name: "notion_search".into(),
                            parameters: serde_json::json!({"query": "latest meeting note"}),
                        }],
                        content: None,
                    },
                    usage: TokenUsage::default(),
                },
                LlmOutput {
                    response: LlmResponse::Text("done".into()),
                    usage: TokenUsage::default(),
                },
            ]),
        });
        let mgr = make_manager_with_effects(llm, store, effects.clone());

        let tid = mgr
            .spawn_thread(
                "install notion and get the latest meeting note",
                ThreadType::Foreground,
                ProjectId::new(),
                ThreadConfig::default(),
                None,
                "user",
            )
            .await
            .unwrap();

        let outcome = mgr.join_thread(tid).await.unwrap();
        assert!(matches!(outcome, ThreadOutcome::Completed { .. }));

        let calls = effects.recorded_calls().await;
        assert_eq!(
            calls,
            vec!["tool_install".to_string(), "notion_search".to_string()],
            "thread should continue from install into the newly exposed tool without pausing for a new user turn"
        );
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
        let _ = mgr.stop_thread(tid, "test-user").await;

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
            "test-user",
            ThreadConfig::default(),
        );
        running.transition_to(ThreadState::Running, None).unwrap();
        store.save_thread(&running).await.unwrap();

        let mut completed = Thread::new(
            "done",
            ThreadType::Foreground,
            project,
            "test-user",
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
            "test-user",
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
            "test-user",
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
            "test-user",
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

    // Skill selection and injection tests are in tests/engine_v2_skill_codeact.rs
    // (skill selection happens in the Python orchestrator, not in Rust).
}
