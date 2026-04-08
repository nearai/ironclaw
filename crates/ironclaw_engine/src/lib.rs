//! IronClaw Engine — unified thread-capability-CodeAct execution model.
//!
//! This crate provides the core execution engine for IronClaw, unifying
//! ~10 separate abstractions (Session, Job, Routine, Channel, Tool, Skill,
//! Hook, Observer, Extension, LoopDelegate) around 5 primitives:
//!
//! - **Thread** — unit of work (replaces Session + Job + Routine + Sub-agent)
//! - **Step** — unit of execution (replaces agentic loop iteration + tool calls)
//! - **Capability** — unit of effect (replaces Tool + Skill + Hook + Extension)
//! - **MemoryDoc** — unit of durable knowledge (replaces workspace memory blobs)
//! - **Project** — unit of context (replaces flat workspace namespace)
//!
//! The engine defines traits for external dependencies ([`LlmBackend`],
//! [`Store`], [`EffectExecutor`]) that the host crate implements via bridge
//! adapters over existing infrastructure.

pub mod capability;
pub mod executor;
pub mod gate;
pub mod memory;
pub mod reliability;
pub mod runtime;
pub mod traits;
pub mod types;

// ── Re-exports: types ───────────────────────────────────────

pub use types::capability::{
    ActionDef, Capability, CapabilityLease, EffectType, GrantedActions, LeaseId, PolicyCondition,
    PolicyEffect, PolicyRule,
};
pub use types::error::{CapabilityError, EngineError, StepError, ThreadError};
pub use types::event::{EventId, EventKind, ThreadEvent};
pub use types::memory::{DocId, DocType, MemoryDoc};
pub use types::message::{MessageRole, ThreadMessage};
pub use types::mission::{Mission, MissionCadence, MissionId, MissionStatus};
pub use types::project::{Project, ProjectId, system_project_id};
pub use types::provenance::Provenance;
pub use types::step::{
    ActionCall, ActionResult, ExecutionTier, LlmResponse, Step, StepId, StepStatus, TokenUsage,
};
pub use types::thread::{
    ActiveSkillProvenance, Thread, ThreadConfig, ThreadId, ThreadState, ThreadType,
};

// ── Re-exports: traits ──────────────────────────────────────

pub use traits::effect::{EffectExecutor, ThreadExecutionContext};
pub use traits::llm::{LlmBackend, LlmCallConfig, LlmOutput};
pub use traits::store::Store;

// ── Re-exports: capability ────────────────────────────────────

pub use capability::lease::LeaseManager;
pub use capability::planner::{CapabilityGrantPlan, LeasePlanner};
pub use capability::policy::{PolicyDecision, PolicyEngine};
pub use capability::registry::CapabilityRegistry;

// ── Re-exports: gate ─────────────────────────────────────────

pub use gate::lease::LeaseGate;
pub use gate::pipeline::GatePipeline;
pub use gate::tool_tier::{ToolTier, classify_tool_tier};
pub use gate::{
    ExecutionGate, ExecutionMode, GateContext, GateDecision, GateResolution, ResumeKind,
};

// ── Re-exports: runtime ───────────────────────────────────────

pub use executor::prompt::PlatformInfo;
pub use runtime::conversation::ConversationManager;
pub use runtime::manager::ThreadManager;
pub use runtime::messaging::ThreadOutcome;
pub use runtime::mission::{MissionManager, MissionNotification, MissionUpdate};
pub use runtime::tree::ThreadTree;

pub use types::conversation::{
    ConversationEntry, ConversationId, ConversationSurface, EntrySender,
};

// ── Re-exports: executor ──────────────────────────────────────

pub use executor::ExecutionLoop;

// ── Re-exports: memory ────────────────────────────────────────

pub use memory::MemoryStore;
pub use memory::RetrievalEngine;

// ── Re-exports: reliability ──────────────────────────────────

pub use reliability::ReliabilityTracker;

// ── Test utilities ──────────────────────────────────────────

#[cfg(test)]
pub(crate) mod tests {
    use tokio::sync::RwLock;

