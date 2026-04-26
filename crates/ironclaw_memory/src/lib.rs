//! Memory document filesystem adapters for IronClaw Reborn.
//!
//! This crate owns memory-specific path grammar and repository seams. The
//! generic filesystem crate owns only virtual path authority, scoped mounts,
//! backend cataloging, and backend routing.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ironclaw_filesystem::{
    DirEntry, FileStat, FileType, FilesystemError, FilesystemOperation, RootFilesystem,
};
use ironclaw_host_api::{HostApiError, VirtualPath};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

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

/// Name of the folder-level configuration document.
pub const CONFIG_FILE_NAME: &str = ".config";

/// Typed overlay for memory document metadata.
///
/// Ported from the current workspace metadata model. Unknown fields are
/// preserved for forward compatibility.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct DocumentMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skip_indexing: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skip_versioning: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hygiene: Option<HygieneMetadata>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<serde_json::Value>,

    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl DocumentMetadata {
    pub fn from_value(value: &serde_json::Value) -> Self {
        serde_json::from_value(value.clone()).unwrap_or_default()
    }

    pub fn to_value(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_else(|_| serde_json::json!({}))
    }

    pub fn merge(base: &serde_json::Value, overlay: &serde_json::Value) -> serde_json::Value {
        let mut merged = match base {
            serde_json::Value::Object(map) => map.clone(),
            _ => serde_json::Map::new(),
        };
        if let serde_json::Value::Object(over) = overlay {
            for (key, value) in over {
                merged.insert(key.clone(), value.clone());
            }
        }
        serde_json::Value::Object(merged)
    }
}

/// Hygiene metadata preserved from the current workspace metadata model.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HygieneMetadata {
    pub enabled: bool,
    #[serde(default = "default_retention_days")]
    pub retention_days: u32,
}

fn default_retention_days() -> u32 {
    30
}

/// Options resolved by the memory backend before persisting a document write.
#[derive(Debug, Clone, Default)]
pub struct MemoryWriteOptions {
    pub metadata: DocumentMetadata,
    pub changed_by: Option<String>,
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

    async fn write_document_with_options(
        &self,
        path: &MemoryDocumentPath,
        bytes: &[u8],
        options: &MemoryWriteOptions,
    ) -> Result<(), FilesystemError> {
        let _ = options;
        self.write_document(path, bytes).await
    }

    async fn read_document_metadata(
        &self,
        path: &MemoryDocumentPath,
    ) -> Result<Option<serde_json::Value>, FilesystemError> {
        let _ = path;
        Ok(None)
    }

    async fn write_document_metadata(
        &self,
        path: &MemoryDocumentPath,
        metadata: &serde_json::Value,
    ) -> Result<(), FilesystemError> {
        let _ = (path, metadata);
        Ok(())
    }

    async fn list_documents(
        &self,
        scope: &MemoryDocumentScope,
    ) -> Result<Vec<MemoryDocumentPath>, FilesystemError>;

    async fn search_documents(
        &self,
        scope: &MemoryDocumentScope,
        request: &MemorySearchRequest,
    ) -> Result<Vec<MemorySearchResult>, FilesystemError> {
        let _ = request;
        Err(memory_backend_unsupported(
            scope,
            FilesystemOperation::ReadFile,
            "memory backend does not support search",
        ))
    }
}

/// Hook invoked after successful memory document writes so derived state can be refreshed.
#[async_trait]
pub trait MemoryDocumentIndexer: Send + Sync {
    async fn reindex_document(&self, path: &MemoryDocumentPath) -> Result<(), FilesystemError>;
}

async fn resolve_document_metadata<R>(
    repository: &R,
    path: &MemoryDocumentPath,
) -> Result<DocumentMetadata, FilesystemError>
where
    R: MemoryDocumentRepository + ?Sized,
{
    let doc_meta = repository
        .read_document_metadata(path)
        .await?
        .unwrap_or_else(|| serde_json::json!({}));
    let configs = repository.list_documents(path.scope()).await?;
    let mut config_metadata = HashMap::<String, serde_json::Value>::new();
    for config_path in configs
        .into_iter()
        .filter(|candidate| is_config_path(candidate.relative_path()))
    {
        if let Some(metadata) = repository.read_document_metadata(&config_path).await? {
            config_metadata.insert(config_path.relative_path().to_string(), metadata);
        }
    }
    let base = find_nearest_config(path.relative_path(), &config_metadata)
        .unwrap_or_else(|| serde_json::json!({}));
    Ok(DocumentMetadata::from_value(&DocumentMetadata::merge(
        &base, &doc_meta,
    )))
}

fn is_config_path(path: &str) -> bool {
    path.rsplit('/').next().unwrap_or(path) == CONFIG_FILE_NAME
}

fn find_nearest_config(
    path: &str,
    configs: &HashMap<String, serde_json::Value>,
) -> Option<serde_json::Value> {
    let mut current = path;
    while let Some(slash_pos) = current.rfind('/') {
        let parent = &current[..slash_pos];
        let config_path = format!("{parent}/{CONFIG_FILE_NAME}");
        if let Some(metadata) = configs.get(config_path.as_str()) {
            return Some(metadata.clone());
        }
        current = parent;
    }
    configs.get(CONFIG_FILE_NAME).cloned()
}

