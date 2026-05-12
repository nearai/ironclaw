use std::sync::Arc;

use ironclaw_host_api::{MountPermissions, MountView, ScopedPath, VirtualPath};

use crate::{DirEntry, FileStat, FilesystemError, FilesystemOperation, RootFilesystem};

/// Invocation-scoped filesystem view over [`ScopedPath`] values.
#[derive(Debug, Clone)]
pub struct ScopedFilesystem<F> {
    root: Arc<F>,
    mounts: MountView,
}

impl<F> ScopedFilesystem<F>
where
    F: RootFilesystem,
{
    pub fn new(root: Arc<F>, mounts: MountView) -> Self {
        Self { root, mounts }
    }

    pub fn mounts(&self) -> &MountView {
        &self.mounts
    }

    pub async fn read_file(&self, path: &ScopedPath) -> Result<Vec<u8>, FilesystemError> {
        let virtual_path = self.resolve_with_permission(path, FilesystemOperation::ReadFile)?;
        self.root.read_file(&virtual_path).await
    }

    pub async fn write_file(&self, path: &ScopedPath, bytes: &[u8]) -> Result<(), FilesystemError> {
        let virtual_path = self.resolve_with_permission(path, FilesystemOperation::WriteFile)?;
        self.root.write_file(&virtual_path, bytes).await
    }

    pub async fn append_file(
        &self,
        path: &ScopedPath,
        bytes: &[u8],
    ) -> Result<(), FilesystemError> {
        let virtual_path = self.resolve_with_permission(path, FilesystemOperation::AppendFile)?;
        self.root.append_file(&virtual_path, bytes).await
    }

    pub async fn list_dir(&self, path: &ScopedPath) -> Result<Vec<DirEntry>, FilesystemError> {
        let virtual_path = self.resolve_with_permission(path, FilesystemOperation::ListDir)?;
        self.root.list_dir(&virtual_path).await
    }

    pub async fn stat(&self, path: &ScopedPath) -> Result<FileStat, FilesystemError> {
        let virtual_path = self.resolve_with_permission(path, FilesystemOperation::Stat)?;
        self.root.stat(&virtual_path).await
    }

    pub async fn delete(&self, path: &ScopedPath) -> Result<(), FilesystemError> {
        let virtual_path = self.resolve_with_permission(path, FilesystemOperation::Delete)?;
        self.root.delete(&virtual_path).await
    }

    pub async fn create_dir_all(&self, path: &ScopedPath) -> Result<(), FilesystemError> {
        let virtual_path = self.resolve_with_permission(path, FilesystemOperation::CreateDirAll)?;
        self.root.create_dir_all(&virtual_path).await
    }

    fn resolve_with_permission(
        &self,
        path: &ScopedPath,
        operation: FilesystemOperation,
    ) -> Result<VirtualPath, FilesystemError> {
        let (virtual_path, grant) = self.mounts.resolve_with_grant(path)?;

        if !operation_allowed(&grant.permissions, operation) {
            return Err(FilesystemError::PermissionDenied {
                path: path.clone(),
                operation,
            });
        }

        Ok(virtual_path)
    }
}

fn operation_allowed(permissions: &MountPermissions, operation: FilesystemOperation) -> bool {
    match operation {
        FilesystemOperation::ReadFile => permissions.read,
        FilesystemOperation::WriteFile => permissions.write,
        FilesystemOperation::AppendFile => permissions.write,
        FilesystemOperation::ListDir => permissions.list,
        // Stat is metadata-only: either read authority or list authority reveals
        // equivalent existence/type information without file contents.
        FilesystemOperation::Stat => permissions.read || permissions.list,
        FilesystemOperation::Delete => permissions.delete,
        FilesystemOperation::CreateDirAll => permissions.write,
        FilesystemOperation::MountLocal => false,
    }
}
