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
//! (for [`IndexKind::Vector`]) projections.
//!
//! ## Hybrid search
//!
//! [`hybrid_search`](search::hybrid_search) prefers the backend-native
//! `Filter::Fts` / `Filter::VectorNearest` paths when the mounted
//! `RootFilesystem` advertises `Capability::IndexFts` /
//! `Capability::IndexVector` (libSQL FTS5, Postgres tsvector + brute-
//! force cosine, the in-memory reference backend). On first call the
//! facade lazily declares the chunk FTS + Vector indexes via
//! `ensure_index` so the backend's planner can serve them.
//!
//! Backends that return `FilesystemError::Unsupported` for the FTS or
//! Vector branch fall back to the in-memory scan-and-rank path that
//! walks the user's chunks and ranks them in Rust. The final fusion
//! (RRF / weighted) and `SearchResult` shape are identical on both
//! paths — only the candidate-set source changes.
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
use tokio::sync::OnceCell;
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
    /// Caches whether we've already declared the FTS + Vector indexes on
    /// `/workspace/chunks` so concurrent searches don't re-issue
    /// `ensure_index` per call. `Ok(true)` means the indexes are
    /// available; `Ok(false)` means the backend rejected the
    /// declaration with `Unsupported` (capability-light backend — the
    /// scan-and-rank fallback path serves these). A poisoned cell
    /// retries on the next call so transient backend errors don't
    /// permanently disable native search.
    pub(crate) chunk_indexes_ready: OnceCell<bool>,
}

impl<F> FilesystemWorkspaceStore<F>
where
    F: RootFilesystem,
{
    pub fn new(filesystem: Arc<F>) -> Self {
        Self {
            filesystem,
            chunk_indexes_ready: OnceCell::new(),
        }
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

    /// Regression for the HIGH-severity finding: `hybrid_search` used to
    /// scan every chunk in memory and rank in Rust even when the
    /// backend advertised `Capability::IndexFts` / `IndexVector`. The
    /// in-memory backend now serves `Filter::Fts` and
    /// `Filter::VectorNearest` directly; this test exercises the native
    /// query path (FTS-only and vector-only branches) end-to-end and
    /// confirms the surviving fusion still returns the expected hits.
    #[tokio::test]
    async fn hybrid_search_uses_native_backend_filters_when_supported() {
        let s = store();
        let doc = s
            .get_or_create_document_by_path("alice", None, "notes/recipe.md")
            .await
            .unwrap();
        // Embedding dim = 3 so we can hand-craft cosine-similarity
        // expectations. Chunk 0 is the closest vector match; chunk 1 is
        // the closest text match.
        let chunks = vec![
            crate::workspace::ChunkWrite {
                content: "ginger lemon turmeric morning tonic".to_string(),
                embedding: Some(vec![1.0_f32, 0.0, 0.0]),
            },
            crate::workspace::ChunkWrite {
                content: "honey lemon glaze for roast carrots".to_string(),
                embedding: Some(vec![0.0_f32, 1.0, 0.0]),
            },
            crate::workspace::ChunkWrite {
                content: "spinach broth no citrus".to_string(),
                embedding: Some(vec![0.0_f32, 0.0, 1.0]),
            },
        ];
        s.replace_chunks(doc.id, &chunks).await.unwrap();

        // FTS branch: `lemon` should match the first two chunks. The
        // in-memory FTS shim returns all matches in stored-path order;
        // both rows survive because both project `content` carrying
        // "lemon".
        let cfg = SearchConfig {
            limit: 5,
            use_fts: true,
            use_vector: false,
            ..SearchConfig::default()
        };
        let fts_only = s
            .hybrid_search("alice", None, "lemon", None, &cfg)
            .await
            .unwrap();
        assert_eq!(fts_only.len(), 2);
        assert!(fts_only.iter().all(|r| r.from_fts()));
        assert!(fts_only.iter().all(|r| !r.from_vector()));

        // Vector branch: query [1,0,0] is identical to chunk 0's
        // embedding; the ranked-top-k brute-force ranker returns chunk
        // 0 first.
        let cfg_vec = SearchConfig {
            limit: 5,
            use_fts: false,
            use_vector: true,
            ..SearchConfig::default()
        };
        let vector_only = s
            .hybrid_search("alice", None, "", Some(&[1.0_f32, 0.0, 0.0]), &cfg_vec)
            .await
            .unwrap();
        assert!(!vector_only.is_empty());
        assert_eq!(vector_only[0].content, chunks[0].content);
        assert!(vector_only.iter().all(|r| r.from_vector()));
        assert!(vector_only.iter().all(|r| !r.from_fts()));

        // Hybrid: both branches contribute, RRF fuses. The chunk that
        // appears in both rankings (chunk 0 — it has "lemon" and is the
        // vector top-1)... wait — chunk 0 has "lemon"? No, it has
        // "turmeric"; chunk 1 has "lemon". Use a query that hits both
        // sources to confirm the is_hybrid path.
        let cfg_hybrid = SearchConfig::default();
        let hybrid = s
            .hybrid_search(
                "alice",
                None,
                "turmeric",
                Some(&[1.0_f32, 0.0, 0.0]),
                &cfg_hybrid,
            )
            .await
            .unwrap();
        // Top result is chunk 0 (matches both FTS "turmeric" and the
        // vector embedding).
        assert_eq!(hybrid[0].content, chunks[0].content);
        assert!(hybrid[0].is_hybrid());
    }

    /// Regression: native FTS/vector results must be filtered by the
    /// caller's `(user_id, agent_id)`. The libsql/postgres FTS table
    /// triggers don't carry scope, and vector-nearest is a top-level
    /// ranker that ignores compound filters — so the facade is the
    /// only place that enforces the scope contract.
    #[tokio::test]
    async fn hybrid_search_native_path_isolates_users_across_shared_prefix() {
        let s = store();
        let alice_doc = s
            .get_or_create_document_by_path("alice", None, "n.md")
            .await
            .unwrap();
        let bob_doc = s
            .get_or_create_document_by_path("bob", None, "n.md")
            .await
            .unwrap();
        s.insert_chunk(alice_doc.id, 0, "secret recipe ginger", None)
            .await
            .unwrap();
        s.insert_chunk(bob_doc.id, 0, "secret recipe lemon", None)
            .await
            .unwrap();

        let cfg = SearchConfig {
            limit: 10,
            use_fts: true,
            use_vector: false,
            ..SearchConfig::default()
        };
        let alice_hits = s
            .hybrid_search("alice", None, "secret", None, &cfg)
            .await
            .unwrap();
        assert_eq!(alice_hits.len(), 1);
        assert!(alice_hits[0].content.contains("ginger"));
        assert_eq!(alice_hits[0].document_path, "n.md");

        let bob_hits = s
            .hybrid_search("bob", None, "secret", None, &cfg)
            .await
            .unwrap();
        assert_eq!(bob_hits.len(), 1);
        assert!(bob_hits[0].content.contains("lemon"));
    }
}
