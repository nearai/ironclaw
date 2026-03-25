//! Storage trait for engine persistence.
//!
//! Defines CRUD operations for all engine types. The main crate implements
//! this by wrapping its dual-backend `Database` trait (PostgreSQL + libSQL).

use crate::types::capability::{CapabilityLease, LeaseId};
use crate::types::conversation::{ConversationId, ConversationSurface};
use crate::types::error::EngineError;
use crate::types::event::ThreadEvent;
use crate::types::memory::{DocId, MemoryDoc};
use crate::types::mission::{Mission, MissionId, MissionStatus};
use crate::types::project::{Project, ProjectId};
use crate::types::step::Step;
use crate::types::thread::{Thread, ThreadId, ThreadState};

/// Persistence abstraction for the engine.
#[async_trait::async_trait]
pub trait Store: Send + Sync {
    // ── Thread operations ───────────────────────────────────

    async fn save_thread(&self, thread: &Thread) -> Result<(), EngineError>;
    async fn load_thread(&self, id: ThreadId) -> Result<Option<Thread>, EngineError>;
    async fn list_threads(&self, project_id: ProjectId) -> Result<Vec<Thread>, EngineError>;
    async fn update_thread_state(
        &self,
        id: ThreadId,
        state: ThreadState,
    ) -> Result<(), EngineError>;

    // ── Step operations ─────────────────────────────────────

    async fn save_step(&self, step: &Step) -> Result<(), EngineError>;
    async fn load_steps(&self, thread_id: ThreadId) -> Result<Vec<Step>, EngineError>;

    // ── Event operations ────────────────────────────────────

    async fn append_events(&self, events: &[ThreadEvent]) -> Result<(), EngineError>;
    async fn load_events(&self, thread_id: ThreadId) -> Result<Vec<ThreadEvent>, EngineError>;

    // ── Project operations ──────────────────────────────────

    async fn save_project(&self, project: &Project) -> Result<(), EngineError>;
    async fn load_project(&self, id: ProjectId) -> Result<Option<Project>, EngineError>;
    async fn list_projects(&self) -> Result<Vec<Project>, EngineError> {
        Ok(Vec::new())
    }

    // ── Conversation operations ─────────────────────────────

    async fn save_conversation(
        &self,
        conversation: &ConversationSurface,
    ) -> Result<(), EngineError> {
        let _ = conversation;
        Ok(())
    }
    async fn load_conversation(
        &self,
        id: ConversationId,
    ) -> Result<Option<ConversationSurface>, EngineError> {
        let _ = id;
        Ok(None)
    }
    async fn list_conversations(
        &self,
        user_id: &str,
    ) -> Result<Vec<ConversationSurface>, EngineError> {
        let _ = user_id;
        Ok(Vec::new())
    }

    // ── Memory doc operations ───────────────────────────────

    async fn save_memory_doc(&self, doc: &MemoryDoc) -> Result<(), EngineError>;
    async fn load_memory_doc(&self, id: DocId) -> Result<Option<MemoryDoc>, EngineError>;
    async fn list_memory_docs(&self, project_id: ProjectId) -> Result<Vec<MemoryDoc>, EngineError>;

    // ── Capability lease operations ─────────────────────────

    async fn save_lease(&self, lease: &CapabilityLease) -> Result<(), EngineError>;
    async fn load_active_leases(
        &self,
        thread_id: ThreadId,
    ) -> Result<Vec<CapabilityLease>, EngineError>;
    async fn revoke_lease(&self, lease_id: LeaseId, reason: &str) -> Result<(), EngineError>;

    // ── Mission operations ───────────────────────────────────

    async fn save_mission(&self, mission: &Mission) -> Result<(), EngineError>;
    async fn load_mission(&self, id: MissionId) -> Result<Option<Mission>, EngineError>;
    async fn list_missions(&self, project_id: ProjectId) -> Result<Vec<Mission>, EngineError>;
    async fn update_mission_status(
        &self,
        id: MissionId,
        status: MissionStatus,
    ) -> Result<(), EngineError>;
}
