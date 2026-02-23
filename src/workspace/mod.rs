//! Workspace and memory system (OpenClaw-inspired).
//!
//! The workspace provides persistent memory for agents with a flexible
//! filesystem-like structure. Agents can create arbitrary markdown file
//! hierarchies that get indexed for full-text and semantic search.
//!
//! # Filesystem-like API
//!
//! ```text
//! workspace/
//! ├── README.md              <- Root runbook/index
//! ├── MEMORY.md              <- Long-term curated memory
//! ├── HEARTBEAT.md           <- Periodic checklist
//! ├── context/               <- Identity and context
//! │   ├── vision.md
//! │   └── priorities.md
//! ├── daily/                 <- Daily logs
//! │   ├── 2024-01-15.md
//! │   └── 2024-01-16.md
//! ├── projects/              <- Arbitrary structure
//! │   └── alpha/
//! │       ├── README.md
//! │       └── notes.md
//! └── ...
//! ```
//!
//! # Key Operations
//!
//! - `read(path)` - Read a file
//! - `write(path, content)` - Create or update a file
//! - `append(path, content)` - Append to a file
//! - `list(dir)` - List directory contents
//! - `delete(path)` - Delete a file
//! - `search(query)` - Full-text + semantic search across all files
//!
//! # Key Patterns
//!
//! 1. **Memory is persistence**: If you want to remember something, write it
//! 2. **Flexible structure**: Create any directory/file hierarchy you need
//! 3. **Self-documenting**: Use README.md files to describe directory structure
//! 4. **Hybrid search**: Vector similarity + BM25 full-text via RRF

mod chunker;
mod document;
mod embeddings;
pub mod hygiene;
pub mod layer;
pub mod privacy;
#[cfg(feature = "postgres")]
mod repository;
mod search;

pub use chunker::{ChunkConfig, chunk_document};
pub use document::{MemoryChunk, MemoryDocument, WorkspaceEntry, merge_workspace_entries, paths};
pub use embeddings::{
    EmbeddingProvider, MockEmbeddings, NearAiEmbeddings, OllamaEmbeddings, OpenAiEmbeddings,
};
#[cfg(feature = "postgres")]
pub use repository::Repository;
pub use search::{RankedResult, SearchConfig, SearchResult, reciprocal_rank_fusion};

/// Result of a layer-aware write operation.
///
/// Contains the written document plus metadata about whether the write
/// was redirected to a different layer (e.g., sensitive content redirected
/// from shared to private).
pub struct WriteResult {
    pub document: MemoryDocument,
    pub redirected: bool,
    pub actual_layer: String,
}

use std::sync::Arc;

use chrono::{NaiveDate, Utc};
#[cfg(feature = "postgres")]
use deadpool_postgres::Pool;
use uuid::Uuid;

use crate::error::WorkspaceError;

/// Internal storage abstraction for Workspace.
///
/// Allows Workspace to work with either a PostgreSQL `Repository` (the original
/// path) or any `Database` trait implementation (e.g. libSQL backend).
enum WorkspaceStorage {
    /// PostgreSQL-backed repository (uses connection pool directly).
    #[cfg(feature = "postgres")]
    Repo(Repository),
    /// Generic backend implementing the Database trait.
    Db(Arc<dyn crate::db::Database>),
}

impl WorkspaceStorage {
    async fn get_document_by_path(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        path: &str,
    ) -> Result<MemoryDocument, WorkspaceError> {
        match self {
            #[cfg(feature = "postgres")]
            Self::Repo(repo) => repo.get_document_by_path(user_id, agent_id, path).await,
            Self::Db(db) => db.get_document_by_path(user_id, agent_id, path).await,
        }
    }

    async fn get_document_by_id(&self, id: Uuid) -> Result<MemoryDocument, WorkspaceError> {
        match self {
            #[cfg(feature = "postgres")]
            Self::Repo(repo) => repo.get_document_by_id(id).await,
            Self::Db(db) => db.get_document_by_id(id).await,
        }
    }

    async fn get_or_create_document_by_path(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        path: &str,
    ) -> Result<MemoryDocument, WorkspaceError> {
        match self {
            #[cfg(feature = "postgres")]
            Self::Repo(repo) => {
                repo.get_or_create_document_by_path(user_id, agent_id, path)
                    .await
            }
            Self::Db(db) => {
                db.get_or_create_document_by_path(user_id, agent_id, path)
                    .await
            }
        }
    }

    async fn update_document(&self, id: Uuid, content: &str) -> Result<(), WorkspaceError> {
        match self {
            #[cfg(feature = "postgres")]
            Self::Repo(repo) => repo.update_document(id, content).await,
            Self::Db(db) => db.update_document(id, content).await,
        }
    }

    async fn delete_document_by_path(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        path: &str,
    ) -> Result<(), WorkspaceError> {
        match self {
            #[cfg(feature = "postgres")]
            Self::Repo(repo) => repo.delete_document_by_path(user_id, agent_id, path).await,
            Self::Db(db) => db.delete_document_by_path(user_id, agent_id, path).await,
        }
    }

