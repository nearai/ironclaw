//! Scope grant management API handlers (admin).

use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};

use crate::channels::web::auth::AdminUser;
use crate::channels::web::server::GatewayState;

/// GET /api/admin/users/{user_id}/scope-grants
pub async fn scope_grants_list_handler(
    State(state): State<Arc<GatewayState>>,
    AdminUser(_admin): AdminUser,
    Path(user_id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    let grants = store
        .list_scope_grants(&user_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let items: Vec<serde_json::Value> = grants
        .iter()
        .map(|g| {
            serde_json::json!({
                "user_id": g.user_id,
                "scope": g.scope,
                "writable": g.writable,
                "granted_by": g.granted_by,
                "created_at": g.created_at.to_rfc3339(),
            })
        })
        .collect();

    Ok(Json(serde_json::json!({ "grants": items })))
}

/// PUT /api/admin/users/{user_id}/scope-grants/{scope}
pub async fn scope_grants_set_handler(
    State(state): State<Arc<GatewayState>>,
    AdminUser(admin): AdminUser,
    Path((user_id, scope)): Path<(String, String)>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    let writable = body
        .get("writable")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    store
        .set_scope_grant(&user_id, &scope, writable, Some(&admin.user_id))
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(serde_json::json!({
        "user_id": user_id,
        "scope": scope,
        "writable": writable,
    })))
}

/// DELETE /api/admin/users/{user_id}/scope-grants/{scope}
pub async fn scope_grants_delete_handler(
    State(state): State<Arc<GatewayState>>,
    AdminUser(_admin): AdminUser,
    Path((user_id, scope)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    let deleted = store
        .revoke_scope_grant(&user_id, &scope)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if deleted {
        Ok(Json(serde_json::json!({ "deleted": true })))
    } else {
        Err((StatusCode::NOT_FOUND, "Scope grant not found".to_string()))
    }
}

/// GET /api/admin/scope-grants/by-scope/{scope}
pub async fn scope_grants_by_scope_handler(
    State(state): State<Arc<GatewayState>>,
    AdminUser(_admin): AdminUser,
    Path(scope): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    let grants = store
        .list_scope_grants_for_scope(&scope)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let items: Vec<serde_json::Value> = grants
        .iter()
        .map(|g| {
            serde_json::json!({
                "user_id": g.user_id,
                "scope": g.scope,
                "writable": g.writable,
                "granted_by": g.granted_by,
                "created_at": g.created_at.to_rfc3339(),
            })
        })
        .collect();

    Ok(Json(serde_json::json!({ "grants": items })))
}
