//! Process management for Claude CLI.
//!
//! Spawns `claude` as a child process, reads NDJSON events from stdout via a
//! background task, and optionally provides stdin access for sidecar mode.

use std::process::Stdio;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::claude_cli::config::ClaudeCodeConfig;
use crate::claude_cli::error::ClaudeCliError;
use crate::claude_cli::types::CliEvent;

/// A running Claude CLI process with async NDJSON event streaming.
///
/// Events are parsed from stdout in a background task and delivered via
/// an internal channel. Call [`next_event`](Self::next_event) to receive them.
///
/// When done reading events, call [`wait`](Self::wait) to ensure the process
/// exits cleanly.
pub struct ClaudeProcess {
    child: Child,
    event_rx: mpsc::Receiver<CliEvent>,
    stdin: Option<ChildStdin>,
    stdout_handle: JoinHandle<()>,
    stderr_handle: JoinHandle<String>,
}

impl ClaudeProcess {
    /// Spawn a Claude CLI process in oneshot mode (`-p "prompt"`).
    pub fn spawn_oneshot(
        config: &ClaudeCodeConfig,
        prompt: &str,
    ) -> Result<Self, ClaudeCliError> {
        let args = config.oneshot_args(prompt);
        Self::spawn_raw(config, args, false)
    }

    /// Spawn a Claude CLI process in sidecar (interactive) mode.
    ///
    /// The process starts with `--input-format stream-json` and waits for
    /// messages on stdin. Use [`write_stdin`](Self::write_stdin) to send prompts.
    pub fn spawn_sidecar(config: &ClaudeCodeConfig) -> Result<Self, ClaudeCliError> {
        let args = config.sidecar_args();
        Self::spawn_raw(config, args, true)
    }

    /// Spawn a Claude CLI process in resume mode (`--resume <session_id>`).
    pub fn spawn_resume(
        config: &ClaudeCodeConfig,
        session_id: &str,
        prompt: &str,
    ) -> Result<Self, ClaudeCliError> {
        let args = config.resume_args(session_id, prompt);
        Self::spawn_raw(config, args, false)
    }

    fn spawn_raw(
        config: &ClaudeCodeConfig,
        args: Vec<String>,
        needs_stdin: bool,
    ) -> Result<Self, ClaudeCliError> {
        let mut cmd = Command::new(&config.binary);
        cmd.args(&args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        if needs_stdin {
            cmd.stdin(Stdio::piped());
        } else {
            cmd.stdin(Stdio::null());
        }

        if let Some(ref cwd) = config.cwd {
            cmd.current_dir(cwd);
        }

        let mut child = cmd.spawn().map_err(ClaudeCliError::SpawnFailed)?;

        let stdout = child.stdout.take().expect("stdout was piped");
        let stderr = child.stderr.take().expect("stderr was piped");
        let stdin = if needs_stdin {
            Some(child.stdin.take().expect("stdin was piped"))
        } else {
            None
        };

        let (tx, rx) = mpsc::channel(256);

        let stdout_handle = tokio::spawn(async move {
            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                match CliEvent::parse(trimmed) {
                    Ok(event) => {
                        if tx.send(event).await.is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        tracing::debug!("Failed to parse claude NDJSON line: {}", e);
                    }
                }
            }
        });

        let stderr_handle = tokio::spawn(async move {
            let reader = BufReader::new(stderr);
            let mut lines = reader.lines();
            let mut collected = String::new();
            while let Ok(Some(line)) = lines.next_line().await {
                tracing::debug!("claude stderr: {}", line);
                if !collected.is_empty() {
                    collected.push('\n');
                }
                collected.push_str(&line);
            }
            collected
        });

        Ok(Self {
            child,
            event_rx: rx,
            stdin,
            stdout_handle,
            stderr_handle,
        })
    }

    /// Receive the next parsed event from the Claude process.
    ///
    /// Returns `None` when the process has finished writing to stdout.
    pub async fn next_event(&mut self) -> Option<CliEvent> {
        self.event_rx.recv().await
    }

    /// Write a JSON message to the process stdin (sidecar mode only).
    ///
    /// Returns `NotSidecarMode` if the process was not started in sidecar mode.
    pub async fn write_stdin(&mut self, message: &serde_json::Value) -> Result<(), ClaudeCliError> {
        let stdin = self.stdin.as_mut().ok_or(ClaudeCliError::NotSidecarMode)?;
        let mut line = serde_json::to_string(message).map_err(|e| {
            ClaudeCliError::StdinWriteFailed(std::io::Error::new(std::io::ErrorKind::InvalidData, e))
        })?;
        line.push('\n');
        stdin
            .write_all(line.as_bytes())
            .await
            .map_err(ClaudeCliError::StdinWriteFailed)?;
        stdin
            .flush()
            .await
            .map_err(ClaudeCliError::StdinWriteFailed)?;
        Ok(())
    }

    /// Wait for the process to exit.
    ///
    /// Drops stdin (signaling EOF) and waits for the child process and all
    /// background reader tasks to complete. Returns an error if the process
    /// exited with a non-zero code.
    ///
    /// Callers should drain all events via [`next_event`](Self::next_event)
    /// before calling this method.
    pub async fn wait(mut self) -> Result<(), ClaudeCliError> {
        drop(self.stdin.take());

        let status = self.child.wait().await.map_err(ClaudeCliError::Io)?;
        let _ = self.stdout_handle.await;
        let stderr = self.stderr_handle.await.unwrap_or_default();

        if !status.success() {
            let code = status.code().unwrap_or(-1);
            if code == -1 {
                return Err(ClaudeCliError::ProcessKilled);
            }
            return Err(ClaudeCliError::ProcessFailed { code, stderr });
        }

        Ok(())
    }

    /// Kill the process immediately.
    pub async fn kill(&mut self) -> Result<(), ClaudeCliError> {
        self.child.kill().await.map_err(ClaudeCliError::Io)
    }
}
