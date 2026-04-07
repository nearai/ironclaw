//! Admin system prompt management handlers.
//!
//! These endpoints allow admins to set a shared system prompt (`SYSTEM.md`)
//! that is injected into every user's system prompt in multi-tenant mode.

use std::sync::Arc;

use axum::{Json, extract::State, http::StatusCode};

use crate::channels::web::auth::AdminUser;
use crate::channels::web::server::GatewayState;
use crate::channels::web::types::{SystemPromptRequest, SystemPromptResponse};
use crate::workspace::{ADMIN_SCOPE, Workspace, paths};

/// `GET /api/admin/system-prompt` — read the admin system prompt.
pub async fn get_handler(
    State(state): State<Arc<GatewayState>>,
    AdminUser(_admin): AdminUser,
) -> Result<Json<SystemPromptResponse>, (StatusCode, String)> {
    // Gate behind multi-tenant mode.
    if state.workspace_pool.is_none() {
        return Err((
            StatusCode::NOT_FOUND,
            "System prompt management requires multi-tenant mode".to_string(),
        ));
    }

    let db = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    let ws = Workspace::new_with_db(ADMIN_SCOPE, Arc::clone(db));

    match ws.read(paths::SYSTEM).await {
        Ok(doc) => Ok(Json(SystemPromptResponse {
            content: doc.content,
            updated_at: Some(doc.updated_at.to_rfc3339()),
        })),
        Err(crate::error::WorkspaceError::DocumentNotFound { .. }) => {
            Ok(Json(SystemPromptResponse {
                content: String::new(),
                updated_at: None,
            }))
        }
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

/// `PUT /api/admin/system-prompt` — set the admin system prompt.
pub async fn put_handler(
    State(state): State<Arc<GatewayState>>,
    AdminUser(_admin): AdminUser,
    Json(req): Json<SystemPromptRequest>,
) -> Result<Json<SystemPromptResponse>, (StatusCode, String)> {
    // Gate behind multi-tenant mode.
    if state.workspace_pool.is_none() {
        return Err((
            StatusCode::NOT_FOUND,
            "System prompt management requires multi-tenant mode".to_string(),
        ));
    }

    let db = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    let ws = Workspace::new_with_db(ADMIN_SCOPE, Arc::clone(db));

    let doc = ws.write(paths::SYSTEM, &req.content).await.map_err(|e| {
        let status = if matches!(e, crate::error::WorkspaceError::InjectionRejected { .. }) {
            StatusCode::BAD_REQUEST
        } else {
            StatusCode::INTERNAL_SERVER_ERROR
        };
        (status, e.to_string())
    })?;

    Ok(Json(SystemPromptResponse {
        content: doc.content,
        updated_at: Some(doc.updated_at.to_rfc3339()),
    }))
}
