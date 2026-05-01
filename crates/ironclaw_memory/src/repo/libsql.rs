//! libSQL repository adapter for the existing `memory_documents` table shape.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_filesystem::{FilesystemError, FilesystemOperation};
use ironclaw_host_api::VirtualPath;

use crate::chunking::{MemoryChunkWrite, content_bytes_sha256, content_sha256};
use crate::embedding::{cosine_similarity, decode_embedding_blob, encode_embedding_blob};
use crate::indexer::MemoryDocumentIndexRepository;
use crate::metadata::MemoryWriteOptions;
use crate::path::{MemoryDocumentPath, MemoryDocumentScope, memory_error, valid_memory_path};
use crate::search::{
    MemorySearchRequest, MemorySearchResult, RankedMemorySearchResult, escape_fts5_query,
    fuse_memory_search_results,
};

use super::{
    MemoryAppendOutcome, MemoryDocumentRepository, db_path_for_memory_document,
    ensure_document_path_does_not_conflict, memory_document_from_db_path, scoped_memory_agent_id,
    scoped_memory_owner_key,
};

/// libSQL repository adapter for the existing `memory_documents` table shape.
pub struct LibSqlMemoryDocumentRepository {
    db: Arc<libsql::Database>,
}

impl LibSqlMemoryDocumentRepository {
    pub fn new(db: Arc<libsql::Database>) -> Self {
        Self { db }
    }

