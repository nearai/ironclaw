//! Memory backend trait, capabilities, context, and repository-backed adapter.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_filesystem::{FilesystemError, FilesystemOperation};

use crate::chunking::{content_bytes_sha256, content_sha256};
use crate::embedding::{EmbeddingProvider, embed_text};
use crate::indexer::MemoryDocumentIndexer;
use crate::metadata::{MemoryWriteOptions, resolve_document_metadata};
use crate::path::{
    MemoryDocumentPath, MemoryDocumentScope, memory_backend_unsupported, memory_error,
    valid_memory_path,
};
use crate::repo::{MemoryAppendOutcome, MemoryDocumentRepository, scoped_memory_owner_key};
use crate::safety::{
    DefaultPromptWriteSafetyPolicy, PromptProtectedPathRegistry, PromptSafetyAllowanceId,
    PromptWriteOperation, PromptWriteSafetyCheck, PromptWriteSafetyEventSink,
    PromptWriteSafetyPolicy, PromptWriteSource, enforce_prompt_write_safety,
    prompt_write_policy_requires_previous_content_hash, prompt_write_protected_classification,
};
use crate::schema::validate_content_against_schema;
use crate::search::{MemorySearchRequest, MemorySearchResult};

/// Declared behavior supported by a memory backend.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemoryBackendCapabilities {
    pub file_documents: bool,
    pub metadata: bool,
    pub versioning: bool,
    /// Backend enforces prompt-write safety for protected write and append operations.
    /// Filesystem adapters can defer duplicate policy checks to backends that advertise this.
    pub prompt_write_safety: bool,
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
    prompt_write_safety_allowance: Option<PromptSafetyAllowanceId>,
}

impl MemoryContext {
    pub fn new(scope: MemoryDocumentScope) -> Self {
        Self {
            scope,
            invocation_id: None,
            prompt_write_safety_allowance: None,
        }
    }

    pub fn with_invocation_id(mut self, invocation_id: impl Into<String>) -> Self {
        self.invocation_id = Some(invocation_id.into());
        self
    }

    pub fn with_prompt_write_safety_allowance(
        mut self,
        allowance: PromptSafetyAllowanceId,
    ) -> Self {
        self.prompt_write_safety_allowance = Some(allowance);
        self
    }

    pub fn scope(&self) -> &MemoryDocumentScope {
        &self.scope
    }

    pub fn invocation_id(&self) -> Option<&str> {
        self.invocation_id.as_deref()
    }

