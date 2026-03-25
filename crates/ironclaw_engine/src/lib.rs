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
pub mod memory;
pub mod reflection;
pub mod reliability;
pub mod runtime;
pub mod traits;
pub mod types;

// ── Re-exports: types ───────────────────────────────────────

pub use types::capability::{
    ActionDef, Capability, CapabilityLease, EffectType, LeaseId, PolicyCondition, PolicyEffect,
    PolicyRule,
};
pub use types::error::{CapabilityError, EngineError, StepError, ThreadError};
pub use types::event::{EventId, EventKind, ThreadEvent};
pub use types::memory::{DocId, DocType, MemoryDoc};
pub use types::message::{MessageRole, ThreadMessage};
pub use types::mission::{Mission, MissionCadence, MissionId, MissionStatus};
pub use types::project::{Project, ProjectId};
pub use types::provenance::Provenance;
pub use types::step::{
    ActionCall, ActionResult, ExecutionTier, LlmResponse, Step, StepId, StepStatus, TokenUsage,
};
pub use types::thread::{Thread, ThreadConfig, ThreadId, ThreadState, ThreadType};

// ── Re-exports: traits ──────────────────────────────────────

pub use traits::effect::{EffectExecutor, ThreadExecutionContext};
pub use traits::llm::{LlmBackend, LlmCallConfig, LlmOutput};
pub use traits::store::Store;

// ── Re-exports: capability ────────────────────────────────────

pub use capability::lease::LeaseManager;
pub use capability::planner::{CapabilityGrantPlan, LeasePlanner};
pub use capability::policy::{PolicyDecision, PolicyEngine};
pub use capability::registry::CapabilityRegistry;

// ── Re-exports: runtime ───────────────────────────────────────

pub use runtime::conversation::ConversationManager;
pub use runtime::manager::ThreadManager;
pub use runtime::messaging::ThreadOutcome;
pub use runtime::mission::MissionManager;
pub use runtime::tree::ThreadTree;

pub use types::conversation::{
    ConversationEntry, ConversationId, ConversationSurface, EntrySender,
};

// ── Re-exports: executor ──────────────────────────────────────

pub use executor::ExecutionLoop;

// ── Re-exports: memory ────────────────────────────────────────

pub use memory::MemoryStore;
pub use memory::RetrievalEngine;

// ── Re-exports: reflection ────────────────────────────────────

pub use reflection::ReflectionResult;

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
    use crate::types::project::{Project, ProjectId};
    use crate::types::step::Step;
    use crate::types::thread::{Thread, ThreadId, ThreadState};

    /// Shared in-memory Store implementation for tests.
    pub struct InMemoryStore {
        docs: RwLock<Vec<MemoryDoc>>,
        missions: RwLock<Vec<Mission>>,
    }

    impl InMemoryStore {
        pub fn with_docs(docs: Vec<MemoryDoc>) -> Self {
            Self {
                docs: RwLock::new(docs),
                missions: RwLock::new(Vec::new()),
            }
        }
    }

    #[async_trait::async_trait]
    impl Store for InMemoryStore {
        async fn save_thread(&self, _: &Thread) -> Result<(), EngineError> {
            Ok(())
        }
        async fn load_thread(&self, _: ThreadId) -> Result<Option<Thread>, EngineError> {
            Ok(None)
        }
        async fn list_threads(&self, _: ProjectId) -> Result<Vec<Thread>, EngineError> {
            Ok(vec![])
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
        async fn append_events(&self, _: &[ThreadEvent]) -> Result<(), EngineError> {
            Ok(())
        }
        async fn load_events(&self, _: ThreadId) -> Result<Vec<ThreadEvent>, EngineError> {
            Ok(vec![])
        }
        async fn save_project(&self, _: &Project) -> Result<(), EngineError> {
            Ok(())
        }
        async fn load_project(&self, _: ProjectId) -> Result<Option<Project>, EngineError> {
            Ok(None)
        }
        async fn list_projects(&self) -> Result<Vec<Project>, EngineError> {
            Ok(vec![])
        }
        async fn save_conversation(&self, _: &ConversationSurface) -> Result<(), EngineError> {
            Ok(())
        }
        async fn load_conversation(
            &self,
            _: ConversationId,
        ) -> Result<Option<ConversationSurface>, EngineError> {
            Ok(None)
        }
        async fn list_conversations(
            &self,
            _: &str,
        ) -> Result<Vec<ConversationSurface>, EngineError> {
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
        async fn list_memory_docs(
            &self,
            project_id: ProjectId,
        ) -> Result<Vec<MemoryDoc>, EngineError> {
            Ok(self
                .docs
                .read()
                .await
                .iter()
                .filter(|d| d.project_id == project_id)
                .cloned()
                .collect())
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
        async fn revoke_lease(&self, _: LeaseId, _: &str) -> Result<(), EngineError> {
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
        async fn list_missions(&self, project_id: ProjectId) -> Result<Vec<Mission>, EngineError> {
            Ok(self
                .missions
                .read()
                .await
                .iter()
                .filter(|m| m.project_id == project_id)
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
    }
}
