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

    // Fetch agent (non-sandbox) jobs from database, deduplicating by ID.
    match store.list_agent_jobs().await {
        Ok(agent_jobs) => {
            for j in &agent_jobs {
                if j.user_id != user.user_id {
                    continue;
                }
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
    AuthenticatedUser(_user): AuthenticatedUser,
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

    // Sandbox job counts.
    match store.sandbox_job_summary().await {
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

    // Agent job counts.
    match store.agent_job_summary().await {
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
            if job.user_id != user.user_id {
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

<<<<<<< HEAD
        let mode = store.get_sandbox_job_mode(job.id).await.ok().flatten();
        let is_claude_code = mode.as_deref() == Some("claude_code");

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
            can_prompt: is_claude_code && state.prompt_queue.is_some(),
            job_kind: Some("sandbox".to_string()),
        }));
=======
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
                job_mode: {
                    let mode = store.get_sandbox_job_mode(job.id).await.ok().flatten();
                    mode.filter(|m| m != "worker")
                },
                transitions,
            }));
        }
        Ok(None) => {}
        Err(e) => {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Database error: {}", e),
            ));
        }
>>>>>>> ea8150f (feat: handler auth and ownership checks)
    }

    // Fall back to agent job from DB.
    match store.get_job(job_id).await {
        Ok(Some(ctx)) => {
            if ctx.user_id != user.user_id {
                return Err((StatusCode::NOT_FOUND, "Job not found".to_string()));
            }
            let elapsed_secs = ctx.started_at.map(|start| {
                let end = ctx.completed_at.unwrap_or_else(chrono::Utc::now);
                (end - start).num_seconds().max(0) as u64
            });

<<<<<<< HEAD
        // Only show prompt bar for jobs that have a running worker (Pending/InProgress).
        // Stuck jobs have no active worker loop, so messages would be silently dropped.
        let is_promptable = matches!(
            ctx.state,
            crate::context::JobState::Pending | crate::context::JobState::InProgress
        );
        return Ok(Json(JobDetailResponse {
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
            transitions: Vec::new(),
            can_restart: state.scheduler.is_some(),
            can_prompt: is_promptable && state.scheduler.is_some(),
            job_kind: Some("agent".to_string()),
        }));
=======
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
                transitions: Vec::new(),
            }))
        }
        Ok(None) => Err((StatusCode::NOT_FOUND, "Job not found".to_string())),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Database error: {}", e),
        )),
>>>>>>> ea8150f (feat: handler auth and ownership checks)
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
                if job.user_id != user.user_id {
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
                return Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Database error: {}", e),
                ));
            }
        }
    }

<<<<<<< HEAD
    // Fall back to agent job cancellation: stop the worker via the scheduler
    // (which updates the in-memory ContextManager AND aborts the task handle),
    // then persist the status to the DB as a fallback.
    if let Some(ref store) = state.store
        && let Ok(Some(job)) = store.get_job(job_id).await
    {
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
=======
    // Fall back to agent job cancellation via DB status update.
    if let Some(ref store) = state.store {
        match store.get_job(job_id).await {
            Ok(Some(job)) => {
                if job.user_id != user.user_id {
                    return Err((StatusCode::NOT_FOUND, "Job not found".to_string()));
                }
                if job.state.is_active() {
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
                return Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Database error: {}", e),
                ));
            }
>>>>>>> ea8150f (feat: handler auth and ownership checks)
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
    if let Ok(Some(old_job)) = store.get_sandbox_job(old_job_id).await {
        if old_job.status != "interrupted" && old_job.status != "failed" {
            return Err((
                StatusCode::CONFLICT,
                format!("Cannot restart job in state '{}'", old_job.status),
            ));
        }

<<<<<<< HEAD
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
        };
        store
            .save_sandbox_job(&record)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        let mode = match store.get_sandbox_job_mode(old_job_id).await {
            Ok(Some(m)) if m == "claude_code" => {
                crate::orchestrator::job_manager::JobMode::ClaudeCode
            }
            _ => crate::orchestrator::job_manager::JobMode::Worker,
        };

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

        let project_dir = std::path::PathBuf::from(&old_job.project_dir);
        let _token = jm
            .create_job(
                new_job_id,
                &task,
                Some(project_dir),
                mode,
                credential_grants,
            )
            .await
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Failed to create container: {}", e),
                )
            })?;

        store
            .update_sandbox_job_status(new_job_id, "running", None, None, Some(now), None)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

        return Ok(Json(serde_json::json!({
            "status": "restarted",
            "old_job_id": old_job_id,
            "new_job_id": new_job_id,
        })));
=======
    if old_job.user_id != user.user_id {
        return Err((StatusCode::NOT_FOUND, "Job not found".to_string()));
    }

    if old_job.status != "interrupted" && old_job.status != "failed" {
        return Err((
            StatusCode::CONFLICT,
            format!("Cannot restart job in state '{}'", old_job.status),
        ));
>>>>>>> ea8150f (feat: handler auth and ownership checks)
    }

    // Try agent job restart: dispatch a new job via the scheduler.
    if let Ok(Some(old_job)) = store.get_job(old_job_id).await {
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

        return Ok(Json(serde_json::json!({
            "status": "restarted",
            "old_job_id": old_job_id,
            "new_job_id": new_job_id,
        })));
    }

    Err((StatusCode::NOT_FOUND, "Job not found".to_string()))
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

    // Verify ownership before queuing a prompt.
    if let Some(ref store) = state.store {
        match store.get_sandbox_job(job_id).await {
            Ok(Some(job)) => {
                if job.user_id != user.user_id {
                    return Err((StatusCode::NOT_FOUND, "Job not found".to_string()));
                }
            }
            Ok(None) => {
                return Err((StatusCode::NOT_FOUND, "Job not found".to_string()));
            }
            Err(e) => {
                return Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Database error: {}", e),
                ));
            }
        }
    }

    let content = body
        .get("content")
        .and_then(|v| v.as_str())
        .ok_or((
            StatusCode::BAD_REQUEST,
            "Missing 'content' field".to_string(),
        ))?
        .to_string();

    let done = body.get("done").and_then(|v| v.as_bool()).unwrap_or(false);

    // Try sandbox job path: check if we have a sandbox record for this ID.
    if let Some(ref s) = state.store
        && let Ok(Some(_)) = s.get_sandbox_job(job_id).await
    {
        // It's a sandbox job. Check if Claude Code mode.
        let mode = s.get_sandbox_job_mode(job_id).await.ok().flatten();
        if mode.as_deref() == Some("claude_code") {
            let prompt_queue = state.prompt_queue.as_ref().ok_or((
                StatusCode::NOT_IMPLEMENTED,
                "Claude Code not configured".to_string(),
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

    // Try agent job path: send via scheduler.
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

    // Verify ownership before returning events.
    match store.get_sandbox_job(job_id).await {
        Ok(Some(job)) => {
            if job.user_id != user.user_id {
                return Err((StatusCode::NOT_FOUND, "Job not found".to_string()));
            }
        }
        Ok(None) => {
            return Err((StatusCode::NOT_FOUND, "Job not found".to_string()));
        }
        Err(e) => {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Database error: {}", e),
            ));
        }
    }

    let events = store
        .list_job_events(job_id, None)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

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

    if job.user_id != user.user_id {
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

    if job.user_id != user.user_id {
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
