//! Memory document filesystem adapters for IronClaw Reborn.
//!
//! This crate owns memory-specific path grammar and repository seams. The
//! generic filesystem crate owns only virtual path authority, scoped mounts,
//! backend cataloging, and backend routing.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ironclaw_filesystem::{
    DirEntry, FileStat, FileType, FilesystemError, FilesystemOperation, RootFilesystem,
};
use ironclaw_host_api::{HostApiError, VirtualPath};

/// Tenant/user/project scope for DB-backed memory documents exposed as virtual files.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MemoryDocumentScope {
    tenant_id: String,
    user_id: String,
    project_id: Option<String>,
}

impl MemoryDocumentScope {
    pub fn new(
        tenant_id: impl Into<String>,
        user_id: impl Into<String>,
        project_id: Option<&str>,
    ) -> Result<Self, HostApiError> {
        let tenant_id = validated_memory_segment("memory tenant", tenant_id.into())?;
        let user_id = validated_memory_segment("memory user", user_id.into())?;
        let project_id = project_id
            .map(|project_id| validated_memory_segment("memory project", project_id.to_string()))
            .transpose()?;
        if project_id.as_deref() == Some("_none") {
            return Err(HostApiError::InvalidId {
                kind: "memory project",
                value: "_none".to_string(),
                reason: "_none is reserved for absent project ids".to_string(),
            });
        }
        Ok(Self {
            tenant_id,
            user_id,
            project_id,
        })
    }

    pub fn tenant_id(&self) -> &str {
        &self.tenant_id
    }

    pub fn user_id(&self) -> &str {
        &self.user_id
    }

    pub fn project_id(&self) -> Option<&str> {
        self.project_id.as_deref()
    }

    fn virtual_prefix(&self) -> Result<VirtualPath, HostApiError> {
        VirtualPath::new(format!(
            "/memory/tenants/{}/users/{}/projects/{}",
            self.tenant_id,
            self.user_id,
            self.project_id.as_deref().unwrap_or("_none")
        ))
    }
}

/// File-shaped memory document key inside the memory document repository.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MemoryDocumentPath {
    scope: MemoryDocumentScope,
    relative_path: String,
}

impl MemoryDocumentPath {
    pub fn new(
        tenant_id: impl Into<String>,
        user_id: impl Into<String>,
        project_id: Option<&str>,
        relative_path: impl Into<String>,
    ) -> Result<Self, HostApiError> {
        let scope = MemoryDocumentScope::new(tenant_id, user_id, project_id)?;
        let relative_path = validated_memory_relative_path(relative_path.into())?;
        Ok(Self {
            scope,
            relative_path,
        })
    }

    pub fn scope(&self) -> &MemoryDocumentScope {
        &self.scope
    }

    pub fn tenant_id(&self) -> &str {
        self.scope.tenant_id()
    }

    pub fn user_id(&self) -> &str {
        self.scope.user_id()
    }

    pub fn project_id(&self) -> Option<&str> {
        self.scope.project_id()
    }

    pub fn relative_path(&self) -> &str {
        &self.relative_path
    }

    fn virtual_path(&self) -> Result<VirtualPath, HostApiError> {
        VirtualPath::new(format!(
            "{}/{}",
            self.scope.virtual_prefix()?.as_str(),
            self.relative_path
        ))
    }
}

struct ParsedMemoryPath {
    scope: MemoryDocumentScope,
    relative_path: Option<String>,
}

