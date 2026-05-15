//! Filesystem-backed implementation of [`WorkspaceStore`].
//!
//! Routes memory documents, chunks, embeddings, and document-version
//! history through the unified [`RootFilesystem`] surface. Mirrors the
//! canonical migration shape from `crates/ironclaw_memory/src/repo/filesystem.rs`
//! while keeping every method on the legacy [`WorkspaceStore`] trait so the
//! composite [`Database`] supertrait can be satisfied by a single
//! filesystem mount.
//!
//! ## Path layout
//!
//! - `/workspace/documents/<user_id>/<doc_id>` — document records.
//! - `/workspace/chunks/<doc_id>/<chunk_index>` — chunked content with
//!   FTS + Vector indexed projections.
//! - `/workspace/versions/<doc_id>/<version>` — full document content
//!   archived at each save point.
//! - `/workspace/path-index/<user_id>/<agent_id>/<encoded_path>` —
//!   pointer file mapping `(user_id, agent_id, path)` to a `doc_id`.
//!   Lets `get_document_by_path` resolve without scanning the user's
//!   tree on every call.
//!
//! ## Indexed projections
//!
//! Every stored entry carries `user_id`, `agent_id`, and `kind` keys so
//! `Filter::Eq` + `Filter::And` queries can scope by user. Chunk entries
//! additionally carry `content` (for [`IndexKind::Fts`]) and `embedding`
//! (for [`IndexKind::Vector`]) projections, served natively by the
//! libSQL FTS5 / Postgres tsvector + vector indexes that the filesystem
//! backends manage.
//!
//! ## Trust boundary
//!
//! The trait's "Versioning" and "Metadata" sections accept bare document
//! UUIDs without a `user_id` guard — see the trait docstring in
//! `src/db/mod.rs`. This facade respects that contract: those methods
//! trust the caller to have resolved a `doc_id` through a user-scoped
//! lookup first. The path layout still puts the version body under the
//! document's tree, so a delete on the document subtree clears versions
//! too.

mod chunks;
mod documents;
mod paths;
mod search;
mod versions;

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_filesystem::RootFilesystem;
use uuid::Uuid;

use crate::db::WorkspaceStore;
use crate::error::WorkspaceError;
use crate::workspace::{
    ChunkWrite, DocumentVersion, MemoryChunk, MemoryDocument, SearchConfig, SearchResult,
    VersionSummary, WorkspaceEntry,
};

/// Filesystem-backed [`WorkspaceStore`].
///
/// Construct with any shared [`RootFilesystem`]. Tests can use
/// [`InMemoryBackend`] which serves the full surface including
/// [`IndexKind::Fts`] and [`IndexKind::Vector`].
pub struct FilesystemWorkspaceStore<F>
where
    F: RootFilesystem,
{
    pub(crate) filesystem: Arc<F>,
}

impl<F> FilesystemWorkspaceStore<F>
where
    F: RootFilesystem,
{
    pub fn new(filesystem: Arc<F>) -> Self {
        Self { filesystem }
    }
}