fn validate_content_against_schema(
    path: &MemoryDocumentPath,
    content: &str,
    schema: &serde_json::Value,
) -> Result<(), FilesystemError> {
    if schema.is_null() {
        return Ok(());
    }
    let instance: serde_json::Value = serde_json::from_str(content).map_err(|error| {
        memory_error(
            path.virtual_path().unwrap_or_else(|_| valid_memory_path()),
            FilesystemOperation::WriteFile,
            format!("schema validation failed: content is not valid JSON: {error}"),
        )
    })?;
    let validator = jsonschema::validator_for(schema).map_err(|error| {
        memory_error(
            path.virtual_path().unwrap_or_else(|_| valid_memory_path()),
            FilesystemOperation::WriteFile,
            format!("schema validation failed: invalid schema: {error}"),
        )
    })?;
    let errors = validator
        .iter_errors(&instance)
        .map(|error| error.to_string())
        .collect::<Vec<_>>();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(memory_error(
            path.virtual_path().unwrap_or_else(|_| valid_memory_path()),
            FilesystemOperation::WriteFile,
            format!("schema validation failed: {}", errors.join("; ")),
        ))
    }
}

/// Declared behavior supported by a memory backend.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemoryBackendCapabilities {
    pub file_documents: bool,
    pub metadata: bool,
    pub versioning: bool,
    pub full_text_search: bool,
    pub vector_search: bool,
    pub embeddings: bool,
    pub graph_memory: bool,
    pub delete: bool,
    pub transactions: bool,
}

/// Host-resolved scoped context passed to memory backends.
///
/// Backends receive this context after the host has parsed and authorized the
/// virtual path. They must not infer broader tenant/user/project authority from
/// their own configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryContext {
    scope: MemoryDocumentScope,
    invocation_id: Option<String>,
}

impl MemoryContext {
    pub fn new(scope: MemoryDocumentScope) -> Self {
        Self {
            scope,
            invocation_id: None,
        }
    }

    pub fn with_invocation_id(mut self, invocation_id: impl Into<String>) -> Self {
        self.invocation_id = Some(invocation_id.into());
        self
    }

    pub fn scope(&self) -> &MemoryDocumentScope {
        &self.scope
    }

    pub fn invocation_id(&self) -> Option<&str> {
        self.invocation_id.as_deref()
    }
}

/// Search request passed to memory backends that expose search APIs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemorySearchRequest {
    query: String,
    limit: usize,
    full_text: bool,
    vector: bool,
}

impl MemorySearchRequest {
    pub fn new(query: impl Into<String>) -> Result<Self, HostApiError> {
        let query = query.into();
        if query.trim().is_empty() {
            return Err(HostApiError::InvalidId {
                kind: "memory search query",
                value: query,
                reason: "query must not be empty".to_string(),
            });
        }
        Ok(Self {
            query,
            limit: 20,
            full_text: true,
            vector: true,
        })
    }

    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = limit.max(1);
        self
    }

    pub fn with_full_text(mut self, enabled: bool) -> Self {
        self.full_text = enabled;
        self
    }

    pub fn with_vector(mut self, enabled: bool) -> Self {
        self.vector = enabled;
        self
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn limit(&self) -> usize {
        self.limit
    }

    pub fn full_text(&self) -> bool {
        self.full_text
    }

    pub fn vector(&self) -> bool {
        self.vector
    }
}

/// Search result returned by memory backends that expose search APIs.
#[derive(Debug, Clone, PartialEq)]
pub struct MemorySearchResult {
    pub path: MemoryDocumentPath,
    pub score: f32,
    pub snippet: String,
}

/// Pluggable memory backend contract.
///
/// The host owns authority, scope parsing, and mount exposure. Backends own
/// storage/search behavior inside the already-resolved [`MemoryContext`].
#[async_trait]
pub trait MemoryBackend: Send + Sync {
    fn capabilities(&self) -> MemoryBackendCapabilities;

    async fn read_document(
        &self,
        context: &MemoryContext,
        path: &MemoryDocumentPath,
    ) -> Result<Option<Vec<u8>>, FilesystemError> {
        let _ = (context, path);
        Err(memory_backend_unsupported(
            context.scope(),
            FilesystemOperation::ReadFile,
            "memory backend does not support file documents",
        ))
    }

    async fn write_document(
        &self,
        context: &MemoryContext,
        path: &MemoryDocumentPath,
        bytes: &[u8],
    ) -> Result<(), FilesystemError> {
        let _ = (path, bytes);
        Err(memory_backend_unsupported(
            context.scope(),
            FilesystemOperation::WriteFile,
            "memory backend does not support file documents",
        ))
    }

    async fn list_documents(
        &self,
        context: &MemoryContext,
        scope: &MemoryDocumentScope,
    ) -> Result<Vec<MemoryDocumentPath>, FilesystemError> {
        let _ = scope;
        Err(memory_backend_unsupported(
            context.scope(),
            FilesystemOperation::ListDir,
            "memory backend does not support file documents",
        ))
    }

    async fn search(
        &self,
        context: &MemoryContext,
        request: MemorySearchRequest,
    ) -> Result<Vec<MemorySearchResult>, FilesystemError> {
        let _ = request;
        Err(memory_backend_unsupported(
            context.scope(),
            FilesystemOperation::ReadFile,
            "memory backend does not support search",
        ))
    }
}

/// [`RootFilesystem`] adapter exposing any [`MemoryBackend`] as `/memory` files.
pub struct MemoryBackendFilesystemAdapter {
    backend: Arc<dyn MemoryBackend>,
}

