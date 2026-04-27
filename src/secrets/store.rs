//! Secret storage with PostgreSQL persistence.
//!
//! Provides CRUD operations for encrypted secrets. The store handles:
//! - Encryption/decryption via SecretsCrypto
//! - Expiration checking
//! - Usage tracking
//! - Access control (which secrets a tool can use)

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
#[cfg(feature = "postgres")]
use deadpool_postgres::Pool;
use secrecy::ExposeSecret;
use uuid::Uuid;

use crate::secrets::crypto::SecretsCrypto;
use crate::secrets::types::{CreateSecretParams, DecryptedSecret, Secret, SecretError, SecretRef};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretConsumeResult {
    Matched,
    Mismatched,
    NotFound,
}

/// Trait for secret storage operations.
///
/// Allows for different implementations (PostgreSQL, in-memory for testing).
#[async_trait]
pub trait SecretsStore: Send + Sync {
    /// Store a new secret.
    async fn create(
        &self,
        user_id: &str,
        params: CreateSecretParams,
    ) -> Result<Secret, SecretError>;

    /// Get a secret by name (encrypted form).
    async fn get(&self, user_id: &str, name: &str) -> Result<Secret, SecretError>;

    /// Get and decrypt a secret.
    async fn get_decrypted(
        &self,
        user_id: &str,
        name: &str,
    ) -> Result<DecryptedSecret, SecretError>;

    /// Atomically consume a secret only when its decrypted plaintext matches
    /// the provided expected value.
    ///
    /// Backends should override this to eliminate read-then-delete races.
    async fn consume_if_matches(
        &self,
        user_id: &str,
        name: &str,
        expected_value: &str,
    ) -> Result<SecretConsumeResult, SecretError> {
        match self.get_decrypted(user_id, name).await {
            Ok(secret) => {
                if secret.expose() != expected_value {
                    return Ok(SecretConsumeResult::Mismatched);
                }
                self.delete(user_id, name).await?;
                Ok(SecretConsumeResult::Matched)
            }
            Err(SecretError::NotFound(_)) => Ok(SecretConsumeResult::NotFound),
            Err(e) => Err(e),
        }
    }

    /// Check if a secret exists.
    async fn exists(&self, user_id: &str, name: &str) -> Result<bool, SecretError>;

    /// Returns `true` if the store contains at least one secret across
    /// all users.
    ///
    /// Used as a startup safety gate: if IronClaw auto-generated a
    /// fresh master key because neither `SECRETS_MASTER_KEY` nor a
    /// keychain entry was available, but the secrets table is already
    /// populated, those rows were encrypted with a different key and
    /// cannot be decrypted. Failing loudly beats silently shadowing
    /// unrecoverable data.
    ///
    /// Default implementation returns `Ok(false)` so test doubles and
    /// ephemeral stores don't need to implement it. Durable backends
    /// (PostgreSQL, libSQL) MUST override.
    async fn any_exist(&self) -> Result<bool, SecretError> {
        Ok(false)
    }

    /// List all secret references for a user (no values).
    async fn list(&self, user_id: &str) -> Result<Vec<SecretRef>, SecretError>;

    /// Delete a secret.
    async fn delete(&self, user_id: &str, name: &str) -> Result<bool, SecretError>;

    /// Update secret usage tracking.
    async fn record_usage(&self, secret_id: Uuid) -> Result<(), SecretError>;

    /// Check if a secret is accessible by a tool (based on allowed_secrets).
    async fn is_accessible(
        &self,
        user_id: &str,
        secret_name: &str,
        allowed_secrets: &[String],
    ) -> Result<bool, SecretError>;
}

/// PostgreSQL implementation of SecretsStore.
#[cfg(feature = "postgres")]
pub struct PostgresSecretsStore {
    pool: Pool,
    crypto: Arc<SecretsCrypto>,
}

