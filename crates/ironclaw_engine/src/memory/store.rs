//! Project-scoped memory document operations.

use std::sync::Arc;

use crate::traits::store::Store;
use crate::types::error::EngineError;
use crate::types::memory::{DocId, DocType, MemoryDoc};
use crate::types::project::ProjectId;
use crate::types::thread::ThreadId;

/// Thin wrapper over the [`Store`] trait for project-scoped doc operations.
pub struct MemoryStore {
    store: Arc<dyn Store>,
}

impl MemoryStore {
    pub fn new(store: Arc<dyn Store>) -> Self {
        Self { store }
    }

    /// Create a new memory document.
    pub async fn create_doc(
        &self,
        project_id: ProjectId,
        doc_type: DocType,
        title: &str,
        content: &str,
    ) -> Result<MemoryDoc, EngineError> {
        let doc = MemoryDoc::new(project_id, doc_type, title, content);
        self.store.save_memory_doc(&doc).await?;
        Ok(doc)
    }

    /// Create a doc linked to a source thread.
    pub async fn create_doc_from_thread(
        &self,
        project_id: ProjectId,
        doc_type: DocType,
        title: &str,
        content: &str,
        source_thread_id: ThreadId,
    ) -> Result<MemoryDoc, EngineError> {
        let doc = MemoryDoc::new(project_id, doc_type, title, content)
            .with_source_thread(source_thread_id);
        self.store.save_memory_doc(&doc).await?;
        Ok(doc)
    }

    /// Load a single doc by ID.
    pub async fn get_doc(&self, id: DocId) -> Result<Option<MemoryDoc>, EngineError> {
        self.store.load_memory_doc(id).await
    }

