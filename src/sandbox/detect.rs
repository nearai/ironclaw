//! Proactive container runtime detection with platform-specific guidance.
//!
//! Checks whether Docker or Podman is both installed (binary on PATH) and
//! running (daemon responding to ping), and provides platform-appropriate
//! installation or startup instructions when it is not.
//!
//! # Detection Limitations
//!
//! - **macOS**: High confidence. Detects standard Docker Desktop socket
//!   (`~/.docker/run/docker.sock`), OrbStack (`~/.orbstack/run/docker.sock`),
//!   and the default `/var/run/docker.sock`. Podman machine sockets under
//!   `~/.local/share/containers/podman/machine/` are also checked.
//!
//! - **Linux**: High confidence for standard installs. Rootless Docker uses
//!   a different socket path (`/run/user/$UID/docker.sock`) which is now
//!   checked by the fallback in `connect_docker()`. Rootless Podman sockets
//!   at `$XDG_RUNTIME_DIR/podman/podman.sock` are also checked. If
//!   `DOCKER_HOST` is set, bollard's default connection still takes precedence.
//!
//! - **Windows**: Medium confidence. Binary detection uses `where.exe` which
//!   works reliably. Daemon detection relies on bollard's default named pipe
//!   connection (`//./pipe/docker_engine`) which works with Docker Desktop.
//!   The Unix socket fallback in `connect_docker()` is a no-op on Windows,
//!   so detection also probes `docker version`/`docker info` via CLI if the
//!   named pipe is unavailable.

use std::fmt;

/// Which container runtime was detected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerRuntime {
    Docker,
    Podman,
}

impl ContainerRuntime {
    /// Short lowercase name suitable for log messages and display.
    pub fn as_str(&self) -> &'static str {
        match self {
            ContainerRuntime::Docker => "docker",
            ContainerRuntime::Podman => "podman",
        }
    }
}

impl fmt::Display for ContainerRuntime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ContainerRuntime::Docker => "Docker",
            ContainerRuntime::Podman => "Podman",
        })
    }
}

/// Docker daemon availability status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DockerStatus {
    /// A container runtime binary was found on PATH and daemon responding to ping.
    Available,
    /// No container runtime binary (`docker` or `podman`) found on PATH.
    NotInstalled,
    /// Binary found but daemon not responding.
    NotRunning,
    /// Sandbox feature not enabled (no check performed).
    Disabled,
}

impl DockerStatus {
    /// Returns true if a container runtime is available and ready.
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

    /// Installation instructions for Docker/Podman on this platform.
    pub fn install_hint(&self) -> &'static str {
        match self {
            Platform::MacOS => {
                "Install Docker Desktop: https://docs.docker.com/desktop/install/mac-install/\n\
                 or Podman Desktop: https://podman-desktop.io/"
            }
            Platform::Linux => {
                "Install Docker Engine: https://docs.docker.com/engine/install/\n\
                 or Podman: https://podman.io/docs/installation#installing-on-linux"
            }
            Platform::Windows => {
                "Install Docker Desktop: https://docs.docker.com/desktop/install/windows-install/\n\
                 or Podman Desktop: https://podman-desktop.io/"
            }
        }
    }

    /// Instructions to start the container runtime daemon on this platform.
    pub fn start_hint(&self) -> &'static str {
        match self {
            Platform::MacOS => {
                "Start Docker Desktop from Applications (open -a Docker), \
                 or start Podman machine: podman machine start"
            }
            Platform::Linux => {
                "Start Docker: sudo systemctl start docker, \
                 or start Podman: systemctl --user start podman.socket"
            }
            Platform::Windows => "Start Docker Desktop or Podman Desktop from the Start menu",
        }
    }
}

/// Result of a container runtime detection check.
pub struct DockerDetection {
    pub status: DockerStatus,
    pub platform: Platform,
    /// Which container runtime was detected (set when status is `Available` or `NotRunning`).
    pub runtime: Option<ContainerRuntime>,
}

/// Check whether a container runtime (Docker or Podman) is installed and running.
///
/// 1. Checks if `docker` or `podman` binary exists on PATH
/// 2. If found, tries to connect and ping the daemon via `connect_docker()`
/// 3. Returns `Available`, `NotInstalled`, or `NotRunning`
///
/// When both Docker and Podman binaries are present, Docker is preferred.
pub async fn check_docker() -> DockerDetection {
    let platform = Platform::current();

    let has_docker = docker_binary_exists();
    let has_podman = podman_binary_exists();

    // Step 1: Check if any container runtime binary is on PATH
    if !has_docker && !has_podman {
        return DockerDetection {
            status: DockerStatus::NotInstalled,
            platform,
            runtime: None,
        };
    }

    // Determine which runtime to attribute (prefer Docker when both present)
    let runtime = if has_docker {
        ContainerRuntime::Docker
    } else {
        ContainerRuntime::Podman
    };

    // Step 2: Try to connect to the daemon (bollard works with both Docker and Podman sockets)
    if crate::sandbox::connect_docker().await.is_ok() {
        return DockerDetection {
            status: DockerStatus::Available,
            platform,
            runtime: Some(runtime),
        };
    }

    // Windows fallback: if the named pipe probe fails but docker CLI can still
    // reach the daemon/server, treat it as available.
    #[cfg(windows)]
    if has_docker && docker_cli_daemon_reachable() {
        return DockerDetection {
            status: DockerStatus::Available,
            platform,
            runtime: Some(ContainerRuntime::Docker),
        };
    }

    DockerDetection {
        status: DockerStatus::NotRunning,
        platform,
        runtime: Some(runtime),
    }
}

/// Check if the `docker` binary exists on PATH.
fn docker_binary_exists() -> bool {
    binary_exists("docker")
}

/// Check if the `podman` binary exists on PATH.
fn podman_binary_exists() -> bool {
    binary_exists("podman")
}

/// Check if a binary exists on PATH.
fn binary_exists(name: &str) -> bool {
    #[cfg(unix)]
    {
        std::process::Command::new("which")
            .arg(name)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }
    #[cfg(windows)]
    {
        std::process::Command::new("where")
            .arg(name)
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
            DockerStatus::Available => {
                assert!(
                    result.runtime.is_some(),
                    "Available status should have runtime"
                );
            }
            DockerStatus::NotInstalled => {
                assert!(
                    result.runtime.is_none(),
                    "NotInstalled should have no runtime"
                );
            }
            DockerStatus::NotRunning => {
                assert!(result.runtime.is_some(), "NotRunning should have runtime");
            }
            DockerStatus::Disabled => panic!("check_docker should never return Disabled"),
        }
    }

    #[test]
    fn test_container_runtime_display() {
        assert_eq!(ContainerRuntime::Docker.to_string(), "Docker");
        assert_eq!(ContainerRuntime::Podman.to_string(), "Podman");
        assert_eq!(ContainerRuntime::Docker.as_str(), "docker");
        assert_eq!(ContainerRuntime::Podman.as_str(), "podman");
    }
}