#[cfg(feature = "postgres")]
impl PostgresSecretsStore {
    /// Create a new store with the given database pool and crypto instance.
    pub fn new(pool: Pool, crypto: Arc<SecretsCrypto>) -> Self {
        Self { pool, crypto }
    }
}

#[cfg(feature = "postgres")]
#[async_trait]
impl SecretsStore for PostgresSecretsStore {
    async fn create(
        &self,
        user_id: &str,
        params: CreateSecretParams,
    ) -> Result<Secret, SecretError> {
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| SecretError::Database(e.to_string()))?;

        // Encrypt the secret value
        let plaintext = params.value.expose_secret().as_bytes();
        let (encrypted_value, key_salt) = self.crypto.encrypt(plaintext)?;

        let id = Uuid::new_v4();
        let now = Utc::now();

        let row = client
            .query_one(
                r#"
                INSERT INTO secrets (id, user_id, name, encrypted_value, key_salt, provider, expires_at, created_at, updated_at)
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $8)
                ON CONFLICT (user_id, name) DO UPDATE SET
                    encrypted_value = EXCLUDED.encrypted_value,
                    key_salt = EXCLUDED.key_salt,
                    provider = EXCLUDED.provider,
                    expires_at = EXCLUDED.expires_at,
                    updated_at = NOW()
                RETURNING id, user_id, name, encrypted_value, key_salt, provider, expires_at,
                          last_used_at, usage_count, created_at, updated_at
                "#,
                &[
                    &id,
                    &user_id,
                    &params.name,
                    &encrypted_value,
                    &key_salt,
                    &params.provider,
                    &params.expires_at,
                    &now,
                ],
            )
            .await
            .map_err(|e| SecretError::Database(e.to_string()))?;

        Ok(row_to_secret(&row))
    }

    async fn get(&self, user_id: &str, name: &str) -> Result<Secret, SecretError> {
        let name = name.to_lowercase();
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| SecretError::Database(e.to_string()))?;

        let row = client
            .query_opt(
                r#"
                SELECT id, user_id, name, encrypted_value, key_salt, provider, expires_at,
                       last_used_at, usage_count, created_at, updated_at
                FROM secrets
                WHERE user_id = $1 AND name = $2
                "#,
                &[&user_id, &name],
            )
            .await
            .map_err(|e| SecretError::Database(e.to_string()))?;

        match row {
            Some(r) => {
                let secret = row_to_secret(&r);

                // Check expiration
                if let Some(expires_at) = secret.expires_at
                    && expires_at < Utc::now()
                {
                    return Err(SecretError::Expired);
                }

                Ok(secret)
            }
            None => Err(SecretError::NotFound(name.to_string())),
        }
    }

    async fn get_decrypted(
        &self,
        user_id: &str,
        name: &str,
    ) -> Result<DecryptedSecret, SecretError> {
        let secret = self.get(user_id, name).await?;
        self.crypto
            .decrypt(&secret.encrypted_value, &secret.key_salt)
    }

    async fn consume_if_matches(
        &self,
        user_id: &str,
        name: &str,
        expected_value: &str,
    ) -> Result<SecretConsumeResult, SecretError> {
        let name = name.to_lowercase();
        let mut client = self
            .pool
            .get()
            .await
            .map_err(|e| SecretError::Database(e.to_string()))?;
        let tx = client
            .transaction()
            .await
            .map_err(|e| SecretError::Database(e.to_string()))?;

        let row = tx
            .query_opt(
                r#"
                SELECT id, user_id, name, encrypted_value, key_salt, provider, expires_at,
                       last_used_at, usage_count, created_at, updated_at
                FROM secrets
                WHERE user_id = $1 AND name = $2
                FOR UPDATE
                "#,
                &[&user_id, &name],
            )
            .await
            .map_err(|e| SecretError::Database(e.to_string()))?;

        let Some(row) = row else {
            return Ok(SecretConsumeResult::NotFound);
        };

        let secret = row_to_secret(&row);
        if let Some(expires_at) = secret.expires_at
            && expires_at < Utc::now()
        {
            return Err(SecretError::Expired);
        }

        let decrypted = self
            .crypto
            .decrypt(&secret.encrypted_value, &secret.key_salt)?;
        if decrypted.expose() != expected_value {
            return Ok(SecretConsumeResult::Mismatched);
        }

        tx.execute(
            "DELETE FROM secrets WHERE user_id = $1 AND name = $2",
            &[&user_id, &name],
        )
        .await
        .map_err(|e| SecretError::Database(e.to_string()))?;

        tx.commit()
            .await
            .map_err(|e| SecretError::Database(e.to_string()))?;

        Ok(SecretConsumeResult::Matched)
    }

    async fn exists(&self, user_id: &str, name: &str) -> Result<bool, SecretError> {
        let name = name.to_lowercase();
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| SecretError::Database(e.to_string()))?;

        let row = client
            .query_one(
                "SELECT EXISTS(SELECT 1 FROM secrets WHERE user_id = $1 AND name = $2)",
                &[&user_id, &name],
            )
            .await
            .map_err(|e| SecretError::Database(e.to_string()))?;

        Ok(row.get(0))
    }

    async fn any_exist(&self) -> Result<bool, SecretError> {
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| SecretError::Database(e.to_string()))?;

        let row = client
            .query_one("SELECT EXISTS(SELECT 1 FROM secrets)", &[])
            .await
            .map_err(|e| SecretError::Database(e.to_string()))?;

        Ok(row.get(0))
    }

    async fn list(&self, user_id: &str) -> Result<Vec<SecretRef>, SecretError> {
        let client = self
            .pool
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
            .into_iter()
            .map(|r| SecretRef {
                name: r.get(0),
                provider: r.get(1),
            })
            .collect())
    }

    async fn delete(&self, user_id: &str, name: &str) -> Result<bool, SecretError> {
        let name = name.to_lowercase();
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| SecretError::Database(e.to_string()))?;

        let result = client
            .execute(
                "DELETE FROM secrets WHERE user_id = $1 AND name = $2",
                &[&user_id, &name],
            )
            .await
            .map_err(|e| SecretError::Database(e.to_string()))?;

        Ok(result > 0)
    }

    async fn record_usage(&self, secret_id: Uuid) -> Result<(), SecretError> {
        let client = self
            .pool
            .get()
            .await
            .map_err(|e| SecretError::Database(e.to_string()))?;

        client
            .execute(
                r#"
                UPDATE secrets
                SET last_used_at = NOW(), usage_count = usage_count + 1
                WHERE id = $1
                "#,
                &[&secret_id],
            )
            .await
            .map_err(|e| SecretError::Database(e.to_string()))?;

        Ok(())
    }

    async fn is_accessible(
        &self,
        user_id: &str,
        secret_name: &str,
        allowed_secrets: &[String],
    ) -> Result<bool, SecretError> {
        let secret_name_lower = secret_name.to_lowercase();
        // First check if the secret exists
        if !self.exists(user_id, &secret_name_lower).await? {
            return Ok(false);
        }

        // Check if secret is in the allowed list
        // Supports glob patterns: "openai_*" matches "openai_api_key"
        for pattern in allowed_secrets {
            let pattern_lower = pattern.to_lowercase();
            if pattern_lower == secret_name_lower {
                return Ok(true);
            }

            // Simple glob: * matches any suffix
            if let Some(prefix) = pattern_lower.strip_suffix('*')
                && secret_name_lower.starts_with(prefix)
            {
                return Ok(true);
            }
        }

        Ok(false)
    }
}

