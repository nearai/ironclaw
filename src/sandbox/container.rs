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
use std::path::Path;
#[cfg(unix)]
use std::path::PathBuf;
use std::time::Duration;

use bollard::Docker;
use bollard::container::{
    Config, CreateContainerOptions, LogOutput, LogsOptions, RemoveContainerOptions,
    StartContainerOptions, WaitContainerOptions,
};
use bollard::exec::{CreateExecOptions, StartExecResults};
use bollard::models::HostConfig;
use futures::StreamExt;

use crate::sandbox::config::{ResourceLimits, SandboxConfig, SandboxPolicy};
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

/// Append `text` into `buffer` up to `limit` bytes without breaking UTF-8.
///
/// Returns `true` when truncation occurred.
fn append_with_limit(buffer: &mut String, text: &str, limit: usize) -> bool {
    if text.is_empty() {
        return false;
    }

    if buffer.len() >= limit {
        return true;
    }

    let remaining = limit - buffer.len();
    if text.len() <= remaining {
        buffer.push_str(text);
        return false;
    }

    let end = crate::util::floor_char_boundary(text, remaining);
    buffer.push_str(&text[..end]);
    true
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

    /// Create a runner for image-only operations (exists / pull / build).
    ///
    /// `proxy_port` is unused for image operations, so this avoids requiring
    /// a real port number when the caller only needs to check or build images.
    pub fn for_image_ops(docker: Docker, image: String) -> Self {
        Self {
            docker,
            image,
            proxy_port: 0,
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

    /// Build the sandbox image from a Dockerfile.
    ///
    /// This is used when the image is not available from a registry and needs
    /// to be built locally from source.
    ///
    /// # Security
    ///
    /// The `dockerfile_path` MUST point to a trusted Dockerfile. Docker builds
    /// execute arbitrary `RUN` commands from the Dockerfile, which is a code
    /// execution vector. Callers must ensure the path is not user-controlled
    /// and points to a known, safe Dockerfile (e.g., bundled with the application).
    pub async fn build_image(&self, dockerfile_path: &Path) -> Result<()> {
        use tokio::io::AsyncBufReadExt;
        use tokio::process::Command;

        const MAX_STDERR_CAPTURE: usize = 4096;

        // Canonicalize so the -f path is absolute and context_dir is its parent.
        // This avoids the bug where a relative path like "docker/sandbox.Dockerfile"
        // would be resolved twice (once for context_dir, once by docker -f).
        let canonical =
            dockerfile_path
                .canonicalize()
                .map_err(|e| SandboxError::ContainerCreationFailed {
                    reason: format!(
                        "cannot resolve Dockerfile path '{}': {}",
                        dockerfile_path.display(),
                        e
                    ),
                })?;

        let context_dir =
            canonical
                .parent()
                .ok_or_else(|| SandboxError::ContainerCreationFailed {
                    reason: format!(
                        "Dockerfile path '{}' has no parent directory",
                        canonical.display()
                    ),
                })?;

        tracing::info!(
            "Building sandbox image from {}: {}",
            canonical.display(),
            self.image
        );

        let mut child = Command::new("docker")
            .arg("build")
            .arg("-f")
            .arg(&canonical)
            .arg("-t")
            .arg(&self.image)
            .arg(".")
            .current_dir(context_dir)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| SandboxError::ContainerCreationFailed {
                reason: format!("failed to run docker build: {}", e),
            })?;

        // Both streams are piped above, so take() returns Some.
        let mut stdout_lines = tokio::io::BufReader::new(child.stdout.take().ok_or_else(|| {
            SandboxError::ContainerCreationFailed {
                reason: "stdout pipe missing".to_string(),
            }
        })?)
        .lines();
        let mut stderr_lines = tokio::io::BufReader::new(child.stderr.take().ok_or_else(|| {
            SandboxError::ContainerCreationFailed {
                reason: "stderr pipe missing".to_string(),
            }
        })?)
        .lines();

        let mut stderr_capture = String::new();
        let mut stdout_done = false;
        let mut stderr_done = false;

        while !stdout_done || !stderr_done {
            tokio::select! {
                line = stdout_lines.next_line(), if !stdout_done => {
                    match line {
                        Ok(Some(line)) => tracing::info!("[docker build] {}", line),
                        Ok(None) => stdout_done = true,
                        Err(e) => {
                            tracing::warn!("Error reading docker build stdout: {}", e);
                            stdout_done = true;
                        }
                    }
                },
                line = stderr_lines.next_line(), if !stderr_done => {
                    match line {
                        Ok(Some(line)) => {
                            tracing::info!("[docker build] {}", line);
                            if stderr_capture.len() < MAX_STDERR_CAPTURE {
                                stderr_capture.push_str(&line);
                                stderr_capture.push('\n');
                            }
                        }
                        Ok(None) => stderr_done = true,
                        Err(e) => {
                            tracing::warn!("Error reading docker build stderr: {}", e);
                            stderr_done = true;
                        }
                    }
                },
            }
        }

        let status = child
            .wait()
            .await
            .map_err(|e| SandboxError::ContainerCreationFailed {
                reason: format!("docker build wait failed: {}", e),
            })?;

        if !status.success() {
            let code = status
                .code()
                .map_or("unknown".to_string(), |c| c.to_string());
            return Err(SandboxError::ContainerCreationFailed {
                reason: format!(
                    "docker build failed (exit {}): {}",
                    code,
                    stderr_capture.trim_end()
                ),
            });
        }

        tracing::info!("Successfully built image: {}", self.image);
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
        config: &SandboxConfig,
    ) -> Result<ContainerOutput> {
        let start_time = std::time::Instant::now();

        // Create the container
        let container_id = self
            .create_container(command, working_dir, policy, limits, env, config)
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

    fn build_container_env(
        &self,
        env: HashMap<String, String>,
        policy: SandboxPolicy,
        config: &SandboxConfig,
    ) -> Vec<String> {
        let mut env_vec: Vec<String> = env.into_iter().map(|(k, v)| format!("{k}={v}")).collect();

        // Add proxy environment when sandboxed and not using host networking.
        // Host networking means the container shares the host's network namespace,
        // so the proxy is unnecessary.
        if self.proxy_port > 0 && policy.is_sandboxed() && !config.host_network {
            let proxy_host = if cfg!(target_os = "linux") {
                "172.17.0.1"
            } else {
                "host.docker.internal"
            };
            for key in ["http_proxy", "https_proxy", "HTTP_PROXY", "HTTPS_PROXY"] {
                env_vec.push(format!("{key}=http://{proxy_host}:{}", self.proxy_port));
            }
        }

        env_vec.extend(config.extra_env.iter().cloned());

        env_vec
    }

    fn build_container_binds(
        working_dir_str: &str,
        policy: SandboxPolicy,
        config: &SandboxConfig,
    ) -> Vec<String> {
        let mut binds = match policy {
            SandboxPolicy::ReadOnly => {
                vec![format!("{working_dir_str}:/workspace:ro")]
            }
            SandboxPolicy::WorkspaceWrite => {
                vec![format!("{working_dir_str}:/workspace:rw")]
            }
            SandboxPolicy::FullAccess => {
                vec![
                    format!("{working_dir_str}:/workspace:rw"),
                    "/tmp:/tmp:rw".to_string(),
                ]
            }
        };

        binds.extend(config.extra_volumes.iter().cloned());

        binds
    }

    fn build_capabilities(config: &SandboxConfig) -> (Vec<String>, Vec<String>) {
        let cap_drop = vec!["ALL".to_string()];
        let mut cap_add = vec!["CHOWN".to_string()];
        if config.run_as_root {
            cap_add.extend([
                "NET_RAW".to_string(),
                "NET_ADMIN".to_string(),
                "SYS_CHROOT".to_string(),
            ]);
        }
        (cap_drop, cap_add)
    }

    fn build_host_config(
        binds: Vec<String>,
        limits: &ResourceLimits,
        config: &SandboxConfig,
        auto_remove: bool,
        readonly_rootfs: bool,
    ) -> HostConfig {
        let (cap_drop, cap_add) = Self::build_capabilities(config);
        HostConfig {
            binds: Some(binds),
            memory: Some(limits.memory_bytes as i64),
            cpu_shares: Some(limits.cpu_shares as i64),
            auto_remove: Some(auto_remove),
            network_mode: Some(if config.host_network {
                "host".to_string()
            } else {
                "bridge".to_string()
            }),
            cap_drop: Some(cap_drop),
            cap_add: Some(cap_add),
            security_opt: Some(vec!["no-new-privileges:true".to_string()]),
            readonly_rootfs: Some(readonly_rootfs),
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
        }
    }

    fn container_user(config: &SandboxConfig) -> &'static str {
        if config.run_as_root {
            "0:0"
        } else {
            "1000:1000"
        }
    }

    async fn register_container(
        &self,
        name_prefix: &str,
        container_config: Config<String>,
    ) -> Result<String> {
        let options = CreateContainerOptions {
            name: format!("{name_prefix}-{}", uuid::Uuid::new_v4()),
            ..Default::default()
        };
        let response = self
            .docker
            .create_container(Some(options), container_config)
            .await
            .map_err(|e| SandboxError::ContainerCreationFailed {
                reason: e.to_string(),
            })?;
        Ok(response.id)
    }

    async fn create_container(
        &self,
        command: &str,
        working_dir: &Path,
        policy: SandboxPolicy,
        limits: &ResourceLimits,
        env: HashMap<String, String>,
        config: &SandboxConfig,
    ) -> Result<String> {
        let working_dir_str = working_dir.display().to_string();
        let env_vec = self.build_container_env(env, policy, config);
        let binds = Self::build_container_binds(&working_dir_str, policy, config);
        let host_config = Self::build_host_config(
            binds,
            limits,
            config,
            true,
            policy != SandboxPolicy::FullAccess,
        );

        let container_config = Config {
            image: Some(self.image.clone()),
            entrypoint: Some(vec!["sh".to_string()]),
            cmd: Some(vec!["-c".to_string(), command.to_string()]),
            working_dir: Some("/workspace".to_string()),
            env: Some(env_vec),
            host_config: Some(host_config),
            user: Some(Self::container_user(config).to_string()),
            ..Default::default()
        };

        self.register_container("sandbox", container_config).await
    }

    /// Create a persistent session container.
    ///
    /// Unlike ephemeral containers, persistent containers:
    /// - Run `sleep infinity` (kept alive for repeated exec calls)
    /// - Have a writable root filesystem (allows package installs)
    /// - Are NOT auto-removed (managed by SandboxManager::shutdown)
    /// - Are labeled for identification by the container reaper
    pub async fn create_persistent_container(
        &self,
        working_dir: &Path,
        policy: SandboxPolicy,
        limits: &ResourceLimits,
        env: HashMap<String, String>,
        config: &SandboxConfig,
    ) -> Result<String> {
        let working_dir_str = working_dir.display().to_string();
        let env_vec = self.build_container_env(env, policy, config);
        let binds = Self::build_container_binds(&working_dir_str, policy, config);
        let host_config = Self::build_host_config(binds, limits, config, false, false);

        let mut labels = std::collections::HashMap::new();
        labels.insert("ironclaw.persistent".to_string(), "true".to_string());

        let container_config = Config {
            image: Some(self.image.clone()),
            entrypoint: Some(vec!["sleep".to_string()]),
            cmd: Some(vec!["infinity".to_string()]),
            working_dir: Some("/workspace".to_string()),
            env: Some(env_vec),
            host_config: Some(host_config),
            user: Some(Self::container_user(config).to_string()),
            labels: Some(labels),
            ..Default::default()
        };

        self.register_container("sandbox-session", container_config)
            .await
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
                    truncated |= append_with_limit(&mut stdout, &text, half_max);
                }
                Ok(LogOutput::StdErr { message }) => {
                    let text = String::from_utf8_lossy(&message);
                    truncated |= append_with_limit(&mut stderr, &text, half_max);
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
                        truncated |= append_with_limit(&mut stdout, &text, half_max);
                    }
                    Ok(LogOutput::StdErr { message }) => {
                        let text = String::from_utf8_lossy(&message);
                        truncated |= append_with_limit(&mut stderr, &text, half_max);
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

/// Connect to the Docker daemon.
///
/// Tries these locations in order:
/// 1. `DOCKER_HOST` env var (bollard default)
/// 2. `/var/run/docker.sock` (Linux default; also used by OrbStack and Podman Desktop on macOS)
/// 3. `~/.docker/run/docker.sock` (Docker Desktop 4.13+ on macOS — primary user-owned socket)
/// 4. `~/.colima/default/docker.sock` (Colima — popular lightweight Docker Desktop alternative)
/// 5. `~/.rd/docker.sock` (Rancher Desktop on macOS)
/// 6. `$XDG_RUNTIME_DIR/docker.sock` (common rootless Docker socket on Linux)
/// 7. `/run/user/$UID/docker.sock` (rootless Docker fallback on Linux)
pub async fn connect_docker() -> Result<Docker> {
    // First try bollard defaults (checks DOCKER_HOST env var, then /var/run/docker.sock).
    // This covers Linux, OrbStack (updates the /var/run symlink), and any user with
    // DOCKER_HOST set to their runtime's socket.
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
        reason: "Could not connect to Docker daemon. Tried: $DOCKER_HOST, \
            /var/run/docker.sock, ~/.docker/run/docker.sock, \
            ~/.colima/default/docker.sock, ~/.rd/docker.sock, \
            $XDG_RUNTIME_DIR/docker.sock, /run/user/$UID/docker.sock"
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

    if let Some(home) = home {
        push_unique(home.join(".docker/run/docker.sock")); // Docker Desktop 4.13+
        push_unique(home.join(".colima/default/docker.sock")); // Colima
        push_unique(home.join(".rd/docker.sock")); // Rancher Desktop
    }

    if let Some(xdg_runtime_dir) = xdg_runtime_dir {
        push_unique(xdg_runtime_dir.join("docker.sock"));
    }

    if let Some(uid) = uid.filter(|value| !value.is_empty()) {
        push_unique(PathBuf::from(format!("/run/user/{uid}/docker.sock")));
    }

    candidates
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_with_limit_truncates_on_utf8_boundary() {
        let mut out = String::new();
        let truncated = append_with_limit(&mut out, "ab🙂cd", 5);
        assert!(truncated);
        assert_eq!(out, "ab");
    }

    #[test]
    fn append_with_limit_marks_truncated_when_full() {
        let mut out = "abc".to_string();
        let truncated = append_with_limit(&mut out, "z", 3);
        assert!(truncated);
        assert_eq!(out, "abc");
    }

    #[test]
    fn append_with_limit_appends_without_truncation() {
        let mut out = String::new();
        let truncated = append_with_limit(&mut out, "hello", 10);
        assert!(!truncated);
        assert_eq!(out, "hello");
    }

    #[cfg(unix)]
    #[test]
    fn test_unix_socket_candidates_include_rootless_paths() {
        let candidates = unix_socket_candidates_from_env(
            Some(PathBuf::from("/home/tester")),
            Some(PathBuf::from("/run/user/1000")),
            Some("1000".to_string()),
        );

        assert!(candidates.contains(&PathBuf::from("/home/tester/.docker/run/docker.sock")));
        assert!(candidates.contains(&PathBuf::from("/home/tester/.colima/default/docker.sock")));
        assert!(candidates.contains(&PathBuf::from("/home/tester/.rd/docker.sock")));
        assert!(candidates.contains(&PathBuf::from("/run/user/1000/docker.sock")));
    }

    #[tokio::test]
    async fn build_image_rejects_nonexistent_dockerfile() {
        // The behavior under test (Dockerfile path canonicalization) happens
        // before any Docker daemon call, so we don't need a reachable daemon.
        let docker = Docker::connect_with_http_defaults().unwrap();
        let runner = ContainerRunner::for_image_ops(docker, "test-nonexistent:latest".to_string());
        let dir = tempfile::tempdir().unwrap();
        let bad_path = dir.path().join("definitely-does-not-exist.Dockerfile");
        let err = runner.build_image(&bad_path).await.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("cannot resolve Dockerfile path"),
            "expected path resolution error, got: {msg}"
        );
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
