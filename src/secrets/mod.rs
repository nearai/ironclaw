//! Secrets management for secure credential storage and injection.
//!
//! This module provides:
//! - AES-256-GCM encrypted secret storage
//! - Per-secret key derivation (HKDF-SHA256)
//! - PostgreSQL persistence
//! - OS keychain integration for master key
//! - Access control for WASM tools
//!
//! # Security Model
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────────────────┐
//! │                              Secret Lifecycle                                │
//! │                                                                              │
//! │   User stores secret ──► Encrypt with AES-256-GCM ──► Store in PostgreSQL  │
//! │                          (per-secret key via HKDF)                          │
//! │                                                                              │
//! │   WASM requests HTTP ──► Host checks allowlist ──► Decrypt secret ──►       │
//! │                          & allowed_secrets        (in memory only)           │
//! │                                                         │                    │
//! │                                                         ▼                    │
//! │                          Inject into request ──► Execute HTTP call          │
//! │                          (WASM never sees value)                            │
//! │                                                         │                    │
//! │                                                         ▼                    │
//! │                          Leak detector scans ──► Return response to WASM   │
//! │                          response for secrets                               │
//! └─────────────────────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Master Key Storage
//!
//! The master key for encrypting secrets can come from:
//! - **OS Keychain** (recommended for local installs): Auto-generated and stored securely
//! - **Environment variable** (for CI/Docker): Set `SECRETS_MASTER_KEY`
//!
//! # Example
//!
//! ```ignore
//! use ironclaw::secrets::{SecretsStore, PostgresSecretsStore, SecretsCrypto, CreateSecretParams};
//! use secrecy::SecretString;
//!
//! // Initialize crypto with master key from environment
//! let master_key = SecretString::from(std::env::var("SECRETS_MASTER_KEY")?);
//! let crypto = Arc::new(SecretsCrypto::new(master_key)?);
//!
//! // Create store
//! let store = PostgresSecretsStore::new(pool, crypto);
//!
//! // Store a secret
//! store.create("user_123", CreateSecretParams::new("openai_key", "sk-...")).await?;
//!
//! // Check if secret exists (WASM can call this)
//! let exists = store.exists("user_123", "openai_key").await?;
//!
//! // Decrypt for injection (host boundary only)
//! let decrypted = store.get_decrypted("user_123", "openai_key").await?;
//! ```

mod crypto;
pub mod keychain;
mod store;
mod types;

pub use crypto::SecretsCrypto;
#[cfg(feature = "libsql")]
pub use store::LibSqlSecretsStore;
#[cfg(feature = "postgres")]
pub use store::PostgresSecretsStore;
pub use store::{SecretConsumeResult, SecretsStore};
pub use types::{
    CreateSecretParams, CredentialLocation, CredentialMapping, DecryptedSecret, Secret,
    SecretError, SecretRef,
};

pub use store::in_memory::InMemorySecretsStore;

/// Create a secrets store from a master key and database handles.
///
/// Returns `None` if no matching backend handle is available (e.g. when
/// running without a database). This is a normal condition in no-db mode,
/// not an error — callers should treat `None` as "secrets unavailable".
pub fn create_secrets_store(
    crypto: std::sync::Arc<SecretsCrypto>,
    handles: &crate::db::DatabaseHandles,
) -> Option<std::sync::Arc<dyn SecretsStore + Send + Sync>> {
    let store: Option<std::sync::Arc<dyn SecretsStore + Send + Sync>> = None;

    #[cfg(feature = "libsql")]
    let store = store.or_else(|| {
        handles.libsql_db.as_ref().map(|db| {
            std::sync::Arc::new(LibSqlSecretsStore::new(
                std::sync::Arc::clone(db),
                std::sync::Arc::clone(&crypto),
            )) as std::sync::Arc<dyn SecretsStore + Send + Sync>
        })
    });

    #[cfg(feature = "postgres")]
    let store = store.or_else(|| {
        handles.pg_pool.as_ref().map(|pool| {
            std::sync::Arc::new(PostgresSecretsStore::new(
                pool.clone(),
                std::sync::Arc::clone(&crypto),
            )) as std::sync::Arc<dyn SecretsStore + Send + Sync>
        })
    });

    store
}

