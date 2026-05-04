//! Reborn-native libSQL repository.
//!
//! Persists memory documents in the dedicated `reborn_memory_*` tables with
//! explicit `tenant_id`, `user_id`, `agent_id`, `project_id` scope columns —
//! never the legacy synthetic `memory_documents.user_id` encoding.
//!
//! Every read/list/search/write/version/chunk query filters by the full
//! `(tenant_id, user_id, agent_id, project_id)` tuple. Absent agent/project
//! IDs use the empty-string DB-only sentinel; the constructor rejects empty
//! and `_none` user-supplied IDs, so equality comparison is unambiguous.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_filesystem::{FilesystemError, FilesystemOperation};
use ironclaw_host_api::VirtualPath;

use crate::chunking::{MemoryChunkWrite, content_sha256};
use crate::embedding::{cosine_similarity, decode_embedding_blob, encode_embedding_blob};
use crate::indexer::MemoryDocumentIndexRepository;
use crate::metadata::MemoryWriteOptions;
use crate::path::{MemoryDocumentPath, MemoryDocumentScope, memory_error, valid_memory_path};
use crate::search::{
    MemorySearchRequest, MemorySearchResult, RankedMemorySearchResult, escape_fts5_query,
    fuse_memory_search_results,
};

use super::{
    MemoryDocumentRepository, ensure_document_path_does_not_conflict, reborn_agent_id_db_value,
    reborn_memory_document_from_row, reborn_project_id_db_value,
};

/// Reborn-native libSQL repository for `reborn_memory_*` tables.
pub struct RebornLibSqlMemoryDocumentRepository {
    db: Arc<libsql::Database>,
}

impl RebornLibSqlMemoryDocumentRepository {
    pub fn new(db: Arc<libsql::Database>) -> Self {
        Self { db }
    }

    /// Create the Reborn-native tables, FTS virtual table, triggers, and
    /// indexes if they do not already exist. Idempotent; safe to call on every
    /// startup.
    pub async fn run_migrations(&self) -> Result<(), FilesystemError> {
        let conn = self
            .connect(valid_memory_path(), FilesystemOperation::CreateDirAll)
            .await?;
        conn.execute_batch(REBORN_LIBSQL_MEMORY_DOCUMENTS_SCHEMA)
            .await
            .map_err(|error| {
                memory_error(
                    valid_memory_path(),
                    FilesystemOperation::CreateDirAll,
                    error.to_string(),
                )
            })?;
        Ok(())
    }

    async fn connect(
        &self,
        path: VirtualPath,
        operation: FilesystemOperation,
    ) -> Result<libsql::Connection, FilesystemError> {
        let conn = self
            .db
            .connect()
            .map_err(|error| memory_error(path.clone(), operation, error.to_string()))?;
        conn.query("PRAGMA busy_timeout = 5000", ())
            .await
            .map_err(|error| memory_error(path, operation, error.to_string()))?;
        Ok(conn)
    }
}

#[async_trait]
impl MemoryDocumentRepository for RebornLibSqlMemoryDocumentRepository {
    async fn read_document(
        &self,
        path: &MemoryDocumentPath,
    ) -> Result<Option<Vec<u8>>, FilesystemError> {
        let virtual_path = path.virtual_path().unwrap_or_else(|_| valid_memory_path());
        let conn = self
            .connect(virtual_path.clone(), FilesystemOperation::ReadFile)
            .await?;
        let scope = path.scope();
        let mut rows = conn
            .query(
                "SELECT content FROM reborn_memory_documents \
                 WHERE tenant_id = ?1 AND user_id = ?2 AND agent_id = ?3 \
                   AND project_id = ?4 AND path = ?5",
                libsql::params![
                    scope.tenant_id(),
                    scope.user_id(),
                    reborn_agent_id_db_value(scope),
                    reborn_project_id_db_value(scope),
                    path.relative_path(),
                ],
            )
            .await
            .map_err(|error| {
                memory_error(
                    virtual_path.clone(),
                    FilesystemOperation::ReadFile,
                    error.to_string(),
                )
            })?;
        let Some(row) = rows.next().await.map_err(|error| {
            memory_error(
                virtual_path.clone(),
                FilesystemOperation::ReadFile,
                error.to_string(),
            )
        })?
        else {
            return Ok(None);
        };
        let content: String = row.get(0).map_err(|error| {
            memory_error(
                virtual_path,
                FilesystemOperation::ReadFile,
                error.to_string(),
            )
        })?;
        Ok(Some(content.into_bytes()))
    }

