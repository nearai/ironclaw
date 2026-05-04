//! Memory-document `RootFilesystem` adapters.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use ironclaw_filesystem::{
    DirEntry, FileStat, FileType, FilesystemError, FilesystemOperation, RootFilesystem,
};
use ironclaw_host_api::VirtualPath;

use crate::backend::{MemoryBackend, MemoryContext};
use crate::chunking::{content_bytes_sha256, content_sha256};
use crate::indexer::MemoryDocumentIndexer;
use crate::metadata::{MemoryWriteOptions, resolve_document_metadata};
use crate::path::{
    MemoryDocumentPath, MemoryDocumentScope, ParsedMemoryPath, memory_error, memory_not_found,
};
use crate::repo::{
    MemoryAppendOutcome, MemoryDocumentRepository, memory_direct_children, scoped_memory_owner_key,
};
use crate::safety::{
    DefaultPromptWriteSafetyPolicy, PromptProtectedPathRegistry, PromptSafetyAllowanceId,
    PromptWriteOperation, PromptWriteSafetyCheck, PromptWriteSafetyEnforcement,
    PromptWriteSafetyEventSink, PromptWriteSafetyPolicy, PromptWriteSource,
    enforce_prompt_write_safety, prompt_write_policy_requires_previous_content_hash,
    prompt_write_protected_classification, take_prompt_safety_allowance,
};
use crate::schema::validate_content_against_schema;

const MAX_MEMORY_APPEND_RETRIES: usize = 8;

fn memory_append_conflict_error(path: VirtualPath) -> FilesystemError {
    memory_error(
        path,
        FilesystemOperation::AppendFile,
        "memory document changed during append; retry limit exceeded",
    )
}

fn memory_context_with_prompt_safety_enforcement(
    context: &MemoryContext,
    enforcement: PromptWriteSafetyEnforcement,
) -> MemoryContext {
    let mut context = context.clone();
    if let Some(allowance) = enforcement.allowance {
        context = context.with_prompt_write_safety_allowance(allowance);
    }
    context
}

/// [`RootFilesystem`] adapter exposing any [`MemoryBackend`] as `/memory` files.
pub struct MemoryBackendFilesystemAdapter {
    backend: Arc<dyn MemoryBackend>,
    prompt_safety_policy: Option<Arc<dyn PromptWriteSafetyPolicy>>,
    prompt_safety_event_sink: Option<Arc<dyn PromptWriteSafetyEventSink>>,
    prompt_protected_path_registry: PromptProtectedPathRegistry,
    prompt_safety_config_overridden: bool,
    one_shot_prompt_safety_allowance: Mutex<Option<PromptSafetyAllowanceId>>,
}

impl MemoryBackendFilesystemAdapter {
    pub fn new<B>(backend: Arc<B>) -> Self
    where
        B: MemoryBackend + 'static,
    {
        let backend: Arc<dyn MemoryBackend> = backend;
        Self::from_dyn(backend)
    }

    pub fn from_dyn(backend: Arc<dyn MemoryBackend>) -> Self {
        let registry = PromptProtectedPathRegistry::default();
        Self {
            backend,
            prompt_safety_policy: Some(Arc::new(DefaultPromptWriteSafetyPolicy::with_registry(
                registry.clone(),
            ))),
            prompt_safety_event_sink: None,
            prompt_protected_path_registry: registry,
            prompt_safety_config_overridden: false,
            one_shot_prompt_safety_allowance: Mutex::new(None),
        }
    }

    pub fn with_prompt_write_safety_policy<P>(mut self, policy: Arc<P>) -> Self
    where
        P: PromptWriteSafetyPolicy + 'static,
    {
        let policy: Arc<dyn PromptWriteSafetyPolicy> = policy;
        self.prompt_safety_policy = Some(policy);
        self.prompt_safety_config_overridden = true;
        self
    }

    pub fn without_prompt_write_safety_policy(mut self) -> Self {
        self.prompt_safety_policy = None;
        self.prompt_safety_config_overridden = true;
        self
    }

    pub fn with_prompt_write_safety_event_sink<S>(mut self, event_sink: Arc<S>) -> Self
    where
        S: PromptWriteSafetyEventSink + 'static,
    {
        let event_sink: Arc<dyn PromptWriteSafetyEventSink> = event_sink;
        self.prompt_safety_event_sink = Some(event_sink);
        self.prompt_safety_config_overridden = true;
        self
    }

    /// Installs an explicit prompt-write safety allowance for the next protected write only.
    ///
    /// The allowance is consumed before policy evaluation so shared filesystem adapters cannot
    /// accidentally retain a bypass for later unrelated callers.
    pub fn with_one_shot_prompt_write_safety_allowance(
        self,
        allowance: PromptSafetyAllowanceId,
    ) -> Self {
        if let Ok(mut slot) = self.one_shot_prompt_safety_allowance.lock() {
            *slot = Some(allowance);
        }
        self
    }

