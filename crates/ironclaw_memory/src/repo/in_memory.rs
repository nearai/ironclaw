//! In-memory memory document repository for tests and examples.

use std::collections::BTreeMap;
use std::sync::Mutex;

use async_trait::async_trait;
use ironclaw_filesystem::{FilesystemError, FilesystemOperation};

use crate::chunking::content_bytes_sha256;
use crate::metadata::MemoryWriteOptions;
use crate::path::{MemoryDocumentPath, MemoryDocumentScope, memory_error, valid_memory_path};

use super::{
    MemoryAppendOutcome, MemoryDocumentRepository, ensure_document_path_does_not_conflict,
};

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