    async fn write_document(
        &self,
        path: &MemoryDocumentPath,
        bytes: &[u8],
    ) -> Result<(), FilesystemError> {
        self.write_document_with_options(path, bytes, &MemoryWriteOptions::default())
            .await
    }

    async fn write_document_with_options(
        &self,
        path: &MemoryDocumentPath,
        bytes: &[u8],
        options: &MemoryWriteOptions,
    ) -> Result<(), FilesystemError> {
        let virtual_path = path.virtual_path().unwrap_or_else(|_| valid_memory_path());
        let content = std::str::from_utf8(bytes).map_err(|_| {
            memory_error(
                virtual_path.clone(),
                FilesystemOperation::WriteFile,
                "memory document content must be UTF-8",
            )
        })?;
        let conn = self
            .connect(virtual_path.clone(), FilesystemOperation::WriteFile)
            .await?;
        let scope = path.scope();
        let agent_id_db = reborn_agent_id_db_value(scope);
        let project_id_db = reborn_project_id_db_value(scope);
        let new_content_hash = content_sha256(content);

        conn.execute("BEGIN IMMEDIATE", libsql::params![])
            .await
            .map_err(|error| {
                memory_error(
                    virtual_path.clone(),
                    FilesystemOperation::WriteFile,
                    error.to_string(),
                )
            })?;

        let result: Result<(), FilesystemError> = async {
            // File/directory prefix-conflict check under the same scope.
            let documents = reborn_libsql_list_paths_for_scope(
                &conn,
                scope,
                &virtual_path,
                FilesystemOperation::WriteFile,
            )
            .await?;
            ensure_document_path_does_not_conflict(
                path,
                &documents,
                FilesystemOperation::WriteFile,
            )?;

            let existing = reborn_libsql_existing_document(
                &conn,
                scope,
                path.relative_path(),
                &virtual_path,
                FilesystemOperation::WriteFile,
            )
            .await?;

            if let Some((document_id, previous_content)) = existing {
                let should_version = options.metadata.skip_versioning != Some(true)
                    && previous_content != content
                    && !previous_content.is_empty();
                if should_version {
                    reborn_libsql_save_document_version(
                        &conn,
                        &virtual_path,
                        &document_id,
                        &previous_content,
                        options.changed_by.as_deref(),
                    )
                    .await?;
                }
                conn.execute(
                    "UPDATE reborn_memory_documents \
                     SET content = ?2, content_hash = ?3, \
                         updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') \
                     WHERE id = ?1",
                    libsql::params![document_id.as_str(), content, new_content_hash.as_str(),],
                )
                .await
                .map_err(|error| {
                    memory_error(
                        virtual_path.clone(),
                        FilesystemOperation::WriteFile,
                        error.to_string(),
                    )
                })?;
            } else {
                conn.execute(
                    "INSERT INTO reborn_memory_documents \
                         (id, tenant_id, user_id, agent_id, project_id, path, \
                          content, content_hash, metadata) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, '{}')",
                    libsql::params![
                        uuid::Uuid::new_v4().to_string(),
                        scope.tenant_id(),
                        scope.user_id(),
                        agent_id_db,
                        project_id_db,
                        path.relative_path(),
                        content,
                        new_content_hash.as_str(),
                    ],
                )
                .await
                .map_err(|error| {
                    memory_error(
                        virtual_path.clone(),
                        FilesystemOperation::WriteFile,
                        error.to_string(),
                    )
                })?;
            }
            Ok(())
        }
        .await;

        if result.is_ok() {
            conn.execute("COMMIT", libsql::params![])
                .await
                .map_err(|error| {
                    memory_error(
                        virtual_path.clone(),
                        FilesystemOperation::WriteFile,
                        error.to_string(),
                    )
                })?;
        } else {
            let _ = conn.execute("ROLLBACK", libsql::params![]).await;
        }
        result
    }

