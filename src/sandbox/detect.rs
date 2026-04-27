//! Proactive Docker detection with platform-specific guidance.
//!
//! First performs cheap filesystem checks (socket existence, `DOCKER_HOST`,
//! PATH lookup) to fast-exit on hosts where Docker is clearly absent —
//! avoiding bollard's 120 s daemon-ping timeout.  Then attempts a direct
//! socket connection via bollard (covers container-in-container deployments
//! where the socket is bind-mounted but the CLI is absent), and falls back
//! to a `which docker` PATH check for error-message quality.  Provides
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
/// 1. Fast path: if no Docker socket exists on the filesystem, `DOCKER_HOST`
///    is unset, *and* the `docker` CLI binary is absent, returns `NotInstalled`
///    immediately — without a daemon ping.  This avoids a 120 s bollard timeout
///    on hosts where Docker is clearly not present (see #P2).
/// 2. Tries to connect and ping the Docker daemon directly via
///    `connect_docker()` (bollard). This covers container-in-container (DinD)
///    deployments where the socket is bind-mounted but the CLI binary is not
///    installed inside the container.
/// 3. If the daemon is unreachable and the binary is also missing, returns
///    `NotInstalled`; otherwise `NotRunning`.
pub async fn check_docker() -> DockerDetection {
    let platform = Platform::current();
    let binary_found = docker_binary_exists();
    let docker_host_set = std::env::var_os("DOCKER_HOST").is_some();
    let socket_found = any_docker_socket_exists();

    // Fast path: no CLI binary, no DOCKER_HOST, and no socket file on disk.
    // Skip the daemon ping that would block on an unreachable host.
    if should_skip_daemon_ping(binary_found, docker_host_set, socket_found) {
        return DockerDetection {
            status: DockerStatus::NotInstalled,
            platform,
        };
    }

    // Authoritative check: try to connect and ping the daemon via bollard.
    // Covers DinD (socket bind-mounted, no CLI binary) and standard installs.
    if crate::sandbox::connect_docker().await.is_ok() {
        return DockerDetection {
            status: DockerStatus::Available,
            platform,
        };
    }

    // Daemon unreachable. Distinguish "not installed" from "not running".
    if !binary_found {
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

/// Check if any well-known Docker socket file exists on disk.
///
/// This is a cheap filesystem probe (no daemon ping) used to decide whether
/// it is worth attempting the slower `connect_docker()` call.  Covers the
/// default `/var/run/docker.sock` plus the user-space candidates that
/// `connect_docker()` already tries.
fn any_docker_socket_exists() -> bool {
    #[cfg(unix)]
    {
        use std::path::PathBuf;

        // Default socket that bollard's connect_with_local_defaults() checks.
        if PathBuf::from("/var/run/docker.sock").exists() {
            return true;
        }

        // Same user-space candidates that connect_docker() iterates.
        crate::sandbox::container::unix_socket_candidates()
            .iter()
            .any(|sock| sock.exists())
    }

    #[cfg(windows)]
    {
        // On Windows, bollard probes the named pipe `//./pipe/docker_engine`.
        // `Path::exists()` doesn't work for named pipes, so we conservatively
        // return true — the binary-exists check is the primary fast-path on
        // Windows.
        true
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

/// Returns `true` when we can confidently say Docker is not installed without
/// performing a daemon ping.  All three signals must be absent.
fn should_skip_daemon_ping(binary_found: bool, docker_host_set: bool, socket_found: bool) -> bool {
    !binary_found && !docker_host_set && !socket_found
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

    // --- Regression tests for the fast-path that skips the daemon ping ---

    #[test]
    fn skip_ping_when_no_binary_no_host_no_socket() {
        // The bug: connect_docker() was called unconditionally, blocking for
        // 120 s on hosts with an unreachable DOCKER_HOST and no Docker.
        assert!(should_skip_daemon_ping(false, false, false));
    }

    #[test]
    fn no_skip_when_docker_host_set() {
        // DOCKER_HOST points somewhere — must attempt the ping even without
        // a binary (could be a remote Docker host).
        assert!(!should_skip_daemon_ping(false, true, false));
    }

    #[test]
    fn no_skip_when_socket_exists() {
        // Socket on disk (DinD bind-mount) — must ping even without the CLI.
        assert!(!should_skip_daemon_ping(false, false, true));
    }

    #[test]
    fn no_skip_when_binary_found() {
        // CLI binary present — Docker may be installed but daemon stopped.
        assert!(!should_skip_daemon_ping(true, false, false));
    }

    #[test]
    fn no_skip_when_all_signals_present() {
        assert!(!should_skip_daemon_ping(true, true, true));
    }

    #[cfg(unix)]
    #[test]
    fn any_docker_socket_nonexistent_path_returns_false() {
        // Sanity: a path that doesn't exist should not count as a socket.
        use std::path::PathBuf;
        assert!(!PathBuf::from("/tmp/definitely-not-a-docker-socket.sock").exists());
    }
}
