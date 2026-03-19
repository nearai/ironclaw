//! Workspace-related WorkspaceStore implementation for LibSqlBackend.

use std::collections::HashMap;

use async_trait::async_trait;
use libsql::params;
use uuid::Uuid;

use super::{
    LibSqlBackend, fmt_ts, get_i64, get_opt_text, get_opt_ts, get_text, get_ts,
    row_to_memory_document,
};
use crate::db::WorkspaceStore;
use crate::error::{DatabaseError, WorkspaceError};
use crate::workspace::{
    MemoryChunk, MemoryDocument, RankedResult, SearchConfig, SearchResult, WorkspaceEntry,
    fuse_results,
};

use chrono::Utc;

/// Resolve the embedding dimension from environment variables.
///
/// Mirrors the logic in `config/embeddings.rs`: checks `EMBEDDING_ENABLED`,
/// reads `EMBEDDING_DIMENSION` or infers from `EMBEDDING_MODEL`.
/// Returns `None` if embeddings are disabled or not configured.
pub(crate) fn resolve_embedding_dimension() -> Option<usize> {
    let enabled = std::env::var("EMBEDDING_ENABLED")
        .map(|v| v.eq_ignore_ascii_case("true") || v == "1")
        .unwrap_or(false);

    if !enabled {
        return None;
    }

    if let Ok(dim_str) = std::env::var("EMBEDDING_DIMENSION")
        && let Ok(dim) = dim_str.parse::<usize>()
        && dim > 0
    {
        return Some(dim);
    }

    let model =
        std::env::var("EMBEDDING_MODEL").unwrap_or_else(|_| "text-embedding-3-small".to_string());

    let dim = match model.as_str() {
        "text-embedding-3-small" => 1536,
        "text-embedding-3-large" => 3072,
        "text-embedding-ada-002" => 1536,
        "nomic-embed-text" => 768,
        "mxbai-embed-large" => 1024,
        "all-minilm" => 384,
        _ => 1536,
    };

    Some(dim)
}

impl LibSqlBackend {
    /// Ensure the `libsql_vector_idx` on `memory_chunks.embedding` matches the
    /// configured embedding dimension.
    ///
    /// The V9 migration dropped the vector index (and changed `F32_BLOB(1536)`
    /// to `BLOB`) to support flexible dimensions. This method restores a
    /// properly-typed `F32_BLOB(N)` column and creates the vector index.
    ///
    /// Uses `_migrations` version `0` (a reserved metadata row) to track the
    /// current dimension. If the dimension is unchanged, this is a no-op.
    pub async fn ensure_vector_index(&self, dimension: usize) -> Result<(), DatabaseError> {
        let conn = self.connect().await?;

        // Check current dimension from _migrations version=0 (reserved metadata row).
        let current_dim = {
            let mut rows = conn
                .query("SELECT name FROM _migrations WHERE version = 0", ())
                .await
                .map_err(|e| {
                    DatabaseError::Migration(format!("Failed to check vector index metadata: {e}"))
                })?;

            rows.next().await.ok().flatten().and_then(|row| {
                row.get::<String>(0)
                    .ok()
                    .and_then(|s| s.parse::<usize>().ok())
            })
        };

        if current_dim == Some(dimension) {
            tracing::debug!(
                dimension,
                "Vector index already matches configured dimension"
            );
            return Ok(());
        }

        tracing::info!(
            old_dimension = ?current_dim,
            new_dimension = dimension,
            "Rebuilding memory_chunks table for vector index"
        );

        let tx = conn.transaction().await.map_err(|e| {
            DatabaseError::Migration(format!(
                "ensure_vector_index: failed to start transaction: {e}"
            ))
        })?;

        // 1. Drop FTS triggers that reference the old table
        tx.execute_batch(
            "DROP TRIGGER IF EXISTS memory_chunks_fts_insert;
             DROP TRIGGER IF EXISTS memory_chunks_fts_delete;
             DROP TRIGGER IF EXISTS memory_chunks_fts_update;",
        )
        .await
        .map_err(|e| DatabaseError::Migration(format!("Failed to drop FTS triggers: {e}")))?;

        // 2. Drop old vector index
        tx.execute_batch("DROP INDEX IF EXISTS idx_memory_chunks_embedding;")
            .await
            .map_err(|e| {
                DatabaseError::Migration(format!("Failed to drop old vector index: {e}"))
            })?;

        // 3. Create new table with F32_BLOB(dim)
        let create_sql = format!(
            "CREATE TABLE IF NOT EXISTS memory_chunks_new (
                _rowid INTEGER PRIMARY KEY AUTOINCREMENT,
                id TEXT NOT NULL UNIQUE,
                document_id TEXT NOT NULL REFERENCES memory_documents(id) ON DELETE CASCADE,
                chunk_index INTEGER NOT NULL,
                content TEXT NOT NULL,
                embedding F32_BLOB({dimension}),
                created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
                UNIQUE (document_id, chunk_index)
            )"
        );
        tx.execute_batch(&create_sql).await.map_err(|e| {
            DatabaseError::Migration(format!(
                "Failed to create memory_chunks_new with F32_BLOB({dimension}): {e}"
            ))
        })?;

