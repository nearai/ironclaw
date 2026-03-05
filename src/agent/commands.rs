//! System commands and job handlers for the agent.
//!
//! Extracted from `agent_loop.rs` to isolate the /help, /model, /status,
//! and other command processing from the core agent loop.

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use serde_json::json;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::agent::session::Session;
use crate::agent::submission::SubmissionResult;
use crate::agent::{Agent, MessageIntent};
use crate::channels::{IncomingMessage, StatusUpdate};
use crate::context::{JobContext, JobState};
use crate::error::Error;
use crate::llm::{ChatMessage, Reasoning};
use crate::tools::ToolError;

#[derive(Debug, Clone)]
struct MentorContext {
    session_id: String,
    voice_mode: bool,
    voice_mode_changed: Option<String>,
    voice_transcript: Option<String>,
    voice_transcription_error: Option<String>,
}

/// Format a count with a suffix, using K/M abbreviations for large numbers.
fn format_count(n: u64, suffix: &str) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M {}", n as f64 / 1_000_000.0, suffix)
    } else if n >= 1_000 {
        format!("{:.1}K {}", n as f64 / 1_000.0, suffix)
    } else {
        format!("{} {}", n, suffix)
    }
}

fn mentor_error_summary(error: &str) -> String {
    let detail = error.replace('\n', " ");
    let detail = detail.trim();
    let detail = if detail.len() > 220 {
        format!("{}...", &detail[..220])
    } else {
        detail.to_string()
    };

    let lower = error.to_lowercase();
    if lower.contains("voice_policy_error")
        || lower.contains("forwarded_voice_blocked")
        || lower.contains("forwarded_bot_voice_blocked")
    {
        format!(
            "voice_policy_error: Forwarded voice notes are blocked by safety policy. Send a direct voice note or allow forwarded voice explicitly. cause={detail}"
        )
    } else if lower.contains("timeout") || lower.contains("timed out") {
        format!(
            "mcp_transport_error: Mentor MCP transport timeout. Text fallback is active. cause={detail}"
        )
    } else if lower.contains("invalid params") || lower.contains("schema") {
        format!(
            "mcp_schema_error: Mentor tool schema error. Check mentor MCP argument contract. cause={detail}"
        )
    } else if lower.contains("transcribe")
        || lower.contains("whisper")
        || lower.contains("asr")
        || lower.contains("stt")
    {
        format!(
            "stt_backend_error: Voice STT backend error while processing the voice note. cause={detail}"
        )
    } else if lower.contains("speak")
        || lower.contains("voice")
        || lower.contains("tts")
        || lower.contains("fish")
        || lower.contains("kokoro")
        || lower.contains("csm")
    {
        format!(
            "tts_backend_error: Voice TTS backend error. Text fallback is active. cause={detail}"
        )
    } else if lower.contains("telegram") || lower.contains("sendvoice") {
        format!(
            "telegram_api_error: Telegram delivery error while sending voice response. cause={detail}"
        )
    } else if lower.contains("novita")
        || lower.contains("image")
        || lower.contains("flux")
        || lower.contains("txt2img")
        || lower.contains("video")
        || lower.contains("t2v")
        || lower.contains("mochi")
        || lower.contains("seedance")
    {
        format!("media_backend_error: Mentor image/video backend error. cause={detail}")
    } else {
        format!("Mentor error: {}", error)
    }
}

fn parse_tool_json_output(raw: &str) -> Result<serde_json::Value, String> {
    let parsed: serde_json::Value =
        serde_json::from_str(raw).map_err(|e| format!("Invalid tool JSON: {}", e))?;

    if let Some(inner) = parsed.as_str() {
        if let Ok(inner_json) = serde_json::from_str::<serde_json::Value>(inner) {
            return Ok(inner_json);
        }
        return Ok(json!({ "text": inner }));
    }

    Ok(parsed)
}

fn map_mentor_tool_error(tool_name: &str, error: ToolError) -> String {
    match error {
        ToolError::InvalidParameters(reason) => {
            format!("Mentor tool schema error ({tool_name}): {reason}")
        }
        ToolError::ExecutionFailed(reason) => reason,
        ToolError::Timeout(timeout) => {
            format!(
                "Mentor MCP transport timeout ({tool_name}) after {}s",
                timeout.as_secs()
            )
        }
        ToolError::ExternalService(reason) => reason,
        ToolError::NotAuthorized(reason) => {
            format!("Mentor tool not authorized ({tool_name}): {reason}")
        }
        ToolError::RateLimited(retry_after) => match retry_after {
            Some(delay) => format!(
                "Mentor tool rate limited ({tool_name}), retry in {}s",
                delay.as_secs()
            ),
            None => format!("Mentor tool rate limited ({tool_name})"),
        },
        ToolError::Sandbox(reason) => format!("Mentor tool sandbox error ({tool_name}): {reason}"),
    }
}

fn mentor_context_from_message(message: Option<&IncomingMessage>) -> MentorContext {
    let mut ctx = MentorContext {
        session_id: "default".to_string(),
        voice_mode: false,
        voice_mode_changed: None,
        voice_transcript: None,
        voice_transcription_error: None,
    };

    let Some(message) = message else {
        return ctx;
    };

    if let Some(chat_id) = message
        .metadata
        .get("chat_id")
        .and_then(|value| value.as_i64())
    {
        ctx.session_id = format!("telegram:{chat_id}");
    } else if let Some(thread_id) = message.thread_id.as_deref() {
        ctx.session_id = thread_id.to_string();
    } else {
        ctx.session_id = format!("{}:{}", message.channel, message.user_id);
    }

    ctx.voice_mode = message
        .metadata
        .get("mentor_voice_mode")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);

    ctx.voice_mode_changed = message
        .metadata
        .get("mentor_voice_mode_changed")
        .and_then(|value| value.as_str())
        .map(|value| value.to_string());

    ctx.voice_transcript = message
        .metadata
        .get("mentor_voice_transcript")
        .and_then(|value| value.as_str())
        .map(|value| value.to_string())
        .or_else(|| {
            message
                .metadata
                .get("voice_transcript")
                .and_then(|value| value.as_str())
                .map(|value| value.to_string())
        });

    ctx.voice_transcription_error = message
        .metadata
        .get("mentor_voice_transcription_error")
        .and_then(|value| value.as_str())
        .map(|value| value.to_string())
        .or_else(|| {
            message
                .metadata
                .get("voice_transcription_error")
                .and_then(|value| value.as_str())
                .map(|value| value.to_string())
        });

    ctx
}

