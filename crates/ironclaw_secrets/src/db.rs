//! Durable credential-store backends for IronClaw Reborn.
//!
//! Account and session payloads are encrypted with [`SecretsCrypto`] before they
//! are written to libSQL or PostgreSQL. Operators must still enable
//! storage-layer encryption for database files, WALs, snapshots, and backups
//! because scope keys, account ids, encrypted blobs, and indexes remain durable
//! metadata outside the encrypted payload.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use ironclaw_host_api::{
    CapabilityId, ExtensionId, InvocationId, ResourceScope, SecretHandle, Timestamp,
};
use secrecy::ExposeSecret;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

const CREDENTIAL_ACCOUNTS_FOR_SCOPE_LIMIT: usize = 1000;
const CREDENTIAL_ACCOUNTS_FOR_SCOPE_QUERY_LIMIT: i64 = 1001;
const SECRET_STORE_KEY_CHECK_ID: &str = "active";
const SECRET_STORE_KEY_CHECK_PLAINTEXT: &str = "reborn-secret-store-key-check-v1";

use crate::legacy_store::{DecryptedSecret, Secret, SecretRef};
use crate::{
    CreateSecretParams, CredentialAccount, CredentialAccountId, CredentialAccountStatus,
    CredentialAccountStore, CredentialBrokerError, CredentialSession, CredentialSessionId,
    CredentialSessionStore, SecretConsumeResult, SecretError, SecretsCrypto, SecretsStore,
    crypto::{
        ScopeKey, credential_account_aad, credential_session_aad, secret_record_aad,
        secret_store_key_check_aad,
    },
};

#[cfg(feature = "libsql")]
const LIBSQL_CREDENTIAL_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS reborn_credential_accounts (
    tenant_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    agent_id TEXT NOT NULL,
    project_id TEXT NOT NULL,
    account_id TEXT NOT NULL,
    status TEXT NOT NULL,
    provider_or_extension_id TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    payload TEXT NOT NULL DEFAULT '{}',
    encrypted_payload BLOB NOT NULL DEFAULT X'',
    payload_key_salt BLOB NOT NULL DEFAULT X'',
    PRIMARY KEY (tenant_id, user_id, agent_id, project_id, account_id)
);
CREATE INDEX IF NOT EXISTS idx_reborn_credential_accounts_scope_status
    ON reborn_credential_accounts(tenant_id, user_id, agent_id, project_id, status);

CREATE TABLE IF NOT EXISTS reborn_credential_sessions (
    tenant_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    agent_id TEXT NOT NULL,
    project_id TEXT NOT NULL,
    mission_id TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    invocation_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    account_id TEXT NOT NULL,
    expires_at TEXT,
    max_uses INTEGER,
    uses INTEGER NOT NULL DEFAULT 0,
    payload TEXT NOT NULL DEFAULT '{}',
    encrypted_payload BLOB NOT NULL DEFAULT X'',
    payload_key_salt BLOB NOT NULL DEFAULT X'',
    PRIMARY KEY (tenant_id, user_id, agent_id, project_id, mission_id, thread_id, invocation_id, session_id),
    CONSTRAINT reborn_credential_sessions_account_fk
        FOREIGN KEY (tenant_id, user_id, agent_id, project_id, account_id)
        REFERENCES reborn_credential_accounts(tenant_id, user_id, agent_id, project_id, account_id)
        ON UPDATE CASCADE
        ON DELETE RESTRICT
);
CREATE INDEX IF NOT EXISTS idx_reborn_credential_sessions_account
    ON reborn_credential_sessions(tenant_id, user_id, agent_id, project_id, account_id);
"#;

#[cfg(feature = "postgres")]
const POSTGRES_CREDENTIAL_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS reborn_credential_accounts (
    tenant_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    agent_id TEXT NOT NULL,
    project_id TEXT NOT NULL,
    account_id TEXT NOT NULL,
    status TEXT NOT NULL,
    provider_or_extension_id TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    encrypted_payload BYTEA NOT NULL DEFAULT '\x'::bytea,
    payload_key_salt BYTEA NOT NULL DEFAULT '\x'::bytea,
    PRIMARY KEY (tenant_id, user_id, agent_id, project_id, account_id)
);
CREATE INDEX IF NOT EXISTS idx_reborn_credential_accounts_scope_status
    ON reborn_credential_accounts(tenant_id, user_id, agent_id, project_id, status);

CREATE TABLE IF NOT EXISTS reborn_credential_sessions (
    tenant_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    agent_id TEXT NOT NULL,
    project_id TEXT NOT NULL,
    mission_id TEXT NOT NULL,
    thread_id TEXT NOT NULL,
    invocation_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    account_id TEXT NOT NULL,
    expires_at TEXT,
    max_uses BIGINT,
    uses BIGINT NOT NULL DEFAULT 0,
    payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    encrypted_payload BYTEA NOT NULL DEFAULT '\x'::bytea,
    payload_key_salt BYTEA NOT NULL DEFAULT '\x'::bytea,
    PRIMARY KEY (tenant_id, user_id, agent_id, project_id, mission_id, thread_id, invocation_id, session_id),
    CONSTRAINT reborn_credential_sessions_account_fk
        FOREIGN KEY (tenant_id, user_id, agent_id, project_id, account_id)
        REFERENCES reborn_credential_accounts(tenant_id, user_id, agent_id, project_id, account_id)
        ON UPDATE CASCADE
        ON DELETE RESTRICT
);
CREATE INDEX IF NOT EXISTS idx_reborn_credential_sessions_account
    ON reborn_credential_sessions(tenant_id, user_id, agent_id, project_id, account_id);
"#;

#[cfg(feature = "libsql")]
pub struct LibSqlCredentialStore {
    db: Arc<libsql::Database>,
    crypto: Arc<SecretsCrypto>,
}

#[cfg(feature = "libsql")]
impl LibSqlCredentialStore {
    pub fn new(db: Arc<libsql::Database>, crypto: Arc<SecretsCrypto>) -> Self {
        Self { db, crypto }
    }

    pub async fn run_migrations(&self) -> Result<(), CredentialBrokerError> {
        let conn = libsql_connect(&self.db).await?;
        conn.execute_batch(LIBSQL_CREDENTIAL_SCHEMA)
            .await
            .map_err(db_error)?;
        libsql_ensure_encrypted_payload_columns(&conn).await?;
        libsql_ensure_session_account_foreign_key(&conn).await?;
        libsql_reject_unencrypted_payload_rows(&conn).await?;
        Ok(())
    }

    async fn connect(&self) -> Result<libsql::Connection, CredentialBrokerError> {
        libsql_connect(&self.db).await
    }
}

#[cfg(feature = "libsql")]
#[async_trait::async_trait]
impl CredentialAccountStore for LibSqlCredentialStore {
    async fn put_account(
        &self,
        account: CredentialAccount,
    ) -> Result<CredentialAccount, CredentialBrokerError> {
        let conn = libsql_begin_immediate(&self.db).await?;
        let result = async {
            libsql_upsert_account(&conn, &self.crypto, &account).await?;
            Ok(account)
        }
        .await;
        finish_libsql_transaction(&conn, result).await
    }

    async fn get_account(
        &self,
        scope: &ResourceScope,
        account_id: &CredentialAccountId,
    ) -> Result<Option<CredentialAccount>, CredentialBrokerError> {
        let conn = self.connect().await?;
        libsql_get_account(&conn, &self.crypto, scope, account_id).await
    }

    async fn accounts_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<CredentialAccount>, CredentialBrokerError> {
        let conn = self.connect().await?;
        libsql_accounts_for_scope(&conn, &self.crypto, scope).await
    }
}

#[cfg(feature = "libsql")]
#[async_trait::async_trait]
impl CredentialSessionStore for LibSqlCredentialStore {
    async fn issue_session(
        &self,
        session: CredentialSession,
    ) -> Result<CredentialSession, CredentialBrokerError> {
        let conn = libsql_begin_immediate(&self.db).await?;
        let result = async {
            libsql_upsert_session(&conn, &self.crypto, &session, 0).await?;
            Ok(session)
        }
        .await;
        finish_libsql_transaction(&conn, result).await
    }

    async fn get_session(
        &self,
        scope: &ResourceScope,
        session_id: CredentialSessionId,
    ) -> Result<Option<CredentialSession>, CredentialBrokerError> {
        let conn = self.connect().await?;
        Ok(
            libsql_get_session_record(&conn, &self.crypto, scope, session_id)
                .await?
                .map(|record| record.session),
        )
    }

    async fn validate_session(
        &self,
        scope: &ResourceScope,
        session_id: CredentialSessionId,
        now: ironclaw_host_api::Timestamp,
    ) -> Result<CredentialSession, CredentialBrokerError> {
        let conn = self.connect().await?;
        let record = libsql_get_session_record(&conn, &self.crypto, scope, session_id)
            .await?
            .ok_or(CredentialBrokerError::UnknownSession { session_id })?;
        ensure_session_usable(&record, session_id, now)?;
        Ok(record.session)
    }

    async fn consume_session_use(
        &self,
        scope: &ResourceScope,
        session_id: CredentialSessionId,
        now: ironclaw_host_api::Timestamp,
    ) -> Result<CredentialSession, CredentialBrokerError> {
        let conn = libsql_begin_immediate(&self.db).await?;
        let result = async {
            if let Some(record) =
                libsql_consume_session_record(&conn, &self.crypto, scope, session_id, now).await?
            {
                return Ok(record.session);
            }
            let record = libsql_get_session_record(&conn, &self.crypto, scope, session_id).await?;
            session_use_denial_result(record, session_id, now)
        }
        .await;
        finish_libsql_transaction(&conn, result).await
    }
}

#[cfg(feature = "postgres")]
pub struct PostgresCredentialStore {
    pool: deadpool_postgres::Pool,
    crypto: Arc<SecretsCrypto>,
}

#[cfg(feature = "postgres")]
impl PostgresCredentialStore {
    pub fn new(pool: deadpool_postgres::Pool, crypto: Arc<SecretsCrypto>) -> Self {
        Self { pool, crypto }
    }

    pub async fn run_migrations(&self) -> Result<(), CredentialBrokerError> {
        let client = self.pool.get().await.map_err(db_error)?;
        client
            .batch_execute(POSTGRES_CREDENTIAL_SCHEMA)
            .await
            .map_err(db_error)?;
        postgres_ensure_encrypted_payload_columns(&client).await?;
        postgres_ensure_session_account_foreign_key(&client).await?;
        postgres_reject_unencrypted_payload_rows(&client).await
    }
}