#[cfg(feature = "postgres")]
fn row_to_secret(row: &tokio_postgres::Row) -> Secret {
    Secret {
        id: row.get("id"),
        user_id: row.get("user_id"),
        name: row.get("name"),
        encrypted_value: row.get("encrypted_value"),
        key_salt: row.get("key_salt"),
        provider: row.get("provider"),
        expires_at: row.get("expires_at"),
        last_used_at: row.get("last_used_at"),
        usage_count: row.get("usage_count"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
    }
}

// ==================== libSQL implementation ====================

/// libSQL/Turso implementation of SecretsStore.
///
/// Holds an `Arc<Database>` handle and creates a fresh connection per operation,
/// matching the connection-per-request pattern used by the main `LibSqlBackend`.
#[cfg(feature = "libsql")]
pub struct LibSqlSecretsStore {
    db: Arc<libsql::Database>,
    crypto: Arc<SecretsCrypto>,
}

#[cfg(feature = "libsql")]
impl LibSqlSecretsStore {
    /// Create a new store with the given shared libsql database handle and crypto instance.
    pub fn new(db: Arc<libsql::Database>, crypto: Arc<SecretsCrypto>) -> Self {
        Self { db, crypto }
    }

    async fn connect(&self) -> Result<libsql::Connection, SecretError> {
        let conn = self
            .db
            .connect()
            .map_err(|e| SecretError::Database(format!("Connection failed: {}", e)))?;
        conn.query("PRAGMA busy_timeout = 5000", ())
            .await
            .map_err(|e| SecretError::Database(format!("Failed to set busy_timeout: {}", e)))?;
        Ok(conn)
    }
}

#[cfg(feature = "libsql")]
#[async_trait]
impl SecretsStore for LibSqlSecretsStore {
    async fn create(
        &self,
        user_id: &str,
        params: CreateSecretParams,
    ) -> Result<Secret, SecretError> {
        let plaintext = params.value.expose_secret().as_bytes();
        let (encrypted_value, key_salt) = self.crypto.encrypt(plaintext)?;

        let id = Uuid::new_v4();
        let now = Utc::now();
        let now_str = now.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let expires_at_str = params
            .expires_at
            .map(|dt| dt.to_rfc3339_opts(chrono::SecondsFormat::Millis, true));

        // Start transaction for atomic upsert + read-back
        let conn = self.connect().await?;
        let tx = conn
            .transaction()
            .await
            .map_err(|e| SecretError::Database(e.to_string()))?;

        tx.execute(
                r#"
                INSERT INTO secrets (id, user_id, name, encrypted_value, key_salt, provider, expires_at, created_at, updated_at)
                VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)
                ON CONFLICT (user_id, name) DO UPDATE SET
                    encrypted_value = excluded.encrypted_value,
                    key_salt = excluded.key_salt,
                    provider = excluded.provider,
                    expires_at = excluded.expires_at,
                    updated_at = ?8
                "#,
                libsql::params![
                    id.to_string(),
                    user_id,
                    params.name.as_str(),
                    libsql::Value::Blob(encrypted_value.clone()),
                    libsql::Value::Blob(key_salt.clone()),
                    libsql_opt_text(params.provider.as_deref()),
                    libsql_opt_text(expires_at_str.as_deref()),
                    now_str.as_str(),
                ],
            )
            .await
            .map_err(|e| SecretError::Database(e.to_string()))?;

        // Read back the row (may have been upserted)
        let mut rows = tx
            .query(
                r#"
                SELECT id, user_id, name, encrypted_value, key_salt, provider, expires_at,
                       last_used_at, usage_count, created_at, updated_at
                FROM secrets
                WHERE user_id = ?1 AND name = ?2
                "#,
                libsql::params![user_id, params.name.as_str()],
            )
            .await
            .map_err(|e| SecretError::Database(e.to_string()))?;

        let row = rows
            .next()
            .await
            .map_err(|e| SecretError::Database(e.to_string()))?
            .ok_or_else(|| SecretError::Database("Insert succeeded but row not found".into()))?;

        let secret = libsql_row_to_secret(&row)?;

        tx.commit()
            .await
            .map_err(|e| SecretError::Database(e.to_string()))?;

        Ok(secret)
    }

    async fn get(&self, user_id: &str, name: &str) -> Result<Secret, SecretError> {
        let name = name.to_lowercase();
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                r#"
                SELECT id, user_id, name, encrypted_value, key_salt, provider, expires_at,
                       last_used_at, usage_count, created_at, updated_at
                FROM secrets
                WHERE user_id = ?1 AND name = ?2
                "#,
                libsql::params![user_id, name.as_str()],
            )
            .await
            .map_err(|e| SecretError::Database(e.to_string()))?;

        match rows
            .next()
            .await
            .map_err(|e| SecretError::Database(e.to_string()))?
        {
            Some(row) => {
                let secret = libsql_row_to_secret(&row)?;

                if let Some(expires_at) = secret.expires_at
                    && expires_at < Utc::now()
                {
                    return Err(SecretError::Expired);
                }

                Ok(secret)
            }
            None => Err(SecretError::NotFound(name.to_string())),
        }
    }

    async fn get_decrypted(
        &self,
        user_id: &str,
        name: &str,
    ) -> Result<DecryptedSecret, SecretError> {
        let secret = self.get(user_id, name).await?;
        self.crypto
            .decrypt(&secret.encrypted_value, &secret.key_salt)
    }

    async fn consume_if_matches(
        &self,
        user_id: &str,
        name: &str,
        expected_value: &str,
    ) -> Result<SecretConsumeResult, SecretError> {
        let name = name.to_lowercase();
        let conn = self.connect().await?;

        // safety: BEGIN IMMEDIATE acquires a write lock up front so the
        // compare-and-delete flow cannot race with another callback.
        conn.execute("BEGIN IMMEDIATE", ())
            .await
            .map_err(|e| SecretError::Database(e.to_string()))?;

        let result = async {
            let mut rows = conn
                .query(
                    r#"
                    SELECT id, user_id, name, encrypted_value, key_salt, provider, expires_at,
                           last_used_at, usage_count, created_at, updated_at
                    FROM secrets
                    WHERE user_id = ?1 AND name = ?2
                    "#,
                    libsql::params![user_id, name.as_str()],
                )
                .await
                .map_err(|e| SecretError::Database(e.to_string()))?;

            let Some(row) = rows
                .next()
                .await
                .map_err(|e| SecretError::Database(e.to_string()))?
            else {
                return Ok(SecretConsumeResult::NotFound);
            };

            let secret = libsql_row_to_secret(&row)?;
            if let Some(expires_at) = secret.expires_at
                && expires_at < Utc::now()
            {
                return Err(SecretError::Expired);
            }

            let decrypted = self
                .crypto
                .decrypt(&secret.encrypted_value, &secret.key_salt)?;
            if decrypted.expose() != expected_value {
                return Ok(SecretConsumeResult::Mismatched);
            }

            conn.execute(
                "DELETE FROM secrets WHERE user_id = ?1 AND name = ?2",
                libsql::params![user_id, name.as_str()],
            )
            .await
            .map_err(|e| SecretError::Database(e.to_string()))?;

            Ok(SecretConsumeResult::Matched)
        }
        .await;

        match result {
            Ok(outcome) => {
                conn.execute("COMMIT", ())
                    .await
                    .map_err(|e| SecretError::Database(e.to_string()))?;
                Ok(outcome)
            }
            Err(e) => {
                let _ = conn.execute("ROLLBACK", ()).await;
                Err(e)
            }
        }
    }

    async fn exists(&self, user_id: &str, name: &str) -> Result<bool, SecretError> {
        let name = name.to_lowercase();
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT 1 FROM secrets WHERE user_id = ?1 AND name = ?2",
                libsql::params![user_id, name.as_str()],
            )
            .await
            .map_err(|e| SecretError::Database(e.to_string()))?;

        Ok(rows
            .next()
            .await
            .map_err(|e| SecretError::Database(e.to_string()))?
            .is_some())
    }

    async fn any_exist(&self) -> Result<bool, SecretError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query("SELECT 1 FROM secrets LIMIT 1", libsql::params![])
            .await
            .map_err(|e| SecretError::Database(e.to_string()))?;

        Ok(rows
            .next()
            .await
            .map_err(|e| SecretError::Database(e.to_string()))?
            .is_some())
    }

    async fn list(&self, user_id: &str) -> Result<Vec<SecretRef>, SecretError> {
        let conn = self.connect().await?;
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
                provider: row.get::<String>(1).ok(),
            });
        }
        Ok(refs)
    }

    async fn delete(&self, user_id: &str, name: &str) -> Result<bool, SecretError> {
        let name = name.to_lowercase();
        let conn = self.connect().await?;
        let affected = conn
            .execute(
                "DELETE FROM secrets WHERE user_id = ?1 AND name = ?2",
                libsql::params![user_id, name.as_str()],
            )
            .await
            .map_err(|e| SecretError::Database(e.to_string()))?;

        Ok(affected > 0)
    }

    async fn record_usage(&self, secret_id: Uuid) -> Result<(), SecretError> {
        let now = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let conn = self.connect().await?;

        conn.execute(
            r#"
                UPDATE secrets
                SET last_used_at = ?1, usage_count = usage_count + 1
                WHERE id = ?2
                "#,
            libsql::params![now.as_str(), secret_id.to_string()],
        )
        .await
        .map_err(|e| SecretError::Database(e.to_string()))?;

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

#[cfg(feature = "libsql")]
fn libsql_opt_text(s: Option<&str>) -> libsql::Value {
    match s {
        Some(s) => libsql::Value::Text(s.to_string()),
        None => libsql::Value::Null,
    }
}

#[cfg(feature = "libsql")]
fn libsql_parse_timestamp(s: &str) -> Result<chrono::DateTime<Utc>, SecretError> {
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Ok(dt.with_timezone(&Utc));
    }
    if let Ok(ndt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S%.f") {
        return Ok(ndt.and_utc());
    }
    if let Ok(ndt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S") {
        return Ok(ndt.and_utc());
    }
    Err(SecretError::Database(format!(
        "unparseable timestamp: {:?}",
        s
    )))
}

