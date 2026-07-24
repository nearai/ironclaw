// arch-exempt: large_file, operator service backend awaits composition helper extraction, plan #4471
//! Local OS service lifecycle backend for the Reborn operator facade.
//!
//! This is the concrete implementation behind
//! `POST /api/webchat/v2/operator/service`. It intentionally accepts only the
//! fixed `ironclaw-reborn` unit/label and fixed command argv shapes; browser
//! input can select an action, not a command line.

use std::borrow::Cow;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[cfg(unix)]
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
#[cfg(unix)]
use std::os::unix::process::CommandExt;

use async_trait::async_trait;
use ironclaw_host_api::{
    ProductSurfaceCaller, ProductSurfaceError, ProductSurfaceErrorCode, ProductSurfaceErrorKind,
    TenantId, UserId,
};
use ironclaw_product::{
    OperatorServiceLifecycleService, RebornServiceLifecycleAction, RebornServiceLifecycleRequest,
    RebornServiceLifecycleResponse, RebornServiceLifecycleState,
};

const LAUNCHD_LABEL: &str = "com.ironclaw.reborn";
const SYSTEMD_UNIT: &str = "ironclaw-reborn.service";
const WEBUI_TOKEN_ENV: &str = "IRONCLAW_REBORN_WEBUI_TOKEN";
const WEBUI_USER_ID_ENV: &str = "IRONCLAW_REBORN_WEBUI_USER_ID";
const SERVICE_COMMAND_OUTPUT_LIMIT_BYTES: usize = 16 * 1024;
const SERVICE_COMMAND_TIMEOUT: Duration = Duration::from_secs(30);

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
    stdout: Vec<u8>,
}

impl CommandOutput {
    fn stdout_text(&self) -> Cow<'_, str> {
        String::from_utf8_lossy(&self.stdout)
    }
}

#[derive(Debug, thiserror::Error)]
enum ServiceCommandError {
    #[error("service manager command could not be started: {0}")]
    Start(std::io::Error),
    #[error("service manager command status could not be read: {0}")]
    Status(std::io::Error),
    #[error("service manager command output could not be read: {0}")]
    Output(std::io::Error),
    #[error("service manager command timed out")]
    Timeout,
}

trait ServiceCommandRunner: Send + Sync {
    fn run(&self, program: &str, args: &[&str]) -> Result<CommandOutput, ServiceCommandError>;
}

#[derive(Debug, Default)]
struct SystemCommandRunner;

impl ServiceCommandRunner for SystemCommandRunner {
    fn run(&self, program: &str, args: &[&str]) -> Result<CommandOutput, ServiceCommandError> {
        let mut stdout_file = tempfile::tempfile().map_err(ServiceCommandError::Output)?;
        let child_stdout = stdout_file
            .try_clone()
            .map_err(ServiceCommandError::Output)?;
        let mut command = Command::new(program);
        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::from(child_stdout))
            .stderr(Stdio::null());
        #[cfg(unix)]
        command.process_group(0);
        let mut child = command.spawn().map_err(ServiceCommandError::Start)?;
        let started = Instant::now();
        loop {
            match child.try_wait().map_err(ServiceCommandError::Status)? {
                Some(status) => {
                    stdout_file
                        .seek(SeekFrom::Start(0))
                        .map_err(ServiceCommandError::Output)?;
                    let stdout = read_command_stdout(&mut stdout_file)?;
                    return Ok(CommandOutput {
                        success: status.success(),
                        stdout,
                    });
                }
                None if started.elapsed() >= SERVICE_COMMAND_TIMEOUT => {
                    terminate_service_command(&mut child);
                    return Err(ServiceCommandError::Timeout);
                }
                None => std::thread::sleep(Duration::from_millis(25)),
            }
        }
    }
}

fn terminate_service_command(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
        let pgid = process_group_id(child).unwrap_or_else(|error| {
            tracing::debug!(%error, "service manager command process group lookup failed");
            child.id() as libc::pid_t
        });
        if let Err(error) = terminate_process_group_with_kill_command(pgid) {
            tracing::debug!(%error, "service manager command process group kill failed");
            if let Err(kill_error) = child.kill() {
                tracing::debug!(error = ?kill_error, "best-effort service child kill failed");
            }
        }
    }

    #[cfg(not(unix))]
    {
        if let Err(kill_error) = child.kill() {
            tracing::debug!(error = ?kill_error, "best-effort service child kill failed");
        }
    }

    if let Err(error) = child.wait() {
        tracing::debug!(?error, "best-effort service child wait failed");
    }
}

#[cfg(unix)]
fn process_group_id(child: &std::process::Child) -> std::io::Result<libc::pid_t> {
    let pid = nix::unistd::Pid::from_raw(child.id() as libc::pid_t);
    nix::unistd::getpgid(Some(pid))
        .map(|pgid| pgid.as_raw())
        .map_err(std::io::Error::other)
}