#[cfg(feature = "postgres")]
#[async_trait::async_trait]
impl CredentialAccountStore for PostgresCredentialStore {
    async fn put_account(
        &self,
        account: CredentialAccount,
    ) -> Result<CredentialAccount, CredentialBrokerError> {
        let client = self.pool.get().await.map_err(db_error)?;
        postgres_upsert_account(&client, &self.crypto, &account).await?;
        Ok(account)
    }

    async fn get_account(
        &self,
        scope: &ResourceScope,
        account_id: &CredentialAccountId,
    ) -> Result<Option<CredentialAccount>, CredentialBrokerError> {
        let client = self.pool.get().await.map_err(db_error)?;
        postgres_get_account(&client, &self.crypto, scope, account_id).await
    }

    async fn accounts_for_scope(
        &self,
        scope: &ResourceScope,
    ) -> Result<Vec<CredentialAccount>, CredentialBrokerError> {
        let client = self.pool.get().await.map_err(db_error)?;
        postgres_accounts_for_scope(&client, &self.crypto, scope).await
    }
}

#[cfg(feature = "postgres")]
#[async_trait::async_trait]
impl CredentialSessionStore for PostgresCredentialStore {
    async fn issue_session(
        &self,
        session: CredentialSession,
    ) -> Result<CredentialSession, CredentialBrokerError> {
        let client = self.pool.get().await.map_err(db_error)?;
        postgres_upsert_session(&client, &self.crypto, &session, 0).await?;
        Ok(session)
    }

    async fn get_session(
        &self,
        scope: &ResourceScope,
        session_id: CredentialSessionId,
    ) -> Result<Option<CredentialSession>, CredentialBrokerError> {
        let client = self.pool.get().await.map_err(db_error)?;
        Ok(
            postgres_get_session_record(&client, &self.crypto, scope, session_id)
                .await?
                .map(|record| record.session),
        )
    }

    async fn validate_session(
        &self,
        scope: &ResourceScope,
        session_id: CredentialSessionId,
        now: ironclaw_host_api::Timestamp,
    ) -> Result<CredentialSession, CredentialBrokerError> {
        let client = self.pool.get().await.map_err(db_error)?;
        let record = postgres_get_session_record(&client, &self.crypto, scope, session_id)
            .await?
            .ok_or(CredentialBrokerError::UnknownSession { session_id })?;
        ensure_session_usable(&record, session_id, now)?;
        Ok(record.session)
    }

    async fn consume_session_use(
        &self,
        scope: &ResourceScope,
        session_id: CredentialSessionId,
        now: ironclaw_host_api::Timestamp,
    ) -> Result<CredentialSession, CredentialBrokerError> {
        let mut client = self.pool.get().await.map_err(db_error)?;
        let transaction = client
            .build_transaction()
            .isolation_level(tokio_postgres::IsolationLevel::Serializable)
            .start()
            .await
            .map_err(db_error)?;
        let result = async {
            if let Some(record) =
                postgres_consume_session_record(&transaction, &self.crypto, scope, session_id, now)
                    .await?
            {
                return Ok(record.session);
            }
            let record =
                postgres_get_session_record(&transaction, &self.crypto, scope, session_id).await?;
            session_use_denial_result(record, session_id, now)
        }
        .await;
        match result {
            Ok(session) => {
                transaction.commit().await.map_err(db_error)?;
                Ok(session)
            }
            Err(error) => {
                let _ = transaction.rollback().await;
                Err(error)
            }
        }
    }
}

#[derive(Debug)]
struct SessionRecord {
    session: CredentialSession,
    uses: u64,
}

fn ensure_session_usable(
    record: &SessionRecord,
    session_id: CredentialSessionId,
    now: ironclaw_host_api::Timestamp,
) -> Result<(), CredentialBrokerError> {
    if record
        .session
        .expires_at()
        .is_some_and(|expires_at| expires_at <= now)
    {
        return Err(CredentialBrokerError::SessionExpired { session_id });
    }
    if record
        .session
        .max_uses()
        .is_some_and(|max_uses| record.uses >= max_uses)
    {
        return Err(CredentialBrokerError::SessionUseLimitExceeded { session_id });
    }
    Ok(())
}

fn session_use_denial_result(
    record: Option<SessionRecord>,
    session_id: CredentialSessionId,
    now: ironclaw_host_api::Timestamp,
) -> Result<CredentialSession, CredentialBrokerError> {
    let record = record.ok_or(CredentialBrokerError::UnknownSession { session_id })?;
    ensure_session_usable(&record, session_id, now)?;
    Err(persistence_error(
        "credential session use was not consumed atomically",
    ))
}

fn ensure_account_result_within_limit(count: usize) -> Result<(), CredentialBrokerError> {
    if count > CREDENTIAL_ACCOUNTS_FOR_SCOPE_LIMIT {
        return Err(persistence_error(
            "scope contains more than 1000 credential accounts; add pagination before listing",
        ));
    }
    Ok(())
}

#[cfg(feature = "libsql")]
async fn libsql_connect(
    db: &libsql::Database,
) -> Result<libsql::Connection, CredentialBrokerError> {
    let conn = db.connect().map_err(db_error)?;
    conn.execute("PRAGMA foreign_keys = ON", ())
        .await
        .map_err(db_error)?;
    conn.query("PRAGMA busy_timeout = 5000", ())
        .await
        .map_err(db_error)?;
    Ok(conn)
}

#[cfg(feature = "libsql")]
async fn libsql_begin_immediate(
    db: &libsql::Database,
) -> Result<libsql::Connection, CredentialBrokerError> {
    let conn = libsql_connect(db).await?;
    conn.execute("BEGIN IMMEDIATE", ())
        .await
        .map_err(db_error)?;
    Ok(conn)
}

#[cfg(feature = "libsql")]
async fn finish_libsql_transaction<T>(
    conn: &libsql::Connection,
    result: Result<T, CredentialBrokerError>,
) -> Result<T, CredentialBrokerError> {
    match result {
        Ok(value) => {
            conn.execute("COMMIT", ()).await.map_err(db_error)?;
            Ok(value)
        }
        Err(error) => {
            let _ = conn.execute("ROLLBACK", ()).await;
            Err(error)
        }
    }
}

#[cfg(feature = "libsql")]
async fn libsql_upsert_account(
    conn: &libsql::Connection,
    crypto: &SecretsCrypto,
    account: &CredentialAccount,
) -> Result<(), CredentialBrokerError> {
    let key = ScopeKey::from_account_scope(&account.scope);
    let payload = encrypt_credential_account_payload(crypto, account)?;
    conn.execute(
        "INSERT INTO reborn_credential_accounts (tenant_id, user_id, agent_id, project_id, account_id, status, provider_or_extension_id, updated_at, payload, encrypted_payload, payload_key_salt) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, '{}', ?9, ?10) ON CONFLICT(tenant_id, user_id, agent_id, project_id, account_id) DO UPDATE SET status = EXCLUDED.status, provider_or_extension_id = EXCLUDED.provider_or_extension_id, updated_at = EXCLUDED.updated_at, payload = '{}', encrypted_payload = EXCLUDED.encrypted_payload, payload_key_salt = EXCLUDED.payload_key_salt",
        libsql::params![
            key.tenant_id,
            key.user_id,
            key.agent_id,
            key.project_id,
            account.id.as_str(),
            account_status_key(account.status),
            account.provider_or_extension_id.as_str(),
            account.updated_at.to_rfc3339(),
            payload.encrypted_value,
            payload.key_salt,
        ],
    )
    .await
    .map_err(db_error)?;
    Ok(())
}

#[cfg(feature = "libsql")]
async fn libsql_get_account(
    conn: &libsql::Connection,
    crypto: &SecretsCrypto,
    scope: &ResourceScope,
    account_id: &CredentialAccountId,
) -> Result<Option<CredentialAccount>, CredentialBrokerError> {
    let key = ScopeKey::from_account_scope(scope);
    let mut rows = conn
        .query(
            "SELECT status, provider_or_extension_id, updated_at, encrypted_payload, payload_key_salt FROM reborn_credential_accounts WHERE tenant_id = ?1 AND user_id = ?2 AND agent_id = ?3 AND project_id = ?4 AND account_id = ?5",
            libsql::params![key.tenant_id, key.user_id, key.agent_id, key.project_id, account_id.as_str()],
        )
        .await
        .map_err(db_error)?;
    let Some(row) = rows.next().await.map_err(db_error)? else {
        return Ok(None);
    };
    let status: String = row.get(0).map_err(db_error)?;
    let provider: String = row.get(1).map_err(db_error)?;
    let updated_at: String = row.get(2).map_err(db_error)?;
    let encrypted_payload: Vec<u8> = row.get(3).map_err(db_error)?;
    let payload_key_salt: Vec<u8> = row.get(4).map_err(db_error)?;
    validate_account_row(
        decrypt_credential_account_payload(
            crypto,
            &encrypted_payload,
            &payload_key_salt,
            scope,
            account_id,
        )?,
        scope,
        account_id,
        &status,
        &provider,
        &updated_at,
    )
    .map(Some)
}

#[cfg(feature = "libsql")]
async fn libsql_accounts_for_scope(
    conn: &libsql::Connection,
    crypto: &SecretsCrypto,
    scope: &ResourceScope,
) -> Result<Vec<CredentialAccount>, CredentialBrokerError> {
    let key = ScopeKey::from_account_scope(scope);
    let mut rows = conn
        .query(
            "SELECT account_id, status, provider_or_extension_id, updated_at, encrypted_payload, payload_key_salt FROM reborn_credential_accounts WHERE tenant_id = ?1 AND user_id = ?2 AND agent_id = ?3 AND project_id = ?4 ORDER BY account_id LIMIT ?5",
            libsql::params![
                key.tenant_id,
                key.user_id,
                key.agent_id,
                key.project_id,
                CREDENTIAL_ACCOUNTS_FOR_SCOPE_QUERY_LIMIT,
            ],
        )
        .await
        .map_err(db_error)?;
    let mut accounts = Vec::new();
    while let Some(row) = rows.next().await.map_err(db_error)? {
        let account_id: String = row.get(0).map_err(db_error)?;
        let account_id = CredentialAccountId::new(account_id)?;
        let status: String = row.get(1).map_err(db_error)?;
        let provider: String = row.get(2).map_err(db_error)?;
        let updated_at: String = row.get(3).map_err(db_error)?;
        let encrypted_payload: Vec<u8> = row.get(4).map_err(db_error)?;
        let payload_key_salt: Vec<u8> = row.get(5).map_err(db_error)?;
        accounts.push(validate_account_row(
            decrypt_credential_account_payload(
                crypto,
                &encrypted_payload,
                &payload_key_salt,
                scope,
                &account_id,
            )?,
            scope,
            &account_id,
            &status,
            &provider,
            &updated_at,
        )?);
    }
    ensure_account_result_within_limit(accounts.len())?;
    Ok(accounts)
}

