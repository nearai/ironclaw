//! Public webhook trigger endpoint for routine webhook triggers.
//!
//! `POST /api/webhooks/{path}` — matches the path against routines with
//! `Trigger::Webhook { path, secret }`, validates the secret via constant-time
//! comparison, and fires the matching routine through the `RoutineEngine`.

use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use subtle::ConstantTimeEq;

use crate::agent::routine::Trigger;
use crate::channels::web::server::GatewayState;

/// Validate the webhook secret for a routine.
///
/// Returns `Ok(())` if the routine has a configured secret and the provided
/// secret matches via constant-time comparison. Returns an appropriate HTTP
/// error if the secret is missing (403) or invalid (401).
fn validate_webhook_secret(
    trigger: &Trigger,
    provided_secret: &str,
) -> Result<(), (StatusCode, String)> {
    // Require webhook secret — routines without a secret cannot be triggered via webhook
    let expected_secret = match trigger {
        Trigger::Webhook {
            secret: Some(s), ..
        } => s,
        _ => {
            return Err((
                StatusCode::FORBIDDEN,
                "Webhook secret not configured for this routine. \
                 Set a secret with: ironclaw routine update <id> --webhook-secret <secret>"
                    .to_string(),
            ));
        }
    };

    if !bool::from(provided_secret.as_bytes().ct_eq(expected_secret.as_bytes())) {
        return Err((
            StatusCode::UNAUTHORIZED,
            "Invalid webhook secret".to_string(),
        ));
    }

    Ok(())
}

/// Handle incoming webhook POST to `/api/webhooks/{path}`.
///
/// This endpoint is **public** (no gateway auth token required) but protected
/// by the per-routine webhook secret sent via the `X-Webhook-Secret` header.
///
/// **Single-user/backward-compatible**: looks up routines by path across all
/// users. Disabled in multi-tenant mode; use the user-scoped endpoint at
/// `/api/webhooks/u/{user_id}/{path}` instead.
pub async fn webhook_trigger_handler(
    State(state): State<Arc<GatewayState>>,
    Path(path): Path<String>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    fire_webhook_inner(state, &path, None, &headers).await
}