    async fn read_document_metadata(
        &self,
        path: &MemoryDocumentPath,
    ) -> Result<Option<serde_json::Value>, FilesystemError> {
        let virtual_path = path.virtual_path().unwrap_or_else(|_| valid_memory_path());
        let conn = self
            .connect(virtual_path.clone(), FilesystemOperation::ReadFile)
            .await?;
        let scope = path.scope();
        let mut rows = conn
            .query(
                "SELECT metadata FROM reborn_memory_documents \
                 WHERE tenant_id = ?1 AND user_id = ?2 AND agent_id = ?3 \
                   AND project_id = ?4 AND path = ?5",
                libsql::params![
                    scope.tenant_id(),
                    scope.user_id(),
                    reborn_agent_id_db_value(scope),
                    reborn_project_id_db_value(scope),
                    path.relative_path(),
                ],
            )
            .await
            .map_err(|error| {
                memory_error(
                    virtual_path.clone(),
                    FilesystemOperation::ReadFile,
                    error.to_string(),
                )
            })?;
        let Some(row) = rows.next().await.map_err(|error| {
            memory_error(
                virtual_path.clone(),
                FilesystemOperation::ReadFile,
                error.to_string(),
            )
        })?
        else {
            return Ok(None);
        };
        let metadata: String = row.get(0).map_err(|error| {
            memory_error(
                virtual_path.clone(),
                FilesystemOperation::ReadFile,
                error.to_string(),
            )
        })?;
        serde_json::from_str(&metadata).map(Some).map_err(|error| {
            memory_error(
                virtual_path,
                FilesystemOperation::ReadFile,
                error.to_string(),
            )
        })
    }

    async fn write_document_metadata(
        &self,
        path: &MemoryDocumentPath,
        metadata: &serde_json::Value,
    ) -> Result<(), FilesystemError> {
        let virtual_path = path.virtual_path().unwrap_or_else(|_| valid_memory_path());
        let conn = self
            .connect(virtual_path.clone(), FilesystemOperation::WriteFile)
            .await?;
        let scope = path.scope();
        let metadata = serde_json::to_string(metadata).map_err(|error| {
            memory_error(
                virtual_path.clone(),
                FilesystemOperation::WriteFile,
                error.to_string(),
            )
        })?;
        conn.execute(
            "UPDATE reborn_memory_documents \
             SET metadata = ?6, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') \
             WHERE tenant_id = ?1 AND user_id = ?2 AND agent_id = ?3 \
               AND project_id = ?4 AND path = ?5",
            libsql::params![
                scope.tenant_id(),
                scope.user_id(),
                reborn_agent_id_db_value(scope),
                reborn_project_id_db_value(scope),
                path.relative_path(),
                metadata,
            ],
        )
        .await
        .map_err(|error| {
            memory_error(
                virtual_path,
                FilesystemOperation::WriteFile,
                error.to_string(),
            )
        })?;
        Ok(())
    }

    async fn list_documents(
        &self,
        scope: &MemoryDocumentScope,
    ) -> Result<Vec<MemoryDocumentPath>, FilesystemError> {
        let virtual_path = scope
            .virtual_prefix()
            .unwrap_or_else(|_| valid_memory_path());
        let conn = self
            .connect(virtual_path.clone(), FilesystemOperation::ListDir)
            .await?;
        reborn_libsql_list_paths_for_scope(
            &conn,
            scope,
            &virtual_path,
            FilesystemOperation::ListDir,
        )
        .await
    }

