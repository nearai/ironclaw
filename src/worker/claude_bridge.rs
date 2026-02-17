//! Claude Code bridge for sandboxed execution.
//!
//! Spawns the `claude` CLI inside a Docker container and streams its NDJSON
//! output back to the orchestrator via HTTP. Supports follow-up prompts via
//! `--resume`.
//!
//! Security model: the Docker container is the primary security boundary
//! (cap-drop ALL, non-root user, memory limits, network isolation).
//! As defense-in-depth, a project-level `.claude/settings.json` is written
//! before spawning with an explicit tool allowlist. Only listed tools are
//! auto-approved; unknown/future tools would require interactive approval,
//! which times out harmlessly in the non-interactive container.
//!
//! ```text
//! ┌──────────────────────────────────────────────┐
//! │ Docker Container                              │
//! │                                               │
//! │  ironclaw claude-bridge --job-id <uuid>       │
//! │    └─ writes /workspace/.claude/settings.json │
//! │    └─ claude -p "task" --output-format        │
//! │       stream-json                             │
//! │    └─ reads stdout line-by-line               │
//! │    └─ POSTs events to orchestrator            │
//! │    └─ polls for follow-up prompts             │
//! │    └─ on follow-up: claude --resume           │
//! └──────────────────────────────────────────────┘
//! ```

use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use uuid::Uuid;

use crate::error::WorkerError;
use crate::worker::api::{CompletionReport, JobEventPayload, PromptResponse, WorkerHttpClient};

/// Configuration for the Claude bridge runtime.
pub struct ClaudeBridgeConfig {
    pub job_id: Uuid,
    pub orchestrator_url: String,
    pub max_turns: u32,
    pub model: String,
    pub timeout: Duration,
    /// Tool patterns to auto-approve via project-level settings.json.
    pub allowed_tools: Vec<String>,
}