    pub fn prompt_write_safety_allowance(&self) -> Option<&PromptSafetyAllowanceId> {
        self.prompt_write_safety_allowance.as_ref()
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

    async fn compare_and_append_document(
        &self,
        context: &MemoryContext,
        path: &MemoryDocumentPath,
        expected_previous_hash: Option<&str>,
        bytes: &[u8],
    ) -> Result<MemoryAppendOutcome, FilesystemError> {
        let _ = (path, expected_previous_hash, bytes);
        Err(memory_backend_unsupported(
            context.scope(),
            FilesystemOperation::AppendFile,
            "memory backend does not support atomic append",
        ))
    }
}

/// Memory backend wrapper for existing repository/indexer implementations.
pub struct RepositoryMemoryBackend<R> {
    repository: Arc<R>,
    indexer: Option<Arc<dyn MemoryDocumentIndexer>>,
    embedding_provider: Option<Arc<dyn EmbeddingProvider>>,
    capabilities: MemoryBackendCapabilities,
    prompt_safety_policy: Option<Arc<dyn PromptWriteSafetyPolicy>>,
    prompt_safety_event_sink: Option<Arc<dyn PromptWriteSafetyEventSink>>,
    prompt_protected_path_registry: PromptProtectedPathRegistry,
}

impl<R> RepositoryMemoryBackend<R>
where
    R: MemoryDocumentRepository + 'static,
{
    pub fn new(repository: Arc<R>) -> Self {
        let registry = PromptProtectedPathRegistry::default();
        Self {
            repository,
            indexer: None,
            embedding_provider: None,
            capabilities: MemoryBackendCapabilities {
                file_documents: true,
                metadata: true,
                versioning: true,
                prompt_write_safety: true,
                ..MemoryBackendCapabilities::default()
            },
            prompt_safety_policy: Some(Arc::new(DefaultPromptWriteSafetyPolicy::with_registry(
                registry.clone(),
            ))),
            prompt_safety_event_sink: None,
            prompt_protected_path_registry: registry,
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

    pub fn with_prompt_write_safety_policy<P>(mut self, policy: Arc<P>) -> Self
    where
        P: PromptWriteSafetyPolicy + 'static,
    {
        let policy: Arc<dyn PromptWriteSafetyPolicy> = policy;
        self.prompt_safety_policy = Some(policy);
        self
    }

    pub fn without_prompt_write_safety_policy(mut self) -> Self {
        self.prompt_safety_policy = None;
        self
    }

    pub fn with_prompt_write_safety_event_sink<S>(mut self, event_sink: Arc<S>) -> Self
    where
        S: PromptWriteSafetyEventSink + 'static,
    {
        let event_sink: Arc<dyn PromptWriteSafetyEventSink> = event_sink;
        self.prompt_safety_event_sink = Some(event_sink);
        self
    }

    pub fn with_prompt_protected_path_registry(
        mut self,
        registry: PromptProtectedPathRegistry,
    ) -> Self {
        self.prompt_protected_path_registry = registry;
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
        context: &MemoryContext,
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
        let previous_hash = if prompt_write_protected_classification(
            self.prompt_safety_policy.as_ref(),
            &self.prompt_protected_path_registry,
            path,
        )
        .is_some()
            && prompt_write_policy_requires_previous_content_hash(
                self.prompt_safety_policy.as_ref(),
            ) {
            self.repository
                .read_document(path)
                .await?
                .and_then(|bytes| std::str::from_utf8(&bytes).ok().map(content_sha256))
        } else {
            None
        };
        enforce_prompt_write_safety(
            self.prompt_safety_policy.as_ref(),
            self.prompt_safety_event_sink.as_ref(),
            &self.prompt_protected_path_registry,
            PromptWriteSafetyCheck {
                scope: context.scope(),
                path,
                operation: PromptWriteOperation::Write,
                source: PromptWriteSource::MemoryBackend,
                content,
                previous_content_hash: previous_hash.as_deref(),
                allowance: context.prompt_write_safety_allowance(),
                filesystem_operation: FilesystemOperation::WriteFile,
            },
        )
        .await?;
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

    async fn compare_and_append_document(
        &self,
        context: &MemoryContext,
        path: &MemoryDocumentPath,
        expected_previous_hash: Option<&str>,
        bytes: &[u8],
    ) -> Result<MemoryAppendOutcome, FilesystemError> {
        let current = self.repository.read_document(path).await?;
        if current.as_deref().map(content_bytes_sha256).as_deref() != expected_previous_hash {
            return Ok(MemoryAppendOutcome::Conflict);
        }
        let previous_bytes = current.unwrap_or_default();
        let mut combined = previous_bytes.clone();
        combined.extend_from_slice(bytes);
        let content = std::str::from_utf8(&combined).map_err(|_| {
            memory_error(
                path.virtual_path().unwrap_or_else(|_| valid_memory_path()),
                FilesystemOperation::AppendFile,
                "memory document content must be UTF-8",
            )
        })?;
        let previous_hash = if prompt_write_protected_classification(
            self.prompt_safety_policy.as_ref(),
            &self.prompt_protected_path_registry,
            path,
        )
        .is_some()
            && prompt_write_policy_requires_previous_content_hash(
                self.prompt_safety_policy.as_ref(),
            ) {
            std::str::from_utf8(&previous_bytes)
                .ok()
                .map(content_sha256)
        } else {
            None
        };
        enforce_prompt_write_safety(
            self.prompt_safety_policy.as_ref(),
            self.prompt_safety_event_sink.as_ref(),
            &self.prompt_protected_path_registry,
            PromptWriteSafetyCheck {
                scope: context.scope(),
                path,
                operation: PromptWriteOperation::Append,
                source: PromptWriteSource::MemoryBackend,
                content,
                previous_content_hash: previous_hash.as_deref(),
                allowance: context.prompt_write_safety_allowance(),
                filesystem_operation: FilesystemOperation::AppendFile,
            },
        )
        .await?;
        let metadata = resolve_document_metadata(self.repository.as_ref(), path).await?;
        if let Some(schema) = &metadata.schema {
            validate_content_against_schema(path, content, schema)?;
        }
        let options = MemoryWriteOptions {
            metadata,
            changed_by: Some(scoped_memory_owner_key(path.scope())),
        };
        let outcome = self
            .repository
            .compare_and_append_document_with_options(path, expected_previous_hash, bytes, &options)
            .await?;
        if outcome == MemoryAppendOutcome::Appended
            && let Some(indexer) = &self.indexer
        {
            let _ = indexer.reindex_document(path).await;
        }
        Ok(outcome)
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
