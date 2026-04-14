//! Job and sandbox API handlers.

use std::collections::HashSet;
use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde::Deserialize;
use uuid::Uuid;

use crate::channels::web::auth::AuthenticatedUser;
use crate::channels::web::server::GatewayState;
use crate::channels::web::types::*;
use crate::orchestrator::job_manager::{ContainerJobManager, JobCreationParams, JobMode};
use crate::ownership::Owned;

fn db_error(context: &str, e: impl std::fmt::Display) -> (StatusCode, String) {
    tracing::error!(%e, context, "Database error in jobs handler");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "Internal database error".to_string(),
    )
}

async fn resolve_sandbox_restart_mode(
    store: &dyn crate::db::Database,
    stored_mode: &str,
    user_id: &str,
) -> Result<(JobMode, Option<crate::config::acp::AcpAgentConfig>), crate::config::acp::AcpConfigError>
{
    if stored_mode == "claude_code" {
        return Ok((JobMode::ClaudeCode, None));
    }

    if let Some(agent_name) = stored_mode.strip_prefix("acp:") {
        let agent =
            crate::config::acp::get_enabled_acp_agent_for_user(Some(store), user_id, agent_name)
                .await?;
        return Ok((JobMode::Acp, Some(agent)));
    }

    if stored_mode == "acp" {
        return Err(crate::config::acp::AcpConfigError::InvalidConfig {
            reason: "legacy ACP jobs without an agent name cannot be restarted".to_string(),
        });
    }

    Ok((JobMode::Worker, None))
}

/// Reject restart requests for modes disabled since the job was created.
fn check_mode_enabled(mode: JobMode, jm: &ContainerJobManager) -> Result<(), (StatusCode, String)> {
    if jm.is_mode_enabled(mode) {
        Ok(())
    } else {
        let env_hint = match mode {
            JobMode::ClaudeCode => " Set CLAUDE_CODE_ENABLED=true to re-enable.",
            JobMode::Acp => " Set ACP_ENABLED=true to re-enable.",
            JobMode::Worker => "", // Worker is always enabled; unreachable in practice.
        };
        Err((
            StatusCode::CONFLICT,
            format!("{mode} mode is no longer enabled.{env_hint}"),
        ))
    }
}

pub async fn jobs_list_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
) -> Result<Json<JobListResponse>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    let mut jobs: Vec<JobInfo> = Vec::new();
    let mut seen_ids: HashSet<Uuid> = HashSet::new();

    // Fetch sandbox jobs scoped to this user.
    match store.list_sandbox_jobs_for_user(&user.user_id).await {
        Ok(sandbox_jobs) => {
            for j in &sandbox_jobs {
                let ui_state = match j.status.as_str() {
                    "creating" => "pending",
                    "running" => "in_progress",
                    s => s,
                };
                seen_ids.insert(j.id);
                jobs.push(JobInfo {
                    id: j.id,
                    title: j.task.clone(),
                    state: ui_state.to_string(),
                    user_id: j.user_id.clone(),
                    created_at: j.created_at.to_rfc3339(),
                    started_at: j.started_at.map(|dt| dt.to_rfc3339()),
                });
            }
        }
        Err(e) => {
            tracing::warn!("Failed to list sandbox jobs: {}", e);
        }
    }

    // Fetch agent (non-sandbox) jobs scoped to this user, deduplicating by ID.
    match store.list_agent_jobs_for_user(&user.user_id).await {
        Ok(agent_jobs) => {
            for j in &agent_jobs {
                if seen_ids.contains(&j.id) {
                    continue;
                }
                jobs.push(JobInfo {
                    id: j.id,
                    title: j.title.clone(),
                    state: j.status.clone(),
                    user_id: j.user_id.clone(),
                    created_at: j.created_at.to_rfc3339(),
                    started_at: j.started_at.map(|dt| dt.to_rfc3339()),
                });
            }
        }
        Err(e) => {
            tracing::warn!("Failed to list agent jobs: {}", e);
        }
    }

    // Most recent first.
    jobs.sort_by(|a, b| b.created_at.cmp(&a.created_at));

    Ok(Json(JobListResponse { jobs }))
}

