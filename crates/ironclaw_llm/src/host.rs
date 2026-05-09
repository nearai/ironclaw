//! Host-side abstractions used by `ironclaw_llm` to talk to the embedding
//! application without depending on its internals.
//!
//! `SessionManager` and the interactive NEAR-AI renewal flow used to call
//! into the main crate's `Database`, `SecretsStore`, `crate::config::helpers`,
//! and `crate::setup::*`. Keeping those calls direct meant `ironclaw_llm`
//! couldn't compile without the rest of the binary. The traits in this
//! module narrow that surface to exactly what the LLM crate needs; the main
//! crate provides adapter impls.

use std::sync::Arc;

use async_trait::async_trait;
use secrecy::SecretString;

use crate::error::LlmError;

/// Subset of a settings table used by `SessionManager` to persist NEAR-AI
/// session tokens (and read them back across restarts).
#[async_trait]
pub trait SessionDb: Send + Sync {
    /// Persist a JSON value under `(user_id, key)`.
    async fn set_setting(
        &self,
        user_id: &str,
        key: &str,
        value: &serde_json::Value,
    ) -> Result<(), String>;

    /// Read a JSON value, returning `Ok(None)` if absent.
    async fn get_setting(
        &self,
        user_id: &str,
        key: &str,
    ) -> Result<Option<serde_json::Value>, String>;
}

/// Subset of an encrypted secrets store used by `SessionManager` to persist
/// NEAR-AI session tokens at rest under user-keyed encryption.
#[async_trait]
pub trait SessionSecrets: Send + Sync {
    /// Create or replace the secret named `name` for `user_id`.
    async fn create(
        &self,
        user_id: &str,
        name: &str,
        value: String,
        provider: Option<&str>,
    ) -> Result<(), String>;

    /// Decrypt the secret named `name` for `user_id`.
    async fn get_decrypted(&self, user_id: &str, name: &str) -> Result<SecretString, String>;
}

/// Hooks the host installs to recover from a session failure.
///
/// The default `NoopRenewer` returns `LlmError::SessionRenewalFailed` for
/// every renewal request, which is appropriate for headless / hosted
/// deployments where the session token is set via env var. CLI builds wire
/// in an interactive impl that drives the OAuth menu in `src/llm_session/`.
#[async_trait]
pub trait SessionRenewer: Send + Sync {
    /// Drive an interactive (or automated) renewal flow.
    ///
    /// The implementation is expected to either:
    /// - call `manager.set_token(...)` and `manager.save_session_for_renewer(...)`
    ///   with a fresh session token, returning `Ok(())`, or
    /// - persist an API key through `SessionKeyPersistor` and return `Ok(())`
    ///   without setting a session token (the caller falls back to API-key
    ///   auth on the next request).
    async fn renew(&self, manager: &super::session::SessionManager) -> Result<(), LlmError>;
}

/// Persistence hooks for one-shot API-key entry: thread-safe runtime env
/// overlay plus best-effort write to the host's `.env` file.
pub trait SessionKeyPersistor: Send + Sync {
    /// Make `value` visible to in-process env lookups for the rest of the
    /// process (thread-safe overlay; no UB from `set_var`).
    fn set_runtime_env(&self, key: &str, value: &str);

    /// Persist `key=value` to the host's bootstrap `.env` so it survives
    /// across restarts. May fail; failure is non-fatal for the caller.
    fn upsert_bootstrap_var(&self, key: &str, value: &str) -> std::io::Result<()>;
}

// Default no-op implementations for headless contexts.

/// A `SessionRenewer` that always fails. Headless deployments use this
/// (the LLM call returns `SessionExpired`, which surfaces to the user).
pub struct NoopSessionRenewer;

#[async_trait]
impl SessionRenewer for NoopSessionRenewer {
    async fn renew(&self, _manager: &super::session::SessionManager) -> Result<(), LlmError> {
        Err(LlmError::SessionRenewalFailed {
            provider: "nearai".to_string(),
            reason: "interactive session renewal is unavailable in this build; \
                 set NEARAI_SESSION_TOKEN or NEARAI_API_KEY env var instead"
                .to_string(),
        })
    }
}

/// A `SessionKeyPersistor` that does nothing.
pub struct NoopKeyPersistor;

impl SessionKeyPersistor for NoopKeyPersistor {
    fn set_runtime_env(&self, _key: &str, _value: &str) {}
    fn upsert_bootstrap_var(&self, _key: &str, _value: &str) -> std::io::Result<()> {
        Ok(())
    }
}

/// Convenience type aliases.
pub type SharedSessionDb = Arc<dyn SessionDb>;
pub type SharedSessionSecrets = Arc<dyn SessionSecrets>;
pub type SharedSessionRenewer = Arc<dyn SessionRenewer>;
pub type SharedSessionKeyPersistor = Arc<dyn SessionKeyPersistor>;
