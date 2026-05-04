//! PostgreSQL repository adapter for the existing `memory_documents` table shape.

use async_trait::async_trait;
use ironclaw_filesystem::{FilesystemError, FilesystemOperation};
use ironclaw_host_api::VirtualPath;

use crate::chunking::{MemoryChunkWrite, content_bytes_sha256, content_sha256};
use crate::indexer::MemoryDocumentIndexRepository;
use crate::metadata::MemoryWriteOptions;
use crate::path::{MemoryDocumentPath, MemoryDocumentScope, memory_error, valid_memory_path};
use crate::search::{
    MemorySearchRequest, MemorySearchResult, RankedMemorySearchResult, fuse_memory_search_results,
};

use super::{
    MemoryAppendOutcome, MemoryDocumentRepository, db_path_for_memory_document,
    ensure_document_path_does_not_conflict, memory_document_from_db_path, scoped_memory_agent_id,
    scoped_memory_owner_key,
};

/// PostgreSQL repository adapter for the existing `memory_documents` table shape.
pub struct PostgresMemoryDocumentRepository {
    pool: deadpool_postgres::Pool,
}

impl PostgresMemoryDocumentRepository {
    pub fn new(pool: deadpool_postgres::Pool) -> Self {
        Self { pool }
    }

    pub async fn run_migrations(&self) -> Result<(), FilesystemError> {
        let client = self
            .client(valid_memory_path(), FilesystemOperation::CreateDirAll)
            .await?;
        client
            .batch_execute(POSTGRES_MEMORY_DOCUMENTS_SCHEMA)
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

    async fn client(
        &self,
        path: VirtualPath,
        operation: FilesystemOperation,
    ) -> Result<deadpool_postgres::Object, FilesystemError> {
        self.pool
            .get()
            .await
            .map_err(|error| memory_error(path, operation, error.to_string()))
    }
}

async fn postgres_list_documents_for_scope<C>(
    client: &C,
    scope: &MemoryDocumentScope,
    virtual_path: &VirtualPath,
    operation: FilesystemOperation,
) -> Result<Vec<MemoryDocumentPath>, FilesystemError>
where
    C: deadpool_postgres::GenericClient + Sync,
{
    let owner_key = scoped_memory_owner_key(scope);
    let agent_id = scoped_memory_agent_id(scope);
    let rows = client
        .query(
            "SELECT path FROM memory_documents WHERE user_id = $1 AND agent_id IS NOT DISTINCT FROM $2 ORDER BY path",
            &[&owner_key, &agent_id],
        )
        .await
        .map_err(|error| memory_error(virtual_path.clone(), operation, error.to_string()))?;

    Ok(rows
        .into_iter()
        .filter_map(|row| {
            let db_path: String = row.get("path");
            memory_document_from_db_path(scope, &db_path)
        })
        .collect())
}

#[async_trait]
impl MemoryDocumentRepository for PostgresMemoryDocumentRepository {
    async fn read_document(
        &self,
        path: &MemoryDocumentPath,
    ) -> Result<Option<Vec<u8>>, FilesystemError> {
        let virtual_path = path.virtual_path().unwrap_or_else(|_| valid_memory_path());
        let client = self
            .client(virtual_path.clone(), FilesystemOperation::ReadFile)
            .await?;
        let owner_key = scoped_memory_owner_key(path.scope());
        let agent_id = scoped_memory_agent_id(path.scope());
        let db_path = db_path_for_memory_document(path);
        let row = client
            .query_opt(
                "SELECT content FROM memory_documents WHERE user_id = $1 AND agent_id IS NOT DISTINCT FROM $2 AND path = $3",
                &[&owner_key, &agent_id, &db_path],
            )
            .await
            .map_err(|error| memory_error(virtual_path, FilesystemOperation::ReadFile, error.to_string()))?;
        Ok(row.map(|row| {
            let content: String = row.get("content");
            content.into_bytes()
        }))
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
        let mut client = self
            .client(virtual_path.clone(), FilesystemOperation::WriteFile)
            .await?;
        let owner_key = scoped_memory_owner_key(path.scope());
        let agent_id = scoped_memory_agent_id(path.scope());
        let db_path = db_path_for_memory_document(path);
        let txn = client.transaction().await.map_err(|error| {
            memory_error(
                virtual_path.clone(),
                FilesystemOperation::WriteFile,
                error.to_string(),
            )
        })?;
        txn.batch_execute("LOCK TABLE memory_documents IN SHARE ROW EXCLUSIVE MODE")
            .await
            .map_err(|error| {
                memory_error(
                    virtual_path.clone(),
                    FilesystemOperation::WriteFile,
                    error.to_string(),
                )
            })?;
        let documents = postgres_list_documents_for_scope(
            &txn,
            path.scope(),
            &virtual_path,
            FilesystemOperation::WriteFile,
        )
        .await?;
        ensure_document_path_does_not_conflict(path, &documents, FilesystemOperation::WriteFile)?;

        let existing = txn
            .query_opt(
                "SELECT id, content FROM memory_documents WHERE user_id = $1 AND agent_id IS NOT DISTINCT FROM $2 AND path = $3 FOR UPDATE",
                &[&owner_key, &agent_id, &db_path],
            )
            .await
            .map_err(|error| memory_error(virtual_path.clone(), FilesystemOperation::WriteFile, error.to_string()))?;
        if let Some(row) = existing {
            let document_id: uuid::Uuid = row.get("id");
            let previous_content: String = row.get("content");
            if previous_content != content && !previous_content.is_empty() {
                postgres_save_document_version(
                    &txn,
                    &virtual_path,
                    document_id,
                    &previous_content,
                    Some(owner_key.as_str()),
                )
                .await?;
            }
            txn.execute(
                "UPDATE memory_documents SET content = $2, updated_at = NOW() WHERE id = $1",
                &[&document_id, &content],
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
            txn.execute(
                r#"
                    INSERT INTO memory_documents (user_id, agent_id, path, content, metadata)
                    VALUES ($1, $2, $3, $4, '{}'::jsonb)
                    "#,
                &[&owner_key, &agent_id, &db_path, &content],
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
        txn.commit().await.map_err(|error| {
            memory_error(
                virtual_path,
                FilesystemOperation::WriteFile,
                error.to_string(),
            )
        })?;
        Ok(())
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
        let mut client = self
            .client(virtual_path.clone(), FilesystemOperation::WriteFile)
            .await?;
        let owner_key = scoped_memory_owner_key(path.scope());
        let agent_id = scoped_memory_agent_id(path.scope());
        let db_path = db_path_for_memory_document(path);
        let txn = client.transaction().await.map_err(|error| {
            memory_error(
                virtual_path.clone(),
                FilesystemOperation::WriteFile,
                error.to_string(),
            )
        })?;
        txn.batch_execute("LOCK TABLE memory_documents IN SHARE ROW EXCLUSIVE MODE")
            .await
            .map_err(|error| {
                memory_error(
                    virtual_path.clone(),
                    FilesystemOperation::WriteFile,
                    error.to_string(),
                )
            })?;
        let documents = postgres_list_documents_for_scope(
            &txn,
            path.scope(),
            &virtual_path,
            FilesystemOperation::WriteFile,
        )
        .await?;
        ensure_document_path_does_not_conflict(path, &documents, FilesystemOperation::WriteFile)?;

        let existing = txn
            .query_opt(
                "SELECT id, content FROM memory_documents WHERE user_id = $1 AND agent_id IS NOT DISTINCT FROM $2 AND path = $3 FOR UPDATE",
                &[&owner_key, &agent_id, &db_path],
            )
            .await
            .map_err(|error| memory_error(virtual_path.clone(), FilesystemOperation::WriteFile, error.to_string()))?;
        if let Some(row) = existing {
            let document_id: uuid::Uuid = row.get("id");
            let previous_content: String = row.get("content");
            if options.metadata.skip_versioning != Some(true)
                && previous_content != content
                && !previous_content.is_empty()
            {
                postgres_save_document_version(
                    &txn,
                    &virtual_path,
                    document_id,
                    &previous_content,
                    options.changed_by.as_deref(),
                )
                .await?;
            }
            txn.execute(
                "UPDATE memory_documents SET content = $2, updated_at = NOW() WHERE id = $1",
                &[&document_id, &content],
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
            txn.execute(
                r#"
                    INSERT INTO memory_documents (user_id, agent_id, path, content, metadata)
                    VALUES ($1, $2, $3, $4, '{}'::jsonb)
                    "#,
                &[&owner_key, &agent_id, &db_path, &content],
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
        txn.commit().await.map_err(|error| {
            memory_error(
                virtual_path,
                FilesystemOperation::WriteFile,
                error.to_string(),
            )
        })?;
        Ok(())
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
        let mut client = self
            .client(virtual_path.clone(), FilesystemOperation::AppendFile)
            .await?;
        let owner_key = scoped_memory_owner_key(path.scope());
        let agent_id = scoped_memory_agent_id(path.scope());
        let db_path = db_path_for_memory_document(path);
        let txn = client.transaction().await.map_err(|error| {
            memory_error(
                virtual_path.clone(),
                FilesystemOperation::AppendFile,
                error.to_string(),
            )
        })?;
        txn.batch_execute("LOCK TABLE memory_documents IN SHARE ROW EXCLUSIVE MODE")
            .await
            .map_err(|error| {
                memory_error(
                    virtual_path.clone(),
                    FilesystemOperation::AppendFile,
                    error.to_string(),
                )
            })?;
        let existing = txn
            .query_opt(
                "SELECT id, content FROM memory_documents WHERE user_id = $1 AND agent_id IS NOT DISTINCT FROM $2 AND path = $3 FOR UPDATE",
                &[&owner_key, &agent_id, &db_path],
            )
            .await
            .map_err(|error| memory_error(virtual_path.clone(), FilesystemOperation::AppendFile, error.to_string()))?;
        let current_hash = existing.as_ref().map(|row| {
            let previous_content: String = row.get("content");
            content_bytes_sha256(previous_content.as_bytes())
        });
        if current_hash.as_deref() != expected_previous_hash {
            txn.commit().await.map_err(|error| {
                memory_error(
                    virtual_path,
                    FilesystemOperation::AppendFile,
                    error.to_string(),
                )
            })?;
            return Ok(MemoryAppendOutcome::Conflict);
        }

        if let Some(row) = existing {
            let document_id: uuid::Uuid = row.get("id");
            let previous_content: String = row.get("content");
            let content = format!("{previous_content}{append_content}");
            if options.metadata.skip_versioning != Some(true)
                && previous_content != content
                && !previous_content.is_empty()
            {
                postgres_save_document_version(
                    &txn,
                    &virtual_path,
                    document_id,
                    &previous_content,
                    options.changed_by.as_deref(),
                )
                .await?;
            }
            txn.execute(
                "UPDATE memory_documents SET content = $2, updated_at = NOW() WHERE id = $1",
                &[&document_id, &content],
            )
            .await
            .map_err(|error| {
                memory_error(
                    virtual_path.clone(),
                    FilesystemOperation::AppendFile,
                    error.to_string(),
                )
            })?;
        } else {
            let documents = postgres_list_documents_for_scope(
                &txn,
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
            txn.execute(
                r#"
                    INSERT INTO memory_documents (user_id, agent_id, path, content, metadata)
                    VALUES ($1, $2, $3, $4, '{}'::jsonb)
                    "#,
                &[&owner_key, &agent_id, &db_path, &append_content],
            )
            .await
            .map_err(|error| {
                memory_error(
                    virtual_path.clone(),
                    FilesystemOperation::AppendFile,
                    error.to_string(),
                )
            })?;
        }
        txn.commit().await.map_err(|error| {
            memory_error(
                virtual_path,
                FilesystemOperation::AppendFile,
                error.to_string(),
            )
        })?;
        Ok(MemoryAppendOutcome::Appended)
    }

    async fn read_document_metadata(
        &self,
        path: &MemoryDocumentPath,
    ) -> Result<Option<serde_json::Value>, FilesystemError> {
        let virtual_path = path.virtual_path().unwrap_or_else(|_| valid_memory_path());
        let client = self
            .client(virtual_path.clone(), FilesystemOperation::ReadFile)
            .await?;
        let owner_key = scoped_memory_owner_key(path.scope());
        let agent_id = scoped_memory_agent_id(path.scope());
        let db_path = db_path_for_memory_document(path);
        let row = client
            .query_opt(
                "SELECT metadata FROM memory_documents WHERE user_id = $1 AND agent_id IS NOT DISTINCT FROM $2 AND path = $3",
                &[&owner_key, &agent_id, &db_path],
            )
            .await
            .map_err(|error| memory_error(virtual_path, FilesystemOperation::ReadFile, error.to_string()))?;
        Ok(row.map(|row| row.get("metadata")))
    }

    async fn write_document_metadata(
        &self,
        path: &MemoryDocumentPath,
        metadata: &serde_json::Value,
    ) -> Result<(), FilesystemError> {
        let virtual_path = path.virtual_path().unwrap_or_else(|_| valid_memory_path());
        let client = self
            .client(virtual_path.clone(), FilesystemOperation::WriteFile)
            .await?;
        let owner_key = scoped_memory_owner_key(path.scope());
        let agent_id = scoped_memory_agent_id(path.scope());
        let db_path = db_path_for_memory_document(path);
        client
            .execute(
                "UPDATE memory_documents SET metadata = $4, updated_at = NOW() WHERE user_id = $1 AND agent_id IS NOT DISTINCT FROM $2 AND path = $3",
                &[&owner_key, &agent_id, &db_path, metadata],
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
        let client = self
            .client(virtual_path.clone(), FilesystemOperation::ListDir)
            .await?;
        postgres_list_documents_for_scope(
            &client,
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
        let client = self
            .client(virtual_path.clone(), FilesystemOperation::ReadFile)
            .await?;
        let full_text_results = if request.full_text() {
            postgres_full_text_search_ranked(&client, scope, request, &virtual_path).await?
        } else {
            Vec::new()
        };
        let vector_results = if request.vector() {
            if let Some(embedding) = request.query_embedding() {
                postgres_vector_search_ranked(&client, scope, request, embedding, &virtual_path)
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

async fn postgres_full_text_search_ranked(
    client: &deadpool_postgres::Object,
    scope: &MemoryDocumentScope,
    request: &MemorySearchRequest,
    virtual_path: &VirtualPath,
) -> Result<Vec<RankedMemorySearchResult>, FilesystemError> {
    let owner_key = scoped_memory_owner_key(scope);
    let agent_id = scoped_memory_agent_id(scope);
    let limit = request.pre_fusion_limit() as i64;
    let rows = client
        .query(
            r#"
            SELECT c.id, d.path, c.content, ts_rank_cd(c.content_tsv, plainto_tsquery('english', $3)) AS rank
            FROM memory_chunks c
            JOIN memory_documents d ON d.id = c.document_id
            WHERE d.user_id = $1 AND d.agent_id IS NOT DISTINCT FROM $2
              AND c.content_tsv @@ plainto_tsquery('english', $3)
            ORDER BY rank DESC
            LIMIT $4
            "#,
            &[&owner_key, &agent_id, &request.query(), &limit],
        )
        .await
    .map_err(|error| memory_error(virtual_path.clone(), FilesystemOperation::ReadFile, error.to_string()))?;

    Ok(rows
        .into_iter()
        .enumerate()
        .filter_map(|(index, row)| {
            let chunk_id: uuid::Uuid = row.get("id");
            let db_path: String = row.get("path");
            let path = memory_document_from_db_path(scope, &db_path)?;
            let snippet: String = row.get("content");
            Some(RankedMemorySearchResult {
                chunk_key: chunk_id.to_string(),
                path,
                snippet,
                rank: index as u32 + 1,
            })
        })
        .collect())
}

async fn postgres_vector_search_ranked(
    client: &deadpool_postgres::Object,
    scope: &MemoryDocumentScope,
    request: &MemorySearchRequest,
    query_embedding: &[f32],
    virtual_path: &VirtualPath,
) -> Result<Vec<RankedMemorySearchResult>, FilesystemError> {
    let owner_key = scoped_memory_owner_key(scope);
    let agent_id = scoped_memory_agent_id(scope);
    let limit = request.pre_fusion_limit() as i64;
    let query_vector = pgvector::Vector::from(query_embedding.to_vec());
    let rows = client
        .query(
            r#"
            SELECT c.id, d.path, c.content
            FROM memory_chunks c
            JOIN memory_documents d ON d.id = c.document_id
            WHERE d.user_id = $1 AND d.agent_id IS NOT DISTINCT FROM $2
              AND c.embedding IS NOT NULL
            ORDER BY c.embedding <=> $3
            LIMIT $4
            "#,
            &[&owner_key, &agent_id, &query_vector, &limit],
        )
        .await
        .map_err(|error| {
            memory_error(
                virtual_path.clone(),
                FilesystemOperation::ReadFile,
                error.to_string(),
            )
        })?;

    Ok(rows
        .into_iter()
        .enumerate()
        .filter_map(|(index, row)| {
            let chunk_id: uuid::Uuid = row.get("id");
            let db_path: String = row.get("path");
            let path = memory_document_from_db_path(scope, &db_path)?;
            let snippet: String = row.get("content");
            Some(RankedMemorySearchResult {
                chunk_key: chunk_id.to_string(),
                path,
                snippet,
                rank: index as u32 + 1,
            })
        })
        .collect())
}

#[async_trait]
impl MemoryDocumentIndexRepository for PostgresMemoryDocumentRepository {
    async fn replace_document_chunks_if_current(
        &self,
        path: &MemoryDocumentPath,
        expected_content_hash: &str,
        chunks: &[MemoryChunkWrite],
    ) -> Result<(), FilesystemError> {
        let virtual_path = path.virtual_path().unwrap_or_else(|_| valid_memory_path());
        let mut client = self
            .client(virtual_path.clone(), FilesystemOperation::WriteFile)
            .await?;
        let owner_key = scoped_memory_owner_key(path.scope());
        let agent_id = scoped_memory_agent_id(path.scope());
        let db_path = db_path_for_memory_document(path);
        let tx = client.transaction().await.map_err(|error| {
            memory_error(
                virtual_path.clone(),
                FilesystemOperation::WriteFile,
                error.to_string(),
            )
        })?;
        let Some(row) = tx
            .query_opt(
                "SELECT id, content FROM memory_documents WHERE user_id = $1 AND agent_id IS NOT DISTINCT FROM $2 AND path = $3 FOR UPDATE",
                &[&owner_key, &agent_id, &db_path],
            )
            .await
            .map_err(|error| memory_error(virtual_path.clone(), FilesystemOperation::WriteFile, error.to_string()))?
        else {
            return Ok(());
        };
        let document_id: uuid::Uuid = row.get("id");
        let content: String = row.get("content");
        if content_sha256(&content) != expected_content_hash {
            return Ok(());
        }
        tx.execute(
            "DELETE FROM memory_chunks WHERE document_id = $1",
            &[&document_id],
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
            let chunk_id = uuid::Uuid::new_v4();
            let chunk_index = index as i32;
            let embedding_vec = chunk
                .embedding
                .as_ref()
                .map(|embedding| pgvector::Vector::from(embedding.clone()));
            tx.execute(
                r#"
                INSERT INTO memory_chunks (id, document_id, chunk_index, content, embedding)
                VALUES ($1, $2, $3, $4, $5)
                "#,
                &[
                    &chunk_id,
                    &document_id,
                    &chunk_index,
                    &chunk.content,
                    &embedding_vec,
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

async fn postgres_save_document_version<C: deadpool_postgres::GenericClient + Sync>(
    client: &C,
    virtual_path: &VirtualPath,
    document_id: uuid::Uuid,
    content: &str,
    changed_by: Option<&str>,
) -> Result<i32, FilesystemError> {
    let row = client
        .query_one(
            "SELECT COALESCE(MAX(version), 0) + 1 AS next_version FROM memory_document_versions WHERE document_id = $1",
            &[&document_id],
        )
        .await
        .map_err(|error| memory_error(virtual_path.clone(), FilesystemOperation::WriteFile, error.to_string()))?;
    let next_version: i32 = row.get(0);
    client
        .execute(
            r#"
            INSERT INTO memory_document_versions
                (id, document_id, version, content, content_hash, changed_by)
            VALUES (gen_random_uuid(), $1, $2, $3, $4, $5)
            "#,
            &[
                &document_id,
                &next_version,
                &content,
                &content_sha256(content),
                &changed_by,
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

const POSTGRES_MEMORY_DOCUMENTS_SCHEMA: &str = r#"
CREATE EXTENSION IF NOT EXISTS pgcrypto;
CREATE EXTENSION IF NOT EXISTS vector;

CREATE TABLE IF NOT EXISTS memory_documents (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id TEXT NOT NULL,
    agent_id TEXT,
    path TEXT NOT NULL,
    content TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    metadata JSONB NOT NULL DEFAULT '{}',
    CONSTRAINT unique_path_per_user UNIQUE (user_id, agent_id, path)
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_memory_documents_reborn_document
    ON memory_documents(user_id, path) WHERE agent_id IS NULL;
CREATE INDEX IF NOT EXISTS idx_memory_documents_user ON memory_documents(user_id);
CREATE INDEX IF NOT EXISTS idx_memory_documents_path ON memory_documents(user_id, path);
CREATE INDEX IF NOT EXISTS idx_memory_documents_path_prefix ON memory_documents(user_id, path text_pattern_ops);
CREATE INDEX IF NOT EXISTS idx_memory_documents_updated ON memory_documents(updated_at DESC);

CREATE OR REPLACE FUNCTION update_updated_at_column()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ language 'plpgsql';

DROP TRIGGER IF EXISTS update_memory_documents_updated_at ON memory_documents;
CREATE TRIGGER update_memory_documents_updated_at
    BEFORE UPDATE ON memory_documents
    FOR EACH ROW
    EXECUTE FUNCTION update_updated_at_column();

CREATE TABLE IF NOT EXISTS memory_chunks (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    document_id UUID NOT NULL REFERENCES memory_documents(id) ON DELETE CASCADE,
    chunk_index INT NOT NULL,
    content TEXT NOT NULL,
    content_tsv TSVECTOR GENERATED ALWAYS AS (to_tsvector('english', content)) STORED,
    embedding VECTOR(1536),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT unique_chunk_per_doc UNIQUE (document_id, chunk_index)
);

CREATE INDEX IF NOT EXISTS idx_memory_chunks_tsv ON memory_chunks USING GIN(content_tsv);
CREATE INDEX IF NOT EXISTS idx_memory_chunks_embedding ON memory_chunks
    USING hnsw(embedding vector_cosine_ops)
    WITH (m = 16, ef_construction = 64);
CREATE INDEX IF NOT EXISTS idx_memory_chunks_document ON memory_chunks(document_id);

CREATE TABLE IF NOT EXISTS memory_document_versions (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    document_id UUID NOT NULL REFERENCES memory_documents(id) ON DELETE CASCADE,
    version INTEGER NOT NULL,
    content TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    changed_by TEXT,
    UNIQUE(document_id, version)
);

CREATE INDEX IF NOT EXISTS idx_doc_versions_lookup
    ON memory_document_versions(document_id, version DESC);
CREATE INDEX IF NOT EXISTS idx_memory_documents_metadata
    ON memory_documents USING GIN (metadata jsonb_path_ops);
"#;
