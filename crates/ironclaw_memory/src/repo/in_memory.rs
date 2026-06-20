//! In-memory memory document repository for tests and examples.

use std::collections::BTreeMap;
use std::sync::Mutex;

use async_trait::async_trait;
use ironclaw_filesystem::{FilesystemError, FilesystemOperation};

use crate::chunking::{MemoryChunkWrite, content_bytes_sha256, content_sha256};
use crate::indexer::{MemoryChunkReplaceOutcome, MemoryDocumentIndexRepository};
use crate::metadata::{MemoryWriteOptions, resolve_document_metadata};
use crate::path::{MemoryDocumentPath, MemoryDocumentScope, memory_error, valid_memory_path};
use crate::search::{
    MemorySearchRequest, MemorySearchResult, RankedMemorySearchResult, fuse_memory_search_results,
};

use super::{
    MemoryAppendOutcome, MemoryDocumentRepository, MemoryWriteOutcome,
    ensure_document_path_does_not_conflict, rank_search_results_with_learning_metadata,
};

/// In-memory memory document repository for tests and examples.
#[derive(Default)]
pub struct InMemoryMemoryDocumentRepository {
    documents: Mutex<BTreeMap<MemoryDocumentPath, Vec<u8>>>,
    metadata: Mutex<BTreeMap<MemoryDocumentPath, serde_json::Value>>,
    chunks: Mutex<BTreeMap<MemoryDocumentPath, Vec<MemoryChunkWrite>>>,
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

    async fn compare_and_append_document_with_options(
        &self,
        path: &MemoryDocumentPath,
        expected_previous_hash: Option<&str>,
        bytes: &[u8],
        options: &MemoryWriteOptions,
    ) -> Result<MemoryAppendOutcome, FilesystemError> {
        let _ = options;
        let mut documents = self.documents.lock().map_err(|_| {
            memory_error(
                path.virtual_path().unwrap_or_else(|_| valid_memory_path()),
                FilesystemOperation::AppendFile,
                "memory document repository lock poisoned",
            )
        })?;
        let current_hash = documents.get(path).map(|bytes| content_bytes_sha256(bytes));
        if current_hash.as_deref() != expected_previous_hash {
            return Ok(MemoryAppendOutcome::Conflict);
        }
        let existing = documents
            .keys()
            .filter(|document| document.scope() == path.scope())
            .cloned()
            .collect::<Vec<_>>();
        ensure_document_path_does_not_conflict(path, &existing, FilesystemOperation::AppendFile)?;
        documents
            .entry(path.clone())
            .or_insert_with(Vec::new)
            .extend_from_slice(bytes);
        Ok(MemoryAppendOutcome::Appended)
    }

    async fn compare_and_write_document_with_options(
        &self,
        path: &MemoryDocumentPath,
        expected_previous_hash: Option<&str>,
        bytes: &[u8],
        options: &MemoryWriteOptions,
    ) -> Result<MemoryWriteOutcome, FilesystemError> {
        let _ = options;
        let mut documents = self.documents.lock().map_err(|_| {
            memory_error(
                path.virtual_path().unwrap_or_else(|_| valid_memory_path()),
                FilesystemOperation::WriteFile,
                "memory document repository lock poisoned",
            )
        })?;
        let current_hash = documents.get(path).map(|bytes| content_bytes_sha256(bytes));
        if current_hash.as_deref() != expected_previous_hash {
            return Ok(MemoryWriteOutcome::Conflict);
        }
        let existing = documents
            .keys()
            .filter(|document| document.scope() == path.scope())
            .cloned()
            .collect::<Vec<_>>();
        ensure_document_path_does_not_conflict(path, &existing, FilesystemOperation::WriteFile)?;
        documents.insert(path.clone(), bytes.to_vec());
        Ok(MemoryWriteOutcome::Written)
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

    async fn search_documents(
        &self,
        scope: &MemoryDocumentScope,
        request: &MemorySearchRequest,
    ) -> Result<Vec<MemorySearchResult>, FilesystemError> {
        let documents = {
            let documents = self.documents.lock().map_err(|_| {
                memory_error(
                    scope
                        .virtual_prefix()
                        .unwrap_or_else(|_| valid_memory_path()),
                    FilesystemOperation::Query,
                    "memory document repository lock poisoned",
                )
            })?;
            documents
                .iter()
                .filter(|(path, _bytes)| path.scope() == scope)
                .map(|(path, bytes)| (path.clone(), bytes.clone()))
                .collect::<Vec<_>>()
        };

        let mut full_text_results = Vec::new();
        if request.full_text() {
            let query = request.query().to_ascii_lowercase();
            for (path, bytes) in documents {
                let metadata = resolve_document_metadata(self, &path).await?;
                if metadata.skip_indexing == Some(true) {
                    continue;
                }
                let content = String::from_utf8_lossy(&bytes).into_owned();
                if !content.to_ascii_lowercase().contains(query.as_str()) {
                    continue;
                }
                full_text_results.push(RankedMemorySearchResult {
                    path,
                    snippet: content,
                    rank: (full_text_results.len() as u32).saturating_add(1),
                });
                if full_text_results.len() >= request.pre_fusion_limit() {
                    break;
                }
            }
        }

        let fusion_request = request.clone().with_limit(request.pre_fusion_limit());
        let fused = fuse_memory_search_results(full_text_results, Vec::new(), &fusion_request);
        rank_search_results_with_learning_metadata(self, request, fused).await
    }
}

#[async_trait]
impl MemoryDocumentIndexRepository for InMemoryMemoryDocumentRepository {
    async fn replace_document_chunks_if_current(
        &self,
        path: &MemoryDocumentPath,
        expected_content_hash: &str,
        chunks: &[MemoryChunkWrite],
    ) -> Result<MemoryChunkReplaceOutcome, FilesystemError> {
        let documents = self.documents.lock().map_err(|_| {
            memory_error(
                path.virtual_path().unwrap_or_else(|_| valid_memory_path()),
                FilesystemOperation::ReadFile,
                "memory document repository lock poisoned",
            )
        })?;
        let Some(bytes) = documents.get(path) else {
            return Ok(MemoryChunkReplaceOutcome::SkippedMissingDocument);
        };
        let current_hash = std::str::from_utf8(bytes)
            .map(content_sha256)
            .map_err(|_| {
                memory_error(
                    path.virtual_path().unwrap_or_else(|_| valid_memory_path()),
                    FilesystemOperation::WriteFile,
                    "memory document content must be UTF-8",
                )
            })?;
        if current_hash != expected_content_hash {
            return Ok(MemoryChunkReplaceOutcome::SkippedStaleContentHash);
        }

        let mut chunk_store = self.chunks.lock().map_err(|_| {
            memory_error(
                path.virtual_path().unwrap_or_else(|_| valid_memory_path()),
                FilesystemOperation::WriteFile,
                "memory document chunk repository lock poisoned",
            )
        })?;
        if chunks.is_empty() {
            chunk_store.remove(path);
        } else {
            chunk_store.insert(path.clone(), chunks.to_vec());
        }
        Ok(MemoryChunkReplaceOutcome::Replaced)
    }
}
