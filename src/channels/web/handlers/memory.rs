//! Memory/workspace API handlers.

use std::sync::Arc;

use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
};
use serde::Deserialize;

use crate::channels::web::auth::{AuthenticatedUser, UserIdentity};
use crate::channels::web::platform::state::GatewayState;
use crate::channels::web::types::*;
use crate::workspace::Workspace;

/// Resolve the workspace for the authenticated user.
///
/// Authenticated memory APIs should prefer the per-user workspace pool whenever
/// it is available so user-scoped reads and writes stay isolated even if the
/// deployment is otherwise using single-user bootstrap/static routes.
pub(crate) async fn resolve_workspace(
    state: &GatewayState,
    user: &UserIdentity,
) -> Result<Arc<Workspace>, (StatusCode, String)> {
    if let Some(ref pool) = state.workspace_pool {
        return Ok(pool.get_or_create(user).await);
    }
    state.workspace.as_ref().cloned().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Workspace not available".to_string(),
    ))
}

#[derive(Deserialize)]
pub struct TreeQuery {
    #[allow(dead_code)]
    pub depth: Option<usize>,
}

pub async fn memory_tree_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Query(_query): Query<TreeQuery>,
) -> Result<Json<MemoryTreeResponse>, (StatusCode, String)> {
    let workspace = resolve_workspace(&state, &user).await?;

    // Build tree from list_all (flat list of all paths)
    let all_paths = workspace
        .list_all()
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    // Collect unique directories and files
    let mut entries: Vec<TreeEntry> = Vec::new();
    let mut seen_dirs: std::collections::HashSet<String> = std::collections::HashSet::new();

    for path in &all_paths {
        // Add parent directories
        let parts: Vec<&str> = path.split('/').collect();
        for i in 0..parts.len().saturating_sub(1) {
            let dir_path = parts[..=i].join("/");
            if seen_dirs.insert(dir_path.clone()) {
                entries.push(TreeEntry {
                    path: dir_path,
                    is_dir: true,
                });
            }
        }
        // Add the file itself
        entries.push(TreeEntry {
            path: path.clone(),
            is_dir: false,
        });
    }

    entries.sort_by(|a, b| a.path.cmp(&b.path));

    Ok(Json(MemoryTreeResponse { entries }))
}

#[derive(Deserialize)]
pub struct ListQuery {
    pub path: Option<String>,
}

pub async fn memory_list_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Query(query): Query<ListQuery>,
) -> Result<Json<MemoryListResponse>, (StatusCode, String)> {
    let workspace = resolve_workspace(&state, &user).await?;

    let path = query.path.as_deref().unwrap_or("");
    let entries = workspace
        .list(path)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let list_entries: Vec<ListEntry> = entries
        .iter()
        .map(|e| ListEntry {
            name: e.path.rsplit('/').next().unwrap_or(&e.path).to_string(),
            path: e.path.clone(),
            is_dir: e.is_directory,
            updated_at: e.updated_at.map(|dt| dt.to_rfc3339()),
        })
        .collect();

    Ok(Json(MemoryListResponse {
        path: path.to_string(),
        entries: list_entries,
    }))
}

#[derive(Deserialize)]
pub struct ReadQuery {
    pub path: String,
}

pub async fn memory_read_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Query(query): Query<ReadQuery>,
) -> Result<Json<MemoryReadResponse>, (StatusCode, String)> {
    let workspace = resolve_workspace(&state, &user).await?;

    let doc = workspace
        .read(&query.path)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;

    Ok(Json(MemoryReadResponse {
        path: query.path,
        content: doc.content,
        updated_at: Some(doc.updated_at.to_rfc3339()),
    }))
}

pub async fn memory_write_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Json(req): Json<MemoryWriteRequest>,
) -> Result<Json<MemoryWriteResponse>, (StatusCode, String)> {
    let workspace = resolve_workspace(&state, &user).await?;

    // Route through layer-aware methods when a layer is specified.
    //
    // Note: unlike MemoryWriteTool, this endpoint does NOT block writes to
    // identity files (IDENTITY.md, SOUL.md, etc.). The HTTP API is an
    // authenticated admin interface; the supervisor uses it to seed identity
    // files at startup. Identity-file protection is enforced at the tool
    // layer (LLM-facing) where the write originates from an untrusted agent.
    if let Some(ref layer_name) = req.layer {
        let result = if req.append {
            workspace
                .append_to_layer(layer_name, &req.path, &req.content, req.force)
                .await
        } else {
            workspace
                .write_to_layer(layer_name, &req.path, &req.content, req.force)
                .await
        }
        .map_err(|e| {
            use crate::error::WorkspaceError;
            let status = match &e {
                WorkspaceError::LayerNotFound { .. } => StatusCode::BAD_REQUEST,
                WorkspaceError::LayerReadOnly { .. } => StatusCode::FORBIDDEN,
                WorkspaceError::PrivacyRedirectFailed => StatusCode::UNPROCESSABLE_ENTITY,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            };
            (status, e.to_string())
        })?;
        return Ok(Json(MemoryWriteResponse {
            path: req.path,
            status: "written",
            redirected: Some(result.redirected),
            actual_layer: Some(result.actual_layer),
        }));
    }

    // Non-layer path: honor the append field
    if req.append {
        workspace
            .append(&req.path, &req.content)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    } else {
        workspace
            .write(&req.path, &req.content)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }

    Ok(Json(MemoryWriteResponse {
        path: req.path,
        status: "written",
        redirected: None,
        actual_layer: None,
    }))
}