fn extract_mentor_reply(payload: &serde_json::Value) -> String {
    let raw = payload
        .get("reply")
        .and_then(|value| value.as_str())
        .or_else(|| payload.get("text").and_then(|value| value.as_str()))
        .or_else(|| payload.get("content").and_then(|value| value.as_str()))
        .unwrap_or_default();

    let stripped = strip_minimax_tool_calls(raw);
    let trimmed = stripped.trim();
    if trimmed.is_empty() {
        "Mentor returned no usable reply. Please retry.".to_string()
    } else {
        trimmed.to_string()
    }
}

fn strip_minimax_tool_calls(input: &str) -> String {
    const TOOL_CALL_OPEN: &str = "<minimax:tool_call>";
    const TOOL_CALL_CLOSE: &str = "</minimax:tool_call>";
    const TOOLCALL_OPEN: &str = "<minimax:toolcall>";
    const TOOLCALL_CLOSE: &str = "</minimax:toolcall>";

    fn remove_block(source: &str, open: &str, close: &str) -> String {
        let mut remaining = source;
        let mut output = String::with_capacity(source.len());

        loop {
            let Some(start) = remaining.find(open) else {
                output.push_str(remaining);
                break;
            };

            output.push_str(&remaining[..start]);
            let block = &remaining[start..];
            if let Some(close_idx) = block.find(close) {
                let after = close_idx + close.len();
                remaining = &block[after..];
            } else {
                // Drop trailing unmatched tool-call block.
                break;
            }
        }

        output
    }

    let without_tool_call = remove_block(input, TOOL_CALL_OPEN, TOOL_CALL_CLOSE);
    remove_block(&without_tool_call, TOOLCALL_OPEN, TOOLCALL_CLOSE)
}

fn enforce_voice_reply_style(reply: &str) -> String {
    let trimmed = reply.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    let mut sentence_count = 0usize;
    let mut limited = String::new();
    for ch in trimmed.chars() {
        limited.push(ch);
        if matches!(ch, '.' | '!' | '?') {
            sentence_count += 1;
            if sentence_count >= 3 {
                break;
            }
        }
        if limited.chars().count() >= 260 {
            break;
        }
    }

    if limited.trim().is_empty() {
        return trimmed.chars().take(260).collect();
    }

    if trimmed.chars().count() > limited.chars().count() {
        let clipped: String = limited.chars().take(257).collect();
        return format!("{clipped}...");
    }

    limited.trim().to_string()
}

fn audio_mime_type_for_path(path: &str) -> &'static str {
    let extension = Path::new(path)
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    match extension.as_str() {
        "wav" => "audio/wav",
        "ogg" => "audio/ogg",
        "m4a" | "mp4" => "audio/mp4",
        "webm" => "audio/webm",
        _ => "audio/mpeg",
    }
}

fn load_audio_attachment_data_url(path: &str) -> Result<String, String> {
    if path.starts_with("data:audio/") {
        return Ok(path.to_string());
    }

    let bytes =
        fs::read(path).map_err(|err| format!("Failed reading mentor audio artifact: {err}"))?;
    if bytes.is_empty() {
        return Err("Mentor audio artifact is empty".to_string());
    }
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    let mime = audio_mime_type_for_path(path);
    Ok(format!("data:{mime};base64,{encoded}"))
}

fn image_mime_type_for_path(path: &str) -> &'static str {
    let extension = Path::new(path)
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    match extension.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        _ => "image/png",
    }
}

fn load_image_attachment_data_url(path: &str) -> Result<String, String> {
    if path.starts_with("data:image/") {
        return Ok(path.to_string());
    }

    let bytes =
        fs::read(path).map_err(|err| format!("Failed reading mentor image artifact: {err}"))?;
    if bytes.is_empty() {
        return Err("Mentor image artifact is empty".to_string());
    }

    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    let mime = image_mime_type_for_path(path);
    Ok(format!("data:{mime};base64,{encoded}"))
}

fn video_mime_type_for_path(path: &str) -> &'static str {
    let extension = Path::new(path)
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    match extension.as_str() {
        "webm" => "video/webm",
        "mov" => "video/quicktime",
        _ => "video/mp4",
    }
}

fn load_video_attachment_data_url(path: &str) -> Result<String, String> {
    if path.starts_with("data:video/") {
        return Ok(path.to_string());
    }

    let bytes =
        fs::read(path).map_err(|err| format!("Failed reading mentor video artifact: {err}"))?;
    if bytes.is_empty() {
        return Err("Mentor video artifact is empty".to_string());
    }

    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    let mime = video_mime_type_for_path(path);
    Ok(format!("data:{mime};base64,{encoded}"))
}

impl Agent {
    /// Handle job-related intents without turn tracking.
    pub(super) async fn handle_job_or_command(
        &self,
        intent: MessageIntent,
        message: &IncomingMessage,
    ) -> Result<SubmissionResult, Error> {
        // Send thinking status for non-trivial operations
        if let MessageIntent::CreateJob { .. } = &intent {
            let _ = self
                .channels
                .send_status(
                    &message.channel,
                    StatusUpdate::Thinking("Processing...".into()),
                    &message.metadata,
                )
                .await;
        }

        let response = match intent {
            MessageIntent::CreateJob {
                title,
                description,
                category,
            } => {
                self.handle_create_job(&message.user_id, title, description, category)
                    .await?
            }
            MessageIntent::CheckJobStatus { job_id } => {
                self.handle_check_status(&message.user_id, job_id).await?
            }
            MessageIntent::CancelJob { job_id } => {
                self.handle_cancel_job(&message.user_id, &job_id).await?
            }
            MessageIntent::ListJobs { filter } => {
                self.handle_list_jobs(&message.user_id, filter).await?
            }
            MessageIntent::HelpJob { job_id } => {
                self.handle_help_job(&message.user_id, &job_id).await?
            }
            MessageIntent::Command { command, args } => {
                return self
                    .handle_system_command(&command, &args, Some(message))
                    .await;
            }
            _ => "Unknown intent".to_string(),
        };
        Ok(SubmissionResult::response(response))
    }

