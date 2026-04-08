//! Settings API handlers.

use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use secrecy::SecretString;

use crate::channels::web::auth::AuthenticatedUser;
use crate::channels::web::server::GatewayState;
use crate::channels::web::types::*;
use crate::secrets::{CreateSecretParams, SecretsStore};

/// Sentinel value the frontend sends to mean "key is unchanged, don't touch it".
const API_KEY_UNCHANGED: &str = "••••••••";

pub async fn settings_list_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
) -> Result<Json<SettingsListResponse>, StatusCode> {
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let rows = store.list_settings(&user.user_id).await.map_err(|e| {
        tracing::error!("Failed to list settings: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    // Build a map of sensitive keys so we can annotate and mask them.
    let sensitive_keys = ["llm_builtin_overrides", "llm_custom_providers"];
    let mut sensitive_map: std::collections::HashMap<String, serde_json::Value> = rows
        .iter()
        .filter(|r| sensitive_keys.contains(&r.key.as_str()))
        .map(|r| (r.key.clone(), r.value.clone()))
        .collect();
    if !sensitive_map.is_empty() {
        annotate_secret_key_presence(&state, &user.user_id, &mut sensitive_map).await;
        mask_settings_api_keys(&mut sensitive_map);
    }

    let settings = rows
        .into_iter()
        .map(|r| {
            let value = if sensitive_keys.contains(&r.key.as_str()) {
                sensitive_map
                    .get(&r.key)
                    .cloned()
                    .unwrap_or(r.value.clone())
            } else {
                r.value
            };
            SettingResponse {
                key: r.key,
                value,
                updated_at: r.updated_at.to_rfc3339(),
            }
        })
        .collect();

    Ok(Json(SettingsListResponse { settings }))
}

pub async fn settings_get_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(key): Path<String>,
) -> Result<Json<SettingResponse>, StatusCode> {
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let row = store
        .get_setting_full(&user.user_id, &key)
        .await
        .map_err(|e| {
            tracing::error!("Failed to get setting '{}': {}", key, e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::NOT_FOUND)?;

    // Mask any plaintext API keys that may exist from legacy data.
    let value = if matches!(
        key.as_str(),
        "llm_builtin_overrides" | "llm_custom_providers"
    ) {
        let mut map = std::collections::HashMap::from([(key.clone(), row.value.clone())]);
        annotate_secret_key_presence(&state, &user.user_id, &mut map).await;
        mask_settings_api_keys(&mut map);
        map.remove(&key).unwrap_or(row.value)
    } else {
        row.value
    };

    Ok(Json(SettingResponse {
        key: row.key,
        value,
        updated_at: row.updated_at.to_rfc3339(),
    }))
}

pub async fn settings_set_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(key): Path<String>,
    Json(body): Json<SettingWriteRequest>,
) -> Result<StatusCode, StatusCode> {
    ensure_setting_write_allowed(&user, &key)?;

    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;

    // Guard: cannot remove a custom provider that is currently active.
    if key == "llm_custom_providers" {
        guard_active_provider_not_removed(store, &user.user_id, &body.value).await?;
        validate_custom_providers(&body.value)?;
    }

    // Extract API keys from LLM settings and vault them in the secrets store.
    // The sanitized value has api_key fields removed (stored encrypted instead).
    let sanitized_value = match key.as_str() {
        "llm_builtin_overrides" => {
            extract_builtin_override_keys(&state, &user.user_id, &body.value).await?
        }
        "llm_custom_providers" => {
            extract_custom_provider_keys(&state, &user.user_id, &body.value).await?
        }
        _ => body.value.clone(),
    };

    store
        .set_setting(&user.user_id, &key, &sanitized_value)
        .await
        .map_err(|e| {
            tracing::error!("Failed to set setting '{}': {}", key, e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(StatusCode::NO_CONTENT)
}

const VALID_ADAPTERS: &[&str] = &["open_ai_completions", "anthropic", "ollama"];

/// Valid provider ID: lowercase alphanumeric, hyphens, and underscores, 1-64 chars.
fn is_valid_provider_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

/// Returns `Err(422)` if any provider has an invalid ID or unrecognised adapter.
fn validate_custom_providers(value: &serde_json::Value) -> Result<(), StatusCode> {
    let providers = match value.as_array() {
        Some(arr) => arr,
        None => return Ok(()),
    };
    for p in providers {
        let id = p.get("id").and_then(|v| v.as_str()).unwrap_or("");
        if !is_valid_provider_id(id) {
            tracing::warn!(
                id = %id,
                "Rejected custom provider with invalid ID (must be lowercase alphanumeric/hyphens/underscores, 1-64 chars)"
            );
            return Err(StatusCode::UNPROCESSABLE_ENTITY);
        }
        let adapter = p.get("adapter").and_then(|v| v.as_str()).unwrap_or("");
        if adapter.is_empty() {
            tracing::warn!(id = %id, "Rejected custom provider with missing adapter field");
            return Err(StatusCode::UNPROCESSABLE_ENTITY);
        }
        if !VALID_ADAPTERS.contains(&adapter) {
            tracing::warn!(id = %id, adapter = %adapter, "Rejected unknown LLM adapter");
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
    let new_providers = match new_value.as_array() {
        Some(arr) => arr,
        None => return Ok(()),
    };

    // Check whether the active backend exists in the OLD custom providers list.
    let old_providers_value = match store.get_setting(user_id, "llm_custom_providers").await {
        Ok(Some(v)) => v,
        _ => return Ok(()),
    };
    let old_providers = match old_providers_value.as_array() {
        Some(arr) => arr,
        None => return Ok(()),
    };

    let active_was_custom = old_providers
        .iter()
        .any(|p| p.get("id").and_then(|v| v.as_str()) == Some(active_backend.as_str()));
    if !active_was_custom {
        return Ok(());
    }

    // Reject if the active provider is absent from the new list.
    let still_present = new_providers
        .iter()
        .any(|p| p.get("id").and_then(|v| v.as_str()) == Some(active_backend.as_str()));
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
    AuthenticatedUser(user): AuthenticatedUser,
    Path(key): Path<String>,
) -> Result<StatusCode, StatusCode> {
    ensure_setting_write_allowed(&user, &key)?;

    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;

    // Guard: deleting llm_custom_providers is equivalent to setting it to [].
    // Reject if the active backend is a custom provider that would be removed.
    if key == "llm_custom_providers" {
        guard_active_provider_not_removed(store, &user.user_id, &serde_json::Value::Array(vec![]))
            .await?;
    }

    store
        .delete_setting(&user.user_id, &key)
        .await
        .map_err(|e| {
            tracing::error!("Failed to delete setting '{}': {}", key, e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(StatusCode::NO_CONTENT)
}

pub async fn settings_export_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
) -> Result<Json<SettingsExportResponse>, StatusCode> {
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let mut settings = store.get_all_settings(&user.user_id).await.map_err(|e| {
        tracing::error!("Failed to export settings: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    // Indicate key presence from secrets store without exposing values.
    annotate_secret_key_presence(&state, &user.user_id, &mut settings).await;

    mask_settings_api_keys(&mut settings);

    Ok(Json(SettingsExportResponse { settings }))
}

pub async fn settings_import_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Json(body): Json<SettingsImportRequest>,
) -> Result<StatusCode, StatusCode> {
    ensure_settings_import_allowed(&user, &body.settings)?;

    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;

    // Vault any API keys present in the imported settings, same as the
    // individual SET handler does, so plaintext keys never reach the DB.
    let mut sanitized = body.settings.clone();
    if let Some(v) = sanitized.get("llm_builtin_overrides").cloned() {
        let clean = extract_builtin_override_keys(&state, &user.user_id, &v).await?;
        sanitized.insert("llm_builtin_overrides".to_string(), clean);
    }
    if let Some(v) = sanitized.get("llm_custom_providers").cloned() {
        let clean = extract_custom_provider_keys(&state, &user.user_id, &v).await?;
        sanitized.insert("llm_custom_providers".to_string(), clean);
    }

    store
        .set_all_settings(&user.user_id, &sanitized)
        .await
        .map_err(|e| {
            tracing::error!("Failed to import settings: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(StatusCode::NO_CONTENT)
}

fn is_admin_only_setting_key(key: &str) -> bool {
    // Single source of truth lives in `crate::config::helpers` so the
    // write-side gate here cannot drift from the read-side strip filter.
    crate::config::helpers::ADMIN_ONLY_LLM_SETTING_KEYS.contains(&key)
}

fn ensure_setting_write_allowed(
    user: &crate::channels::web::auth::UserIdentity,
    key: &str,
) -> Result<(), StatusCode> {
    if is_admin_only_setting_key(key) && user.role != "admin" {
        tracing::warn!(
            user_id = %user.user_id,
            role = %user.role,
            key = %key,
            "Rejected non-admin write to admin-only setting"
        );
        return Err(StatusCode::FORBIDDEN);
    }

    Ok(())
}

fn ensure_settings_import_allowed(
    user: &crate::channels::web::auth::UserIdentity,
    settings: &std::collections::HashMap<String, serde_json::Value>,
) -> Result<(), StatusCode> {
    if user.role == "admin" {
        return Ok(());
    }

    if let Some(key) = settings.keys().find(|key| is_admin_only_setting_key(key)) {
        tracing::warn!(
            user_id = %user.user_id,
            role = %user.role,
            key = %key,
            "Rejected non-admin import containing admin-only setting"
        );
        return Err(StatusCode::FORBIDDEN);
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// LLM API key vaulting helpers
// ---------------------------------------------------------------------------

use crate::settings::{builtin_secret_name, custom_secret_name};

/// Returns true if the `api_key` value is a real key (not sentinel/empty).
fn is_real_api_key(key: &str) -> bool {
    !key.is_empty() && key != API_KEY_UNCHANGED
}

/// Require the secrets store when real API keys are present.
/// Returns `Ok(None)` when no secrets store and no real keys (passthrough).
fn require_secrets_store(
    state: &GatewayState,
    has_real_keys: bool,
) -> Result<Option<&Arc<dyn SecretsStore + Send + Sync>>, StatusCode> {
    match state.secrets_store.as_ref() {
        Some(s) => Ok(Some(s)),
        None if has_real_keys => {
            tracing::error!("Cannot store API keys: secrets store is not available");
            Err(StatusCode::SERVICE_UNAVAILABLE)
        }
        None => Ok(None),
    }
}

/// Extract API keys from builtin overrides, store in secrets, return sanitized JSON.
async fn extract_builtin_override_keys(
    state: &GatewayState,
    user_id: &str,
    value: &serde_json::Value,
) -> Result<serde_json::Value, StatusCode> {
    let obj = match value.as_object() {
        Some(o) => o,
        None => return Ok(value.clone()),
    };

    let has_real_keys = obj.values().any(|v| {
        v.get("api_key")
            .and_then(|k| k.as_str())
            .is_some_and(is_real_api_key)
    });
    let secrets = match require_secrets_store(state, has_real_keys)? {
        Some(s) => s,
        None => return Ok(value.clone()),
    };

    let mut sanitized = obj.clone();

    for (provider_id, override_val) in obj {
        if let Some(api_key) = override_val.get("api_key").and_then(|v| v.as_str()) {
            if !is_real_api_key(api_key) {
                // Unchanged or empty — remove from settings, keep existing secret.
                if let Some(o) = sanitized
                    .get_mut(provider_id)
                    .and_then(|v| v.as_object_mut())
                {
                    o.remove("api_key");
                }
                continue;
            }
            vault_secret(
                secrets.as_ref(),
                user_id,
                &builtin_secret_name(provider_id),
                api_key,
                provider_id,
            )
            .await?;
            if let Some(o) = sanitized
                .get_mut(provider_id)
                .and_then(|v| v.as_object_mut())
            {
                o.remove("api_key");
            }
        }
    }

    Ok(serde_json::Value::Object(sanitized))
}

/// Extract API keys from custom providers, store in secrets, return sanitized JSON.
async fn extract_custom_provider_keys(
    state: &GatewayState,
    user_id: &str,
    value: &serde_json::Value,
) -> Result<serde_json::Value, StatusCode> {
    let arr = match value.as_array() {
        Some(a) => a,
        None => return Ok(value.clone()),
    };

    let has_real_keys = arr.iter().any(|v| {
        v.get("api_key")
            .and_then(|k| k.as_str())
            .is_some_and(is_real_api_key)
    });
    let secrets = match require_secrets_store(state, has_real_keys)? {
        Some(s) => s,
        None => return Ok(value.clone()),
    };

    let mut sanitized = arr.clone();

    for (idx, provider_val) in arr.iter().enumerate() {
        let provider_id = provider_val
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if provider_id.is_empty() {
            continue;
        }

        if let Some(api_key) = provider_val.get("api_key").and_then(|v| v.as_str()) {
            if !is_real_api_key(api_key) {
                if let Some(o) = sanitized[idx].as_object_mut() {
                    o.remove("api_key");
                }
                continue;
            }
            vault_secret(
                secrets.as_ref(),
                user_id,
                &custom_secret_name(provider_id),
                api_key,
                provider_id,
            )
            .await?;
            if let Some(o) = sanitized[idx].as_object_mut() {
                o.remove("api_key");
            }
        }
    }

    Ok(serde_json::Value::Array(sanitized))
}

/// Encrypt and store an API key in the secrets store.
async fn vault_secret(
    secrets: &(dyn SecretsStore + Send + Sync),
    user_id: &str,
    secret_name: &str,
    api_key: &str,
    provider_id: &str,
) -> Result<(), StatusCode> {
    secrets
        .create(
            user_id,
            CreateSecretParams {
                name: secret_name.to_string(),
                value: SecretString::from(api_key.to_string()),
                provider: Some(provider_id.to_string()),
                expires_at: None,
            },
        )
        .await
        .map_err(|e| {
            tracing::error!(
                "Failed to store secret '{}' for provider '{}': {}",
                secret_name,
                provider_id,
                e
            );
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    Ok(())
}

/// Mask plaintext API keys in settings values before returning to the frontend.
///
/// Any `api_key` field still present in the settings JSON (legacy plaintext)
/// is replaced with the sentinel so the frontend shows "key configured".
fn mask_settings_api_keys(settings: &mut std::collections::HashMap<String, serde_json::Value>) {
    if let Some(obj) = settings
        .get_mut("llm_builtin_overrides")
        .and_then(|v| v.as_object_mut())
    {
        for override_val in obj.values_mut() {
            if let Some(o) = override_val.as_object_mut()
                && o.contains_key("api_key")
            {
                o.insert(
                    "api_key".to_string(),
                    serde_json::Value::String(API_KEY_UNCHANGED.to_string()),
                );
            }
        }
    }

    if let Some(arr) = settings
        .get_mut("llm_custom_providers")
        .and_then(|v| v.as_array_mut())
    {
        for provider_val in arr.iter_mut() {
            if let Some(o) = provider_val.as_object_mut()
                && o.contains_key("api_key")
            {
                o.insert(
                    "api_key".to_string(),
                    serde_json::Value::String(API_KEY_UNCHANGED.to_string()),
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tool Permissions API
// ---------------------------------------------------------------------------

/// `GET /api/settings/tools` — list all tools with current permission state.
pub async fn settings_tools_list_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
) -> Result<Json<ToolPermissionsResponse>, StatusCode> {
    use crate::tools::ApprovalRequirement;
    use crate::tools::permissions::{TOOL_RISK_DEFAULTS, effective_permission};

    let registry = state
        .tool_registry
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;

    // Load current user tool permission overrides from the DB.
    let store = state
        .store
        .as_ref()
        .ok_or(StatusCode::SERVICE_UNAVAILABLE)?;
    let db_map = store.get_all_settings(&user.user_id).await.map_err(|e| {
        tracing::error!("Failed to load settings for tool permissions: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    let user_overrides = crate::settings::Settings::from_db_map(&db_map).tool_permissions;

    let tools = registry.all().await;
    let mut entries: Vec<ToolPermissionEntry> = tools
        .iter()
        .map(|tool| {
            let name = tool.name().to_string();
            let description = tool.description().to_string();

            let current = effective_permission(&name, &user_overrides);
            let default = TOOL_RISK_DEFAULTS
                .get(name.as_str())
                .copied()
                .unwrap_or(crate::tools::permissions::PermissionState::AskEachTime);

            let locked = matches!(
                tool.requires_approval(&serde_json::Value::Null),
                ApprovalRequirement::Always
            );
            let locked_reason = if locked {
                Some("Always requires approval due to risk level".to_string())
            } else {
                None
            };

            ToolPermissionEntry {
                name,
                description,
                current_state: permission_state_to_str(current).to_string(),
                default_state: permission_state_to_str(default).to_string(),
                locked,
                locked_reason,
            }
        })
        .collect();

    entries.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(Json(ToolPermissionsResponse { tools: entries }))
}

/// `PUT /api/settings/tools/:name` — update permission state for a single tool.
pub async fn settings_tools_set_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(name): Path<String>,
    Json(body): Json<UpdateToolPermissionRequest>,
) -> Result<Json<ToolPermissionEntry>, (StatusCode, axum::Json<serde_json::Value>)> {
    use crate::tools::ApprovalRequirement;
    use crate::tools::permissions::{PermissionState, TOOL_RISK_DEFAULTS};

    let registry = state.tool_registry.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        axum::Json(serde_json::json!({"error": "Tool registry unavailable"})),
    ))?;

    // Validate tool exists.
    let tool = registry.get(&name).await.ok_or((
        StatusCode::NOT_FOUND,
        axum::Json(serde_json::json!({"error": format!("Tool '{}' not found", name)})),
    ))?;

    // Reject if tool is locked (ApprovalRequirement::Always).
    if matches!(
        tool.requires_approval(&serde_json::Value::Null),
        ApprovalRequirement::Always
    ) {
        return Err((
            StatusCode::BAD_REQUEST,
            axum::Json(serde_json::json!({
                "error": format!("Tool '{}' is locked and cannot have its permission changed", name)
            })),
        ));
    }

    // Parse the requested state.
    let new_state = str_to_permission_state(&body.state).ok_or((
        StatusCode::UNPROCESSABLE_ENTITY,
        axum::Json(
            serde_json::json!({"error": format!("Invalid permission state: '{}'", body.state)}),
        ),
    ))?;

    // Persist the permission override to the DB, scoped to the authenticated user.
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        axum::Json(serde_json::json!({"error": "Settings store unavailable"})),
    ))?;

    let json_value = serde_json::to_value(new_state).map_err(|e| {
        tracing::error!("Failed to serialize permission state: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(serde_json::json!({"error": "Internal error"})),
        )
    })?;

    store
        .set_setting(
            &user.user_id,
            &format!("tool_permissions.{}", name),
            &json_value,
        )
        .await
        .map_err(|e| {
            tracing::error!("Failed to set tool permission '{}': {}", name, e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                axum::Json(serde_json::json!({"error": "Failed to persist permission"})),
            )
        })?;

    // Use new_state directly — we just wrote it, no need for an extra DB round-trip.
    let default = TOOL_RISK_DEFAULTS
        .get(name.as_str())
        .copied()
        .unwrap_or(PermissionState::AskEachTime);

    Ok(Json(ToolPermissionEntry {
        description: tool.description().to_string(),
        name,
        current_state: permission_state_to_str(new_state).to_string(),
        default_state: permission_state_to_str(default).to_string(),
        locked: false,
        locked_reason: None,
    }))
}

fn permission_state_to_str(state: crate::tools::permissions::PermissionState) -> &'static str {
    use crate::tools::permissions::PermissionState;
    match state {
        PermissionState::AlwaysAllow => "always_allow",
        PermissionState::AskEachTime => "ask_each_time",
        PermissionState::Disabled => "disabled",
    }
}

fn str_to_permission_state(s: &str) -> Option<crate::tools::permissions::PermissionState> {
    serde_json::from_value(serde_json::Value::String(s.to_string())).ok()
}

/// Check the secrets store for vaulted API keys and annotate the settings map.
///
/// For builtin overrides and custom providers whose API key was stripped from
/// settings (stored in secrets), this adds `api_key: "••••••••"` so the
/// frontend knows a key is configured without seeing the actual value.
async fn annotate_secret_key_presence(
    state: &GatewayState,
    user_id: &str,
    settings: &mut std::collections::HashMap<String, serde_json::Value>,
) {
    let secrets = match state.secrets_store.as_ref() {
        Some(s) => s,
        None => return,
    };

    // Annotate builtin overrides
    if let Some(obj) = settings
        .get_mut("llm_builtin_overrides")
        .and_then(|v| v.as_object_mut())
    {
        let provider_ids: Vec<String> = obj.keys().cloned().collect();
        for provider_id in provider_ids {
            let has_key_in_settings = obj
                .get(&provider_id)
                .and_then(|v| v.get("api_key"))
                .is_some();
            if has_key_in_settings {
                continue; // Will be masked by mask_settings_api_keys
            }
            let secret_name = builtin_secret_name(&provider_id);
            if secrets.exists(user_id, &secret_name).await.unwrap_or(false)
                && let Some(o) = obj.get_mut(&provider_id).and_then(|v| v.as_object_mut())
            {
                o.insert(
                    "api_key".to_string(),
                    serde_json::Value::String(API_KEY_UNCHANGED.to_string()),
                );
            }
        }
    }

    // Annotate custom providers
    if let Some(arr) = settings
        .get_mut("llm_custom_providers")
        .and_then(|v| v.as_array_mut())
    {
        for provider_val in arr.iter_mut() {
            let provider_id = provider_val
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if provider_id.is_empty() {
                continue;
            }
            let has_key_in_settings = provider_val.get("api_key").is_some();
            if has_key_in_settings {
                continue;
            }
            let secret_name = custom_secret_name(&provider_id);
            if secrets.exists(user_id, &secret_name).await.unwrap_or(false)
                && let Some(o) = provider_val.as_object_mut()
            {
                o.insert(
                    "api_key".to_string(),
                    serde_json::Value::String(API_KEY_UNCHANGED.to_string()),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Arc;

    use axum::{
        Json,
        extract::{Path, State},
        http::StatusCode,
    };

    use crate::channels::web::auth::UserIdentity;

    #[test]
    fn test_mask_settings_api_keys_builtin_overrides() {
        let mut settings = HashMap::new();
        settings.insert(
            "llm_builtin_overrides".to_string(),
            serde_json::json!({
                "openai": { "api_key": "sk-secret-123", "model": "gpt-4" },
                "anthropic": { "model": "claude-3" }
            }),
        );

        mask_settings_api_keys(&mut settings);

        let overrides = settings["llm_builtin_overrides"].as_object().unwrap();
        assert_eq!(
            overrides["openai"]["api_key"].as_str().unwrap(),
            API_KEY_UNCHANGED,
        );
        assert_eq!(overrides["openai"]["model"].as_str().unwrap(), "gpt-4");
        assert!(overrides["anthropic"].get("api_key").is_none());
    }

    #[test]
    fn test_mask_settings_api_keys_custom_providers() {
        let mut settings = HashMap::new();
        settings.insert(
            "llm_custom_providers".to_string(),
            serde_json::json!([
                { "id": "my-llm", "api_key": "secret-key", "adapter": "open_ai_completions" },
                { "id": "no-key", "adapter": "ollama" }
            ]),
        );

        mask_settings_api_keys(&mut settings);

        let providers = settings["llm_custom_providers"].as_array().unwrap();
        assert_eq!(providers[0]["api_key"].as_str().unwrap(), API_KEY_UNCHANGED,);
        assert!(providers[1].get("api_key").is_none());
    }

    #[test]
    fn test_mask_settings_no_llm_keys_is_noop() {
        let mut settings = HashMap::new();
        settings.insert("some_other_setting".to_string(), serde_json::json!("value"));

        mask_settings_api_keys(&mut settings);

        assert_eq!(settings["some_other_setting"].as_str().unwrap(), "value");
    }

    #[test]
    fn test_builtin_secret_name_format() {
        assert_eq!(builtin_secret_name("openai"), "llm_builtin_openai_api_key");
    }

    #[test]
    fn test_custom_secret_name_format() {
        assert_eq!(custom_secret_name("my-groq"), "llm_custom_my-groq_api_key");
    }

    fn test_secrets_store() -> Arc<dyn SecretsStore + Send + Sync> {
        let crypto = Arc::new(
            crate::secrets::SecretsCrypto::new(secrecy::SecretString::from(
                crate::secrets::keychain::generate_master_key_hex(),
            ))
            .unwrap(),
        );
        Arc::new(crate::secrets::InMemorySecretsStore::new(crypto))
    }

    fn test_gateway_state(secrets: Arc<dyn SecretsStore + Send + Sync>) -> GatewayState {
        GatewayState {
            msg_tx: tokio::sync::RwLock::new(None),
            sse: Arc::new(crate::channels::web::sse::SseManager::new()),
            workspace: None,
            workspace_pool: None,
            session_manager: None,
            log_broadcaster: None,
            log_level_handle: None,
            extension_manager: None,
            tool_registry: None,
            store: None,
            job_manager: None,
            prompt_queue: None,
            scheduler: None,
            owner_id: "test".to_string(),
            shutdown_tx: tokio::sync::RwLock::new(None),
            ws_tracker: None,
            llm_provider: None,
            skill_registry: None,
            skill_catalog: None,
            auth_manager: None,
            chat_rate_limiter: crate::channels::web::server::PerUserRateLimiter::new(30, 60),
            oauth_rate_limiter: crate::channels::web::server::PerUserRateLimiter::new(20, 60),
            webhook_rate_limiter: crate::channels::web::server::RateLimiter::new(10, 60),
            registry_entries: Vec::new(),
            cost_guard: None,
            routine_engine: Arc::new(tokio::sync::RwLock::new(None)),
            startup_time: std::time::Instant::now(),
            active_config: crate::channels::web::server::ActiveConfigSnapshot::default(),
            secrets_store: Some(secrets),
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
        }
    }

    async fn test_gateway_state_with_store(
        secrets: Arc<dyn SecretsStore + Send + Sync>,
    ) -> (Arc<GatewayState>, tempfile::TempDir) {
        let (db, tmp) = crate::testing::test_db().await;
        let mut state = test_gateway_state(secrets);
        state.store = Some(db);
        (Arc::new(state), tmp)
    }

    #[tokio::test]
    async fn test_extract_builtin_keys_vaults_and_strips() {
        let secrets = test_secrets_store();
        let state = test_gateway_state(Arc::clone(&secrets));

        let input = serde_json::json!({
            "openai": { "api_key": "sk-test-key", "model": "gpt-4" },
            "anthropic": { "model": "claude-3" }
        });

        let result = extract_builtin_override_keys(&state, "test", &input)
            .await
            .unwrap();

        let obj = result.as_object().unwrap();
        assert!(
            obj["openai"].get("api_key").is_none(),
            "api_key should be stripped"
        );
        assert_eq!(obj["openai"]["model"].as_str().unwrap(), "gpt-4");
        assert_eq!(obj["anthropic"]["model"].as_str().unwrap(), "claude-3");

        let decrypted = secrets
            .get_decrypted("test", "llm_builtin_openai_api_key")
            .await
            .unwrap();
        assert_eq!(decrypted.expose(), "sk-test-key");
    }

    #[tokio::test]
    async fn test_extract_custom_keys_vaults_and_strips() {
        let secrets = test_secrets_store();
        let state = test_gateway_state(Arc::clone(&secrets));

        let input = serde_json::json!([
            { "id": "my-llm", "api_key": "gsk-custom-key", "adapter": "open_ai_completions" },
            { "id": "local", "adapter": "ollama" }
        ]);

        let result = extract_custom_provider_keys(&state, "test", &input)
            .await
            .unwrap();

        let arr = result.as_array().unwrap();
        assert!(
            arr[0].get("api_key").is_none(),
            "api_key should be stripped"
        );
        assert_eq!(arr[0]["id"].as_str().unwrap(), "my-llm");
        assert!(arr[1].get("api_key").is_none());

        let decrypted = secrets
            .get_decrypted("test", "llm_custom_my-llm_api_key")
            .await
            .unwrap();
        assert_eq!(decrypted.expose(), "gsk-custom-key");
    }

    #[tokio::test]
    async fn test_unchanged_sentinel_preserves_existing_secret() {
        let secrets = test_secrets_store();

        secrets
            .create(
                "test",
                CreateSecretParams {
                    name: "llm_builtin_openai_api_key".to_string(),
                    value: SecretString::from("sk-original".to_string()),
                    provider: Some("openai".to_string()),
                    expires_at: None,
                },
            )
            .await
            .unwrap();

        let state = test_gateway_state(Arc::clone(&secrets));

        let input = serde_json::json!({
            "openai": { "api_key": "••••••••", "model": "gpt-4" }
        });

        let result = extract_builtin_override_keys(&state, "test", &input)
            .await
            .unwrap();

        assert!(result["openai"].get("api_key").is_none());

        let decrypted = secrets
            .get_decrypted("test", "llm_builtin_openai_api_key")
            .await
            .unwrap();
        assert_eq!(decrypted.expose(), "sk-original");
    }

    /// When secrets store is unavailable, attempting to save a real API key
    /// must fail with 503 rather than silently storing plaintext.
    #[tokio::test]
    async fn test_extract_builtin_keys_rejects_without_secrets_store() {
        let state = GatewayState {
            secrets_store: None,
            ..test_gateway_state(test_secrets_store())
        };

        let input = serde_json::json!({
            "openai": { "api_key": "sk-real-key", "model": "gpt-4" }
        });

        let err = extract_builtin_override_keys(&state, "test", &input)
            .await
            .unwrap_err();
        assert_eq!(err, StatusCode::SERVICE_UNAVAILABLE);
    }

    /// When secrets store is unavailable but no real keys are present
    /// (only sentinels or no api_key at all), the call should succeed.
    #[tokio::test]
    async fn test_extract_builtin_keys_allows_no_keys_without_secrets_store() {
        let state = GatewayState {
            secrets_store: None,
            ..test_gateway_state(test_secrets_store())
        };

        let input = serde_json::json!({
            "openai": { "api_key": "••••••••", "model": "gpt-4" },
            "anthropic": { "model": "claude-3" }
        });

        let result = extract_builtin_override_keys(&state, "test", &input)
            .await
            .unwrap();
        // Without secrets store, the value passes through unchanged (no vaulting needed).
        assert!(result.as_object().is_some());
    }

    #[tokio::test]
    async fn test_extract_custom_keys_rejects_without_secrets_store() {
        let state = GatewayState {
            secrets_store: None,
            ..test_gateway_state(test_secrets_store())
        };

        let input = serde_json::json!([
            { "id": "my-llm", "api_key": "gsk-real-key", "adapter": "open_ai_completions" }
        ]);

        let err = extract_custom_provider_keys(&state, "test", &input)
            .await
            .unwrap_err();
        assert_eq!(err, StatusCode::SERVICE_UNAVAILABLE);
    }

    // --- Provider ID validation tests ---

    #[test]
    fn test_valid_provider_ids() {
        assert!(is_valid_provider_id("my-llm"));
        assert!(is_valid_provider_id("openai"));
        assert!(is_valid_provider_id("custom-provider-123"));
        assert!(is_valid_provider_id("a"));
        assert!(is_valid_provider_id("my_llm"), "underscores allowed");
        assert!(
            is_valid_provider_id("openai_compatible"),
            "matches builtin naming"
        );
    }

    #[test]
    fn test_invalid_provider_ids() {
        assert!(!is_valid_provider_id(""), "empty ID");
        assert!(!is_valid_provider_id("My-LLM"), "uppercase");
        assert!(!is_valid_provider_id("my llm"), "spaces");
        assert!(!is_valid_provider_id("../../etc"), "path traversal");
        assert!(!is_valid_provider_id("a.b"), "dots");
        assert!(
            !is_valid_provider_id(&"a".repeat(65)),
            "exceeds 64 char limit"
        );
    }

    #[test]
    fn test_validate_custom_providers_rejects_bad_id() {
        let input = serde_json::json!([
            { "id": "UPPER-CASE", "adapter": "open_ai_completions" }
        ]);
        assert_eq!(
            validate_custom_providers(&input).unwrap_err(),
            StatusCode::UNPROCESSABLE_ENTITY,
        );
    }

    #[test]
    fn test_validate_custom_providers_accepts_valid() {
        let input = serde_json::json!([
            { "id": "my-llm", "adapter": "open_ai_completions" },
            { "id": "local-ollama", "adapter": "ollama" }
        ]);
        assert!(validate_custom_providers(&input).is_ok());
    }

    // --- Adapter validation tests ---

    #[test]
    fn test_validate_custom_providers_rejects_unknown_adapter() {
        let input = serde_json::json!([
            { "id": "test", "adapter": "not_a_real_adapter" }
        ]);
        assert_eq!(
            validate_custom_providers(&input).unwrap_err(),
            StatusCode::UNPROCESSABLE_ENTITY,
        );
    }

    #[test]
    fn test_validate_custom_providers_rejects_missing_adapter() {
        let input = serde_json::json!([
            { "id": "test" }
        ]);
        assert_eq!(
            validate_custom_providers(&input).unwrap_err(),
            StatusCode::UNPROCESSABLE_ENTITY,
        );
    }

    #[test]
    fn test_validate_custom_providers_accepts_all_valid_adapters() {
        for adapter in VALID_ADAPTERS {
            let input = serde_json::json!([
                { "id": "test", "adapter": adapter }
            ]);
            assert!(
                validate_custom_providers(&input).is_ok(),
                "adapter '{}' should be accepted",
                adapter
            );
        }
    }

    #[test]
    fn test_validate_custom_providers_non_array_is_ok() {
        let input = serde_json::json!("not-an-array");
        assert!(validate_custom_providers(&input).is_ok());
    }

    #[test]
    fn test_admin_only_setting_keys_include_network_destinations() {
        assert!(is_admin_only_setting_key("llm_builtin_overrides"));
        assert!(is_admin_only_setting_key("llm_custom_providers"));
        assert!(is_admin_only_setting_key("ollama_base_url"));
        assert!(is_admin_only_setting_key("openai_compatible_base_url"));
        assert!(!is_admin_only_setting_key("selected_model"));
    }

    #[tokio::test]
    async fn test_settings_set_rejects_member_for_admin_only_key() {
        let secrets = test_secrets_store();
        let (state, _tmp) = test_gateway_state_with_store(secrets).await;

        let status = settings_set_handler(
            State(state),
            AuthenticatedUser(UserIdentity {
                user_id: "member".to_string(),
                role: "member".to_string(),
                workspace_read_scopes: Vec::new(),
            }),
            Path("ollama_base_url".to_string()),
            Json(SettingWriteRequest {
                value: serde_json::json!("http://192.168.1.50:11434"),
            }),
        )
        .await
        .unwrap_err();

        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn test_settings_delete_rejects_member_for_admin_only_key() {
        let secrets = test_secrets_store();
        let (state, _tmp) = test_gateway_state_with_store(secrets).await;

        let status = settings_delete_handler(
            State(state),
            AuthenticatedUser(UserIdentity {
                user_id: "member".to_string(),
                role: "member".to_string(),
                workspace_read_scopes: Vec::new(),
            }),
            Path("llm_custom_providers".to_string()),
        )
        .await
        .unwrap_err();

        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn test_settings_import_rejects_member_for_admin_only_keys() {
        let secrets = test_secrets_store();
        let (state, _tmp) = test_gateway_state_with_store(secrets).await;
        let mut settings = HashMap::new();
        settings.insert(
            "openai_compatible_base_url".to_string(),
            serde_json::json!("https://192.168.1.60/v1"),
        );

        let status = settings_import_handler(
            State(state),
            AuthenticatedUser(UserIdentity {
                user_id: "member".to_string(),
                role: "member".to_string(),
                workspace_read_scopes: Vec::new(),
            }),
            Json(SettingsImportRequest { settings }),
        )
        .await
        .unwrap_err();

        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    // --- Tool permissions helpers ---

    #[test]
    fn test_permission_state_roundtrip() {
        use crate::tools::permissions::PermissionState;

        for (state, expected) in [
            (PermissionState::AlwaysAllow, "always_allow"),
            (PermissionState::AskEachTime, "ask_each_time"),
            (PermissionState::Disabled, "disabled"),
        ] {
            let s = permission_state_to_str(state);
            assert_eq!(s, expected);
            let back = str_to_permission_state(s).expect("roundtrip failed");
            assert_eq!(back, state);
        }
    }

    #[test]
    fn test_str_to_permission_state_rejects_unknown() {
        assert!(str_to_permission_state("invalid_value").is_none());
        assert!(str_to_permission_state("").is_none());
        assert!(str_to_permission_state("ALWAYS_ALLOW").is_none());
    }

    /// `PUT /api/settings/tools/:name` must return 400 for locked tools.
    ///
    /// A tool that returns `ApprovalRequirement::Always` from `requires_approval`
    /// is locked — callers cannot override its permission state via the API.
    /// We test this by checking the rejection path directly via the handler
    /// function using a minimal in-memory tool registry containing a mock
    /// "always-locked" tool.
    #[tokio::test]
    async fn test_put_locked_tool_returns_400() {
        use std::sync::Arc;

        use crate::context::JobContext;
        use crate::tools::ToolRegistry;
        use crate::tools::{ApprovalRequirement, Tool, ToolError, ToolOutput};
        use axum::Json;
        use axum::extract::{Path, State};

        // Minimal locked tool implementation.
        struct LockedTool;

        #[async_trait::async_trait]
        impl Tool for LockedTool {
            fn name(&self) -> &str {
                "locked_shell"
            }
            fn description(&self) -> &str {
                "A high-risk tool that always requires approval"
            }
            fn parameters_schema(&self) -> serde_json::Value {
                serde_json::json!({"type":"object","properties":{}})
            }
            async fn execute(
                &self,
                _params: serde_json::Value,
                _ctx: &JobContext,
            ) -> Result<ToolOutput, ToolError> {
                unreachable!("should never be called in this test")
            }
            fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
                ApprovalRequirement::Always
            }
        }

        let registry = Arc::new(ToolRegistry::new());
        registry.register(Arc::new(LockedTool)).await;

        let state = Arc::new(GatewayState {
            tool_registry: Some(registry),
            ..test_gateway_state(test_secrets_store())
        });

        let result = settings_tools_set_handler(
            State(state),
            crate::channels::web::auth::AuthenticatedUser(
                crate::channels::web::auth::UserIdentity {
                    user_id: "test".to_string(),
                    role: "admin".to_string(),
                    workspace_read_scopes: vec![],
                },
            ),
            Path::<String>("locked_shell".to_string()),
            Json(UpdateToolPermissionRequest {
                state: "always_allow".to_string(),
            }),
        )
        .await;

        let (status, _body) = result.unwrap_err();
        assert_eq!(
            status,
            axum::http::StatusCode::BAD_REQUEST,
            "locked tools should return 400"
        );
    }

    /// `GET /api/settings/tools` must return a list with the expected shape.
    ///
    /// Registers a single unlocked tool and verifies the response contains
    /// name, description, current_state, default_state, and locked=false.
    #[cfg(feature = "libsql")]
    #[tokio::test]
    async fn test_get_tools_returns_expected_shape() {
        use std::sync::Arc;

        use crate::context::JobContext;
        use crate::tools::ToolRegistry;
        use crate::tools::{ApprovalRequirement, Tool, ToolError, ToolOutput};
        use axum::extract::State;

        struct EchoLikeTool;

        #[async_trait::async_trait]
        impl Tool for EchoLikeTool {
            fn name(&self) -> &str {
                "echo_test"
            }
            fn description(&self) -> &str {
                "Test echo tool"
            }
            fn parameters_schema(&self) -> serde_json::Value {
                serde_json::json!({"type":"object","properties":{}})
            }
            async fn execute(
                &self,
                _params: serde_json::Value,
                _ctx: &JobContext,
            ) -> Result<ToolOutput, ToolError> {
                unreachable!()
            }
            fn requires_approval(&self, _params: &serde_json::Value) -> ApprovalRequirement {
                ApprovalRequirement::Never
            }
        }

        let registry = Arc::new(ToolRegistry::new());
        registry.register(Arc::new(EchoLikeTool)).await;

        // Provide a file-backed temp DB so the handler can load tool permissions.
        // In-memory databases do not share state between connections in libsql,
        // so we use a temporary file instead.
        use crate::db::Database;
        let tmp_dir = tempfile::tempdir().expect("tempdir");
        let db_path = tmp_dir.path().join("test.db");
        let db = crate::db::libsql::LibSqlBackend::new_local(&db_path)
            .await
            .expect("temp db");
        db.run_migrations().await.expect("migrations");
        let db: Arc<dyn Database> = Arc::new(db);

        let state = Arc::new(GatewayState {
            tool_registry: Some(registry),
            store: Some(db),
            ..test_gateway_state(test_secrets_store())
        });

        let result = settings_tools_list_handler(
            State(state),
            crate::channels::web::auth::AuthenticatedUser(
                crate::channels::web::auth::UserIdentity {
                    user_id: "test".to_string(),
                    role: "admin".to_string(),
                    workspace_read_scopes: vec![],
                },
            ),
        )
        .await;

        let axum::Json(response) = result.expect("handler should succeed");
        assert!(!response.tools.is_empty(), "should have at least one tool");

        let entry = response
            .tools
            .iter()
            .find(|t| t.name == "echo_test")
            .expect("echo_test tool should be in the list");

        assert_eq!(entry.name, "echo_test");
        assert_eq!(entry.description, "Test echo tool");
        assert!(!entry.locked);
        assert!(entry.locked_reason.is_none());
        // current_state and default_state must be valid strings
        assert!(matches!(
            entry.current_state.as_str(),
            "always_allow" | "ask_each_time" | "disabled"
        ));
        assert!(matches!(
            entry.default_state.as_str(),
            "always_allow" | "ask_each_time" | "disabled"
        ));
    }
}
