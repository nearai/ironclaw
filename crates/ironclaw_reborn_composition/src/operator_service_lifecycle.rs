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
use ironclaw_host_api::UserId;
use ironclaw_product_workflow::{
    OperatorServiceLifecycleService, RebornServiceLifecycleAction, RebornServiceLifecycleRequest,
    RebornServiceLifecycleResponse, RebornServiceLifecycleState, RebornServicesError,
    RebornServicesErrorCode, RebornServicesErrorKind, WebUiAuthenticatedCaller,
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
            .map_err(|error| format!("service manager command could not be started: {error}"))?;
        Ok(CommandOutput {
            success: output.status.success(),
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        })
    }
}

/// Platform-backed local service lifecycle manager.
#[derive(Clone)]
pub(crate) struct RebornLocalServiceLifecycle {
    platform: ServicePlatform,
    home_dir: Option<PathBuf>,
    executable: Result<PathBuf, String>,
    operator_user_id: Option<UserId>,
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
    pub(crate) fn new() -> Self {
        Self {
            platform: ServicePlatform::current(),
            home_dir: std::env::var_os("HOME").map(PathBuf::from),
            executable: std::env::current_exe()
                .map_err(|error| format!("current executable path could not be resolved: {error}")),
            operator_user_id: None,
            runner: Arc::new(SystemCommandRunner),
        }
    }

    pub(crate) fn new_for_operator(operator_user_id: UserId) -> Self {
        Self {
            platform: ServicePlatform::current(),
            home_dir: std::env::var_os("HOME").map(PathBuf::from),
            executable: std::env::current_exe()
                .map_err(|error| format!("current executable path could not be resolved: {error}")),
            operator_user_id: Some(operator_user_id),
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
            executable: Ok(executable),
            operator_user_id: Some(test_operator_user_id()),
            runner,
        }
    }

    #[cfg(test)]
    fn for_test_with_executable_error(
        platform: ServicePlatform,
        home_dir: Option<PathBuf>,
        executable_error: String,
        runner: Arc<dyn ServiceCommandRunner>,
    ) -> Self {
        Self {
            platform,
            home_dir,
            executable: Err(executable_error),
            operator_user_id: Some(test_operator_user_id()),
            runner,
        }
    }

