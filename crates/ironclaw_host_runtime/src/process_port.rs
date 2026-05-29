//! Runtime process effect port for command-style first-party capabilities.
//!
//! The port keeps process placement outside individual tools. A capability such
//! as `builtin.shell` describes the command to run; host-runtime composition
//! decides which port implementation receives it. This first slice wires the
//! existing local-host behavior behind an explicit port without changing
//! placement semantics.

use std::{collections::HashMap, path::PathBuf, process::Stdio, time::Duration};

use async_trait::async_trait;
use ironclaw_host_api::{MountView, ResourceScope};
#[cfg(unix)]
use libc::{SIGKILL, kill};
use thiserror::Error;
use tokio::process::Command;

use crate::process_aliases::{
    LocalHostWorkdirAlias, resolve_local_host_workdir, rewrite_local_host_command_aliases,
};
use crate::process_output::{
    CapturedCommandOutput, SavedCommandOutput, StreamCapture, capture_command_output,
    read_stream_capped, truncate_output,
};

const DEFAULT_COMMAND_TIMEOUT: Duration = Duration::from_secs(120);

/// Environment variables safe to forward to local child processes.
const SAFE_ENV_VARS: &[&str] = &[
    "PATH",
    "USER",
    "LOGNAME",
    "SHELL",
    "TERM",
    "COLORTERM",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "LC_MESSAGES",
    "PWD",
    "TMPDIR",
    "TMP",
    "TEMP",
    "XDG_RUNTIME_DIR",
    "XDG_DATA_HOME",
    "XDG_CONFIG_HOME",
    "XDG_CACHE_HOME",
    "CARGO_HOME",
    "RUSTUP_HOME",
    "NODE_PATH",
    "NPM_CONFIG_PREFIX",
    "EDITOR",
    "VISUAL",
    "SystemRoot",
    "SYSTEMROOT",
    "ComSpec",
    "PATHEXT",
    "APPDATA",
    "LOCALAPPDATA",
    "USERPROFILE",
    "ProgramFiles",
    "ProgramFiles(x86)",
    "WINDIR",
];

/// Placement-neutral command request handed to the selected process port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandExecutionRequest {
    pub scope: ResourceScope,
    pub mounts: Option<MountView>,
    pub command: String,
    pub workdir: Option<String>,
    pub timeout_secs: Option<u64>,
    pub extra_env: HashMap<String, String>,
}

/// Process-port command result normalized for capability handlers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandExecutionOutput {
    pub output: String,
    pub saved_output: Option<SavedCommandOutput>,
    pub exit_code: i64,
    pub sandboxed: bool,
    pub duration: Duration,
}

/// Stable redacted process-port failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RuntimeProcessError {
    #[error("command timed out after {0:?}")]
    Timeout(Duration),
    #[error("process execution failed: {0}")]
    ExecutionFailed(String),
}

/// Abstract process effect used by process-backed capabilities.
#[async_trait]
pub trait RuntimeProcessPort: Send + Sync {
    async fn run_command(
        &self,
        request: CommandExecutionRequest,
    ) -> Result<CommandExecutionOutput, RuntimeProcessError>;
}

/// Transport for tenant-sandbox command execution.
///
/// This trait intentionally hides Docker/daemon details from host-runtime tool
/// code. Product adapters can implement it with the V1 sandbox daemon JSON-RPC
/// transport or another tenant-isolated runner.
///
/// Implementations must enforce `CommandExecutionRequest::timeout_secs` and
/// clean up any remote process/container before returning
/// `RuntimeProcessError::Timeout`.
#[async_trait]
pub trait SandboxCommandTransport: Send + Sync {
    async fn run_command(
        &self,
        request: CommandExecutionRequest,
    ) -> Result<CommandExecutionOutput, RuntimeProcessError>;
}

/// Tenant-isolated process port backed by a sandbox command transport.
#[derive(Clone)]
pub struct TenantSandboxProcessPort {
    transport: std::sync::Arc<dyn SandboxCommandTransport>,
}

impl std::fmt::Debug for TenantSandboxProcessPort {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TenantSandboxProcessPort")
            .field("transport", &"<sandbox command transport>")
            .finish()
    }
}

