//! Routine management API handlers.

use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::Deserialize;
use uuid::Uuid;

use crate::agent::routine::{
    RoutineDisplayStatus, RoutineVerificationStatus, Trigger, next_cron_fire,
    routine_display_status_for_verification, routine_verification_status,
};
use crate::channels::web::auth::AuthenticatedUser;
use crate::channels::web::platform::state::GatewayState;
use crate::channels::web::types::*;
use crate::error::RoutineError;
use crate::ownership::Owned;

pub async fn routines_list_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
) -> Result<Json<RoutineListResponse>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    let routines = store
        .list_routines(&user.user_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let routine_ids: Vec<Uuid> = routines.iter().map(|routine| routine.id).collect();
    let last_run_statuses = store
        .batch_get_last_run_status(&routine_ids)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let items: Vec<RoutineInfo> = routines
        .iter()
        .map(|routine| {
            RoutineInfo::from_routine(routine, last_run_statuses.get(&routine.id).copied())
        })
        .collect();

    Ok(Json(RoutineListResponse { routines: items }))
}

pub async fn routines_summary_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
) -> Result<Json<RoutineSummaryResponse>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    let routines = store
        .list_routines(&user.user_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let routine_ids: Vec<Uuid> = routines.iter().map(|routine| routine.id).collect();
    let last_run_statuses = store
        .batch_get_last_run_status(&routine_ids)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let total = routines.len() as u64;
    let mut enabled = 0u64;
    let mut disabled = 0u64;
    let mut unverified = 0u64;
    let mut failing = 0u64;

    for routine in &routines {
        let verification_status = routine_verification_status(routine);
        if routine.enabled {
            enabled += 1;
        } else {
            disabled += 1;
        }

        if verification_status == RoutineVerificationStatus::Unverified {
            unverified += 1;
        }

        if routine_display_status_for_verification(
            routine,
            verification_status,
            last_run_statuses.get(&routine.id).copied(),
        ) == RoutineDisplayStatus::Failing
        {
            failing += 1;
        }
    }

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
        unverified,
        failing,
        runs_today,
    }))
}

pub async fn routines_detail_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
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

    if !routine.is_owned_by(&user.user_id) {
        return Err((StatusCode::NOT_FOUND, "Routine not found".to_string()));
    }

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
            status: run.status.to_string(),
            result_summary: run.result_summary.clone(),
            tokens_used: run.tokens_used,
            job_id: run.job_id,
        })
        .collect();
    let routine_info = RoutineInfo::from_routine(&routine, runs.first().map(|run| run.status));

    // Read-only lookup — do not create a conversation on a GET request.
    // The conversation is created lazily when the routine first executes.
    let conversation_id = store
        .find_routine_conversation(routine.id, &routine.user_id)
        .await
        .unwrap_or_else(|e| {
            tracing::warn!(routine_id = %routine.id, error = %e, "Failed to look up routine conversation");
            None
        });

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
        status: routine_info.status.clone(),
        verification_status: routine_info.verification_status.clone(),
        created_at: routine.created_at.to_rfc3339(),
        conversation_id,
        recent_runs,
    }))
}

pub async fn routines_trigger_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
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

    // Verify ownership before triggering.
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let routine = store
        .get_routine(routine_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Routine not found".to_string()))?;
    if !routine.is_owned_by(&user.user_id) {
        return Err((StatusCode::NOT_FOUND, "Routine not found".to_string()));
    }

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
    AuthenticatedUser(user): AuthenticatedUser,
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

    if !routine.is_owned_by(&user.user_id) {
        return Err((StatusCode::NOT_FOUND, "Routine not found".to_string()));
    }

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
    // Extract into a block so the RwLockReadGuard is dropped before the async call.
    let engine = { state.routine_engine.read().await.as_ref().cloned() };
    if let Some(engine) = engine {
        engine.refresh_event_cache().await;
    }

    Ok(Json(serde_json::json!({
        "status": if routine.enabled { "enabled" } else { "disabled" },
        "routine_id": routine_id,
    })))
}

pub async fn routines_delete_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    let routine_id = Uuid::parse_str(&id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid routine ID".to_string()))?;

    // Verify ownership before deleting.
    let routine = store
        .get_routine(routine_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Routine not found".to_string()))?;

    if !routine.is_owned_by(&user.user_id) {
        return Err((StatusCode::NOT_FOUND, "Routine not found".to_string()));
    }

    let deleted = store
        .delete_routine(routine_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if deleted {
        // Refresh the in-memory event trigger cache so deleted event/system_event
        // routines stop firing immediately (issue #1076).
        // Extract into a block so the RwLockReadGuard is dropped before the async call.
        let engine = { state.routine_engine.read().await.as_ref().cloned() };
        if let Some(engine) = engine {
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

pub(crate) async fn routines_runs_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    let routine_id = Uuid::parse_str(&id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid routine ID".to_string()))?;

    // Verify ownership before listing runs.
    let routine = store
        .get_routine(routine_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "Routine not found".to_string()))?;

    if !routine.is_owned_by(&user.user_id) {
        return Err((StatusCode::NOT_FOUND, "Routine not found".to_string()));
    }

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
            status: run.status.to_string(),
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