    async fn handle_create_job(
        &self,
        user_id: &str,
        title: String,
        description: String,
        category: Option<String>,
    ) -> Result<String, Error> {
        let job_id = self
            .scheduler
            .dispatch_job(user_id, &title, &description, None)
            .await?;

        // Set the dedicated category field (not stored in metadata)
        if let Some(cat) = category
            && let Err(e) = self
                .context_manager
                .update_context(job_id, |ctx| {
                    ctx.category = Some(cat);
                })
                .await
        {
            tracing::warn!(job_id = %job_id, "Failed to set job category: {}", e);
        }

        Ok(format!(
            "Created job: {}\nID: {}\n\nThe job has been scheduled and is now running.",
            title, job_id
        ))
    }

    async fn handle_check_status(
        &self,
        user_id: &str,
        job_id: Option<String>,
    ) -> Result<String, Error> {
        match job_id {
            Some(id) => {
                let uuid = Uuid::parse_str(&id)
                    .map_err(|_| crate::error::JobError::NotFound { id: Uuid::nil() })?;

                // Try DB first for persistent state, fall back to ContextManager.
                if let Some(store) = self.store()
                    && let Ok(Some(ctx)) = store.get_job(uuid).await
                {
                    return Ok(format!(
                        "Job: {}\nStatus: {:?}\nCreated: {}\nStarted: {}\nActual cost: {}",
                        ctx.title,
                        ctx.state,
                        ctx.created_at.format("%Y-%m-%d %H:%M:%S"),
                        ctx.started_at
                            .map(|t| t.format("%Y-%m-%d %H:%M:%S").to_string())
                            .unwrap_or_else(|| "Not started".to_string()),
                        ctx.actual_cost
                    ));
                }

                let ctx = self.context_manager.get_context(uuid).await?;
                if ctx.user_id != user_id {
                    return Err(crate::error::JobError::NotFound { id: uuid }.into());
                }

                Ok(format!(
                    "Job: {}\nStatus: {:?}\nCreated: {}\nStarted: {}\nActual cost: {}",
                    ctx.title,
                    ctx.state,
                    ctx.created_at.format("%Y-%m-%d %H:%M:%S"),
                    ctx.started_at
                        .map(|t| t.format("%Y-%m-%d %H:%M:%S").to_string())
                        .unwrap_or_else(|| "Not started".to_string()),
                    ctx.actual_cost
                ))
            }
            None => {
                // Show summary from DB for consistency with Jobs tab.
                if let Some(store) = self.store() {
                    let mut total = 0;
                    let mut in_progress = 0;
                    let mut completed = 0;
                    let mut failed = 0;
                    let mut stuck = 0;

                    if let Ok(s) = store.agent_job_summary().await {
                        total += s.total;
                        in_progress += s.in_progress;
                        completed += s.completed;
                        failed += s.failed;
                        stuck += s.stuck;
                    }
                    if let Ok(s) = store.sandbox_job_summary().await {
                        total += s.total;
                        in_progress += s.running;
                        completed += s.completed;
                        failed += s.failed + s.interrupted;
                    }

                    return Ok(format!(
                        "Jobs summary: Total: {} In Progress: {} Completed: {} Failed: {} Stuck: {}",
                        total, in_progress, completed, failed, stuck
                    ));
                }

                // Fallback to ContextManager if no DB.
                let summary = self.context_manager.summary_for(user_id).await;
                Ok(format!(
                    "Jobs summary: Total: {} In Progress: {} Completed: {} Failed: {} Stuck: {}",
                    summary.total,
                    summary.in_progress,
                    summary.completed,
                    summary.failed,
                    summary.stuck
                ))
            }
        }
    }

    async fn handle_cancel_job(&self, user_id: &str, job_id: &str) -> Result<String, Error> {
        let uuid = Uuid::parse_str(job_id)
            .map_err(|_| crate::error::JobError::NotFound { id: Uuid::nil() })?;

        let ctx = self.context_manager.get_context(uuid).await?;
        if ctx.user_id != user_id {
            return Err(crate::error::JobError::NotFound { id: uuid }.into());
        }

        self.scheduler.stop(uuid).await?;

        // Also update DB so the Jobs tab reflects cancellation immediately.
        if let Some(store) = self.store()
            && let Err(e) = store
                .update_job_status(uuid, JobState::Cancelled, Some("Cancelled by user"))
                .await
        {
            tracing::warn!(job_id = %uuid, "Failed to persist cancellation to DB: {}", e);
        }

        Ok(format!("Job {} has been cancelled.", job_id))
    }

    async fn handle_list_jobs(
        &self,
        user_id: &str,
        _filter: Option<String>,
    ) -> Result<String, Error> {
        // List from DB for consistency with Jobs tab.
        if let Some(store) = self.store() {
            let agent_jobs = match store.list_agent_jobs().await {
                Ok(jobs) => jobs,
                Err(e) => {
                    tracing::warn!("Failed to list agent jobs: {}", e);
                    Vec::new()
                }
            };
            let sandbox_jobs = match store.list_sandbox_jobs().await {
                Ok(jobs) => jobs,
                Err(e) => {
                    tracing::warn!("Failed to list sandbox jobs: {}", e);
                    Vec::new()
                }
            };

            if agent_jobs.is_empty() && sandbox_jobs.is_empty() {
                return Ok("No jobs found.".to_string());
            }

            let mut output = String::from("Jobs:\n");
            for j in &agent_jobs {
                output.push_str(&format!("  {} - {} ({})\n", j.id, j.title, j.status));
            }
            for j in &sandbox_jobs {
                output.push_str(&format!("  {} - {} ({})\n", j.id, j.task, j.status));
            }
            return Ok(output);
        }

        // Fallback to ContextManager if no DB.
        let jobs = self.context_manager.all_jobs_for(user_id).await;
        if jobs.is_empty() {
            return Ok("No jobs found.".to_string());
        }

        let mut output = String::from("Jobs:\n");
        for job_id in jobs {
            if let Ok(ctx) = self.context_manager.get_context(job_id).await {
                output.push_str(&format!("  {} - {} ({:?})\n", job_id, ctx.title, ctx.state));
            }
        }
        Ok(output)
    }

