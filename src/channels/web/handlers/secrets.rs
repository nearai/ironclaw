//! Secret provisioning handlers.
//!
//! Supports both admin-managed per-user secret CRUD and authenticated
//! self-service user secret management for the web UI.

use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::channels::web::auth::{AdminUser, AuthenticatedUser};
use crate::channels::web::platform::state::GatewayState;
use crate::secrets::binding_approvals::{
    list_binding_approvals, location_risk, revoke_binding_approval, revoke_secret_binding_approvals,
};
use crate::secrets::{CreateSecretParams, SecretsStore};

const MAX_SECRET_EXPIRY_DAYS: u64 = 36_500;
const MAX_SECRET_IMPORT_ITEMS: usize = 100;

#[derive(Debug, Deserialize)]
pub(crate) struct SecretWriteRequest {
    value: String,
    #[serde(default)]
    provider: Option<String>,
    #[serde(default)]
    expires_in_days: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SecretsImportRequest {
    #[serde(default)]
    secrets: Vec<ImportedSecretRequest>,
}

#[derive(Debug, Deserialize)]
struct ImportedSecretRequest {
    name: String,
    value: String,
    #[serde(default)]
    provider: Option<String>,
    #[serde(default)]
    expires_in_days: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SecretApprovalRevokeRequest {
    approval_id: String,
}

#[derive(Debug, Serialize)]
struct SecretBindingApprovalResponse {
    approval_id: String,
    artifact_kind: String,
    artifact_name: String,
    host: String,
    location: serde_json::Value,
    risk: String,
    approved_at: String,
    auto_bound: bool,
}

#[derive(Debug, Serialize)]
struct SecretListItemResponse {
    name: String,
    provider: Option<String>,
    configured: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    expires_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    updated_at: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    approvals: Vec<SecretBindingApprovalResponse>,
}

fn normalize_secret_name(name: &str) -> Result<String, (StatusCode, String)> {
    let normalized = name.trim().to_lowercase();
    if !ironclaw_skills::validate_credential_name(&normalized) {
        return Err((
            StatusCode::BAD_REQUEST,
            "Secret name must be lowercase alphanumeric/underscores, 1-64 chars".to_string(),
        ));
    }
    Ok(normalized)
}

fn normalize_provider(provider: Option<String>) -> Option<String> {
    provider
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn expiry_from_days(
    days: Option<u64>,
) -> Result<Option<chrono::DateTime<Utc>>, (StatusCode, String)> {
    if let Some(days) = days {
        if days > MAX_SECRET_EXPIRY_DAYS {
            return Err((
                StatusCode::BAD_REQUEST,
                format!("expires_in_days must be at most {}", MAX_SECRET_EXPIRY_DAYS),
            ));
        }
        return Ok(Some(Utc::now() + Duration::days(days as i64)));
    }
    Ok(None)
}

fn create_secret_params(
    name: String,
    value: String,
    provider: Option<String>,
    expires_in_days: Option<u64>,
) -> Result<CreateSecretParams, (StatusCode, String)> {
    if value.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "Missing required field 'value'".to_string(),
        ));
    }

    let mut params = CreateSecretParams::new(name, value);
    if let Some(provider) = normalize_provider(provider) {
        params = params.with_provider(provider);
    }
    if let Some(expires_at) = expiry_from_days(expires_in_days)? {
        params = params.with_expiry(expires_at);
    }
    Ok(params)
}

async fn upsert_secret_for_user(
    secrets: &(dyn SecretsStore + Send + Sync),
    user_id: &str,
    name: String,
    body: SecretWriteRequest,
) -> Result<serde_json::Value, (StatusCode, String)> {
    let already_exists = secrets
        .exists(user_id, &name)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let params = create_secret_params(
        name.clone(),
        body.value,
        body.provider,
        body.expires_in_days,
    )?;

    secrets
        .create(user_id, params)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(serde_json::json!({
        "user_id": user_id,
        "name": name,
        "status": if already_exists { "updated" } else { "created" },
    }))
}

fn resolve_optional_settings_store(
    state: &GatewayState,
) -> Option<&(dyn crate::db::SettingsStore + Send + Sync)> {
    if let Some(ref cache) = state.settings_cache {
        Some(cache.as_ref())
    } else {
        state.store.as_ref().map(|db| db.as_ref() as _)
    }
}