impl MemoryBackendFilesystemAdapter {
    pub fn new<B>(backend: Arc<B>) -> Self
    where
        B: MemoryBackend + 'static,
    {
        let backend: Arc<dyn MemoryBackend> = backend;
        Self { backend }
    }

    pub fn from_dyn(backend: Arc<dyn MemoryBackend>) -> Self {
        Self { backend }
    }

    fn ensure_file_documents(
        &self,
        path: &VirtualPath,
        operation: FilesystemOperation,
    ) -> Result<(), FilesystemError> {
        if self.backend.capabilities().file_documents {
            Ok(())
        } else {
            Err(memory_error(
                path.clone(),
                operation,
                "memory backend does not support file documents",
            ))
        }
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
}

#[async_trait]
impl RootFilesystem for MemoryBackendFilesystemAdapter {
    async fn read_file(&self, path: &VirtualPath) -> Result<Vec<u8>, FilesystemError> {
        self.ensure_file_documents(path, FilesystemOperation::ReadFile)?;
        let document_path = self.parse_file_path(path, FilesystemOperation::ReadFile)?;
        let context = MemoryContext::new(document_path.scope().clone());
        self.backend
            .read_document(&context, &document_path)
            .await?
            .ok_or_else(|| memory_not_found(path.clone(), FilesystemOperation::ReadFile))
    }