    #[cfg(test)]
    fn with_operator_user_id(mut self, operator_user_id: UserId) -> Self {
        self.operator_user_id = Some(operator_user_id);
        self
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

    fn executable_path_for_action(
        &self,
        action: RebornServiceLifecycleAction,
    ) -> Result<&PathBuf, RebornServiceLifecycleResponse> {
        self.executable
            .as_ref()
            .map_err(|message| Self::failed_response(action, message))
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
        if let Err(error) = std::fs::create_dir_all(parent) {
            return Self::failed_response(
                action,
                &format!("local service unit directory could not be created: {error}"),
            );
        }
        let write = match self.platform {
            ServicePlatform::Linux => match self.systemd_unit(action) {
                Ok(unit) => std::fs::write(&path, unit),
                Err(response) => return response,
            },
            ServicePlatform::Macos => match self.launchd_plist(action) {
                Ok(plist) => std::fs::write(&path, plist),
                Err(response) => return response,
            },
            ServicePlatform::Unsupported => unreachable!("handled above"),
        };
        if let Err(error) = write {
            return Self::failed_response(
                action,
                &format!("local service unit could not be written: {error}"),
            );
        }
        if self.platform == ServicePlatform::Linux {
            // silent-ok: best-effort post-install reload, operator can manually retry.
            let _ = self.runner.run("systemctl", &["--user", "daemon-reload"]);
            // silent-ok: best-effort post-install enable, unit has already been written.
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
                // silent-ok: best-effort reload before start, failure does not block start attempt.
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
                let _ = self.runner.run("launchctl", &["load", "-w", &path]);
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
                // silent-ok: best-effort stop+unload, idempotent on already-stopped service.
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
                    Ok(output) if launchd_status_is_running(&output.stdout) => {
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

    fn systemd_unit(
        &self,
        action: RebornServiceLifecycleAction,
    ) -> Result<String, RebornServiceLifecycleResponse> {
        let executable = self.executable_path_for_action(action)?;
        let exe = systemd_escape(executable.to_string_lossy().as_ref());
        Ok(format!(
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
        ))
    }

    fn launchd_plist(
        &self,
        action: RebornServiceLifecycleAction,
    ) -> Result<String, RebornServiceLifecycleResponse> {
        let executable = self.executable_path_for_action(action)?;
        let exe = xml_escape(executable.to_string_lossy().as_ref());
        Ok(format!(
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
        ))
    }

    fn ensure_authorized_operator(
        &self,
        caller: &WebUiAuthenticatedCaller,
    ) -> Result<(), RebornServicesError> {
        if self
            .operator_user_id
            .as_ref()
            .is_some_and(|operator_user_id| caller.user_id == *operator_user_id)
        {
            return Ok(());
        }
        Err(RebornServicesError {
            code: RebornServicesErrorCode::Forbidden,
            kind: RebornServicesErrorKind::ParticipantDenied,
            status_code: 403,
            retryable: false,
            field: None,
            validation_code: None,
        })
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
        caller: WebUiAuthenticatedCaller,
        request: RebornServiceLifecycleRequest,
    ) -> Result<RebornServiceLifecycleResponse, RebornServicesError> {
        self.ensure_authorized_operator(&caller)?;
        let service = self.clone();
        let action = request.action;
        tokio::task::spawn_blocking(move || match action {
            RebornServiceLifecycleAction::Install => service.install(),
            RebornServiceLifecycleAction::Start => service.start(),
            RebornServiceLifecycleAction::Stop => service.stop(),
            RebornServiceLifecycleAction::Status => service.status(),
        })
        .await
        .map_err(|error| {
            tracing::warn!(%error, "service lifecycle task failed");
            RebornServicesError::internal()
        })
    }
}

fn systemd_escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('%', "%%")
        .replace('$', "$$")
}

fn launchd_status_is_running(stdout: &str) -> bool {
    stdout.lines().any(|line| {
        if !line.contains(LAUNCHD_LABEL) {
            return false;
        }
        let Some(pid) = line.split_whitespace().next() else {
            return false;
        };
        pid.parse::<i32>().is_ok()
    })
}

fn xml_escape(raw: &str) -> String {
    raw.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
fn test_operator_user_id() -> UserId {
    ironclaw_host_api::UserId::new("user-test").expect("test operator user")
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
            let reports_status = (program == "systemctl"
                && args.ends_with(&["is-active", SYSTEMD_UNIT]))
                || (program == "launchctl" && args == ["list"]);
            let stdout = if reports_status {
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

    fn macos_service(temp: &TempDir, runner: Arc<RecordingRunner>) -> RebornLocalServiceLifecycle {
        RebornLocalServiceLifecycle::for_test(
            ServicePlatform::Macos,
            Some(temp.path().to_path_buf()),
            PathBuf::from("/usr/local/bin/ironclaw-reborn"),
            runner,
        )
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
    async fn linux_install_escapes_systemd_special_characters_in_executable_path() {
        let temp = TempDir::new().expect("tempdir");
        let runner = Arc::new(RecordingRunner::new("inactive"));
        let service = RebornLocalServiceLifecycle::for_test(
            ServicePlatform::Linux,
            Some(temp.path().to_path_buf()),
            PathBuf::from("/usr/local/bin/iron%claw-$reborn"),
            runner,
        );

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
        assert!(unit.contains("ExecStart=\"/usr/local/bin/iron%%claw-$$reborn\" serve"));
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
    async fn install_without_executable_path_fails_before_writing_unit() {
        let temp = TempDir::new().expect("tempdir");
        let service = RebornLocalServiceLifecycle::for_test_with_executable_error(
            ServicePlatform::Linux,
            Some(temp.path().to_path_buf()),
            "current executable path could not be resolved: denied".to_string(),
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
            .expect("executable failure response");

        assert_eq!(response.state, RebornServiceLifecycleState::Failed);
        assert!(
            response
                .message
                .contains("current executable path could not be resolved")
        );
        assert!(
            !temp
                .path()
                .join(".config/systemd/user")
                .join(SYSTEMD_UNIT)
                .exists()
        );
    }

    #[tokio::test]
    async fn macos_start_continues_when_launchctl_load_reports_already_loaded() {
        let temp = TempDir::new().expect("tempdir");
        let runner = Arc::new(RecordingRunner::new(""));
        let path = temp
            .path()
            .join("Library")
            .join("LaunchAgents")
            .join(format!("{LAUNCHD_LABEL}.plist"));
        let path_string = path.to_string_lossy().to_string();
        runner.fail_command("launchctl", &["load", "-w", &path_string]);
        let service = macos_service(&temp, runner.clone());

        let response = service
            .control_service(
                test_caller(),
                RebornServiceLifecycleRequest {
                    action: RebornServiceLifecycleAction::Start,
                },
            )
            .await
            .expect("start response");

        assert_eq!(response.state, RebornServiceLifecycleState::Running);
        assert_eq!(
            runner.calls(),
            vec![
                (
                    "launchctl".to_string(),
                    vec!["load".to_string(), "-w".to_string(), path_string],
                ),
                (
                    "launchctl".to_string(),
                    vec!["start".to_string(), LAUNCHD_LABEL.to_string()],
                ),
            ]
        );
    }

    #[tokio::test]
    async fn macos_status_requires_numeric_pid_for_running_state() {
        let temp = TempDir::new().expect("tempdir");
        let runner = Arc::new(RecordingRunner::new(&format!("-\t0\t{LAUNCHD_LABEL}\n")));
        let service = macos_service(&temp, runner);

        let response = service
            .control_service(
                test_caller(),
                RebornServiceLifecycleRequest {
                    action: RebornServiceLifecycleAction::Status,
                },
            )
            .await
            .expect("status response");

        assert_eq!(response.state, RebornServiceLifecycleState::Stopped);
    }

    #[tokio::test]
    async fn control_service_rejects_non_operator_callers_before_commands() {
        let temp = TempDir::new().expect("tempdir");
        let runner = Arc::new(RecordingRunner::new("inactive"));
        let operator_user_id =
            ironclaw_host_api::UserId::new("operator-test").expect("operator user");
        let service = linux_service(&temp, runner.clone()).with_operator_user_id(operator_user_id);

        let error = service
            .control_service(
                test_caller(),
                RebornServiceLifecycleRequest {
                    action: RebornServiceLifecycleAction::Start,
                },
            )
            .await
            .expect_err("non-operator rejected");

        assert_eq!(error.code, RebornServicesErrorCode::Forbidden);
        assert!(runner.calls().is_empty());
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