impl TenantSandboxProcessPort {
    pub fn new(transport: std::sync::Arc<dyn SandboxCommandTransport>) -> Self {
        Self { transport }
    }
}

#[async_trait]
impl RuntimeProcessPort for TenantSandboxProcessPort {
    async fn run_command(
        &self,
        request: CommandExecutionRequest,
    ) -> Result<CommandExecutionOutput, RuntimeProcessError> {
        let timeout = request
            .timeout_secs
            .map(Duration::from_secs)
            .unwrap_or(DEFAULT_COMMAND_TIMEOUT);
        let mut request = request;
        request.timeout_secs = Some(timeout.as_secs());
        let mut output = self.transport.run_command(request).await?;
        output.output = truncate_output(&output.output);
        output.sandboxed = true;
        Ok(output)
    }
}

/// Local provider-host command environment handling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum LocalHostProcessEnvMode {
    /// Clear the child environment, forward only `SAFE_ENV_VARS`, and rewrite
    /// `HOME` to the command workdir.
    #[default]
    Scrubbed,
    /// Inherit the host process environment and real `HOME`.
    Inherited,
}

/// Local provider-host command implementation matching the existing shell path.
#[derive(Debug, Clone, Default)]
pub struct LocalHostProcessPort {
    env_mode: LocalHostProcessEnvMode,
    workdir_aliases: Vec<LocalHostWorkdirAlias>,
}

impl LocalHostProcessPort {
    pub fn new() -> Self {
        Self {
            env_mode: LocalHostProcessEnvMode::Scrubbed,
            workdir_aliases: Vec::new(),
        }
    }

    pub fn new_inherited_env() -> Self {
        Self {
            env_mode: LocalHostProcessEnvMode::Inherited,
            workdir_aliases: Vec::new(),
        }
    }

    pub fn with_workdir_alias(
        mut self,
        alias: impl Into<String>,
        host_path: impl Into<PathBuf>,
    ) -> Self {
        match LocalHostWorkdirAlias::try_new(alias, host_path) {
            Ok(alias) => self.workdir_aliases.push(alias),
            Err(reason) => tracing::debug!(
                reason = %reason,
                "ignoring invalid local host process workdir alias"
            ),
        }
        self
    }
}

#[async_trait]
impl RuntimeProcessPort for LocalHostProcessPort {
    async fn run_command(
        &self,
        request: CommandExecutionRequest,
    ) -> Result<CommandExecutionOutput, RuntimeProcessError> {
        let cwd = resolve_local_host_workdir(request.workdir.as_deref(), &self.workdir_aliases)
            .map_err(|e| {
                RuntimeProcessError::ExecutionFailed(format!(
                    "cannot determine working directory: {e}"
                ))
            })?;
        let timeout = request
            .timeout_secs
            .map(Duration::from_secs)
            .unwrap_or(DEFAULT_COMMAND_TIMEOUT);
        if self.env_mode == LocalHostProcessEnvMode::Inherited {
            tracing::warn!(
                host_access = "full-local",
                "running local host command with inherited environment"
            );
        }
        let command = rewrite_local_host_command_aliases(&request.command, &self.workdir_aliases);
        let start = std::time::Instant::now();
        let (output, exit_code) = execute_local_command(
            &request.scope,
            &command,
            &cwd,
            timeout,
            &request.extra_env,
            self.env_mode,
        )
        .await?;
        Ok(CommandExecutionOutput {
            output: output.preview,
            saved_output: output.saved_output,
            exit_code: i64::from(exit_code),
            sandboxed: false,
            duration: start.elapsed(),
        })
    }
}

