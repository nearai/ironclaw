//! Memory backend trait, capabilities, context, and repository-backed adapter.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_filesystem::{FilesystemError, FilesystemOperation};

use crate::chunking::{content_bytes_sha256, content_sha256};
use crate::embedding::{EmbeddingProvider, embed_text};
use crate::events::{
    MemorySignificantEvent, MemorySignificantEventSink, MemorySignificantEventSource,
    record_memory_significant_event,
};
use crate::indexer::MemoryDocumentIndexer;
use crate::metadata::{MemoryBackendWriteOptions, MemoryWriteOptions};
use crate::path::{
    MemoryDocumentPath, MemoryDocumentScope, memory_backend_unsupported, memory_error,
    valid_memory_path,
};
use crate::repo::{
    MemoryAppendOutcome, MemoryDocumentRepository, MemoryWriteOutcome, scoped_memory_changed_by_key,
};
use crate::safety::{
    DefaultPromptWriteSafetyPolicy, PromptProtectedPathRegistry, PromptWriteOperation,
    PromptWriteSafetyCheck, PromptWriteSafetyEventSink, PromptWriteSafetyPolicy, PromptWriteSource,
    enforce_prompt_write_safety, prompt_write_policy_requires_previous_content_hash,
    prompt_write_protected_classification,
};
use crate::schema::validate_content_against_schema;
use crate::search::{MemorySearchRequest, MemorySearchResult};
use crate::write_metadata::resolve_write_metadata;

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

impl MemoryBackendCapabilities {
    pub fn set_file_documents(mut self, file_documents: bool) -> Self {
        self.file_documents = file_documents;
        self
    }

    pub fn set_metadata(mut self, metadata: bool) -> Self {
        self.metadata = metadata;
        self
    }

    pub fn set_versioning(mut self, versioning: bool) -> Self {
        self.versioning = versioning;
        self
    }

    pub fn set_prompt_write_safety(mut self, prompt_write_safety: bool) -> Self {
        self.prompt_write_safety = prompt_write_safety;
        self
    }

    pub fn set_full_text_search(mut self, full_text_search: bool) -> Self {
        self.full_text_search = full_text_search;
        self
    }

    pub fn set_vector_search(mut self, vector_search: bool) -> Self {
        self.vector_search = vector_search;
        self
    }

    pub fn set_embeddings(mut self, embeddings: bool) -> Self {
        self.embeddings = embeddings;
        self
    }

    pub fn set_graph_memory(mut self, graph_memory: bool) -> Self {
        self.graph_memory = graph_memory;
        self
    }

    pub fn set_delete(mut self, delete: bool) -> Self {
        self.delete = delete;
        self
    }

    pub fn set_transactions(mut self, transactions: bool) -> Self {
        self.transactions = transactions;
        self
    }
}

// `MemoryContext` moved to `ironclaw_memory`; re-exported so
// `crate::backend::MemoryContext` and the backend code below keep resolving.
// NOTE: the backend's "already enforced" signal (`prompt_safety_already_enforced`)
// lives on the native-owned `MemoryBackendWriteOptions`, which only the filesystem
// adapter sets before deferring backend re-enforcement. (The prompt-write
// allowance carried on `MemoryContext` is a separate, pre-existing mechanism.)
pub use ironclaw_memory::MemoryContext;

/// Pluggable memory backend contract.
///
/// The host owns authority, scope parsing, and mount exposure. Backends own
/// storage/search behavior inside the already-resolved [`MemoryContext`].
#[async_trait]
pub trait MemoryBackend: Send + Sync {
    fn capabilities(&self) -> MemoryBackendCapabilities;

    fn prompt_write_safety_protects_path(&self, path: &MemoryDocumentPath) -> bool {
        let _ = path;
        false
    }

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

