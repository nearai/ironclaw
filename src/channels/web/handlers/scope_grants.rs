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

    let mut resp = serde_json::json!({
        "user_id": user_id,
        "scope": scope,
        "writable": writable,
    });
    if let Some(ref exp) = expires_at {
        resp["expires_at"] = serde_json::json!(exp.to_rfc3339());
    }
    Ok(Json(resp))
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

    let mut resp = serde_json::json!({
        "user_id": grantee,
        "scope": scope,
        "writable": writable,
    });
    if let Some(ref exp) = expires_at {
        resp["expires_at"] = serde_json::json!(exp.to_rfc3339());
    }
    Ok(Json(resp))
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

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
#[cfg(feature = "libsql")]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use axum::Router;
    use axum::body::Body;
    use axum::http::{Method, Request, StatusCode};
    use axum::middleware;
    use axum::routing::{get, put};
    use tower::ServiceExt;

    use crate::channels::web::auth::{MultiAuthState, UserIdentity, auth_middleware};
    use crate::channels::web::server::{GatewayState, PerUserRateLimiter, RateLimiter};
    use crate::channels::web::sse::SseManager;
    use crate::db::{Database, UserRecord};

    // ── Helpers ────────────────────────────────────────────────────────

    async fn test_db() -> (Arc<dyn Database>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.db");
        let backend = crate::db::libsql::LibSqlBackend::new_local(&path)
            .await
            .unwrap();
        backend.run_migrations().await.unwrap();
        (Arc::new(backend) as Arc<dyn Database>, dir)
    }

    fn make_user(id: &str) -> UserRecord {
        let now = chrono::Utc::now();
        UserRecord {
            id: id.to_string(),
            email: None,
            display_name: id.to_string(),
            status: "active".to_string(),
            role: "member".to_string(),
            created_at: now,
            updated_at: now,
            last_login_at: None,
            created_by: None,
            metadata: serde_json::json!({}),
        }
    }

    fn build_state(store: Arc<dyn Database>) -> Arc<GatewayState> {
        Arc::new(GatewayState {
            msg_tx: tokio::sync::RwLock::new(None),
            sse: Arc::new(SseManager::new()),
            workspace: None,
            workspace_pool: None,
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
            auth_manager: None,
            scheduler: None,
            chat_rate_limiter: PerUserRateLimiter::new(30, 60),
            oauth_rate_limiter: PerUserRateLimiter::new(20, 60),
            webhook_rate_limiter: RateLimiter::new(10, 60),
            registry_entries: Vec::new(),
            cost_guard: None,
            routine_engine: Arc::new(tokio::sync::RwLock::new(None)),
            startup_time: std::time::Instant::now(),
            active_config: crate::channels::web::server::ActiveConfigSnapshot::default(),
            secrets_store: None,
            db_auth: None,
            pairing_store: None,
            oauth_providers: None,
            oauth_state_store: None,
            oauth_base_url: None,
            oauth_allowed_domains: Vec::new(),
            near_nonce_store: None,
            near_rpc_url: None,
            near_network: None,
            oauth_sweep_shutdown: None,
            frontend_html_cache: std::sync::Arc::new(tokio::sync::RwLock::new(None)),
            tool_dispatcher: None,
        })
    }

    /// Build a router with admin scope-grant endpoints.
    fn admin_router(state: Arc<GatewayState>, auth: MultiAuthState) -> Router {
        Router::new()
            .route(
                "/api/admin/users/{user_id}/scope-grants",
                get(super::scope_grants_list_handler),
            )
            .route(
                "/api/admin/users/{user_id}/scope-grants/{scope}",
                put(super::scope_grants_set_handler)
                    .delete(super::scope_grants_delete_handler),
            )
            .route(
                "/api/admin/scope-grants/by-scope/{scope}",
                get(super::scope_grants_by_scope_handler),
            )
            .layer(middleware::from_fn_with_state(
                crate::channels::web::auth::CombinedAuthState::from(auth),
                auth_middleware,
            ))
            .with_state(state)
    }

    /// Build a router with self-service scope-grant endpoints.
    fn self_service_router(state: Arc<GatewayState>, auth: MultiAuthState) -> Router {
        Router::new()
            .route(
                "/api/scope-grants",
                get(super::my_scope_grants_handler),
            )
            .route(
                "/api/scope-grants/{scope}",
                get(super::scope_members_handler),
            )
            .route(
                "/api/scope-grants/{scope}/{grantee}",
                put(super::scope_grant_set_handler)
                    .delete(super::scope_grant_revoke_handler),
            )
            .layer(middleware::from_fn_with_state(
                crate::channels::web::auth::CombinedAuthState::from(auth),
                auth_middleware,
            ))
            .with_state(state)
    }

    fn admin_auth() -> MultiAuthState {
        let mut tokens = HashMap::new();
        tokens.insert(
            "tok-admin".to_string(),
            UserIdentity {
                user_id: "admin-user".to_string(),
                role: "admin".to_string(),
                workspace_read_scopes: vec![],
                workspace_write_scopes: vec![],
            },
        );
        MultiAuthState::multi(tokens)
    }

    fn self_service_auth() -> MultiAuthState {
        let mut tokens = HashMap::new();
        // alice owns the "alice" scope
        tokens.insert(
            "tok-alice".to_string(),
            UserIdentity {
                user_id: "alice".to_string(),
                role: "member".to_string(),
                workspace_read_scopes: vec![],
                workspace_write_scopes: vec![],
            },
        );
        // bob is a regular user
        tokens.insert(
            "tok-bob".to_string(),
            UserIdentity {
                user_id: "bob".to_string(),
                role: "member".to_string(),
                workspace_read_scopes: vec![],
                workspace_write_scopes: vec![],
            },
        );
        MultiAuthState::multi(tokens)
    }

    async fn body_json(resp: axum::http::Response<Body>) -> serde_json::Value {
        let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    async fn body_string(resp: axum::http::Response<Body>) -> String {
        let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
            .await
            .unwrap();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    // ── Admin endpoint tests ───────────────────────────────────────────

    #[tokio::test]
    async fn admin_self_grant_prevention() {
        let (db, _dir) = test_db().await;
        db.create_user(&make_user("alice")).await.unwrap();
        let app = admin_router(build_state(db), admin_auth());

        // PUT /api/admin/users/alice/scope-grants/alice should return 400
        let req = Request::builder()
            .method(Method::PUT)
            .uri("/api/admin/users/alice/scope-grants/alice")
            .header("Authorization", "Bearer tok-admin")
            .header("Content-Type", "application/json")
            .body(Body::from(r#"{"writable": false}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let text = body_string(resp).await;
        assert!(
            text.contains("own scope"),
            "error should mention own scope: {text}"
        );
    }

    #[tokio::test]
    async fn admin_scope_validation_nonexistent_user() {
        let (db, _dir) = test_db().await;
        // alice exists, but "nonexistent" does not
        db.create_user(&make_user("alice")).await.unwrap();
        let app = admin_router(build_state(db), admin_auth());

        // PUT with a non-existent scope user should return 404
        let req = Request::builder()
            .method(Method::PUT)
            .uri("/api/admin/users/alice/scope-grants/nonexistent")
            .header("Authorization", "Bearer tok-admin")
            .header("Content-Type", "application/json")
            .body(Body::from(r#"{"writable": false}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        let text = body_string(resp).await;
        assert!(
            text.contains("nonexistent"),
            "error should mention the missing user: {text}"
        );
    }

    #[tokio::test]
    async fn admin_set_grant_happy_path() {
        let (db, _dir) = test_db().await;
        db.create_user(&make_user("alice")).await.unwrap();
        db.create_user(&make_user("shared")).await.unwrap();
        let state = build_state(Arc::clone(&db));
        let app = admin_router(state, admin_auth());

        // PUT to create a grant
        let req = Request::builder()
            .method(Method::PUT)
            .uri("/api/admin/users/alice/scope-grants/shared")
            .header("Authorization", "Bearer tok-admin")
            .header("Content-Type", "application/json")
            .body(Body::from(r#"{"writable": true}"#))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp).await;
        assert_eq!(json["user_id"], "alice");
        assert_eq!(json["scope"], "shared");
        assert_eq!(json["writable"], true);

        // GET to verify the grant was stored
        let req = Request::builder()
            .uri("/api/admin/users/alice/scope-grants")
            .header("Authorization", "Bearer tok-admin")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp).await;
        let grants = json["grants"].as_array().unwrap();
        assert_eq!(grants.len(), 1);
        assert_eq!(grants[0]["scope"], "shared");
        assert_eq!(grants[0]["writable"], true);
    }

    #[tokio::test]
    async fn admin_set_grant_with_expires_at() {
        let (db, _dir) = test_db().await;
        db.create_user(&make_user("alice")).await.unwrap();
        db.create_user(&make_user("shared")).await.unwrap();
        let state = build_state(Arc::clone(&db));
        let app = admin_router(state, admin_auth());

        let future = chrono::Utc::now() + chrono::Duration::hours(1);
        let body = serde_json::json!({
            "writable": false,
            "expires_at": future.to_rfc3339(),
        });

        let req = Request::builder()
            .method(Method::PUT)
            .uri("/api/admin/users/alice/scope-grants/shared")
            .header("Authorization", "Bearer tok-admin")
            .header("Content-Type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // Verify via DB that expires_at was stored
        let grant = db.get_scope_grant("alice", "shared").await.unwrap().unwrap();
        assert!(grant.expires_at.is_some());
    }

    #[tokio::test]
    async fn admin_revoke_happy_path() {
        let (db, _dir) = test_db().await;
        db.create_user(&make_user("alice")).await.unwrap();
        db.create_user(&make_user("shared")).await.unwrap();
        let state = build_state(Arc::clone(&db));
        let app = admin_router(state, admin_auth());

        // Create a grant first
        db.set_scope_grant("alice", "shared", false, Some("admin-user"), None)
            .await
            .unwrap();

        // DELETE to revoke
        let req = Request::builder()
            .method(Method::DELETE)
            .uri("/api/admin/users/alice/scope-grants/shared")
            .header("Authorization", "Bearer tok-admin")
            .body(Body::empty())
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp).await;
        assert_eq!(json["deleted"], true);

        // Verify grant is removed
        let grant = db.get_scope_grant("alice", "shared").await.unwrap();
        assert!(grant.is_none());
    }

    #[tokio::test]
    async fn admin_revoke_nonexistent_returns_404() {
        let (db, _dir) = test_db().await;
        db.create_user(&make_user("alice")).await.unwrap();
        let app = admin_router(build_state(db), admin_auth());

        let req = Request::builder()
            .method(Method::DELETE)
            .uri("/api/admin/users/alice/scope-grants/nonexistent")
            .header("Authorization", "Bearer tok-admin")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    // ── Self-service endpoint tests ────────────────────────────────────

    #[tokio::test]
    async fn self_service_self_grant_prevention() {
        let (db, _dir) = test_db().await;
        db.create_user(&make_user("alice")).await.unwrap();
        db.create_user(&make_user("bob")).await.unwrap();
        let app = self_service_router(build_state(db), self_service_auth());

        // alice tries to grant alice access to her own scope: PUT /api/scope-grants/alice/alice
        let req = Request::builder()
            .method(Method::PUT)
            .uri("/api/scope-grants/alice/alice")
            .header("Authorization", "Bearer tok-alice")
            .header("Content-Type", "application/json")
            .body(Body::from(r#"{"writable": false}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn self_service_writer_cannot_grant_writable() {
        let (db, _dir) = test_db().await;
        db.create_user(&make_user("alice")).await.unwrap();
        db.create_user(&make_user("bob")).await.unwrap();
        db.create_user(&make_user("charlie")).await.unwrap();
        // Give bob writable access to alice's scope
        db.set_scope_grant("bob", "alice", true, Some("alice"), None)
            .await
            .unwrap();
        let state = build_state(db);

        // bob has writable access to alice's scope but is not the owner
        let mut tokens = HashMap::new();
        tokens.insert(
            "tok-bob".to_string(),
            UserIdentity {
                user_id: "bob".to_string(),
                role: "member".to_string(),
                workspace_read_scopes: vec!["alice".to_string()],
                workspace_write_scopes: vec!["alice".to_string()],
            },
        );
        let auth = MultiAuthState::multi(tokens);
        let app = self_service_router(state, auth);

        // bob tries to grant charlie writable access to alice's scope
        let req = Request::builder()
            .method(Method::PUT)
            .uri("/api/scope-grants/alice/charlie")
            .header("Authorization", "Bearer tok-bob")
            .header("Content-Type", "application/json")
            .body(Body::from(r#"{"writable": true}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::FORBIDDEN,
            "non-owner writer should not be able to grant writable access"
        );
    }

    #[tokio::test]
    async fn self_service_writer_can_grant_readonly() {
        let (db, _dir) = test_db().await;
        db.create_user(&make_user("alice")).await.unwrap();
        db.create_user(&make_user("bob")).await.unwrap();
        db.create_user(&make_user("charlie")).await.unwrap();
        // Give bob writable access to alice's scope
        db.set_scope_grant("bob", "alice", true, Some("alice"), None)
            .await
            .unwrap();
        let state = build_state(Arc::clone(&db));

        let mut tokens = HashMap::new();
        tokens.insert(
            "tok-bob".to_string(),
            UserIdentity {
                user_id: "bob".to_string(),
                role: "member".to_string(),
                workspace_read_scopes: vec!["alice".to_string()],
                workspace_write_scopes: vec!["alice".to_string()],
            },
        );
        let auth = MultiAuthState::multi(tokens);
        let app = self_service_router(state, auth);

        // bob grants charlie read-only access -- this should succeed
        let req = Request::builder()
            .method(Method::PUT)
            .uri("/api/scope-grants/alice/charlie")
            .header("Authorization", "Bearer tok-bob")
            .header("Content-Type", "application/json")
            .body(Body::from(r#"{"writable": false}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // Verify grant was created with bob as granter
        let grant = db.get_scope_grant("charlie", "alice").await.unwrap().unwrap();
        assert!(!grant.writable);
        assert_eq!(grant.granted_by.as_deref(), Some("bob"));
    }

    #[tokio::test]
    async fn self_service_writer_revocation_restriction() {
        let (db, _dir) = test_db().await;
        db.create_user(&make_user("alice")).await.unwrap();
        db.create_user(&make_user("bob")).await.unwrap();
        db.create_user(&make_user("charlie")).await.unwrap();
        db.create_user(&make_user("writer_a")).await.unwrap();
        db.create_user(&make_user("writer_b")).await.unwrap();

        // Give both writers writable access to alice's scope
        db.set_scope_grant("writer_a", "alice", true, Some("alice"), None)
            .await
            .unwrap();
        db.set_scope_grant("writer_b", "alice", true, Some("alice"), None)
            .await
            .unwrap();

        // writer_a grants charlie access
        db.set_scope_grant("charlie", "alice", false, Some("writer_a"), None)
            .await
            .unwrap();

        let state = build_state(Arc::clone(&db));

        // writer_b tries to revoke charlie's access -- should fail (not the granter)
        let mut tokens_b = HashMap::new();
        tokens_b.insert(
            "tok-writer-b".to_string(),
            UserIdentity {
                user_id: "writer_b".to_string(),
                role: "member".to_string(),
                workspace_read_scopes: vec!["alice".to_string()],
                workspace_write_scopes: vec!["alice".to_string()],
            },
        );
        let auth_b = MultiAuthState::multi(tokens_b);
        let app_b = self_service_router(Arc::clone(&state), auth_b);

        let req = Request::builder()
            .method(Method::DELETE)
            .uri("/api/scope-grants/alice/charlie")
            .header("Authorization", "Bearer tok-writer-b")
            .body(Body::empty())
            .unwrap();
        let resp = app_b.oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::NOT_FOUND,
            "writer_b should not be able to revoke writer_a's grant"
        );

        // writer_a can revoke their own grant
        let mut tokens_a = HashMap::new();
        tokens_a.insert(
            "tok-writer-a".to_string(),
            UserIdentity {
                user_id: "writer_a".to_string(),
                role: "member".to_string(),
                workspace_read_scopes: vec!["alice".to_string()],
                workspace_write_scopes: vec!["alice".to_string()],
            },
        );
        let auth_a = MultiAuthState::multi(tokens_a);
        let app_a = self_service_router(state, auth_a);

        let req = Request::builder()
            .method(Method::DELETE)
            .uri("/api/scope-grants/alice/charlie")
            .header("Authorization", "Bearer tok-writer-a")
            .body(Body::empty())
            .unwrap();
        let resp = app_a.oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "writer_a should be able to revoke their own grant"
        );

        // Verify the grant is actually removed
        let grant = db.get_scope_grant("charlie", "alice").await.unwrap();
        assert!(grant.is_none());
    }

    #[tokio::test]
    async fn self_service_owner_can_revoke_any_grant() {
        let (db, _dir) = test_db().await;
        db.create_user(&make_user("alice")).await.unwrap();
        db.create_user(&make_user("bob")).await.unwrap();
        db.create_user(&make_user("writer_a")).await.unwrap();

        // writer_a has writable access and granted bob access
        db.set_scope_grant("writer_a", "alice", true, Some("alice"), None)
            .await
            .unwrap();
        db.set_scope_grant("bob", "alice", false, Some("writer_a"), None)
            .await
            .unwrap();

        let state = build_state(Arc::clone(&db));

        // alice (the owner) revokes bob's access that writer_a granted
        let mut tokens = HashMap::new();
        tokens.insert(
            "tok-alice".to_string(),
            UserIdentity {
                user_id: "alice".to_string(),
                role: "member".to_string(),
                workspace_read_scopes: vec![],
                workspace_write_scopes: vec![],
            },
        );
        let auth = MultiAuthState::multi(tokens);
        let app = self_service_router(state, auth);

        let req = Request::builder()
            .method(Method::DELETE)
            .uri("/api/scope-grants/alice/bob")
            .header("Authorization", "Bearer tok-alice")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "owner should be able to revoke any grant on their scope"
        );

        let grant = db.get_scope_grant("bob", "alice").await.unwrap();
        assert!(grant.is_none());
    }

    #[tokio::test]
    async fn self_service_no_access_returns_403() {
        let (db, _dir) = test_db().await;
        db.create_user(&make_user("alice")).await.unwrap();
        db.create_user(&make_user("bob")).await.unwrap();
        db.create_user(&make_user("charlie")).await.unwrap();
        // bob has NO writable grant to alice's scope
        let state = build_state(db);

        let mut tokens = HashMap::new();
        tokens.insert(
            "tok-bob".to_string(),
            UserIdentity {
                user_id: "bob".to_string(),
                role: "member".to_string(),
                workspace_read_scopes: vec![],
                workspace_write_scopes: vec![],
            },
        );
        let auth = MultiAuthState::multi(tokens);
        let app = self_service_router(state, auth);

        // bob tries to grant charlie access to alice's scope -- 403
        let req = Request::builder()
            .method(Method::PUT)
            .uri("/api/scope-grants/alice/charlie")
            .header("Authorization", "Bearer tok-bob")
            .header("Content-Type", "application/json")
            .body(Body::from(r#"{"writable": false}"#))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn admin_by_scope_lists_all_grantees() {
        let (db, _dir) = test_db().await;
        db.create_user(&make_user("alice")).await.unwrap();
        db.create_user(&make_user("bob")).await.unwrap();
        db.create_user(&make_user("shared")).await.unwrap();

        db.set_scope_grant("alice", "shared", false, Some("admin"), None)
            .await
            .unwrap();
        db.set_scope_grant("bob", "shared", true, Some("admin"), None)
            .await
            .unwrap();

        let app = admin_router(build_state(db), admin_auth());

        let req = Request::builder()
            .uri("/api/admin/scope-grants/by-scope/shared")
            .header("Authorization", "Bearer tok-admin")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp).await;
        let grants = json["grants"].as_array().unwrap();
        assert_eq!(grants.len(), 2);
    }

    #[tokio::test]
    async fn self_service_my_grants_lists_own() {
        let (db, _dir) = test_db().await;
        db.create_user(&make_user("alice")).await.unwrap();
        db.create_user(&make_user("shared")).await.unwrap();

        db.set_scope_grant("alice", "shared", false, Some("admin"), None)
            .await
            .unwrap();

        let state = build_state(db);
        let mut tokens = HashMap::new();
        tokens.insert(
            "tok-alice".to_string(),
            UserIdentity {
                user_id: "alice".to_string(),
                role: "member".to_string(),
                workspace_read_scopes: vec![],
                workspace_write_scopes: vec![],
            },
        );
        let auth = MultiAuthState::multi(tokens);
        let app = self_service_router(state, auth);

        let req = Request::builder()
            .uri("/api/scope-grants")
            .header("Authorization", "Bearer tok-alice")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp).await;
        let grants = json["grants"].as_array().unwrap();
        assert_eq!(grants.len(), 1);
        assert_eq!(grants[0]["scope"], "shared");
    }

    // ── Cache invalidation (best-effort) ──────────────────────────────
    //
    // Direct cache invalidation testing requires injecting a DbAuthenticator
    // and WorkspacePool into GatewayState, which couples to too many
    // subsystems for unit tests. The invalidation paths are verified
    // structurally: `invalidate_caches()` is called in both set and revoke
    // handlers, and `DbAuthenticator::invalidate_user` / `WorkspacePool::invalidate_user`
    // are independently unit-tested. The integration-level test below
    // verifies that the DB state changes propagate through the set+get flow.

    #[tokio::test]
    async fn admin_set_then_get_shows_updated_grant() {
        let (db, _dir) = test_db().await;
        db.create_user(&make_user("alice")).await.unwrap();
        db.create_user(&make_user("shared")).await.unwrap();
        let state = build_state(Arc::clone(&db));
        let app = admin_router(state, admin_auth());

        // Set a read-only grant
        let req = Request::builder()
            .method(Method::PUT)
            .uri("/api/admin/users/alice/scope-grants/shared")
            .header("Authorization", "Bearer tok-admin")
            .header("Content-Type", "application/json")
            .body(Body::from(r#"{"writable": false}"#))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // Upsert to writable
        let req = Request::builder()
            .method(Method::PUT)
            .uri("/api/admin/users/alice/scope-grants/shared")
            .header("Authorization", "Bearer tok-admin")
            .header("Content-Type", "application/json")
            .body(Body::from(r#"{"writable": true}"#))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // GET should show the updated grant
        let req = Request::builder()
            .uri("/api/admin/users/alice/scope-grants")
            .header("Authorization", "Bearer tok-admin")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp).await;
        let grants = json["grants"].as_array().unwrap();
        assert_eq!(grants.len(), 1);
        assert_eq!(grants[0]["writable"], true);
    }

    #[tokio::test]
    async fn admin_revoke_then_get_shows_empty() {
        let (db, _dir) = test_db().await;
        db.create_user(&make_user("alice")).await.unwrap();
        db.create_user(&make_user("shared")).await.unwrap();
        let state = build_state(Arc::clone(&db));
        let app = admin_router(state, admin_auth());

        // Set a grant
        db.set_scope_grant("alice", "shared", false, Some("admin-user"), None)
            .await
            .unwrap();

        // Revoke via handler
        let req = Request::builder()
            .method(Method::DELETE)
            .uri("/api/admin/users/alice/scope-grants/shared")
            .header("Authorization", "Bearer tok-admin")
            .body(Body::empty())
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // GET should show empty grants
        let req = Request::builder()
            .uri("/api/admin/users/alice/scope-grants")
            .header("Authorization", "Bearer tok-admin")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp).await;
        let grants = json["grants"].as_array().unwrap();
        assert!(grants.is_empty());
    }

    // ── Manual test plan ──────────────────────────────────────────────
    //
    // Validate against a running IronClaw instance with curl:
    //
    // 1. Self-grant prevention (expect 400):
    //    curl -X PUT http://localhost:3003/api/admin/users/alice/scope-grants/alice \
    //      -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
    //      -d '{"writable": false}'
    //
    // 2. Scope validation (expect 404 for non-existent scope):
    //    curl -X PUT http://localhost:3003/api/admin/users/alice/scope-grants/nonexistent \
    //      -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
    //      -d '{"writable": false}'
    //
    // 3. Grant with expires_at:
    //    curl -X PUT http://localhost:3003/api/admin/users/alice/scope-grants/shared \
    //      -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
    //      -d '{"writable": false, "expires_at": "2026-12-31T23:59:59Z"}'
    //
    // 4. Writer can only revoke own grants:
    //    # As writer_a, grant bob access:
    //    curl -X PUT http://localhost:3003/api/scope-grants/alice/bob \
    //      -H "Authorization: Bearer $WRITER_A_TOKEN" -H "Content-Type: application/json" \
    //      -d '{"writable": false}'
    //    # As writer_b, try to revoke (expect 404):
    //    curl -X DELETE http://localhost:3003/api/scope-grants/alice/bob \
    //      -H "Authorization: Bearer $WRITER_B_TOKEN"
    //    # As writer_a, revoke (expect 200):
    //    curl -X DELETE http://localhost:3003/api/scope-grants/alice/bob \
    //      -H "Authorization: Bearer $WRITER_A_TOKEN"
    //
    // 5. Cache invalidation (grant, verify access, revoke, verify access removed):
    //    # Grant alice access to shared scope:
    //    curl -X PUT http://localhost:3003/api/admin/users/alice/scope-grants/shared \
    //      -H "Authorization: Bearer $ADMIN_TOKEN" -H "Content-Type: application/json" \
    //      -d '{"writable": false}'
    //    # Verify alice can read shared scope via memory API:
    //    curl http://localhost:3003/api/memory/tree \
    //      -H "Authorization: Bearer $ALICE_TOKEN"
    //    # Revoke the grant:
    //    curl -X DELETE http://localhost:3003/api/admin/users/alice/scope-grants/shared \
    //      -H "Authorization: Bearer $ADMIN_TOKEN"
    //    # Verify alice can no longer see shared scope:
    //    curl http://localhost:3003/api/memory/tree \
    //      -H "Authorization: Bearer $ALICE_TOKEN"
}
