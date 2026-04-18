//! [`ProjectSandboxManager`] — owns one [`DockerTransport`] per project.
//!
//! Lazily creates the per-project sandbox container on first use, hands out
//! a shared [`SandboxTransport`] handle that the project's
//! [`ContainerizedFilesystemBackend`] dispatches into, and exposes lifecycle
//! hooks (`shutdown_project`, `shutdown_all`) for engine teardown.
//!
//! The manager is the single owner of `bollard::Docker` so all sandbox
//! activity routes through one connection. The Phase 6 router constructs
//! exactly one `ProjectSandboxManager` and shares it across all projects.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use bollard::Docker;
use ironclaw_engine::{MountError, ProjectId};
use tokio::sync::Mutex;
use tracing::debug;

use super::docker_transport::DockerTransport;
use super::lifecycle;
use super::transport::SandboxTransport;

/// One process-wide manager that vends sandbox transports per project.
pub struct ProjectSandboxManager {
    docker: Docker,
    transports: Mutex<HashMap<ProjectId, Arc<DockerTransport>>>,
}

impl std::fmt::Debug for ProjectSandboxManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProjectSandboxManager").finish()
    }
}

impl ProjectSandboxManager {
    pub fn new(docker: Docker) -> Self {
        Self {
            docker,
            transports: Mutex::new(HashMap::new()),
        }
    }

    /// Get-or-create the transport for `project_id`. The first call ensures
    /// the container is running and starts a `docker exec` session into the
    /// daemon; subsequent calls return the cached handle.
    ///
    /// Uses double-checked locking so the mutex is *not* held across the
    /// Docker `ensure_running` await — concurrent calls for different
    /// projects proceed in parallel instead of head-of-line blocking.
    pub async fn transport_for(
        &self,
        project_id: ProjectId,
        host_workspace_path: PathBuf,
    ) -> Result<Arc<dyn SandboxTransport>, MountError> {
        // Fast path: return cached transport without holding the lock
        // across any I/O.
        {
            let guard = self.transports.lock().await;
            if let Some(existing) = guard.get(&project_id) {
                return Ok(existing.clone() as Arc<dyn SandboxTransport>);
            }
        }

        // Slow path: create the container and transport *without* holding
        // the lock, so other projects aren't blocked by Docker latency.
        let container_id =
            lifecycle::ensure_running(&self.docker, project_id, &host_workspace_path).await?;
        debug!(
            project_id = %project_id,
            container_id = %container_id,
            "ProjectSandboxManager: created sandbox transport"
        );
        let transport = Arc::new(DockerTransport::new(self.docker.clone(), container_id));

        // Re-lock and insert. If another task raced us for the same
        // project_id, use their transport (first-writer-wins) — the extra
        // container is harmless and will be reaped by the idle reaper.
        let mut guard = self.transports.lock().await;
        let entry = guard.entry(project_id).or_insert_with(|| transport.clone());
        Ok(entry.clone() as Arc<dyn SandboxTransport>)
    }

    /// Stop and forget the cached transport for `project_id`. The container
    /// itself is left around (still on disk) so the next call resumes
    /// quickly. Use [`Self::reset_project`] for full removal.
    #[allow(dead_code)]
    pub async fn shutdown_project(&self, project_id: ProjectId) {
        let mut guard = self.transports.lock().await;
        if guard.remove(&project_id).is_some() {
            lifecycle::stop(&self.docker, project_id).await;
        }
    }

    /// Stop the container *and* remove it from Docker. Used by project
    /// deletion / explicit user reset. The host workspace directory stays
    /// untouched — it's the user's data, not the sandbox's.
    #[allow(dead_code)]
    pub async fn reset_project(&self, project_id: ProjectId) {
        let mut guard = self.transports.lock().await;
        guard.remove(&project_id);
        lifecycle::stop(&self.docker, project_id).await;
        lifecycle::remove(&self.docker, project_id).await;
    }

    /// Stop every cached transport. Called at engine teardown.
    #[allow(dead_code)]
    pub async fn shutdown_all(&self) {
        let mut guard = self.transports.lock().await;
        let pids: Vec<ProjectId> = guard.keys().copied().collect();
        guard.clear();
        for pid in pids {
            lifecycle::stop(&self.docker, pid).await;
        }
    }
}
