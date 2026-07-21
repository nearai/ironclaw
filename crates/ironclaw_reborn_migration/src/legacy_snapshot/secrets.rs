//! Frozen port of the v1 secrets store (`src/secrets/{crypto,store,types}.rs`)
//! — read-only (list/get/decrypt), just enough for [`crate::convert::secrets`]
//! to decrypt v1 secrets and re-encrypt them through Reborn's secret store.
//!
//! The AES-256-GCM + HKDF-SHA256 scheme below MUST stay byte-for-byte
//! identical to v1's, or previously-encrypted secrets fail to decrypt. Ported
//! verbatim from `src/secrets/crypto.rs` — do not "simplify" this file.

use std::sync::Arc;

use aes_gcm::aead::Aead;
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use chrono::{DateTime, Utc};
use hkdf::Hkdf;
use secrecy::{ExposeSecret, SecretString};
use sha2::Sha256;

use super::connect::LegacyHandles;

const KEY_SIZE: usize = 32;
const NONCE_SIZE: usize = 12;
const TAG_SIZE: usize = 16;

#[derive(Debug, Clone, thiserror::Error)]
pub(crate) enum SecretError {
    #[error("Secret not found: {0}")]
    NotFound(String),
    #[error("Secret has expired")]
    Expired,
    #[error("Decryption failed: {0}")]
    DecryptionFailed(String),
    #[error("Invalid master key")]
    InvalidMasterKey,
    #[error("Secret value is not valid UTF-8")]
    InvalidUtf8,
    #[error("Database error: {0}")]
    Database(String),
}

/// Frozen mirror of `ironclaw::secrets::SecretsCrypto`.
pub(crate) struct SecretsCrypto {
    master_key: SecretString,
}

impl SecretsCrypto {
    pub(crate) fn new(master_key: SecretString) -> Result<Self, SecretError> {
        if master_key.expose_secret().len() < KEY_SIZE {
            return Err(SecretError::InvalidMasterKey);
        }
        Ok(Self { master_key })
    }

    pub(crate) fn decrypt(
        &self,
        encrypted_value: &[u8],
        salt: &[u8],
    ) -> Result<DecryptedSecret, SecretError> {
        if encrypted_value.len() < NONCE_SIZE + TAG_SIZE {
            return Err(SecretError::DecryptionFailed(
                "Encrypted value too short".to_string(),
            ));
        }
        let derived_key = self.derive_key(salt)?;
        let cipher = Aes256Gcm::new_from_slice(&derived_key)
            .map_err(|e| SecretError::DecryptionFailed(format!("Failed to create cipher: {e}")))?;
        let (nonce_bytes, ciphertext) = encrypted_value.split_at(NONCE_SIZE);
        let nonce = Nonce::from_slice(nonce_bytes);
        let plaintext = cipher
            .decrypt(nonce, ciphertext)
            .map_err(|e| SecretError::DecryptionFailed(format!("Decryption failed: {e}")))?;
        DecryptedSecret::from_bytes(plaintext)
    }

    fn derive_key(&self, salt: &[u8]) -> Result<[u8; KEY_SIZE], SecretError> {
        let master_bytes = self.master_key.expose_secret().as_bytes();
        let hk = Hkdf::<Sha256>::new(Some(salt), master_bytes);
        let mut derived = [0u8; KEY_SIZE];
        hk.expand(b"near-agent-secrets-v1", &mut derived)
            .map_err(|_| SecretError::DecryptionFailed("HKDF expansion failed".to_string()))?;
        Ok(derived)
    }
}

/// Frozen mirror of `ironclaw::secrets::types::Secret`.
pub(crate) struct Secret {
    pub(crate) encrypted_value: Vec<u8>,
    pub(crate) key_salt: Vec<u8>,
    pub(crate) expires_at: Option<DateTime<Utc>>,
}

/// Frozen mirror of `ironclaw::secrets::types::SecretRef`.
pub(crate) struct SecretRef {
    pub(crate) name: String,
}

/// Frozen mirror of `ironclaw::secrets::types::DecryptedSecret`.
pub(crate) struct DecryptedSecret {
    value: SecretString,
}

impl DecryptedSecret {
    fn from_bytes(bytes: Vec<u8>) -> Result<Self, SecretError> {
        let s = String::from_utf8(bytes).map_err(|_| SecretError::InvalidUtf8)?;
        Ok(Self {
            value: SecretString::from(s),
        })
    }

    pub(crate) fn expose(&self) -> &str {
        self.value.expose_secret()
    }
}

/// Frozen mirror of `ironclaw::secrets::create_secrets_store` — returns `None`
/// (not an error) when no backend handle is available, same as the original.
pub(crate) fn create_secrets_store(
    crypto: Arc<SecretsCrypto>,
    handles: &LegacyHandles,
) -> Option<LegacySecretsSource> {
    if let Some(db) = handles.libsql_db.as_ref() {
        return Some(LegacySecretsSource::LibSql(Arc::clone(db), crypto));
    }
    if let Some(pool) = handles.pg_pool.as_ref() {
        return Some(LegacySecretsSource::Postgres(pool.clone(), crypto));
    }
    None
}

/// Frozen, read-only mirror of `ironclaw::secrets::SecretsStore` — only the
/// `list`/`get`/`get_decrypted` methods `convert::secrets` calls.
pub(crate) enum LegacySecretsSource {
    LibSql(Arc<libsql::Database>, Arc<SecretsCrypto>),
    Postgres(deadpool_postgres::Pool, Arc<SecretsCrypto>),
}