/// A Claude Code streaming event (NDJSON line from `--output-format stream-json`).
///
/// Claude Code emits one JSON object per line. We capture the key fields
/// we need and forward the rest as opaque data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudeStreamEvent {
    #[serde(rename = "type")]
    pub event_type: String,

    /// For `system` events: the session ID.
    #[serde(default)]
    pub session_id: Option<String>,

    /// For `assistant` events: the text content blocks.
    #[serde(default)]
    pub content: Option<Vec<ContentBlock>>,

    /// For `result` events: final status info.
    #[serde(default)]
    pub result: Option<ResultInfo>,

    /// For `tool_use`/`tool_result`: the tool name.
    #[serde(default)]
    pub tool_name: Option<String>,

    /// For `tool_use`: the input parameters.
    #[serde(default)]
    pub input: Option<serde_json::Value>,

    /// For `tool_result`: the output content.
    #[serde(default)]
    pub output: Option<String>,

    /// Subtype discriminator (e.g. "text", "tool_use", "tool_result").
    #[serde(default)]
    pub subtype: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentBlock {
    #[serde(rename = "type")]
    pub block_type: String,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub input: Option<serde_json::Value>,
    #[serde(default)]
    pub content: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResultInfo {
    #[serde(default)]
    pub is_error: Option<bool>,
    #[serde(default)]
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub num_turns: Option<u32>,
}

/// The Claude Code bridge runtime.
pub struct ClaudeBridgeRuntime {
    config: ClaudeBridgeConfig,
    client: Arc<WorkerHttpClient>,
}

impl ClaudeBridgeRuntime {
    /// Create a new bridge runtime.
    ///
    /// Reads `IRONCLAW_WORKER_TOKEN` from the environment for auth.
    pub fn new(config: ClaudeBridgeConfig) -> Result<Self, WorkerError> {
        let client = Arc::new(WorkerHttpClient::from_env(
            config.orchestrator_url.clone(),
            config.job_id,
        )?);

        Ok(Self { config, client })
    }

    /// Write project-level `.claude/settings.json` with the tool allowlist.
    ///
    /// This replaces `--dangerously-skip-permissions` with an explicit set of
    /// auto-approved tools. The Docker container is still the primary security
    /// boundary; this is defense-in-depth.
    fn write_permission_settings(&self) -> Result<(), WorkerError> {
        let settings_json = build_permission_settings(&self.config.allowed_tools);
        let settings_dir = std::path::Path::new("/workspace/.claude");
        std::fs::create_dir_all(settings_dir).map_err(|e| WorkerError::ExecutionFailed {
            reason: format!("failed to create /workspace/.claude/: {e}"),
        })?;
        std::fs::write(settings_dir.join("settings.json"), &settings_json).map_err(|e| {
            WorkerError::ExecutionFailed {
                reason: format!("failed to write settings.json: {e}"),
            }
        })?;
        tracing::info!(
            job_id = %self.config.job_id,
            tools = ?self.config.allowed_tools,
            "Wrote Claude Code permission settings"
        );
        Ok(())
    }

    /// Copy auth files from the read-only host mount into the writable home dir.
    ///
    /// When the orchestrator mounts the host's `~/.claude` at
    /// `/home/sandbox/.claude-host:ro`, this copies everything into the
    /// container's own `/home/sandbox/.claude` so Claude Code can read auth
    /// credentials AND write its state (todos, debug files, etc.) without
    /// touching the host filesystem.
    fn copy_auth_from_mount(&self) -> Result<(), WorkerError> {
        let mount = std::path::Path::new("/home/sandbox/.claude-host");
        if !mount.exists() {
            return Ok(());
        }

        let target = std::path::Path::new("/home/sandbox/.claude");
        std::fs::create_dir_all(target).map_err(|e| WorkerError::ExecutionFailed {
            reason: format!("failed to create ~/.claude: {e}"),
        })?;

        let copied =
            copy_dir_recursive(mount, target).map_err(|e| WorkerError::ExecutionFailed {
                reason: format!("failed to copy auth from host mount: {e}"),
            })?;

        tracing::info!(
            job_id = %self.config.job_id,
            files_copied = copied,
            "Copied auth config from host mount into container"
        );
        Ok(())
    }

    /// Run the bridge: fetch job, spawn claude, stream events, handle follow-ups.
    pub async fn run(&self) -> Result<(), WorkerError> {
        // Copy auth files from read-only host mount (if present) into the
        // writable home directory before Claude Code needs them.
        self.copy_auth_from_mount()?;

        // Write project-level settings with explicit tool allowlist.
        // This replaces --dangerously-skip-permissions with defense-in-depth:
        // only the listed tools are auto-approved, unknown tools fail safely.
        self.write_permission_settings()?;

        // Fetch the job description from the orchestrator
        let job = self.client.get_job().await?;

        tracing::info!(
            job_id = %self.config.job_id,
            "Starting Claude Code bridge for: {}",
            truncate(&job.description, 100)
        );

        // Fetch and set credentials before spawning Claude.
        // The spawned `claude` Command inherits env vars from this process.
        let credentials = self.client.fetch_credentials().await?;
        for cred in &credentials {
            // SAFETY: bridge is single-threaded at this point (no child processes yet)
            unsafe { std::env::set_var(&cred.env_var, &cred.value) };
        }
        if !credentials.is_empty() {
            tracing::info!(
                job_id = %self.config.job_id,
                "Injected {} credential(s) into environment",
                credentials.len()
            );
        }

        // Warn if no auth method is available.
        if std::env::var("ANTHROPIC_API_KEY").is_err()
            && std::env::var("CLAUDE_CODE_OAUTH_TOKEN").is_err()
        {
            tracing::warn!(
                job_id = %self.config.job_id,
                "No Claude Code auth available. Set ANTHROPIC_API_KEY or run \
                 `claude login` on the host to authenticate."
            );
        }

        // Report that we're running
        self.client
            .report_status(&crate::worker::api::StatusUpdate {
                state: "running".to_string(),
                message: Some("Spawning Claude Code".to_string()),
                iteration: 0,
            })
            .await?;

        // Run the initial Claude session
        let session_id = match self.run_claude_session(&job.description, None).await {
            Ok(sid) => sid,
            Err(e) => {
                tracing::error!(job_id = %self.config.job_id, "Claude session failed: {}", e);
                self.client
                    .report_complete(&CompletionReport {
                        success: false,
                        message: Some(format!("Claude Code failed: {}", e)),
                        iterations: 1,
                    })
                    .await?;
                return Ok(());
            }
        };

        // Follow-up loop: poll for prompts, resume Claude sessions
        let mut iteration = 1u32;
        loop {
            // Poll for a follow-up prompt (2 second intervals)
            match self.poll_for_prompt().await {
                Ok(Some(prompt)) => {
                    if prompt.done {
                        tracing::info!(job_id = %self.config.job_id, "Orchestrator signaled done");
                        break;
                    }
                    iteration += 1;
                    tracing::info!(
                        job_id = %self.config.job_id,
                        "Got follow-up prompt, resuming session"
                    );
                    if let Err(e) = self
                        .run_claude_session(&prompt.content, session_id.as_deref())
                        .await
                    {
                        tracing::error!(
                            job_id = %self.config.job_id,
                            "Follow-up Claude session failed: {}", e
                        );
                        // Don't fail the whole job on a follow-up error, just report it
                        self.report_event(
                            "status",
                            &serde_json::json!({
                                "message": format!("Follow-up session failed: {}", e),
                            }),
                        )
                        .await;
                    }
                }
                Ok(None) => {
                    // No prompt available, wait and poll again
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
                Err(e) => {
                    tracing::warn!(
                        job_id = %self.config.job_id,
                        "Prompt polling error: {}", e
                    );
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            }
        }

        self.client
            .report_complete(&CompletionReport {
                success: true,
                message: Some("Claude Code session completed".to_string()),
                iterations: iteration,
            })
            .await?;

        Ok(())
    }

    /// Spawn a `claude` CLI process and stream its output.
    ///
    /// Returns the session_id if captured from the `system` init message.
    async fn run_claude_session(
        &self,
        prompt: &str,
        resume_session_id: Option<&str>,
    ) -> Result<Option<String>, WorkerError> {
        let mut cmd = Command::new("claude");
        cmd.arg("-p")
            .arg(prompt)
            .arg("--output-format")
            .arg("stream-json")
            .arg("--verbose")
            .arg("--max-turns")
            .arg(self.config.max_turns.to_string())
            .arg("--model")
            .arg(&self.config.model);

        if let Some(sid) = resume_session_id {
            cmd.arg("--resume").arg(sid);
        }

        cmd.current_dir("/workspace")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        let mut child = cmd.spawn().map_err(|e| WorkerError::ExecutionFailed {
            reason: format!("failed to spawn claude: {}", e),
        })?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| WorkerError::ExecutionFailed {
                reason: "failed to capture claude stdout".to_string(),
            })?;

        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| WorkerError::ExecutionFailed {
                reason: "failed to capture claude stderr".to_string(),
            })?;

        // Spawn stderr reader that forwards lines as log events
        let client_for_stderr = Arc::clone(&self.client);
        let job_id = self.config.job_id;
        let stderr_handle = tokio::spawn(async move {
            let reader = BufReader::new(stderr);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                tracing::debug!(job_id = %job_id, "claude stderr: {}", line);
                let payload = JobEventPayload {
                    event_type: "status".to_string(),
                    data: serde_json::json!({ "message": line }),
                };
                client_for_stderr.post_event(&payload).await;
            }
        });

        // Read stdout NDJSON line by line
        let reader = BufReader::new(stdout);
        let mut lines = reader.lines();
        let mut session_id: Option<String> = None;

        while let Ok(Some(line)) = lines.next_line().await {
            let line = line.trim().to_string();
            if line.is_empty() {
                continue;
            }

            match serde_json::from_str::<ClaudeStreamEvent>(&line) {
                Ok(event) => {
                    // Capture session_id from system init
                    if event.event_type == "system"
                        && let Some(ref sid) = event.session_id
                    {
                        session_id = Some(sid.clone());
                        tracing::info!(
                            job_id = %self.config.job_id,
                            session_id = %sid,
                            "Captured Claude session ID"
                        );
                    }

                    // Convert to our event payload and forward
                    let payloads = stream_event_to_payloads(&event);
                    for payload in payloads {
                        self.report_event(&payload.event_type, &payload.data).await;
                    }
                }
                Err(e) => {
                    // Not valid JSON, forward as a status message
                    tracing::debug!(
                        job_id = %self.config.job_id,
                        "Non-JSON claude output: {} (parse error: {})", line, e
                    );
                    self.report_event("status", &serde_json::json!({ "message": line }))
                        .await;
                }
            }
        }

        // Wait for the process to exit
        let status = child
            .wait()
            .await
            .map_err(|e| WorkerError::ExecutionFailed {
                reason: format!("failed waiting for claude: {}", e),
            })?;

        // Wait for stderr reader to finish
        let _ = stderr_handle.await;

        if !status.success() {
            let code = status.code().unwrap_or(-1);
            tracing::warn!(
                job_id = %self.config.job_id,
                exit_code = code,
                "Claude process exited with non-zero status"
            );

            // Report result event
            self.report_event(
                "result",
                &serde_json::json!({
                    "status": "error",
                    "exit_code": code,
                    "session_id": session_id,
                }),
            )
            .await;

            return Err(WorkerError::ExecutionFailed {
                reason: format!("claude exited with code {}", code),
            });
        }

        // Report successful result
        self.report_event(
            "result",
            &serde_json::json!({
                "status": "completed",
                "session_id": session_id,
            }),
        )
        .await;

        Ok(session_id)
    }

    /// Post a job event to the orchestrator.
    async fn report_event(&self, event_type: &str, data: &serde_json::Value) {
        let payload = JobEventPayload {
            event_type: event_type.to_string(),
            data: data.clone(),
        };
        self.client.post_event(&payload).await;
    }

    /// Poll the orchestrator for a follow-up prompt.
    async fn poll_for_prompt(&self) -> Result<Option<PromptResponse>, WorkerError> {
        self.client.poll_prompt().await
    }
}

