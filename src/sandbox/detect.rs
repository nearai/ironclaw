//! Proactive Docker detection with platform-specific guidance.
//!
//! Checks whether Docker is running by first attempting a direct socket
//! connection via bollard (covers container-in-container deployments where
//! the socket is bind-mounted but the CLI is absent), then falls back to
//! a `which docker` PATH check for error-message quality. Provides
//! platform-appropriate installation or startup instructions when Docker
//! is not available.
//!
//! # Detection Limitations
//!
//! - **macOS**: High confidence. Detects both standard Docker Desktop socket
//!   (`~/.docker/run/docker.sock`) and the default `/var/run/docker.sock`.
//!
//! - **Linux**: High confidence for standard installs. Rootless Docker uses
//!   a different socket path (`/run/user/$UID/docker.sock`) which is now
//!   checked by the fallback in `connect_docker()`. If `DOCKER_HOST` is set,
//!   bollard's default connection still takes precedence.
//!
//! - **Windows**: Medium confidence. Binary detection uses `where.exe` which
//!   works reliably. Daemon detection relies on bollard's default named pipe
//!   connection (`//./pipe/docker_engine`) which works with Docker Desktop.
//!   The Unix socket fallback in `connect_docker()` is a no-op on Windows,
//!   so detection also probes `docker version`/`docker info` via CLI if the
//!   named pipe is unavailable.

/// Docker daemon availability status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DockerStatus {
    /// Docker binary found on PATH and daemon responding to ping.
    Available,
    /// `docker` binary not found on PATH.
    NotInstalled,
    /// Binary found but daemon not responding.
    NotRunning,
    /// Sandbox feature not enabled (no check performed).
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
/// 1. Tries to connect and ping the Docker daemon directly via `connect_docker()`
///    (bollard). This covers container-in-container deployments where the Docker
///    socket is bind-mounted but the CLI binary is not installed.
/// 2. If the socket connection fails, checks if the `docker` binary exists on
///    PATH to distinguish "not installed" from "installed but not running".
/// 3. Returns `Available`, `NotInstalled`, or `NotRunning`.
pub async fn check_docker() -> DockerDetection {
    let platform = Platform::current();

    // Step 1: Try to connect to the daemon directly via the socket (bollard).
    // This is the authoritative check — if bollard can ping, Docker is available
    // regardless of whether the CLI binary is on PATH.
    if crate::sandbox::connect_docker().await.is_ok() {
        return DockerDetection {
            status: DockerStatus::Available,
            platform,
        };
    }

    // Step 2: Socket connection failed. Check if the CLI binary exists to
    // provide a more helpful error message.
    if !docker_binary_exists() {
        return DockerDetection {
            status: DockerStatus::NotInstalled,
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

    // `docker version` requires daemon reachability for server fields.
    let version_ok = std::process::Command::new("docker")
        .args(["version", "--format", "{{.Server.Version}}"])
        .stdout(stdout)
        .stderr(stderr)
        .status()
        .is_ok_and(|s| s.success());

    if version_ok {
        return true;
    }

    // Fallback for environments where `docker version --format` behaves differently.
    std::process::Command::new("docker")
        .args(["info", "--format", "{{.ServerVersion}}"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
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
}
