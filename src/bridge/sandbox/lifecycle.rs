//! Per-project sandbox container lifecycle.
//!
//! Wraps `bollard` calls to `docker create` / `docker start` / `docker stop`
//! / `docker rm` so the rest of the bridge can speak in `(project_id)`
//! terms instead of container ids. Naming is deterministic
//! (`ironclaw-sandbox-<project_id>`) so multiple IronClaw runs against the
//! same project re-use the same container — that's how installed
//! dependencies and build caches accumulate over the project's lifetime.

use std::collections::HashMap;
use std::path::Path;

use bollard::Docker;
use bollard::container::{
    Config, CreateContainerOptions, ListContainersOptions, RemoveContainerOptions,
    StartContainerOptions, StopContainerOptions,
};
use bollard::models::{HostConfig, Mount, MountTypeEnum};
use ironclaw_engine::{MountError, ProjectId};
use tracing::{debug, warn};

/// Default image. Override with `IRONCLAW_SANDBOX_IMAGE`.
pub const DEFAULT_IMAGE: &str = "ironclaw/sandbox:dev";

/// Stop timeout in seconds before SIGKILL.
#[allow(dead_code)]
const STOP_TIMEOUT_SECS: i64 = 10;

/// Resolve the configured sandbox image, falling back to the default.
pub fn sandbox_image() -> String {
    std::env::var("IRONCLAW_SANDBOX_IMAGE").unwrap_or_else(|_| DEFAULT_IMAGE.to_string())
}

/// Build the deterministic container name for a project.
pub fn container_name_for(project_id: ProjectId) -> String {
    format!("ironclaw-sandbox-{}", project_id.0)
}

/// Ensure that a container exists and is running for `project_id`. Creates
/// it if necessary, starts it if it's stopped, returns the container id
/// either way. The bind-mount source is `host_workspace_path` and is
/// expected to already exist on disk (the caller — the mount factory in
/// Phase 6 — runs `ensure_project_workspace_dir` first).
pub async fn ensure_running(
    docker: &Docker,
    project_id: ProjectId,
    host_workspace_path: &Path,
) -> Result<String, MountError> {
    let name = container_name_for(project_id);

    // Look up by name.
    let mut filters = HashMap::new();
    filters.insert("name".to_string(), vec![format!("^/{name}$")]);
    let existing = docker
        .list_containers(Some(ListContainersOptions {
            all: true,
            filters,
            ..Default::default()
        }))
        .await
        .map_err(|e| MountError::Backend {
            reason: format!("list_containers({name}): {e}"),
        })?;

    let container_id = if let Some(c) = existing.into_iter().next() {
        let id = c.id.ok_or_else(|| MountError::Backend {
            reason: format!("container {name} exists but Docker returned no ID"),
        })?;
        let state = c.state.unwrap_or_default();
        if state != "running" {
            debug!(container = %id, state = %state, "starting existing sandbox container");
            docker
                .start_container(&id, None::<StartContainerOptions<String>>)
                .await
                .map_err(|e| MountError::Backend {
                    reason: format!("start_container({name}): {e}"),
                })?;
        }
        id
    } else {
        debug!(project_id = %project_id, "creating sandbox container");
        create_container(docker, &name, host_workspace_path).await?
    };

    Ok(container_id)
}

async fn create_container(
    docker: &Docker,
    name: &str,
    host_workspace_path: &Path,
) -> Result<String, MountError> {
    let image = sandbox_image();
    let host_str = host_workspace_path
        .canonicalize()
        .unwrap_or_else(|_| host_workspace_path.to_path_buf())
        .display()
        .to_string();

    let mounts = vec![Mount {
        target: Some("/project".into()),
        source: Some(host_str),
        typ: Some(MountTypeEnum::BIND),
        read_only: Some(false),
        ..Default::default()
    }];

    let host_config = HostConfig {
        mounts: Some(mounts),
        // Default Docker bridge networking so the container can reach the
        // internet for `git clone`, `cargo build`, `pip install`, etc.
        // Outbound network restriction (domain allowlist via the existing
        // proxy in `src/sandbox/proxy/`) is a follow-up; until then the
        // container has the same outbound access as the host.
        ..Default::default()
    };

    let config = Config {
        image: Some(image.clone()),
        cmd: Some(vec!["sleep".into(), "infinity".into()]),
        working_dir: Some("/project".into()),
        host_config: Some(host_config),
        attach_stdin: Some(false),
        attach_stdout: Some(false),
        attach_stderr: Some(false),
        tty: Some(false),
        ..Default::default()
    };

    let create = docker
        .create_container(
            Some(CreateContainerOptions {
                name: name.to_string(),
                platform: None,
            }),
            config,
        )
        .await
        .map_err(|e| MountError::Backend {
            reason: format!("create_container({name}, image={image}): {e}"),
        })?;

    docker
        .start_container(&create.id, None::<StartContainerOptions<String>>)
        .await
        .map_err(|e| MountError::Backend {
            reason: format!("start_container({name}): {e}"),
        })?;

    Ok(create.id)
}

/// Stop a project's container without removing it. Idempotent. Errors are
/// logged but not propagated — stop is best-effort and the next start
/// (which goes through `ensure_running`) will recover regardless.
#[allow(dead_code)]
pub async fn stop(docker: &Docker, project_id: ProjectId) {
    let name = container_name_for(project_id);
    if let Err(e) = docker
        .stop_container(
            &name,
            Some(StopContainerOptions {
                t: STOP_TIMEOUT_SECS,
            }),
        )
        .await
    {
        warn!(container = %name, error = %e, "stop_container failed");
    }
}

/// Remove a project's container entirely. Used by `project delete` /
/// "reset environment". The bind-mounted host directory is left untouched.
#[allow(dead_code)]
pub async fn remove(docker: &Docker, project_id: ProjectId) {
    let name = container_name_for(project_id);
    if let Err(e) = docker
        .remove_container(
            &name,
            Some(RemoveContainerOptions {
                force: true,
                v: false,
                ..Default::default()
            }),
        )
        .await
    {
        warn!(container = %name, error = %e, "remove_container failed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn container_name_is_deterministic() {
        let pid = ProjectId::new();
        let n1 = container_name_for(pid);
        let n2 = container_name_for(pid);
        assert_eq!(n1, n2);
        assert!(n1.starts_with("ironclaw-sandbox-"));
    }
}