    pub async fn run_migrations(&self) -> Result<(), FilesystemError> {
        let conn = self
            .connect(valid_memory_path(), FilesystemOperation::CreateDirAll)
            .await?;
        conn.execute_batch(LIBSQL_MEMORY_DOCUMENTS_SCHEMA)
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

async fn libsql_list_documents_for_scope(
    conn: &libsql::Connection,
    scope: &MemoryDocumentScope,
    virtual_path: &VirtualPath,
    operation: FilesystemOperation,
) -> Result<Vec<MemoryDocumentPath>, FilesystemError> {
    let owner_key = scoped_memory_owner_key(scope);
    let agent_id = scoped_memory_agent_id(scope);
    let mut documents = Vec::new();
    let mut rows = conn
        .query(
            "SELECT path FROM memory_documents WHERE user_id = ?1 AND ((?2 IS NULL AND agent_id IS NULL) OR agent_id = ?2) ORDER BY path",
            libsql::params![owner_key, agent_id],
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
        if let Some(memory_path) = memory_document_from_db_path(scope, &db_path) {
            documents.push(memory_path);
        }
    }
    Ok(documents)
}

#[async_trait]
impl MemoryDocumentRepository for LibSqlMemoryDocumentRepository {
    async fn read_document(
        &self,
        path: &MemoryDocumentPath,
    ) -> Result<Option<Vec<u8>>, FilesystemError> {
        let virtual_path = path.virtual_path().unwrap_or_else(|_| valid_memory_path());
        let conn = self
            .connect(virtual_path.clone(), FilesystemOperation::ReadFile)
            .await?;
        let owner_key = scoped_memory_owner_key(path.scope());
        let agent_id = scoped_memory_agent_id(path.scope());
        let db_path = db_path_for_memory_document(path);
        let mut rows = conn
            .query(
                "SELECT content FROM memory_documents WHERE user_id = ?1 AND ((?2 IS NULL AND agent_id IS NULL) OR agent_id = ?2) AND path = ?3",
                libsql::params![owner_key, agent_id, db_path],
            )
            .await
            .map_err(|error| memory_error(virtual_path.clone(), FilesystemOperation::ReadFile, error.to_string()))?;
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
        let content = std::str::from_utf8(bytes).map_err(|_| {
            memory_error(
                path.virtual_path().unwrap_or_else(|_| valid_memory_path()),
                FilesystemOperation::WriteFile,
                "memory document content must be UTF-8",
            )
        })?;
        let virtual_path = path.virtual_path().unwrap_or_else(|_| valid_memory_path());
        let conn = self
            .connect(virtual_path.clone(), FilesystemOperation::WriteFile)
            .await?;
        let owner_key = scoped_memory_owner_key(path.scope());
        let agent_id = scoped_memory_agent_id(path.scope());
        let db_path = db_path_for_memory_document(path);

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
            let documents = libsql_list_documents_for_scope(
                &conn,
                path.scope(),
                &virtual_path,
                FilesystemOperation::WriteFile,
            )
            .await?;
            ensure_document_path_does_not_conflict(
                path,
                &documents,
                FilesystemOperation::WriteFile,
            )?;

            let existing = {
                let mut rows = conn
                    .query(
                        "SELECT id, content FROM memory_documents WHERE user_id = ?1 AND ((?2 IS NULL AND agent_id IS NULL) OR agent_id = ?2) AND path = ?3",
                        libsql::params![owner_key.as_str(), agent_id, db_path.as_str()],
                    )
                    .await
                    .map_err(|error| memory_error(virtual_path.clone(), FilesystemOperation::WriteFile, error.to_string()))?;
                rows.next()
                    .await
                    .map_err(|error| {
                        memory_error(virtual_path.clone(), FilesystemOperation::WriteFile, error.to_string())
                    })?
                    .map(|row| {
                        let id: String = row.get(0)?;
                        let previous_content: String = row.get(1)?;
                        Ok::<_, libsql::Error>((id, previous_content))
                    })
                    .transpose()
                    .map_err(|error| {
                        memory_error(virtual_path.clone(), FilesystemOperation::WriteFile, error.to_string())
                    })?
            };

            if let Some((document_id, previous_content)) = existing {
                if previous_content != content && !previous_content.is_empty() {
                    libsql_save_document_version(
                        &conn,
                        &virtual_path,
                        &document_id,
                        &previous_content,
                        Some(owner_key.as_str()),
                    )
                    .await?;
                }
                conn.execute(
                    "UPDATE memory_documents SET content = ?2, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?1",
                    libsql::params![document_id, content],
                )
                .await
                .map_err(|error| memory_error(virtual_path.clone(), FilesystemOperation::WriteFile, error.to_string()))?;
            } else {
                conn.execute(
                    r#"
                INSERT INTO memory_documents (id, user_id, agent_id, path, content, metadata)
                VALUES (?1, ?2, ?3, ?4, ?5, '{}')
                "#,
                    libsql::params![
                        uuid::Uuid::new_v4().to_string(),
                        owner_key.as_str(),
                        agent_id,
                        db_path.as_str(),
                        content,
                    ],
                )
                .await
                .map_err(|error| {
                    memory_error(virtual_path.clone(), FilesystemOperation::WriteFile, error.to_string())
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

    async fn write_document_with_options(
        &self,
        path: &MemoryDocumentPath,
        bytes: &[u8],
        options: &MemoryWriteOptions,
    ) -> Result<(), FilesystemError> {
        let content = std::str::from_utf8(bytes).map_err(|_| {
            memory_error(
                path.virtual_path().unwrap_or_else(|_| valid_memory_path()),
                FilesystemOperation::WriteFile,
                "memory document content must be UTF-8",
            )
        })?;
        let virtual_path = path.virtual_path().unwrap_or_else(|_| valid_memory_path());
        let conn = self
            .connect(virtual_path.clone(), FilesystemOperation::WriteFile)
            .await?;
        let owner_key = scoped_memory_owner_key(path.scope());
        let agent_id = scoped_memory_agent_id(path.scope());
        let db_path = db_path_for_memory_document(path);

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
            let documents = libsql_list_documents_for_scope(
                &conn,
                path.scope(),
                &virtual_path,
                FilesystemOperation::WriteFile,
            )
            .await?;
            ensure_document_path_does_not_conflict(
                path,
                &documents,
                FilesystemOperation::WriteFile,
            )?;

            let existing = {
                let mut rows = conn
                    .query(
                        "SELECT id, content FROM memory_documents WHERE user_id = ?1 AND ((?2 IS NULL AND agent_id IS NULL) OR agent_id = ?2) AND path = ?3",
                        libsql::params![owner_key.as_str(), agent_id, db_path.as_str()],
                    )
                    .await
                    .map_err(|error| memory_error(virtual_path.clone(), FilesystemOperation::WriteFile, error.to_string()))?;
                rows.next()
                    .await
                    .map_err(|error| {
                        memory_error(virtual_path.clone(), FilesystemOperation::WriteFile, error.to_string())
                    })?
                    .map(|row| {
                        let id: String = row.get(0)?;
                        let previous_content: String = row.get(1)?;
                        Ok::<_, libsql::Error>((id, previous_content))
                    })
                    .transpose()
                    .map_err(|error| {
                        memory_error(virtual_path.clone(), FilesystemOperation::WriteFile, error.to_string())
                    })?
            };

            if let Some((document_id, previous_content)) = existing {
                if options.metadata.skip_versioning != Some(true)
                    && previous_content != content
                    && !previous_content.is_empty()
                {
                    libsql_save_document_version(
                        &conn,
                        &virtual_path,
                        &document_id,
                        &previous_content,
                        options.changed_by.as_deref(),
                    )
                    .await?;
                }
                conn.execute(
                    "UPDATE memory_documents SET content = ?2, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?1",
                    libsql::params![document_id, content],
                )
                .await
                .map_err(|error| memory_error(virtual_path.clone(), FilesystemOperation::WriteFile, error.to_string()))?;
            } else {
                conn.execute(
                    r#"
                INSERT INTO memory_documents (id, user_id, agent_id, path, content, metadata)
                VALUES (?1, ?2, ?3, ?4, ?5, '{}')
                "#,
                    libsql::params![
                        uuid::Uuid::new_v4().to_string(),
                        owner_key.as_str(),
                        agent_id,
                        db_path.as_str(),
                        content,
                    ],
                )
                .await
                .map_err(|error| {
                    memory_error(virtual_path.clone(), FilesystemOperation::WriteFile, error.to_string())
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

    async fn compare_and_append_document_with_options(
        &self,
        path: &MemoryDocumentPath,
        expected_previous_hash: Option<&str>,
        bytes: &[u8],
        options: &MemoryWriteOptions,
    ) -> Result<MemoryAppendOutcome, FilesystemError> {
        let append_content = std::str::from_utf8(bytes).map_err(|_| {
            memory_error(
                path.virtual_path().unwrap_or_else(|_| valid_memory_path()),
                FilesystemOperation::AppendFile,
                "memory document content must be UTF-8",
            )
        })?;
        let virtual_path = path.virtual_path().unwrap_or_else(|_| valid_memory_path());
        let conn = self
            .connect(virtual_path.clone(), FilesystemOperation::AppendFile)
            .await?;
        let owner_key = scoped_memory_owner_key(path.scope());
        let agent_id = scoped_memory_agent_id(path.scope());
        let db_path = db_path_for_memory_document(path);

        conn.execute("BEGIN IMMEDIATE", libsql::params![])
            .await
            .map_err(|error| {
                memory_error(
                    virtual_path.clone(),
                    FilesystemOperation::AppendFile,
                    error.to_string(),
                )
            })?;

        let result: Result<MemoryAppendOutcome, FilesystemError> = async {
            let existing = {
                let mut rows = conn
                    .query(
                        "SELECT id, content FROM memory_documents WHERE user_id = ?1 AND ((?2 IS NULL AND agent_id IS NULL) OR agent_id = ?2) AND path = ?3",
                        libsql::params![owner_key.as_str(), agent_id, db_path.as_str()],
                    )
                    .await
                    .map_err(|error| memory_error(virtual_path.clone(), FilesystemOperation::AppendFile, error.to_string()))?;
                rows.next()
                    .await
                    .map_err(|error| {
                        memory_error(virtual_path.clone(), FilesystemOperation::AppendFile, error.to_string())
                    })?
                    .map(|row| {
                        let id: String = row.get(0)?;
                        let previous_content: String = row.get(1)?;
                        Ok::<_, libsql::Error>((id, previous_content))
                    })
                    .transpose()
                    .map_err(|error| {
                        memory_error(virtual_path.clone(), FilesystemOperation::AppendFile, error.to_string())
                    })?
            };
            let current_hash = existing
                .as_ref()
                .map(|(_, content)| content_bytes_sha256(content.as_bytes()));
            if current_hash.as_deref() != expected_previous_hash {
                return Ok(MemoryAppendOutcome::Conflict);
            }

            if let Some((document_id, previous_content)) = existing {
                let content = format!("{previous_content}{append_content}");
                if options.metadata.skip_versioning != Some(true)
                    && previous_content != content
                    && !previous_content.is_empty()
                {
                    libsql_save_document_version(
                        &conn,
                        &virtual_path,
                        &document_id,
                        &previous_content,
                        options.changed_by.as_deref(),
                    )
                    .await?;
                }
                conn.execute(
                    "UPDATE memory_documents SET content = ?2, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?1",
                    libsql::params![document_id, content],
                )
                .await
                .map_err(|error| memory_error(virtual_path.clone(), FilesystemOperation::AppendFile, error.to_string()))?;
            } else {
                let documents = libsql_list_documents_for_scope(
                    &conn,
                    path.scope(),
                    &virtual_path,
                    FilesystemOperation::AppendFile,
                )
                .await?;
                ensure_document_path_does_not_conflict(
                    path,
                    &documents,
                    FilesystemOperation::AppendFile,
                )?;
                conn.execute(
                    r#"
                INSERT INTO memory_documents (id, user_id, agent_id, path, content, metadata)
                VALUES (?1, ?2, ?3, ?4, ?5, '{}')
                "#,
                    libsql::params![
                        uuid::Uuid::new_v4().to_string(),
                        owner_key.as_str(),
                        agent_id,
                        db_path.as_str(),
                        append_content,
                    ],
                )
                .await
                .map_err(|error| {
                    memory_error(virtual_path.clone(), FilesystemOperation::AppendFile, error.to_string())
                })?;
            }
            Ok(MemoryAppendOutcome::Appended)
        }
        .await;

        if result.is_ok() {
            conn.execute("COMMIT", libsql::params![])
                .await
                .map_err(|error| {
                    memory_error(
                        virtual_path.clone(),
                        FilesystemOperation::AppendFile,
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
        let owner_key = scoped_memory_owner_key(path.scope());
        let agent_id = scoped_memory_agent_id(path.scope());
        let db_path = db_path_for_memory_document(path);
        let mut rows = conn
            .query(
                "SELECT metadata FROM memory_documents WHERE user_id = ?1 AND ((?2 IS NULL AND agent_id IS NULL) OR agent_id = ?2) AND path = ?3",
                libsql::params![owner_key, agent_id, db_path],
            )
            .await
            .map_err(|error| memory_error(virtual_path.clone(), FilesystemOperation::ReadFile, error.to_string()))?;
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
        let owner_key = scoped_memory_owner_key(path.scope());
        let agent_id = scoped_memory_agent_id(path.scope());
        let db_path = db_path_for_memory_document(path);
        let metadata = serde_json::to_string(metadata).map_err(|error| {
            memory_error(
                virtual_path.clone(),
                FilesystemOperation::WriteFile,
                error.to_string(),
            )
        })?;
        conn.execute(
            "UPDATE memory_documents SET metadata = ?4, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE user_id = ?1 AND ((?2 IS NULL AND agent_id IS NULL) OR agent_id = ?2) AND path = ?3",
            libsql::params![owner_key, agent_id, db_path, metadata],
        )
        .await
        .map_err(|error| memory_error(virtual_path, FilesystemOperation::WriteFile, error.to_string()))?;
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
        libsql_list_documents_for_scope(&conn, scope, &virtual_path, FilesystemOperation::ListDir)
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
            libsql_full_text_search_ranked(&conn, scope, request, &virtual_path).await?
        } else {
            Vec::new()
        };
        let vector_results = if request.vector() {
            if let Some(embedding) = request.query_embedding() {
                libsql_vector_search_ranked(&conn, scope, request, embedding, &virtual_path).await?
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

async fn libsql_full_text_search_ranked(
    conn: &libsql::Connection,
    scope: &MemoryDocumentScope,
    request: &MemorySearchRequest,
    virtual_path: &VirtualPath,
) -> Result<Vec<RankedMemorySearchResult>, FilesystemError> {
    let Some(fts_query) = escape_fts5_query(request.query()) else {
        return Ok(Vec::new());
    };
    let owner_key = scoped_memory_owner_key(scope);
    let agent_id = scoped_memory_agent_id(scope);
    let mut rows = conn
        .query(
            r#"
            SELECT c.id, d.path, c.content
            FROM memory_chunks_fts fts
            JOIN memory_chunks c ON c._rowid = fts.rowid
            JOIN memory_documents d ON d.id = c.document_id
            WHERE d.user_id = ?1 AND ((?2 IS NULL AND d.agent_id IS NULL) OR d.agent_id = ?2)
              AND memory_chunks_fts MATCH ?3
            ORDER BY rank
            LIMIT ?4
            "#,
            libsql::params![
                owner_key,
                agent_id,
                fts_query,
                request.pre_fusion_limit() as i64
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
        let db_path: String = row.get(1).map_err(|error| {
            memory_error(
                virtual_path.clone(),
                FilesystemOperation::ReadFile,
                error.to_string(),
            )
        })?;
        let Some(path) = memory_document_from_db_path(scope, &db_path) else {
            continue;
        };
        let snippet: String = row.get(2).map_err(|error| {
            memory_error(
                virtual_path.clone(),
                FilesystemOperation::ReadFile,
                error.to_string(),
            )
        })?;
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

async fn libsql_vector_search_ranked(
    conn: &libsql::Connection,
    scope: &MemoryDocumentScope,
    request: &MemorySearchRequest,
    query_embedding: &[f32],
    virtual_path: &VirtualPath,
) -> Result<Vec<RankedMemorySearchResult>, FilesystemError> {
    let owner_key = scoped_memory_owner_key(scope);
    let agent_id = scoped_memory_agent_id(scope);
    let mut rows = conn
        .query(
            r#"
            SELECT c.id, d.path, c.content, c.embedding
            FROM memory_chunks c
            JOIN memory_documents d ON d.id = c.document_id
            WHERE d.user_id = ?1 AND ((?2 IS NULL AND d.agent_id IS NULL) OR d.agent_id = ?2)
              AND c.embedding IS NOT NULL
            "#,
            libsql::params![owner_key, agent_id],
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
        let db_path: String = row.get(1).map_err(|error| {
            memory_error(
                virtual_path.clone(),
                FilesystemOperation::ReadFile,
                error.to_string(),
            )
        })?;
        let Some(path) = memory_document_from_db_path(scope, &db_path) else {
            continue;
        };
        let snippet: String = row.get(2).map_err(|error| {
            memory_error(
                virtual_path.clone(),
                FilesystemOperation::ReadFile,
                error.to_string(),
            )
        })?;
        let embedding_blob: Vec<u8> = row.get(3).map_err(|error| {
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

#[async_trait]
impl MemoryDocumentIndexRepository for LibSqlMemoryDocumentRepository {
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
        let owner_key = scoped_memory_owner_key(path.scope());
        let agent_id = scoped_memory_agent_id(path.scope());
        let db_path = db_path_for_memory_document(path);
        let Some((document_id, content)) = ({
            let mut rows = tx
                .query(
                    "SELECT id, content FROM memory_documents WHERE user_id = ?1 AND ((?2 IS NULL AND agent_id IS NULL) OR agent_id = ?2) AND path = ?3",
                    libsql::params![owner_key, agent_id, db_path],
                )
                .await
                .map_err(|error| memory_error(virtual_path.clone(), FilesystemOperation::WriteFile, error.to_string()))?;
            rows.next()
                .await
                .map_err(|error| {
                    memory_error(
                        virtual_path.clone(),
                        FilesystemOperation::WriteFile,
                        error.to_string(),
                    )
                })?
                .map(|row| {
                    let id: String = row.get(0)?;
                    let content: String = row.get(1)?;
                    Ok::<_, libsql::Error>((id, content))
                })
                .transpose()
                .map_err(|error| {
                    memory_error(
                        virtual_path.clone(),
                        FilesystemOperation::WriteFile,
                        error.to_string(),
                    )
                })?
        }) else {
            return Ok(());
        };
        if content_sha256(&content) != expected_content_hash {
            return Ok(());
        }
        tx.execute(
            "DELETE FROM memory_chunks WHERE document_id = ?1",
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
            let embedding_blob = chunk
                .embedding
                .as_ref()
                .map(|embedding| libsql::Value::Blob(encode_embedding_blob(embedding)));
            tx.execute(
                r#"
                INSERT INTO memory_chunks (id, document_id, chunk_index, content, embedding)
                VALUES (?1, ?2, ?3, ?4, ?5)
                "#,
                libsql::params![
                    uuid::Uuid::new_v4().to_string(),
                    document_id.as_str(),
                    index as i64,
                    chunk.content.as_str(),
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

    async fn delete_document_chunks(
        &self,
        path: &MemoryDocumentPath,
    ) -> Result<(), FilesystemError> {
        let virtual_path = path.virtual_path().unwrap_or_else(|_| valid_memory_path());
        let conn = self
            .connect(virtual_path.clone(), FilesystemOperation::WriteFile)
            .await?;
        let Some((document_id, _content)) =
            libsql_document_id_and_content(&conn, path, &virtual_path).await?
        else {
            return Ok(());
        };
        conn.execute(
            "DELETE FROM memory_chunks WHERE document_id = ?1",
            libsql::params![document_id],
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
}

async fn libsql_document_id_and_content(
    conn: &libsql::Connection,
    path: &MemoryDocumentPath,
    virtual_path: &VirtualPath,
) -> Result<Option<(String, String)>, FilesystemError> {
    let owner_key = scoped_memory_owner_key(path.scope());
    let agent_id = scoped_memory_agent_id(path.scope());
    let db_path = db_path_for_memory_document(path);
    let mut rows = conn
        .query(
            "SELECT id, content FROM memory_documents WHERE user_id = ?1 AND ((?2 IS NULL AND agent_id IS NULL) OR agent_id = ?2) AND path = ?3",
            libsql::params![owner_key, agent_id, db_path],
        )
        .await
        .map_err(|error| memory_error(virtual_path.clone(), FilesystemOperation::WriteFile, error.to_string()))?;
    rows.next()
        .await
        .map_err(|error| {
            memory_error(
                virtual_path.clone(),
                FilesystemOperation::WriteFile,
                error.to_string(),
            )
        })?
        .map(|row| {
            let id: String = row.get(0)?;
            let content: String = row.get(1)?;
            Ok::<_, libsql::Error>((id, content))
        })
        .transpose()
        .map_err(|error| {
            memory_error(
                virtual_path.clone(),
                FilesystemOperation::WriteFile,
                error.to_string(),
            )
        })
}

// Caller must hold an active transaction on `conn` (e.g. via `BEGIN IMMEDIATE`).
async fn libsql_save_document_version(
    conn: &libsql::Connection,
    virtual_path: &VirtualPath,
    document_id: &str,
    content: &str,
    changed_by: Option<&str>,
) -> Result<i32, FilesystemError> {
    let next_version = {
        let mut rows = conn
            .query(
                "SELECT COALESCE(MAX(version), 0) + 1 FROM memory_document_versions WHERE document_id = ?1",
                libsql::params![document_id],
            )
            .await
            .map_err(|error| memory_error(virtual_path.clone(), FilesystemOperation::WriteFile, error.to_string()))?;
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
        row.get::<i64>(0)
            .map(|version| version as i32)
            .map_err(|error| {
                memory_error(
                    virtual_path.clone(),
                    FilesystemOperation::WriteFile,
                    error.to_string(),
                )
            })?
    };
    conn.execute(
        r#"
            INSERT INTO memory_document_versions
                (id, document_id, version, content, content_hash, changed_by)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            "#,
        libsql::params![
            uuid::Uuid::new_v4().to_string(),
            document_id,
            next_version as i64,
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

const LIBSQL_MEMORY_DOCUMENTS_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS memory_documents (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    agent_id TEXT,
    path TEXT NOT NULL,
    content TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    metadata TEXT NOT NULL DEFAULT '{}',
    UNIQUE (user_id, agent_id, path)
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_memory_documents_reborn_document
    ON memory_documents(user_id, path) WHERE agent_id IS NULL;
CREATE INDEX IF NOT EXISTS idx_memory_documents_user ON memory_documents(user_id);
CREATE INDEX IF NOT EXISTS idx_memory_documents_path ON memory_documents(user_id, path);
CREATE INDEX IF NOT EXISTS idx_memory_documents_updated ON memory_documents(updated_at DESC);

CREATE TRIGGER IF NOT EXISTS update_memory_documents_updated_at
    AFTER UPDATE ON memory_documents
    FOR EACH ROW
    WHEN NEW.updated_at = OLD.updated_at
    BEGIN
        UPDATE memory_documents SET updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = NEW.id;
    END;

CREATE TABLE IF NOT EXISTS memory_chunks (
    _rowid INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    document_id TEXT NOT NULL REFERENCES memory_documents(id) ON DELETE CASCADE,
    chunk_index INTEGER NOT NULL,
    content TEXT NOT NULL,
    embedding BLOB,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    UNIQUE (document_id, chunk_index)
);

CREATE INDEX IF NOT EXISTS idx_memory_chunks_document ON memory_chunks(document_id);

CREATE VIRTUAL TABLE IF NOT EXISTS memory_chunks_fts USING fts5(
    content,
    content='memory_chunks',
    content_rowid='_rowid'
);

CREATE TRIGGER IF NOT EXISTS memory_chunks_fts_insert AFTER INSERT ON memory_chunks BEGIN
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
END;

CREATE TABLE IF NOT EXISTS memory_document_versions (
    id TEXT PRIMARY KEY,
    document_id TEXT NOT NULL REFERENCES memory_documents(id) ON DELETE CASCADE,
    version INTEGER NOT NULL,
    content TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    changed_by TEXT,
    UNIQUE(document_id, version)
);

CREATE INDEX IF NOT EXISTS idx_doc_versions_lookup
    ON memory_document_versions(document_id, version DESC);
"#;