impl ParsedMemoryPath {
    fn from_virtual_path(
        path: &VirtualPath,
        operation: FilesystemOperation,
    ) -> Result<Self, FilesystemError> {
        let segments: Vec<&str> = path.as_str().trim_matches('/').split('/').collect();
        if segments.len() < 7
            || segments.first() != Some(&"memory")
            || segments.get(1) != Some(&"tenants")
            || segments.get(3) != Some(&"users")
            || segments.get(5) != Some(&"projects")
        {
            return Err(memory_error(
                path.clone(),
                operation,
                "expected /memory/tenants/{tenant}/users/{user}/projects/{project}/{path}",
            ));
        }

        let tenant_id = *segments.get(2).ok_or_else(|| {
            memory_error(path.clone(), operation, "memory tenant segment is missing")
        })?;
        let user_id = *segments.get(4).ok_or_else(|| {
            memory_error(path.clone(), operation, "memory user segment is missing")
        })?;
        let raw_project_id = *segments.get(6).ok_or_else(|| {
            memory_error(path.clone(), operation, "memory project segment is missing")
        })?;
        let project_id = if raw_project_id == "_none" {
            None
        } else {
            Some(raw_project_id)
        };
        let scope = MemoryDocumentScope::new(tenant_id, user_id, project_id).map_err(|error| {
            memory_error(
                path.clone(),
                operation,
                format!("invalid memory document scope: {error}"),
            )
        })?;
        let relative_path = if segments.len() > 7 {
            Some(
                validated_memory_relative_path(segments[7..].join("/")).map_err(|error| {
                    memory_error(
                        path.clone(),
                        operation,
                        format!("invalid memory document path: {error}"),
                    )
                })?,
            )
        } else {
            None
        };

        Ok(Self {
            scope,
            relative_path,
        })
    }
}

/// Repository for file-shaped memory documents.
///
/// Implementations own the actual source of truth, such as the existing
/// `memory_documents` table. Search chunks and embeddings should be updated by
/// the memory service/indexer, not by generic filesystem routing code.
#[async_trait]
pub trait MemoryDocumentRepository: Send + Sync {
    async fn read_document(
        &self,
        path: &MemoryDocumentPath,
    ) -> Result<Option<Vec<u8>>, FilesystemError>;

    async fn write_document(
        &self,
        path: &MemoryDocumentPath,
        bytes: &[u8],
    ) -> Result<(), FilesystemError>;

    async fn list_documents(
        &self,
        scope: &MemoryDocumentScope,
    ) -> Result<Vec<MemoryDocumentPath>, FilesystemError>;
}

/// Hook invoked after successful memory document writes so derived state can be refreshed.
#[async_trait]
pub trait MemoryDocumentIndexer: Send + Sync {
    async fn reindex_document(&self, path: &MemoryDocumentPath) -> Result<(), FilesystemError>;
}

/// In-memory memory document repository for tests and examples.
#[derive(Default)]
pub struct InMemoryMemoryDocumentRepository {
    documents: Mutex<BTreeMap<MemoryDocumentPath, Vec<u8>>>,
}

impl InMemoryMemoryDocumentRepository {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl MemoryDocumentRepository for InMemoryMemoryDocumentRepository {
    async fn read_document(
        &self,
        path: &MemoryDocumentPath,
    ) -> Result<Option<Vec<u8>>, FilesystemError> {
        let documents = self.documents.lock().map_err(|_| {
            memory_error(
                path.virtual_path().unwrap_or_else(|_| valid_memory_path()),
                FilesystemOperation::ReadFile,
                "memory document repository lock poisoned",
            )
        })?;
        Ok(documents.get(path).cloned())
    }

    async fn write_document(
        &self,
        path: &MemoryDocumentPath,
        bytes: &[u8],
    ) -> Result<(), FilesystemError> {
        let mut documents = self.documents.lock().map_err(|_| {
            memory_error(
                path.virtual_path().unwrap_or_else(|_| valid_memory_path()),
                FilesystemOperation::WriteFile,
                "memory document repository lock poisoned",
            )
        })?;
        let existing = documents
            .keys()
            .filter(|document| document.scope() == path.scope())
            .cloned()
            .collect::<Vec<_>>();
        ensure_document_path_does_not_conflict(path, &existing, FilesystemOperation::WriteFile)?;
        documents.insert(path.clone(), bytes.to_vec());
        Ok(())
    }