pub async fn jobs_summary_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
) -> Result<Json<JobSummaryResponse>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    let mut total = 0;
    let mut pending = 0;
    let mut in_progress = 0;
    let mut completed = 0;
    let mut failed = 0;
    let mut stuck = 0;

    // Sandbox job counts scoped to this user.
    match store.sandbox_job_summary_for_user(&user.user_id).await {
        Ok(s) => {
            total += s.total;
            pending += s.creating;
            in_progress += s.running;
            completed += s.completed;
            failed += s.failed + s.interrupted;
        }
        Err(e) => {
            tracing::warn!("Failed to fetch sandbox job summary: {}", e);
        }
    }

    // Agent job counts scoped to this user.
    match store.agent_job_summary_for_user(&user.user_id).await {
        Ok(s) => {
            total += s.total;
            pending += s.pending;
            in_progress += s.in_progress;
            completed += s.completed;
            failed += s.failed;
            stuck += s.stuck;
        }
        Err(e) => {
            tracing::warn!("Failed to fetch agent job summary: {}", e);
        }
    }

    Ok(Json(JobSummaryResponse {
        total,
        pending,
        in_progress,
        completed,
        failed,
        stuck,
    }))
}

pub async fn jobs_detail_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(id): Path<String>,
) -> Result<Json<JobDetailResponse>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    let job_id = Uuid::parse_str(&id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid job ID".to_string()))?;

    // Try sandbox job from DB first.
    match store.get_sandbox_job(job_id).await {
        Ok(Some(job)) => {
            if !job.is_owned_by(&user.user_id) {
                return Err((StatusCode::NOT_FOUND, "Job not found".to_string()));
            }
            let browse_id = std::path::Path::new(&job.project_dir)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| job.id.to_string());

            let ui_state = match job.status.as_str() {
                "creating" => "pending",
                "running" => "in_progress",
                s => s,
            };

            let elapsed_secs = job.started_at.map(|start| {
                let end = job.completed_at.unwrap_or_else(chrono::Utc::now);
                (end - start).num_seconds().max(0) as u64
            });

            // Synthesize transitions from timestamps.
            let mut transitions = Vec::new();
            if let Some(started) = job.started_at {
                transitions.push(TransitionInfo {
                    from: "creating".to_string(),
                    to: "running".to_string(),
                    timestamp: started.to_rfc3339(),
                    reason: None,
                });
            }
            if let Some(completed) = job.completed_at {
                transitions.push(TransitionInfo {
                    from: "running".to_string(),
                    to: job.status.clone(),
                    timestamp: completed.to_rfc3339(),
                    reason: job.failure_reason.clone(),
                });
            }

            let mode = store.get_sandbox_job_mode(job.id).await.ok().flatten();
            let supports_prompts = mode
                .as_deref()
                .is_some_and(|m| m == "claude_code" || m.starts_with("acp"));

            return Ok(Json(JobDetailResponse {
                id: job.id,
                title: job.task.clone(),
                description: String::new(),
                state: ui_state.to_string(),
                user_id: job.user_id.clone(),
                created_at: job.created_at.to_rfc3339(),
                started_at: job.started_at.map(|dt| dt.to_rfc3339()),
                completed_at: job.completed_at.map(|dt| dt.to_rfc3339()),
                elapsed_secs,
                project_dir: Some(job.project_dir.clone()),
                browse_url: Some(format!("/projects/{}/", browse_id)),
                job_mode: mode.filter(|m| m != "worker"),
                transitions,
                can_restart: state.job_manager.is_some(),
                can_prompt: supports_prompts && state.prompt_queue.is_some(),
                job_kind: Some("sandbox".to_string()),
            }));
        }
        Ok(None) => {}
        Err(e) => {
            return Err(db_error("jobs_handler", e));
        }
    }

    // Fall back to agent job from DB.
    match store.get_job(job_id).await {
        Ok(Some(ctx)) => {
            if !ctx.is_owned_by(&user.user_id) {
                return Err((StatusCode::NOT_FOUND, "Job not found".to_string()));
            }
            let elapsed_secs = ctx.started_at.map(|start| {
                let end = ctx.completed_at.unwrap_or_else(chrono::Utc::now);
                (end - start).num_seconds().max(0) as u64
            });

            // Build transitions from the job's state transition history.
            let transitions: Vec<TransitionInfo> = ctx
                .transitions
                .iter()
                .map(|t| TransitionInfo {
                    from: t.from.to_string(),
                    to: t.to.to_string(),
                    timestamp: t.timestamp.to_rfc3339(),
                    reason: t.reason.clone(),
                })
                .collect();

            // Only show prompt bar for jobs that have a running worker (Pending/InProgress).
            // Stuck jobs have no active worker loop, so messages would be silently dropped.
            let is_promptable = matches!(
                ctx.state,
                crate::context::JobState::Pending | crate::context::JobState::InProgress
            );
            Ok(Json(JobDetailResponse {
                id: ctx.job_id,
                title: ctx.title.clone(),
                description: ctx.description.clone(),
                state: ctx.state.to_string(),
                user_id: ctx.user_id.clone(),
                created_at: ctx.created_at.to_rfc3339(),
                started_at: ctx.started_at.map(|dt| dt.to_rfc3339()),
                completed_at: ctx.completed_at.map(|dt| dt.to_rfc3339()),
                elapsed_secs,
                project_dir: None,
                browse_url: None,
                job_mode: None,
                transitions,
                can_restart: state.scheduler.is_some(),
                can_prompt: is_promptable && state.scheduler.is_some(),
                job_kind: Some("agent".to_string()),
            }))
        }
        Ok(None) => Err((StatusCode::NOT_FOUND, "Job not found".to_string())),
        Err(e) => Err(db_error("jobs_handler", e)),
    }
}

