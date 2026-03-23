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