    async fn search_documents(
        &self,
        scope: &MemoryDocumentScope,
        request: &MemorySearchRequest,
    ) -> Result<Vec<MemorySearchResult>, FilesystemError> {
        let virtual_path = scope
            .virtual_prefix()
            .unwrap_or_else(|_| valid_memory_path());
        let conn = self
            .connect(virtual_path.clone(), FilesystemOperation::ReadFile)
            .await?;
        let full_text_results = if request.full_text() {
            reborn_libsql_full_text_search_ranked(&conn, scope, request, &virtual_path).await?
        } else {
            Vec::new()
        };
        let vector_results = if request.vector() {
            if let Some(embedding) = request.query_embedding() {
                reborn_libsql_vector_search_ranked(&conn, scope, request, embedding, &virtual_path)
                    .await?
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };
        Ok(fuse_memory_search_results(
            full_text_results,
            vector_results,
            request,
        ))
    }
}

#[async_trait]
impl MemoryDocumentIndexRepository for RebornLibSqlMemoryDocumentRepository {
    async fn replace_document_chunks_if_current(
        &self,
        path: &MemoryDocumentPath,
        expected_content_hash: &str,
        chunks: &[MemoryChunkWrite],
    ) -> Result<(), FilesystemError> {
        let virtual_path = path.virtual_path().unwrap_or_else(|_| valid_memory_path());
        let conn = self
            .connect(virtual_path.clone(), FilesystemOperation::WriteFile)
            .await?;
        let tx = conn
            .transaction_with_behavior(libsql::TransactionBehavior::Immediate)
            .await
            .map_err(|error| {
                memory_error(
                    virtual_path.clone(),
                    FilesystemOperation::WriteFile,
                    error.to_string(),
                )
            })?;
        let scope = path.scope();
        let Some((document_id, current_hash)) = reborn_libsql_document_id_and_hash(
            &tx,
            scope,
            path.relative_path(),
            &virtual_path,
            FilesystemOperation::WriteFile,
        )
        .await?
        else {
            return Ok(());
        };
        if current_hash != expected_content_hash {
            // Document was rewritten between the read and the index refresh;
            // the next reindex will pick it up. Do not corrupt the index.
            return Ok(());
        }
        tx.execute(
            "DELETE FROM reborn_memory_chunks WHERE document_id = ?1",
            libsql::params![document_id.as_str()],
        )
        .await
        .map_err(|error| {
            memory_error(
                virtual_path.clone(),
                FilesystemOperation::WriteFile,
                error.to_string(),
            )
        })?;
        for (index, chunk) in chunks.iter().enumerate() {
            let chunk_hash = content_sha256(&chunk.content);
            let embedding_blob = chunk
                .embedding
                .as_ref()
                .map(|embedding| libsql::Value::Blob(encode_embedding_blob(embedding)));
            tx.execute(
                "INSERT INTO reborn_memory_chunks \
                     (id, document_id, chunk_index, content, content_hash, embedding) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                libsql::params![
                    uuid::Uuid::new_v4().to_string(),
                    document_id.as_str(),
                    index as i64,
                    chunk.content.as_str(),
                    chunk_hash.as_str(),
                    embedding_blob,
                ],
            )
            .await
            .map_err(|error| {
                memory_error(
                    virtual_path.clone(),
                    FilesystemOperation::WriteFile,
                    error.to_string(),
                )
            })?;
        }
        tx.commit().await.map_err(|error| {
            memory_error(
                virtual_path,
                FilesystemOperation::WriteFile,
                error.to_string(),
            )
        })?;
        Ok(())
    }
}