pub async fn jobs_cancel_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let job_id = Uuid::parse_str(&id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid job ID".to_string()))?;

    // Try sandbox job cancellation.
    if let Some(ref store) = state.store {
        match store.get_sandbox_job(job_id).await {
            Ok(Some(job)) => {
                if !job.is_owned_by(&user.user_id) {
                    return Err((StatusCode::NOT_FOUND, "Job not found".to_string()));
                }
                if job.status == "running" || job.status == "creating" {
                    if let Some(ref jm) = state.job_manager
                        && let Err(e) = jm.stop_job(job_id).await
                    {
                        tracing::warn!(job_id = %job_id, error = %e, "Failed to stop container during cancellation");
                    }
                    store
                        .update_sandbox_job_status(
                            job_id,
                            "failed",
                            Some(false),
                            Some("Cancelled by user"),
                            None,
                            Some(chrono::Utc::now()),
                        )
                        .await
                        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
                }
                return Ok(Json(serde_json::json!({
                    "status": "cancelled",
                    "job_id": job_id,
                })));
            }
            Ok(None) => {}
            Err(e) => {
                return Err(db_error("jobs_handler", e));
            }
        }
    }

    // Fall back to agent job cancellation: stop the worker via the scheduler
    // (which updates the in-memory ContextManager AND aborts the task handle),
    // then persist the status to the DB as a fallback.
    if let Some(ref store) = state.store {
        match store.get_job(job_id).await {
            Ok(Some(job)) => {
                if !job.is_owned_by(&user.user_id) {
                    return Err((StatusCode::NOT_FOUND, "Job not found".to_string()));
                }
                if job.state.is_active() {
                    // Try to stop via scheduler (aborts the worker task + updates
                    // in-memory ContextManager). This is best-effort — the job may
                    // not be in the scheduler map if it already finished.
                    if let Some(ref slot) = state.scheduler
                        && let Some(ref scheduler) = *slot.read().await
                    {
                        let _ = scheduler.stop(job_id).await;
                    }

                    // Always persist cancellation to the DB so the state is
                    // consistent even if the scheduler wasn't available or the
                    // job wasn't in its in-memory map.
                    store
                        .update_job_status(
                            job_id,
                            crate::context::JobState::Cancelled,
                            Some("Cancelled by user"),
                        )
                        .await
                        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
                }
                return Ok(Json(serde_json::json!({
                    "status": "cancelled",
                    "job_id": job_id,
                })));
            }
            Ok(None) => {}
            Err(e) => {
                return Err(db_error("jobs_handler", e));
            }
        }
    }

    Err((StatusCode::NOT_FOUND, "Job not found".to_string()))
}

