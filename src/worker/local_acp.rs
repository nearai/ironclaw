//! Local ACP execution — runs ACP agents as subprocesses without Docker or an orchestrator.
//!
//! Designed for desktop mode: the IronClaw host process spawns an ACP-compliant
//! agent (e.g., OpenCode) directly, communicating via JSON-RPC over stdio.
//! Events are delivered through a tokio broadcast channel instead of HTTP POST
//! to an orchestrator.
//!
//! ```text
//! ┌───────────────────────────────────────────────┐
//! │ IronClaw host process (desktop mode)          │
//! │                                               │
//! │  run_local_acp_session(config, prompt)         │
//! │    └─ spawns ACP agent subprocess             │
//! │    └─ ACP handshake (initialize + session)    │
//! │    └─ sends prompt via prompt()               │
//! │    └─ translates ACP events → JobEventPayload │
//! │    └─ emits events via mpsc channel           │
//! └───────────────────────────────────────────────┘
//! ```

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use agent_client_protocol::{self as acp, Agent as _};
use serde_json::json;
use tokio::io::AsyncBufReadExt;
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};
use uuid::Uuid;

use crate::config::acp::AcpAgentConfig;
use crate::error::WorkerError;
use crate::worker::acp_bridge::{AcpEventSink, IronClawAcpClient, ironclaw_init_request};
use crate::worker::api::JobEventPayload;

/// Event sink that delivers events through a tokio mpsc channel.
/// Used by the web gateway SSE or any other consumer in the host process.
pub struct ChannelAcpEventSink {
    tx: mpsc::Sender<JobEventPayload>,
}

impl ChannelAcpEventSink {
    pub fn new(tx: mpsc::Sender<JobEventPayload>) -> Self {
        Self { tx }
    }
}

impl AcpEventSink for ChannelAcpEventSink {
    async fn emit_event(&self, payload: &JobEventPayload) {
        if self.tx.send(payload.clone()).await.is_err() {
            tracing::debug!("local ACP event sink: receiver dropped, event lost");
        }
    }
}

impl AcpEventSink for Arc<ChannelAcpEventSink> {
    async fn emit_event(&self, payload: &JobEventPayload) {
        (**self).emit_event(payload).await;
    }
}

/// Configuration for a local ACP session.
pub struct LocalAcpConfig {
    /// Unique ID for this session (used for logging).
    pub session_id: Uuid,
    /// ACP agent configuration (command, args, env).
    pub agent: AcpAgentConfig,
    /// Working directory for the agent subprocess.
    /// Overrides `agent.current_dir` if set.
    pub working_dir: Option<PathBuf>,
    /// Maximum time to wait for the agent to complete.
    pub timeout: Duration,
}

/// Run an ACP session locally, delivering events through the returned receiver.
///
/// Returns `(event_receiver, join_handle)`:
/// - `event_receiver`: stream of `JobEventPayload` events (status, text, tool calls, etc.)
/// - `join_handle`: completes when the session finishes (call `.await` to get the result)
///
/// Spawns a dedicated OS thread with a single-threaded tokio runtime because the
/// ACP SDK uses `!Send` futures (`LocalSet`). This mirrors how the container-based
/// ACP bridge runs on its own thread inside Docker.
pub fn spawn_local_acp_session(
    config: LocalAcpConfig,
    prompt: String,
) -> (
    mpsc::Receiver<JobEventPayload>,
    tokio::task::JoinHandle<Result<(), WorkerError>>,
) {
    let (tx, rx) = mpsc::channel(256);

    // Use spawn_blocking to get a thread that can run a LocalSet.
    // Inside, we create a current-thread runtime for the !Send ACP futures.
    let handle = tokio::task::spawn_blocking(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| WorkerError::ExecutionFailed {
                reason: format!("failed to create ACP runtime: {e}"),
            })?;

        rt.block_on(run_local_acp_session_inner(config, prompt, tx))
    });

    // Wrap the JoinHandle to flatten the Result<Result<..>, JoinError>
    let flattened = tokio::task::spawn(async move {
        match handle.await {
            Ok(result) => result,
            Err(e) => Err(WorkerError::ExecutionFailed {
                reason: format!("ACP thread panicked: {e}"),
            }),
        }
    });

    (rx, flattened)
}

