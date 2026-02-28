//! Docker container lifecycle management.
//!
//! Handles creating, running, and cleaning up containers for sandboxed execution.
//!
//! # Container Setup
//!
//! ```text
//! ┌────────────────────────────────────────────────────────────────────────┐
//! │                          Docker Container                               │
//! │                                                                         │
//! │  Environment:                                                           │
//! │    http_proxy=http://host.docker.internal:PORT                          │
//! │    https_proxy=http://host.docker.internal:PORT                         │
//! │    (No secrets or credentials)                                          │
//! │                                                                         │
//! │  Mounts:                                                                │
//! │    /workspace ─▶ Host working directory (ro or rw based on policy)     │
//! │    /output    ─▶ Output directory for artifacts (rw)                   │
//! │                                                                         │
//! │  Limits:                                                                │
//! │    Memory: 2GB (default)                                                │
//! │    CPU: 1024 shares                                                     │
//! │    No privileged mode                                                   │
//! │    Non-root user (UID 1000)                                             │
//! └────────────────────────────────────────────────────────────────────────┘
//! ```

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use bollard::Docker;
use bollard::container::{
    Config, CreateContainerOptions, LogOutput, LogsOptions, RemoveContainerOptions,
    StartContainerOptions, WaitContainerOptions,
};
use bollard::exec::{CreateExecOptions, StartExecResults};
use bollard::models::HostConfig;
use futures::StreamExt;

use crate::sandbox::config::{ResourceLimits, SandboxPolicy};
use crate::sandbox::error::{Result, SandboxError};

/// Output from container execution.
#[derive(Debug, Clone)]
pub struct ContainerOutput {
    /// Exit code from the command.
    pub exit_code: i64,
    /// Standard output.
    pub stdout: String,
    /// Standard error.
    pub stderr: String,
    /// How long the command ran.
    pub duration: Duration,
    /// Whether output was truncated.
    pub truncated: bool,
}

/// Manages Docker container lifecycle.
pub struct ContainerRunner {
    docker: Docker,
    image: String,
    proxy_port: u16,
}

impl ContainerRunner {
    /// Create a new container runner.
    pub fn new(docker: Docker, image: String, proxy_port: u16) -> Self {
        Self {
            docker,
            image,
            proxy_port,
        }
    }

    /// Check if the Docker daemon is available.
    pub async fn is_available(&self) -> bool {
        self.docker.ping().await.is_ok()
    }

    /// Check if the sandbox image exists locally.
    pub async fn image_exists(&self) -> bool {
        self.docker.inspect_image(&self.image).await.is_ok()
    }

    /// Pull the sandbox image.
    pub async fn pull_image(&self) -> Result<()> {
        use bollard::image::CreateImageOptions;

        tracing::info!("Pulling sandbox image: {}", self.image);

        let options = CreateImageOptions {
            from_image: self.image.clone(),
            ..Default::default()
        };

        let mut stream = self.docker.create_image(Some(options), None, None);

        while let Some(result) = stream.next().await {
            match result {
                Ok(info) => {
                    if let Some(status) = info.status {
                        tracing::debug!("Pull status: {}", status);
                    }
                }
                Err(e) => {
                    return Err(SandboxError::ContainerCreationFailed {
                        reason: format!("image pull failed: {}", e),
                    });
                }
            }
        }

        tracing::info!("Successfully pulled image: {}", self.image);
        Ok(())
    }

    /// Execute a command in a new container.
    pub async fn execute(
        &self,
        command: &str,
        working_dir: &Path,
        policy: SandboxPolicy,
        limits: &ResourceLimits,
        env: HashMap<String, String>,
    ) -> Result<ContainerOutput> {
        let start_time = std::time::Instant::now();

        // Create the container
        let container_id = self
            .create_container(command, working_dir, policy, limits, env)
            .await?;

        // Start the container
        self.docker
            .start_container(&container_id, None::<StartContainerOptions<String>>)
            .await
            .map_err(|e| SandboxError::ContainerStartFailed {
                reason: e.to_string(),
            })?;

        // Wait for completion with timeout
        let result = tokio::time::timeout(limits.timeout, async {
            self.wait_for_container(&container_id, limits.max_output_bytes)
                .await
        })
        .await;

        // Always clean up the container
        let _ = self
            .docker
            .remove_container(
                &container_id,
                Some(RemoveContainerOptions {
                    force: true,
                    ..Default::default()
                }),
            )
            .await;

        match result {
            Ok(Ok(mut output)) => {
                output.duration = start_time.elapsed();
                Ok(output)
            }
            Ok(Err(e)) => Err(e),
            Err(_) => Err(SandboxError::Timeout(limits.timeout)),
        }
    }