    /// List all docs in a project, optionally filtered by type.
    pub async fn list_docs(
        &self,
        project_id: ProjectId,
        doc_type: Option<DocType>,
    ) -> Result<Vec<MemoryDoc>, EngineError> {
        let all = self.store.list_memory_docs(project_id).await?;
        match doc_type {
            Some(dt) => Ok(all.into_iter().filter(|d| d.doc_type == dt).collect()),
            None => Ok(all),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tokio::sync::RwLock;

    use crate::traits::store::Store;
    use crate::types::capability::{CapabilityLease, LeaseId};
    use crate::types::error::EngineError;
    use crate::types::event::ThreadEvent;
    use crate::types::memory::{DocId, DocType, MemoryDoc};
    use crate::types::mission::{Mission, MissionId, MissionStatus};
    use crate::types::project::{Project, ProjectId};
    use crate::types::step::Step;
    use crate::types::thread::{Thread, ThreadId, ThreadState};

    use super::MemoryStore;

    // ── In-memory Store implementation ───────────────────────

    struct InMemoryDocStore {
        docs: RwLock<Vec<MemoryDoc>>,
        threads: RwLock<Vec<Thread>>,
        steps: RwLock<Vec<Step>>,
        events: RwLock<Vec<ThreadEvent>>,
        projects: RwLock<Vec<Project>>,
        leases: RwLock<Vec<CapabilityLease>>,
        missions: RwLock<Vec<Mission>>,
    }

    impl InMemoryDocStore {
        fn new() -> Self {
            Self {
                docs: RwLock::new(Vec::new()),
                threads: RwLock::new(Vec::new()),
                steps: RwLock::new(Vec::new()),
                events: RwLock::new(Vec::new()),
                projects: RwLock::new(Vec::new()),
                leases: RwLock::new(Vec::new()),
                missions: RwLock::new(Vec::new()),
            }
        }
    }

    #[async_trait::async_trait]
    impl Store for InMemoryDocStore {
        // ── Thread operations ────────────────────────────────

        async fn save_thread(&self, thread: &Thread) -> Result<(), EngineError> {
            let mut threads = self.threads.write().await;
            threads.retain(|t| t.id != thread.id);
            threads.push(thread.clone());
            Ok(())
        }

        async fn load_thread(&self, id: ThreadId) -> Result<Option<Thread>, EngineError> {
            let threads = self.threads.read().await;
            Ok(threads.iter().find(|t| t.id == id).cloned())
        }

        async fn list_threads(&self, project_id: ProjectId) -> Result<Vec<Thread>, EngineError> {
            let threads = self.threads.read().await;
            Ok(threads
                .iter()
                .filter(|t| t.project_id == project_id)
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

        // ── Step operations ──────────────────────────────────

        async fn save_step(&self, step: &Step) -> Result<(), EngineError> {
            let mut steps = self.steps.write().await;
            steps.retain(|s| s.id != step.id);
            steps.push(step.clone());
            Ok(())
        }

        async fn load_steps(&self, thread_id: ThreadId) -> Result<Vec<Step>, EngineError> {
            let steps = self.steps.read().await;
            Ok(steps
                .iter()
                .filter(|s| s.thread_id == thread_id)
                .cloned()
                .collect())
        }

        // ── Event operations ─────────────────────────────────

        async fn append_events(&self, events: &[ThreadEvent]) -> Result<(), EngineError> {
            let mut stored = self.events.write().await;
            stored.extend(events.iter().cloned());
            Ok(())
        }

        async fn load_events(&self, thread_id: ThreadId) -> Result<Vec<ThreadEvent>, EngineError> {
            let events = self.events.read().await;
            Ok(events
                .iter()
                .filter(|e| e.thread_id == thread_id)
                .cloned()
                .collect())
        }

        // ── Project operations ───────────────────────────────

        async fn save_project(&self, project: &Project) -> Result<(), EngineError> {
            let mut projects = self.projects.write().await;
            projects.retain(|p| p.id != project.id);
            projects.push(project.clone());
            Ok(())
        }

        async fn load_project(&self, id: ProjectId) -> Result<Option<Project>, EngineError> {
            let projects = self.projects.read().await;
            Ok(projects.iter().find(|p| p.id == id).cloned())
        }

        // ── Memory doc operations ────────────────────────────

        async fn save_memory_doc(&self, doc: &MemoryDoc) -> Result<(), EngineError> {
            let mut docs = self.docs.write().await;
            docs.push(doc.clone());
            Ok(())
        }

        async fn load_memory_doc(&self, id: DocId) -> Result<Option<MemoryDoc>, EngineError> {
            let docs = self.docs.read().await;
            Ok(docs.iter().find(|d| d.id == id).cloned())
        }

        async fn list_memory_docs(
            &self,
            project_id: ProjectId,
        ) -> Result<Vec<MemoryDoc>, EngineError> {
            let docs = self.docs.read().await;
            Ok(docs
                .iter()
                .filter(|d| d.project_id == project_id)
                .cloned()
                .collect())
        }

        // ── Capability lease operations ──────────────────────

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
            let leases = self.leases.read().await;
            Ok(leases
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

        // ── Mission operations ───────────────────────────────

        async fn save_mission(&self, mission: &Mission) -> Result<(), EngineError> {
            let mut missions = self.missions.write().await;
            missions.retain(|m| m.id != mission.id);
            missions.push(mission.clone());
            Ok(())
        }

        async fn load_mission(&self, id: MissionId) -> Result<Option<Mission>, EngineError> {
            let missions = self.missions.read().await;
            Ok(missions.iter().find(|m| m.id == id).cloned())
        }

        async fn list_missions(&self, project_id: ProjectId) -> Result<Vec<Mission>, EngineError> {
            let missions = self.missions.read().await;
            Ok(missions
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

    fn make_store() -> MemoryStore {
        MemoryStore::new(Arc::new(InMemoryDocStore::new()))
    }

    // ── Tests ────────────────────────────────────────────────

    #[tokio::test]
    async fn create_doc_and_get() {
        let store = make_store();
        let project_id = ProjectId::new();

        let doc = store
            .create_doc(project_id, DocType::Summary, "Test Doc", "Some content")
            .await
            .unwrap();

        assert_eq!(doc.title, "Test Doc");
        assert_eq!(doc.content, "Some content");
        assert_eq!(doc.doc_type, DocType::Summary);
        assert_eq!(doc.project_id, project_id);
        assert!(doc.source_thread_id.is_none());

        let loaded = store.get_doc(doc.id).await.unwrap();
        let loaded = loaded.unwrap();
        assert_eq!(loaded.id, doc.id);
        assert_eq!(loaded.title, "Test Doc");
        assert_eq!(loaded.content, "Some content");
    }

    #[tokio::test]
    async fn create_doc_from_thread_links_source() {
        let store = make_store();
        let project_id = ProjectId::new();
        let thread_id = ThreadId::new();

        let doc = store
            .create_doc_from_thread(
                project_id,
                DocType::Lesson,
                "Thread Lesson",
                "Learned something",
                thread_id,
            )
            .await
            .unwrap();

        assert_eq!(doc.source_thread_id, Some(thread_id));
        assert_eq!(doc.doc_type, DocType::Lesson);

        let loaded = store.get_doc(doc.id).await.unwrap().unwrap();
        assert_eq!(loaded.source_thread_id, Some(thread_id));
    }

    #[tokio::test]
    async fn list_docs_by_project() {
        let store = make_store();
        let project_a = ProjectId::new();
        let project_b = ProjectId::new();

        store
            .create_doc(project_a, DocType::Note, "A1", "content a1")
            .await
            .unwrap();
        store
            .create_doc(project_a, DocType::Note, "A2", "content a2")
            .await
            .unwrap();
        store
            .create_doc(project_b, DocType::Note, "B1", "content b1")
            .await
            .unwrap();

        let docs_a = store.list_docs(project_a, None).await.unwrap();
        assert_eq!(docs_a.len(), 2);
        assert!(docs_a.iter().all(|d| d.project_id == project_a));

        let docs_b = store.list_docs(project_b, None).await.unwrap();
        assert_eq!(docs_b.len(), 1);
        assert_eq!(docs_b[0].title, "B1");
    }

    #[tokio::test]
    async fn list_docs_filters_by_type() {
        let store = make_store();
        let project_id = ProjectId::new();

        store
            .create_doc(project_id, DocType::Summary, "S1", "summary content")
            .await
            .unwrap();
        store
            .create_doc(project_id, DocType::Lesson, "L1", "lesson content")
            .await
            .unwrap();
        store
            .create_doc(project_id, DocType::Summary, "S2", "another summary")
            .await
            .unwrap();

        let summaries = store
            .list_docs(project_id, Some(DocType::Summary))
            .await
            .unwrap();
        assert_eq!(summaries.len(), 2);
        assert!(summaries.iter().all(|d| d.doc_type == DocType::Summary));

        let lessons = store
            .list_docs(project_id, Some(DocType::Lesson))
            .await
            .unwrap();
        assert_eq!(lessons.len(), 1);
        assert_eq!(lessons[0].title, "L1");
    }

    #[tokio::test]
    async fn get_nonexistent_returns_none() {
        let store = make_store();
        let result = store.get_doc(DocId::new()).await.unwrap();
        assert!(result.is_none());
    }
}
