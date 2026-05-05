//! Reborn-native PostgreSQL repository.
//!
//! Persists memory documents in the dedicated `reborn_memory_*` tables with
//! explicit `tenant_id`, `user_id`, `agent_id`, `project_id` scope columns —
//! never the legacy synthetic `memory_documents.user_id` encoding.
//!
//! Every read/list/search/write/version/chunk query filters by the full
//! `(tenant_id, user_id, agent_id, project_id)` tuple. Absent agent/project
//! IDs use the empty-string DB-only sentinel; the constructor rejects empty
//! and `_none` user-supplied IDs, so equality comparison is unambiguous.

use async_trait::async_trait;
use ironclaw_filesystem::{FilesystemError, FilesystemOperation};
use ironclaw_host_api::VirtualPath;

use crate::chunking::{MemoryChunkWrite, content_sha256};
use crate::indexer::MemoryDocumentIndexRepository;
use crate::metadata::MemoryWriteOptions;
use crate::path::{MemoryDocumentPath, MemoryDocumentScope, memory_error, valid_memory_path};
use crate::search::{
    MemorySearchRequest, MemorySearchResult, RankedMemorySearchResult, fuse_memory_search_results,
};

use super::{
    MemoryAppendOutcome, MemoryDocumentRepository, ensure_document_path_does_not_conflict,
    reborn_agent_id_db_value, reborn_memory_document_from_row, reborn_project_id_db_value,
    scoped_memory_owner_key,
};

/// Render a tokio_postgres error with its full source chain.
///
/// `tokio_postgres::Error` displays only the error kind ("db error",
/// "deserializing column", …) at the top level — the actual SQLSTATE and
/// server message live in the source chain. Surfacing the chain here so
/// `FilesystemError::Backend.reason` is diagnosable in test output.
fn pg_error_chain(error: &dyn std::error::Error) -> String {
    let mut out = error.to_string();
    let mut source = error.source();
    while let Some(next) = source {
        out.push_str(" :: ");
        out.push_str(&next.to_string());
        source = next.source();
    }
    out
}

/// Reborn-native PostgreSQL repository for `reborn_memory_*` tables.
pub struct RebornPostgresMemoryDocumentRepository {
    pool: deadpool_postgres::Pool,
}

impl RebornPostgresMemoryDocumentRepository {
    pub fn new(pool: deadpool_postgres::Pool) -> Self {
        Self { pool }
    }

    /// Create the Reborn-native tables, vector/text indexes, and triggers if
    /// they do not already exist. Idempotent; safe to call on every startup.
    ///
    /// Wrapped in a Postgres session-level advisory lock so concurrent
    /// callers (multiple processes / parallel tests) serialize cleanly —
    /// `CREATE EXTENSION pgcrypto/vector` is not safe under concurrent
    /// execution otherwise.
    pub async fn run_migrations(&self) -> Result<(), FilesystemError> {
        let client = self
            .client(valid_memory_path(), FilesystemOperation::CreateDirAll)
            .await?;
        // Stable per-crate lock id; chosen by hashing "ironclaw_memory.reborn"
        // and folding into i64 range. The exact value is not load-bearing as
        // long as it is consistent across processes.
        const REBORN_MIGRATION_LOCK_ID: i64 = 0x2026_0501_5238_3118_u64 as i64;
        client
            .execute("SELECT pg_advisory_lock($1)", &[&REBORN_MIGRATION_LOCK_ID])
            .await
            .map_err(|error| {
                memory_error(
                    valid_memory_path(),
                    FilesystemOperation::CreateDirAll,
                    pg_error_chain(&error),
                )
            })?;
        // Short-circuit if the schema is already present. Re-running the full
        // DDL on every `run_migrations` call would take an AccessExclusiveLock
        // on `reborn_memory_documents` for the FK validation in the dependent
        // `CREATE TABLE IF NOT EXISTS reborn_memory_chunks`, which would
        // deadlock against concurrent writers holding RowExclusiveLock once
        // the global write-side LOCK TABLE was removed (zmanian H1+H2).
        // The advisory lock above still serializes the *first* migration
        // across all processes; subsequent calls find the schema and skip.
        let already_migrated = client
            .query_opt(
                "SELECT 1 FROM information_schema.tables \
                 WHERE table_schema = current_schema() \
                   AND table_name = 'reborn_memory_documents'",
                &[],
            )
            .await
            .map_err(|error| {
                memory_error(
                    valid_memory_path(),
                    FilesystemOperation::CreateDirAll,
                    pg_error_chain(&error),
                )
            })?
            .is_some();
        let migration_result = if already_migrated {
            Ok(())
        } else {
            client
                .batch_execute(REBORN_POSTGRES_MEMORY_DOCUMENTS_SCHEMA)
                .await
                .map_err(|error| {
                    memory_error(
                        valid_memory_path(),
                        FilesystemOperation::CreateDirAll,
                        pg_error_chain(&error),
                    )
                })
        };
        // Best-effort unlock; if the lock release fails we still surface the
        // migration result. The lock is session-scoped so the connection
        // returning to the pool will release it on drop anyway.
        let _ = client
            .execute(
                "SELECT pg_advisory_unlock($1)",
                &[&REBORN_MIGRATION_LOCK_ID],
            )
            .await;
        migration_result?;
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
            .map_err(|error| memory_error(path, operation, pg_error_chain(&error)))
    }
}