    async fn write_document_with_backend_options(
        &self,
        context: &MemoryContext,
        path: &MemoryDocumentPath,
        bytes: &[u8],
        backend_options: &MemoryBackendWriteOptions,
    ) -> Result<(), FilesystemError> {
        let _ = backend_options;
        self.write_document(context, path, bytes).await
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

    async fn compare_and_append_document_with_backend_options(
        &self,
        context: &MemoryContext,
        path: &MemoryDocumentPath,
        expected_previous_hash: Option<&str>,
        bytes: &[u8],
        backend_options: &MemoryBackendWriteOptions,
    ) -> Result<MemoryAppendOutcome, FilesystemError> {
        let _ = backend_options;
        self.compare_and_append_document(context, path, expected_previous_hash, bytes)
            .await
    }

    async fn compare_and_write_document_with_backend_options(
        &self,
        context: &MemoryContext,
        path: &MemoryDocumentPath,
        expected_previous_hash: Option<&str>,
        bytes: &[u8],
        backend_options: &MemoryBackendWriteOptions,
    ) -> Result<MemoryWriteOutcome, FilesystemError> {
        let _ = (expected_previous_hash, backend_options);
        self.write_document(context, path, bytes)
            .await
            .map(|_| MemoryWriteOutcome::Written)
    }

    async fn append_document_with_backend_options(
        &self,
        context: &MemoryContext,
        path: &MemoryDocumentPath,
        bytes: &[u8],
        backend_options: &MemoryBackendWriteOptions,
    ) -> Result<(), FilesystemError> {
        for _ in 0..8 {
            let previous = self.read_document(context, path).await?;
            let expected = previous.as_deref().map(content_bytes_sha256);
            let outcome = self
                .compare_and_append_document_with_backend_options(
                    context,
                    path,
                    expected.as_deref(),
                    bytes,
                    backend_options,
                )
                .await?;
            if outcome == MemoryAppendOutcome::Appended {
                return Ok(());
            }
        }
        Err(memory_error(
            path.virtual_path().unwrap_or_else(|_| valid_memory_path()),
            FilesystemOperation::AppendFile,
            "memory document changed during append; retry limit exceeded",
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
    memory_event_sink: Option<Arc<dyn MemorySignificantEventSink>>,
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
            capabilities: MemoryBackendCapabilities::default()
                .set_file_documents(true)
                .set_metadata(true)
                .set_versioning(true)
                .set_prompt_write_safety(true),
            prompt_safety_policy: Some(Arc::new(DefaultPromptWriteSafetyPolicy::with_registry(
                registry.clone(),
            ))),
            prompt_safety_event_sink: None,
            memory_event_sink: None,
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

    pub fn with_prompt_write_safety_event_sink(
        mut self,
        event_sink: Arc<dyn PromptWriteSafetyEventSink>,
    ) -> Self {
        self.prompt_safety_event_sink = Some(event_sink);
        self
    }

    pub fn with_memory_event_sink<S>(mut self, event_sink: Arc<S>) -> Self
    where
        S: MemorySignificantEventSink + 'static,
    {
        let event_sink: Arc<dyn MemorySignificantEventSink> = event_sink;
        self.memory_event_sink = Some(event_sink);
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

// Defense-in-depth scope guards for the public `MemoryBackend` seam.
fn ensure_path_matches_context(
    context: &MemoryContext,
    path: &MemoryDocumentPath,
    operation: FilesystemOperation,
) -> Result<(), FilesystemError> {
    if path.scope() == context.scope() {
        return Ok(());
    }
    Err(memory_error(
        path.virtual_path().unwrap_or_else(|_| valid_memory_path()),
        operation,
        "memory document scope does not match authorized memory context",
    ))
}

fn ensure_file_documents_supported(
    context: &MemoryContext,
    operation: FilesystemOperation,
    supported: bool,
) -> Result<(), FilesystemError> {
    if supported {
        return Ok(());
    }
    Err(memory_backend_unsupported(
        context.scope(),
        operation,
        "memory backend does not support file documents",
    ))
}

fn ensure_scope_matches_context(
    context: &MemoryContext,
    scope: &MemoryDocumentScope,
    operation: FilesystemOperation,
) -> Result<(), FilesystemError> {
    if scope == context.scope() {
        return Ok(());
    }
    Err(memory_backend_unsupported(
        context.scope(),
        operation,
        "memory document scope does not match authorized memory context",
    ))
}

#[async_trait]
impl<R> MemoryBackend for RepositoryMemoryBackend<R>
where
    R: MemoryDocumentRepository + 'static,
{
    fn capabilities(&self) -> MemoryBackendCapabilities {
        self.capabilities.clone()
    }

    fn prompt_write_safety_protects_path(&self, path: &MemoryDocumentPath) -> bool {
        prompt_write_protected_classification(
            self.prompt_safety_policy.as_ref(),
            &self.prompt_protected_path_registry,
            path,
        )
        .is_some()
    }

    async fn read_document(
        &self,
        context: &MemoryContext,
        path: &MemoryDocumentPath,
    ) -> Result<Option<Vec<u8>>, FilesystemError> {
        ensure_file_documents_supported(
            context,
            FilesystemOperation::ReadFile,
            self.capabilities.file_documents,
        )?;
        ensure_path_matches_context(context, path, FilesystemOperation::ReadFile)?;
        self.repository.read_document(path).await
    }

    async fn write_document(
        &self,
        context: &MemoryContext,
        path: &MemoryDocumentPath,
        bytes: &[u8],
    ) -> Result<(), FilesystemError> {
        self.write_document_with_backend_options(
            context,
            path,
            bytes,
            &MemoryBackendWriteOptions::default(),
        )
        .await
    }

    async fn write_document_with_backend_options(
        &self,
        context: &MemoryContext,
        path: &MemoryDocumentPath,
        bytes: &[u8],
        backend_options: &MemoryBackendWriteOptions,
    ) -> Result<(), FilesystemError> {
        ensure_file_documents_supported(
            context,
            FilesystemOperation::WriteFile,
            self.capabilities.file_documents,
        )?;
        ensure_path_matches_context(context, path, FilesystemOperation::WriteFile)?;
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
        if !backend_options.prompt_safety_already_enforced {
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
                    audit_context: context.audit_context(),
                    filesystem_operation: FilesystemOperation::WriteFile,
                },
            )
            .await?;
        }
        let (metadata, metadata_to_persist) =
            resolve_write_metadata(self.repository.as_ref(), path, backend_options).await?;
        if let Some(schema) = &metadata.schema {
            validate_content_against_schema(path, content, schema)?;
        }
        let options = MemoryWriteOptions {
            metadata,
            changed_by: Some(scoped_memory_changed_by_key(path.scope())),
        };
        self.repository
            .write_document_with_options(path, bytes, &options)
            .await?;
        if let Some(metadata) = metadata_to_persist {
            self.repository
                .write_document_metadata(path, &metadata)
                .await?;
        }
        record_memory_significant_event(
            self.memory_event_sink.as_ref(),
            MemorySignificantEvent::document_written(
                path,
                MemorySignificantEventSource::RepositoryMemoryBackend,
                bytes.len() as u64,
            )
            .with_audit_context(context.audit_context()),
        )
        .await;
        if let Some(indexer) = &self.indexer {
            let _ = indexer
                .reindex_document_with_audit_context(path, context.audit_context())
                .await;
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
        self.compare_and_append_document_with_backend_options(
            context,
            path,
            expected_previous_hash,
            bytes,
            &MemoryBackendWriteOptions::default(),
        )
        .await
    }

    async fn compare_and_append_document_with_backend_options(
        &self,
        context: &MemoryContext,
        path: &MemoryDocumentPath,
        expected_previous_hash: Option<&str>,
        bytes: &[u8],
        backend_options: &MemoryBackendWriteOptions,
    ) -> Result<MemoryAppendOutcome, FilesystemError> {
        ensure_file_documents_supported(
            context,
            FilesystemOperation::AppendFile,
            self.capabilities.file_documents,
        )?;
        ensure_path_matches_context(context, path, FilesystemOperation::AppendFile)?;
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
        if !backend_options.prompt_safety_already_enforced {
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
                    audit_context: context.audit_context(),
                    filesystem_operation: FilesystemOperation::AppendFile,
                },
            )
            .await?;
        }
        let (metadata, metadata_to_persist) =
            resolve_write_metadata(self.repository.as_ref(), path, backend_options).await?;
        if let Some(schema) = &metadata.schema {
            validate_content_against_schema(path, content, schema)?;
        }
        let options = MemoryWriteOptions {
            metadata,
            changed_by: Some(scoped_memory_changed_by_key(path.scope())),
        };
        let outcome = self
            .repository
            .compare_and_append_document_with_options(path, expected_previous_hash, bytes, &options)
            .await?;
        if outcome == MemoryAppendOutcome::Appended {
            if let Some(metadata) = metadata_to_persist {
                self.repository
                    .write_document_metadata(path, &metadata)
                    .await?;
            }
            record_memory_significant_event(
                self.memory_event_sink.as_ref(),
                MemorySignificantEvent::document_written(
                    path,
                    MemorySignificantEventSource::RepositoryMemoryBackend,
                    bytes.len() as u64,
                )
                .with_audit_context(context.audit_context()),
            )
            .await;
            if let Some(indexer) = &self.indexer {
                let _ = indexer
                    .reindex_document_with_audit_context(path, context.audit_context())
                    .await;
            }
        }
        Ok(outcome)
    }

    async fn append_document_with_backend_options(
        &self,
        context: &MemoryContext,
        path: &MemoryDocumentPath,
        bytes: &[u8],
        backend_options: &MemoryBackendWriteOptions,
    ) -> Result<(), FilesystemError> {
        for _ in 0..8 {
            ensure_file_documents_supported(
                context,
                FilesystemOperation::AppendFile,
                self.capabilities.file_documents,
            )?;
            ensure_path_matches_context(context, path, FilesystemOperation::AppendFile)?;
            let current = self.repository.read_document(path).await?;
            let expected_previous_hash = current.as_deref().map(content_bytes_sha256);
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
            if !backend_options.prompt_safety_already_enforced {
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
                        audit_context: context.audit_context(),
                        filesystem_operation: FilesystemOperation::AppendFile,
                    },
                )
                .await?;
            }
            let (metadata, metadata_to_persist) =
                resolve_write_metadata(self.repository.as_ref(), path, backend_options).await?;
            if let Some(schema) = &metadata.schema {
                validate_content_against_schema(path, content, schema)?;
            }
            let repository_options = MemoryWriteOptions {
                metadata,
                changed_by: Some(scoped_memory_changed_by_key(path.scope())),
            };
            let outcome = self
                .repository
                .compare_and_append_document_with_options(
                    path,
                    expected_previous_hash.as_deref(),
                    bytes,
                    &repository_options,
                )
                .await?;
            if outcome == MemoryAppendOutcome::Appended {
                if let Some(metadata) = metadata_to_persist {
                    self.repository
                        .write_document_metadata(path, &metadata)
                        .await?;
                }
                record_memory_significant_event(
                    self.memory_event_sink.as_ref(),
                    MemorySignificantEvent::document_written(
                        path,
                        MemorySignificantEventSource::RepositoryMemoryBackend,
                        bytes.len() as u64,
                    )
                    .with_audit_context(context.audit_context()),
                )
                .await;
                if let Some(indexer) = &self.indexer {
                    let _ = indexer
                        .reindex_document_with_audit_context(path, context.audit_context())
                        .await;
                }
                return Ok(());
            }
            std::thread::yield_now();
        }
        Err(memory_error(
            path.virtual_path().unwrap_or_else(|_| valid_memory_path()),
            FilesystemOperation::AppendFile,
            "memory document changed during append; retry limit exceeded",
        ))
    }

    async fn compare_and_write_document_with_backend_options(
        &self,
        context: &MemoryContext,
        path: &MemoryDocumentPath,
        expected_previous_hash: Option<&str>,
        bytes: &[u8],
        backend_options: &MemoryBackendWriteOptions,
    ) -> Result<MemoryWriteOutcome, FilesystemError> {
        ensure_file_documents_supported(
            context,
            FilesystemOperation::WriteFile,
            self.capabilities.file_documents,
        )?;
        ensure_path_matches_context(context, path, FilesystemOperation::WriteFile)?;
        let content = std::str::from_utf8(bytes).map_err(|_| {
            memory_error(
                path.virtual_path().unwrap_or_else(|_| valid_memory_path()),
                FilesystemOperation::WriteFile,
                "memory document content must be UTF-8",
            )
        })?;
        let previous = self.repository.read_document(path).await?;
        let current_hash = previous.as_deref().map(content_bytes_sha256);
        if current_hash.as_deref() != expected_previous_hash {
            return Ok(MemoryWriteOutcome::Conflict);
        }
        let previous_hash = if prompt_write_protected_classification(
            self.prompt_safety_policy.as_ref(),
            &self.prompt_protected_path_registry,
            path,
        )
        .is_some()
            && prompt_write_policy_requires_previous_content_hash(
                self.prompt_safety_policy.as_ref(),
            ) {
            previous
                .as_deref()
                .and_then(|bytes| std::str::from_utf8(bytes).ok().map(content_sha256))
        } else {
            None
        };
        if !backend_options.prompt_safety_already_enforced {
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
                    audit_context: context.audit_context(),
                    filesystem_operation: FilesystemOperation::WriteFile,
                },
            )
            .await?;
        }
        let (metadata, metadata_to_persist) =
            resolve_write_metadata(self.repository.as_ref(), path, backend_options).await?;
        if let Some(schema) = &metadata.schema {
            validate_content_against_schema(path, content, schema)?;
        }
        let options = MemoryWriteOptions {
            metadata,
            changed_by: Some(scoped_memory_changed_by_key(path.scope())),
        };
        let outcome = self
            .repository
            .compare_and_write_document_with_options(path, expected_previous_hash, bytes, &options)
            .await?;
        if outcome == MemoryWriteOutcome::Written {
            if let Some(metadata) = metadata_to_persist {
                self.repository
                    .write_document_metadata(path, &metadata)
                    .await?;
            }
            record_memory_significant_event(
                self.memory_event_sink.as_ref(),
                MemorySignificantEvent::document_written(
                    path,
                    MemorySignificantEventSource::RepositoryMemoryBackend,
                    bytes.len() as u64,
                )
                .with_audit_context(context.audit_context()),
            )
            .await;
            if let Some(indexer) = &self.indexer {
                let _ = indexer
                    .reindex_document_with_audit_context(path, context.audit_context())
                    .await;
            }
        }
        Ok(outcome)
    }

    async fn list_documents(
        &self,
        context: &MemoryContext,
        scope: &MemoryDocumentScope,
    ) -> Result<Vec<MemoryDocumentPath>, FilesystemError> {
        ensure_file_documents_supported(
            context,
            FilesystemOperation::ListDir,
            self.capabilities.file_documents,
        )?;
        ensure_scope_matches_context(context, scope, FilesystemOperation::ListDir)?;
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
        if request.vector() && !self.capabilities.vector_search {
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
        {
            if !self.capabilities.embeddings {
                return Err(memory_backend_unsupported(
                    context.scope(),
                    FilesystemOperation::ReadFile,
                    "memory backend does not support embedding generation",
                ));
            }
            let Some(provider) = &self.embedding_provider else {
                return Err(memory_backend_unsupported(
                    context.scope(),
                    FilesystemOperation::ReadFile,
                    "memory backend cannot generate query embeddings",
                ));
            };
            let embedding = embed_text(provider.as_ref(), context.scope(), request.query()).await?;
            request = request.with_query_embedding(embedding);
        }

        // Fail-fast on caller-supplied embeddings whose dimension disagrees with the
        // configured provider, instead of silently producing no/wrong results downstream
        // (libsql cosine_similarity skips mismatched chunks; postgres pgvector errors
        // opaquely).
        if request.vector()
            && let (Some(provider), Some(embedding)) =
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

        let results = self
            .repository
            .search_documents(context.scope(), &request)
            .await?;
        record_memory_significant_event(
            self.memory_event_sink.as_ref(),
            MemorySignificantEvent::search_performed(
                context.scope(),
                MemorySignificantEventSource::RepositoryMemoryBackend,
                request.full_text(),
                request.vector(),
                results.len() as u64,
            )
            .with_audit_context(context.audit_context()),
        )
        .await;
        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embedding::EmbeddingError;
    use crate::repo::InMemoryMemoryDocumentRepository;
    use crate::safety::PromptSafetyAllowanceId;

    struct FailingEmbeddingProvider;

    #[async_trait]
    impl EmbeddingProvider for FailingEmbeddingProvider {
        fn dimension(&self) -> usize {
            3
        }

        fn model_name(&self) -> &str {
            "failing"
        }

        async fn embed(&self, _text: &str) -> Result<Vec<f32>, EmbeddingError> {
            Err(EmbeddingError::ProviderUnavailable {
                reason:
                    "provider exploded at /tmp/HOST_PATH_SENTINEL with token RAW_TOKEN_SENTINEL"
                        .to_string(),
            })
        }
    }

    struct UnitEmbeddingProvider;

    #[async_trait]
    impl EmbeddingProvider for UnitEmbeddingProvider {
        fn dimension(&self) -> usize {
            3
        }

        fn model_name(&self) -> &str {
            "unit"
        }

        async fn embed(&self, _text: &str) -> Result<Vec<f32>, EmbeddingError> {
            Ok(vec![1.0, 0.0, 0.0])
        }
    }

    #[test]
    fn default_backend_options_do_not_claim_prompt_safety_enforced() {
        // Locks the native-owned safety marker's default (fail-closed) state.
        // Any direct backend caller using `MemoryBackendWriteOptions::default()`
        // must have the backend re-enforce prompt-write safety.
        let options = MemoryBackendWriteOptions::default();
        assert!(!options.prompt_safety_already_enforced);
        // Setting an allowance on the agnostic context is independent of the
        // native enforcement marker — the allowance never lives on the options
        // struct and never flips the marker.
        let ctx = MemoryContext::new(
            MemoryDocumentScope::new("tenant", "alpha", Some("project")).unwrap(),
        );
        let _with_allowance = ctx
            .with_prompt_write_safety_allowance(PromptSafetyAllowanceId::empty_prompt_file_clear());
        assert!(
            !MemoryBackendWriteOptions::default().prompt_safety_already_enforced,
            "the allowance path must not flip the native enforcement marker"
        );
    }

    fn alpha_path() -> MemoryDocumentPath {
        MemoryDocumentPath::new("tenant", "alpha", Some("project"), "note.md").unwrap()
    }

    fn beta_path() -> MemoryDocumentPath {
        MemoryDocumentPath::new("tenant", "beta", Some("project"), "note.md").unwrap()
    }

    fn alpha_context() -> MemoryContext {
        MemoryContext::new(MemoryDocumentScope::new("tenant", "alpha", Some("project")).unwrap())
    }

    fn make_backend() -> RepositoryMemoryBackend<InMemoryMemoryDocumentRepository> {
        let repo = Arc::new(InMemoryMemoryDocumentRepository::new());
        RepositoryMemoryBackend::new(repo).without_prompt_write_safety_policy()
    }

    fn make_search_backend(
        capabilities: MemoryBackendCapabilities,
    ) -> RepositoryMemoryBackend<InMemoryMemoryDocumentRepository> {
        let repo = Arc::new(InMemoryMemoryDocumentRepository::new());
        RepositoryMemoryBackend::new(repo)
            .without_prompt_write_safety_policy()
            .with_embedding_provider(Arc::new(UnitEmbeddingProvider))
            .with_capabilities(capabilities)
    }

    // Regression: backend callers cannot use an authorized context for one
    // scope to operate on another scope.
    #[tokio::test]
    async fn read_document_rejects_path_with_scope_outside_authorized_context() {
        let backend = make_backend();
        let result = backend.read_document(&alpha_context(), &beta_path()).await;
        assert!(
            result.is_err(),
            "expected scope mismatch on read_document to fail closed"
        );
    }

    #[tokio::test]
    async fn write_document_rejects_path_with_scope_outside_authorized_context() {
        let backend = make_backend();
        let result = backend
            .write_document(&alpha_context(), &beta_path(), b"hello")
            .await;
        assert!(
            result.is_err(),
            "expected scope mismatch on write_document to fail closed"
        );
    }

    #[tokio::test]
    async fn compare_and_append_document_rejects_path_with_scope_outside_authorized_context() {
        let backend = make_backend();
        let result = backend
            .compare_and_append_document(&alpha_context(), &beta_path(), None, b"hello")
            .await;
        assert!(
            result.is_err(),
            "expected scope mismatch on compare_and_append_document to fail closed"
        );
    }

    #[tokio::test]
    async fn list_documents_rejects_scope_outside_authorized_context() {
        let backend = make_backend();
        let other_scope = MemoryDocumentScope::new("tenant", "beta", Some("project")).unwrap();
        let result = backend.list_documents(&alpha_context(), &other_scope).await;
        assert!(
            result.is_err(),
            "expected scope mismatch on list_documents to fail closed"
        );
    }

    #[tokio::test]
    async fn matching_context_and_path_succeed() {
        let backend = make_backend();
        // Sanity: the same scope on both sides is the happy path.
        backend
            .read_document(&alpha_context(), &alpha_path())
            .await
            .expect("matching scope should not be rejected");
    }

    #[tokio::test]
    async fn file_document_capability_rejects_direct_backend_file_operations() {
        let backend = make_backend()
            .with_capabilities(MemoryBackendCapabilities::default().set_file_documents(false));
        assert!(
            backend
                .read_document(&alpha_context(), &alpha_path())
                .await
                .is_err()
        );
        assert!(
            backend
                .write_document(&alpha_context(), &alpha_path(), b"x")
                .await
                .is_err()
        );
        assert!(
            backend
                .compare_and_append_document(&alpha_context(), &alpha_path(), None, b"x")
                .await
                .is_err()
        );
        assert!(
            backend
                .list_documents(&alpha_context(), alpha_path().scope())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn vector_request_fails_closed_when_vector_search_is_unsupported() {
        let backend = make_search_backend(
            MemoryBackendCapabilities::default()
                .set_full_text_search(true)
                .set_vector_search(false)
                .set_embeddings(true),
        );
        let request = MemorySearchRequest::new("query").unwrap();
        let err = backend.search(&alpha_context(), request).await.unwrap_err();
        assert!(err.to_string().contains("vector search"));
    }

    #[tokio::test]
    async fn vector_request_fails_closed_when_embedding_generation_is_disabled() {
        let backend = make_search_backend(
            MemoryBackendCapabilities::default()
                .set_full_text_search(true)
                .set_vector_search(true)
                .set_embeddings(false),
        );
        let request = MemorySearchRequest::new("query").unwrap();
        let err = backend.search(&alpha_context(), request).await.unwrap_err();
        assert!(err.to_string().contains("embedding generation"));
    }

    #[tokio::test]
    async fn embedding_provider_error_is_sanitized_at_backend_boundary() {
        let repo = Arc::new(InMemoryMemoryDocumentRepository::new());
        let backend = RepositoryMemoryBackend::new(repo)
            .without_prompt_write_safety_policy()
            .with_embedding_provider(Arc::new(FailingEmbeddingProvider))
            .with_capabilities(
                MemoryBackendCapabilities::default()
                    .set_full_text_search(true)
                    .set_vector_search(true)
                    .set_embeddings(true),
            );

        let request = MemorySearchRequest::new("query").unwrap();
        let err = backend.search(&alpha_context(), request).await.unwrap_err();
        let displayed = err.to_string();

        assert!(displayed.contains("embedding provider unavailable"));
        assert!(
            !displayed.contains("HOST_PATH_SENTINEL")
                && !displayed.contains("RAW_TOKEN_SENTINEL")
                && !displayed.contains("/tmp/"),
            "public memory error leaked backend/provider details: {displayed}"
        );
    }

    #[tokio::test]
    async fn full_text_only_search_ignores_stale_query_embedding_dimension() {
        let backend = make_search_backend(
            MemoryBackendCapabilities::default()
                .set_full_text_search(true)
                .set_vector_search(true)
                .set_embeddings(true),
        );
        let request = MemorySearchRequest::new("query")
            .unwrap()
            .with_vector(false)
            .with_query_embedding(vec![1.0]);
        let err = backend.search(&alpha_context(), request).await.unwrap_err();
        assert!(
            !err.to_string().contains("dimension"),
            "full-text-only retry must not validate stale vector dimensions: {err}"
        );
    }
}
