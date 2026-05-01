//! Reborn-native PostgreSQL repository.
//!
//! Persists memory documents in the dedicated `reborn_memory_*` tables with
//! explicit `tenant_id`, `user_id`, `agent_id`, `project_id` scope columns —
//! never the legacy synthetic `memory_documents.user_id` encoding.
//!
//! Behavior is intentionally not yet implemented in this PR (#3118 phase 3).
//! All trait methods return a "not yet implemented" error so callers fail
//! closed; only [`run_migrations`](RebornPostgresMemoryDocumentRepository::run_migrations)
//! does real work, so the schema is testable in isolation.

use async_trait::async_trait;
use ironclaw_filesystem::{FilesystemError, FilesystemOperation};
use ironclaw_host_api::VirtualPath;

use crate::chunking::MemoryChunkWrite;
use crate::indexer::MemoryDocumentIndexRepository;
use crate::path::{MemoryDocumentPath, MemoryDocumentScope, memory_error, valid_memory_path};
use crate::search::{MemorySearchRequest, MemorySearchResult};

use super::MemoryDocumentRepository;

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
    pub async fn run_migrations(&self) -> Result<(), FilesystemError> {
        let client = self
            .client(valid_memory_path(), FilesystemOperation::CreateDirAll)
            .await?;
        client
            .batch_execute(REBORN_POSTGRES_MEMORY_DOCUMENTS_SCHEMA)
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

fn not_yet_implemented(path: VirtualPath, operation: FilesystemOperation) -> FilesystemError {
    memory_error(
        path,
        operation,
        "reborn-native postgres memory repository is not yet implemented",
    )
}

#[async_trait]
impl MemoryDocumentRepository for RebornPostgresMemoryDocumentRepository {
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
impl MemoryDocumentIndexRepository for RebornPostgresMemoryDocumentRepository {
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

DROP TRIGGER IF EXISTS update_reborn_memory_documents_updated_at ON reborn_memory_documents;
CREATE TRIGGER update_reborn_memory_documents_updated_at
    BEFORE UPDATE ON reborn_memory_documents
    FOR EACH ROW
    EXECUTE FUNCTION reborn_memory_documents_set_updated_at();

CREATE TABLE IF NOT EXISTS reborn_memory_chunks (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    document_id UUID NOT NULL REFERENCES reborn_memory_documents(id) ON DELETE CASCADE,
    chunk_index INT NOT NULL,
    content TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    content_tsv TSVECTOR GENERATED ALWAYS AS (to_tsvector('english', content)) STORED,
    embedding VECTOR(1536),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT reborn_memory_chunks_unique_chunk_per_doc UNIQUE (document_id, chunk_index)
);

CREATE INDEX IF NOT EXISTS idx_reborn_memory_chunks_tsv
    ON reborn_memory_chunks USING GIN(content_tsv);
CREATE INDEX IF NOT EXISTS idx_reborn_memory_chunks_embedding
    ON reborn_memory_chunks
    USING hnsw(embedding vector_cosine_ops)
    WITH (m = 16, ef_construction = 64);
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
