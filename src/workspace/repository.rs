//! Database repository for workspace persistence.
//!
//! All workspace data is stored in PostgreSQL:
//! - Documents in `memory_documents` table
//! - Chunks in `memory_chunks` table (with FTS and vector indexes)

use chrono::{DateTime, Utc};
use deadpool_postgres::Pool;
use pgvector::Vector;
use tokio_postgres::error::SqlState;
use uuid::Uuid;

use crate::error::WorkspaceError;

use crate::workspace::document::{
    ChunkWrite, DocumentVersion, MemoryChunk, MemoryDocument, VersionSummary, WorkspaceEntry,
};
use crate::workspace::search::{RankedResult, SearchConfig, SearchResult, fuse_results};

/// Database repository for workspace operations.
#[derive(Clone)]
pub struct Repository {
    pool: Pool,
}

impl Repository {
    /// Create a new repository with a connection pool.
    pub fn new(pool: Pool) -> Self {
        Self { pool }
    }

    /// Get a connection from the pool.
    async fn conn(&self) -> Result<deadpool_postgres::Object, WorkspaceError> {
        self.pool
            .get()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("Failed to get connection: {}", e),
            })
    }

    fn workspace_scope_id(user_id: &str) -> Option<Uuid> {
        user_id
            .strip_prefix("workspace:")
            .and_then(|id| Uuid::parse_str(id).ok())
    }

    // ==================== Document Operations ====================

    /// Get a document by its path.
    pub async fn get_document_by_path(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        path: &str,
    ) -> Result<MemoryDocument, WorkspaceError> {
        let conn = self.conn().await?;

        let row = conn
            .query_opt(
                r#"
                SELECT id, user_id, workspace_id, agent_id, path, content,
                       created_at, updated_at, metadata
                FROM memory_documents
                WHERE user_id = $1 AND agent_id IS NOT DISTINCT FROM $2 AND path = $3
                "#,
                &[&user_id, &agent_id, &path],
            )
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("Query failed: {}", e),
            })?;

        match row {
            Some(row) => Ok(self.row_to_document(&row)),
            None => Err(WorkspaceError::DocumentNotFound {
                doc_type: path.to_string(),
                user_id: user_id.to_string(),
            }),
        }
    }

    /// Get a document by ID.
    pub async fn get_document_by_id(&self, id: Uuid) -> Result<MemoryDocument, WorkspaceError> {
        let conn = self.conn().await?;

        let row = conn
            .query_opt(
                r#"
                SELECT id, user_id, workspace_id, agent_id, path, content,
                       created_at, updated_at, metadata
                FROM memory_documents WHERE id = $1
                "#,
                &[&id],
            )
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("Query failed: {}", e),
            })?;

        match row {
            Some(row) => Ok(self.row_to_document(&row)),
            None => Err(WorkspaceError::DocumentNotFound {
                doc_type: "unknown".to_string(),
                user_id: "unknown".to_string(),
            }),
        }
    }

    /// Get or create a document by path.
    pub async fn get_or_create_document_by_path(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        path: &str,
    ) -> Result<MemoryDocument, WorkspaceError> {
        // Try to get existing document first
        match self.get_document_by_path(user_id, agent_id, path).await {
            Ok(doc) => return Ok(doc),
            Err(WorkspaceError::DocumentNotFound { .. }) => {}
            Err(e) => return Err(e),
        }

        // Create new document
        let conn = self.conn().await?;
        let id = Uuid::new_v4();
        let now = Utc::now();
        let metadata = serde_json::json!({});
        let workspace_id = Self::workspace_scope_id(user_id);

        match conn
            .execute(
            r#"
            INSERT INTO memory_documents (id, user_id, workspace_id, agent_id, path, content, metadata, created_at, updated_at)
            VALUES ($1, $2, $3, $4, $5, '', $6, $7, $8)
            "#,
            &[&id, &user_id, &workspace_id, &agent_id, &path, &metadata, &now, &now],
        )
        .await
        {
            Ok(_) => {}
            Err(e) if e.code() == Some(&SqlState::UNIQUE_VIOLATION) => {}
            Err(e) => {
                return Err(WorkspaceError::SearchFailed {
                    reason: format!("Insert failed: {}", e),
                });
            }
        }

        // Fetch the document (might have been created by concurrent request)
        self.get_document_by_path(user_id, agent_id, path).await
    }

    /// Update a document's content.
    pub async fn update_document(&self, id: Uuid, content: &str) -> Result<(), WorkspaceError> {
        let conn = self.conn().await?;

        conn.execute(
            "UPDATE memory_documents SET content = $2, updated_at = NOW() WHERE id = $1",
            &[&id, &content],
        )
        .await
        .map_err(|e| WorkspaceError::SearchFailed {
            reason: format!("Update failed: {}", e),
        })?;

        Ok(())
    }

    /// Delete a document by its path.
    pub async fn delete_document_by_path(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        path: &str,
    ) -> Result<(), WorkspaceError> {
        let conn = self.conn().await?;

        let doc = self.get_document_by_path(user_id, agent_id, path).await?;
        self.delete_chunks(doc.id).await?;

        conn.execute(
            r#"
            DELETE FROM memory_documents
            WHERE user_id = $1 AND agent_id IS NOT DISTINCT FROM $2 AND path = $3
            "#,
            &[&user_id, &agent_id, &path],
        )
        .await
        .map_err(|e| WorkspaceError::SearchFailed {
            reason: format!("Delete failed: {}", e),
        })?;

        Ok(())
    }

    /// List files and directories in a directory path.
    ///
    /// Returns immediate children (not recursive).
    /// Empty string lists the root directory.
    pub async fn list_directory(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        directory: &str,
    ) -> Result<Vec<WorkspaceEntry>, WorkspaceError> {
        let conn = self.conn().await?;

        let rows = conn
            .query(
                "SELECT path, is_directory, updated_at, content_preview FROM list_workspace_files($1, $2, $3)",
                &[&user_id, &agent_id, &directory],
            )
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("List directory failed: {}", e),
            })?;

        Ok(rows
            .iter()
            .map(|row| {
                let updated_at: Option<DateTime<Utc>> = row.get("updated_at");
                WorkspaceEntry {
                    path: row.get("path"),
                    is_directory: row.get("is_directory"),
                    updated_at,
                    content_preview: row.get("content_preview"),
                }
            })
            .collect())
    }

    /// List all file paths in the workspace (flat list).
    pub async fn list_all_paths(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
    ) -> Result<Vec<String>, WorkspaceError> {
        let conn = self.conn().await?;

        let rows = conn
            .query(
                r#"
                SELECT path FROM memory_documents
                WHERE user_id = $1 AND agent_id IS NOT DISTINCT FROM $2
                ORDER BY path
                "#,
                &[&user_id, &agent_id],
            )
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("List paths failed: {}", e),
            })?;

        Ok(rows.iter().map(|row| row.get("path")).collect())
    }

    /// List all documents for a user.
    pub async fn list_documents(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
    ) -> Result<Vec<MemoryDocument>, WorkspaceError> {
        let conn = self.conn().await?;

        let rows = conn
            .query(
                r#"
                SELECT id, user_id, workspace_id, agent_id, path, content,
                       created_at, updated_at, metadata
                FROM memory_documents
                WHERE user_id = $1 AND agent_id IS NOT DISTINCT FROM $2
                ORDER BY updated_at DESC
                "#,
                &[&user_id, &agent_id],
            )
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("Query failed: {}", e),
            })?;

        Ok(rows.iter().map(|r| self.row_to_document(r)).collect())
    }

    fn row_to_document(&self, row: &tokio_postgres::Row) -> MemoryDocument {
        MemoryDocument {
            id: row.get("id"),
            user_id: row.get("user_id"),
            workspace_id: row.get("workspace_id"),
            agent_id: row.get("agent_id"),
            path: row.get("path"),
            content: row.get("content"),
            created_at: row.get("created_at"),
            updated_at: row.get("updated_at"),
            metadata: row.get("metadata"),
        }
    }

    // ==================== Chunk Operations ====================

    /// Delete all chunks for a document.
    pub async fn delete_chunks(&self, document_id: Uuid) -> Result<(), WorkspaceError> {
        let conn = self.conn().await?;

        conn.execute(
            "DELETE FROM memory_chunks WHERE document_id = $1",
            &[&document_id],
        )
        .await
        .map_err(|e| WorkspaceError::ChunkingFailed {
            reason: format!("Delete failed: {}", e),
        })?;

        Ok(())
    }

    /// Insert a chunk.
    pub async fn insert_chunk(
        &self,
        document_id: Uuid,
        chunk_index: i32,
        content: &str,
        embedding: Option<&[f32]>,
    ) -> Result<Uuid, WorkspaceError> {
        let conn = self.conn().await?;
        let id = Uuid::new_v4();

        let embedding_vec = embedding.map(|e| Vector::from(e.to_vec()));

        conn.execute(
            r#"
            INSERT INTO memory_chunks (id, document_id, chunk_index, content, embedding)
            VALUES ($1, $2, $3, $4, $5)
            "#,
            &[&id, &document_id, &chunk_index, &content, &embedding_vec],
        )
        .await
        .map_err(|e| WorkspaceError::ChunkingFailed {
            reason: format!("Insert failed: {}", e),
        })?;

        Ok(id)
    }

    /// Atomically replace all chunks for a document.
    ///
    /// Runs `DELETE` + N `INSERT`s inside a single transaction so two
    /// concurrent reindexers for the same document cannot race each other
    /// into a `UNIQUE (document_id, chunk_index)` violation. Passing an
    /// empty slice is equivalent to `delete_chunks(document_id)`.
    ///
    /// Postgres-specific concurrency note: bundling DELETE + INSERTs in one
    /// transaction is **not** sufficient on Postgres. Two concurrent
    /// reindexers running under separate snapshots can both DELETE (each
    /// sees its own pre-delete state, neither sees the other's), then race
    /// to INSERT chunk_index 0 — at which point one side hits the
    /// `UNIQUE (document_id, chunk_index)` constraint. We serialize
    /// per-document by acquiring a row lock on the parent `memory_documents`
    /// row at the start of the transaction. The lock is released
    /// automatically on commit/rollback. The `FOR UPDATE` row exists in the
    /// schema with an FK from `memory_chunks.document_id`, so the lookup is
    /// always cheap and always finds a row (callers wouldn't be reindexing
    /// chunks for a document that doesn't exist).
    pub async fn replace_chunks(
        &self,
        document_id: Uuid,
        chunks: &[ChunkWrite],
    ) -> Result<(), WorkspaceError> {
        let mut conn = self.conn().await?;

        let tx = conn
            .transaction()
            .await
            .map_err(|e| WorkspaceError::ChunkingFailed {
                reason: format!("Begin transaction failed: {e}"),
            })?;

        // Per-document serialization: block any other reindexer of the
        // same document until this transaction commits. Pinned by
        // `concurrent_writes_to_same_doc_do_not_collide_on_chunk_index`
        // (the libsql variant of the same regression test).
        let locked = tx
            .execute(
                "SELECT 1 FROM memory_documents WHERE id = $1 FOR UPDATE",
                &[&document_id],
            )
            .await
            .map_err(|e| WorkspaceError::ChunkingFailed {
                reason: format!("Acquire row lock failed: {e}"),
            })?;
        if locked == 0 {
            return Err(WorkspaceError::ChunkingFailed {
                reason: format!(
                    "Document {document_id} not found — cannot acquire per-document lock for chunk replacement"
                ),
            });
        }

        tx.execute(
            "DELETE FROM memory_chunks WHERE document_id = $1",
            &[&document_id],
        )
        .await
        .map_err(|e| WorkspaceError::ChunkingFailed {
            reason: format!("Delete failed: {}", e),
        })?;

        for (index, chunk) in chunks.iter().enumerate() {
            let id = Uuid::new_v4();
            let chunk_index = index as i32;
            let embedding_vec = chunk.embedding.as_ref().map(|e| Vector::from(e.clone()));
            let content = chunk.content.as_str();

            tx.execute(
                r#"
                INSERT INTO memory_chunks (id, document_id, chunk_index, content, embedding)
                VALUES ($1, $2, $3, $4, $5)
                "#,
                &[&id, &document_id, &chunk_index, &content, &embedding_vec],
            )
            .await
            .map_err(|e| WorkspaceError::ChunkingFailed {
                reason: format!("Insert failed: {}", e),
            })?;
        }

        tx.commit()
            .await
            .map_err(|e| WorkspaceError::ChunkingFailed {
                reason: format!("Commit failed: {e}"),
            })?;

        Ok(())
    }

    /// Update a chunk's embedding.
    pub async fn update_chunk_embedding(
        &self,
        chunk_id: Uuid,
        embedding: &[f32],
    ) -> Result<(), WorkspaceError> {
        let conn = self.conn().await?;
        let embedding_vec = Vector::from(embedding.to_vec());

        conn.execute(
            "UPDATE memory_chunks SET embedding = $2 WHERE id = $1",
            &[&chunk_id, &embedding_vec],
        )
        .await
        .map_err(|e| WorkspaceError::EmbeddingFailed {
            reason: format!("Update failed: {}", e),
        })?;

        Ok(())
    }

    /// Get chunks without embeddings for backfilling.
    pub async fn get_chunks_without_embeddings(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        limit: usize,
    ) -> Result<Vec<MemoryChunk>, WorkspaceError> {
        let conn = self.conn().await?;

        let rows = conn
            .query(
                r#"
                SELECT c.id, c.document_id, c.chunk_index, c.content, c.created_at
                FROM memory_chunks c
                JOIN memory_documents d ON d.id = c.document_id
                WHERE d.user_id = $1 AND d.agent_id IS NOT DISTINCT FROM $2
                  AND c.embedding IS NULL
                LIMIT $3
                "#,
                &[&user_id, &agent_id, &(limit as i64)],
            )
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("Query failed: {}", e),
            })?;

        Ok(rows
            .iter()
            .map(|row| MemoryChunk {
                id: row.get("id"),
                document_id: row.get("document_id"),
                chunk_index: row.get("chunk_index"),
                content: row.get("content"),
                embedding: None,
                created_at: row.get("created_at"),
            })
            .collect())
    }

    // ==================== Search Operations ====================

    /// Perform hybrid search combining FTS and vector similarity.
    pub async fn hybrid_search(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        query: &str,
        embedding: Option<&[f32]>,
        config: &SearchConfig,
    ) -> Result<Vec<SearchResult>, WorkspaceError> {
        let fts_results = if config.use_fts {
            self.fts_search(user_id, agent_id, query, config.pre_fusion_limit)
                .await?
        } else {
            Vec::new()
        };

        let vector_results = if config.use_vector {
            if let Some(embedding) = embedding {
                self.vector_search(user_id, agent_id, embedding, config.pre_fusion_limit)
                    .await?
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };

        Ok(fuse_results(fts_results, vector_results, config))
    }

    /// Full-text search using PostgreSQL ts_rank_cd.
    async fn fts_search(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        query: &str,
        limit: usize,
    ) -> Result<Vec<RankedResult>, WorkspaceError> {
        let conn = self.conn().await?;

        let rows = conn
            .query(
                r#"
                SELECT c.id as chunk_id, c.document_id, d.path as document_path, c.content,
                       ts_rank_cd(c.content_tsv, plainto_tsquery('english', $3)) as rank
                FROM memory_chunks c
                JOIN memory_documents d ON d.id = c.document_id
                WHERE d.user_id = $1 AND d.agent_id IS NOT DISTINCT FROM $2
                  AND c.content_tsv @@ plainto_tsquery('english', $3)
                ORDER BY rank DESC
                LIMIT $4
                "#,
                &[&user_id, &agent_id, &query, &(limit as i64)],
            )
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("FTS query failed: {}", e),
            })?;

        Ok(rows
            .iter()
            .enumerate()
            .map(|(i, row)| RankedResult {
                chunk_id: row.get("chunk_id"),
                document_id: row.get("document_id"),
                document_path: row.get("document_path"),
                content: row.get("content"),
                rank: (i + 1) as u32,
            })
            .collect())
    }

    /// Vector similarity search using pgvector cosine distance.
    async fn vector_search(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        embedding: &[f32],
        limit: usize,
    ) -> Result<Vec<RankedResult>, WorkspaceError> {
        let conn = self.conn().await?;
        let embedding_vec = Vector::from(embedding.to_vec());

        let rows = conn
            .query(
                r#"
                SELECT c.id as chunk_id, c.document_id, d.path as document_path, c.content,
                       1 - (c.embedding <=> $3) as similarity
                FROM memory_chunks c
                JOIN memory_documents d ON d.id = c.document_id
                WHERE d.user_id = $1 AND d.agent_id IS NOT DISTINCT FROM $2
                  AND c.embedding IS NOT NULL
                ORDER BY c.embedding <=> $3
                LIMIT $4
                "#,
                &[&user_id, &agent_id, &embedding_vec, &(limit as i64)],
            )
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("Vector query failed: {}", e),
            })?;

        Ok(rows
            .iter()
            .enumerate()
            .map(|(i, row)| RankedResult {
                chunk_id: row.get("chunk_id"),
                document_id: row.get("document_id"),
                document_path: row.get("document_path"),
                content: row.get("content"),
                rank: (i + 1) as u32,
            })
            .collect())
    }

    // ==================== Multi-scope search (optimized SQL) ====================

    /// Hybrid search across multiple user scopes with efficient SQL.
    ///
    /// Uses `user_id = ANY($1::text[])` instead of N separate queries.
    pub async fn hybrid_search_multi(
        &self,
        user_ids: &[String],
        agent_id: Option<Uuid>,
        query: &str,
        embedding: Option<&[f32]>,
        config: &SearchConfig,
    ) -> Result<Vec<SearchResult>, WorkspaceError> {
        let fts_results = if config.use_fts {
            self.fts_search_multi(user_ids, agent_id, query, config.pre_fusion_limit)
                .await?
        } else {
            Vec::new()
        };

        let vector_results = if config.use_vector {
            if let Some(embedding) = embedding {
                self.vector_search_multi(user_ids, agent_id, embedding, config.pre_fusion_limit)
                    .await?
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };

        Ok(fuse_results(fts_results, vector_results, config))
    }

    /// FTS search across multiple user scopes.
    async fn fts_search_multi(
        &self,
        user_ids: &[String],
        agent_id: Option<Uuid>,
        query: &str,
        limit: usize,
    ) -> Result<Vec<RankedResult>, WorkspaceError> {
        let conn = self.conn().await?;

        let rows = conn
            .query(
                r#"
                SELECT c.id as chunk_id, c.document_id, d.path as document_path,
                       c.content,
                       ts_rank_cd(c.content_tsv, plainto_tsquery('english', $3)) as rank
                FROM memory_chunks c
                JOIN memory_documents d ON d.id = c.document_id
                WHERE d.user_id = ANY($1::text[]) AND d.agent_id IS NOT DISTINCT FROM $2
                  AND c.content_tsv @@ plainto_tsquery('english', $3)
                ORDER BY rank DESC
                LIMIT $4
                "#,
                &[&user_ids, &agent_id, &query, &(limit as i64)],
            )
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("FTS multi-scope query failed: {}", e),
            })?;

        Ok(rows
            .iter()
            .enumerate()
            .map(|(i, row)| RankedResult {
                chunk_id: row.get("chunk_id"),
                document_id: row.get("document_id"),
                document_path: row.get("document_path"),
                content: row.get("content"),
                rank: (i + 1) as u32,
            })
            .collect())
    }

    /// Vector search across multiple user scopes.
    async fn vector_search_multi(
        &self,
        user_ids: &[String],
        agent_id: Option<Uuid>,
        embedding: &[f32],
        limit: usize,
    ) -> Result<Vec<RankedResult>, WorkspaceError> {
        let conn = self.conn().await?;
        let embedding_vec = Vector::from(embedding.to_vec());

        let rows = conn
            .query(
                r#"
                SELECT c.id as chunk_id, c.document_id, d.path as document_path,
                       c.content, 1 - (c.embedding <=> $3) as similarity
                FROM memory_chunks c
                JOIN memory_documents d ON d.id = c.document_id
                WHERE d.user_id = ANY($1::text[]) AND d.agent_id IS NOT DISTINCT FROM $2
                  AND c.embedding IS NOT NULL
                ORDER BY c.embedding <=> $3
                LIMIT $4
                "#,
                &[&user_ids, &agent_id, &embedding_vec, &(limit as i64)],
            )
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("Vector multi-scope query failed: {}", e),
            })?;

        Ok(rows
            .iter()
            .enumerate()
            .map(|(i, row)| RankedResult {
                chunk_id: row.get("chunk_id"),
                document_id: row.get("document_id"),
                document_path: row.get("document_path"),
                content: row.get("content"),
                rank: (i + 1) as u32,
            })
            .collect())
    }

    /// List all file paths across multiple user scopes with a single query.
    pub async fn list_all_paths_multi(
        &self,
        user_ids: &[String],
        agent_id: Option<Uuid>,
    ) -> Result<Vec<String>, WorkspaceError> {
        let conn = self.conn().await?;

        let rows = conn
            .query(
                r#"
                SELECT DISTINCT path FROM memory_documents
                WHERE user_id = ANY($1::text[]) AND agent_id IS NOT DISTINCT FROM $2
                ORDER BY path
                "#,
                &[&user_ids, &agent_id],
            )
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("List paths multi-scope failed: {}", e),
            })?;

        Ok(rows.iter().map(|row| row.get("path")).collect())
    }

    /// Get a document by path across multiple user scopes.
    ///
    /// Returns the first match (ordered by the input user_ids priority).
    pub async fn get_document_by_path_multi(
        &self,
        user_ids: &[String],
        agent_id: Option<Uuid>,
        path: &str,
    ) -> Result<MemoryDocument, WorkspaceError> {
        let conn = self.conn().await?;

        let row = conn
            .query_opt(
                r#"
                SELECT id, user_id, workspace_id, agent_id, path, content,
                       created_at, updated_at, metadata
                FROM memory_documents
                WHERE user_id = ANY($1::text[]) AND agent_id IS NOT DISTINCT FROM $2 AND path = $3
                ORDER BY array_position($1::text[], user_id)
                LIMIT 1
                "#,
                &[&user_ids, &agent_id, &path],
            )
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("get_document_by_path_multi failed: {}", e),
            })?;

        match row {
            Some(row) => Ok(self.row_to_document(&row)),
            None => Err(WorkspaceError::DocumentNotFound {
                doc_type: path.to_string(),
                user_id: format!("[{}]", user_ids.join(", ")),
            }),
        }
    }

    /// List directory contents across multiple user scopes.
    ///
    /// Iterates per scope and merges results. A future migration could add an
    /// optimised SQL function, at which point this method can call it directly.
    pub async fn list_directory_multi(
        &self,
        user_ids: &[String],
        agent_id: Option<Uuid>,
        directory: &str,
    ) -> Result<Vec<WorkspaceEntry>, WorkspaceError> {
        let mut all_entries = Vec::new();
        for uid in user_ids {
            all_entries.extend(self.list_directory(uid, agent_id, directory).await?);
        }
        Ok(crate::workspace::merge_workspace_entries(all_entries))
    }

    // ==================== Metadata ====================

    pub async fn update_document_metadata(
        &self,
        id: Uuid,
        metadata: &serde_json::Value,
    ) -> Result<(), WorkspaceError> {
        let conn = self.conn().await?;
        conn.execute(
            "UPDATE memory_documents SET metadata = $2, updated_at = NOW() WHERE id = $1",
            &[&id, &metadata],
        )
        .await
        .map_err(|e| WorkspaceError::SearchFailed {
            reason: format!("Failed to update metadata: {e}"),
        })?;
        Ok(())
    }

    pub async fn find_config_documents(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
    ) -> Result<Vec<MemoryDocument>, WorkspaceError> {
        let conn = self.conn().await?;
        let rows = conn
            .query(
                r#"
                SELECT id, user_id, workspace_id, agent_id, path, content,
                       created_at, updated_at, metadata
                FROM memory_documents
                WHERE user_id = $1 AND agent_id IS NOT DISTINCT FROM $2
                  AND (path LIKE '%/.config' OR path = '.config')
                ORDER BY path
                "#,
                &[&user_id, &agent_id],
            )
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("Failed to find config documents: {e}"),
            })?;
        Ok(rows.iter().map(|r| self.row_to_document(r)).collect())
    }

    // ==================== Versioning ====================

    pub async fn save_version(
        &self,
        document_id: Uuid,
        content: &str,
        content_hash: &str,
        changed_by: Option<&str>,
    ) -> Result<i32, WorkspaceError> {
        let mut conn = self.conn().await?;

        // Use a transaction to prevent concurrent writers from allocating
        // the same version number. The SELECT FOR UPDATE locks the existing
        // version rows for this document, serializing concurrent inserts.
        let tx = conn
            .transaction()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("Failed to start transaction: {e}"),
            })?;

        // Lock the parent document row to serialize concurrent version writes.
        // We lock memory_documents (which always exists) instead of
        // memory_document_versions (which may have no rows yet — FOR UPDATE
        // on an empty result set locks nothing).
        tx.execute(
            "SELECT 1 FROM memory_documents WHERE id = $1 FOR UPDATE",
            &[&document_id],
        )
        .await
        .map_err(|e| WorkspaceError::SearchFailed {
            reason: format!("Failed to lock document for versioning: {e}"),
        })?;

        let row = tx
            .query_one(
                "SELECT COALESCE(MAX(version), 0) + 1 AS next_version \
                 FROM memory_document_versions WHERE document_id = $1",
                &[&document_id],
            )
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("Failed to get next version number: {e}"),
            })?;
        let next_version: i32 = row.get(0);

        tx.execute(
            r#"
            INSERT INTO memory_document_versions
                (id, document_id, version, content, content_hash, changed_by)
            VALUES (gen_random_uuid(), $1, $2, $3, $4, $5)
            "#,
            &[
                &document_id,
                &next_version,
                &content,
                &content_hash,
                &changed_by,
            ],
        )
        .await
        .map_err(|e| WorkspaceError::SearchFailed {
            reason: format!("Failed to save version: {e}"),
        })?;

        tx.commit()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("Failed to commit version: {e}"),
            })?;

        Ok(next_version)
    }

    pub async fn get_version(
        &self,
        document_id: Uuid,
        version: i32,
    ) -> Result<DocumentVersion, WorkspaceError> {
        let conn = self.conn().await?;
        let row = conn
            .query_opt(
                r#"
                SELECT id, document_id, version, content, content_hash,
                       created_at, changed_by
                FROM memory_document_versions
                WHERE document_id = $1 AND version = $2
                "#,
                &[&document_id, &version],
            )
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("Failed to get version: {e}"),
            })?
            .ok_or(WorkspaceError::VersionNotFound {
                document_id,
                version,
            })?;
        Ok(DocumentVersion {
            id: row.get(0),
            document_id: row.get(1),
            version: row.get(2),
            content: row.get(3),
            content_hash: row.get(4),
            created_at: row.get(5),
            changed_by: row.get(6),
        })
    }

    pub async fn list_versions(
        &self,
        document_id: Uuid,
        limit: i64,
    ) -> Result<Vec<VersionSummary>, WorkspaceError> {
        let conn = self.conn().await?;
        let rows = conn
            .query(
                r#"
                SELECT version, content_hash, created_at, changed_by
                FROM memory_document_versions
                WHERE document_id = $1
                ORDER BY version DESC
                LIMIT $2
                "#,
                &[&document_id, &limit],
            )
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("Failed to list versions: {e}"),
            })?;
        Ok(rows
            .iter()
            .map(|row| VersionSummary {
                version: row.get(0),
                content_hash: row.get(1),
                created_at: row.get(2),
                changed_by: row.get(3),
            })
            .collect())
    }

    pub async fn get_latest_version_number(
        &self,
        document_id: Uuid,
    ) -> Result<Option<i32>, WorkspaceError> {
        let conn = self.conn().await?;
        let row = conn
            .query_one(
                "SELECT MAX(version) FROM memory_document_versions WHERE document_id = $1",
                &[&document_id],
            )
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("Failed to get latest version number: {e}"),
            })?;
        Ok(row.get(0))
    }

    pub async fn prune_versions(
        &self,
        document_id: Uuid,
        keep_count: i32,
    ) -> Result<u64, WorkspaceError> {
        let conn = self.conn().await?;
        let result = conn
            .execute(
                r#"
                DELETE FROM memory_document_versions
                WHERE document_id = $1
                  AND version NOT IN (
                      SELECT version FROM memory_document_versions
                      WHERE document_id = $1
                      ORDER BY version DESC
                      LIMIT $2
                  )
                "#,
                &[&document_id, &(keep_count as i64)],
            )
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("Failed to prune versions: {e}"),
            })?;
        Ok(result)
    }
}
