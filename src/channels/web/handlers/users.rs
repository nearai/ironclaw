//! User management API handlers (admin).

use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use rand::RngCore;
use uuid::Uuid;

use crate::channels::web::auth::{AdminUser, AuthenticatedUser};
use crate::channels::web::server::GatewayState;
use crate::db::{Database, UserRecord};
use crate::tools::permissions::ADMIN_SETTINGS_USER_ID;

/// Check whether `user_id` is the sole active admin. Returns true if demoting,
/// suspending, or deleting this user would leave zero admins.
async fn is_last_admin(store: &dyn Database, user_id: &str) -> Result<bool, String> {
    let users = store
        .list_users(Some("active"))
        .await
        .map_err(|e| e.to_string())?;
    let active_admins: Vec<_> = users.iter().filter(|u| u.is_admin()).collect();
    Ok(active_admins.len() == 1 && active_admins[0].id == user_id)
}

/// POST /api/admin/users — create a new user.
pub async fn users_create_handler(
    State(state): State<Arc<GatewayState>>,
    AdminUser(user): AdminUser,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    let display_name = body
        .get("display_name")
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .ok_or((
            StatusCode::BAD_REQUEST,
            "Missing or empty 'display_name'".to_string(),
        ))?
        .to_string();

    let email = body
        .get("email")
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(String::from);
    let role = body
        .get("role")
        .and_then(|v| v.as_str())
        .unwrap_or("member")
        .to_string();
    if role != "admin" && role != "member" {
        return Err((
            StatusCode::BAD_REQUEST,
            "role must be 'admin' or 'member'".to_string(),
        ));
    }

    let user_id = Uuid::new_v4().to_string();
    if user_id == ADMIN_SETTINGS_USER_ID {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            "Generated user id collided with reserved admin settings scope".to_string(),
        ));
    }

    let now = chrono::Utc::now();
    let user_record = UserRecord {
        id: user_id.clone(),
        email,
        display_name: display_name.clone(),
        status: "active".to_string(),
        role,
        created_at: now,
        updated_at: now,
        last_login_at: None,
        created_by: Some(user.user_id.clone()),
        metadata: serde_json::json!({}),
    };

    // Generate a first API token so the new user can authenticate immediately.
    // Hash the hex-encoded plaintext (what the user sends as Bearer token),
    // NOT the raw bytes — must match hash_token() in auth.rs.
    let mut token_bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut token_bytes);
    let plaintext_token = hex::encode(token_bytes);
    let token_hash = crate::channels::web::auth::hash_token(&plaintext_token);
    let token_prefix = &plaintext_token[..8];

    // Create user and initial token atomically — if either fails, both roll back.
    let _token_record = store
        .create_user_with_token(&user_record, "initial", &token_hash, token_prefix, None)
        .await
        .map_err(|e| {
            let msg = e.to_string();
            let lower = msg.to_ascii_lowercase();
            if lower.contains("unique")
                || lower.contains("duplicate")
                || lower.contains("already exists")
            {
                (StatusCode::CONFLICT, msg)
            } else {
                (StatusCode::INTERNAL_SERVER_ERROR, msg)
            }
        })?;

    Ok(Json(serde_json::json!({
        "id": user_record.id,
        "email": user_record.email,
        "display_name": user_record.display_name,
        "status": user_record.status,
        "role": user_record.role,
        "token": plaintext_token,
        "created_at": user_record.created_at.to_rfc3339(),
        "created_by": user_record.created_by,
    })))
}

/// GET /api/admin/users — list all users with inline usage stats.
pub async fn users_list_handler(
    State(state): State<Arc<GatewayState>>,
    AdminUser(_user): AdminUser,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    let users = store
        .list_users(None)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Fetch per-user summary stats from DB (agent_jobs + llm_calls).
    let summary_stats = store
        .user_summary_stats(None)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let stats_map: std::collections::HashMap<String, _> = summary_stats
        .into_iter()
        .map(|s| (s.user_id.clone(), s))
        .collect();

    let mut users_json: Vec<serde_json::Value> = Vec::with_capacity(users.len());
    for u in users {
        let db_stats = stats_map.get(&u.id);
        let total_cost = db_stats.map_or(rust_decimal::Decimal::ZERO, |s| s.total_cost);

        // Last active: prefer DB timestamp, fall back to last_login_at.
        let last_active = db_stats.and_then(|s| s.last_active_at).or(u.last_login_at);

        users_json.push(serde_json::json!({
            "id": u.id,
            "email": u.email,
            "display_name": u.display_name,
            "status": u.status,
            "role": u.role,
            "created_at": u.created_at.to_rfc3339(),
            "updated_at": u.updated_at.to_rfc3339(),
            "last_login_at": u.last_login_at.map(|dt| dt.to_rfc3339()),
            "created_by": u.created_by,
            "job_count": db_stats.map_or(0, |s| s.job_count),
            "total_cost": total_cost.to_string(),
            "last_active_at": last_active.map(|dt| dt.to_rfc3339()),
        }));
    }

    Ok(Json(serde_json::json!({ "users": users_json })))
}

