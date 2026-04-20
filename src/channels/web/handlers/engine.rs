//! Engine v2 API handlers — threads, projects, missions.

use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde::Deserialize;

#[derive(Debug, Deserialize, Default)]
pub struct ProjectFilter {
    #[serde(default)]
    pub project_id: Option<String>,
}

use crate::channels::web::auth::AuthenticatedUser;
use crate::channels::web::platform::state::GatewayState;
use crate::channels::web::types::*;
use crate::tools::builtin::memory::WorkspaceResolver;

// ── Threads ─────────────────────────────────────────────────

pub async fn engine_threads_handler(
    State(_state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Query(filter): Query<ProjectFilter>,
) -> Result<Json<EngineThreadListResponse>, (StatusCode, String)> {
    let threads = crate::bridge::list_engine_threads(filter.project_id.as_deref(), &user.user_id)
        .await
        .map_err(|e| {
            tracing::debug!("engine API error: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal engine error".to_string(),
            )
        })?;
    Ok(Json(EngineThreadListResponse { threads }))
}

pub async fn engine_thread_detail_handler(
    State(_state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(id): Path<String>,
) -> Result<Json<EngineThreadDetailResponse>, (StatusCode, String)> {
    let thread = crate::bridge::get_engine_thread(&id, &user.user_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Thread not found".to_string()))?;
    Ok(Json(EngineThreadDetailResponse { thread }))
}

pub async fn engine_thread_steps_handler(
    State(_state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(id): Path<String>,
) -> Result<Json<EngineStepListResponse>, (StatusCode, String)> {
    let steps = crate::bridge::list_engine_thread_steps(&id, &user.user_id)
        .await
        .map_err(|e| {
            tracing::debug!("engine API error: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal engine error".to_string(),
            )
        })?;
    Ok(Json(EngineStepListResponse { steps }))
}

pub async fn engine_thread_events_handler(
    State(_state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(id): Path<String>,
) -> Result<Json<EngineEventListResponse>, (StatusCode, String)> {
    let events = crate::bridge::list_engine_thread_events(&id, &user.user_id)
        .await
        .map_err(|e| {
            tracing::debug!("engine API error: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal engine error".to_string(),
            )
        })?;
    Ok(Json(EngineEventListResponse { events }))
}

// ── Projects ────────────────────────────────────────────────

pub async fn engine_projects_handler(
    State(_state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
) -> Result<Json<EngineProjectListResponse>, (StatusCode, String)> {
    let projects = crate::bridge::list_engine_projects(&user.user_id)
        .await
        .map_err(|e| {
            tracing::debug!("engine API error: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal engine error".to_string(),
            )
        })?;
    Ok(Json(EngineProjectListResponse { projects }))
}

pub async fn engine_projects_overview_handler(
    State(_state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
) -> Result<Json<crate::bridge::ProjectsOverviewResponse>, (StatusCode, String)> {
    let overview = crate::bridge::get_engine_projects_overview(&user.user_id)
        .await
        .map_err(|e| {
            tracing::debug!("engine API error: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal engine error".to_string(),
            )
        })?;
    Ok(Json(overview))
}

pub async fn engine_project_detail_handler(
    State(_state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(id): Path<String>,
) -> Result<Json<EngineProjectDetailResponse>, (StatusCode, String)> {
    let project = crate::bridge::get_engine_project(&id, &user.user_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Project not found".to_string()))?;
    Ok(Json(EngineProjectDetailResponse { project }))
}

// ── Missions ────────────────────────────────────────────────

pub async fn engine_missions_handler(
    State(_state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Query(filter): Query<ProjectFilter>,
) -> Result<Json<EngineMissionListResponse>, (StatusCode, String)> {
    let missions = crate::bridge::list_engine_missions(filter.project_id.as_deref(), &user.user_id)
        .await
        .map_err(|e| {
            tracing::debug!("engine API error: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal engine error".to_string(),
            )
        })?;
    Ok(Json(EngineMissionListResponse { missions }))
}

pub async fn engine_missions_summary_handler(
    State(_state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
) -> Result<Json<EngineMissionSummaryResponse>, (StatusCode, String)> {
    let missions = crate::bridge::list_engine_missions(None, &user.user_id)
        .await
        .map_err(|e| {
            tracing::debug!("engine API error: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal engine error".to_string(),
            )
        })?;

    let total = missions.len() as u64;
    let active = missions.iter().filter(|m| m.status == "Active").count() as u64;
    let paused = missions.iter().filter(|m| m.status == "Paused").count() as u64;
    let completed = missions.iter().filter(|m| m.status == "Completed").count() as u64;
    let failed = missions.iter().filter(|m| m.status == "Failed").count() as u64;

    Ok(Json(EngineMissionSummaryResponse {
        total,
        active,
        paused,
        completed,
        failed,
    }))
}

pub async fn engine_mission_detail_handler(
    State(_state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(id): Path<String>,
) -> Result<Json<EngineMissionDetailResponse>, (StatusCode, String)> {
    let mission = crate::bridge::get_engine_mission(&id, &user.user_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Mission not found".to_string()))?;
    Ok(Json(EngineMissionDetailResponse { mission }))
}

pub async fn engine_mission_fire_handler(
    State(_state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(id): Path<String>,
) -> Result<Json<EngineMissionFireResponse>, (StatusCode, String)> {
    let thread_id = crate::bridge::fire_engine_mission(&id, &user.user_id)
        .await
        .map_err(|e| {
            tracing::debug!("engine API error: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal engine error".to_string(),
            )
        })?;
    Ok(Json(EngineMissionFireResponse {
        fired: thread_id.is_some(),
        thread_id,
    }))
}

pub async fn engine_mission_pause_handler(
    State(_state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(id): Path<String>,
) -> Result<Json<EngineActionResponse>, (StatusCode, String)> {
    let is_admin = crate::ownership::UserRole::from_db_role(&user.role).is_admin();
    crate::bridge::pause_engine_mission(&id, &user.user_id, is_admin)
        .await
        .map_err(|e| {
            let msg = e.to_string();
            let (status, body) = if msg.contains("forbidden") {
                (StatusCode::FORBIDDEN, "Forbidden".to_string())
            } else {
                tracing::debug!("engine API error: {msg}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal engine error".to_string(),
                )
            };
            (status, body)
        })?;
    Ok(Json(EngineActionResponse { ok: true }))
}

pub async fn engine_mission_resume_handler(
    State(_state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(id): Path<String>,
) -> Result<Json<EngineActionResponse>, (StatusCode, String)> {
    let is_admin = crate::ownership::UserRole::from_db_role(&user.role).is_admin();
    crate::bridge::resume_engine_mission(&id, &user.user_id, is_admin)
        .await
        .map_err(|e| {
            let msg = e.to_string();
            let (status, body) = if msg.contains("forbidden") {
                (StatusCode::FORBIDDEN, "Forbidden".to_string())
            } else {
                tracing::debug!("engine API error: {msg}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Internal engine error".to_string(),
                )
            };
            (status, body)
        })?;
    Ok(Json(EngineActionResponse { ok: true }))
}

// ── Project mutations (dispatcher-routed) ───────────────────────────
//
// Gateway-initiated project mutations dispatch through `ToolDispatcher`
// per `.claude/rules/tools.md` ("Everything Goes Through Tools"). The
// dispatcher provides the audit trail + safety pipeline; the handler's
// job is to translate HTTP request → JSON tool params, then parse the
// tool's JSON result into the response DTO.

/// Resolve the dispatcher or surface a 503 — mirrors the pattern in
/// `.claude/rules/tools.md`.
fn require_dispatcher(
    state: &GatewayState,
) -> Result<Arc<crate::tools::dispatch::ToolDispatcher>, (StatusCode, String)> {
    state.tool_dispatcher.clone().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "tool dispatcher unavailable".to_string(),
    ))
}

fn map_dispatch_err(e: crate::tools::ToolError) -> (StatusCode, String) {
    use crate::tools::ToolError;
    match e {
        ToolError::InvalidParameters(msg) => (StatusCode::BAD_REQUEST, msg),
        ToolError::NotAuthorized(msg) => (StatusCode::FORBIDDEN, msg),
        ToolError::Timeout(d) => (
            StatusCode::GATEWAY_TIMEOUT,
            format!("tool timed out after {d:?}"),
        ),
        other => (StatusCode::INTERNAL_SERVER_ERROR, other.to_string()),
    }
}

fn parse_tool_project_info(
    output: crate::tools::ToolOutput,
) -> Result<crate::bridge::EngineProjectInfo, (StatusCode, String)> {
    // Project tools emit `ToolOutput::text(serialized_json, ...)` where the
    // inner string is itself a JSON object. `result` is therefore a
    // `Value::String(...)` whose content we must deserialize, not the object
    // directly.
    let body = output.result.as_str().ok_or_else(|| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "project tool returned non-string payload".to_string(),
        )
    })?;
    serde_json::from_str::<crate::bridge::EngineProjectInfo>(body).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("project tool returned unparseable payload: {e}"),
        )
    })
}

pub async fn engine_project_create_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<EngineProjectDetailResponse>, (StatusCode, String)> {
    let dispatcher = require_dispatcher(&state)?;
    let output = dispatcher
        .dispatch(
            "project_create",
            body,
            &user.user_id,
            crate::tools::dispatch::DispatchSource::Channel("gateway".into()),
        )
        .await
        .map_err(map_dispatch_err)?;
    let project = parse_tool_project_info(output)?;
    Ok(Json(EngineProjectDetailResponse { project }))
}

pub async fn engine_project_update_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(id): Path<String>,
    Json(mut body): Json<serde_json::Value>,
) -> Result<Json<EngineProjectDetailResponse>, (StatusCode, String)> {
    if let Some(obj) = body.as_object_mut() {
        obj.insert("id".into(), serde_json::Value::String(id));
    } else {
        return Err((
            StatusCode::BAD_REQUEST,
            "request body must be a JSON object".to_string(),
        ));
    }
    let dispatcher = require_dispatcher(&state)?;
    let output = dispatcher
        .dispatch(
            "project_update",
            body,
            &user.user_id,
            crate::tools::dispatch::DispatchSource::Channel("gateway".into()),
        )
        .await
        .map_err(map_dispatch_err)?;
    let project = parse_tool_project_info(output)?;
    Ok(Json(EngineProjectDetailResponse { project }))
}

pub async fn engine_project_set_active_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<EngineActionResponse>, (StatusCode, String)> {
    let dispatcher = require_dispatcher(&state)?;
    dispatcher
        .dispatch(
            "project_set_active",
            body,
            &user.user_id,
            crate::tools::dispatch::DispatchSource::Channel("gateway".into()),
        )
        .await
        .map_err(map_dispatch_err)?;
    Ok(Json(EngineActionResponse { ok: true }))
}

/// Read the active-project pointer. Dispatch-exempt: pure read aggregation
/// across a single workspace MemoryDoc (`projects/_active.json`) plus an
/// engine `Store::load_project` call — the audit trail value is low, the
/// channel already has `user_id` scoped access, and no mutation is
/// performed. See `.claude/rules/tools.md` § "When direct access IS allowed".
pub async fn engine_project_get_active_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    // Workspace pool is the single source of per-user workspaces.
    let pool = state.workspace_pool.clone().ok_or((
        // dispatch-exempt: read aggregation of active pointer + engine project lookup
        StatusCode::SERVICE_UNAVAILABLE,
        "workspace unavailable".to_string(),
    ))?;
    let workspace = pool.resolve(&user.user_id).await; // dispatch-exempt: read of active-pointer MemoryDoc

    let active_id = match workspace.read("projects/_active.json").await {
        Ok(doc) => {
            if doc.content.trim().is_empty() {
                None
            } else {
                serde_json::from_str::<serde_json::Value>(&doc.content)
                    .ok()
                    .and_then(|v| {
                        v.get("project_id")
                            .and_then(|id| id.as_str())
                            .map(str::to_string)
                    })
            }
        }
        Err(_) => None,
    };

    let project = match &active_id {
        Some(id) => {
            crate::bridge::get_engine_project(id, &user.user_id) // dispatch-exempt: read of engine project by id
                .await
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        }
        None => None,
    };

    Ok(Json(serde_json::json!({
        "project_id": active_id,
        "project": project,
    })))
}

/// Resolve which project a thread should show chrome for.
///
/// Precedence:
///   1. Per-thread override in `conversations.metadata.project_id`.
///   2. User-level active pointer in `projects/_active.json`.
///   3. `None` — the thread shows no project chrome.
///
/// Returns `(project_info, is_override)` so the chrome can indicate
/// whether the current binding is a thread-specific override.
///
/// Dispatch-exempt: read-only aggregation across the v1 DB, the workspace,
/// and the engine store — see `.claude/rules/tools.md` § "When direct
/// access IS allowed".
pub async fn resolve_thread_project(
    state: &GatewayState,
    user_id: &str,
    thread_id: uuid::Uuid,
) -> Option<(crate::bridge::EngineProjectInfo, bool)> {
    // 1. Per-thread override from conversations.metadata.project_id.
    let db_override = match state.store.as_ref() {
        // dispatch-exempt: read-only aggregation of thread metadata
        Some(store) => store
            .get_conversation_metadata(thread_id)
            .await
            .ok()
            .flatten()
            .and_then(|m| {
                m.get("project_id")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
            }),
        None => None,
    };

    if let Some(id) = db_override
        && let Ok(Some(info)) = crate::bridge::get_engine_project(&id, user_id).await
    {
        return Some((info, true));
    }

    // 2. Active pointer in workspace.
    let pool = state.workspace_pool.as_ref()?; // dispatch-exempt: read-only aggregation of active-pointer MemoryDoc
    let workspace = pool.resolve(user_id).await;
    let active_id = match workspace.read("projects/_active.json").await {
        Ok(doc) if !doc.content.trim().is_empty() => {
            serde_json::from_str::<serde_json::Value>(&doc.content)
                .ok()
                .and_then(|v| {
                    v.get("project_id")
                        .and_then(|id| id.as_str())
                        .map(str::to_string)
                })
        }
        _ => None,
    };

    if let Some(id) = active_id
        && let Ok(Some(info)) = crate::bridge::get_engine_project(&id, user_id).await
    {
        return Some((info, false));
    }

    // 3. No per-thread override and no active pointer. Fall back to
    // the user's shared "default" project (rendered as "General" in
    // the UI) — this is the per-user bucket the engine auto-creates
    // on first use, so fresh users without any `project_set_active`
    // call still get a valid workspace for `!`-mode shell dispatches
    // and chrome rendering. Without this fallback the very first
    // `!ls` after login returned 409 and the gateway showed no
    // output, which is what the user hit.
    let projects = crate::bridge::list_engine_projects(user_id).await.ok()?;
    let default_project = projects.into_iter().find(|p| p.name == "default")?;
    Some((default_project, false))
}

/// Build a `ThreadProjectContext` for a thread, including any cached
/// git state. The cache spawns refresh tasks for stale fields; this
/// call returns immediately with the last-known snapshot.
pub async fn thread_project_context(
    state: &GatewayState,
    user_id: &str,
    thread_id: uuid::Uuid,
) -> Option<crate::channels::web::types::ThreadProjectContext> {
    use crate::bridge::sandbox::workspace_path::default_project_workspace_path;

    let (info, is_override) = resolve_thread_project(state, user_id, thread_id).await?;

    let default_branch = info
        .metadata
        .default_branch
        .clone()
        .unwrap_or_else(|| "staging".to_string());

    // Resolve the host path: explicit override or default ~/.ironclaw/... path.
    let project_id = uuid::Uuid::parse_str(&info.id).ok()?;
    let resolved_path = info
        .workspace_path
        .clone()
        .unwrap_or_else(|| default_project_workspace_path(user_id, project_id));

    let cached = if let Some(cache) = state.project_context_cache.as_ref() {
        Some(
            cache
                .get(
                    ironclaw_engine::ProjectId(project_id),
                    &resolved_path,
                    user_id,
                )
                .await,
        )
    } else {
        None
    };

    Some(crate::channels::web::types::ThreadProjectContext {
        id: info.id,
        name: info.name,
        workspace_path: Some(resolved_path),
        github_repo: info.metadata.github_repo,
        default_branch,
        branch: cached.as_ref().and_then(|c| c.branch.clone()),
        dirty: cached.as_ref().and_then(|c| c.dirty),
        dirty_summary: cached.as_ref().and_then(|c| c.dirty_summary.clone()),
        pr: cached.as_ref().and_then(|c| c.pr.clone()),
        is_override,
    })
}

pub async fn engine_thread_assign_project_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(thread_id): Path<String>,
    Json(mut body): Json<serde_json::Value>,
) -> Result<Json<EngineActionResponse>, (StatusCode, String)> {
    if let Some(obj) = body.as_object_mut() {
        obj.insert("thread_id".into(), serde_json::Value::String(thread_id));
    } else {
        return Err((
            StatusCode::BAD_REQUEST,
            "request body must be a JSON object".to_string(),
        ));
    }
    let dispatcher = require_dispatcher(&state)?;
    dispatcher
        .dispatch(
            "project_assign_thread",
            body,
            &user.user_id,
            crate::tools::dispatch::DispatchSource::Channel("gateway".into()),
        )
        .await
        .map_err(map_dispatch_err)?;
    Ok(Json(EngineActionResponse { ok: true }))
}