#[async_trait]
impl<F> WorkspaceStore for FilesystemWorkspaceStore<F>
where
    F: RootFilesystem + 'static,
{
    async fn get_document_by_path(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        path: &str,
    ) -> Result<MemoryDocument, WorkspaceError> {
        documents::get_by_path(self, user_id, agent_id, path).await
    }

    async fn get_document_by_id(&self, id: Uuid) -> Result<MemoryDocument, WorkspaceError> {
        documents::get_by_id(self, id).await
    }

    async fn get_or_create_document_by_path(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        path: &str,
    ) -> Result<MemoryDocument, WorkspaceError> {
        documents::get_or_create(self, user_id, agent_id, path).await
    }

    async fn update_document(&self, id: Uuid, content: &str) -> Result<(), WorkspaceError> {
        documents::update_content(self, id, content).await
    }

    async fn delete_document_by_path(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        path: &str,
    ) -> Result<(), WorkspaceError> {
        documents::delete_by_path(self, user_id, agent_id, path).await
    }

    async fn list_directory(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        directory: &str,
    ) -> Result<Vec<WorkspaceEntry>, WorkspaceError> {
        documents::list_directory(self, user_id, agent_id, directory).await
    }

    async fn list_all_paths(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
    ) -> Result<Vec<String>, WorkspaceError> {
        documents::list_all_paths(self, user_id, agent_id).await
    }

    async fn list_documents(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
    ) -> Result<Vec<MemoryDocument>, WorkspaceError> {
        documents::list_documents(self, user_id, agent_id).await
    }

    async fn delete_chunks(&self, document_id: Uuid) -> Result<(), WorkspaceError> {
        chunks::delete_all(self, document_id).await
    }

    async fn insert_chunk(
        &self,
        document_id: Uuid,
        chunk_index: i32,
        content: &str,
        embedding: Option<&[f32]>,
    ) -> Result<Uuid, WorkspaceError> {
        chunks::insert(self, document_id, chunk_index, content, embedding).await
    }

    async fn replace_chunks(
        &self,
        document_id: Uuid,
        chunks_in: &[ChunkWrite],
    ) -> Result<(), WorkspaceError> {
        chunks::replace_all(self, document_id, chunks_in).await
    }

    async fn update_chunk_embedding(
        &self,
        chunk_id: Uuid,
        embedding: &[f32],
    ) -> Result<(), WorkspaceError> {
        chunks::update_embedding(self, chunk_id, embedding).await
    }

    async fn get_chunks_without_embeddings(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        limit: usize,
    ) -> Result<Vec<MemoryChunk>, WorkspaceError> {
        chunks::list_without_embeddings(self, user_id, agent_id, limit).await
    }

    async fn hybrid_search(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        query: &str,
        embedding: Option<&[f32]>,
        config: &SearchConfig,
    ) -> Result<Vec<SearchResult>, WorkspaceError> {
        search::hybrid_search(self, user_id, agent_id, query, embedding, config).await
    }

    async fn update_document_metadata(
        &self,
        id: Uuid,
        metadata: &serde_json::Value,
    ) -> Result<(), WorkspaceError> {
        documents::update_metadata(self, id, metadata).await
    }

    async fn find_config_documents(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
    ) -> Result<Vec<MemoryDocument>, WorkspaceError> {
        documents::find_config_documents(self, user_id, agent_id).await
    }

    async fn save_version(
        &self,
        document_id: Uuid,
        content: &str,
        content_hash: &str,
        changed_by: Option<&str>,
    ) -> Result<i32, WorkspaceError> {
        versions::save(self, document_id, content, content_hash, changed_by).await
    }

    async fn get_version(
        &self,
        document_id: Uuid,
        version: i32,
    ) -> Result<DocumentVersion, WorkspaceError> {
        versions::get(self, document_id, version).await
    }

    async fn list_versions(
        &self,
        document_id: Uuid,
        limit: i64,
    ) -> Result<Vec<VersionSummary>, WorkspaceError> {
        versions::list(self, document_id, limit).await
    }

    async fn get_latest_version_number(
        &self,
        document_id: Uuid,
    ) -> Result<Option<i32>, WorkspaceError> {
        versions::get_latest_number(self, document_id).await
    }

    async fn prune_versions(
        &self,
        document_id: Uuid,
        keep_count: i32,
    ) -> Result<u64, WorkspaceError> {
        versions::prune(self, document_id, keep_count).await
    }
}

pub(crate) fn fs_to_workspace_error(error: ironclaw_filesystem::FilesystemError) -> WorkspaceError {
    WorkspaceError::IoError {
        reason: format!("filesystem error: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_filesystem::InMemoryBackend;

    pub(crate) fn store() -> FilesystemWorkspaceStore<InMemoryBackend> {
        FilesystemWorkspaceStore::new(Arc::new(InMemoryBackend::new()))
    }

    #[tokio::test]
    async fn get_or_create_document_round_trips() {
        let s = store();
        let doc = s
            .get_or_create_document_by_path("alice", None, "notes/welcome.md")
            .await
            .unwrap();
        assert_eq!(doc.user_id, "alice");
        assert_eq!(doc.path, "notes/welcome.md");
        // Idempotent: second call returns the same doc id.
        let same = s
            .get_or_create_document_by_path("alice", None, "notes/welcome.md")
            .await
            .unwrap();
        assert_eq!(doc.id, same.id);
    }

    #[tokio::test]
    async fn update_document_persists_content() {
        let s = store();
        let doc = s
            .get_or_create_document_by_path("alice", None, "notes/a.md")
            .await
            .unwrap();
        s.update_document(doc.id, "hello world").await.unwrap();
        let fetched = s.get_document_by_id(doc.id).await.unwrap();
        assert_eq!(fetched.content, "hello world");
    }

    #[tokio::test]
    async fn get_by_path_returns_not_found_for_missing() {
        let s = store();
        let err = s
            .get_document_by_path("alice", None, "missing.md")
            .await
            .unwrap_err();
        assert!(matches!(err, WorkspaceError::DocumentNotFound { .. }));
    }

    #[tokio::test]
    async fn delete_document_by_path_clears_state() {
        let s = store();
        let _doc = s
            .get_or_create_document_by_path("alice", None, "notes/a.md")
            .await
            .unwrap();
        s.delete_document_by_path("alice", None, "notes/a.md")
            .await
            .unwrap();
        let err = s
            .get_document_by_path("alice", None, "notes/a.md")
            .await
            .unwrap_err();
        assert!(matches!(err, WorkspaceError::DocumentNotFound { .. }));
    }

    #[tokio::test]
    async fn list_documents_isolates_users() {
        let s = store();
        s.get_or_create_document_by_path("alice", None, "a.md")
            .await
            .unwrap();
        s.get_or_create_document_by_path("bob", None, "b.md")
            .await
            .unwrap();
        let alice = s.list_documents("alice", None).await.unwrap();
        assert_eq!(alice.len(), 1);
        assert_eq!(alice[0].path, "a.md");
    }
}