/// GET /api/admin/users/{id} — get a single user.
pub async fn users_detail_handler(
    State(state): State<Arc<GatewayState>>,
    AdminUser(_user): AdminUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    let user_record = store
        .get_user(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "User not found".to_string()))?;

    Ok(Json(serde_json::json!({
        "id": user_record.id,
        "email": user_record.email,
        "display_name": user_record.display_name,
        "status": user_record.status,
        "role": user_record.role,
        "created_at": user_record.created_at.to_rfc3339(),
        "updated_at": user_record.updated_at.to_rfc3339(),
        "last_login_at": user_record.last_login_at.map(|dt| dt.to_rfc3339()),
        "created_by": user_record.created_by,
        "metadata": user_record.metadata,
    })))
}

/// PATCH /api/admin/users/{id} — update a user's profile.
pub async fn users_update_handler(
    State(state): State<Arc<GatewayState>>,
    AdminUser(_user): AdminUser,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    // Verify the user exists.
    let existing = store
        .get_user(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "User not found".to_string()))?;

    let display_name = body
        .get("display_name")
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .unwrap_or(&existing.display_name);

    let metadata = if let Some(m) = body.get("metadata") {
        if !m.is_object() {
            return Err((
                StatusCode::BAD_REQUEST,
                "metadata must be a JSON object".to_string(),
            ));
        }
        m
    } else {
        &existing.metadata
    };

    // Update role if provided and valid.
    if let Some(role) = body.get("role").and_then(|v| v.as_str()) {
        if role != "admin" && role != "member" {
            return Err((
                StatusCode::BAD_REQUEST,
                "role must be 'admin' or 'member'".to_string(),
            ));
        }
        if role != existing.role {
            // Prevent demoting the last admin.
            if existing.is_admin()
                && role == "member"
                && is_last_admin(store.as_ref(), &id)
                    .await
                    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
            {
                return Err((
                    StatusCode::CONFLICT,
                    "Cannot demote the last admin".to_string(),
                ));
            }
            store
                .update_user_role(&id, role)
                .await
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
            // Evict cached auth so role change takes effect immediately.
            if let Some(ref db_auth) = state.db_auth {
                db_auth.invalidate_user(&id).await;
            }
        }
    }

    store
        .update_user_profile(&id, display_name, metadata)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Re-fetch the updated record to return consistent data.
    let updated = store
        .get_user(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "User not found".to_string()))?;

    Ok(Json(serde_json::json!({
        "id": updated.id,
        "email": updated.email,
        "display_name": updated.display_name,
        "status": updated.status,
        "role": updated.role,
        "created_at": updated.created_at.to_rfc3339(),
        "updated_at": updated.updated_at.to_rfc3339(),
        "metadata": updated.metadata,
    })))
}

/// POST /api/admin/users/{id}/suspend — suspend a user.
pub async fn users_suspend_handler(
    State(state): State<Arc<GatewayState>>,
    AdminUser(_user): AdminUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    // Verify the user exists.
    store
        .get_user(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "User not found".to_string()))?;

    // Prevent suspending the last admin.
    if is_last_admin(store.as_ref(), &id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
    {
        return Err((
            StatusCode::CONFLICT,
            "Cannot suspend the last admin".to_string(),
        ));
    }

    store
        .update_user_status(&id, "suspended")
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Evict cached auth so suspension takes effect immediately.
    if let Some(ref db_auth) = state.db_auth {
        db_auth.invalidate_user(&id).await;
    }

    // Evict cached ownership identity so the suspended user cannot
    // resolve via pairing cache until process restart or re-approval.
    if let Some(ref ps) = state.pairing_store {
        ps.evict_user(&id);
    }

    // Drop the suspended user's auth-descriptor cache entry so any
    // in-flight credential resolution falls back to the live store
    // (which now sees the user as suspended) instead of serving stale
    // metadata until the 60s TTL expires.
    crate::auth::invalidate_auth_descriptor_cache(&id).await;

    Ok(Json(serde_json::json!({
        "id": id,
        "status": "suspended",
    })))
}