/// Build the JSON content for `.claude/settings.json` with the given tool allowlist.
///
/// Produces a Claude Code project settings file that auto-approves the listed
/// tools while leaving any unknown/future tools unapproved (defense-in-depth).
fn build_permission_settings(allowed_tools: &[String]) -> String {
    let settings = serde_json::json!({
        "permissions": {
            "allow": allowed_tools,
        }
    });
    serde_json::to_string_pretty(&settings).expect("static JSON structure is always valid")
}

/// Convert a Claude stream event into one or more event payloads for the orchestrator.
fn stream_event_to_payloads(event: &ClaudeStreamEvent) -> Vec<JobEventPayload> {
    let mut payloads = Vec::new();

    match event.event_type.as_str() {
        "system" => {
            payloads.push(JobEventPayload {
                event_type: "status".to_string(),
                data: serde_json::json!({
                    "message": "Claude Code session started",
                    "session_id": event.session_id,
                }),
            });
        }
        "assistant" => {
            // Extract text content and tool_use blocks
            if let Some(ref blocks) = event.content {
                for block in blocks {
                    match block.block_type.as_str() {
                        "text" => {
                            if let Some(ref text) = block.text {
                                payloads.push(JobEventPayload {
                                    event_type: "message".to_string(),
                                    data: serde_json::json!({
                                        "role": "assistant",
                                        "content": text,
                                    }),
                                });
                            }
                        }
                        "tool_use" => {
                            payloads.push(JobEventPayload {
                                event_type: "tool_use".to_string(),
                                data: serde_json::json!({
                                    "tool_name": block.name,
                                    "input": block.input,
                                }),
                            });
                        }
                        "tool_result" => {
                            payloads.push(JobEventPayload {
                                event_type: "tool_result".to_string(),
                                data: serde_json::json!({
                                    "tool_name": block.name.as_deref().unwrap_or("unknown"),
                                    "output": block.content.as_deref().unwrap_or(""),
                                }),
                            });
                        }
                        _ => {}
                    }
                }
            }
        }
        "result" => {
            let is_error = event
                .result
                .as_ref()
                .and_then(|r| r.is_error)
                .unwrap_or(false);
            payloads.push(JobEventPayload {
                event_type: "result".to_string(),
                data: serde_json::json!({
                    "status": if is_error { "error" } else { "completed" },
                    "session_id": event.session_id,
                    "duration_ms": event.result.as_ref().and_then(|r| r.duration_ms),
                    "num_turns": event.result.as_ref().and_then(|r| r.num_turns),
                }),
            });
        }
        _ => {
            // Forward unknown event types as status
            payloads.push(JobEventPayload {
                event_type: "status".to_string(),
                data: serde_json::json!({
                    "message": format!("Claude event: {}", event.event_type),
                    "raw_type": event.event_type,
                }),
            });
        }
    }

    payloads
}