async fn list_secret_items(
    secrets: &(dyn SecretsStore + Send + Sync),
    settings_store: Option<&(dyn crate::db::SettingsStore + Send + Sync)>,
    user_id: &str,
) -> Result<Vec<SecretListItemResponse>, (StatusCode, String)> {
    let refs = secrets
        .list(user_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let approvals = list_binding_approvals(settings_store, user_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let mut approvals_by_secret: std::collections::HashMap<
        String,
        Vec<SecretBindingApprovalResponse>,
    > = std::collections::HashMap::new();
    for approval in approvals {
        approvals_by_secret
            .entry(approval.secret_name.clone())
            .or_default()
            .push(SecretBindingApprovalResponse {
                approval_id: approval.approval_id(),
                artifact_kind: approval.artifact_kind.as_str().to_string(),
                artifact_name: approval.artifact_name,
                host: approval.host,
                location: serde_json::to_value(&approval.location).unwrap_or_default(),
                risk: location_risk(&approval.location).to_string(),
                approved_at: approval.approved_at.to_rfc3339(),
                auto_bound: false,
            });
    }

    let mut items = Vec::with_capacity(refs.len());
    for secret_ref in refs {
        let metadata = secrets.get(user_id, &secret_ref.name).await.ok();
        items.push(SecretListItemResponse {
            name: secret_ref.name.clone(),
            provider: secret_ref.provider,
            configured: true,
            expires_at: metadata
                .as_ref()
                .and_then(|secret| secret.expires_at.map(|dt| dt.to_rfc3339())),
            updated_at: metadata
                .as_ref()
                .map(|secret| secret.updated_at.to_rfc3339()),
            approvals: approvals_by_secret
                .remove(&secret_ref.name)
                .unwrap_or_default(),
        });
    }

    Ok(items)
}

async fn import_secrets_for_user(
    secrets: &(dyn SecretsStore + Send + Sync),
    user_id: &str,
    body: SecretsImportRequest,
) -> Result<serde_json::Value, (StatusCode, String)> {
    if body.secrets.len() > MAX_SECRET_IMPORT_ITEMS {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "At most {} secrets may be imported at once",
                MAX_SECRET_IMPORT_ITEMS
            ),
        ));
    }

    let mut prepared = Vec::with_capacity(body.secrets.len());
    for entry in body.secrets {
        let name = normalize_secret_name(&entry.name)?;
        let params = create_secret_params(
            name.clone(),
            entry.value,
            entry.provider,
            entry.expires_in_days,
        )?;
        prepared.push((name, params));
    }

    let mut created = 0usize;
    let mut updated = 0usize;
    for (name, params) in prepared {
        let already_exists = secrets
            .exists(user_id, &name)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        secrets
            .create(user_id, params)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        if already_exists {
            updated += 1;
        } else {
            created += 1;
        }
    }

    Ok(serde_json::json!({
        "user_id": user_id,
        "imported": created + updated,
        "created": created,
        "updated": updated,
    }))
}

/// PUT /api/admin/users/{user_id}/secrets/{name} — create or update a secret.
///
/// Upserts: if a secret with the same (user_id, name) already exists it is
/// overwritten. The plaintext value is encrypted at rest (AES-256-GCM) and
/// never returned by any endpoint.
pub async fn secrets_put_handler(
    State(state): State<Arc<GatewayState>>,
    AdminUser(_admin): AdminUser,
    Path((user_id, name)): Path<(String, String)>,
    Json(body): Json<SecretWriteRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let name = normalize_secret_name(&name)?;

    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    store
        .get_user(&user_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "User not found".to_string()))?;

    let secrets = state.secrets_store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Secrets store not available".to_string(),
    ))?;

    Ok(Json(
        upsert_secret_for_user(secrets.as_ref(), &user_id, name, body).await?,
    ))
}