/// POST /api/admin/users/{id}/activate — activate a user.
pub async fn users_activate_handler(
    State(state): State<Arc<GatewayState>>,
    AdminUser(_user): AdminUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    // Verify the user exists.
    store
        .get_user(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "User not found".to_string()))?;

    store
        .update_user_status(&id, "active")
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Evict cached auth so reactivation takes effect immediately.
    if let Some(ref db_auth) = state.db_auth {
        db_auth.invalidate_user(&id).await;
    }

    Ok(Json(serde_json::json!({
        "id": id,
        "status": "active",
    })))
}

/// DELETE /api/admin/users/{id} — delete a user and all their data.
pub async fn users_delete_handler(
    State(state): State<Arc<GatewayState>>,
    AdminUser(_user): AdminUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    // Prevent deleting the last admin.
    if is_last_admin(store.as_ref(), &id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e))?
    {
        return Err((
            StatusCode::CONFLICT,
            "Cannot delete the last admin".to_string(),
        ));
    }

    let deleted = store
        .delete_user(&id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if !deleted {
        return Err((StatusCode::NOT_FOUND, "User not found".to_string()));
    }

    if let Some(ref db_auth) = state.db_auth {
        db_auth.invalidate_user(&id).await;
    }

    if let Some(ref ps) = state.pairing_store {
        ps.evict_user(&id);
    }

    // Drop the deleted user's auth-descriptor cache entry. The 60s TTL
    // would otherwise let the in-process cache keep serving the deleted
    // user's credential metadata until expiry, even though the underlying
    // rows are gone.
    crate::auth::invalidate_auth_descriptor_cache(&id).await;

    Ok(Json(serde_json::json!({
        "id": id,
        "deleted": true,
    })))
}

/// GET /api/profile — get the authenticated user's own profile.
pub async fn profile_get_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    let record = store
        .get_user(&user.user_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "User not found".to_string()))?;

    // Try to get avatar_url from linked OAuth identities.
    let identities = match store.list_identities_for_user(&user.user_id).await {
        Ok(ids) => ids,
        Err(e) => {
            tracing::warn!(user_id = %user.user_id, error = %e, "Failed to fetch identities for avatar");
            Vec::new()
        }
    };
    let avatar_url = identities.iter().find_map(|id| id.avatar_url.clone());
    tracing::trace!(
        user_id = %user.user_id,
        identity_count = identities.len(),
        avatar_url = ?avatar_url,
        "Profile handler: fetched avatar_url from identities"
    );

    Ok(Json(serde_json::json!({
        "id": record.id,
        "email": record.email,
        "display_name": record.display_name,
        "status": record.status,
        "role": record.role,
        "avatar_url": avatar_url,
        "created_at": record.created_at.to_rfc3339(),
        "last_login_at": record.last_login_at.map(|dt| dt.to_rfc3339()),
    })))
}

/// PATCH /api/profile — update the authenticated user's own profile.
pub async fn profile_update_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    let current = store
        .get_user(&user.user_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "User not found".to_string()))?;

    let display_name = body
        .get("display_name")
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .unwrap_or(&current.display_name);
    let metadata = if let Some(m) = body.get("metadata") {
        if !m.is_object() {
            return Err((
                StatusCode::BAD_REQUEST,
                "metadata must be a JSON object".to_string(),
            ));
        }
        m
    } else {
        &current.metadata
    };

    store
        .update_user_profile(&user.user_id, display_name, metadata)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(serde_json::json!({
        "id": user.user_id,
        "display_name": display_name,
        "updated": true,
    })))
}

/// GET /api/admin/usage — per-user LLM usage stats.
pub async fn usage_stats_handler(
    State(state): State<Arc<GatewayState>>,
    AdminUser(_user): AdminUser,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    let user_id = params.get("user_id").map(|s| s.as_str());
    let period = params.get("period").map(|s| s.as_str()).unwrap_or("day");
    let since = match period {
        "week" => chrono::Utc::now() - chrono::Duration::days(7),
        "month" => chrono::Utc::now() - chrono::Duration::days(30),
        _ => chrono::Utc::now() - chrono::Duration::days(1),
    };

    let stats = store
        .user_usage_stats(user_id, since)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let entries: Vec<serde_json::Value> = stats
        .iter()
        .map(|s| {
            serde_json::json!({
                "user_id": s.user_id,
                "model": s.model,
                "call_count": s.call_count,
                "input_tokens": s.input_tokens,
                "output_tokens": s.output_tokens,
                "total_cost": s.total_cost.to_string(),
            })
        })
        .collect();

    Ok(Json(serde_json::json!({
        "period": period,
        "since": since.to_rfc3339(),
        "usage": entries,
    })))
}
