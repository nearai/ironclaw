//! Admin secrets provisioning handlers.
//!
//! Allows an admin (typically an application backend) to create, list, and
//! delete secrets on behalf of individual users so their IronClaw agent can
//! call back to external services with per-user credentials.

use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};

use crate::channels::web::auth::AdminUser;
use crate::channels::web::server::GatewayState;
use crate::secrets::CreateSecretParams;

/// PUT /api/admin/users/{user_id}/secrets/{name} — create or update a secret.
///
/// Upserts: if a secret with the same (user_id, name) already exists it is
/// overwritten. The plaintext value is encrypted at rest (AES-256-GCM) and
/// never returned by any endpoint.
pub async fn secrets_put_handler(
    State(state): State<Arc<GatewayState>>,
    AdminUser(_admin): AdminUser,
    Path((user_id, name)): Path<(String, String)>,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let name = name.to_lowercase();

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

    let value = body
        .get("value")
        .and_then(|v| v.as_str())
        .ok_or((
            StatusCode::BAD_REQUEST,
            "Missing required field 'value'".to_string(),
        ))?
        .to_string();

    let provider = body
        .get("provider")
        .and_then(|v| v.as_str())
        .map(String::from);

    let expires_in_days = body.get("expires_in_days").and_then(|v| v.as_u64());
    if let Some(days) = expires_in_days
        && days > 36500
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "expires_in_days must be at most 36500".to_string(),
        ));
    }
    let expires_at =
        expires_in_days.map(|days| chrono::Utc::now() + chrono::Duration::days(days as i64));

    let mut params = CreateSecretParams::new(name.clone(), value);
    if let Some(p) = provider {
        params = params.with_provider(p);
    }
    if let Some(exp) = expires_at {
        params = params.with_expiry(exp);
    }

    let already_exists = secrets
        .exists(&user_id, &name)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    secrets
        .create(&user_id, params)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok(Json(serde_json::json!({
        "user_id": user_id,
        "name": name,
        "status": if already_exists { "updated" } else { "created" },
    })))
}

/// GET /api/admin/users/{user_id}/secrets — list a user's secrets (names only).
///
/// Never returns secret values or hashes.
pub async fn secrets_list_handler(
    State(state): State<Arc<GatewayState>>,
    AdminUser(_admin): AdminUser,
    Path(user_id): Path<String>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    // Verify the target user exists (consistent with PUT/DELETE).
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

    let refs = secrets
        .list(&user_id)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let secrets_json: Vec<serde_json::Value> = refs
        .into_iter()
        .map(|r| {
            serde_json::json!({
                "name": r.name,
                "provider": r.provider,
            })
        })
        .collect();

    Ok(Json(serde_json::json!({
        "user_id": user_id,
        "secrets": secrets_json,
    })))
}

