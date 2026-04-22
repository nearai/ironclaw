//! Admin tool policy handlers.
//!
//! Allows an admin to define which tools are disabled for all non-admin users
//! or for specific users. The policy is stored in the settings table under the
//! well-known `__admin__` scope.
//!
//! dispatch-exempt: These endpoints are not routed through `ToolDispatcher`
//! because they are admin-only infrastructure operations gated behind
//! `AdminUser` auth. Settings reads/writes go through `resolve_settings_store`
//! so the `CachedSettingsStore` is invalidated on writes (keeping the agent
//! loop's view of admin tool policies coherent).

use std::sync::Arc;

use axum::{Json, extract::State, http::StatusCode};

use crate::channels::web::auth::AdminUser;
use crate::channels::web::features::settings::resolve_settings_store;
use crate::channels::web::platform::state::GatewayState;
use crate::tools::permissions::{
    ADMIN_SETTINGS_USER_ID, ADMIN_TOOL_POLICY_KEY, AdminToolPolicy, parse_admin_tool_policy,
    validate_admin_tool_policy,
};

/// GET /api/admin/tool-policy — retrieve the current admin tool policy.
///
/// Only available in multi-tenant mode (returns 404 in single-user deployments).
pub async fn tool_policy_get_handler(
    State(state): State<Arc<GatewayState>>,
    AdminUser(_admin): AdminUser,
) -> Result<Json<AdminToolPolicy>, (StatusCode, String)> {
    if !state.multi_tenant_mode {
        return Err((
            StatusCode::NOT_FOUND,
            "Admin tool policy is only available in multi-tenant mode".to_string(),
        ));
    }

    let store = resolve_settings_store(&state)
        .map_err(|status| (status, "Settings store unavailable".to_string()))?;

    let policy = match store
        .get_setting(ADMIN_SETTINGS_USER_ID, ADMIN_TOOL_POLICY_KEY)
        .await
    {
        Ok(Some(value)) => parse_admin_tool_policy(value, "http_get").map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Stored admin tool policy is corrupt: {e}"),
            )
        })?,
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
/// This endpoint is a full replacement with last-write-wins semantics.
/// Each PUT overwrites the previously stored policy; there is no merge/patch.
///
/// Only available in multi-tenant mode (returns 404 in single-user deployments).
pub async fn tool_policy_put_handler(
    State(state): State<Arc<GatewayState>>,
    AdminUser(_admin): AdminUser,
    Json(policy): Json<AdminToolPolicy>,
) -> Result<Json<AdminToolPolicy>, (StatusCode, String)> {
    if !state.multi_tenant_mode {
        return Err((
            StatusCode::NOT_FOUND,
            "Admin tool policy is only available in multi-tenant mode".to_string(),
        ));
    }

    validate_admin_tool_policy(&policy).map_err(|error| (StatusCode::BAD_REQUEST, error))?;

    let store = resolve_settings_store(&state)
        .map_err(|status| (status, "Settings store unavailable".to_string()))?;

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
