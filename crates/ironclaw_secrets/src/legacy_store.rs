//! Crypto-facing error and decrypted-material types ported from v1.
//!
//! These types mirror the existing `src/secrets/{types,store}.rs` behavior:
//! redacted Debug output and decrypted material exposed only via an explicit
//! host-boundary method. The legacy `SecretsStore` trait and its encrypted
//! in-memory engine were removed with the §4.3 secrets-cluster consolidation
//! (`docs/reborn/2026-07-17-architecture-simplification-dto-dyn-local.md`);
//! volatile stores now use `SecretStore::ephemeral()`.

use std::fmt;

use secrecy::{ExposeSecret, SecretString};

pub struct DecryptedSecret {
    value: SecretString,
}

impl DecryptedSecret {
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, SecretError> {
        let value = String::from_utf8(bytes).map_err(|_| SecretError::InvalidUtf8)?;
        Ok(Self {
            value: SecretString::from(value),
        })
    }

    pub fn expose(&self) -> &str {
        self.value.expose_secret()
    }

    pub fn len(&self) -> usize {
        self.value.expose_secret().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl fmt::Debug for DecryptedSecret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "DecryptedSecret([REDACTED, {} bytes])",
            self.len()
        )
    }
}

impl Clone for DecryptedSecret {
    fn clone(&self) -> Self {
        Self {
            value: SecretString::from(self.value.expose_secret().to_string()),
        }
    }
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum SecretError {
    #[error("Secret not found: {0}")]
    NotFound(String),
    #[error("Secret has expired")]
    Expired,
    #[error("Decryption failed: {0}")]
    DecryptionFailed(String),
    #[error("Encryption failed: {0}")]
    EncryptionFailed(String),
    #[error("Invalid master key")]
    InvalidMasterKey,
    #[error("Secret value is not valid UTF-8")]
    InvalidUtf8,
    #[error("Database error: {0}")]
    Database(String),
    #[error("Secret access denied for tool")]
    AccessDenied,
    #[error("Keychain error: {0}")]
    KeychainError(String),
}
