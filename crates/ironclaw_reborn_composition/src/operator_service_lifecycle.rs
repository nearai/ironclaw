//! Local OS service lifecycle backend for the Reborn operator facade.
//!
//! This is the concrete implementation behind
//! `POST /api/webchat/v2/operator/service`. It intentionally accepts only the
//! fixed `ironclaw-reborn` unit/label and fixed command argv shapes; browser
//! input can select an action, not a command line.

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_product_workflow::{
    OperatorServiceLifecycleService, RebornServiceLifecycleAction, RebornServiceLifecycleRequest,
    RebornServiceLifecycleResponse, RebornServiceLifecycleState, RebornServicesError,
    WebUiAuthenticatedCaller,
};

const LAUNCHD_LABEL: &str = "com.ironclaw.reborn";
const SYSTEMD_UNIT: &str = "ironclaw-reborn.service";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ServicePlatform {
    Linux,
    Macos,
    Unsupported,
}

impl ServicePlatform {
    fn current() -> Self {
        if cfg!(target_os = "linux") {
            Self::Linux
        } else if cfg!(target_os = "macos") {
            Self::Macos
        } else {
            Self::Unsupported
        }
    }
}

#[derive(Debug, Clone)]
struct CommandOutput {
    success: bool,
    stdout: String,
}

trait ServiceCommandRunner: Send + Sync {
    fn run(&self, program: &str, args: &[&str]) -> Result<CommandOutput, String>;
}

#[derive(Debug, Default)]
struct SystemCommandRunner;

impl ServiceCommandRunner for SystemCommandRunner {
    fn run(&self, program: &str, args: &[&str]) -> Result<CommandOutput, String> {
        let output = Command::new(program)
            .args(args)
            .output()
            .map_err(|_| "service manager command could not be started".to_string())?;
        Ok(CommandOutput {
            success: output.status.success(),
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        })
    }
}

/// Platform-backed local service lifecycle manager.
#[derive(Clone)]
pub struct RebornLocalServiceLifecycle {
    platform: ServicePlatform,
    home_dir: Option<PathBuf>,
    executable: PathBuf,
    runner: Arc<dyn ServiceCommandRunner>,
}

impl std::fmt::Debug for RebornLocalServiceLifecycle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RebornLocalServiceLifecycle")
            .field("platform", &self.platform)
            .field("home_dir", &self.home_dir.is_some())
            .field("executable", &"<redacted>")
            .finish_non_exhaustive()
    }
}

impl RebornLocalServiceLifecycle {
    pub fn new() -> Self {
        Self {
            platform: ServicePlatform::current(),
            home_dir: std::env::var_os("HOME").map(PathBuf::from),
            executable: std::env::current_exe()
                .unwrap_or_else(|_| PathBuf::from("ironclaw-reborn")),
            runner: Arc::new(SystemCommandRunner),
        }
    }

    #[cfg(test)]
    fn for_test(
        platform: ServicePlatform,
        home_dir: Option<PathBuf>,
        executable: PathBuf,
        runner: Arc<dyn ServiceCommandRunner>,
    ) -> Self {
        Self {
            platform,
            home_dir,
            executable,
            runner,
        }
    }

    fn unsupported_response(
        action: RebornServiceLifecycleAction,
    ) -> RebornServiceLifecycleResponse {
        RebornServiceLifecycleResponse {
            action,
            state: RebornServiceLifecycleState::Unsupported,
            message: "local service lifecycle is unsupported on this OS target".to_string(),
            remediation: Some(
                "manage this deployment with the host process supervisor and keep the WebUI operator service endpoint disabled for lifecycle control"
                    .to_string(),
            ),
        }
    }

    fn missing_home_response(
        action: RebornServiceLifecycleAction,
    ) -> RebornServiceLifecycleResponse {
        RebornServiceLifecycleResponse {
            action,
            state: RebornServiceLifecycleState::Failed,
            message: "local service lifecycle cannot resolve the operator home directory"
                .to_string(),
            remediation: Some("set HOME and retry the lifecycle operation".to_string()),
        }
    }