    /// Execute a command in an existing container using exec.
    pub async fn exec_in_container(
        &self,
        container_id: &str,
        command: &str,
        working_dir: &str,
        limits: &ResourceLimits,
    ) -> Result<ContainerOutput> {
        let start_time = std::time::Instant::now();

        let exec = self
            .docker
            .create_exec(
                container_id,
                CreateExecOptions {
                    cmd: Some(vec!["sh", "-c", command]),
                    attach_stdout: Some(true),
                    attach_stderr: Some(true),
                    working_dir: Some(working_dir),
                    ..Default::default()
                },
            )
            .await
            .map_err(|e| SandboxError::ExecutionFailed {
                reason: format!("exec create failed: {}", e),
            })?;

        let result = tokio::time::timeout(
            limits.timeout,
            self.run_exec(&exec.id, limits.max_output_bytes),
        )
        .await;

        match result {
            Ok(Ok(mut output)) => {
                output.duration = start_time.elapsed();
                Ok(output)
            }
            Ok(Err(e)) => Err(e),
            Err(_) => Err(SandboxError::Timeout(limits.timeout)),
        }
    }

    /// Create a container with the appropriate configuration.
    async fn create_container(
        &self,
        command: &str,
        working_dir: &Path,
        policy: SandboxPolicy,
        limits: &ResourceLimits,
        env: HashMap<String, String>,
    ) -> Result<String> {
        let working_dir_str = working_dir.display().to_string();

        // Build environment variables
        let mut env_vec: Vec<String> = env
            .into_iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect();

        // Add proxy environment (uses host.docker.internal for Mac/Windows, 172.17.0.1 for Linux)
        let proxy_host = if cfg!(target_os = "linux") {
            "172.17.0.1"
        } else {
            "host.docker.internal"
        };

        if self.proxy_port > 0 && policy.is_sandboxed() {
            env_vec.push(format!(
                "http_proxy=http://{}:{}",
                proxy_host, self.proxy_port
            ));
            env_vec.push(format!(
                "https_proxy=http://{}:{}",
                proxy_host, self.proxy_port
            ));
            env_vec.push(format!(
                "HTTP_PROXY=http://{}:{}",
                proxy_host, self.proxy_port
            ));
            env_vec.push(format!(
                "HTTPS_PROXY=http://{}:{}",
                proxy_host, self.proxy_port
            ));
        }

        // Build volume mounts based on policy
        let binds = match policy {
            SandboxPolicy::ReadOnly => {
                vec![format!("{}:/workspace:ro", working_dir_str)]
            }
            SandboxPolicy::WorkspaceWrite => {
                vec![format!("{}:/workspace:rw", working_dir_str)]
            }
            SandboxPolicy::FullAccess => {
                // Full access - mount more of the host
                vec![
                    format!("{}:/workspace:rw", working_dir_str),
                    "/tmp:/tmp:rw".to_string(),
                ]
            }
        };

        let host_config = HostConfig {
            binds: Some(binds),
            memory: Some((limits.memory_bytes) as i64),
            cpu_shares: Some(limits.cpu_shares as i64),
            auto_remove: Some(true),
            network_mode: Some("bridge".to_string()),
            // Security: drop all capabilities and add back only what's needed
            cap_drop: Some(vec!["ALL".to_string()]),
            cap_add: Some(vec!["CHOWN".to_string()]),
            // Prevent privilege escalation
            security_opt: Some(vec!["no-new-privileges:true".to_string()]),
            // Read-only root filesystem (workspace is still writable if policy allows)
            readonly_rootfs: Some(policy != SandboxPolicy::FullAccess),
            // Tmpfs mounts for /tmp and cargo cache
            tmpfs: Some(
                [
                    ("/tmp".to_string(), "size=512M".to_string()),
                    (
                        "/home/sandbox/.cargo/registry".to_string(),
                        "size=1G".to_string(),
                    ),
                ]
                .into_iter()
                .collect(),
            ),
            ..Default::default()
        };

        let config = Config {
            image: Some(self.image.clone()),
            cmd: Some(vec![
                "sh".to_string(),
                "-c".to_string(),
                command.to_string(),
            ]),
            working_dir: Some("/workspace".to_string()),
            env: Some(env_vec),
            host_config: Some(host_config),
            user: Some("1000:1000".to_string()), // Non-root user
            ..Default::default()
        };

        let options = CreateContainerOptions {
            name: format!("sandbox-{}", uuid::Uuid::new_v4()),
            ..Default::default()
        };

        let response = self
            .docker
            .create_container(Some(options), config)
            .await
            .map_err(|e| SandboxError::ContainerCreationFailed {
                reason: e.to_string(),
            })?;

        Ok(response.id)
    }

