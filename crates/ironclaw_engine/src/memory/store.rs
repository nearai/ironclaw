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
        user_id: &str,
        doc_type: DocType,
        title: &str,
        content: &str,
    ) -> Result<MemoryDoc, EngineError> {
        let doc = MemoryDoc::new(project_id, user_id, doc_type, title, content);
        self.store.save_memory_doc(&doc).await?;
        Ok(doc)
    }

    /// Create a doc linked to a source thread.
    pub async fn create_doc_from_thread(
        &self,
        project_id: ProjectId,
        user_id: &str,
        doc_type: DocType,
        title: &str,
        content: &str,
        source_thread_id: ThreadId,
    ) -> Result<MemoryDoc, EngineError> {
        let doc = MemoryDoc::new(project_id, user_id, doc_type, title, content)
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
        user_id: &str,
        doc_type: Option<DocType>,
    ) -> Result<Vec<MemoryDoc>, EngineError> {
        let all = self.store.list_memory_docs(project_id, user_id).await?;
        match doc_type {
            Some(dt) => Ok(all.into_iter().filter(|d| d.doc_type == dt).collect()),
            None => Ok(all),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::types::memory::{DocId, DocType};
    use crate::types::project::ProjectId;
    use crate::types::thread::ThreadId;

    use super::MemoryStore;

    fn make_store() -> MemoryStore {
        MemoryStore::new(Arc::new(crate::tests::InMemoryStore::new()))
    }

    // ── Tests ────────────────────────────────────────────────

    #[tokio::test]
    async fn create_doc_and_get() {
        let store = make_store();
        let project_id = ProjectId::new();

        let doc = store
            .create_doc(
                project_id,
                "test-user",
                DocType::Summary,
                "Test Doc",
                "Some content",
            )
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
                "test-user",
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
            .create_doc(project_a, "test-user", DocType::Note, "A1", "content a1")
            .await
            .unwrap();
        store
            .create_doc(project_a, "test-user", DocType::Note, "A2", "content a2")
            .await
            .unwrap();
        store
            .create_doc(project_b, "test-user", DocType::Note, "B1", "content b1")
            .await
            .unwrap();

        let docs_a = store.list_docs(project_a, "test-user", None).await.unwrap();
        assert_eq!(docs_a.len(), 2);
        assert!(docs_a.iter().all(|d| d.project_id == project_a));

        let docs_b = store.list_docs(project_b, "test-user", None).await.unwrap();
        assert_eq!(docs_b.len(), 1);
        assert_eq!(docs_b[0].title, "B1");
    }

    #[tokio::test]
    async fn list_docs_filters_by_type() {
        let store = make_store();
        let project_id = ProjectId::new();

        store
            .create_doc(
                project_id,
                "test-user",
                DocType::Summary,
                "S1",
                "summary content",
            )
            .await
            .unwrap();
        store
            .create_doc(
                project_id,
                "test-user",
                DocType::Lesson,
                "L1",
                "lesson content",
            )
            .await
            .unwrap();
        store
            .create_doc(
                project_id,
                "test-user",
                DocType::Summary,
                "S2",
                "another summary",
            )
            .await
            .unwrap();

        let summaries = store
            .list_docs(project_id, "test-user", Some(DocType::Summary))
            .await
            .unwrap();
        assert_eq!(summaries.len(), 2);
        assert!(summaries.iter().all(|d| d.doc_type == DocType::Summary));

        let lessons = store
            .list_docs(project_id, "test-user", Some(DocType::Lesson))
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