    fn failed_response(
        action: RebornServiceLifecycleAction,
        message: &str,
    ) -> RebornServiceLifecycleResponse {
        RebornServiceLifecycleResponse {
            action,
            state: RebornServiceLifecycleState::Failed,
            message: message.to_string(),
            remediation: Some("inspect the local service manager and retry".to_string()),
        }
    }

    fn service_file(&self) -> Option<PathBuf> {
        let home = self.home_dir.as_ref()?;
        match self.platform {
            ServicePlatform::Linux => Some(home.join(".config/systemd/user").join(SYSTEMD_UNIT)),
            ServicePlatform::Macos => Some(
                home.join("Library")
                    .join("LaunchAgents")
                    .join(format!("{LAUNCHD_LABEL}.plist")),
            ),
            ServicePlatform::Unsupported => None,
        }
    }

    fn service_file_for_action(
        &self,
        action: RebornServiceLifecycleAction,
    ) -> Result<PathBuf, RebornServiceLifecycleResponse> {
        if self.platform == ServicePlatform::Unsupported {
            return Err(Self::unsupported_response(action));
        }
        self.service_file()
            .ok_or_else(|| Self::missing_home_response(action))
    }

    fn install(&self) -> RebornServiceLifecycleResponse {
        let action = RebornServiceLifecycleAction::Install;
        let path = match self.service_file_for_action(action) {
            Ok(path) => path,
            Err(response) => return response,
        };
        let Some(parent) = path.parent() else {
            return Self::missing_home_response(action);
        };
        if std::fs::create_dir_all(parent).is_err() {
            return Self::failed_response(
                action,
                "local service unit directory could not be created",
            );
        }
        let write = match self.platform {
            ServicePlatform::Linux => std::fs::write(&path, self.systemd_unit()),
            ServicePlatform::Macos => std::fs::write(&path, self.launchd_plist()),
            ServicePlatform::Unsupported => unreachable!("handled above"),
        };
        if write.is_err() {
            return Self::failed_response(action, "local service unit could not be written");
        }
        if self.platform == ServicePlatform::Linux {
            let _ = self.runner.run("systemctl", &["--user", "daemon-reload"]);
            let _ = self
                .runner
                .run("systemctl", &["--user", "enable", SYSTEMD_UNIT]);
        }
        RebornServiceLifecycleResponse {
            action,
            state: RebornServiceLifecycleState::Installed,
            message: "local Reborn service unit is installed".to_string(),
            remediation: None,
        }
    }

    fn start(&self) -> RebornServiceLifecycleResponse {
        let action = RebornServiceLifecycleAction::Start;
        match self.platform {
            ServicePlatform::Linux => {
                let _ = self.runner.run("systemctl", &["--user", "daemon-reload"]);
                self.run_checked(
                    action,
                    "systemctl",
                    &["--user", "start", SYSTEMD_UNIT],
                    RebornServiceLifecycleState::Running,
                    "local Reborn service is running",
                )
            }
            ServicePlatform::Macos => {
                let path = match self.service_file_for_action(action) {
                    Ok(path) => path,
                    Err(response) => return response,
                };
                let path = path.to_string_lossy().to_string();
                let load = self.runner.run("launchctl", &["load", "-w", &path]);
                if !matches!(load.as_ref(), Ok(output) if output.success) {
                    return Self::failed_response(action, "launchd service could not be loaded");
                }
                self.run_checked(
                    action,
                    "launchctl",
                    &["start", LAUNCHD_LABEL],
                    RebornServiceLifecycleState::Running,
                    "local Reborn service is running",
                )
            }
            ServicePlatform::Unsupported => Self::unsupported_response(action),
        }
    }

