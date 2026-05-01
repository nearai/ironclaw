//! Memory backend trait, capabilities, context, and repository-backed adapter.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_filesystem::{FilesystemError, FilesystemOperation};

use crate::embedding::{EmbeddingProvider, embed_text};
use crate::indexer::MemoryDocumentIndexer;
use crate::metadata::{MemoryWriteOptions, resolve_document_metadata};
use crate::path::{
    MemoryDocumentPath, MemoryDocumentScope, memory_backend_unsupported, memory_error,
    valid_memory_path,
};
use crate::repo::{MemoryDocumentRepository, scoped_memory_owner_key};
use crate::schema::validate_content_against_schema;
use crate::search::{MemorySearchRequest, MemorySearchResult};

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

/// Memory backend wrapper for existing repository/indexer implementations.
pub struct RepositoryMemoryBackend<R> {
    repository: Arc<R>,
    indexer: Option<Arc<dyn MemoryDocumentIndexer>>,
    embedding_provider: Option<Arc<dyn EmbeddingProvider>>,
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
            embedding_provider: None,
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

    pub fn with_embedding_provider<P>(mut self, provider: Arc<P>) -> Self
    where
        P: EmbeddingProvider + 'static,
    {
        self.embedding_provider = Some(provider);
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
            let _ = indexer.reindex_document(path).await;
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
        if (request.full_text() || request.vector())
            && !self.capabilities.full_text_search
            && !self.capabilities.vector_search
        {
            return Err(memory_backend_unsupported(
                context.scope(),
                FilesystemOperation::ReadFile,
                "memory backend does not support search",
            ));
        }
        if request.full_text() && !self.capabilities.full_text_search {
            return Err(memory_backend_unsupported(
                context.scope(),
                FilesystemOperation::ReadFile,
                "memory backend does not support full-text search",
            ));
        }
        if request.vector()
            && !self.capabilities.vector_search
            && (request.query_embedding().is_some() || !request.full_text())
        {
            return Err(memory_backend_unsupported(
                context.scope(),
                FilesystemOperation::ReadFile,
                "memory backend does not support vector search",
            ));
        }
        if !request.full_text()
            && (!request.vector()
                || (request.query_embedding().is_none() && self.embedding_provider.is_none()))
        {
            return Err(memory_backend_unsupported(
                context.scope(),
                FilesystemOperation::ReadFile,
                "memory backend does not support search",
            ));
        }

        let mut request = request;
        if request.vector()
            && self.capabilities.vector_search
            && request.query_embedding().is_none()
            && let Some(provider) = &self.embedding_provider
        {
            let embedding = embed_text(provider.as_ref(), context.scope(), request.query()).await?;
            request = request.with_query_embedding(embedding);
        }

        // Fail-fast on caller-supplied embeddings whose dimension disagrees with the
        // configured provider, instead of silently producing no/wrong results downstream
        // (libsql cosine_similarity skips mismatched chunks; postgres pgvector errors
        // opaquely).
        if let (Some(provider), Some(embedding)) =
            (&self.embedding_provider, request.query_embedding())
        {
            let expected = provider.dimension();
            let actual = embedding.len();
            if expected != actual {
                return Err(memory_backend_unsupported(
                    context.scope(),
                    FilesystemOperation::ReadFile,
                    format!(
                        "query embedding dimension {actual} does not match configured provider dimension {expected}"
                    ),
                ));
            }
        }

        self.repository
            .search_documents(context.scope(), &request)
            .await
    }
}