    /// Wait for a container to complete and collect output.
    async fn wait_for_container(
        &self,
        container_id: &str,
        max_output: usize,
    ) -> Result<ContainerOutput> {
        // Wait for the container to finish
        let mut wait_stream = self.docker.wait_container(
            container_id,
            Some(WaitContainerOptions {
                condition: "not-running",
            }),
        );

        let exit_code = match wait_stream.next().await {
            Some(Ok(response)) => response.status_code,
            Some(Err(e)) => {
                return Err(SandboxError::ExecutionFailed {
                    reason: format!("wait failed: {}", e),
                });
            }
            None => {
                return Err(SandboxError::ExecutionFailed {
                    reason: "container wait stream ended unexpectedly".to_string(),
                });
            }
        };

        // Collect logs
        let (stdout, stderr, truncated) = self.collect_logs(container_id, max_output).await?;

        Ok(ContainerOutput {
            exit_code,
            stdout,
            stderr,
            duration: Duration::ZERO, // Will be set by caller
            truncated,
        })
    }

    /// Collect stdout and stderr from a container.
    async fn collect_logs(
        &self,
        container_id: &str,
        max_output: usize,
    ) -> Result<(String, String, bool)> {
        let options = LogsOptions::<String> {
            stdout: true,
            stderr: true,
            follow: false,
            ..Default::default()
        };

        let mut stream = self.docker.logs(container_id, Some(options));

        let mut stdout = String::new();
        let mut stderr = String::new();
        let mut truncated = false;
        let half_max = max_output / 2;

        while let Some(result) = stream.next().await {
            match result {
                Ok(LogOutput::StdOut { message }) => {
                    let text = String::from_utf8_lossy(&message);
                    if stdout.len() + text.len() > half_max {
                        truncated = true;
                        let remaining = half_max.saturating_sub(stdout.len());
                        stdout.push_str(&text[..remaining.min(text.len())]);
                    } else {
                        stdout.push_str(&text);
                    }
                }
                Ok(LogOutput::StdErr { message }) => {
                    let text = String::from_utf8_lossy(&message);
                    if stderr.len() + text.len() > half_max {
                        truncated = true;
                        let remaining = half_max.saturating_sub(stderr.len());
                        stderr.push_str(&text[..remaining.min(text.len())]);
                    } else {
                        stderr.push_str(&text);
                    }
                }
                Ok(_) => {}
                Err(e) => {
                    tracing::warn!("Error reading container logs: {}", e);
                }
            }
        }

        Ok((stdout, stderr, truncated))
    }