    fn stop(&self) -> RebornServiceLifecycleResponse {
        let action = RebornServiceLifecycleAction::Stop;
        match self.platform {
            ServicePlatform::Linux => self.run_checked(
                action,
                "systemctl",
                &["--user", "stop", SYSTEMD_UNIT],
                RebornServiceLifecycleState::Stopped,
                "local Reborn service is stopped",
            ),
            ServicePlatform::Macos => {
                let path = match self.service_file_for_action(action) {
                    Ok(path) => path,
                    Err(response) => return response,
                };
                let path = path.to_string_lossy().to_string();
                let _ = self.runner.run("launchctl", &["stop", LAUNCHD_LABEL]);
                let _ = self.runner.run("launchctl", &["unload", "-w", &path]);
                RebornServiceLifecycleResponse {
                    action,
                    state: RebornServiceLifecycleState::Stopped,
                    message: "local Reborn service is stopped".to_string(),
                    remediation: None,
                }
            }
            ServicePlatform::Unsupported => Self::unsupported_response(action),
        }
    }

    fn status(&self) -> RebornServiceLifecycleResponse {
        let action = RebornServiceLifecycleAction::Status;
        match self.platform {
            ServicePlatform::Linux => {
                let output = self
                    .runner
                    .run("systemctl", &["--user", "is-active", SYSTEMD_UNIT]);
                match output {
                    Ok(output) if output.success && output.stdout.trim() == "active" => {
                        Self::status_response(
                            RebornServiceLifecycleState::Running,
                            "local Reborn service is running",
                        )
                    }
                    Ok(output) if matches!(output.stdout.trim(), "inactive" | "deactivating") => {
                        Self::status_response(
                            RebornServiceLifecycleState::Stopped,
                            "local Reborn service is stopped",
                        )
                    }
                    Ok(output) if output.stdout.trim() == "failed" => Self::status_response(
                        RebornServiceLifecycleState::Failed,
                        "local Reborn service is failed",
                    ),
                    Ok(_) => Self::status_response(
                        RebornServiceLifecycleState::Unknown,
                        "local Reborn service state is unknown",
                    ),
                    Err(_) => Self::failed_response(
                        action,
                        "local service manager status could not be queried",
                    ),
                }
            }
            ServicePlatform::Macos => {
                let output = self.runner.run("launchctl", &["list"]);
                match output {
                    Ok(output)
                        if output
                            .stdout
                            .lines()
                            .any(|line| line.contains(LAUNCHD_LABEL)) =>
                    {
                        Self::status_response(
                            RebornServiceLifecycleState::Running,
                            "local Reborn service is running",
                        )
                    }
                    Ok(_) => Self::status_response(
                        RebornServiceLifecycleState::Stopped,
                        "local Reborn service is stopped",
                    ),
                    Err(_) => Self::failed_response(
                        action,
                        "local service manager status could not be queried",
                    ),
                }
            }
            ServicePlatform::Unsupported => Self::unsupported_response(action),
        }
    }

    fn status_response(
        state: RebornServiceLifecycleState,
        message: &str,
    ) -> RebornServiceLifecycleResponse {
        RebornServiceLifecycleResponse {
            action: RebornServiceLifecycleAction::Status,
            state,
            message: message.to_string(),
            remediation: None,
        }
    }

    fn run_checked(
        &self,
        action: RebornServiceLifecycleAction,
        program: &str,
        args: &[&str],
        success_state: RebornServiceLifecycleState,
        success_message: &str,
    ) -> RebornServiceLifecycleResponse {
        match self.runner.run(program, args) {
            Ok(output) if output.success => RebornServiceLifecycleResponse {
                action,
                state: success_state,
                message: success_message.to_string(),
                remediation: None,
            },
            Ok(_) | Err(_) => Self::failed_response(action, "local service manager command failed"),
        }
    }

    fn systemd_unit(&self) -> String {
        let exe = systemd_escape(self.executable.to_string_lossy().as_ref());
        format!(
            "[Unit]\n\
             Description=IronClaw Reborn WebUI service\n\
             After=network.target\n\
             \n\
             [Service]\n\
             Type=simple\n\
             ExecStart=\"{exe}\" serve\n\
             Restart=always\n\
             RestartSec=3\n\
             \n\
             [Install]\n\
             WantedBy=default.target\n"
        )
    }

