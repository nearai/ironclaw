//! System commands and job handlers for the agent.
//!
//! Extracted from `agent_loop.rs` to isolate the /help, /model, /status,
//! and other command processing from the core agent loop.

use std::sync::Arc;

use tokio::sync::Mutex;
use uuid::Uuid;

use crate::agent::session::Session;
use crate::agent::submission::SubmissionResult;
use crate::agent::{Agent, MessageIntent};
use crate::channels::{IncomingMessage, StatusUpdate};
use crate::context::JobState;
use crate::error::{ConfigError, Error};
use crate::llm::{ChatMessage, Reasoning};

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

fn format_history_line(
    prefix: char,
    id: Uuid,
    label: &str,
    count_label: &str,
    count: i64,
    updated_at: &str,
    title: Option<&str>,
) -> String {
    let mut line = format!(
        "{} {} [{}] {}={} updated={}",
        prefix, id, label, count_label, count, updated_at
    );
    if let Some(title) = title
        && !title.is_empty()
    {
        line.push_str(&format!(" — {}", title));
    }
    line
}

impl Agent {
    /// Handle job-related intents without turn tracking.
    pub(super) async fn handle_job_or_command(
        &self,
        session: Arc<Mutex<Session>>,
        intent: MessageIntent,
        message: &IncomingMessage,
        tenant: &crate::tenant::TenantCtx,
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
                self.handle_create_job(tenant, title, description, category)
                    .await?
            }
            MessageIntent::CheckJobStatus { job_id } => {
                self.handle_check_status(tenant, job_id).await?
            }
            MessageIntent::CancelJob { job_id } => self.handle_cancel_job(tenant, &job_id).await?,
            MessageIntent::ListJobs { filter } => self.handle_list_jobs(tenant, filter).await?,
            MessageIntent::HelpJob { job_id } => self.handle_help_job(tenant, &job_id).await?,
            MessageIntent::Command { command, args } => {
                match self
                    .handle_command(session, &command, &args, &message.channel, tenant)
                    .await?
                {
                    Some(s) => s,
                    None => return Ok(SubmissionResult::Ok { message: None }), // Shutdown signal
                }
            }
            _ => "Unknown intent".to_string(),
        };
        Ok(SubmissionResult::response(response))
    }

    async fn handle_create_job(
        &self,
        tenant: &crate::tenant::TenantCtx,
        title: String,
        description: String,
        category: Option<String>,
    ) -> Result<String, Error> {
        let job_id = self
            .scheduler
            .dispatch_job(tenant.user_id(), &title, &description, None)
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

    async fn handle_history_command(
        &self,
        session: Arc<Mutex<Session>>,
        tenant: &crate::tenant::TenantCtx,
        args: &[String],
    ) -> Result<String, Error> {
        // Parse pagination arguments: --limit N (default: 50), --page N (default: 1)
        let mut limit: i64 = 50;
        let mut page: i64 = 1;
        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "--limit" | "-l" => {
                    if i + 1 < args.len() {
                        limit = args[i + 1].parse().unwrap_or(50);
                        limit = limit.clamp(1, 200); // Cap at 200 to avoid excessive output
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                "--page" | "-p" => {
                    if i + 1 < args.len() {
                        page = args[i + 1].parse().unwrap_or(1);
                        page = page.clamp(1, 100);
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                _ => i += 1,
            }
        }
        let offset = (page - 1) * limit;

        let (session_id, active_thread, threads) = {
            let sess = session.lock().await;
            (
                sess.id,
                sess.active_thread,
                sess.threads.values().cloned().collect::<Vec<_>>(),
            )
        };

        let mut output = String::new();
        output.push_str(&format!("Session: {}\n", session_id));
        if let Some(active) = active_thread {
            output.push_str(&format!("Active thread: {}\n", active));
        }

        let mut listed_any = false;
        let mut seen_threads = std::collections::HashSet::new();

        if let Some(store) = tenant.store() {
            match store.list_conversations_all_channels_paginated(limit, offset).await {
                Ok(mut summaries) if !summaries.is_empty() => {
                    summaries.sort_by_key(|s| std::cmp::Reverse(s.last_activity));
                    output.push_str("Persistent threads (use /thread <id> to hydrate):\n");
                    for summary in summaries {
                        seen_threads.insert(summary.id);
                        listed_any = true;
                        let prefix = if Some(summary.id) == active_thread {
                            '*'
                        } else {
                            ' '
                        };
                        let label = match summary.thread_type.as_deref() {
                            Some(thread_type) => format!("{}/{}", thread_type, summary.channel),
                            None => summary.channel.clone(),
                        };
                        // Add [DB] indicator to show this thread is persisted but not yet hydrated
                        let line = format_history_line(
                            prefix,
                            summary.id,
                            &label,
                            "messages",
                            summary.message_count.max(0),
                            &summary.last_activity.to_rfc3339(),
                            summary.title.as_deref(),
                        );
                        output.push_str(&line);
                        output.push_str(" [DB]\n");
                    }
                }
                Ok(_) => {}
                Err(e) => {
                    tracing::debug!(
                        "[commands::history] Failed to list persistent conversations: {}",
                        e
                    );
                }
            }
        }

        let mut in_memory_threads: Vec<_> = threads
            .into_iter()
            .filter(|thread| !seen_threads.contains(&thread.id))
            .collect();
        in_memory_threads.sort_by_key(|thread| std::cmp::Reverse(thread.updated_at));

        if !in_memory_threads.is_empty() || !listed_any {
            if listed_any {
                output.push('\n');
            }
            output.push_str("Current session threads:\n");
            if in_memory_threads.is_empty() {
                output.push_str("  (no threads yet)\n");
            } else {
                for thread in in_memory_threads {
                    let prefix = if Some(thread.id) == active_thread {
                        '*'
                    } else {
                        ' '
                    };
                    let title = thread.metadata.get("title").and_then(|v| v.as_str());
                    let line = format_history_line(
                        prefix,
                        thread.id,
                        &format!("{:?}", thread.state),
                        "turns",
                        thread.turns.len() as i64,
                        &thread.updated_at.to_rfc3339(),
                        title,
                    );
                    output.push_str(&line);
                    output.push('\n');
                }
            }
        }

        output.push_str("\nUse /thread <id> to switch threads.");
        Ok(output.trim_end().to_string())
    }

    /// Handle /history messages <thread-id> [--limit N] [--page N]
    async fn handle_history_messages_command(
        &self,
        session: Arc<Mutex<Session>>,
        tenant: &crate::tenant::TenantCtx,
        args: &[String],
    ) -> Result<String, Error> {
        // Parse arguments: <thread-id> [--limit N] [--page N]
        if args.is_empty() {
            return Ok("Usage: /history messages <thread-id> [--limit N] [--page N]\n\nList messages from a specific thread with pagination.".to_string());
        }

        let thread_id_str = &args[0];
        let thread_id = Uuid::parse_str(thread_id_str)
            .map_err(|e| ConfigError::InvalidValue { 
                key: "thread_id".to_string(), 
                message: format!("Invalid UUID '{}': {}", thread_id_str, e) 
            })?;

        // Parse pagination arguments
        let mut limit: i64 = 50;
        let mut page: i64 = 1;
        let mut i = 1;
        while i < args.len() {
            match args[i].as_str() {
                "--limit" | "-l" => {
                    if i + 1 < args.len() {
                        limit = args[i + 1].parse().unwrap_or(50);
                        limit = limit.clamp(1, 200);
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                "--page" | "-p" => {
                    if i + 1 < args.len() {
                        page = args[i + 1].parse().unwrap_or(1);
                        page = page.clamp(1, 100);
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                _ => i += 1,
            }
        }

        // Try to get messages from database first (persistent threads)
        if let Some(store) = tenant.store() {
            match store.list_conversation_messages_paginated(thread_id, None, limit).await {
                Ok((messages, has_more)) => {
                    if messages.is_empty() {
                        return Ok(format!("Thread {} has no messages.", thread_id));
                    }

                    let mut output = String::new();
                    output.push_str(&format!("Thread: {} (Page {} of ~{}, showing {} messages)\n", 
                        thread_id, page, if has_more { "N+" } else { "1" }, messages.len()));
                    output.push_str(&format!("Limit: {}\n\n", limit));

                    for msg in messages {
                        let timestamp = msg.created_at.format("%Y-%m-%d %H:%M:%S");
                        output.push_str(&format!("[{}] {}: {}\n", 
                            timestamp, 
                            msg.role, 
                            msg.content.lines().next().unwrap_or("").chars().take(200).collect::<String>()));
                    }

                    if has_more {
                        output.push_str(&format!("\nUse --page {} to see more messages.", page + 1));
                    }

                    return Ok(output);
                }
                Err(_) => {
                    // Fall through to in-memory check
                }
            }
        }

        // Check in-memory session threads
        let sess = session.lock().await;
        if let Some(thread) = sess.threads.get(&thread_id) {
            if thread.turns.is_empty() {
                return Ok(format!("Thread {} has no turns in current session.", thread_id));
            }

            let mut output = String::new();
            output.push_str(&format!("Thread: {} (in-memory, {} turns)\n\n", thread_id, thread.turns.len()));

            for (i, turn) in thread.turns.iter().enumerate() {
                output.push_str(&format!("Turn {} - User: {}\n", i + 1, 
                    turn.user_input.chars().take(200).collect::<String>()));
                if let Some(ref response) = turn.response {
                    output.push_str(&format!("         Assistant: {}\n", 
                        response.chars().take(200).collect::<String>()));
                }
                output.push('\n');
            }

            return Ok(output.trim_end().to_string());
        }

        Err(ConfigError::InvalidValue {
            key: "thread_id".to_string(),
            message: format!("Thread {} not found. Use /history to list available threads.", thread_id),
        }.into())
    }

    async fn handle_check_status(
        &self,
        tenant: &crate::tenant::TenantCtx,
        job_id: Option<String>,
    ) -> Result<String, Error> {
        match job_id {
            Some(id) => {
                let uuid = Uuid::parse_str(&id)
                    .map_err(|_| crate::error::JobError::NotFound { id: Uuid::nil() })?;

                // Try DB first for persistent state, fall back to ContextManager.
                // TenantScope.get_job() auto-filters by ownership — no manual check needed.
                if let Some(store) = tenant.store()
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
                if ctx.user_id != tenant.user_id() {
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
                // TenantScope methods auto-scope to user — no user_id parameter needed.
                if let Some(store) = tenant.store() {
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
                let summary = self.context_manager.summary_for(tenant.user_id()).await;
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

    async fn handle_cancel_job(
        &self,
        tenant: &crate::tenant::TenantCtx,
        job_id: &str,
    ) -> Result<String, Error> {
        let uuid = Uuid::parse_str(job_id)
            .map_err(|_| crate::error::JobError::NotFound { id: Uuid::nil() })?;

        let ctx = self.context_manager.get_context(uuid).await?;
        if ctx.user_id != tenant.user_id() {
            return Err(crate::error::JobError::NotFound { id: uuid }.into());
        }

        self.scheduler.stop(uuid).await?;

        // Also update DB so the Jobs tab reflects cancellation immediately.
        // Use TenantScope — ownership already verified above.
        if let Some(store) = tenant.store()
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
        tenant: &crate::tenant::TenantCtx,
        _filter: Option<String>,
    ) -> Result<String, Error> {
        // List from DB for consistency with Jobs tab.
        // TenantScope methods auto-scope to user.
        if let Some(store) = tenant.store() {
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
        let jobs = self.context_manager.all_jobs_for(tenant.user_id()).await;
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

    async fn handle_help_job(
        &self,
        tenant: &crate::tenant::TenantCtx,
        job_id: &str,
    ) -> Result<String, Error> {
        let uuid = Uuid::parse_str(job_id)
            .map_err(|_| crate::error::JobError::NotFound { id: Uuid::nil() })?;

        let ctx = self.context_manager.get_context(uuid).await?;
        if ctx.user_id != tenant.user_id() {
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
        tenant: &crate::tenant::TenantCtx,
        job_id: Option<&str>,
    ) -> Result<SubmissionResult, Error> {
        match self
            .handle_check_status(tenant, job_id.map(|s| s.to_string()))
            .await
        {
            Ok(text) => Ok(SubmissionResult::response(text)),
            Err(e) => Ok(SubmissionResult::error(format!("Job status error: {}", e))),
        }
    }

    /// Cancel a job by ID.
    pub(super) async fn process_job_cancel(
        &self,
        tenant: &crate::tenant::TenantCtx,
        job_id: &str,
    ) -> Result<SubmissionResult, Error> {
        match self.handle_cancel_job(tenant, job_id).await {
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

        let reasoning =
            Reasoning::new(self.llm().clone()).with_model_name(self.llm().active_model_name());
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

        let reasoning =
            Reasoning::new(self.llm().clone()).with_model_name(self.llm().active_model_name());
        match reasoning.complete(request).await {
            Ok((text, _usage)) => Ok(SubmissionResult::response(format!(
                "Suggested Next Steps:\n\n{}",
                text.trim()
            ))),
            Err(e) => Ok(SubmissionResult::error(format!("Suggest failed: {}", e))),
        }
    }

    /// Handle `/reasoning [N|all]` — show reasoning history for the active thread.
    pub(super) async fn handle_reasoning_command(
        &self,
        args: &[String],
        session: &Arc<Mutex<Session>>,
        thread_id: Uuid,
    ) -> SubmissionResult {
        // Clone the turn data we need, then drop the session lock.
        let turns_snapshot: Vec<(
            usize,
            Option<String>,
            Vec<crate::agent::session::TurnToolCall>,
        )>;
        {
            let sess = session.lock().await;
            let thread = match sess.threads.get(&thread_id) {
                Some(t) => t,
                None => return SubmissionResult::error("No active thread."),
            };

            if thread.turns.is_empty() {
                return SubmissionResult::ok_with_message("No turns yet.");
            }

            // Parse argument: default=last turn, "all"=all turns, N=specific turn (1-based).
            let selected: Vec<&crate::agent::session::Turn> = match args.first().map(|s| s.as_str())
            {
                Some("all") => thread.turns.iter().collect(),
                Some(n) => match n.parse::<usize>() {
                    Ok(0) => return SubmissionResult::error("Turn numbers start at 1."),
                    Ok(num) if num > thread.turns.len() => {
                        return SubmissionResult::error(format!(
                            "Turn {} does not exist (max: {}).",
                            num,
                            thread.turns.len()
                        ));
                    }
                    Ok(num) => vec![&thread.turns[num - 1]],
                    Err(_) => return SubmissionResult::error("Usage: /reasoning [N|all]"),
                },
                None => {
                    // Default: last turn that has tool calls
                    match thread.turns.iter().rev().find(|t| !t.tool_calls.is_empty()) {
                        Some(t) => vec![t],
                        None => {
                            return SubmissionResult::ok_with_message("No turns with tool calls.");
                        }
                    }
                }
            };

            turns_snapshot = selected
                .into_iter()
                .map(|t| (t.turn_number, t.narrative.clone(), t.tool_calls.clone()))
                .collect();
        }
        // Session lock is now dropped — format output without holding it.

        let mut output = String::new();
        for (turn_number, narrative, tool_calls) in &turns_snapshot {
            output.push_str(&format!("--- Turn {} ---\n", turn_number + 1));
            if let Some(narrative) = narrative {
                output.push_str(&format!("Reasoning: {}\n", narrative));
            }
            if tool_calls.is_empty() {
                output.push_str("  (no tool calls)\n");
            } else {
                for tc in tool_calls {
                    let status = if tc.error.is_some() {
                        "error"
                    } else if tc.result.is_some() {
                        "ok"
                    } else {
                        "pending"
                    };
                    output.push_str(&format!("  {} [{}]", tc.name, status));
                    if let Some(ref rationale) = tc.rationale {
                        output.push_str(&format!(" — {}", rationale));
                    }
                    output.push('\n');
                }
            }
            output.push('\n');
        }

        SubmissionResult::response(output.trim_end())
    }

    /// Handle system commands that bypass thread-state checks entirely.
    pub(super) async fn handle_system_command(
        &self,
        session: Arc<Mutex<Session>>,
        command: &str,
        args: &[String],
        channel: &str,
        tenant: &crate::tenant::TenantCtx,
    ) -> Result<SubmissionResult, Error> {
        match command {
            "help" => Ok(SubmissionResult::response(concat!(
                "System:\n",
                "  /help             Show this help\n",
                "  /model [name]     Show or switch the active model\n",
                "  /version          Show version info\n",
                "  /tools            List available tools\n",
                "  /debug            Toggle debug mode\n",
                "  /reasoning [N|all] Show agent reasoning for turns\n",
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
                "  /new | /thread new  Create new conversation thread\n",
                "  /thread <id>      Switch to existing thread (UUID)\n",
                "  /history          List all threads (persistent + session)\\n",
                "  /history messages <id> [--limit N] [--page N]  List messages from a thread\\n",
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
                "  /restart          Gracefully restart the process\n",
                "\n",
                "  /quit             Exit",
            ))),

            "ping" => Ok(SubmissionResult::response("pong!")),

            "restart" => {
                tracing::info!("[commands::restart] Restart command received");
                // Channel authorization check: restart is only available via web interface
                if channel != "gateway" {
                    tracing::warn!(
                        "[commands::restart] Restart rejected: not from gateway channel (from: {})",
                        channel
                    );
                    return Ok(SubmissionResult::error(
                        "Restart is only available through the web interface with explicit user confirmation. \
                         Use the Restart button in the UI."
                            .to_string(),
                    ));
                }
                // Environment check: restart is only available in Docker containers
                let in_docker = std::env::var("IRONCLAW_IN_DOCKER")
                    .map(|v| v.to_lowercase() == "true")
                    .unwrap_or(false);

                tracing::debug!("[commands::restart] IRONCLAW_IN_DOCKER={}", in_docker);

                if !in_docker {
                    tracing::warn!(
                        "[commands::restart] Restart rejected: not in Docker environment"
                    );
                    return Ok(SubmissionResult::error(
                        "Restart is not available in this environment. \
                         The IRONCLAW_IN_DOCKER environment variable must be set to 'true' for Docker deployments."
                            .to_string(),
                    ));
                }

                // Execute restart tool directly (don't dispatch as a job for LLM planning)
                // This ensures the tool runs immediately without LLM involvement
                use crate::tools::Tool;
                let tool = crate::tools::builtin::RestartTool;
                let params = serde_json::json!({});

                // Create a minimal JobContext for the tool
                let dummy_ctx =
                    crate::context::JobContext::with_user("system", "Restart", "Graceful restart");

                match tool.execute(params, &dummy_ctx).await {
                    Ok(output) => {
                        tracing::info!("[commands::restart] RestartTool executed successfully");
                        // Extract text from the ToolOutput result
                        let response = match output.result {
                            serde_json::Value::String(s) => s,
                            _ => output.result.to_string(),
                        };
                        Ok(SubmissionResult::response(response))
                    }
                    Err(e) => {
                        tracing::error!(
                            "[commands::restart] RestartTool execution failed: {:?}",
                            e
                        );
                        Ok(SubmissionResult::error(format!("Restart failed: {}", e)))
                    }
                }
            }

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

            "history" => {
                // Support subcommands: /history messages <thread-id> [--limit N] [--page N]
                if args.first().map(|s| s.as_str()) == Some("messages") {
                    let history = self.handle_history_messages_command(session, tenant, &args[1..]).await?;
                    Ok(SubmissionResult::response(history))
                } else {
                    let history = self.handle_history_command(session, tenant, args).await?;
                    Ok(SubmissionResult::response(history))
                }
            }

            "thread" => {
                // /thread (no args) - show current thread info
                if args.is_empty() {
                    let (session_id, active_thread) = {
                        let sess = session.lock().await;
                        (sess.id, sess.active_thread)
                    };

                    let mut output = String::new();
                    output.push_str(&format!("Session: {}\n", session_id));

                    if let Some(thread_id) = active_thread {
                        let sess = session.lock().await;
                        if let Some(thread) = sess.threads.get(&thread_id) {
                            output.push_str(&format!("Current thread: {}\n", thread_id));
                            output.push_str(&format!("State: {:?}\n", thread.state));
                            output.push_str(&format!("Turns: {}\n", thread.turns.len()));

                            if let Some(title) =
                                thread.metadata.get("title").and_then(|v| v.as_str())
                            {
                                output.push_str(&format!("Title: {}\n", title));
                            }

                            output.push_str(&format!(
                                "Updated: {}\n",
                                thread.updated_at.to_rfc3339()
                            ));
                        } else {
                            output.push_str(&format!(
                                "Active thread: {} (not found in memory)\n",
                                thread_id
                            ));
                        }
                    } else {
                        output.push_str("No active thread.\n");
                    }

                    output.push_str("\nUse /history or /thread list to see all threads.");
                    Ok(SubmissionResult::response(output))
                } else if args.first().map(|s| s.as_str()) == Some("list") {
                    // /thread list - alias for /history
                    let history = self.handle_history_command(session, tenant, args).await?;
                    Ok(SubmissionResult::response(history))
                } else if args.first().map(|s| s.as_str()) == Some("new") {
                    // /thread new - delegate to submission parser (should be handled upstream)
                    // If we get here, create a new thread directly
                    let session_id = {
                        let sess = session.lock().await;
                        sess.id
                    };
                    let new_thread = crate::agent::session::Thread::new(session_id);
                    let new_thread_id = new_thread.id;
                    {
                        let mut sess = session.lock().await;
                        sess.threads.insert(new_thread_id, new_thread);
                        sess.active_thread = Some(new_thread_id);
                        sess.last_active_at = chrono::Utc::now();
                    }
                    Ok(SubmissionResult::response(format!(
                        "Created new thread: {}\n\nUse /thread to see current thread info.",
                        new_thread_id
                    )))
                } else {
                    // /thread <uuid> is parsed by SubmissionParser as Submission::SwitchThread
                    // and handled in agent_loop.rs -> process_switch_thread with proper message context.
                    // This branch handles unknown /thread subcommands.
                    Ok(SubmissionResult::error(format!(
                        "Unknown /thread subcommand: {}. Use /thread (no args), /thread list, or /thread new.",
                        args.join(" ")
                    )))
                }
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

                    if self.config.multi_tenant {
                        // Multi-tenant: only persist to per-user DB settings.
                        // Do NOT call set_model() on the shared provider — that
                        // would change the default for all users. The per-request
                        // model_override in the dispatcher reads from the same
                        // "selected_model" setting and applies it per-user.
                        self.persist_selected_model(tenant, requested).await;
                        Ok(SubmissionResult::response(format!(
                            "Model preference set to: {} (per-user)",
                            requested
                        )))
                    } else {
                        match self.llm().set_model(requested) {
                            Ok(()) => {
                                // Persist the model choice so it survives restarts.
                                self.persist_selected_model(tenant, requested).await;
                                Ok(SubmissionResult::response(format!(
                                    "Switched model to: {}",
                                    requested
                                )))
                            }
                            Err(e) => Ok(SubmissionResult::error(format!(
                                "Failed to switch model: {}",
                                e
                            ))),
                        }
                    }
                }
            }

            _ => Ok(SubmissionResult::error(format!(
                "Unknown command: {}. Try /help",
                command
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

    /// Handle legacy command routing from the Router (job commands that go through
    /// process_user_input -> router -> handle_job_or_command -> here).
    pub(super) async fn handle_command(
        &self,
        session: Arc<Mutex<Session>>,
        command: &str,
        args: &[String],
        channel: &str,
        tenant: &crate::tenant::TenantCtx,
    ) -> Result<Option<String>, Error> {
        // System commands are now handled directly via Submission::SystemCommand,
        // but the router may still send us unknown /commands.
        match self
            .handle_system_command(session, command, args, channel, tenant)
            .await?
        {
            SubmissionResult::Response { content } => Ok(Some(content)),
            SubmissionResult::Ok { message } => Ok(message),
            SubmissionResult::Error { message } => Ok(Some(format!("Error: {}", message))),
            _ => Ok(None),
        }
    }

    /// Persist the selected model to the settings store (DB and/or TOML config).
    ///
    /// Best-effort: logs warnings on failure but does not propagate errors,
    /// since the in-memory model switch already succeeded.
    ///
    /// In multi-tenant mode, only the per-user DB setting is written — global
    /// .env and TOML files are shared across users and must not be mutated.
    async fn persist_selected_model(&self, tenant: &crate::tenant::TenantCtx, model: &str) {
        // 1. Persist to DB if available (per-user scoped via TenantScope).
        if let Some(store) = tenant.store() {
            let value = serde_json::Value::String(model.to_string());
            if let Err(e) = store.set_setting("selected_model", &value).await {
                tracing::warn!("Failed to persist model to DB: {}", e);
            } else {
                tracing::debug!(
                    user_id = tenant.user_id(),
                    "Persisted selected_model to DB: {}",
                    model
                );
            }
        } else {
            tracing::warn!("No database store available — model choice will not persist to DB");
        }

        // 2. In multi-tenant mode, skip .env/TOML writes — these are global
        // files shared by all users. The per-user DB setting is sufficient.
        if self.config.multi_tenant {
            return;
        }

        // 3. Update .env and TOML config file (sync I/O in spawn_blocking).
        let model_owned = model.to_string();
        let backend = self.deps.llm_backend.clone();
        if let Err(e) = tokio::task::spawn_blocking(move || {
            // 2a. Update the backend-specific model env var in ~/.ironclaw/.env.
            //
            // Env vars have the HIGHEST priority in LlmConfig::resolve_model()
            // (env var > TOML > DB > default). If the .env file has e.g.
            // NEARAI_MODEL=old-model, it shadows everything else. We must
            // update this var or the /model change is invisible on restart.
            let registry = crate::llm::ProviderRegistry::load();
            let model_env = registry.model_env_var(&backend);
            let env_var_prefix = format!("{}=", model_env);

            // Only update the .env file if the var is actually set there
            // (avoid injecting new vars the user never configured).
            let env_path = crate::bootstrap::ironclaw_env_path();
            let env_has_var = std::fs::read_to_string(&env_path)
                .ok()
                .is_some_and(|content| {
                    content.lines().any(|line| {
                        let trimmed = line.trim_start();
                        !trimmed.starts_with('#') && trimmed.starts_with(&env_var_prefix)
                    })
                });
            if env_has_var {
                if let Err(e) = crate::bootstrap::upsert_bootstrap_var(model_env, &model_owned) {
                    tracing::warn!("Failed to update {} in .env: {}", model_env, e);
                } else {
                    tracing::debug!("Updated {} in .env to {}", model_env, model_owned);
                }
            }

            // 2b. Update (or create) the TOML config file.
            //
            // The TOML overlay has higher priority than DB settings on
            // startup, so it MUST stay in sync with the DB.
            let toml_path = crate::settings::Settings::default_toml_path();
            match crate::settings::Settings::load_toml(&toml_path) {
                Ok(Some(mut settings)) => {
                    settings.selected_model = Some(model_owned);
                    if let Err(e) = settings.save_toml(&toml_path) {
                        tracing::warn!("Failed to persist model to config.toml: {}", e);
                    }
                }
                Ok(None) => {
                    // No config file yet — create one so the model choice
                    // survives restarts even when the DB is unavailable.
                    let settings = crate::settings::Settings {
                        selected_model: Some(model_owned),
                        ..Default::default()
                    };
                    if let Err(e) = settings.save_toml(&toml_path) {
                        tracing::warn!("Failed to create config.toml for model persistence: {}", e);
                    }
                }
                Err(e) => {
                    tracing::warn!("Failed to load config.toml for model persistence: {}", e);
                }
            }
        })
        .await
        {
            tracing::warn!("Model persistence task failed: {}", e);
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(dead_code)]

    use super::*;
    use crate::agent::session::{Session, Thread, ThreadState};
    use std::collections::{HashMap, HashSet, VecDeque};
    use std::sync::Arc;
    use tokio::sync::Mutex;
    use uuid::Uuid;

    fn create_test_session() -> Arc<Mutex<Session>> {
        let session = Session {
            id: Uuid::new_v4(),
            user_id: "test-user".to_string(),
            active_thread: None,
            threads: HashMap::new(),
            created_at: chrono::Utc::now(),
            last_active_at: chrono::Utc::now(),
            metadata: serde_json::Value::Null,
            auto_approved_tools: HashSet::new(),
        };
        Arc::new(Mutex::new(session))
    }

    fn add_test_thread(session: &mut Session) -> Uuid {
        let thread_id = Uuid::new_v4();
        let thread = Thread {
            id: thread_id,
            session_id: session.id,
            state: ThreadState::Idle,
            turns: vec![],
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            metadata: serde_json::Value::Null,
            pending_approval: None,
            pending_auth: None,
            pending_messages: VecDeque::new(),
        };
        session.threads.insert(thread_id, thread);
        thread_id
    }

    #[test]
    fn test_format_count() {
        assert_eq!(format_count(5, "items"), "5 items");
        assert_eq!(format_count(1500, "messages"), "1.5K messages");
        assert_eq!(format_count(2500000, "tokens"), "2.5M tokens");
    }

    #[test]
    fn test_history_pagination_args_parsing() {
        // Test default values
        let args: Vec<String> = vec![];
        let mut limit: i64 = 50;
        let mut page: i64 = 1;
        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "--limit" | "-l" => {
                    if i + 1 < args.len() {
                        limit = args[i + 1].parse().unwrap_or(50);
                        limit = limit.clamp(1, 200);
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                "--page" | "-p" => {
                    if i + 1 < args.len() {
                        page = args[i + 1].parse().unwrap_or(1);
                        page = page.clamp(1, 100);
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                _ => i += 1,
            }
        }
        assert_eq!(limit, 50);
        assert_eq!(page, 1);

        // Test custom limit
        let args: Vec<String> = vec!["--limit".to_string(), "25".to_string()];
        limit = 50;
        page = 1;
        i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "--limit" | "-l" => {
                    if i + 1 < args.len() {
                        limit = args[i + 1].parse().unwrap_or(50);
                        limit = limit.clamp(1, 200);
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                "--page" | "-p" => {
                    if i + 1 < args.len() {
                        page = args[i + 1].parse().unwrap_or(1);
                        page = page.clamp(1, 100);
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                _ => i += 1,
            }
        }
        assert_eq!(limit, 25);
        assert_eq!(page, 1);

        // Test clamping
        let args: Vec<String> = vec!["--limit".to_string(), "500".to_string()];
        limit = 50;
        i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "--limit" | "-l" => {
                    if i + 1 < args.len() {
                        limit = args[i + 1].parse().unwrap_or(50);
                        limit = limit.clamp(1, 200);
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                _ => i += 1,
            }
        }
        assert_eq!(limit, 200); // Clamped to max
    }

    #[test]
    fn test_format_history_line_without_title() {
        let id = Uuid::new_v4();
        let line = format_history_line(
            '*',
            id,
            "Idle/test-channel",
            "turns",
            5,
            "2026-03-31T20:00:00Z",
            None,
        );
        assert!(line.starts_with(&format!("* {} [Idle/test-channel] turns=5", id)));
        assert!(line.contains("updated=2026-03-31T20:00:00Z"));
    }

    #[test]
    fn test_format_history_line_with_title() {
        let id = Uuid::new_v4();
        let line = format_history_line(
            ' ',
            id,
            "Processing/telegram",
            "messages",
            42,
            "2026-03-31T19:00:00Z",
            Some("Debug build issue"),
        );
        assert!(line.contains("messages=42"));
        assert!(line.contains("— Debug build issue"));
    }

    #[test]
    fn test_format_history_line_empty_title() {
        let id = Uuid::new_v4();
        let line = format_history_line(
            ' ',
            id,
            "Idle/discord",
            "turns",
            0,
            "2026-03-31T18:00:00Z",
            Some(""),
        );
        // Empty title should be omitted
        assert!(!line.contains("—"));
    }

    #[test]
    fn test_format_count_edge_cases() {
        // Zero
        assert_eq!(format_count(0, "items"), "0 items");
        // Exact thousands
        assert_eq!(format_count(1000, "msg"), "1.0K msg");
        assert_eq!(format_count(1000000, "tok"), "1.0M tok");
        // Large numbers
        assert_eq!(format_count(1500000, "ops"), "1.5M ops");
        assert_eq!(format_count(999999, "req"), "1000.0K req");
    }

    #[test]
    fn test_format_history_line_variations() {
        let id = Uuid::new_v4();

        // Different prefixes
        let line_star =
            format_history_line('*', id, "Idle/tg", "turns", 5, "2026-01-01T00:00:00Z", None);
        assert!(line_star.starts_with("* "));

        let line_space = format_history_line(
            ' ',
            id,
            "Processing/web",
            "msgs",
            10,
            "2026-02-01T00:00:00Z",
            Some("Test"),
        );
        assert!(line_space.starts_with("  "));
        assert!(line_space.contains("— Test"));

        // Different states and channels
        let line_completed = format_history_line(
            ' ',
            id,
            "Completed/discord",
            "turns",
            100,
            "2026-03-01T00:00:00Z",
            Some("Long title here"),
        );
        assert!(line_completed.contains("Completed/discord"));
        assert!(line_completed.contains("— Long title here"));
    }

    #[test]
    fn test_format_history_line_special_characters() {
        let id = Uuid::new_v4();

        // Title with special chars
        let line = format_history_line(
            ' ',
            id,
            "Idle/tg",
            "turns",
            1,
            "2026-01-01T00:00:00Z",
            Some("Test: with \"quotes\" & symbols"),
        );
        assert!(line.contains("Test: with \"quotes\" & symbols"));

        // Empty string vs None
        let line_empty = format_history_line(
            ' ',
            id,
            "Idle/tg",
            "turns",
            1,
            "2026-01-01T00:00:00Z",
            Some(""),
        );
        let line_none =
            format_history_line(' ', id, "Idle/tg", "turns", 1, "2026-01-01T00:00:00Z", None);
        assert_eq!(line_empty, line_none);
    }
}