    async fn write_file(&self, path: &VirtualPath, bytes: &[u8]) -> Result<(), FilesystemError> {
        self.ensure_file_documents(path, FilesystemOperation::WriteFile)?;
        let document_path = self.parse_file_path(path, FilesystemOperation::WriteFile)?;
        let context = MemoryContext::new(document_path.scope().clone());
        self.backend
            .write_document(&context, &document_path, bytes)
            .await
    }

    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        self.ensure_file_documents(path, FilesystemOperation::ListDir)?;
        let parsed = ParsedMemoryPath::from_virtual_path(path, FilesystemOperation::ListDir)?;
        let context = MemoryContext::new(parsed.scope.clone());
        let documents = self.backend.list_documents(&context, &parsed.scope).await?;
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
        self.ensure_file_documents(path, FilesystemOperation::Stat)?;
        let parsed = ParsedMemoryPath::from_virtual_path(path, FilesystemOperation::Stat)?;
        let context = MemoryContext::new(parsed.scope.clone());
        let documents = self.backend.list_documents(&context, &parsed.scope).await?;
        if let Some(relative_path) = parsed.relative_path.as_deref() {
            if let Some(document) = documents
                .iter()
                .find(|document| document.relative_path() == relative_path)
            {
                let len = self
                    .backend
                    .read_document(&context, document)
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

/// Memory backend wrapper for existing repository/indexer implementations.
pub struct RepositoryMemoryBackend<R> {
    repository: Arc<R>,
    indexer: Option<Arc<dyn MemoryDocumentIndexer>>,
    capabilities: MemoryBackendCapabilities,
}

impl<R> RepositoryMemoryBackend<R>
where
    R: MemoryDocumentRepository + 'static,
{
    pub fn new(repository: Arc<R>) -> Self {
        Self {
            repository,
            indexer: None,
            capabilities: MemoryBackendCapabilities {
                file_documents: true,
                metadata: true,
                versioning: true,
                ..MemoryBackendCapabilities::default()
            },
        }
    }

    pub fn with_indexer<I>(mut self, indexer: Arc<I>) -> Self
    where
        I: MemoryDocumentIndexer + 'static,
    {
        self.indexer = Some(indexer);
        self
    }

    pub fn with_capabilities(mut self, capabilities: MemoryBackendCapabilities) -> Self {
        self.capabilities = capabilities;
        self
    }
}

#[async_trait]
impl<R> MemoryBackend for RepositoryMemoryBackend<R>
where
    R: MemoryDocumentRepository + 'static,
{
    fn capabilities(&self) -> MemoryBackendCapabilities {
        self.capabilities.clone()
    }

    async fn read_document(
        &self,
        _context: &MemoryContext,
        path: &MemoryDocumentPath,
    ) -> Result<Option<Vec<u8>>, FilesystemError> {
        self.repository.read_document(path).await
    }

    async fn write_document(
        &self,
        _context: &MemoryContext,
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
        let metadata = resolve_document_metadata(self.repository.as_ref(), path).await?;
        if let Some(schema) = &metadata.schema {
            validate_content_against_schema(path, content, schema)?;
        }
        let options = MemoryWriteOptions {
            metadata,
            changed_by: Some(scoped_memory_owner_key(path.scope())),
        };
        self.repository
            .write_document_with_options(path, bytes, &options)
            .await?;
        if let Some(indexer) = &self.indexer {
            indexer.reindex_document(path).await?;
        }
        Ok(())
    }

    async fn list_documents(
        &self,
        _context: &MemoryContext,
        scope: &MemoryDocumentScope,
    ) -> Result<Vec<MemoryDocumentPath>, FilesystemError> {
        self.repository.list_documents(scope).await
    }

    async fn search(
        &self,
        context: &MemoryContext,
        request: MemorySearchRequest,
    ) -> Result<Vec<MemorySearchResult>, FilesystemError> {
        if !self.capabilities.full_text_search && !self.capabilities.vector_search {
            return Err(memory_backend_unsupported(
                context.scope(),
                FilesystemOperation::ReadFile,
                "memory backend does not support search",
            ));
        }
        self.repository
            .search_documents(context.scope(), &request)
            .await
    }
}

/// Repository operations used by the memory indexer to keep chunk/search rows in sync.
#[async_trait]
pub trait MemoryDocumentIndexRepository: Send + Sync {
    async fn replace_document_chunks_if_current(
        &self,
        path: &MemoryDocumentPath,
        expected_content_hash: &str,
        chunks: &[MemoryChunkWrite],
    ) -> Result<(), FilesystemError>;

    async fn delete_document_chunks(
        &self,
        path: &MemoryDocumentPath,
    ) -> Result<(), FilesystemError>;
}

/// Configuration for document chunking.
///
/// Ported from the current workspace chunker so Reborn memory indexing preserves
/// existing search recall behavior.
#[derive(Debug, Clone)]
pub struct ChunkConfig {
    pub chunk_size: usize,
    pub overlap_percent: f32,
    pub min_chunk_size: usize,
}

impl Default for ChunkConfig {
    fn default() -> Self {
        Self {
            chunk_size: 800,
            overlap_percent: 0.15,
            min_chunk_size: 50,
        }
    }
}

impl ChunkConfig {
    pub fn with_chunk_size(mut self, size: usize) -> Self {
        self.chunk_size = size;
        self
    }

    pub fn with_overlap(mut self, percent: f32) -> Self {
        self.overlap_percent = percent.clamp(0.0, 0.5);
        self
    }

    fn overlap_size(&self) -> usize {
        (self.chunk_size as f32 * self.overlap_percent) as usize
    }

    fn step_size(&self) -> usize {
        self.chunk_size.saturating_sub(self.overlap_size())
    }
}

/// A new chunk to insert for a document.
#[derive(Debug, Clone, PartialEq)]
pub struct MemoryChunkWrite {
    pub content: String,
    pub embedding: Option<Vec<f32>>,
}

/// Split a document into overlapping chunks using current workspace semantics.
pub fn chunk_document(content: &str, config: ChunkConfig) -> Vec<String> {
    if content.is_empty() {
        return Vec::new();
    }

    let words: Vec<&str> = content.split_whitespace().collect();
    if words.is_empty() {
        return Vec::new();
    }

    if words.len() <= config.chunk_size {
        return vec![content.to_string()];
    }

    let step = config.step_size();
    let mut chunks = Vec::new();
    let mut start = 0;

    while start < words.len() {
        let end = (start + config.chunk_size).min(words.len());
        let chunk_words = &words[start..end];

        if chunk_words.len() < config.min_chunk_size
            && let Some(last) = chunks.pop()
        {
            let combined = format!("{} {}", last, chunk_words.join(" "));
            chunks.push(combined);
            break;
        }

        chunks.push(chunk_words.join(" "));
        start += step;

        if start + config.min_chunk_size >= words.len() && end == words.len() {
            break;
        }
    }

    chunks
}

/// Compute a SHA-256 content hash using the current workspace format.
pub fn content_sha256(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("sha256:{:x}", hasher.finalize())
}

/// Memory document indexer that chunks documents and updates DB-backed chunk rows.
pub struct ChunkingMemoryDocumentIndexer<R> {
    repository: Arc<R>,
    chunk_config: ChunkConfig,
}

impl<R> ChunkingMemoryDocumentIndexer<R>
where
    R: MemoryDocumentRepository + MemoryDocumentIndexRepository + 'static,
{
    pub fn new(repository: Arc<R>) -> Self {
        Self {
            repository,
            chunk_config: ChunkConfig::default(),
        }
    }

    pub fn with_chunk_config(mut self, chunk_config: ChunkConfig) -> Self {
        self.chunk_config = chunk_config;
        self
    }
}

#[async_trait]
impl<R> MemoryDocumentIndexer for ChunkingMemoryDocumentIndexer<R>
where
    R: MemoryDocumentRepository + MemoryDocumentIndexRepository + 'static,
{
    async fn reindex_document(&self, path: &MemoryDocumentPath) -> Result<(), FilesystemError> {
        let Some(bytes) = self.repository.read_document(path).await? else {
            return Ok(());
        };
        let content = std::str::from_utf8(&bytes).map_err(|_| {
            memory_error(
                path.virtual_path().unwrap_or_else(|_| valid_memory_path()),
                FilesystemOperation::WriteFile,
                "memory document content must be UTF-8",
            )
        })?;
        let metadata = resolve_document_metadata(self.repository.as_ref(), path).await?;
        if metadata.skip_indexing == Some(true) {
            return self.repository.delete_document_chunks(path).await;
        }
        let content_hash_at_read = content_sha256(content);
        let chunks = chunk_document(content, self.chunk_config.clone())
            .into_iter()
            .map(|content| MemoryChunkWrite {
                content,
                embedding: None,
            })
            .collect::<Vec<_>>();
        if chunks.is_empty() {
            self.repository.delete_document_chunks(path).await
        } else {
            self.repository
                .replace_document_chunks_if_current(path, &content_hash_at_read, &chunks)
                .await
        }
    }
}

/// In-memory memory document repository for tests and examples.
#[derive(Default)]
pub struct InMemoryMemoryDocumentRepository {
    documents: Mutex<BTreeMap<MemoryDocumentPath, Vec<u8>>>,
    metadata: Mutex<BTreeMap<MemoryDocumentPath, serde_json::Value>>,
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
        documents.insert(path.clone(), bytes.to_vec());
        Ok(())
    }

    async fn read_document_metadata(
        &self,
        path: &MemoryDocumentPath,
    ) -> Result<Option<serde_json::Value>, FilesystemError> {
        let metadata = self.metadata.lock().map_err(|_| {
            memory_error(
                path.virtual_path().unwrap_or_else(|_| valid_memory_path()),
                FilesystemOperation::ReadFile,
                "memory document metadata repository lock poisoned",
            )
        })?;
        Ok(metadata.get(path).cloned())
    }

    async fn write_document_metadata(
        &self,
        path: &MemoryDocumentPath,
        metadata: &serde_json::Value,
    ) -> Result<(), FilesystemError> {
        let mut metadata_store = self.metadata.lock().map_err(|_| {
            memory_error(
                path.virtual_path().unwrap_or_else(|_| valid_memory_path()),
                FilesystemOperation::WriteFile,
                "memory document metadata repository lock poisoned",
            )
        })?;
        metadata_store.insert(path.clone(), metadata.clone());
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
        self.repository
            .write_document(&document_path, bytes)
            .await?;
        if let Some(indexer) = &self.indexer {
            indexer.reindex_document(&document_path).await?;
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
        let owner_key = scoped_memory_owner_key(path.scope());
        let db_path = db_path_for_memory_document(path);
        let existing = {
            let mut rows = conn
                .query(
                    "SELECT id, content FROM memory_documents WHERE user_id = ?1 AND agent_id IS NULL AND path = ?2",
                    libsql::params![owner_key.as_str(), db_path.as_str()],
                )
                .await
                .map_err(|error| {
                    memory_error(
                        virtual_path.clone(),
                        FilesystemOperation::WriteFile,
                        error.to_string(),
                    )
                })?;
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
                    let previous_content: String = row.get(1)?;
                    Ok::<_, libsql::Error>((id, previous_content))
                })
                .transpose()
                .map_err(|error| {
                    memory_error(
                        virtual_path.clone(),
                        FilesystemOperation::WriteFile,
                        error.to_string(),
                    )
                })?
        };

        if let Some((document_id, previous_content)) = existing {
            if previous_content != content && !previous_content.is_empty() {
                let _ = libsql_save_document_version(
                    &conn,
                    &virtual_path,
                    &document_id,
                    &previous_content,
                    Some(owner_key.as_str()),
                )
                .await;
            }
            conn.execute(
                "UPDATE memory_documents SET content = ?2, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?1",
                libsql::params![document_id, content],
            )
            .await
            .map_err(|error| memory_error(virtual_path, FilesystemOperation::WriteFile, error.to_string()))?;
        } else {
            conn.execute(
                r#"
                INSERT INTO memory_documents (id, user_id, agent_id, path, content, metadata)
                VALUES (?1, ?2, NULL, ?3, ?4, '{}')
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
                    virtual_path,
                    FilesystemOperation::WriteFile,
                    error.to_string(),
                )
            })?;
        }
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
        let conn = self
            .connect(virtual_path.clone(), FilesystemOperation::WriteFile)
            .await?;
        let owner_key = scoped_memory_owner_key(path.scope());
        let db_path = db_path_for_memory_document(path);
        let existing = {
            let mut rows = conn
                .query(
                    "SELECT id, content FROM memory_documents WHERE user_id = ?1 AND agent_id IS NULL AND path = ?2",
                    libsql::params![owner_key.as_str(), db_path.as_str()],
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
                    let previous_content: String = row.get(1)?;
                    Ok::<_, libsql::Error>((id, previous_content))
                })
                .transpose()
                .map_err(|error| {
                    memory_error(
                        virtual_path.clone(),
                        FilesystemOperation::WriteFile,
                        error.to_string(),
                    )
                })?
        };

