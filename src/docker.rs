//! Docker daemon detection and connection utilities.
//!
//! Provides lightweight functions for detecting Docker availability and
//! connecting to the Docker daemon. These utilities are used by the
//! orchestrator (container job management) and diagnostics (boot screen,
//! doctor, setup wizard).

use std::path::PathBuf;

use bollard::Docker;

/// Docker daemon availability status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DockerStatus {
    /// Docker binary found on PATH and daemon responding to ping.
    Available,
    /// `docker` binary not found on PATH.
    NotInstalled,
    /// Binary found but daemon not responding.
    NotRunning,
    /// Docker feature not enabled (no check performed).
    Disabled,
}

impl DockerStatus {
    /// Returns true if Docker is available and ready.
    pub fn is_ok(&self) -> bool {
        matches!(self, DockerStatus::Available)
    }

    /// Human-readable status string.
    pub fn as_str(&self) -> &'static str {
        match self {
            DockerStatus::Available => "available",
            DockerStatus::NotInstalled => "not installed",
            DockerStatus::NotRunning => "not running",
            DockerStatus::Disabled => "disabled",
        }
    }
}

/// Host platform for install guidance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    MacOS,
    Linux,
    Windows,
}

impl Platform {
    /// Detect the current platform.
    pub fn current() -> Self {
        match std::env::consts::OS {
            "macos" => Platform::MacOS,
            "windows" => Platform::Windows,
            _ => Platform::Linux,
        }
    }

    /// Installation instructions for Docker on this platform.
    pub fn install_hint(&self) -> &'static str {
        match self {
            Platform::MacOS => {
                "Install Docker Desktop: https://docs.docker.com/desktop/install/mac-install/"
            }
            Platform::Linux => "Install Docker Engine: https://docs.docker.com/engine/install/",
            Platform::Windows => {
                "Install Docker Desktop: https://docs.docker.com/desktop/install/windows-install/"
            }
        }
    }

    /// Instructions to start the Docker daemon on this platform.
    pub fn start_hint(&self) -> &'static str {
        match self {
            Platform::MacOS => {
                "Start Docker Desktop from Applications, or run: open -a Docker\n\n  To auto-start at login: System Settings > General > Login Items > add Docker.app"
            }
            Platform::Linux => "Start the Docker daemon: sudo systemctl start docker",
            Platform::Windows => "Start Docker Desktop from the Start menu",
        }
    }
}

/// Result of a Docker detection check.
pub struct DockerDetection {
    pub status: DockerStatus,
    pub platform: Platform,
}

/// Check whether Docker is installed and running.
///
/// 1. Checks if `docker` binary exists on PATH
/// 2. If found, tries to connect and ping the Docker daemon via `connect_docker()`
/// 3. Returns `Available`, `NotInstalled`, or `NotRunning`
pub async fn check_docker() -> DockerDetection {
    let platform = Platform::current();

    // Step 1: Check if docker binary is on PATH
    if !docker_binary_exists() {
        return DockerDetection {
            status: DockerStatus::NotInstalled,
            platform,
        };
    }

    // Step 2: Try to connect to the daemon
    if connect_docker().await.is_ok() {
        return DockerDetection {
            status: DockerStatus::Available,
            platform,
        };
    }

    // Windows fallback: if the named pipe probe fails but docker CLI can still
    // reach the daemon/server, treat Docker as available.
    #[cfg(windows)]
    if docker_cli_daemon_reachable() {
        return DockerDetection {
            status: DockerStatus::Available,
            platform,
        };
    }

    DockerDetection {
        status: DockerStatus::NotRunning,
        platform,
    }
}