    async fn handle_help_job(&self, user_id: &str, job_id: &str) -> Result<String, Error> {
        let uuid = Uuid::parse_str(job_id)
            .map_err(|_| crate::error::JobError::NotFound { id: Uuid::nil() })?;

        let ctx = self.context_manager.get_context(uuid).await?;
        if ctx.user_id != user_id {
            return Err(crate::error::JobError::NotFound { id: uuid }.into());
        }

        if ctx.state == crate::context::JobState::Stuck {
            // Attempt recovery
            self.context_manager
                .update_context(uuid, |ctx| ctx.attempt_recovery())
                .await?
                .map_err(|s| crate::error::JobError::ContextError {
                    id: uuid,
                    reason: s,
                })?;

            // Reschedule
            self.scheduler.schedule(uuid).await?;

            Ok(format!(
                "Job {} was stuck. Attempting recovery (attempt #{}).",
                job_id,
                ctx.repair_attempts + 1
            ))
        } else {
            Ok(format!(
                "Job {} is not stuck (current state: {:?}). No help needed.",
                job_id, ctx.state
            ))
        }
    }

    /// Show job status inline — either all jobs (no id) or a specific job.
    pub(super) async fn process_job_status(
        &self,
        user_id: &str,
        job_id: Option<&str>,
    ) -> Result<SubmissionResult, Error> {
        match self
            .handle_check_status(user_id, job_id.map(|s| s.to_string()))
            .await
        {
            Ok(text) => Ok(SubmissionResult::response(text)),
            Err(e) => Ok(SubmissionResult::error(format!("Job status error: {}", e))),
        }
    }

    /// Cancel a job by ID.
    pub(super) async fn process_job_cancel(
        &self,
        user_id: &str,
        job_id: &str,
    ) -> Result<SubmissionResult, Error> {
        match self.handle_cancel_job(user_id, job_id).await {
            Ok(text) => Ok(SubmissionResult::response(text)),
            Err(e) => Ok(SubmissionResult::error(format!("Cancel error: {}", e))),
        }
    }

    /// Trigger a manual heartbeat check.
    pub(super) async fn process_heartbeat(&self) -> Result<SubmissionResult, Error> {
        let Some(workspace) = self.workspace() else {
            return Ok(SubmissionResult::error(
                "Heartbeat requires a workspace (database must be connected).",
            ));
        };

        let runner = crate::agent::HeartbeatRunner::new(
            crate::agent::HeartbeatConfig::default(),
            crate::workspace::hygiene::HygieneConfig::default(),
            workspace.clone(),
            self.llm().clone(),
            self.safety().clone(),
        );

        match runner.check_heartbeat().await {
            crate::agent::HeartbeatResult::Ok => Ok(SubmissionResult::ok_with_message(
                "Heartbeat: all clear, nothing needs attention.",
            )),
            crate::agent::HeartbeatResult::NeedsAttention(msg) => Ok(SubmissionResult::response(
                format!("Heartbeat findings:\n\n{}", msg),
            )),
            crate::agent::HeartbeatResult::Skipped => Ok(SubmissionResult::ok_with_message(
                "Heartbeat skipped: no HEARTBEAT.md checklist found in workspace.",
            )),
            crate::agent::HeartbeatResult::Failed(err) => Ok(SubmissionResult::error(format!(
                "Heartbeat failed: {}",
                err
            ))),
        }
    }

    /// Summarize the current thread's conversation.
    pub(super) async fn process_summarize(
        &self,
        session: Arc<Mutex<Session>>,
        thread_id: Uuid,
    ) -> Result<SubmissionResult, Error> {
        let messages = {
            let sess = session.lock().await;
            let thread = sess
                .threads
                .get(&thread_id)
                .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;
            thread.messages()
        };

        if messages.is_empty() {
            return Ok(SubmissionResult::ok_with_message(
                "Nothing to summarize (empty thread).",
            ));
        }

        // Build a summary prompt with the conversation
        let mut context = Vec::new();
        context.push(ChatMessage::system(
            "Summarize the conversation so far in 3-5 concise bullet points. \
             Focus on decisions made, actions taken, and key outcomes. \
             Be brief and factual.",
        ));
        // Include the conversation messages (truncate to last 20 to avoid context overflow)
        let start = if messages.len() > 20 {
            messages.len() - 20
        } else {
            0
        };
        context.extend_from_slice(&messages[start..]);
        context.push(ChatMessage::user("Summarize this conversation."));

        let request = crate::llm::CompletionRequest::new(context)
            .with_max_tokens(512)
            .with_temperature(0.3);

        let reasoning = Reasoning::new(self.llm().clone(), self.safety().clone());
        match reasoning.complete(request).await {
            Ok((text, _usage)) => Ok(SubmissionResult::response(format!(
                "Thread Summary:\n\n{}",
                text.trim()
            ))),
            Err(e) => Ok(SubmissionResult::error(format!("Summarize failed: {}", e))),
        }
    }