/// Handle incoming webhook POST to `/api/webhooks/u/{user_id}/{path}`.
///
/// User-scoped variant for multi-tenant deployments. The `user_id` in the URL
/// restricts the routine lookup to that user only, preventing cross-user
/// webhook triggering even when paths collide.
pub async fn webhook_trigger_user_scoped_handler(
    State(state): State<Arc<GatewayState>>,
    Path((user_id, path)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    fire_webhook_inner(state, &path, Some(&user_id), &headers).await
}

/// Shared webhook logic for both scoped and unscoped endpoints.
async fn fire_webhook_inner(
    state: Arc<GatewayState>,
    path: &str,
    user_id: Option<&str>,
    headers: &HeaderMap,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    if user_id.is_none() && state.workspace_pool.is_some() {
        return Err((
            StatusCode::GONE,
            "Unscoped webhooks are disabled in multi-tenant mode. Use /api/webhooks/u/{user_id}/{path}."
                .to_string(),
        ));
    }

    // Rate limit check
    if !state.webhook_rate_limiter.check() {
        return Err((
            StatusCode::TOO_MANY_REQUESTS,
            "Rate limit exceeded. Try again shortly.".to_string(),
        ));
    }

    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    // Targeted query — when user_id is provided, restrict to that user's routines
    let routine = store
        .get_webhook_routine_by_path(path, user_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((
            StatusCode::NOT_FOUND,
            "No routine matches this webhook path".to_string(),
        ))?;

    let provided_secret = headers
        .get("x-webhook-secret")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    validate_webhook_secret(&routine.trigger, provided_secret)?;

    // Fire through the RoutineEngine so guardrails, run tracking,
    // notifications, and FullJob dispatch all work correctly.
    let engine = {
        let guard = state.routine_engine.read().await;
        guard.as_ref().cloned().ok_or((
            StatusCode::SERVICE_UNAVAILABLE,
            "Routine engine not available".to_string(),
        ))?
    };

    let run_id = engine.fire_webhook(routine.id, path).await.map_err(|e| {
        let status = match &e {
            crate::error::RoutineError::NotFound { .. } => StatusCode::NOT_FOUND,
            crate::error::RoutineError::Disabled { .. }
            | crate::error::RoutineError::Cooldown { .. }
            | crate::error::RoutineError::MaxConcurrent { .. } => StatusCode::CONFLICT,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (status, e.to_string())
    })?;

    Ok(Json(serde_json::json!({
        "status": "triggered",
        "routine_id": routine.id,
        "routine_name": routine.name,
        "run_id": run_id,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use crate::channels::web::server::{
        ActiveConfigSnapshot, GatewayState, PerUserRateLimiter, RateLimiter, WorkspacePool,
    };
    use crate::channels::web::sse::SseManager;
    use crate::config::{WorkspaceConfig, WorkspaceSearchConfig};
    use crate::workspace::EmbeddingCacheConfig;

    /// Routines with `secret: None` must be rejected with 403.
    #[test]
    fn test_validate_rejects_missing_secret() {
        let trigger = Trigger::Webhook {
            path: Some("my-hook".to_string()),
            secret: None,
        };
        let result = validate_webhook_secret(&trigger, "any-secret");
        let (status, msg) = result.unwrap_err();
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert!(
            msg.contains("not configured"),
            "Error should tell user to configure a secret, got: {msg}"
        );
    }

    /// Non-webhook triggers must be rejected with 403.
    #[test]
    fn test_validate_rejects_non_webhook_trigger() {
        let trigger = Trigger::Manual;
        let result = validate_webhook_secret(&trigger, "any-secret");
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    /// Correct secret passes validation.
    #[test]
    fn test_validate_accepts_correct_secret() {
        let trigger = Trigger::Webhook {
            path: Some("my-hook".to_string()),
            secret: Some("s3cret-token".to_string()),
        };
        assert!(validate_webhook_secret(&trigger, "s3cret-token").is_ok());
    }

    /// Wrong secret returns 401.
    #[test]
    fn test_validate_rejects_wrong_secret() {
        let trigger = Trigger::Webhook {
            path: Some("my-hook".to_string()),
            secret: Some("correct-secret".to_string()),
        };
        let result = validate_webhook_secret(&trigger, "wrong-secret");
        let (status, msg) = result.unwrap_err();
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert!(msg.contains("Invalid"), "Expected 'Invalid' in: {msg}");
    }

    /// Empty provided secret returns 401 (not a false positive).
    #[test]
    fn test_validate_rejects_empty_provided_secret() {
        let trigger = Trigger::Webhook {
            path: Some("my-hook".to_string()),
            secret: Some("real-secret".to_string()),
        };
        let result = validate_webhook_secret(&trigger, "");
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    /// Constant-time comparison: secrets of different lengths are still rejected
    /// (not short-circuited in a way that leaks length info).
    #[test]
    fn test_validate_rejects_different_length_secret() {
        let trigger = Trigger::Webhook {
            path: None,
            secret: Some("short".to_string()),
        };
        let result = validate_webhook_secret(&trigger, "a-much-longer-secret-value");
        let (status, _) = result.unwrap_err();
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn test_unscoped_webhooks_are_disabled_in_multi_tenant_mode() {
        use crate::db::Database;

        let backend = crate::db::libsql::LibSqlBackend::new_memory()
            .await
            .unwrap();
        backend.run_migrations().await.unwrap();
        let store: Arc<dyn Database> = Arc::new(backend);
        let workspace_pool = Arc::new(WorkspacePool::new(
            Arc::clone(&store),
            None,
            EmbeddingCacheConfig::default(),
            WorkspaceSearchConfig::default(),
            WorkspaceConfig::default(),
        ));

        let state = Arc::new(GatewayState {
            msg_tx: tokio::sync::RwLock::new(None),
            sse: Arc::new(SseManager::new()),
            workspace: None,
            workspace_pool: Some(workspace_pool),
            session_manager: None,
            log_broadcaster: None,
            log_level_handle: None,
            extension_manager: None,
            tool_registry: None,
            store: Some(store),
            job_manager: None,
            prompt_queue: None,
            owner_id: "test".to_string(),
            shutdown_tx: tokio::sync::RwLock::new(None),
            ws_tracker: None,
            llm_provider: None,
            skill_registry: None,
            skill_catalog: None,
            scheduler: None,
            chat_rate_limiter: PerUserRateLimiter::new(30, 60),
            oauth_rate_limiter: PerUserRateLimiter::new(10, 60),
            webhook_rate_limiter: RateLimiter::new(10, 60),
            registry_entries: Vec::new(),
            cost_guard: None,
            routine_engine: Arc::new(tokio::sync::RwLock::new(None)),
            startup_time: std::time::Instant::now(),
            active_config: ActiveConfigSnapshot::default(),
            secrets_store: None,
            db_auth: None,
            oauth_providers: None,
            oauth_state_store: None,
            oauth_base_url: None,
            oauth_allowed_domains: Vec::new(),
            near_nonce_store: None,
            near_rpc_url: None,
            near_network: None,
            oauth_sweep_shutdown: None,
        });

        let err = fire_webhook_inner(state, "my-hook", None, &HeaderMap::new())
            .await
            .unwrap_err();
        assert_eq!(err.0, StatusCode::GONE);
        assert!(err.1.contains("/api/webhooks/u/{user_id}/{path}"));
    }
}
