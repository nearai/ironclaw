//! Admin secrets provisioning handlers.
//!
//! Allows an admin (typically an application backend) to create, list, and
//! delete secrets on behalf of individual users so their IronClaw agent can
//! call back to external services with per-user credentials.

use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};

use crate::channels::web::auth::AdminUser;
use crate::channels::web::platform::state::GatewayState;
use crate::secrets::CreateSecretParams;

/// PUT /api/admin/users/{user_id}/secrets/{name} — create or update a secret.
///
/// Upserts: if a secret with the same (user_id, name) already exists it is
/// overwritten. The plaintext value is encrypted at rest (AES-256-GCM) and
/// never returned by any endpoint.
pub async fn secrets_put_handler(
    State(state): State<Arc<GatewayState>>,
    AdminUser(_admin): AdminUser,
    Path((user_id, name)): Path<(String, String)>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let name = name.to_lowercase();

    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    store
        .get_user(&user_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "User not found".to_string()))?;

    let secrets = state.secrets_store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Secrets store not available".to_string(),
    ))?;

    let value = body
        .get("value")
        .and_then(|v| v.as_str())
        .ok_or((
            StatusCode::BAD_REQUEST,
            "Missing required field 'value'".to_string(),
        ))?
        .to_string();

    let provider = body
        .get("provider")
        .and_then(|v| v.as_str())
        .map(String::from);

    let expires_in_days = body.get("expires_in_days").and_then(|v| v.as_u64());
    if let Some(days) = expires_in_days
        && days > 36500
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "expires_in_days must be at most 36500".to_string(),
        ));
    }
    let expires_at =
        expires_in_days.map(|days| chrono::Utc::now() + chrono::Duration::days(days as i64));

    let mut params = CreateSecretParams::new(name.clone(), value);
    if let Some(p) = provider {
        params = params.with_provider(p);
    }
    if let Some(exp) = expires_at {
        params = params.with_expiry(exp);
    }

    let already_exists = secrets
        .exists(&user_id, &name)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    secrets
        .create(&user_id, params)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(serde_json::json!({
        "user_id": user_id,
        "name": name,
        "status": if already_exists { "updated" } else { "created" },
    })))
}

/// GET /api/admin/users/{user_id}/secrets — list a user's secrets (names only).
///
/// Never returns secret values or hashes.
pub async fn secrets_list_handler(
    State(state): State<Arc<GatewayState>>,
    AdminUser(_admin): AdminUser,
    Path(user_id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    // Verify the target user exists (consistent with PUT/DELETE).
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    if store
        .get_user(&user_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .is_none()
    {
        return Err((StatusCode::NOT_FOUND, "User not found".to_string()));
    }

    let secrets = state.secrets_store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Secrets store not available".to_string(),
    ))?;

    let refs = secrets
        .list(&user_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let secrets_json: Vec<serde_json::Value> = refs
        .into_iter()
        .map(|r| {
            serde_json::json!({
                "name": r.name,
                "provider": r.provider,
            })
        })
        .collect();

    Ok(Json(serde_json::json!({
        "user_id": user_id,
        "secrets": secrets_json,
    })))
}

/// DELETE /api/admin/users/{user_id}/secrets/{name} — delete a user's secret.
pub async fn secrets_delete_handler(
    State(state): State<Arc<GatewayState>>,
    AdminUser(_admin): AdminUser,
    Path((user_id, name)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let name = name.to_lowercase();

    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    store
        .get_user(&user_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "User not found".to_string()))?;

    let secrets = state.secrets_store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Secrets store not available".to_string(),
    ))?;

    let deleted = secrets
        .delete(&user_id, &name)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if !deleted {
        return Err((StatusCode::NOT_FOUND, "Secret not found".to_string()));
    }

    Ok(Json(serde_json::json!({
        "user_id": user_id,
        "name": name,
        "deleted": true,
    })))
}