    /// Suggest next steps based on the current thread.
    pub(super) async fn process_suggest(
        &self,
        session: Arc<Mutex<Session>>,
        thread_id: Uuid,
    ) -> Result<SubmissionResult, Error> {
        let messages = {
            let sess = session.lock().await;
            let thread = sess
                .threads
                .get(&thread_id)
                .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;
            thread.messages()
        };

        if messages.is_empty() {
            return Ok(SubmissionResult::ok_with_message(
                "Nothing to suggest from (empty thread).",
            ));
        }

        let mut context = Vec::new();
        context.push(ChatMessage::system(
            "Based on the conversation so far, suggest 2-4 concrete next steps the user could take. \
             Be actionable and specific. Format as a numbered list.",
        ));
        let start = if messages.len() > 20 {
            messages.len() - 20
        } else {
            0
        };
        context.extend_from_slice(&messages[start..]);
        context.push(ChatMessage::user("What should I do next?"));

        let request = crate::llm::CompletionRequest::new(context)
            .with_max_tokens(512)
            .with_temperature(0.5);

        let reasoning = Reasoning::new(self.llm().clone(), self.safety().clone());
        match reasoning.complete(request).await {
            Ok((text, _usage)) => Ok(SubmissionResult::response(format!(
                "Suggested Next Steps:\n\n{}",
                text.trim()
            ))),
            Err(e) => Ok(SubmissionResult::error(format!("Suggest failed: {}", e))),
        }
    }

    /// Handle system commands that bypass thread-state checks entirely.
    pub(super) async fn handle_system_command(
        &self,
        command: &str,
        args: &[String],
        message: Option<&IncomingMessage>,
    ) -> Result<SubmissionResult, Error> {
        match command {
            "help" => Ok(SubmissionResult::response(concat!(
                "System:\n",
                "  /help             Show this help\n",
                "  /model [name]     Show or switch the active model\n",
                "  /version          Show version info\n",
                "  /tools            List available tools\n",
                "  /debug            Toggle debug mode\n",
                "  /ping             Connectivity check\n",
                "\n",
                "Jobs:\n",
                "  /job <desc>       Create a new job\n",
                "  /status [id]      Check job status\n",
                "  /cancel <id>      Cancel a job\n",
                "  /list             List all jobs\n",
                "\n",
                "Session:\n",
                "  /undo             Undo last turn\n",
                "  /redo             Redo undone turn\n",
                "  /compact          Compress context window\n",
                "  /clear            Clear current thread\n",
                "  /interrupt        Stop current operation\n",
                "  /new              New conversation thread\n",
                "  /thread <id>      Switch to thread\n",
                "  /resume <id>      Resume from checkpoint\n",
                "\n",
                "Skills:\n",
                "  /skills             List installed skills\n",
                "  /skills search <q>  Search ClawHub registry\n",
                "\n",
                "Agent:\n",
                "  /heartbeat        Run heartbeat check\n",
                "  /summarize        Summarize current thread\n",
                "  /suggest          Suggest next steps\n",
                "  /mentor <msg>     Ask mentor (text route)\n",
                "  /mentor_voice on|off|status|<msg>\n",
                "  /mentor_image <prompt>\n",
                "  /mentor_video <prompt>\n",
                "\n",
                "  /quit             Exit",
            ))),

            "ping" => Ok(SubmissionResult::response("pong!")),

            "version" => Ok(SubmissionResult::response(format!(
                "{} v{}",
                env!("CARGO_PKG_NAME"),
                env!("CARGO_PKG_VERSION")
            ))),

            "tools" => {
                let tools = self.tools().list().await;
                Ok(SubmissionResult::response(format!(
                    "Available tools: {}",
                    tools.join(", ")
                )))
            }

            "debug" => {
                // Debug toggle is handled client-side in the REPL.
                // For non-REPL channels, just acknowledge.
                Ok(SubmissionResult::ok_with_message(
                    "Debug toggle is handled by your client.",
                ))
            }

            "skills" => {
                if args.first().map(|s| s.as_str()) == Some("search") {
                    let query = args[1..].join(" ");
                    if query.is_empty() {
                        return Ok(SubmissionResult::error("Usage: /skills search <query>"));
                    }
                    self.handle_skills_search(&query).await
                } else if args.is_empty() {
                    self.handle_skills_list().await
                } else {
                    Ok(SubmissionResult::error(
                        "Usage: /skills or /skills search <query>",
                    ))
                }
            }

            "model" => {
                let current = self.llm().active_model_name();

                if args.is_empty() {
                    // Show current model and list available models
                    let mut out = format!("Active model: {}\n", current);
                    match self.llm().list_models().await {
                        Ok(models) if !models.is_empty() => {
                            out.push_str("\nAvailable models:\n");
                            for m in &models {
                                let marker = if *m == current { " (active)" } else { "" };
                                out.push_str(&format!("  {}{}\n", m, marker));
                            }
                            out.push_str("\nUse /model <name> to switch.");
                        }
                        Ok(_) => {
                            out.push_str(
                                "\nCould not fetch model list. Use /model <name> to switch.",
                            );
                        }
                        Err(e) => {
                            out.push_str(&format!(
                                "\nCould not fetch models: {}. Use /model <name> to switch.",
                                e
                            ));
                        }
                    }
                    Ok(SubmissionResult::response(out))
                } else {
                    let requested = &args[0];

                    // Validate the model exists
                    match self.llm().list_models().await {
                        Ok(models) if !models.is_empty() => {
                            if !models.iter().any(|m| m == requested) {
                                return Ok(SubmissionResult::error(format!(
                                    "Unknown model: {}. Available models:\n  {}",
                                    requested,
                                    models.join("\n  ")
                                )));
                            }
                        }
                        Ok(_) => {
                            // Empty model list, can't validate but try anyway
                        }
                        Err(e) => {
                            tracing::warn!("Could not fetch model list for validation: {}", e);
                        }
                    }

                    match self.llm().set_model(requested) {
                        Ok(()) => Ok(SubmissionResult::response(format!(
                            "Switched model to: {}",
                            requested
                        ))),
                        Err(e) => Ok(SubmissionResult::error(format!(
                            "Failed to switch model: {}",
                            e
                        ))),
                    }
                }
            }

            "mentor" => {
                let mentor_input = args.join(" ").trim().to_string();
                if mentor_input.is_empty() {
                    Ok(SubmissionResult::response("Usage: /mentor <message>"))
                } else {
                    self.handle_mentor_chat_command(message, &mentor_input, false)
                        .await
                }
            }

            "mentor_voice" => self.handle_mentor_voice_command(message, args).await,

            "mentor_image" => self.handle_mentor_image_command(message, args).await,
            "mentor_video" => self.handle_mentor_video_command(message, args).await,

            _ => Ok(SubmissionResult::error(format!(
                "Unknown command. Try /help"
            ))),
        }
    }