#[cfg(feature = "libsql")]
async fn libsql_upsert_session(
    conn: &libsql::Connection,
    crypto: &SecretsCrypto,
    session: &CredentialSession,
    uses: u64,
) -> Result<(), CredentialBrokerError> {
    let key = ScopeKey::from_full_scope(session.scope());
    let payload = encrypt_session_payload(crypto, session)?;
    conn.execute(
        "INSERT INTO reborn_credential_sessions (tenant_id, user_id, agent_id, project_id, mission_id, thread_id, invocation_id, session_id, account_id, expires_at, max_uses, uses, payload, encrypted_payload, payload_key_salt) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, '{}', ?13, ?14) ON CONFLICT(tenant_id, user_id, agent_id, project_id, mission_id, thread_id, invocation_id, session_id) DO UPDATE SET account_id = EXCLUDED.account_id, expires_at = EXCLUDED.expires_at, max_uses = EXCLUDED.max_uses, uses = EXCLUDED.uses, payload = '{}', encrypted_payload = EXCLUDED.encrypted_payload, payload_key_salt = EXCLUDED.payload_key_salt",
        libsql::params![
            key.tenant_id,
            key.user_id,
            key.agent_id,
            key.project_id,
            key.mission_id,
            key.thread_id,
            key.invocation_id,
            session.correlation_id().to_private_storage_string(),
            session.account_id().as_str(),
            session.expires_at().map(|value| value.to_rfc3339()),
            session.max_uses().map(|value| value as i64),
            uses as i64,
            payload.encrypted_value,
            payload.key_salt,
        ],
    )
    .await
    .map_err(db_error)?;
    Ok(())
}

#[cfg(feature = "libsql")]
async fn libsql_get_session_record(
    conn: &libsql::Connection,
    crypto: &SecretsCrypto,
    scope: &ResourceScope,
    session_id: CredentialSessionId,
) -> Result<Option<SessionRecord>, CredentialBrokerError> {
    let key = ScopeKey::from_full_scope(scope);
    let mut rows = conn
        .query(
            "SELECT account_id, expires_at, max_uses, uses, encrypted_payload, payload_key_salt FROM reborn_credential_sessions WHERE tenant_id = ?1 AND user_id = ?2 AND agent_id = ?3 AND project_id = ?4 AND mission_id = ?5 AND thread_id = ?6 AND invocation_id = ?7 AND session_id = ?8",
            libsql::params![key.tenant_id, key.user_id, key.agent_id, key.project_id, key.mission_id, key.thread_id, key.invocation_id, session_id.to_private_storage_string()],
        )
        .await
        .map_err(db_error)?;
    let Some(row) = rows.next().await.map_err(db_error)? else {
        return Ok(None);
    };
    let account_id: String = row.get(0).map_err(db_error)?;
    let expires_at: Option<String> = row.get(1).map_err(db_error)?;
    let max_uses: Option<i64> = row.get(2).map_err(db_error)?;
    let uses: i64 = row.get(3).map_err(db_error)?;
    let encrypted_payload: Vec<u8> = row.get(4).map_err(db_error)?;
    let payload_key_salt: Vec<u8> = row.get(5).map_err(db_error)?;
    validate_session_row(
        decrypt_session_payload(
            crypto,
            &encrypted_payload,
            &payload_key_salt,
            scope,
            session_id,
        )?,
        scope,
        session_id,
        &account_id,
        expires_at.as_deref(),
        max_uses,
        uses,
    )
    .map(Some)
}

#[cfg(feature = "libsql")]
async fn libsql_consume_session_record(
    conn: &libsql::Connection,
    crypto: &SecretsCrypto,
    scope: &ResourceScope,
    session_id: CredentialSessionId,
    now: ironclaw_host_api::Timestamp,
) -> Result<Option<SessionRecord>, CredentialBrokerError> {
    let key = ScopeKey::from_full_scope(scope);
    let mut rows = conn
        .query(
            "UPDATE reborn_credential_sessions SET uses = uses + 1 WHERE tenant_id = ?1 AND user_id = ?2 AND agent_id = ?3 AND project_id = ?4 AND mission_id = ?5 AND thread_id = ?6 AND invocation_id = ?7 AND session_id = ?8 AND (expires_at IS NULL OR expires_at > ?9) AND (max_uses IS NULL OR uses < max_uses) RETURNING account_id, expires_at, max_uses, uses, encrypted_payload, payload_key_salt",
            libsql::params![
                key.tenant_id,
                key.user_id,
                key.agent_id,
                key.project_id,
                key.mission_id,
                key.thread_id,
                key.invocation_id,
                session_id.to_private_storage_string(),
                now.to_rfc3339(),
            ],
        )
        .await
        .map_err(db_error)?;
    let Some(row) = rows.next().await.map_err(db_error)? else {
        return Ok(None);
    };
    let account_id: String = row.get(0).map_err(db_error)?;
    let expires_at: Option<String> = row.get(1).map_err(db_error)?;
    let max_uses: Option<i64> = row.get(2).map_err(db_error)?;
    let uses: i64 = row.get(3).map_err(db_error)?;
    let encrypted_payload: Vec<u8> = row.get(4).map_err(db_error)?;
    let payload_key_salt: Vec<u8> = row.get(5).map_err(db_error)?;
    validate_session_row(
        decrypt_session_payload(
            crypto,
            &encrypted_payload,
            &payload_key_salt,
            scope,
            session_id,
        )?,
        scope,
        session_id,
        &account_id,
        expires_at.as_deref(),
        max_uses,
        uses,
    )
    .map(Some)
}

#[cfg(feature = "postgres")]
async fn postgres_upsert_account(
    client: &impl deadpool_postgres::GenericClient,
    crypto: &SecretsCrypto,
    account: &CredentialAccount,
) -> Result<(), CredentialBrokerError> {
    let key = ScopeKey::from_account_scope(&account.scope);
    let payload = encrypt_credential_account_payload(crypto, account)?;
    client.execute("INSERT INTO reborn_credential_accounts (tenant_id, user_id, agent_id, project_id, account_id, status, provider_or_extension_id, updated_at, payload, encrypted_payload, payload_key_salt) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, '{}'::jsonb, $9, $10) ON CONFLICT(tenant_id, user_id, agent_id, project_id, account_id) DO UPDATE SET status = EXCLUDED.status, provider_or_extension_id = EXCLUDED.provider_or_extension_id, updated_at = EXCLUDED.updated_at, payload = '{}'::jsonb, encrypted_payload = EXCLUDED.encrypted_payload, payload_key_salt = EXCLUDED.payload_key_salt", &[&key.tenant_id, &key.user_id, &key.agent_id, &key.project_id, &account.id.as_str(), &account_status_key(account.status), &account.provider_or_extension_id.as_str(), &account.updated_at.to_rfc3339(), &payload.encrypted_value, &payload.key_salt]).await.map_err(db_error)?;
    Ok(())
}

#[cfg(feature = "postgres")]
async fn postgres_get_account(
    client: &impl deadpool_postgres::GenericClient,
    crypto: &SecretsCrypto,
    scope: &ResourceScope,
    account_id: &CredentialAccountId,
) -> Result<Option<CredentialAccount>, CredentialBrokerError> {
    let key = ScopeKey::from_account_scope(scope);
    let row = client.query_opt("SELECT status, provider_or_extension_id, updated_at, encrypted_payload, payload_key_salt FROM reborn_credential_accounts WHERE tenant_id = $1 AND user_id = $2 AND agent_id = $3 AND project_id = $4 AND account_id = $5", &[&key.tenant_id, &key.user_id, &key.agent_id, &key.project_id, &account_id.as_str()]).await.map_err(db_error)?;
    let Some(row) = row else {
        return Ok(None);
    };
    let status: String = row.get(0);
    let provider: String = row.get(1);
    let updated_at: String = row.get(2);
    let encrypted_payload: Vec<u8> = row.get(3);
    let payload_key_salt: Vec<u8> = row.get(4);
    validate_account_row(
        decrypt_credential_account_payload(
            crypto,
            &encrypted_payload,
            &payload_key_salt,
            scope,
            account_id,
        )?,
        scope,
        account_id,
        &status,
        &provider,
        &updated_at,
    )
    .map(Some)
}

#[cfg(feature = "postgres")]
async fn postgres_accounts_for_scope(
    client: &impl deadpool_postgres::GenericClient,
    crypto: &SecretsCrypto,
    scope: &ResourceScope,
) -> Result<Vec<CredentialAccount>, CredentialBrokerError> {
    let key = ScopeKey::from_account_scope(scope);
    let rows = client.query("SELECT account_id, status, provider_or_extension_id, updated_at, encrypted_payload, payload_key_salt FROM reborn_credential_accounts WHERE tenant_id = $1 AND user_id = $2 AND agent_id = $3 AND project_id = $4 ORDER BY account_id LIMIT $5", &[&key.tenant_id, &key.user_id, &key.agent_id, &key.project_id, &CREDENTIAL_ACCOUNTS_FOR_SCOPE_QUERY_LIMIT]).await.map_err(db_error)?;
    let mut accounts = Vec::new();
    for row in rows {
        let account_id: String = row.get(0);
        let account_id = CredentialAccountId::new(account_id)?;
        let status: String = row.get(1);
        let provider: String = row.get(2);
        let updated_at: String = row.get(3);
        let encrypted_payload: Vec<u8> = row.get(4);
        let payload_key_salt: Vec<u8> = row.get(5);
        accounts.push(validate_account_row(
            decrypt_credential_account_payload(
                crypto,
                &encrypted_payload,
                &payload_key_salt,
                scope,
                &account_id,
            )?,
            scope,
            &account_id,
            &status,
            &provider,
            &updated_at,
        )?);
    }
    ensure_account_result_within_limit(accounts.len())?;
    Ok(accounts)
}

