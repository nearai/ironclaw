//! Adapters that bridge main-crate types into the trait surface
//! `ironclaw_llm` exposes for session management.
//!
//! `ironclaw_llm::SessionManager` owns NEAR-AI session-token lifecycle and
//! authenticates API requests, but it deliberately does not depend on the
//! main crate's `Database`, `SecretsStore`, `setup` UI, or `bootstrap` env
//! file helpers. The adapters in this module wire those concrete impls into
//! the LLM-side traits without forcing `ironclaw_llm` to know about them.
//!
//! See `crates/ironclaw_llm/src/host.rs` for the trait definitions.

use std::sync::Arc;

use async_trait::async_trait;
use ironclaw_llm::host::{SessionDb, SessionKeyPersistor, SessionSecrets};
use secrecy::SecretString;

use crate::db::Database;
use crate::secrets::{CreateSecretParams, SecretsStore};

/// Adapter exposing the main-crate `Database` as the narrow `SessionDb` trait.
pub struct DatabaseSessionDb {
    db: Arc<dyn Database>,
}

impl DatabaseSessionDb {
    pub fn new(db: Arc<dyn Database>) -> Self {
        Self { db }
    }
}

#[async_trait]
impl SessionDb for DatabaseSessionDb {
    async fn set_setting(
        &self,
        user_id: &str,
        key: &str,
        value: &serde_json::Value,
    ) -> Result<(), String> {
        self.db
            .set_setting(user_id, key, value)
            .await
            .map_err(|e| e.to_string())
    }

    async fn get_setting(
        &self,
        user_id: &str,
        key: &str,
    ) -> Result<Option<serde_json::Value>, String> {
        self.db
            .get_setting(user_id, key)
            .await
            .map_err(|e| e.to_string())
    }
}

/// Adapter exposing the encrypted `SecretsStore` as the narrow `SessionSecrets` trait.
pub struct SecretsStoreSessionSecrets {
    secrets: Arc<dyn SecretsStore + Send + Sync>,
}

impl SecretsStoreSessionSecrets {
    pub fn new(secrets: Arc<dyn SecretsStore + Send + Sync>) -> Self {
        Self { secrets }
    }
}

#[async_trait]
impl SessionSecrets for SecretsStoreSessionSecrets {
    async fn create(
        &self,
        user_id: &str,
        name: &str,
        value: String,
        provider: Option<&str>,
    ) -> Result<(), String> {
        let mut params = CreateSecretParams::new(name, value);
        if let Some(p) = provider {
            params = params.with_provider(p);
        }
        self.secrets
            .create(user_id, params)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    async fn get_decrypted(&self, user_id: &str, name: &str) -> Result<SecretString, String> {
        let plaintext = self
            .secrets
            .get_decrypted(user_id, name)
            .await
            .map_err(|e| e.to_string())?;
        Ok(SecretString::from(plaintext.expose().to_string()))
    }
}

/// `SessionKeyPersistor` impl wired to the main crate's runtime env overlay
/// and bootstrap `.env` writer. Used when the interactive renewer collects a
/// fresh NEAR-AI Cloud API key.
pub struct BootstrapKeyPersistor;

impl SessionKeyPersistor for BootstrapKeyPersistor {
    fn set_runtime_env(&self, key: &str, value: &str) {
        crate::config::helpers::set_runtime_env(key, value);
    }

    fn upsert_bootstrap_var(&self, key: &str, value: &str) -> std::io::Result<()> {
        crate::bootstrap::upsert_bootstrap_var(key, value)
    }
}