/// GET /api/admin/users/{user_id}/secrets — list a user's secrets (names only).
///
/// Never returns secret values or hashes.
pub async fn secrets_list_handler(
    State(state): State<Arc<GatewayState>>,
    AdminUser(_admin): AdminUser,
    Path(user_id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    if store
        .get_user(&user_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .is_none()
    {
        return Err((StatusCode::NOT_FOUND, "User not found".to_string()));
    }

    let secrets = state.secrets_store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Secrets store not available".to_string(),
    ))?;

    let items = list_secret_items(
        secrets.as_ref(),
        resolve_optional_settings_store(state.as_ref()),
        &user_id,
    )
    .await?;

    Ok(Json(serde_json::json!({
        "user_id": user_id,
        "secrets": items,
    })))
}

/// DELETE /api/admin/users/{user_id}/secrets/{name} — delete a user's secret.
pub async fn secrets_delete_handler(
    State(state): State<Arc<GatewayState>>,
    AdminUser(_admin): AdminUser,
    Path((user_id, name)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let name = normalize_secret_name(&name)?;

    let store = state.store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Database not available".to_string(),
    ))?;
    store
        .get_user(&user_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or((StatusCode::NOT_FOUND, "User not found".to_string()))?;

    let secrets = state.secrets_store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Secrets store not available".to_string(),
    ))?;

    let deleted = secrets
        .delete(&user_id, &name)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if !deleted {
        return Err((StatusCode::NOT_FOUND, "Secret not found".to_string()));
    }

    if let Err(error) = revoke_secret_binding_approvals(
        resolve_optional_settings_store(state.as_ref()),
        &user_id,
        &name,
    )
    .await
    {
        tracing::warn!(user_id = %user_id, secret_name = %name, error = %error, "Failed to revoke binding approvals after admin secret deletion");
    }

    Ok(Json(serde_json::json!({
        "user_id": user_id,
        "name": name,
        "deleted": true,
    })))
}

/// GET /api/secrets — list the authenticated user's secrets.
pub async fn user_secrets_list_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let secrets = state.secrets_store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Secrets store not available".to_string(),
    ))?;

    let items = list_secret_items(
        secrets.as_ref(),
        resolve_optional_settings_store(state.as_ref()),
        &user.user_id,
    )
    .await?;

    Ok(Json(serde_json::json!({
        "secrets": items,
    })))
}

/// PUT /api/secrets/{name} — create or update the authenticated user's secret.
pub async fn user_secrets_put_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(name): Path<String>,
    Json(body): Json<SecretWriteRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let name = normalize_secret_name(&name)?;
    let secrets = state.secrets_store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Secrets store not available".to_string(),
    ))?;

    Ok(Json(
        upsert_secret_for_user(secrets.as_ref(), &user.user_id, name, body).await?,
    ))
}

/// DELETE /api/secrets/{name} — delete the authenticated user's secret.
pub async fn user_secrets_delete_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(name): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let name = normalize_secret_name(&name)?;
    let secrets = state.secrets_store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Secrets store not available".to_string(),
    ))?;

    let deleted = secrets
        .delete(&user.user_id, &name)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if !deleted {
        return Err((StatusCode::NOT_FOUND, "Secret not found".to_string()));
    }

    if let Err(error) = revoke_secret_binding_approvals(
        resolve_optional_settings_store(state.as_ref()),
        &user.user_id,
        &name,
    )
    .await
    {
        tracing::warn!(user_id = %user.user_id, secret_name = %name, error = %error, "Failed to revoke binding approvals after secret deletion");
    }

    Ok(Json(serde_json::json!({
        "name": name,
        "deleted": true,
    })))
}

/// POST /api/secrets/import — bulk import the authenticated user's secrets.
pub async fn user_secrets_import_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Json(body): Json<SecretsImportRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let secrets = state.secrets_store.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Secrets store not available".to_string(),
    ))?;

    Ok(Json(
        import_secrets_for_user(secrets.as_ref(), &user.user_id, body).await?,
    ))
}