    use crate::traits::store::Store;
    use crate::types::capability::{CapabilityLease, LeaseId};
    use crate::types::conversation::{ConversationId, ConversationSurface};
    use crate::types::error::EngineError;
    use crate::types::event::ThreadEvent;
    use crate::types::memory::{DocId, MemoryDoc};
    use crate::types::mission::{Mission, MissionId, MissionStatus};
    use crate::types::project::{Project, ProjectId, system_project_id};
    use crate::types::step::Step;
    use crate::types::thread::{Thread, ThreadId, ThreadState};

    /// Shared in-memory Store implementation for tests.
    ///
    /// Stores all entity types with proper CRUD semantics and filtering by
    /// project_id / user_id. Use this instead of defining per-module mocks.
    pub struct InMemoryStore {
        threads: RwLock<Vec<Thread>>,
        steps: RwLock<Vec<Step>>,
        events: RwLock<Vec<ThreadEvent>>,
        projects: RwLock<Vec<Project>>,
        conversations: RwLock<Vec<ConversationSurface>>,
        docs: RwLock<Vec<MemoryDoc>>,
        leases: RwLock<Vec<CapabilityLease>>,
        missions: RwLock<Vec<Mission>>,
    }

    impl InMemoryStore {
        pub fn new() -> Self {
            Self {
                threads: RwLock::new(Vec::new()),
                steps: RwLock::new(Vec::new()),
                events: RwLock::new(Vec::new()),
                projects: RwLock::new(Vec::new()),
                conversations: RwLock::new(Vec::new()),
                docs: RwLock::new(Vec::new()),
                leases: RwLock::new(Vec::new()),
                missions: RwLock::new(Vec::new()),
            }
        }

        pub fn with_docs(docs: Vec<MemoryDoc>) -> Self {
            Self {
                docs: RwLock::new(docs),
                ..Self::new()
            }
        }
    }