async fn reborn_libsql_list_paths_for_scope(
    conn: &libsql::Connection,
    scope: &MemoryDocumentScope,
    virtual_path: &VirtualPath,
    operation: FilesystemOperation,
) -> Result<Vec<MemoryDocumentPath>, FilesystemError> {
    let mut documents = Vec::new();
    let mut rows = conn
        .query(
            "SELECT path FROM reborn_memory_documents \
             WHERE tenant_id = ?1 AND user_id = ?2 AND agent_id = ?3 AND project_id = ?4 \
             ORDER BY path",
            libsql::params![
                scope.tenant_id(),
                scope.user_id(),
                reborn_agent_id_db_value(scope),
                reborn_project_id_db_value(scope),
            ],
        )
        .await
        .map_err(|error| memory_error(virtual_path.clone(), operation, error.to_string()))?;
    while let Some(row) = rows
        .next()
        .await
        .map_err(|error| memory_error(virtual_path.clone(), operation, error.to_string()))?
    {
        let db_path: String = row
            .get(0)
            .map_err(|error| memory_error(virtual_path.clone(), operation, error.to_string()))?;
        if let Some(memory_path) = reborn_memory_document_from_row(
            scope.tenant_id(),
            scope.user_id(),
            reborn_agent_id_db_value(scope),
            reborn_project_id_db_value(scope),
            &db_path,
        ) {
            documents.push(memory_path);
        }
    }
    Ok(documents)
}

async fn reborn_libsql_existing_document(
    conn: &libsql::Connection,
    scope: &MemoryDocumentScope,
    relative_path: &str,
    virtual_path: &VirtualPath,
    operation: FilesystemOperation,
) -> Result<Option<(String, String)>, FilesystemError> {
    let mut rows = conn
        .query(
            "SELECT id, content FROM reborn_memory_documents \
             WHERE tenant_id = ?1 AND user_id = ?2 AND agent_id = ?3 \
               AND project_id = ?4 AND path = ?5",
            libsql::params![
                scope.tenant_id(),
                scope.user_id(),
                reborn_agent_id_db_value(scope),
                reborn_project_id_db_value(scope),
                relative_path,
            ],
        )
        .await
        .map_err(|error| memory_error(virtual_path.clone(), operation, error.to_string()))?;
    rows.next()
        .await
        .map_err(|error| memory_error(virtual_path.clone(), operation, error.to_string()))?
        .map(|row| {
            let id: String = row.get(0)?;
            let content: String = row.get(1)?;
            Ok::<_, libsql::Error>((id, content))
        })
        .transpose()
        .map_err(|error| memory_error(virtual_path.clone(), operation, error.to_string()))
}

async fn reborn_libsql_document_id_and_hash(
    tx: &libsql::Transaction,
    scope: &MemoryDocumentScope,
    relative_path: &str,
    virtual_path: &VirtualPath,
    operation: FilesystemOperation,
) -> Result<Option<(String, String)>, FilesystemError> {
    let mut rows = tx
        .query(
            "SELECT id, content_hash FROM reborn_memory_documents \
             WHERE tenant_id = ?1 AND user_id = ?2 AND agent_id = ?3 \
               AND project_id = ?4 AND path = ?5",
            libsql::params![
                scope.tenant_id(),
                scope.user_id(),
                reborn_agent_id_db_value(scope),
                reborn_project_id_db_value(scope),
                relative_path,
            ],
        )
        .await
        .map_err(|error| memory_error(virtual_path.clone(), operation, error.to_string()))?;
    rows.next()
        .await
        .map_err(|error| memory_error(virtual_path.clone(), operation, error.to_string()))?
        .map(|row| {
            let id: String = row.get(0)?;
            let hash: String = row.get(1)?;
            Ok::<_, libsql::Error>((id, hash))
        })
        .transpose()
        .map_err(|error| memory_error(virtual_path.clone(), operation, error.to_string()))
}