    fn launchd_plist(&self) -> String {
        let exe = xml_escape(self.executable.to_string_lossy().as_ref());
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{LAUNCHD_LABEL}</string>
  <key>ProgramArguments</key>
  <array>
    <string>{exe}</string>
    <string>serve</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <true/>
</dict>
</plist>
"#
        )
    }
}

impl Default for RebornLocalServiceLifecycle {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl OperatorServiceLifecycleService for RebornLocalServiceLifecycle {
    async fn control_service(
        &self,
        _caller: WebUiAuthenticatedCaller,
        request: RebornServiceLifecycleRequest,
    ) -> Result<RebornServiceLifecycleResponse, RebornServicesError> {
        let service = self.clone();
        let action = request.action;
        tokio::task::spawn_blocking(move || match action {
            RebornServiceLifecycleAction::Install => service.install(),
            RebornServiceLifecycleAction::Start => service.start(),
            RebornServiceLifecycleAction::Stop => service.stop(),
            RebornServiceLifecycleAction::Status => service.status(),
        })
        .await
        .map_err(|_| RebornServicesError::internal())
    }
}

fn systemd_escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn xml_escape(raw: &str) -> String {
    raw.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use tempfile::TempDir;

    #[derive(Debug)]
    struct RecordingRunner {
        calls: Mutex<Vec<(String, Vec<String>)>>,
        status_stdout: Mutex<String>,
        fail_command: Mutex<Option<(String, Vec<String>)>>,
    }

    impl RecordingRunner {
        fn new(status_stdout: &str) -> Self {
            Self {
                calls: Mutex::default(),
                status_stdout: Mutex::new(status_stdout.to_string()),
                fail_command: Mutex::new(None),
            }
        }

        fn fail_command(&self, program: &str, args: &[&str]) {
            *self.fail_command.lock().expect("lock") = Some((
                program.to_string(),
                args.iter().map(|arg| (*arg).to_string()).collect(),
            ));
        }

        fn calls(&self) -> Vec<(String, Vec<String>)> {
            self.calls.lock().expect("lock").clone()
        }
    }

    impl ServiceCommandRunner for RecordingRunner {
        fn run(&self, program: &str, args: &[&str]) -> Result<CommandOutput, String> {
            self.calls.lock().expect("lock").push((
                program.to_string(),
                args.iter().map(|arg| (*arg).to_string()).collect(),
            ));
            let command = (
                program.to_string(),
                args.iter()
                    .map(|arg| (*arg).to_string())
                    .collect::<Vec<_>>(),
            );
            if self
                .fail_command
                .lock()
                .expect("lock")
                .as_ref()
                .is_some_and(|failed_command| failed_command == &command)
            {
                return Ok(CommandOutput {
                    success: false,
                    stdout: String::new(),
                });
            }
            let stdout = if program == "systemctl" && args.ends_with(&["is-active", SYSTEMD_UNIT]) {
                self.status_stdout.lock().expect("lock").clone()
            } else {
                String::new()
            };
            Ok(CommandOutput {
                success: true,
                stdout,
            })
        }
    }

    fn linux_service(temp: &TempDir, runner: Arc<RecordingRunner>) -> RebornLocalServiceLifecycle {
        RebornLocalServiceLifecycle::for_test(
            ServicePlatform::Linux,
            Some(temp.path().to_path_buf()),
            PathBuf::from("/usr/local/bin/ironclaw-reborn"),
            runner,
        )
    }

    #[tokio::test]
    async fn linux_install_writes_unit_and_runs_allowlisted_systemctl_commands() {
        let temp = TempDir::new().expect("tempdir");
        let runner = Arc::new(RecordingRunner::new("inactive"));
        let service = linux_service(&temp, runner.clone());

        let response = service
            .control_service(
                test_caller(),
                RebornServiceLifecycleRequest {
                    action: RebornServiceLifecycleAction::Install,
                },
            )
            .await
            .expect("install response");

        assert_eq!(response.state, RebornServiceLifecycleState::Installed);
        let unit_path = temp.path().join(".config/systemd/user").join(SYSTEMD_UNIT);
        let unit = std::fs::read_to_string(unit_path).expect("unit file");
        assert!(unit.contains("ExecStart=\"/usr/local/bin/ironclaw-reborn\" serve"));
        assert_eq!(
            runner.calls(),
            vec![
                (
                    "systemctl".to_string(),
                    vec!["--user".to_string(), "daemon-reload".to_string()],
                ),
                (
                    "systemctl".to_string(),
                    vec![
                        "--user".to_string(),
                        "enable".to_string(),
                        SYSTEMD_UNIT.to_string()
                    ],
                ),
            ]
        );
    }

    #[tokio::test]
    async fn linux_status_maps_service_manager_output_without_raw_command_text() {
        let temp = TempDir::new().expect("tempdir");
        let runner = Arc::new(RecordingRunner::new("active\n"));
        let service = linux_service(&temp, runner);

        let response = service
            .control_service(
                test_caller(),
                RebornServiceLifecycleRequest {
                    action: RebornServiceLifecycleAction::Status,
                },
            )
            .await
            .expect("status response");

        assert_eq!(response.state, RebornServiceLifecycleState::Running);
        assert_eq!(response.message, "local Reborn service is running");
        assert!(!response.message.contains("systemctl"));
    }

    #[tokio::test]
    async fn linux_start_failure_returns_failed_state() {
        let temp = TempDir::new().expect("tempdir");
        let runner = Arc::new(RecordingRunner::new("inactive"));
        runner.fail_command("systemctl", &["--user", "start", SYSTEMD_UNIT]);
        let service = linux_service(&temp, runner);

        let response = service
            .control_service(
                test_caller(),
                RebornServiceLifecycleRequest {
                    action: RebornServiceLifecycleAction::Start,
                },
            )
            .await
            .expect("start response");

        assert_eq!(response.state, RebornServiceLifecycleState::Failed);
        assert!(response.remediation.is_some());
    }

    #[tokio::test]
    async fn install_without_home_reports_failed_resolution() {
        let service = RebornLocalServiceLifecycle::for_test(
            ServicePlatform::Linux,
            None,
            PathBuf::from("/usr/local/bin/ironclaw-reborn"),
            Arc::new(RecordingRunner::new("")),
        );

        let response = service
            .control_service(
                test_caller(),
                RebornServiceLifecycleRequest {
                    action: RebornServiceLifecycleAction::Install,
                },
            )
            .await
            .expect("missing home response");

        assert_eq!(response.state, RebornServiceLifecycleState::Failed);
        assert!(response.message.contains("home directory"));
    }

    #[tokio::test]
    async fn unsupported_platform_reports_unsupported() {
        let service = RebornLocalServiceLifecycle::for_test(
            ServicePlatform::Unsupported,
            None,
            PathBuf::from("/usr/local/bin/ironclaw-reborn"),
            Arc::new(RecordingRunner::new("")),
        );

        let response = service
            .control_service(
                test_caller(),
                RebornServiceLifecycleRequest {
                    action: RebornServiceLifecycleAction::Install,
                },
            )
            .await
            .expect("unsupported response");

        assert_eq!(response.state, RebornServiceLifecycleState::Unsupported);
        assert!(response.remediation.is_some());
    }

    fn test_caller() -> WebUiAuthenticatedCaller {
        WebUiAuthenticatedCaller::new(
            ironclaw_host_api::TenantId::new("tenant-test").expect("tenant"),
            ironclaw_host_api::UserId::new("user-test").expect("user"),
            Some(ironclaw_host_api::AgentId::new("agent-test").expect("agent")),
            None,
        )
    }
}
