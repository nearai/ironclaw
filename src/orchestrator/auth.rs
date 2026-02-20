//! Per-job bearer token authentication for worker-to-orchestrator communication.
//!
//! Security properties:
//! - Tokens are cryptographically random (32 bytes, hex-encoded)
//! - Tokens are scoped to a specific job_id
//! - Tokens are ephemeral (in-memory only, never persisted)
//! - A token for Job A cannot access endpoints for Job B
//! - Credential grants are per-job: only secrets explicitly granted are accessible

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::Response;
use rand::Rng;
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;
use tokio::sync::RwLock;
use uuid::Uuid;

/// A credential grant that maps a secret (stored in SecretsStore) to an
/// environment variable name the container worker expects.
///
/// For example: `{ secret_name: "github_token", env_var: "GITHUB_TOKEN" }`
/// means "decrypt the secret named `github_token` and provide it as the
/// env var `GITHUB_TOKEN` to the container".
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CredentialGrant {
    pub secret_name: String,
    pub env_var: String,
}

/// In-memory store for per-job authentication tokens and credential grants.
#[derive(Clone)]
pub struct TokenStore {
    /// Maps job_id -> bearer token. Never logged or persisted.
    tokens: Arc<RwLock<HashMap<Uuid, String>>>,
    /// Maps job_id -> granted credentials. Revoked alongside the token.
    credential_grants: Arc<RwLock<HashMap<Uuid, Vec<CredentialGrant>>>>,
}