        // 4. Copy data — embeddings with wrong byte length get NULLed
        //    (they will be re-embedded on next background pass)
        let expected_bytes = dimension * 4;
        let copy_sql = format!(
            "INSERT OR IGNORE INTO memory_chunks_new
                (_rowid, id, document_id, chunk_index, content, embedding, created_at)
             SELECT _rowid, id, document_id, chunk_index, content,
                    CASE WHEN length(embedding) = {expected_bytes} THEN embedding ELSE NULL END,
                    created_at
             FROM memory_chunks"
        );
        tx.execute_batch(&copy_sql).await.map_err(|e| {
            DatabaseError::Migration(format!("Failed to copy data to memory_chunks_new: {e}"))
        })?;

        // 5. Swap tables
        tx.execute_batch(
            "DROP TABLE memory_chunks;
             ALTER TABLE memory_chunks_new RENAME TO memory_chunks;",
        )
        .await
        .map_err(|e| {
            DatabaseError::Migration(format!("Failed to swap memory_chunks tables: {e}"))
        })?;

        // 6. Recreate document index + vector index
        tx.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_memory_chunks_document ON memory_chunks(document_id);
             CREATE INDEX IF NOT EXISTS idx_memory_chunks_embedding ON memory_chunks(libsql_vector_idx(embedding));",
        )
        .await
        .map_err(|e| {
            DatabaseError::Migration(format!("Failed to create indexes: {e}"))
        })?;

        // 7. Recreate FTS triggers
        tx.execute_batch(
            "CREATE TRIGGER IF NOT EXISTS memory_chunks_fts_insert AFTER INSERT ON memory_chunks BEGIN
                INSERT INTO memory_chunks_fts(rowid, content) VALUES (new._rowid, new.content);
            END;

            CREATE TRIGGER IF NOT EXISTS memory_chunks_fts_delete AFTER DELETE ON memory_chunks BEGIN
                INSERT INTO memory_chunks_fts(memory_chunks_fts, rowid, content)
                    VALUES ('delete', old._rowid, old.content);
            END;

            CREATE TRIGGER IF NOT EXISTS memory_chunks_fts_update AFTER UPDATE ON memory_chunks BEGIN
                INSERT INTO memory_chunks_fts(memory_chunks_fts, rowid, content)
                    VALUES ('delete', old._rowid, old.content);
                INSERT INTO memory_chunks_fts(rowid, content) VALUES (new._rowid, new.content);
            END;",
        )
        .await
        .map_err(|e| {
            DatabaseError::Migration(format!("Failed to recreate FTS triggers: {e}"))
        })?;

        // 8. Upsert dimension into _migrations(version=0)
        tx.execute(
            "INSERT INTO _migrations (version, name) VALUES (0, ?1)
             ON CONFLICT(version) DO UPDATE SET name = ?1,
                applied_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')",
            params![dimension.to_string()],
        )
        .await
        .map_err(|e| {
            DatabaseError::Migration(format!("Failed to record vector index dimension: {e}"))
        })?;

        tx.commit().await.map_err(|e| {
            DatabaseError::Migration(format!("ensure_vector_index: commit failed: {e}"))
        })?;

        tracing::info!(dimension, "Vector index created successfully");
        Ok(())
    }
}