#[cfg(feature = "postgres")]
async fn postgres_upsert_session(
    client: &impl deadpool_postgres::GenericClient,
    crypto: &SecretsCrypto,
    session: &CredentialSession,
    uses: u64,
) -> Result<(), CredentialBrokerError> {
    let key = ScopeKey::from_full_scope(session.scope());
    let max_uses = session.max_uses().map(|value| value as i64);
    let payload = encrypt_session_payload(crypto, session)?;
    client.execute("INSERT INTO reborn_credential_sessions (tenant_id, user_id, agent_id, project_id, mission_id, thread_id, invocation_id, session_id, account_id, expires_at, max_uses, uses, payload, encrypted_payload, payload_key_salt) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, '{}'::jsonb, $13, $14) ON CONFLICT(tenant_id, user_id, agent_id, project_id, mission_id, thread_id, invocation_id, session_id) DO UPDATE SET account_id = EXCLUDED.account_id, expires_at = EXCLUDED.expires_at, max_uses = EXCLUDED.max_uses, uses = EXCLUDED.uses, payload = '{}'::jsonb, encrypted_payload = EXCLUDED.encrypted_payload, payload_key_salt = EXCLUDED.payload_key_salt", &[&key.tenant_id, &key.user_id, &key.agent_id, &key.project_id, &key.mission_id, &key.thread_id, &key.invocation_id, &session.correlation_id().to_private_storage_string(), &session.account_id().as_str(), &session.expires_at().map(|value| value.to_rfc3339()), &max_uses, &(uses as i64), &payload.encrypted_value, &payload.key_salt]).await.map_err(db_error)?;
    Ok(())
}

#[cfg(feature = "postgres")]
async fn postgres_get_session_record(
    client: &impl deadpool_postgres::GenericClient,
    crypto: &SecretsCrypto,
    scope: &ResourceScope,
    session_id: CredentialSessionId,
) -> Result<Option<SessionRecord>, CredentialBrokerError> {
    let key = ScopeKey::from_full_scope(scope);
    let row = client
        .query_opt(
            "SELECT account_id, expires_at, max_uses, uses, encrypted_payload, payload_key_salt FROM reborn_credential_sessions WHERE tenant_id = $1 AND user_id = $2 AND agent_id = $3 AND project_id = $4 AND mission_id = $5 AND thread_id = $6 AND invocation_id = $7 AND session_id = $8",
            &[
                &key.tenant_id,
                &key.user_id,
                &key.agent_id,
                &key.project_id,
                &key.mission_id,
                &key.thread_id,
                &key.invocation_id,
                &session_id.to_private_storage_string(),
            ],
        )
        .await
        .map_err(db_error)?;
    let Some(row) = row else {
        return Ok(None);
    };
    let account_id: String = row.get(0);
    let expires_at: Option<String> = row.get(1);
    let max_uses: Option<i64> = row.get(2);
    let uses: i64 = row.get(3);
    let encrypted_payload: Vec<u8> = row.get(4);
    let payload_key_salt: Vec<u8> = row.get(5);
    validate_session_row(
        decrypt_session_payload(
            crypto,
            &encrypted_payload,
            &payload_key_salt,
            scope,
            session_id,
        )?,
        scope,
        session_id,
        &account_id,
        expires_at.as_deref(),
        max_uses,
        uses,
    )
    .map(Some)
}

#[cfg(feature = "postgres")]
async fn postgres_consume_session_record(
    client: &impl deadpool_postgres::GenericClient,
    crypto: &SecretsCrypto,
    scope: &ResourceScope,
    session_id: CredentialSessionId,
    now: ironclaw_host_api::Timestamp,
) -> Result<Option<SessionRecord>, CredentialBrokerError> {
    let key = ScopeKey::from_full_scope(scope);
    let row = client
        .query_opt(
            "UPDATE reborn_credential_sessions SET uses = uses + 1 WHERE tenant_id = $1 AND user_id = $2 AND agent_id = $3 AND project_id = $4 AND mission_id = $5 AND thread_id = $6 AND invocation_id = $7 AND session_id = $8 AND (expires_at IS NULL OR expires_at > $9) AND (max_uses IS NULL OR uses < max_uses) RETURNING account_id, expires_at, max_uses, uses, encrypted_payload, payload_key_salt",
            &[
                &key.tenant_id,
                &key.user_id,
                &key.agent_id,
                &key.project_id,
                &key.mission_id,
                &key.thread_id,
                &key.invocation_id,
                &session_id.to_private_storage_string(),
                &now.to_rfc3339(),
            ],
        )
        .await
        .map_err(db_error)?;
    let Some(row) = row else {
        return Ok(None);
    };
    let account_id: String = row.get(0);
    let expires_at: Option<String> = row.get(1);
    let max_uses: Option<i64> = row.get(2);
    let uses: i64 = row.get(3);
    let encrypted_payload: Vec<u8> = row.get(4);
    let payload_key_salt: Vec<u8> = row.get(5);
    validate_session_row(
        decrypt_session_payload(
            crypto,
            &encrypted_payload,
            &payload_key_salt,
            scope,
            session_id,
        )?,
        scope,
        session_id,
        &account_id,
        expires_at.as_deref(),
        max_uses,
        uses,
    )
    .map(Some)
}

#[cfg(feature = "libsql")]
async fn libsql_ensure_encrypted_payload_columns(
    conn: &libsql::Connection,
) -> Result<(), CredentialBrokerError> {
    for statement in [
        "ALTER TABLE reborn_credential_accounts ADD COLUMN encrypted_payload BLOB NOT NULL DEFAULT X''",
        "ALTER TABLE reborn_credential_accounts ADD COLUMN payload_key_salt BLOB NOT NULL DEFAULT X''",
        "ALTER TABLE reborn_credential_sessions ADD COLUMN encrypted_payload BLOB NOT NULL DEFAULT X''",
        "ALTER TABLE reborn_credential_sessions ADD COLUMN payload_key_salt BLOB NOT NULL DEFAULT X''",
    ] {
        match conn.execute(statement, ()).await {
            Ok(_) => {}
            Err(error) => ignore_duplicate_column_error(error)?,
        }
    }
    Ok(())
}

#[cfg(feature = "libsql")]
fn ignore_duplicate_column_error(error: libsql::Error) -> Result<(), CredentialBrokerError> {
    let message = error.to_string();
    if message.contains("duplicate column name") {
        Ok(())
    } else {
        Err(db_error(error))
    }
}

#[cfg(feature = "libsql")]
async fn libsql_ensure_session_account_foreign_key(
    conn: &libsql::Connection,
) -> Result<(), CredentialBrokerError> {
    if libsql_session_account_foreign_key_exists(conn).await? {
        return Ok(());
    }
    let migration_result = conn
        .execute_batch(
            r#"
        BEGIN IMMEDIATE;
        DROP TABLE IF EXISTS reborn_credential_sessions_with_account_fk;
        CREATE TABLE reborn_credential_sessions_with_account_fk (
            tenant_id TEXT NOT NULL,
            user_id TEXT NOT NULL,
            agent_id TEXT NOT NULL,
            project_id TEXT NOT NULL,
            mission_id TEXT NOT NULL,
            thread_id TEXT NOT NULL,
            invocation_id TEXT NOT NULL,
            session_id TEXT NOT NULL,
            account_id TEXT NOT NULL,
            expires_at TEXT,
            max_uses INTEGER,
            uses INTEGER NOT NULL DEFAULT 0,
            payload TEXT NOT NULL DEFAULT '{}',
            encrypted_payload BLOB NOT NULL DEFAULT X'',
            payload_key_salt BLOB NOT NULL DEFAULT X'',
            PRIMARY KEY (tenant_id, user_id, agent_id, project_id, mission_id, thread_id, invocation_id, session_id),
            CONSTRAINT reborn_credential_sessions_account_fk
                FOREIGN KEY (tenant_id, user_id, agent_id, project_id, account_id)
                REFERENCES reborn_credential_accounts(tenant_id, user_id, agent_id, project_id, account_id)
                ON UPDATE CASCADE
                ON DELETE RESTRICT
        );
        INSERT INTO reborn_credential_sessions_with_account_fk (
            tenant_id, user_id, agent_id, project_id, mission_id, thread_id, invocation_id,
            session_id, account_id, expires_at, max_uses, uses, payload, encrypted_payload,
            payload_key_salt
        )
        SELECT
            tenant_id, user_id, agent_id, project_id, mission_id, thread_id, invocation_id,
            session_id, account_id, expires_at, max_uses, uses, payload, encrypted_payload,
            payload_key_salt
        FROM reborn_credential_sessions;
        DROP TABLE reborn_credential_sessions;
        ALTER TABLE reborn_credential_sessions_with_account_fk RENAME TO reborn_credential_sessions;
        CREATE INDEX IF NOT EXISTS idx_reborn_credential_sessions_account
            ON reborn_credential_sessions(tenant_id, user_id, agent_id, project_id, account_id);
        COMMIT;
        "#,
        )
        .await;
    if let Err(error) = migration_result {
        let _ = conn.execute("ROLLBACK", ()).await;
        return Err(db_error(error));
    }
    Ok(())
}

#[cfg(feature = "libsql")]
async fn libsql_session_account_foreign_key_exists(
    conn: &libsql::Connection,
) -> Result<bool, CredentialBrokerError> {
    let mut rows = conn
        .query("PRAGMA foreign_key_list(reborn_credential_sessions)", ())
        .await
        .map_err(db_error)?;
    while let Some(row) = rows.next().await.map_err(db_error)? {
        let table: String = row.get(2).map_err(db_error)?;
        if table == "reborn_credential_accounts" {
            return Ok(true);
        }
    }
    Ok(false)
}

