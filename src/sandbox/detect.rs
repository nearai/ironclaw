//! Proactive Docker detection with platform-specific guidance.
//!
//! Checks whether Docker is both installed (binary on PATH) and running
//! (daemon responding to ping), and provides platform-appropriate
//! installation or startup instructions when it is not.
//!
//! # Detection Limitations
//!
//! - **macOS**: High confidence. Detects both standard Docker Desktop socket
//!   (`~/.docker/run/docker.sock`) and the default `/var/run/docker.sock`.
//!
//! - **Linux**: High confidence for standard installs. Rootless Docker uses
//!   a different socket path (`/run/user/$UID/docker.sock`) which is not
//!   checked by the fallback in `connect_docker()`. If `DOCKER_HOST` is set,
//!   bollard's default connection will use it correctly.
//!
//! - **Windows**: Medium confidence. Binary detection uses `where.exe` which
//!   works reliably. Daemon detection relies on bollard's default named pipe
//!   connection (`//./pipe/docker_engine`) which works with Docker Desktop.
//!   The Unix socket fallback in `connect_docker()` is a no-op on Windows.
//!   If Docker Desktop is running but the named pipe is not available,
//!   detection may report `NotRunning` even though Docker is functional.

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
            Platform::Linux => {
                "Install Docker Engine: https://docs.docker.com/engine/install/"
            }
            Platform::Windows => {
                "Install Docker Desktop: https://docs.docker.com/desktop/install/windows-install/"
            }
        }
    }

    /// Instructions to start the Docker daemon on this platform.
    pub fn start_hint(&self) -> &'static str {
        match self {
            Platform::MacOS => "Start Docker Desktop from Applications, or run: open -a Docker",
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
    match crate::sandbox::connect_docker().await {
        Ok(_) => DockerDetection {
            status: DockerStatus::Available,
            platform,
        },
        Err(_) => DockerDetection {
            status: DockerStatus::NotRunning,
            platform,
        },
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