/// Try to resolve an existing master key from env var or OS keychain.
///
/// Resolution order:
/// 1. `SECRETS_MASTER_KEY` environment variable (hex-encoded)
/// 2. OS keychain (macOS Keychain / Linux secret-service)
///
/// Returns `None` if no key is available (caller should generate one).
pub async fn resolve_master_key() -> Option<String> {
    // 1. Check env var
    if let Ok(env_key) = std::env::var("SECRETS_MASTER_KEY")
        && !env_key.is_empty()
    {
        return Some(env_key);
    }

    // 2. Try OS keychain
    if let Ok(keychain_key_bytes) = keychain::get_master_key().await {
        let key_hex: String = keychain_key_bytes
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect();
        return Some(key_hex);
    }

    None
}

/// Create a `SecretsCrypto` from a master key string.
///
/// The key is typically hex-encoded (from `generate_master_key_hex` or
/// the `SECRETS_MASTER_KEY` env var), but `SecretsCrypto::new` validates
/// only key length, not encoding. Any sufficiently long string works.
pub fn crypto_from_hex(hex: &str) -> Result<std::sync::Arc<SecretsCrypto>, SecretError> {
    let crypto = SecretsCrypto::new(secrecy::SecretString::from(hex.to_string()))?;
    Ok(std::sync::Arc::new(crypto))
}

/// Outcome of [`verify_generated_key_safe`].
///
/// Separate from `Result<(), E>` so the probe-failure case surfaces
/// as an error the caller is forced to handle, not as a silent
/// `Ok(())`. Fail-closed: an unreadable store is treated as a hard
/// stop, because we cannot rule out the stale-data hazard the gate
/// exists to catch.
#[derive(Debug, thiserror::Error)]
pub enum GeneratedKeySafetyError {
    /// A freshly-generated master key met a secrets store that
    /// already contained encrypted data. Those rows were encrypted
    /// with a previous key, so the new key cannot decrypt them and
    /// continuing would shadow unrecoverable data.
    #[error(
        "Secrets store already contains encrypted data, but IronClaw auto-generated \
         a new master key because no SECRETS_MASTER_KEY env var and no OS-keychain \
         entry were available. The existing rows were encrypted with a different key \
         and cannot be decrypted. Restore the original key (set SECRETS_MASTER_KEY or \
         re-populate the keychain entry) before restarting. If the existing data is \
         expendable, clear the `secrets` table first."
    )]
    StoreAlreadyPopulated,

    /// The safety probe itself failed. Treated as fail-closed: we
    /// can't confirm the store is safe to write to, so we refuse to
    /// proceed rather than silently risk shadowing data.
    #[error(
        "Unable to verify secrets store state during post-auto-generate safety \
         check: {0}. Refusing to proceed — re-run once the store is reachable."
    )]
    ProbeFailed(SecretError),
}