    async fn list_documents(
        &self,
        scope: &MemoryDocumentScope,
    ) -> Result<Vec<MemoryDocumentPath>, FilesystemError> {
        let documents = self.documents.lock().map_err(|_| {
            memory_error(
                scope
                    .virtual_prefix()
                    .unwrap_or_else(|_| valid_memory_path()),
                FilesystemOperation::ListDir,
                "memory document repository lock poisoned",
            )
        })?;
        Ok(documents
            .keys()
            .filter(|path| path.scope() == scope)
            .cloned()
            .collect())
    }
}

/// [`RootFilesystem`] backend exposing DB-backed memory documents as virtual files.
pub struct MemoryDocumentFilesystem {
    repository: Arc<dyn MemoryDocumentRepository>,
    indexer: Option<Arc<dyn MemoryDocumentIndexer>>,
}

impl MemoryDocumentFilesystem {
    pub fn new<R>(repository: Arc<R>) -> Self
    where
        R: MemoryDocumentRepository + 'static,
    {
        Self {
            repository,
            indexer: None,
        }
    }

    pub fn with_indexer<I>(mut self, indexer: Arc<I>) -> Self
    where
        I: MemoryDocumentIndexer + 'static,
    {
        self.indexer = Some(indexer);
        self
    }

    fn parse_file_path(
        &self,
        path: &VirtualPath,
        operation: FilesystemOperation,
    ) -> Result<MemoryDocumentPath, FilesystemError> {
        let parsed = ParsedMemoryPath::from_virtual_path(path, operation)?;
        let Some(relative_path) = parsed.relative_path else {
            return Err(memory_error(
                path.clone(),
                operation,
                "memory document path must include a file path after project id",
            ));
        };
        Ok(MemoryDocumentPath {
            scope: parsed.scope,
            relative_path,
        })
    }

    async fn list_for_scope(
        &self,
        scope: &MemoryDocumentScope,
    ) -> Result<Vec<MemoryDocumentPath>, FilesystemError> {
        self.repository.list_documents(scope).await
    }

    async fn ensure_write_path_does_not_conflict(
        &self,
        path: &MemoryDocumentPath,
    ) -> Result<(), FilesystemError> {
        let documents = self.list_for_scope(path.scope()).await?;
        ensure_document_path_does_not_conflict(path, &documents, FilesystemOperation::WriteFile)
    }
}

#[async_trait]
impl RootFilesystem for MemoryDocumentFilesystem {
    async fn read_file(&self, path: &VirtualPath) -> Result<Vec<u8>, FilesystemError> {
        let document_path = self.parse_file_path(path, FilesystemOperation::ReadFile)?;
        self.repository
            .read_document(&document_path)
            .await?
            .ok_or_else(|| memory_not_found(path.clone(), FilesystemOperation::ReadFile))
    }

    async fn write_file(&self, path: &VirtualPath, bytes: &[u8]) -> Result<(), FilesystemError> {
        let document_path = self.parse_file_path(path, FilesystemOperation::WriteFile)?;
        self.ensure_write_path_does_not_conflict(&document_path)
            .await?;
        self.repository
            .write_document(&document_path, bytes)
            .await?;
        if let Some(indexer) = &self.indexer {
            // The repository is the source of truth. Indexing is derived state,
            // so an index refresh failure must not make a committed write look
            // like it failed to the filesystem caller.
            let _ = indexer.reindex_document(&document_path).await;
        }
        Ok(())
    }

    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        let parsed = ParsedMemoryPath::from_virtual_path(path, FilesystemOperation::ListDir)?;
        let documents = self.list_for_scope(&parsed.scope).await?;
        if let Some(relative_path) = parsed.relative_path.as_deref()
            && documents
                .iter()
                .any(|document| document.relative_path() == relative_path)
        {
            return Err(memory_error(
                path.clone(),
                FilesystemOperation::ListDir,
                "not a directory",
            ));
        }
        memory_direct_children(path, parsed.relative_path.as_deref(), documents)
    }

    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        let parsed = ParsedMemoryPath::from_virtual_path(path, FilesystemOperation::Stat)?;
        let documents = self.list_for_scope(&parsed.scope).await?;
        if let Some(relative_path) = parsed.relative_path.as_deref() {
            if let Some(document) = documents
                .iter()
                .find(|document| document.relative_path() == relative_path)
            {
                let len = self
                    .repository
                    .read_document(document)
                    .await?
                    .map(|bytes| bytes.len() as u64)
                    .unwrap_or(0);
                return Ok(FileStat {
                    path: path.clone(),
                    file_type: FileType::File,
                    len,
                });
            }
            let directory_prefix = format!("{relative_path}/");
            if documents
                .iter()
                .any(|document| document.relative_path().starts_with(&directory_prefix))
            {
                return Ok(FileStat {
                    path: path.clone(),
                    file_type: FileType::Directory,
                    len: 0,
                });
            }
            return Err(memory_not_found(path.clone(), FilesystemOperation::Stat));
        }

