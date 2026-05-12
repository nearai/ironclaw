use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_host_api::VirtualPath;

use crate::{
    BackendCapabilities, BackendId, BackendKind, ContentKind, DirEntry, FileStat, FilesystemError,
    IndexPolicy, RootFilesystem, StorageClass, path_prefix_matches,
};

/// Trusted catalog record for one virtual filesystem mount.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountDescriptor {
    pub virtual_root: VirtualPath,
    pub backend_id: BackendId,
    pub backend_kind: BackendKind,
    pub storage_class: StorageClass,
    pub content_kind: ContentKind,
    pub index_policy: IndexPolicy,
    pub capabilities: BackendCapabilities,
}

/// Catalog answer for the backend that owns a virtual path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathPlacement {
    pub path: VirtualPath,
    pub matched_root: VirtualPath,
    pub backend_id: BackendId,
    pub backend_kind: BackendKind,
    pub storage_class: StorageClass,
    pub content_kind: ContentKind,
    pub index_policy: IndexPolicy,
    pub capabilities: BackendCapabilities,
}

impl PathPlacement {
    fn from_descriptor(path: VirtualPath, descriptor: &MountDescriptor) -> Self {
        Self {
            path,
            matched_root: descriptor.virtual_root.clone(),
            backend_id: descriptor.backend_id.clone(),
            backend_kind: descriptor.backend_kind.clone(),
            storage_class: descriptor.storage_class,
            content_kind: descriptor.content_kind,
            index_policy: descriptor.index_policy,
            capabilities: descriptor.capabilities,
        }
    }
}

/// Trusted catalog over virtual filesystem mount placement.
///
/// The catalog explains where a [`VirtualPath`] is placed; it does not grant
/// runtime access. Untrusted callers must still go through [`ScopedFilesystem`]
/// and a scoped [`MountView`].
#[async_trait]
pub trait FilesystemCatalog: Send + Sync {
    async fn describe_path(&self, path: &VirtualPath) -> Result<PathPlacement, FilesystemError>;

    async fn mounts(&self) -> Result<Vec<MountDescriptor>, FilesystemError>;
}

/// Root filesystem that composes multiple backend roots behind one virtual namespace.
pub struct CompositeRootFilesystem {
    mounts: Vec<CompositeMount>,
}

struct CompositeMount {
    descriptor: MountDescriptor,
    backend: Arc<dyn RootFilesystem>,
}

impl CompositeRootFilesystem {
    pub fn new() -> Self {
        Self { mounts: Vec::new() }
    }

    pub fn mount<F>(
        &mut self,
        descriptor: MountDescriptor,
        backend: Arc<F>,
    ) -> Result<(), FilesystemError>
    where
        F: RootFilesystem + 'static,
    {
        let backend: Arc<dyn RootFilesystem> = backend;
        self.mount_dyn(descriptor, backend)
    }

    pub fn mount_dyn(
        &mut self,
        descriptor: MountDescriptor,
        backend: Arc<dyn RootFilesystem>,
    ) -> Result<(), FilesystemError> {
        if self
            .mounts
            .iter()
            .any(|mount| mount.descriptor.virtual_root.as_str() == descriptor.virtual_root.as_str())
        {
            return Err(FilesystemError::MountConflict {
                path: descriptor.virtual_root,
            });
        }
        self.mounts.push(CompositeMount {
            descriptor,
            backend,
        });
        Ok(())
    }

    fn matching_mount(&self, path: &VirtualPath) -> Result<&CompositeMount, FilesystemError> {
        self.mounts
            .iter()
            .filter(|mount| {
                path_prefix_matches(mount.descriptor.virtual_root.as_str(), path.as_str())
            })
            .max_by_key(|mount| mount.descriptor.virtual_root.as_str().len())
            .ok_or_else(|| FilesystemError::MountNotFound { path: path.clone() })
    }
}

impl Default for CompositeRootFilesystem {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl FilesystemCatalog for CompositeRootFilesystem {
    async fn describe_path(&self, path: &VirtualPath) -> Result<PathPlacement, FilesystemError> {
        let mount = self.matching_mount(path)?;
        Ok(PathPlacement::from_descriptor(
            path.clone(),
            &mount.descriptor,
        ))
    }

    async fn mounts(&self) -> Result<Vec<MountDescriptor>, FilesystemError> {
        let mut mounts: Vec<_> = self
            .mounts
            .iter()
            .map(|mount| mount.descriptor.clone())
            .collect();
        mounts.sort_by(|left, right| left.virtual_root.as_str().cmp(right.virtual_root.as_str()));
        Ok(mounts)
    }
}

#[async_trait]
impl RootFilesystem for CompositeRootFilesystem {
    async fn read_file(&self, path: &VirtualPath) -> Result<Vec<u8>, FilesystemError> {
        self.matching_mount(path)?.backend.read_file(path).await
    }

    async fn write_file(&self, path: &VirtualPath, bytes: &[u8]) -> Result<(), FilesystemError> {
        self.matching_mount(path)?
            .backend
            .write_file(path, bytes)
            .await
    }

    async fn append_file(&self, path: &VirtualPath, bytes: &[u8]) -> Result<(), FilesystemError> {
        self.matching_mount(path)?
            .backend
            .append_file(path, bytes)
            .await
    }

    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        self.matching_mount(path)?.backend.list_dir(path).await
    }

    async fn stat(&self, path: &VirtualPath) -> Result<FileStat, FilesystemError> {
        self.matching_mount(path)?.backend.stat(path).await
    }

    async fn delete(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.matching_mount(path)?.backend.delete(path).await
    }

    async fn create_dir_all(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.matching_mount(path)?
            .backend
            .create_dir_all(path)
            .await
    }
}