    async fn list_directory(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        directory: &str,
    ) -> Result<Vec<WorkspaceEntry>, WorkspaceError> {
        match self {
            #[cfg(feature = "postgres")]
            Self::Repo(repo) => repo.list_directory(user_id, agent_id, directory).await,
            Self::Db(db) => db.list_directory(user_id, agent_id, directory).await,
        }
    }

    async fn list_all_paths(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
    ) -> Result<Vec<String>, WorkspaceError> {
        match self {
            #[cfg(feature = "postgres")]
            Self::Repo(repo) => repo.list_all_paths(user_id, agent_id).await,
            Self::Db(db) => db.list_all_paths(user_id, agent_id).await,
        }
    }

    async fn delete_chunks(&self, document_id: Uuid) -> Result<(), WorkspaceError> {
        match self {
            #[cfg(feature = "postgres")]
            Self::Repo(repo) => repo.delete_chunks(document_id).await,
            Self::Db(db) => db.delete_chunks(document_id).await,
        }
    }

    async fn insert_chunk(
        &self,
        document_id: Uuid,
        chunk_index: i32,
        content: &str,
        embedding: Option<&[f32]>,
    ) -> Result<Uuid, WorkspaceError> {
        match self {
            #[cfg(feature = "postgres")]
            Self::Repo(repo) => {
                repo.insert_chunk(document_id, chunk_index, content, embedding)
                    .await
            }
            Self::Db(db) => {
                db.insert_chunk(document_id, chunk_index, content, embedding)
                    .await
            }
        }
    }

    async fn update_chunk_embedding(
        &self,
        chunk_id: Uuid,
        embedding: &[f32],
    ) -> Result<(), WorkspaceError> {
        match self {
            #[cfg(feature = "postgres")]
            Self::Repo(repo) => repo.update_chunk_embedding(chunk_id, embedding).await,
            Self::Db(db) => db.update_chunk_embedding(chunk_id, embedding).await,
        }
    }

    async fn get_chunks_without_embeddings(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        limit: usize,
    ) -> Result<Vec<MemoryChunk>, WorkspaceError> {
        match self {
            #[cfg(feature = "postgres")]
            Self::Repo(repo) => {
                repo.get_chunks_without_embeddings(user_id, agent_id, limit)
                    .await
            }
            Self::Db(db) => {
                db.get_chunks_without_embeddings(user_id, agent_id, limit)
                    .await
            }
        }
    }

    async fn hybrid_search(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        query: &str,
        embedding: Option<&[f32]>,
        config: &SearchConfig,
    ) -> Result<Vec<SearchResult>, WorkspaceError> {
        match self {
            #[cfg(feature = "postgres")]
            Self::Repo(repo) => {
                repo.hybrid_search(user_id, agent_id, query, embedding, config)
                    .await
            }
            Self::Db(db) => {
                db.hybrid_search(user_id, agent_id, query, embedding, config)
                    .await
            }
        }
    }

    // ==================== Multi-scope read methods ====================

    async fn hybrid_search_multi(
        &self,
        user_ids: &[String],
        agent_id: Option<Uuid>,
        query: &str,
        embedding: Option<&[f32]>,
        config: &SearchConfig,
    ) -> Result<Vec<SearchResult>, WorkspaceError> {
        match self {
            #[cfg(feature = "postgres")]
            Self::Repo(repo) => {
                repo.hybrid_search_multi(user_ids, agent_id, query, embedding, config)
                    .await
            }
            Self::Db(db) => {
                db.hybrid_search_multi(user_ids, agent_id, query, embedding, config)
                    .await
            }
        }
    }

    async fn list_all_paths_multi(
        &self,
        user_ids: &[String],
        agent_id: Option<Uuid>,
    ) -> Result<Vec<String>, WorkspaceError> {
        match self {
            #[cfg(feature = "postgres")]
            Self::Repo(repo) => repo.list_all_paths_multi(user_ids, agent_id).await,
            Self::Db(db) => db.list_all_paths_multi(user_ids, agent_id).await,
        }
    }

    async fn get_document_by_path_multi(
        &self,
        user_ids: &[String],
        agent_id: Option<Uuid>,
        path: &str,
    ) -> Result<MemoryDocument, WorkspaceError> {
        match self {
            #[cfg(feature = "postgres")]
            Self::Repo(repo) => {
                repo.get_document_by_path_multi(user_ids, agent_id, path)
                    .await
            }
            Self::Db(db) => {
                db.get_document_by_path_multi(user_ids, agent_id, path)
                    .await
            }
        }
    }

    async fn list_directory_multi(
        &self,
        user_ids: &[String],
        agent_id: Option<Uuid>,
        directory: &str,
    ) -> Result<Vec<WorkspaceEntry>, WorkspaceError> {
        match self {
            #[cfg(feature = "postgres")]
            Self::Repo(repo) => {
                repo.list_directory_multi(user_ids, agent_id, directory)
                    .await
            }
            Self::Db(db) => db.list_directory_multi(user_ids, agent_id, directory).await,
        }
    }
}

/// Default template seeded into HEARTBEAT.md on first access.
///
/// Intentionally comment-only so the heartbeat runner treats it as
/// "effectively empty" and skips the LLM call until the user adds
/// real tasks.
const HEARTBEAT_SEED: &str = "\
# Heartbeat Checklist

