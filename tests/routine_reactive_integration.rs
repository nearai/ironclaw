//! Integration test: `routine_create` → reactive fire end-to-end.
//!
//! Drives the full chain (alias → effect adapter → mission manager →
//! `fire_on_message_event`) with zeroed guardrails and three inbound
//! events, asserting every event spawns a thread. Complements the
//! engine-level `reactive_mission_guardrails_are_respected_on_every_fire`
//! unit test, which bypasses the alias and guardrail-forwarding paths.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::RwLock;

use ironclaw::bridge::EffectBridgeAdapter;
use ironclaw::hooks::HookRegistry;
use ironclaw::tools::ToolRegistry;
use ironclaw_engine::types::capability::LeaseId;
use ironclaw_engine::{
    ActionDef, ActionResult, CapabilityLease, CapabilityRegistry, DocId, EffectExecutor,
    EngineError, GrantedActions, LeaseManager, LlmBackend, LlmCallConfig, LlmOutput, LlmResponse,
    MemoryDoc, Mission, MissionCadence, MissionId, MissionManager, MissionStatus, PolicyEngine,
    Project, ProjectId, Step, Store, Thread, ThreadEvent, ThreadId, ThreadManager, ThreadMessage,
    ThreadState, TokenUsage,
};
use ironclaw_safety::{SafetyConfig, SafetyLayer};

// ── Minimal in-memory Store impl ─────────────────────────────

struct TestStore {
    threads: RwLock<HashMap<ThreadId, Thread>>,
    events: RwLock<Vec<ThreadEvent>>,
    steps: RwLock<Vec<Step>>,
    missions: RwLock<Vec<Mission>>,
    leases: RwLock<Vec<CapabilityLease>>,
    docs: RwLock<Vec<MemoryDoc>>,
}

impl TestStore {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            threads: RwLock::new(HashMap::new()),
            events: RwLock::new(Vec::new()),
            steps: RwLock::new(Vec::new()),
            missions: RwLock::new(Vec::new()),
            leases: RwLock::new(Vec::new()),
            docs: RwLock::new(Vec::new()),
        })
    }
}