        if let Some((document_id, previous_content)) = existing {
            if options.metadata.skip_versioning != Some(true)
                && previous_content != content
                && !previous_content.is_empty()
            {
                let _ = libsql_save_document_version(
                    &conn,
                    &virtual_path,
                    &document_id,
                    &previous_content,
                    options.changed_by.as_deref(),
                )
                .await;
            }
            conn.execute(
                "UPDATE memory_documents SET content = ?2, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?1",
                libsql::params![document_id, content],
            )
            .await
            .map_err(|error| memory_error(virtual_path, FilesystemOperation::WriteFile, error.to_string()))?;
        } else {
            conn.execute(
                r#"
                INSERT INTO memory_documents (id, user_id, agent_id, path, content, metadata)
                VALUES (?1, ?2, NULL, ?3, ?4, '{}')
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
                    virtual_path,
                    FilesystemOperation::WriteFile,
                    error.to_string(),
                )
            })?;
        }
        Ok(())
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
        let db_path = db_path_for_memory_document(path);
        let mut rows = conn
            .query(
                "SELECT metadata FROM memory_documents WHERE user_id = ?1 AND agent_id IS NULL AND path = ?2",
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
        let db_path = db_path_for_memory_document(path);
        let metadata = serde_json::to_string(metadata).map_err(|error| {
            memory_error(
                virtual_path.clone(),
                FilesystemOperation::WriteFile,
                error.to_string(),
            )
        })?;
        conn.execute(
            "UPDATE memory_documents SET metadata = ?3, updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE user_id = ?1 AND agent_id IS NULL AND path = ?2",
            libsql::params![owner_key, db_path, metadata],
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
        let owner_key = scoped_memory_owner_key(scope);
        let mut documents = Vec::new();
        if let Some(project_id) = scope.project_id() {
            let prefix = format!("projects/{project_id}/");
            let mut rows = conn
                .query(
                    "SELECT path FROM memory_documents WHERE user_id = ?1 AND agent_id IS NULL AND path LIKE ?2 ORDER BY path",
                    libsql::params![owner_key, format!("{prefix}%")],
                )
                .await
                .map_err(|error| memory_error(virtual_path.clone(), FilesystemOperation::ListDir, error.to_string()))?;
            while let Some(row) = rows.next().await.map_err(|error| {
                memory_error(
                    virtual_path.clone(),
                    FilesystemOperation::ListDir,
                    error.to_string(),
                )
            })? {
                let db_path: String = row.get(0).map_err(|error| {
                    memory_error(
                        virtual_path.clone(),
                        FilesystemOperation::ListDir,
                        error.to_string(),
                    )
                })?;
                if let Some(memory_path) = memory_document_from_db_path(scope, &db_path) {
                    documents.push(memory_path);
                }
            }
        } else {
            let mut rows = conn
                .query(
                    "SELECT path FROM memory_documents WHERE user_id = ?1 AND agent_id IS NULL AND path NOT LIKE 'projects/%' ORDER BY path",
                    libsql::params![owner_key],
                )
                .await
                .map_err(|error| memory_error(virtual_path.clone(), FilesystemOperation::ListDir, error.to_string()))?;
            while let Some(row) = rows.next().await.map_err(|error| {
                memory_error(
                    virtual_path.clone(),
                    FilesystemOperation::ListDir,
                    error.to_string(),
                )
            })? {
                let db_path: String = row.get(0).map_err(|error| {
                    memory_error(
                        virtual_path.clone(),
                        FilesystemOperation::ListDir,
                        error.to_string(),
                    )
                })?;
                if let Some(memory_path) = memory_document_from_db_path(scope, &db_path) {
                    documents.push(memory_path);
                }
            }
        }
        Ok(documents)
    }

    async fn search_documents(
        &self,
        scope: &MemoryDocumentScope,
        request: &MemorySearchRequest,
    ) -> Result<Vec<MemorySearchResult>, FilesystemError> {
        if !request.full_text() {
            return Ok(Vec::new());
        }
        let Some(fts_query) = escape_fts5_query(request.query()) else {
            return Ok(Vec::new());
        };
        let virtual_path = scope
            .virtual_prefix()
            .unwrap_or_else(|_| valid_memory_path());
        let conn = self
            .connect(virtual_path.clone(), FilesystemOperation::ReadFile)
            .await?;
        let owner_key = scoped_memory_owner_key(scope);
        let mut rows = conn
            .query(
                r#"
                SELECT d.path, c.content
                FROM memory_chunks_fts fts
                JOIN memory_chunks c ON c._rowid = fts.rowid
                JOIN memory_documents d ON d.id = c.document_id
                WHERE d.user_id = ?1 AND d.agent_id IS NULL
                  AND memory_chunks_fts MATCH ?2
                ORDER BY rank
                LIMIT ?3
                "#,
                libsql::params![owner_key, fts_query, request.limit() as i64],
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
            let db_path: String = row.get(0).map_err(|error| {
                memory_error(
                    virtual_path.clone(),
                    FilesystemOperation::ReadFile,
                    error.to_string(),
                )
            })?;
            let Some(path) = memory_document_from_db_path(scope, &db_path) else {
                continue;
            };
            let snippet: String = row.get(1).map_err(|error| {
                memory_error(
                    virtual_path.clone(),
                    FilesystemOperation::ReadFile,
                    error.to_string(),
                )
            })?;
            let score = 1.0 / (results.len() as f32 + 1.0);
            results.push(MemorySearchResult {
                path,
                score,
                snippet,
            });
        }
        Ok(results)
    }
}