    /// Run an exec and collect output.
    async fn run_exec(&self, exec_id: &str, max_output: usize) -> Result<ContainerOutput> {
        let start_result = self.docker.start_exec(exec_id, None).await.map_err(|e| {
            SandboxError::ExecutionFailed {
                reason: format!("exec start failed: {}", e),
            }
        })?;

        let mut stdout = String::new();
        let mut stderr = String::new();
        let mut truncated = false;
        let half_max = max_output / 2;

        if let StartExecResults::Attached { mut output, .. } = start_result {
            while let Some(result) = output.next().await {
                match result {
                    Ok(LogOutput::StdOut { message }) => {
                        let text = String::from_utf8_lossy(&message);
                        if stdout.len() < half_max {
                            let remaining = half_max.saturating_sub(stdout.len());
                            stdout.push_str(&text[..remaining.min(text.len())]);
                            if text.len() > remaining {
                                truncated = true;
                            }
                        }
                    }
                    Ok(LogOutput::StdErr { message }) => {
                        let text = String::from_utf8_lossy(&message);
                        if stderr.len() < half_max {
                            let remaining = half_max.saturating_sub(stderr.len());
                            stderr.push_str(&text[..remaining.min(text.len())]);
                            if text.len() > remaining {
                                truncated = true;
                            }
                        }
                    }
                    Ok(_) => {}
                    Err(e) => {
                        tracing::warn!("Error reading exec output: {}", e);
                    }
                }
            }
        }

        // Get exec exit code
        let inspect =
            self.docker
                .inspect_exec(exec_id)
                .await
                .map_err(|e| SandboxError::ExecutionFailed {
                    reason: format!("exec inspect failed: {}", e),
                })?;

        let exit_code = inspect.exit_code.unwrap_or(-1);

        Ok(ContainerOutput {
            exit_code,
            stdout,
            stderr,
            duration: Duration::ZERO,
            truncated,
        })
    }
}

/// Connect to the Docker-compatible container runtime daemon (Docker or Podman).
///
/// Tries these locations in order:
/// 1. `DOCKER_HOST` env var (bollard default)
/// 2. `/var/run/docker.sock` (Linux default; also used by OrbStack and Podman Desktop on macOS)
/// 3. `~/.orbstack/run/docker.sock` (OrbStack on macOS — user-owned socket)
/// 4. `~/.docker/run/docker.sock` (Docker Desktop 4.13+ on macOS — primary user-owned socket)
/// 5. `~/.colima/default/docker.sock` (Colima — popular lightweight Docker Desktop alternative)
/// 6. `~/.rd/docker.sock` (Rancher Desktop on macOS)
/// 7. `$XDG_RUNTIME_DIR/docker.sock` (common rootless Docker socket on Linux)
/// 8. `/run/user/$UID/docker.sock` (rootless Docker fallback on Linux)
/// 9. `$XDG_RUNTIME_DIR/podman/podman.sock` (rootless Podman on Linux)
/// 10. `/run/user/$UID/podman/podman.sock` (rootless Podman fallback on Linux)
/// 11. `/run/podman/podman.sock` (rootful Podman on Linux)
/// 12. `~/.local/share/containers/podman/machine/*/podman.sock` (Podman machine on macOS)
pub async fn connect_docker() -> Result<Docker> {
    // First try bollard defaults (checks DOCKER_HOST env var, then /var/run/docker.sock).
    // This covers Linux and any user with DOCKER_HOST set to their runtime's socket.
    // OrbStack may also create a /var/run/docker.sock symlink, but its user-owned
    // socket at ~/.orbstack/run/docker.sock is checked separately below.
    if let Ok(docker) = Docker::connect_with_local_defaults()
        && docker.ping().await.is_ok()
    {
        return Ok(docker);
    }

    #[cfg(unix)]
    {
        // Try well-known user-owned socket locations for desktop and rootless runtimes.
        // Docker Desktop 4.13+ (stabilised in 4.18) stopped creating the
        // /var/run/docker.sock symlink by default and moved the API socket
        // to ~/.docker/run/docker.sock.
        for sock in unix_socket_candidates() {
            if sock.exists() {
                let sock_str = sock.to_string_lossy();
                if let Ok(docker) =
                    Docker::connect_with_socket(&sock_str, 120, bollard::API_DEFAULT_VERSION)
                    && docker.ping().await.is_ok()
                {
                    return Ok(docker);
                }
            }
        }
    }

    Err(SandboxError::DockerNotAvailable {
        reason: "Could not connect to container runtime daemon. Tried: $DOCKER_HOST, \
            /var/run/docker.sock, ~/.orbstack/run/docker.sock, \
            ~/.docker/run/docker.sock, ~/.colima/default/docker.sock, ~/.rd/docker.sock, \
            $XDG_RUNTIME_DIR/docker.sock, /run/user/$UID/docker.sock, \
            $XDG_RUNTIME_DIR/podman/podman.sock, /run/user/$UID/podman/podman.sock, \
            /run/podman/podman.sock, ~/.local/share/containers/podman/machine/*/podman.sock"
            .to_string(),
    })
}

