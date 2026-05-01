//! In-memory memory document repository for tests and examples.

use std::collections::BTreeMap;
use std::sync::Mutex;

use async_trait::async_trait;
use ironclaw_filesystem::{FilesystemError, FilesystemOperation};

use crate::path::{MemoryDocumentPath, MemoryDocumentScope, memory_error, valid_memory_path};

use super::{MemoryDocumentRepository, ensure_document_path_does_not_conflict};

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