#[cfg(feature = "libsql")]
fn libsql_row_to_secret(row: &libsql::Row) -> Result<Secret, SecretError> {
    let id_str: String = row
        .get(0)
        .map_err(|e| SecretError::Database(e.to_string()))?;
    let user_id: String = row
        .get(1)
        .map_err(|e| SecretError::Database(e.to_string()))?;
    let name: String = row
        .get(2)
        .map_err(|e| SecretError::Database(e.to_string()))?;
    let encrypted_value: Vec<u8> = row
        .get(3)
        .map_err(|e| SecretError::Database(e.to_string()))?;
    let key_salt: Vec<u8> = row
        .get(4)
        .map_err(|e| SecretError::Database(e.to_string()))?;
    let provider: Option<String> = row.get::<String>(5).ok().filter(|s| !s.is_empty());
    let expires_at = row
        .get::<String>(6)
        .ok()
        .filter(|s| !s.is_empty())
        .and_then(|s| libsql_parse_timestamp(&s).ok());
    let last_used_at = row
        .get::<String>(7)
        .ok()
        .filter(|s| !s.is_empty())
        .and_then(|s| libsql_parse_timestamp(&s).ok());
    let usage_count: i64 = row.get::<i64>(8).unwrap_or(0);
    let created_at_str: String = row
        .get(9)
        .map_err(|e| SecretError::Database(e.to_string()))?;
    let updated_at_str: String = row
        .get(10)
        .map_err(|e| SecretError::Database(e.to_string()))?;

    Ok(Secret {
        id: id_str
            .parse()
            .map_err(|e: uuid::Error| SecretError::Database(e.to_string()))?,
        user_id,
        name,
        encrypted_value,
        key_salt,
        provider,
        expires_at,
        last_used_at,
        usage_count,
        created_at: libsql_parse_timestamp(&created_at_str)?,
        updated_at: libsql_parse_timestamp(&updated_at_str)?,
    })
}

