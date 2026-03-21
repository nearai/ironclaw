//! Routine management API handlers.

use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::Deserialize;
use uuid::Uuid;

use crate::agent::routine::{Trigger, next_cron_fire};
use crate::channels::web::server::GatewayState;
use crate::channels::web::types::*;
use crate::error::RoutineError;

pub async fn routines_list_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<Json<RoutineListResponse>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    let routines = store
        .list_all_routines()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let items: Vec<RoutineInfo> = routines.iter().map(RoutineInfo::from_routine).collect();

    Ok(Json(RoutineListResponse { routines: items }))
}

pub async fn routines_summary_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<Json<RoutineSummaryResponse>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    let routines = store
        .list_all_routines()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let total = routines.len() as u64;
    let enabled = routines.iter().filter(|r| r.enabled).count() as u64;
    let disabled = total - enabled;
    let failing = routines
        .iter()
        .filter(|r| r.consecutive_failures > 0)
        .count() as u64;

    let today_start = chrono::Utc::now()
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .map(|dt| dt.and_utc());
    let runs_today = if let Some(start) = today_start {
        routines
            .iter()
            .filter(|r| r.last_run_at.is_some_and(|ts| ts >= start))
            .count() as u64
    } else {
        0
    };

    Ok(Json(RoutineSummaryResponse {
        total,
        enabled,
        disabled,
        failing,
        runs_today,
    }))
}

pub async fn routines_detail_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
) -> Result<Json<RoutineDetailResponse>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    let routine_id = Uuid::parse_str(&id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid routine ID".to_string()))?;

    let routine = store
        .get_routine(routine_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Routine not found".to_string()))?;

    let runs = store
        .list_routine_runs(routine_id, 20)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let recent_runs: Vec<RoutineRunInfo> = runs
        .iter()
        .map(|run| RoutineRunInfo {
            id: run.id,
            trigger_type: run.trigger_type.clone(),
            started_at: run.started_at.to_rfc3339(),
            completed_at: run.completed_at.map(|dt| dt.to_rfc3339()),
            status: format!("{:?}", run.status),
            result_summary: run.result_summary.clone(),
            tokens_used: run.tokens_used,
            job_id: run.job_id,
        })
        .collect();
    let routine_info = RoutineInfo::from_routine(&routine);

    Ok(Json(RoutineDetailResponse {
        id: routine.id,
        name: routine.name.clone(),
        description: routine.description.clone(),
        enabled: routine.enabled,
        trigger_type: routine_info.trigger_type,
        trigger_raw: routine_info.trigger_raw,
        trigger_summary: routine_info.trigger_summary,
        trigger: serde_json::to_value(&routine.trigger).unwrap_or_default(),
        action: serde_json::to_value(&routine.action).unwrap_or_default(),
        guardrails: serde_json::to_value(&routine.guardrails).unwrap_or_default(),
        notify: serde_json::to_value(&routine.notify).unwrap_or_default(),
        last_run_at: routine.last_run_at.map(|dt| dt.to_rfc3339()),
        next_fire_at: routine.next_fire_at.map(|dt| dt.to_rfc3339()),
        run_count: routine.run_count,
        consecutive_failures: routine.consecutive_failures,
        created_at: routine.created_at.to_rfc3339(),
        recent_runs,
    }))
}

pub async fn routines_trigger_handler(
    State(state): State<Arc<GatewayState>>,
    crate::channels::web::auth::AuthenticatedUser(user): crate::channels::web::auth::AuthenticatedUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    // Clone the Arc out of the lock to avoid holding the RwLock across .await.
    let engine = {
        let guard = state.routine_engine.read().await;
        guard.as_ref().cloned().ok_or((
            StatusCode::SERVICE_UNAVAILABLE,
            "Routine engine not available".to_string(),
        ))?
    };

    let routine_id = Uuid::parse_str(&id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid routine ID".to_string()))?;

    let run_id = engine
        .fire_manual(routine_id, Some(&user.user_id))
        .await
        .map_err(|e| (routine_error_status(&e), e.to_string()))?;

    Ok(Json(serde_json::json!({
        "status": "triggered",
        "routine_id": routine_id,
        "run_id": run_id,
    })))
}

#[derive(Deserialize)]
pub struct ToggleRequest {
    pub enabled: Option<bool>,
}

pub async fn routines_toggle_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
    body: Option<Json<ToggleRequest>>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    let routine_id = Uuid::parse_str(&id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid routine ID".to_string()))?;

    let mut routine = store
        .get_routine(routine_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Routine not found".to_string()))?;

    let was_enabled = routine.enabled;
    // If a specific value was provided, use it; otherwise toggle.
    routine.enabled = match body {
        Some(Json(req)) => req.enabled.unwrap_or(!routine.enabled),
        None => !routine.enabled,
    };

    // When re-enabling a cron routine, recompute next_fire_at so the cron
    // ticker can pick it up. Mirrors the CLI behavior (issue #1077).
    if routine.enabled
        && !was_enabled
        && let Trigger::Cron {
            ref schedule,
            ref timezone,
        } = routine.trigger
    {
        routine.next_fire_at = next_cron_fire(schedule, timezone.as_deref()).map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to compute next fire: {e}"),
            )
        })?;
    }

    store
        .update_routine(&routine)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Refresh the in-memory event trigger cache so event/system_event
    // routines reflect the new enabled state immediately (issue #1076).
    if let Some(engine) = state.routine_engine.read().await.as_ref() {
        engine.refresh_event_cache().await;
    }

    Ok(Json(serde_json::json!({
        "status": if routine.enabled { "enabled" } else { "disabled" },
        "routine_id": routine_id,
    })))
}

pub async fn routines_delete_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    let routine_id = Uuid::parse_str(&id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid routine ID".to_string()))?;

    let deleted = store
        .delete_routine(routine_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if deleted {
        // Refresh the in-memory event trigger cache so deleted event/system_event
        // routines stop firing immediately (issue #1076).
        if let Some(engine) = state.routine_engine.read().await.as_ref() {
            engine.refresh_event_cache().await;
        }

        Ok(Json(serde_json::json!({
            "status": "deleted",
            "routine_id": routine_id,
        })))
    } else {
        Err((StatusCode::NOT_FOUND, "Routine not found".to_string()))
    }
}

pub async fn routines_runs_handler(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    let routine_id = Uuid::parse_str(&id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid routine ID".to_string()))?;

    let runs = store
        .list_routine_runs(routine_id, 50)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let run_infos: Vec<RoutineRunInfo> = runs
        .iter()
        .map(|run| RoutineRunInfo {
            id: run.id,
            trigger_type: run.trigger_type.clone(),
            started_at: run.started_at.to_rfc3339(),
            completed_at: run.completed_at.map(|dt| dt.to_rfc3339()),
            status: format!("{:?}", run.status),
            result_summary: run.result_summary.clone(),
            tokens_used: run.tokens_used,
            job_id: run.job_id,
        })
        .collect();

    Ok(Json(serde_json::json!({
        "routine_id": routine_id,
        "runs": run_infos,
    })))
}

/// Map `RoutineError` variants to appropriate HTTP status codes.
fn routine_error_status(err: &RoutineError) -> StatusCode {
    match err {
        RoutineError::NotFound { .. } => StatusCode::NOT_FOUND,
        RoutineError::NotAuthorized { .. } => StatusCode::FORBIDDEN,
        RoutineError::Disabled { .. }
        | RoutineError::Cooldown { .. }
        | RoutineError::MaxConcurrent { .. } => StatusCode::CONFLICT,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}