/// Startup safety gate: if this process auto-generated a fresh master
/// key (because no `SECRETS_MASTER_KEY` env var, no keychain entry,
/// and no concurrent writer supplied one) but the secrets store
/// already holds encrypted rows, those rows were encrypted with a
/// previous key and cannot be decrypted with the new one. Continuing
/// would layer new writes on top of unrecoverable data.
///
/// `generated = false` short-circuits: a key sourced from env,
/// keychain, or a concurrent-writer TOCTOU reuse is already
/// consistent with the stored rows (or the store is empty and the
/// point is moot).
///
/// Fail-closed on probe error — see [`GeneratedKeySafetyError`].
pub async fn verify_generated_key_safe(
    generated: bool,
    store: &(dyn SecretsStore + Send + Sync),
) -> Result<(), GeneratedKeySafetyError> {
    if !generated {
        return Ok(());
    }
    match store.any_exist().await {
        Ok(false) => Ok(()),
        Ok(true) => Err(GeneratedKeySafetyError::StoreAlreadyPopulated),
        Err(e) => Err(GeneratedKeySafetyError::ProbeFailed(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::types::CreateSecretParams;
    use crate::testing::credentials::{TEST_SECRET_VALUE, test_secrets_store};

    #[test]
    fn test_crypto_from_hex_valid() {
        // 32 bytes = 64 hex chars
        let hex = "0123456789abcdef".repeat(4); // 64 hex chars
        let result = crypto_from_hex(&hex);
        assert!(result.is_ok()); // safety: test assertion
    }

    #[test]
    fn test_crypto_from_hex_invalid() {
        let result = crypto_from_hex("too_short");
        assert!(result.is_err()); // safety: test assertion
    }

    /// `generated = false` always short-circuits — a key that came
    /// from env, keychain, or a TOCTOU-reused sibling is consistent
    /// with whatever is in the store.
    #[tokio::test]
    async fn verify_generated_key_safe_allows_non_generated_key() {
        let store = test_secrets_store();
        store
            .create("u", CreateSecretParams::new("k", TEST_SECRET_VALUE))
            .await
            .unwrap();

        verify_generated_key_safe(false, &store)
            .await
            .expect("non-generated key must always pass, even against a populated store");
    }

    /// Happy path for a first-time install: freshly generated key
    /// meets an empty store. Must proceed.
    #[tokio::test]
    async fn verify_generated_key_safe_allows_generated_key_with_empty_store() {
        let store = test_secrets_store();

        verify_generated_key_safe(true, &store)
            .await
            .expect("generated key on empty store is the first-install happy path");
    }

    /// Headline behavior: freshly generated key meets a populated
    /// store. Must abort — those rows were encrypted with a different
    /// key and silently continuing would shadow unrecoverable data.
    #[tokio::test]
    async fn verify_generated_key_safe_blocks_generated_key_against_populated_store() {
        let store = test_secrets_store();
        store
            .create("u", CreateSecretParams::new("k", TEST_SECRET_VALUE))
            .await
            .unwrap();

        let err = verify_generated_key_safe(true, &store)
            .await
            .expect_err("generated key + populated store must fail loudly");

        assert!(
            matches!(err, GeneratedKeySafetyError::StoreAlreadyPopulated),
            "wrong variant: {err:?}"
        );
        let msg = err.to_string();
        assert!(
            msg.contains("SECRETS_MASTER_KEY"),
            "error must name the env var the user can restore; got: {msg}"
        );
        assert!(
            msg.contains("secrets"),
            "error must reference the store / table the user can clear; got: {msg}"
        );
    }

    /// Fail-closed on probe failure: a store whose `any_exist`
    /// returns an error must abort startup rather than silently
    /// proceed. Otherwise a transient DB hiccup would let us skip
    /// the check and the next write could shadow stale rows once
    /// the DB recovers.
    #[tokio::test]
    async fn verify_generated_key_safe_fails_closed_on_probe_error() {
        use crate::secrets::types::DecryptedSecret;
        use async_trait::async_trait;

        struct ErroringStore;

        #[async_trait]
        impl SecretsStore for ErroringStore {
            async fn create(
                &self,
                _: &str,
                _: CreateSecretParams,
            ) -> Result<crate::secrets::types::Secret, SecretError> {
                unreachable!()
            }
            async fn get(
                &self,
                _: &str,
                _: &str,
            ) -> Result<crate::secrets::types::Secret, SecretError> {
                unreachable!()
            }
            async fn get_decrypted(
                &self,
                _: &str,
                _: &str,
            ) -> Result<DecryptedSecret, SecretError> {
                unreachable!()
            }
            async fn exists(&self, _: &str, _: &str) -> Result<bool, SecretError> {
                unreachable!()
            }
            async fn any_exist(&self) -> Result<bool, SecretError> {
                Err(SecretError::Database("simulated probe failure".into()))
            }
            async fn list(
                &self,
                _: &str,
            ) -> Result<Vec<crate::secrets::types::SecretRef>, SecretError> {
                unreachable!()
            }
            async fn delete(&self, _: &str, _: &str) -> Result<bool, SecretError> {
                unreachable!()
            }
            async fn record_usage(&self, _: uuid::Uuid) -> Result<(), SecretError> {
                unreachable!()
            }
            async fn is_accessible(
                &self,
                _: &str,
                _: &str,
                _: &[String],
            ) -> Result<bool, SecretError> {
                unreachable!()
            }
        }

        let err = verify_generated_key_safe(true, &ErroringStore)
            .await
            .expect_err("probe error must not be swallowed as `safe`");
        assert!(
            matches!(err, GeneratedKeySafetyError::ProbeFailed(_)),
            "wrong variant: {err:?}"
        );

        // `generated = false` must still short-circuit without invoking
        // the probe — otherwise a probe-broken store would fail every
        // startup, even when the user has a known-good env-var key.
        verify_generated_key_safe(false, &ErroringStore)
            .await
            .expect("generated=false must never call the probe");
    }
}