// Caller must hold an active transaction on `conn` (e.g. via `BEGIN IMMEDIATE`).
async fn reborn_libsql_save_document_version(
    conn: &libsql::Connection,
    virtual_path: &VirtualPath,
    document_id: &str,
    content: &str,
    changed_by: Option<&str>,
) -> Result<i64, FilesystemError> {
    let next_version = {
        let mut rows = conn
            .query(
                "SELECT COALESCE(MAX(version), 0) + 1 \
                 FROM reborn_memory_document_versions WHERE document_id = ?1",
                libsql::params![document_id],
            )
            .await
            .map_err(|error| {
                memory_error(
                    virtual_path.clone(),
                    FilesystemOperation::WriteFile,
                    error.to_string(),
                )
            })?;
        let row = rows
            .next()
            .await
            .map_err(|error| {
                memory_error(
                    virtual_path.clone(),
                    FilesystemOperation::WriteFile,
                    error.to_string(),
                )
            })?
            .ok_or_else(|| {
                memory_error(
                    virtual_path.clone(),
                    FilesystemOperation::WriteFile,
                    "missing version row",
                )
            })?;
        row.get::<i64>(0).map_err(|error| {
            memory_error(
                virtual_path.clone(),
                FilesystemOperation::WriteFile,
                error.to_string(),
            )
        })?
    };
    conn.execute(
        "INSERT INTO reborn_memory_document_versions \
             (id, document_id, version, content, content_hash, changed_by) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        libsql::params![
            uuid::Uuid::new_v4().to_string(),
            document_id,
            next_version,
            content,
            content_sha256(content),
            changed_by,
        ],
    )
    .await
    .map_err(|error| {
        memory_error(
            virtual_path.clone(),
            FilesystemOperation::WriteFile,
            error.to_string(),
        )
    })?;
    Ok(next_version)
}

async fn reborn_libsql_full_text_search_ranked(
    conn: &libsql::Connection,
    scope: &MemoryDocumentScope,
    request: &MemorySearchRequest,
    virtual_path: &VirtualPath,
) -> Result<Vec<RankedMemorySearchResult>, FilesystemError> {
    let Some(fts_query) = escape_fts5_query(request.query()) else {
        return Ok(Vec::new());
    };
    let mut rows = conn
        .query(
            "SELECT c.id, d.tenant_id, d.user_id, d.agent_id, d.project_id, d.path, c.content \
             FROM reborn_memory_chunks_fts fts \
             JOIN reborn_memory_chunks c ON c._rowid = fts.rowid \
             JOIN reborn_memory_documents d ON d.id = c.document_id \
             WHERE d.tenant_id = ?1 AND d.user_id = ?2 AND d.agent_id = ?3 \
               AND d.project_id = ?4 AND reborn_memory_chunks_fts MATCH ?5 \
             ORDER BY rank \
             LIMIT ?6",
            libsql::params![
                scope.tenant_id(),
                scope.user_id(),
                reborn_agent_id_db_value(scope),
                reborn_project_id_db_value(scope),
                fts_query,
                request.pre_fusion_limit() as i64,
            ],
        )
        .await
        .map_err(|error| {
            memory_error(
                virtual_path.clone(),
                FilesystemOperation::ReadFile,
                error.to_string(),
            )
        })?;

    let mut results = Vec::new();
    while let Some(row) = rows.next().await.map_err(|error| {
        memory_error(
            virtual_path.clone(),
            FilesystemOperation::ReadFile,
            error.to_string(),
        )
    })? {
        let chunk_key: String = row.get(0).map_err(|error| {
            memory_error(
                virtual_path.clone(),
                FilesystemOperation::ReadFile,
                error.to_string(),
            )
        })?;
        let tenant_id: String = row.get(1).map_err(|error| {
            memory_error(
                virtual_path.clone(),
                FilesystemOperation::ReadFile,
                error.to_string(),
            )
        })?;
        let user_id: String = row.get(2).map_err(|error| {
            memory_error(
                virtual_path.clone(),
                FilesystemOperation::ReadFile,
                error.to_string(),
            )
        })?;
        let agent_id_db: String = row.get(3).map_err(|error| {
            memory_error(
                virtual_path.clone(),
                FilesystemOperation::ReadFile,
                error.to_string(),
            )
        })?;
        let project_id_db: String = row.get(4).map_err(|error| {
            memory_error(
                virtual_path.clone(),
                FilesystemOperation::ReadFile,
                error.to_string(),
            )
        })?;
        let db_path: String = row.get(5).map_err(|error| {
            memory_error(
                virtual_path.clone(),
                FilesystemOperation::ReadFile,
                error.to_string(),
            )
        })?;
        let snippet: String = row.get(6).map_err(|error| {
            memory_error(
                virtual_path.clone(),
                FilesystemOperation::ReadFile,
                error.to_string(),
            )
        })?;
        let Some(path) = reborn_memory_document_from_row(
            &tenant_id,
            &user_id,
            &agent_id_db,
            &project_id_db,
            &db_path,
        ) else {
            continue;
        };
        let rank = results.len() as u32 + 1;
        results.push(RankedMemorySearchResult {
            chunk_key,
            path,
            snippet,
            rank,
        });
    }
    Ok(results)
}

