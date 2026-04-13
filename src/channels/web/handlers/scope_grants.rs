//! Scope grant management API handlers.
//!
//! Two access levels:
//! - **Admin** endpoints under `/api/admin/` — full CRUD on any user's grants.
//! - **Self-service** endpoints under `/api/scope-grants/` — users can manage
//!   grants for scopes they own or have writable access to.

use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};

use crate::channels::web::auth::{AdminUser, AuthenticatedUser};
use crate::channels::web::server::GatewayState;
use crate::db::ScopeGrantStore;

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

    Ok(Json(serde_json::json!({ "grants": grants_to_json(&grants) })))
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

/// GET /api/admin/scope-grants/by-scope/{scope} — admin: list who has access.
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

    Ok(Json(serde_json::json!({ "grants": grants_to_json(&grants) })))
}

// ── Helpers ─────────────────────────────────────────────────────────────

fn grants_to_json(grants: &[crate::db::ScopeGrantRecord]) -> Vec<serde_json::Value> {
    grants
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
        .collect()
}

/// Check whether `caller_id` can manage grants for `scope`.
///
/// Authorization: caller owns the scope (caller_id == scope) OR has a
/// writable grant to it.
async fn require_scope_writer(
    store: &(dyn ScopeGrantStore + '_),
    caller_id: &str,
    scope: &str,
) -> Result<(), (StatusCode, String)> {
    if caller_id == scope {
        return Ok(());
    }
    let grants = store
        .list_scope_grants(caller_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    if grants.iter().any(|g| g.scope == scope && g.writable) {
        return Ok(());
    }
    Err((
        StatusCode::FORBIDDEN,
        "You must own this scope or have writable access to manage its grants".to_string(),
    ))
}

// ── Self-service endpoints ──────────────────────────────────────────────

/// GET /api/scope-grants — list the caller's own grants (what can I access?)
pub async fn my_scope_grants_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let grants = store
        .list_scope_grants(&user.user_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(serde_json::json!({ "grants": grants_to_json(&grants) })))
}

/// GET /api/scope-grants/{scope} — list who has access to this scope.
/// Caller must own the scope or have writable access.
pub async fn scope_members_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(scope): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    require_scope_writer(store.as_ref(), &user.user_id, &scope).await?;
    let grants = store
        .list_scope_grants_for_scope(&scope)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(serde_json::json!({ "grants": grants_to_json(&grants) })))
}

/// PUT /api/scope-grants/{scope}/{grantee} — grant a user access to a scope.
/// Caller must own the scope or have writable access.
pub async fn scope_grant_set_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path((scope, grantee)): Path<(String, String)>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    require_scope_writer(store.as_ref(), &user.user_id, &scope).await?;
    let writable = body
        .get("writable")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    store
        .set_scope_grant(&grantee, &scope, writable, Some(&user.user_id))
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(serde_json::json!({
        "user_id": grantee,
        "scope": scope,
        "writable": writable,
    })))
}

/// DELETE /api/scope-grants/{scope}/{grantee} — revoke a user's access.
/// Caller must own the scope or have writable access.
pub async fn scope_grant_revoke_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path((scope, grantee)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    require_scope_writer(store.as_ref(), &user.user_id, &scope).await?;
    let deleted = store
        .revoke_scope_grant(&grantee, &scope)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    if deleted {
        Ok(Json(serde_json::json!({ "deleted": true })))
    } else {
        Err((StatusCode::NOT_FOUND, "Scope grant not found".to_string()))
    }
}
