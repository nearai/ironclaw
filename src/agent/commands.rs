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
use crate::error::Error;
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
                match self.handle_command(&command, &args).await? {
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
                // Show summary of all jobs
                let summary = self.context_manager.summary_for(user_id).await;
                Ok(format!(
                    "Jobs summary:\n  Total: {}\n  In Progress: {}\n  Completed: {}\n  Failed: {}\n  Stuck: {}",
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

        Ok(format!("Job {} has been cancelled.", job_id))
    }

    async fn handle_list_jobs(
        &self,
        user_id: &str,
        _filter: Option<String>,
    ) -> Result<String, Error> {
        let jobs = self.context_manager.all_jobs_for(user_id).await;

        if jobs.is_empty() {
            return Ok("No jobs found.".to_string());
        }

        let mut output = String::from("Jobs:\n");
        for job_id in jobs {
            if let Ok(ctx) = self.context_manager.get_context(job_id).await
                && ctx.user_id == user_id
            {
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

    pub(super) async fn process_reasoning(
        &self,
        session: Arc<Mutex<Session>>,
        thread_id: Uuid,
        arg: Option<String>,
    ) -> Result<SubmissionResult, Error> {
        let sess = session.lock().await;
        let thread = sess
            .threads
            .get(&thread_id)
            .ok_or_else(|| Error::from(crate::error::JobError::NotFound { id: thread_id }))?;

        let format_turn = |turn: &crate::agent::session::Turn| -> String {
            if turn.tool_calls.is_empty() {
                return format!("  ─ Turn {} had no tool calls.", turn.turn_number + 1);
            }

            let mut out = format!("  ┄ Reasoning — turn {}\n", turn.turn_number + 1);
            if let Some(narrative) = turn.narrative.as_deref().map(str::trim)
                && !narrative.is_empty()
            {
                out.push_str(&format!("  Narrative: \"{}\"\n", narrative));
            }

            let mut groups: std::collections::BTreeMap<
                Option<usize>,
                Vec<&crate::agent::session::TurnToolCall>,
            > = std::collections::BTreeMap::new();
            for call in &turn.tool_calls {
                groups.entry(call.parallel_group).or_default().push(call);
            }

            for (group, calls) in groups {
                if let Some(g) = group
                    && calls.len() > 1
                {
                    out.push_str(&format!("\n  ┄ [parallel batch {}]\n", g));
                }

                for call in calls {
                    let prefix = if call.parallel_group.is_some() && turn.tool_calls.len() > 1 {
                        "  ┄   ↳"
                    } else {
                        "  ┄"
                    };
                    out.push_str(&format!("{} {}\n", prefix, call.name));
                    out.push_str(&format!("    rationale: \"{}\"\n", call.rationale.trim()));

                    let params = crate::tools::redaction::redact_sensitive_json(&call.parameters)
                        .to_string();
                    let params_preview = if params.chars().count() > 200 {
                        let truncated: String = params.chars().take(200).collect();
                        format!("{}...", truncated)
                    } else {
                        params
                    };
                    out.push_str(&format!("    params:    {}\n", params_preview));

                    let outcome = if let Some(err) = &call.error {
                        format!("error ({})", err)
                    } else if call.result.is_some() {
                        "success".to_string()
                    } else {
                        "pending".to_string()
                    };
                    out.push_str(&format!("    outcome:   {}\n", outcome));
                }
            }

            out.trim_end().to_string()
        };

        let response = match arg.as_deref().map(str::trim) {
            None => {
                if let Some(turn) = thread.turns.last() {
                    format_turn(turn)
                } else {
                    "  ✗ No turns in this thread yet.".to_string()
                }
            }
            Some("all") => {
                if thread.turns.is_empty() {
                    "  ✗ No turns in this thread yet.".to_string()
                } else {
                    thread
                        .turns
                        .iter()
                        .map(format_turn)
                        .collect::<Vec<_>>()
                        .join("\n\n")
                }
            }
            Some(raw_turn) => {
                let turn_number = match raw_turn.parse::<usize>() {
                    Ok(n) if n > 0 => n,
                    _ => {
                        return Ok(SubmissionResult::error(
                            "Usage: /reasoning [N|all] (N must be >= 1)",
                        ));
                    }
                };

                if let Some(turn) = thread
                    .turns
                    .iter()
                    .find(|t| t.turn_number + 1 == turn_number)
                {
                    format_turn(turn)
                } else {
                    format!(
                        "  ✗ No reasoning data for turn {} in this thread.",
                        turn_number
                    )
                }
            }
        };

        Ok(SubmissionResult::response(response))
    }

    /// Handle system commands that bypass thread-state checks entirely.
    pub(super) async fn handle_system_command(
        &self,
        command: &str,
        args: &[String],
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
                "  /reasoning [N|all] Show reasoning summaries\n",
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
        command: &str,
        args: &[String],
    ) -> Result<Option<String>, Error> {
        // System commands are now handled directly via Submission::SystemCommand,
        // but the router may still send us unknown /commands.
        match self.handle_system_command(command, args).await? {
            SubmissionResult::Response { content } => Ok(Some(content)),
            SubmissionResult::Ok { message } => Ok(message),
            SubmissionResult::Error { message } => Ok(Some(format!("Error: {}", message))),
            _ => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use async_trait::async_trait;
    use rust_decimal::Decimal;
    use tokio::sync::Mutex;
    use uuid::Uuid;

    use crate::agent::Agent;
    use crate::agent::agent_loop::AgentDeps;
    use crate::agent::cost_guard::{CostGuard, CostGuardConfig};
    use crate::agent::session::Session;
    use crate::agent::submission::SubmissionResult;
    use crate::channels::ChannelManager;
    use crate::config::{AgentConfig, SafetyConfig, SkillsConfig};
    use crate::context::ContextManager;
    use crate::hooks::HookRegistry;
    use crate::llm::{
        CompletionRequest, CompletionResponse, FinishReason, LlmProvider, ToolCompletionRequest,
        ToolCompletionResponse,
    };
    use crate::safety::SafetyLayer;
    use crate::tools::ToolRegistry;

    struct StaticLlmProvider;

    #[async_trait]
    impl LlmProvider for StaticLlmProvider {
        fn model_name(&self) -> &str {
            "static-mock"
        }

        fn cost_per_token(&self) -> (Decimal, Decimal) {
            (Decimal::ZERO, Decimal::ZERO)
        }

        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, crate::error::LlmError> {
            Ok(CompletionResponse {
                content: "ok".to_string(),
                input_tokens: 0,
                output_tokens: 0,
                finish_reason: FinishReason::Stop,
            })
        }

        async fn complete_with_tools(
            &self,
            _request: ToolCompletionRequest,
        ) -> Result<ToolCompletionResponse, crate::error::LlmError> {
            Ok(ToolCompletionResponse {
                content: Some("ok".to_string()),
                tool_calls: Vec::new(),
                input_tokens: 0,
                output_tokens: 0,
                finish_reason: FinishReason::Stop,
            })
        }
    }

    fn make_test_agent() -> Agent {
        let deps = AgentDeps {
            store: None,
            llm: Arc::new(StaticLlmProvider),
            cheap_llm: None,
            safety: Arc::new(SafetyLayer::new(&SafetyConfig {
                max_output_length: 100_000,
                injection_check_enabled: true,
            })),
            tools: Arc::new(ToolRegistry::new()),
            workspace: None,
            extension_manager: None,
            skill_registry: None,
            skill_catalog: None,
            skills_config: SkillsConfig::default(),
            hooks: Arc::new(HookRegistry::new()),
            cost_guard: Arc::new(CostGuard::new(CostGuardConfig::default())),
        };

        Agent::new(
            AgentConfig {
                name: "test-agent".to_string(),
                max_parallel_jobs: 1,
                job_timeout: Duration::from_secs(60),
                stuck_threshold: Duration::from_secs(60),
                repair_check_interval: Duration::from_secs(30),
                max_repair_attempts: 1,
                use_planning: false,
                session_idle_timeout: Duration::from_secs(300),
                allow_local_tools: false,
                max_cost_per_day_cents: None,
                max_actions_per_hour: None,
                max_tool_iterations: 50,
                auto_approve_tools: false,
            },
            deps,
            Arc::new(ChannelManager::new()),
            None,
            None,
            None,
            Some(Arc::new(ContextManager::new(1))),
            None,
        )
    }

    fn session_with_reasoning_turn() -> (Arc<Mutex<Session>>, Uuid) {
        let mut sess = Session::new("user-test");
        let thread_id = sess.create_thread().id;

        {
            let thread = sess
                .threads
                .get_mut(&thread_id)
                .expect("thread should exist");
            let turn = thread.start_turn("hello");
            turn.record_tool_call(
                "echo",
                serde_json::json!({"message": "hi"}),
                "confirm greeting".to_string(),
                None,
            );
            turn.record_tool_result(serde_json::json!("hi"));
            thread.complete_turn("done");
            thread.updated_at = thread
                .updated_at
                .checked_add_signed(chrono::TimeDelta::seconds(1))
                .expect("valid timestamp shift");
        }

        (Arc::new(Mutex::new(sess)), thread_id)
    }

    #[tokio::test]
    async fn test_process_reasoning_out_of_range_turn() {
        let agent = make_test_agent();
        let (session, thread_id) = session_with_reasoning_turn();

        let result = agent
            .process_reasoning(session, thread_id, Some("99".to_string()))
            .await
            .expect("process_reasoning should succeed");

        match result {
            SubmissionResult::Response { content } => {
                assert!(content.contains("No reasoning data for turn 99"));
            }
            _ => panic!("expected response submission result"),
        }
    }

    #[tokio::test]
    async fn test_process_reasoning_invalid_turn_usage_error() {
        let agent = make_test_agent();
        let (session, thread_id) = session_with_reasoning_turn();

        let result = agent
            .process_reasoning(session, thread_id, Some("0".to_string()))
            .await
            .expect("process_reasoning should succeed");

        match result {
            SubmissionResult::Error { message } => {
                assert_eq!(message, "Usage: /reasoning [N|all] (N must be >= 1)");
            }
            _ => panic!("expected error submission result"),
        }
    }

    #[tokio::test]
    async fn test_process_reasoning_redacts_sensitive_parameters_in_output() {
        let agent = make_test_agent();
        let mut sess = Session::new("user-test");
        let thread_id = sess.create_thread().id;

        {
            let thread = sess
                .threads
                .get_mut(&thread_id)
                .expect("thread should exist");
            let turn = thread.start_turn("hello");
            turn.record_tool_call(
                "http",
                serde_json::json!({
                    "headers": { "Authorization": "Bearer abc" },
                    "token": "secret-token"
                }),
                "call http".to_string(),
                None,
            );
            turn.record_tool_result(serde_json::json!("ok"));
            thread.complete_turn("done");
        }

        let session = Arc::new(Mutex::new(sess));
        let result = agent
            .process_reasoning(session, thread_id, None)
            .await
            .expect("process_reasoning should succeed");

        match result {
            SubmissionResult::Response { content } => {
                assert!(content.contains("[REDACTED]"));
                assert!(!content.contains("secret-token"));
                assert!(!content.contains("Bearer abc"));
            }
            _ => panic!("expected response submission result"),
        }
    }
}