async fn reborn_libsql_vector_search_ranked(
    conn: &libsql::Connection,
    scope: &MemoryDocumentScope,
    request: &MemorySearchRequest,
    query_embedding: &[f32],
    virtual_path: &VirtualPath,
) -> Result<Vec<RankedMemorySearchResult>, FilesystemError> {
    let mut rows = conn
        .query(
            "SELECT c.id, d.tenant_id, d.user_id, d.agent_id, d.project_id, d.path, \
                    c.content, c.embedding \
             FROM reborn_memory_chunks c \
             JOIN reborn_memory_documents d ON d.id = c.document_id \
             WHERE d.tenant_id = ?1 AND d.user_id = ?2 AND d.agent_id = ?3 \
               AND d.project_id = ?4 AND c.embedding IS NOT NULL",
            libsql::params![
                scope.tenant_id(),
                scope.user_id(),
                reborn_agent_id_db_value(scope),
                reborn_project_id_db_value(scope),
            ],
        )
        .await
        .map_err(|error| {
            memory_error(
                virtual_path.clone(),
                FilesystemOperation::ReadFile,
                error.to_string(),
            )
        })?;

    let mut scored = Vec::<(f32, RankedMemorySearchResult)>::new();
    while let Some(row) = rows.next().await.map_err(|error| {
        memory_error(
            virtual_path.clone(),
            FilesystemOperation::ReadFile,
            error.to_string(),
        )
    })? {
        let chunk_key: String = row.get(0).map_err(|error| {
            memory_error(
                virtual_path.clone(),
                FilesystemOperation::ReadFile,
                error.to_string(),
            )
        })?;
        let tenant_id: String = row.get(1).map_err(|error| {
            memory_error(
                virtual_path.clone(),
                FilesystemOperation::ReadFile,
                error.to_string(),
            )
        })?;
        let user_id: String = row.get(2).map_err(|error| {
            memory_error(
                virtual_path.clone(),
                FilesystemOperation::ReadFile,
                error.to_string(),
            )
        })?;
        let agent_id_db: String = row.get(3).map_err(|error| {
            memory_error(
                virtual_path.clone(),
                FilesystemOperation::ReadFile,
                error.to_string(),
            )
        })?;
        let project_id_db: String = row.get(4).map_err(|error| {
            memory_error(
                virtual_path.clone(),
                FilesystemOperation::ReadFile,
                error.to_string(),
            )
        })?;
        let db_path: String = row.get(5).map_err(|error| {
            memory_error(
                virtual_path.clone(),
                FilesystemOperation::ReadFile,
                error.to_string(),
            )
        })?;
        let snippet: String = row.get(6).map_err(|error| {
            memory_error(
                virtual_path.clone(),
                FilesystemOperation::ReadFile,
                error.to_string(),
            )
        })?;
        let embedding_blob: Vec<u8> = row.get(7).map_err(|error| {
            memory_error(
                virtual_path.clone(),
                FilesystemOperation::ReadFile,
                error.to_string(),
            )
        })?;
        let Some(embedding) = decode_embedding_blob(&embedding_blob) else {
            continue;
        };
        let Some(score) = cosine_similarity(query_embedding, &embedding) else {
            continue;
        };
        let Some(path) = reborn_memory_document_from_row(
            &tenant_id,
            &user_id,
            &agent_id_db,
            &project_id_db,
            &db_path,
        ) else {
            continue;
        };
        scored.push((
            score,
            RankedMemorySearchResult {
                chunk_key,
                path,
                snippet,
                rank: 0,
            },
        ));
    }
    scored.sort_by(|left, right| {
        right
            .0
            .partial_cmp(&left.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                left.1
                    .path
                    .relative_path()
                    .cmp(right.1.path.relative_path())
            })
    });
    scored.truncate(request.pre_fusion_limit());
    Ok(scored
        .into_iter()
        .enumerate()
        .map(|(index, (_score, mut result))| {
            result.rank = index as u32 + 1;
            result
        })
        .collect())
}

