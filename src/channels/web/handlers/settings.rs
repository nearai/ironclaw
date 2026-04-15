//! Settings API handlers.

use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use secrecy::SecretString;

use crate::channels::web::auth::AuthenticatedUser;
use crate::channels::web::server::GatewayState;
use crate::channels::web::types::*;
use crate::config::Config;
use crate::secrets::{CreateSecretParams, SecretsStore};

/// Sentinel value the frontend sends to mean "key is unchanged, don't touch it".
const API_KEY_UNCHANGED: &str = "••••••••";

/// Resolve the settings store from gateway state.
///
/// Prefers the `CachedSettingsStore` so writes invalidate the cache
/// (keeping the agent loop's view consistent). Falls back to the raw
/// `Database` when no cached store is configured.
pub(super) fn resolve_settings_store(
    state: &GatewayState,
) -> Result<&(dyn crate::db::SettingsStore + Send + Sync), StatusCode> {
    if let Some(ref sc) = state.settings_cache {
        Ok(sc.as_ref())
    } else if let Some(ref db) = state.store {
        Ok(db.as_ref())
    } else {
        Err(StatusCode::SERVICE_UNAVAILABLE)
    }
}

/// Resolve the effective user_id for a settings operation.
///
/// When `scope=admin`, the operation targets the shared admin-default scope
/// (`__admin__`). Only admin users may use this scope; non-admins get 403.
/// Without the scope parameter (or any other value), operations target the
/// calling user's own settings.
fn resolve_settings_scope(
    user: &crate::channels::web::auth::UserIdentity,
    query: &SettingScopeQuery,
) -> Result<String, StatusCode> {
    if query.scope.as_deref() == Some("admin") {
        if user.role != "admin" {
            tracing::warn!(
                user_id = %user.user_id,
                role = %user.role,
                "Non-admin attempted to use scope=admin on settings endpoint"
            );
            return Err(StatusCode::FORBIDDEN);
        }
        Ok(crate::tools::permissions::ADMIN_SETTINGS_USER_ID.to_string())
    } else {
        Ok(user.user_id.clone())
    }
}

pub async fn settings_list_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
) -> Result<Json<SettingsListResponse>, StatusCode> {
    let store = resolve_settings_store(&state)?;
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
    Query(query): Query<SettingScopeQuery>,
) -> Result<Json<SettingResponse>, StatusCode> {
    let effective_user_id = resolve_settings_scope(&user, &query)?;

    let store = resolve_settings_store(&state)?;
    let row = store
        .get_setting_full(&effective_user_id, &key)
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
        annotate_secret_key_presence(&state, &effective_user_id, &mut map).await;
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
    Query(query): Query<SettingScopeQuery>,
    Json(body): Json<SettingWriteRequest>,
) -> Result<StatusCode, StatusCode> {
    let effective_user_id = resolve_settings_scope(&user, &query)?;
    ensure_setting_write_allowed(&user, &key)?;

    let store = resolve_settings_store(&state)?;

    // Guard: cannot remove a custom provider that is currently active.
    if key == "llm_custom_providers" {
        guard_active_provider_not_removed(store, &effective_user_id, &body.value).await?;
        validate_custom_providers(&body.value)?;
    }

    // Extract API keys from LLM settings and vault them in the secrets store.
    // The sanitized value has api_key fields removed (stored encrypted instead).
    let sanitized_value = match key.as_str() {
        "llm_builtin_overrides" => {
            extract_builtin_override_keys(&state, &effective_user_id, &body.value).await?
        }
        "llm_custom_providers" => {
            extract_custom_provider_keys(&state, &effective_user_id, &body.value).await?
        }
        _ => body.value.clone(),
    };

    store
        .set_setting(&effective_user_id, &key, &sanitized_value)
        .await
        .map_err(|e| {
            tracing::error!("Failed to set setting '{}': {}", key, e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    if llm_setting_requires_reload(&key) {
        reload_llm_after_settings_change(&state).await?;
    }

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
    store: &(dyn crate::db::SettingsStore + Send + Sync),
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
    Query(query): Query<SettingScopeQuery>,
) -> Result<StatusCode, StatusCode> {
    let effective_user_id = resolve_settings_scope(&user, &query)?;
    ensure_setting_write_allowed(&user, &key)?;

    let store = resolve_settings_store(&state)?;

    // Guard: deleting llm_custom_providers is equivalent to setting it to [].
    // Reject if the active backend is a custom provider that would be removed.
    if key == "llm_custom_providers" {
        guard_active_provider_not_removed(
            store,
            &effective_user_id,
            &serde_json::Value::Array(vec![]),
        )
        .await?;
    }

    store
        .delete_setting(&effective_user_id, &key)
        .await
        .map_err(|e| {
            tracing::error!("Failed to delete setting '{}': {}", key, e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    if llm_setting_requires_reload(&key) {
        reload_llm_after_settings_change(&state).await?;
    }

    Ok(StatusCode::NO_CONTENT)
}

pub async fn settings_export_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
) -> Result<Json<SettingsExportResponse>, StatusCode> {
    let store = resolve_settings_store(&state)?;
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

    let store = resolve_settings_store(&state)?;

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

    if body
        .settings
        .keys()
        .any(|key| llm_setting_requires_reload(key))
    {
        reload_llm_after_settings_change(&state).await?;
    }

    Ok(StatusCode::NO_CONTENT)
}

fn llm_setting_requires_reload(key: &str) -> bool {
    matches!(
        key,
        "llm_backend"
            | "selected_model"
            | "ollama_base_url"
            | "openai_compatible_base_url"
            | "bedrock_region"
            | "bedrock_cross_region"
            | "bedrock_profile"
    )
}

async fn reload_llm_after_settings_change(state: &GatewayState) -> Result<(), StatusCode> {
    let Some(reloader) = state.llm_reload.as_ref() else {
        return Ok(());
    };
    let Some(store) = state.store.as_ref() else {
        return Ok(());
    };
    let Some(session_manager) = state.llm_session_manager.as_ref() else {
        return Ok(());
    };

    let config = Config::from_db_with_toml(
        store.as_ref(),
        &state.owner_id,
        state.config_toml_path.as_deref(),
        true,
    )
    .await
    .map_err(|e| {
        tracing::error!("Failed to reload config for LLM hot reload: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    reloader
        .reload(&config.llm, Arc::clone(session_manager))
        .await
        .map_err(|e| {
            tracing::error!("Failed to hot-reload LLM provider: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let active_model = state
        .llm_provider
        .as_ref()
        .map(|provider| provider.active_model_name())
        .unwrap_or_else(|| config.llm.active_model_name());
    let mut active_config = state.active_config.write().await;
    active_config.llm_backend = config.llm.backend.clone();
    active_config.llm_model = active_model;
    Ok(())
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

    // Load current user tool permission overrides from the cache.
    let store = resolve_settings_store(&state)?;
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

    // Persist the permission override, routed through the cached settings store
    // so the agent loop sees the change immediately.
    let store = resolve_settings_store(&state).map_err(|status| {
        (
            status,
            axum::Json(serde_json::json!({"error": "Settings store unavailable"})),
        )
    })?;

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
