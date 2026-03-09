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
pub use store::SecretsStore;
pub use types::{
    CreateSecretParams, CredentialLocation, CredentialMapping, DecryptedSecret, Secret,
    SecretError, SecretRef,
};

pub use store::in_memory::InMemorySecretsStore;

/// Create a secrets store from a master key and database handles.
///
/// Returns `None` if no matching backend handle is available.
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