#[cfg(feature = "libsql")]
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
        let Some((document_id, content)) =
            libsql_document_id_and_content(&conn, path, &virtual_path).await?
        else {
            return Ok(());
        };
        if content_sha256(&content) != expected_content_hash {
            return Ok(());
        }

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
            let embedding_blob = chunk.embedding.as_ref().map(|embedding| {
                libsql::Value::Blob(
                    embedding
                        .iter()
                        .flat_map(|value| value.to_le_bytes())
                        .collect(),
                )
            });
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

#[cfg(feature = "libsql")]
async fn libsql_document_id_and_content(
    conn: &libsql::Connection,
    path: &MemoryDocumentPath,
    virtual_path: &VirtualPath,
) -> Result<Option<(String, String)>, FilesystemError> {
    let owner_key = scoped_memory_owner_key(path.scope());
    let db_path = db_path_for_memory_document(path);
    let mut rows = conn
        .query(
            "SELECT id, content FROM memory_documents WHERE user_id = ?1 AND agent_id IS NULL AND path = ?2",
            libsql::params![owner_key, db_path],
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

#[cfg(feature = "libsql")]
async fn libsql_save_document_version(
    conn: &libsql::Connection,
    virtual_path: &VirtualPath,
    document_id: &str,
    content: &str,
    changed_by: Option<&str>,
) -> Result<i32, FilesystemError> {
    conn.execute("BEGIN IMMEDIATE", libsql::params![])
        .await
        .map_err(|error| {
            memory_error(
                virtual_path.clone(),
                FilesystemOperation::WriteFile,
                error.to_string(),
            )
        })?;
    let result = async {
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
                .map_err(|error| memory_error(virtual_path.clone(), FilesystemOperation::WriteFile, error.to_string()))?
                .ok_or_else(|| memory_error(virtual_path.clone(), FilesystemOperation::WriteFile, "missing version row"))?;
            row.get::<i64>(0)
                .map(|version| version as i32)
                .map_err(|error| memory_error(virtual_path.clone(), FilesystemOperation::WriteFile, error.to_string()))?
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
        .map_err(|error| memory_error(virtual_path.clone(), FilesystemOperation::WriteFile, error.to_string()))?;
        Ok::<i32, FilesystemError>(next_version)
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

#[cfg(feature = "libsql")]
fn escape_fts5_query(query: &str) -> Option<String> {
    let phrases = query
        .split_whitespace()
        .map(|token| format!("\"{}\"", token.replace('"', "\"\"")))
        .collect::<Vec<_>>();
    if phrases.is_empty() {
        None
    } else {
        Some(phrases.join(" "))
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
    metadata TEXT NOT NULL DEFAULT '{}',
    UNIQUE (user_id, agent_id, path)
);

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
        let client = self
            .client(virtual_path.clone(), FilesystemOperation::WriteFile)
            .await?;
        let owner_key = scoped_memory_owner_key(path.scope());
        let db_path = db_path_for_memory_document(path);
        let existing = client
            .query_opt(
                "SELECT id, content FROM memory_documents WHERE user_id = $1 AND agent_id IS NULL AND path = $2",
                &[&owner_key, &db_path],
            )
            .await
            .map_err(|error| memory_error(virtual_path.clone(), FilesystemOperation::WriteFile, error.to_string()))?;
        if let Some(row) = existing {
            let document_id: uuid::Uuid = row.get("id");
            let previous_content: String = row.get("content");
            if previous_content != content && !previous_content.is_empty() {
                let _ = postgres_save_document_version(
                    &client,
                    &virtual_path,
                    document_id,
                    &previous_content,
                    Some(owner_key.as_str()),
                )
                .await;
            }
            client
                .execute(
                    "UPDATE memory_documents SET content = $2, updated_at = NOW() WHERE id = $1",
                    &[&document_id, &content],
                )
                .await
                .map_err(|error| {
                    memory_error(
                        virtual_path,
                        FilesystemOperation::WriteFile,
                        error.to_string(),
                    )
                })?;
        } else {
            client
                .execute(
                    r#"
                    INSERT INTO memory_documents (user_id, agent_id, path, content, metadata)
                    VALUES ($1, NULL, $2, $3, '{}'::jsonb)
                    "#,
                    &[&owner_key, &db_path, &content],
                )
                .await
                .map_err(|error| {
                    memory_error(
                        virtual_path,
                        FilesystemOperation::WriteFile,
                        error.to_string(),
                    )
                })?;
        }
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
        let client = self
            .client(virtual_path.clone(), FilesystemOperation::WriteFile)
            .await?;
        let owner_key = scoped_memory_owner_key(path.scope());
        let db_path = db_path_for_memory_document(path);
        let existing = client
            .query_opt(
                "SELECT id, content FROM memory_documents WHERE user_id = $1 AND agent_id IS NULL AND path = $2",
                &[&owner_key, &db_path],
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
                let _ = postgres_save_document_version(
                    &client,
                    &virtual_path,
                    document_id,
                    &previous_content,
                    options.changed_by.as_deref(),
                )
                .await;
            }
            client
                .execute(
                    "UPDATE memory_documents SET content = $2, updated_at = NOW() WHERE id = $1",
                    &[&document_id, &content],
                )
                .await
                .map_err(|error| {
                    memory_error(
                        virtual_path,
                        FilesystemOperation::WriteFile,
                        error.to_string(),
                    )
                })?;
        } else {
            client
                .execute(
                    r#"
                    INSERT INTO memory_documents (user_id, agent_id, path, content, metadata)
                    VALUES ($1, NULL, $2, $3, '{}'::jsonb)
                    "#,
                    &[&owner_key, &db_path, &content],
                )
                .await
                .map_err(|error| {
                    memory_error(
                        virtual_path,
                        FilesystemOperation::WriteFile,
                        error.to_string(),
                    )
                })?;
        }
        Ok(())
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
        let db_path = db_path_for_memory_document(path);
        let row = client
            .query_opt(
                "SELECT metadata FROM memory_documents WHERE user_id = $1 AND agent_id IS NULL AND path = $2",
                &[&owner_key, &db_path],
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
        let db_path = db_path_for_memory_document(path);
        client
            .execute(
                "UPDATE memory_documents SET metadata = $3, updated_at = NOW() WHERE user_id = $1 AND agent_id IS NULL AND path = $2",
                &[&owner_key, &db_path, metadata],
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
        let owner_key = scoped_memory_owner_key(scope);
        let rows = if let Some(project_id) = scope.project_id() {
            let prefix = format!("projects/{project_id}/");
            client
                .query(
                    "SELECT path FROM memory_documents WHERE user_id = $1 AND agent_id IS NULL AND path LIKE $2 ORDER BY path",
                    &[&owner_key, &format!("{prefix}%")],
                )
                .await
        } else {
            client
                .query(
                    "SELECT path FROM memory_documents WHERE user_id = $1 AND agent_id IS NULL AND path NOT LIKE 'projects/%' ORDER BY path",
                    &[&owner_key],
                )
                .await
        }
        .map_err(|error| memory_error(virtual_path, FilesystemOperation::ListDir, error.to_string()))?;

        Ok(rows
            .into_iter()
            .filter_map(|row| {
                let db_path: String = row.get("path");
                memory_document_from_db_path(scope, &db_path)
            })
            .collect())
    }

    async fn search_documents(
        &self,
        scope: &MemoryDocumentScope,
        request: &MemorySearchRequest,
    ) -> Result<Vec<MemorySearchResult>, FilesystemError> {
        if !request.full_text() {
            return Ok(Vec::new());
        }
        let virtual_path = scope
            .virtual_prefix()
            .unwrap_or_else(|_| valid_memory_path());
        let client = self
            .client(virtual_path.clone(), FilesystemOperation::ReadFile)
            .await?;
        let owner_key = scoped_memory_owner_key(scope);
        let rows = client
            .query(
                r#"
                SELECT d.path, c.content, ts_rank_cd(c.content_tsv, plainto_tsquery('english', $2)) as rank
                FROM memory_chunks c
                JOIN memory_documents d ON d.id = c.document_id
                WHERE d.user_id = $1 AND d.agent_id IS NULL
                  AND c.content_tsv @@ plainto_tsquery('english', $2)
                ORDER BY rank DESC
                LIMIT $3
                "#,
                &[&owner_key, &request.query(), &(request.limit() as i64)],
            )
            .await
            .map_err(|error| memory_error(virtual_path, FilesystemOperation::ReadFile, error.to_string()))?;
        Ok(rows
            .into_iter()
            .filter_map(|row| {
                let db_path: String = row.get("path");
                let path = memory_document_from_db_path(scope, &db_path)?;
                let snippet: String = row.get("content");
                let score: f32 = row.try_get::<_, f32>("rank").unwrap_or(0.0);
                Some(MemorySearchResult {
                    path,
                    score,
                    snippet,
                })
            })
            .collect())
    }
}

#[cfg(feature = "postgres")]
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
                "SELECT id, content FROM memory_documents WHERE user_id = $1 AND agent_id IS NULL AND path = $2 FOR UPDATE",
                &[&owner_key, &db_path],
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

    async fn delete_document_chunks(
        &self,
        path: &MemoryDocumentPath,
    ) -> Result<(), FilesystemError> {
        let virtual_path = path.virtual_path().unwrap_or_else(|_| valid_memory_path());
        let client = self
            .client(virtual_path.clone(), FilesystemOperation::WriteFile)
            .await?;
        let owner_key = scoped_memory_owner_key(path.scope());
        let db_path = db_path_for_memory_document(path);
        client
            .execute(
                r#"
                DELETE FROM memory_chunks
                WHERE document_id IN (
                    SELECT id FROM memory_documents
                    WHERE user_id = $1 AND agent_id IS NULL AND path = $2
                )
                "#,
                &[&owner_key, &db_path],
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

#[cfg(feature = "postgres")]
async fn postgres_save_document_version(
    client: &deadpool_postgres::Object,
    virtual_path: &VirtualPath,
    document_id: uuid::Uuid,
    content: &str,
    changed_by: Option<&str>,
) -> Result<i32, FilesystemError> {
    client
        .execute(
            "SELECT 1 FROM memory_documents WHERE id = $1 FOR UPDATE",
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

#[cfg(feature = "postgres")]
const POSTGRES_MEMORY_DOCUMENTS_SCHEMA: &str = r#"
CREATE EXTENSION IF NOT EXISTS pgcrypto;
CREATE EXTENSION IF NOT EXISTS vector;

CREATE TABLE IF NOT EXISTS memory_documents (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id TEXT NOT NULL,
    agent_id UUID,
    path TEXT NOT NULL,
    content TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    metadata JSONB NOT NULL DEFAULT '{}',
    CONSTRAINT unique_path_per_user UNIQUE (user_id, agent_id, path)
);

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

fn scoped_memory_owner_key(scope: &MemoryDocumentScope) -> String {
    format!("tenant:{}:user:{}", scope.tenant_id(), scope.user_id())
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
fn db_path_for_memory_document(path: &MemoryDocumentPath) -> String {
    match path.project_id() {
        Some(project_id) => format!("projects/{project_id}/{}", path.relative_path()),
        None => path.relative_path().to_string(),
    }
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
fn memory_document_from_db_path(
    scope: &MemoryDocumentScope,
    db_path: &str,
) -> Option<MemoryDocumentPath> {
    let relative_path = match scope.project_id() {
        Some(project_id) => db_path.strip_prefix(&format!("projects/{project_id}/"))?,
        None if db_path.starts_with("projects/") => return None,
        None => db_path,
    };
    validated_memory_relative_path(relative_path.to_string())
        .ok()
        .map(|relative_path| MemoryDocumentPath {
            scope: scope.clone(),
            relative_path,
        })
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

fn memory_backend_unsupported(
    scope: &MemoryDocumentScope,
    operation: FilesystemOperation,
    reason: &'static str,
) -> FilesystemError {
    memory_error(
        scope
            .virtual_prefix()
            .unwrap_or_else(|_| valid_memory_path()),
        operation,
        reason,
    )
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