pub async fn jobs_restart_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    let old_job_id = Uuid::parse_str(&id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid job ID".to_string()))?;

    // Try sandbox job restart first.
    match store.get_sandbox_job(old_job_id).await {
        Ok(Some(old_job)) => {
            if !old_job.is_owned_by(&user.user_id) {
                return Err((StatusCode::NOT_FOUND, "Job not found".to_string()));
            }
            if old_job.status != "interrupted" && old_job.status != "failed" {
                return Err((
                    StatusCode::CONFLICT,
                    format!("Cannot restart job in state '{}'", old_job.status),
                ));
            }

            let jm = state.job_manager.as_ref().ok_or((
                StatusCode::SERVICE_UNAVAILABLE,
                "Sandbox not enabled".to_string(),
            ))?;

            // Enrich the task with failure context.
            let task = if let Some(ref reason) = old_job.failure_reason {
                format!(
                    "Previous attempt failed: {}. Retry: {}",
                    reason, old_job.task
                )
            } else {
                old_job.task.clone()
            };

            let new_job_id = Uuid::new_v4();
            let now = chrono::Utc::now();

            let stored_mode = store
                .get_sandbox_job_mode(old_job_id)
                .await
                .map_err(|e| db_error("jobs_restart_handler", e))?
                .unwrap_or_default();

            let (mode, acp_agent) =
                resolve_sandbox_restart_mode(store.as_ref(), &stored_mode, &old_job.user_id)
                    .await
                    .map_err(|e| (StatusCode::CONFLICT, format!("Cannot restart job: {}", e)))?;
            check_mode_enabled(mode, jm.as_ref())?;

            // Carry the original mcp_servers filter and max_iterations cap
            // through the restart. Without this the restarted job would mount
            // the *full* MCP master config (the opposite of the original
            // filter) and run with the default worker iteration cap, silently
            // diverging from the original job's constraints.
            let restart_mcp_servers = old_job.mcp_servers.clone();
            let restart_max_iterations = old_job.max_iterations;

            let record = crate::history::SandboxJobRecord {
                id: new_job_id,
                task: task.clone(),
                status: "creating".to_string(),
                user_id: old_job.user_id.clone(),
                project_dir: old_job.project_dir.clone(),
                success: None,
                failure_reason: None,
                created_at: now,
                started_at: None,
                completed_at: None,
                credential_grants_json: old_job.credential_grants_json.clone(),
                mcp_servers: restart_mcp_servers.clone(),
                max_iterations: restart_max_iterations,
            };
            store
                .save_sandbox_job(&record)
                .await
                .map_err(|e| db_error("jobs_restart_handler", e))?;

            if mode != JobMode::Worker {
                let mode_str = if mode == JobMode::Acp {
                    format!(
                        "acp:{}",
                        acp_agent
                            .as_ref()
                            .map(|agent| agent.name.as_str())
                            .unwrap_or_default()
                    )
                } else {
                    mode.as_str().to_string()
                };
                store
                    .update_sandbox_job_mode(new_job_id, &mode_str)
                    .await
                    .map_err(|e| db_error("jobs_restart_handler", e))?;
            }

            let credential_grants: Vec<crate::orchestrator::auth::CredentialGrant> =
                serde_json::from_str(&old_job.credential_grants_json).unwrap_or_else(|e| {
                    tracing::warn!(
                        job_id = %old_job.id,
                        "Failed to deserialize credential grants from stored job: {}. \
                         Restarted job will have no credentials.",
                        e
                    );
                    vec![]
                });

            // Load the master MCP config for the original job's user so the
            // restart re-creates the same MCP environment as the initial run.
            // Without this the orchestrator would fall back to no mount even
            // when the user has servers configured (staging-regressions
            // issue 3 — the orchestrator used to read from a hardcoded host
            // file path that bootstrap moves into the DB on first run).
            let master_mcp_config = crate::tools::mcp::config::load_master_mcp_config_value(
                store.as_ref(),
                &old_job.user_id,
            )
            .await;

            let project_dir = std::path::PathBuf::from(&old_job.project_dir);
            let create_result = jm
                .create_job(
                    new_job_id,
                    &task,
                    Some(project_dir),
                    mode,
                    JobCreationParams {
                        credential_grants,
                        mcp_servers: restart_mcp_servers,
                        max_iterations: restart_max_iterations,
                        acp_agent,
                        master_mcp_config,
                    },
                )
                .await;
            let _token = match create_result {
                Ok(token) => token,
                Err(e) => {
                    let error_text = e.to_string();
                    let _ = store
                        .update_sandbox_job_status(
                            new_job_id,
                            "failed",
                            Some(false),
                            Some(error_text.as_str()),
                            None,
                            Some(chrono::Utc::now()),
                        )
                        .await;
                    return Err((
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("Failed to create container: {}", error_text),
                    ));
                }
            };

            store
                .update_sandbox_job_status(new_job_id, "running", None, None, Some(now), None)
                .await
                .map_err(|e| db_error("jobs_restart_handler", e))?;

            return Ok(Json(serde_json::json!({
                "status": "restarted",
                "old_job_id": old_job_id,
                "new_job_id": new_job_id,
            })));
        }
        Ok(None) => {}
        Err(e) => {
            return Err(db_error("jobs_restart_handler", e));
        }
    }

    // Try agent job restart: dispatch a new job via the scheduler.
    match store.get_job(old_job_id).await {
        Ok(Some(old_job)) => {
            if !old_job.is_owned_by(&user.user_id) {
                return Err((StatusCode::NOT_FOUND, "Job not found".to_string()));
            }
            if old_job.state.is_active() {
                return Err((
                    StatusCode::CONFLICT,
                    format!("Cannot restart job in state '{}'", old_job.state),
                ));
            }

            let slot = state.scheduler.as_ref().ok_or((
                StatusCode::SERVICE_UNAVAILABLE,
                "Scheduler not available".to_string(),
            ))?;
            let scheduler_guard = slot.read().await;
            let scheduler = scheduler_guard.as_ref().ok_or((
                StatusCode::SERVICE_UNAVAILABLE,
                "Agent not started yet".to_string(),
            ))?;

            // Look up failure reason (O(1) point lookup).
            let failure_reason = store
                .get_agent_job_failure_reason(old_job_id)
                .await
                .ok()
                .flatten()
                .unwrap_or_default();

            let title = if !failure_reason.is_empty() {
                format!(
                    "Previous attempt failed: {}. Retry: {}",
                    failure_reason, old_job.title
                )
            } else {
                old_job.title.clone()
            };

            let new_job_id = scheduler
                .dispatch_job(&old_job.user_id, &title, &old_job.description, None)
                .await
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

            Ok(Json(serde_json::json!({
                "status": "restarted",
                "old_job_id": old_job_id,
                "new_job_id": new_job_id,
            })))
        }
        Ok(None) => Err((StatusCode::NOT_FOUND, "Job not found".to_string())),
        Err(e) => Err(db_error("jobs_handler", e)),
    }
}