/// Connect to the Docker daemon.
///
/// Tries multiple socket paths in order:
/// 1. `DOCKER_HOST` env var / `/var/run/docker.sock` (bollard defaults)
/// 2. `~/.docker/run/docker.sock` (Docker Desktop 4.13+)
/// 3. `~/.colima/default/docker.sock` (Colima)
/// 4. `~/.rd/docker.sock` (Rancher Desktop)
/// 5. `$XDG_RUNTIME_DIR/docker.sock` (rootless Docker)
/// 6. `/run/user/$UID/docker.sock` (rootless Linux)
pub async fn connect_docker() -> Result<Docker, DockerError> {
    // First try bollard defaults (checks DOCKER_HOST env var, then /var/run/docker.sock).
    if let Ok(docker) = Docker::connect_with_local_defaults()
        && docker.ping().await.is_ok()
    {
        return Ok(docker);
    }

    #[cfg(unix)]
    {
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

    Err(DockerError::NotAvailable {
        reason: "Could not connect to Docker daemon. Tried: $DOCKER_HOST, \
            /var/run/docker.sock, ~/.docker/run/docker.sock, \
            ~/.colima/default/docker.sock, ~/.rd/docker.sock, \
            $XDG_RUNTIME_DIR/docker.sock, /run/user/$UID/docker.sock"
            .to_string(),
    })
}

/// Helper for Docker image operations (exists, pull, build).
///
/// Used by the setup wizard and orchestrator to manage worker images.
pub struct ImageOps {
    docker: Docker,
    image: String,
}

impl ImageOps {
    /// Create an image ops helper for the given image name.
    pub fn new(docker: Docker, image: String) -> Self {
        Self { docker, image }
    }

    /// Check if the image exists locally.
    pub async fn image_exists(&self) -> bool {
        self.docker.inspect_image(&self.image).await.is_ok()
    }

    /// Pull the image from a registry.
    pub async fn pull_image(&self) -> Result<(), DockerError> {
        use bollard::image::CreateImageOptions;
        use futures::StreamExt;

        tracing::info!("Pulling image: {}", self.image);

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
                    return Err(DockerError::ImageOperation {
                        reason: format!("image pull failed: {}", e),
                    });
                }
            }
        }

        tracing::info!("Successfully pulled image: {}", self.image);
        Ok(())
    }

    /// Build the image from a Dockerfile.
    ///
    /// # Security
    ///
    /// The `dockerfile_path` MUST point to a trusted Dockerfile. Docker builds
    /// execute arbitrary `RUN` commands, which is a code execution vector.
    pub async fn build_image(&self, dockerfile_path: &std::path::Path) -> Result<(), DockerError> {
        use tokio::io::AsyncBufReadExt;
        use tokio::process::Command;

        const MAX_STDERR_CAPTURE: usize = 4096;

        let canonical =
            dockerfile_path
                .canonicalize()
                .map_err(|e| DockerError::ImageOperation {
                    reason: format!(
                        "cannot resolve Dockerfile path '{}': {}",
                        dockerfile_path.display(),
                        e
                    ),
                })?;

        let context_dir = canonical
            .parent()
            .ok_or_else(|| DockerError::ImageOperation {
                reason: format!(
                    "Dockerfile path '{}' has no parent directory",
                    canonical.display()
                ),
            })?;

        tracing::info!(
            "Building image from {}: {}",
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
            .map_err(|e| DockerError::ImageOperation {
                reason: format!("failed to run docker build: {}", e),
            })?;

        let mut stdout_lines = tokio::io::BufReader::new(child.stdout.take().ok_or_else(|| {
            DockerError::ImageOperation {
                reason: "stdout pipe missing".to_string(),
            }
        })?)
        .lines();
        let mut stderr_lines = tokio::io::BufReader::new(child.stderr.take().ok_or_else(|| {
            DockerError::ImageOperation {
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
            .map_err(|e| DockerError::ImageOperation {
                reason: format!("docker build wait failed: {}", e),
            })?;

        if !status.success() {
            let code = status
                .code()
                .map_or("unknown".to_string(), |c| c.to_string());
            return Err(DockerError::ImageOperation {
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
}

/// Errors from Docker operations.
#[derive(Debug, thiserror::Error)]
pub enum DockerError {
    /// Docker daemon is not available or not running.
    #[error("Docker not available: {reason}")]
    NotAvailable { reason: String },

    /// Image operation failed.
    #[error("Image operation failed: {reason}")]
    ImageOperation { reason: String },

    /// Docker API error.
    #[error("Docker API error: {0}")]
    Api(#[from] bollard::errors::Error),
}

/// Check if the `docker` binary exists on PATH.
fn docker_binary_exists() -> bool {
    #[cfg(unix)]
    {
        std::process::Command::new("which")
            .arg("docker")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }
    #[cfg(windows)]
    {
        std::process::Command::new("where")
            .arg("docker")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }
}

#[cfg(windows)]
fn docker_cli_daemon_reachable() -> bool {
    let stdout = std::process::Stdio::null();
    let stderr = std::process::Stdio::null();

    let version_ok = std::process::Command::new("docker")
        .args(["version", "--format", "{{.Server.Version}}"])
        .stdout(stdout)
        .stderr(stderr)
        .status()
        .is_ok_and(|s| s.success());

    if version_ok {
        return true;
    }

    std::process::Command::new("docker")
        .args(["info", "--format", "{{.ServerVersion}}"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
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
    fn test_detect_platform() {
        let platform = Platform::current();
        match platform {
            Platform::MacOS | Platform::Linux | Platform::Windows => {}
        }
    }

    #[test]
    fn test_install_hint_not_empty() {
        for platform in [Platform::MacOS, Platform::Linux, Platform::Windows] {
            assert!(!platform.install_hint().is_empty());
            assert!(!platform.start_hint().is_empty());
        }
    }

    #[test]
    fn test_docker_status_display() {
        assert_eq!(DockerStatus::Available.as_str(), "available");
        assert_eq!(DockerStatus::NotInstalled.as_str(), "not installed");
        assert_eq!(DockerStatus::NotRunning.as_str(), "not running");
        assert_eq!(DockerStatus::Disabled.as_str(), "disabled");
    }

    #[test]
    fn test_docker_status_is_ok() {
        assert!(DockerStatus::Available.is_ok());
        assert!(!DockerStatus::NotInstalled.is_ok());
        assert!(!DockerStatus::NotRunning.is_ok());
        assert!(!DockerStatus::Disabled.is_ok());
    }

    #[tokio::test]
    async fn test_check_docker_returns_valid_status() {
        let result = check_docker().await;
        match result.status {
            DockerStatus::Available | DockerStatus::NotInstalled | DockerStatus::NotRunning => {}
            DockerStatus::Disabled => panic!("check_docker should never return Disabled"),
        }
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
}