#[cfg(unix)]
fn terminate_process_group_with_kill_command(pgid: libc::pid_t) -> std::io::Result<()> {
    let mut kill = Command::new("/bin/kill")
        .arg("-KILL")
        .arg(format!("-{pgid}"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let started = Instant::now();
    loop {
        if let Some(status) = kill.try_wait()? {
            return if status.success() {
                Ok(())
            } else {
                Err(std::io::Error::other(format!(
                    "/bin/kill exited with {status}"
                )))
            };
        }
        if started.elapsed() >= Duration::from_secs(1) {
            if let Err(error) = kill.kill() {
                tracing::debug!(?error, "best-effort kill-command kill failed");
            }
            if let Err(error) = kill.wait() {
                tracing::debug!(?error, "best-effort kill-command wait failed");
            }
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "/bin/kill did not finish within timeout",
            ));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn write_service_file(path: &Path, contents: &str) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)?;
        file.write_all(contents.as_bytes())?;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        file.sync_all()?;
        Ok(())
    }

    #[cfg(not(unix))]
    {
        std::fs::write(path, contents)
    }
}

fn read_command_stdout(
    stdout_file: &mut (impl Read + Seek),
) -> Result<Vec<u8>, ServiceCommandError> {
    let mut stdout = Vec::new();
    stdout_file
        .take((SERVICE_COMMAND_OUTPUT_LIMIT_BYTES + 1) as u64)
        .read_to_end(&mut stdout)
        .map_err(ServiceCommandError::Output)?;
    if stdout.len() > SERVICE_COMMAND_OUTPUT_LIMIT_BYTES {
        return Err(ServiceCommandError::Output(std::io::Error::other(
            "service manager output exceeded limit",
        )));
    }
    Ok(stdout)
}

/// Platform-backed local service lifecycle manager.
#[derive(Clone)]
pub(crate) struct OperatorServiceLifecycle {
    platform: ServicePlatform,
    home_dir: Option<PathBuf>,
    executable: Result<PathBuf, String>,
    webui_boot_env: Result<WebuiBootEnv, String>,
    operator_identity: Option<OperatorIdentity>,
    runner: Arc<dyn ServiceCommandRunner>,
    operation_permits: Arc<tokio::sync::Semaphore>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OperatorIdentity {
    tenant_id: TenantId,
    user_id: UserId,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WebuiBootEnv {
    token_env_name: String,
    token: String,
    user_id_env_name: String,
    user_id: UserId,
}

impl std::fmt::Debug for OperatorServiceLifecycle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OperatorServiceLifecycle")
            .field("platform", &self.platform)
            .field("home_dir", &self.home_dir.is_some())
            .field("executable", &"<redacted>")
            .field("webui_boot_env", &self.webui_boot_env.is_ok())
            .field("operator_identity", &self.operator_identity.is_some())
            .finish_non_exhaustive()
    }
}

impl OperatorServiceLifecycle {
    pub(crate) fn new() -> Self {
        Self {
            platform: ServicePlatform::current(),
            home_dir: std::env::var_os("HOME").map(PathBuf::from),
            executable: std::env::current_exe()
                .map_err(|error| format!("current executable path could not be resolved: {error}")),
            webui_boot_env: webui_boot_env_from_env(),
            operator_identity: None,
            runner: Arc::new(SystemCommandRunner),
            operation_permits: Arc::new(tokio::sync::Semaphore::new(1)),
        }
    }

    pub(crate) fn new_for_operator_with_boot_config(
        operator_tenant_id: TenantId,
        operator_user_id: UserId,
        boot: Option<&ironclaw_reborn_config::RebornBootConfig>,
    ) -> Self {
        Self {
            platform: ServicePlatform::current(),
            home_dir: std::env::var_os("HOME").map(PathBuf::from),
            executable: std::env::current_exe()
                .map_err(|error| format!("current executable path could not be resolved: {error}")),
            webui_boot_env: webui_boot_env_from_env_for_boot_config(boot, &operator_user_id),
            operator_identity: Some(OperatorIdentity {
                tenant_id: operator_tenant_id,
                user_id: operator_user_id,
            }),
            runner: Arc::new(SystemCommandRunner),
            operation_permits: Arc::new(tokio::sync::Semaphore::new(1)),
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
            webui_boot_env: Ok(WebuiBootEnv {
                token_env_name: WEBUI_TOKEN_ENV.to_string(),
                token: "test-webui-token".to_string(),
                user_id_env_name: WEBUI_USER_ID_ENV.to_string(),
                user_id: test_operator_identity().user_id,
            }),
            operator_identity: Some(test_operator_identity()),
            runner,
            operation_permits: Arc::new(tokio::sync::Semaphore::new(1)),
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
            webui_boot_env: Ok(WebuiBootEnv {
                token_env_name: WEBUI_TOKEN_ENV.to_string(),
                token: "test-webui-token".to_string(),
                user_id_env_name: WEBUI_USER_ID_ENV.to_string(),
                user_id: test_operator_identity().user_id,
            }),
            operator_identity: Some(test_operator_identity()),
            runner,
            operation_permits: Arc::new(tokio::sync::Semaphore::new(1)),
        }
    }

    #[cfg(test)]
    fn with_operator_identity(mut self, tenant_id: TenantId, user_id: UserId) -> Self {
        self.operator_identity = Some(OperatorIdentity { tenant_id, user_id });
        self
    }

    #[cfg(test)]
    fn with_webui_boot_env_error(mut self, error: &str) -> Self {
        self.webui_boot_env = Err(error.to_string());
        self
    }

