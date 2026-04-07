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
use crate::types::{is_shared_owner, shared_owner_candidates};

/// Persistence abstraction for the engine.
#[async_trait::async_trait]
pub trait Store: Send + Sync {
    // ── Thread operations ───────────────────────────────────

    async fn save_thread(&self, thread: &Thread) -> Result<(), EngineError>;
    async fn load_thread(&self, id: ThreadId) -> Result<Option<Thread>, EngineError>;
    async fn list_threads(
        &self,
        project_id: ProjectId,
        user_id: &str,
    ) -> Result<Vec<Thread>, EngineError>;
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
    async fn list_projects(&self, user_id: &str) -> Result<Vec<Project>, EngineError> {
        Err(EngineError::Store {
            reason: format!("Store::list_projects not implemented for user '{user_id}'"),
        })
    }
    async fn list_all_projects(&self) -> Result<Vec<Project>, EngineError> {
        Err(EngineError::Store {
            reason: "Store::list_all_projects not implemented".into(),
        })
    }

    // ── Conversation operations ─────────────────────────────

    async fn save_conversation(
        &self,
        conversation: &ConversationSurface,
    ) -> Result<(), EngineError> {
        Err(EngineError::Store {
            reason: format!(
                "Store::save_conversation not implemented for conversation '{}'",
                conversation.id
            ),
        })
    }
    async fn load_conversation(
        &self,
        id: ConversationId,
    ) -> Result<Option<ConversationSurface>, EngineError> {
        Err(EngineError::Store {
            reason: format!("Store::load_conversation not implemented for '{id}'"),
        })
    }
    async fn list_conversations(
        &self,
        user_id: &str,
    ) -> Result<Vec<ConversationSurface>, EngineError> {
        Err(EngineError::Store {
            reason: format!("Store::list_conversations not implemented for user '{user_id}'"),
        })
    }

    // ── Memory doc operations ───────────────────────────────

    async fn save_memory_doc(&self, doc: &MemoryDoc) -> Result<(), EngineError>;
    async fn load_memory_doc(&self, id: DocId) -> Result<Option<MemoryDoc>, EngineError>;
    async fn list_memory_docs(
        &self,
        project_id: ProjectId,
        user_id: &str,
    ) -> Result<Vec<MemoryDoc>, EngineError>;

    /// List memory docs visible to a user: their own docs + shared docs.
    ///
    /// This is the "shared space" pattern: admins can install skills and
    /// knowledge under the shared owner id, and they're visible to all users
    /// alongside their personal docs. Used for skill listing, context
    /// retrieval, and any place where shared knowledge should be accessible.
    async fn list_memory_docs_with_shared(
        &self,
        project_id: ProjectId,
        user_id: &str,
    ) -> Result<Vec<MemoryDoc>, EngineError> {
        if is_shared_owner(user_id) {
            return self.list_shared_memory_docs(project_id).await;
        }
        let mut docs = self.list_memory_docs(project_id, user_id).await?;
        docs.extend(self.list_shared_memory_docs(project_id).await?);
        Ok(docs)
    }

    async fn list_shared_memory_docs(
        &self,
        project_id: ProjectId,
    ) -> Result<Vec<MemoryDoc>, EngineError> {
        let mut docs = Vec::new();
        for owner_id in shared_owner_candidates() {
            docs.extend(self.list_memory_docs(project_id, owner_id).await?);
        }
        docs.sort_by_key(|doc| doc.id.0);
        docs.dedup_by_key(|doc| doc.id);
        Ok(docs)
    }

    /// List all skill docs owned by shared/admin owners across ALL projects.
    ///
    /// Unlike `list_shared_memory_docs`, this ignores `project_id` — it returns
    /// every `DocType::Skill` with a shared owner regardless of which project it
    /// lives in. Used by `__list_skills__` so that admin-installed skills
    /// (migrated into the owner's default project) are visible to gateway users
    /// whose threads run in a per-user project.
    async fn list_skills_global(&self) -> Result<Vec<MemoryDoc>, EngineError> {
        let mut docs = Vec::new();
        for owner_id in shared_owner_candidates() {
            docs.extend(self.list_memory_docs_by_owner(owner_id).await?);
        }
        docs.retain(|d| d.doc_type == crate::types::memory::DocType::Skill);
        docs.sort_by_key(|d| d.id.0);
        docs.dedup_by_key(|d| d.id);
        Ok(docs)
    }

    /// List all memory docs owned by a specific user across ALL projects.
    ///
    /// Default implementation fans out over ALL projects via `list_all_projects`.
    /// This is necessary for shared owners (`__shared__`, `system`) who don't own
    /// any projects themselves but have docs stored inside other users' projects.
    /// Implementors with a flat store should override for efficiency.
    async fn list_memory_docs_by_owner(
        &self,
        user_id: &str,
    ) -> Result<Vec<MemoryDoc>, EngineError> {
        let projects = self.list_all_projects().await?;
        let mut docs = Vec::new();
        for project in projects {
            docs.extend(self.list_memory_docs(project.id, user_id).await?);
        }
        Ok(docs)
    }

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
    async fn list_missions(
        &self,
        project_id: ProjectId,
        user_id: &str,
    ) -> Result<Vec<Mission>, EngineError>;
    async fn update_mission_status(
        &self,
        id: MissionId,
        status: MissionStatus,
    ) -> Result<(), EngineError>;

    /// List missions visible to a user: their own + shared missions.
    ///
    /// Shared learning missions (self-improvement, skill-extraction, etc.) are
    /// created under the shared owner id and should be visible/manageable by all
    /// users through the API.
    async fn list_missions_with_shared(
        &self,
        project_id: ProjectId,
        user_id: &str,
    ) -> Result<Vec<Mission>, EngineError> {
        if is_shared_owner(user_id) {
            return self.list_shared_missions(project_id).await;
        }
        let mut missions = self.list_missions(project_id, user_id).await?;
        missions.extend(self.list_shared_missions(project_id).await?);
        Ok(missions)
    }

    async fn list_shared_missions(
        &self,
        project_id: ProjectId,
    ) -> Result<Vec<Mission>, EngineError> {
        let mut missions = Vec::new();
        for owner_id in shared_owner_candidates() {
            missions.extend(self.list_missions(project_id, owner_id).await?);
        }
        missions.sort_by_key(|mission| mission.id.0);
        missions.dedup_by_key(|mission| mission.id);
        Ok(missions)
    }

    // ── Admin operations (system-level, cross-tenant) ──────────

    /// List all threads in a project regardless of user.
    /// Used by: recovery, background thread resume at startup.
    async fn list_all_threads(&self, project_id: ProjectId) -> Result<Vec<Thread>, EngineError> {
        Err(EngineError::Store {
            reason: format!("Store::list_all_threads not implemented for project '{project_id}'"),
        })
    }

    /// List all missions in a project regardless of user.
    /// Used by: cron ticker, event listener, bootstrap.
    async fn list_all_missions(&self, project_id: ProjectId) -> Result<Vec<Mission>, EngineError> {
        Err(EngineError::Store {
            reason: format!("Store::list_all_missions not implemented for project '{project_id}'"),
        })
    }
}