/// In-memory secrets store. Used for testing and as a fallback when no
/// persistent secrets backend is configured (extension listing/install still
/// works, but stored secrets won't survive a restart).
pub mod in_memory {
    use std::collections::HashMap;
    use std::sync::Arc;

    use async_trait::async_trait;
    use chrono::Utc;
    use secrecy::ExposeSecret;
    use tokio::sync::RwLock;
    use uuid::Uuid;

    use crate::secrets::crypto::SecretsCrypto;
    use crate::secrets::store::SecretConsumeResult;
    use crate::secrets::store::SecretsStore;
    use crate::secrets::types::{
        CreateSecretParams, DecryptedSecret, Secret, SecretError, SecretRef,
    };

    pub struct InMemorySecretsStore {
        secrets: RwLock<HashMap<(String, String), Secret>>,
        crypto: Arc<SecretsCrypto>,
    }

    impl InMemorySecretsStore {
        pub fn new(crypto: Arc<SecretsCrypto>) -> Self {
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
            let (encrypted_value, key_salt) = self.crypto.encrypt(plaintext)?;

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
            self.crypto
                .decrypt(&secret.encrypted_value, &secret.key_salt)
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

            let decrypted = self
                .crypto
                .decrypt(&secret.encrypted_value, &secret.key_salt)?;
            if decrypted.expose() != expected_value {
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
                .filter(|((uid, _), _)| uid == user_id)
                .map(|((_, _), s)| SecretRef {
                    name: s.name.clone(),
                    provider: s.provider.clone(),
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

        async fn record_usage(&self, _secret_id: Uuid) -> Result<(), SecretError> {
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
}

#[cfg(test)]
mod tests {
    use crate::secrets::store::{SecretConsumeResult, SecretsStore};
    use crate::secrets::types::CreateSecretParams;
    use crate::testing::credentials::{
        TEST_OPENAI_API_KEY_SHORT, TEST_SECRET_VALUE, TEST_STRIPE_KEY, test_secrets_store,
    };

    fn test_store() -> crate::secrets::store::in_memory::InMemorySecretsStore {
        test_secrets_store()
    }

    #[tokio::test]
    async fn test_create_and_get() {
        let store = test_store();
        let params = CreateSecretParams::new("api_key", TEST_SECRET_VALUE);

        store.create("user1", params).await.unwrap();

        let decrypted = store.get_decrypted("user1", "api_key").await.unwrap();
        assert_eq!(decrypted.expose(), TEST_SECRET_VALUE);
    }

    #[tokio::test]
    async fn test_exists() {
        let store = test_store();
        let params = CreateSecretParams::new("my_secret", "value");

        assert!(!store.exists("user1", "my_secret").await.unwrap());
        store.create("user1", params).await.unwrap();
        assert!(store.exists("user1", "my_secret").await.unwrap());
    }

    /// `any_exist` backs the startup safety gate: if IronClaw
    /// auto-generates a master key and the store is already populated,
    /// those rows were encrypted with a different key and startup must
    /// fail. The predicate must report the global store state, not
    /// scoped to a user.
    #[tokio::test]
    async fn any_exist_reflects_global_store_state() {
        let store = test_store();
        assert!(
            !store.any_exist().await.unwrap(),
            "empty store must report no rows"
        );

        store
            .create("user_a", CreateSecretParams::new("k", "v"))
            .await
            .unwrap();
        assert!(
            store.any_exist().await.unwrap(),
            "store with a row must report rows present, even for a different user"
        );
    }

    #[tokio::test]
    async fn test_delete() {
        let store = test_store();
        let params = CreateSecretParams::new("to_delete", "value");

        store.create("user1", params).await.unwrap();
        assert!(store.exists("user1", "to_delete").await.unwrap());

        store.delete("user1", "to_delete").await.unwrap();
        assert!(!store.exists("user1", "to_delete").await.unwrap());
    }

    #[tokio::test]
    async fn test_consume_if_matches_deletes_only_on_match() {
        let store = test_store();
        let params = CreateSecretParams::new("oauth_state", "expected-nonce");

        store.create("user1", params).await.unwrap();

        let mismatched = store
            .consume_if_matches("user1", "oauth_state", "wrong-nonce")
            .await
            .unwrap();
        assert_eq!(mismatched, SecretConsumeResult::Mismatched);
        assert!(store.exists("user1", "oauth_state").await.unwrap());

        let matched = store
            .consume_if_matches("user1", "oauth_state", "expected-nonce")
            .await
            .unwrap();
        assert_eq!(matched, SecretConsumeResult::Matched);
        assert!(!store.exists("user1", "oauth_state").await.unwrap());
    }

    #[tokio::test]
    async fn test_list() {
        let store = test_store();

        store
            .create("user1", CreateSecretParams::new("key1", "v1"))
            .await
            .unwrap();
        store
            .create(
                "user1",
                CreateSecretParams::new("key2", "v2").with_provider("openai"),
            )
            .await
            .unwrap();
        store
            .create("user2", CreateSecretParams::new("key3", "v3"))
            .await
            .unwrap();

        let list = store.list("user1").await.unwrap();
        assert_eq!(list.len(), 2);
    }

    #[tokio::test]
    async fn test_is_accessible() {
        let store = test_store();
        store
            .create(
                "user1",
                CreateSecretParams::new("openai_key", TEST_OPENAI_API_KEY_SHORT),
            )
            .await
            .unwrap();
        store
            .create(
                "user1",
                CreateSecretParams::new("stripe_key", TEST_STRIPE_KEY),
            )
            .await
            .unwrap();

        // Exact match
        let allowed = vec!["openai_key".to_string()];
        assert!(
            store
                .is_accessible("user1", "openai_key", &allowed)
                .await
                .unwrap()
        );
        assert!(
            !store
                .is_accessible("user1", "stripe_key", &allowed)
                .await
                .unwrap()
        );

        // Glob pattern
        let allowed = vec!["openai_*".to_string()];
        assert!(
            store
                .is_accessible("user1", "openai_key", &allowed)
                .await
                .unwrap()
        );
        assert!(
            !store
                .is_accessible("user1", "stripe_key", &allowed)
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn test_expired_secret_returns_error() {
        let store = test_store();
        let expires_at = chrono::Utc::now() - chrono::Duration::hours(1);
        let params = CreateSecretParams::new("expired_key", "value").with_expiry(expires_at);

        store.create("user1", params).await.unwrap();

        let result = store.get("user1", "expired_key").await;
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            crate::secrets::SecretError::Expired
        ));
    }

    #[tokio::test]
    async fn test_non_expired_secret_succeeds() {
        let store = test_store();
        let expires_at = chrono::Utc::now() + chrono::Duration::hours(1);
        let params = CreateSecretParams::new("fresh_key", "value").with_expiry(expires_at);

        store.create("user1", params).await.unwrap();

        let result = store.get("user1", "fresh_key").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_user_isolation() {
        let store = test_store();

        store
            .create(
                "user1",
                CreateSecretParams::new("shared_name", "user1_value"),
            )
            .await
            .unwrap();
        store
            .create(
                "user2",
                CreateSecretParams::new("shared_name", "user2_value"),
            )
            .await
            .unwrap();

        let v1 = store.get_decrypted("user1", "shared_name").await.unwrap();
        let v2 = store.get_decrypted("user2", "shared_name").await.unwrap();

        assert_eq!(v1.expose(), "user1_value");
        assert_eq!(v2.expose(), "user2_value");
    }
}
