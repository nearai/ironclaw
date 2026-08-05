//! Provider-neutral security posture for the Docker worker container.
//!
//! Local Docker and Railway both launch the same kind of untrusted worker;
//! they differ only in how IronClaw reaches the Docker daemon and persists
//! `/workspace`. Keep the security-sensitive launch flags here so a provider
//! adapter cannot silently drift to a weaker worker.

pub(super) const DOCKER_WORKER_USER: &str = "1000:1000";
pub(super) const DOCKER_WORKER_PIDS_LIMIT: i64 = 1024;
pub(super) const DOCKER_WORKER_NANO_CPUS: i64 = 1_000_000_000;
pub(super) const DOCKER_WORKER_TMPFS: &str = "rw,noexec,nosuid,size=64m";
pub(super) const DOCKER_WORKER_LOG_DRIVER: &str = "json-file";
pub(super) const DOCKER_WORKER_LOG_MAX_SIZE: &str = "1m";
pub(super) const DOCKER_WORKER_LOG_MAX_FILES: &str = "1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DockerWorkerSecuritySpec {
    network_mode: Option<String>,
}

impl DockerWorkerSecuritySpec {
    pub(super) fn new(network_mode: Option<String>) -> Self {
        Self { network_mode }
    }

    pub(super) fn user(&self) -> String {
        DOCKER_WORKER_USER.to_string()
    }

    pub(super) fn cap_drop(&self) -> Vec<String> {
        vec!["ALL".to_string()]
    }

    pub(super) fn readonly_rootfs(&self) -> bool {
        true
    }

    pub(super) fn network_mode(&self) -> Option<String> {
        self.network_mode.clone()
    }

    pub(super) fn pids_limit(&self) -> i64 {
        DOCKER_WORKER_PIDS_LIMIT
    }

    pub(super) fn nano_cpus(&self) -> i64 {
        DOCKER_WORKER_NANO_CPUS
    }

    pub(super) fn tmpfs_options(&self) -> String {
        DOCKER_WORKER_TMPFS.to_string()
    }

    pub(super) fn log_driver(&self) -> String {
        DOCKER_WORKER_LOG_DRIVER.to_string()
    }

    pub(super) fn log_options(&self) -> Vec<(String, String)> {
        vec![
            (
                "max-size".to_string(),
                DOCKER_WORKER_LOG_MAX_SIZE.to_string(),
            ),
            (
                "max-file".to_string(),
                DOCKER_WORKER_LOG_MAX_FILES.to_string(),
            ),
        ]
    }

    pub(super) fn security_options(&self) -> Vec<String> {
        vec!["no-new-privileges:true".to_string()]
    }

    /// Render the shared posture for a trusted adapter that invokes the
    /// Docker CLI. Provider-owned image, workspace, and resource-limit args
    /// are appended separately.
    pub(super) fn docker_run_args(&self) -> Vec<String> {
        let mut args = Vec::new();
        if let Some(network_mode) = &self.network_mode {
            args.extend(["--network".to_string(), network_mode.clone()]);
        }
        args.push("--read-only".to_string());
        args.extend(["--user".to_string(), self.user()]);
        args.extend(["--cap-drop".to_string(), "ALL".to_string()]);
        args.extend([
            "--security-opt".to_string(),
            "no-new-privileges:true".to_string(),
        ]);
        args.extend(["--pids-limit".to_string(), self.pids_limit().to_string()]);
        args.extend(["--cpus".to_string(), "1.0".to_string()]);
        args.extend([
            "--tmpfs".to_string(),
            format!("/tmp:{}", self.tmpfs_options()),
        ]);
        args.extend(["--log-driver".to_string(), self.log_driver()]);
        for (key, value) in self.log_options() {
            args.extend(["--log-opt".to_string(), format!("{key}={value}")]);
        }
        args
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn docker_cli_renderer_contains_the_complete_shared_posture() {
        let args = DockerWorkerSecuritySpec::new(Some("none".to_string())).docker_run_args();
        assert_eq!(
            args,
            vec![
                "--network",
                "none",
                "--read-only",
                "--user",
                DOCKER_WORKER_USER,
                "--cap-drop",
                "ALL",
                "--security-opt",
                "no-new-privileges:true",
                "--pids-limit",
                "1024",
                "--cpus",
                "1.0",
                "--tmpfs",
                "/tmp:rw,noexec,nosuid,size=64m",
                "--log-driver",
                "json-file",
                "--log-opt",
                "max-size=1m",
                "--log-opt",
                "max-file=1",
            ]
        );
    }
}