pub async fn memory_search_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Json(req): Json<MemorySearchRequest>,
) -> Result<Json<MemorySearchResponse>, (StatusCode, String)> {
    let workspace = resolve_workspace(&state, &user).await?;

    let limit = req.limit.unwrap_or(10);
    let results = workspace
        .search(&req.query, limit)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let hits: Vec<SearchHit> = results
        .iter()
        .map(|r| SearchHit {
            path: r.document_id.to_string(),
            content: r.content.clone(),
            score: r.score as f64,
        })
        .collect();

    Ok(Json(MemorySearchResponse { results: hits }))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use axum::Router;
    use axum::body::Body;
    use axum::http::Request;
    use axum::middleware;
    use axum::routing::get;
    use tower::ServiceExt;

    use crate::channels::web::auth::{MultiAuthState, UserIdentity, auth_middleware};
    use crate::channels::web::platform::state::{
        ActiveConfigSnapshot, GatewayState, PerUserRateLimiter, RateLimiter, WorkspacePool,
    };
    use crate::channels::web::sse::SseManager;
    use crate::config::{WorkspaceConfig, WorkspaceSearchConfig};
    use crate::db::Database;
    use crate::workspace::{EmbeddingCacheConfig, Workspace};

    use super::memory_read_handler;

    async fn test_db() -> (Arc<dyn Database>, tempfile::TempDir) {
        use crate::db::libsql::LibSqlBackend;

        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("memory-handler-test.db");
        let backend = LibSqlBackend::new_local(&path)
            .await
            .expect("create libsql backend");
        backend.run_migrations().await.expect("run migrations");
        (Arc::new(backend) as Arc<dyn Database>, dir)
    }

    fn test_state(
        db: Arc<dyn Database>,
        workspace: Arc<Workspace>,
        pool: Arc<WorkspacePool>,
    ) -> Arc<GatewayState> {
        Arc::new(GatewayState {
            msg_tx: tokio::sync::RwLock::new(None),
            sse: Arc::new(SseManager::new()),
            workspace: Some(workspace),
            workspace_pool: Some(pool),
            multi_tenant_mode: false,
            session_manager: None,
            log_broadcaster: None,
            log_level_handle: None,
            extension_manager: None,
            tool_registry: None,
            store: Some(db),
            settings_cache: None,
            job_manager: None,
            prompt_queue: None,
            scheduler: None,
            owner_id: "owner".to_string(),
            shutdown_tx: tokio::sync::RwLock::new(None),
            ws_tracker: None,
            llm_provider: None,
            llm_reload: None,
            llm_session_manager: None,
            config_toml_path: None,
            skill_registry: None,
            skill_catalog: None,
            auth_manager: None,
            chat_rate_limiter: PerUserRateLimiter::new(30, 60),
            oauth_rate_limiter: PerUserRateLimiter::new(20, 60),
            webhook_rate_limiter: RateLimiter::new(10, 60),
            registry_entries: Vec::new(),
            cost_guard: None,
            routine_engine: Arc::new(tokio::sync::RwLock::new(None)),
            startup_time: std::time::Instant::now(),
            active_config: Arc::new(tokio::sync::RwLock::new(ActiveConfigSnapshot::default())),
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
            frontend_html_cache: Arc::new(tokio::sync::RwLock::new(None)),
            tool_dispatcher: None,
        })
    }

    fn read_router(state: Arc<GatewayState>) -> Router {
        let mut tokens = HashMap::new();
        tokens.insert(
            "tok-bob".to_string(),
            UserIdentity {
                user_id: "bob".to_string(),
                role: "admin".to_string(),
                workspace_read_scopes: Vec::new(),
            },
        );
        let auth = MultiAuthState::multi(tokens);

        Router::new()
            .route("/api/memory/read", get(memory_read_handler))
            .layer(middleware::from_fn_with_state(auth.into(), auth_middleware))
            .with_state(state)
    }

    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn memory_read_prefers_workspace_pool_even_when_multi_tenant_mode_is_false() {
        let (db, _dir) = test_db().await;
        let owner_workspace = Arc::new(Workspace::new_with_db("owner", Arc::clone(&db)));
        owner_workspace
            .write("note.md", "owner note")
            .await
            .expect("write owner note");

        let pool = Arc::new(WorkspacePool::new(
            Arc::clone(&db),
            None,
            EmbeddingCacheConfig::default(),
            WorkspaceSearchConfig::default(),
            WorkspaceConfig::default(),
        ));
        let bob = UserIdentity {
            user_id: "bob".to_string(),
            role: "admin".to_string(),
            workspace_read_scopes: Vec::new(),
        };
        let bob_workspace = pool.get_or_create(&bob).await;
        bob_workspace
            .write("note.md", "bob note")
            .await
            .expect("write bob note");

        let app = read_router(test_state(db, owner_workspace, pool));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/memory/read?path=note.md")
                    .header("Authorization", "Bearer tok-bob")
                    .body(Body::empty())
                    .expect("build request"),
            )
            .await
            .expect("dispatch request");

        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read body");
        let payload: serde_json::Value =
            serde_json::from_slice(&body).expect("parse memory read response");
        assert_eq!(payload["content"], "bob note");
    }
}
