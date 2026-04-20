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
/// In multi-tenant mode, route through `workspace_pool` so each authenticated
/// user gets their own workspace. In single-user mode, always use the shared
/// owner workspace even if a pool is present for deeper runtime/tool plumbing;
/// this keeps authenticated gateway reads/writes aligned with the unauthenticated
/// bootstrap routes (`/`, `/style.css`), which also read from `state.workspace`.
pub(crate) async fn resolve_workspace(
    state: &GatewayState,
    user: &UserIdentity,
) -> Result<Arc<Workspace>, (StatusCode, String)> {
    if state.multi_tenant_mode
        && let Some(ref pool) = state.workspace_pool
    {
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
