use async_trait::async_trait;
use ironclaw_host_api::VirtualPath;

use crate::{DirEntry, FileStat, FilesystemError, FilesystemOperation};

/// Trusted root filesystem interface over canonical virtual paths.
#[async_trait]
pub trait RootFilesystem: Send + Sync {
    /// Reads a file by canonical virtual path without exposing backend host paths in errors.
    async fn read_file(&self, path: &VirtualPath) -> Result<Vec<u8>, FilesystemError>;

    /// Writes bytes to a canonical virtual path while preserving backend containment.
    async fn write_file(&self, path: &VirtualPath, bytes: &[u8]) -> Result<(), FilesystemError>;

    /// Appends bytes to a canonical virtual path. Backends that do not support append must fail closed before side effects.
    async fn append_file(&self, path: &VirtualPath, _bytes: &[u8]) -> Result<(), FilesystemError> {
        Err(FilesystemError::Backend {
            path: path.clone(),
            operation: FilesystemOperation::AppendFile,
            reason: "append_file is not supported by this backend".to_string(),
        })
    }

    /// Lists direct children of a canonical virtual directory; callers must handle pagination/backends in future implementations without bypassing scope.
    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError>;

    /// Returns metadata for a canonical virtual path without revealing raw host paths.
    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError>;

    /// Deletes an existing canonical virtual file or directory. Missing paths return [`FilesystemError::NotFound`]; backends that do not support delete must fail closed before side effects.
    async fn delete(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        Err(FilesystemError::Backend {
            path: path.clone(),
            operation: FilesystemOperation::Delete,
            reason: "delete is not supported by this backend".to_string(),
        })
    }

    /// Creates a canonical virtual directory and any missing parents. Backends that do not support directories must fail closed before side effects.
    async fn create_dir_all(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        Err(FilesystemError::Backend {
            path: path.clone(),
            operation: FilesystemOperation::CreateDirAll,
            reason: "create_dir_all is not supported by this backend".to_string(),
        })
    }
}
