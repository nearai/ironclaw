//! Operator-scoped storage for the Google OAuth client secret value.
//!
//! Google OAuth client identity (`client_id`, `redirect_uri`,
//! `hosted_domain_hint`) is public and lives in `config.toml`'s `[google]`
//! section; the client secret must never appear there (same law
//! `ironclaw_reborn_config::reject_inline_secret` enforces file-wide), so
//! `config set google.client_secret` puts it in this store instead.
//!
//! Mirrors [`crate::LlmKeyStore`]'s shape but with a single fixed handle,
//! since there is exactly one Google OAuth client per instance today.

use std::sync::Arc;

use ironclaw_host_api::{ResourceScope, SecretHandle};
use ironclaw_secrets::{SecretMaterial, SecretStore, SecretStoreError};
use thiserror::Error;

const HANDLE: &str = "google_oauth_client_secret";

/// Thin, operator-scoped wrapper over the shared [`SecretStore`] for the
/// Google OAuth client secret.
#[derive(Clone)]
pub struct GoogleOauthSecretStore {
    store: Arc<dyn SecretStore>,
}

impl GoogleOauthSecretStore {
    /// Wrap the instance's shared secret store.
    pub fn new(store: Arc<dyn SecretStore>) -> Self {
        Self { store }
    }

    /// Store (or replace) the Google OAuth client secret value.
    pub async fn put(&self, value: SecretMaterial) -> Result<(), GoogleOauthSecretStoreError> {
        self.store
            .put(scope(), handle()?, value, None)
            .await
            .map_err(GoogleOauthSecretStoreError::Store)?;
        Ok(())
    }

    /// [`Self::put`] taking a plain `String` rather than [`SecretMaterial`] —
    /// for callers outside this crate (namely `ironclaw_reborn_cli::commands::
    /// config::set`) that must not depend on `ironclaw_secrets` directly (see
    /// `crates/ironclaw_architecture/tests/reborn_dependency_boundaries.rs::reborn_cli_binary_crate_stays_separate_from_v1_root`,
    /// which pins `ironclaw_reborn_cli`'s allowed workspace dependency set —
    /// mirrors `LlmKeyStore::put_plaintext`'s reasoning).
    pub async fn put_plaintext(&self, value: String) -> Result<(), GoogleOauthSecretStoreError> {
        self.put(SecretMaterial::from(value)).await
    }

    /// Whether a client secret is currently stored (without revealing it).
    pub async fn exists(&self) -> Result<bool, GoogleOauthSecretStoreError> {
        Ok(self
            .store
            .metadata(&scope(), &handle()?)
            .await
            .map_err(GoogleOauthSecretStoreError::Store)?
            .is_some())
    }

    /// Read back the stored client secret, if any. Uses a one-shot lease +
    /// consume; the underlying secret persists, so this is repeatable
    /// across reloads (mirrors [`crate::LlmKeyStore::read`]).
    pub async fn read(&self) -> Result<Option<SecretMaterial>, GoogleOauthSecretStoreError> {
        let scope = scope();
        let lease = match self.store.lease_once(&scope, &handle()?).await {
            Ok(lease) => lease,
            Err(error) if error.is_unknown_secret() => return Ok(None),
            Err(error) => return Err(GoogleOauthSecretStoreError::Store(error)),
        };
        let material = self
            .store
            .consume(&scope, lease.id)
            .await
            .map_err(GoogleOauthSecretStoreError::Store)?;
        Ok(Some(material))
    }

    /// Delete the stored client secret. Returns whether one existed.
    pub async fn delete(&self) -> Result<bool, GoogleOauthSecretStoreError> {
        self.store
            .delete(&scope(), &handle()?)
            .await
            .map_err(GoogleOauthSecretStoreError::Store)
    }
}

fn scope() -> ResourceScope {
    ResourceScope::system()
}

fn handle() -> Result<SecretHandle, GoogleOauthSecretStoreError> {
    SecretHandle::new(HANDLE).map_err(|source| GoogleOauthSecretStoreError::InvalidHandle {
        reason: source.to_string(),
    })
}

/// Errors surfaced when storing or reading the Google OAuth client secret.
#[derive(Debug, Error)]
pub enum GoogleOauthSecretStoreError {
    #[error("invalid secret handle for Google OAuth client secret: {reason}")]
    InvalidHandle { reason: String },
    #[error("secret store error: {0}")]
    Store(#[source] SecretStoreError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclaw_secrets::InMemorySecretStore;

    fn store() -> GoogleOauthSecretStore {
        GoogleOauthSecretStore::new(Arc::new(InMemorySecretStore::new()))
    }

    #[tokio::test]
    async fn put_then_read_round_trips() {
        let secret = store();
        assert!(!secret.exists().await.expect("exists"));
        assert!(secret.read().await.expect("read").is_none());

        secret
            .put(SecretMaterial::from("GOCSPX-test-value"))
            .await
            .expect("put");

        assert!(secret.exists().await.expect("exists"));
        let value = secret.read().await.expect("read").expect("some");
        assert_eq!(
            secrecy::ExposeSecret::expose_secret(&value),
            "GOCSPX-test-value"
        );
    }

    #[tokio::test]
    async fn read_is_repeatable_across_reloads() {
        let secret = store();
        secret
            .put(SecretMaterial::from("GOCSPX-test-value"))
            .await
            .expect("put");
        assert!(secret.read().await.expect("read 1").is_some());
        assert!(secret.read().await.expect("read 2").is_some());
    }

    #[tokio::test]
    async fn delete_removes_secret() {
        let secret = store();
        secret.put(SecretMaterial::from("v")).await.expect("put");
        assert!(secret.delete().await.expect("delete"));
        assert!(!secret.exists().await.expect("exists"));
        assert!(!secret.delete().await.expect("delete again"));
    }
}