#[async_trait::async_trait]
impl Store for TestStore {
    async fn save_thread(&self, thread: &Thread) -> Result<(), EngineError> {
        self.threads.write().await.insert(thread.id, thread.clone());
        Ok(())
    }
    async fn load_thread(&self, id: ThreadId) -> Result<Option<Thread>, EngineError> {
        Ok(self.threads.read().await.get(&id).cloned())
    }
    async fn list_threads(
        &self,
        pid: ProjectId,
        _user_id: &str,
    ) -> Result<Vec<Thread>, EngineError> {
        Ok(self
            .threads
            .read()
            .await
            .values()
            .filter(|t| t.project_id == pid)
            .cloned()
            .collect())
    }
    async fn update_thread_state(
        &self,
        id: ThreadId,
        state: ThreadState,
    ) -> Result<(), EngineError> {
        if let Some(t) = self.threads.write().await.get_mut(&id) {
            t.state = state;
        }
        Ok(())
    }
    async fn save_step(&self, step: &Step) -> Result<(), EngineError> {
        let mut steps = self.steps.write().await;
        steps.retain(|s| s.id != step.id);
        steps.push(step.clone());
        Ok(())
    }
    async fn load_steps(&self, thread_id: ThreadId) -> Result<Vec<Step>, EngineError> {
        Ok(self
            .steps
            .read()
            .await
            .iter()
            .filter(|s| s.thread_id == thread_id)
            .cloned()
            .collect())
    }
    async fn append_events(&self, events: &[ThreadEvent]) -> Result<(), EngineError> {
        self.events.write().await.extend(events.iter().cloned());
        Ok(())
    }
    async fn load_events(&self, thread_id: ThreadId) -> Result<Vec<ThreadEvent>, EngineError> {
        Ok(self
            .events
            .read()
            .await
            .iter()
            .filter(|e| e.thread_id == thread_id)
            .cloned()
            .collect())
    }
    async fn save_project(&self, _: &Project) -> Result<(), EngineError> {
        Ok(())
    }
    async fn load_project(&self, _: ProjectId) -> Result<Option<Project>, EngineError> {
        Ok(None)
    }
    async fn list_projects(&self, _: &str) -> Result<Vec<Project>, EngineError> {
        Ok(vec![])
    }
    async fn list_all_projects(&self) -> Result<Vec<Project>, EngineError> {
        Ok(vec![])
    }
    async fn save_conversation(
        &self,
        _: &ironclaw_engine::ConversationSurface,
    ) -> Result<(), EngineError> {
        Ok(())
    }
    async fn load_conversation(
        &self,
        _: ironclaw_engine::ConversationId,
    ) -> Result<Option<ironclaw_engine::ConversationSurface>, EngineError> {
        Ok(None)
    }
    async fn list_conversations(
        &self,
        _: &str,
    ) -> Result<Vec<ironclaw_engine::ConversationSurface>, EngineError> {
        Ok(vec![])
    }
    async fn save_memory_doc(&self, doc: &MemoryDoc) -> Result<(), EngineError> {
        let mut docs = self.docs.write().await;
        docs.retain(|d| d.id != doc.id);
        docs.push(doc.clone());
        Ok(())
    }
    async fn load_memory_doc(&self, id: DocId) -> Result<Option<MemoryDoc>, EngineError> {
        Ok(self.docs.read().await.iter().find(|d| d.id == id).cloned())
    }
    async fn list_memory_docs(&self, _: ProjectId, _: &str) -> Result<Vec<MemoryDoc>, EngineError> {
        Ok(self.docs.read().await.clone())
    }
    async fn save_lease(&self, lease: &CapabilityLease) -> Result<(), EngineError> {
        let mut leases = self.leases.write().await;
        leases.retain(|l| l.id != lease.id);
        leases.push(lease.clone());
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
            .iter()
            .filter(|l| l.thread_id == thread_id && !l.revoked)
            .cloned()
            .collect())
    }
    async fn revoke_lease(&self, lease_id: LeaseId, _: &str) -> Result<(), EngineError> {
        if let Some(l) = self
            .leases
            .write()
            .await
            .iter_mut()
            .find(|l| l.id == lease_id)
        {
            l.revoked = true;
        }
        Ok(())
    }
    async fn save_mission(&self, mission: &Mission) -> Result<(), EngineError> {
        let mut missions = self.missions.write().await;
        missions.retain(|m| m.id != mission.id);
        missions.push(mission.clone());
        Ok(())
    }
    async fn load_mission(&self, id: MissionId) -> Result<Option<Mission>, EngineError> {
        Ok(self
            .missions
            .read()
            .await
            .iter()
            .find(|m| m.id == id)
            .cloned())
    }
    async fn list_missions(&self, _: ProjectId, _: &str) -> Result<Vec<Mission>, EngineError> {
        Ok(self.missions.read().await.clone())
    }
    async fn update_mission_status(
        &self,
        _: MissionId,
        _: MissionStatus,
    ) -> Result<(), EngineError> {
        Ok(())
    }
}

// ── Mock LLM that returns "done" immediately ─────────────────

struct MockLlm;

#[async_trait::async_trait]
impl LlmBackend for MockLlm {
    async fn complete(
        &self,
        _: &[ThreadMessage],
        _: &[ActionDef],
        _: &LlmCallConfig,
    ) -> Result<LlmOutput, EngineError> {
        Ok(LlmOutput {
            response: LlmResponse::Text("done".into()),
            usage: TokenUsage::default(),
        })
    }
    fn model_name(&self) -> &str {
        "mock"
    }
}

struct NoopEffects;

