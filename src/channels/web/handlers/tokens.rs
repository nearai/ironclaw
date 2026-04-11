//! API token management handlers.

use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use rand::RngCore;
use rand::rngs::OsRng;
use uuid::Uuid;

use crate::channels::web::auth::AuthenticatedUser;
use crate::channels::web::server::GatewayState;

/// POST /api/tokens — create a new API token (returns plaintext ONCE).
pub async fn tokens_create_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let store = state.store().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    let name = body
        .get("name")
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .ok_or((
            StatusCode::BAD_REQUEST,
            "Missing or empty 'name'".to_string(),
        ))?
        .to_string();

    let expires_in_days: Option<i64> = match body.get("expires_in_days").and_then(|v| v.as_u64()) {
        Some(d) if d > 36500 => {
            return Err((
                StatusCode::BAD_REQUEST,
                "expires_in_days must not exceed 36500 (100 years)".to_string(),
            ));
        }
        Some(d) => Some(d as i64),
        None => None,
    };

    let expires_at = expires_in_days.map(|days| chrono::Utc::now() + chrono::Duration::days(days));

    // Generate 32 random bytes for the token.
    // Hash the hex-encoded plaintext (what the user sends as Bearer token),
    // NOT the raw bytes — must match hash_token() in auth.rs.
    let mut token_bytes = [0u8; 32];
    OsRng.fill_bytes(&mut token_bytes);
    let plaintext_token = hex::encode(token_bytes);
    let hash = crate::channels::web::auth::hash_token(&plaintext_token);

    // First 8 chars of the hex token as a prefix for identification.
    let token_prefix = &plaintext_token[..8];

    // Admin users can create tokens for other users via optional "user_id" field.
    let target_user = body
        .get("user_id")
        .and_then(|v| v.as_str())
        .filter(|_| user.role == "admin")
        .unwrap_or(&user.user_id);

    // Verify the target user exists to prevent orphan tokens.
    if target_user != user.user_id {
        store
            .get_user(target_user)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
            .ok_or((
                StatusCode::NOT_FOUND,
                format!("Target user '{target_user}' not found"),
            ))?;
    }

    let record = store
        .create_api_token(target_user, &name, &hash, token_prefix, expires_at)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Return the plaintext token — this is the ONLY time it is shown.
    Ok(Json(serde_json::json!({
        "token": plaintext_token,
        "id": record.id.to_string(),
        "name": record.name,
        "token_prefix": record.token_prefix,
        "expires_at": record.expires_at.map(|dt| dt.to_rfc3339()),
        "created_at": record.created_at.to_rfc3339(),
    })))
}

/// GET /api/tokens — list the current user's tokens (no hashes).
pub async fn tokens_list_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    let tokens = store
        .list_api_tokens(&user.user_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let tokens_json: Vec<serde_json::Value> = tokens
        .into_iter()
        .map(|t| {
            serde_json::json!({
                "id": t.id.to_string(),
                "name": t.name,
                "token_prefix": t.token_prefix,
                "expires_at": t.expires_at.map(|dt| dt.to_rfc3339()),
                "last_used_at": t.last_used_at.map(|dt| dt.to_rfc3339()),
                "created_at": t.created_at.to_rfc3339(),
                "revoked_at": t.revoked_at.map(|dt| dt.to_rfc3339()),
            })
        })
        .collect();

    Ok(Json(serde_json::json!({ "tokens": tokens_json })))
}

/// DELETE /api/tokens/{id} — revoke a token.
pub async fn tokens_revoke_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    let token_id = Uuid::parse_str(&id)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid token ID".to_string()))?;

    let revoked = store
        .revoke_api_token(token_id, &user.user_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if !revoked {
        return Err((StatusCode::NOT_FOUND, "Token not found".to_string()));
    }

    // Evict cached auth so revocation takes effect immediately.
    if let Some(ref db_auth) = state.db_auth {
        db_auth.invalidate_user(&user.user_id).await;
    }

    Ok(Json(serde_json::json!({
        "status": "revoked",
        "id": token_id.to_string(),
    })))
}
