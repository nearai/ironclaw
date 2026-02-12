//! Claude Code bridge for sandboxed execution.
//!
//! Spawns the `claude` CLI inside a Docker container and streams its NDJSON
//! output back to the orchestrator via HTTP. Supports follow-up prompts via
//! `--resume`.
//!
//! ```text
//! ┌─────────────────────────────────────────────┐
//! │ Docker Container                             │
//! │                                              │
//! │  ironclaw claude-bridge --job-id <uuid>      │
//! │    └─ claude -p "task" --output-format        │
//! │       stream-json --dangerously-skip-perms   │
//! │    └─ reads stdout line-by-line              │
//! │    └─ POSTs events to orchestrator           │
//! │    └─ polls for follow-up prompts            │
//! │    └─ on follow-up: claude --resume          │
//! └─────────────────────────────────────────────┘
//! ```

use std::sync::Arc;
use std::time::Duration;

use uuid::Uuid;

use crate::claude_cli::{ClaudeCodeConfig, ClaudeProcess, CliEvent, ContentBlock};
use crate::error::WorkerError;
use crate::worker::api::{CompletionReport, JobEventPayload, PromptResponse, WorkerHttpClient};

/// Configuration for the Claude bridge runtime.
pub struct ClaudeBridgeConfig {
    pub job_id: Uuid,
    pub orchestrator_url: String,
    pub max_turns: u32,
    pub model: String,
    pub timeout: Duration,
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

    /// Build a `ClaudeCodeConfig` from the bridge settings.
    fn claude_config(&self) -> ClaudeCodeConfig {
        ClaudeCodeConfig::new()
            .model(&self.config.model)
            .max_turns(self.config.max_turns)
            .cwd("/workspace")
            .dangerously_skip_permissions()
    }