#[async_trait::async_trait]
impl EffectExecutor for NoopEffects {
    async fn execute_action(
        &self,
        _: &str,
        _: serde_json::Value,
        _: &CapabilityLease,
        _: &ironclaw_engine::ThreadExecutionContext,
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

fn make_lease() -> CapabilityLease {
    CapabilityLease {
        id: LeaseId::new(),
        thread_id: ThreadId::new(),
        capability_name: "tools".into(),
        granted_actions: GrantedActions::All,
        granted_at: chrono::Utc::now(),
        expires_at: None,
        max_uses: None,
        uses_remaining: None,
        revoked: false,
        revoked_reason: None,
    }
}

fn make_ctx(
    project_id: ProjectId,
    user_id: &str,
    call_id: &str,
) -> ironclaw_engine::ThreadExecutionContext {
    ironclaw_engine::ThreadExecutionContext {
        thread_id: ThreadId::new(),
        thread_type: ironclaw_engine::types::thread::ThreadType::Foreground,
        project_id,
        user_id: user_id.to_string(),
        step_id: ironclaw_engine::StepId::new(),
        current_call_id: Some(call_id.to_string()),
        source_channel: None,
        user_timezone: None,
    }
}

/// End-to-end: `routine_create` wires up a reactive mission with
/// zeroed guardrails, and subsequent `fire_on_message_event` calls spawn
/// a thread on every match.
#[tokio::test]
async fn routine_create_reactive_mission_fires_on_every_message() {
    // 1. Mission manager backed by an in-memory store.
    let store = TestStore::new();
    let store_dyn: Arc<dyn Store> = Arc::clone(&store) as Arc<dyn Store>;
    let thread_manager = Arc::new(ThreadManager::new(
        Arc::new(MockLlm),
        Arc::new(NoopEffects),
        Arc::clone(&store_dyn),
        Arc::new(CapabilityRegistry::new()),
        Arc::new(LeaseManager::new()),
        Arc::new(PolicyEngine::new()),
    ));
    let mission_manager = Arc::new(MissionManager::new(
        Arc::clone(&store_dyn),
        Arc::clone(&thread_manager),
    ));

    // 2. Adapter + mission manager.
    let adapter = EffectBridgeAdapter::new(
        Arc::new(ToolRegistry::new()),
        Arc::new(SafetyLayer::new(&SafetyConfig {
            max_output_length: 10_000,
            injection_check_enabled: false,
        })),
        Arc::new(HookRegistry::default()),
    );
    adapter
        .set_mission_manager(Arc::clone(&mission_manager))
        .await;

    // 3. Call routine_create. Zeroed guardrails + `.*` on telegram model
    // the "log every telegram message" case.
    let project_id = ProjectId::new();
    let params = serde_json::json!({
        "name": "telegram-logger",
        "prompt": "log every incoming telegram message",
        "request": {
            "kind": "message_event",
            "pattern": ".*",
            "channel": "telegram",
        },
        "advanced": {
            "cooldown_secs": 0,
        },
        "guardrails": {
            "max_concurrent": 0,
            "max_threads_per_day": 0,
            "dedup_window_secs": 0,
        },
    });

    let create_result = adapter
        .execute_action(
            "routine_create",
            params,
            &make_lease(),
            &make_ctx(project_id, "alice", "call_routine_create_1"),
        )
        .await
        .expect("routine_create should return a tool result, not an engine error");

    assert!(
        !create_result.is_error,
        "routine_create must succeed. output = {:?}",
        create_result.output
    );
    let created_status = create_result
        .output
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    assert!(
        created_status.starts_with("created"),
        "expected created/created_with_warnings, got: {created_status:?} (full output: {:?})",
        create_result.output
    );

    // 4. Mission persisted with real cadence and guardrails.
    let missions = mission_manager
        .list_missions(project_id, "alice")
        .await
        .expect("list_missions");
    let mission = missions
        .iter()
        .find(|m| m.name == "telegram-logger")
        .expect("mission must have been persisted under its name");
    match &mission.cadence {
        MissionCadence::OnEvent {
            event_pattern,
            channel,
        } => {
            assert_eq!(event_pattern, ".*");
            assert_eq!(channel.as_deref(), Some("telegram"));
        }
        other => panic!("expected OnEvent cadence, got {other:?}"),
    }
    assert_eq!(
        mission.cooldown_secs, 0,
        "cooldown_secs=0 must land on the first persisted save"
    );
    assert_eq!(
        mission.max_concurrent, 0,
        "max_concurrent=0 must land on the first persisted save"
    );
    assert_eq!(
        mission.max_threads_per_day, 0,
        "max_threads_per_day=0 must land on the first persisted save"
    );
    assert_eq!(
        mission.dedup_window_secs, 0,
        "dedup_window_secs=0 must land on the first persisted save"
    );

    // 5. Three inbound events must each spawn a thread.
    for msg in ["first", "second", "third"] {
        let spawned = mission_manager
            .fire_on_message_event("telegram", msg, "alice", None)
            .await
            .expect("fire_on_message_event should not error");
        assert_eq!(
            spawned.len(),
            1,
            "event {msg:?} must spawn exactly one thread (zeroed guardrails should not gate fires)"
        );
    }

    // 6. Thread history reflects all three fires.
    let after = mission_manager
        .get_mission(mission.id)
        .await
        .expect("get_mission")
        .expect("mission still present");
    assert_eq!(
        after.thread_history.len(),
        3,
        "three inbound events should produce three threads in the mission history"
    );
}