impl TokenStore {
    pub fn new() -> Self {
        Self {
            tokens: Arc::new(RwLock::new(HashMap::new())),
            credential_grants: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Generate and store a new token for a job.
    pub async fn create_token(&self, job_id: Uuid) -> String {
        let token = generate_token();
        self.tokens.write().await.insert(job_id, token.clone());
        token
    }

    /// Validate a token for a specific job (constant-time comparison).
    pub async fn validate(&self, job_id: Uuid, token: &str) -> bool {
        self.tokens
            .read()
            .await
            .get(&job_id)
            .map(|stored| stored.as_bytes().ct_eq(token.as_bytes()).into())
            .unwrap_or(false)
    }

    /// Remove a token and its credential grants (on container cleanup).
    pub async fn revoke(&self, job_id: Uuid) {
        self.tokens.write().await.remove(&job_id);
        self.credential_grants.write().await.remove(&job_id);
    }

    /// Get the number of active tokens (for diagnostics).
    pub async fn active_count(&self) -> usize {
        self.tokens.read().await.len()
    }

    /// Store credential grants for a job. Call right after `create_token()`.
    pub async fn store_grants(&self, job_id: Uuid, grants: Vec<CredentialGrant>) {
        if !grants.is_empty() {
            self.credential_grants.write().await.insert(job_id, grants);
        }
    }

    /// Retrieve credential grants for a job.
    pub async fn get_grants(&self, job_id: Uuid) -> Option<Vec<CredentialGrant>> {
        self.credential_grants.read().await.get(&job_id).cloned()
    }
}

impl Default for TokenStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Generate a cryptographically random token (32 bytes, hex-encoded = 64 chars).
fn generate_token() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill(&mut bytes);
    // Hex-encode without pulling in a crate: fixed-size array, no allocation concern.
    bytes.iter().fold(String::with_capacity(64), |mut s, b| {
        use std::fmt::Write;
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// Axum middleware that validates worker bearer tokens.
///
/// Extracts the job_id from the path (`/worker/{job_id}/...`) and validates
/// the `Authorization: Bearer <token>` header against the token store.
///
/// Wire up with `axum::middleware::from_fn_with_state(token_store, worker_auth_middleware)`.
pub async fn worker_auth_middleware(
    State(token_store): State<TokenStore>,
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let path = request.uri().path().to_string();
    let job_id = extract_job_id_from_path(&path).ok_or(StatusCode::BAD_REQUEST)?;

    let token = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or(StatusCode::UNAUTHORIZED)?;

    if !token_store.validate(job_id, token).await {
        return Err(StatusCode::UNAUTHORIZED);
    }

    Ok(next.run(request).await)
}

/// Extract job UUID from a path like `/worker/{uuid}/...`
fn extract_job_id_from_path(path: &str) -> Option<Uuid> {
    let parts: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    if parts.len() >= 2 && parts[0] == "worker" {
        Uuid::parse_str(parts[1]).ok()
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_token_create_and_validate() {
        let store = TokenStore::new();
        let job_id = Uuid::new_v4();

        let token = store.create_token(job_id).await;
        assert_eq!(token.len(), 64); // 32 bytes hex = 64 chars

        assert!(store.validate(job_id, &token).await);
        assert!(!store.validate(job_id, "wrong-token").await);
        assert!(!store.validate(Uuid::new_v4(), &token).await);
    }

    #[tokio::test]
    async fn test_token_revoke() {
        let store = TokenStore::new();
        let job_id = Uuid::new_v4();

        let token = store.create_token(job_id).await;
        assert!(store.validate(job_id, &token).await);

        store.revoke(job_id).await;
        assert!(!store.validate(job_id, &token).await);
    }

    #[test]
    fn test_extract_job_id() {
        let id = Uuid::new_v4();
        let path = format!("/worker/{}/llm/complete", id);
        assert_eq!(extract_job_id_from_path(&path), Some(id));

        assert_eq!(extract_job_id_from_path("/other/path"), None);
        assert_eq!(extract_job_id_from_path("/worker/not-a-uuid/foo"), None);
    }

    #[test]
    fn test_token_is_random() {
        let t1 = generate_token();
        let t2 = generate_token();
        assert_ne!(t1, t2);
    }

    #[tokio::test]
    async fn test_store_and_get_grants() {
        let store = TokenStore::new();
        let job_id = Uuid::new_v4();

        // No grants initially
        assert!(store.get_grants(job_id).await.is_none());

        let grants = vec![
            CredentialGrant {
                secret_name: "github_token".to_string(),
                env_var: "GITHUB_TOKEN".to_string(),
            },
            CredentialGrant {
                secret_name: "npm_token".to_string(),
                env_var: "NPM_TOKEN".to_string(),
            },
        ];

        store.store_grants(job_id, grants).await;

        let retrieved = store.get_grants(job_id).await.unwrap();
        assert_eq!(retrieved.len(), 2);
        assert_eq!(retrieved[0].secret_name, "github_token");
        assert_eq!(retrieved[0].env_var, "GITHUB_TOKEN");
        assert_eq!(retrieved[1].secret_name, "npm_token");
    }

    #[tokio::test]
    async fn test_revoke_clears_grants() {
        let store = TokenStore::new();
        let job_id = Uuid::new_v4();

        let _token = store.create_token(job_id).await;
        store
            .store_grants(
                job_id,
                vec![CredentialGrant {
                    secret_name: "my_secret".to_string(),
                    env_var: "MY_SECRET".to_string(),
                }],
            )
            .await;

        assert!(store.get_grants(job_id).await.is_some());

        store.revoke(job_id).await;

        assert!(!store.validate(job_id, "anything").await);
        assert!(store.get_grants(job_id).await.is_none());
    }

    #[tokio::test]
    async fn test_empty_grants_not_stored() {
        let store = TokenStore::new();
        let job_id = Uuid::new_v4();

        store.store_grants(job_id, vec![]).await;

        // Empty vec should not create an entry
        assert!(store.get_grants(job_id).await.is_none());
    }

    #[tokio::test]
    async fn test_grants_isolated_per_job() {
        let store = TokenStore::new();
        let job_a = Uuid::new_v4();
        let job_b = Uuid::new_v4();

        store
            .store_grants(
                job_a,
                vec![CredentialGrant {
                    secret_name: "secret_a".to_string(),
                    env_var: "SECRET_A".to_string(),
                }],
            )
            .await;

        store
            .store_grants(
                job_b,
                vec![CredentialGrant {
                    secret_name: "secret_b".to_string(),
                    env_var: "SECRET_B".to_string(),
                }],
            )
            .await;

        let grants_a = store.get_grants(job_a).await.unwrap();
        assert_eq!(grants_a.len(), 1);
        assert_eq!(grants_a[0].secret_name, "secret_a");

        let grants_b = store.get_grants(job_b).await.unwrap();
        assert_eq!(grants_b.len(), 1);
        assert_eq!(grants_b[0].secret_name, "secret_b");
    }
}
