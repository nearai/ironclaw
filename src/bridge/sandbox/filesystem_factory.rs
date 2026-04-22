//! Default [`ProjectMountFactory`] backed by the host filesystem.
//!
//! This factory is used when the per-project sandbox container is **not**
//! enabled (i.e. `SANDBOX_ENABLED` is unset). For each project it builds
//! a [`ProjectMounts`] table with `/project/` pointed at the host directory
//! for that project.
//!
//! The factory is decoupled from `Store` via a [`ProjectPathResolver`]
//! closure so it can be unit-tested without a full store mock and so the
//! same factory works whether the project record is loaded from the engine
//! store, computed from a default, or stubbed in tests.

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_engine::workspace::FilesystemBackend;
use ironclaw_engine::{MountError, ProjectId, ProjectMountFactory, ProjectMounts};
use tracing::debug;

/// `/project/` is the canonical agent-facing prefix for the user's project
/// files. The same name is used by every backend (filesystem today,
/// containerized in Phase 5+) so swapping backends is invisible to the agent.
pub const PROJECT_MOUNT_PREFIX: &str = "/project/";

/// Resolves a project id to a host filesystem path. Implementations are
/// responsible for any directory creation. Returning an error short-circuits
/// the factory and the engine surfaces a backend error.
pub type ProjectPathResolver = Arc<
    dyn Fn(ProjectId) -> Pin<Box<dyn Future<Output = Result<PathBuf, MountError>> + Send>>
        + Send
        + Sync,
>;

/// Builds [`ProjectMounts`] backed by the host filesystem.
pub struct FilesystemMountFactory {
    resolver: ProjectPathResolver,
}

impl std::fmt::Debug for FilesystemMountFactory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FilesystemMountFactory").finish()
    }
}

impl FilesystemMountFactory {
    pub fn new(resolver: ProjectPathResolver) -> Self {
        Self { resolver }
    }
}

#[async_trait]
impl ProjectMountFactory for FilesystemMountFactory {
    async fn build(&self, project_id: ProjectId) -> Result<ProjectMounts, MountError> {
        let path = (self.resolver)(project_id).await?;
        debug!(
            project_id = %project_id,
            path = %path.display(),
            "FilesystemMountFactory: built /project/ mount"
        );
        let mut mounts = ProjectMounts::new();
        mounts.add(PROJECT_MOUNT_PREFIX, Arc::new(FilesystemBackend::new(path)));
        Ok(mounts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[tokio::test]
    async fn factory_uses_resolver_path() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();

        let resolver: ProjectPathResolver = {
            let root = root.clone();
            Arc::new(move |_pid| {
                let root = root.clone();
                Box::pin(async move { Ok(root) })
            })
        };
        let factory = FilesystemMountFactory::new(resolver);
        let mounts = factory.build(ProjectId::new()).await.unwrap();
        let (backend, rel) = mounts.resolve("/project/foo.txt").expect("should resolve");
        assert_eq!(rel, Path::new("foo.txt"));
        backend.write(&rel, b"hi").await.unwrap();
        assert_eq!(backend.read(&rel).await.unwrap(), b"hi");
        assert!(root.join("foo.txt").exists());
    }

    #[tokio::test]
    async fn resolver_error_propagates() {
        let resolver: ProjectPathResolver = Arc::new(|_pid| {
            Box::pin(async {
                Err(MountError::Backend {
                    reason: "no such project".into(),
                })
            })
        });
        let factory = FilesystemMountFactory::new(resolver);
        let err = factory.build(ProjectId::new()).await.unwrap_err();
        assert!(matches!(err, MountError::Backend { .. }));
    }
}
