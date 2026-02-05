//! Secret types for credential management.
//!
//! WASM tools NEVER see plaintext secrets. This module provides types
//! for secure storage and reference without exposing actual values.

use std::fmt;

use chrono::{DateTime, Utc};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A stored secret with encrypted value.
///
/// The plaintext is never stored; only the encrypted form exists in the database.
#[derive(Clone)]
pub struct Secret {
    pub id: Uuid,
    pub user_id: String,
    pub name: String,
    /// AES-256-GCM encrypted value (nonce || ciphertext || tag).
    pub encrypted_value: Vec<u8>,
    /// Per-secret salt for key derivation.
    pub key_salt: Vec<u8>,
    /// Optional provider hint (e.g., "openai", "stripe").
    pub provider: Option<String>,
    /// When this secret expires (None = never).
    pub expires_at: Option<DateTime<Utc>>,
    /// Last time this secret was used for injection.
    pub last_used_at: Option<DateTime<Utc>>,
    /// Total number of times this secret has been used.
    pub usage_count: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Secret")
            .field("id", &self.id)
            .field("user_id", &self.user_id)
            .field("name", &self.name)
            .field("encrypted_value", &"[REDACTED]")
            .field("key_salt", &"[REDACTED]")
            .field("provider", &self.provider)
            .field("expires_at", &self.expires_at)
            .field("last_used_at", &self.last_used_at)
            .field("usage_count", &self.usage_count)
            .finish()
    }
}

/// A reference to a secret by name, without exposing the value.
///
/// WASM tools receive these references and can check if secrets exist,
/// but they cannot read the actual values.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretRef {
    pub name: String,
    pub provider: Option<String>,
}

impl SecretRef {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            provider: None,
        }
    }

    pub fn with_provider(mut self, provider: impl Into<String>) -> Self {
        self.provider = Some(provider.into());
        self
    }
}

/// A decrypted secret value, held in secure memory.
///
/// This type:
/// - Zeros memory on drop
/// - Never appears in Debug output
/// - Only exists briefly during credential injection
pub struct DecryptedSecret {
    value: SecretString,
}

impl DecryptedSecret {
    /// Create a new decrypted secret from raw bytes.
    ///
    /// The bytes are converted to a UTF-8 string. For binary secrets,
    /// consider base64 encoding before storage.
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, SecretError> {
        // Convert to string, then wrap in SecretString
        let s = String::from_utf8(bytes).map_err(|_| SecretError::InvalidUtf8)?;
        Ok(Self {
            value: SecretString::from(s),
        })
    }

    /// Expose the secret value for injection.
    ///
    /// This is the ONLY way to access the plaintext. Use sparingly
    /// and ensure the exposed value isn't logged or persisted.
    pub fn expose(&self) -> &str {
        self.value.expose_secret()
    }

    /// Get the length of the secret without exposing it.
    pub fn len(&self) -> usize {
        self.value.expose_secret().len()
    }

    /// Check if the secret is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl fmt::Debug for DecryptedSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DecryptedSecret([REDACTED, {} bytes])", self.len())
    }
}

impl Clone for DecryptedSecret {
    fn clone(&self) -> Self {
        Self {
            value: SecretString::from(self.value.expose_secret().to_string()),
        }
    }
}

/// Errors that can occur during secret operations.
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

/// Parameters for creating a new secret.
#[derive(Debug)]
pub struct CreateSecretParams {
    pub name: String,
    pub value: SecretString,
    pub provider: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
}

impl CreateSecretParams {
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value: SecretString::from(value.into()),
            provider: None,
            expires_at: None,
        }
    }

    pub fn with_provider(mut self, provider: impl Into<String>) -> Self {
        self.provider = Some(provider.into());
        self
    }

    pub fn with_expiry(mut self, expires_at: DateTime<Utc>) -> Self {
        self.expires_at = Some(expires_at);
        self
    }
}

/// Where a credential should be injected in an HTTP request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CredentialLocation {
    /// Inject as Authorization header (e.g., "Bearer {secret}")
    AuthorizationBearer,
    /// Inject as Authorization header with Basic auth
    AuthorizationBasic { username: String },
    /// Inject as a custom header
    Header {
        name: String,
        prefix: Option<String>,
    },
    /// Inject as a query parameter
    QueryParam { name: String },
}

impl Default for CredentialLocation {
    fn default() -> Self {
        Self::AuthorizationBearer
    }
}

/// Mapping from a secret name to where it should be injected.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CredentialMapping {
    /// Name of the secret to use.
    pub secret_name: String,
    /// Where to inject the credential.
    pub location: CredentialLocation,
    /// Host patterns this credential applies to (glob syntax).
    pub host_patterns: Vec<String>,
}

impl CredentialMapping {
    pub fn bearer(secret_name: impl Into<String>, host_pattern: impl Into<String>) -> Self {
        Self {
            secret_name: secret_name.into(),
            location: CredentialLocation::AuthorizationBearer,
            host_patterns: vec![host_pattern.into()],
        }
    }

    pub fn header(
        secret_name: impl Into<String>,
        header_name: impl Into<String>,
        host_pattern: impl Into<String>,
    ) -> Self {
        Self {
            secret_name: secret_name.into(),
            location: CredentialLocation::Header {
                name: header_name.into(),
                prefix: None,
            },
            host_patterns: vec![host_pattern.into()],
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::secrets::types::{CreateSecretParams, DecryptedSecret, SecretRef};

    #[test]
    fn test_secret_ref_creation() {
        let r = SecretRef::new("my_api_key").with_provider("openai");
        assert_eq!(r.name, "my_api_key");
        assert_eq!(r.provider, Some("openai".to_string()));
    }

    #[test]
    fn test_decrypted_secret_redaction() {
        let secret = DecryptedSecret::from_bytes(b"super_secret_value".to_vec()).unwrap();
        let debug_str = format!("{:?}", secret);
        assert!(!debug_str.contains("super_secret_value"));
        assert!(debug_str.contains("REDACTED"));
    }

    #[test]
    fn test_decrypted_secret_expose() {
        let secret = DecryptedSecret::from_bytes(b"test_value".to_vec()).unwrap();
        assert_eq!(secret.expose(), "test_value");
        assert_eq!(secret.len(), 10);
    }

    #[test]
    fn test_create_params() {
        let params = CreateSecretParams::new("key", "value").with_provider("stripe");
        assert_eq!(params.name, "key");
        assert_eq!(params.provider, Some("stripe".to_string()));
    }
}
