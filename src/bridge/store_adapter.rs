//! In-memory store adapter — implements `ironclaw_engine::Store` without database tables.
//!
//! Phase 6: threads and state live in memory during execution. Persistent
//! storage comes in Phase 7 when we add database migrations.

use std::collections::HashMap;

use tokio::sync::RwLock;

use ironclaw_engine::{
    CapabilityLease, DocId, EngineError, LeaseId, MemoryDoc, Project, ProjectId, Step, Thread,
    ThreadEvent, ThreadId, ThreadState, Store,
};

/// In-memory implementation of the engine's `Store` trait.
///
/// All state is discarded when the agent process restarts. This is
/// sufficient for Phase 6 (proving the engine works end-to-end).
pub struct InMemoryStore {
    threads: RwLock<HashMap<ThreadId, Thread>>,
    steps: RwLock<HashMap<ThreadId, Vec<Step>>>,
    events: RwLock<HashMap<ThreadId, Vec<ThreadEvent>>>,
    projects: RwLock<HashMap<ProjectId, Project>>,
    docs: RwLock<HashMap<DocId, MemoryDoc>>,
    leases: RwLock<HashMap<LeaseId, CapabilityLease>>,
}

impl InMemoryStore {
    pub fn new() -> Self {
        Self {
            threads: RwLock::new(HashMap::new()),
            steps: RwLock::new(HashMap::new()),
            events: RwLock::new(HashMap::new()),
            projects: RwLock::new(HashMap::new()),
            docs: RwLock::new(HashMap::new()),
            leases: RwLock::new(HashMap::new()),
        }
    }
}

impl Default for InMemoryStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl Store for InMemoryStore {
    // ── Thread ──────────────────────────────────────────────

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
            .filter(|t| t.project_id == project_id)
            .cloned()
            .collect())
    }

    async fn update_thread_state(
        &self,
        id: ThreadId,
        state: ThreadState,
    ) -> Result<(), EngineError> {
        if let Some(thread) = self.threads.write().await.get_mut(&id) {
            thread.state = state;
        }
        Ok(())
    }

    // ── Step ────────────────────────────────────────────────

    async fn save_step(&self, step: &Step) -> Result<(), EngineError> {
        self.steps
            .write()
            .await
            .entry(step.thread_id)
            .or_default()
            .push(step.clone());
        Ok(())
    }

    async fn load_steps(&self, thread_id: ThreadId) -> Result<Vec<Step>, EngineError> {
        Ok(self
            .steps
            .read()
            .await
            .get(&thread_id)
            .cloned()
            .unwrap_or_default())
    }

    // ── Event ───────────────────────────────────────────────

    async fn append_events(&self, events: &[ThreadEvent]) -> Result<(), EngineError> {
        let mut store = self.events.write().await;
        for event in events {
            store.entry(event.thread_id).or_default().push(event.clone());
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

    // ── Project ─────────────────────────────────────────────

    async fn save_project(&self, project: &Project) -> Result<(), EngineError> {
        self.projects
            .write()
            .await
            .insert(project.id, project.clone());
        Ok(())
    }

    async fn load_project(&self, id: ProjectId) -> Result<Option<Project>, EngineError> {
        Ok(self.projects.read().await.get(&id).cloned())
    }

    // ── MemoryDoc ───────────────────────────────────────────

    async fn save_memory_doc(&self, doc: &MemoryDoc) -> Result<(), EngineError> {
        self.docs.write().await.insert(doc.id, doc.clone());
        Ok(())
    }

    async fn load_memory_doc(&self, id: DocId) -> Result<Option<MemoryDoc>, EngineError> {
        Ok(self.docs.read().await.get(&id).cloned())
    }

    async fn list_memory_docs(&self, project_id: ProjectId) -> Result<Vec<MemoryDoc>, EngineError> {
        Ok(self
            .docs
            .read()
            .await
            .values()
            .filter(|d| d.project_id == project_id)
            .cloned()
            .collect())
    }

    // ── Lease ───────────────────────────────────────────────

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
            .filter(|l| l.thread_id == thread_id && l.is_valid())
            .cloned()
            .collect())
    }

    async fn revoke_lease(&self, lease_id: LeaseId, _reason: &str) -> Result<(), EngineError> {
        if let Some(lease) = self.leases.write().await.get_mut(&lease_id) {
            lease.revoked = true;
        }
        Ok(())
    }
}
