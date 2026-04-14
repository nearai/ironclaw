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
use chrono::{DateTime, Utc};

use crate::channels::web::auth::{AdminUser, AuthenticatedUser};
use crate::channels::web::server::GatewayState;
use crate::db::Database;

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

    // Prevent granting yourself access to your own scope.
    if user_id == scope {
        return Err((
            StatusCode::BAD_REQUEST,
            "Cannot grant a user access to their own scope".to_string(),
        ));
    }

    // Validate that the scope (target user) exists.
    require_user_exists(store.as_ref(), &scope).await?;

    let writable = body
        .get("writable")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let expires_at = parse_expires_at(&body)?;

    store
        .set_scope_grant(&user_id, &scope, writable, Some(&admin.user_id), expires_at)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Invalidate caches so the grant takes effect immediately.
    invalidate_caches(&state, &user_id).await;

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
        // Invalidate caches so the revocation takes effect immediately.
        invalidate_caches(&state, &user_id).await;
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
            let mut obj = serde_json::json!({
                "user_id": g.user_id,
                "scope": g.scope,
                "writable": g.writable,
                "granted_by": g.granted_by,
                "created_at": g.created_at.to_rfc3339(),
            });
            if let Some(ref exp) = g.expires_at {
                obj["expires_at"] = serde_json::json!(exp.to_rfc3339());
            }
            obj
        })
        .collect()
}

/// Whether the caller is the scope owner (user_id == scope).
fn is_scope_owner(caller_id: &str, scope: &str) -> bool {
    caller_id == scope
}

/// Check whether `caller_id` can manage grants for `scope`.
///
/// Returns `Ok(true)` for scope owners, `Ok(false)` for writable grantees,
/// `Err(403)` otherwise. Callers use the bool to enforce escalation limits:
/// only owners can grant writable access.
async fn check_scope_access(
    store: &dyn Database,
    caller_id: &str,
    scope: &str,
) -> Result<bool, (StatusCode, String)> {
    if is_scope_owner(caller_id, scope) {
        return Ok(true);
    }
    let has_write = store
        .has_writable_grant(caller_id, scope)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    if has_write {
        return Ok(false); // writer, not owner
    }
    Err((
        StatusCode::FORBIDDEN,
        "You must own this scope or have writable access to manage its grants".to_string(),
    ))
}

/// Validate that a user ID exists. Returns 404 if not found.
async fn require_user_exists(
    store: &dyn Database,
    user_id: &str,
) -> Result<(), (StatusCode, String)> {
    let user = store
        .get_user(user_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    if user.is_none() {
        return Err((
            StatusCode::NOT_FOUND,
            format!("User '{}' not found", user_id),
        ));
    }
    Ok(())
}

/// Parse an optional `expires_at` field from a JSON request body.
fn parse_expires_at(
    body: &serde_json::Value,
) -> Result<Option<DateTime<Utc>>, (StatusCode, String)> {
    match body.get("expires_at").and_then(|v| v.as_str()) {
        Some(s) => {
            let dt = DateTime::parse_from_rfc3339(s)
                .map(|dt| dt.with_timezone(&Utc))
                .map_err(|e| {
                    (
                        StatusCode::BAD_REQUEST,
                        format!("Invalid expires_at: {e}"),
                    )
                })?;
            Ok(Some(dt))
        }
        None => Ok(None),
    }
}

/// Invalidate auth and workspace caches for a user after a scope grant change.
async fn invalidate_caches(state: &GatewayState, user_id: &str) {
    if let Some(ref db_auth) = state.db_auth {
        db_auth.invalidate_user(user_id).await;
    }
    if let Some(ref pool) = state.workspace_pool {
        pool.invalidate_user(user_id).await;
    }
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
    check_scope_access(store.as_ref(), &user.user_id, &scope).await?;
    let grants = store
        .list_scope_grants_for_scope(&scope)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(serde_json::json!({ "grants": grants_to_json(&grants) })))
}

/// PUT /api/scope-grants/{scope}/{grantee} — grant a user access to a scope.
///
/// Authorization:
/// - Scope owners can grant read or read-write access.
/// - Writers can only grant read-only access (no privilege escalation).
///
/// The grantee must be an existing user.
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

    // Prevent granting a user access to their own scope.
    if grantee == scope {
        return Err((
            StatusCode::BAD_REQUEST,
            "Cannot grant a user access to their own scope".to_string(),
        ));
    }

    let is_owner = check_scope_access(store.as_ref(), &user.user_id, &scope).await?;

    let writable = body
        .get("writable")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    // Only scope owners can grant writable access.
    if writable && !is_owner {
        return Err((
            StatusCode::FORBIDDEN,
            "Only the scope owner can grant writable access".to_string(),
        ));
    }

    // Validate grantee exists.
    require_user_exists(store.as_ref(), &grantee).await?;

    let expires_at = parse_expires_at(&body)?;

    store
        .set_scope_grant(&grantee, &scope, writable, Some(&user.user_id), expires_at)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Invalidate caches so the grant takes effect immediately.
    invalidate_caches(&state, &grantee).await;

    Ok(Json(serde_json::json!({
        "user_id": grantee,
        "scope": scope,
        "writable": writable,
    })))
}

/// DELETE /api/scope-grants/{scope}/{grantee} — revoke a user's access.
///
/// Authorization:
/// - Scope owners can revoke any grant.
/// - Writers can only revoke grants they themselves created (matched by `granted_by`).
pub async fn scope_grant_revoke_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path((scope, grantee)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    let is_owner = check_scope_access(store.as_ref(), &user.user_id, &scope).await?;

    let deleted = if is_owner {
        // Owners can revoke any grant on their scope.
        store
            .revoke_scope_grant(&grantee, &scope)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    } else {
        // Non-owner writers can only revoke grants they created.
        store
            .revoke_scope_grant_by_granter(&grantee, &scope, &user.user_id)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    };

    if deleted {
        // Invalidate caches so the revocation takes effect immediately.
        invalidate_caches(&state, &grantee).await;
        Ok(Json(serde_json::json!({ "deleted": true })))
    } else {
        Err((StatusCode::NOT_FOUND, "Scope grant not found".to_string()))
    }
}
