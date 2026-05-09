//! Durable credential-store backends for IronClaw Reborn.
//!
//! Account and session payloads are encrypted with [`SecretsCrypto`] before they
//! are written to libSQL or PostgreSQL. Operators must still enable
//! storage-layer encryption for database files, WALs, snapshots, and backups
//! because scope keys, account ids, encrypted blobs, and indexes remain durable
//! metadata outside the encrypted payload.

use std::sync::Arc;

use ironclaw_host_api::{
    CapabilityId, ExtensionId, InvocationId, ResourceScope, SecretHandle, Timestamp,
};
use serde::{Deserialize, Serialize};

const CREDENTIAL_ACCOUNTS_FOR_SCOPE_LIMIT: usize = 1000;
const CREDENTIAL_ACCOUNTS_FOR_SCOPE_QUERY_LIMIT: i64 = 1001;

use crate::{
    CredentialAccount, CredentialAccountId, CredentialAccountStatus, CredentialAccountStore,
    CredentialBrokerError, CredentialSession, CredentialSessionId, CredentialSessionStore,
    SecretsCrypto,
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
    let key = DbScopeKey::from_account_scope(&account.scope);
    let payload = encrypt_json_payload(crypto, account)?;
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
    let key = DbScopeKey::from_account_scope(scope);
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
        decrypt_json_payload(crypto, &encrypted_payload, &payload_key_salt)?,
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
    let key = DbScopeKey::from_account_scope(scope);
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
            decrypt_json_payload(crypto, &encrypted_payload, &payload_key_salt)?,
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
    let key = DbScopeKey::from_full_scope(session.scope());
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
            session.correlation_id().to_string(),
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
    let key = DbScopeKey::from_full_scope(scope);
    let mut rows = conn
        .query(
            "SELECT account_id, expires_at, max_uses, uses, encrypted_payload, payload_key_salt FROM reborn_credential_sessions WHERE tenant_id = ?1 AND user_id = ?2 AND agent_id = ?3 AND project_id = ?4 AND mission_id = ?5 AND thread_id = ?6 AND invocation_id = ?7 AND session_id = ?8",
            libsql::params![key.tenant_id, key.user_id, key.agent_id, key.project_id, key.mission_id, key.thread_id, key.invocation_id, session_id.to_string()],
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
        decrypt_session_payload(crypto, &encrypted_payload, &payload_key_salt)?,
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
    let key = DbScopeKey::from_full_scope(scope);
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
                session_id.to_string(),
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
        decrypt_session_payload(crypto, &encrypted_payload, &payload_key_salt)?,
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
    let key = DbScopeKey::from_account_scope(&account.scope);
    let payload = encrypt_json_payload(crypto, account)?;
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
    let key = DbScopeKey::from_account_scope(scope);
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
        decrypt_json_payload(crypto, &encrypted_payload, &payload_key_salt)?,
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
    let key = DbScopeKey::from_account_scope(scope);
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
            decrypt_json_payload(crypto, &encrypted_payload, &payload_key_salt)?,
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
    let key = DbScopeKey::from_full_scope(session.scope());
    let max_uses = session.max_uses().map(|value| value as i64);
    let payload = encrypt_session_payload(crypto, session)?;
    client.execute("INSERT INTO reborn_credential_sessions (tenant_id, user_id, agent_id, project_id, mission_id, thread_id, invocation_id, session_id, account_id, expires_at, max_uses, uses, payload, encrypted_payload, payload_key_salt) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, '{}'::jsonb, $13, $14) ON CONFLICT(tenant_id, user_id, agent_id, project_id, mission_id, thread_id, invocation_id, session_id) DO UPDATE SET account_id = EXCLUDED.account_id, expires_at = EXCLUDED.expires_at, max_uses = EXCLUDED.max_uses, uses = EXCLUDED.uses, payload = '{}'::jsonb, encrypted_payload = EXCLUDED.encrypted_payload, payload_key_salt = EXCLUDED.payload_key_salt", &[&key.tenant_id, &key.user_id, &key.agent_id, &key.project_id, &key.mission_id, &key.thread_id, &key.invocation_id, &session.correlation_id().to_string(), &session.account_id().as_str(), &session.expires_at().map(|value| value.to_rfc3339()), &max_uses, &(uses as i64), &payload.encrypted_value, &payload.key_salt]).await.map_err(db_error)?;
    Ok(())
}

#[cfg(feature = "postgres")]
async fn postgres_get_session_record(
    client: &impl deadpool_postgres::GenericClient,
    crypto: &SecretsCrypto,
    scope: &ResourceScope,
    session_id: CredentialSessionId,
) -> Result<Option<SessionRecord>, CredentialBrokerError> {
    let key = DbScopeKey::from_full_scope(scope);
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
                &session_id.to_string(),
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
        decrypt_session_payload(crypto, &encrypted_payload, &payload_key_salt)?,
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
    let key = DbScopeKey::from_full_scope(scope);
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
                &session_id.to_string(),
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
        decrypt_session_payload(crypto, &encrypted_payload, &payload_key_salt)?,
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
            correlation_id: session.correlation_id.to_string(),
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

fn encrypt_session_payload(
    crypto: &SecretsCrypto,
    session: &CredentialSession,
) -> Result<EncryptedPayload, CredentialBrokerError> {
    encrypt_json_payload(crypto, &PersistedCredentialSession::from(session))
}

fn decrypt_session_payload(
    crypto: &SecretsCrypto,
    encrypted_value: &[u8],
    key_salt: &[u8],
) -> Result<CredentialSession, CredentialBrokerError> {
    let persisted: PersistedCredentialSession =
        decrypt_json_payload(crypto, encrypted_value, key_salt)?;
    persisted.try_into()
}

fn encrypt_json_payload<T: serde::Serialize>(
    crypto: &SecretsCrypto,
    value: &T,
) -> Result<EncryptedPayload, CredentialBrokerError> {
    let json = serde_json::to_vec(value).map_err(|error| persistence_error(error.to_string()))?;
    encrypt_payload_bytes(crypto, &json)
}

fn encrypt_payload_bytes(
    crypto: &SecretsCrypto,
    bytes: &[u8],
) -> Result<EncryptedPayload, CredentialBrokerError> {
    let (encrypted_value, key_salt) = crypto.encrypt(bytes).map_err(credential_crypto_error)?;
    Ok(EncryptedPayload {
        encrypted_value,
        key_salt,
    })
}

fn decrypt_json_payload<T: serde::de::DeserializeOwned>(
    crypto: &SecretsCrypto,
    encrypted_value: &[u8],
    key_salt: &[u8],
) -> Result<T, CredentialBrokerError> {
    let decrypted = crypto
        .decrypt(encrypted_value, key_salt)
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

#[derive(Debug)]
struct DbScopeKey {
    tenant_id: String,
    user_id: String,
    agent_id: String,
    project_id: String,
    mission_id: String,
    thread_id: String,
    invocation_id: String,
}

impl DbScopeKey {
    fn from_account_scope(scope: &ResourceScope) -> Self {
        Self {
            tenant_id: scope.tenant_id.to_string(),
            user_id: scope.user_id.to_string(),
            agent_id: scope
                .agent_id
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_default(),
            project_id: scope
                .project_id
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_default(),
            mission_id: String::new(),
            thread_id: String::new(),
            invocation_id: String::new(),
        }
    }

    fn from_full_scope(scope: &ResourceScope) -> Self {
        Self {
            tenant_id: scope.tenant_id.to_string(),
            user_id: scope.user_id.to_string(),
            agent_id: scope
                .agent_id
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_default(),
            project_id: scope
                .project_id
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_default(),
            mission_id: scope
                .mission_id
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_default(),
            thread_id: scope
                .thread_id
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_default(),
            invocation_id: scope.invocation_id.to_string(),
        }
    }
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