    pub fn with_prompt_protected_path_registry(
        mut self,
        registry: PromptProtectedPathRegistry,
    ) -> Self {
        self.prompt_protected_path_registry = registry;
        self.prompt_safety_config_overridden = true;
        self
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
        let mut context = MemoryContext::new(document_path.scope().clone());
        let is_protected = prompt_write_protected_classification(
            self.prompt_safety_policy.as_ref(),
            &self.prompt_protected_path_registry,
            &document_path,
        )
        .is_some();
        let backend_capabilities = self.backend.capabilities();
        let adapter_should_enforce_prompt_safety = is_protected
            && (!backend_capabilities.prompt_write_safety || self.prompt_safety_config_overridden);
        let prompt_safety_allowance = if is_protected || backend_capabilities.prompt_write_safety {
            take_prompt_safety_allowance(
                &self.one_shot_prompt_safety_allowance,
                path,
                FilesystemOperation::WriteFile,
            )?
        } else {
            None
        };
        if let Some(allowance) = &prompt_safety_allowance {
            context = context.with_prompt_write_safety_allowance(allowance.clone());
        }
        let mut backend_context = context.clone();
        if adapter_should_enforce_prompt_safety {
            let content = std::str::from_utf8(bytes).map_err(|_| {
                memory_error(
                    path.clone(),
                    FilesystemOperation::WriteFile,
                    "memory document content must be UTF-8",
                )
            })?;
            let previous_hash = if prompt_write_policy_requires_previous_content_hash(
                self.prompt_safety_policy.as_ref(),
            ) {
                self.backend
                    .read_document(&context, &document_path)
                    .await?
                    .and_then(|bytes| std::str::from_utf8(&bytes).ok().map(content_sha256))
            } else {
                None
            };
            let enforcement = enforce_prompt_write_safety(
                self.prompt_safety_policy.as_ref(),
                self.prompt_safety_event_sink.as_ref(),
                &self.prompt_protected_path_registry,
                PromptWriteSafetyCheck {
                    scope: context.scope(),
                    path: &document_path,
                    operation: PromptWriteOperation::Write,
                    source: PromptWriteSource::MemoryFilesystemAdapter,
                    content,
                    previous_content_hash: previous_hash.as_deref(),
                    allowance: context.prompt_write_safety_allowance(),
                    filesystem_operation: FilesystemOperation::WriteFile,
                },
            )
            .await?;
            backend_context = memory_context_with_prompt_safety_enforcement(&context, enforcement);
        }
        self.backend
            .write_document(&backend_context, &document_path, bytes)
            .await
    }

