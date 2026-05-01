//! Reborn-native libSQL repository.
//!
//! Persists memory documents in the dedicated `reborn_memory_*` tables with
//! explicit `tenant_id`, `user_id`, `agent_id`, `project_id` scope columns —
//! never the legacy synthetic `memory_documents.user_id` encoding.
//!
//! Behavior is intentionally not yet implemented in this PR (#3118 phase 3).
//! All trait methods return a "not yet implemented" error so callers fail
//! closed; only [`run_migrations`](RebornLibSqlMemoryDocumentRepository::run_migrations)
//! does real work, so the schema is testable in isolation.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_filesystem::{FilesystemError, FilesystemOperation};
use ironclaw_host_api::VirtualPath;

use crate::chunking::MemoryChunkWrite;
use crate::indexer::MemoryDocumentIndexRepository;
use crate::path::{MemoryDocumentPath, MemoryDocumentScope, memory_error, valid_memory_path};
use crate::search::{MemorySearchRequest, MemorySearchResult};

use super::MemoryDocumentRepository;

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

fn not_yet_implemented(path: VirtualPath, operation: FilesystemOperation) -> FilesystemError {
    memory_error(
        path,
        operation,
        "reborn-native libsql memory repository is not yet implemented",
    )
}

#[async_trait]
impl MemoryDocumentRepository for RebornLibSqlMemoryDocumentRepository {
    async fn read_document(
        &self,
        path: &MemoryDocumentPath,
    ) -> Result<Option<Vec<u8>>, FilesystemError> {
        Err(not_yet_implemented(
            path.virtual_path().unwrap_or_else(|_| valid_memory_path()),
            FilesystemOperation::ReadFile,
        ))
    }

    async fn write_document(
        &self,
        path: &MemoryDocumentPath,
        _bytes: &[u8],
    ) -> Result<(), FilesystemError> {
        Err(not_yet_implemented(
            path.virtual_path().unwrap_or_else(|_| valid_memory_path()),
            FilesystemOperation::WriteFile,
        ))
    }

    async fn list_documents(
        &self,
        scope: &MemoryDocumentScope,
    ) -> Result<Vec<MemoryDocumentPath>, FilesystemError> {
        Err(not_yet_implemented(
            scope
                .virtual_prefix()
                .unwrap_or_else(|_| valid_memory_path()),
            FilesystemOperation::ListDir,
        ))
    }

    async fn search_documents(
        &self,
        scope: &MemoryDocumentScope,
        _request: &MemorySearchRequest,
    ) -> Result<Vec<MemorySearchResult>, FilesystemError> {
        Err(not_yet_implemented(
            scope
                .virtual_prefix()
                .unwrap_or_else(|_| valid_memory_path()),
            FilesystemOperation::ReadFile,
        ))
    }
}

#[async_trait]
impl MemoryDocumentIndexRepository for RebornLibSqlMemoryDocumentRepository {
    async fn replace_document_chunks_if_current(
        &self,
        path: &MemoryDocumentPath,
        _expected_content_hash: &str,
        _chunks: &[MemoryChunkWrite],
    ) -> Result<(), FilesystemError> {
        Err(not_yet_implemented(
            path.virtual_path().unwrap_or_else(|_| valid_memory_path()),
            FilesystemOperation::WriteFile,
        ))
    }

    async fn delete_document_chunks(
        &self,
        path: &MemoryDocumentPath,
    ) -> Result<(), FilesystemError> {
        Err(not_yet_implemented(
            path.virtual_path().unwrap_or_else(|_| valid_memory_path()),
            FilesystemOperation::WriteFile,
        ))
    }
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
