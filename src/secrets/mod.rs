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

use std::sync::{Arc, OnceLock, RwLock};

pub use crypto::SecretsCrypto;
#[cfg(feature = "libsql")]
pub use store::LibSqlSecretsStore;
#[cfg(feature = "postgres")]
pub use store::PostgresSecretsStore;
pub use store::SecretsStore;
pub use types::{
    CreateSecretParams, CredentialLocation, CredentialMapping, DecryptedSecret, Secret,
    SecretError, SecretRef,
};

pub use store::in_memory::InMemorySecretsStore;

static GLOBAL_SECRETS_STORE: OnceLock<RwLock<Option<Arc<dyn SecretsStore + Send + Sync>>>> =
    OnceLock::new();

/// Set the process-wide secrets store used by shell credential injection.
///
/// This is intentionally optional so the rest of the application can run
/// without secrets support in environments that do not configure a store.
pub fn set_global_store(store: Option<Arc<dyn SecretsStore + Send + Sync>>) {
    if let Ok(mut guard) = GLOBAL_SECRETS_STORE
        .get_or_init(|| RwLock::new(None))
        .write()
    {
        *guard = store;
    }
}

/// Get the process-wide secrets store used by shell credential injection.
pub fn global_store() -> Option<Arc<dyn SecretsStore + Send + Sync>> {
    GLOBAL_SECRETS_STORE.get().and_then(|store| {
        store
            .read()
            .ok()
            .and_then(|guard| guard.as_ref().map(Arc::clone))
    })
}

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

    set_global_store(store.as_ref().map(Arc::clone));

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

#[cfg(test)]
mod tests;
