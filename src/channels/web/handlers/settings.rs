//! Settings API handlers.

use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};

use crate::channels::web::server::GatewayState;
use crate::channels::web::types::*;

pub async fn settings_list_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<Json<SettingsListResponse>, StatusCode> {
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let rows = store.list_settings(&state.user_id).await.map_err(|e| {
        tracing::error!("Failed to list settings: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let settings = rows
        .into_iter()
        .map(|r| SettingResponse {
            key: r.key,
            value: r.value,
            updated_at: r.updated_at.to_rfc3339(),
        })
        .collect();

    Ok(Json(SettingsListResponse { settings }))
}

pub async fn settings_get_handler(
    State(state): State<Arc<GatewayState>>,
    Path(key): Path<String>,
) -> Result<Json<SettingResponse>, StatusCode> {
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let row = store
        .get_setting_full(&state.user_id, &key)
        .await
        .map_err(|e| {
            tracing::error!("Failed to get setting '{}': {}", key, e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::NOT_FOUND)?;

    Ok(Json(SettingResponse {
        key: row.key,
        value: row.value,
        updated_at: row.updated_at.to_rfc3339(),
    }))
}

pub async fn settings_set_handler(
    State(state): State<Arc<GatewayState>>,
    Path(key): Path<String>,
    Json(body): Json<SettingWriteRequest>,
) -> Result<StatusCode, StatusCode> {
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;

    // Guard: cannot remove a custom provider that is currently active.
    if key == "llm_custom_providers" {
        guard_active_provider_not_removed(store, &state.user_id, &body.value).await?;
        validate_custom_providers_adapters(&body.value)?;
    }

    store
        .set_setting(&state.user_id, &key, &body.value)
        .await
        .map_err(|e| {
            tracing::error!("Failed to set setting '{}': {}", key, e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(StatusCode::NO_CONTENT)
}

const VALID_ADAPTERS: &[&str] = &["open_ai_completions", "anthropic", "ollama"];

/// Returns `Err(422)` if any provider in the incoming list has an unrecognised adapter.
fn validate_custom_providers_adapters(value: &serde_json::Value) -> Result<(), StatusCode> {
    let providers = match value.as_array() {
        Some(arr) => arr,
        None => return Ok(()),
    };
    for p in providers {
        if let Some(adapter) = p.get("adapter").and_then(|v| v.as_str())
            && !VALID_ADAPTERS.contains(&adapter)
        {
            tracing::warn!(adapter = %adapter, "Rejected unknown LLM adapter");
            return Err(StatusCode::UNPROCESSABLE_ENTITY);
        }
    }
    Ok(())
}

/// Returns `Err(409)` if the active `llm_backend` is a custom provider that
/// would be removed by the incoming update to `llm_custom_providers`.
async fn guard_active_provider_not_removed(
    store: &Arc<dyn crate::db::Database>,
    user_id: &str,
    new_value: &serde_json::Value,
) -> Result<(), StatusCode> {
    // Get the currently active backend.
    let active_backend = match store.get_setting(user_id, "llm_backend").await {
        Ok(Some(v)) => match v.as_str() {
            Some(s) if !s.is_empty() => s.to_string(),
            _ => return Ok(()),
        },
        _ => return Ok(()),
    };

    // Parse the incoming provider list.
    let new_providers: Vec<serde_json::Value> = match new_value.as_array() {
        Some(arr) => arr.clone(),
        None => return Ok(()),
    };

    // Check whether the active backend exists in the OLD custom providers list.
    let old_providers_value = match store.get_setting(user_id, "llm_custom_providers").await {
        Ok(Some(v)) => v,
        _ => return Ok(()),
    };
    let old_providers: Vec<serde_json::Value> = match old_providers_value.as_array() {
        Some(arr) => arr.clone(),
        None => return Ok(()),
    };

    let active_was_custom = old_providers
        .iter()
        .any(|p| p.get("id").and_then(|v| v.as_str()) == Some(&active_backend));
    if !active_was_custom {
        return Ok(());
    }

    // Reject if the active provider is absent from the new list.
    let still_present = new_providers
        .iter()
        .any(|p| p.get("id").and_then(|v| v.as_str()) == Some(&active_backend));
    if !still_present {
        tracing::warn!(
            active_backend = %active_backend,
            "Rejected attempt to delete the active custom LLM provider"
        );
        return Err(StatusCode::CONFLICT);
    }

    Ok(())
}

pub async fn settings_delete_handler(
    State(state): State<Arc<GatewayState>>,
    Path(key): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;

    // Guard: deleting llm_custom_providers is equivalent to setting it to [].
    // Reject if the active backend is a custom provider that would be removed.
    if key == "llm_custom_providers" {
        guard_active_provider_not_removed(store, &state.user_id, &serde_json::Value::Array(vec![]))
            .await?;
    }

    store
        .delete_setting(&state.user_id, &key)
        .await
        .map_err(|e| {
            tracing::error!("Failed to delete setting '{}': {}", key, e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(StatusCode::NO_CONTENT)
}

pub async fn settings_export_handler(
    State(state): State<Arc<GatewayState>>,
) -> Result<Json<SettingsExportResponse>, StatusCode> {
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let settings = store.get_all_settings(&state.user_id).await.map_err(|e| {
        tracing::error!("Failed to export settings: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok(Json(SettingsExportResponse { settings }))
}

pub async fn settings_import_handler(
    State(state): State<Arc<GatewayState>>,
    Json(body): Json<SettingsImportRequest>,
) -> Result<StatusCode, StatusCode> {
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    store
        .set_all_settings(&state.user_id, &body.settings)
        .await
        .map_err(|e| {
            tracing::error!("Failed to import settings: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(StatusCode::NO_CONTENT)
}