/// POST /api/secrets/{name}/approvals/revoke — revoke one stored binding approval.
pub async fn user_secret_approval_revoke_handler(
    State(state): State<Arc<GatewayState>>,
    AuthenticatedUser(user): AuthenticatedUser,
    Path(name): Path<String>,
    Json(body): Json<SecretApprovalRevokeRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let name = normalize_secret_name(&name)?;
    if body.approval_id.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "Missing required field 'approval_id'".to_string(),
        ));
    }

    let revoked = revoke_binding_approval(
        resolve_optional_settings_store(state.as_ref()),
        &user.user_id,
        body.approval_id.trim(),
    )
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    if !revoked {
        return Err((StatusCode::NOT_FOUND, "Approval not found".to_string()));
    }

    Ok(Json(serde_json::json!({
        "name": name,
        "approval_id": body.approval_id.trim(),
        "revoked": true,
    })))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use async_trait::async_trait;
    use axum::{Json, extract::Path};
    use chrono::Utc;

    use super::{
        SecretWriteRequest, SecretsImportRequest, user_secret_approval_revoke_handler,
        user_secrets_delete_handler, user_secrets_import_handler, user_secrets_list_handler,
        user_secrets_put_handler,
    };
    use crate::channels::IncomingMessage;
    use crate::channels::web::auth::{AuthenticatedUser, UserIdentity};
    use crate::channels::web::platform::state::{
        ActiveConfigSnapshot, GatewayState, PerUserRateLimiter, RateLimiter,
    };
    use crate::db::SettingsStore;
    use crate::history::SettingRow;
    use crate::secrets::binding_approvals::{grant_binding_approval, list_binding_approvals};
    use crate::secrets::{
        CredentialArtifactKind, CredentialLocation, InMemorySecretsStore, SecretBindingApproval,
        SecretsCrypto, SecretsStore,
    };

    struct MemorySettingsStore {
        values: tokio::sync::RwLock<HashMap<(String, String), serde_json::Value>>,
    }

    impl MemorySettingsStore {
        fn new() -> Self {
            Self {
                values: tokio::sync::RwLock::new(HashMap::new()),
            }
        }
    }

    #[async_trait]
    impl SettingsStore for MemorySettingsStore {
        async fn get_setting(
            &self,
            user_id: &str,
            key: &str,
        ) -> Result<Option<serde_json::Value>, crate::error::DatabaseError> {
            Ok(self
                .values
                .read()
                .await
                .get(&(user_id.to_string(), key.to_string()))
                .cloned())
        }

        async fn get_setting_full(
            &self,
            _user_id: &str,
            _key: &str,
        ) -> Result<Option<SettingRow>, crate::error::DatabaseError> {
            Ok(None)
        }

        async fn set_setting(
            &self,
            user_id: &str,
            key: &str,
            value: &serde_json::Value,
        ) -> Result<(), crate::error::DatabaseError> {
            self.values
                .write()
                .await
                .insert((user_id.to_string(), key.to_string()), value.clone());
            Ok(())
        }

        async fn delete_setting(
            &self,
            user_id: &str,
            key: &str,
        ) -> Result<bool, crate::error::DatabaseError> {
            Ok(self
                .values
                .write()
                .await
                .remove(&(user_id.to_string(), key.to_string()))
                .is_some())
        }

        async fn list_settings(
            &self,
            _user_id: &str,
        ) -> Result<Vec<SettingRow>, crate::error::DatabaseError> {
            Ok(Vec::new())
        }

        async fn get_all_settings(
            &self,
            user_id: &str,
        ) -> Result<HashMap<String, serde_json::Value>, crate::error::DatabaseError> {
            Ok(self
                .values
                .read()
                .await
                .iter()
                .filter(|((stored_user_id, _), _)| stored_user_id == user_id)
                .map(|((_, key), value)| (key.clone(), value.clone()))
                .collect())
        }

        async fn set_all_settings(
            &self,
            user_id: &str,
            settings: &HashMap<String, serde_json::Value>,
        ) -> Result<(), crate::error::DatabaseError> {
            let mut values = self.values.write().await;
            values.retain(|(stored_user_id, _), _| stored_user_id != user_id);
            for (key, value) in settings {
                values.insert((user_id.to_string(), key.clone()), value.clone());
            }
            Ok(())
        }

        async fn has_settings(&self, user_id: &str) -> Result<bool, crate::error::DatabaseError> {
            Ok(self
                .values
                .read()
                .await
                .keys()
                .any(|(stored_user_id, _)| stored_user_id == user_id))
        }
    }

    fn test_user(user_id: &str) -> AuthenticatedUser {
        AuthenticatedUser(UserIdentity {
            user_id: user_id.to_string(),
            role: "member".to_string(),
            workspace_read_scopes: Vec::new(),
        })
    }

    fn sample_approval(secret_name: &str) -> SecretBindingApproval {
        SecretBindingApproval {
            secret_name: secret_name.to_string(),
            artifact_kind: CredentialArtifactKind::Skill,
            artifact_name: "github-workflow".to_string(),
            artifact_fingerprint: "skill-hash-v1".to_string(),
            host: "api.github.com".to_string(),
            location: CredentialLocation::AuthorizationBearer,
            approved_at: Utc::now(),
        }
    }

    fn test_gateway_state() -> Arc<GatewayState> {
        test_gateway_state_with_settings(None)
    }

    fn test_gateway_state_with_settings(
        settings_cache: Option<Arc<crate::db::cached_settings::CachedSettingsStore>>,
    ) -> Arc<GatewayState> {
        let crypto = Arc::new(
            SecretsCrypto::new(secrecy::SecretString::from(
                crate::secrets::keychain::generate_master_key_hex(),
            ))
            .unwrap(),
        );
        let secrets: Arc<dyn SecretsStore + Send + Sync> =
            Arc::new(InMemorySecretsStore::new(crypto));

        Arc::new(GatewayState {
            msg_tx: tokio::sync::RwLock::new(None::<tokio::sync::mpsc::Sender<IncomingMessage>>),
            sse: Arc::new(crate::channels::web::sse::SseManager::new()),
            workspace: None,
            workspace_pool: None,
            session_manager: None,
            log_broadcaster: None,
            log_level_handle: None,
            extension_manager: None,
            tool_registry: None,
            store: None,
            settings_cache,
            job_manager: None,
            prompt_queue: None,
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
            scheduler: None,
            chat_rate_limiter: PerUserRateLimiter::new(30, 60),
            oauth_rate_limiter: PerUserRateLimiter::new(20, 60),
            webhook_rate_limiter: RateLimiter::new(10, 60),
            registry_entries: Vec::new(),
            cost_guard: None,
            routine_engine: Arc::new(tokio::sync::RwLock::new(None)),
            startup_time: std::time::Instant::now(),
            active_config: Arc::new(tokio::sync::RwLock::new(ActiveConfigSnapshot::default())),
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
            frontend_html_cache: Arc::new(tokio::sync::RwLock::new(None)),
            tool_dispatcher: None,
        })
    }

    #[tokio::test]
    async fn user_secret_round_trip_stays_scoped_to_authenticated_user() {
        let state = test_gateway_state();

        let Json(created) = user_secrets_put_handler(
            axum::extract::State(Arc::clone(&state)),
            test_user("alice"),
            Path("github_token".to_string()),
            Json(SecretWriteRequest {
                value: "ghp_secret".to_string(),
                provider: Some("github".to_string()),
                expires_in_days: Some(30),
            }),
        )
        .await
        .expect("create secret");
        assert_eq!(created["status"], "created");

        let Json(listed) =
            user_secrets_list_handler(axum::extract::State(Arc::clone(&state)), test_user("alice"))
                .await
                .expect("list secrets");
        let secrets = listed["secrets"].as_array().expect("secrets array");
        assert_eq!(secrets.len(), 1);
        assert_eq!(secrets[0]["name"], "github_token");
        assert_eq!(secrets[0]["provider"], "github");
        assert!(
            secrets[0].get("value").is_none(),
            "list must never return secret values"
        );

        let Json(other_user) =
            user_secrets_list_handler(axum::extract::State(Arc::clone(&state)), test_user("bob"))
                .await
                .expect("list secrets for second user");
        assert_eq!(other_user["secrets"].as_array().unwrap().len(), 0);

        let Json(deleted) = user_secrets_delete_handler(
            axum::extract::State(Arc::clone(&state)),
            test_user("alice"),
            Path("github_token".to_string()),
        )
        .await
        .expect("delete secret");
        assert_eq!(deleted["deleted"], true);
    }

    #[tokio::test]
    async fn user_secret_import_creates_multiple_entries() {
        let state = test_gateway_state();

        let Json(imported) = user_secrets_import_handler(
            axum::extract::State(Arc::clone(&state)),
            test_user("alice"),
            Json(SecretsImportRequest {
                secrets: vec![
                    super::ImportedSecretRequest {
                        name: "github_token".to_string(),
                        value: "ghp_secret".to_string(),
                        provider: Some("github".to_string()),
                        expires_in_days: None,
                    },
                    super::ImportedSecretRequest {
                        name: "linear_api_key".to_string(),
                        value: "lin_secret".to_string(),
                        provider: Some("linear".to_string()),
                        expires_in_days: Some(7),
                    },
                ],
            }),
        )
        .await
        .expect("import secrets");

        assert_eq!(imported["imported"], 2);
        assert_eq!(imported["created"], 2);
        assert_eq!(imported["updated"], 0);

        let Json(listed) =
            user_secrets_list_handler(axum::extract::State(state), test_user("alice"))
                .await
                .expect("list imported secrets");
        assert_eq!(listed["secrets"].as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn user_secret_list_and_revoke_surface_binding_approvals() {
        let settings_inner: Arc<dyn SettingsStore + Send + Sync> =
            Arc::new(MemorySettingsStore::new());
        let settings_cache = Arc::new(crate::db::cached_settings::CachedSettingsStore::new(
            Arc::clone(&settings_inner),
        ));
        let state = test_gateway_state_with_settings(Some(Arc::clone(&settings_cache)));

        let _ = user_secrets_put_handler(
            axum::extract::State(Arc::clone(&state)),
            test_user("alice"),
            Path("github_token".to_string()),
            Json(SecretWriteRequest {
                value: "ghp_secret".to_string(),
                provider: Some("github".to_string()),
                expires_in_days: None,
            }),
        )
        .await
        .expect("create secret");

        let approval = sample_approval("github_token");
        grant_binding_approval(Some(settings_cache.as_ref()), "alice", approval.clone())
            .await
            .expect("grant approval");

        let Json(listed) =
            user_secrets_list_handler(axum::extract::State(Arc::clone(&state)), test_user("alice"))
                .await
                .expect("list secrets with approvals");
        let approvals = listed["secrets"][0]["approvals"]
            .as_array()
            .expect("approvals array");
        assert_eq!(approvals.len(), 1);
        assert_eq!(approvals[0]["host"], "api.github.com");
        assert_eq!(approvals[0]["risk"], "normal");

        let Json(revoked) = user_secret_approval_revoke_handler(
            axum::extract::State(Arc::clone(&state)),
            test_user("alice"),
            Path("github_token".to_string()),
            Json(super::SecretApprovalRevokeRequest {
                approval_id: approval.approval_id(),
            }),
        )
        .await
        .expect("revoke approval");
        assert_eq!(revoked["revoked"], true);

        let Json(listed_after) =
            user_secrets_list_handler(axum::extract::State(state), test_user("alice"))
                .await
                .expect("list secrets after revoke");
        assert!(listed_after["secrets"][0].get("approvals").is_none());
    }

    #[tokio::test]
    async fn user_secret_delete_revokes_persisted_binding_approvals() {
        let settings_inner: Arc<dyn SettingsStore + Send + Sync> =
            Arc::new(MemorySettingsStore::new());
        let settings_cache = Arc::new(crate::db::cached_settings::CachedSettingsStore::new(
            Arc::clone(&settings_inner),
        ));
        let state = test_gateway_state_with_settings(Some(Arc::clone(&settings_cache)));

        let _ = user_secrets_put_handler(
            axum::extract::State(Arc::clone(&state)),
            test_user("alice"),
            Path("github_token".to_string()),
            Json(SecretWriteRequest {
                value: "ghp_secret".to_string(),
                provider: Some("github".to_string()),
                expires_in_days: None,
            }),
        )
        .await
        .expect("create secret");

        grant_binding_approval(
            Some(settings_cache.as_ref()),
            "alice",
            sample_approval("github_token"),
        )
        .await
        .expect("grant approval");

        let _ = user_secrets_delete_handler(
            axum::extract::State(Arc::clone(&state)),
            test_user("alice"),
            Path("github_token".to_string()),
        )
        .await
        .expect("delete secret");

        let approvals = list_binding_approvals(Some(settings_cache.as_ref()), "alice")
            .await
            .expect("list approvals after delete");
        assert!(
            approvals.is_empty(),
            "secret delete should revoke approvals"
        );
    }
}