/// Recursively copy files and directories from `src` to `dst`, skipping
/// entries that can't be read (e.g. permission-restricted files owned by a
/// different uid on a read-only bind mount). Returns the number of files
/// successfully copied.
fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<usize> {
    let entries = match std::fs::read_dir(src) {
        Ok(e) => e,
        Err(e) => {
            tracing::debug!("Skipping unreadable directory {}: {}", src.display(), e);
            return Ok(0);
        }
    };

    let mut copied = 0;
    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                tracing::debug!("Skipping unreadable entry in {}: {}", src.display(), e);
                continue;
            }
        };

        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if src_path.is_dir() {
            if std::fs::create_dir_all(&dst_path).is_ok() {
                copied += copy_dir_recursive(&src_path, &dst_path)?;
            }
        } else {
            match std::fs::copy(&src_path, &dst_path) {
                Ok(_) => copied += 1,
                Err(e) => {
                    tracing::debug!("Skipping unreadable file {}: {}", src_path.display(), e);
                }
            }
        }
    }
    Ok(copied)
}

fn truncate(s: &str, max_len: usize) -> &str {
    if s.len() <= max_len {
        s
    } else {
        // Walk back from max_len to find a valid UTF-8 char boundary.
        let mut end = max_len;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        &s[..end]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_system_event() {
        let json = r#"{"type":"system","session_id":"abc-123","subtype":"init"}"#;
        let event: ClaudeStreamEvent = serde_json::from_str(json).unwrap();
        assert_eq!(event.event_type, "system");
        assert_eq!(event.session_id.as_deref(), Some("abc-123"));
    }

    #[test]
    fn test_parse_assistant_text_event() {
        let json = r#"{"type":"assistant","content":[{"type":"text","text":"Hello world"}]}"#;
        let event: ClaudeStreamEvent = serde_json::from_str(json).unwrap();
        assert_eq!(event.event_type, "assistant");
        let blocks = event.content.unwrap();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].block_type, "text");
        assert_eq!(blocks[0].text.as_deref(), Some("Hello world"));
    }

    #[test]
    fn test_parse_assistant_tool_use_event() {
        let json = r#"{"type":"assistant","content":[{"type":"tool_use","name":"Bash","input":{"command":"ls"}}]}"#;
        let event: ClaudeStreamEvent = serde_json::from_str(json).unwrap();
        let blocks = event.content.unwrap();
        assert_eq!(blocks[0].block_type, "tool_use");
        assert_eq!(blocks[0].name.as_deref(), Some("Bash"));
        assert!(blocks[0].input.is_some());
    }

    #[test]
    fn test_parse_result_event() {
        let json =
            r#"{"type":"result","result":{"is_error":false,"duration_ms":5000,"num_turns":3}}"#;
        let event: ClaudeStreamEvent = serde_json::from_str(json).unwrap();
        assert_eq!(event.event_type, "result");
        let result = event.result.unwrap();
        assert_eq!(result.is_error, Some(false));
        assert_eq!(result.duration_ms, Some(5000));
        assert_eq!(result.num_turns, Some(3));
    }

    #[test]
    fn test_parse_result_error_event() {
        let json = r#"{"type":"result","result":{"is_error":true}}"#;
        let event: ClaudeStreamEvent = serde_json::from_str(json).unwrap();
        let result = event.result.unwrap();
        assert_eq!(result.is_error, Some(true));
    }

    #[test]
    fn test_stream_event_to_payloads_system() {
        let event = ClaudeStreamEvent {
            event_type: "system".to_string(),
            session_id: Some("sid-123".to_string()),
            content: None,
            result: None,
            tool_name: None,
            input: None,
            output: None,
            subtype: None,
        };
        let payloads = stream_event_to_payloads(&event);
        assert_eq!(payloads.len(), 1);
        assert_eq!(payloads[0].event_type, "status");
        assert_eq!(payloads[0].data["session_id"], "sid-123");
    }

    #[test]
    fn test_stream_event_to_payloads_assistant_text() {
        let event = ClaudeStreamEvent {
            event_type: "assistant".to_string(),
            session_id: None,
            content: Some(vec![ContentBlock {
                block_type: "text".to_string(),
                text: Some("Here's the answer".to_string()),
                name: None,
                input: None,
                content: None,
            }]),
            result: None,
            tool_name: None,
            input: None,
            output: None,
            subtype: None,
        };
        let payloads = stream_event_to_payloads(&event);
        assert_eq!(payloads.len(), 1);
        assert_eq!(payloads[0].event_type, "message");
        assert_eq!(payloads[0].data["role"], "assistant");
        assert_eq!(payloads[0].data["content"], "Here's the answer");
    }

    #[test]
    fn test_stream_event_to_payloads_result_success() {
        let event = ClaudeStreamEvent {
            event_type: "result".to_string(),
            session_id: Some("s1".to_string()),
            content: None,
            result: Some(ResultInfo {
                is_error: Some(false),
                duration_ms: Some(12000),
                num_turns: Some(5),
            }),
            tool_name: None,
            input: None,
            output: None,
            subtype: None,
        };
        let payloads = stream_event_to_payloads(&event);
        assert_eq!(payloads.len(), 1);
        assert_eq!(payloads[0].event_type, "result");
        assert_eq!(payloads[0].data["status"], "completed");
    }

    #[test]
    fn test_stream_event_to_payloads_result_error() {
        let event = ClaudeStreamEvent {
            event_type: "result".to_string(),
            session_id: None,
            content: None,
            result: Some(ResultInfo {
                is_error: Some(true),
                duration_ms: None,
                num_turns: None,
            }),
            tool_name: None,
            input: None,
            output: None,
            subtype: None,
        };
        let payloads = stream_event_to_payloads(&event);
        assert_eq!(payloads[0].data["status"], "error");
    }

    #[test]
    fn test_stream_event_to_payloads_unknown_type() {
        let event = ClaudeStreamEvent {
            event_type: "fancy_new_thing".to_string(),
            session_id: None,
            content: None,
            result: None,
            tool_name: None,
            input: None,
            output: None,
            subtype: None,
        };
        let payloads = stream_event_to_payloads(&event);
        assert_eq!(payloads.len(), 1);
        assert_eq!(payloads[0].event_type, "status");
    }

    #[test]
    fn test_claude_event_payload_serde() {
        let payload = JobEventPayload {
            event_type: "message".to_string(),
            data: serde_json::json!({ "role": "assistant", "content": "hi" }),
        };
        let json = serde_json::to_string(&payload).unwrap();
        let parsed: JobEventPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.event_type, "message");
        assert_eq!(parsed.data["content"], "hi");
    }

    #[test]
    fn test_truncate() {
        assert_eq!(truncate("hello", 10), "hello");
        assert_eq!(truncate("hello world", 5), "hello");
        assert_eq!(truncate("", 5), "");
    }

    #[test]
    fn test_build_permission_settings_default_tools() {
        let tools: Vec<String> = ["Bash(*)", "Read", "Edit(*)", "Glob", "Grep"]
            .into_iter()
            .map(String::from)
            .collect();
        let json_str = build_permission_settings(&tools);
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        let allow = parsed["permissions"]["allow"].as_array().unwrap();
        assert_eq!(allow.len(), 5);
        assert_eq!(allow[0], "Bash(*)");
        assert_eq!(allow[1], "Read");
        assert_eq!(allow[2], "Edit(*)");
    }

    #[test]
    fn test_build_permission_settings_empty_tools() {
        let json_str = build_permission_settings(&[]);
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        let allow = parsed["permissions"]["allow"].as_array().unwrap();
        assert!(allow.is_empty());
    }

    #[test]
    fn test_build_permission_settings_is_valid_json() {
        let tools = vec!["Bash(npm run *)".to_string(), "Read".to_string()];
        let json_str = build_permission_settings(&tools);
        // Must be valid JSON
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
        // Must have the expected structure
        assert!(parsed["permissions"].is_object());
        assert!(parsed["permissions"]["allow"].is_array());
    }

    #[test]
    fn test_copy_dir_recursive() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();

        // Create a nested structure in src
        std::fs::write(src.path().join("auth.json"), r#"{"token":"abc"}"#).unwrap();
        std::fs::create_dir_all(src.path().join("subdir")).unwrap();
        std::fs::write(src.path().join("subdir").join("nested.txt"), "nested").unwrap();

        let copied = copy_dir_recursive(src.path(), dst.path()).unwrap();
        assert_eq!(copied, 2);

        // Verify files were copied
        assert_eq!(
            std::fs::read_to_string(dst.path().join("auth.json")).unwrap(),
            r#"{"token":"abc"}"#
        );
        assert_eq!(
            std::fs::read_to_string(dst.path().join("subdir").join("nested.txt")).unwrap(),
            "nested"
        );
    }

    #[test]
    fn test_copy_dir_recursive_empty_source() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();

        let copied = copy_dir_recursive(src.path(), dst.path()).unwrap();
        assert_eq!(copied, 0);
    }

    #[test]
    fn test_copy_dir_recursive_skips_nonexistent_source() {
        let dst = tempfile::tempdir().unwrap();
        let nonexistent = std::path::Path::new("/no/such/path");

        // Should gracefully return 0 instead of failing
        let copied = copy_dir_recursive(nonexistent, dst.path()).unwrap();
        assert_eq!(copied, 0);
    }
}