#[cfg(feature = "libsql")]
async fn libsql_reject_unencrypted_payload_rows(
    conn: &libsql::Connection,
) -> Result<(), CredentialBrokerError> {
    let unencrypted_accounts = libsql_has_unencrypted_rows(
        conn,
        "SELECT 1 FROM reborn_credential_accounts WHERE length(encrypted_payload) = 0 OR payload <> '{}' LIMIT 1",
    )
    .await?;
    let unencrypted_sessions = libsql_has_unencrypted_rows(
        conn,
        "SELECT 1 FROM reborn_credential_sessions WHERE length(encrypted_payload) = 0 OR payload <> '{}' LIMIT 1",
    )
    .await?;
    if unencrypted_accounts || unencrypted_sessions {
        return Err(persistence_error(
            "credential store contains unencrypted legacy payload rows; rotate or migrate credentials before enabling the durable Reborn credential store",
        ));
    }
    Ok(())
}

#[cfg(feature = "libsql")]
async fn libsql_has_unencrypted_rows(
    conn: &libsql::Connection,
    query: &str,
) -> Result<bool, CredentialBrokerError> {
    let mut rows = conn.query(query, ()).await.map_err(db_error)?;
    Ok(rows.next().await.map_err(db_error)?.is_some())
}

#[cfg(feature = "postgres")]
async fn postgres_ensure_encrypted_payload_columns(
    client: &impl deadpool_postgres::GenericClient,
) -> Result<(), CredentialBrokerError> {
    client
        .batch_execute(
            r#"
            ALTER TABLE reborn_credential_accounts ADD COLUMN IF NOT EXISTS encrypted_payload BYTEA NOT NULL DEFAULT '\x'::bytea;
            ALTER TABLE reborn_credential_accounts ADD COLUMN IF NOT EXISTS payload_key_salt BYTEA NOT NULL DEFAULT '\x'::bytea;
            ALTER TABLE reborn_credential_sessions ADD COLUMN IF NOT EXISTS encrypted_payload BYTEA NOT NULL DEFAULT '\x'::bytea;
            ALTER TABLE reborn_credential_sessions ADD COLUMN IF NOT EXISTS payload_key_salt BYTEA NOT NULL DEFAULT '\x'::bytea;
            "#,
        )
        .await
        .map_err(db_error)
}

#[cfg(feature = "postgres")]
async fn postgres_ensure_session_account_foreign_key(
    client: &impl deadpool_postgres::GenericClient,
) -> Result<(), CredentialBrokerError> {
    client
        .batch_execute(
            r#"
            DO $$
            BEGIN
                ALTER TABLE reborn_credential_sessions
                    ADD CONSTRAINT reborn_credential_sessions_account_fk
                    FOREIGN KEY (tenant_id, user_id, agent_id, project_id, account_id)
                    REFERENCES reborn_credential_accounts(tenant_id, user_id, agent_id, project_id, account_id)
                    ON UPDATE CASCADE
                    ON DELETE RESTRICT;
            EXCEPTION
                WHEN duplicate_object THEN NULL;
            END $$;
            "#,
        )
        .await
        .map_err(db_error)
}

#[cfg(feature = "postgres")]
async fn postgres_reject_unencrypted_payload_rows(
    client: &impl deadpool_postgres::GenericClient,
) -> Result<(), CredentialBrokerError> {
    let unencrypted_accounts = postgres_has_unencrypted_rows(
        client,
        "SELECT 1 FROM reborn_credential_accounts WHERE octet_length(encrypted_payload) = 0 OR payload <> '{}'::jsonb LIMIT 1",
    )
    .await?;
    let unencrypted_sessions = postgres_has_unencrypted_rows(
        client,
        "SELECT 1 FROM reborn_credential_sessions WHERE octet_length(encrypted_payload) = 0 OR payload <> '{}'::jsonb LIMIT 1",
    )
    .await?;
    if unencrypted_accounts || unencrypted_sessions {
        return Err(persistence_error(
            "credential store contains unencrypted legacy payload rows; rotate or migrate credentials before enabling the durable Reborn credential store",
        ));
    }
    Ok(())
}

#[cfg(feature = "postgres")]
async fn postgres_has_unencrypted_rows(
    client: &impl deadpool_postgres::GenericClient,
    query: &str,
) -> Result<bool, CredentialBrokerError> {
    Ok(client
        .query_opt(query, &[])
        .await
        .map_err(db_error)?
        .is_some())
}

#[cfg(feature = "libsql")]
const LIBSQL_SECRET_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS reborn_secret_records (
    user_id TEXT NOT NULL,
    name TEXT NOT NULL,
    id TEXT NOT NULL,
    encrypted_value BLOB NOT NULL,
    key_salt BLOB NOT NULL,
    provider TEXT,
    expires_at TEXT,
    last_used_at TEXT,
    usage_count INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (user_id, name)
);
CREATE INDEX IF NOT EXISTS idx_reborn_secret_records_id
    ON reborn_secret_records(id);
