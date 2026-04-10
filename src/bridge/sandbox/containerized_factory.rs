//! Containerized [`ProjectMountFactory`] backed by [`ProjectSandboxManager`].
//!
//! This factory is selected when `ENGINE_V2_SANDBOX=true`. For each project
//! it asks the manager for a transport (which lazily creates the container
//! and the daemon exec session) and wraps it in a
//! [`ContainerizedFilesystemBackend`] registered at `/project/`.
//!
//! Like [`super::filesystem_factory::FilesystemMountFactory`] it takes a
//! [`ProjectPathResolver`] so it stays decoupled from the engine `Store` and
//! testable without one.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_engine::{MountError, ProjectId, ProjectMountFactory, ProjectMounts};
use tracing::debug;

use super::containerized_backend::ContainerizedFilesystemBackend;
use super::filesystem_factory::{PROJECT_MOUNT_PREFIX, ProjectPathResolver};
use super::manager::ProjectSandboxManager;

/// [`ProjectMountFactory`] that asks a shared [`ProjectSandboxManager`] for
/// a transport per project and wraps it in a [`ContainerizedFilesystemBackend`].
pub struct ContainerizedMountFactory {
    manager: Arc<ProjectSandboxManager>,
    resolver: ProjectPathResolver,
}

impl std::fmt::Debug for ContainerizedMountFactory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ContainerizedMountFactory").finish()
    }
}

impl ContainerizedMountFactory {
    pub fn new(manager: Arc<ProjectSandboxManager>, resolver: ProjectPathResolver) -> Self {
        Self { manager, resolver }
    }
}

#[async_trait]
impl ProjectMountFactory for ContainerizedMountFactory {
    async fn build(&self, project_id: ProjectId) -> Result<ProjectMounts, MountError> {
        let host_path = (self.resolver)(project_id).await?;
        debug!(
            project_id = %project_id,
            host_path = %host_path.display(),
            "ContainerizedMountFactory: starting per-project sandbox"
        );
        let transport = self.manager.transport_for(project_id, host_path).await?;
        let backend = Arc::new(ContainerizedFilesystemBackend::new(transport));
        let mut mounts = ProjectMounts::new();
        mounts.add(PROJECT_MOUNT_PREFIX, backend);
        Ok(mounts)
    }
}