const REBORN_LIBSQL_MEMORY_DOCUMENTS_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS reborn_memory_documents (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    agent_id TEXT NOT NULL DEFAULT '',
    project_id TEXT NOT NULL DEFAULT '',
    path TEXT NOT NULL,
    content TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    metadata TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (tenant_id, user_id, agent_id, project_id, path)
);

CREATE INDEX IF NOT EXISTS idx_reborn_memory_documents_scope
    ON reborn_memory_documents(tenant_id, user_id, agent_id, project_id);
CREATE INDEX IF NOT EXISTS idx_reborn_memory_documents_scope_path
    ON reborn_memory_documents(tenant_id, user_id, agent_id, project_id, path);
CREATE INDEX IF NOT EXISTS idx_reborn_memory_documents_updated
    ON reborn_memory_documents(updated_at DESC);

CREATE TRIGGER IF NOT EXISTS update_reborn_memory_documents_updated_at
    AFTER UPDATE ON reborn_memory_documents
    FOR EACH ROW
    WHEN NEW.updated_at = OLD.updated_at
    BEGIN
        UPDATE reborn_memory_documents
        SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
        WHERE id = NEW.id;
    END;

CREATE TABLE IF NOT EXISTS reborn_memory_chunks (
    _rowid INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    document_id TEXT NOT NULL REFERENCES reborn_memory_documents(id) ON DELETE CASCADE,
    chunk_index INTEGER NOT NULL,
    content TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    embedding BLOB,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (document_id, chunk_index)
);

CREATE INDEX IF NOT EXISTS idx_reborn_memory_chunks_document
    ON reborn_memory_chunks(document_id);

CREATE VIRTUAL TABLE IF NOT EXISTS reborn_memory_chunks_fts USING fts5(
    content,
    content='reborn_memory_chunks',
    content_rowid='_rowid'
);

CREATE TRIGGER IF NOT EXISTS reborn_memory_chunks_fts_insert
    AFTER INSERT ON reborn_memory_chunks BEGIN
        INSERT INTO reborn_memory_chunks_fts(rowid, content)
        VALUES (new._rowid, new.content);
    END;

CREATE TRIGGER IF NOT EXISTS reborn_memory_chunks_fts_delete
    AFTER DELETE ON reborn_memory_chunks BEGIN
        INSERT INTO reborn_memory_chunks_fts(reborn_memory_chunks_fts, rowid, content)
        VALUES ('delete', old._rowid, old.content);
    END;

CREATE TRIGGER IF NOT EXISTS reborn_memory_chunks_fts_update
    AFTER UPDATE ON reborn_memory_chunks BEGIN
        INSERT INTO reborn_memory_chunks_fts(reborn_memory_chunks_fts, rowid, content)
        VALUES ('delete', old._rowid, old.content);
        INSERT INTO reborn_memory_chunks_fts(rowid, content)
        VALUES (new._rowid, new.content);
    END;

CREATE TABLE IF NOT EXISTS reborn_memory_document_versions (
    id TEXT PRIMARY KEY,
    document_id TEXT NOT NULL REFERENCES reborn_memory_documents(id) ON DELETE CASCADE,
    version INTEGER NOT NULL,
    content TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    changed_by TEXT,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (document_id, version)
);

CREATE INDEX IF NOT EXISTS idx_reborn_memory_document_versions_lookup
    ON reborn_memory_document_versions(document_id, version DESC);
"#;
