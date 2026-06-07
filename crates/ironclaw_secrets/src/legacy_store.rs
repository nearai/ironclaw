//! Ported legacy secret storage contracts used by Reborn adapters.
//!
//! These types mirror the existing `src/secrets/{types,store}.rs` behavior:
//! encrypted records, redacted Debug output, decrypted material exposed only via
//! an explicit host-boundary method, and an encrypted in-memory store for tests.

use std::{collections::HashMap, fmt, sync::Arc};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::SecretsCrypto;
use crate::crypto::secret_record_aad;

#[derive(Clone)]
pub struct Secret {
    pub id: Uuid,
    pub user_id: String,
    pub name: String,
    pub encrypted_value: Vec<u8>,
    pub key_salt: Vec<u8>,
    pub provider: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub usage_count: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl fmt::Debug for Secret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Secret")
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretRef {
    pub name: String,
    pub provider: Option<String>,
}

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
            name: name.into().to_lowercase(),
            value: SecretString::from(value.into()),
            provider: None,
            expires_at: None,
        }
    }

    pub fn from_secret(name: impl Into<String>, value: SecretString) -> Self {
        Self {
            name: name.into().to_lowercase(),
            value,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretConsumeResult {
    Matched,
    Mismatched,
    NotFound,
}

#[async_trait]
pub trait SecretsStore: Send + Sync {
    async fn create(
        &self,
        user_id: &str,
        params: CreateSecretParams,
    ) -> Result<Secret, SecretError>;
    async fn get(&self, user_id: &str, name: &str) -> Result<Secret, SecretError>;
    async fn get_decrypted(
        &self,
        user_id: &str,
        name: &str,
    ) -> Result<DecryptedSecret, SecretError>;

    async fn consume_if_matches(
        &self,
        user_id: &str,
        name: &str,
        expected_value: &str,
    ) -> Result<SecretConsumeResult, SecretError> {
        match self.get_decrypted(user_id, name).await {
            Ok(secret) => {
                // Constant-time comparison: AES-GCM authenticated decrypt rules out
                // any direct ciphertext oracle, but `decrypted.expose() != expected`
                // would short-circuit on the first differing byte and leak the
                // plaintext through response-latency timing. `ct_eq` walks the full
                // buffer regardless of where the bytes diverge. See F1 (HIGH —
                // timing oracle) in the 2026-05 audit.
                let matches: bool =
                    ConstantTimeEq::ct_eq(secret.expose().as_bytes(), expected_value.as_bytes())
                        .into();
                if !matches {
                    return Ok(SecretConsumeResult::Mismatched);
                }
                self.delete(user_id, name).await?;
                Ok(SecretConsumeResult::Matched)
            }
            Err(SecretError::NotFound(_)) => Ok(SecretConsumeResult::NotFound),
            Err(error) => Err(error),
        }
    }

    async fn exists(&self, user_id: &str, name: &str) -> Result<bool, SecretError>;
    async fn any_exist(&self) -> Result<bool, SecretError> {
        Ok(false)
    }
    async fn list(&self, user_id: &str) -> Result<Vec<SecretRef>, SecretError>;
    async fn delete(&self, user_id: &str, name: &str) -> Result<bool, SecretError>;
    async fn record_usage(&self, secret_id: Uuid) -> Result<(), SecretError>;
    async fn is_accessible(
        &self,
        user_id: &str,
        secret_name: &str,
        allowed_secrets: &[String],
    ) -> Result<bool, SecretError>;
}

#[derive(Debug)]
pub(crate) struct InMemorySecretsStore {
    secrets: RwLock<HashMap<(String, String), Secret>>,
    crypto: Arc<SecretsCrypto>,
}

impl InMemorySecretsStore {
    pub(crate) fn new(crypto: Arc<SecretsCrypto>) -> Self {
        Self {
            secrets: RwLock::new(HashMap::new()),
            crypto,
        }
    }
}

#[async_trait]
impl SecretsStore for InMemorySecretsStore {
    async fn create(
        &self,
        user_id: &str,
        params: CreateSecretParams,
    ) -> Result<Secret, SecretError> {
        let plaintext = params.value.expose_secret().as_bytes();
        let aad = secret_record_aad(user_id, &params.name);
        let (encrypted_value, key_salt) = self.crypto.encrypt(plaintext, &aad)?;
        let now = Utc::now();
        let secret = Secret {
            id: Uuid::new_v4(),
            user_id: user_id.to_string(),
            name: params.name.clone(),
            encrypted_value,
            key_salt,
            provider: params.provider,
            expires_at: params.expires_at,
            last_used_at: None,
            usage_count: 0,
            created_at: now,
            updated_at: now,
        };
        self.secrets
            .write()
            .await
            .insert((user_id.to_string(), params.name), secret.clone());
        Ok(secret)
    }

    async fn get(&self, user_id: &str, name: &str) -> Result<Secret, SecretError> {
        let name = name.to_lowercase();
        let secret = self
            .secrets
            .read()
            .await
            .get(&(user_id.to_string(), name.clone()))
            .cloned()
            .ok_or_else(|| SecretError::NotFound(name.clone()))?;
        if let Some(expires_at) = secret.expires_at
            && expires_at < Utc::now()
        {
            return Err(SecretError::Expired);
        }
        Ok(secret)
    }

    async fn get_decrypted(
        &self,
        user_id: &str,
        name: &str,
    ) -> Result<DecryptedSecret, SecretError> {
        let secret = self.get(user_id, name).await?;
        let aad = secret_record_aad(user_id, &secret.name);
        self.crypto
            .decrypt(&secret.encrypted_value, &secret.key_salt, &aad)
    }

    async fn consume_if_matches(
        &self,
        user_id: &str,
        name: &str,
        expected_value: &str,
    ) -> Result<SecretConsumeResult, SecretError> {
        let name = name.to_lowercase();
        let mut secrets = self.secrets.write().await;
        let key = (user_id.to_string(), name.clone());
        let Some(secret) = secrets.get(&key).cloned() else {
            return Ok(SecretConsumeResult::NotFound);
        };
        if let Some(expires_at) = secret.expires_at
            && expires_at < Utc::now()
        {
            return Err(SecretError::Expired);
        }
        let aad = secret_record_aad(user_id, &secret.name);
        let decrypted = self
            .crypto
            .decrypt(&secret.encrypted_value, &secret.key_salt, &aad)?;
        // Constant-time comparison — see F1 in the 2026-05 audit and the matching
        // doc comment on the trait default body. `!=` short-circuits and leaks the
        // first differing byte through measurable response-latency variance.
        let matches: bool =
            ConstantTimeEq::ct_eq(decrypted.expose().as_bytes(), expected_value.as_bytes()).into();
        if !matches {
            return Ok(SecretConsumeResult::Mismatched);
        }
        secrets.remove(&key);
        Ok(SecretConsumeResult::Matched)
    }

    async fn exists(&self, user_id: &str, name: &str) -> Result<bool, SecretError> {
        Ok(self
            .secrets
            .read()
            .await
            .contains_key(&(user_id.to_string(), name.to_lowercase())))
    }

    async fn any_exist(&self) -> Result<bool, SecretError> {
        Ok(!self.secrets.read().await.is_empty())
    }

    async fn list(&self, user_id: &str) -> Result<Vec<SecretRef>, SecretError> {
        Ok(self
            .secrets
            .read()
            .await
            .iter()
            .filter(|((stored_user_id, _), _)| stored_user_id == user_id)
            .map(|(_, secret)| SecretRef {
                name: secret.name.clone(),
                provider: secret.provider.clone(),
            })
            .collect())
    }

    async fn delete(&self, user_id: &str, name: &str) -> Result<bool, SecretError> {
        Ok(self
            .secrets
            .write()
            .await
            .remove(&(user_id.to_string(), name.to_lowercase()))
            .is_some())
    }

    async fn record_usage(&self, secret_id: Uuid) -> Result<(), SecretError> {
        let mut secrets = self.secrets.write().await;
        let Some(secret) = secrets.values_mut().find(|secret| secret.id == secret_id) else {
            return Err(SecretError::NotFound(secret_id.to_string()));
        };
        secret.last_used_at = Some(Utc::now());
        secret.usage_count += 1;
        Ok(())
    }

    async fn is_accessible(
        &self,
        user_id: &str,
        secret_name: &str,
        allowed_secrets: &[String],
    ) -> Result<bool, SecretError> {
        let secret_name_lower = secret_name.to_lowercase();
        if !self.exists(user_id, &secret_name_lower).await? {
            return Ok(false);
        }
        for pattern in allowed_secrets {
            let pattern_lower = pattern.to_lowercase();
            if pattern_lower == secret_name_lower {
                return Ok(true);
            }
            if let Some(prefix) = pattern_lower.strip_suffix('*')
                && secret_name_lower.starts_with(prefix)
            {
                return Ok(true);
            }
        }
        Ok(false)
    }
}
