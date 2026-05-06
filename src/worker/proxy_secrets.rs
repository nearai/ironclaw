//! Proxy `SecretsStore` that delegates to the orchestrator via HTTP.
//!
//! Used inside container workers to give MCP OAuth clients access to
//! the host's secrets store without exposing it directly. Only the
//! subset of `SecretsStore` methods needed by the MCP token lifecycle
//! (`get_decrypted`, `exists`, `create`) are implemented; the rest
//! return sensible defaults.

use std::sync::Arc;

use async_trait::async_trait;
use secrecy::ExposeSecret;
use uuid::Uuid;

use crate::secrets::{
    CreateSecretParams, DecryptedSecret, Secret, SecretError, SecretRef, SecretsStore,
};
use crate::worker::api::WorkerHttpClient;

/// A `SecretsStore` that proxies read/write operations to the orchestrator
/// via the `/worker/{job_id}/mcp/secrets/*` endpoints.
pub struct ProxySecretsStore {
    client: Arc<WorkerHttpClient>,
}

impl ProxySecretsStore {
    pub fn new(client: Arc<WorkerHttpClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl SecretsStore for ProxySecretsStore {
    async fn create(
        &self,
        _user_id: &str,
        params: CreateSecretParams,
    ) -> Result<Secret, SecretError> {
        self.client
            .mcp_secret_create(
                &params.name,
                params.value.expose_secret(),
                params.expires_at,
            )
            .await
            .map_err(|e| classify_worker_error(&params.name, e))?;

        // Return a minimal Secret — the caller only needs confirmation, not the full record.
        Ok(Secret {
            id: Uuid::new_v4(),
            user_id: String::new(),
            name: params.name,
            encrypted_value: Vec::new(),
            key_salt: Vec::new(),
            provider: params.provider,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            last_used_at: None,
            expires_at: params.expires_at,
            usage_count: 0,
        })
    }

    async fn get(&self, _user_id: &str, name: &str) -> Result<Secret, SecretError> {
        // Not implemented — MCP OAuth flow only uses get_decrypted.
        Err(SecretError::NotFound(name.to_string()))
    }

    async fn get_decrypted(
        &self,
        _user_id: &str,
        name: &str,
    ) -> Result<DecryptedSecret, SecretError> {
        let value = self
            .client
            .mcp_secret_get(name)
            .await
            .map_err(|e| classify_worker_error(name, e))?;

        DecryptedSecret::from_bytes(value.into_bytes())
    }

    async fn exists(&self, _user_id: &str, name: &str) -> Result<bool, SecretError> {
        self.client
            .mcp_secret_exists(name)
            .await
            .map_err(|e| classify_worker_error(name, e))
    }

    async fn list(&self, _user_id: &str) -> Result<Vec<SecretRef>, SecretError> {
        Ok(vec![])
    }

    async fn delete(&self, _user_id: &str, _name: &str) -> Result<bool, SecretError> {
        Ok(false)
    }

    async fn record_usage(&self, _secret_id: Uuid) -> Result<(), SecretError> {
        Ok(())
    }

    async fn is_accessible(
        &self,
        _user_id: &str,
        _secret_name: &str,
        _allowed_secrets: &[String],
    ) -> Result<bool, SecretError> {
        // In container context, access control is handled by the orchestrator's allowlist.
        Ok(true)
    }
}

/// Map a `WorkerError` to the appropriate `SecretError` variant based on the
/// HTTP status code embedded in the error message.
fn classify_worker_error(name: &str, e: crate::error::WorkerError) -> SecretError {
    let msg = e.to_string();
    if msg.contains("returned 403") {
        SecretError::AccessDenied
    } else if msg.contains("returned 404") {
        SecretError::NotFound(name.to_string())
    } else if msg.contains("returned 503") {
        SecretError::Database("secrets store unavailable".to_string())
    } else {
        SecretError::Database(format!("{}: {}", name, msg))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_secrets_store_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<ProxySecretsStore>();
    }

    #[test]
    fn classify_403_maps_to_access_denied() {
        let err = crate::error::WorkerError::LlmProxyFailed {
            reason: "mcp secret get: orchestrator returned 403 Forbidden: ".to_string(),
        };
        let result = classify_worker_error("my_secret", err);
        assert!(matches!(result, SecretError::AccessDenied));
    }

    #[test]
    fn classify_404_maps_to_not_found() {
        let err = crate::error::WorkerError::LlmProxyFailed {
            reason: "mcp secret get: orchestrator returned 404 Not Found: ".to_string(),
        };
        let result = classify_worker_error("my_secret", err);
        assert!(matches!(result, SecretError::NotFound(ref name) if name == "my_secret"));
    }

    #[test]
    fn classify_503_maps_to_unavailable() {
        let err = crate::error::WorkerError::LlmProxyFailed {
            reason: "mcp secret get: orchestrator returned 503 Service Unavailable: ".to_string(),
        };
        let result = classify_worker_error("my_secret", err);
        assert!(
            matches!(result, SecretError::Database(ref msg) if msg == "secrets store unavailable")
        );
    }

    #[test]
    fn classify_other_maps_to_database_with_name() {
        let err = crate::error::WorkerError::LlmProxyFailed {
            reason: "mcp secret get: connection refused".to_string(),
        };
        let result = classify_worker_error("my_secret", err);
        assert!(matches!(result, SecretError::Database(ref msg) if msg.starts_with("my_secret:")));
    }

    #[test]
    fn classify_body_containing_403_does_not_misclassify() {
        // Body contains "403" but the status is 500 — should NOT match AccessDenied.
        let err = crate::error::WorkerError::LlmProxyFailed {
            reason: "mcp secret get: orchestrator returned 500 Internal Server Error: error code 403 in upstream".to_string(),
        };
        let result = classify_worker_error("my_secret", err);
        assert!(matches!(result, SecretError::Database(_)));
    }
}