    #[cfg(test)]
    fn with_webui_boot_env(
        mut self,
        token_env_name: &str,
        token: &str,
        user_id_env_name: &str,
        user_id: &str,
    ) -> Self {
        self.webui_boot_env = Ok(WebuiBootEnv {
            token_env_name: token_env_name.to_string(),
            token: token.to_string(),
            user_id_env_name: user_id_env_name.to_string(),
            user_id: UserId::new(user_id).expect("test webui user id"),
        });
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

    fn webui_boot_env_for_action(
        &self,
        action: RebornServiceLifecycleAction,
    ) -> Result<&WebuiBootEnv, RebornServiceLifecycleResponse> {
        self.webui_boot_env
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
                Ok(unit) => write_service_file(&path, &unit),
                Err(response) => return response,
            },
            ServicePlatform::Macos => match self.launchd_plist(action) {
                Ok(plist) => write_service_file(&path, &plist),
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
            // Best-effort post-install reload, operator can manually retry.
            if let Err(error) = self.runner.run("systemctl", &["--user", "daemon-reload"]) {
                tracing::debug!(?error, "best-effort post-install daemon-reload failed");
            }
            // Best-effort post-install enable, unit has already been written.
            if let Err(error) = self
                .runner
                .run("systemctl", &["--user", "enable", SYSTEMD_UNIT])
            {
                tracing::debug!(?error, "best-effort post-install unit enable failed");
            }
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
                // Best-effort reload before start, failure does not block start attempt.
                if let Err(error) = self.runner.run("systemctl", &["--user", "daemon-reload"]) {
                    tracing::debug!(?error, "best-effort pre-start daemon-reload failed");
                }
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
                // Best-effort load; launchctl reports failure when the agent is already loaded.
                if let Err(error) = self.runner.run("launchctl", &["load", "-w", &path]) {
                    tracing::debug!(?error, "best-effort launchctl load failed");
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
                if let Err(response) =
                    self.require_command_success(action, "launchctl", &["stop", LAUNCHD_LABEL])
                {
                    return response;
                }
                if let Err(response) =
                    self.require_command_success(action, "launchctl", &["unload", "-w", &path])
                {
                    return response;
                }
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
                    Ok(output) if output.success && output.stdout_text().trim() == "active" => {
                        Self::status_response(
                            RebornServiceLifecycleState::Running,
                            "local Reborn service is running",
                        )
                    }
                    Ok(output)
                        if matches!(output.stdout_text().trim(), "inactive" | "deactivating") =>
                    {
                        Self::status_response(
                            RebornServiceLifecycleState::Stopped,
                            "local Reborn service is stopped",
                        )
                    }
                    Ok(output) if output.stdout_text().trim() == "failed" => Self::status_response(
                        RebornServiceLifecycleState::Failed,
                        "local Reborn service is failed",
                    ),
                    Ok(_) => Self::status_response(
                        RebornServiceLifecycleState::Unknown,
                        "local Reborn service state is unknown",
                    ),
                    Err(error) => Self::status_query_failed_response(action, error),
                }
            }
            ServicePlatform::Macos => {
                let output = self.runner.run("launchctl", &["list", LAUNCHD_LABEL]);
                match output {
                    Ok(output) => match launchd_status(output.stdout_text().as_ref()) {
                        LaunchdStatus::Running => Self::status_response(
                            RebornServiceLifecycleState::Running,
                            "local Reborn service is running",
                        ),
                        LaunchdStatus::Stopped => Self::status_response(
                            RebornServiceLifecycleState::Stopped,
                            "local Reborn service is stopped",
                        ),
                        LaunchdStatus::Failed => Self::status_response(
                            RebornServiceLifecycleState::Failed,
                            "local Reborn service is failed",
                        ),
                    },
                    Err(error) => Self::status_query_failed_response(action, error),
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
        match self.require_command_success(action, program, args) {
            Ok(()) => RebornServiceLifecycleResponse {
                action,
                state: success_state,
                message: success_message.to_string(),
                remediation: None,
            },
            Err(response) => response,
        }
    }

    fn require_command_success(
        &self,
        action: RebornServiceLifecycleAction,
        program: &str,
        args: &[&str],
    ) -> Result<(), RebornServiceLifecycleResponse> {
        match self.runner.run(program, args) {
            Ok(output) if output.success => Ok(()),
            Ok(_) => Err(Self::failed_response(
                action,
                "local service manager command failed",
            )),
            Err(ServiceCommandError::Timeout) => Err(Self::failed_response(
                action,
                "local service manager command timed out",
            )),
            Err(error) => {
                tracing::debug!(%error, "service manager command failed");
                Err(Self::failed_response(
                    action,
                    "local service manager command failed",
                ))
            }
        }
    }

    fn status_query_failed_response(
        action: RebornServiceLifecycleAction,
        error: ServiceCommandError,
    ) -> RebornServiceLifecycleResponse {
        match error {
            ServiceCommandError::Timeout => {
                Self::failed_response(action, "local service manager command timed out")
            }
            error => {
                tracing::debug!(%error, "service manager status query failed");
                Self::failed_response(action, "local service manager status could not be queried")
            }
        }
    }

    fn systemd_unit(
        &self,
        action: RebornServiceLifecycleAction,
    ) -> Result<String, RebornServiceLifecycleResponse> {
        let executable = self.executable_path_for_action(action)?;
        let boot_env = self.webui_boot_env_for_action(action)?;
        let exe = systemd_exec_escape(executable.to_string_lossy().as_ref())
            .map_err(|message| Self::failed_response(action, message))?;
        let token_env_name = systemd_environment_escape(&boot_env.token_env_name)
            .map_err(|message| Self::failed_response(action, message))?;
        let token = systemd_environment_escape(&boot_env.token)
            .map_err(|message| Self::failed_response(action, message))?;
        let user_id_env_name = systemd_environment_escape(&boot_env.user_id_env_name)
            .map_err(|message| Self::failed_response(action, message))?;
        let user_id = systemd_environment_escape(boot_env.user_id.as_str())
            .map_err(|message| Self::failed_response(action, message))?;
        Ok(format!(
            "[Unit]\n\
             Description=IronClaw Reborn WebUI service\n\
             After=network.target\n\
             \n\
             [Service]\n\
             Type=simple\n\
             Environment=\"{token_env_name}={token}\"\n\
             Environment=\"{user_id_env_name}={user_id}\"\n\
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
        let boot_env = self.webui_boot_env_for_action(action)?;
        let exe = xml_escape(executable.to_string_lossy().as_ref());
        let token_env_name = xml_escape(&boot_env.token_env_name);
        let token = xml_escape(&boot_env.token);
        let user_id_env_name = xml_escape(&boot_env.user_id_env_name);
        let user_id = xml_escape(boot_env.user_id.as_str());
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
  <key>EnvironmentVariables</key>
  <dict>
    <key>{token_env_name}</key>
    <string>{token}</string>
    <key>{user_id_env_name}</key>
    <string>{user_id}</string>
  </dict>
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
        caller: &ProductSurfaceCaller,
    ) -> Result<(), ProductSurfaceError> {
        if self.operator_identity.as_ref().is_some_and(|operator| {
            caller.tenant_id == operator.tenant_id && caller.user_id == operator.user_id
        }) {
            return Ok(());
        }
        Err(ProductSurfaceError {
            code: ProductSurfaceErrorCode::Forbidden,
            kind: ProductSurfaceErrorKind::ParticipantDenied,
            status_code: 403,
            retryable: false,
            field: None,
            validation_code: None,
        })
    }
}

impl Default for OperatorServiceLifecycle {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl OperatorServiceLifecycleService for OperatorServiceLifecycle {
    async fn control_service(
        &self,
        caller: ProductSurfaceCaller,
        request: RebornServiceLifecycleRequest,
    ) -> Result<RebornServiceLifecycleResponse, ProductSurfaceError> {
        self.ensure_authorized_operator(&caller)?;
        let permit = self
            .operation_permits
            .clone()
            .acquire_owned()
            .await
            .map_err(|error| ProductSurfaceError::internal_from(error.to_string()))?;
        let service = self.clone();
        let action = request.action;
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            match action {
                RebornServiceLifecycleAction::Install => service.install(),
                RebornServiceLifecycleAction::Start => service.start(),
                RebornServiceLifecycleAction::Stop => service.stop(),
                RebornServiceLifecycleAction::Status => service.status(),
            }
        })
        .await
        .map_err(|error| {
            tracing::debug!(%error, "service lifecycle task failed");
            ProductSurfaceError::internal_from("service lifecycle task failed")
        })
    }
}

fn systemd_exec_escape(value: &str) -> Result<String, &'static str> {
    reject_systemd_line_breaks(value)?;
    Ok(value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('%', "%%")
        .replace('$', "$$"))
}

fn systemd_environment_escape(value: &str) -> Result<String, &'static str> {
    reject_systemd_line_breaks(value)?;
    Ok(value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('%', "%%"))
}

fn reject_systemd_line_breaks(value: &str) -> Result<(), &'static str> {
    if value.chars().any(|ch| matches!(ch, '\r' | '\n')) {
        return Err("local service unit values must not contain line breaks");
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LaunchdStatus {
    Running,
    Stopped,
    Failed,
}

fn launchd_status(stdout: &str) -> LaunchdStatus {
    stdout
        .lines()
        .find_map(|line| {
            launchd_status_from_line(line).filter(|(_, label)| label == &LAUNCHD_LABEL)
        })
        .map_or(LaunchdStatus::Stopped, |(status, _)| status)
}

fn launchd_status_from_line(line: &str) -> Option<(LaunchdStatus, &str)> {
    let mut columns = line.split_whitespace();
    let pid = columns.next()?;
    let exit_status = columns.next()?;
    let label = columns.next()?;
    let status = if pid.parse::<i32>().is_ok() {
        LaunchdStatus::Running
    } else if exit_status.parse::<i32>().is_ok_and(|status| status != 0) {
        LaunchdStatus::Failed
    } else {
        LaunchdStatus::Stopped
    };
    Some((status, label))
}

fn xml_escape(raw: &str) -> String {
    raw.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn webui_boot_env_from_env() -> Result<WebuiBootEnv, String> {
    webui_boot_env_from_env_names(WebuiEnvNames::default(), None)
}

fn webui_boot_env_from_env_for_boot_config(
    boot: Option<&ironclaw_reborn_config::RebornBootConfig>,
    operator_user_id: &UserId,
) -> Result<WebuiBootEnv, String> {
    webui_env_names_for_boot_config(boot)
        .and_then(|names| webui_boot_env_from_env_names(names, Some(operator_user_id)))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WebuiEnvNames {
    token: String,
    user_id: String,
}

impl Default for WebuiEnvNames {
    fn default() -> Self {
        Self {
            token: WEBUI_TOKEN_ENV.to_string(),
            user_id: WEBUI_USER_ID_ENV.to_string(),
        }
    }
}

fn webui_env_names_for_boot_config(
    boot: Option<&ironclaw_reborn_config::RebornBootConfig>,
) -> Result<WebuiEnvNames, String> {
    let Some(boot) = boot else {
        return Ok(WebuiEnvNames::default());
    };
    let config_path = boot.home().config_file_path();
    let config = ironclaw_reborn_config::RebornConfigFile::load(&config_path).map_err(|error| {
        tracing::debug!(%error, "Reborn config file could not be loaded for service lifecycle");
        "Reborn config file could not be loaded".to_string()
    })?;
    let Some(webui) = config.and_then(|config| config.webui) else {
        return Ok(WebuiEnvNames::default());
    };
    Ok(WebuiEnvNames {
        token: webui
            .env_token_var
            .unwrap_or_else(|| WEBUI_TOKEN_ENV.to_string()),
        user_id: webui
            .env_user_id_var
            .unwrap_or_else(|| WEBUI_USER_ID_ENV.to_string()),
    })
}

fn webui_boot_env_from_env_names(
    names: WebuiEnvNames,
    expected_user_id: Option<&UserId>,
) -> Result<WebuiBootEnv, String> {
    let token = required_env(&names.token)?;
    let user_id = webui_user_id_from_env_value(
        &names.user_id,
        required_env(&names.user_id)?,
        expected_user_id,
    )?;
    Ok(WebuiBootEnv {
        token_env_name: names.token,
        token,
        user_id_env_name: names.user_id,
        user_id,
    })
}

fn webui_user_id_from_env_value(
    env_name: &str,
    raw_user_id: String,
    expected_user_id: Option<&UserId>,
) -> Result<UserId, String> {
    let user_id = UserId::new(raw_user_id).map_err(|error| {
        tracing::debug!(%error, "WebUI operator user id env value was rejected");
        format!("{env_name} must match the authorized operator user")
    })?;
    if expected_user_id.is_some_and(|expected| expected != &user_id) {
        return Err(format!(
            "{env_name} must match the authorized operator user"
        ));
    }
    Ok(user_id)
}

fn required_env(name: &str) -> Result<String, String> {
    match std::env::var(name) {
        Ok(value) if !value.trim().is_empty() => Ok(value),
        Ok(_) => Err(format!(
            "{name} is empty and cannot be persisted into the local service"
        )),
        Err(std::env::VarError::NotPresent) => Err(format!(
            "{name} must be set before installing the local service"
        )),
        Err(std::env::VarError::NotUnicode(_)) => Err(format!("{name} must be valid UTF-8")),
    }
}

#[cfg(test)]
fn test_operator_identity() -> OperatorIdentity {
    OperatorIdentity {
        tenant_id: ironclaw_host_api::TenantId::new("tenant-test").expect("test operator tenant"),
        user_id: ironclaw_host_api::UserId::new("user-test").expect("test operator user"),
    }
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
        scoped_status_stdout: Mutex<Option<String>>,
        fail_command: Mutex<Option<(String, Vec<String>)>>,
        timeout_command: Mutex<Option<(String, Vec<String>)>>,
    }

    impl RecordingRunner {
        fn new(status_stdout: &str) -> Self {
            Self {
                calls: Mutex::default(),
                status_stdout: Mutex::new(status_stdout.to_string()),
                scoped_status_stdout: Mutex::new(None),
                fail_command: Mutex::new(None),
                timeout_command: Mutex::new(None),
            }
        }

        fn fail_command(&self, program: &str, args: &[&str]) {
            *self.fail_command.lock().expect("lock") = Some((
                program.to_string(),
                args.iter().map(|arg| (*arg).to_string()).collect(),
            ));
        }

        fn timeout_command(&self, program: &str, args: &[&str]) {
            *self.timeout_command.lock().expect("lock") = Some((
                program.to_string(),
                args.iter().map(|arg| (*arg).to_string()).collect(),
            ));
        }

        fn calls(&self) -> Vec<(String, Vec<String>)> {
            self.calls.lock().expect("lock").clone()
        }

        fn set_scoped_status_stdout(&self, stdout: &str) {
            *self.scoped_status_stdout.lock().expect("lock") = Some(stdout.to_string());
        }
    }

    impl ServiceCommandRunner for RecordingRunner {
        fn run(&self, program: &str, args: &[&str]) -> Result<CommandOutput, ServiceCommandError> {
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
                    stdout: Vec::new(),
                });
            }
            if self
                .timeout_command
                .lock()
                .expect("lock")
                .as_ref()
                .is_some_and(|timeout_command| timeout_command == &command)
            {
                return Err(ServiceCommandError::Timeout);
            }
            let reports_status = (program == "systemctl"
                && args.ends_with(&["is-active", SYSTEMD_UNIT]))
                || (program == "launchctl" && args == ["list"])
                || (program == "launchctl" && args == ["list", LAUNCHD_LABEL]);
            let stdout = if reports_status {
                if program == "launchctl" && args == ["list", LAUNCHD_LABEL] {
                    self.scoped_status_stdout
                        .lock()
                        .expect("lock")
                        .as_ref()
                        .map_or_else(
                            || self.status_stdout.lock().expect("lock").as_bytes().to_vec(),
                            |stdout| stdout.as_bytes().to_vec(),
                        )
                } else {
                    self.status_stdout.lock().expect("lock").as_bytes().to_vec()
                }
            } else {
                Vec::new()
            };
            Ok(CommandOutput {
                success: true,
                stdout,
            })
        }
    }

