//! Engine v2 API handlers — threads, projects, missions.

use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};

use crate::channels::web::auth::AuthenticatedUser;
use crate::channels::web::server::GatewayState;
use crate::channels::web::types::*;

// ── Threads ─────────────────────────────────────────────────

pub async fn engine_threads_handler(
    State(_state): State<Arc<GatewayState>>,
    AuthenticatedUser(_user): AuthenticatedUser,
) -> Result<Json<EngineThreadListResponse>, (StatusCode, String)> {
    let threads = crate::bridge::list_engine_threads(None)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(EngineThreadListResponse { threads }))
}

pub async fn engine_thread_detail_handler(
    State(_state): State<Arc<GatewayState>>,
    AuthenticatedUser(_user): AuthenticatedUser,
    Path(id): Path<String>,
) -> Result<Json<EngineThreadDetailResponse>, (StatusCode, String)> {
    let thread = crate::bridge::get_engine_thread(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Thread not found".to_string()))?;
    Ok(Json(EngineThreadDetailResponse { thread }))
}

pub async fn engine_thread_steps_handler(
    State(_state): State<Arc<GatewayState>>,
    AuthenticatedUser(_user): AuthenticatedUser,
    Path(id): Path<String>,
) -> Result<Json<EngineStepListResponse>, (StatusCode, String)> {
    let steps = crate::bridge::list_engine_thread_steps(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(EngineStepListResponse { steps }))
}

pub async fn engine_thread_events_handler(
    State(_state): State<Arc<GatewayState>>,
    AuthenticatedUser(_user): AuthenticatedUser,
    Path(id): Path<String>,
) -> Result<Json<EngineEventListResponse>, (StatusCode, String)> {
    let events = crate::bridge::list_engine_thread_events(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(EngineEventListResponse { events }))
}

// ── Projects ────────────────────────────────────────────────

pub async fn engine_projects_handler(
    State(_state): State<Arc<GatewayState>>,
    AuthenticatedUser(_user): AuthenticatedUser,
) -> Result<Json<EngineProjectListResponse>, (StatusCode, String)> {
    let projects = crate::bridge::list_engine_projects()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(EngineProjectListResponse { projects }))
}

pub async fn engine_project_detail_handler(
    State(_state): State<Arc<GatewayState>>,
    AuthenticatedUser(_user): AuthenticatedUser,
    Path(id): Path<String>,
) -> Result<Json<EngineProjectDetailResponse>, (StatusCode, String)> {
    let project = crate::bridge::get_engine_project(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Project not found".to_string()))?;
    Ok(Json(EngineProjectDetailResponse { project }))
}

// ── Missions ────────────────────────────────────────────────

pub async fn engine_missions_handler(
    State(_state): State<Arc<GatewayState>>,
    AuthenticatedUser(_user): AuthenticatedUser,
) -> Result<Json<EngineMissionListResponse>, (StatusCode, String)> {
    let missions = crate::bridge::list_engine_missions(None)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(EngineMissionListResponse { missions }))
}

pub async fn engine_missions_summary_handler(
    State(_state): State<Arc<GatewayState>>,
    AuthenticatedUser(_user): AuthenticatedUser,
) -> Result<Json<EngineMissionSummaryResponse>, (StatusCode, String)> {
    let missions = crate::bridge::list_engine_missions(None)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

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
    AuthenticatedUser(_user): AuthenticatedUser,
    Path(id): Path<String>,
) -> Result<Json<EngineMissionDetailResponse>, (StatusCode, String)> {
    let mission = crate::bridge::get_engine_mission(&id)
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
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(EngineMissionFireResponse {
        fired: thread_id.is_some(),
        thread_id,
    }))
}

pub async fn engine_mission_pause_handler(
    State(_state): State<Arc<GatewayState>>,
    AuthenticatedUser(_user): AuthenticatedUser,
    Path(id): Path<String>,
) -> Result<Json<EngineActionResponse>, (StatusCode, String)> {
    crate::bridge::pause_engine_mission(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(EngineActionResponse { ok: true }))
}

pub async fn engine_mission_resume_handler(
    State(_state): State<Arc<GatewayState>>,
    AuthenticatedUser(_user): AuthenticatedUser,
    Path(id): Path<String>,
) -> Result<Json<EngineActionResponse>, (StatusCode, String)> {
    crate::bridge::resume_engine_mission(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(EngineActionResponse { ok: true }))
}