async fn run_local_acp_session_inner(
    config: LocalAcpConfig,
    prompt: String,
    tx: mpsc::Sender<JobEventPayload>,
) -> Result<(), WorkerError> {
    let session_id = config.session_id;
    let agent = &config.agent;

    // Determine working directory
    let working_dir = config
        .working_dir
        .or_else(|| agent.current_dir.clone())
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    tracing::info!(
        session_id = %session_id,
        agent = %agent.name,
        working_dir = %working_dir.display(),
        "Starting local ACP session"
    );

    // Emit starting status
    let _ = tx
        .send(JobEventPayload {
            event_type: "status".to_string(),
            data: json!({ "message": format!("Spawning ACP agent: {}", agent.command) }),
        })
        .await;

    // Spawn the ACP agent subprocess
    let mut cmd = Command::new(&agent.command);
    cmd.args(&agent.args);
    cmd.envs(&agent.env);
    cmd.current_dir(&working_dir)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);

    let mut child = cmd.spawn().map_err(|e| WorkerError::ExecutionFailed {
        reason: format!("failed to spawn ACP agent '{}': {}", agent.command, e),
    })?;

    let child_stdin = child.stdin.take().ok_or(WorkerError::ExecutionFailed {
        reason: "failed to capture ACP agent stdin".to_string(),
    })?;
    let child_stdout = child.stdout.take().ok_or(WorkerError::ExecutionFailed {
        reason: "failed to capture ACP agent stdout".to_string(),
    })?;
    let child_stderr = child.stderr.take().ok_or(WorkerError::ExecutionFailed {
        reason: "failed to capture ACP agent stderr".to_string(),
    })?;

    // Spawn stderr reader that emits status events
    let tx_for_stderr = tx.clone();
    let stderr_handle = tokio::spawn(async move {
        let reader = tokio::io::BufReader::new(child_stderr);
        let mut lines = reader.lines();
        while let Ok(Some(line)) = lines.next_line().await {
            tracing::debug!(session_id = %session_id, "acp agent stderr: {}", line);
            let _ = tx_for_stderr
                .send(JobEventPayload {
                    event_type: "status".to_string(),
                    data: json!({ "message": line }),
                })
                .await;
        }
    });

    // Monitor child process for exit
    let (child_exit_tx, child_exit_rx) = tokio::sync::oneshot::channel();
    let (kill_tx, kill_rx) = tokio::sync::oneshot::channel::<()>();
    tokio::spawn(async move {
        let exit_code = tokio::select! {
            status = child.wait() => status.ok().and_then(|s| s.code()),
            Ok(()) = kill_rx => {
                let _ = child.kill().await;
                child.wait().await.ok().and_then(|s| s.code())
            }
        };
        let _ = child_exit_tx.send(exit_code);
    });

    // Run the ACP protocol (SDK futures are !Send — needs LocalSet)
    let sink = Arc::new(ChannelAcpEventSink::new(tx.clone()));
    let timeout = config.timeout;

    let local_set = tokio::task::LocalSet::new();
    let acp_result = local_set
        .run_until(async move {
            let outgoing = child_stdin.compat_write();
            let incoming = child_stdout.compat();

            let ironclaw_client = IronClawAcpClient::new(sink);

            let (conn, handle_io) =
                acp::ClientSideConnection::new(ironclaw_client, outgoing, incoming, |fut| {
                    tokio::task::spawn_local(fut);
                });
            tokio::task::spawn_local(handle_io);

            // ACP handshake
            conn.initialize(ironclaw_init_request())
                .await
                .map_err(|e| WorkerError::ExecutionFailed {
                    reason: format!("ACP initialize failed: {e}"),
                })?;

            tracing::info!(session_id = %session_id, "ACP handshake complete");

            // Create session with working directory
            let session_response = conn
                .new_session(acp::NewSessionRequest::new(working_dir.clone()))
                .await
                .map_err(|e| WorkerError::ExecutionFailed {
                    reason: format!("ACP new_session failed: {e}"),
                })?;

            let acp_session_id = session_response.session_id.clone();
            tracing::info!(
                session_id = %session_id,
                acp_session = %acp_session_id,
                "ACP session created"
            );

            // Send prompt with timeout
            let prompt_result = tokio::time::timeout(timeout, async {
                conn.prompt(acp::PromptRequest::new(
                    acp_session_id.clone(),
                    vec![prompt.into()],
                ))
                .await
            })
            .await;

            match prompt_result {
                Ok(Ok(_resp)) => {
                    tracing::info!(session_id = %session_id, "ACP prompt completed");
                }
                Ok(Err(e)) => {
                    return Err(WorkerError::ExecutionFailed {
                        reason: format!("ACP prompt failed: {e}"),
                    });
                }
                Err(_) => {
                    return Err(WorkerError::ExecutionFailed {
                        reason: "ACP prompt timed out".to_string(),
                    });
                }
            }

            Ok::<(), WorkerError>(())
        })
        .await;

    // Kill child on failure so stderr closes
    if acp_result.is_err() {
        let _ = kill_tx.send(());
    }

    // Wait for stderr reader to finish
    let _ = stderr_handle.await;

    // Wait for child exit
    let exit_code = child_exit_rx.await.ok().flatten();

    // Emit terminal event
    let (success, message) = match &acp_result {
        Ok(()) => (true, "ACP session completed".to_string()),
        Err(e) => (false, format!("ACP session failed: {e}")),
    };

    let _ = tx
        .send(JobEventPayload {
            event_type: "result".to_string(),
            data: json!({
                "status": if success { "completed" } else { "error" },
                "message": &message,
                "exit_code": exit_code,
            }),
        })
        .await;

    acp_result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn channel_event_sink_constructs() {
        let (tx, _rx) = mpsc::channel(16);
        let _sink = ChannelAcpEventSink::new(tx);
    }

    #[test]
    fn local_acp_config_constructs() {
        let config = LocalAcpConfig {
            session_id: Uuid::new_v4(),
            agent: AcpAgentConfig::new("opencode", "opencode", vec!["acp".into()], HashMap::new()),
            working_dir: Some(PathBuf::from("/tmp/test-project")),
            timeout: Duration::from_secs(300),
        };
        assert_eq!(config.agent.name, "opencode");
    }
}