    /// List installed skills.
    async fn handle_skills_list(&self) -> Result<SubmissionResult, Error> {
        let Some(registry) = self.skill_registry() else {
            return Ok(SubmissionResult::error("Skills system not enabled."));
        };

        let guard = match registry.read() {
            Ok(g) => g,
            Err(e) => {
                return Ok(SubmissionResult::error(format!(
                    "Skill registry lock error: {}",
                    e
                )));
            }
        };

        let skills = guard.skills();
        if skills.is_empty() {
            return Ok(SubmissionResult::response(
                "No skills installed.\n\nUse /skills search <query> to find skills on ClawHub.",
            ));
        }

        let mut out = String::from("Installed skills:\n\n");
        for s in skills {
            let desc = if s.manifest.description.chars().count() > 60 {
                let truncated: String = s.manifest.description.chars().take(57).collect();
                format!("{}...", truncated)
            } else {
                s.manifest.description.clone()
            };
            out.push_str(&format!(
                "  {:<24} v{:<10} [{}]  {}\n",
                s.manifest.name, s.manifest.version, s.trust, desc,
            ));
        }
        out.push_str("\nUse /skills search <query> to find more on ClawHub.");

        Ok(SubmissionResult::response(out))
    }

    /// Search ClawHub for skills.
    async fn handle_skills_search(&self, query: &str) -> Result<SubmissionResult, Error> {
        let catalog = match self.skill_catalog() {
            Some(c) => c,
            None => {
                return Ok(SubmissionResult::error("Skill catalog not available."));
            }
        };

        let outcome = catalog.search(query).await;

        // Enrich top results with detail data (stars, downloads, owner)
        let mut entries = outcome.results;
        catalog.enrich_search_results(&mut entries, 5).await;

        let mut out = format!("ClawHub results for \"{}\":\n\n", query);

        if entries.is_empty() {
            if let Some(ref err) = outcome.error {
                out.push_str(&format!("  (registry error: {})\n", err));
            } else {
                out.push_str("  No results found.\n");
            }
        } else {
            for entry in &entries {
                let owner_str = entry
                    .owner
                    .as_deref()
                    .map(|o| format!("  by {}", o))
                    .unwrap_or_default();

                let stats_parts: Vec<String> = [
                    entry.stars.map(|s| format!("{} stars", s)),
                    entry.downloads.map(|d| format_count(d, "downloads")),
                ]
                .into_iter()
                .flatten()
                .collect();
                let stats_str = if stats_parts.is_empty() {
                    String::new()
                } else {
                    format!("  {}", stats_parts.join("  "))
                };

                out.push_str(&format!(
                    "  {:<24} v{:<10}{}{}\n",
                    entry.name, entry.version, owner_str, stats_str,
                ));
                if !entry.description.is_empty() {
                    out.push_str(&format!("    {}\n\n", entry.description));
                }
            }
        }

        // Show matching installed skills
        if let Some(registry) = self.skill_registry()
            && let Ok(guard) = registry.read()
        {
            let query_lower = query.to_lowercase();
            let matches: Vec<_> = guard
                .skills()
                .iter()
                .filter(|s| {
                    s.manifest.name.to_lowercase().contains(&query_lower)
                        || s.manifest.description.to_lowercase().contains(&query_lower)
                })
                .collect();

            if !matches.is_empty() {
                out.push_str(&format!("Installed skills matching \"{}\":\n", query));
                for s in &matches {
                    out.push_str(&format!(
                        "  {:<24} v{:<10} [{}]\n",
                        s.manifest.name, s.manifest.version, s.trust,
                    ));
                }
            }
        }

        Ok(SubmissionResult::response(out))
    }

    async fn resolve_mentor_tool_name(&self, logical_tool_name: &str) -> Option<String> {
        let suffix = format!("_{logical_tool_name}");
        let tools = self.tools().list().await;

        tools
            .iter()
            .find(|name| {
                name.starts_with("mentor_")
                    && (name.as_str() == logical_tool_name || name.ends_with(&suffix))
            })
            .cloned()
            .or_else(|| {
                tools
                    .iter()
                    .find(|name| name.as_str() == logical_tool_name || name.ends_with(&suffix))
                    .cloned()
            })
    }

    async fn execute_mentor_tool(
        &self,
        logical_tool_name: &str,
        params: serde_json::Value,
        user_id: &str,
    ) -> Result<serde_json::Value, String> {
        let tool_name = self
            .resolve_mentor_tool_name(logical_tool_name)
            .await
            .ok_or_else(|| format!("Mentor tool not registered: {logical_tool_name}"))?;

        let tool = self
            .tools()
            .get(&tool_name)
            .await
            .ok_or_else(|| format!("Mentor tool unavailable at runtime: {tool_name}"))?;

        let mut job_ctx = JobContext::with_user(
            user_id,
            format!("mentor:{logical_tool_name}"),
            "Mentor command invocation",
        );
        job_ctx.metadata = json!({
            "source": "mentor_command",
            "logical_tool_name": logical_tool_name,
            "tool_name": tool_name,
        });

        let voice_reply_requested = params
            .get("voiceReply")
            .and_then(|value| value.as_bool())
            .unwrap_or(false);
        let timeout_secs = if logical_tool_name == "mentor.chat" && voice_reply_requested {
            180
        } else if logical_tool_name == "mentor.speak"
            || logical_tool_name == "mentor.image"
            || logical_tool_name == "mentor.video"
        {
            180
        } else {
            60
        };

        let output = tokio::time::timeout(Duration::from_secs(timeout_secs), async {
            tool.execute(params, &job_ctx).await
        })
        .await
        .map_err(|_| format!("Mentor MCP transport timeout while waiting for {logical_tool_name}"))?
        .map_err(|err| map_mentor_tool_error(logical_tool_name, err))?;

        let raw_payload = match output.result {
            serde_json::Value::String(raw) => raw,
            other => other.to_string(),
        };

        parse_tool_json_output(&raw_payload).or_else(|_| Ok(json!({ "text": raw_payload })))
    }