    #[test]
    fn service_manager_stdout_is_bounded() {
        let mut stdout_file = tempfile::tempfile().expect("tempfile");
        let oversized = vec![b'x'; SERVICE_COMMAND_OUTPUT_LIMIT_BYTES + 1];
        std::io::Write::write_all(&mut stdout_file, &oversized).expect("write stdout");
        std::io::Seek::seek(&mut stdout_file, SeekFrom::Start(0)).expect("rewind");

        let error = read_command_stdout(&mut stdout_file).expect_err("oversized output");

        match error {
            ServiceCommandError::Output(source) => {
                assert!(source.to_string().contains("exceeded limit"));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    fn macos_service(temp: &TempDir, runner: Arc<RecordingRunner>) -> OperatorServiceLifecycle {
        OperatorServiceLifecycle::for_test(
            ServicePlatform::Macos,
            Some(temp.path().to_path_buf()),
            PathBuf::from("/usr/local/bin/ironclaw-reborn"),
            runner,
        )
    }

    fn linux_service(temp: &TempDir, runner: Arc<RecordingRunner>) -> OperatorServiceLifecycle {
        OperatorServiceLifecycle::for_test(
            ServicePlatform::Linux,
            Some(temp.path().to_path_buf()),
            PathBuf::from("/usr/local/bin/ironclaw-reborn"),
            runner,
        )
    }

    #[cfg(unix)]
    fn assert_service_file_owner_only(path: &Path) {
        let mode = std::fs::metadata(path)
            .expect("service file metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
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
        let unit = std::fs::read_to_string(&unit_path).expect("unit file");
        #[cfg(unix)]
        assert_service_file_owner_only(&unit_path);
        assert!(unit.contains("ExecStart=\"/usr/local/bin/ironclaw-reborn\" serve"));
        assert!(unit.contains("Environment=\"IRONCLAW_REBORN_WEBUI_TOKEN=test-webui-token\""));
        assert!(unit.contains("Environment=\"IRONCLAW_REBORN_WEBUI_USER_ID=user-test\""));
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
    async fn linux_install_uses_custom_webui_env_names() {
        let temp = TempDir::new().expect("tempdir");
        let runner = Arc::new(RecordingRunner::new("inactive"));
        let service = linux_service(&temp, runner).with_webui_boot_env(
            "CUSTOM_WEBUI_TOKEN",
            "test-webui-token",
            "CUSTOM_WEBUI_USER_ID",
            "user-test",
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
        assert!(unit.contains("Environment=\"CUSTOM_WEBUI_TOKEN=test-webui-token\""));
        assert!(unit.contains("Environment=\"CUSTOM_WEBUI_USER_ID=user-test\""));
        assert!(!unit.contains(WEBUI_TOKEN_ENV));
        assert!(!unit.contains(WEBUI_USER_ID_ENV));
    }

    #[test]
    fn webui_env_names_follow_reborn_config_file() {
        let temp = TempDir::new().expect("tempdir");
        std::fs::write(
            temp.path().join("config.toml"),
            r#"
[webui]
env_token_var = "CUSTOM_WEBUI_TOKEN"
env_user_id_var = "CUSTOM_WEBUI_USER_ID"
"#,
        )
        .expect("config file");
        let boot = ironclaw_reborn_config::RebornBootConfig::resolve_from_env_parts(
            Some(temp.path().as_os_str().to_os_string()),
            None,
            None,
            None,
        )
        .expect("boot config");

        let names = webui_env_names_for_boot_config(Some(&boot)).expect("env names");

        assert_eq!(names.token, "CUSTOM_WEBUI_TOKEN");
        assert_eq!(names.user_id, "CUSTOM_WEBUI_USER_ID");
    }

    #[test]
    fn webui_env_names_sanitize_reborn_config_load_errors() {
        let temp = TempDir::new().expect("tempdir");
        std::fs::create_dir(temp.path().join("config.toml")).expect("config dir");
        let boot = ironclaw_reborn_config::RebornBootConfig::resolve_from_env_parts(
            Some(temp.path().as_os_str().to_os_string()),
            None,
            None,
            None,
        )
        .expect("boot config");

        let error = webui_env_names_for_boot_config(Some(&boot)).expect_err("load error");

        assert_eq!(error, "Reborn config file could not be loaded");
        assert!(!error.contains(temp.path().to_string_lossy().as_ref()));
    }

    #[test]
    fn webui_user_id_from_env_value_rejects_operator_mismatch() {
        let expected = UserId::new("user-test").expect("expected user id");

        let error = webui_user_id_from_env_value(
            "CUSTOM_WEBUI_USER_ID",
            "other-user".to_string(),
            Some(&expected),
        )
        .expect_err("user mismatch");

        assert_eq!(
            error,
            "CUSTOM_WEBUI_USER_ID must match the authorized operator user"
        );
    }

    #[tokio::test]
    async fn linux_install_escapes_systemd_special_characters_in_executable_path() {
        let temp = TempDir::new().expect("tempdir");
        let runner = Arc::new(RecordingRunner::new("inactive"));
        let service = OperatorServiceLifecycle::for_test(
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
    async fn linux_install_preserves_dollar_signs_in_systemd_environment_values() {
        let temp = TempDir::new().expect("tempdir");
        let runner = Arc::new(RecordingRunner::new("inactive"));
        let service = linux_service(&temp, runner).with_webui_boot_env(
            WEBUI_TOKEN_ENV,
            "test$webui%token",
            WEBUI_USER_ID_ENV,
            "user-test",
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
        assert!(unit.contains("Environment=\"IRONCLAW_REBORN_WEBUI_TOKEN=test$webui%%token\""));
        assert!(!unit.contains("test$$webui"));
    }

    #[tokio::test]
    async fn linux_install_rejects_systemd_environment_line_breaks_before_writing_unit() {
        let temp = TempDir::new().expect("tempdir");
        let runner = Arc::new(RecordingRunner::new("inactive"));
        let service = linux_service(&temp, runner).with_webui_boot_env(
            WEBUI_TOKEN_ENV,
            "test-webui-token\nEnvironment=\"INJECTED=value\"",
            WEBUI_USER_ID_ENV,
            "user-test",
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

        assert_eq!(response.state, RebornServiceLifecycleState::Failed);
        assert_eq!(
            response.message,
            "local service unit values must not contain line breaks"
        );
        let unit_path = temp.path().join(".config/systemd/user").join(SYSTEMD_UNIT);
        assert!(!unit_path.exists());
    }

    #[tokio::test]
    async fn linux_install_fails_when_webui_boot_env_is_unavailable() {
        let temp = TempDir::new().expect("tempdir");
        let runner = Arc::new(RecordingRunner::new("inactive"));
        let service = linux_service(&temp, runner.clone())
            .with_webui_boot_env_error("IRONCLAW_REBORN_WEBUI_TOKEN must be set");

        let response = service
            .control_service(
                test_caller(),
                RebornServiceLifecycleRequest {
                    action: RebornServiceLifecycleAction::Install,
                },
            )
            .await
            .expect("install response");

        assert_eq!(response.state, RebornServiceLifecycleState::Failed);
        assert!(response.message.contains("IRONCLAW_REBORN_WEBUI_TOKEN"));
        assert!(runner.calls().is_empty());
        assert!(
            !temp
                .path()
                .join(".config/systemd/user")
                .join(SYSTEMD_UNIT)
                .exists()
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn install_rejects_symlinked_service_file_path() {
        let temp = TempDir::new().expect("tempdir");
        let runner = Arc::new(RecordingRunner::new("inactive"));
        let service = linux_service(&temp, runner.clone());
        let unit_dir = temp.path().join(".config/systemd/user");
        std::fs::create_dir_all(&unit_dir).expect("unit dir");
        let target = temp.path().join("target.service");
        std::os::unix::fs::symlink(&target, unit_dir.join(SYSTEMD_UNIT)).expect("unit symlink");

        let response = service
            .control_service(
                test_caller(),
                RebornServiceLifecycleRequest {
                    action: RebornServiceLifecycleAction::Install,
                },
            )
            .await
            .expect("install response");

        assert_eq!(response.state, RebornServiceLifecycleState::Failed);
        assert!(
            response
                .message
                .contains("local service unit could not be written")
        );
        assert!(!target.exists());
        assert!(runner.calls().is_empty());
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
    async fn linux_start_timeout_returns_timeout_failed_state() {
        let temp = TempDir::new().expect("tempdir");
        let runner = Arc::new(RecordingRunner::new("inactive"));
        runner.timeout_command("systemctl", &["--user", "start", SYSTEMD_UNIT]);
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
        assert_eq!(response.message, "local service manager command timed out");
    }

    #[tokio::test]
    async fn install_without_home_reports_failed_resolution() {
        let service = OperatorServiceLifecycle::for_test(
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
        let service = OperatorServiceLifecycle::for_test_with_executable_error(
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
    async fn macos_install_persists_webui_boot_environment() {
        let temp = TempDir::new().expect("tempdir");
        let runner = Arc::new(RecordingRunner::new(""));
        let service = macos_service(&temp, runner);

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
        let plist_path = temp
            .path()
            .join("Library")
            .join("LaunchAgents")
            .join(format!("{LAUNCHD_LABEL}.plist"));
        let plist = std::fs::read_to_string(&plist_path).expect("plist file");
        #[cfg(unix)]
        assert_service_file_owner_only(&plist_path);
        assert!(plist.contains("<key>IRONCLAW_REBORN_WEBUI_TOKEN</key>"));
        assert!(plist.contains("<string>test-webui-token</string>"));
        assert!(plist.contains("<key>IRONCLAW_REBORN_WEBUI_USER_ID</key>"));
        assert!(plist.contains("<string>user-test</string>"));
    }

    #[tokio::test]
    async fn macos_install_uses_custom_webui_env_names() {
        let temp = TempDir::new().expect("tempdir");
        let runner = Arc::new(RecordingRunner::new(""));
        let service = macos_service(&temp, runner).with_webui_boot_env(
            "CUSTOM_WEBUI_TOKEN",
            "test-webui-token",
            "CUSTOM_WEBUI_USER_ID",
            "user-test",
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
        let plist_path = temp
            .path()
            .join("Library")
            .join("LaunchAgents")
            .join(format!("{LAUNCHD_LABEL}.plist"));
        let plist = std::fs::read_to_string(plist_path).expect("plist file");
        assert!(plist.contains("<key>CUSTOM_WEBUI_TOKEN</key>"));
        assert!(plist.contains("<key>CUSTOM_WEBUI_USER_ID</key>"));
        assert!(!plist.contains(WEBUI_TOKEN_ENV));
        assert!(!plist.contains(WEBUI_USER_ID_ENV));
    }

    #[tokio::test]
    async fn macos_stop_failure_returns_failed_state() {
        let temp = TempDir::new().expect("tempdir");
        let runner = Arc::new(RecordingRunner::new(""));
        runner.fail_command("launchctl", &["stop", LAUNCHD_LABEL]);
        let service = macos_service(&temp, runner);

        let response = service
            .control_service(
                test_caller(),
                RebornServiceLifecycleRequest {
                    action: RebornServiceLifecycleAction::Stop,
                },
            )
            .await
            .expect("stop response");

        assert_eq!(response.state, RebornServiceLifecycleState::Failed);
    }

    #[tokio::test]
    async fn macos_stop_timeout_returns_timeout_failed_state() {
        let temp = TempDir::new().expect("tempdir");
        let runner = Arc::new(RecordingRunner::new(""));
        runner.timeout_command("launchctl", &["stop", LAUNCHD_LABEL]);
        let service = macos_service(&temp, runner);

        let response = service
            .control_service(
                test_caller(),
                RebornServiceLifecycleRequest {
                    action: RebornServiceLifecycleAction::Stop,
                },
            )
            .await
            .expect("stop response");

        assert_eq!(response.state, RebornServiceLifecycleState::Failed);
        assert_eq!(response.message, "local service manager command timed out");
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
    async fn macos_status_maps_nonzero_launchd_exit_to_failed() {
        let temp = TempDir::new().expect("tempdir");
        let runner = Arc::new(RecordingRunner::new(&format!("-\t78\t{LAUNCHD_LABEL}\n")));
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

        assert_eq!(response.state, RebornServiceLifecycleState::Failed);
    }

    #[tokio::test]
    async fn macos_status_timeout_returns_timeout_failed_state() {
        let temp = TempDir::new().expect("tempdir");
        let runner = Arc::new(RecordingRunner::new(""));
        runner.timeout_command("launchctl", &["list", LAUNCHD_LABEL]);
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

        assert_eq!(response.state, RebornServiceLifecycleState::Failed);
        assert_eq!(response.message, "local service manager command timed out");
    }

    #[tokio::test]
    async fn macos_status_uses_label_scoped_query_when_listing_is_large() {
        let temp = TempDir::new().expect("tempdir");
        let runner = Arc::new(RecordingRunner::new(&format!(
            "{:0>16384}\n123\t0\t{LAUNCHD_LABEL}\n",
            ""
        )));
        runner.set_scoped_status_stdout(&format!("123\t0\t{LAUNCHD_LABEL}\n"));
        let service = macos_service(&temp, runner.clone());

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
        assert_eq!(
            runner.calls(),
            vec![(
                "launchctl".to_string(),
                vec!["list".to_string(), LAUNCHD_LABEL.to_string()]
            )]
        );
    }

    #[tokio::test]
    async fn macos_status_requires_exact_launchd_label_for_running_state() {
        let temp = TempDir::new().expect("tempdir");
        let runner = Arc::new(RecordingRunner::new(&format!(
            "123\t0\t{LAUNCHD_LABEL}-helper\n"
        )));
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
        let service = linux_service(&temp, runner.clone()).with_operator_identity(
            ironclaw_host_api::TenantId::new("tenant-test").expect("operator tenant"),
            operator_user_id,
        );

        let error = service
            .control_service(
                test_caller(),
                RebornServiceLifecycleRequest {
                    action: RebornServiceLifecycleAction::Start,
                },
            )
            .await
            .expect_err("non-operator rejected");

        assert_eq!(error.code, ProductSurfaceErrorCode::Forbidden);
        assert!(runner.calls().is_empty());
    }

    #[tokio::test]
    async fn control_service_rejects_same_user_from_different_tenant_before_commands() {
        let temp = TempDir::new().expect("tempdir");
        let runner = Arc::new(RecordingRunner::new("inactive"));
        let service = linux_service(&temp, runner.clone()).with_operator_identity(
            ironclaw_host_api::TenantId::new("other-tenant").expect("operator tenant"),
            ironclaw_host_api::UserId::new("user-test").expect("operator user"),
        );

        let error = service
            .control_service(
                test_caller(),
                RebornServiceLifecycleRequest {
                    action: RebornServiceLifecycleAction::Start,
                },
            )
            .await
            .expect_err("cross-tenant caller rejected");

        assert_eq!(error.code, ProductSurfaceErrorCode::Forbidden);
        assert!(runner.calls().is_empty());
    }

    #[tokio::test]
    async fn unsupported_platform_reports_unsupported() {
        let service = OperatorServiceLifecycle::for_test(
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

    fn test_caller() -> ProductSurfaceCaller {
        ProductSurfaceCaller::new(
            ironclaw_host_api::TenantId::new("tenant-test").expect("tenant"),
            ironclaw_host_api::UserId::new("user-test").expect("user"),
            Some(ironclaw_host_api::AgentId::new("agent-test").expect("agent")),
            None,
        )
    }
}