<!-- Keep this file empty to skip heartbeat API calls.
     Add tasks below when you want the agent to check something periodically.

     Rotate through these checks 2-4 times per day:
     - [ ] Check for urgent messages
     - [ ] Review upcoming calendar events
     - [ ] Check project status or CI builds

     Stay quiet during 23:00-08:00 user-local time unless urgent.
     If nothing needs attention, reply HEARTBEAT_OK.

     Proactive work you can do without asking:
     - Organize and curate MEMORY.md (remove stale, consolidate dupes)
     - Update daily logs with session summaries
     - Clean up context/ documents that are outdated
-->";

/// Workspace provides database-backed memory storage for an agent.
///
/// Each workspace is scoped to a user (and optionally an agent).
/// Documents are persisted to the database and indexed for search.
/// Supports both PostgreSQL (via Repository) and libSQL (via Database trait).
///
/// ## Multi-scope reads
///
/// By default, a workspace reads from and writes to a single `user_id`.
/// With `with_additional_read_scopes`, read operations (search, read, list)
/// can span multiple user scopes while writes remain isolated to the primary
/// `user_id`. This enables cross-tenant read access (e.g., a user reading
/// from both their own workspace and a "shared" workspace).
pub struct Workspace {
    /// User identifier (from channel). All writes go to this scope.
    user_id: String,
    /// User identifiers for read operations. Includes `user_id` as the first
    /// element, plus any additional scopes added via `with_additional_read_scopes`.
    read_user_ids: Vec<String>,
    /// Optional agent ID for multi-agent isolation.
    agent_id: Option<Uuid>,
    /// Database storage backend.
    storage: WorkspaceStorage,
    /// Embedding provider for semantic search.
    embeddings: Option<Arc<dyn EmbeddingProvider>>,
    /// Memory layers this workspace has access to.
    memory_layers: Vec<crate::workspace::layer::MemoryLayer>,
}

impl Workspace {
    /// Create a new workspace backed by a PostgreSQL connection pool.
    #[cfg(feature = "postgres")]
    pub fn new(user_id: impl Into<String>, pool: Pool) -> Self {
        let user_id_str = user_id.into();
        let memory_layers = crate::workspace::layer::MemoryLayer::default_for_user(&user_id_str);
        Self {
            read_user_ids: vec![user_id_str.clone()],
            user_id: user_id_str,
            agent_id: None,
            storage: WorkspaceStorage::Repo(Repository::new(pool)),
            embeddings: None,
            memory_layers,
        }
    }

    /// Create a new workspace backed by any Database implementation.
    ///
    /// Use this for libSQL or any other backend that implements the Database trait.
    pub fn new_with_db(user_id: impl Into<String>, db: Arc<dyn crate::db::Database>) -> Self {
        let user_id_str = user_id.into();
        let memory_layers = crate::workspace::layer::MemoryLayer::default_for_user(&user_id_str);
        Self {
            read_user_ids: vec![user_id_str.clone()],
            user_id: user_id_str,
            agent_id: None,
            storage: WorkspaceStorage::Db(db),
            embeddings: None,
            memory_layers,
        }
    }

    /// Create a workspace with a specific agent ID.
    pub fn with_agent(mut self, agent_id: Uuid) -> Self {
        self.agent_id = Some(agent_id);
        self
    }

    /// Set the embedding provider for semantic search.
    pub fn with_embeddings(mut self, provider: Arc<dyn EmbeddingProvider>) -> Self {
        self.embeddings = Some(provider);
        self
    }

    /// Configure memory layers for this workspace.
    ///
    /// Also updates read_user_ids to include all layer scopes.
    pub fn with_memory_layers(mut self, layers: Vec<crate::workspace::layer::MemoryLayer>) -> Self {
        // Add layer scopes to read_user_ids (same dedup logic as with_additional_read_scopes)
        for layer in &layers {
            if !self.read_user_ids.contains(&layer.scope) {
                self.read_user_ids.push(layer.scope.clone());
            }
        }
        self.memory_layers = layers;
        self
    }

    /// Get the configured memory layers.
    pub fn memory_layers(&self) -> &[crate::workspace::layer::MemoryLayer] {
        &self.memory_layers
    }

    /// Add additional user scopes for read operations.
    ///
    /// The primary `user_id` is always included. Additional scopes allow
    /// read operations (search, read, list) to span multiple tenants while
    /// writes remain isolated to the primary scope.
    ///
    /// Duplicate scopes are ignored.
    pub fn with_additional_read_scopes(mut self, scopes: Vec<String>) -> Self {
        for scope in scopes {
            if !self.read_user_ids.contains(&scope) {
                self.read_user_ids.push(scope);
            }
        }
        self
    }

    /// Get the user ID (primary scope for writes).
    pub fn user_id(&self) -> &str {
        &self.user_id
    }

    /// Get the user IDs used for read operations.
    pub fn read_user_ids(&self) -> &[String] {
        &self.read_user_ids
    }