    #[async_trait::async_trait]
    impl Store for InMemoryStore {
        async fn save_thread(&self, thread: &Thread) -> Result<(), EngineError> {
            let mut threads = self.threads.write().await;
            threads.retain(|t| t.id != thread.id);
            threads.push(thread.clone());
            Ok(())
        }
        async fn load_thread(&self, id: ThreadId) -> Result<Option<Thread>, EngineError> {
            Ok(self
                .threads
                .read()
                .await
                .iter()
                .find(|t| t.id == id)
                .cloned())
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
                .iter()
                .filter(|t| t.project_id == project_id && t.user_id == user_id)
                .cloned()
                .collect())
        }
        async fn update_thread_state(
            &self,
            id: ThreadId,
            state: ThreadState,
        ) -> Result<(), EngineError> {
            let mut threads = self.threads.write().await;
            if let Some(t) = threads.iter_mut().find(|t| t.id == id) {
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
        async fn save_project(&self, project: &Project) -> Result<(), EngineError> {
            let mut projects = self.projects.write().await;
            projects.retain(|p| p.id != project.id);
            projects.push(project.clone());
            Ok(())
        }
        async fn load_project(&self, id: ProjectId) -> Result<Option<Project>, EngineError> {
            Ok(self
                .projects
                .read()
                .await
                .iter()
                .find(|p| p.id == id)
                .cloned())
        }
        async fn list_projects(&self, user_id: &str) -> Result<Vec<Project>, EngineError> {
            Ok(self
                .projects
                .read()
                .await
                .iter()
                .filter(|p| p.user_id == user_id)
                .cloned()
                .collect())
        }
        async fn list_all_projects(&self) -> Result<Vec<Project>, EngineError> {
            Ok(self.projects.read().await.iter().cloned().collect())
        }
        async fn save_conversation(
            &self,
            conversation: &ConversationSurface,
        ) -> Result<(), EngineError> {
            let mut conversations = self.conversations.write().await;
            conversations.retain(|c| c.id != conversation.id);
            conversations.push(conversation.clone());
            Ok(())
        }
        async fn load_conversation(
            &self,
            id: ConversationId,
        ) -> Result<Option<ConversationSurface>, EngineError> {
            Ok(self
                .conversations
                .read()
                .await
                .iter()
                .find(|c| c.id == id)
                .cloned())
        }
        async fn list_conversations(
            &self,
            user_id: &str,
        ) -> Result<Vec<ConversationSurface>, EngineError> {
            Ok(self
                .conversations
                .read()
                .await
                .iter()
                .filter(|c| c.user_id == user_id)
                .cloned()
                .collect())
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
        async fn list_memory_docs(
            &self,
            project_id: ProjectId,
            user_id: &str,
        ) -> Result<Vec<MemoryDoc>, EngineError> {
            Ok(self
                .docs
                .read()
                .await
                .iter()
                .filter(|d| d.project_id == project_id && d.user_id == user_id)
                .cloned()
                .collect())
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
        async fn revoke_lease(&self, lease_id: LeaseId, _reason: &str) -> Result<(), EngineError> {
            let mut leases = self.leases.write().await;
            if let Some(l) = leases.iter_mut().find(|l| l.id == lease_id) {
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
        async fn list_missions(
            &self,
            project_id: ProjectId,
            user_id: &str,
        ) -> Result<Vec<Mission>, EngineError> {
            Ok(self
                .missions
                .read()
                .await
                .iter()
                .filter(|m| m.project_id == project_id && m.user_id == user_id)
                .cloned()
                .collect())
        }
        async fn update_mission_status(
            &self,
            id: MissionId,
            status: MissionStatus,
        ) -> Result<(), EngineError> {
            let mut missions = self.missions.write().await;
            if let Some(m) = missions.iter_mut().find(|m| m.id == id) {
                m.status = status;
            }
            Ok(())
        }
        async fn list_all_threads(
            &self,
            project_id: ProjectId,
        ) -> Result<Vec<Thread>, EngineError> {
            Ok(self
                .threads
                .read()
                .await
                .iter()
                .filter(|t| t.project_id == project_id)
                .cloned()
                .collect())
        }
        async fn list_all_missions(
            &self,
            project_id: ProjectId,
        ) -> Result<Vec<Mission>, EngineError> {
            Ok(self
                .missions
                .read()
                .await
                .iter()
                .filter(|m| m.project_id == project_id)
                .cloned()
                .collect())
        }
    }

    struct MinimalStore;

    #[async_trait::async_trait]
    impl Store for MinimalStore {
        async fn save_thread(&self, _thread: &Thread) -> Result<(), EngineError> {
            Ok(())
        }
        async fn load_thread(&self, _id: ThreadId) -> Result<Option<Thread>, EngineError> {
            Ok(None)
        }
        async fn list_threads(
            &self,
            _project_id: ProjectId,
            _user_id: &str,
        ) -> Result<Vec<Thread>, EngineError> {
            Ok(Vec::new())
        }
        async fn update_thread_state(
            &self,
            _id: ThreadId,
            _state: ThreadState,
        ) -> Result<(), EngineError> {
            Ok(())
        }
        async fn save_step(&self, _step: &Step) -> Result<(), EngineError> {
            Ok(())
        }
        async fn load_steps(&self, _thread_id: ThreadId) -> Result<Vec<Step>, EngineError> {
            Ok(Vec::new())
        }
        async fn append_events(&self, _events: &[ThreadEvent]) -> Result<(), EngineError> {
            Ok(())
        }
        async fn load_events(&self, _thread_id: ThreadId) -> Result<Vec<ThreadEvent>, EngineError> {
            Ok(Vec::new())
        }
        async fn save_project(&self, _project: &Project) -> Result<(), EngineError> {
            Ok(())
        }
        async fn load_project(&self, _id: ProjectId) -> Result<Option<Project>, EngineError> {
            Ok(None)
        }
        async fn save_memory_doc(&self, _doc: &MemoryDoc) -> Result<(), EngineError> {
            Ok(())
        }
        async fn load_memory_doc(&self, _id: DocId) -> Result<Option<MemoryDoc>, EngineError> {
            Ok(None)
        }
        async fn list_memory_docs(
            &self,
            _project_id: ProjectId,
            _user_id: &str,
        ) -> Result<Vec<MemoryDoc>, EngineError> {
            Ok(Vec::new())
        }
        async fn save_lease(&self, _lease: &CapabilityLease) -> Result<(), EngineError> {
            Ok(())
        }
        async fn load_active_leases(
            &self,
            _thread_id: ThreadId,
        ) -> Result<Vec<CapabilityLease>, EngineError> {
            Ok(Vec::new())
        }
        async fn revoke_lease(&self, _lease_id: LeaseId, _reason: &str) -> Result<(), EngineError> {
            Ok(())
        }
        async fn save_mission(&self, _mission: &Mission) -> Result<(), EngineError> {
            Ok(())
        }
        async fn load_mission(&self, _id: MissionId) -> Result<Option<Mission>, EngineError> {
            Ok(None)
        }
        async fn list_missions(
            &self,
            _project_id: ProjectId,
            _user_id: &str,
        ) -> Result<Vec<Mission>, EngineError> {
            Ok(Vec::new())
        }
        async fn update_mission_status(
            &self,
            _id: MissionId,
            _status: MissionStatus,
        ) -> Result<(), EngineError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn store_defaults_fail_closed() {
        let store = MinimalStore;
        assert!(matches!(
            store.list_projects("alice").await,
            Err(EngineError::Store { .. })
        ));
        assert!(matches!(
            store.list_all_projects().await,
            Err(EngineError::Store { .. })
        ));
        assert!(matches!(
            store.load_conversation(ConversationId::new()).await,
            Err(EngineError::Store { .. })
        ));
        assert!(matches!(
            store.list_all_threads(ProjectId::new()).await,
            Err(EngineError::Store { .. })
        ));
    }

    #[tokio::test]
    async fn shared_queries_include_legacy_and_current_shared_owner() {
        use crate::types::memory::DocType;
        use crate::types::{LEGACY_SHARED_OWNER_ID, shared_owner_id};

        let project_id = ProjectId::new();
        let mut legacy = MemoryDoc::new(
            project_id,
            LEGACY_SHARED_OWNER_ID,
            DocType::Note,
            "legacy",
            "a",
        );
        let current = MemoryDoc::new(project_id, shared_owner_id(), DocType::Note, "current", "b");
        legacy.id = DocId::new();
        let store = InMemoryStore::with_docs(vec![legacy, current]);

        let docs = store
            .list_memory_docs_with_shared(project_id, "alice")
            .await
            .unwrap();
        assert_eq!(docs.len(), 2);
    }

    /// Regression test for issue #2084.
    ///
    /// Admin skills live in `system_project_id()`. A tenant whose thread runs
    /// in a different per-user project must still see those skills via
    /// `list_shared_memory_docs` / `list_memory_docs_with_shared`.
    #[tokio::test]
    async fn admin_skills_in_system_project_visible_to_all_tenants() {
        use crate::types::memory::DocType;
        use crate::types::shared_owner_id;

        // Admin installs a skill into the system project.
        let admin_skill = MemoryDoc::new(
            system_project_id(),
            shared_owner_id(),
            DocType::Skill,
            "skill:admin-tool",
            "# Admin Tool\nDo admin things.",
        );

        // Alice has her own skill in her own project.
        let alice_project = ProjectId::new();
        let alice_skill = MemoryDoc::new(
            alice_project,
            "alice",
            DocType::Skill,
            "skill:alice-tool",
            "# Alice Tool\nAlice's personal skill.",
        );

        // Bob's thread runs in yet another project.
        let bob_project = ProjectId::new();

        let store = InMemoryStore::with_docs(vec![admin_skill.clone(), alice_skill.clone()]);

        // Bob sees admin skill via list_shared_memory_docs from his project.
        let shared = store
            .list_shared_memory_docs(bob_project)
            .await
            .unwrap();
        assert!(
            shared.iter().any(|d| d.id == admin_skill.id),
            "Bob should see admin skill from system project"
        );

        // Bob does NOT see Alice's personal skill.
        assert!(
            !shared.iter().any(|d| d.id == alice_skill.id),
            "Bob should not see Alice's personal skill"
        );

        // Alice sees both her own skill and the admin skill.
        let alice_docs = store
            .list_memory_docs_with_shared(alice_project, "alice")
            .await
            .unwrap();
        assert!(
            alice_docs.iter().any(|d| d.id == admin_skill.id),
            "Alice should see admin skill"
        );
        assert!(
            alice_docs.iter().any(|d| d.id == alice_skill.id),
            "Alice should see her own skill"
        );

        // Bob's full doc list (own + shared) includes admin skill, not Alice's.
        let bob_docs = store
            .list_memory_docs_with_shared(bob_project, "bob")
            .await
            .unwrap();
        assert!(bob_docs.iter().any(|d| d.id == admin_skill.id));
        assert!(!bob_docs.iter().any(|d| d.id == alice_skill.id));
    }
}