    async fn handle_mentor_chat_command(
        &self,
        message: Option<&IncomingMessage>,
        mentor_input: &str,
        voice_reply: bool,
    ) -> Result<SubmissionResult, Error> {
        let mentor_ctx = mentor_context_from_message(message);
        let user_id = message.map(|msg| msg.user_id.as_str()).unwrap_or("default");

        let payload = self
            .execute_mentor_tool(
                "mentor.chat",
                json!({
                    "message": mentor_input,
                    "sessionId": mentor_ctx.session_id,
                    "voiceReply": voice_reply,
                }),
                user_id,
            )
            .await;

        let payload = match payload {
            Ok(payload) => payload,
            Err(primary_error) => {
                if voice_reply {
                    let fallback = self
                        .execute_mentor_tool(
                            "mentor.chat",
                            json!({
                                "message": mentor_input,
                                "sessionId": mentor_ctx.session_id,
                                "voiceReply": false,
                            }),
                            user_id,
                        )
                        .await;

                    if let Ok(fallback_payload) = fallback {
                        let text = extract_mentor_reply(&fallback_payload);
                        let text = if text.is_empty() {
                            "Mentor returned no content.".to_string()
                        } else {
                            text
                        };
                        let fallback_note = mentor_error_summary(&primary_error);
                        return Ok(SubmissionResult::response(format!(
                            "{text}\n\nVoice fallback: {fallback_note}"
                        )));
                    }
                }

                return Ok(SubmissionResult::error(mentor_error_summary(
                    &primary_error,
                )));
            }
        };

        let mut reply = extract_mentor_reply(&payload);
        if reply.is_empty() {
            reply = "Mentor returned no content.".to_string();
        }
        if voice_reply {
            reply = enforce_voice_reply_style(&reply);
        }

        if !voice_reply {
            return Ok(SubmissionResult::response(reply));
        }

        let voice_artifact = payload
            .get("voiceArtifact")
            .and_then(|value| value.as_str())
            .map(|value| value.to_string());

        let Some(voice_artifact) = voice_artifact else {
            return Ok(SubmissionResult::response(reply));
        };

        match load_audio_attachment_data_url(&voice_artifact) {
            Ok(data_url) => Ok(SubmissionResult::response_with_attachments(
                reply,
                vec![data_url],
            )),
            Err(error) => Ok(SubmissionResult::response(format!(
                "{reply}\n\nVoice fallback: {}",
                mentor_error_summary(&error)
            ))),
        }
    }