    async fn append_file(&self, path: &VirtualPath, bytes: &[u8]) -> Result<(), FilesystemError> {
        self.ensure_file_documents(path, FilesystemOperation::AppendFile)?;
        let document_path = self.parse_file_path(path, FilesystemOperation::AppendFile)?;
        let mut context = MemoryContext::new(document_path.scope().clone());
        let is_protected = prompt_write_protected_classification(
            self.prompt_safety_policy.as_ref(),
            &self.prompt_protected_path_registry,
            &document_path,
        )
        .is_some();
        let backend_capabilities = self.backend.capabilities();
        let adapter_should_enforce_prompt_safety = is_protected
            && (!backend_capabilities.prompt_write_safety || self.prompt_safety_config_overridden);
        let prompt_safety_allowance = if is_protected || backend_capabilities.prompt_write_safety {
            take_prompt_safety_allowance(
                &self.one_shot_prompt_safety_allowance,
                path,
                FilesystemOperation::AppendFile,
            )?
        } else {
            None
        };
        if let Some(allowance) = &prompt_safety_allowance {
            context = context.with_prompt_write_safety_allowance(allowance.clone());
        }

        for _ in 0..MAX_MEMORY_APPEND_RETRIES {
            let previous = self.backend.read_document(&context, &document_path).await?;
            let expected_previous_hash = previous.as_deref().map(content_bytes_sha256);
            let previous_bytes = previous.unwrap_or_default();
            let previous_prompt_hash = if adapter_should_enforce_prompt_safety
                && prompt_write_policy_requires_previous_content_hash(
                    self.prompt_safety_policy.as_ref(),
                ) {
                std::str::from_utf8(&previous_bytes)
                    .ok()
                    .map(content_sha256)
            } else {
                None
            };
            let mut combined = previous_bytes;
            combined.extend_from_slice(bytes);
            let mut backend_context = context.clone();
            if adapter_should_enforce_prompt_safety {
                let content = std::str::from_utf8(&combined).map_err(|_| {
                    memory_error(
                        path.clone(),
                        FilesystemOperation::AppendFile,
                        "memory document content must be UTF-8",
                    )
                })?;
                let enforcement = enforce_prompt_write_safety(
                    self.prompt_safety_policy.as_ref(),
                    self.prompt_safety_event_sink.as_ref(),
                    &self.prompt_protected_path_registry,
                    PromptWriteSafetyCheck {
                        scope: context.scope(),
                        path: &document_path,
                        operation: PromptWriteOperation::Append,
                        source: PromptWriteSource::MemoryFilesystemAdapter,
                        content,
                        previous_content_hash: previous_prompt_hash.as_deref(),
                        allowance: context.prompt_write_safety_allowance(),
                        filesystem_operation: FilesystemOperation::AppendFile,
                    },
                )
                .await?;
                backend_context =
                    memory_context_with_prompt_safety_enforcement(&context, enforcement);
            }
            match self
                .backend
                .compare_and_append_document(
                    &backend_context,
                    &document_path,
                    expected_previous_hash.as_deref(),
                    bytes,
                )
                .await?
            {
                MemoryAppendOutcome::Appended => return Ok(()),
                MemoryAppendOutcome::Conflict => continue,
            }
        }
        Err(memory_append_conflict_error(path.clone()))
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

/// [`RootFilesystem`] backend exposing DB-backed memory documents as virtual files.
pub struct MemoryDocumentFilesystem {
    repository: Arc<dyn MemoryDocumentRepository>,
    indexer: Option<Arc<dyn MemoryDocumentIndexer>>,
    prompt_safety_policy: Option<Arc<dyn PromptWriteSafetyPolicy>>,
    prompt_safety_event_sink: Option<Arc<dyn PromptWriteSafetyEventSink>>,
    prompt_protected_path_registry: PromptProtectedPathRegistry,
    one_shot_prompt_safety_allowance: Mutex<Option<PromptSafetyAllowanceId>>,
}

impl MemoryDocumentFilesystem {
    pub fn new<R>(repository: Arc<R>) -> Self
    where
        R: MemoryDocumentRepository + 'static,
    {
        let repository: Arc<dyn MemoryDocumentRepository> = repository;
        Self::from_dyn(repository)
    }

    pub fn from_dyn(repository: Arc<dyn MemoryDocumentRepository>) -> Self {
        let registry = PromptProtectedPathRegistry::default();
        Self {
            repository,
            indexer: None,
            prompt_safety_policy: Some(Arc::new(DefaultPromptWriteSafetyPolicy::with_registry(
                registry.clone(),
            ))),
            prompt_safety_event_sink: None,
            prompt_protected_path_registry: registry,
            one_shot_prompt_safety_allowance: Mutex::new(None),
        }
    }

    pub fn with_indexer<I>(mut self, indexer: Arc<I>) -> Self
    where
        I: MemoryDocumentIndexer + 'static,
    {
        self.indexer = Some(indexer);
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

    /// Installs an explicit prompt-write safety allowance for the next protected write only.
    ///
    /// The allowance is consumed before policy evaluation so shared filesystem adapters cannot
    /// accidentally retain a bypass for later unrelated callers.
    pub fn with_one_shot_prompt_write_safety_allowance(
        self,
        allowance: PromptSafetyAllowanceId,
    ) -> Self {
        if let Ok(mut slot) = self.one_shot_prompt_safety_allowance.lock() {
            *slot = Some(allowance);
        }
        self
    }

    pub fn with_prompt_protected_path_registry(
        mut self,
        registry: PromptProtectedPathRegistry,
    ) -> Self {
        self.prompt_protected_path_registry = registry;
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
        let is_protected = prompt_write_protected_classification(
            self.prompt_safety_policy.as_ref(),
            &self.prompt_protected_path_registry,
            &document_path,
        )
        .is_some();
        let prompt_safety_allowance = if is_protected {
            take_prompt_safety_allowance(
                &self.one_shot_prompt_safety_allowance,
                path,
                FilesystemOperation::WriteFile,
            )?
        } else {
            None
        };
        let metadata = resolve_document_metadata(self.repository.as_ref(), &document_path).await?;
        let mut content_for_schema = None;
        if is_protected {
            let content = std::str::from_utf8(bytes).map_err(|_| {
                memory_error(
                    path.clone(),
                    FilesystemOperation::WriteFile,
                    "memory document content must be UTF-8",
                )
            })?;
            content_for_schema = Some(content);
            let previous_hash = if prompt_write_policy_requires_previous_content_hash(
                self.prompt_safety_policy.as_ref(),
            ) {
                self.repository
                    .read_document(&document_path)
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
                    scope: document_path.scope(),
                    path: &document_path,
                    operation: PromptWriteOperation::Write,
                    source: PromptWriteSource::MemoryDocumentFilesystem,
                    content,
                    previous_content_hash: previous_hash.as_deref(),
                    allowance: prompt_safety_allowance.as_ref(),
                    filesystem_operation: FilesystemOperation::WriteFile,
                },
            )
            .await?;
        }
        if let Some(schema) = &metadata.schema {
            let content = match content_for_schema {
                Some(content) => content,
                None => std::str::from_utf8(bytes).map_err(|_| {
                    memory_error(
                        path.clone(),
                        FilesystemOperation::WriteFile,
                        "memory document content must be UTF-8",
                    )
                })?,
            };
            validate_content_against_schema(&document_path, content, schema)?;
        }
        let options = MemoryWriteOptions {
            metadata,
            changed_by: Some(scoped_memory_owner_key(document_path.scope())),
        };
        self.repository
            .write_document_with_options(&document_path, bytes, &options)
            .await?;
        if let Some(indexer) = &self.indexer {
            let _ = indexer.reindex_document(&document_path).await;
        }
        Ok(())
    }

    async fn append_file(&self, path: &VirtualPath, bytes: &[u8]) -> Result<(), FilesystemError> {
        let document_path = self.parse_file_path(path, FilesystemOperation::AppendFile)?;
        let is_protected = prompt_write_protected_classification(
            self.prompt_safety_policy.as_ref(),
            &self.prompt_protected_path_registry,
            &document_path,
        )
        .is_some();
        let prompt_safety_allowance = if is_protected {
            take_prompt_safety_allowance(
                &self.one_shot_prompt_safety_allowance,
                path,
                FilesystemOperation::AppendFile,
            )?
        } else {
            None
        };
        let metadata = resolve_document_metadata(self.repository.as_ref(), &document_path).await?;
        let options = MemoryWriteOptions {
            metadata,
            changed_by: Some(scoped_memory_owner_key(document_path.scope())),
        };
        for _ in 0..MAX_MEMORY_APPEND_RETRIES {
            let previous = self.repository.read_document(&document_path).await?;
            let expected_previous_hash = previous.as_deref().map(content_bytes_sha256);
            let previous_bytes = previous.unwrap_or_default();
            let previous_prompt_hash = if is_protected
                && prompt_write_policy_requires_previous_content_hash(
                    self.prompt_safety_policy.as_ref(),
                ) {
                std::str::from_utf8(&previous_bytes)
                    .ok()
                    .map(content_sha256)
            } else {
                None
            };
            let mut combined = previous_bytes;
            combined.extend_from_slice(bytes);
            let mut content_for_schema = None;
            if is_protected {
                let content = std::str::from_utf8(&combined).map_err(|_| {
                    memory_error(
                        path.clone(),
                        FilesystemOperation::AppendFile,
                        "memory document content must be UTF-8",
                    )
                })?;
                content_for_schema = Some(content);
                enforce_prompt_write_safety(
                    self.prompt_safety_policy.as_ref(),
                    self.prompt_safety_event_sink.as_ref(),
                    &self.prompt_protected_path_registry,
                    PromptWriteSafetyCheck {
                        scope: document_path.scope(),
                        path: &document_path,
                        operation: PromptWriteOperation::Append,
                        source: PromptWriteSource::MemoryDocumentFilesystem,
                        content,
                        previous_content_hash: previous_prompt_hash.as_deref(),
                        allowance: prompt_safety_allowance.as_ref(),
                        filesystem_operation: FilesystemOperation::AppendFile,
                    },
                )
                .await?;
            }
            if let Some(schema) = &options.metadata.schema {
                let content = match content_for_schema {
                    Some(content) => content,
                    None => std::str::from_utf8(&combined).map_err(|_| {
                        memory_error(
                            path.clone(),
                            FilesystemOperation::AppendFile,
                            "memory document content must be UTF-8",
                        )
                    })?,
                };
                validate_content_against_schema(&document_path, content, schema)?;
            }
            match self
                .repository
                .compare_and_append_document_with_options(
                    &document_path,
                    expected_previous_hash.as_deref(),
                    bytes,
                    &options,
                )
                .await?
            {
                MemoryAppendOutcome::Appended => {
                    if let Some(indexer) = &self.indexer {
                        let _ = indexer.reindex_document(&document_path).await;
                    }
                    return Ok(());
                }
                MemoryAppendOutcome::Conflict => continue,
            }
        }
        Err(memory_append_conflict_error(path.clone()))
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