impl LegacySecretsSource {
    fn crypto(&self) -> &SecretsCrypto {
        match self {
            LegacySecretsSource::LibSql(_, crypto) => crypto,
            LegacySecretsSource::Postgres(_, crypto) => crypto,
        }
    }

    pub(crate) async fn list(&self, user_id: &str) -> Result<Vec<SecretRef>, SecretError> {
        match self {
            LegacySecretsSource::LibSql(db, _) => libsql_list(db, user_id).await,
            LegacySecretsSource::Postgres(pool, _) => postgres_list(pool, user_id).await,
        }
    }

    pub(crate) async fn get(&self, user_id: &str, name: &str) -> Result<Secret, SecretError> {
        let name = name.to_lowercase();
        let secret = match self {
            LegacySecretsSource::LibSql(db, _) => libsql_get(db, user_id, &name).await?,
            LegacySecretsSource::Postgres(pool, _) => postgres_get(pool, user_id, &name).await?,
        };
        if let Some(expires_at) = secret.expires_at
            && expires_at < Utc::now()
        {
            return Err(SecretError::Expired);
        }
        Ok(secret)
    }

    pub(crate) async fn get_decrypted(
        &self,
        user_id: &str,
        name: &str,
    ) -> Result<DecryptedSecret, SecretError> {
        let secret = self.get(user_id, name).await?;
        self.crypto()
            .decrypt(&secret.encrypted_value, &secret.key_salt)
    }
}

const SECRET_COLUMNS: &str = "id, user_id, name, encrypted_value, key_salt, provider, expires_at, \
     last_used_at, usage_count, created_at, updated_at";

async fn libsql_connect(db: &Arc<libsql::Database>) -> Result<libsql::Connection, SecretError> {
    let conn = db
        .connect()
        .map_err(|e| SecretError::Database(format!("Connection failed: {e}")))?;
    conn.query("PRAGMA busy_timeout = 5000", ())
        .await
        .map_err(|e| SecretError::Database(format!("Failed to set busy_timeout: {e}")))?;
    Ok(conn)
}

async fn libsql_list(
    db: &Arc<libsql::Database>,
    user_id: &str,
) -> Result<Vec<SecretRef>, SecretError> {
    let conn = libsql_connect(db).await?;
    let mut rows = conn
        .query(
            "SELECT name, provider FROM secrets WHERE user_id = ?1 ORDER BY name",
            libsql::params![user_id],
        )
        .await
        .map_err(|e| SecretError::Database(e.to_string()))?;
    let mut refs = Vec::new();
    while let Some(row) = rows
        .next()
        .await
        .map_err(|e| SecretError::Database(e.to_string()))?
    {
        refs.push(SecretRef {
            name: row.get::<String>(0).unwrap_or_default(),
        });
    }
    Ok(refs)
}

async fn libsql_get(
    db: &Arc<libsql::Database>,
    user_id: &str,
    name: &str,
) -> Result<Secret, SecretError> {
    let conn = libsql_connect(db).await?;
    let mut rows = conn
        .query(
            &format!("SELECT {SECRET_COLUMNS} FROM secrets WHERE user_id = ?1 AND name = ?2"),
            libsql::params![user_id, name],
        )
        .await
        .map_err(|e| SecretError::Database(e.to_string()))?;
    match rows
        .next()
        .await
        .map_err(|e| SecretError::Database(e.to_string()))?
    {
        Some(row) => libsql_row_to_secret(&row),
        None => Err(SecretError::NotFound(name.to_string())),
    }
}

fn libsql_row_to_secret(row: &libsql::Row) -> Result<Secret, SecretError> {
    use super::libsql_helpers::parse_timestamp;

    let encrypted_value: Vec<u8> = row
        .get(3)
        .map_err(|e| SecretError::Database(e.to_string()))?;
    let key_salt: Vec<u8> = row
        .get(4)
        .map_err(|e| SecretError::Database(e.to_string()))?;
    let expires_at = row
        .get::<String>(6)
        .ok()
        .filter(|s| !s.is_empty())
        .and_then(|s| parse_timestamp(&s).ok());
    Ok(Secret {
        encrypted_value,
        key_salt,
        expires_at,
    })
}

async fn postgres_list(
    pool: &deadpool_postgres::Pool,
    user_id: &str,
) -> Result<Vec<SecretRef>, SecretError> {
    let client = pool
        .get()
        .await
        .map_err(|e| SecretError::Database(e.to_string()))?;
    let rows = client
        .query(
            "SELECT name, provider FROM secrets WHERE user_id = $1 ORDER BY name",
            &[&user_id],
        )
        .await
        .map_err(|e| SecretError::Database(e.to_string()))?;
    Ok(rows
        .iter()
        .map(|r| SecretRef {
            name: r.get("name"),
        })
        .collect())
}

async fn postgres_get(
    pool: &deadpool_postgres::Pool,
    user_id: &str,
    name: &str,
) -> Result<Secret, SecretError> {
    let client = pool
        .get()
        .await
        .map_err(|e| SecretError::Database(e.to_string()))?;
    let row = client
        .query_opt(
            &format!("SELECT {SECRET_COLUMNS} FROM secrets WHERE user_id = $1 AND name = $2"),
            &[&user_id, &name],
        )
        .await
        .map_err(|e| SecretError::Database(e.to_string()))?;
    match row {
        Some(r) => Ok(Secret {
            encrypted_value: r.get("encrypted_value"),
            key_salt: r.get("key_salt"),
            expires_at: r.get("expires_at"),
        }),
        None => Err(SecretError::NotFound(name.to_string())),
    }
}
