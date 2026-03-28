//! Frontend extension API handlers.
//!
//! Provides endpoints for reading/writing layout configuration and
//! discovering/serving widget files from the workspace.

use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
    http::{StatusCode, header},
    response::IntoResponse,
};

use ironclaw_frontend::{LayoutConfig, WidgetManifest};

use crate::channels::web::auth::AuthenticatedUser;
use crate::channels::web::handlers::memory::resolve_workspace;
use crate::channels::web::server::GatewayState;

/// `GET /api/frontend/layout` — return the current layout configuration.
///
/// Reads `frontend/layout.json` from the workspace. Returns an empty
/// default config if the file doesn't exist.
pub async fn frontend_layout_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
) -> Result<Json<LayoutConfig>, (StatusCode, String)> {
    let workspace = resolve_workspace(&state, &user).await?;

    let layout = match workspace.read("frontend/layout.json").await {
        Ok(doc) => serde_json::from_str(&doc.content).unwrap_or_default(),
        Err(_) => LayoutConfig::default(),
    };

    Ok(Json(layout))
}

/// `PUT /api/frontend/layout` — update the layout configuration.
///
/// Writes the provided layout config to `frontend/layout.json` in workspace.
pub async fn frontend_layout_update_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Json(layout): Json<LayoutConfig>,
) -> Result<StatusCode, (StatusCode, String)> {
    let workspace = resolve_workspace(&state, &user).await?;

    let content = serde_json::to_string_pretty(&layout).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("Invalid layout config: {e}"),
        )
    })?;

    workspace
        .write("frontend/layout.json", &content)
        .await
        .map_err(|e| {
            tracing::error!("Failed to write layout config: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to write layout config".to_string(),
            )
        })?;

    Ok(StatusCode::OK)
}

/// `GET /api/frontend/widgets` — list all widget manifests.
///
/// Scans `frontend/widgets/` in workspace for directories containing
/// `manifest.json` and returns their parsed manifests.
pub async fn frontend_widgets_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
) -> Result<Json<Vec<WidgetManifest>>, (StatusCode, String)> {
    let workspace = resolve_workspace(&state, &user).await?;

    let entries = workspace.list("frontend/widgets/").await.unwrap_or_default();

    let mut manifests = Vec::new();
    for entry in entries {
        if !entry.is_directory {
            continue;
        }
        let manifest_path = format!("frontend/widgets/{}/manifest.json", entry.name());
        if let Ok(doc) = workspace.read(&manifest_path).await {
            if let Ok(manifest) = serde_json::from_str::<WidgetManifest>(&doc.content) {
                manifests.push(manifest);
            }
        }
    }

    Ok(Json(manifests))
}

/// `GET /api/frontend/widget/{id}/{*file}` — serve a widget file.
///
/// Serves JS/CSS files from `frontend/widgets/{id}/{file}` in workspace
/// with appropriate MIME types.
pub async fn frontend_widget_file_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path((id, file)): Path<(String, String)>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    // Reject path traversal
    if id.contains("..") || file.contains("..") {
        return Err((StatusCode::BAD_REQUEST, "Invalid path".to_string()));
    }

    let workspace = resolve_workspace(&state, &user).await?;
    let path = format!("frontend/widgets/{}/{}", id, file);

    let doc = workspace.read(&path).await.map_err(|_| {
        (
            StatusCode::NOT_FOUND,
            format!("Widget file not found: {path}"),
        )
    })?;

    // Determine MIME type from extension
    let content_type = if file.ends_with(".js") {
        "application/javascript"
    } else if file.ends_with(".css") {
        "text/css"
    } else if file.ends_with(".json") {
        "application/json"
    } else {
        "text/plain"
    };

    Ok((
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        doc.content,
    ))
}