/// Submit a follow-up prompt to a running job.
///
/// Routes to the appropriate backend:
/// - Claude Code sandbox jobs → prompt queue (polled by the bridge)
/// - Agent (non-sandbox) jobs → WorkerMessage injection via scheduler
/// - Worker-mode sandbox jobs → not supported (no mechanism to inject)
pub async fn jobs_prompt_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let job_id: uuid::Uuid = id
        .parse()
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid job ID".to_string()))?;

    let content = body
        .get("content")
        .and_then(|v| v.as_str())
        .ok_or((
            StatusCode::BAD_REQUEST,
            "Missing 'content' field".to_string(),
        ))?
        .to_string();

    let done = body.get("done").and_then(|v| v.as_bool()).unwrap_or(false);

    // Try sandbox job path first: verify ownership, then route to Claude Code or reject.
    if let Some(ref s) = state.store
        && let Ok(Some(sandbox_job)) = s.get_sandbox_job(job_id).await
    {
        // Verify ownership.
        if !sandbox_job.is_owned_by(&user.user_id) {
            return Err((StatusCode::NOT_FOUND, "Job not found".to_string()));
        }

        // It's a sandbox job. Check if Claude Code or ACP mode (both support follow-up prompts).
        let mode = s.get_sandbox_job_mode(job_id).await.ok().flatten();
        if mode
            .as_deref()
            .is_some_and(|m| m == "claude_code" || m.starts_with("acp"))
        {
            let prompt_queue = state.prompt_queue.as_ref().ok_or((
                StatusCode::NOT_IMPLEMENTED,
                "Follow-up prompts are not configured".to_string(),
            ))?;
            let prompt = crate::orchestrator::api::PendingPrompt { content, done };
            {
                let mut queue = prompt_queue.lock().await;
                queue.entry(job_id).or_default().push_back(prompt);
            }
            return Ok(Json(serde_json::json!({
                "status": "queued",
                "job_id": job_id.to_string(),
            })));
        } else {
            return Err((
                StatusCode::NOT_IMPLEMENTED,
                "Follow-up prompts are not supported for worker-mode sandbox jobs".to_string(),
            ));
        }
    }

    // Try agent job path: verify ownership, then send via scheduler.
    if let Some(ref store) = state.store {
        match store.get_job(job_id).await {
            Ok(Some(agent_job)) => {
                if !agent_job.is_owned_by(&user.user_id) {
                    return Err((StatusCode::NOT_FOUND, "Job not found".to_string()));
                }
            }
            Ok(None) => {
                return Err((StatusCode::NOT_FOUND, "Job not found".to_string()));
            }
            Err(e) => {
                return Err(db_error("jobs_handler", e));
            }
        }
    }

    let slot = state.scheduler.as_ref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "Agent job prompts require the scheduler to be configured".to_string(),
    ))?;
    let scheduler_guard = slot.read().await;
    if let Some(ref scheduler) = *scheduler_guard
        && scheduler.is_running(job_id).await
    {
        scheduler
            .send_message(job_id, content)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        return Ok(Json(serde_json::json!({
            "status": "sent",
            "job_id": job_id.to_string(),
        })));
    }

    Err((
        StatusCode::NOT_FOUND,
        "Job not found or not running".to_string(),
    ))
}