/// DELETE /api/admin/users/{user_id}/secrets/{name} — delete a user's secret.
pub async fn secrets_delete_handler(
    State(state): State<Arc<GatewayState>>,
    AdminUser(_admin): AdminUser,
    Path((user_id, name)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let name = name.to_lowercase();

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

    Ok(Json(serde_json::json!({
        "user_id": user_id,
        "name": name,
        "deleted": true,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channels::web::auth::UserIdentity;
    use crate::db::{Database, UserRecord};
    use crate::secrets::{InMemorySecretsStore, SecretsCrypto, SecretsStore};
    use secrecy::SecretString;

    fn test_admin() -> AdminUser {
        AdminUser(UserIdentity {
            user_id: "admin-1".to_string(),
            role: "admin".to_string(),
            workspace_read_scopes: Vec::new(),
        })
    }

    fn test_secrets_store() -> Arc<dyn SecretsStore + Send + Sync> {
        let crypto = Arc::new(
            SecretsCrypto::new(SecretString::from(
                crate::secrets::keychain::generate_master_key_hex(),
            ))
            .unwrap(),
        );
        Arc::new(InMemorySecretsStore::new(crypto))
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
            owner_id: "owner-1".to_string(),
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
            tool_dispatcher: None,
        }
    }

    async fn insert_test_user(db: &Arc<dyn Database>, id: &str, role: &str) {
        db.get_or_create_user(UserRecord {
            id: id.to_string(),
            role: role.to_string(),
            display_name: id.to_string(),
            status: "active".to_string(),
            email: None,
            last_login_at: None,
            created_by: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            metadata: serde_json::Value::Null,
        })
        .await
        .expect("create test user");
    }

    async fn test_gateway_state_with_store(
        secrets: Arc<dyn SecretsStore + Send + Sync>,
    ) -> (Arc<GatewayState>, tempfile::TempDir) {
        let (db, tmp) = crate::testing::test_db().await;
        insert_test_user(&db, "owner-1", "admin").await;
        let mut state = test_gateway_state(secrets);
        state.store = Some(db);
        (Arc::new(state), tmp)
    }

    #[tokio::test]
    async fn secrets_put_handler_preserves_multiline_values() {
        let secrets = test_secrets_store();
        let (state, _tmp) = test_gateway_state_with_store(Arc::clone(&secrets)).await;
        let kubeconfig = "apiVersion: v1\nclusters:\n- name: test\n";

        let response = secrets_put_handler(
            State(Arc::clone(&state)),
            test_admin(),
            Path((
                "owner-1".to_string(),
                "sandbox_kubernetes_kubeconfig".to_string(),
            )),
            Json(serde_json::json!({ "value": kubeconfig })),
        )
        .await
        .expect("store multiline kubeconfig");

        let body = response.0;
        assert_eq!(body["status"], "created");

        let stored = secrets
            .get_decrypted("owner-1", "sandbox_kubernetes_kubeconfig")
            .await
            .expect("read stored kubeconfig");
        assert_eq!(stored.expose(), kubeconfig);
    }

    #[tokio::test]
    async fn secrets_put_handler_updates_existing_kubeconfig_secret() {
        let secrets = test_secrets_store();
        let (state, _tmp) = test_gateway_state_with_store(Arc::clone(&secrets)).await;

        let _ = secrets_put_handler(
            State(Arc::clone(&state)),
            test_admin(),
            Path((
                "owner-1".to_string(),
                "sandbox_kubernetes_kubeconfig".to_string(),
            )),
            Json(serde_json::json!({ "value": "first: value\n" })),
        )
        .await
        .expect("create kubeconfig secret");

        let response = secrets_put_handler(
            State(state),
            test_admin(),
            Path((
                "owner-1".to_string(),
                "sandbox_kubernetes_kubeconfig".to_string(),
            )),
            Json(serde_json::json!({ "value": "second: value\n" })),
        )
        .await
        .expect("update kubeconfig secret");

        let body = response.0;
        assert_eq!(body["status"], "updated");

        let stored = secrets
            .get_decrypted("owner-1", "sandbox_kubernetes_kubeconfig")
            .await
            .expect("read updated kubeconfig");
        assert_eq!(stored.expose(), "second: value\n");
    }

    #[cfg(feature = "kubernetes")]
    struct EnvGuard(&'static str, Option<String>);

    #[cfg(feature = "kubernetes")]
    impl Drop for EnvGuard {
        fn drop(&mut self) {
            unsafe {
                match &self.1 {
                    Some(value) => std::env::set_var(self.0, value),
                    None => std::env::remove_var(self.0),
                }
            }
        }
    }

    #[cfg(feature = "kubernetes")]
    fn valid_kubeconfig_yaml() -> &'static str {
        r#"
apiVersion: v1
clusters:
- cluster:
    certificate-authority-data: aGVsbG8K
    server: https://127.0.0.1:6443
  name: test
contexts:
- context:
    cluster: test
    namespace: ironclaw
    user: test-user
  name: test
current-context: test
kind: Config
preferences: {}
users:
- name: test-user
  user:
    token: test-token
"#
    }

    #[cfg(feature = "kubernetes")]
    #[tokio::test]
    async fn admin_provisioned_kubeconfig_is_visible_to_runtime_resolver() {
        let _guard = crate::config::helpers::lock_env();
        let missing = std::env::temp_dir().join("ironclaw-missing-admin-kubeconfig");
        let _kubeconfig_guard = EnvGuard("KUBECONFIG", std::env::var("KUBECONFIG").ok());
        let _service_host_guard = EnvGuard(
            "KUBERNETES_SERVICE_HOST",
            std::env::var("KUBERNETES_SERVICE_HOST").ok(),
        );
        let _service_port_guard = EnvGuard(
            "KUBERNETES_SERVICE_PORT",
            std::env::var("KUBERNETES_SERVICE_PORT").ok(),
        );
        unsafe {
            std::env::set_var("KUBECONFIG", &missing);
            std::env::remove_var("KUBERNETES_SERVICE_HOST");
            std::env::remove_var("KUBERNETES_SERVICE_PORT");
        }

        let secrets = test_secrets_store();
        let (state, _tmp) = test_gateway_state_with_store(Arc::clone(&secrets)).await;

        let _ = secrets_put_handler(
            State(Arc::clone(&state)),
            test_admin(),
            Path((
                "owner-1".to_string(),
                "sandbox_kubernetes_kubeconfig".to_string(),
            )),
            Json(serde_json::json!({ "value": valid_kubeconfig_yaml() })),
        )
        .await
        .expect("provision kubeconfig secret");

        let resolved = crate::sandbox::kubernetes::KubernetesRuntime::resolve_with_auth(
            crate::sandbox::runtime::KubernetesAuthContext::new(
                Some("owner-1"),
                Some(secrets.as_ref()),
            ),
        )
        .await
        .expect("resolve kubeconfig from admin-provisioned secret");

        assert_eq!(
            resolved.source(),
            crate::sandbox::kubernetes::KubernetesCredentialSource::PlatformSecret
        );
    }
}