#[async_trait]
impl WorkspaceStore for LibSqlBackend {
    async fn get_document_by_path(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        path: &str,
    ) -> Result<MemoryDocument, WorkspaceError> {
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: e.to_string(),
            })?;
        let agent_id_str = agent_id.map(|id| id.to_string());
        let mut rows = conn
            .query(
                r#"
                SELECT id, user_id, agent_id, path, content,
                       created_at, updated_at, metadata
                FROM memory_documents
                WHERE user_id = ?1 AND agent_id IS ?2 AND path = ?3
                "#,
                params![user_id, agent_id_str.as_deref(), path],
            )
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("Query failed: {}", e),
            })?;

        match rows
            .next()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("Query failed: {}", e),
            })? {
            Some(row) => Ok(row_to_memory_document(&row)),
            None => Err(WorkspaceError::DocumentNotFound {
                doc_type: path.to_string(),
                user_id: user_id.to_string(),
            }),
        }
    }

    async fn get_document_by_id(&self, id: Uuid) -> Result<MemoryDocument, WorkspaceError> {
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: e.to_string(),
            })?;
        let mut rows = conn
            .query(
                r#"
                SELECT id, user_id, agent_id, path, content,
                       created_at, updated_at, metadata
                FROM memory_documents WHERE id = ?1
                "#,
                params![id.to_string()],
            )
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("Query failed: {}", e),
            })?;

        match rows
            .next()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("Query failed: {}", e),
            })? {
            Some(row) => Ok(row_to_memory_document(&row)),
            None => Err(WorkspaceError::DocumentNotFound {
                doc_type: "unknown".to_string(),
                user_id: "unknown".to_string(),
            }),
        }
    }

    async fn get_or_create_document_by_path(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        path: &str,
    ) -> Result<MemoryDocument, WorkspaceError> {
        // Try get
        match self.get_document_by_path(user_id, agent_id, path).await {
            Ok(doc) => return Ok(doc),
            Err(WorkspaceError::DocumentNotFound { .. }) => {}
            Err(e) => return Err(e),
        }

        // Create
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: e.to_string(),
            })?;
        let id = Uuid::new_v4();
        let agent_id_str = agent_id.map(|id| id.to_string());
        conn.execute(
            r#"
                INSERT INTO memory_documents (id, user_id, agent_id, path, content, metadata)
                VALUES (?1, ?2, ?3, ?4, '', '{}')
                ON CONFLICT (user_id, agent_id, path) DO NOTHING
                "#,
            params![id.to_string(), user_id, agent_id_str.as_deref(), path],
        )
        .await
        .map_err(|e| WorkspaceError::SearchFailed {
            reason: format!("Insert failed: {}", e),
        })?;

        self.get_document_by_path(user_id, agent_id, path).await
    }

    async fn update_document(&self, id: Uuid, content: &str) -> Result<(), WorkspaceError> {
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: e.to_string(),
            })?;
        let now = fmt_ts(&Utc::now());
        conn.execute(
            "UPDATE memory_documents SET content = ?2, updated_at = ?3 WHERE id = ?1",
            params![id.to_string(), content, now],
        )
        .await
        .map_err(|e| WorkspaceError::SearchFailed {
            reason: format!("Update failed: {}", e),
        })?;
        Ok(())
    }

    async fn delete_document_by_path(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        path: &str,
    ) -> Result<(), WorkspaceError> {
        let doc = self.get_document_by_path(user_id, agent_id, path).await?;
        self.delete_chunks(doc.id).await?;

        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: e.to_string(),
            })?;
        let agent_id_str = agent_id.map(|id| id.to_string());
        conn.execute(
            "DELETE FROM memory_documents WHERE user_id = ?1 AND agent_id IS ?2 AND path = ?3",
            params![user_id, agent_id_str.as_deref(), path],
        )
        .await
        .map_err(|e| WorkspaceError::SearchFailed {
            reason: format!("Delete failed: {}", e),
        })?;
        Ok(())
    }

    async fn list_directory(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        directory: &str,
    ) -> Result<Vec<WorkspaceEntry>, WorkspaceError> {
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: e.to_string(),
            })?;
        let dir = if !directory.is_empty() && !directory.ends_with('/') {
            format!("{}/", directory)
        } else {
            directory.to_string()
        };

        let agent_id_str = agent_id.map(|id| id.to_string());
        let pattern = if dir.is_empty() {
            "%".to_string()
        } else {
            format!("{}%", dir)
        };

        let mut rows = conn
            .query(
                r#"
                SELECT path, updated_at, substr(content, 1, 200) as content_preview
                FROM memory_documents
                WHERE user_id = ?1 AND agent_id IS ?2
                  AND (?3 = '%' OR path LIKE ?3)
                ORDER BY path
                "#,
                params![user_id, agent_id_str.as_deref(), pattern],
            )
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("List directory failed: {}", e),
            })?;

        let mut entries_map: HashMap<String, WorkspaceEntry> = HashMap::new();

        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("Query failed: {}", e),
            })?
        {
            let full_path = get_text(&row, 0);
            let updated_at = get_opt_ts(&row, 1);
            let content_preview = get_opt_text(&row, 2);

            let relative = if dir.is_empty() {
                &full_path
            } else if let Some(stripped) = full_path.strip_prefix(&dir) {
                stripped
            } else {
                continue;
            };

            let child_name = if let Some(slash_pos) = relative.find('/') {
                &relative[..slash_pos]
            } else {
                relative
            };

            if child_name.is_empty() {
                continue;
            }

            let is_dir = relative.contains('/');
            let entry_path = if dir.is_empty() {
                child_name.to_string()
            } else {
                format!("{}{}", dir, child_name)
            };

            entries_map
                .entry(child_name.to_string())
                .and_modify(|e| {
                    if is_dir {
                        e.is_directory = true;
                        e.content_preview = None;
                    }
                    if let (Some(existing), Some(new)) = (&e.updated_at, &updated_at)
                        && new > existing
                    {
                        e.updated_at = Some(*new);
                    }
                })
                .or_insert(WorkspaceEntry {
                    path: entry_path,
                    is_directory: is_dir,
                    updated_at,
                    content_preview: if is_dir { None } else { content_preview },
                });
        }

        let mut entries: Vec<WorkspaceEntry> = entries_map.into_values().collect();
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(entries)
    }

    async fn list_all_paths(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
    ) -> Result<Vec<String>, WorkspaceError> {
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: e.to_string(),
            })?;
        let agent_id_str = agent_id.map(|id| id.to_string());
        let mut rows = conn
            .query(
                "SELECT path FROM memory_documents WHERE user_id = ?1 AND agent_id IS ?2 ORDER BY path",
                params![user_id, agent_id_str.as_deref()],
            )
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("List paths failed: {}", e),
            })?;

        let mut paths = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("Query failed: {}", e),
            })?
        {
            paths.push(get_text(&row, 0));
        }
        Ok(paths)
    }

    async fn list_documents(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
    ) -> Result<Vec<MemoryDocument>, WorkspaceError> {
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: e.to_string(),
            })?;
        let agent_id_str = agent_id.map(|id| id.to_string());
        let mut rows = conn
            .query(
                r#"
                SELECT id, user_id, agent_id, path, content,
                       created_at, updated_at, metadata
                FROM memory_documents
                WHERE user_id = ?1 AND agent_id IS ?2
                ORDER BY updated_at DESC
                "#,
                params![user_id, agent_id_str.as_deref()],
            )
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("Query failed: {}", e),
            })?;

        let mut docs = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("Query failed: {}", e),
            })?
        {
            docs.push(row_to_memory_document(&row));
        }
        Ok(docs)
    }

    async fn delete_chunks(&self, document_id: Uuid) -> Result<(), WorkspaceError> {
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::ChunkingFailed {
                reason: e.to_string(),
            })?;
        conn.execute(
            "DELETE FROM memory_chunks WHERE document_id = ?1",
            params![document_id.to_string()],
        )
        .await
        .map_err(|e| WorkspaceError::ChunkingFailed {
            reason: format!("Delete failed: {}", e),
        })?;
        Ok(())
    }

    async fn insert_chunk(
        &self,
        document_id: Uuid,
        chunk_index: i32,
        content: &str,
        embedding: Option<&[f32]>,
    ) -> Result<Uuid, WorkspaceError> {
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::ChunkingFailed {
                reason: e.to_string(),
            })?;
        let id = Uuid::new_v4();
        let embedding_blob = embedding.map(|e| {
            let bytes: Vec<u8> = e.iter().flat_map(|f| f.to_le_bytes()).collect();
            bytes
        });

        conn.execute(
            r#"
                INSERT INTO memory_chunks (id, document_id, chunk_index, content, embedding)
                VALUES (?1, ?2, ?3, ?4, ?5)
                "#,
            params![
                id.to_string(),
                document_id.to_string(),
                chunk_index as i64,
                content,
                embedding_blob.map(libsql::Value::Blob),
            ],
        )
        .await
        .map_err(|e| WorkspaceError::ChunkingFailed {
            reason: format!("Insert failed: {}", e),
        })?;
        Ok(id)
    }

    async fn update_chunk_embedding(
        &self,
        chunk_id: Uuid,
        embedding: &[f32],
    ) -> Result<(), WorkspaceError> {
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::EmbeddingFailed {
                reason: e.to_string(),
            })?;
        let bytes: Vec<u8> = embedding.iter().flat_map(|f| f.to_le_bytes()).collect();

        conn.execute(
            "UPDATE memory_chunks SET embedding = ?2 WHERE id = ?1",
            params![chunk_id.to_string(), libsql::Value::Blob(bytes)],
        )
        .await
        .map_err(|e| WorkspaceError::EmbeddingFailed {
            reason: format!("Update failed: {}", e),
        })?;
        Ok(())
    }

    async fn get_chunks_without_embeddings(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        limit: usize,
    ) -> Result<Vec<MemoryChunk>, WorkspaceError> {
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: e.to_string(),
            })?;
        let agent_id_str = agent_id.map(|id| id.to_string());
        let mut rows = conn
            .query(
                r#"
                SELECT c.id, c.document_id, c.chunk_index, c.content, c.created_at
                FROM memory_chunks c
                JOIN memory_documents d ON d.id = c.document_id
                WHERE d.user_id = ?1 AND d.agent_id IS ?2
                  AND c.embedding IS NULL
                LIMIT ?3
                "#,
                params![user_id, agent_id_str.as_deref(), limit as i64],
            )
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("Query failed: {}", e),
            })?;

        let mut chunks = Vec::new();
        while let Some(row) = rows
            .next()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: format!("Query failed: {}", e),
            })?
        {
            chunks.push(MemoryChunk {
                id: get_text(&row, 0).parse().unwrap_or_default(),
                document_id: get_text(&row, 1).parse().unwrap_or_default(),
                chunk_index: get_i64(&row, 2) as i32,
                content: get_text(&row, 3),
                embedding: None,
                created_at: get_ts(&row, 4),
            });
        }
        Ok(chunks)
    }

    async fn hybrid_search(
        &self,
        user_id: &str,
        agent_id: Option<Uuid>,
        query: &str,
        embedding: Option<&[f32]>,
        config: &SearchConfig,
    ) -> Result<Vec<SearchResult>, WorkspaceError> {
        let conn = self
            .connect()
            .await
            .map_err(|e| WorkspaceError::SearchFailed {
                reason: e.to_string(),
            })?;
        let agent_id_str = agent_id.map(|id| id.to_string());
        let pre_limit = config.pre_fusion_limit as i64;

        let fts_results = if config.use_fts {
            let mut rows = conn
                .query(
                    r#"
                    SELECT c.id, c.document_id, d.path, c.content
                    FROM memory_chunks_fts fts
                    JOIN memory_chunks c ON c._rowid = fts.rowid
                    JOIN memory_documents d ON d.id = c.document_id
                    WHERE d.user_id = ?1 AND d.agent_id IS ?2
                      AND memory_chunks_fts MATCH ?3
                    ORDER BY rank
                    LIMIT ?4
                    "#,
                    params![user_id, agent_id_str.as_deref(), query, pre_limit],
                )
                .await
                .map_err(|e| WorkspaceError::SearchFailed {
                    reason: format!("FTS query failed: {}", e),
                })?;

            let mut results = Vec::new();
            while let Some(row) = rows
                .next()
                .await
                .map_err(|e| WorkspaceError::SearchFailed {
                    reason: format!("FTS row fetch failed: {}", e),
                })?
            {
                results.push(RankedResult {
                    chunk_id: get_text(&row, 0).parse().unwrap_or_default(),
                    document_id: get_text(&row, 1).parse().unwrap_or_default(),
                    document_path: get_text(&row, 2),
                    content: get_text(&row, 3),
                    rank: results.len() as u32 + 1,
                });
            }
            results
        } else {
            Vec::new()
        };

        let vector_results = if let (true, Some(emb)) = (config.use_vector, embedding) {
            let vector_json = format!(
                "[{}]",
                emb.iter()
                    .map(|f| f.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            );

            // vector_top_k requires a libsql_vector_idx index created by
            // ensure_vector_index(). If the index is missing (embeddings not
            // configured or dimension mismatch), fall back to FTS-only.
            match conn
                .query(
                    r#"
                    SELECT c.id, c.document_id, d.path, c.content
                    FROM vector_top_k('idx_memory_chunks_embedding', vector(?1), ?2) AS top_k
                    JOIN memory_chunks c ON c._rowid = top_k.id
                    JOIN memory_documents d ON d.id = c.document_id
                    WHERE d.user_id = ?3 AND d.agent_id IS ?4
                    "#,
                    params![vector_json, pre_limit, user_id, agent_id_str.as_deref()],
                )
                .await
            {
                Ok(mut rows) => {
                    let mut results = Vec::new();
                    while let Some(row) =
                        rows.next()
                            .await
                            .map_err(|e| WorkspaceError::SearchFailed {
                                reason: format!("Vector row fetch failed: {}", e),
                            })?
                    {
                        results.push(RankedResult {
                            chunk_id: get_text(&row, 0).parse().unwrap_or_default(),
                            document_id: get_text(&row, 1).parse().unwrap_or_default(),
                            document_path: get_text(&row, 2),
                            content: get_text(&row, 3),
                            rank: results.len() as u32 + 1,
                        });
                    }
                    results
                }
                Err(e) => {
                    tracing::warn!(
                        "Vector index query failed (ensure_vector_index may not have run \
                         or dimension mismatch), falling back to FTS-only: {e}"
                    );
                    Vec::new()
                }
            }
        } else {
            Vec::new()
        };

        if embedding.is_some() && !config.use_vector {
            tracing::warn!(
                "Embedding provided but vector search is disabled in config; using FTS-only results"
            );
        }

        Ok(fuse_results(fts_results, vector_results, config))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    /// Helper: create a file-backed backend with migrations applied.
    async fn setup_backend() -> (LibSqlBackend, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("tempdir"); // safety: test-only
        let db_path = dir.path().join("test_vector.db");
        let backend = LibSqlBackend::new_local(&db_path).await.expect("new_local"); // safety: test-only
        backend.run_migrations().await.expect("migrations"); // safety: test-only
        (backend, dir)
    }

    /// Helper: insert a document and chunk with an optional embedding.
    async fn insert_test_chunk(
        backend: &LibSqlBackend,
        user_id: &str,
        path: &str,
        content: &str,
        embedding: Option<&[f32]>,
    ) -> (Uuid, Uuid) {
        let conn = backend.connect().await.expect("connect"); // safety: test-only
        let doc_id = Uuid::new_v4();
        let now = super::fmt_ts(&Utc::now());
        conn.execute(
            "INSERT INTO memory_documents (id, user_id, path, content, created_at, updated_at, metadata)
             VALUES (?1, ?2, ?3, '', ?4, ?4, '{}')",
            params![doc_id.to_string(), user_id, path, now],
        )
        .await
        .expect("insert doc"); // safety: test-only

        let chunk_id = backend
            .insert_chunk(doc_id, 0, content, embedding)
            .await
            .expect("insert chunk"); // safety: test-only

        (doc_id, chunk_id)
    }

    #[tokio::test]
    async fn test_ensure_vector_index_enables_vector_search() {
        let (backend, _dir) = setup_backend().await;

        // Create vector index with dim=4
        backend.ensure_vector_index(4).await.expect("ensure dim=4"); // safety: test-only

        // Insert a chunk with a 4-dim embedding
        let embedding = [1.0_f32, 0.0, 0.0, 0.0];
        let (_doc_id, _chunk_id) = insert_test_chunk(
            &backend,
            "test",
            "notes.md",
            "hello world",
            Some(&embedding),
        )
        .await;

        // Query using vector_top_k — should find the chunk
        let conn = backend.connect().await.expect("connect"); // safety: test-only
        let mut rows = conn
            .query(
                r#"SELECT c.id
                   FROM vector_top_k('idx_memory_chunks_embedding', vector('[1,0,0,0]'), 5) AS top_k
                   JOIN memory_chunks c ON c._rowid = top_k.id"#,
                (),
            )
            .await
            .expect("vector_top_k query"); // safety: test-only

        let row = rows
            .next()
            .await
            .expect("row fetch") // safety: test-only
            .expect("expected a result row"); // safety: test-only
        let id: String = row.get(0).expect("get id"); // safety: test-only
        assert!(!id.is_empty(), "vector search should return the chunk"); // safety: test-only assertion
    }

    #[tokio::test]
    async fn test_ensure_vector_index_dimension_change() {
        let (backend, _dir) = setup_backend().await;

        // Create with dim=4 and insert data
        backend.ensure_vector_index(4).await.expect("ensure dim=4"); // safety: test-only
        let embedding_4d = [1.0_f32, 2.0, 3.0, 4.0];
        insert_test_chunk(&backend, "test", "a.md", "content a", Some(&embedding_4d)).await;

        // Recreate with dim=8 — old 4-dim embeddings should be NULLed
        backend.ensure_vector_index(8).await.expect("ensure dim=8"); // safety: test-only

        // Verify metadata updated
        let conn = backend.connect().await.expect("connect"); // safety: test-only
        let mut rows = conn
            .query("SELECT name FROM _migrations WHERE version = 0", ())
            .await
            .expect("query metadata"); // safety: test-only
        let row = rows.next().await.expect("fetch").expect("metadata row"); // safety: test-only
        let dim_str: String = row.get(0).expect("get name"); // safety: test-only
        assert_eq!(dim_str, "8"); // safety: test-only assertion

        // Verify old embedding was NULLed (wrong byte length for dim=8)
        let mut rows = conn
            .query("SELECT embedding IS NULL FROM memory_chunks LIMIT 1", ())
            .await
            .expect("query embedding"); // safety: test-only
        let row = rows.next().await.expect("fetch").expect("chunk row"); // safety: test-only
        let is_null: i64 = row.get(0).expect("get is_null"); // safety: test-only
        // safety: test-only assertion
        assert_eq!(
            is_null, 1,
            "old 4-dim embedding should be NULLed after dim change to 8"
        );
    }

    #[tokio::test]
    async fn test_ensure_vector_index_noop_when_unchanged() {
        let (backend, _dir) = setup_backend().await;

        // Create with dim=4 and insert data
        backend.ensure_vector_index(4).await.expect("ensure dim=4"); // safety: test-only
        let embedding = [1.0_f32, 0.0, 0.0, 0.0];
        insert_test_chunk(&backend, "test", "b.md", "content b", Some(&embedding)).await;

        // Run again with same dimension — should be a no-op
        backend
            .ensure_vector_index(4)
            .await
            .expect("ensure dim=4 again"); // safety: test-only

        // Verify data is untouched (embedding not NULLed)
        let conn = backend.connect().await.expect("connect"); // safety: test-only
        let mut rows = conn
            .query(
                "SELECT embedding IS NOT NULL FROM memory_chunks LIMIT 1",
                (),
            )
            .await
            .expect("query embedding"); // safety: test-only
        let row = rows.next().await.expect("fetch").expect("chunk row"); // safety: test-only
        let has_embedding: i64 = row.get(0).expect("get"); // safety: test-only
        // safety: test-only assertion
        assert_eq!(
            has_embedding, 1,
            "embedding should be preserved on no-op call"
        );
    }

    #[tokio::test]
    async fn test_hybrid_search_returns_vector_results() {
        let (backend, _dir) = setup_backend().await;

        // Create vector index with dim=4
        backend.ensure_vector_index(4).await.expect("ensure dim=4"); // safety: test-only

        // Insert chunk with embedding and searchable content
        let embedding = [0.5_f32, 0.5, 0.0, 0.0];
        insert_test_chunk(
            &backend,
            "user1",
            "notes.md",
            "quantum computing research",
            Some(&embedding),
        )
        .await;

        // Search via the WorkspaceStore trait with vector enabled
        let query_emb = [0.5_f32, 0.5, 0.0, 0.0];
        let config = SearchConfig::default().with_limit(5);
        let results = backend
            .hybrid_search("user1", None, "quantum", Some(&query_emb), &config)
            .await
            .expect("hybrid_search"); // safety: test-only

        assert!(!results.is_empty(), "hybrid search should return results"); // safety: test-only assertion
        let first = &results[0];
        // safety: test-only assertion
        assert!(
            first.vector_rank.is_some(),
            "result should have a vector_rank"
        );
        assert_eq!(first.content, "quantum computing research"); // safety: test-only assertion
    }
}
