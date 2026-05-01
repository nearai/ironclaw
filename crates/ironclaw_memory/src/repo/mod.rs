//! Memory document repository trait and shared repo helpers.

use std::collections::BTreeMap;

use async_trait::async_trait;
use ironclaw_filesystem::{DirEntry, FileType, FilesystemError, FilesystemOperation};
use ironclaw_host_api::VirtualPath;

use crate::metadata::MemoryWriteOptions;
#[cfg(any(feature = "libsql", feature = "postgres"))]
use crate::path::validated_memory_relative_path;
use crate::path::{
    MemoryDocumentPath, MemoryDocumentScope, memory_backend_unsupported, memory_error,
    memory_not_found, valid_memory_path,
};
use crate::search::{MemorySearchRequest, MemorySearchResult};

mod in_memory;
#[cfg(feature = "libsql")]
mod libsql;
#[cfg(feature = "libsql")]
mod native_libsql;
#[cfg(feature = "postgres")]
mod native_postgres;
#[cfg(feature = "postgres")]
mod postgres;

pub use in_memory::InMemoryMemoryDocumentRepository;
#[cfg(feature = "libsql")]
pub use libsql::LibSqlMemoryDocumentRepository;
#[cfg(feature = "libsql")]
pub use native_libsql::RebornLibSqlMemoryDocumentRepository;
#[cfg(feature = "postgres")]
pub use native_postgres::RebornPostgresMemoryDocumentRepository;
#[cfg(feature = "postgres")]
pub use postgres::PostgresMemoryDocumentRepository;

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

pub(crate) fn scoped_memory_owner_key(scope: &MemoryDocumentScope) -> String {
    format!(
        "tenant:{}:user:{}:project:{}",
        scope.tenant_id(),
        scope.user_id(),
        scope.project_id().unwrap_or("_none")
    )
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
pub(crate) fn scoped_memory_agent_id(scope: &MemoryDocumentScope) -> Option<&str> {
    scope.agent_id()
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
pub(crate) fn db_path_for_memory_document(path: &MemoryDocumentPath) -> String {
    path.relative_path().to_string()
}

#[cfg(any(feature = "libsql", feature = "postgres"))]
pub(crate) fn memory_document_from_db_path(
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

pub(crate) fn ensure_document_path_does_not_conflict(
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

pub(crate) fn memory_direct_children(
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