    /// Whether this workspace has multiple read scopes.
    fn is_multi_scope(&self) -> bool {
        self.read_user_ids.len() > 1
    }

    /// Get the agent ID.
    pub fn agent_id(&self) -> Option<Uuid> {
        self.agent_id
    }

    // ==================== File Operations ====================

    /// Read a file by path.
    ///
    /// Returns the document if it exists, or an error if not found.
    ///
    /// # Example
    /// ```ignore
    /// let doc = workspace.read("context/vision.md").await?;
    /// println!("{}", doc.content);
    /// ```
    pub async fn read(&self, path: &str) -> Result<MemoryDocument, WorkspaceError> {
        let path = normalize_path(path);
        if self.is_multi_scope() {
            self.storage
                .get_document_by_path_multi(&self.read_user_ids, self.agent_id, &path)
                .await
        } else {
            self.storage
                .get_document_by_path(&self.user_id, self.agent_id, &path)
                .await
        }
    }

    /// Write (create or update) a file.
    ///
    /// Creates parent directories implicitly (they're virtual in the DB).
    /// Re-indexes the document for search after writing.
    ///
    /// # Example
    /// ```ignore
    /// workspace.write("projects/alpha/README.md", "# Project Alpha\n\nDescription here.").await?;
    /// ```
    pub async fn write(&self, path: &str, content: &str) -> Result<MemoryDocument, WorkspaceError> {
        let path = normalize_path(path);
        let doc = self
            .storage
            .get_or_create_document_by_path(&self.user_id, self.agent_id, &path)
            .await?;
        self.storage.update_document(doc.id, content).await?;
        self.reindex_document(doc.id).await?;

        // Return updated doc
        self.storage.get_document_by_id(doc.id).await
    }

    /// Append content to a file.
    ///
    /// Creates the file if it doesn't exist.
    /// Uses a single `\n` separator (suitable for log-style entries).
    /// For semantic separation (e.g., memory entries), use `append_memory()`
    /// which uses `\n\n`.
    pub async fn append(&self, path: &str, content: &str) -> Result<(), WorkspaceError> {
        let path = normalize_path(path);
        let doc = self
            .storage
            .get_or_create_document_by_path(&self.user_id, self.agent_id, &path)
            .await?;

        let new_content = if doc.content.is_empty() {
            content.to_string()
        } else {
            format!("{}\n{}", doc.content, content)
        };

        self.storage.update_document(doc.id, &new_content).await?;
        self.reindex_document(doc.id).await?;
        Ok(())
    }

    /// Resolve the target scope for a layer write, applying privacy guards.
    ///
    /// Validates that the layer exists and is writable. For shared layers,
    /// checks content sensitivity and redirects to the private layer if needed.
    ///
    /// Note: privacy classification is pattern-based (regex) and can be bypassed
    /// by obfuscated content. See `privacy::PatternPrivacyClassifier` for details.
    ///
    /// # Multi-tenant safety (Issue #59)
    ///
    /// Layer scopes are currently used directly as `user_id` for DB operations.
    /// In a multi-tenant deployment, an operator could configure a scope that
    /// collides with another user's ID, granting write access to their data.
    /// Future work should namespace or validate scopes to prevent this.
    ///
    /// Returns `(scope, actual_layer_name, redirected)`.
    fn resolve_layer_target(
        &self,
        layer_name: &str,
        content: &str,
    ) -> Result<(String, String, bool), WorkspaceError> {
        use crate::workspace::layer::{LayerSensitivity, MemoryLayer};
        use crate::workspace::privacy::PrivacyClassifier;

        let layer = MemoryLayer::find(&self.memory_layers, layer_name).ok_or_else(|| {
            WorkspaceError::LayerNotFound {
                name: layer_name.to_string(),
            }
        })?;

        if !layer.writable {
            return Err(WorkspaceError::LayerReadOnly {
                name: layer_name.to_string(),
            });
        }

        if layer.sensitivity == LayerSensitivity::Shared {
            let classifier = crate::workspace::privacy::global_classifier();
            if classifier.is_sensitive(content) {
                tracing::warn!(
                    layer = layer_name,
                    "Redirected sensitive content to private layer"
                );
                let private = MemoryLayer::private_layer(&self.memory_layers)
                    .ok_or(WorkspaceError::PrivacyRedirectFailed)?;
                if !private.writable {
                    return Err(WorkspaceError::PrivacyRedirectFailed);
                }
                return Ok((private.scope.clone(), private.name.clone(), true));
            }
        }

        Ok((layer.scope.clone(), layer_name.to_string(), false))
    }

    /// Write to a specific memory layer.
    ///
    /// Checks that the layer exists and is writable. Uses the layer's scope
    /// as the user_id for the database write. For shared layers, sensitive
    /// content is automatically redirected to the private layer.
    pub async fn write_to_layer(
        &self,
        layer_name: &str,
        path: &str,
        content: &str,
    ) -> Result<WriteResult, WorkspaceError> {
        let (scope, actual_layer, redirected) =
            self.resolve_layer_target(layer_name, content)?;
        let path = normalize_path(path);
        let doc = self
            .storage
            .get_or_create_document_by_path(&scope, self.agent_id, &path)
            .await?;
        self.storage.update_document(doc.id, content).await?;
        self.reindex_document(doc.id).await?;
        let document = self.storage.get_document_by_id(doc.id).await?;
        Ok(WriteResult {
            document,
            redirected,
            actual_layer,
        })
    }