#[cfg(unix)]
fn unix_socket_candidates() -> Vec<PathBuf> {
    unix_socket_candidates_from_env(
        std::env::var_os("HOME").map(PathBuf::from),
        std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from),
        std::env::var("UID").ok(),
    )
}

#[cfg(unix)]
fn unix_socket_candidates_from_env(
    home: Option<PathBuf>,
    xdg_runtime_dir: Option<PathBuf>,
    uid: Option<String>,
) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    let mut push_unique = |path: PathBuf| {
        if !candidates.iter().any(|existing| existing == &path) {
            candidates.push(path);
        }
    };

    // Docker sockets
    if let Some(ref home) = home {
        push_unique(home.join(".orbstack/run/docker.sock")); // OrbStack on macOS
        push_unique(home.join(".docker/run/docker.sock")); // Docker Desktop 4.13+
        push_unique(home.join(".colima/default/docker.sock")); // Colima
        push_unique(home.join(".rd/docker.sock")); // Rancher Desktop
    }

    if let Some(ref xdg_runtime_dir) = xdg_runtime_dir {
        push_unique(xdg_runtime_dir.join("docker.sock"));
    }

    let uid = uid.filter(|value| !value.is_empty());

    if let Some(ref uid) = uid {
        push_unique(PathBuf::from(format!("/run/user/{uid}/docker.sock")));
    }

    // Podman sockets
    if let Some(ref xdg_runtime_dir) = xdg_runtime_dir {
        push_unique(xdg_runtime_dir.join("podman/podman.sock")); // Rootless Podman
    }

    if let Some(ref uid) = uid {
        push_unique(PathBuf::from(format!("/run/user/{uid}/podman/podman.sock"))); // Rootless Podman fallback
    }

    push_unique(PathBuf::from("/run/podman/podman.sock")); // Rootful Podman

    if let Some(ref home) = home {
        // Podman machine on macOS — glob for any machine name
        let machine_dir = home.join(".local/share/containers/podman/machine");
        if let Ok(entries) = std::fs::read_dir(&machine_dir) {
            for entry in entries.flatten() {
                let sock = entry.path().join("podman.sock");
                push_unique(sock);
            }
        }
    }

    candidates
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn test_unix_socket_candidates_include_rootless_paths() {
        let candidates = unix_socket_candidates_from_env(
            Some(PathBuf::from("/home/tester")),
            Some(PathBuf::from("/run/user/1000")),
            Some("1000".to_string()),
        );

        // Docker sockets
        assert!(candidates.contains(&PathBuf::from("/home/tester/.orbstack/run/docker.sock")));
        assert!(candidates.contains(&PathBuf::from("/home/tester/.docker/run/docker.sock")));
        assert!(candidates.contains(&PathBuf::from("/home/tester/.colima/default/docker.sock")));
        assert!(candidates.contains(&PathBuf::from("/home/tester/.rd/docker.sock")));
        assert!(candidates.contains(&PathBuf::from("/run/user/1000/docker.sock")));

        // Podman sockets
        assert!(candidates.contains(&PathBuf::from("/run/user/1000/podman/podman.sock")));
        assert!(candidates.contains(&PathBuf::from("/run/podman/podman.sock")));
    }

    #[tokio::test]
    async fn test_docker_connection() {
        // This test requires Docker to be running
        let result = connect_docker().await;
        // Don't fail if Docker isn't available, just skip
        if result.is_err() {
            eprintln!("Skipping Docker test: Docker not available");
            return;
        }

        let docker = result.unwrap();
        let runner = ContainerRunner::new(docker, "alpine:latest".to_string(), 0);
        // Just check that we can query Docker (result doesn't matter for CI)
        let _available = runner.is_available().await;
    }
}