CREATE TABLE IF NOT EXISTS reborn_secret_store_key_check (
    id TEXT PRIMARY KEY,
    encrypted_value BLOB NOT NULL,
    key_salt BLOB NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
"#;

#[cfg(feature = "postgres")]
const POSTGRES_SECRET_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS reborn_secret_records (
    user_id TEXT NOT NULL,
    name TEXT NOT NULL,
    id TEXT NOT NULL,
    encrypted_value BYTEA NOT NULL,
    key_salt BYTEA NOT NULL,
    provider TEXT,
    expires_at TEXT,
    last_used_at TEXT,
    usage_count BIGINT NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (user_id, name)
);
CREATE INDEX IF NOT EXISTS idx_reborn_secret_records_id
    ON reborn_secret_records(id);
CREATE TABLE IF NOT EXISTS reborn_secret_store_key_check (
    id TEXT PRIMARY KEY,
    encrypted_value BYTEA NOT NULL,
    key_salt BYTEA NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
"#;

#[cfg(feature = "libsql")]
pub struct LibSqlSecretsStore {
    db: Arc<libsql::Database>,
    crypto: Arc<SecretsCrypto>,
}

#[cfg(feature = "libsql")]
impl LibSqlSecretsStore {
    pub fn new(db: Arc<libsql::Database>, crypto: Arc<SecretsCrypto>) -> Self {
        Self { db, crypto }
    }

    pub async fn run_migrations(&self) -> Result<(), SecretError> {
        let conn = libsql_secret_connect(&self.db).await?;
        conn.execute_batch(LIBSQL_SECRET_SCHEMA)
            .await
            .map_err(secret_db_error)?;
        Ok(())
    }

    async fn connect(&self) -> Result<libsql::Connection, SecretError> {
        libsql_secret_connect(&self.db).await
    }

    /// Verifies the durable store key-check sentinel.
    ///
    /// Stores that predate the sentinel are bootstrapped by decrypting all
    /// existing secret rows before installing the key-check record.
    pub async fn verify_can_decrypt_existing_secrets(&self) -> Result<(), SecretError> {
        let conn = self.connect().await?;
        libsql_verify_or_bootstrap_secret_store_key_check(&conn, &self.crypto).await
    }
}

#[cfg(feature = "libsql")]
#[async_trait::async_trait]
impl SecretsStore for LibSqlSecretsStore {
    async fn create(
        &self,
        user_id: &str,
        params: CreateSecretParams,
    ) -> Result<Secret, SecretError> {
        let conn = libsql_secret_begin_immediate(&self.db).await?;
        let result = async {
            let secret = build_encrypted_secret(user_id, params, &self.crypto)?;
            libsql_upsert_secret(&conn, &secret).await
        }
        .await;
        finish_libsql_secret_transaction(&conn, result).await
    }

    async fn get(&self, user_id: &str, name: &str) -> Result<Secret, SecretError> {
        let conn = self.connect().await?;
        let name = normalize_secret_name(name);
        let secret = libsql_get_secret(&conn, user_id, &name)
            .await?
            .ok_or_else(|| SecretError::NotFound(name.clone()))?;
        ensure_secret_not_expired(&secret)?;
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
        let conn = libsql_secret_begin_immediate(&self.db).await?;
        let result = async {
            let name = normalize_secret_name(name);
            let Some(secret) = libsql_get_secret(&conn, user_id, &name).await? else {
                return Ok(SecretConsumeResult::NotFound);
            };
            ensure_secret_not_expired(&secret)?;
            let aad = secret_record_aad(user_id, &secret.name);
            let decrypted = self
                .crypto
                .decrypt(&secret.encrypted_value, &secret.key_salt, &aad)?;
            if decrypted.expose() != expected_value {
                return Ok(SecretConsumeResult::Mismatched);
            }
            libsql_delete_secret(&conn, user_id, &name).await?;
            Ok(SecretConsumeResult::Matched)
        }
        .await;
        finish_libsql_secret_transaction(&conn, result).await
    }

    async fn exists(&self, user_id: &str, name: &str) -> Result<bool, SecretError> {
        let conn = self.connect().await?;
        let name = normalize_secret_name(name);
        Ok(libsql_get_secret(&conn, user_id, &name).await?.is_some())
    }

    async fn any_exist(&self) -> Result<bool, SecretError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query("SELECT 1 FROM reborn_secret_records LIMIT 1", ())
            .await
            .map_err(secret_db_error)?;
        Ok(rows.next().await.map_err(secret_db_error)?.is_some())
    }

    async fn list(&self, user_id: &str) -> Result<Vec<SecretRef>, SecretError> {
        let conn = self.connect().await?;
        let mut rows = conn
            .query(
                "SELECT name, provider FROM reborn_secret_records WHERE user_id = ?1 ORDER BY name",
                libsql::params![user_id],
            )
            .await
            .map_err(secret_db_error)?;
        let mut refs = Vec::new();
        while let Some(row) = rows.next().await.map_err(secret_db_error)? {
            refs.push(SecretRef {
                name: row.get(0).map_err(secret_db_error)?,
                provider: row.get(1).map_err(secret_db_error)?,
            });
        }
        Ok(refs)
    }

    async fn delete(&self, user_id: &str, name: &str) -> Result<bool, SecretError> {
        let conn = self.connect().await?;
        let name = normalize_secret_name(name);
        libsql_delete_secret(&conn, user_id, &name).await
    }

    async fn record_usage(&self, secret_id: Uuid) -> Result<(), SecretError> {
        let conn = self.connect().await?;
        let changed = conn
            .execute(
                "UPDATE reborn_secret_records SET last_used_at = ?1, usage_count = usage_count + 1, updated_at = ?1 WHERE id = ?2",
                libsql::params![Utc::now().to_rfc3339(), secret_id.to_string()],
            )
            .await
            .map_err(secret_db_error)?;
        if changed == 0 {
            return Err(SecretError::NotFound(secret_id.to_string()));
        }
        Ok(())
    }

    async fn is_accessible(
        &self,
        user_id: &str,
        secret_name: &str,
        allowed_secrets: &[String],
    ) -> Result<bool, SecretError> {
        secret_accessible(self, user_id, secret_name, allowed_secrets).await
    }
}

#[cfg(feature = "postgres")]
pub struct PostgresSecretsStore {
    pool: deadpool_postgres::Pool,
    crypto: Arc<SecretsCrypto>,
}

#[cfg(feature = "postgres")]
impl PostgresSecretsStore {
    pub fn new(pool: deadpool_postgres::Pool, crypto: Arc<SecretsCrypto>) -> Self {
        Self { pool, crypto }
    }

    pub async fn run_migrations(&self) -> Result<(), SecretError> {
        let client = self.pool.get().await.map_err(secret_db_error)?;
        client
            .batch_execute(POSTGRES_SECRET_SCHEMA)
            .await
            .map_err(secret_db_error)
    }

    /// Verifies the durable store key-check sentinel.
    ///
    /// Stores that predate the sentinel are bootstrapped by decrypting all
    /// existing secret rows before installing the key-check record.
    pub async fn verify_can_decrypt_existing_secrets(&self) -> Result<(), SecretError> {
        let client = self.pool.get().await.map_err(secret_db_error)?;
        postgres_verify_or_bootstrap_secret_store_key_check(&client, &self.crypto).await
    }
}

#[cfg(feature = "postgres")]
#[async_trait::async_trait]
impl SecretsStore for PostgresSecretsStore {
    async fn create(
        &self,
        user_id: &str,
        params: CreateSecretParams,
    ) -> Result<Secret, SecretError> {
        let client = self.pool.get().await.map_err(secret_db_error)?;
        let secret = build_encrypted_secret(user_id, params, &self.crypto)?;
        postgres_upsert_secret(&client, &secret).await
    }

    async fn get(&self, user_id: &str, name: &str) -> Result<Secret, SecretError> {
        let client = self.pool.get().await.map_err(secret_db_error)?;
        let name = normalize_secret_name(name);
        let secret = postgres_get_secret(&client, user_id, &name)
            .await?
            .ok_or_else(|| SecretError::NotFound(name.clone()))?;
        ensure_secret_not_expired(&secret)?;
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
        let mut client = self.pool.get().await.map_err(secret_db_error)?;
        let transaction = client.transaction().await.map_err(secret_db_error)?;
        let result = async {
            let name = normalize_secret_name(name);
            let Some(secret) = postgres_get_secret_for_update(&transaction, user_id, &name).await?
            else {
                return Ok(SecretConsumeResult::NotFound);
            };
            ensure_secret_not_expired(&secret)?;
            let aad = secret_record_aad(user_id, &secret.name);
            let decrypted = self
                .crypto
                .decrypt(&secret.encrypted_value, &secret.key_salt, &aad)?;
            if decrypted.expose() != expected_value {
                return Ok(SecretConsumeResult::Mismatched);
            }
            postgres_delete_secret(&transaction, user_id, &name).await?;
            Ok(SecretConsumeResult::Matched)
        }
        .await;
        match result {
            Ok(value) => {
                transaction.commit().await.map_err(secret_db_error)?;
                Ok(value)
            }
            Err(error) => {
                let _ = transaction.rollback().await;
                Err(error)
            }
        }
    }

    async fn exists(&self, user_id: &str, name: &str) -> Result<bool, SecretError> {
        let client = self.pool.get().await.map_err(secret_db_error)?;
        let name = normalize_secret_name(name);
        Ok(postgres_get_secret(&client, user_id, &name)
            .await?
            .is_some())
    }

    async fn any_exist(&self) -> Result<bool, SecretError> {
        let client = self.pool.get().await.map_err(secret_db_error)?;
        Ok(client
            .query_opt("SELECT 1 FROM reborn_secret_records LIMIT 1", &[])
            .await
            .map_err(secret_db_error)?
            .is_some())
    }

    async fn list(&self, user_id: &str) -> Result<Vec<SecretRef>, SecretError> {
        let client = self.pool.get().await.map_err(secret_db_error)?;
        let rows = client
            .query(
                "SELECT name, provider FROM reborn_secret_records WHERE user_id = $1 ORDER BY name",
                &[&user_id],
            )
            .await
            .map_err(secret_db_error)?;
        Ok(rows
            .into_iter()
            .map(|row| SecretRef {
                name: row.get(0),
                provider: row.get(1),
            })
            .collect())
    }

    async fn delete(&self, user_id: &str, name: &str) -> Result<bool, SecretError> {
        let client = self.pool.get().await.map_err(secret_db_error)?;
        let name = normalize_secret_name(name);
        postgres_delete_secret(&client, user_id, &name).await
    }

    async fn record_usage(&self, secret_id: Uuid) -> Result<(), SecretError> {
        let client = self.pool.get().await.map_err(secret_db_error)?;
        let changed = client
            .execute(
                "UPDATE reborn_secret_records SET last_used_at = $1, usage_count = usage_count + 1, updated_at = $1 WHERE id = $2",
                &[&Utc::now().to_rfc3339(), &secret_id.to_string()],
            )
            .await
            .map_err(secret_db_error)?;
        if changed == 0 {
            return Err(SecretError::NotFound(secret_id.to_string()));
        }
        Ok(())
    }

    async fn is_accessible(
        &self,
        user_id: &str,
        secret_name: &str,
        allowed_secrets: &[String],
    ) -> Result<bool, SecretError> {
        secret_accessible(self, user_id, secret_name, allowed_secrets).await
    }
}

#[cfg(feature = "libsql")]
async fn libsql_secret_connect(db: &libsql::Database) -> Result<libsql::Connection, SecretError> {
    let conn = db.connect().map_err(secret_db_error)?;
    conn.query("PRAGMA busy_timeout = 5000", ())
        .await
        .map_err(secret_db_error)?;
    Ok(conn)
}

#[cfg(feature = "libsql")]
async fn libsql_secret_begin_immediate(
    db: &libsql::Database,
) -> Result<libsql::Connection, SecretError> {
    let conn = libsql_secret_connect(db).await?;
    conn.execute("BEGIN IMMEDIATE", ())
        .await
        .map_err(secret_db_error)?;
    Ok(conn)
}

#[cfg(feature = "libsql")]
async fn finish_libsql_secret_transaction<T>(
    conn: &libsql::Connection,
    result: Result<T, SecretError>,
) -> Result<T, SecretError> {
    match result {
        Ok(value) => {
            conn.execute("COMMIT", ()).await.map_err(secret_db_error)?;
            Ok(value)
        }
        Err(error) => {
            let _ = conn.execute("ROLLBACK", ()).await;
            Err(error)
        }
    }
}

#[cfg(feature = "libsql")]
async fn libsql_verify_or_bootstrap_secret_store_key_check(
    conn: &libsql::Connection,
    crypto: &SecretsCrypto,
) -> Result<(), SecretError> {
    if let Some((encrypted_value, key_salt)) = libsql_secret_store_key_check(conn).await? {
        return verify_secret_store_key_check(crypto, &encrypted_value, &key_salt);
    }
    // Legacy/bootstrap path for stores created before the key-check row existed:
    // validate every existing row before installing the sentinel. This is a
    // one-time migration/readiness cost; steady-state checks use the sentinel.
    libsql_verify_all_secret_payloads(conn, crypto).await?;
    libsql_insert_secret_store_key_check(conn, crypto).await?;
    let Some((encrypted_value, key_salt)) = libsql_secret_store_key_check(conn).await? else {
        return Err(SecretError::Database(
            "secret store key check missing after bootstrap".to_string(),
        ));
    };
    verify_secret_store_key_check(crypto, &encrypted_value, &key_salt)
}

#[cfg(feature = "libsql")]
async fn libsql_secret_store_key_check(
    conn: &libsql::Connection,
) -> Result<Option<(Vec<u8>, Vec<u8>)>, SecretError> {
    let mut rows = conn
        .query(
            "SELECT encrypted_value, key_salt FROM reborn_secret_store_key_check WHERE id = ?1",
            libsql::params![SECRET_STORE_KEY_CHECK_ID],
        )
        .await
        .map_err(secret_db_error)?;
    let Some(row) = rows.next().await.map_err(secret_db_error)? else {
        return Ok(None);
    };
    Ok(Some((
        row.get(0).map_err(secret_db_error)?,
        row.get(1).map_err(secret_db_error)?,
    )))
}

#[cfg(feature = "libsql")]
async fn libsql_verify_all_secret_payloads(
    conn: &libsql::Connection,
    crypto: &SecretsCrypto,
) -> Result<(), SecretError> {
    let mut rows = conn
        .query(
            "SELECT user_id, name, encrypted_value, key_salt FROM reborn_secret_records \
             ORDER BY user_id, name",
            (),
        )
        .await
        .map_err(secret_db_error)?;
    while let Some(row) = rows.next().await.map_err(secret_db_error)? {
        let user_id: String = row.get(0).map_err(secret_db_error)?;
        let name: String = row.get(1).map_err(secret_db_error)?;
        let encrypted_value: Vec<u8> = row.get(2).map_err(secret_db_error)?;
        let key_salt: Vec<u8> = row.get(3).map_err(secret_db_error)?;
        let aad = secret_record_aad(&user_id, &name);
        crypto.decrypt(&encrypted_value, &key_salt, &aad)?;
    }
    Ok(())
}

#[cfg(feature = "libsql")]
async fn libsql_insert_secret_store_key_check(
    conn: &libsql::Connection,
    crypto: &SecretsCrypto,
) -> Result<(), SecretError> {
    let aad = secret_store_key_check_aad();
    let (encrypted_value, key_salt) =
        crypto.encrypt(SECRET_STORE_KEY_CHECK_PLAINTEXT.as_bytes(), &aad)?;
    let now = Utc::now().to_rfc3339();
    conn.execute(
        "INSERT OR IGNORE INTO reborn_secret_store_key_check (id, encrypted_value, key_salt, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?4)",
        libsql::params![SECRET_STORE_KEY_CHECK_ID, encrypted_value, key_salt, now],
    )
    .await
    .map_err(secret_db_error)?;
    Ok(())
}

#[cfg(feature = "libsql")]
async fn libsql_upsert_secret(
    conn: &libsql::Connection,
    secret: &Secret,
) -> Result<Secret, SecretError> {
    conn.execute(
        "INSERT INTO reborn_secret_records (user_id, name, id, encrypted_value, key_salt, provider, expires_at, last_used_at, usage_count, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11) ON CONFLICT(user_id, name) DO UPDATE SET encrypted_value = EXCLUDED.encrypted_value, key_salt = EXCLUDED.key_salt, provider = EXCLUDED.provider, expires_at = EXCLUDED.expires_at, updated_at = EXCLUDED.updated_at",
        libsql::params![
            secret.user_id.clone(),
            secret.name.clone(),
            secret.id.to_string(),
            secret.encrypted_value.clone(),
            secret.key_salt.clone(),
            secret.provider.clone(),
            secret.expires_at.map(|value| value.to_rfc3339()),
            secret.last_used_at.map(|value| value.to_rfc3339()),
            secret.usage_count,
            secret.created_at.to_rfc3339(),
            secret.updated_at.to_rfc3339(),
        ],
    )
    .await
    .map_err(secret_db_error)?;
    libsql_get_secret(conn, &secret.user_id, &secret.name)
        .await?
        .ok_or_else(|| {
            SecretError::Database("secret upsert succeeded but row was not found".to_string())
        })
}

#[cfg(feature = "libsql")]
async fn libsql_get_secret(
    conn: &libsql::Connection,
    user_id: &str,
    name: &str,
) -> Result<Option<Secret>, SecretError> {
    let mut rows = conn
        .query(
            "SELECT id, user_id, name, encrypted_value, key_salt, provider, expires_at, last_used_at, usage_count, created_at, updated_at FROM reborn_secret_records WHERE user_id = ?1 AND name = ?2",
            libsql::params![user_id, name],
        )
        .await
        .map_err(secret_db_error)?;
    let Some(row) = rows.next().await.map_err(secret_db_error)? else {
        return Ok(None);
    };
    libsql_secret_from_row(&row).map(Some)
}

#[cfg(feature = "libsql")]
fn libsql_secret_from_row(row: &libsql::Row) -> Result<Secret, SecretError> {
    let id: String = row.get(0).map_err(secret_db_error)?;
    let expires_at: Option<String> = row.get(6).map_err(secret_db_error)?;
    let last_used_at: Option<String> = row.get(7).map_err(secret_db_error)?;
    Ok(Secret {
        id: parse_secret_uuid(&id)?,
        user_id: row.get(1).map_err(secret_db_error)?,
        name: row.get(2).map_err(secret_db_error)?,
        encrypted_value: row.get(3).map_err(secret_db_error)?,
        key_salt: row.get(4).map_err(secret_db_error)?,
        provider: row.get(5).map_err(secret_db_error)?,
        expires_at: parse_optional_timestamp(expires_at.as_deref())?,
        last_used_at: parse_optional_timestamp(last_used_at.as_deref())?,
        usage_count: row.get(8).map_err(secret_db_error)?,
        created_at: parse_timestamp(&row.get::<String>(9).map_err(secret_db_error)?)?,
        updated_at: parse_timestamp(&row.get::<String>(10).map_err(secret_db_error)?)?,
    })
}

#[cfg(feature = "libsql")]
async fn libsql_delete_secret(
    conn: &libsql::Connection,
    user_id: &str,
    name: &str,
) -> Result<bool, SecretError> {
    let changed = conn
        .execute(
            "DELETE FROM reborn_secret_records WHERE user_id = ?1 AND name = ?2",
            libsql::params![user_id, name],
        )
        .await
        .map_err(secret_db_error)?;
    Ok(changed > 0)
}

#[cfg(feature = "postgres")]
async fn postgres_verify_or_bootstrap_secret_store_key_check(
    client: &impl deadpool_postgres::GenericClient,
    crypto: &SecretsCrypto,
) -> Result<(), SecretError> {
    if let Some((encrypted_value, key_salt)) = postgres_secret_store_key_check(client).await? {
        return verify_secret_store_key_check(crypto, &encrypted_value, &key_salt);
    }
    // Legacy/bootstrap path for stores created before the key-check row existed:
    // validate every existing row before installing the sentinel. This is a
    // one-time migration/readiness cost; steady-state checks use the sentinel.
    postgres_verify_all_secret_payloads(client, crypto).await?;
    postgres_insert_secret_store_key_check(client, crypto).await?;
    let Some((encrypted_value, key_salt)) = postgres_secret_store_key_check(client).await? else {
        return Err(SecretError::Database(
            "secret store key check missing after bootstrap".to_string(),
        ));
    };
    verify_secret_store_key_check(crypto, &encrypted_value, &key_salt)
}

#[cfg(feature = "postgres")]
async fn postgres_secret_store_key_check(
    client: &impl deadpool_postgres::GenericClient,
) -> Result<Option<(Vec<u8>, Vec<u8>)>, SecretError> {
    let row = client
        .query_opt(
            "SELECT encrypted_value, key_salt FROM reborn_secret_store_key_check WHERE id = $1",
            &[&SECRET_STORE_KEY_CHECK_ID],
        )
        .await
        .map_err(secret_db_error)?;
    Ok(row.map(|row| (row.get(0), row.get(1))))
}

#[cfg(feature = "postgres")]
async fn postgres_verify_all_secret_payloads(
    client: &impl deadpool_postgres::GenericClient,
    crypto: &SecretsCrypto,
) -> Result<(), SecretError> {
    let rows = client
        .query(
            "SELECT user_id, name, encrypted_value, key_salt FROM reborn_secret_records \
             ORDER BY user_id, name",
            &[],
        )
        .await
        .map_err(secret_db_error)?;
    for row in rows {
        let user_id: String = row.get(0);
        let name: String = row.get(1);
        let encrypted_value: Vec<u8> = row.get(2);
        let key_salt: Vec<u8> = row.get(3);
        let aad = secret_record_aad(&user_id, &name);
        crypto.decrypt(&encrypted_value, &key_salt, &aad)?;
    }
    Ok(())
}

#[cfg(feature = "postgres")]
async fn postgres_insert_secret_store_key_check(
    client: &impl deadpool_postgres::GenericClient,
    crypto: &SecretsCrypto,
) -> Result<(), SecretError> {
    let aad = secret_store_key_check_aad();
    let (encrypted_value, key_salt) =
        crypto.encrypt(SECRET_STORE_KEY_CHECK_PLAINTEXT.as_bytes(), &aad)?;
    let now = Utc::now().to_rfc3339();
    client
        .execute(
            "INSERT INTO reborn_secret_store_key_check (id, encrypted_value, key_salt, created_at, updated_at) VALUES ($1, $2, $3, $4, $4) ON CONFLICT(id) DO NOTHING",
            &[&SECRET_STORE_KEY_CHECK_ID, &encrypted_value, &key_salt, &now],
        )
        .await
        .map_err(secret_db_error)?;
    Ok(())
}

#[cfg(feature = "postgres")]
async fn postgres_upsert_secret(
    client: &impl deadpool_postgres::GenericClient,
    secret: &Secret,
) -> Result<Secret, SecretError> {
    let row = client.query_one("INSERT INTO reborn_secret_records (user_id, name, id, encrypted_value, key_salt, provider, expires_at, last_used_at, usage_count, created_at, updated_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11) ON CONFLICT(user_id, name) DO UPDATE SET encrypted_value = EXCLUDED.encrypted_value, key_salt = EXCLUDED.key_salt, provider = EXCLUDED.provider, expires_at = EXCLUDED.expires_at, updated_at = EXCLUDED.updated_at RETURNING id, user_id, name, encrypted_value, key_salt, provider, expires_at, last_used_at, usage_count, created_at, updated_at", &[&secret.user_id, &secret.name, &secret.id.to_string(), &secret.encrypted_value, &secret.key_salt, &secret.provider, &secret.expires_at.map(|value| value.to_rfc3339()), &secret.last_used_at.map(|value| value.to_rfc3339()), &secret.usage_count, &secret.created_at.to_rfc3339(), &secret.updated_at.to_rfc3339()]).await.map_err(secret_db_error)?;
    postgres_secret_from_row(&row)
}

#[cfg(feature = "postgres")]
async fn postgres_get_secret(
    client: &impl deadpool_postgres::GenericClient,
    user_id: &str,
    name: &str,
) -> Result<Option<Secret>, SecretError> {
    postgres_get_secret_query(client, user_id, name, false).await
}

#[cfg(feature = "postgres")]
async fn postgres_get_secret_for_update(
    client: &impl deadpool_postgres::GenericClient,
    user_id: &str,
    name: &str,
) -> Result<Option<Secret>, SecretError> {
    postgres_get_secret_query(client, user_id, name, true).await
}

#[cfg(feature = "postgres")]
async fn postgres_get_secret_query(
    client: &impl deadpool_postgres::GenericClient,
    user_id: &str,
    name: &str,
    for_update: bool,
) -> Result<Option<Secret>, SecretError> {
    let suffix = if for_update { " FOR UPDATE" } else { "" };
    let query = format!(
        "SELECT id, user_id, name, encrypted_value, key_salt, provider, expires_at, last_used_at, usage_count, created_at, updated_at FROM reborn_secret_records WHERE user_id = $1 AND name = $2{suffix}"
    );
    let row = client
        .query_opt(&query, &[&user_id, &name])
        .await
        .map_err(secret_db_error)?;
    row.map(|row| postgres_secret_from_row(&row)).transpose()
}

#[cfg(feature = "postgres")]
fn postgres_secret_from_row(row: &tokio_postgres::Row) -> Result<Secret, SecretError> {
    let id: String = row.get(0);
    let expires_at: Option<String> = row.get(6);
    let last_used_at: Option<String> = row.get(7);
    Ok(Secret {
        id: parse_secret_uuid(&id)?,
        user_id: row.get(1),
        name: row.get(2),
        encrypted_value: row.get(3),
        key_salt: row.get(4),
        provider: row.get(5),
        expires_at: parse_optional_timestamp(expires_at.as_deref())?,
        last_used_at: parse_optional_timestamp(last_used_at.as_deref())?,
        usage_count: row.get(8),
        created_at: parse_timestamp(&row.get::<_, String>(9))?,
        updated_at: parse_timestamp(&row.get::<_, String>(10))?,
    })
}

#[cfg(feature = "postgres")]
async fn postgres_delete_secret(
    client: &impl deadpool_postgres::GenericClient,
    user_id: &str,
    name: &str,
) -> Result<bool, SecretError> {
    let changed = client
        .execute(
            "DELETE FROM reborn_secret_records WHERE user_id = $1 AND name = $2",
            &[&user_id, &name],
        )
        .await
        .map_err(secret_db_error)?;
    Ok(changed > 0)
}

fn verify_secret_store_key_check(
    crypto: &SecretsCrypto,
    encrypted_value: &[u8],
    key_salt: &[u8],
) -> Result<(), SecretError> {
    let aad = secret_store_key_check_aad();
    let decrypted = crypto.decrypt(encrypted_value, key_salt, &aad)?;
    if decrypted.expose() != SECRET_STORE_KEY_CHECK_PLAINTEXT {
        return Err(SecretError::DecryptionFailed(
            "secret store key check mismatch".to_string(),
        ));
    }
    Ok(())
}

fn build_encrypted_secret(
    user_id: &str,
    params: CreateSecretParams,
    crypto: &SecretsCrypto,
) -> Result<Secret, SecretError> {
    let plaintext = params.value.expose_secret().as_bytes();
    let name = normalize_secret_name(&params.name);
    let aad = secret_record_aad(user_id, &name);
    let (encrypted_value, key_salt) = crypto.encrypt(plaintext, &aad)?;
    let now = Utc::now();
    Ok(Secret {
        id: Uuid::new_v4(),
        user_id: user_id.to_string(),
        name,
        encrypted_value,
        key_salt,
        provider: params.provider,
        expires_at: params.expires_at,
        last_used_at: None,
        usage_count: 0,
        created_at: now,
        updated_at: now,
    })
}

fn normalize_secret_name(name: &str) -> String {
    name.to_lowercase()
}

fn ensure_secret_not_expired(secret: &Secret) -> Result<(), SecretError> {
    if let Some(expires_at) = secret.expires_at
        && expires_at < Utc::now()
    {
        return Err(SecretError::Expired);
    }
    Ok(())
}

async fn secret_accessible<S: SecretsStore + ?Sized>(
    store: &S,
    user_id: &str,
    secret_name: &str,
    allowed_secrets: &[String],
) -> Result<bool, SecretError> {
    let secret_name_lower = normalize_secret_name(secret_name);
    if !store.exists(user_id, &secret_name_lower).await? {
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

fn parse_secret_uuid(value: &str) -> Result<Uuid, SecretError> {
    Uuid::parse_str(value).map_err(secret_db_error)
}

fn parse_optional_timestamp(value: Option<&str>) -> Result<Option<DateTime<Utc>>, SecretError> {
    value.map(parse_timestamp).transpose()
}

fn parse_timestamp(value: &str) -> Result<DateTime<Utc>, SecretError> {
    DateTime::parse_from_rfc3339(value)
        .map(|timestamp| timestamp.with_timezone(&Utc))
        .map_err(secret_db_error)
}

fn secret_db_error(error: impl std::fmt::Display) -> SecretError {
    SecretError::Database(error.to_string())
}

struct EncryptedPayload {
    encrypted_value: Vec<u8>,
    key_salt: Vec<u8>,
}

#[derive(Serialize, Deserialize)]
struct PersistedCredentialSession {
    scope: ResourceScope,
    invocation_id: InvocationId,
    capability_id: CapabilityId,
    extension_id: ExtensionId,
    account_id: CredentialAccountId,
    secret_handles: Vec<SecretHandle>,
    allowed_targets: Vec<crate::CredentialTargetPolicy>,
    expires_at: Option<Timestamp>,
    max_uses: Option<u64>,
    correlation_id: String,
}

impl From<&CredentialSession> for PersistedCredentialSession {
    fn from(session: &CredentialSession) -> Self {
        Self {
            scope: session.scope.clone(),
            invocation_id: session.invocation_id,
            capability_id: session.capability_id.clone(),
            extension_id: session.extension_id.clone(),
            account_id: session.account_id.clone(),
            secret_handles: session.secret_handles.clone(),
            allowed_targets: session.allowed_targets.clone(),
            expires_at: session.expires_at,
            max_uses: session.max_uses,
            correlation_id: session.correlation_id.to_private_storage_string(),
        }
    }
}

impl TryFrom<PersistedCredentialSession> for CredentialSession {
    type Error = CredentialBrokerError;

    fn try_from(value: PersistedCredentialSession) -> Result<Self, Self::Error> {
        Ok(Self {
            scope: value.scope,
            invocation_id: value.invocation_id,
            capability_id: value.capability_id,
            extension_id: value.extension_id,
            account_id: value.account_id,
            secret_handles: value.secret_handles,
            allowed_targets: value.allowed_targets,
            expires_at: value.expires_at,
            max_uses: value.max_uses,
            correlation_id: CredentialSessionId::parse(&value.correlation_id)
                .map_err(|error| persistence_error(error.to_string()))?,
        })
    }
}

fn encrypt_credential_account_payload(
    crypto: &SecretsCrypto,
    account: &CredentialAccount,
) -> Result<EncryptedPayload, CredentialBrokerError> {
    let aad = credential_account_aad(&account.scope, &account.id);
    encrypt_json_payload(crypto, account, &aad)
}

fn decrypt_credential_account_payload(
    crypto: &SecretsCrypto,
    encrypted_value: &[u8],
    key_salt: &[u8],
    scope: &ResourceScope,
    account_id: &CredentialAccountId,
) -> Result<CredentialAccount, CredentialBrokerError> {
    let aad = credential_account_aad(scope, account_id);
    decrypt_json_payload(crypto, encrypted_value, key_salt, &aad)
}

fn encrypt_session_payload(
    crypto: &SecretsCrypto,
    session: &CredentialSession,
) -> Result<EncryptedPayload, CredentialBrokerError> {
    let aad = credential_session_aad(session.scope(), session.correlation_id());
    encrypt_json_payload(crypto, &PersistedCredentialSession::from(session), &aad)
}

fn decrypt_session_payload(
    crypto: &SecretsCrypto,
    encrypted_value: &[u8],
    key_salt: &[u8],
    scope: &ResourceScope,
    session_id: CredentialSessionId,
) -> Result<CredentialSession, CredentialBrokerError> {
    let aad = credential_session_aad(scope, session_id);
    let persisted: PersistedCredentialSession =
        decrypt_json_payload(crypto, encrypted_value, key_salt, &aad)?;
    persisted.try_into()
}

fn encrypt_json_payload<T: serde::Serialize>(
    crypto: &SecretsCrypto,
    value: &T,
    aad: &[u8],
) -> Result<EncryptedPayload, CredentialBrokerError> {
    let json = serde_json::to_vec(value).map_err(|error| persistence_error(error.to_string()))?;
    encrypt_payload_bytes(crypto, &json, aad)
}

fn encrypt_payload_bytes(
    crypto: &SecretsCrypto,
    bytes: &[u8],
    aad: &[u8],
) -> Result<EncryptedPayload, CredentialBrokerError> {
    let (encrypted_value, key_salt) = crypto
        .encrypt(bytes, aad)
        .map_err(credential_crypto_error)?;
    Ok(EncryptedPayload {
        encrypted_value,
        key_salt,
    })
}

fn decrypt_json_payload<T: serde::de::DeserializeOwned>(
    crypto: &SecretsCrypto,
    encrypted_value: &[u8],
    key_salt: &[u8],
    aad: &[u8],
) -> Result<T, CredentialBrokerError> {
    let decrypted = crypto
        .decrypt(encrypted_value, key_salt, aad)
        .map_err(credential_crypto_error)?;
    from_json(decrypted.expose())
}

fn credential_crypto_error(error: crate::SecretError) -> CredentialBrokerError {
    match error {
        crate::SecretError::InvalidMasterKey => CredentialBrokerError::BrokerUnavailable {
            reason: "credential payload encryption key is invalid".to_string(),
        },
        other => CredentialBrokerError::BrokerUnavailable {
            reason: other.to_string(),
        },
    }
}

fn validate_account_row(
    account: CredentialAccount,
    expected_scope: &ResourceScope,
    expected_account_id: &CredentialAccountId,
    row_status: &str,
    row_provider: &str,
    row_updated_at: &str,
) -> Result<CredentialAccount, CredentialBrokerError> {
    if account.id != *expected_account_id
        || account_status_key(account.status) != row_status
        || account.provider_or_extension_id.as_str() != row_provider
        || account.updated_at.to_rfc3339() != row_updated_at
        || account.scope.tenant_id != expected_scope.tenant_id
        || account.scope.user_id != expected_scope.user_id
        || account.scope.agent_id != expected_scope.agent_id
        || account.scope.project_id != expected_scope.project_id
    {
        return Err(persistence_error("credential account row payload mismatch"));
    }
    Ok(account)
}

fn validate_session_row(
    session: CredentialSession,
    expected_scope: &ResourceScope,
    expected_session_id: CredentialSessionId,
    row_account_id: &str,
    row_expires_at: Option<&str>,
    row_max_uses: Option<i64>,
    row_uses: i64,
) -> Result<SessionRecord, CredentialBrokerError> {
    if session.scope() != expected_scope
        || session.correlation_id() != expected_session_id
        || session.account_id().as_str() != row_account_id
        || session
            .expires_at()
            .map(|value| value.to_rfc3339())
            .as_deref()
            != row_expires_at
        || session.max_uses().map(|value| value as i64) != row_max_uses
        || row_uses < 0
    {
        return Err(persistence_error("credential session row payload mismatch"));
    }
    Ok(SessionRecord {
        session,
        uses: row_uses as u64,
    })
}

fn account_status_key(status: CredentialAccountStatus) -> &'static str {
    match status {
        CredentialAccountStatus::Active => "active",
        CredentialAccountStatus::Expired => "expired",
        CredentialAccountStatus::Revoked => "revoked",
    }
}

fn from_json<T: serde::de::DeserializeOwned>(payload: &str) -> Result<T, CredentialBrokerError> {
    serde_json::from_str(payload).map_err(|error| persistence_error(error.to_string()))
}

fn db_error(error: impl std::fmt::Display) -> CredentialBrokerError {
    CredentialBrokerError::BrokerUnavailable {
        reason: error.to_string(),
    }
}

fn persistence_error(reason: impl Into<String>) -> CredentialBrokerError {
    CredentialBrokerError::BrokerUnavailable {
        reason: reason.into(),
    }
}
