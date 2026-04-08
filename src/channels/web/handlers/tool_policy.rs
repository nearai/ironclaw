//! Admin tool policy handlers.
//!
//! Allows an admin to define which tools are disabled for all non-admin users
//! or for specific users. The policy is stored in the settings table under the
//! well-known `__admin__` scope.

use std::sync::Arc;

use axum::{Json, extract::State, http::StatusCode};

use crate::channels::web::auth::AdminUser;
use crate::channels::web::server::GatewayState;
use crate::tools::permissions::{ADMIN_SETTINGS_USER_ID, ADMIN_TOOL_POLICY_KEY, AdminToolPolicy};

/// GET /api/admin/tool-policy — retrieve the current admin tool policy.
///
/// Only available in multi-tenant mode (returns 404 in single-user deployments).
pub async fn tool_policy_get_handler(
    State(state): State<Arc<GatewayState>>,
    AdminUser(_admin): AdminUser,
) -> Result<Json<AdminToolPolicy>, (StatusCode, String)> {
    if state.workspace_pool.is_none() {
        return Err((
            StatusCode::NOT_FOUND,
            "Admin tool policy is only available in multi-tenant mode".to_string(),
        ));
    }

    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    let policy = match store
        .get_setting(ADMIN_SETTINGS_USER_ID, ADMIN_TOOL_POLICY_KEY)
        .await
    {
        Ok(Some(value)) => serde_json::from_value(value).unwrap_or_default(),
        Ok(None) => AdminToolPolicy::default(),
        Err(e) => {
            return Err((StatusCode::INTERNAL_SERVER_ERROR, e.to_string()));
        }
    };

    Ok(Json(policy))
}

/// PUT /api/admin/tool-policy — replace the admin tool policy.
///
/// Body must be a JSON `AdminToolPolicy`. Tool names and user IDs are
/// validated for basic sanity (non-empty, reasonable length).
///
/// Only available in multi-tenant mode (returns 404 in single-user deployments).
pub async fn tool_policy_put_handler(
    State(state): State<Arc<GatewayState>>,
    AdminUser(_admin): AdminUser,
    Json(policy): Json<AdminToolPolicy>,
) -> Result<Json<AdminToolPolicy>, (StatusCode, String)> {
    if state.workspace_pool.is_none() {
        return Err((
            StatusCode::NOT_FOUND,
            "Admin tool policy is only available in multi-tenant mode".to_string(),
        ));
    }

    // Validate tool names
    for name in &policy.disabled_tools {
        if name.is_empty() || name.len() > 128 {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("Invalid tool name: '{name}'"),
            ));
        }
    }
    for (user_id, tools) in &policy.user_disabled_tools {
        if user_id.is_empty() || user_id.len() > 256 {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("Invalid user_id: '{user_id}'"),
            ));
        }
        for name in tools {
            if name.is_empty() || name.len() > 128 {
                return Err((
                    StatusCode::BAD_REQUEST,
                    format!("Invalid tool name for user '{user_id}': '{name}'"),
                ));
            }
        }
    }

    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;

    let value = serde_json::to_value(&policy).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to serialize policy: {e}"),
        )
    })?;

    store
        .set_setting(ADMIN_SETTINGS_USER_ID, ADMIN_TOOL_POLICY_KEY, &value)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(policy))
}