        if documents.is_empty() {
            return Err(memory_not_found(path.clone(), FilesystemOperation::Stat));
        }
        Ok(FileStat {
            path: path.clone(),
            file_type: FileType::Directory,
            len: 0,
        })
    }
}

/// libSQL repository adapter for the existing `memory_documents` table shape.
#[cfg(feature = "libsql")]
pub struct LibSqlMemoryDocumentRepository {
    db: Arc<libsql::Database>,
}

#[cfg(feature = "libsql")]
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

#[cfg(feature = "libsql")]
async fn libsql_list_documents_for_scope(
    conn: &libsql::Connection,
    scope: &MemoryDocumentScope,
    virtual_path: &VirtualPath,
    operation: FilesystemOperation,
) -> Result<Vec<MemoryDocumentPath>, FilesystemError> {
    let owner_key = scoped_memory_owner_key(scope);
    let mut documents = Vec::new();
    let mut rows = conn
        .query(
            "SELECT path FROM memory_documents WHERE user_id = ?1 AND agent_id IS NULL ORDER BY path",
            libsql::params![owner_key],
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

#[cfg(feature = "libsql")]
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
        let db_path = db_path_for_memory_document(path);
        let mut rows = conn
            .query(
                "SELECT content FROM memory_documents WHERE user_id = ?1 AND agent_id IS NULL AND path = ?2",
                libsql::params![owner_key, db_path],
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
        conn.execute("BEGIN IMMEDIATE", ()).await.map_err(|error| {
            memory_error(
                virtual_path.clone(),
                FilesystemOperation::WriteFile,
                error.to_string(),
            )
        })?;

        let result = async {
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

            let owner_key = scoped_memory_owner_key(path.scope());
            let db_path = db_path_for_memory_document(path);
            conn.execute(
                r#"
                INSERT INTO memory_documents (id, user_id, agent_id, path, content, metadata)
                VALUES (?1, ?2, NULL, ?3, ?4, '{}')
                ON CONFLICT(user_id, path) WHERE agent_id IS NULL DO UPDATE SET
                    content = excluded.content,
                    updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
                "#,
                libsql::params![
                    uuid::Uuid::new_v4().to_string(),
                    owner_key,
                    db_path,
                    content
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
            Ok(())
        }
        .await;

        match result {
            Ok(()) => {
                conn.execute("COMMIT", ()).await.map_err(|error| {
                    memory_error(
                        virtual_path,
                        FilesystemOperation::WriteFile,
                        error.to_string(),
                    )
                })?;
                Ok(())
            }
            Err(error) => {
                let _ = conn.execute("ROLLBACK", ()).await;
                Err(error)
            }
        }
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
}

#[cfg(feature = "libsql")]
const LIBSQL_MEMORY_DOCUMENTS_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS memory_documents (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL,
    agent_id TEXT,
    path TEXT NOT NULL,
    content TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    metadata TEXT NOT NULL DEFAULT '{}'
);
CREATE INDEX IF NOT EXISTS idx_memory_documents_user ON memory_documents(user_id);
CREATE INDEX IF NOT EXISTS idx_memory_documents_path ON memory_documents(user_id, path);
CREATE UNIQUE INDEX IF NOT EXISTS idx_memory_documents_reborn_document
    ON memory_documents(user_id, path)
    WHERE agent_id IS NULL;
"#;

/// PostgreSQL repository adapter for the existing `memory_documents` table shape.
#[cfg(feature = "postgres")]
pub struct PostgresMemoryDocumentRepository {
    pool: deadpool_postgres::Pool,
}

#[cfg(feature = "postgres")]
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

#[cfg(feature = "postgres")]
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
    let rows = client
        .query(
            "SELECT path FROM memory_documents WHERE user_id = $1 AND agent_id IS NULL ORDER BY path",
            &[&owner_key],
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

#[cfg(feature = "postgres")]
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
        let db_path = db_path_for_memory_document(path);
        let row = client
            .query_opt(
                "SELECT content FROM memory_documents WHERE user_id = $1 AND agent_id IS NULL AND path = $2",
                &[&owner_key, &db_path],
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
        let tx = client.transaction().await.map_err(|error| {
            memory_error(
                virtual_path.clone(),
                FilesystemOperation::WriteFile,
                error.to_string(),
            )
        })?;
        tx.batch_execute("LOCK TABLE memory_documents IN SHARE ROW EXCLUSIVE MODE")
            .await
            .map_err(|error| {
                memory_error(
                    virtual_path.clone(),
                    FilesystemOperation::WriteFile,
                    error.to_string(),
                )
            })?;
        let documents = postgres_list_documents_for_scope(
            &tx,
            path.scope(),
            &virtual_path,
            FilesystemOperation::WriteFile,
        )
        .await?;
        ensure_document_path_does_not_conflict(path, &documents, FilesystemOperation::WriteFile)?;

        let owner_key = scoped_memory_owner_key(path.scope());
        let db_path = db_path_for_memory_document(path);
        tx.execute(
            r#"
            INSERT INTO memory_documents (user_id, agent_id, path, content, metadata)
            VALUES ($1, NULL, $2, $3, '{}'::jsonb)
            ON CONFLICT (user_id, path) WHERE agent_id IS NULL DO UPDATE SET
                content = EXCLUDED.content,
                updated_at = NOW()
            "#,
            &[&owner_key, &db_path, &content],
        )
        .await
        .map_err(|error| {
            memory_error(
                virtual_path.clone(),
                FilesystemOperation::WriteFile,
                error.to_string(),
            )
        })?;
        tx.commit().await.map_err(|error| {
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
}

#[cfg(feature = "postgres")]
const POSTGRES_MEMORY_DOCUMENTS_SCHEMA: &str = r#"
CREATE EXTENSION IF NOT EXISTS pgcrypto;
CREATE TABLE IF NOT EXISTS memory_documents (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id TEXT NOT NULL,
    agent_id UUID,
    path TEXT NOT NULL,
    content TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    metadata JSONB NOT NULL DEFAULT '{}'
);
CREATE INDEX IF NOT EXISTS idx_memory_documents_user ON memory_documents(user_id);
CREATE INDEX IF NOT EXISTS idx_memory_documents_path ON memory_documents(user_id, path);
CREATE UNIQUE INDEX IF NOT EXISTS idx_memory_documents_reborn_document
    ON memory_documents(user_id, path)
    WHERE agent_id IS NULL;
"#;

#[cfg(any(feature = "libsql", feature = "postgres"))]
fn scoped_memory_owner_key(scope: &MemoryDocumentScope) -> String {
    format!(
        "tenant:{}:user:{}:project:{}",
        scope.tenant_id(),
        scope.user_id(),
        scope.project_id().unwrap_or("_none")
    )
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
fn db_path_for_memory_document(path: &MemoryDocumentPath) -> String {
    path.relative_path().to_string()
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
fn memory_document_from_db_path(
    scope: &MemoryDocumentScope,
    db_path: &str,
) -> Option<MemoryDocumentPath> {
    validated_memory_relative_path(db_path.to_string())
        .ok()
        .map(|relative_path| MemoryDocumentPath {
            scope: scope.clone(),
            relative_path,
        })
}

fn ensure_document_path_does_not_conflict(
    path: &MemoryDocumentPath,
    documents: &[MemoryDocumentPath],
    operation: FilesystemOperation,
) -> Result<(), FilesystemError> {
    let relative_path = path.relative_path();
    let descendant_prefix = format!("{relative_path}/");
    if documents
        .iter()
        .any(|document| document.relative_path().starts_with(&descendant_prefix))
    {
        return Err(memory_error(
            path.virtual_path().unwrap_or_else(|_| valid_memory_path()),
            operation,
            "memory document path conflicts with an existing directory",
        ));
    }

    let segments: Vec<&str> = relative_path.split('/').collect();
    for end in 1..segments.len() {
        let ancestor = segments[..end].join("/");
        if documents
            .iter()
            .any(|document| document.relative_path() == ancestor)
        {
            return Err(memory_error(
                path.virtual_path().unwrap_or_else(|_| valid_memory_path()),
                operation,
                "memory document path conflicts with an existing file ancestor",
            ));
        }
    }

    Ok(())
}

fn memory_direct_children(
    parent: &VirtualPath,
    prefix: Option<&str>,
    documents: Vec<MemoryDocumentPath>,
) -> Result<Vec<DirEntry>, FilesystemError> {
    let mut entries = BTreeMap::<String, FileType>::new();
    let directory_prefix = prefix.map(|prefix| format!("{}/", prefix.trim_end_matches('/')));
    for document in documents {
        let tail = match directory_prefix.as_deref() {
            Some(prefix) => {
                let Some(tail) = document.relative_path().strip_prefix(prefix) else {
                    continue;
                };
                tail
            }
            None => document.relative_path(),
        };
        if tail.is_empty() {
            continue;
        }
        let (name, file_type) = if let Some((directory, _rest)) = tail.split_once('/') {
            (directory.to_string(), FileType::Directory)
        } else {
            (tail.to_string(), FileType::File)
        };
        entries
            .entry(name)
            .and_modify(|existing| {
                if file_type == FileType::Directory {
                    *existing = FileType::Directory;
                }
            })
            .or_insert(file_type);
    }

    if entries.is_empty() {
        return Err(memory_not_found(
            parent.clone(),
            FilesystemOperation::ListDir,
        ));
    }

    entries
        .into_iter()
        .map(|(name, file_type)| {
            Ok(DirEntry {
                path: VirtualPath::new(format!(
                    "{}/{}",
                    parent.as_str().trim_end_matches('/'),
                    name
                ))?,
                name,
                file_type,
            })
        })
        .collect()
}

fn validated_memory_segment(kind: &'static str, value: String) -> Result<String, HostApiError> {
    if value.trim().is_empty() {
        return Err(HostApiError::InvalidId {
            kind,
            value,
            reason: "segment must not be empty".to_string(),
        });
    }
    if value == "." || value == ".." {
        return Err(HostApiError::InvalidId {
            kind,
            value,
            reason: "dot segments are not allowed".to_string(),
        });
    }
    if value.contains(':') {
        return Err(HostApiError::InvalidId {
            kind,
            value,
            reason: "colon is reserved for memory owner key encoding".to_string(),
        });
    }
    if value.contains('/')
        || value.contains('\\')
        || value.contains('\0')
        || value.chars().any(char::is_control)
    {
        return Err(HostApiError::InvalidId {
            kind,
            value,
            reason: "segment must not contain path separators or control characters".to_string(),
        });
    }
    Ok(value)
}

fn validated_memory_relative_path(value: String) -> Result<String, HostApiError> {
    if value.trim().is_empty() {
        return Err(HostApiError::InvalidPath {
            value,
            reason: "memory document path must not be empty".to_string(),
        });
    }
    if value.starts_with('/') || value.contains('\\') || value.contains('\0') {
        return Err(HostApiError::InvalidPath {
            value,
            reason: "memory document path must be relative and use forward slashes".to_string(),
        });
    }
    if value.chars().any(char::is_control) {
        return Err(HostApiError::InvalidPath {
            value,
            reason: "memory document path must not contain control characters".to_string(),
        });
    }
    if value
        .split('/')
        .any(|segment| segment.is_empty() || segment == "." || segment == "..")
    {
        return Err(HostApiError::InvalidPath {
            value,
            reason: "memory document path must not contain empty, '.', or '..' segments"
                .to_string(),
        });
    }
    Ok(value)
}

fn memory_not_found(path: VirtualPath, operation: FilesystemOperation) -> FilesystemError {
    memory_error(path, operation, "not found")
}

fn memory_error(
    path: VirtualPath,
    operation: FilesystemOperation,
    reason: impl Into<String>,
) -> FilesystemError {
    FilesystemError::Backend {
        path,
        operation,
        reason: reason.into(),
    }
}

fn valid_memory_path() -> VirtualPath {
    VirtualPath::new("/memory").unwrap_or_else(|_| unreachable!("literal virtual path is valid"))
}
