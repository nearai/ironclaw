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
use tokio::{io::AsyncReadExt, process::Command};

/// Maximum captured output before middle truncation.
pub(crate) const COMMAND_MAX_OUTPUT_SIZE: usize = 64 * 1024;

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

/// Local provider-host command implementation matching the existing shell path.
#[derive(Debug, Clone, Default)]
pub struct LocalHostProcessPort;

impl LocalHostProcessPort {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl RuntimeProcessPort for LocalHostProcessPort {
    async fn run_command(
        &self,
        request: CommandExecutionRequest,
    ) -> Result<CommandExecutionOutput, RuntimeProcessError> {
        let cwd = request
            .workdir
            .as_deref()
            .map(PathBuf::from)
            .map(Ok)
            .unwrap_or_else(std::env::current_dir)
            .map_err(|e| {
                RuntimeProcessError::ExecutionFailed(format!(
                    "cannot determine working directory: {e}"
                ))
            })?;
        let timeout = request
            .timeout_secs
            .map(Duration::from_secs)
            .unwrap_or(DEFAULT_COMMAND_TIMEOUT);
        let start = std::time::Instant::now();
        let (output, exit_code) =
            execute_local_command(&request.command, &cwd, timeout, &request.extra_env).await?;
        Ok(CommandExecutionOutput {
            output,
            exit_code: i64::from(exit_code),
            sandboxed: false,
            duration: start.elapsed(),
        })
    }
}

async fn execute_local_command(
    cmd: &str,
    workdir: &PathBuf,
    timeout: Duration,
    extra_env: &HashMap<String, String>,
) -> Result<(String, i32), RuntimeProcessError> {
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

    command.env_clear();
    for var in SAFE_ENV_VARS {
        if let Ok(val) = std::env::var(var) {
            command.env(var, val);
        }
    }
    command.envs(extra_env);
    // Keep shell "~" expansion available without exposing the host user's home.
    command.env("HOME", workdir);
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
                read_stream_limited(out).await
            } else {
                String::new()
            }
        };

        let stderr_fut = async {
            if let Some(err) = stderr_handle {
                read_stream_limited(err).await
            } else {
                String::new()
            }
        };

        let (stdout, stderr, wait_result) = tokio::join!(stdout_fut, stderr_fut, child.wait());
        let status = wait_result?;
        let output = if stderr.is_empty() {
            stdout
        } else if stdout.is_empty() {
            stderr
        } else {
            format!("{stdout}\n\n--- stderr ---\n{stderr}")
        };
        Ok::<_, std::io::Error>((output, status.code().unwrap_or(-1)))
    })
    .await;

    match result {
        Ok(Ok((output, code))) => Ok((truncate_output(&output), code)),
        Ok(Err(e)) => Err(RuntimeProcessError::ExecutionFailed(format!(
            "Command execution failed: {e}"
        ))),
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

async fn read_stream_limited<R>(mut stream: R) -> String
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut buf = Vec::new();
    (&mut stream)
        .take((COMMAND_MAX_OUTPUT_SIZE + 1) as u64)
        .read_to_end(&mut buf)
        .await
        .ok();
    tokio::io::copy(&mut stream, &mut tokio::io::sink())
        .await
        .ok();
    let output = String::from_utf8_lossy(&buf).to_string();
    truncate_output(&output)
}

fn truncate_output(s: &str) -> String {
    if s.len() <= COMMAND_MAX_OUTPUT_SIZE {
        s.to_string()
    } else {
        let half = COMMAND_MAX_OUTPUT_SIZE / 2;
        let head_end = floor_char_boundary(s, half);
        let tail_start = floor_char_boundary(s, s.len() - half);
        format!(
            "{}\n\n... [truncated {} bytes] ...\n\n{}",
            &s[..head_end], // safety: head_end was clamped to a UTF-8 character boundary.
            s.len() - COMMAND_MAX_OUTPUT_SIZE,
            &s[tail_start..]
        )
    }
}

fn floor_char_boundary(s: &str, pos: usize) -> usize {
    if pos >= s.len() {
        return s.len();
    }
    let mut i = pos;
    while !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_output_preserves_exact_limit() {
        let output = "x".repeat(COMMAND_MAX_OUTPUT_SIZE);

        assert_eq!(truncate_output(&output), output);
    }

    #[test]
    fn truncate_output_respects_utf8_boundaries() {
        let output = format!(
            "{}{}{}",
            "a".repeat(COMMAND_MAX_OUTPUT_SIZE / 2 - 1),
            "é",
            "b".repeat(COMMAND_MAX_OUTPUT_SIZE)
        );

        let truncated = truncate_output(&output);

        assert!(truncated.is_char_boundary(COMMAND_MAX_OUTPUT_SIZE / 2 - 1));
        assert!(truncated.contains("... [truncated "));
        assert!(truncated.starts_with(&"a".repeat(COMMAND_MAX_OUTPUT_SIZE / 2 - 1)));
        assert!(truncated.ends_with(&"b".repeat(COMMAND_MAX_OUTPUT_SIZE / 2)));
    }

    #[tokio::test]
    async fn read_stream_limited_truncates_after_limit() {
        let input = "x".repeat(COMMAND_MAX_OUTPUT_SIZE + 1);

        let output = read_stream_limited(input.as_bytes()).await;

        assert!(output.len() > COMMAND_MAX_OUTPUT_SIZE);
        assert!(output.contains("... [truncated 1 bytes] ..."));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn execute_local_command_overrides_home_to_workdir() {
        let workdir = tempfile::tempdir().expect("tempdir");

        let (output, exit_code) = execute_local_command(
            "printf '%s' \"$HOME\"",
            &workdir.path().to_path_buf(),
            Duration::from_secs(5),
            &HashMap::new(),
        )
        .await
        .expect("command succeeds");

        assert_eq!(exit_code, 0);
        assert_eq!(output, workdir.path().display().to_string());
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn execute_local_command_runs_through_windows_cmd() {
        let workdir = tempfile::tempdir().expect("tempdir");

        let (output, exit_code) = execute_local_command(
            "echo %HOME%",
            &workdir.path().to_path_buf(),
            Duration::from_secs(5),
            &HashMap::new(),
        )
        .await
        .expect("command succeeds");

        assert_eq!(exit_code, 0);
        assert_eq!(output.trim(), workdir.path().display().to_string());
    }
}