    /// Run the bridge: fetch job, spawn claude, stream events, handle follow-ups.
    pub async fn run(&self) -> Result<(), WorkerError> {
        // Fetch the job description from the orchestrator
        let job = self.client.get_job().await?;

        tracing::info!(
            job_id = %self.config.job_id,
            "Starting Claude Code bridge for: {}",
            truncate(&job.description, 100)
        );

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
        let config = self.claude_config();

        let mut process = if let Some(sid) = resume_session_id {
            ClaudeProcess::spawn_resume(&config, sid, prompt)
        } else {
            ClaudeProcess::spawn_oneshot(&config, prompt)
        }
        .map_err(|e| WorkerError::ExecutionFailed {
            reason: format!("failed to spawn claude: {}", e),
        })?;

        let mut session_id: Option<String> = None;

        while let Some(event) = process.next_event().await {
            // Capture session_id from system init
            if let Some(sid) = event.session_id() {
                if session_id.is_none() {
                    session_id = Some(sid.to_string());
                    tracing::info!(
                        job_id = %self.config.job_id,
                        session_id = %sid,
                        "Captured Claude session ID"
                    );
                }
            }

            // Convert to event payloads and forward to orchestrator
            let payloads = event_to_payloads(&event);
            for payload in payloads {
                self.report_event(&payload.event_type, &payload.data).await;
            }
        }

        // Wait for process exit and report final status
        match process.wait().await {
            Ok(()) => {
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
            Err(crate::claude_cli::ClaudeCliError::ProcessFailed { code, .. }) => {
                tracing::warn!(
                    job_id = %self.config.job_id,
                    exit_code = code,
                    "Claude process exited with non-zero status"
                );
                self.report_event(
                    "result",
                    &serde_json::json!({
                        "status": "error",
                        "exit_code": code,
                        "session_id": session_id,
                    }),
                )
                .await;
                Err(WorkerError::ExecutionFailed {
                    reason: format!("claude exited with code {}", code),
                })
            }
            Err(e) => Err(WorkerError::ExecutionFailed {
                reason: format!("claude process error: {}", e),
            }),
        }
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

/// Convert a typed `CliEvent` into one or more event payloads for the orchestrator.
fn event_to_payloads(event: &CliEvent) -> Vec<JobEventPayload> {
    match event {
        CliEvent::System(sys) => {
            vec![JobEventPayload {
                event_type: "status".to_string(),
                data: serde_json::json!({
                    "message": "Claude Code session started",
                    "session_id": sys.session_id,
                }),
            }]
        }
        CliEvent::Assistant(asst) => {
            let mut payloads = Vec::new();
            for block in &asst.content {
                match block {
                    ContentBlock::Text { text } => {
                        payloads.push(JobEventPayload {
                            event_type: "message".to_string(),
                            data: serde_json::json!({
                                "role": "assistant",
                                "content": text,
                            }),
                        });
                    }
                    ContentBlock::ToolUse { name, input, .. } => {
                        payloads.push(JobEventPayload {
                            event_type: "tool_use".to_string(),
                            data: serde_json::json!({
                                "tool_name": name,
                                "input": input,
                            }),
                        });
                    }
                    ContentBlock::ToolResult { content, .. } => {
                        payloads.push(JobEventPayload {
                            event_type: "tool_result".to_string(),
                            data: serde_json::json!({
                                "tool_name": "unknown",
                                "output": content.as_deref().unwrap_or(""),
                            }),
                        });
                    }
                }
            }
            payloads
        }
        CliEvent::Result(res) => {
            let is_error = res
                .is_error
                .unwrap_or(false)
                || res
                    .result
                    .as_ref()
                    .and_then(|r| r.is_error)
                    .unwrap_or(false);
            vec![JobEventPayload {
                event_type: "result".to_string(),
                data: serde_json::json!({
                    "status": if is_error { "error" } else { "completed" },
                    "session_id": res.session_id,
                    "duration_ms": res.result.as_ref().and_then(|r| r.duration_ms),
                    "num_turns": res.result.as_ref().and_then(|r| r.num_turns),
                }),
            }]
        }
        CliEvent::User(_) => Vec::new(),
        CliEvent::Unknown { event_type, .. } => {
            vec![JobEventPayload {
                event_type: "status".to_string(),
                data: serde_json::json!({
                    "message": format!("Claude event: {}", event_type),
                    "raw_type": event_type,
                }),
            }]
        }
    }
}

fn truncate(s: &str, max_len: usize) -> &str {
    if s.len() <= max_len {
        s
    } else {
        &s[..max_len]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claude_cli::types::*;

    #[test]
    fn test_event_to_payloads_system() {
        let event = CliEvent::System(SystemEvent {
            session_id: Some("sid-123".to_string()),
            subtype: Some("init".to_string()),
            model: None,
            tools: None,
        });
        let payloads = event_to_payloads(&event);
        assert_eq!(payloads.len(), 1);
        assert_eq!(payloads[0].event_type, "status");
        assert_eq!(payloads[0].data["session_id"], "sid-123");
    }

    #[test]
    fn test_event_to_payloads_assistant_text() {
        let event = CliEvent::Assistant(AssistantEvent {
            content: vec![ContentBlock::Text {
                text: "Here's the answer".to_string(),
            }],
            session_id: None,
        });
        let payloads = event_to_payloads(&event);
        assert_eq!(payloads.len(), 1);
        assert_eq!(payloads[0].event_type, "message");
        assert_eq!(payloads[0].data["role"], "assistant");
        assert_eq!(payloads[0].data["content"], "Here's the answer");
    }

    #[test]
    fn test_event_to_payloads_assistant_tool_use() {
        let event = CliEvent::Assistant(AssistantEvent {
            content: vec![ContentBlock::ToolUse {
                id: Some("tu_1".to_string()),
                name: "Bash".to_string(),
                input: serde_json::json!({"command": "ls"}),
            }],
            session_id: None,
        });
        let payloads = event_to_payloads(&event);
        assert_eq!(payloads.len(), 1);
        assert_eq!(payloads[0].event_type, "tool_use");
        assert_eq!(payloads[0].data["tool_name"], "Bash");
    }

    #[test]
    fn test_event_to_payloads_result_success() {
        let event = CliEvent::Result(ResultEvent {
            session_id: Some("s1".to_string()),
            subtype: None,
            result: Some(ResultInfo {
                is_error: Some(false),
                duration_ms: Some(12000),
                duration_api_ms: None,
                num_turns: Some(5),
                cost_usd: None,
                input_tokens: None,
                output_tokens: None,
            }),
            is_error: None,
        });
        let payloads = event_to_payloads(&event);
        assert_eq!(payloads.len(), 1);
        assert_eq!(payloads[0].event_type, "result");
        assert_eq!(payloads[0].data["status"], "completed");
    }

    #[test]
    fn test_event_to_payloads_result_error() {
        let event = CliEvent::Result(ResultEvent {
            session_id: None,
            subtype: None,
            result: Some(ResultInfo {
                is_error: Some(true),
                duration_ms: None,
                duration_api_ms: None,
                num_turns: None,
                cost_usd: None,
                input_tokens: None,
                output_tokens: None,
            }),
            is_error: None,
        });
        let payloads = event_to_payloads(&event);
        assert_eq!(payloads[0].data["status"], "error");
    }

    #[test]
    fn test_event_to_payloads_unknown_type() {
        let event = CliEvent::Unknown {
            event_type: "fancy_new_thing".to_string(),
            raw: serde_json::json!({}),
        };
        let payloads = event_to_payloads(&event);
        assert_eq!(payloads.len(), 1);
        assert_eq!(payloads[0].event_type, "status");
    }

    #[test]
    fn test_event_to_payloads_user_empty() {
        let event = CliEvent::User(UserEvent {
            content: vec![],
            session_id: None,
        });
        let payloads = event_to_payloads(&event);
        assert!(payloads.is_empty());
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
}