/// Load persisted job events for a job (for history replay on page open).
pub async fn jobs_events_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::NOT_IMPLEMENTED,
        "Database not available".to_string(),
    ))?;

    let job_id: uuid::Uuid = id
        .parse()
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid job ID".to_string()))?;

    // Verify ownership before returning events (check both sandbox and agent jobs).
    let is_owner = match store.get_sandbox_job(job_id).await {
        Ok(Some(job)) => job.is_owned_by(&user.user_id),
        Ok(None) => {
            // Fall back to agent job ownership check.
            match store.get_job(job_id).await {
                Ok(Some(ctx)) => ctx.is_owned_by(&user.user_id),
                _ => false,
            }
        }
        Err(e) => {
            return Err(db_error("jobs_events_handler", e));
        }
    };
    if !is_owner {
        return Err((StatusCode::NOT_FOUND, "Job not found".to_string()));
    }

    let events = store
        .list_job_events(job_id, None)
        .await
        .map_err(|e| db_error("jobs_events_handler", e))?;

    let events_json: Vec<serde_json::Value> = events
        .into_iter()
        .map(|e| {
            serde_json::json!({
                "id": e.id,
                "event_type": e.event_type,
                "data": e.data,
                "created_at": e.created_at.to_rfc3339(),
            })
        })
        .collect();

    Ok(Json(serde_json::json!({
        "job_id": job_id.to_string(),
        "events": events_json,
    })))
}