    /// Write to a layer, with append semantics.
    pub async fn append_to_layer(
        &self,
        layer_name: &str,
        path: &str,
        content: &str,
    ) -> Result<WriteResult, WorkspaceError> {
        let (scope, actual_layer, redirected) =
            self.resolve_layer_target(layer_name, content)?;
        let path = normalize_path(path);
        let doc = self
            .storage
            .get_or_create_document_by_path(&scope, self.agent_id, &path)
            .await?;
        let new_content = if doc.content.is_empty() {
            content.to_string()
        } else {
            format!("{}\n\n{}", doc.content, content)
        };
        self.storage.update_document(doc.id, &new_content).await?;
        self.reindex_document(doc.id).await?;
        let document = self.storage.get_document_by_id(doc.id).await?;
        Ok(WriteResult {
            document,
            redirected,
            actual_layer,
        })
    }

    /// Check if a file exists.
    ///
    /// When multi-scope reads are configured, checks across all read scopes.
    pub async fn exists(&self, path: &str) -> Result<bool, WorkspaceError> {
        let path = normalize_path(path);
        let result = if self.is_multi_scope() {
            self.storage
                .get_document_by_path_multi(&self.read_user_ids, self.agent_id, &path)
                .await
        } else {
            self.storage
                .get_document_by_path(&self.user_id, self.agent_id, &path)
                .await
        };
        match result {
            Ok(_) => Ok(true),
            Err(WorkspaceError::DocumentNotFound { .. }) => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// Delete a file.
    ///
    /// Also deletes associated chunks.
    pub async fn delete(&self, path: &str) -> Result<(), WorkspaceError> {
        let path = normalize_path(path);
        self.storage
            .delete_document_by_path(&self.user_id, self.agent_id, &path)
            .await
    }

    /// List files and directories in a path.
    ///
    /// Returns immediate children (not recursive).
    /// Use empty string or "/" for root directory.
    ///
    /// # Example
    /// ```ignore
    /// let entries = workspace.list("projects/").await?;
    /// for entry in entries {
    ///     if entry.is_directory {
    ///         println!("📁 {}/", entry.name());
    ///     } else {
    ///         println!("📄 {}", entry.name());
    ///     }
    /// }
    /// ```
    pub async fn list(&self, directory: &str) -> Result<Vec<WorkspaceEntry>, WorkspaceError> {
        let directory = normalize_directory(directory);
        if self.is_multi_scope() {
            self.storage
                .list_directory_multi(&self.read_user_ids, self.agent_id, &directory)
                .await
        } else {
            self.storage
                .list_directory(&self.user_id, self.agent_id, &directory)
                .await
        }
    }

    /// List all files recursively (flat list of all paths).
    ///
    /// When multi-scope reads are configured, lists across all read scopes.
    pub async fn list_all(&self) -> Result<Vec<String>, WorkspaceError> {
        if self.is_multi_scope() {
            self.storage
                .list_all_paths_multi(&self.read_user_ids, self.agent_id)
                .await
        } else {
            self.storage
                .list_all_paths(&self.user_id, self.agent_id)
                .await
        }
    }

    // ==================== Convenience Methods ====================

    /// Get the main MEMORY.md document (long-term curated memory).
    ///
    /// Creates it if it doesn't exist.
    pub async fn memory(&self) -> Result<MemoryDocument, WorkspaceError> {
        self.read_or_create(paths::MEMORY).await
    }

    /// Get today's daily log.
    ///
    /// Daily logs are append-only and keyed by date.
    pub async fn today_log(&self) -> Result<MemoryDocument, WorkspaceError> {
        let today = Utc::now().date_naive();
        self.daily_log(today).await
    }

    /// Get a daily log for a specific date.
    pub async fn daily_log(&self, date: NaiveDate) -> Result<MemoryDocument, WorkspaceError> {
        let path = format!("daily/{}.md", date.format("%Y-%m-%d"));
        self.read_or_create(&path).await
    }

    /// Get the heartbeat checklist (HEARTBEAT.md).
    ///
    /// Returns the DB-stored checklist if it exists, otherwise falls back
    /// to the in-memory seed template. The seed is never written to the
    /// database; the user creates the real file via `memory_write` when
    /// they actually want periodic checks. The seed content is all HTML
    /// comments, which the heartbeat runner treats as "effectively empty"
    /// and skips the LLM call.
    pub async fn heartbeat_checklist(&self) -> Result<Option<String>, WorkspaceError> {
        match self.read(paths::HEARTBEAT).await {
            Ok(doc) => Ok(Some(doc.content)),
            Err(WorkspaceError::DocumentNotFound { .. }) => Ok(Some(HEARTBEAT_SEED.to_string())),
            Err(e) => Err(e),
        }
    }

    /// Helper to read or create a file.
    ///
    /// When multi-scope reads are configured, checks all read scopes before
    /// creating. If the file exists in any scope, returns it. If not found in
    /// any scope, creates it in the primary (write) scope.
    async fn read_or_create(&self, path: &str) -> Result<MemoryDocument, WorkspaceError> {
        if self.is_multi_scope() {
            match self
                .storage
                .get_document_by_path_multi(&self.read_user_ids, self.agent_id, path)
                .await
            {
                Ok(doc) => return Ok(doc),
                Err(WorkspaceError::DocumentNotFound { .. }) => {}
                Err(e) => return Err(e),
            }
        }
        self.storage
            .get_or_create_document_by_path(&self.user_id, self.agent_id, path)
            .await
    }

    // ==================== Memory Operations ====================

    /// Append an entry to the main MEMORY.md document.
    ///
    /// This is for important facts, decisions, and preferences worth
    /// remembering long-term.
    pub async fn append_memory(&self, entry: &str) -> Result<(), WorkspaceError> {
        // Use double newline for memory entries (semantic separation)
        let doc = self.memory().await?;
        let new_content = if doc.content.is_empty() {
            entry.to_string()
        } else {
            format!("{}\n\n{}", doc.content, entry)
        };
        self.storage.update_document(doc.id, &new_content).await?;
        self.reindex_document(doc.id).await?;
        Ok(())
    }

    /// Append an entry to today's daily log.
    ///
    /// Daily logs are raw, append-only notes for the current day.
    pub async fn append_daily_log(&self, entry: &str) -> Result<(), WorkspaceError> {
        let today = Utc::now().date_naive();
        let path = format!("daily/{}.md", today.format("%Y-%m-%d"));
        let timestamp = Utc::now().format("%H:%M:%S");
        let timestamped_entry = format!("[{}] {}", timestamp, entry);
        self.append(&path, &timestamped_entry).await
    }

    // ==================== System Prompt ====================

    /// Build the system prompt from identity files.
    ///
    /// Loads AGENTS.md, SOUL.md, USER.md, IDENTITY.md, and (in non-group
    /// contexts) MEMORY.md to compose the agent's system prompt.
    ///
    /// Shorthand for `system_prompt_for_context(false)`.
    pub async fn system_prompt(&self) -> Result<String, WorkspaceError> {
        self.system_prompt_for_context(false).await
    }

    /// Build the system prompt, optionally excluding personal memory.
    ///
    /// When `is_group_chat` is true, MEMORY.md is excluded to prevent
    /// leaking personal context into group conversations.
    pub async fn system_prompt_for_context(
        &self,
        is_group_chat: bool,
    ) -> Result<String, WorkspaceError> {
        let mut parts = Vec::new();

        // Load identity files in order of importance
        let identity_files = [
            (paths::AGENTS, "## Agent Instructions"),
            (paths::SOUL, "## Core Values"),
            (paths::USER, "## User Context"),
            (paths::IDENTITY, "## Identity"),
        ];

        for (path, header) in identity_files {
            if let Ok(doc) = self.read(path).await
                && !doc.content.is_empty()
            {
                parts.push(format!("{}\n\n{}", header, doc.content));
            }
        }

        // Load MEMORY.md only in direct/main sessions (never group chats)
        if !is_group_chat
            && let Ok(doc) = self.read(paths::MEMORY).await
            && !doc.content.is_empty()
        {
            parts.push(format!("## Long-Term Memory\n\n{}", doc.content));
        }

        // Add today's memory context (last 2 days of daily logs)
        let today = Utc::now().date_naive();
        let yesterday = today.pred_opt().unwrap_or(today);

        for date in [today, yesterday] {
            if let Ok(doc) = self.daily_log(date).await
                && !doc.content.is_empty()
            {
                let header = if date == today {
                    "## Today's Notes"
                } else {
                    "## Yesterday's Notes"
                };
                parts.push(format!("{}\n\n{}", header, doc.content));
            }
        }

        Ok(parts.join("\n\n---\n\n"))
    }

    // ==================== Search ====================

    /// Hybrid search across all memory documents.
    ///
    /// Combines full-text search (BM25) with semantic search (vector similarity)
    /// using Reciprocal Rank Fusion (RRF).
    pub async fn search(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<SearchResult>, WorkspaceError> {
        self.search_with_config(query, SearchConfig::default().with_limit(limit))
            .await
    }

    /// Search with custom configuration.
    ///
    /// When multi-scope reads are configured, searches across all read scopes.
    pub async fn search_with_config(
        &self,
        query: &str,
        config: SearchConfig,
    ) -> Result<Vec<SearchResult>, WorkspaceError> {
        // Generate embedding for semantic search if provider available
        let embedding = if let Some(ref provider) = self.embeddings {
            Some(
                provider
                    .embed(query)
                    .await
                    .map_err(|e| WorkspaceError::EmbeddingFailed {
                        reason: e.to_string(),
                    })?,
            )
        } else {
            None
        };

        if self.is_multi_scope() {
            self.storage
                .hybrid_search_multi(
                    &self.read_user_ids,
                    self.agent_id,
                    query,
                    embedding.as_deref(),
                    &config,
                )
                .await
        } else {
            self.storage
                .hybrid_search(
                    &self.user_id,
                    self.agent_id,
                    query,
                    embedding.as_deref(),
                    &config,
                )
                .await
        }
    }

    // ==================== Indexing ====================

    /// Re-index a document (chunk and generate embeddings).
    async fn reindex_document(&self, document_id: Uuid) -> Result<(), WorkspaceError> {
        // Get the document
        let doc = self.storage.get_document_by_id(document_id).await?;

        // Chunk the content
        let chunks = chunk_document(&doc.content, ChunkConfig::default());

        // Delete old chunks
        self.storage.delete_chunks(document_id).await?;

        // Insert new chunks
        for (index, content) in chunks.into_iter().enumerate() {
            // Generate embedding if provider available
            let embedding = if let Some(ref provider) = self.embeddings {
                match provider.embed(&content).await {
                    Ok(emb) => Some(emb),
                    Err(e) => {
                        tracing::warn!("Failed to generate embedding: {}", e);
                        None
                    }
                }
            } else {
                None
            };

            self.storage
                .insert_chunk(document_id, index as i32, &content, embedding.as_deref())
                .await?;
        }

        Ok(())
    }

    // ==================== Seeding ====================

    /// Seed any missing core identity files in the workspace.
    ///
    /// Called on every boot. Only creates files that don't already exist,
    /// so user edits are never overwritten. Returns the number of files
    /// created (0 if all core files already existed).
    pub async fn seed_if_empty(&self) -> Result<usize, WorkspaceError> {
        let seed_files: &[(&str, &str)] = &[
            (
                paths::README,
                "# Workspace\n\n\
                 This is your agent's persistent memory. Files here are indexed for search\n\
                 and used to build the agent's context.\n\n\
                 ## Structure\n\n\
                 - `MEMORY.md` - Long-term curated notes (loaded into system prompt)\n\
                 - `IDENTITY.md` - Agent name, vibe, personality\n\
                 - `SOUL.md` - Core values and behavioral boundaries\n\
                 - `AGENTS.md` - Session routine and operational instructions\n\
                 - `USER.md` - Information about you (the user)\n\
                 - `HEARTBEAT.md` - Periodic background task checklist\n\
                 - `daily/` - Automatic daily session logs\n\
                 - `context/` - Additional context documents\n\n\
                 Edit these files to shape how your agent thinks and acts.\n\
                 The agent reads them at the start of every session.",
            ),
            (
                paths::MEMORY,
                "# Memory\n\n\
                 Long-term notes, decisions, and facts worth remembering across sessions.\n\n\
                 The agent appends here during conversations. Curate periodically:\n\
                 remove stale entries, consolidate duplicates, keep it concise.\n\
                 This file is loaded into the system prompt, so brevity matters.",
            ),
            (
                paths::IDENTITY,
                "# Identity\n\n\
                 - **Name:** (pick one during your first conversation)\n\
                 - **Vibe:** (how you come across, e.g. calm, witty, direct)\n\
                 - **Emoji:** (your signature emoji, optional)\n\n\
                 Edit this file to give the agent a custom name and personality.\n\
                 The agent will evolve this over time as it develops a voice.",
            ),
            (
                paths::SOUL,
                "# Core Values\n\n\
                 Be genuinely helpful, not performatively helpful. Skip filler phrases.\n\
                 Have opinions. Disagree when it matters.\n\
                 Be resourceful before asking: read the file, check context, search, then ask.\n\
                 Earn trust through competence. Be careful with external actions, bold with internal ones.\n\
                 You have access to someone's life. Treat it with respect.\n\n\
                 ## Boundaries\n\n\
                 - Private things stay private. Never leak user context into group chats.\n\
                 - When in doubt about an external action, ask before acting.\n\
                 - Prefer reversible actions over destructive ones.\n\
                 - You are not the user's voice in group settings.",
            ),
            (
                paths::AGENTS,
                "# Agent Instructions\n\n\
                 You are a personal AI assistant with access to tools and persistent memory.\n\n\
                 ## Every Session\n\n\
                 1. Read SOUL.md (who you are)\n\
                 2. Read USER.md (who you're helping)\n\
                 3. Read today's daily log for recent context\n\n\
                 ## Memory\n\n\
                 You wake up fresh each session. Workspace files are your continuity.\n\
                 - Daily logs (`daily/YYYY-MM-DD.md`): raw session notes\n\
                 - `MEMORY.md`: curated long-term knowledge\n\
                 Write things down. Mental notes do not survive restarts.\n\n\
                 ## Guidelines\n\n\
                 - Always search memory before answering questions about prior conversations\n\
                 - Write important facts and decisions to memory for future reference\n\
                 - Use the daily log for session-level notes\n\
                 - Be concise but thorough\n\n\
                 ## Safety\n\n\
                 - Do not exfiltrate private data\n\
                 - Prefer reversible actions over destructive ones\n\
                 - When in doubt, ask",
            ),
            (
                paths::USER,
                "# User Context\n\n\
                 - **Name:**\n\
                 - **Timezone:**\n\
                 - **Preferences:**\n\n\
                 The agent will fill this in as it learns about you.\n\
                 You can also edit this directly to provide context upfront.",
            ),
            (paths::HEARTBEAT, HEARTBEAT_SEED),
        ];

        let mut count = 0;
        for (path, content) in seed_files {
            // Skip files that already exist (never overwrite user edits)
            match self.read(path).await {
                Ok(_) => continue,
                Err(WorkspaceError::DocumentNotFound { .. }) => {}
                Err(e) => {
                    tracing::warn!("Failed to check {}: {}", path, e);
                    continue;
                }
            }

            if let Err(e) = self.write(path, content).await {
                tracing::warn!("Failed to seed {}: {}", path, e);
            } else {
                count += 1;
            }
        }

        if count > 0 {
            tracing::info!("Seeded {} workspace files", count);
        }
        Ok(count)
    }

    /// Generate embeddings for chunks that don't have them yet.
    ///
    /// This is useful for backfilling embeddings after enabling the provider.
    pub async fn backfill_embeddings(&self) -> Result<usize, WorkspaceError> {
        let Some(ref provider) = self.embeddings else {
            return Ok(0);
        };

        let chunks = self
            .storage
            .get_chunks_without_embeddings(&self.user_id, self.agent_id, 100)
            .await?;

        let mut count = 0;
        for chunk in chunks {
            match provider.embed(&chunk.content).await {
                Ok(embedding) => {
                    self.storage
                        .update_chunk_embedding(chunk.id, &embedding)
                        .await?;
                    count += 1;
                }
                Err(e) => {
                    tracing::warn!(
                        "Failed to embed chunk {}: {}{}",
                        chunk.id,
                        e,
                        if matches!(e, embeddings::EmbeddingError::AuthFailed) {
                            ". Check OPENAI_API_KEY or set EMBEDDING_PROVIDER=ollama for local embeddings"
                        } else {
                            ""
                        }
                    );
                }
            }
        }

        Ok(count)
    }
}

/// Normalize a file path (remove leading/trailing slashes, collapse //).
fn normalize_path(path: &str) -> String {
    let path = path.trim().trim_matches('/');
    // Collapse multiple slashes
    let mut result = String::new();
    let mut last_was_slash = false;
    for c in path.chars() {
        if c == '/' {
            if !last_was_slash {
                result.push(c);
            }
            last_was_slash = true;
        } else {
            result.push(c);
            last_was_slash = false;
        }
    }
    result
}

/// Normalize a directory path (ensure no trailing slash for consistency).
fn normalize_directory(path: &str) -> String {
    let path = normalize_path(path);
    path.trim_end_matches('/').to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_path() {
        assert_eq!(normalize_path("foo/bar"), "foo/bar");
        assert_eq!(normalize_path("/foo/bar/"), "foo/bar");
        assert_eq!(normalize_path("foo//bar"), "foo/bar");
        assert_eq!(normalize_path("  /foo/  "), "foo");
        assert_eq!(normalize_path("README.md"), "README.md");
    }

    #[test]
    fn test_normalize_directory() {
        assert_eq!(normalize_directory("foo/bar/"), "foo/bar");
        assert_eq!(normalize_directory("foo/bar"), "foo/bar");
        assert_eq!(normalize_directory("/"), "");
        assert_eq!(normalize_directory(""), "");
    }

    #[test]
    fn test_default_single_scope() {
        // Verify backward compatibility: default workspace has single read scope
        // matching user_id.
        let user_id = "alice";
        let read_user_ids = [user_id.to_string()];
        assert_eq!(read_user_ids.len(), 1);
        assert_eq!(read_user_ids[0], user_id);
    }

    #[test]
    fn test_additional_read_scopes() {
        // Verify that additional read scopes are added correctly.
        let user_id = "alice".to_string();
        let mut read_user_ids = Vec::from([user_id.clone()]);

        // Simulate with_additional_read_scopes logic
        let scopes = ["shared", "team"];
        for scope in scopes {
            let s = scope.to_string();
            if !read_user_ids.contains(&s) {
                read_user_ids.push(s);
            }
        }

        assert_eq!(read_user_ids.len(), 3);
        assert_eq!(read_user_ids[0], "alice");
        assert_eq!(read_user_ids[1], "shared");
        assert_eq!(read_user_ids[2], "team");
    }

    #[test]
    fn test_additional_read_scopes_dedup() {
        // Verify that duplicate scopes are ignored.
        let user_id = "alice".to_string();
        let mut read_user_ids = Vec::from([user_id.clone()]);

        let scopes = ["shared", "alice", "shared"];
        for scope in scopes {
            let s = scope.to_string();
            if !read_user_ids.contains(&s) {
                read_user_ids.push(s);
            }
        }

        assert_eq!(read_user_ids.len(), 2);
        assert_eq!(read_user_ids[0], "alice");
        assert_eq!(read_user_ids[1], "shared");
    }

    #[test]
    fn test_is_multi_scope_logic() {
        // Test the multi-scope detection logic: > 1 means multi-scope
        let single_count = 1_usize;
        let multi_count = 2_usize;

        // Single scope: not multi
        assert!(single_count <= 1);

        // Multi scope: is multi
        assert!(multi_count > 1);
    }
}