    async fn mentor_voice_status(
        &self,
        message: Option<&IncomingMessage>,
    ) -> Result<SubmissionResult, Error> {
        let mentor_ctx = mentor_context_from_message(message);
        let user_id = message.map(|msg| msg.user_id.as_str()).unwrap_or("default");

        let status = self
            .execute_mentor_tool("mentor.status", json!({}), user_id)
            .await;
        match status {
            Ok(status) => {
                let mentor_name = status
                    .get("mentor")
                    .and_then(|value| value.as_str())
                    .unwrap_or("Lippyclaw Mentor");
                let llm_ready = status
                    .get("llm")
                    .and_then(|value| value.get("apiKeyConfigured"))
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false);
                let voice_enabled = status
                    .get("voice")
                    .and_then(|value| value.get("enabled"))
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false);
                let voice_provider = status
                    .get("voice")
                    .and_then(|value| value.get("provider"))
                    .and_then(|value| value.as_str())
                    .unwrap_or("unknown");
                let sample_ready = status
                    .get("voice")
                    .and_then(|value| value.get("sampleReady"))
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false);
                let context_ready = status
                    .get("voice")
                    .and_then(|value| value.get("contextReady"))
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false);

                Ok(SubmissionResult::response(format!(
                    "{mentor_name} status\nvoice_mode={}\nvoice_provider={}\nllm_ready={}\nvoice_enabled={}\nsample_ready={}\ncontext_ready={}",
                    if mentor_ctx.voice_mode { "on" } else { "off" },
                    voice_provider,
                    llm_ready,
                    voice_enabled,
                    sample_ready,
                    context_ready
                )))
            }
            Err(error) => Ok(SubmissionResult::error(mentor_error_summary(&error))),
        }
    }

    async fn handle_mentor_image_command(
        &self,
        message: Option<&IncomingMessage>,
        args: &[String],
    ) -> Result<SubmissionResult, Error> {
        if args.is_empty() {
            return Ok(SubmissionResult::response(
                "Usage: /mentor_image [--provider chutes|novita] <prompt>",
            ));
        }

        let mut provider: Option<String> = None;
        let mut prompt_start = 0usize;

        if args.len() >= 3 && args[0] == "--provider" {
            provider = Some(args[1].to_ascii_lowercase());
            prompt_start = 2;
        } else if let Some(raw_provider) = args[0].strip_prefix("provider=") {
            provider = Some(raw_provider.to_ascii_lowercase());
            prompt_start = 1;
        }

        let prompt = args[prompt_start..].join(" ").trim().to_string();
        if prompt.is_empty() {
            return Ok(SubmissionResult::response(
                "Usage: /mentor_image [--provider chutes|novita] <prompt>",
            ));
        }

        let user_id = message.map(|msg| msg.user_id.as_str()).unwrap_or("default");
        let mut params = json!({
            "prompt": prompt,
        });
        if let Some(provider_value) = provider {
            params["provider"] = json!(provider_value);
        }

        let payload = match self
            .execute_mentor_tool("mentor.image", params, user_id)
            .await
        {
            Ok(payload) => payload,
            Err(error) => return Ok(SubmissionResult::error(mentor_error_summary(&error))),
        };

        let image_artifact = payload
            .get("imageArtifact")
            .and_then(|value| value.as_str())
            .map(|value| value.to_string());
        let image_provider = payload
            .get("imageProvider")
            .and_then(|value| value.as_str())
            .unwrap_or("unknown");

        let Some(image_artifact) = image_artifact else {
            return Ok(SubmissionResult::response(format!(
                "Mentor image generation completed, but no image artifact path was returned (provider={image_provider})."
            )));
        };

        let summary = format!("Generated image with {image_provider}.");
        match load_image_attachment_data_url(&image_artifact) {
            Ok(data_url) => Ok(SubmissionResult::response_with_attachments(
                summary,
                vec![data_url],
            )),
            Err(error) => Ok(SubmissionResult::response(format!(
                "{summary}\n\nImage delivery fallback: {}",
                mentor_error_summary(&error)
            ))),
        }
    }

    async fn handle_mentor_video_command(
        &self,
        message: Option<&IncomingMessage>,
        args: &[String],
    ) -> Result<SubmissionResult, Error> {
        if args.is_empty() {
            return Ok(SubmissionResult::response(
                "Usage: /mentor_video [--provider chutes|novita] <prompt>",
            ));
        }

        let mut provider: Option<String> = None;
        let mut prompt_start = 0usize;

        if args.len() >= 3 && args[0] == "--provider" {
            provider = Some(args[1].to_ascii_lowercase());
            prompt_start = 2;
        } else if let Some(raw_provider) = args[0].strip_prefix("provider=") {
            provider = Some(raw_provider.to_ascii_lowercase());
            prompt_start = 1;
        }

        let prompt = args[prompt_start..].join(" ").trim().to_string();
        if prompt.is_empty() {
            return Ok(SubmissionResult::response(
                "Usage: /mentor_video [--provider chutes|novita] <prompt>",
            ));
        }

        let user_id = message.map(|msg| msg.user_id.as_str()).unwrap_or("default");
        let mut params = json!({
            "prompt": prompt,
        });
        if let Some(provider_value) = provider {
            params["provider"] = json!(provider_value);
        }

        let payload = match self
            .execute_mentor_tool("mentor.video", params, user_id)
            .await
        {
            Ok(payload) => payload,
            Err(error) => return Ok(SubmissionResult::error(mentor_error_summary(&error))),
        };

        let video_artifact = payload
            .get("videoArtifact")
            .and_then(|value| value.as_str())
            .map(|value| value.to_string());
        let video_provider = payload
            .get("videoProvider")
            .and_then(|value| value.as_str())
            .unwrap_or("unknown");

        let Some(video_artifact) = video_artifact else {
            return Ok(SubmissionResult::response(format!(
                "Mentor video generation completed, but no video artifact path was returned (provider={video_provider})."
            )));
        };

        let summary = format!("Generated video with {video_provider}.");
        match load_video_attachment_data_url(&video_artifact) {
            Ok(data_url) => Ok(SubmissionResult::response_with_attachments(
                summary,
                vec![data_url],
            )),
            Err(error) => Ok(SubmissionResult::response(format!(
                "{summary}\n\nVideo delivery fallback: {}",
                mentor_error_summary(&error)
            ))),
        }
    }

    async fn handle_mentor_voice_command(
        &self,
        message: Option<&IncomingMessage>,
        args: &[String],
    ) -> Result<SubmissionResult, Error> {
        let mentor_ctx = mentor_context_from_message(message);

        if args.is_empty() {
            if let Some(transcript) = mentor_ctx.voice_transcript.as_deref()
                && !transcript.trim().is_empty()
            {
                return self
                    .handle_mentor_chat_command(message, transcript.trim(), true)
                    .await;
            }

            if let Some(transcription_error) = mentor_ctx.voice_transcription_error {
                return Ok(SubmissionResult::error(mentor_error_summary(
                    &transcription_error,
                )));
            }

            if let Some(mode) = mentor_ctx.voice_mode_changed.as_deref() {
                let current_mode = if mentor_ctx.voice_mode { "on" } else { "off" };
                return Ok(SubmissionResult::response(format!(
                    "mentor_voice mode set to {mode} (current={current_mode})"
                )));
            }

            if mentor_ctx.voice_mode {
                return self.mentor_voice_status(message).await;
            }

            return Ok(SubmissionResult::response(
                "Usage: /mentor_voice on|off|status|<message>",
            ));
        }

        let first = args[0].to_lowercase();
        if args.len() == 1 {
            match first.as_str() {
                "on" | "off" => {
                    let mode = if first == "on" { "on" } else { "off" };
                    let applied = mentor_ctx
                        .voice_mode_changed
                        .as_deref()
                        .map(|value| value.eq_ignore_ascii_case(mode))
                        .unwrap_or(false);
                    let current_mode = if mentor_ctx.voice_mode { "on" } else { "off" };
                    let status_line = if applied {
                        format!("mentor_voice mode set to {mode} (current={current_mode})")
                    } else {
                        format!(
                            "mentor_voice mode request={mode} (current={current_mode}). If this is Telegram, ensure the channel persisted the toggle."
                        )
                    };

                    return Ok(SubmissionResult::response(status_line));
                }
                "status" => {
                    return self.mentor_voice_status(message).await;
                }
                _ => {}
            }
        }

        let mentor_input = args.join(" ");
        self.handle_mentor_chat_command(message, mentor_input.trim(), true)
            .await
    }

    /// Handle legacy command routing from the Router (job commands that go through
    /// process_user_input -> router -> handle_job_or_command -> here).
    #[allow(dead_code)]
    pub(super) async fn handle_command(
        &self,
        command: &str,
        args: &[String],
    ) -> Result<Option<String>, Error> {
        // System commands are now handled directly via Submission::SystemCommand,
        // but the router may still send us unknown /commands.
        match self.handle_system_command(command, args, None).await? {
            SubmissionResult::Response { content, .. } => Ok(Some(content)),
            SubmissionResult::Ok { message } => Ok(message),
            SubmissionResult::Error { message } => Ok(Some(format!("Error: {}", message))),
            _ => Ok(None),
        }
    }
}