// --- Project file handlers for sandbox jobs ---

#[derive(Deserialize)]
pub struct FilePathQuery {
    pub path: Option<String>,
}

pub async fn job_files_list_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(id): Path<String>,
    Query(query): Query<FilePathQuery>,
) -> Result<Json<ProjectFilesResponse>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    let job_id = Uuid::parse_str(&id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid job ID".to_string()))?;

    let job = store
        .get_sandbox_job(job_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Job not found".to_string()))?;

    if !job.is_owned_by(&user.user_id) {
        return Err((StatusCode::NOT_FOUND, "Job not found".to_string()));
    }

    let base = std::path::PathBuf::from(&job.project_dir);
    let rel_path = query.path.as_deref().unwrap_or("");
    let target = base.join(rel_path);

    // Path traversal guard.
    let canonical = target
        .canonicalize()
        .map_err(|_| (StatusCode::NOT_FOUND, "Path not found".to_string()))?;
    let base_canonical = base
        .canonicalize()
        .map_err(|_| (StatusCode::NOT_FOUND, "Project dir not found".to_string()))?;
    if !canonical.starts_with(&base_canonical) {
        return Err((StatusCode::FORBIDDEN, "Forbidden".to_string()));
    }

    let mut entries = Vec::new();
    let mut read_dir = tokio::fs::read_dir(&canonical)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "Cannot read directory".to_string()))?;

    while let Ok(Some(entry)) = read_dir.next_entry().await {
        let name = entry.file_name().to_string_lossy().to_string();
        let is_dir = entry
            .file_type()
            .await
            .map(|ft| ft.is_dir())
            .unwrap_or(false);
        let rel = if rel_path.is_empty() {
            name.clone()
        } else {
            format!("{}/{}", rel_path, name)
        };
        entries.push(ProjectFileEntry {
            name,
            path: rel,
            is_dir,
        });
    }

    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.cmp(&b.name)));

    Ok(Json(ProjectFilesResponse { entries }))
}