#[async_trait]
impl MemoryDocumentRepository for RebornPostgresMemoryDocumentRepository {
    async fn read_document(
        &self,
        path: &MemoryDocumentPath,
    ) -> Result<Option<Vec<u8>>, FilesystemError> {
        let virtual_path = path.virtual_path().unwrap_or_else(|_| valid_memory_path());
        let client = self
            .client(virtual_path.clone(), FilesystemOperation::ReadFile)
            .await?;
        let scope = path.scope();
        let row = client
            .query_opt(
                "SELECT content FROM reborn_memory_documents \
                 WHERE tenant_id = $1 AND user_id = $2 AND agent_id = $3 \
                   AND project_id = $4 AND path = $5",
                &[
                    &scope.tenant_id(),
                    &scope.user_id(),
                    &reborn_agent_id_db_value(scope),
                    &reborn_project_id_db_value(scope),
                    &path.relative_path(),
                ],
            )
            .await
            .map_err(|error| {
                memory_error(
                    virtual_path,
                    FilesystemOperation::ReadFile,
                    pg_error_chain(&error),
                )
            })?;
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
        // Direct repository writes go through the same archive path as
        // backend/filesystem writes; without `changed_by` the version row
        // gets `NULL`. The scoped owner key matches the legacy Postgres
        // direct-write behavior so version history stays attributable
        // when operators bypass the higher backend seam.
        let options = MemoryWriteOptions {
            changed_by: Some(scoped_memory_owner_key(path.scope())),
            ..MemoryWriteOptions::default()
        };
        self.write_document_with_options(path, bytes, &options)
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
        let mut client = self
            .client(virtual_path.clone(), FilesystemOperation::WriteFile)
            .await?;
        let scope = path.scope();
        let agent_id_db = reborn_agent_id_db_value(scope);
        let project_id_db = reborn_project_id_db_value(scope);
        let new_content_hash = content_sha256(content);

        let txn = client.transaction().await.map_err(|error| {
            memory_error(
                virtual_path.clone(),
                FilesystemOperation::WriteFile,
                pg_error_chain(&error),
            )
        })?;
        // Per-scope advisory lock instead of a table-level LOCK so writes
        // for different (tenant, user, agent, project) tuples do not
        // serialize globally — see `reborn_postgres_lock_scope` (zmanian H2).
        reborn_postgres_lock_scope(&txn, scope, &virtual_path, FilesystemOperation::WriteFile)
            .await?;

        let documents = reborn_postgres_list_paths_for_scope(
            &txn,
            scope,
            &virtual_path,
            FilesystemOperation::WriteFile,
        )
        .await?;
        ensure_document_path_does_not_conflict(path, &documents, FilesystemOperation::WriteFile)?;

        let existing = txn
            .query_opt(
                "SELECT id, content FROM reborn_memory_documents \
                 WHERE tenant_id = $1 AND user_id = $2 AND agent_id = $3 \
                   AND project_id = $4 AND path = $5 FOR UPDATE",
                &[
                    &scope.tenant_id(),
                    &scope.user_id(),
                    &agent_id_db,
                    &project_id_db,
                    &path.relative_path(),
                ],
            )
            .await
            .map_err(|error| {
                memory_error(
                    virtual_path.clone(),
                    FilesystemOperation::WriteFile,
                    pg_error_chain(&error),
                )
            })?;

        if let Some(row) = existing {
            let document_id: uuid::Uuid = row.get("id");
            let previous_content: String = row.get("content");
            let should_version = options.metadata.skip_versioning != Some(true)
                && previous_content != content
                && !previous_content.is_empty();
            if should_version {
                reborn_postgres_save_document_version(
                    &txn,
                    &virtual_path,
                    document_id,
                    &previous_content,
                    options.changed_by.as_deref(),
                )
                .await?;
            }
            txn.execute(
                "UPDATE reborn_memory_documents \
                 SET content = $2, content_hash = $3, updated_at = NOW() \
                 WHERE id = $1",
                &[&document_id, &content, &new_content_hash],
            )
            .await
            .map_err(|error| {
                memory_error(
                    virtual_path.clone(),
                    FilesystemOperation::WriteFile,
                    pg_error_chain(&error),
                )
            })?;
        } else {
            txn.execute(
                "INSERT INTO reborn_memory_documents \
                     (tenant_id, user_id, agent_id, project_id, path, \
                      content, content_hash, metadata) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, '{}'::jsonb)",
                &[
                    &scope.tenant_id(),
                    &scope.user_id(),
                    &agent_id_db,
                    &project_id_db,
                    &path.relative_path(),
                    &content,
                    &new_content_hash,
                ],
            )
            .await
            .map_err(|error| {
                memory_error(
                    virtual_path.clone(),
                    FilesystemOperation::WriteFile,
                    pg_error_chain(&error),
                )
            })?;
        }

        txn.commit().await.map_err(|error| {
            memory_error(
                virtual_path,
                FilesystemOperation::WriteFile,
                pg_error_chain(&error),
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
        let virtual_path = path.virtual_path().unwrap_or_else(|_| valid_memory_path());
        let append_content = std::str::from_utf8(bytes).map_err(|_| {
            memory_error(
                virtual_path.clone(),
                FilesystemOperation::AppendFile,
                "memory document content must be UTF-8",
            )
        })?;
        let mut client = self
            .client(virtual_path.clone(), FilesystemOperation::AppendFile)
            .await?;
        let scope = path.scope();
        let agent_id_db = reborn_agent_id_db_value(scope);
        let project_id_db = reborn_project_id_db_value(scope);
        let txn = client.transaction().await.map_err(|error| {
            memory_error(
                virtual_path.clone(),
                FilesystemOperation::AppendFile,
                pg_error_chain(&error),
            )
        })?;
        // Per-scope advisory lock keeps concurrent appends within the same
        // (tenant, user, agent, project) tuple serialized; `FOR UPDATE`
        // on the SELECT pins the specific row across the transaction.
        // Different scopes do not contend — see `reborn_postgres_lock_scope`
        // (zmanian H2).
        reborn_postgres_lock_scope(&txn, scope, &virtual_path, FilesystemOperation::AppendFile)
            .await?;
        let existing = txn
            .query_opt(
                "SELECT id, content FROM reborn_memory_documents \
                 WHERE tenant_id = $1 AND user_id = $2 AND agent_id = $3 \
                   AND project_id = $4 AND path = $5 \
                 FOR UPDATE",
                &[
                    &scope.tenant_id(),
                    &scope.user_id(),
                    &agent_id_db,
                    &project_id_db,
                    &path.relative_path(),
                ],
            )
            .await
            .map_err(|error| {
                memory_error(
                    virtual_path.clone(),
                    FilesystemOperation::AppendFile,
                    pg_error_chain(&error),
                )
            })?;
        let current_hash = existing.as_ref().map(|row| {
            let previous_content: String = row.get("content");
            content_sha256(&previous_content)
        });
        if current_hash.as_deref() != expected_previous_hash {
            txn.commit().await.map_err(|error| {
                memory_error(
                    virtual_path,
                    FilesystemOperation::AppendFile,
                    pg_error_chain(&error),
                )
            })?;
            return Ok(MemoryAppendOutcome::Conflict);
        }

        if let Some(row) = existing {
            let document_id: uuid::Uuid = row.get("id");
            let previous_content: String = row.get("content");
            let combined = format!("{previous_content}{append_content}");
            let new_content_hash = content_sha256(&combined);
            let should_version = options.metadata.skip_versioning != Some(true)
                && previous_content != combined
                && !previous_content.is_empty();
            if should_version {
                reborn_postgres_save_document_version(
                    &txn,
                    &virtual_path,
                    document_id,
                    &previous_content,
                    options.changed_by.as_deref(),
                )
                .await?;
            }
            txn.execute(
                "UPDATE reborn_memory_documents \
                 SET content = $2, content_hash = $3, updated_at = NOW() \
                 WHERE id = $1",
                &[&document_id, &combined, &new_content_hash],
            )
            .await
            .map_err(|error| {
                memory_error(
                    virtual_path.clone(),
                    FilesystemOperation::AppendFile,
                    pg_error_chain(&error),
                )
            })?;
        } else {
            let documents = reborn_postgres_list_paths_for_scope(
                &txn,
                scope,
                &virtual_path,
                FilesystemOperation::AppendFile,
            )
            .await?;
            ensure_document_path_does_not_conflict(
                path,
                &documents,
                FilesystemOperation::AppendFile,
            )?;
            let new_content_hash = content_sha256(append_content);
            txn.execute(
                "INSERT INTO reborn_memory_documents \
                     (tenant_id, user_id, agent_id, project_id, path, \
                      content, content_hash, metadata) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, '{}'::jsonb)",
                &[
                    &scope.tenant_id(),
                    &scope.user_id(),
                    &agent_id_db,
                    &project_id_db,
                    &path.relative_path(),
                    &append_content,
                    &new_content_hash,
                ],
            )
            .await
            .map_err(|error| {
                memory_error(
                    virtual_path.clone(),
                    FilesystemOperation::AppendFile,
                    pg_error_chain(&error),
                )
            })?;
        }
        txn.commit().await.map_err(|error| {
            memory_error(
                virtual_path,
                FilesystemOperation::AppendFile,
                pg_error_chain(&error),
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
        let scope = path.scope();
        let row = client
            .query_opt(
                "SELECT metadata FROM reborn_memory_documents \
                 WHERE tenant_id = $1 AND user_id = $2 AND agent_id = $3 \
                   AND project_id = $4 AND path = $5",
                &[
                    &scope.tenant_id(),
                    &scope.user_id(),
                    &reborn_agent_id_db_value(scope),
                    &reborn_project_id_db_value(scope),
                    &path.relative_path(),
                ],
            )
            .await
            .map_err(|error| {
                memory_error(
                    virtual_path,
                    FilesystemOperation::ReadFile,
                    pg_error_chain(&error),
                )
            })?;
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
        let scope = path.scope();
        client
            .execute(
                "UPDATE reborn_memory_documents \
                 SET metadata = $6, updated_at = NOW() \
                 WHERE tenant_id = $1 AND user_id = $2 AND agent_id = $3 \
                   AND project_id = $4 AND path = $5",
                &[
                    &scope.tenant_id(),
                    &scope.user_id(),
                    &reborn_agent_id_db_value(scope),
                    &reborn_project_id_db_value(scope),
                    &path.relative_path(),
                    metadata,
                ],
            )
            .await
            .map_err(|error| {
                memory_error(
                    virtual_path,
                    FilesystemOperation::WriteFile,
                    pg_error_chain(&error),
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
        let client = self
            .client(virtual_path.clone(), FilesystemOperation::ListDir)
            .await?;
        reborn_postgres_list_paths_for_scope(
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
            reborn_postgres_full_text_search_ranked(&client, scope, request, &virtual_path).await?
        } else {
            Vec::new()
        };
        let vector_results = if request.vector() {
            if let Some(embedding) = request.query_embedding() {
                reborn_postgres_vector_search_ranked(
                    &client,
                    scope,
                    request,
                    embedding,
                    &virtual_path,
                )
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
impl MemoryDocumentIndexRepository for RebornPostgresMemoryDocumentRepository {
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
        let scope = path.scope();
        let tx = client.transaction().await.map_err(|error| {
            memory_error(
                virtual_path.clone(),
                FilesystemOperation::WriteFile,
                pg_error_chain(&error),
            )
        })?;
        let Some(row) = tx
            .query_opt(
                "SELECT id, content_hash FROM reborn_memory_documents \
                 WHERE tenant_id = $1 AND user_id = $2 AND agent_id = $3 \
                   AND project_id = $4 AND path = $5 FOR UPDATE",
                &[
                    &scope.tenant_id(),
                    &scope.user_id(),
                    &reborn_agent_id_db_value(scope),
                    &reborn_project_id_db_value(scope),
                    &path.relative_path(),
                ],
            )
            .await
            .map_err(|error| {
                memory_error(
                    virtual_path.clone(),
                    FilesystemOperation::WriteFile,
                    pg_error_chain(&error),
                )
            })?
        else {
            return Ok(());
        };
        let document_id: uuid::Uuid = row.get("id");
        let current_hash: String = row.get("content_hash");
        if current_hash != expected_content_hash {
            return Ok(());
        }
        tx.execute(
            "DELETE FROM reborn_memory_chunks WHERE document_id = $1",
            &[&document_id],
        )
        .await
        .map_err(|error| {
            memory_error(
                virtual_path.clone(),
                FilesystemOperation::WriteFile,
                pg_error_chain(&error),
            )
        })?;
        for (index, chunk) in chunks.iter().enumerate() {
            let chunk_id = uuid::Uuid::new_v4();
            let chunk_index = index as i32;
            let chunk_hash = content_sha256(&chunk.content);
            let embedding_vec = chunk
                .embedding
                .as_ref()
                .map(|embedding| pgvector::Vector::from(embedding.clone()));
            tx.execute(
                "INSERT INTO reborn_memory_chunks \
                     (id, document_id, chunk_index, content, content_hash, embedding) \
                 VALUES ($1, $2, $3, $4, $5, $6)",
                &[
                    &chunk_id,
                    &document_id,
                    &chunk_index,
                    &chunk.content,
                    &chunk_hash,
                    &embedding_vec,
                ],
            )
            .await
            .map_err(|error| {
                memory_error(
                    virtual_path.clone(),
                    FilesystemOperation::WriteFile,
                    pg_error_chain(&error),
                )
            })?;
        }
        tx.commit().await.map_err(|error| {
            memory_error(
                virtual_path,
                FilesystemOperation::WriteFile,
                pg_error_chain(&error),
            )
        })?;
        Ok(())
    }
}

/// Stable per-scope identifier used as the input to `pg_advisory_xact_lock`.
///
/// Two writes only contend when they share a `(tenant, user, agent, project)`
/// tuple — different scopes hash to different lock keys and proceed in
/// parallel. This avoids the global serialization that a table-level
/// `LOCK TABLE … IN SHARE ROW EXCLUSIVE MODE` would impose under
/// multi-tenant load (zmanian H2). Empty strings for absent agent/project
/// match the on-disk DB sentinels so the lock identity tracks the actual
/// row identity.
fn reborn_postgres_scope_lock_key(scope: &MemoryDocumentScope) -> String {
    format!(
        "tenant:{}:user:{}:agent:{}:project:{}",
        scope.tenant_id(),
        scope.user_id(),
        reborn_agent_id_db_value(scope),
        reborn_project_id_db_value(scope),
    )
}

/// Acquire the transaction-scoped advisory lock for `scope`.
///
/// The lock is released automatically when the surrounding transaction
/// commits or rolls back; no explicit unlock is needed. `hashtext()`
/// returns int4 which we cast to bigint for the single-arg
/// `pg_advisory_xact_lock(bigint)` overload — collisions across unrelated
/// scopes are tolerable because a colliding pair just shares a lock
/// briefly, never affecting correctness.
async fn reborn_postgres_lock_scope<C>(
    client: &C,
    scope: &MemoryDocumentScope,
    virtual_path: &VirtualPath,
    operation: FilesystemOperation,
) -> Result<(), FilesystemError>
where
    C: deadpool_postgres::GenericClient + Sync,
{
    let key = reborn_postgres_scope_lock_key(scope);
    client
        .execute(
            "SELECT pg_advisory_xact_lock(hashtext($1)::bigint)",
            &[&key],
        )
        .await
        .map_err(|error| {
            let mut chain = pg_error_chain(&error);
            let mut source = std::error::Error::source(&error);
            while let Some(next) = source {
                chain.push_str(" :: ");
                chain.push_str(&next.to_string());
                source = next.source();
            }
            memory_error(virtual_path.clone(), operation, chain)
        })?;
    Ok(())
}

async fn reborn_postgres_list_paths_for_scope<C>(
    client: &C,
    scope: &MemoryDocumentScope,
    virtual_path: &VirtualPath,
    operation: FilesystemOperation,
) -> Result<Vec<MemoryDocumentPath>, FilesystemError>
where
    C: deadpool_postgres::GenericClient + Sync,
{
    let rows = client
        .query(
            "SELECT path FROM reborn_memory_documents \
             WHERE tenant_id = $1 AND user_id = $2 AND agent_id = $3 AND project_id = $4 \
             ORDER BY path",
            &[
                &scope.tenant_id(),
                &scope.user_id(),
                &reborn_agent_id_db_value(scope),
                &reborn_project_id_db_value(scope),
            ],
        )
        .await
        .map_err(|error| memory_error(virtual_path.clone(), operation, pg_error_chain(&error)))?;
    Ok(rows
        .into_iter()
        .filter_map(|row| {
            let db_path: String = row.get("path");
            reborn_memory_document_from_row(
                scope.tenant_id(),
                scope.user_id(),
                reborn_agent_id_db_value(scope),
                reborn_project_id_db_value(scope),
                &db_path,
            )
        })
        .collect())
}

async fn reborn_postgres_save_document_version<C>(
    client: &C,
    virtual_path: &VirtualPath,
    document_id: uuid::Uuid,
    content: &str,
    changed_by: Option<&str>,
) -> Result<i32, FilesystemError>
where
    C: deadpool_postgres::GenericClient + Sync,
{
    let row = client
        .query_one(
            "SELECT COALESCE(MAX(version), 0) + 1 AS next_version \
             FROM reborn_memory_document_versions WHERE document_id = $1",
            &[&document_id],
        )
        .await
        .map_err(|error| {
            memory_error(
                virtual_path.clone(),
                FilesystemOperation::WriteFile,
                pg_error_chain(&error),
            )
        })?;
    let next_version: i32 = row.get(0);
    client
        .execute(
            "INSERT INTO reborn_memory_document_versions \
                 (id, document_id, version, content, content_hash, changed_by) \
             VALUES (gen_random_uuid(), $1, $2, $3, $4, $5)",
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
                pg_error_chain(&error),
            )
        })?;
    Ok(next_version)
}

async fn reborn_postgres_full_text_search_ranked(
    client: &deadpool_postgres::Object,
    scope: &MemoryDocumentScope,
    request: &MemorySearchRequest,
    virtual_path: &VirtualPath,
) -> Result<Vec<RankedMemorySearchResult>, FilesystemError> {
    let limit = request.pre_fusion_limit() as i64;
    let rows = client
        .query(
            "SELECT c.id, d.tenant_id, d.user_id, d.agent_id, d.project_id, d.path, c.content, \
                    ts_rank_cd(c.content_tsv, plainto_tsquery('english', $5)) AS rank \
             FROM reborn_memory_chunks c \
             JOIN reborn_memory_documents d ON d.id = c.document_id \
             WHERE d.tenant_id = $1 AND d.user_id = $2 AND d.agent_id = $3 \
               AND d.project_id = $4 \
               AND c.content_tsv @@ plainto_tsquery('english', $5) \
             ORDER BY rank DESC \
             LIMIT $6",
            &[
                &scope.tenant_id(),
                &scope.user_id(),
                &reborn_agent_id_db_value(scope),
                &reborn_project_id_db_value(scope),
                &request.query(),
                &limit,
            ],
        )
        .await
        .map_err(|error| {
            memory_error(
                virtual_path.clone(),
                FilesystemOperation::ReadFile,
                pg_error_chain(&error),
            )
        })?;

    Ok(rows
        .into_iter()
        .enumerate()
        .filter_map(|(index, row)| {
            let chunk_id: uuid::Uuid = row.get("id");
            let tenant_id: String = row.get("tenant_id");
            let user_id: String = row.get("user_id");
            let agent_id_db: String = row.get("agent_id");
            let project_id_db: String = row.get("project_id");
            let db_path: String = row.get("path");
            let snippet: String = row.get("content");
            let path = reborn_memory_document_from_row(
                &tenant_id,
                &user_id,
                &agent_id_db,
                &project_id_db,
                &db_path,
            )?;
            Some(RankedMemorySearchResult {
                chunk_key: chunk_id.to_string(),
                path,
                snippet,
                rank: index as u32 + 1,
            })
        })
        .collect())
}

async fn reborn_postgres_vector_search_ranked(
    client: &deadpool_postgres::Object,
    scope: &MemoryDocumentScope,
    request: &MemorySearchRequest,
    query_embedding: &[f32],
    virtual_path: &VirtualPath,
) -> Result<Vec<RankedMemorySearchResult>, FilesystemError> {
    let limit = request.pre_fusion_limit() as i64;
    let query_vector = pgvector::Vector::from(query_embedding.to_vec());
    let rows = client
        .query(
            "SELECT c.id, d.tenant_id, d.user_id, d.agent_id, d.project_id, d.path, c.content \
             FROM reborn_memory_chunks c \
             JOIN reborn_memory_documents d ON d.id = c.document_id \
             WHERE d.tenant_id = $1 AND d.user_id = $2 AND d.agent_id = $3 \
               AND d.project_id = $4 \
               AND c.embedding IS NOT NULL \
             ORDER BY c.embedding <=> $5 \
             LIMIT $6",
            &[
                &scope.tenant_id(),
                &scope.user_id(),
                &reborn_agent_id_db_value(scope),
                &reborn_project_id_db_value(scope),
                &query_vector,
                &limit,
            ],
        )
        .await
        .map_err(|error| {
            memory_error(
                virtual_path.clone(),
                FilesystemOperation::ReadFile,
                pg_error_chain(&error),
            )
        })?;

    Ok(rows
        .into_iter()
        .enumerate()
        .filter_map(|(index, row)| {
            let chunk_id: uuid::Uuid = row.get("id");
            let tenant_id: String = row.get("tenant_id");
            let user_id: String = row.get("user_id");
            let agent_id_db: String = row.get("agent_id");
            let project_id_db: String = row.get("project_id");
            let db_path: String = row.get("path");
            let snippet: String = row.get("content");
            let path = reborn_memory_document_from_row(
                &tenant_id,
                &user_id,
                &agent_id_db,
                &project_id_db,
                &db_path,
            )?;
            Some(RankedMemorySearchResult {
                chunk_key: chunk_id.to_string(),
                path,
                snippet,
                rank: index as u32 + 1,
            })
        })
        .collect())
}

const REBORN_POSTGRES_MEMORY_DOCUMENTS_SCHEMA: &str = r#"
CREATE EXTENSION IF NOT EXISTS pgcrypto;
CREATE EXTENSION IF NOT EXISTS vector;

CREATE TABLE IF NOT EXISTS reborn_memory_documents (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    tenant_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    agent_id TEXT NOT NULL DEFAULT '',
    project_id TEXT NOT NULL DEFAULT '',
    path TEXT NOT NULL,
    content TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    metadata JSONB NOT NULL DEFAULT '{}',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT reborn_memory_documents_unique_scope_path
        UNIQUE (tenant_id, user_id, agent_id, project_id, path)
);

CREATE INDEX IF NOT EXISTS idx_reborn_memory_documents_scope
    ON reborn_memory_documents(tenant_id, user_id, agent_id, project_id);
CREATE INDEX IF NOT EXISTS idx_reborn_memory_documents_scope_path
    ON reborn_memory_documents(tenant_id, user_id, agent_id, project_id, path);
CREATE INDEX IF NOT EXISTS idx_reborn_memory_documents_updated
    ON reborn_memory_documents(updated_at DESC);
CREATE INDEX IF NOT EXISTS idx_reborn_memory_documents_metadata
    ON reborn_memory_documents USING GIN (metadata jsonb_path_ops);

CREATE OR REPLACE FUNCTION reborn_memory_documents_set_updated_at()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

-- Create the updated_at trigger only if it does not already exist. The
-- previous DROP-then-CREATE pattern took an AccessExclusiveLock on the
-- table on every `run_migrations` call and would deadlock with concurrent
-- writers holding RowExclusiveLock once we removed the global table-level
-- LOCK on the write path (zmanian H2 → exposes H1). The function body is
-- still `CREATE OR REPLACE FUNCTION` above, so refreshing the function
-- logic does not require touching the trigger.
DO $reborn_trigger$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_trigger
        WHERE tgname = 'update_reborn_memory_documents_updated_at'
          AND tgrelid = 'reborn_memory_documents'::regclass
    ) THEN
        CREATE TRIGGER update_reborn_memory_documents_updated_at
            BEFORE UPDATE ON reborn_memory_documents
            FOR EACH ROW
            EXECUTE FUNCTION reborn_memory_documents_set_updated_at();
    END IF;
END
$reborn_trigger$;

CREATE TABLE IF NOT EXISTS reborn_memory_chunks (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    document_id UUID NOT NULL REFERENCES reborn_memory_documents(id) ON DELETE CASCADE,
    chunk_index INT NOT NULL,
    content TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    content_tsv TSVECTOR GENERATED ALWAYS AS (to_tsvector('english', content)) STORED,
    -- Unbounded `vector` so any provider dimension is accepted (Ollama
    -- 768/1024-dim, OpenAI 1536/3072-dim, etc.). pgvector's HNSW index
    -- requires a fixed dimension, so we omit it here and rely on exact
    -- (sequential) cosine distance at search time. This matches the
    -- legacy migration `migrations/V9__flexible_embedding_dimension.sql`
    -- decision: for a personal-assistant-scale dataset the linear scan
    -- has negligible impact, and provider flexibility is the higher-value
    -- contract.
    embedding vector,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT reborn_memory_chunks_unique_chunk_per_doc UNIQUE (document_id, chunk_index)
);

CREATE INDEX IF NOT EXISTS idx_reborn_memory_chunks_tsv
    ON reborn_memory_chunks USING GIN(content_tsv);
CREATE INDEX IF NOT EXISTS idx_reborn_memory_chunks_document
    ON reborn_memory_chunks(document_id);

CREATE TABLE IF NOT EXISTS reborn_memory_document_versions (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    document_id UUID NOT NULL REFERENCES reborn_memory_documents(id) ON DELETE CASCADE,
    version INTEGER NOT NULL,
    content TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    changed_by TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (document_id, version)
);

CREATE INDEX IF NOT EXISTS idx_reborn_memory_document_versions_lookup
    ON reborn_memory_document_versions(document_id, version DESC);
"#;