async fn execute_local_command(
    scope: &ResourceScope,
    cmd: &str,
    workdir: &PathBuf,
    timeout: Duration,
    extra_env: &HashMap<String, String>,
    env_mode: LocalHostProcessEnvMode,
) -> Result<(CapturedCommandOutput, i32), RuntimeProcessError> {
    let mut command = if cfg!(target_os = "windows") {
        let mut c = Command::new("cmd");
        c.args(["/C", cmd]);
        c
    } else {
        let mut c = Command::new("sh");
        c.args(["-c", cmd]);
        c
    };

    #[cfg(unix)]
    command.process_group(0);

    match env_mode {
        LocalHostProcessEnvMode::Scrubbed => {
            command.env_clear();
            for var in SAFE_ENV_VARS {
                if let Ok(val) = std::env::var(var) {
                    command.env(var, val);
                }
            }
            // Keep shell "~" expansion available without exposing the host user's home.
            command.env("HOME", workdir);
        }
        LocalHostProcessEnvMode::Inherited => {}
    }
    command.envs(extra_env);
    command
        .current_dir(workdir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = command.spawn().map_err(|e| {
        RuntimeProcessError::ExecutionFailed(format!("Failed to spawn command: {e}"))
    })?;

    let stdout_handle = child.stdout.take();
    let stderr_handle = child.stderr.take();

    let result = tokio::time::timeout(timeout, async {
        let stdout_fut = async {
            if let Some(out) = stdout_handle {
                read_stream_capped(scope, out).await
            } else {
                Ok(StreamCapture::default())
            }
        };

        let stderr_fut = async {
            if let Some(err) = stderr_handle {
                read_stream_capped(scope, err).await
            } else {
                Ok(StreamCapture::default())
            }
        };

        let (stdout, stderr, wait_result) = tokio::join!(stdout_fut, stderr_fut, child.wait());
        let status = wait_result.map_err(|error| {
            RuntimeProcessError::ExecutionFailed(format!("Command execution failed: {error}"))
        })?;
        Ok::<_, RuntimeProcessError>((stdout?, stderr?, status.code().unwrap_or(-1)))
    })
    .await;

    match result {
        Ok(Ok((stdout, stderr, code))) => {
            Ok((capture_command_output(scope, stdout, stderr)?, code))
        }
        Ok(Err(e)) => Err(e),
        Err(_) => {
            terminate_child_tree(&mut child).await;
            Err(RuntimeProcessError::Timeout(timeout))
        }
    }
}

async fn terminate_child_tree(child: &mut tokio::process::Child) {
    #[cfg(unix)]
    if let Some(pid) = child.id() {
        // SAFETY: Child was spawned into its own process group with pgid == pid.
        // Negative pid targets only that process group; result is best-effort.
        unsafe {
            let _ = kill(-(pid as i32), SIGKILL);
        }
    }
    let _ = child.kill().await;
    let _ = child.wait().await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process_output::{COMMAND_MAX_OUTPUT_SIZE, SavedCommandOutputSanitization};
    use std::sync::Mutex;

    #[derive(Debug)]
    struct RecordingSandboxTransport {
        requests: Mutex<Vec<CommandExecutionRequest>>,
        output: String,
    }

    impl Default for RecordingSandboxTransport {
        fn default() -> Self {
            Self {
                requests: Mutex::new(Vec::new()),
                output: "echo sandbox".to_string(),
            }
        }
    }

    #[derive(Debug)]
    struct FailingSandboxTransport;

    #[derive(Debug)]
    struct TimeoutSandboxTransport;

    #[async_trait]
    impl SandboxCommandTransport for RecordingSandboxTransport {
        async fn run_command(
            &self,
            request: CommandExecutionRequest,
        ) -> Result<CommandExecutionOutput, RuntimeProcessError> {
            self.requests.lock().unwrap().push(request);
            Ok(CommandExecutionOutput {
                output: self.output.clone(),
                saved_output: None,
                exit_code: 0,
                sandboxed: false,
                duration: Duration::from_millis(3),
            })
        }
    }

    #[async_trait]
    impl SandboxCommandTransport for FailingSandboxTransport {
        async fn run_command(
            &self,
            _request: CommandExecutionRequest,
        ) -> Result<CommandExecutionOutput, RuntimeProcessError> {
            Err(RuntimeProcessError::ExecutionFailed(
                "sandbox transport failed".to_string(),
            ))
        }
    }

    #[async_trait]
    impl SandboxCommandTransport for TimeoutSandboxTransport {
        async fn run_command(
            &self,
            request: CommandExecutionRequest,
        ) -> Result<CommandExecutionOutput, RuntimeProcessError> {
            Err(RuntimeProcessError::Timeout(Duration::from_secs(
                request.timeout_secs.unwrap_or_default(),
            )))
        }
    }

    #[tokio::test]
    async fn tenant_sandbox_process_port_marks_output_sandboxed() {
        let transport = std::sync::Arc::new(RecordingSandboxTransport::default());
        let port = TenantSandboxProcessPort::new(transport);

        let output = port
            .run_command(CommandExecutionRequest {
                scope: ResourceScope::system(),
                mounts: None,
                command: "echo sandbox".to_string(),
                workdir: None,
                timeout_secs: None,
                extra_env: HashMap::new(),
            })
            .await
            .unwrap();

        assert_eq!(output.output, "echo sandbox");
        assert!(output.sandboxed);
    }

    #[tokio::test]
    async fn tenant_sandbox_process_port_sets_default_timeout_on_transport_request() {
        let transport = std::sync::Arc::new(RecordingSandboxTransport::default());
        let port = TenantSandboxProcessPort::new(transport.clone());

        port.run_command(CommandExecutionRequest {
            scope: ResourceScope::system(),
            mounts: None,
            command: "echo sandbox".to_string(),
            workdir: None,
            timeout_secs: None,
            extra_env: HashMap::new(),
        })
        .await
        .unwrap();

        let requests = transport.requests.lock().unwrap();
        assert_eq!(
            requests[0].timeout_secs,
            Some(DEFAULT_COMMAND_TIMEOUT.as_secs())
        );
    }

    #[tokio::test]
    async fn tenant_sandbox_process_port_propagates_transport_error() {
        let port = TenantSandboxProcessPort::new(std::sync::Arc::new(FailingSandboxTransport));

        let error = port
            .run_command(CommandExecutionRequest {
                scope: ResourceScope::system(),
                mounts: None,
                command: "echo sandbox".to_string(),
                workdir: None,
                timeout_secs: None,
                extra_env: HashMap::new(),
            })
            .await
            .unwrap_err();

        assert_eq!(
            error,
            RuntimeProcessError::ExecutionFailed("sandbox transport failed".to_string())
        );
    }

    #[tokio::test]
    async fn tenant_sandbox_process_port_propagates_transport_timeout() {
        let port = TenantSandboxProcessPort::new(std::sync::Arc::new(TimeoutSandboxTransport));

        let error = port
            .run_command(CommandExecutionRequest {
                scope: ResourceScope::system(),
                mounts: None,
                command: "echo sandbox".to_string(),
                workdir: None,
                timeout_secs: Some(1),
                extra_env: HashMap::new(),
            })
            .await
            .unwrap_err();

        assert_eq!(error, RuntimeProcessError::Timeout(Duration::from_secs(1)));
    }

    #[tokio::test]
    async fn tenant_sandbox_process_port_truncates_transport_output() {
        let transport = std::sync::Arc::new(RecordingSandboxTransport {
            requests: Mutex::new(Vec::new()),
            output: "x".repeat(COMMAND_MAX_OUTPUT_SIZE + 1),
        });
        let port = TenantSandboxProcessPort::new(transport);

        let output = port
            .run_command(CommandExecutionRequest {
                scope: ResourceScope::system(),
                mounts: None,
                command: "echo sandbox".to_string(),
                workdir: None,
                timeout_secs: None,
                extra_env: HashMap::new(),
            })
            .await
            .unwrap();

        assert!(output.output.contains("... [truncated 1 bytes] ..."));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn execute_local_command_saves_large_output_file() {
        let workdir = tempfile::tempdir().expect("tempdir");
        let middle = "MIDDLE-FROM-COMMAND";

        let (output, exit_code) = execute_local_command(
            &ResourceScope::system(),
            "yes a | head -c 70000; printf 'MIDDLE-FROM-COMMAND'; yes z | head -c 70000",
            &workdir.path().to_path_buf(),
            Duration::from_secs(5),
            &HashMap::new(),
            LocalHostProcessEnvMode::Scrubbed,
        )
        .await
        .expect("command succeeds");
        let saved_output = output.saved_output.expect("saved output metadata");
        let saved = std::fs::read_to_string(&saved_output.path).expect("saved output readable");
        let _ = std::fs::remove_file(&saved_output.path);

        assert_eq!(exit_code, 0);
        assert!(!output.preview.contains(middle));
        assert_eq!(
            saved_output.sanitization,
            SavedCommandOutputSanitization::Clean
        );
        assert!(saved.contains(middle));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn execute_local_command_overrides_home_to_workdir() {
        let workdir = tempfile::tempdir().expect("tempdir");

        let (output, exit_code) = execute_local_command(
            &ResourceScope::system(),
            "printf '%s' \"$HOME\"",
            &workdir.path().to_path_buf(),
            Duration::from_secs(5),
            &HashMap::new(),
            LocalHostProcessEnvMode::Scrubbed,
        )
        .await
        .expect("command succeeds");

        assert_eq!(exit_code, 0);
        assert_eq!(output.preview, workdir.path().display().to_string());
        assert_eq!(output.saved_output, None);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn execute_local_command_inherited_env_preserves_home_and_host_env() {
        let workdir = tempfile::tempdir().expect("tempdir");
        let home = std::env::var("HOME").expect("HOME set for inherited env test");

        let (output, exit_code) = execute_local_command(
            &ResourceScope::system(),
            "printf '%s\\n%s' \"$HOME\" \"$IRONCLAW_REBORN_SENTINEL\"",
            &workdir.path().to_path_buf(),
            Duration::from_secs(5),
            &HashMap::from([(
                "IRONCLAW_REBORN_SENTINEL".to_string(),
                "inherited".to_string(),
            )]),
            LocalHostProcessEnvMode::Inherited,
        )
        .await
        .expect("command succeeds");

        assert_eq!(exit_code, 0);
        assert_eq!(output.preview, format!("{home}\ninherited"));
        assert_eq!(output.saved_output, None);
    }

    #[tokio::test]
    async fn local_host_process_port_translates_workspace_workdir_when_configured() {
        let workspace = tempfile::tempdir().expect("workspace tempdir");
        let nested = workspace.path().join("qa-coding-smoke");
        std::fs::create_dir_all(&nested).expect("nested workspace dir");
        let port = LocalHostProcessPort::new_inherited_env()
            .with_workdir_alias("/workspace", workspace.path().to_path_buf());

        let output = port
            .run_command(CommandExecutionRequest {
                scope: ResourceScope::system(),
                mounts: None,
                command: "printf '%s' \"$PWD\"".to_string(),
                workdir: Some("/workspace/qa-coding-smoke".to_string()),
                timeout_secs: Some(5),
                extra_env: HashMap::new(),
            })
            .await
            .expect("command succeeds");

        assert_eq!(output.exit_code, 0);
        assert_eq!(
            output.output,
            nested
                .canonicalize()
                .expect("canonical nested")
                .display()
                .to_string()
        );
    }

    #[tokio::test]
    async fn local_host_process_port_rewrites_command_path_aliases() {
        let workspace = tempfile::tempdir().expect("workspace tempdir");
        let scratch = workspace.path().join("qa-coding-smoke");
        let port = LocalHostProcessPort::new_inherited_env()
            .with_workdir_alias("/workspace", workspace.path().to_path_buf());

        let output = port
            .run_command(CommandExecutionRequest {
                scope: ResourceScope::system(),
                mounts: None,
                command: "mkdir -p /workspace/qa-coding-smoke && test -d /workspace/qa-coding-smoke && printf ok".to_string(),
                workdir: None,
                timeout_secs: Some(5),
                extra_env: HashMap::new(),
            })
            .await
            .expect("command succeeds");

        assert_eq!(output.exit_code, 0);
        assert_eq!(output.output, "ok");
        assert!(scratch.exists());
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn execute_local_command_runs_through_windows_cmd() {
        let workdir = tempfile::tempdir().expect("tempdir");

        let (output, exit_code) = execute_local_command(
            &ResourceScope::system(),
            "echo %HOME%",
            &workdir.path().to_path_buf(),
            Duration::from_secs(5),
            &HashMap::new(),
            LocalHostProcessEnvMode::Scrubbed,
        )
        .await
        .expect("command succeeds");

        assert_eq!(exit_code, 0);
        assert_eq!(output.preview.trim(), workdir.path().display().to_string());
        assert_eq!(output.saved_output, None);
    }
}