pub async fn job_files_read_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(id): Path<String>,
    Query(query): Query<FilePathQuery>,
) -> Result<Json<ProjectFileReadResponse>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    let job_id = Uuid::parse_str(&id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid job ID".to_string()))?;

    let job = store
        .get_sandbox_job(job_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Job not found".to_string()))?;

    if !job.is_owned_by(&user.user_id) {
        return Err((StatusCode::NOT_FOUND, "Job not found".to_string()));
    }

    let path = query.path.as_deref().ok_or((
        StatusCode::BAD_REQUEST,
        "path parameter required".to_string(),
    ))?;

    let base = std::path::PathBuf::from(&job.project_dir);
    let file_path = base.join(path);

    let canonical = file_path
        .canonicalize()
        .map_err(|_| (StatusCode::NOT_FOUND, "File not found".to_string()))?;
    let base_canonical = base
        .canonicalize()
        .map_err(|_| (StatusCode::NOT_FOUND, "Project dir not found".to_string()))?;
    if !canonical.starts_with(&base_canonical) {
        return Err((StatusCode::FORBIDDEN, "Forbidden".to_string()));
    }

    let content = tokio::fs::read_to_string(&canonical)
        .await
        .map_err(|_| (StatusCode::NOT_FOUND, "Cannot read file".to_string()))?;

    Ok(Json(ProjectFileReadResponse {
        path: path.to_string(),
        content,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::orchestrator::TokenStore;
    use crate::orchestrator::job_manager::ContainerJobConfig;

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn sandbox_restart_mode_uses_original_job_owner_scope() {
        let (db, _tmp) = crate::testing::test_db().await;

        let mut agents = crate::config::acp::AcpAgentsFile::default();
        agents.upsert(crate::config::acp::AcpAgentConfig::new(
            "codex",
            "codex",
            vec!["acp".into()],
            std::collections::HashMap::new(),
        ));
        crate::config::acp::save_acp_agents_for_user(Some(db.as_ref()), "owner-123", &agents)
            .await
            .unwrap();

        let (mode, agent) = resolve_sandbox_restart_mode(db.as_ref(), "acp:codex", "owner-123")
            .await
            .unwrap();

        assert_eq!(mode, JobMode::Acp);
        assert_eq!(
            agent.as_ref().map(|agent| agent.name.as_str()),
            Some("codex")
        );
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn sandbox_restart_mode_rejects_disabled_acp_agent() {
        let (db, _tmp) = crate::testing::test_db().await;

        let mut agents = crate::config::acp::AcpAgentsFile::default();
        let mut agent = crate::config::acp::AcpAgentConfig::new(
            "codex",
            "codex",
            vec!["acp".into()],
            std::collections::HashMap::new(),
        );
        agent.enabled = false;
        agents.upsert(agent);
        crate::config::acp::save_acp_agents_for_user(Some(db.as_ref()), "owner-123", &agents)
            .await
            .unwrap();

        let err = resolve_sandbox_restart_mode(db.as_ref(), "acp:codex", "owner-123")
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            crate::config::acp::AcpConfigError::AgentDisabled { .. }
        ));
    }

    #[test]
    fn test_db_error_does_not_leak_details() {
        let (status, body) = db_error("test_context", "relation \"jobs\" does not exist");
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body, "Internal database error");
        assert!(!body.contains("relation"));
        assert!(!body.contains("does not exist"));
    }

    fn make_job_manager(claude_code: bool, acp: bool) -> ContainerJobManager {
        ContainerJobManager::new(
            ContainerJobConfig {
                claude_code_enabled: claude_code,
                acp_enabled: acp,
                ..Default::default()
            },
            TokenStore::new(),
        )
    }

    #[test]
    fn test_check_mode_rejects_disabled_claude_code() {
        let jm = make_job_manager(false, false);
        let result = check_mode_enabled(JobMode::ClaudeCode, &jm);
        let (status, body) = result.unwrap_err(); // safety: test
        assert_eq!(status, StatusCode::CONFLICT);
        assert!(
            body.contains("claude_code"),
            "error should mention claude_code, got: {body}"
        );
    }

    #[test]
    fn test_check_mode_rejects_disabled_acp() {
        let jm = make_job_manager(false, false);
        let result = check_mode_enabled(JobMode::Acp, &jm);
        let (status, body) = result.unwrap_err(); // safety: test
        assert_eq!(status, StatusCode::CONFLICT);
        assert!(
            body.contains("acp"),
            "error should mention acp, got: {body}"
        );
    }

    #[test]
    fn test_check_mode_allows_worker_always() {
        let jm = make_job_manager(false, false);
        let result = check_mode_enabled(JobMode::Worker, &jm);
        assert!(result.is_ok(), "worker mode should always be allowed");
    }
}
